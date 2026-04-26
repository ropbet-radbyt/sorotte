use super::*;

impl ClientSession {
    pub(crate) fn snapshot_local_action_state(&self) -> ClientSessionLocalActionSnapshot {
        ClientSessionLocalActionSnapshot {
            user_views: self.user_views.clone(),
            local_position: self.local_position,
            local_paused: self.local_paused,
            last_seek_position_before_manual_seek: self.last_seek_position_before_manual_seek,
            last_paused_on_leave_at_seconds: self.last_paused_on_leave_at_seconds,
            last_rewound_at_seconds: self.last_rewound_at_seconds,
            autoplay_timer_running: self.autoplay_timer_running,
            autoplay_time_left_seconds: self.autoplay_time_left_seconds,
        }
    }

    pub(crate) fn restore_local_action_state(
        &mut self,
        snapshot: ClientSessionLocalActionSnapshot,
    ) {
        self.user_views = snapshot.user_views;
        self.local_position = snapshot.local_position;
        self.local_paused = snapshot.local_paused;
        self.last_seek_position_before_manual_seek = snapshot.last_seek_position_before_manual_seek;
        self.last_paused_on_leave_at_seconds = snapshot.last_paused_on_leave_at_seconds;
        self.last_rewound_at_seconds = snapshot.last_rewound_at_seconds;
        self.autoplay_timer_running = snapshot.autoplay_timer_running;
        self.autoplay_time_left_seconds = snapshot.autoplay_time_left_seconds;
    }

    pub fn apply_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) {
        if let Some(paused) = update.paused {
            self.local_paused = Some(paused);
        }
        if let Some(position_seconds) = update.position_seconds.filter(|value| value.is_finite()) {
            self.local_position = Some(position_seconds);
        }
    }

    pub(super) fn reset_playlist_index_transition_tracking(&mut self) {
        self.received_first_playlist_index = false;
        self.pending_playlist_index_reset_pause_before_sync = None;
        self.suppress_next_self_playlist_index_reset = false;
        self.playlist_active_targets_before_index_update.clear();
    }

    pub(super) fn note_recent_rewind(&mut self, now_seconds: f64) {
        if now_seconds.is_finite() {
            self.last_rewound_at_seconds = Some(now_seconds);
        }
    }

    pub(super) fn recently_rewound(&self, now_seconds: f64, threshold_seconds: f64) -> bool {
        if !threshold_seconds.is_finite() || threshold_seconds <= 0.0 {
            return false;
        }
        self.last_rewound_at_seconds
            .is_some_and(|last_rewound_at_seconds| {
                let elapsed = now_seconds - last_rewound_at_seconds;
                elapsed >= 0.0 && elapsed < threshold_seconds
            })
    }

    pub(super) fn queue_playlist_index_reset_intent(&mut self, pause_before_sync: bool) {
        self.pending_playlist_index_reset_pause_before_sync = Some(
            self.pending_playlist_index_reset_pause_before_sync
                .unwrap_or(false)
                || pause_before_sync,
        );
    }

    pub fn begin_local_playlist_index_reset_intent(
        &mut self,
        pause_before_sync: bool,
        now_seconds: f64,
    ) {
        self.received_first_playlist_index = true;
        self.queue_playlist_index_reset_intent(pause_before_sync);
        self.suppress_next_self_playlist_index_reset = true;
        self.last_advanced_at_seconds = Some(now_seconds);
        self.note_recent_rewind(now_seconds);
    }

    pub fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        let pending_reset = self.pending_playlist_index_reset_pause_before_sync.take();
        if pending_reset.is_some() {
            self.note_recent_rewind(unix_wall_clock_time_seconds_legacy_compatible());
        }
        pending_reset
    }

    pub fn has_pending_playlist_index_reset_intent(&self) -> bool {
        self.pending_playlist_index_reset_pause_before_sync
            .is_some()
    }

    pub(super) fn clear_reconnect_state_restore_validation_state(&mut self) {
        self.reconnect_state_restore_validation_pending = false;
        self.reconnect_state_restore_validation_retry_attempts = 0;
        self.reconnect_state_restore_validation_retry_cooldown_ticks = 0;
        self.reconnect_state_restore_validation_mismatch_notified = false;
        self.reconnect_state_restore_validation_mismatch_seen_in_cycle = false;
        self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = false;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
    }

    pub(super) fn reconnect_state_restore_correction_policy_mode(
        &self,
    ) -> ReconnectStateRestoreCorrectionPolicyMode {
        self.behavior_config
            .reconnect_state_restore_correction_policy_mode_override
            .unwrap_or(
                if self.behavior_config.reconnect_state_restore_auto_correct {
                    ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect
                } else {
                    ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
                },
            )
    }

    pub(super) fn reconnect_state_restore_position_tolerance_seconds_effective(&self) -> f64 {
        let position_tolerance_seconds = self
            .behavior_config
            .reconnect_state_restore_position_tolerance_seconds;
        if position_tolerance_seconds.is_finite() && position_tolerance_seconds >= 0.0 {
            position_tolerance_seconds
        } else {
            SEEK_THRESHOLD_SECONDS
        }
    }

    pub(super) fn reconnect_state_restore_correction_retry_cooldown_for_failed_attempt(
        &self,
        failed_attempts: u32,
    ) -> u32 {
        let base_cooldown_ticks = self
            .behavior_config
            .reconnect_state_restore_correction_retry_cooldown_ticks;
        if base_cooldown_ticks == 0 {
            return base_cooldown_ticks;
        }
        let use_exponential_backoff = self
            .behavior_config
            .reconnect_state_restore_correction_retry_exponential_backoff;
        let adaptive_cycle_backoff_shift = if self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_backoff
        {
            self.reconnect_state_restore_correction_consecutive_retry_exhaustions
        } else {
            0
        };
        if !use_exponential_backoff && adaptive_cycle_backoff_shift == 0 {
            return base_cooldown_ticks;
        }

        let max_cooldown_ticks = self
            .behavior_config
            .reconnect_state_restore_correction_retry_max_cooldown_ticks
            .max(base_cooldown_ticks);
        let per_attempt_shift = if use_exponential_backoff {
            failed_attempts.saturating_sub(1)
        } else {
            0
        };
        let shift = per_attempt_shift
            .saturating_add(adaptive_cycle_backoff_shift)
            .min(63);
        let multiplier = 1_u64 << shift;
        let scaled_cooldown_ticks = u64::from(base_cooldown_ticks).saturating_mul(multiplier);
        scaled_cooldown_ticks.min(u64::from(max_cooldown_ticks)) as u32
    }

    pub(super) fn reconnect_state_restore_correction_effective_retry_max_attempts(&self) -> u32 {
        let configured_max_attempts = self
            .behavior_config
            .reconnect_state_restore_correction_retry_max_attempts;
        if !self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget
        {
            return configured_max_attempts;
        }

        let min_attempts = self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts
            .min(configured_max_attempts);
        configured_max_attempts
            .saturating_sub(self.reconnect_state_restore_correction_consecutive_retry_exhaustions)
            .max(min_attempts)
    }

    pub(super) fn note_reconnect_state_restore_correction_retry_exhaustion(&mut self) {
        self.reconnect_state_restore_correction_consecutive_retry_exhaustions = self
            .reconnect_state_restore_correction_consecutive_retry_exhaustions
            .saturating_add(1);
    }

    pub(super) fn reset_reconnect_state_restore_correction_retry_exhaustions(&mut self) {
        self.reconnect_state_restore_correction_consecutive_retry_exhaustions = 0;
    }

    pub(super) fn activate_reconnect_state_restore_correction_recovery_cooldown_if_configured(
        &mut self,
    ) -> bool {
        let recovery_cooldown_reconnect_cycles = self
            .behavior_config
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles;
        if recovery_cooldown_reconnect_cycles == 0 {
            return false;
        }
        self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining =
            recovery_cooldown_reconnect_cycles;
        self.reconnect_state_restore_correction_recovery_reenable_notification_pending = true;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
        true
    }

    pub(super) fn begin_reconnect_state_restore_validation_cycle(&mut self) {
        self.reconnect_state_restore_correction_metrics
            .validation_cycles_started = self
            .reconnect_state_restore_correction_metrics
            .validation_cycles_started
            .saturating_add(1);
        self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = false;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
        if self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining > 0
        {
            self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = self
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining
                .saturating_sub(1);
            self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = true;
            return;
        }
        if self.reconnect_state_restore_correction_recovery_reenable_notification_pending {
            self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = true;
            self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
        }
    }

    pub(crate) fn defer_reconnect_state_restore_validation_after_correction_failure(
        &mut self,
    ) -> Option<ReconnectTransitionNotification> {
        if !self.reconnect_state_restore_validation_pending {
            return None;
        }

        let retry_max_attempts =
            self.reconnect_state_restore_correction_effective_retry_max_attempts();
        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        self.reconnect_state_restore_validation_retry_attempts = self
            .reconnect_state_restore_validation_retry_attempts
            .saturating_add(1);
        let failed_attempts = self.reconnect_state_restore_validation_retry_attempts;
        if failed_attempts > retry_max_attempts {
            self.note_reconnect_state_restore_correction_retry_exhaustion();
            self.reconnect_state_restore_correction_metrics
                .correction_retry_exhaustions = self
                .reconnect_state_restore_correction_metrics
                .correction_retry_exhaustions
                .saturating_add(1);
            self.activate_reconnect_state_restore_correction_recovery_cooldown_if_configured();
            self.clear_reconnect_state_restore_validation_state();
            return Some(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: failed_attempts,
                    max_attempts: retry_max_attempts,
                },
            );
        }

        let cooldown_ticks = self
            .reconnect_state_restore_correction_retry_cooldown_for_failed_attempt(failed_attempts);
        self.reconnect_state_restore_validation_retry_cooldown_ticks = cooldown_ticks;
        self.reconnect_state_restore_correction_metrics
            .correction_retries_scheduled = self
            .reconnect_state_restore_correction_metrics
            .correction_retries_scheduled
            .saturating_add(1);
        if matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion
        ) {
            return None;
        }
        Some(
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: failed_attempts,
                max_attempts: retry_max_attempts,
                cooldown_ticks,
            },
        )
    }

    pub(crate) fn complete_reconnect_state_restore_validation_after_success(&mut self) {
        self.reconnect_state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction = self
            .reconnect_state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction
            .saturating_add(1);
        if self.reconnect_state_restore_validation_mismatch_seen_in_cycle {
            let decay = self
                .behavior_config
                .reconnect_state_restore_correction_disable_after_mismatch_decay_on_success;
            if decay > 0 {
                self.reconnect_state_restore_correction_consecutive_mismatch_cycles = self
                    .reconnect_state_restore_correction_consecutive_mismatch_cycles
                    .saturating_sub(decay);
            }
        }
        self.reset_reconnect_state_restore_correction_retry_exhaustions();
        self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 0;
        self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
        self.clear_reconnect_state_restore_validation_state();
    }

    pub(crate) fn apply_successful_reconnect_state_restore_validation_action(
        &mut self,
        action: &ClientRuntimeAction,
    ) {
        match action {
            ClientRuntimeAction::SetPaused(paused) => {
                self.local_paused = Some(*paused);
            }
            ClientRuntimeAction::SetPosition(position_seconds) if position_seconds.is_finite() => {
                self.local_position = Some(*position_seconds);
            }
            _ => {}
        }
    }

    pub fn dispatch_runtime_actions<P, C>(
        actions: &[ClientRuntimeAction],
        player: &mut P,
        control: &mut C,
    ) -> Result<(), PlayerError>
    where
        P: PlayerAdapter,
        C: ClientRuntimeControl,
    {
        for action in actions {
            match action {
                ClientRuntimeAction::SetPaused(paused) => {
                    player.set_paused(*paused)?;
                }
                ClientRuntimeAction::RequestUserList => {
                    control.request_user_list();
                }
                ClientRuntimeAction::SetRoom { room } => {
                    control.set_room(room.clone());
                }
                ClientRuntimeAction::SetReady {
                    ready,
                    manually_initiated,
                } => {
                    control.set_ready(*ready, *manually_initiated);
                }
                ClientRuntimeAction::SetReadyForUser {
                    ready,
                    manually_initiated,
                    username,
                } => {
                    control.set_ready_for_user(*ready, *manually_initiated, username.clone());
                }
                ClientRuntimeAction::SetFile { file_payload } => {
                    control.set_file(file_payload.clone());
                }
                ClientRuntimeAction::SetPlaylist { files } => {
                    control.set_playlist(files.clone());
                }
                ClientRuntimeAction::SetPlaylistIndex { index } => {
                    control.set_playlist_index(*index);
                }
                ClientRuntimeAction::RequestControllerAuth { room, password } => {
                    control.request_controller_auth(room.clone(), password.clone());
                }
                ClientRuntimeAction::SendChat { message } => {
                    control.send_chat(message.clone());
                }
                ClientRuntimeAction::NotifyChat(notification) => {
                    control.notify_chat(notification.clone());
                }
                ClientRuntimeAction::NotifyControlledRoomCreation(notification) => {
                    control.notify_controlled_room_creation(notification.clone());
                }
                ClientRuntimeAction::NotifyControllerAuthTransition(notification) => {
                    control.notify_controller_auth_transition(notification.clone());
                }
                ClientRuntimeAction::NotifyUserChange(notification) => {
                    control.notify_user_change(notification.clone());
                }
                ClientRuntimeAction::NotifyReconnectTransition(notification) => {
                    control.notify_reconnect_transition(notification.clone());
                }
                ClientRuntimeAction::NotifyAutoplayCountdown(notification) => {
                    control.notify_autoplay_countdown(notification.clone());
                }
                ClientRuntimeAction::SetPosition(position) => {
                    player.set_position(*position)?;
                }
                ClientRuntimeAction::SetPlaybackRate(rate) => {
                    player.set_playback_rate(*rate)?;
                }
                ClientRuntimeAction::ScheduleReconnect { delay_seconds } => {
                    control.schedule_reconnect(*delay_seconds);
                }
                ClientRuntimeAction::StopReconnect => {
                    control.stop_reconnect();
                }
            }
        }
        Ok(())
    }

    pub fn apply_hello_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        let message = decode_message_line(json_line)?;
        let hello = extract_hello_from_message(message)?;
        self.apply_hello(hello);
        Ok(())
    }

    pub fn apply_protocol_message(
        &mut self,
        message: ProtocolMessage,
    ) -> Result<(), ProtocolError> {
        self.apply_protocol_message_with_now(message, None)
    }

    pub fn apply_protocol_message_at(
        &mut self,
        message: ProtocolMessage,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        self.apply_protocol_message_with_now(message, Some(now_seconds))
    }

    pub(super) fn apply_protocol_message_with_now(
        &mut self,
        message: ProtocolMessage,
        now_seconds: Option<f64>,
    ) -> Result<(), ProtocolError> {
        match message {
            ProtocolMessage::Hello(hello_message) => self.apply_hello(hello_message.hello),
            ProtocolMessage::Set(set_message) => self.apply_set(set_message.set, now_seconds),
            ProtocolMessage::List(list_message) => self.apply_list(list_message.list),
            ProtocolMessage::State(state_message) => {
                self.apply_state_at(state_message.state, now_seconds)
            }
            ProtocolMessage::Chat(chat_message) => self.apply_chat(chat_message.chat),
            ProtocolMessage::Error(error_message) => {
                return Err(ProtocolError::ServerError {
                    message: error_message.error.message,
                });
            }
            ProtocolMessage::Tls(tls_message) => {
                return Err(ProtocolError::UnexpectedTlsMessage {
                    start_tls: tls_message.tls.start_tls,
                });
            }
        }
        Ok(())
    }

    pub fn apply_message_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        for message in decode_message_lines(json_line)? {
            self.apply_protocol_message(message)?;
        }
        Ok(())
    }

    pub fn apply_message_json_at(
        &mut self,
        json_line: &str,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        for message in decode_message_lines(json_line)? {
            self.apply_protocol_message_at(message, now_seconds)?;
        }
        Ok(())
    }
}
