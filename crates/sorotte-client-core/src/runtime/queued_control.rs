use super::*;

macro_rules! runtime_notification_outbox_methods {
    (
        $pending:ident,
        $acknowledge:ident,
        $flush_to_sink:ident,
        $control_front:ident,
        $control_acknowledge:ident,
        $control_flush:ident,
        $notification:ty
    ) => {
        pub fn $pending(&self) -> Option<&$notification> {
            self.control.$control_front()
        }

        pub fn $acknowledge(&mut self) -> Option<$notification> {
            self.control.$control_acknowledge()
        }

        /// Delivers pending notifications in FIFO order, acknowledging each
        /// notification only after the sink reports success.
        pub fn $flush_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
        where
            F: FnMut(&$notification) -> Result<(), E>,
        {
            self.control.$control_flush(notify)
        }
    };
}

impl<P> ClientRuntime<P, QueuedRuntimeControl>
where
    P: PlayerAdapter,
{
    pub(crate) fn outbound_state_sync_position_seconds(
        &self,
        now_seconds: f64,
        dont_slow_down_with_me: bool,
    ) -> Option<f64> {
        let local_position = self.session.model.playback.local_position?;
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
        let inbound_state = normalize_client_state_payload(inbound_state);
        self.ping_metrics_legacy_compatible
            .observe_normalized_inbound_state(&inbound_state);
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
            normalize_client_state_payload(inbound_state),
            client_latency_calculation,
            client_rtt,
            dont_slow_down_with_me,
            None,
        )
    }

    pub(crate) fn run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
        &mut self,
        inbound_state: ClientStateUpdate,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
    ) -> bool {
        // Legacy peers may send State before their authoritative Hello. The
        // inbound message itself proves which live transport generation owns
        // the response, so activate that generation while periodic heartbeats
        // remain gated on an active session below.
        self.control.activate_protocol_connection_generation();
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.model.playback.local_paused,
        ) else {
            let outbound_state = self.session.reconcile_ping_only_state_response(
                inbound_state,
                client_latency_calculation,
                client_rtt,
            );
            return self.control.queue_connection_scoped_state(outbound_state);
        };

        let outbound_state = self
            .session
            .reconcile_normalized_state_and_build_response_with_local_state_change_override(
                inbound_state,
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
                local_state_change_global_playstate,
            );
        self.control.queue_connection_scoped_state(outbound_state)
    }

    pub(crate) fn adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
        &self,
        inbound_state: &ClientStateUpdate,
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
        if !self.session.is_active() {
            return false;
        }
        self.control.activate_protocol_connection_generation();

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let client_latency_calculation = self
            .ping_metrics_legacy_compatible
            .client_latency_calculation_now();
        let client_rtt = self.ping_metrics_legacy_compatible.client_rtt_seconds();

        let outbound_state = if let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.model.playback.local_paused,
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

        self.control.queue_connection_scoped_state(outbound_state)
    }

    /// Transfers ownership of every queued protocol message to the caller.
    /// Fallible transports must use [`Self::flush_queued_protocol_lines_to_transport`]
    /// or the pending-line acknowledgement API instead.
    pub fn flush_queued_protocol_messages(&mut self) -> Vec<ProtocolMessage> {
        self.control.drain_outbound_messages()
    }

    /// Transfers an encoded batch to an infallible in-memory owner.
    /// Fallible transports must use [`Self::flush_queued_protocol_lines_to_transport`]
    /// or [`Self::pending_protocol_line`] plus [`Self::acknowledge_protocol_line`].
    pub fn flush_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        self.control.drain_outbound_message_lines()
    }

    pub fn pending_protocol_line(&self) -> Result<Option<String>, ProtocolError> {
        self.control.front_outbound_message_line()
    }

    pub fn acknowledge_protocol_line(&mut self) -> Option<ProtocolMessage> {
        self.control.acknowledge_outbound_message()
    }

    pub fn drain_reconnect_requests(&mut self) -> Vec<f64> {
        self.control.drain_reconnect_delays()
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        self.control.take_stop_reconnect_requested()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use [`Self::drain_autoplay_notifications_to_sink`].
    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.control.drain_autoplay_notifications()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use [`Self::drain_chat_notifications_to_sink`].
    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.control.drain_chat_notifications()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use
    /// [`Self::drain_controlled_room_creation_notifications_to_sink`].
    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        self.control.drain_controlled_room_creation_notifications()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use [`Self::drain_controller_auth_notifications_to_sink`].
    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.control.drain_controller_auth_notifications()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use [`Self::drain_user_change_notifications_to_sink`].
    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.control.drain_user_change_notifications()
    }

    /// Best-effort ownership transfer for an infallible consumer.
    /// Fallible consumers must use [`Self::drain_reconnect_notifications_to_sink`].
    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.control.drain_reconnect_notifications()
    }

    /// Transfers the latest coalesced telemetry snapshot to an infallible consumer.
    /// Fallible consumers must use
    /// [`Self::drain_player_playback_telemetry_updates_to_sink`].
    pub fn drain_player_playback_telemetry_updates(
        &mut self,
    ) -> Vec<PlayerPlaybackTelemetryUpdate> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.pending_player_playback_telemetry_updates.drain()
    }

    /// Reliably delivers protocol lines in FIFO order. A line is acknowledged
    /// only after `send_line` succeeds; on failure, that line and its tail stay queued.
    pub fn flush_queued_protocol_lines_to_transport<F>(
        &mut self,
        send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        self.control.flush_outbound_message_lines(send_line)
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

    runtime_notification_outbox_methods!(
        pending_autoplay_notification,
        acknowledge_autoplay_notification,
        drain_autoplay_notifications_to_sink,
        front_autoplay_notification,
        acknowledge_autoplay_notification,
        flush_autoplay_notifications,
        AutoplayCountdownNotification
    );
    runtime_notification_outbox_methods!(
        pending_controller_auth_notification,
        acknowledge_controller_auth_notification,
        drain_controller_auth_notifications_to_sink,
        front_controller_auth_notification,
        acknowledge_controller_auth_notification,
        flush_controller_auth_notifications,
        ControllerAuthTransitionNotification
    );
    runtime_notification_outbox_methods!(
        pending_controlled_room_creation_notification,
        acknowledge_controlled_room_creation_notification,
        drain_controlled_room_creation_notifications_to_sink,
        front_controlled_room_creation_notification,
        acknowledge_controlled_room_creation_notification,
        flush_controlled_room_creation_notifications,
        ControlledRoomCreationNotification
    );
    runtime_notification_outbox_methods!(
        pending_chat_notification,
        acknowledge_chat_notification,
        drain_chat_notifications_to_sink,
        front_chat_notification,
        acknowledge_chat_notification,
        flush_chat_notifications,
        ChatNotification
    );
    runtime_notification_outbox_methods!(
        pending_user_change_notification,
        acknowledge_user_change_notification,
        drain_user_change_notifications_to_sink,
        front_user_change_notification,
        acknowledge_user_change_notification,
        flush_user_change_notifications,
        UserChangeNotification
    );
    runtime_notification_outbox_methods!(
        pending_reconnect_notification,
        acknowledge_reconnect_notification,
        drain_reconnect_notifications_to_sink,
        front_reconnect_notification,
        acknowledge_reconnect_notification,
        flush_reconnect_notifications,
        ReconnectTransitionNotification
    );

    pub fn drain_player_playback_telemetry_updates_to_sink<F, E>(
        &mut self,
        notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&PlayerPlaybackTelemetryUpdate) -> Result<(), E>,
    {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.pending_player_playback_telemetry_updates
            .try_flush(notify)
    }
}
