use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn with_config_path(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path,
            session: None,
            session_projects_to_shell: false,
            session_transport: None,
            session_transport_driver: None,
            session_transport_reconnect_due_at: None,
            session_transport_reconnect_failures: 0,
            session_transport_disconnect_pending_cleanup: false,
            runtime_pump_generation: 0,
            session_default_room: None,
            pending_room_change_request: None,
            startup_saved_connect_attempted: false,
            startup_remote_actions_attempted: false,
            startup_remote_actions_rx: None,
            startup_stream_helper_probe_completed: false,
            startup_stream_helper_probe_rx: None,
            player: None,
            player_launch_state: GuiPlayerLaunchRuntimeState::None,
            managed_mpv_process: None,
            player_unavailability_reason: None,
            player_local_file: None,
            player_local_file_placeholder: false,
            last_published_local_file: None,
            attached_media_search_index: None,
            attached_media_search_next_retry_at: None,
            pending_attached_media_resolution: None,
            attached_media_search_progress: None,
            attached_media_search_progress_updated_at: None,
            attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
            attached_media_search_build_roots: Vec::new(),
            attached_media_search_index_revision: 0,
            unresolved_attached_media_target: None,
            last_attached_media_resolution_trigger: None,
            last_applied_attached_room_playstate: None,
            suppressed_attached_room_playstate_after_playlist_reset: None,
            pending_local_attached_pause_override: None,
            pending_attached_cache_unpause: false,
            pending_attached_player_pause_confirmation_pump: None,
            player_position_seconds: None,
            player_paused: None,
            player_paused_for_cache: None,
            player_cache_buffering_percent: None,
            active_shared_playlist_index: None,
            playlist_auto_advance_eof_latched: false,
            user_offset_seconds: 0.0,
            stream_helper_runtime_snapshot: GuiStreamHelperRuntimeSnapshot::default(),
            stream_helper_remediation_runtime_snapshot:
                GuiStreamHelperRemediationRuntimeSnapshot::default(),
            plex_client: None,
            plex_auth_session: None,
            plex_auth_start_rx: None,
            plex_auth_poll_rx: None,
            plex_auth_poll_due_at: None,
            plex_servers: Vec::new(),
            plex_server_reachability: HashMap::new(),
            startup_plex_server_refresh_attempted: false,
            startup_plex_server_refresh_rx: None,
            plex_server_refresh_rx: None,
            plex_server_refresh_context: None,
            plex_sync_engine: None,
            plex_sync_rx: None,
            plex_sync_next_tick_due_at: None,
            plex_runtime_snapshot: GuiPlexRuntimeSnapshot::default(),
            pending_stream_retry_target: None,
            managed_stream_helper_refresh_required: false,
            pending_stream_feedback: VecDeque::new(),
            pending_stream_load_context: None,
        }
    }

    pub(in crate::app) fn with_config_path_and_startup_player(
        config_path: Option<PathBuf>,
    ) -> Self {
        Self::with_config_path_and_startup_player_lookup(config_path, &env_trimmed)
    }

    pub(in crate::app) fn with_config_path_and_startup_player_lookup<F>(
        config_path: Option<PathBuf>,
        lookup: &F,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut owner = Self::with_config_path(config_path);
        let startup_settings = owner.load_startup_player_settings_from_config_path();
        owner.configure_startup_player_from_lookup_and_settings(lookup, startup_settings.as_ref());
        owner.refresh_startup_stream_helper_snapshot();
        owner
    }

    fn load_startup_player_settings_from_config_path(&self) -> Option<StoredClientSettingsMvp> {
        self.config_path.as_ref().and_then(|path| {
            load_syncplay_ini_stored_client_settings_mvp_from_path(path)
                .ok()
                .flatten()
        })
    }

    fn no_player_configured_unavailability_reason(
        launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> Option<String> {
        matches!(launch_state, GuiPlayerLaunchRuntimeState::None).then_some(
            "Set playerPath to mpv in GUI settings, or set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
                .to_owned(),
        )
    }

    fn startup_player_unavailability_reason(
        launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> Option<String> {
        if let Some(reason) = launch_state.default_unavailability_reason() {
            return Some(reason);
        }
        if let GuiPlayerLaunchRuntimeState::ManagedMpv(config) = launch_state {
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
        }
        Self::no_player_configured_unavailability_reason(launch_state)
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
        self.player_local_file_placeholder = false;
        self.player_position_seconds = None;
        self.player_paused = None;
        self.pending_attached_cache_unpause = false;
        self.pending_attached_player_pause_confirmation_pump = None;
        self.stream_helper_runtime_snapshot = GuiStreamHelperRuntimeSnapshot::default();
        self.pending_stream_retry_target = None;
        self.pending_stream_feedback.clear();
        self.pending_stream_load_context = None;
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
        self.clear_session_attached_player_sync_state();
        self.playlist_auto_advance_eof_latched = false;
    }

    fn clear_attached_media_search_runtime_cache(&mut self) {
        if let Some(pending_resolution) = self.pending_attached_media_resolution.take() {
            pending_resolution
                .cancel_flag
                .store(true, Ordering::Relaxed);
        }
        self.attached_media_search_index = None;
        self.attached_media_search_next_retry_at = None;
        self.attached_media_search_progress = None;
        self.attached_media_search_progress_updated_at = None;
        self.attached_media_search_build_state = GuiAttachedMediaSearchBuildState::Idle;
        self.attached_media_search_build_roots.clear();
        self.attached_media_search_index_revision =
            self.attached_media_search_index_revision.wrapping_add(1);
        self.unresolved_attached_media_target = None;
        self.last_attached_media_resolution_trigger = None;
    }

    pub(in crate::app) fn clear_session_attached_player_sync_state(&mut self) {
        self.last_applied_attached_room_playstate = None;
        self.suppressed_attached_room_playstate_after_playlist_reset = None;
        self.pending_local_attached_pause_override = None;
        self.pending_attached_cache_unpause = false;
        self.pending_attached_player_pause_confirmation_pump = None;
    }

    fn detach_player(&mut self) {
        self.player = None;
        self.managed_mpv_process = None;
        self.clear_player_runtime_cache();
    }

    fn attach_player_from_launch_state(&mut self, launch_state: GuiPlayerLaunchRuntimeState) {
        self.detach_player();
        self.player_launch_state = launch_state.clone();
        if !matches!(launch_state, GuiPlayerLaunchRuntimeState::ManagedMpv(_)) {
            self.managed_stream_helper_refresh_required = false;
        }
        self.player_unavailability_reason = launch_state
            .default_unavailability_reason()
            .or_else(|| Self::no_player_configured_unavailability_reason(&launch_state));
        match launch_state {
            GuiPlayerLaunchRuntimeState::None => {}
            GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {}
            GuiPlayerLaunchRuntimeState::TestPlayer => {
                self.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
                self.managed_stream_helper_refresh_required = false;
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
                        self.managed_stream_helper_refresh_required = false;
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
                let helper_root = self.legacy_gui_qsettings_root();
                let helper_path_prefixes =
                    managed_stream_helper_path_prefixes(helper_root.as_deref());
                let helper_downloader_path = helper_root
                    .as_deref()
                    .map(managed_stream_helper_downloader_path)
                    .filter(|path| path.is_file());
                match mpv_launch::spawn_managed_mpv_and_attach(
                    &config,
                    &helper_path_prefixes,
                    helper_downloader_path.as_deref(),
                ) {
                    Ok((mut adapter, guard)) => {
                        match apply_legacy_syncplay_ui_settings_to_mpv_adapter(
                            &mut adapter,
                            &config.ui_settings,
                        ) {
                            Ok(()) => {
                                self.managed_mpv_process = Some(guard);
                                self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
                                self.managed_stream_helper_refresh_required = false;
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

    pub(in crate::app) fn sync_player_from_lookup_and_settings<F>(
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

    pub(super) fn ensure_configured_player_attached(&mut self) {
        if self.player.is_some() {
            return;
        }
        if self.player_launch_state.can_attach_on_demand() {
            self.attach_player_from_launch_state(self.player_launch_state.clone());
        }
    }

    pub(in crate::app) fn player_runtime_available_for_actions(&self) -> bool {
        self.player.is_some() || self.player_launch_state.can_attach_on_demand()
    }

    pub(super) fn ensure_configured_player_attached_for_active_session(&mut self) {
        if self.session_active() {
            self.ensure_configured_player_attached();
        }
    }

    pub(super) fn poll_managed_mpv_process(&mut self) {
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

    pub(super) fn legacy_gui_qsettings_root(&self) -> Option<PathBuf> {
        self.config_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    pub(super) fn player_stream_helper_attach_mode(&self) -> StreamHelperAttachMode {
        match self.player_launch_state {
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc { .. } => {
                StreamHelperAttachMode::ExternalPlayer
            }
            GuiPlayerLaunchRuntimeState::None
            | GuiPlayerLaunchRuntimeState::TestPlayer
            | GuiPlayerLaunchRuntimeState::ManagedMpv(_)
            | GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {
                StreamHelperAttachMode::ManagedPlayer
            }
        }
    }

    pub(super) fn refresh_stream_helper_runtime_snapshot_for_target(
        &mut self,
        target: Option<&str>,
    ) -> GuiStreamHelperRuntimeSnapshot {
        let snapshot = probe_stream_helper_runtime_snapshot(
            self.legacy_gui_qsettings_root().as_deref(),
            self.player_stream_helper_attach_mode(),
            target,
        );
        self.stream_helper_runtime_snapshot = snapshot.clone();
        snapshot
    }

    pub(super) fn refresh_startup_stream_helper_snapshot(&mut self) {
        let snapshot = probe_stream_helper_startup_snapshot(
            self.legacy_gui_qsettings_root().as_deref(),
            self.player_stream_helper_attach_mode(),
        );
        self.stream_helper_runtime_snapshot = snapshot;
    }

    pub(super) fn queue_stream_feedback_actions(&mut self, actions: Vec<GuiShellAction>) {
        if actions.is_empty() {
            return;
        }
        self.pending_stream_feedback.push_back(actions);
    }

    pub(super) fn flush_pending_stream_feedback(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        while let Some(actions) = self.pending_stream_feedback.pop_front() {
            Self::push_actions_and_project(handle, projected_state, actions);
        }
    }

    pub(super) fn stream_helper_target_candidate(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<String> {
        self.pending_stream_retry_target
            .clone()
            .or_else(|| self.current_shared_playlist_target(state))
            .or_else(|| {
                self.player_local_file
                    .as_ref()
                    .and_then(|file| file.path.clone())
                    .filter(|target| target.contains("://"))
            })
            .or_else(|| self.stream_helper_runtime_snapshot.target.clone())
    }

    pub(super) fn recheck_stream_helper_runtime_snapshot(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> GuiStreamHelperRuntimeSnapshot {
        let target = self.stream_helper_target_candidate(state);
        self.refresh_stream_helper_runtime_snapshot_for_target(target.as_deref())
    }

    pub(super) fn update_stream_helper_remediation_runtime_snapshot(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        snapshot: GuiStreamHelperRemediationRuntimeSnapshot,
    ) {
        self.stream_helper_remediation_runtime_snapshot = snapshot.clone();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(snapshot)],
        );
    }

    pub(super) fn report_stream_helper_remediation_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        label: impl Into<String>,
        detail: Option<String>,
        progress_fraction: f32,
    ) {
        self.update_stream_helper_remediation_runtime_snapshot(
            handle,
            projected_state,
            GuiStreamHelperRemediationRuntimeSnapshot {
                active: true,
                label: Some(label.into()),
                detail,
                progress_fraction,
            },
        );
    }

    pub(super) fn clear_stream_helper_remediation_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        self.update_stream_helper_remediation_runtime_snapshot(
            handle,
            projected_state,
            GuiStreamHelperRemediationRuntimeSnapshot::default(),
        );
    }

    pub(super) fn mark_managed_player_stream_helper_refresh_required(&mut self) {
        if matches!(
            self.player_launch_state,
            GuiPlayerLaunchRuntimeState::ManagedMpv(_)
        ) && self.player.is_some()
        {
            self.managed_stream_helper_refresh_required = true;
        }
    }

    pub(super) fn refresh_managed_player_if_stream_helper_refresh_required(
        &mut self,
    ) -> Result<bool, String> {
        if !self.managed_stream_helper_refresh_required {
            return Ok(false);
        }
        let GuiPlayerLaunchRuntimeState::ManagedMpv(_) = self.player_launch_state else {
            self.managed_stream_helper_refresh_required = false;
            return Ok(false);
        };

        let launch_state = self.player_launch_state.clone();
        self.attach_player_from_launch_state(launch_state);
        self.refresh_player_state();
        if self.player.is_some() {
            self.managed_stream_helper_refresh_required = false;
            Ok(true)
        } else {
            Err(self
                .player_unavailability_reason
                .clone()
                .unwrap_or_else(|| {
                    "Retrying mpv with the updated stream helper did not attach a playback runtime."
                        .to_owned()
                }))
        }
    }

    pub(super) fn clear_gui_data(&mut self) -> Result<(), String> {
        self.clear_attached_media_search_runtime_cache();
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
}
