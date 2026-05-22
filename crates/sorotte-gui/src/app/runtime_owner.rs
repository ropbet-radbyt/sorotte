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

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use sorotte_client_app::app_boundary::{
    persistence::{
        clear_sorotte_ini_stored_client_settings_mvp_at_path,
        load_sorotte_ini_stored_client_settings_mvp_from_path,
    },
    state::{StoredClientSettingsMvp, stored_client_settings_runtime_snapshot_legacy_compatible},
};
use sorotte_player_api::{LocalFileUpdate, PlayerAdapter};
use sorotte_player_mpv::MpvAdapter;
use sorotte_plex::{
    PlexAuthPollResult, PlexAuthSession, PlexClientConfig, PlexHttpClient, PlexMatchCache,
    PlexServerConnection, PlexSyncEngine, PlexSyncState, PlexSyncStatus, PlexWatchEvent,
    plex_server_connection_kind_from_uri,
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
    GuiClientCoreChatSessionRuntimeAdapter, GuiLoopbackSessionTransportDriver, GuiOwnedPlayer,
    GuiPlayerLaunchRuntimeState, GuiQueuedSessionTransportHandle, GuiSessionRoomPlaystate,
    GuiSessionRuntimeAdapter, GuiSessionTransportDriver, GuiTcpSessionTransportDriver,
    GuiTestPlayerAdapter,
};
use super::shell_state::{
    GuiCommandAvailabilityState, GuiConfigurationRuntimeSnapshot, GuiPlexRuntimeSnapshot,
    GuiPlexServerReachability, GuiPlexServerRow, GuiShellAction,
    GuiStreamHelperRemediationRuntimeSnapshot, GuiStreamHelperRuntimeSnapshot,
    GuiTransientNotificationLevel, SorotteGuiShellAppState,
};
use super::startup::{
    explicit_mpv_ipc_path_from_lookup, gui_startup_remote_actions,
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

pub(super) struct GuiPersistedConfigRuntimeOwner {
    pub(super) config_path: Option<PathBuf>,
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
    pub(super) startup_remote_actions_rx: Option<mpsc::Receiver<Vec<GuiShellAction>>>,
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
    pub(super) plex_client: Option<PlexHttpClient>,
    pub(super) plex_auth_session: Option<PlexAuthSession>,
    pub(super) plex_auth_start_rx: Option<mpsc::Receiver<Result<PlexAuthSession, String>>>,
    pub(super) plex_auth_poll_rx:
        Option<mpsc::Receiver<(bool, Result<PlexAuthPollResult, String>)>>,
    pub(super) plex_auth_poll_due_at: Option<Instant>,
    pub(super) plex_servers: Vec<PlexServerConnection>,
    pub(super) plex_server_reachability: HashMap<String, GuiPlexServerReachability>,
    pub(super) startup_plex_server_refresh_attempted: bool,
    pub(super) startup_plex_server_refresh_rx:
        Option<mpsc::Receiver<Result<GuiPlexServerRefreshOutcome, String>>>,
    pub(super) plex_server_refresh_rx:
        Option<mpsc::Receiver<Result<GuiPlexServerRefreshOutcome, String>>>,
    pub(super) plex_server_refresh_context: Option<GuiPlexServerRefreshContext>,
    pub(super) plex_sync_engine: Option<PlexSyncEngine<PlexHttpClient>>,
    pub(super) plex_sync_rx: Option<mpsc::Receiver<GuiPlexSyncWorkerResult>>,
    pub(super) plex_sync_next_tick_due_at: Option<Instant>,
    pub(super) plex_runtime_snapshot: GuiPlexRuntimeSnapshot,
    pub(super) pending_stream_retry_target: Option<String>,
    pub(super) managed_stream_helper_refresh_required: bool,
    pub(super) pending_stream_feedback: VecDeque<Vec<GuiShellAction>>,
    pub(super) pending_stream_load_context: Option<GuiPendingStreamLoadContext>,
}

pub(super) struct GuiPlexServerRefreshOutcome {
    pub(super) servers: Vec<PlexServerConnection>,
    pub(super) reachability: HashMap<String, GuiPlexServerReachability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPlexServerRefreshContext {
    Manual,
    Login,
}

pub(super) struct GuiPlexSyncWorkerResult {
    pub(super) engine: PlexSyncEngine<PlexHttpClient>,
    pub(super) status: PlexSyncStatus,
    pub(super) cache_save_error: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiUserMediaTargetResolution {
    Resolved(String),
    Pending,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiAutomaticMediaResolutionTrigger {
    pub(super) target: String,
    pub(super) roots: Vec<String>,
    pub(super) current_player_path: Option<String>,
    pub(super) index_revision: u64,
    pub(super) retry_due: bool,
}

pub(super) struct GuiPendingAttachedMediaResolution {
    pub(super) roots: Vec<String>,
    pub(super) cancel_flag: Arc<AtomicBool>,
    pub(super) latest_progress: Arc<Mutex<Option<GuiAttachedMediaSearchBuildProgress>>>,
    pub(super) result_rx: std::sync::mpsc::Receiver<GuiAttachedMediaSearchBuildStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPendingStreamLoadContext {
    pub(super) requested_target: String,
    pub(super) user_initiated: bool,
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
