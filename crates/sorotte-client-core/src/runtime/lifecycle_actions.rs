use super::*;
use crate::control::client_effect_player_error;

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn begin_protocol_connection_generation(&mut self) {
        self.control.begin_protocol_connection_generation();
        self.playback_coordination
            .begin_protocol_connection_generation(&self.session);
    }

    pub fn run_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let current_gate_holds_play = self.readiness_gate_holds_current_playback();
        let actions = self
            .session
            .runtime_actions_for_readiness_unpause_attempt_with_gate_hold(
                now_seconds,
                readiness_supported,
                local_can_control,
                is_playing_music,
                Some(current_gate_holds_play),
            );
        self.dispatch_runtime_actions_with_session_rollback_and_pause_cause(
            session_snapshot,
            &actions,
            PlayerCommandCause::ReadinessGateHold,
        )
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
        self.dispatch_runtime_actions_with_session_rollback_and_pause_cause(
            session_snapshot,
            &actions,
            PlayerCommandCause::AutomaticReadinessStart,
        )
    }

    pub fn run_reconnect_retry(&mut self, retries: u32) -> Result<(), PlayerError> {
        self.begin_protocol_connection_generation();
        let actions = self.session.runtime_actions_for_reconnect_retry(retries);
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_auth_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        self.run_controller_auth_notifications_if_needed_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn run_controller_auth_notifications_if_needed_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_auth_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.emit_pending_playback_barrier_request_at(now_seconds)
    }

    /// Emits a playback-coordination retry once its correlated server delay
    /// has elapsed. Calling this method before or repeatedly after the
    /// deadline is safe: an initiated attempt suppresses duplicate emission.
    pub fn run_pending_playback_barrier_retry_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        if self
            .playback_coordination
            .pending_playback_barrier_retry_delay_at(now_seconds)
            .is_none()
        {
            return Ok(());
        }
        self.emit_pending_playback_barrier_request_at(now_seconds)
    }

    /// Remaining delay for the current correlated playback retry.
    pub fn pending_playback_barrier_retry_delay_at(&self, now_seconds: f64) -> Option<f64> {
        self.playback_coordination
            .pending_playback_barrier_retry_delay_at(now_seconds)
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
        self.dispatch_runtime_actions_with_causal_tracking(&actions)
    }

    pub fn run_reconnect_state_restore_validation_if_needed(&mut self) -> Result<(), PlayerError> {
        self.run_reconnect_state_restore_validation_if_needed_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn run_reconnect_state_restore_validation_if_needed_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let validation_pending = self
            .session
            .model
            .reconnect
            .state_restore_validation_pending;
        if !validation_pending {
            self.playback_coordination.finish_reconnect_reconciliation();
            return Ok(());
        }
        if validation_pending {
            if self
                .player
                .capabilities()
                .contains(sorotte_player_api::PlayerCapability::Telemetry)
            {
                self.playback_coordination
                    .mark_transport_telemetry_available();
            }
            // Discover and drain rich transport telemetry before choosing a
            // correction owner. Once this runtime has seen generation-aware
            // telemetry, reconnect correction remains coordinator-owned even
            // during a later load attempt where no fresh sample has arrived
            // yet. This prevents a transient telemetry gap from re-enabling
            // unsafe direct seeks/unpauses.
            self.drain_player_transport_coordination(now_seconds)?;
            if self.playback_coordination.reconnect_coordinator_available() {
                let reconciliation_started =
                    self.playback_coordination.begin_reconnect_reconciliation();
                if reconciliation_started {
                    self.session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_actions_attempted = self
                        .session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_actions_attempted
                        .saturating_add(1);
                }

                // Beginning reconciliation forces a new desired revision. A
                // second drain applies that revision through the same tracked,
                // transport-safe coordinator command seam used during normal
                // synchronization.
                self.drain_player_transport_coordination(now_seconds)?;
                if self
                    .playback_coordination
                    .reconnect_reconciliation_complete()
                {
                    self.session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_actions_succeeded = self
                        .session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_actions_succeeded
                        .saturating_add(1);
                    self.session
                        .complete_reconnect_state_restore_validation_after_success();
                    self.playback_coordination.finish_reconnect_reconciliation();
                }
                return Ok(());
            }
        }

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
                    .model
                    .reconnect
                    .state_restore_correction_metrics
                    .correction_actions_attempted = self
                    .session
                    .model
                    .reconnect
                    .state_restore_correction_metrics
                    .correction_actions_attempted
                    .saturating_add(1);
            }

            if let Err(err) =
                self.dispatch_runtime_actions_with_causal_tracking(std::slice::from_ref(action))
            {
                if is_correction_action {
                    self.session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_action_failures = self
                        .session
                        .model
                        .reconnect
                        .state_restore_correction_metrics
                        .correction_action_failures
                        .saturating_add(1);
                    if let Some(notification) = self
                        .session
                        .defer_reconnect_state_restore_validation_after_correction_failure()
                    {
                        self.control
                            .emit(ClientEffect::NotifyReconnectTransition(notification))
                            .map_err(client_effect_player_error)?;
                    }
                    return Ok(());
                }
                return Err(err);
            }

            if is_correction_action {
                self.session
                    .model
                    .reconnect
                    .state_restore_correction_metrics
                    .correction_actions_succeeded = self
                    .session
                    .model
                    .reconnect
                    .state_restore_correction_metrics
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
        self.run_room_pause_sync_if_needed_at(unix_wall_clock_time_seconds_legacy_compatible())
    }

    pub fn run_room_pause_sync_if_needed_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.drain_player_transport_coordination(now_seconds)?;

        // Reconnect validation owns correction immediately after reconnect
        // state restore. Transport telemetry is deliberately drained first so
        // the coordinator can still observe unsafe loading/cache/seeking
        // phases while legacy room synchronization remains suspended.
        if self
            .session
            .model
            .reconnect
            .state_restore_validation_pending
        {
            return Ok(());
        }

        if self
            .playback_coordination_snapshot()
            .transport_telemetry_observed
        {
            return Ok(());
        }

        let Some(room_playstate) =
            self.current_room_playstate_legacy_ping_compatible_at(now_seconds)
        else {
            return Ok(());
        };
        if room_playstate.paused == Some(true) {
            self.session
                .model
                .playback
                .pending_cache_room_playstate_resync = false;
            self.session
                .model
                .playback
                .cache_recovery_observation_position = None;
            self.session
                .model
                .playback
                .cache_recovery_waiting_for_post_cache_position = false;
        }
        let room_seeked = room_playstate.do_seek == Some(true);
        let cache_pause_active = self.session.model.playback.local_paused_for_cache == Some(true);
        let cache_recovery_pending = self
            .session
            .model
            .playback
            .pending_cache_room_playstate_resync;
        let cache_blocks_room_unpause =
            (cache_pause_active || cache_recovery_pending) && room_playstate.paused == Some(false);
        if cache_blocks_room_unpause {
            self.session
                .model
                .playback
                .pending_cache_room_playstate_resync = true;
        }
        let pause_mismatch = room_playstate.paused.is_some_and(|room_paused| {
            self.session.model.playback.local_paused.unwrap_or(true) != room_paused
        });
        let pause_mismatch_actionable = pause_mismatch && !cache_blocks_room_unpause;
        if !room_seeked && !pause_mismatch_actionable {
            return Ok(());
        }
        let set_by_is_self = self
            .session
            .model
            .connection
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
        let target_paused = if pause_mismatch_actionable {
            room_playstate.paused
        } else {
            None
        };
        if target_position.is_none() && target_paused.is_none() {
            return Ok(());
        }

        let retain_desired_unpause_until_advancement = target_paused == Some(false);
        let result = self.run_model_event(ClientEvent::RoomPauseSyncRequested {
            original_position: self.session.model.playback.local_position,
            target_position,
            target_paused,
            clear_cache_resync_on_success: false,
        });
        if result.is_ok() && retain_desired_unpause_until_advancement {
            self.session
                .model
                .playback
                .pending_cache_room_playstate_resync = true;
            self.session
                .model
                .playback
                .cache_recovery_observation_position = self.session.model.playback.local_position;
            self.session
                .model
                .playback
                .cache_recovery_waiting_for_post_cache_position = false;
        }
        result
    }

    pub fn run_desync_correction_if_needed(
        &mut self,
        now_seconds: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.drain_player_transport_coordination(now_seconds)?;

        // Reconnect validation owns the correction window immediately after
        // reconnect restore, but transport observations must continue flowing
        // so coordinator-owned correction can make progress safely.
        if self
            .session
            .model
            .reconnect
            .state_restore_validation_pending
        {
            return Ok(());
        }

        let coordination = self.playback_coordination_snapshot();
        if coordination.transport_telemetry_observed && coordination.ordinary_correction_blocked {
            self.session.model.playback.behind_first_detected_at_seconds = None;
            return Ok(());
        }
        if self.session.model.playback.local_paused_for_cache == Some(true)
            || self
                .session
                .model
                .playback
                .pending_cache_room_playstate_resync
        {
            self.session.model.playback.behind_first_detected_at_seconds = None;
            return Ok(());
        }
        let Some(local_position) = self.session.model.playback.local_position else {
            return Ok(());
        };
        let local_position =
            self.desync_local_position_with_legacy_ping_forward_delay(local_position);
        let Some(room_playstate) = self.session.current_room_playstate_at(now_seconds) else {
            self.session.model.playback.behind_first_detected_at_seconds = None;
            return Ok(());
        };

        let session_snapshot = self.session.snapshot_local_action_state();
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
        match self.dispatch_runtime_actions_with_causal_tracking(&actions) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.session.restore_local_action_state(session_snapshot);
                Err(error)
            }
        }
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
        self.dispatch_runtime_actions_with_room_switch_coordination(&actions, true)
    }
}
