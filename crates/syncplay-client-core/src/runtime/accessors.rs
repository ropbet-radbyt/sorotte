use super::*;

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientRuntimeControl,
{
    pub fn new(session: ClientSession, player: P, control: C) -> Self {
        Self {
            session,
            player,
            control,
            ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible::default(),
            pending_player_playback_telemetry_updates: Vec::new(),
            last_local_file_update: None,
        }
    }

    pub(crate) fn finalize_local_playlist_index_switch_if_needed(
        &mut self,
        actions: &[ClientRuntimeAction],
    ) {
        if !actions
            .iter()
            .any(|action| matches!(action, ClientRuntimeAction::SetPlaylistIndex { .. }))
        {
            return;
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.session
            .begin_local_playlist_index_reset_intent(true, now_seconds);
        self.session.apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(0.0),
        );
        self.control.send_state(
            StatePayload::new()
                .with_playstate(PlaystatePayload::new().with_position(0.0).with_paused(true)),
        );
    }

    pub(crate) fn dispatch_runtime_actions_with_session_rollback(
        &mut self,
        session_snapshot: ClientSessionLocalActionSnapshot,
        actions: &[ClientRuntimeAction],
    ) -> Result<(), PlayerError> {
        match ClientSession::dispatch_runtime_actions(actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(()),
            Err(err) => {
                self.session.restore_local_action_state(session_snapshot);
                Err(err)
            }
        }
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        self.session.reconnect_state_restore_correction_metrics()
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        self.session
            .reconnect_state_restore_correction_state_snapshot()
    }

    pub fn control(&self) -> &C {
        &self.control
    }

    pub fn control_mut(&mut self) -> &mut C {
        &mut self.control
    }

    pub fn player(&self) -> &P {
        &self.player
    }

    pub fn player_mut(&mut self) -> &mut P {
        &mut self.player
    }

    pub fn last_local_file_update(&self) -> Option<&LocalFileUpdate> {
        self.last_local_file_update.as_ref()
    }

    pub fn current_room_playstate_legacy_ping_compatible_at(
        &self,
        now_seconds: f64,
    ) -> Option<RoomPlaystateView> {
        let mut room_playstate = self.session.current_room_playstate_at(now_seconds)?;
        if room_playstate.paused == Some(false)
            && let Some(position) = room_playstate.position
        {
            let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
            if forward_delay.is_finite() && forward_delay > 0.0 {
                room_playstate.position = Some(position + forward_delay);
            }
        }
        Some(room_playstate)
    }

    pub fn current_room_playstate_legacy_ping_compatible_now(&self) -> Option<RoomPlaystateView> {
        self.current_room_playstate_legacy_ping_compatible_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn into_parts(self) -> (ClientSession, P, C) {
        (self.session, self.player, self.control)
    }
}
