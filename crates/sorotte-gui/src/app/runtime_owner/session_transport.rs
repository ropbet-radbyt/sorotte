use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn with_session_runtime(
        mut self,
        session: Box<dyn GuiSessionRuntimeAdapter + Send>,
    ) -> Self {
        self.install_session_runtime(session);
        self.session_projects_to_shell = true;
        self
    }

    pub(in crate::app) fn install_session_runtime(
        &mut self,
        session: Box<dyn GuiSessionRuntimeAdapter + Send>,
    ) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.active_session_settings = None;
        self.active_session_configured_settings = None;
        self.session = Some(session);
    }

    pub(in crate::app) fn install_active_session_runtime(
        &mut self,
        session: Box<dyn GuiSessionRuntimeAdapter + Send>,
        runtime_settings: StoredClientSettingsRuntimeSnapshot,
    ) {
        self.install_session_runtime(session);
        self.active_session_configured_settings = Some(runtime_settings.clone());
        self.active_session_settings = Some(runtime_settings);
        self.session_projects_to_shell = true;
    }

    pub(in crate::app) fn remove_session_runtime(&mut self) {
        if self.session.take().is_some() {
            self.session_generation = self.session_generation.wrapping_add(1);
        }
        self.active_session_settings = None;
        self.active_session_configured_settings = None;
    }

    fn with_session_default_room(mut self, room: impl Into<String>) -> Self {
        self.session_default_room = Some(room.into());
        self
    }

    fn with_session_transport(
        mut self,
        session_transport: GuiQueuedSessionTransportHandle,
    ) -> Self {
        self.session_transport = Some(session_transport);
        self
    }

    pub(in crate::app::runtime_owner) fn with_session_transport_driver(
        mut self,
        session_transport_driver: Box<dyn GuiSessionTransportDriver + Send>,
    ) -> Self {
        self.session_transport_driver = Some(session_transport_driver);
        self
    }

    pub(in crate::app) fn reset_session_transport_reconnect_state(&mut self) {
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

    pub(in crate::app) fn apply_session_transport_disconnect_pause(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
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

        if self.player.is_some() {
            let command_id = match self
                .begin_attached_player_pause_command(true, PlayerCommandCause::TransportRefresh)
            {
                Ok(command_id) => command_id,
                Err(error) => {
                    eprintln!("warning: failed to register pause-on-leave causal command: {error}");
                    return;
                }
            };
            let player = self
                .player
                .as_mut()
                .expect("pause-on-leave player was checked");
            let player_name = player.name().to_owned();
            let result = player.set_paused(true);
            let command_succeeded = result.is_ok();
            let command_result_error = self
                .finish_attached_player_pause_command(command_id, command_succeeded)
                .err();
            if let Err(error) = result {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!(
                            "Pause-on-leave dispatch through the attached {player_name} player failed: {error}"
                        ),
                    }],
                );
                if let Some(command_result_error) = command_result_error {
                    eprintln!(
                        "warning: failed to register pause-on-leave command failure: {command_result_error}"
                    );
                }
                return;
            }
            if let Some(error) = command_result_error {
                eprintln!("warning: failed to register pause-on-leave command: {error}");
            }
            self.note_local_attached_player_pause_command(true);
        }
        self.refresh_player_state();
    }

    pub(in crate::app::runtime_owner) fn handle_session_transport_failure(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        error: String,
    ) {
        let error_message = format!("Session transport driver pump failed: {error}");
        eprintln!("{error_message}");
        let now_seconds = system_time_seconds();
        let _ = self.interrupt_attached_playback_recovery_impl("transport disconnect");
        if Self::session_transport_failure_is_terminal(&error) {
            self.handle_terminal_session_transport_failure(
                handle,
                projected_state,
                error_message,
                now_seconds,
            );
            return;
        }

        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message: error_message,
        }];
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
        self.pending_room_change_request = None;
        self.clear_session_attached_player_sync_state();

        if let Some(delay_seconds) = reconnect_delay
            && !stop_reconnect_requested
        {
            self.schedule_session_transport_reconnect(delay_seconds);
            return;
        }

        self.session_transport_reconnect_due_at = None;
        self.session_transport_disconnect_pending_cleanup = true;
    }

    fn session_transport_failure_is_terminal(error: &str) -> bool {
        error.starts_with("Session transport TCP received an invalid protocol line:")
            || error.starts_with("Session transport TCP received a non-UTF-8 line:")
    }

    fn handle_terminal_session_transport_failure(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        error_message: String,
        now_seconds: f64,
    ) {
        let disconnect_error = self
            .session
            .as_mut()
            .and_then(|session| session.disconnect_session(now_seconds).err());
        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message: error_message,
        }];
        if let Some(disconnect_error) = disconnect_error {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: disconnect_error,
            });
        }

        self.pending_room_change_request = None;
        self.session_transport_reconnect_due_at = None;
        self.session_transport_disconnect_pending_cleanup = true;
        Self::push_actions_and_project(handle, projected_state, actions);
        self.apply_session_transport_disconnect_pause(handle, projected_state);
    }

    fn handle_terminal_session_transport_apply_failure(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        error: String,
    ) {
        self.handle_terminal_session_transport_failure(
            handle,
            projected_state,
            format!("Inbound session transport message apply failed: {error}"),
            system_time_seconds(),
        );
    }

    pub(super) fn pump_due_session_transport_reconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
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
        session_transport.clear_inbound_protocol_lines();

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
        session_transport_driver.set_protocol_liveness_enabled(false);
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
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.remove_session_runtime();
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.session_default_room = None;
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
        self.pending_attached_media_resolution = None;
        self.unresolved_attached_media_target = None;
        self.clear_session_attached_player_sync_state();
        self.reset_session_transport_reconnect_state();

        let mut actions = self.sessionless_projection_actions(projected_state);
        if matches!(
            projected_state
                .pending_operation
                .as_ref()
                .map(|pending| pending.kind),
            Some(crate::app::GuiPendingOperationKind::DisconnectSession)
        ) {
            actions.push(GuiShellAction::CompleteSessionDisconnect);
        }
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn finish_pending_session_transport_disconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !self.session_transport_disconnect_pending_cleanup {
            return;
        }
        self.clear_session_runtime_after_transport_disconnect(handle, projected_state);
    }

    pub(super) fn drain_session_runtime_actions_and_finish_transport_disconnect(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        self.drain_session_runtime_actions(handle, projected_state);
        self.sync_session_transport_reconnect_state_from_handshake();
        self.finish_pending_session_transport_disconnect(handle, projected_state);
        self.reconcile_playlist_resolution_scope(handle, projected_state);
    }

    pub(in crate::app) fn with_client_core_chat_session_runtime(
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
                .config
                .connection
                .username
                .as_ref()
                .map(|username| username.as_str().to_owned())
                .unwrap_or_default(),
            runtime_settings
                .config
                .connection
                .room
                .as_ref()
                .map(|room| room.as_str().to_owned())
                .unwrap_or_default(),
            runtime_settings
                .config
                .connection
                .controlled_room_password
                .clone(),
        )?;
        session.apply_runtime_settings_snapshot(&runtime_settings)?;
        let session = Box::new(session);
        let session_transport = GuiQueuedSessionTransportHandle::default();
        Ok((
            self.with_session_runtime(session)
                .with_session_default_room(room)
                .with_session_transport(session_transport.clone()),
            session_transport,
        ))
    }

    pub(in crate::app) fn with_client_core_chat_loopback_session_runtime(
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

    pub(in crate::app) fn with_client_core_chat_tcp_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
        tls_policy: TlsPolicy,
    ) -> Result<Self, String> {
        let (owner, _session_transport) =
            self.with_client_core_chat_session_runtime(username, room)?;
        Ok(owner.with_session_transport_driver(Box::new(
            GuiThreadedTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
                host_arg.as_ref(),
                tls_policy,
            )?,
        )))
    }

    pub(super) fn pump_session_transport_driver(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        const MAX_RELIABLE_FRAME_WRITES_PER_PUMP: usize = 64;

        if self.apply_session_transport_outbound_delivery_results(handle, projected_state) {
            self.flush_session_transport_outbound(handle, projected_state);
        }
        let Some(session_transport) = self.session_transport.as_ref().cloned() else {
            return;
        };
        for _ in 0..MAX_RELIABLE_FRAME_WRITES_PER_PUMP {
            let liveness_enabled = self
                .session
                .as_ref()
                .is_some_and(|session| session.server_handshake_completed());
            let pump_result = {
                let Some(session_transport_driver) = self.session_transport_driver.as_mut() else {
                    return;
                };
                session_transport_driver.set_protocol_liveness_enabled(liveness_enabled);
                session_transport_driver.pump(&session_transport)
            };
            self.drain_session_transport_warnings(handle, projected_state, &session_transport);
            let frame_written =
                self.apply_session_transport_outbound_delivery_results(handle, projected_state);
            if let Err(error) = pump_result {
                session_transport.fail_pending_outbound_protocol_delivery(0, error.clone());
                self.apply_session_transport_outbound_delivery_results(handle, projected_state);
                // Complete frames received before FIN belong to the current
                // connection and remain valid. Apply them before disconnect
                // advances the generation so any State response is then
                // cleared with that failed generation.
                if self.drain_session_transport_inbound(handle, projected_state) {
                    return;
                }
                self.handle_session_transport_failure(handle, projected_state, error);
                return;
            }
            if !frame_written {
                return;
            }
            self.flush_session_transport_outbound(handle, projected_state);
        }
    }

    fn drain_session_transport_warnings(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        session_transport: &GuiQueuedSessionTransportHandle,
    ) {
        let actions = session_transport
            .drain_transport_warnings()
            .into_iter()
            .flat_map(|message| {
                [
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Warning,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            })
            .collect::<Vec<_>>();
        if !actions.is_empty() {
            Self::push_actions_and_project(handle, projected_state, actions);
        }
    }

    fn apply_session_transport_outbound_delivery_results(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return false;
        };
        let results = session_transport.drain_outbound_protocol_delivery_results();
        if results.is_empty() {
            return false;
        }

        let mut actions = Vec::new();
        let mut frame_written = false;
        for result in results {
            match result {
                GuiOutboundProtocolDeliveryResult::FrameWritten { token } => {
                    frame_written = true;
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.acknowledge_outbound_protocol_delivery(token)
                    {
                        actions.push(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: format!(
                                "Outbound protocol delivery acknowledgement failed: {error}"
                            ),
                        });
                    }
                }
                GuiOutboundProtocolDeliveryResult::FrameFailed {
                    token,
                    bytes_written,
                    message,
                } => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.fail_outbound_protocol_delivery(token)
                    {
                        actions.push(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: format!(
                                "Outbound protocol delivery failure handling failed: {error}"
                            ),
                        });
                    }
                    actions.push(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!(
                            "Outbound protocol frame failed after {bytes_written} bytes and remains queued for retry: {message}"
                        ),
                    });
                }
            }
        }
        if !actions.is_empty() {
            Self::push_actions_and_project(handle, projected_state, actions);
        }
        frame_written
    }

    pub(super) fn drain_session_transport_inbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return false;
        };
        let inbound_protocol_lines = session_transport.drain_inbound_protocol_lines();
        if inbound_protocol_lines.is_empty() {
            return false;
        }
        for inbound_protocol_line in inbound_protocol_lines {
            let apply_result = {
                let Some(session) = self.session.as_mut() else {
                    return false;
                };
                session.apply_message_json(&inbound_protocol_line)
            };
            if let Err(error) = apply_result {
                let stop_reconnect_requested = self
                    .session
                    .as_mut()
                    .is_some_and(|session| session.take_stop_reconnect_requested());
                if stop_reconnect_requested {
                    self.handle_terminal_session_transport_apply_failure(
                        handle,
                        projected_state,
                        error,
                    );
                    return true;
                }
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
        false
    }

    fn drain_session_runtime_actions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !self.session_projects_to_shell {
            if let Some(session) = self.session.as_mut() {
                let _ = session.drain_gui_actions(projected_state);
            }
            self.reconcile_playlist_resolution_scope(handle, projected_state);
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
        self.reconcile_playlist_resolution_scope(handle, projected_state);
        self.sync_active_shared_playlist_media_and_playstate_impl(projected_state);
    }

    pub(super) fn sync_active_shared_playlist_media_and_playstate_impl(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) {
        let selected_media_sync =
            self.sync_selected_shared_playlist_media_to_attached_player_impl(projected_state);
        let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
            self.session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent()),
        );
        self.apply_pending_playlist_index_reset_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );
        self.sync_session_playstate_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );
    }

    pub(super) fn flush_session_transport_outbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let _ = self.apply_session_transport_outbound_delivery_results(handle, projected_state);
        let Some(session_transport) = self.session_transport.as_ref().cloned() else {
            return;
        };
        let Some(session) = self.session.as_mut() else {
            return;
        };
        #[cfg(test)]
        if self.session_transport_driver.is_none() {
            // Legacy owner tests use the shared handle itself as an infallible
            // capture sink. Keep their batch-observation surface while all
            // fallible production drivers use staged, receipt-based delivery.
            match session.flush_outbound_protocol_lines() {
                Ok(lines) => session_transport.push_outbound_protocol_lines(lines),
                Err(error) => Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Outbound session transport flush failed: {error}"),
                    }],
                ),
            }
            return;
        }
        match session.begin_outbound_protocol_delivery() {
            Ok(Some(delivery)) => {
                if let Err(delivery) =
                    session_transport.try_push_outbound_protocol_delivery(delivery)
                {
                    let token = delivery.token();
                    let _ = session.fail_outbound_protocol_delivery(token);
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message:
                                "Outbound session transport already owns a reliable protocol frame."
                                    .to_owned(),
                        }],
                    );
                }
            }
            Ok(None) => {}
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
}
