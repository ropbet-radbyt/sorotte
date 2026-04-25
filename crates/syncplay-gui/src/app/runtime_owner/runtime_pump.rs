use super::*;

impl Default for GuiPersistedConfigRuntimeOwner {
    fn default() -> Self {
        Self::with_config_path_and_startup_player(
            resolve_syncplay_gui_config_path_legacy_compatible(),
        )
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn pump_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
    ) {
        self.poll_managed_mpv_process();
        let mut projected_state = state.clone();
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
        for request in handle.drain_requests() {
            if !self.handle_runtime_request(handle, &mut projected_state, request) {
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
        }
        self.ensure_configured_player_attached_for_active_session();
        self.sync_player_runtime_state(handle, &projected_state);
    }

    fn sync_detached_session_runtime_state_or_notify(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        self.refresh_player_state();
        self.flush_pending_stream_feedback(handle, projected_state);
        if let Err(error) = self.sync_detached_session_preferences_and_player_state(projected_state)
        {
            Self::push_runtime_unavailable(handle, error);
        }
    }
}
