use super::*;

impl<P> ClientRuntime<P, QueuedRuntimeControl>
where
    P: PlayerAdapter,
{
    pub(crate) fn outbound_state_sync_position_seconds(
        &self,
        now_seconds: f64,
        dont_slow_down_with_me: bool,
    ) -> Option<f64> {
        let local_position = self.session.local_position?;
        if !dont_slow_down_with_me {
            return Some(local_position);
        }

        self.session
            .current_room_playstate_at(now_seconds)
            .and_then(|playstate| playstate.position)
            .or(Some(local_position))
    }

    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        &mut self,
        inbound_state: StatePayload,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.ping_metrics_legacy_compatible
            .observe_inbound_state(&inbound_state);
        let local_state_change_global_playstate = self
            .adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
                &inbound_state,
            );
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            inbound_state,
            self.ping_metrics_legacy_compatible
                .client_latency_calculation_now(),
            self.ping_metrics_legacy_compatible.client_rtt_seconds(),
            dont_slow_down_with_me,
            local_state_change_global_playstate,
        )
    }

    pub fn run_state_sync_reconcile_with_inbound_state(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            inbound_state,
            client_latency_calculation,
            client_rtt,
            dont_slow_down_with_me,
            None,
        )
    }

    pub(crate) fn run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
    ) -> bool {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.local_paused,
        ) else {
            self.session.apply_state(inbound_state);
            return false;
        };

        let outbound_state = self
            .session
            .reconcile_state_and_build_response_with_local_state_change_override(
                inbound_state,
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
                local_state_change_global_playstate,
            );
        self.control
            .outbound_messages
            .push(ProtocolMessage::state(outbound_state));
        true
    }

    pub(crate) fn adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
        &self,
        inbound_state: &StatePayload,
    ) -> Option<RoomPlaystateView> {
        let playstate = inbound_state.playstate.as_ref()?;
        let mut position = playstate.position;
        if playstate.paused == Some(false) {
            let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
            if forward_delay.is_finite()
                && forward_delay > 0.0
                && let Some(raw_position) = position.filter(|value| value.is_finite())
            {
                position = Some(raw_position + forward_delay);
            }
        }

        Some(RoomPlaystateView {
            position,
            paused: playstate.paused,
            do_seek: Some(playstate.do_seek.unwrap_or(false)),
            set_by: playstate.set_by.clone(),
        })
    }

    pub fn run_state_sync_heartbeat_legacy_ping_compatible(
        &mut self,
        dont_slow_down_with_me: bool,
    ) -> bool {
        if self.session.server_chat_supported().is_none() {
            return false;
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let client_latency_calculation = self
            .ping_metrics_legacy_compatible
            .client_latency_calculation_now();
        let client_rtt = self.ping_metrics_legacy_compatible.client_rtt_seconds();

        let outbound_state = if let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.local_paused,
        ) {
            self.session.reconcile_state_and_build_response(
                StatePayload::new(),
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
            )
        } else {
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(client_latency_calculation)
                    .with_client_rtt(client_rtt),
            )
        };

        self.control
            .outbound_messages
            .push(ProtocolMessage::state(outbound_state));
        true
    }

    pub fn flush_queued_protocol_messages(&mut self) -> Vec<ProtocolMessage> {
        self.control.drain_outbound_messages()
    }

    pub fn flush_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        self.control.drain_outbound_message_lines()
    }

    pub fn drain_reconnect_requests(&mut self) -> Vec<f64> {
        self.control.drain_reconnect_delays()
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        self.control.take_stop_reconnect_requested()
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.control.drain_autoplay_notifications()
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.control.drain_chat_notifications()
    }

    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        self.control.drain_controlled_room_creation_notifications()
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.control.drain_controller_auth_notifications()
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.control.drain_user_change_notifications()
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.control.drain_reconnect_notifications()
    }

    pub fn drain_player_playback_telemetry_updates(
        &mut self,
    ) -> Vec<PlayerPlaybackTelemetryUpdate> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        std::mem::take(&mut self.pending_player_playback_telemetry_updates)
    }

    pub fn flush_queued_protocol_lines_to_transport<F>(
        &mut self,
        mut send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        let lines = self.flush_queued_protocol_lines()?;
        for line in &lines {
            send_line(line)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_intents<FS, FT>(
        &mut self,
        mut schedule_reconnect: FS,
        mut stop_reconnect: FT,
    ) where
        FS: FnMut(f64),
        FT: FnMut(),
    {
        for delay_seconds in self.drain_reconnect_requests() {
            schedule_reconnect(delay_seconds);
        }
        if self.take_stop_reconnect_requested() {
            stop_reconnect();
        }
    }

    pub fn drain_autoplay_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&AutoplayCountdownNotification) -> Result<(), E>,
    {
        for notification in self.drain_autoplay_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_controller_auth_notifications_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&ControllerAuthTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_controller_auth_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_controlled_room_creation_notifications_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&ControlledRoomCreationNotification) -> Result<(), E>,
    {
        for notification in self.drain_controlled_room_creation_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_chat_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ChatNotification) -> Result<(), E>,
    {
        for notification in self.drain_chat_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_user_change_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&UserChangeNotification) -> Result<(), E>,
    {
        for notification in self.drain_user_change_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ReconnectTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_reconnect_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_player_playback_telemetry_updates_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&PlayerPlaybackTelemetryUpdate) -> Result<(), E>,
    {
        for update in self.drain_player_playback_telemetry_updates() {
            notify(&update)?;
        }
        Ok(())
    }
}
