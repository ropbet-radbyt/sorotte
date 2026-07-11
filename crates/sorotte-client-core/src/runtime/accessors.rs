use super::*;
use crate::control::client_effect_player_error;
use sorotte_player_api::PlayerCommand;

pub struct ClientSessionUpdate<'a> {
    session: &'a mut ClientSession,
}

impl<'a> ClientSessionUpdate<'a> {
    pub fn new(session: &'a mut ClientSession) -> Self {
        Self { session }
    }

    pub fn apply_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) -> bool {
        self.session.apply_player_playback_telemetry_update(update)
    }

    pub fn initialize_local_identity(&mut self, username: String, room: String) {
        self.session.initialize_local_identity(username, room);
    }

    pub fn apply_protocol_message(
        &mut self,
        message: ProtocolMessage,
    ) -> Result<(), ProtocolError> {
        self.session.apply_protocol_message(message)
    }

    pub fn apply_protocol_message_at(
        &mut self,
        message: ProtocolMessage,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        self.session.apply_protocol_message_at(message, now_seconds)
    }

    pub fn apply_message_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        self.session.apply_message_json(json_line)
    }

    pub fn apply_message_json_at(
        &mut self,
        json_line: &str,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        self.session.apply_message_json_at(json_line, now_seconds)
    }

    pub fn mark_connecting(&mut self) {
        self.session.mark_connecting();
    }

    pub fn mark_awaiting_hello(&mut self) {
        self.session.mark_awaiting_hello();
    }

    pub fn mark_reconnecting(&mut self, attempt: u32) {
        self.session.mark_reconnecting(attempt);
    }

    pub fn mark_closing(&mut self) {
        self.session.mark_closing();
    }

    pub fn mark_disconnected(&mut self) {
        self.session.mark_disconnected();
    }

    pub fn reset_sync_state_for_reconnect(&mut self) {
        self.session.reset_sync_state_for_reconnect();
    }

    pub fn reconnect_policy_mut(&mut self) -> &mut ReconnectPolicyConfig {
        self.session.reconnect_policy_mut()
    }

    pub fn behavior_config_mut(&mut self) -> &mut SessionBehaviorConfig {
        self.session.behavior_config_mut()
    }

    pub fn desync_config_mut(&mut self) -> &mut DesyncCorrectionConfig {
        self.session.desync_config_mut()
    }

    pub fn readiness_autoplay_config_mut(&mut self) -> &mut ReadinessAutoplayConfig {
        self.session.readiness_autoplay_config_mut()
    }

    pub fn chat_config_mut(&mut self) -> &mut ChatConfig {
        self.session.chat_config_mut()
    }

    pub fn begin_local_playlist_index_reset_intent(
        &mut self,
        pause_before_sync: bool,
        now_seconds: f64,
    ) {
        self.session
            .begin_local_playlist_index_reset_intent(pause_before_sync, now_seconds);
    }

    pub fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        self.session.take_pending_playlist_index_reset_intent()
    }

    pub fn runtime_actions_for_desync_correction_against_room_playstate(
        &mut self,
        room_playstate: RoomPlaystateView,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.session
            .runtime_actions_for_desync_correction_against_room_playstate(
                room_playstate,
                now_seconds,
                local_position,
                local_can_control,
                dont_slow_down_with_me,
                speed_supported,
            )
    }

    pub fn set_autoplay_enabled(&mut self, enabled: bool) {
        self.session.set_autoplay_enabled(enabled);
    }

    pub fn set_media_match_peer_tiers(&mut self, tiers: BTreeMap<String, MediaMatchTier>) {
        self.session.set_media_match_peer_tiers(tiers);
    }

    pub fn remember_control_password_for_room(&mut self, room_name: &str, password: &str) {
        self.session
            .remember_control_password_for_room(room_name, password);
    }
}

pub struct ClientPlayerIo<'a, P> {
    player: &'a mut P,
}

impl<P: PlayerAdapter> ClientPlayerIo<'_, P> {
    pub fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        self.player
            .execute(PlayerCommand::OpenFile(path.to_owned()))
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.player.execute(PlayerCommand::SetPaused(paused))
    }

    pub fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
        self.player
            .execute(PlayerCommand::SetPosition(position_seconds))
    }

    pub fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.player.execute(PlayerCommand::SetPlaybackRate(rate))
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn new(session: ClientSession, player: P, control: C) -> Self {
        Self {
            session,
            player,
            control,
            ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible::default(),
            pending_player_playback_telemetry_updates: EffectOutbox::default(),
            last_local_file_update: None,
        }
    }

    pub(crate) fn finalize_local_playlist_index_switch_if_needed(
        &mut self,
        actions: &[ClientRuntimeAction],
    ) -> Result<(), PlayerError> {
        if !actions
            .iter()
            .any(|action| matches!(action, ClientRuntimeAction::SetPlaylistIndex { .. }))
        {
            return Ok(());
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.session
            .begin_local_playlist_index_reset_intent(true, now_seconds);
        self.session.apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(0.0),
        );
        self.control
            .emit(ClientEffect::SendState(StatePayload::new().with_playstate(
                PlaystatePayload::new().with_position(0.0).with_paused(true),
            )))
            .map_err(client_effect_player_error)
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

    pub(crate) fn run_model_event(&mut self, event: ClientEvent) -> Result<(), PlayerError> {
        let mut effects = std::collections::VecDeque::from(self.session.model.apply(event));
        let mut first_error = None;
        while let Some(effect) = effects.pop_front() {
            let result = self.execute_client_effect(effect.clone());
            let feedback = match result {
                Ok(()) => ClientEvent::EffectSucceeded(effect),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    ClientEvent::EffectFailed(effect)
                }
            };
            effects.extend(self.session.model.apply(feedback));
        }
        first_error.map_or(Ok(()), Err)
    }

    fn execute_client_effect(&mut self, effect: ClientEffect) -> Result<(), PlayerError> {
        match effect {
            ClientEffect::SetPlayerPaused(paused) => {
                self.player.execute(PlayerCommand::SetPaused(paused))
            }
            ClientEffect::SetPlayerPosition(position) => {
                self.player.execute(PlayerCommand::SetPosition(position))
            }
            ClientEffect::SetPlayerPlaybackRate(rate) => {
                self.player.execute(PlayerCommand::SetPlaybackRate(rate))
            }
            control_effect => self
                .control
                .emit(control_effect)
                .map_err(client_effect_player_error),
        }
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    #[cfg(test)]
    pub(crate) fn session_mut_for_test(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub fn session_mut(&mut self) -> ClientSessionUpdate<'_> {
        ClientSessionUpdate::new(&mut self.session)
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

    pub fn emit_effect(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        self.control.emit(effect)
    }

    pub fn player(&self) -> &P {
        &self.player
    }

    #[cfg(test)]
    pub(crate) fn player_mut_for_test(&mut self) -> &mut P {
        &mut self.player
    }

    pub fn player_mut(&mut self) -> ClientPlayerIo<'_, P> {
        ClientPlayerIo {
            player: &mut self.player,
        }
    }

    pub fn with_player_io<R>(&mut self, io: impl FnOnce(&mut P) -> R) -> R {
        io(&mut self.player)
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
