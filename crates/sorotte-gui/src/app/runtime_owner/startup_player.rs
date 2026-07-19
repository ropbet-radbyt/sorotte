use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn with_config_path(config_path: Option<PathBuf>) -> Self {
        let update_config_root = config_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        Self {
            config_path,
            legacy_projection: None,
            session: None,
            active_session_settings: None,
            active_session_configured_settings: None,
            session_generation: 0,
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
            startup_public_server_hydration: StartupPublicServerHydrationState::default(),
            update_runtime: GuiUpdateRuntime::new(update_config_root),
            startup_stream_helper_probe_completed: false,
            startup_stream_helper_probe_rx: None,
            player: None,
            player_launch_state: GuiPlayerLaunchRuntimeState::None,
            player_apply_state: GuiPlayerApplyState::default(),
            managed_mpv_process: None,
            player_unavailability_reason: None,
            core_player_configuration_health: GuiCorePlayerConfigurationHealth::Ready,
            network_options_hook_failure_reason: None,
            pending_apply_requirements_refresh_required: false,
            player_integration_health: GuiPlayerIntegrationHealth::Ready,
            player_local_file: None,
            player_local_file_placeholder: false,
            last_published_local_file: None,
            last_published_media_match_signature: None,
            local_shared_playlist_media_match_signature_path: None,
            playlist_resolution: GuiPlaylistResolutionCoordinator::default(),
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
            pending_attached_room_unpause_observation: None,
            pending_attached_player_pause_confirmation_pump: None,
            pending_attached_player_pause_command: None,
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
            media_match_runtime_snapshot: GuiMediaMatchRuntimeSnapshot::default(),
            media_match_remediation_runtime_snapshot:
                GuiMediaMatchRemediationRuntimeSnapshot::default(),
            media_match_tool_worker_rx: None,
            media_match_background_worker_rx: None,
            media_match_background_worker_cancel: None,
            media_match_background_trigger_key: None,
            media_match_background_index_backup: None,
            media_match_background_cancel_disposition: None,
            media_match_remote_lookup_rx: None,
            media_match_remote_lookup_trigger_key: None,
            media_match_remote_lookup_result: None,
            media_match_wire_sync_token: None,
            plex_client: None,
            plex_auth_session: None,
            plex_auth_start_rx: None,
            plex_auth_poll_rx: None,
            plex_auth_poll_due_at: None,
            plex_servers: Vec::new(),
            plex_server_reachability: HashMap::new(),
            startup_plex_server_refresh_attempted: false,
            plex_server_discovery: GuiPlexServerDiscoveryCoordinator::default(),
            plex_sync_engine: None,
            plex_sync_rx: None,
            plex_sync_next_tick_due_at: None,
            plex_runtime_snapshot: GuiPlexRuntimeSnapshot::default(),
            plex_playlist_job_generation: 0,
            plex_playlist_search_job: None,
            plex_playlist_resolve_job: None,
            plex_stream_resolve_rx: None,
            plex_stream_resolve_trigger_key: None,
            plex_stream_resolve_context: None,
            plex_stream_resolve_result: None,
            pending_playlist_source_resolution: None,
            pending_stream_retry_target: None,
            managed_stream_helper_refresh_required: false,
            pending_stream_feedback: VecDeque::new(),
            pending_stream_load_context: None,
            pending_logical_media_override: None,
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
        owner.refresh_startup_stream_helper_snapshot(startup_settings.as_ref());
        owner.refresh_startup_media_match_snapshot(startup_settings.as_ref());
        owner
    }

    fn load_startup_player_settings_from_config_path(&self) -> Option<StoredClientSettingsMvp> {
        self.config_path.as_ref().and_then(|path| {
            load_sorotte_ini_stored_client_settings_mvp_from_path(path)
                .ok()
                .flatten()
        })
    }

    fn no_player_configured_unavailability_reason(
        launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> Option<String> {
        matches!(launch_state, GuiPlayerLaunchRuntimeState::None).then_some(
            "Set playerPath to mpv in GUI settings, or set SOROTTE_CLIENT_MPV_IPC_PATH or SOROTTE_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
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
        self.pending_attached_room_unpause_observation = None;
        self.pending_attached_player_pause_confirmation_pump = None;
        self.pending_attached_player_pause_command = None;
        self.stream_helper_runtime_snapshot = GuiStreamHelperRuntimeSnapshot::default();
        self.media_match_runtime_snapshot.current_decision = None;
        self.media_match_runtime_snapshot.nearest_match = None;
        self.media_match_runtime_snapshot.last_evidence = None;
        self.media_match_runtime_snapshot.remote_status = Some("unavailable".to_owned());
        self.clear_media_match_remote_lookup_state();
        self.clear_plex_stream_resolution_state();
        self.media_match_wire_sync_token = None;
        self.pending_stream_retry_target = None;
        self.pending_stream_feedback.clear();
        self.pending_stream_load_context = None;
        self.pending_logical_media_override = None;
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

    pub(super) fn clear_attached_media_search_runtime_cache(&mut self) {
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
        self.clear_media_match_remote_lookup_state();
    }

    pub(in crate::app) fn clear_session_attached_player_sync_state(&mut self) {
        self.last_applied_attached_room_playstate = None;
        self.suppressed_attached_room_playstate_after_playlist_reset = None;
        self.pending_local_attached_pause_override = None;
        self.pending_attached_room_unpause_observation = None;
        self.pending_attached_player_pause_confirmation_pump = None;
        self.pending_attached_player_pause_command = None;
    }

    pub(in crate::app) fn clear_media_match_remote_lookup_state(&mut self) {
        self.media_match_remote_lookup_rx = None;
        self.media_match_remote_lookup_trigger_key = None;
        self.media_match_remote_lookup_result = None;
    }

    pub(in crate::app::runtime_owner) fn detach_player(&mut self) {
        self.release_attached_sorotte_bridge_best_effort();
        self.player = None;
        self.managed_mpv_process = None;
        self.network_options_hook_failure_reason = None;
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        self.player_integration_health = GuiPlayerIntegrationHealth::Ready;
        self.clear_player_runtime_cache();
    }

    pub(in crate::app::runtime_owner) fn release_attached_sorotte_bridge_best_effort(&mut self) {
        if let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) {
            player.release_sorotte_bridge_best_effort();
        }
    }

    pub(in crate::app::runtime_owner) fn record_sorotte_bridge_health(
        &mut self,
        health: SorotteBridgeHealth,
    ) {
        self.player_integration_health =
            GuiPlayerIntegrationHealth::from_sorotte_bridge_health(health);
    }

    #[cfg(test)]
    pub(in crate::app::runtime_owner) fn record_fully_applied_player_launch_state(
        &mut self,
        launch_state: &GuiPlayerLaunchRuntimeState,
    ) {
        self.player_apply_state.record_core_apply(launch_state);
        self.player_apply_state.applied_mpv_ui_settings = launch_state.mpv_ui_settings().cloned();
        self.player_apply_state.acknowledged_bridge_settings =
            launch_state.mpv_ui_settings().cloned();
        self.player_apply_state.acknowledged_bridge_generation = launch_state
            .mpv_ui_settings()
            .is_some_and(LegacySyncplayUiSettings::uses_syncplayintf_bridge)
            .then_some(1);
    }

    pub(in crate::app::runtime_owner) fn retain_mpv_after_optional_bridge_attempt(
        &mut self,
        adapter: MpvAdapter,
        managed_process: Option<ManagedMpvProcessGuard>,
        bridge_health: SorotteBridgeHealth,
        core_ipc_was_connected: bool,
    ) -> Result<(), String> {
        if core_ipc_was_connected && !adapter.is_connected() {
            return Err(
                "mpv JSON IPC became unavailable while configuring optional Chat/OSD integration"
                    .to_owned(),
            );
        }
        self.managed_mpv_process = managed_process;
        self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
        self.record_sorotte_bridge_health(bridge_health);
        self.managed_stream_helper_refresh_required = false;
        self.player_unavailability_reason = None;
        Ok(())
    }

    pub(in crate::app::runtime_owner) fn complete_mpv_attachment_after_core_configuration(
        &mut self,
        mut adapter: MpvAdapter,
        managed_process: Option<ManagedMpvProcessGuard>,
        ui_settings: &sorotte_player_mpv::LegacySyncplayUiSettings,
    ) -> Result<(), String> {
        let core_ipc_was_connected = adapter.is_connected();
        let integration = configure_sorotte_chat_osd_integration(&mut adapter, ui_settings);
        let acknowledged_generation = adapter.sorotte_bridge_acknowledged_generation();
        self.retain_mpv_after_optional_bridge_attempt(
            adapter,
            managed_process,
            integration.bridge_health.clone(),
            core_ipc_was_connected,
        )?;
        if integration.mpv_ui_settings_applied {
            self.player_apply_state.applied_mpv_ui_settings = Some(ui_settings.clone());
        }
        if matches!(
            integration.bridge_health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Disabled
        ) {
            self.player_apply_state.acknowledged_bridge_settings = Some(ui_settings.clone());
            self.player_apply_state.acknowledged_bridge_generation = acknowledged_generation;
        }
        Ok(())
    }

    fn record_streaming_options_applied(&mut self, launch_state: &GuiPlayerLaunchRuntimeState) {
        self.player_apply_state
            .record_streaming_options_applied(launch_state);
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        self.restore_network_options_hook_degradation();
    }

    pub(in crate::app::runtime_owner) fn restore_network_options_hook_degradation(
        &mut self,
    ) -> bool {
        let Some(reason) = self.network_options_hook_failure_reason.clone() else {
            return false;
        };
        self.player_apply_state.core_reapply_required = true;
        self.pending_apply_requirements_refresh_required = true;
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place: true,
                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
            };
        self.player_unavailability_reason = Some(reason);
        true
    }

    fn record_streaming_apply_superseded(&mut self) {
        self.player_apply_state.mark_streaming_apply_superseded();
    }

    pub(super) fn mark_streaming_apply_failed(&mut self, reason: String, retryable_in_place: bool) {
        self.player_apply_state.mark_streaming_apply_failed();
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place,
                origin: GuiStreamingDegradationOrigin::ExplicitApply,
            };
        self.player_unavailability_reason = Some(reason);
    }

    pub(in crate::app::runtime_owner) fn complete_explicit_mpv_attachment_after_ipc_connect(
        &mut self,
        ipc_path: &str,
        ui_settings: &LegacySyncplayUiSettings,
        effective_streaming_options: &[EffectiveMpvStreamingOption],
        adapter: MpvAdapter,
    ) {
        self.complete_explicit_mpv_attachment_with_active_apply(
            ipc_path,
            ui_settings,
            effective_streaming_options,
            adapter,
            apply_effective_streaming_options_to_active_network_media_classified,
        );
    }

    fn complete_explicit_mpv_attachment_with_active_apply<F>(
        &mut self,
        ipc_path: &str,
        ui_settings: &LegacySyncplayUiSettings,
        effective_streaming_options: &[EffectiveMpvStreamingOption],
        mut adapter: MpvAdapter,
        apply_to_active_media: F,
    ) where
        F: FnOnce(
            &mut MpvAdapter,
        )
            -> Result<sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome, String>,
    {
        let launch_state = self.player_launch_state.clone();
        self.player_apply_state
            .record_process_target_applied(&launch_state);
        configure_effective_streaming_options_for_network_media(
            &mut adapter,
            effective_streaming_options,
        );

        let streaming_failure = match apply_to_active_media(&mut adapter) {
            Ok(sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome::Superseded) => {
                self.record_streaming_apply_superseded();
                None
            }
            Ok(_) => {
                self.record_streaming_options_applied(&launch_state);
                None
            }
            Err(error) if adapter.is_connected() => {
                let reason = format!(
                    "mpv JSON IPC attach succeeded at '{ipc_path}', but player streaming settings could not be applied: {error}"
                );
                self.mark_streaming_apply_failed(reason.clone(), true);
                Some(reason)
            }
            Err(error) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.player_unavailability_reason = Some(format!(
                    "mpv JSON IPC at '{ipc_path}' became unavailable while applying player streaming settings: {error}"
                ));
                return;
            }
        };

        if let Err(error) =
            self.complete_mpv_attachment_after_core_configuration(adapter, None, ui_settings)
        {
            self.player_unavailability_reason = Some(format!(
                "mpv JSON IPC at '{ipc_path}' became unavailable: {error}"
            ));
            return;
        }

        if let Some(reason) = streaming_failure {
            // Retaining the healthy adapter clears generic unavailability state. Restore the
            // scoped core error after optional Chat/OSD setup so it remains visible and cannot be
            // mistaken for an IPC attachment failure.
            self.player_unavailability_reason = Some(reason);
        }
    }

    #[cfg(test)]
    pub(in crate::app::runtime_owner) fn complete_explicit_mpv_attachment_with_active_apply_for_test<
        F,
    >(
        &mut self,
        ipc_path: &str,
        ui_settings: &LegacySyncplayUiSettings,
        effective_streaming_options: &[EffectiveMpvStreamingOption],
        adapter: MpvAdapter,
        apply_to_active_media: F,
    ) where
        F: FnOnce(
            &mut MpvAdapter,
        )
            -> Result<sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome, String>,
    {
        self.complete_explicit_mpv_attachment_with_active_apply(
            ipc_path,
            ui_settings,
            effective_streaming_options,
            adapter,
            apply_to_active_media,
        );
    }

    pub(in crate::app::runtime_owner) fn complete_managed_mpv_attachment_after_ipc_connect(
        &mut self,
        config: &mpv_launch::ManagedMpvLaunchConfig,
        adapter: MpvAdapter,
        guard: ManagedMpvProcessGuard,
    ) {
        self.complete_managed_mpv_attachment_with_active_apply(
            config,
            adapter,
            guard,
            apply_effective_streaming_options_to_active_network_media_classified,
        );
    }

    fn complete_managed_mpv_attachment_with_active_apply<F>(
        &mut self,
        config: &mpv_launch::ManagedMpvLaunchConfig,
        mut adapter: MpvAdapter,
        guard: ManagedMpvProcessGuard,
        apply_to_active_media: F,
    ) where
        F: FnOnce(
            &mut MpvAdapter,
        )
            -> Result<sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome, String>,
    {
        let launch_state = self.player_launch_state.clone();
        self.player_apply_state
            .record_process_target_applied(&launch_state);
        configure_effective_streaming_options_for_network_media(
            &mut adapter,
            &config.effective_streaming_options,
        );

        let streaming_failure = match apply_to_active_media(&mut adapter) {
            Ok(sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome::Superseded) => {
                self.record_streaming_apply_superseded();
                None
            }
            Ok(_) => {
                // No active media and local media are both complete policy installs: the adapter
                // applies these configured options when a later authoritative network path
                // becomes active, without guessing which launch arguments are positional media.
                self.record_streaming_options_applied(&launch_state);
                None
            }
            Err(error) if adapter.is_connected() => {
                let reason = format!(
                    "GUI-owned mpv launched and attached, but player streaming settings could not be applied: {error}"
                );
                self.mark_streaming_apply_failed(reason.clone(), true);
                Some(reason)
            }
            Err(error) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.player_unavailability_reason = Some(format!(
                    "GUI-owned mpv became unavailable while applying player streaming settings: {error}"
                ));
                return;
            }
        };

        if let Err(error) = self.complete_mpv_attachment_after_core_configuration(
            adapter,
            Some(guard),
            &config.ui_settings,
        ) {
            self.player_unavailability_reason =
                Some(format!("GUI-owned mpv became unavailable: {error}"));
            return;
        }

        if let Some(reason) = streaming_failure {
            // Retaining the healthy adapter and guard clears generic unavailability state. Restore
            // the scoped core error after optional Chat/OSD setup so the partial configuration
            // remains visible and independently retryable.
            self.player_unavailability_reason = Some(reason);
        }
    }

    #[cfg(test)]
    pub(in crate::app::runtime_owner) fn complete_managed_mpv_attachment_with_active_apply_for_test<
        F,
    >(
        &mut self,
        config: &mpv_launch::ManagedMpvLaunchConfig,
        adapter: MpvAdapter,
        guard: ManagedMpvProcessGuard,
        apply_to_active_media: F,
    ) where
        F: FnOnce(
            &mut MpvAdapter,
        )
            -> Result<sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome, String>,
    {
        self.complete_managed_mpv_attachment_with_active_apply(
            config,
            adapter,
            guard,
            apply_to_active_media,
        );
    }

    fn attach_player_from_launch_state(&mut self, launch_state: GuiPlayerLaunchRuntimeState) {
        self.detach_player();
        self.player_apply_state.clear_integration_baselines();
        if let Some(session) = self.session.as_mut() {
            session.reset_playback_transport_adapter_epoch(system_time_seconds());
        }
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
                self.player_apply_state
                    .record_core_apply(&self.player_launch_state);
            }
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings,
                effective_streaming_options,
            } => match MpvAdapter::with_json_ipc(&ipc_path) {
                Ok(adapter) => {
                    self.complete_explicit_mpv_attachment_after_ipc_connect(
                        &ipc_path,
                        &ui_settings,
                        &effective_streaming_options,
                        adapter,
                    );
                }
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
                    Ok((adapter, guard)) => {
                        self.complete_managed_mpv_attachment_after_ipc_connect(
                            &config, adapter, guard,
                        );
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
        if matches!(self.player_launch_state, GuiPlayerLaunchRuntimeState::None) {
            self.player_apply_state
                .record_core_apply(&self.player_launch_state);
        }
    }

    pub(super) fn configured_player_launch_state_from_lookup_and_settings<F>(
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
    ) -> Result<GuiPlayerLaunchRuntimeState, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        match env_flag_enabled_lookup(lookup, "SOROTTE_GUI_ENABLE_TEST_PLAYER") {
            Ok(true) => {
                return Ok(GuiPlayerLaunchRuntimeState::TestPlayer);
            }
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "SOROTTE_GUI_ENABLE_TEST_PLAYER could not be parsed: {error}"
                ));
            }
        }

        if let Some(ipc_path) = explicit_mpv_ipc_path_from_lookup(lookup) {
            let ui_settings =
                mpv_launch::legacy_syncplay_ui_settings_from_stored_settings(settings);
            let streaming = settings
                .map(|settings| ClientConfig::resolve(settings).config.playback.streaming)
                .unwrap_or_default();
            let advanced_arguments = settings
                .and_then(|settings| {
                    let player_path = settings.player_path.as_deref()?.trim();
                    settings.per_player_arguments.as_ref()?.get(player_path)
                })
                .map(Vec::as_slice)
                .unwrap_or_default();
            let effective_streaming_options = streaming.effective_mpv_options(advanced_arguments);
            return Ok(GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings: Box::new(ui_settings),
                effective_streaming_options,
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
        force_core_reapply: bool,
    ) -> bool {
        if !self
            .player_apply_state
            .process_target_is_applied(next_launch_state)
        {
            return false;
        }
        let Some(next_ui_settings) = next_launch_state.mpv_ui_settings().cloned() else {
            return false;
        };
        let Some(next_streaming_options) = next_launch_state
            .effective_mpv_streaming_options()
            .map(<[_]>::to_vec)
        else {
            return false;
        };
        let mpv_ui_settings_changed = !self
            .player_apply_state
            .mpv_ui_settings_are_applied(next_launch_state);
        let bridge_settings_changed = !self
            .player_apply_state
            .bridge_settings_are_acknowledged(next_launch_state);
        let integration_settings_changed = mpv_ui_settings_changed || bridge_settings_changed;
        let streaming_options_changed = force_core_reapply
            || self.player_apply_state.applied_streaming_options.as_ref()
                != Some(&next_streaming_options);
        if !integration_settings_changed && !streaming_options_changed {
            return false;
        }
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return false;
        };
        let core_ipc_was_connected = player.is_connected();
        if streaming_options_changed {
            configure_effective_streaming_options_for_network_media(
                player,
                &next_streaming_options,
            );
            match apply_effective_streaming_options_to_active_network_media_classified(player) {
                Ok(sorotte_player_mpv::MpvActiveNetworkMediaOptionsApplyOutcome::Superseded) => {
                    self.player_apply_state.mark_streaming_apply_superseded();
                }
                Ok(_) => {
                    self.player_apply_state
                        .record_streaming_options_applied(next_launch_state);
                    self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
                    if let Some(reason) = self.network_options_hook_failure_reason.clone() {
                        self.player_apply_state.core_reapply_required = true;
                        self.pending_apply_requirements_refresh_required = true;
                        self.core_player_configuration_health =
                            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                                reason: reason.clone(),
                                retryable_in_place: true,
                                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
                            };
                        self.player_unavailability_reason = Some(reason);
                    } else {
                        self.player_unavailability_reason = None;
                    }
                }
                Err(error) => {
                    let ipc_unhealthy = !player.is_connected();
                    if ipc_unhealthy {
                        self.detach_player();
                        self.player_unavailability_reason = Some(format!(
                            "mpv JSON IPC became unavailable while applying core streaming settings: {error}"
                        ));
                    } else {
                        self.mark_streaming_apply_failed(error, true);
                    }
                    return false;
                }
            }
        }
        self.player_launch_state = next_launch_state.clone();
        if integration_settings_changed {
            let integration = configure_sorotte_chat_osd_integration(player, &next_ui_settings);
            let acknowledged_generation = player.sorotte_bridge_acknowledged_generation();
            if core_ipc_was_connected && !player.is_connected() {
                self.detach_player();
                self.player_unavailability_reason = Some(
                    "mpv JSON IPC became unavailable while configuring optional Chat/OSD integration"
                        .to_owned(),
                );
                return false;
            }
            if integration.mpv_ui_settings_applied {
                self.player_apply_state.applied_mpv_ui_settings = Some(next_ui_settings.clone());
            }
            self.record_sorotte_bridge_health(integration.bridge_health.clone());
            if matches!(
                integration.bridge_health,
                SorotteBridgeHealth::Ready | SorotteBridgeHealth::Disabled
            ) {
                self.player_apply_state.acknowledged_bridge_settings = Some(next_ui_settings);
                self.player_apply_state.acknowledged_bridge_generation = acknowledged_generation;
            }
        }
        true
    }

    pub(in crate::app) fn apply_saved_player_settings_in_place(
        &mut self,
        settings: &StoredClientSettingsMvp,
    ) -> bool {
        let Ok(next_launch_state) = Self::configured_player_launch_state_from_lookup_and_settings(
            &env_trimmed,
            Some(settings),
        ) else {
            return false;
        };
        if matches!(next_launch_state, GuiPlayerLaunchRuntimeState::None) && self.player.is_none() {
            self.player_launch_state = GuiPlayerLaunchRuntimeState::None;
            self.player_apply_state
                .record_core_apply(&GuiPlayerLaunchRuntimeState::None);
            self.player_apply_state.clear_integration_baselines();
            self.player_unavailability_reason =
                Self::no_player_configured_unavailability_reason(&self.player_launch_state);
            return true;
        }
        if self
            .player_apply_state
            .process_target_is_applied(&next_launch_state)
        {
            let target_was_stale = self.player_launch_state != next_launch_state;
            let streaming_state_differs = !self
                .player_apply_state
                .streaming_options_are_applied(&next_launch_state);
            let bridge_state_differs = !self
                .player_apply_state
                .bridge_settings_are_acknowledged(&next_launch_state);
            let mpv_ui_state_differs = !self
                .player_apply_state
                .mpv_ui_settings_are_applied(&next_launch_state);
            if self.player.is_some()
                && (self.player_apply_state.core_reapply_required
                    || streaming_state_differs
                    || mpv_ui_state_differs
                    || bridge_state_differs)
            {
                let core_retry_required = self.player_apply_state.core_reapply_required;
                return self
                    .try_apply_mpv_ui_settings_in_place(&next_launch_state, core_retry_required)
                    && !self.player_apply_state.core_reapply_required
                    && !self.player_apply_state.streaming_apply_awaiting_transition;
            }
            // A failed relaunch can leave the desired launch state pointing at the failed target
            // while the last-applied baseline still describes the saved target. Reconcile the
            // on-demand target before clearing restart guidance on a later revert.
            self.player_launch_state = next_launch_state;
            if target_was_stale
                && self.player.is_none()
                && self.player_launch_state.can_attach_on_demand()
            {
                self.player_apply_state.core_reapply_required = true;
                self.player_unavailability_reason = Some(
                    "The saved player target was restored, but no playback runtime is attached; retry the player launch to apply it."
                        .to_owned(),
                );
                return false;
            }
            if target_was_stale {
                self.player_unavailability_reason = if self.player.is_some() {
                    None
                } else {
                    self.player_launch_state
                        .default_unavailability_reason()
                        .or_else(|| {
                            Self::no_player_configured_unavailability_reason(
                                &self.player_launch_state,
                            )
                        })
                };
            }
            return !self.player_apply_state.streaming_apply_awaiting_transition
                && (target_was_stale
                    || self.player.is_some()
                    || matches!(self.player_launch_state, GuiPlayerLaunchRuntimeState::None));
        }
        self.try_apply_mpv_ui_settings_in_place(&next_launch_state, false)
            && !self.player_apply_state.core_reapply_required
            && !self.player_apply_state.streaming_apply_awaiting_transition
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
        if !force_relaunch && self.try_apply_mpv_ui_settings_in_place(&next_launch_state, false) {
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

    pub(in crate::app) fn current_player_core_state_is_applied(&self) -> bool {
        !self.player_apply_state.core_reapply_required
            && !self.player_apply_state.streaming_apply_awaiting_transition
            && self
                .player_apply_state
                .process_target_is_applied(&self.player_launch_state)
            && self
                .player_apply_state
                .streaming_options_are_applied(&self.player_launch_state)
            && (self.player.is_some()
                || matches!(self.player_launch_state, GuiPlayerLaunchRuntimeState::None))
    }

    #[cfg(test)]
    pub(in crate::app) fn current_player_launch_state_is_applied(&self) -> bool {
        self.current_player_core_state_is_applied()
            && matches!(
                self.player_integration_health,
                GuiPlayerIntegrationHealth::Ready
            )
            && self
                .player_apply_state
                .mpv_ui_settings_are_applied(&self.player_launch_state)
            && self
                .player_apply_state
                .bridge_settings_are_acknowledged(&self.player_launch_state)
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

    pub(in crate::app) fn legacy_gui_qsettings_root(&self) -> Option<PathBuf> {
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

    pub(super) fn refresh_startup_stream_helper_snapshot(
        &mut self,
        settings: Option<&StoredClientSettingsMvp>,
    ) {
        if let Some(settings) = settings
            && !ClientConfig::resolve(settings)
                .config
                .plugins
                .stream_support_enabled
        {
            self.stream_helper_runtime_snapshot = GuiStreamHelperRuntimeSnapshot::default();
            return;
        }
        let snapshot = probe_stream_helper_startup_snapshot(
            self.legacy_gui_qsettings_root().as_deref(),
            self.player_stream_helper_attach_mode(),
        );
        self.stream_helper_runtime_snapshot = snapshot;
    }

    pub(super) fn refresh_media_match_runtime_snapshot(
        &mut self,
        settings: &sorotte_media_match::MediaMatchSettings,
    ) -> GuiMediaMatchRuntimeSnapshot {
        let mut snapshot = probe_media_match_runtime_snapshot(
            self.legacy_gui_qsettings_root().as_deref(),
            settings,
        );
        snapshot.current_decision = self.media_match_runtime_snapshot.current_decision.clone();
        snapshot.nearest_match = self.media_match_runtime_snapshot.nearest_match.clone();
        snapshot.last_evidence = self.media_match_runtime_snapshot.last_evidence.clone();
        snapshot.remote_status = self.media_match_runtime_snapshot.remote_status.clone();
        snapshot.background_status = self.media_match_runtime_snapshot.background_status.clone();
        self.media_match_runtime_snapshot = snapshot.clone();
        snapshot
    }

    pub(super) fn refresh_startup_media_match_snapshot(
        &mut self,
        settings: Option<&StoredClientSettingsMvp>,
    ) {
        if let Some(settings) = settings
            && !ClientConfig::resolve(settings)
                .config
                .plugins
                .media_matching_enabled
        {
            let state = GuiMediaMatchState::from_stored_settings(settings);
            self.media_match_runtime_snapshot = GuiMediaMatchRuntimeSnapshot::from(&state);
            return;
        }
        let snapshot = probe_media_match_startup_snapshot(
            self.legacy_gui_qsettings_root().as_deref(),
            settings,
        );
        self.media_match_runtime_snapshot = snapshot;
    }

    pub(super) fn update_media_match_remediation_runtime_snapshot(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        snapshot: GuiMediaMatchRemediationRuntimeSnapshot,
    ) {
        self.media_match_remediation_runtime_snapshot = snapshot.clone();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(snapshot)],
        );
    }

    pub(super) fn report_media_match_remediation_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        label: impl Into<String>,
        detail: Option<String>,
        progress_fraction: f32,
    ) {
        self.update_media_match_remediation_runtime_snapshot(
            handle,
            projected_state,
            GuiMediaMatchRemediationRuntimeSnapshot {
                active: true,
                label: Some(label.into()),
                detail,
                progress_fraction,
            },
        );
    }

    pub(super) fn clear_media_match_remediation_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        self.update_media_match_remediation_runtime_snapshot(
            handle,
            projected_state,
            GuiMediaMatchRemediationRuntimeSnapshot::default(),
        );
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
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        while let Some(actions) = self.pending_stream_feedback.pop_front() {
            Self::push_actions_and_project(handle, projected_state, actions);
        }
    }

    pub(super) fn stream_helper_target_candidate(
        &self,
        state: &SorotteGuiShellAppState,
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
        state: &SorotteGuiShellAppState,
    ) -> GuiStreamHelperRuntimeSnapshot {
        let target = self.stream_helper_target_candidate(state);
        self.refresh_stream_helper_runtime_snapshot_for_target(target.as_deref())
    }

    pub(super) fn update_stream_helper_remediation_runtime_snapshot(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
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
        projected_state: &mut SorotteGuiShellAppState,
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
        projected_state: &mut SorotteGuiShellAppState,
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
            clear_sorotte_ini_stored_client_settings_mvp_at_path(path).map_err(|error| {
                format!(
                    "failed clearing stored settings {}: {error}",
                    path.display()
                )
            })?;
        }
        if let Some(root) = self.legacy_gui_qsettings_root() {
            clear_legacy_gui_qsettings_files_at_root(&root)?;
            clear_persisted_media_search_cache_at_root(&root)?;
            clear_persisted_media_match_cache_at_root(&root)?;
        }
        self.clear_persisted_plex_match_cache()?;
        self.remove_session_runtime();
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.reset_session_transport_reconnect_state();
        Ok(())
    }
}

impl Drop for GuiPersistedConfigRuntimeOwner {
    fn drop(&mut self) {
        self.release_attached_sorotte_bridge_best_effort();
    }
}
