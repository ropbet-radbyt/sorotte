use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
struct StateSyncReconcileClocks {
    received_at_seconds: f64,
    response_at_seconds: f64,
}

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
    fn refresh_player_projection_before_state_sync(&mut self, now_seconds: f64) -> bool {
        if self.player.player_event_delivery_mode()
            != sorotte_player_api::PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        {
            self.sync_player_playback_telemetry_into_session_and_buffer();
            return true;
        }
        if self.pending_state_sync_player_error.is_some() {
            return false;
        }
        match self.drain_player_transport_coordination(now_seconds) {
            Ok(()) => true,
            Err(error) => {
                self.pending_state_sync_player_error = Some(error);
                false
            }
        }
    }

    fn queue_connection_scoped_state_with_participant_status(
        &mut self,
        mut state: StatePayload,
        force: bool,
        now_seconds: f64,
    ) -> bool {
        let pending = self
            .playback_coordination
            .pending_participant_status_report(&self.session, force, now_seconds);
        if let Some(pending) = pending.as_ref() {
            state = state.with_participant_status_v1(
                ParticipantStatusStateExtension::new().with_report(pending.report.clone()),
            );
        }
        let queued = self.control.queue_connection_scoped_state(state);
        if queued && let Some(pending) = pending.as_ref() {
            self.playback_coordination
                .commit_participant_status_report(pending);
        }
        queued
    }
    pub(crate) fn outbound_state_sync_position_seconds(
        &self,
        now_seconds: f64,
        dont_slow_down_with_me: bool,
    ) -> Option<f64> {
        if dont_slow_down_with_me {
            return self
                .session
                .current_room_playstate_at(now_seconds)
                .and_then(|playstate| playstate.position)
                .or_else(|| self.projected_local_position_at(now_seconds));
        }

        self.projected_local_position_at(now_seconds)
    }

    fn outbound_state_sync_position_seconds_for_pause_mutation(
        &self,
        now_seconds: f64,
        dont_slow_down_with_me: bool,
        has_local_pause_mutation_intent: bool,
    ) -> Option<f64> {
        self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me)
            .or_else(|| {
                has_local_pause_mutation_intent.then_some(())?;
                self.playback_coordination
                    .local_pause_mutation_position_at(
                        now_seconds,
                        self.session.model.playback.local_position,
                    )
                    .or_else(|| {
                        // A canonical room position is the neutral fallback
                        // when the local player is between trustworthy
                        // samples. The outgoing mutation carries doSeek=false,
                        // so this preserves the Play/Pause command without
                        // laundering cache telemetry into a seek.
                        self.session
                            .current_room_playstate_at(now_seconds)
                            .and_then(|playstate| playstate.position)
                            .filter(|position| position.is_finite() && *position >= 0.0)
                    })
            })
    }

    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        &mut self,
        inbound_state: StatePayload,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            inbound_state,
            dont_slow_down_with_me,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
        &mut self,
        inbound_state: StatePayload,
        dont_slow_down_with_me: bool,
        received_at_seconds: f64,
    ) -> bool {
        self.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks(
            inbound_state,
            dont_slow_down_with_me,
            received_at_seconds,
            received_at_seconds,
            received_at_seconds,
        )
    }

    /// Reconciles an inbound State while preserving separate receipt, reply,
    /// and legacy ping clocks. The local playback sample is projected to the
    /// reply clock; inbound room state remains anchored to network receipt.
    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks(
        &mut self,
        inbound_state: StatePayload,
        dont_slow_down_with_me: bool,
        received_at_seconds: f64,
        response_at_seconds: f64,
        ping_received_at_seconds: f64,
    ) -> bool {
        let inbound_state = normalize_client_state_payload(inbound_state);
        self.ping_metrics_legacy_compatible
            .observe_normalized_inbound_state_at(&inbound_state, ping_received_at_seconds);
        let local_state_change_global_playstate = self
            .adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
                &inbound_state,
                received_at_seconds,
                response_at_seconds,
            );
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            inbound_state,
            self.ping_metrics_legacy_compatible
                .client_latency_calculation_now(),
            self.ping_metrics_legacy_compatible.client_rtt_seconds(),
            dont_slow_down_with_me,
            local_state_change_global_playstate,
            StateSyncReconcileClocks {
                received_at_seconds,
                response_at_seconds,
            },
        )
    }

    pub fn run_state_sync_reconcile_with_inbound_state(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
    ) -> bool {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            normalize_client_state_payload(inbound_state),
            client_latency_calculation,
            client_rtt,
            dont_slow_down_with_me,
            None,
            StateSyncReconcileClocks {
                received_at_seconds: now_seconds,
                response_at_seconds: now_seconds,
            },
        )
    }

    fn run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
        &mut self,
        inbound_state: ClientStateUpdate,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
        clocks: StateSyncReconcileClocks,
    ) -> bool {
        // Legacy peers may send State before their authoritative Hello. The
        // inbound message itself proves which live transport generation owns
        // the response, so activate that generation while periodic heartbeats
        // remain gated on an active session below.
        self.control.activate_protocol_connection_generation();
        let player_projection_is_current =
            self.refresh_player_projection_before_state_sync(clocks.response_at_seconds);
        let inbound_transport_revision = inbound_state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.transport_revision);
        let inbound_do_seek = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.do_seek == Some(true));
        let local_pause_mutation_intent = self
            .playback_coordination
            .active_local_pause_state_mutation_intent_for_inbound_transport(
                &self.session,
                inbound_transport_revision,
                inbound_do_seek,
            );
        let local_playback = player_projection_is_current.then(|| {
            (
                self.outbound_state_sync_position_seconds_for_pause_mutation(
                    clocks.response_at_seconds,
                    dont_slow_down_with_me,
                    local_pause_mutation_intent.is_some(),
                ),
                self.session.model.playback.local_paused,
            )
        });
        let Some((Some(local_position), Some(local_paused))) = local_playback else {
            let outbound_state = self.session.reconcile_ping_only_state_response(
                inbound_state,
                client_latency_calculation,
                client_rtt,
                clocks.received_at_seconds,
            );
            return self.queue_connection_scoped_state_with_participant_status(
                outbound_state,
                true,
                clocks.response_at_seconds,
            );
        };

        let outbound_state = self
            .session
            .reconcile_normalized_state_and_build_response_with_local_state_change_override(
                inbound_state,
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
                crate::session::StateReconcileContext {
                    local_state_change_global_playstate,
                    local_pause_mutation_intent,
                    received_at_seconds: clocks.received_at_seconds,
                },
            );
        self.queue_connection_scoped_state_with_participant_status(
            outbound_state,
            true,
            clocks.response_at_seconds,
        )
    }

    pub(crate) fn adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
        &self,
        inbound_state: &ClientStateUpdate,
        received_at_seconds: f64,
        response_at_seconds: f64,
    ) -> Option<RoomPlaystateView> {
        let playstate = inbound_state.playstate.as_ref()?;
        let mut position = playstate.position;
        if playstate.paused == Some(false) {
            let mut projection_seconds =
                self.ping_metrics_legacy_compatible.forward_delay_seconds();
            let response_delay_seconds = response_at_seconds - received_at_seconds;
            if response_delay_seconds.is_finite() && response_delay_seconds > 0.0 {
                projection_seconds += response_delay_seconds;
            }
            if projection_seconds.is_finite()
                && projection_seconds > 0.0
                && let Some(raw_position) = position.filter(|value| value.is_finite())
            {
                position = Some(raw_position + projection_seconds);
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
        let local_pause_mutation_intent = self
            .playback_coordination
            .active_local_pause_state_mutation_intent(&self.session);

        let outbound_state = if let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds_for_pause_mutation(
                now_seconds,
                dont_slow_down_with_me,
                local_pause_mutation_intent.is_some(),
            ),
            self.session.model.playback.local_paused,
        ) {
            self.session
                .reconcile_state_and_build_response_at_with_pause_mutation_intent(
                    StatePayload::new(),
                    local_position,
                    local_paused,
                    client_latency_calculation,
                    client_rtt,
                    now_seconds,
                    local_pause_mutation_intent,
                )
        } else {
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(client_latency_calculation)
                    .with_client_rtt(client_rtt),
            )
        };

        self.queue_connection_scoped_state_with_participant_status(
            outbound_state,
            true,
            now_seconds,
        )
    }

    /// Publishes only the additive participant-status heartbeat. Legacy
    /// canonical State synchronization remains independently gated until the
    /// server has established its ordinary State cadence.
    pub fn run_participant_status_heartbeat(&mut self, now_seconds: f64) -> bool {
        if !self.session.is_active() {
            return false;
        }
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let Some(pending) = self
            .playback_coordination
            .pending_participant_status_report(&self.session, true, now_seconds)
        else {
            return false;
        };
        self.control.activate_protocol_connection_generation();
        let queued = self.control.queue_connection_scoped_state(
            StatePayload::new().with_participant_status_v1(
                ParticipantStatusStateExtension::new().with_report(pending.report.clone()),
            ),
        );
        if queued {
            self.playback_coordination
                .commit_participant_status_report(&pending);
        }
        queued
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

    pub fn pending_protocol_line(&self) -> Result<Option<PendingProtocolLine>, ProtocolError> {
        self.control.front_outbound_message_line()
    }

    pub fn acknowledge_protocol_line(
        &mut self,
        lease: ProtocolLineLease,
    ) -> Option<ProtocolMessage> {
        self.control.acknowledge_outbound_message(lease)
    }

    pub fn release_protocol_line(&mut self, lease: ProtocolLineLease) -> bool {
        self.control.release_outbound_message(lease)
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
        mut send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        while let Some(pending) = self.pending_protocol_line()? {
            if let Err(error) = send_line(pending.line()) {
                let _ = self.release_protocol_line(pending.lease());
                return Err(error);
            }
            self.acknowledge_protocol_line(pending.lease())
                .expect("a delivered pending protocol line must remain acknowledgeable");
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
