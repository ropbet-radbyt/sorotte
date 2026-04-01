use super::runtime_bridge::GuiSharedPlaylistOpenDispatch;

#[cfg(test)]
#[path = "app_runtime_owner/tests.rs"]
mod tests;

#[path = "app_runtime_player.rs"]
mod player;
#[path = "app_runtime_projection.rs"]
mod projection;
#[path = "app_runtime_requests.rs"]
mod requests;

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use syncplay_client_app::app_boundary::{
    persistence::{
        clear_syncplay_ini_stored_client_settings_mvp_at_path,
        load_syncplay_ini_stored_client_settings_mvp_from_path,
    },
    state::{StoredClientSettingsMvp, stored_client_settings_runtime_snapshot_legacy_compatible},
};
use syncplay_player_api::{LocalFileUpdate, PlayerAdapter};
use syncplay_player_mpv::MpvAdapter;

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
    GuiCommandAvailabilityState, GuiShellAction, GuiTransientNotificationLevel,
    SyncplayGuiShellAppState,
};
use super::startup::{
    explicit_mpv_ipc_path_from_lookup, resolve_syncplay_gui_config_path_legacy_compatible,
};
use super::startup_support::{env_flag_enabled_lookup, env_trimmed};
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
    pub(super) session_default_room: Option<String>,
    pub(super) pending_room_change_request: Option<GuiPendingRoomChangeRequest>,
    pub(super) startup_saved_connect_attempted: bool,
    pub(super) player: Option<GuiOwnedPlayer>,
    pub(super) player_launch_state: GuiPlayerLaunchRuntimeState,
    pub(super) managed_mpv_process: Option<ManagedMpvProcessGuard>,
    pub(super) player_unavailability_reason: Option<String>,
    pub(super) player_local_file: Option<LocalFileUpdate>,
    pub(super) last_published_local_file: Option<LocalFileUpdate>,
    pub(super) attached_media_search_index: Option<GuiAttachedMediaSearchIndex>,
    pub(super) attached_media_search_next_retry_at: Option<Instant>,
    pub(super) pending_attached_media_resolution: Option<GuiPendingAttachedMediaResolution>,
    pub(super) attached_media_search_progress: Option<GuiAttachedMediaSearchBuildProgress>,
    pub(super) attached_media_search_progress_updated_at: Option<Instant>,
    pub(super) attached_media_search_build_state: GuiAttachedMediaSearchBuildState,
    pub(super) attached_media_search_build_roots: Vec<String>,
    pub(super) attached_media_search_job_sequence: u64,
    pub(super) attached_media_search_index_revision: u64,
    pub(super) unresolved_attached_media_target: Option<String>,
    pub(super) last_attached_media_resolution_trigger: Option<GuiAutomaticMediaResolutionTrigger>,
    pub(super) last_applied_attached_room_playstate: Option<GuiSessionRoomPlaystate>,
    pub(super) suppressed_attached_room_playstate_after_playlist_reset:
        Option<GuiSessionRoomPlaystate>,
    pub(super) player_position_seconds: Option<f64>,
    pub(super) player_paused: Option<bool>,
    pub(super) user_offset_seconds: f64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct GuiMediaIndexJobId(pub(super) u64);

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
    pub(super) job_id: GuiMediaIndexJobId,
    pub(super) roots: Vec<String>,
    pub(super) cancel_flag: Arc<AtomicBool>,
    pub(super) latest_progress: Arc<Mutex<Option<GuiAttachedMediaSearchBuildProgress>>>,
    pub(super) result_rx: std::sync::mpsc::Receiver<GuiAttachedMediaSearchBuildStatus>,
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

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn with_config_path(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path,
            session: None,
            session_projects_to_shell: false,
            session_transport: None,
            session_transport_driver: None,
            session_transport_reconnect_due_at: None,
            session_transport_reconnect_failures: 0,
            session_transport_disconnect_pending_cleanup: false,
            session_default_room: None,
            pending_room_change_request: None,
            startup_saved_connect_attempted: false,
            player: None,
            player_launch_state: GuiPlayerLaunchRuntimeState::None,
            managed_mpv_process: None,
            player_unavailability_reason: None,
            player_local_file: None,
            last_published_local_file: None,
            attached_media_search_index: None,
            attached_media_search_next_retry_at: None,
            pending_attached_media_resolution: None,
            attached_media_search_progress: None,
            attached_media_search_progress_updated_at: None,
            attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
            attached_media_search_build_roots: Vec::new(),
            attached_media_search_job_sequence: 0,
            attached_media_search_index_revision: 0,
            unresolved_attached_media_target: None,
            last_attached_media_resolution_trigger: None,
            last_applied_attached_room_playstate: None,
            suppressed_attached_room_playstate_after_playlist_reset: None,
            player_position_seconds: None,
            player_paused: None,
            user_offset_seconds: 0.0,
        }
    }

    pub(super) fn with_config_path_and_startup_player(config_path: Option<PathBuf>) -> Self {
        Self::with_config_path_and_startup_player_lookup(config_path, &env_trimmed)
    }

    pub(super) fn with_config_path_and_startup_player_lookup<F>(
        config_path: Option<PathBuf>,
        lookup: &F,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut owner = Self::with_config_path(config_path);
        let startup_settings = owner.load_startup_player_settings_from_config_path();
        owner.configure_startup_player_from_lookup_and_settings(lookup, startup_settings.as_ref());
        owner
    }

    fn load_startup_player_settings_from_config_path(&self) -> Option<StoredClientSettingsMvp> {
        self.config_path.as_ref().and_then(|path| {
            load_syncplay_ini_stored_client_settings_mvp_from_path(path)
                .ok()
                .flatten()
        })
    }

    fn startup_player_unavailability_reason(
        launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> Option<String> {
        if let Some(reason) = launch_state.default_unavailability_reason() {
            return Some(reason);
        }
        let GuiPlayerLaunchRuntimeState::ManagedMpv(config) = launch_state else {
            return None;
        };
        let program = &config.program;
        let requires_existing_file = program.is_absolute()
            || program
                .to_string_lossy()
                .chars()
                .any(|character| matches!(character, '/' | '\\'));
        if requires_existing_file && !program.is_file() {
            return Some(format!(
                "GUI-owned mpv launch failed from saved player path '{}': managed mpv binary does not exist: {}",
                config.requested_player_path,
                program.display()
            ));
        }
        None
    }

    fn configure_startup_player_from_lookup_and_settings<F>(
        &mut self,
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
    ) where
        F: Fn(&str) -> Option<String>,
    {
        let launch_state =
            match Self::configured_player_launch_state_from_lookup_and_settings(lookup, settings) {
                Ok(state) => state,
                Err(error) => {
                    self.detach_player();
                    self.player_launch_state = GuiPlayerLaunchRuntimeState::None;
                    self.player_unavailability_reason = Some(error);
                    return;
                }
            };
        if matches!(launch_state, GuiPlayerLaunchRuntimeState::TestPlayer) {
            self.attach_player_from_launch_state(launch_state);
            return;
        }
        self.detach_player();
        self.player_launch_state = launch_state.clone();
        self.player_unavailability_reason =
            Self::startup_player_unavailability_reason(&launch_state);
    }

    fn clear_player_runtime_cache(&mut self) {
        self.player_local_file = None;
        self.player_position_seconds = None;
        self.player_paused = None;
        if let Some(pending_resolution) = self.pending_attached_media_resolution.take() {
            pending_resolution
                .cancel_flag
                .store(true, Ordering::Relaxed);
        }
        self.attached_media_search_next_retry_at = None;
        self.pending_attached_media_resolution = None;
        self.attached_media_search_progress = None;
        self.attached_media_search_progress_updated_at = None;
        self.attached_media_search_build_state = GuiAttachedMediaSearchBuildState::Idle;
        self.attached_media_search_build_roots.clear();
        self.unresolved_attached_media_target = None;
        self.last_attached_media_resolution_trigger = None;
        self.last_applied_attached_room_playstate = None;
        self.suppressed_attached_room_playstate_after_playlist_reset = None;
    }

    fn detach_player(&mut self) {
        self.player = None;
        self.managed_mpv_process = None;
        self.clear_player_runtime_cache();
    }

    fn attach_player_from_launch_state(&mut self, launch_state: GuiPlayerLaunchRuntimeState) {
        self.detach_player();
        self.player_launch_state = launch_state.clone();
        self.player_unavailability_reason = launch_state.default_unavailability_reason();
        match launch_state {
            GuiPlayerLaunchRuntimeState::None => {}
            GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {}
            GuiPlayerLaunchRuntimeState::TestPlayer => {
                self.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
                self.player_unavailability_reason = None;
            }
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings,
            } => match MpvAdapter::with_json_ipc(&ipc_path) {
                Ok(mut adapter) => match apply_legacy_syncplay_ui_settings_to_mpv_adapter(
                    &mut adapter,
                    &ui_settings,
                ) {
                    Ok(()) => {
                        self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
                        self.player_unavailability_reason = None;
                    }
                    Err(error) => {
                        self.player_unavailability_reason = Some(format!(
                            "mpv JSON IPC attach succeeded at '{ipc_path}', but legacy GUI settings could not be applied: {error}"
                        ));
                    }
                },
                Err(error) => {
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC attach failed at '{ipc_path}': {error}"
                    ));
                }
            },
            GuiPlayerLaunchRuntimeState::ManagedMpv(config) => {
                match mpv_launch::spawn_managed_mpv_and_attach(&config) {
                    Ok((mut adapter, guard)) => {
                        match apply_legacy_syncplay_ui_settings_to_mpv_adapter(
                            &mut adapter,
                            &config.ui_settings,
                        ) {
                            Ok(()) => {
                                self.managed_mpv_process = Some(guard);
                                self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
                                self.player_unavailability_reason = None;
                            }
                            Err(error) => {
                                self.player_unavailability_reason = Some(format!(
                                    "GUI-owned mpv started, but legacy GUI settings could not be applied: {error}"
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        self.player_unavailability_reason = Some(format!(
                            "GUI-owned mpv launch failed from saved player path '{}': {error}",
                            config.requested_player_path
                        ));
                    }
                }
            }
        }
    }

    fn configured_player_launch_state_from_lookup_and_settings<F>(
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
    ) -> Result<GuiPlayerLaunchRuntimeState, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        match env_flag_enabled_lookup(lookup, "SYNCPLAY_GUI_ENABLE_TEST_PLAYER") {
            Ok(true) => {
                return Ok(GuiPlayerLaunchRuntimeState::TestPlayer);
            }
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "SYNCPLAY_GUI_ENABLE_TEST_PLAYER could not be parsed: {error}"
                ));
            }
        }

        if let Some(ipc_path) = explicit_mpv_ipc_path_from_lookup(lookup) {
            let ui_settings =
                mpv_launch::legacy_syncplay_ui_settings_from_stored_settings(settings);
            return Ok(GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings: Box::new(ui_settings),
            });
        }

        Ok(
            match managed_mpv_settings_decision_from_settings(settings) {
                ManagedMpvSettingsDecision::NotConfigured => GuiPlayerLaunchRuntimeState::None,
                ManagedMpvSettingsDecision::UnsupportedConfiguredPlayer { player_path } => {
                    GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { player_path }
                }
                ManagedMpvSettingsDecision::Launch(config) => {
                    GuiPlayerLaunchRuntimeState::ManagedMpv(config)
                }
            },
        )
    }

    fn try_apply_mpv_ui_settings_in_place(
        &mut self,
        next_launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> bool {
        if !self
            .player_launch_state
            .can_apply_mpv_ui_settings_in_place(next_launch_state)
        {
            return false;
        }
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return false;
        };
        let Some(ui_settings) = next_launch_state.mpv_ui_settings() else {
            return false;
        };
        if let Err(error) = apply_legacy_syncplay_ui_settings_to_mpv_adapter(player, ui_settings) {
            self.player_unavailability_reason =
                Some(format!("mpv legacy GUI settings reapply failed: {error}"));
            return false;
        }
        self.player_launch_state = next_launch_state.clone();
        self.player_unavailability_reason = None;
        true
    }

    pub(super) fn sync_player_from_lookup_and_settings<F>(
        &mut self,
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
        force_relaunch: bool,
    ) where
        F: Fn(&str) -> Option<String>,
    {
        let next_launch_state =
            match Self::configured_player_launch_state_from_lookup_and_settings(lookup, settings) {
                Ok(state) => state,
                Err(error) => {
                    self.detach_player();
                    self.player_launch_state = GuiPlayerLaunchRuntimeState::None;
                    self.player_unavailability_reason = Some(error);
                    return;
                }
            };

        if !force_relaunch
            && self.player_launch_state == next_launch_state
            && (self.player.is_some() || self.player_unavailability_reason.is_some())
        {
            return;
        }
        if !force_relaunch && self.try_apply_mpv_ui_settings_in_place(&next_launch_state) {
            return;
        }
        self.attach_player_from_launch_state(next_launch_state);
    }

    fn ensure_configured_player_attached(&mut self) {
        if self.player.is_some() {
            return;
        }
        if self.player_launch_state.can_attach_on_demand() {
            self.attach_player_from_launch_state(self.player_launch_state.clone());
        }
    }

    pub(super) fn player_runtime_available_for_actions(&self) -> bool {
        self.player.is_some() || self.player_launch_state.can_attach_on_demand()
    }

    fn ensure_configured_player_attached_for_active_session(&mut self) {
        if self.session_active() {
            self.ensure_configured_player_attached();
        }
    }

    fn poll_managed_mpv_process(&mut self) {
        let exit_status = match self.managed_mpv_process.as_mut() {
            Some(guard) => match guard.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    self.detach_player();
                    self.player_unavailability_reason = Some(error);
                    return;
                }
            },
            None => return,
        };
        let Some(exit_status) = exit_status else {
            return;
        };

        let status_text = exit_status
            .code()
            .map(|code| format!("with exit code {code}"))
            .unwrap_or_else(|| "after an abnormal termination".to_owned());
        self.detach_player();
        self.player_unavailability_reason = Some(format!(
            "GUI-owned mpv exited {status_text}. Open media or save/reload configuration to relaunch it."
        ));
    }

    fn legacy_gui_qsettings_root(&self) -> Option<PathBuf> {
        self.config_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    fn clear_gui_data(&mut self) -> Result<(), String> {
        if let Some(path) = self.config_path.as_ref() {
            clear_syncplay_ini_stored_client_settings_mvp_at_path(path).map_err(|error| {
                format!(
                    "failed clearing stored settings {}: {error}",
                    path.display()
                )
            })?;
        }
        if let Some(root) = self.legacy_gui_qsettings_root() {
            clear_legacy_gui_qsettings_files_at_root(&root)?;
            clear_persisted_media_search_cache_at_root(&root)?;
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.reset_session_transport_reconnect_state();
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn with_session_runtime(
        mut self,
        session: Box<dyn GuiSessionRuntimeAdapter + Send>,
    ) -> Self {
        self.session = Some(session);
        self.session_projects_to_shell = true;
        self
    }

    fn with_session_default_room(mut self, room: impl Into<String>) -> Self {
        self.session_default_room = Some(room.into());
        self
    }

    #[allow(dead_code)]
    fn with_session_transport(
        mut self,
        session_transport: GuiQueuedSessionTransportHandle,
    ) -> Self {
        self.session_transport = Some(session_transport);
        self
    }

    #[allow(dead_code)]
    fn with_session_transport_driver(
        mut self,
        session_transport_driver: Box<dyn GuiSessionTransportDriver + Send>,
    ) -> Self {
        self.session_transport_driver = Some(session_transport_driver);
        self
    }

    pub(super) fn reset_session_transport_reconnect_state(&mut self) {
        self.session_transport_reconnect_due_at = None;
        self.session_transport_reconnect_failures = 0;
        self.session_transport_disconnect_pending_cleanup = false;
    }

    fn schedule_session_transport_reconnect(&mut self, delay_seconds: f64) {
        self.session_transport_reconnect_due_at =
            Some(Instant::now() + Duration::from_secs_f64(delay_seconds.max(0.0)));
        self.session_transport_reconnect_failures =
            self.session_transport_reconnect_failures.saturating_add(1);
        self.session_transport_disconnect_pending_cleanup = false;
    }

    fn sync_session_transport_reconnect_state_from_handshake(&mut self) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.server_handshake_completed())
        {
            self.session_transport_reconnect_due_at = None;
            self.session_transport_reconnect_failures = 0;
            self.session_transport_disconnect_pending_cleanup = false;
        }
    }

    pub(super) fn apply_session_transport_disconnect_pause(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let should_pause = self
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state())
            == Some(true)
            && self.player_paused != Some(true);
        if !should_pause {
            return;
        }

        if let Some(player) = self.player.as_mut()
            && let Err(error) = player.set_paused(true)
        {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: format!(
                        "Pause-on-leave dispatch through the attached {} player failed: {error}",
                        player.name()
                    ),
                }],
            );
            return;
        }
        self.refresh_player_state();
    }

    fn handle_session_transport_failure(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        error: String,
    ) {
        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message: format!("Session transport driver pump failed: {error}"),
        }];
        let now_seconds = system_time_seconds();
        let retries = self.session_transport_reconnect_failures;
        let mut reconnect_delay = None;
        let stop_reconnect_requested;

        if let Some(session) = self.session.as_mut() {
            if let Err(session_error) = session.handle_transport_disconnect(now_seconds, retries) {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: session_error,
                });
                stop_reconnect_requested = true;
            } else {
                reconnect_delay = session.drain_reconnect_delays().into_iter().next();
                stop_reconnect_requested = session.take_stop_reconnect_requested();
            }
        } else {
            stop_reconnect_requested = true;
        }

        Self::push_actions_and_project(handle, projected_state, actions);
        self.apply_session_transport_disconnect_pause(handle, projected_state);

        if let Some(delay_seconds) = reconnect_delay
            && !stop_reconnect_requested
        {
            self.schedule_session_transport_reconnect(delay_seconds);
            return;
        }

        self.session_transport_reconnect_due_at = None;
        self.session_transport_disconnect_pending_cleanup = true;
    }

    fn pump_due_session_transport_reconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(due_at) = self.session_transport_reconnect_due_at else {
            return;
        };
        if Instant::now() < due_at {
            return;
        }
        self.session_transport_reconnect_due_at = None;

        let Some(session_transport) = self.session_transport.as_ref() else {
            self.session_transport_disconnect_pending_cleanup = true;
            return;
        };
        session_transport.clear_protocol_lines();

        let Some(session) = self.session.as_mut() else {
            self.session_transport_disconnect_pending_cleanup = true;
            return;
        };
        if let Err(error) = session.prepare_for_transport_reconnect() {
            self.handle_session_transport_failure(
                handle,
                projected_state,
                format!("Session transport reconnect preparation failed: {error}"),
            );
            return;
        }

        let Some(session_transport_driver) = self.session_transport_driver.as_mut() else {
            self.handle_session_transport_failure(
                handle,
                projected_state,
                "Session transport reconnect failed: no transport driver is attached.".to_owned(),
            );
            return;
        };
        if let Err(error) = session_transport_driver.reconnect() {
            self.handle_session_transport_failure(
                handle,
                projected_state,
                format!("Session transport reconnect failed: {error}"),
            );
        }
    }

    fn clear_session_runtime_after_transport_disconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.session_default_room = None;
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.pending_attached_media_resolution = None;
        self.unresolved_attached_media_target = None;
        self.last_applied_attached_room_playstate = None;
        self.reset_session_transport_reconnect_state();

        let actions = self.sessionless_projection_actions(projected_state);
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn finish_pending_session_transport_disconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if !self.session_transport_disconnect_pending_cleanup {
            return;
        }
        self.clear_session_runtime_after_transport_disconnect(handle, projected_state);
    }

    fn drain_session_runtime_actions_and_finish_transport_disconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        self.drain_session_runtime_actions(handle, projected_state);
        self.sync_session_transport_reconnect_state_from_handshake();
        self.finish_pending_session_transport_disconnect(handle, projected_state);
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        let room = room.into();
        let runtime_settings =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                username: Some(username.into()),
                room: Some(room.clone()),
                ..StoredClientSettingsMvp::default()
            });
        let mut session = GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
            runtime_settings
                .settings
                .username
                .clone()
                .unwrap_or_default(),
            runtime_settings.settings.room.clone().unwrap_or_default(),
            runtime_settings.controlled_room_password_override.clone(),
        )?;
        session.apply_runtime_settings_snapshot(&runtime_settings);
        let session = Box::new(session);
        let session_transport = GuiQueuedSessionTransportHandle::default();
        Ok((
            self.with_session_runtime(session)
                .with_session_default_room(room)
                .with_session_transport(session_transport.clone()),
            session_transport,
        ))
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_loopback_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let (owner, _session_transport) =
            self.with_client_core_chat_session_runtime(username.clone(), room)?;
        Ok(
            owner.with_session_transport_driver(Box::new(GuiLoopbackSessionTransportDriver::new(
                username,
            ))),
        )
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_tcp_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
    ) -> Result<Self, String> {
        let (owner, _session_transport) =
            self.with_client_core_chat_session_runtime(username, room)?;
        Ok(owner.with_session_transport_driver(Box::new(
            GuiTcpSessionTransportDriver::connect_from_host_arg(host_arg.as_ref())?,
        )))
    }

    fn pump_session_transport_driver(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let Some(session_transport_driver) = self.session_transport_driver.as_mut() else {
            return;
        };
        if let Err(error) = session_transport_driver.pump(session_transport) {
            self.handle_session_transport_failure(handle, projected_state, error);
        }
    }

    fn drain_session_transport_inbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let inbound_protocol_lines = session_transport.drain_inbound_protocol_lines();
        if inbound_protocol_lines.is_empty() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        for inbound_protocol_line in inbound_protocol_lines {
            if let Err(error) = session.apply_message_json(&inbound_protocol_line) {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Inbound session transport message apply failed: {error}"),
                    }],
                );
            }
        }
    }

    fn drain_session_runtime_actions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if !self.session_projects_to_shell {
            if let Some(session) = self.session.as_mut() {
                let _ = session.drain_gui_actions(projected_state);
            }
            return;
        }
        let actions = {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            session.drain_gui_actions(projected_state)
        };
        let actions = self.augment_runtime_actions_for_room_transitions(projected_state, actions);
        self.emit_gui_actions_to_attached_player(&actions);
        Self::push_actions_and_project(handle, projected_state, actions);
        let selected_media_sync =
            self.sync_selected_shared_playlist_media_to_attached_player_impl(projected_state);
        self.apply_pending_playlist_index_reset_to_attached_player_impl(
            selected_media_sync.selection_ready(),
        );
        self.sync_session_playstate_to_attached_player_impl(
            projected_state,
            selected_media_sync.selection_ready(),
        );
    }

    fn flush_session_transport_outbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match session.flush_outbound_protocol_lines() {
            Ok(outbound_protocol_lines) => {
                if !outbound_protocol_lines.is_empty() {
                    session_transport.push_outbound_protocol_lines(outbound_protocol_lines);
                }
            }
            Err(error) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Outbound session transport flush failed: {error}"),
                    }],
                );
            }
        }
    }

    fn push_runtime_unavailable(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    fn default_room_for_legacy_fallback(
        &self,
        projected_state: &SyncplayGuiShellAppState,
    ) -> String {
        self.session_default_room
            .clone()
            .or_else(|| {
                projected_state
                    .saved_session_connect_target()
                    .map(|target| target.room)
            })
            .unwrap_or_else(|| {
                Self::detached_runtime_settings_for_state(projected_state)
                    .settings
                    .room
                    .unwrap_or_default()
            })
    }

    fn augment_runtime_actions_for_room_transitions(
        &mut self,
        projected_state: &SyncplayGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) -> Vec<GuiShellAction> {
        let mut current_room = projected_state.main_window.room_name.clone();
        let mut augmented_actions = Vec::with_capacity(actions.len());
        for action in actions {
            match action {
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                    let next_room = snapshot.room_name.clone();
                    let room_transition_actions =
                        self.room_transition_confirmation_actions(&current_room, &next_room);
                    current_room = next_room;
                    augmented_actions
                        .push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
                    augmented_actions.extend(room_transition_actions);
                }
                other => augmented_actions.push(other),
            }
        }
        augmented_actions
    }

    fn room_transition_confirmation_actions(
        &mut self,
        previous_room: &str,
        next_room: &str,
    ) -> Vec<GuiShellAction> {
        if previous_room == next_room {
            return Vec::new();
        }

        let Some(request) = self.pending_room_change_request.take() else {
            return Vec::new();
        };

        let (level, message) = match request {
            GuiPendingRoomChangeRequest::Join { .. } => (
                GuiTransientNotificationLevel::Success,
                format!("Room joined: {next_room}."),
            ),
            GuiPendingRoomChangeRequest::ReturnToDefault { .. } => (
                GuiTransientNotificationLevel::Info,
                format!("Returned to default room: {next_room}."),
            ),
        };

        vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: next_room.to_owned(),
            },
            GuiShellAction::PushTransientNotification {
                level,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn request_room_join_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        room: String,
    ) {
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Changing rooms requires an active session runtime.".to_owned(),
            );
            return;
        };

        match session.set_room(room.clone()) {
            Ok(()) => {
                self.pending_room_change_request = Some(GuiPendingRoomChangeRequest::Join {
                    requested_room: room,
                });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }

    fn request_room_leave_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let previous_room = projected_state.main_window.room_name.clone();
        let default_room = self.default_room_for_legacy_fallback(projected_state);
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Returning to the default room requires an active session runtime.".to_owned(),
            );
            return;
        };

        let room_change_result = if default_room.trim().is_empty() {
            session.set_room_with_legacy_fallback(default_room)
        } else {
            session.set_room(default_room)
        };
        match room_change_result {
            Ok(()) => {
                self.pending_room_change_request =
                    Some(GuiPendingRoomChangeRequest::ReturnToDefault { previous_room });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }

    fn open_media_unavailable_message(&self, selected_paths: &[String]) -> String {
        self.open_media_unavailable_message_impl(selected_paths)
    }

    fn shared_playlist_open_unavailable_message(&self, selected_paths: &[String]) -> String {
        self.shared_playlist_open_unavailable_message_impl(selected_paths)
    }

    fn shared_playlist_session_unavailable_message(&self) -> String {
        self.shared_playlist_session_unavailable_message_impl()
    }

    pub(super) fn shared_playlist_open_dispatch_for_paths(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        Self::shared_playlist_open_dispatch_for_paths_impl(paths)
    }

    fn seek_unavailable_message(&self, offset_seconds: f64) -> String {
        self.seek_unavailable_message_impl(offset_seconds)
    }

    fn toggle_pause_unavailable_message(&self) -> String {
        self.toggle_pause_unavailable_message_impl()
    }

    fn send_chat_unavailable_message(&self) -> String {
        self.send_chat_unavailable_message_impl()
    }

    fn push_player_success(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        Self::push_player_success_impl(handle, message)
    }

    fn push_player_error(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        Self::push_player_error_impl(handle, message)
    }

    fn open_media_files_through_attached_player_result(
        &mut self,
        paths: &[String],
    ) -> Option<Result<String, String>> {
        self.open_media_files_through_attached_player_result_impl(paths)
    }

    fn open_media_files_through_attached_player(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        self.open_media_files_through_attached_player_impl(handle, paths)
    }

    fn open_main_window_user_media_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        self.open_main_window_user_media_runtime_impl(handle, projected_state, target)
    }

    fn open_main_window_user_containing_folder_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        self.open_main_window_user_containing_folder_runtime_impl(handle, projected_state, target)
    }

    fn open_media_files_through_shared_playlist_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        paths: Vec<String>,
    ) {
        self.open_media_files_through_shared_playlist_runtime_impl(handle, projected_state, paths)
    }

    fn emit_gui_actions_to_attached_player(&mut self, actions: &[GuiShellAction]) {
        self.emit_gui_actions_to_attached_player_impl(actions)
    }

    fn drain_player_chat_input(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        self.drain_player_chat_input_impl(handle, projected_state)
    }

    fn refresh_player_state(&mut self) {
        self.refresh_player_state_impl()
    }

    fn player_target_position_seconds_for_global_position(
        &self,
        global_position_seconds: f64,
    ) -> f64 {
        self.player_target_position_seconds_for_global_position_impl(global_position_seconds)
    }

    fn sync_manual_seek_into_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.sync_manual_seek_into_detached_session_impl(
            state,
            previous_position_seconds,
            target_position_seconds,
        )
    }

    fn sync_playback_pause_into_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(), String> {
        self.sync_playback_pause_into_detached_session_impl(state, previous_paused, target_paused)
    }

    fn undo_seek_target_position_from_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.undo_seek_target_position_from_detached_session_impl(state)
    }

    pub(super) fn player_local_file_playlist_entries(&self) -> Vec<String> {
        self.player_local_file_playlist_entries_impl()
    }

    pub(super) fn command_availability_for_runtime_state(
        &self,
        state: &SyncplayGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        self.command_availability_for_runtime_state_impl(state, player_attached)
    }
}

impl Default for GuiPersistedConfigRuntimeOwner {
    fn default() -> Self {
        let mut owner = Self::with_config_path_and_startup_player(
            resolve_syncplay_gui_config_path_legacy_compatible(),
        );
        if owner.player.is_none()
            && owner.player_unavailability_reason.is_none()
            && !owner.player_launch_state.can_attach_on_demand()
        {
            owner.player_unavailability_reason = Some(
                "Set playerPath to mpv in GUI settings, or set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
                    .to_owned(),
            );
        }
        owner
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn pump_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
    ) {
        self.poll_managed_mpv_process();
        let mut projected_state = state.clone();
        self.pump_due_session_transport_reconnect(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions_and_finish_transport_disconnect(
            handle,
            &mut projected_state,
        );
        self.drain_player_chat_input(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
        self.flush_session_transport_outbound(handle, &mut projected_state);
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions_and_finish_transport_disconnect(
            handle,
            &mut projected_state,
        );
        self.drain_player_chat_input(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
        if !self.startup_saved_connect_attempted {
            self.startup_saved_connect_attempted = true;
            if projected_state.pending_operation.is_none()
                && !self.session_active()
                && projected_state.saved_session_connect_target().is_some()
            {
                self.complete_saved_server_connect_runtime(handle, &mut projected_state, false);
                self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
                self.flush_session_transport_outbound(handle, &mut projected_state);
                self.pump_session_transport_driver(handle, &mut projected_state);
                self.drain_session_transport_inbound(handle, &mut projected_state);
                self.drain_session_runtime_actions_and_finish_transport_disconnect(
                    handle,
                    &mut projected_state,
                );
                self.drain_player_chat_input(handle, &mut projected_state);
                self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
            }
        }
        for request in handle.drain_requests() {
            if !self.handle_runtime_request(handle, &mut projected_state, request) {
                continue;
            }
            self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
            self.flush_session_transport_outbound(handle, &mut projected_state);
            self.pump_session_transport_driver(handle, &mut projected_state);
            self.drain_session_transport_inbound(handle, &mut projected_state);
            self.drain_session_runtime_actions_and_finish_transport_disconnect(
                handle,
                &mut projected_state,
            );
            self.drain_player_chat_input(handle, &mut projected_state);
            self.sync_detached_session_runtime_state_or_notify(handle, &projected_state);
        }
        self.ensure_configured_player_attached_for_active_session();
        self.sync_player_runtime_state(handle, &projected_state);
    }

    fn sync_detached_session_runtime_state_or_notify(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
    ) {
        self.refresh_player_state();
        if let Err(error) = self.sync_detached_session_preferences_and_player_state(state) {
            Self::push_runtime_unavailable(handle, error);
        }
    }
}
