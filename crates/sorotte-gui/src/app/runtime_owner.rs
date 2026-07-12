use super::runtime_bridge::GuiSharedPlaylistOpenDispatch;

#[cfg(test)]
mod tests;

mod player;
mod player_facade;
mod plex;
mod projection;
mod requests;
mod room_transitions;
mod runtime_pump;
mod session_transport;
mod startup_player;
mod updates;

use std::{
    collections::{BTreeSet, HashMap, VecDeque, hash_map::RandomState},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use sorotte_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    persistence::{
        clear_sorotte_ini_stored_client_settings_mvp_at_path,
        load_sorotte_ini_stored_client_settings_mvp_from_path,
    },
    state::{
        ClientConfig, StoredClientSettingsMvp,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use sorotte_player_api::{LocalFileUpdate, PlayerAdapter};
use sorotte_player_mpv::MpvAdapter;
use sorotte_plex::{
    PlexClientConfig, PlexMatchCacheStagedWrite, SecretPlexPlaybackUrl,
    auth::{PlexAuthPollResult, PlexAuthService, PlexAuthSession},
    cache::PlexMatchCache,
    discovery::{PlexDiscoveryService, PlexServerConnection},
    format_plex_playlist_uri,
    http::PlexHttpClient,
    library::{PlexLibraryService, PlexStreamTarget},
    plex_server_connection_kind_from_uri,
    timeline::{PlexSyncEngine, PlexSyncState, PlexSyncStatus, PlexWatchEvent},
};

use self::updates::GuiUpdateRuntime;
use super::media_match_support::{
    MediaMatchIndexRebuildResult, MediaMatchToolProgress,
    clear_persisted_media_match_cache_at_root, probe_media_match_runtime_snapshot,
    probe_media_match_startup_snapshot,
};
use super::media_search_cache::clear_persisted_media_search_cache_at_root;
use super::mpv_launch;
use super::mpv_launch::{
    ManagedMpvProcessGuard, ManagedMpvSettingsDecision,
    apply_legacy_syncplay_ui_settings_to_mpv_adapter, managed_mpv_settings_decision_from_settings,
};
use super::runtime_bridge::GuiPendingRoomChangeRequest;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiLoopbackSessionTransportDriver,
    GuiOutboundProtocolDeliveryResult, GuiOwnedPlayer, GuiPlayerLaunchRuntimeState,
    GuiQueuedSessionTransportHandle, GuiSessionRoomPlaystate, GuiSessionRuntimeAdapter,
    GuiSessionTransportDriver, GuiTestPlayerAdapter, GuiThreadedTcpSessionTransportDriver,
};
use super::shell_state::{
    GuiCommandAvailabilityState, GuiConfigurationRuntimeSnapshot,
    GuiMediaMatchRemediationRuntimeSnapshot, GuiMediaMatchRuntimeSnapshot, GuiMediaMatchState,
    GuiMediaSourceProviderId, GuiPlexPlaylistSearchResult, GuiPlexRuntimeSnapshot,
    GuiPlexServerReachability, GuiPlexServerRow, GuiPluginSelection, GuiShellAction,
    GuiStreamHelperRemediationRuntimeSnapshot, GuiStreamHelperRuntimeSnapshot,
    GuiTransientNotificationLevel, SorotteGuiShellAppState,
};
use super::startup::{
    StartupPublicServerOutcome, explicit_mpv_ipc_path_from_lookup,
    gui_startup_public_server_outcome_with_fetcher,
    resolve_sorotte_gui_config_path_legacy_compatible,
};
use super::startup_support::{env_flag_enabled_lookup, env_trimmed};
use super::stream_support::{
    StreamHelperAttachMode, managed_stream_helper_downloader_path,
    managed_stream_helper_path_prefixes, probe_stream_helper_runtime_snapshot,
    probe_stream_helper_startup_snapshot,
};
use super::support::system_time_seconds;
use super::ui_state::clear_legacy_gui_qsettings_files_at_root;

const STARTUP_PUBLIC_SERVER_MAX_ATTEMPTS: u8 = 3;
const STARTUP_PUBLIC_SERVER_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StartupPublicServerHydrationContext {
    language: String,
}

impl StartupPublicServerHydrationContext {
    fn from_settings(settings: &StoredClientSettingsMvp) -> Option<Self> {
        if settings.check_for_updates_automatically != Some(true)
            || settings
                .public_servers
                .as_ref()
                .is_some_and(|servers| !servers.is_empty())
        {
            return None;
        }
        let language = settings
            .language
            .as_deref()
            .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
            .unwrap_or("en")
            .to_owned();
        Some(Self { language })
    }
}

#[derive(Debug, Default)]
pub(super) struct StartupPublicServerHydrationState {
    pub(super) attempts_started: u8,
    pub(super) next_retry_at: Option<Instant>,
    pub(super) last_warning: Option<String>,
    pub(super) completed: bool,
    pub(super) context: Option<StartupPublicServerHydrationContext>,
}

pub(super) struct GuiPersistedConfigRuntimeOwner {
    pub(super) config_path: Option<PathBuf>,
    pub(super) legacy_projection: Option<SorotteGuiShellAppState>,
    pub(super) session: Option<Box<dyn GuiSessionRuntimeAdapter + Send>>,
    pub(super) session_projects_to_shell: bool,
    pub(super) session_transport: Option<GuiQueuedSessionTransportHandle>,
    pub(super) session_transport_driver: Option<Box<dyn GuiSessionTransportDriver + Send>>,
    pub(super) session_transport_reconnect_due_at: Option<Instant>,
    pub(super) session_transport_reconnect_failures: u32,
    pub(super) session_transport_disconnect_pending_cleanup: bool,
    pub(super) runtime_pump_generation: u64,
    pub(super) session_default_room: Option<String>,
    pub(super) pending_room_change_request: Option<GuiPendingRoomChangeRequest>,
    pub(super) startup_saved_connect_attempted: bool,
    pub(super) startup_remote_actions_attempted: bool,
    pub(super) startup_remote_actions_rx: Option<mpsc::Receiver<StartupPublicServerOutcome>>,
    pub(super) startup_public_server_hydration: StartupPublicServerHydrationState,
    pub(super) update_runtime: GuiUpdateRuntime,
    pub(super) startup_stream_helper_probe_completed: bool,
    pub(super) startup_stream_helper_probe_rx:
        Option<mpsc::Receiver<GuiStreamHelperRuntimeSnapshot>>,
    pub(super) player: Option<GuiOwnedPlayer>,
    pub(super) player_launch_state: GuiPlayerLaunchRuntimeState,
    pub(super) managed_mpv_process: Option<ManagedMpvProcessGuard>,
    pub(super) player_unavailability_reason: Option<String>,
    pub(super) player_local_file: Option<LocalFileUpdate>,
    pub(super) player_local_file_placeholder: bool,
    pub(super) last_published_local_file: Option<LocalFileUpdate>,
    pub(super) last_published_media_match_signature: Option<serde_json::Value>,
    pub(super) local_shared_playlist_media_match_signature_path: Option<String>,
    pub(super) attached_media_search_index: Option<GuiAttachedMediaSearchIndex>,
    pub(super) attached_media_search_next_retry_at: Option<Instant>,
    pub(super) pending_attached_media_resolution: Option<GuiPendingAttachedMediaResolution>,
    pub(super) attached_media_search_progress: Option<GuiAttachedMediaSearchBuildProgress>,
    pub(super) attached_media_search_progress_updated_at: Option<Instant>,
    pub(super) attached_media_search_build_state: GuiAttachedMediaSearchBuildState,
    pub(super) attached_media_search_build_roots: Vec<String>,
    pub(super) attached_media_search_index_revision: u64,
    pub(super) unresolved_attached_media_target: Option<String>,
    pub(super) last_attached_media_resolution_trigger: Option<GuiAutomaticMediaResolutionTrigger>,
    pub(super) last_applied_attached_room_playstate: Option<GuiSessionRoomPlaystate>,
    pub(super) suppressed_attached_room_playstate_after_playlist_reset:
        Option<GuiSessionRoomPlaystate>,
    pub(super) pending_local_attached_pause_override: Option<bool>,
    pub(super) pending_attached_cache_unpause: bool,
    pub(super) pending_attached_player_pause_confirmation_pump: Option<u64>,
    pub(super) pending_attached_player_pause_command: Option<GuiPendingAttachedPlayerPauseCommand>,
    pub(super) player_position_seconds: Option<f64>,
    pub(super) player_paused: Option<bool>,
    pub(super) player_paused_for_cache: Option<bool>,
    pub(super) player_cache_buffering_percent: Option<f64>,
    pub(super) active_shared_playlist_index: Option<usize>,
    pub(super) playlist_auto_advance_eof_latched: bool,
    pub(super) user_offset_seconds: f64,
    pub(super) stream_helper_runtime_snapshot: GuiStreamHelperRuntimeSnapshot,
    pub(super) stream_helper_remediation_runtime_snapshot:
        GuiStreamHelperRemediationRuntimeSnapshot,
    pub(super) media_match_runtime_snapshot: GuiMediaMatchRuntimeSnapshot,
    pub(super) media_match_remediation_runtime_snapshot: GuiMediaMatchRemediationRuntimeSnapshot,
    pub(super) media_match_tool_worker_rx: Option<mpsc::Receiver<GuiMediaMatchToolWorkerEvent>>,
    pub(super) media_match_background_worker_rx:
        Option<mpsc::Receiver<GuiMediaMatchBackgroundWorkerEvent>>,
    pub(super) media_match_background_worker_cancel: Option<Arc<AtomicBool>>,
    pub(super) media_match_background_trigger_key: Option<String>,
    pub(super) media_match_background_index_backup: Option<GuiMediaMatchIndexRebuildBackup>,
    pub(super) media_match_background_cancel_disposition:
        Option<GuiMediaMatchBackgroundCancelDisposition>,
    pub(super) media_match_remote_lookup_rx:
        Option<mpsc::Receiver<GuiMediaMatchRemoteLookupResult>>,
    pub(super) media_match_remote_lookup_trigger_key: Option<String>,
    pub(super) media_match_remote_lookup_result: Option<GuiMediaMatchRemoteLookupResult>,
    pub(super) media_match_wire_sync_token: Option<String>,
    pub(super) plex_client: Option<PlexHttpClient>,
    pub(super) plex_auth_session: Option<PlexAuthSession>,
    pub(super) plex_auth_start_rx: Option<mpsc::Receiver<Result<PlexAuthSession, String>>>,
    pub(super) plex_auth_poll_rx:
        Option<mpsc::Receiver<(bool, Result<PlexAuthPollResult, String>)>>,
    pub(super) plex_auth_poll_due_at: Option<Instant>,
    pub(super) plex_servers: Vec<PlexServerConnection>,
    pub(super) plex_server_reachability: HashMap<String, GuiPlexServerReachability>,
    pub(super) startup_plex_server_refresh_attempted: bool,
    pub(super) plex_server_discovery: GuiPlexServerDiscoveryCoordinator,
    pub(super) plex_sync_engine: Option<PlexSyncEngine<PlexHttpClient>>,
    pub(super) plex_sync_rx: Option<mpsc::Receiver<GuiPlexSyncWorkerResult>>,
    pub(super) plex_sync_next_tick_due_at: Option<Instant>,
    pub(super) plex_runtime_snapshot: GuiPlexRuntimeSnapshot,
    pub(super) plex_playlist_search_rx: Option<mpsc::Receiver<GuiPlexPlaylistSearchWorkerResult>>,
    pub(super) plex_playlist_resolve_rx: Option<mpsc::Receiver<GuiPlexPlaylistResolveWorkerResult>>,
    pub(super) plex_stream_resolve_rx: Option<mpsc::Receiver<GuiPlexStreamResolveWorkerResult>>,
    pub(super) plex_stream_resolve_trigger_key: Option<String>,
    pub(super) plex_stream_resolve_context: Option<GuiPlexOperationContext>,
    pub(super) plex_stream_resolve_result: Option<GuiPlexStreamResolveWorkerResult>,
    pub(super) pending_playlist_source_resolution: Option<GuiPendingPlaylistSourceResolution>,
    pub(super) pending_stream_retry_target: Option<String>,
    pub(super) managed_stream_helper_refresh_required: bool,
    pub(super) pending_stream_feedback: VecDeque<Vec<GuiShellAction>>,
    pub(super) pending_stream_load_context: Option<GuiPendingStreamLoadContext>,
    pub(super) pending_logical_media_override: Option<GuiPendingLogicalMediaOverride>,
}

pub(super) const ATTACHED_PLAYER_PAUSE_COMMAND_SUPPRESSION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GuiPendingAttachedPlayerPauseCommand {
    pub(super) target_paused: bool,
    pub(super) suppress_until: Instant,
}

pub(super) struct GuiPlexServerRefreshOutcome {
    pub(super) servers: Vec<PlexServerConnection>,
    pub(super) reachability: HashMap<String, GuiPlexServerReachability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPlexServerRefreshContext {
    Startup,
    Manual,
    Login,
}

#[derive(Default)]
pub(super) struct GuiPlexServerDiscoveryCoordinator {
    pub(super) generation: u64,
    pub(super) identity_generation: u64,
    token_fingerprint_state: RandomState,
    pub(super) active: Option<GuiPlexServerDiscoveryJob>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GuiPlexOperationContext {
    pub(super) identity_generation: u64,
    user_token_fingerprint: Option<u64>,
    selected_server_token_fingerprint: Option<u64>,
    pub(super) selected_server_id: Option<String>,
    pub(super) selected_server_url: Option<String>,
    pub(super) plugin_enabled: bool,
    pub(super) sync_enabled: bool,
    pub(super) streaming_enabled: bool,
}

impl std::fmt::Debug for GuiPlexOperationContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlexOperationContext")
            .field("identity_generation", &self.identity_generation)
            .field("authenticated", &self.user_token_fingerprint.is_some())
            .field(
                "has_selected_server_token",
                &self.selected_server_token_fingerprint.is_some(),
            )
            .field("selected_server_id", &self.selected_server_id)
            .field(
                "selected_server_url_configured",
                &self.selected_server_url.is_some(),
            )
            .field("plugin_enabled", &self.plugin_enabled)
            .field("sync_enabled", &self.sync_enabled)
            .field("streaming_enabled", &self.streaming_enabled)
            .finish()
    }
}

pub(super) struct GuiPlexServerDiscoveryJob {
    pub(super) generation: u64,
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) context: GuiPlexServerRefreshContext,
    pub(super) receiver: mpsc::Receiver<GuiPlexServerDiscoveryWorkerResult>,
}

pub(super) struct GuiPlexServerDiscoveryWorkerResult {
    pub(super) generation: u64,
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) context: GuiPlexServerRefreshContext,
    pub(super) result: Result<GuiPlexServerRefreshOutcome, String>,
}

pub(super) struct GuiPlexSyncWorkerResult {
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) engine: PlexSyncEngine<PlexHttpClient>,
    pub(super) status: PlexSyncStatus,
    pub(super) staged_cache_write: Option<Result<PlexMatchCacheStagedWrite, String>>,
}

pub(super) struct GuiPlexPlaylistSearchWorkerResult {
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) query: String,
    pub(super) result: Result<Vec<GuiPlexPlaylistSearchResult>, String>,
}

pub(super) struct GuiPlexPlaylistResolveWorkerResult {
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) rating_key: String,
    pub(super) result: Result<String, String>,
}

#[derive(Clone, PartialEq)]
pub(super) struct GuiPlexStreamResolveOutcome {
    pub(super) stream_target: Option<PlexStreamTarget>,
    pub(super) cache: PlexMatchCache,
}

impl std::fmt::Debug for GuiPlexStreamResolveOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlexStreamResolveOutcome")
            .field("stream_target_resolved", &self.stream_target.is_some())
            .field("cache", &sorotte_secret::REDACTED_SECRET)
            .finish()
    }
}

pub(super) struct GuiPlexStreamResolveWorkerResult {
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) trigger_key: String,
    pub(super) result: Result<GuiPlexStreamResolveOutcome, String>,
    pub(super) staged_cache_write: Option<Result<PlexMatchCacheStagedWrite, String>>,
}

impl std::fmt::Debug for GuiPlexStreamResolveWorkerResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlexStreamResolveWorkerResult")
            .field("trigger_key", &sorotte_secret::REDACTED_SECRET)
            .field("result_succeeded", &self.result.is_ok())
            .field("cache_write_staged", &self.staged_cache_write.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GuiPendingPlaylistSourceResolution {
    pub(super) index: usize,
    pub(super) target: String,
    pub(super) provider_id: GuiMediaSourceProviderId,
}

impl std::fmt::Debug for GuiPendingPlaylistSourceResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPendingPlaylistSourceResolution")
            .field("index", &self.index)
            .field("target", &sorotte_secret::REDACTED_SECRET)
            .field("provider_id", &self.provider_id)
            .finish()
    }
}

#[derive(Debug)]
pub(super) enum GuiMediaMatchToolWorkerEvent {
    Progress(MediaMatchToolProgress),
    Finished {
        result: Result<String, String>,
        failure_label: &'static str,
    },
}

pub(super) enum GuiMediaMatchBackgroundWorkerEvent {
    Progress(MediaMatchToolProgress),
    Finished(Result<MediaMatchIndexRebuildResult, String>),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GuiMediaMatchRemoteLookupResult {
    pub(super) trigger_key: String,
    pub(super) candidate_path: Option<String>,
}

impl std::fmt::Debug for GuiMediaMatchRemoteLookupResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiMediaMatchRemoteLookupResult")
            .field("trigger_key", &sorotte_secret::REDACTED_SECRET)
            .field(
                "candidate_path",
                &self
                    .candidate_path
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiMediaMatchIndexRebuildBackup {
    pub(super) root: PathBuf,
    pub(super) backup_existed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiMediaMatchBackgroundCancelDisposition {
    RestorePrevious,
    KeepCheckpoint,
}

pub(super) struct GuiAttachedMediaSearchIndex {
    pub(super) roots: Vec<String>,
    pub(super) root_indexes_by_key: HashMap<String, GuiAttachedMediaSearchRootIndex>,
    pub(super) roots_requiring_refresh: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiAttachedMediaSearchRootIndex {
    pub(super) root_key: String,
    pub(super) root_path: PathBuf,
    pub(super) built_at_unix_ms: u64,
    pub(super) candidates_by_name: HashMap<String, Vec<String>>,
}

pub(super) struct GuiAttachedMediaSearchRootRefreshResult {
    pub(super) root_key: String,
    pub(super) index: Option<GuiAttachedMediaSearchRootIndex>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiAttachedMediaSearchBuildProgress {
    pub(super) total_roots: usize,
    pub(super) completed_roots: usize,
    pub(super) current_root_key: String,
    pub(super) current_root_path: PathBuf,
    pub(super) scanned_directories: usize,
    pub(super) indexed_files: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiAttachedMediaSearchBuildState {
    Idle,
    Queued,
    Building,
    Ready,
    Stale,
    Failed,
}

pub(super) enum GuiAttachedMediaSearchBuildStatus {
    Completed(Vec<GuiAttachedMediaSearchRootRefreshResult>),
    Cancelled,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum GuiUserMediaTargetResolution {
    Resolved {
        path: String,
        source: GuiUserMediaTargetResolutionSource,
    },
    Pending,
    Missing,
}

impl std::fmt::Debug for GuiUserMediaTargetResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resolved { source, .. } => formatter
                .debug_struct("Resolved")
                .field("path", &sorotte_secret::REDACTED_SECRET)
                .field("source", source)
                .finish(),
            Self::Pending => formatter.write_str("Pending"),
            Self::Missing => formatter.write_str("Missing"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiUserMediaTargetResolutionSource {
    QuickLocal,
    MediaMatchExactInventory,
    MediaSearchIndex,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GuiAutomaticMediaResolutionTrigger {
    pub(super) target: String,
    pub(super) source_provider: String,
    pub(super) roots: Vec<String>,
    pub(super) media_match_remote_targets: String,
    pub(super) current_player_path: Option<String>,
    pub(super) index_revision: u64,
    pub(super) retry_due: bool,
}

impl std::fmt::Debug for GuiAutomaticMediaResolutionTrigger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiAutomaticMediaResolutionTrigger")
            .field("target", &sorotte_secret::REDACTED_SECRET)
            .field("source_provider", &self.source_provider)
            .field("root_count", &self.roots.len())
            .field(
                "media_match_remote_targets",
                &sorotte_secret::REDACTED_SECRET,
            )
            .field(
                "current_player_path",
                &self
                    .current_player_path
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("index_revision", &self.index_revision)
            .field("retry_due", &self.retry_due)
            .finish()
    }
}

pub(super) struct GuiPendingAttachedMediaResolution {
    pub(super) roots: Vec<String>,
    pub(super) cancel_flag: Arc<AtomicBool>,
    pub(super) latest_progress: Arc<Mutex<Option<GuiAttachedMediaSearchBuildProgress>>>,
    pub(super) result_rx: std::sync::mpsc::Receiver<GuiAttachedMediaSearchBuildStatus>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GuiPendingStreamLoadContext {
    pub(super) requested_target: String,
    pub(super) user_initiated: bool,
}

impl std::fmt::Debug for GuiPendingStreamLoadContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPendingStreamLoadContext")
            .field("requested_target", &sorotte_secret::REDACTED_SECRET)
            .field("user_initiated", &self.user_initiated)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct GuiPendingLogicalMediaOverride {
    pub(super) requested_target: String,
    pub(super) loaded_target_secret: SecretPlexPlaybackUrl,
    pub(super) logical_file: LocalFileUpdate,
    pub(super) user_initiated: bool,
}

impl std::fmt::Debug for GuiPendingLogicalMediaOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuiPendingLogicalMediaOverride")
            .field("requested_target", &sorotte_secret::REDACTED_SECRET)
            .field("loaded_target_secret", &self.loaded_target_secret)
            .field("logical_file", &sorotte_secret::REDACTED_SECRET)
            .field("user_initiated", &self.user_initiated)
            .finish()
    }
}

impl Drop for GuiPendingAttachedMediaResolution {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}

impl GuiAttachedMediaSearchIndex {
    pub(super) fn new(roots: Vec<String>) -> Self {
        Self {
            roots,
            root_indexes_by_key: HashMap::new(),
            roots_requiring_refresh: BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod media_target_debug_tests {
    use super::*;

    #[test]
    fn media_resolution_and_stream_contexts_redact_tokenized_targets() {
        let secret = "https://media.example/item?token=runtime-owner-canary";
        let trigger = GuiAutomaticMediaResolutionTrigger {
            target: secret.to_owned(),
            source_provider: "plex".to_owned(),
            roots: vec![secret.to_owned()],
            media_match_remote_targets: secret.to_owned(),
            current_player_path: Some(secret.to_owned()),
            index_revision: 7,
            retry_due: true,
        };
        let stream_context = GuiPendingStreamLoadContext {
            requested_target: secret.to_owned(),
            user_initiated: true,
        };
        let pending_playlist = GuiPendingPlaylistSourceResolution {
            index: 2,
            target: secret.to_owned(),
            provider_id: GuiMediaSourceProviderId::plex_stream(),
        };
        let resolved = GuiUserMediaTargetResolution::Resolved {
            path: secret.to_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        };
        let remote_lookup = GuiMediaMatchRemoteLookupResult {
            trigger_key: secret.to_owned(),
            candidate_path: Some(secret.to_owned()),
        };
        let plex_worker = GuiPlexStreamResolveWorkerResult {
            operation_context: GuiPlexOperationContext {
                identity_generation: 1,
                user_token_fingerprint: Some(7),
                selected_server_token_fingerprint: Some(8),
                selected_server_id: Some("machine".to_owned()),
                selected_server_url: Some(secret.to_owned()),
                plugin_enabled: true,
                sync_enabled: true,
                streaming_enabled: true,
            },
            trigger_key: secret.to_owned(),
            result: Err(secret.to_owned()),
            staged_cache_write: None,
        };

        for debug in [
            format!("{trigger:?}"),
            format!("{stream_context:?}"),
            format!("{pending_playlist:?}"),
            format!("{resolved:?}"),
            format!("{remote_lookup:?}"),
            format!("{plex_worker:?}"),
        ] {
            assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!debug.contains("runtime-owner-canary"));
        }
    }
}
