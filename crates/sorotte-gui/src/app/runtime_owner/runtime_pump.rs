use super::super::feature_slices::GuiClientCommand;
use super::super::remote_services;
use super::*;

impl Default for GuiPersistedConfigRuntimeOwner {
    fn default() -> Self {
        Self::with_config_path_and_startup_player(
            resolve_sorotte_gui_config_path_legacy_compatible(),
        )
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn poll_cached_runtime(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        let Some(mut projected_state) = self.legacy_projection.take() else {
            return;
        };
        projected_state = self.pump_runtime_projection_owned(handle, projected_state);
        self.legacy_projection = Some(projected_state);
    }

    fn pump_runtime_projection_owned(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        mut projected_state: SorotteGuiShellAppState,
    ) -> SorotteGuiShellAppState {
        self.runtime_pump_generation = self.runtime_pump_generation.wrapping_add(1);
        self.poll_managed_mpv_process();
        let mut media_resolution_completed = false;
        self.pump_due_session_transport_reconnect(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions_and_finish_transport_disconnect(
            handle,
            &mut projected_state,
        );
        self.drain_player_chat_input(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
        self.flush_session_transport_outbound(handle, &mut projected_state);
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions_and_finish_transport_disconnect(
            handle,
            &mut projected_state,
        );
        self.drain_player_chat_input(handle, &mut projected_state);
        self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
        self.pump_media_match_tool_worker(handle, &mut projected_state);
        self.pump_media_match_background_worker(handle, &mut projected_state);
        media_resolution_completed |= self.pump_media_match_remote_lookup_worker();
        let _ = self.maybe_sync_media_match_wire_decisions(handle, &mut projected_state);
        if !self.startup_saved_connect_attempted {
            self.startup_saved_connect_attempted = true;
            if projected_state.pending_operation.is_none()
                && !self.session_active()
                && projected_state.saved_session_connect_target().is_some()
            {
                self.complete_saved_server_connect_runtime(handle, &mut projected_state, false);
                self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
                self.flush_session_transport_outbound(handle, &mut projected_state);
                self.pump_session_transport_driver(handle, &mut projected_state);
                self.drain_session_transport_inbound(handle, &mut projected_state);
                self.drain_session_runtime_actions_and_finish_transport_disconnect(
                    handle,
                    &mut projected_state,
                );
                self.drain_player_chat_input(handle, &mut projected_state);
                self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
            }
        }
        for command in handle.drain_client_commands() {
            let handled = match command {
                GuiClientCommand::Player(command) => {
                    self.handle_player_command(handle, &mut projected_state, command)
                }
                GuiClientCommand::Updates(command) => {
                    self.update_runtime.handle_command(handle, *command);
                    true
                }
                GuiClientCommand::Legacy { request, .. } => {
                    self.handle_runtime_request(handle, &mut projected_state, *request)
                }
            };
            if !handled {
                continue;
            }
            self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
            self.flush_session_transport_outbound(handle, &mut projected_state);
            self.pump_session_transport_driver(handle, &mut projected_state);
            self.drain_session_transport_inbound(handle, &mut projected_state);
            self.drain_session_runtime_actions_and_finish_transport_disconnect(
                handle,
                &mut projected_state,
            );
            self.drain_player_chat_input(handle, &mut projected_state);
            self.sync_detached_session_runtime_state_or_notify(handle, &mut projected_state);
            self.pump_media_match_tool_worker(handle, &mut projected_state);
            self.pump_media_match_background_worker(handle, &mut projected_state);
            media_resolution_completed |= self.pump_media_match_remote_lookup_worker();
            let _ = self.maybe_sync_media_match_wire_decisions(handle, &mut projected_state);
        }
        self.ensure_configured_player_attached_for_active_session();
        self.maybe_queue_media_match_exact_playlist_signature(handle, &mut projected_state);
        media_resolution_completed |= self.pump_media_match_remote_lookup_worker();
        let _ = self.maybe_sync_media_match_wire_decisions(handle, &mut projected_state);
        self.maybe_queue_media_match_background_warmup(handle, &mut projected_state);
        media_resolution_completed |= self.pump_plex_stream_resolution_worker();
        if media_resolution_completed {
            let _ = self.retry_pending_playlist_source_resolution(handle, &mut projected_state);
            self.sync_active_shared_playlist_media_and_playstate_impl(&projected_state);
        }
        self.sync_player_runtime_state(handle, &projected_state);
        self.pump_startup_plex_server_refresh(handle, &mut projected_state);
        self.pump_plex_server_refresh(handle, &mut projected_state);
        self.pump_plex_auth_poll(handle, &mut projected_state);
        self.pump_plex_playlist_workers(handle, &mut projected_state);
        self.sync_plex_watch_state(handle, &mut projected_state);
        self.run_deferred_startup_remote_actions(handle, &mut projected_state);
        self.update_runtime.pump_background_check(handle);
        self.run_deferred_startup_stream_helper_probe(handle, &mut projected_state);
        projected_state
    }

    fn run_deferred_startup_remote_actions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !self.startup_remote_actions_attempted {
            self.startup_remote_actions_attempted = true;
            let settings = projected_state.configuration.to_stored_settings();
            if remote_services::should_run_automatic_update_check(
                Some(&settings),
                std::time::SystemTime::now(),
            ) {
                self.update_runtime.start_startup_check(handle);
                return;
            }

            if settings.check_for_updates_automatically != Some(true)
                || settings
                    .public_servers
                    .as_ref()
                    .is_some_and(|rows| !rows.is_empty())
            {
                return;
            }

            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("sorotte-gui-startup-remote".to_owned())
                .spawn(move || {
                    let actions =
                        gui_startup_public_server_actions_with_fetcher(&settings, |language| {
                            remote_services::fetch_public_servers(Some(language))
                        });
                    let _ = tx.send(actions);
                }) {
                Ok(_thread) => {
                    self.startup_remote_actions_rx = Some(rx);
                }
                Err(error) => {
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::AnnounceSystemChatEvent(format!(
                            "Unable to start startup remote-service worker: {error}"
                        ))],
                    );
                }
            }
        }

        let Some(rx) = self.startup_remote_actions_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(actions) => {
                self.update_runtime.observe_actions(&actions);
                Self::push_actions_and_project(handle, projected_state, actions);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.startup_remote_actions_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
    }

    fn run_deferred_startup_stream_helper_probe(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            self.startup_stream_helper_probe_completed = true;
            self.startup_stream_helper_probe_rx = None;
            return;
        }
        if self.startup_stream_helper_probe_completed {
            return;
        }
        if self.startup_stream_helper_probe_rx.is_none() {
            if cfg!(test) {
                self.startup_stream_helper_probe_completed = true;
                return;
            }
            let root = self.legacy_gui_qsettings_root();
            let attach_mode = self.player_stream_helper_attach_mode();
            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("sorotte-gui-startup-stream-helper".to_owned())
                .spawn(move || {
                    let snapshot =
                        probe_stream_helper_runtime_snapshot(root.as_deref(), attach_mode, None);
                    let _ = tx.send(snapshot);
                }) {
                Ok(_thread) => {
                    self.startup_stream_helper_probe_rx = Some(rx);
                }
                Err(_error) => {
                    let snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(None);
                    self.startup_stream_helper_probe_completed = true;
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                            snapshot,
                        )],
                    );
                    return;
                }
            }
        }

        let Some(rx) = self.startup_stream_helper_probe_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(snapshot) => {
                self.startup_stream_helper_probe_completed = true;
                self.stream_helper_runtime_snapshot = snapshot.clone();
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                        snapshot,
                    )],
                );
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.startup_stream_helper_probe_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.startup_stream_helper_probe_completed = true;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn apply_deferred_startup_remote_actions_for_test(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) {
        if self.startup_remote_actions_attempted {
            return;
        }
        self.startup_remote_actions_attempted = true;
        self.update_runtime.observe_actions(&actions);
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    #[cfg(test)]
    pub(super) fn apply_deferred_startup_stream_helper_snapshot_for_test(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        snapshot: GuiStreamHelperRuntimeSnapshot,
    ) {
        if self.startup_stream_helper_probe_completed {
            return;
        }
        self.startup_stream_helper_probe_completed = true;
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                snapshot,
            )],
        );
    }

    fn sync_detached_session_runtime_state_or_notify(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        self.refresh_player_state();
        self.flush_pending_stream_feedback(handle, projected_state);
        if let Err(error) = self.sync_detached_session_preferences_and_player_state(projected_state)
        {
            Self::push_runtime_unavailable(handle, error);
        }
    }
}
