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
        ClientConfig, EffectiveMpvStreamingOption, StoredClientSettingsMvp,
        StoredClientSettingsRuntimeSnapshot,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use sorotte_client_core::PlayerCommandCause;
use sorotte_player_api::{LocalFileUpdate, PlayerAdapter};
use sorotte_player_mpv::{LegacySyncplayUiSettings, MpvAdapter, SorotteBridgeHealth};
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
    apply_effective_streaming_options_to_active_network_media_classified,
    configure_effective_streaming_options_for_network_media,
    configure_sorotte_chat_osd_integration, managed_mpv_settings_decision_from_settings,
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
    FirstRunConfigurationDialogDraft, GuiMediaMatchRemediationRuntimeSnapshot,
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchState, GuiMediaSourceProviderId,
    GuiPersistedSettingsPatch, GuiPlaylistEntryId, GuiPlexPlaylistSearchResult,
    GuiPlexRuntimeSnapshot, GuiPlexServerReachability, GuiPlexServerRow, GuiPluginSelection,
    GuiSettingApplyRequirement, GuiShellAction, GuiStreamHelperRemediationRuntimeSnapshot,
    GuiStreamHelperRuntimeSnapshot, GuiTransientNotificationLevel, SettingId,
    SorotteGuiShellAppState,
};
use super::startup::{
    StartupPublicServerOutcome, explicit_mpv_ipc_path_from_lookup,
    gui_startup_public_server_outcome_with_fetcher,
    resolve_sorotte_gui_config_path_legacy_compatible, should_hydrate_startup_public_servers,
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
        if !should_hydrate_startup_public_servers(settings) {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum GuiPlayerIntegrationHealth {
    #[default]
    Ready,
    BridgeDegraded {
        reason: String,
        retryable_in_place: bool,
    },
}

impl GuiPlayerIntegrationHealth {
    fn from_sorotte_bridge_health(health: SorotteBridgeHealth) -> Self {
        match health {
            SorotteBridgeHealth::Disabled
            | SorotteBridgeHealth::Ready
            | SorotteBridgeHealth::Recovering => Self::Ready,
            SorotteBridgeHealth::Degraded(failure) => Self::BridgeDegraded {
                retryable_in_place: failure.retryable_in_place(),
                reason: failure.reason,
            },
        }
    }

    fn bridge_degraded_reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::BridgeDegraded { reason, .. } => Some(reason),
        }
    }

    fn bridge_retryable_in_place(&self) -> bool {
        matches!(
            self,
            Self::BridgeDegraded {
                retryable_in_place: true,
                ..
            }
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum GuiCorePlayerConfigurationHealth {
    #[default]
    Ready,
    StreamingDegraded {
        reason: String,
        retryable_in_place: bool,
        origin: GuiStreamingDegradationOrigin,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GuiStreamingDegradationOrigin {
    ExplicitApply,
    AuthoritativeMediaTransition,
}

impl GuiCorePlayerConfigurationHealth {
    fn streaming_degraded_reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::StreamingDegraded { reason, .. } => Some(reason),
        }
    }

    fn streaming_retryable_in_place(&self) -> bool {
        matches!(
            self,
            Self::StreamingDegraded {
                retryable_in_place: true,
                ..
            }
        )
    }
}

#[derive(Debug, Default)]
pub(super) struct GuiPlayerApplyState {
    /// The last process target and launch arguments that actually became active.
    pub(super) applied_process_target: Option<GuiPlayerProcessTarget>,
    /// The effective streaming options last accepted by the active mpv runtime.
    pub(super) applied_streaming_options: Option<Vec<EffectiveMpvStreamingOption>>,
    /// The mpv UI properties last accepted by the attached mpv process.
    pub(super) applied_mpv_ui_settings: Option<LegacySyncplayUiSettings>,
    /// The Lua bridge settings last acknowledged independently of mpv UI-property application.
    pub(super) acknowledged_bridge_settings: Option<LegacySyncplayUiSettings>,
    /// The exact Lua settings generation acknowledged for `acknowledged_bridge_settings`.
    pub(super) acknowledged_bridge_generation: Option<u64>,
    /// A process or streaming apply failed and still needs a core player retry/restart.
    pub(super) core_reapply_required: bool,
    /// An explicit apply was superseded by a newer authoritative media transition. The ordered
    /// transition result owns whether the desired streaming baseline may be promoted.
    pub(super) streaming_apply_awaiting_transition: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiPlayerProcessTarget {
    None,
    TestPlayer,
    ExplicitMpvIpc {
        ipc_path: String,
    },
    ManagedMpv {
        requested_player_path: String,
        program: PathBuf,
        extra_args: Vec<String>,
    },
    UnsupportedConfiguredPlayer {
        player_path: String,
    },
}

impl From<&GuiPlayerLaunchRuntimeState> for GuiPlayerProcessTarget {
    fn from(launch_state: &GuiPlayerLaunchRuntimeState) -> Self {
        match launch_state {
            GuiPlayerLaunchRuntimeState::None => Self::None,
            GuiPlayerLaunchRuntimeState::TestPlayer => Self::TestPlayer,
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc { ipc_path, .. } => Self::ExplicitMpvIpc {
                ipc_path: ipc_path.clone(),
            },
            GuiPlayerLaunchRuntimeState::ManagedMpv(config) => Self::ManagedMpv {
                requested_player_path: config.requested_player_path.clone(),
                program: config.program.clone(),
                extra_args: config.extra_args.clone(),
            },
            GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { player_path } => {
                Self::UnsupportedConfiguredPlayer {
                    player_path: player_path.clone(),
                }
            }
        }
    }
}

impl GuiPlayerApplyState {
    fn process_target_is_applied(&self, desired: &GuiPlayerLaunchRuntimeState) -> bool {
        self.applied_process_target.as_ref() == Some(&GuiPlayerProcessTarget::from(desired))
    }

    fn streaming_options_are_applied(&self, desired: &GuiPlayerLaunchRuntimeState) -> bool {
        desired.effective_mpv_streaming_options() == self.applied_streaming_options.as_deref()
    }

    fn mpv_ui_settings_are_applied(&self, desired: &GuiPlayerLaunchRuntimeState) -> bool {
        desired.mpv_ui_settings() == self.applied_mpv_ui_settings.as_ref()
    }

    fn bridge_settings_are_acknowledged(&self, desired: &GuiPlayerLaunchRuntimeState) -> bool {
        let desired_settings = desired.mpv_ui_settings();
        desired_settings == self.acknowledged_bridge_settings.as_ref()
            && desired_settings.is_none_or(|settings| {
                !settings.uses_syncplayintf_bridge()
                    || self.acknowledged_bridge_generation.is_some()
            })
    }

    fn record_core_apply(&mut self, launch_state: &GuiPlayerLaunchRuntimeState) {
        self.record_process_target_applied(launch_state);
        self.record_streaming_options_applied(launch_state);
    }

    fn record_process_target_applied(&mut self, launch_state: &GuiPlayerLaunchRuntimeState) {
        self.applied_process_target = Some(GuiPlayerProcessTarget::from(launch_state));
    }

    fn record_streaming_options_applied(&mut self, launch_state: &GuiPlayerLaunchRuntimeState) {
        self.applied_streaming_options = launch_state
            .effective_mpv_streaming_options()
            .map(<[_]>::to_vec);
        self.core_reapply_required = false;
        self.streaming_apply_awaiting_transition = false;
    }

    fn mark_streaming_apply_failed(&mut self) {
        self.core_reapply_required = true;
        self.streaming_apply_awaiting_transition = false;
    }

    fn mark_streaming_apply_superseded(&mut self) {
        self.core_reapply_required = false;
        self.streaming_apply_awaiting_transition = true;
    }

    fn clear_integration_baselines(&mut self) {
        self.applied_mpv_ui_settings = None;
        self.acknowledged_bridge_settings = None;
        self.acknowledged_bridge_generation = None;
    }
}

pub(super) struct GuiPersistedConfigRuntimeOwner {
    pub(super) config_path: Option<PathBuf>,
    pub(super) legacy_projection: Option<SorotteGuiShellAppState>,
    pub(super) session: Option<Box<dyn GuiSessionRuntimeAdapter + Send>>,
    pub(super) active_session_settings: Option<StoredClientSettingsRuntimeSnapshot>,
    pub(super) active_session_configured_settings: Option<StoredClientSettingsRuntimeSnapshot>,
    pub(super) session_generation: u64,
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
    pub(super) player_apply_state: GuiPlayerApplyState,
    pub(super) managed_mpv_process: Option<ManagedMpvProcessGuard>,
    pub(super) player_unavailability_reason: Option<String>,
    pub(super) core_player_configuration_health: GuiCorePlayerConfigurationHealth,
    pub(super) pending_apply_requirements_refresh_required: bool,
    pub(super) player_integration_health: GuiPlayerIntegrationHealth,
    pub(super) player_local_file: Option<LocalFileUpdate>,
    pub(super) player_local_file_placeholder: bool,
    pub(super) last_published_local_file: Option<LocalFileUpdate>,
    pub(super) last_published_media_match_signature: Option<serde_json::Value>,
    pub(super) local_shared_playlist_media_match_signature_path: Option<String>,
    pub(super) playlist_resolution: GuiPlaylistResolutionCoordinator,
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
    pub(super) pending_attached_room_unpause_observation:
        Option<GuiPendingAttachedRoomUnpauseObservation>,
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
    pub(super) plex_playlist_job_generation: u64,
    pub(super) plex_playlist_search_job: Option<GuiActivePlexPlaylistSearchJob>,
    pub(super) plex_playlist_resolve_job: Option<GuiActivePlexPlaylistResolveJob>,
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

impl GuiPersistedConfigRuntimeOwner {
    /// Settings that are allowed to influence ordinary runtime work right now.
    ///
    /// A connected session is pinned to its explicit active snapshot. Detached work uses the
    /// last saved configuration. Merely editing the Settings draft must not alter playback,
    /// privacy, media lookup, or integration behavior until Save/reconnect or an explicit
    /// immediate feature action applies a scoped patch.
    pub(in crate::app::runtime_owner) fn runtime_operation_settings(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> StoredClientSettingsMvp {
        self.active_session_settings
            .as_ref()
            .map(|runtime_settings| runtime_settings.settings.clone())
            .unwrap_or_else(|| state.saved_configuration.clone())
    }

    pub(in crate::app::runtime_owner) fn runtime_shared_playlist_enabled(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> bool {
        stored_client_settings_runtime_snapshot_legacy_compatible(
            &self.runtime_operation_settings(state),
        )
        .config
        .playback
        .shared_playlist_enabled
    }

    pub(in crate::app::runtime_owner) fn apply_patch_to_active_session_settings(
        &mut self,
        patch: &GuiPersistedSettingsPatch,
    ) {
        for active_settings in [
            self.active_session_settings.as_mut(),
            self.active_session_configured_settings.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let mut settings = active_settings.settings.clone();
            patch.apply_to(&mut settings);
            Self::replace_active_runtime_settings_preserving_controlled_room_password(
                active_settings,
                &settings,
            );
        }
    }

    /// Promotes fields whose taxonomy says they become effective on an ordinary successful Save
    /// and that are consumed directly by connected runtime work. Connection, playback/privacy,
    /// player, streaming, and application-restart fields deliberately remain pinned until their
    /// stronger lifecycle boundary is crossed.
    pub(in crate::app) fn promote_on_save_runtime_fields(
        &mut self,
        saved_settings: &StoredClientSettingsMvp,
    ) {
        for active_settings in [
            self.active_session_settings.as_mut(),
            self.active_session_configured_settings.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let mut settings = active_settings.settings.clone();
            settings.media_search_directories = saved_settings.media_search_directories.clone();
            settings.folder_search_first_file_timeout_seconds =
                saved_settings.folder_search_first_file_timeout_seconds;
            settings.folder_search_timeout_seconds = saved_settings.folder_search_timeout_seconds;
            settings.folder_search_double_check_interval_seconds =
                saved_settings.folder_search_double_check_interval_seconds;
            settings.folder_search_warning_threshold_seconds =
                saved_settings.folder_search_warning_threshold_seconds;
            Self::replace_active_runtime_settings_preserving_controlled_room_password(
                active_settings,
                &settings,
            );
        }
    }

    pub(in crate::app) fn promote_restart_player_runtime_fields(
        &mut self,
        saved_settings: &StoredClientSettingsMvp,
    ) {
        for active_settings in [
            self.active_session_settings.as_mut(),
            self.active_session_configured_settings.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let settings = FirstRunConfigurationDialogDraft::merge_apply_requirement_from_settings(
                &active_settings.settings,
                saved_settings,
                GuiSettingApplyRequirement::RestartPlayer,
            );
            Self::replace_active_runtime_settings_preserving_controlled_room_password(
                active_settings,
                &settings,
            );
        }
    }

    pub(in crate::app) fn adopt_saved_player_launch_state_when_inactive(
        &mut self,
        saved_settings: &StoredClientSettingsMvp,
    ) {
        if self.player_apply_state.applied_process_target.is_some() || self.player.is_some() {
            return;
        }
        if let Ok(launch_state) = Self::configured_player_launch_state_from_lookup_and_settings(
            &env_trimmed,
            Some(saved_settings),
        ) {
            self.player_launch_state = launch_state;
        }
    }

    fn replace_active_runtime_settings_preserving_controlled_room_password(
        active_settings: &mut StoredClientSettingsRuntimeSnapshot,
        settings: &StoredClientSettingsMvp,
    ) {
        let controlled_room_password = active_settings
            .controlled_room_password_override
            .clone()
            .or_else(|| {
                active_settings
                    .config
                    .connection
                    .controlled_room_password
                    .clone()
            });
        let mut replacement = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
        if let Some(password) = controlled_room_password {
            replacement.controlled_room_password_override = Some(password.clone());
            replacement.config.connection.controlled_room_password = Some(password);
        }
        *active_settings = replacement;
    }

    fn comparable_settings_for_runtime_snapshot(
        snapshot: &StoredClientSettingsRuntimeSnapshot,
    ) -> StoredClientSettingsMvp {
        let mut settings = snapshot.settings.clone();
        if let (Some(room), Some(password)) = (
            snapshot.config.connection.room.as_ref(),
            snapshot
                .controlled_room_password_override
                .as_ref()
                .or(snapshot.config.connection.controlled_room_password.as_ref()),
        ) {
            settings.room = Some(format!("{}:{}", room.as_str(), password.expose_secret()));
        }
        settings
    }

    fn settings_differ_for_apply_requirement(
        saved_settings: &StoredClientSettingsMvp,
        active_settings: &StoredClientSettingsRuntimeSnapshot,
        requirement: GuiSettingApplyRequirement,
    ) -> bool {
        let saved_snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(saved_settings);
        let saved_settings = Self::comparable_settings_for_runtime_snapshot(&saved_snapshot);
        let active_settings = Self::comparable_settings_for_runtime_snapshot(active_settings);
        let saved = FirstRunConfigurationDialogDraft::from_stored_settings(&saved_settings);
        let active = FirstRunConfigurationDialogDraft::from_stored_settings(&active_settings);
        SettingId::ALL
            .iter()
            .copied()
            .filter(|id| id.apply_requirement() == requirement)
            .any(|id| {
                if id == SettingId::ConnectionServerPassword {
                    return saved_settings.server_password != active_settings.server_password;
                }
                saved.control_value(id) != active.control_value(id)
            })
    }

    pub(in crate::app) fn pending_apply_requirements_for_settings(
        &self,
        projected_state: &SorotteGuiShellAppState,
        saved_settings: &StoredClientSettingsMvp,
    ) -> Vec<GuiSettingApplyRequirement> {
        let mut requirements = BTreeSet::new();
        if self.session_projects_to_shell
            && self
                .active_session_configured_settings
                .as_ref()
                .or(self.active_session_settings.as_ref())
                .is_some_and(|active| {
                    Self::settings_differ_for_apply_requirement(
                        saved_settings,
                        active,
                        GuiSettingApplyRequirement::Reconnect,
                    )
                })
        {
            requirements.insert(GuiSettingApplyRequirement::Reconnect);
        }

        let desired_player_launch_state =
            Self::configured_player_launch_state_from_lookup_and_settings(
                &env_trimmed,
                Some(saved_settings),
            );
        let core_player_state_is_active =
            self.player_apply_state.applied_process_target.is_some() || self.player.is_some();
        let process_target_differs = core_player_state_is_active
            && desired_player_launch_state
                .as_ref()
                .map_or(true, |desired| {
                    !self.player_apply_state.process_target_is_applied(desired)
                });
        let streaming_options_differ = core_player_state_is_active
            && !self.player_apply_state.streaming_apply_awaiting_transition
            && desired_player_launch_state
                .as_ref()
                .map_or(true, |desired| {
                    !self
                        .player_apply_state
                        .streaming_options_are_applied(desired)
                });
        let retryable_streaming_degradation_on_attached_target = self.player.is_some()
            && self.player_apply_state.core_reapply_required
            && desired_player_launch_state
                .as_ref()
                .is_ok_and(|desired| self.player_apply_state.process_target_is_applied(desired))
            && matches!(
                self.core_player_configuration_health,
                GuiCorePlayerConfigurationHealth::StreamingDegraded {
                    retryable_in_place: true,
                    ..
                }
            );
        if self.player_apply_state.core_reapply_required
            || process_target_differs
            || streaming_options_differ
        {
            requirements.insert(if retryable_streaming_degradation_on_attached_target {
                GuiSettingApplyRequirement::PlayerSettingsRetryAvailable
            } else {
                GuiSettingApplyRequirement::RestartPlayer
            });
        }

        let saved_language = saved_settings
            .language
            .as_deref()
            .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
            .unwrap_or("en");
        let active_language = projected_state
            .active_application_language
            .as_deref()
            .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
            .unwrap_or("en");
        if saved_language != active_language
            || saved_settings.force_gui_prompt.unwrap_or(false)
                != projected_state
                    .active_application_force_gui_prompt
                    .unwrap_or(false)
        {
            requirements.insert(GuiSettingApplyRequirement::RestartApplication);
        }

        requirements.into_iter().collect()
    }

    pub(in crate::app) fn pending_apply_requirements_action(
        &self,
        projected_state: &SorotteGuiShellAppState,
        saved_settings: &StoredClientSettingsMvp,
    ) -> GuiShellAction {
        GuiShellAction::ApplyPendingApplyRequirementsSnapshot(
            self.pending_apply_requirements_for_settings(projected_state, saved_settings),
        )
    }
}

pub(super) const ATTACHED_PLAYER_PAUSE_COMMAND_SUPPRESSION: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct GuiPlaylistResolutionCoordinator {
    pub(super) room_name: Option<String>,
    pub(super) session_generation: u64,
    pub(super) session_active: bool,
    pub(super) playlist_revision: Option<u64>,
    pub(super) remote_playlist_revision: u64,
    pub(super) generation: u64,
    pub(super) row_ids: Vec<GuiPlaylistEntryId>,
    pub(super) local_origins_by_row: HashMap<GuiPlaylistEntryId, PathBuf>,
    pub(super) row_scope_reset_pending: bool,
}

impl std::fmt::Debug for GuiPlaylistResolutionCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlaylistResolutionCoordinator")
            .field("room_name", &self.room_name)
            .field("session_generation", &self.session_generation)
            .field("session_active", &self.session_active)
            .field("playlist_revision", &self.playlist_revision)
            .field("remote_playlist_revision", &self.remote_playlist_revision)
            .field("generation", &self.generation)
            .field("row_count", &self.row_ids.len())
            .field("local_origin_count", &self.local_origins_by_row.len())
            .field("row_scope_reset_pending", &self.row_scope_reset_pending)
            .finish()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct GuiPlaylistLocalOriginBindingOutcome {
    pub(super) bound_row_ids: Vec<GuiPlaylistEntryId>,
    pub(super) unavailable_row_ids: Vec<GuiPlaylistEntryId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GuiPendingAttachedPlayerPauseCommand {
    pub(super) target_paused: bool,
    pub(super) suppress_until: Instant,
}

/// Temporary P0 observation state for a desired room unpause.
///
/// The source-independent playback coordinator will eventually own this state. Until then, the
/// GUI keeps only enough information to avoid treating IPC acceptance or cache release as proof
/// that playback resumed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum GuiPendingAttachedRoomUnpauseObservation {
    CachePaused,
    AwaitingAdvancement {
        baseline_position_seconds: Option<f64>,
    },
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

pub(super) struct GuiActivePlexPlaylistSearchJob {
    pub(super) id: u64,
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) query: String,
    pub(super) result_rx: mpsc::Receiver<GuiPlexPlaylistSearchWorkerResult>,
}

pub(super) struct GuiPlexPlaylistSearchWorkerResult {
    pub(super) id: u64,
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) query: String,
    pub(super) result: Result<Vec<GuiPlexPlaylistSearchResult>, String>,
}

pub(super) struct GuiActivePlexPlaylistResolveJob {
    pub(super) id: u64,
    pub(super) operation_context: GuiPlexOperationContext,
    pub(super) rating_key: String,
    pub(super) result_rx: mpsc::Receiver<GuiPlexPlaylistResolveWorkerResult>,
}

pub(super) struct GuiPlexPlaylistResolveWorkerResult {
    pub(super) id: u64,
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
    pub(super) entry_id: GuiPlaylistEntryId,
    pub(super) generation: u64,
    pub(super) target: String,
    pub(super) provider_id: GuiMediaSourceProviderId,
}

impl std::fmt::Debug for GuiPendingPlaylistSourceResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPendingPlaylistSourceResolution")
            .field("index", &self.index)
            .field("entry_id", &self.entry_id)
            .field("generation", &self.generation)
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
    pub(super) playlist_entry_id: Option<GuiPlaylistEntryId>,
    pub(super) playlist_generation: u64,
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
            .field("playlist_entry_id", &self.playlist_entry_id)
            .field("playlist_generation", &self.playlist_generation)
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
            playlist_entry_id: Some(GuiPlaylistEntryId::next()),
            playlist_generation: 11,
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
            entry_id: GuiPlaylistEntryId::next(),
            generation: 3,
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
