use super::*;

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientRuntimeControl,
{
    pub fn run_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_readiness_unpause_attempt(
            now_seconds,
            readiness_supported,
            local_can_control,
            is_playing_music,
        );
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn update_autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.session.autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
    }

    pub fn tick_autoplay(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.autoplay_countdown_tick(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn run_reconnect_retry(&mut self, retries: u32) -> Result<(), PlayerError> {
        let actions = self.session.runtime_actions_for_reconnect_retry(retries);
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_auth_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_auth_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controlled_room_creation_notifications_if_needed(
        &mut self,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controlled_room_creation_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_chat_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_chat_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_user_change_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_user_change_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_transition_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_transition_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_validation_if_needed(&mut self) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_validation_if_needed();
        if actions.is_empty() {
            return Ok(());
        }

        let mut attempted_correction = false;
        for action in &actions {
            let is_correction_action = matches!(
                action,
                ClientRuntimeAction::SetPaused(_) | ClientRuntimeAction::SetPosition(_)
            );
            attempted_correction |= is_correction_action;
            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted
                    .saturating_add(1);
            }

            if let Err(err) = ClientSession::dispatch_runtime_actions(
                std::slice::from_ref(action),
                &mut self.player,
                &mut self.control,
            ) {
                if is_correction_action {
                    self.session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures = self
                        .session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures
                        .saturating_add(1);
                    if let Some(notification) = self
                        .session
                        .defer_reconnect_state_restore_validation_after_correction_failure()
                    {
                        self.control.notify_reconnect_transition(notification);
                    }
                    return Ok(());
                }
                return Err(err);
            }

            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded
                    .saturating_add(1);
            }
            self.session
                .apply_successful_reconnect_state_restore_validation_action(action);
        }

        if attempted_correction {
            self.session
                .complete_reconnect_state_restore_validation_after_success();
        }

        Ok(())
    }

    pub fn run_room_pause_sync_if_needed(&mut self) -> Result<(), PlayerError> {
        // Reconnect validation owns correction immediately after reconnect state restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();

        let Some(room_playstate) = self.current_room_playstate_legacy_ping_compatible_now() else {
            return Ok(());
        };
        let room_seeked = room_playstate.do_seek == Some(true);
        let pause_mismatch = room_playstate
            .paused
            .is_some_and(|room_paused| self.session.local_paused.unwrap_or(true) != room_paused);
        if !room_seeked && !pause_mismatch {
            return Ok(());
        }
        let set_by_is_self = self
            .session
            .username
            .as_deref()
            .zip(room_playstate.set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);
        if set_by_is_self {
            return Ok(());
        }

        let target_position = if (room_seeked || room_playstate.paused == Some(true))
            && let Some(room_position) = room_playstate.position.filter(|value| value.is_finite())
        {
            Some(room_position)
        } else {
            None
        };
        let target_paused = if pause_mismatch {
            room_playstate.paused
        } else {
            None
        };
        if target_position.is_none() && target_paused.is_none() {
            return Ok(());
        }

        if let Some(room_position) = target_position {
            ClientSession::dispatch_runtime_actions(
                &[ClientRuntimeAction::SetPosition(room_position)],
                &mut self.player,
                &mut self.control,
            )?;
            self.session.local_position = Some(room_position);
        }
        if let Some(room_paused) = target_paused {
            ClientSession::dispatch_runtime_actions(
                &[ClientRuntimeAction::SetPaused(room_paused)],
                &mut self.player,
                &mut self.control,
            )?;
            // Mirror the confirmed local state to avoid duplicate correction attempts until telemetry catches up.
            self.session.local_paused = Some(room_paused);
        }
        Ok(())
    }

    pub fn run_desync_correction_if_needed(
        &mut self,
        now_seconds: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Result<(), PlayerError> {
        // Reconnect validation owns the correction window immediately after reconnect restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let Some(local_position) = self.session.local_position else {
            return Ok(());
        };
        let local_position =
            self.desync_local_position_with_legacy_ping_forward_delay(local_position);
        let Some(room_playstate) = self.session.current_room_playstate_at(now_seconds) else {
            self.session.behind_first_detected_at_seconds = None;
            return Ok(());
        };

        let actions = self
            .session
            .runtime_actions_for_desync_correction_against_room_playstate(
                room_playstate,
                now_seconds,
                local_position,
                local_can_control,
                dont_slow_down_with_me,
                speed_supported,
            );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub(crate) fn desync_local_position_with_legacy_ping_forward_delay(
        &self,
        local_position: f64,
    ) -> f64 {
        let Some(room_playstate) = self.session.current_room_playstate() else {
            return local_position;
        };
        if room_playstate.paused != Some(false) || room_playstate.do_seek == Some(true) {
            return local_position;
        }

        let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
        if !forward_delay.is_finite() || forward_delay <= 0.0 {
            return local_position;
        }

        // Compare against an estimate of "room position now" by moving local position back by
        // the inferred one-way/forward delay before evaluating threshold-based desync actions.
        local_position - forward_delay
    }

    pub fn run_reconnect_playlist_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_playlist_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_reidentify_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_reidentify_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }
}
