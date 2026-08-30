use super::*;
use crate::control::client_effect_player_error;
use sorotte_player_api::PlayerCommand;

const CACHE_RECOVERY_MIN_OBSERVED_ADVANCEMENT_SECONDS: f64 = 0.01;

impl ClientSession {
    pub(crate) fn desync_correction_dispatch_snapshot(&self) -> DesyncCorrectionDispatchSnapshot {
        DesyncCorrectionDispatchSnapshot {
            speed_changed: self.model.playback.speed_changed,
            speed_correction_rate: self.model.playback.speed_correction_rate,
            local_playback_rate: self.model.playback.local_playback_rate,
        }
    }

    pub(crate) fn restore_desync_correction_dispatch_snapshot(
        &mut self,
        snapshot: DesyncCorrectionDispatchSnapshot,
    ) {
        self.model.playback.speed_changed = snapshot.speed_changed;
        self.model.playback.speed_correction_rate = snapshot.speed_correction_rate;
        self.model.playback.local_playback_rate = snapshot.local_playback_rate;
    }

    pub(crate) fn snapshot_local_action_state(&self) -> ClientSessionLocalActionSnapshot {
        ClientSessionLocalActionSnapshot {
            user_views: self.model.room.users.clone(),
            media_match_peer_tiers: self.model.room.media_match_peer_tiers.clone(),
            local_position: self.model.playback.local_position,
            local_paused: self.model.playback.local_paused,
            local_playback_rate: self.model.playback.local_playback_rate,
            speed_changed: self.model.playback.speed_changed,
            speed_correction_rate: self.model.playback.speed_correction_rate,
            local_paused_for_cache: self.model.playback.local_paused_for_cache,
            local_cache_buffering_percent: self.model.playback.local_cache_buffering_percent,
            pending_cache_room_playstate_resync: self
                .model
                .playback
                .pending_cache_room_playstate_resync,
            cache_recovery_observation_position: self
                .model
                .playback
                .cache_recovery_observation_position,
            cache_recovery_waiting_for_post_cache_position: self
                .model
                .playback
                .cache_recovery_waiting_for_post_cache_position,
            last_seek_position_before_manual_seek: self
                .model
                .playlist
                .last_seek_position_before_manual_seek,
            last_paused_on_leave_at_seconds: self.model.playback.last_paused_on_leave_at_seconds,
            last_rewound_at_seconds: self.model.playback.last_rewound_at_seconds,
            autoplay_timer_running: self.model.readiness.autoplay_timer_running,
            autoplay_time_left_seconds: self.model.readiness.autoplay_time_left_seconds,
        }
    }

    pub(crate) fn restore_local_action_state(
        &mut self,
        snapshot: ClientSessionLocalActionSnapshot,
    ) {
        self.model.room.users = snapshot.user_views;
        self.model.room.media_match_peer_tiers = snapshot.media_match_peer_tiers;
        self.model.playback.local_position = snapshot.local_position;
        self.model.playback.local_paused = snapshot.local_paused;
        self.model.playback.local_playback_rate = snapshot.local_playback_rate;
        self.model.playback.speed_changed = snapshot.speed_changed;
        self.model.playback.speed_correction_rate = snapshot.speed_correction_rate;
        self.model.playback.local_paused_for_cache = snapshot.local_paused_for_cache;
        self.model.playback.local_cache_buffering_percent = snapshot.local_cache_buffering_percent;
        self.model.playback.pending_cache_room_playstate_resync =
            snapshot.pending_cache_room_playstate_resync;
        self.model.playback.cache_recovery_observation_position =
            snapshot.cache_recovery_observation_position;
        self.model
            .playback
            .cache_recovery_waiting_for_post_cache_position =
            snapshot.cache_recovery_waiting_for_post_cache_position;
        self.model.playlist.last_seek_position_before_manual_seek =
            snapshot.last_seek_position_before_manual_seek;
        self.model.playback.last_paused_on_leave_at_seconds =
            snapshot.last_paused_on_leave_at_seconds;
        self.model.playback.last_rewound_at_seconds = snapshot.last_rewound_at_seconds;
        self.model.readiness.autoplay_timer_running = snapshot.autoplay_timer_running;
        self.model.readiness.autoplay_time_left_seconds = snapshot.autoplay_time_left_seconds;
    }

    pub(crate) fn replace_player_playback_telemetry_from_authoritative_snapshot(
        &mut self,
        snapshot: &PlayerPlaybackTelemetryUpdate,
    ) -> bool {
        let local_position = snapshot
            .position_seconds
            .filter(|value| value.is_finite() && *value >= 0.0);
        let local_playback_rate = snapshot
            .playback_rate
            .filter(|value| value.is_finite() && *value > 0.0);
        let local_cache_buffering_percent = snapshot
            .cache_buffering_percent
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
        let cache_pause_active = snapshot.paused_for_cache == Some(true);
        let pending_cache_room_playstate_resync = cache_pause_active;
        let cache_recovery_observation_position =
            cache_pause_active.then_some(local_position).flatten();
        let cache_recovery_waiting_for_post_cache_position = false;

        let changed = self.model.playback.local_position != local_position
            || self.model.playback.local_paused != snapshot.paused
            || self.model.playback.local_playback_rate != local_playback_rate
            || self.model.playback.local_paused_for_cache != snapshot.paused_for_cache
            || self.model.playback.local_cache_buffering_percent != local_cache_buffering_percent
            || self.model.playback.pending_cache_room_playstate_resync
                != pending_cache_room_playstate_resync
            || self.model.playback.cache_recovery_observation_position
                != cache_recovery_observation_position
            || self
                .model
                .playback
                .cache_recovery_waiting_for_post_cache_position
                != cache_recovery_waiting_for_post_cache_position;

        self.model.playback.local_position = local_position;
        self.model.playback.local_paused = snapshot.paused;
        self.model.playback.local_playback_rate = local_playback_rate;
        self.model.playback.local_paused_for_cache = snapshot.paused_for_cache;
        self.model.playback.local_cache_buffering_percent = local_cache_buffering_percent;
        self.model.playback.pending_cache_room_playstate_resync =
            pending_cache_room_playstate_resync;
        self.model.playback.cache_recovery_observation_position =
            cache_recovery_observation_position;
        self.model
            .playback
            .cache_recovery_waiting_for_post_cache_position =
            cache_recovery_waiting_for_post_cache_position;
        changed
    }

    pub fn apply_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) -> bool {
        let mut changed = false;
        let observed_position = update
            .position_seconds
            .filter(|value| value.is_finite() && *value >= 0.0);
        let cache_pause_was_active = self.model.playback.local_paused_for_cache == Some(true);
        if let Some(paused_for_cache) = update.paused_for_cache {
            if self.model.playback.local_paused_for_cache != Some(paused_for_cache) {
                self.model.playback.local_paused_for_cache = Some(paused_for_cache);
                changed = true;
            }
            if paused_for_cache {
                if !self.model.playback.pending_cache_room_playstate_resync {
                    self.model.playback.pending_cache_room_playstate_resync = true;
                    changed = true;
                }
                let recovery_position = observed_position.or(self.model.playback.local_position);
                if self.model.playback.cache_recovery_observation_position != recovery_position {
                    self.model.playback.cache_recovery_observation_position = recovery_position;
                    changed = true;
                }
                if self
                    .model
                    .playback
                    .cache_recovery_waiting_for_post_cache_position
                {
                    self.model
                        .playback
                        .cache_recovery_waiting_for_post_cache_position = false;
                    changed = true;
                }
            } else if cache_pause_was_active
                && self.model.playback.pending_cache_room_playstate_resync
                && !self
                    .model
                    .playback
                    .cache_recovery_waiting_for_post_cache_position
            {
                self.model
                    .playback
                    .cache_recovery_waiting_for_post_cache_position = true;
                changed = true;
            }
        }
        if let Some(cache_buffering_percent) = update
            .cache_buffering_percent
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
            && self.model.playback.local_cache_buffering_percent != Some(cache_buffering_percent)
        {
            self.model.playback.local_cache_buffering_percent = Some(cache_buffering_percent);
            changed = true;
        }
        let cache_pause_active = self.model.playback.local_paused_for_cache == Some(true);
        if let Some(paused) = update.paused
            && !cache_pause_active
            && self.model.playback.local_paused != Some(paused)
        {
            self.model.playback.local_paused = Some(paused);
            changed = true;
        }
        if let Some(position_seconds) = observed_position
            && self.model.playback.local_position != Some(position_seconds)
        {
            self.model.playback.local_position = Some(position_seconds);
            changed = true;
        }
        if self.model.playback.pending_cache_room_playstate_resync
            && !cache_pause_active
            && let Some(position_seconds) = observed_position
        {
            if self
                .model
                .playback
                .cache_recovery_waiting_for_post_cache_position
            {
                self.model.playback.cache_recovery_observation_position = Some(position_seconds);
                self.model
                    .playback
                    .cache_recovery_waiting_for_post_cache_position = false;
                changed = true;
            } else if self.model.playback.local_paused == Some(false)
                && self
                    .model
                    .playback
                    .cache_recovery_observation_position
                    .is_some_and(|baseline_position| {
                        position_seconds - baseline_position
                            > CACHE_RECOVERY_MIN_OBSERVED_ADVANCEMENT_SECONDS
                    })
            {
                self.model.playback.pending_cache_room_playstate_resync = false;
                self.model.playback.cache_recovery_observation_position = None;
                changed = true;
            } else if self
                .model
                .playback
                .cache_recovery_observation_position
                .is_none()
            {
                self.model.playback.cache_recovery_observation_position = Some(position_seconds);
                changed = true;
            }
        }
        if let Some(playback_rate) = update
            .playback_rate
            .filter(|value| value.is_finite() && *value > 0.0)
            && self.model.playback.local_playback_rate != Some(playback_rate)
        {
            self.model.playback.local_playback_rate = Some(playback_rate);
            changed = true;
        }
        changed
    }

    /// Applies an ordered logical-pause classification, which is already
    /// distinct from mpv's physical cache-induced pause state.
    pub(crate) fn apply_ordered_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) -> bool {
        let logical_pause = update.paused;
        let mut changed = self.apply_player_playback_telemetry_update(update);
        if let Some(logical_pause) = logical_pause
            && self.model.playback.local_paused != Some(logical_pause)
        {
            self.model.playback.local_paused = Some(logical_pause);
            changed = true;
        }
        changed
    }

    pub fn initialize_local_identity(&mut self, username: String, room: String) {
        self.model.connection.username = Some(username.clone());
        self.update_local_room(room.clone());
        self.set_user_room(&username, Some(room));
        self.set_user_ready(&username, false);
    }

    pub(super) fn reset_playlist_index_transition_tracking(&mut self) {
        self.model.playlist.received_first_index = false;
        self.model.playlist.pending_index_reset_pause_before_sync = None;
        self.model
            .playlist
            .pending_index_reset_refresh_recently_advanced = false;
        self.model.playlist.suppress_next_self_index_reset = false;
        self.model
            .playlist
            .active_targets_before_index_update
            .clear();
    }

    pub(super) fn note_recent_rewind(&mut self, now_seconds: f64) {
        if now_seconds.is_finite() {
            self.model.playback.last_rewound_at_seconds = Some(now_seconds);
        }
    }

    pub(super) fn recently_rewound(&self, now_seconds: f64, threshold_seconds: f64) -> bool {
        if !threshold_seconds.is_finite() || threshold_seconds <= 0.0 {
            return false;
        }
        self.model
            .playback
            .last_rewound_at_seconds
            .is_some_and(|last_rewound_at_seconds| {
                let elapsed = now_seconds - last_rewound_at_seconds;
                elapsed >= 0.0 && elapsed < threshold_seconds
            })
    }

    pub(super) fn queue_playlist_index_reset_intent(&mut self, pause_before_sync: bool) {
        self.model.playlist.pending_index_reset_pause_before_sync = Some(
            self.model
                .playlist
                .pending_index_reset_pause_before_sync
                .unwrap_or(false)
                || pause_before_sync,
        );
    }

    pub fn begin_local_playlist_index_reset_intent(
        &mut self,
        pause_before_sync: bool,
        now_seconds: f64,
    ) {
        self.model.playlist.received_first_index = true;
        self.queue_playlist_index_reset_intent(pause_before_sync);
        self.model.playlist.suppress_next_self_index_reset = true;
        self.model.playback.last_advanced_at_seconds = Some(now_seconds);
        self.model
            .playlist
            .pending_index_reset_refresh_recently_advanced = true;
        self.note_recent_rewind(now_seconds);
    }

    pub fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        self.take_pending_playlist_index_reset_intent_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub(crate) fn take_pending_playlist_index_reset_intent_at(
        &mut self,
        now_seconds: f64,
    ) -> Option<bool> {
        let pending_reset = self
            .model
            .playlist
            .pending_index_reset_pause_before_sync
            .take();
        if pending_reset.is_some() {
            if self
                .model
                .playlist
                .pending_index_reset_refresh_recently_advanced
                && now_seconds.is_finite()
            {
                self.model.playback.last_advanced_at_seconds = Some(now_seconds);
            }
            self.model
                .playlist
                .pending_index_reset_refresh_recently_advanced = false;
            self.note_recent_rewind(now_seconds);
        }
        pending_reset
    }

    pub fn has_pending_playlist_index_reset_intent(&self) -> bool {
        self.model
            .playlist
            .pending_index_reset_pause_before_sync
            .is_some()
    }

    /// Returns the reset still owed to the physical player without consuming it.
    ///
    /// Player owners use this to wait until a newly-selected playlist item is
    /// actually available before committing the pause-and-rewind side effect.
    pub fn pending_playlist_index_reset_intent(&self) -> Option<bool> {
        self.model.playlist.pending_index_reset_pause_before_sync
    }

    pub(super) fn clear_reconnect_state_restore_validation_state(&mut self) {
        self.model.reconnect.state_restore_validation_pending = false;
        self.model.reconnect.state_restore_validation_retry_attempts = 0;
        self.model
            .reconnect
            .state_restore_validation_retry_cooldown_ticks = 0;
        self.model
            .reconnect
            .state_restore_validation_mismatch_notified = false;
        self.model
            .reconnect
            .state_restore_validation_mismatch_seen_in_cycle = false;
        self.model
            .reconnect
            .state_restore_correction_recovery_suppressed_this_cycle = false;
        self.model
            .reconnect
            .state_restore_correction_recovery_reenabled_this_cycle = false;
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
            self.model
                .reconnect
                .state_restore_correction_consecutive_retry_exhaustions
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
            .saturating_sub(
                self.model
                    .reconnect
                    .state_restore_correction_consecutive_retry_exhaustions,
            )
            .max(min_attempts)
    }

    pub(super) fn note_reconnect_state_restore_correction_retry_exhaustion(&mut self) {
        self.model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions = self
            .model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions
            .saturating_add(1);
    }

    pub(super) fn reset_reconnect_state_restore_correction_retry_exhaustions(&mut self) {
        self.model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions = 0;
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
        self.model
            .reconnect
            .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining =
            recovery_cooldown_reconnect_cycles;
        self.model
            .reconnect
            .state_restore_correction_recovery_reenable_notification_pending = true;
        self.model
            .reconnect
            .state_restore_correction_recovery_reenabled_this_cycle = false;
        true
    }

    pub(super) fn begin_reconnect_state_restore_validation_cycle(&mut self) {
        self.model
            .reconnect
            .state_restore_correction_metrics
            .validation_cycles_started = self
            .model
            .reconnect
            .state_restore_correction_metrics
            .validation_cycles_started
            .saturating_add(1);
        self.model
            .reconnect
            .state_restore_correction_recovery_suppressed_this_cycle = false;
        self.model
            .reconnect
            .state_restore_correction_recovery_reenabled_this_cycle = false;
        if self
            .model
            .reconnect
            .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining
            > 0
        {
            self.model
                .reconnect
                .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = self
                .model
                .reconnect
                .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining
                .saturating_sub(1);
            self.model
                .reconnect
                .state_restore_correction_recovery_suppressed_this_cycle = true;
            return;
        }
        if self
            .model
            .reconnect
            .state_restore_correction_recovery_reenable_notification_pending
        {
            self.model
                .reconnect
                .state_restore_correction_recovery_reenabled_this_cycle = true;
            self.model
                .reconnect
                .state_restore_correction_recovery_reenable_notification_pending = false;
        }
    }

    pub(crate) fn defer_reconnect_state_restore_validation_after_correction_failure(
        &mut self,
    ) -> Option<ReconnectTransitionNotification> {
        if !self.model.reconnect.state_restore_validation_pending {
            return None;
        }

        let retry_max_attempts =
            self.reconnect_state_restore_correction_effective_retry_max_attempts();
        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        self.model.reconnect.state_restore_validation_retry_attempts = self
            .model
            .reconnect
            .state_restore_validation_retry_attempts
            .saturating_add(1);
        let failed_attempts = self.model.reconnect.state_restore_validation_retry_attempts;
        if failed_attempts > retry_max_attempts {
            self.note_reconnect_state_restore_correction_retry_exhaustion();
            self.model
                .reconnect
                .state_restore_correction_metrics
                .correction_retry_exhaustions = self
                .model
                .reconnect
                .state_restore_correction_metrics
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
        self.model
            .reconnect
            .state_restore_validation_retry_cooldown_ticks = cooldown_ticks;
        self.model
            .reconnect
            .state_restore_correction_metrics
            .correction_retries_scheduled = self
            .model
            .reconnect
            .state_restore_correction_metrics
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
        self.model
            .reconnect
            .state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction = self
            .model
            .reconnect
            .state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction
            .saturating_add(1);
        if self
            .model
            .reconnect
            .state_restore_validation_mismatch_seen_in_cycle
        {
            let decay = self
                .behavior_config
                .reconnect_state_restore_correction_disable_after_mismatch_decay_on_success;
            if decay > 0 {
                self.model
                    .reconnect
                    .state_restore_correction_consecutive_mismatch_cycles = self
                    .model
                    .reconnect
                    .state_restore_correction_consecutive_mismatch_cycles
                    .saturating_sub(decay);
            }
        }
        self.reset_reconnect_state_restore_correction_retry_exhaustions();
        self.model
            .reconnect
            .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 0;
        self.model
            .reconnect
            .state_restore_correction_recovery_reenable_notification_pending = false;
        self.clear_reconnect_state_restore_validation_state();
    }

    pub(crate) fn apply_successful_reconnect_state_restore_validation_action(
        &mut self,
        action: &ClientRuntimeAction,
    ) {
        match action {
            ClientRuntimeAction::SetPaused(paused) => {
                self.model.playback.local_paused = Some(*paused);
            }
            ClientRuntimeAction::SetPosition(position_seconds) if position_seconds.is_finite() => {
                self.model.playback.local_position = Some(*position_seconds);
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
        C: ClientEffectSink,
    {
        for action in actions {
            match action {
                ClientRuntimeAction::SetPaused(paused) => {
                    player.execute(PlayerCommand::SetPaused(*paused))?;
                }
                ClientRuntimeAction::RequestUserList => {
                    control
                        .emit(ClientEffect::RequestUserList)
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetRoom { room } => {
                    control
                        .emit(ClientEffect::SetRoom(room.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetReady {
                    ready,
                    manually_initiated,
                } => {
                    control
                        .emit(ClientEffect::SetReady {
                            ready: *ready,
                            manually_initiated: *manually_initiated,
                        })
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetReadyForUser {
                    ready,
                    manually_initiated,
                    username,
                } => {
                    control
                        .emit(ClientEffect::SetReadyForUser {
                            ready: *ready,
                            manually_initiated: *manually_initiated,
                            username: username.clone(),
                        })
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetReadinessIntent { request, scope } => {
                    control.activate_protocol_connection_generation();
                    control
                        .emit(ClientEffect::SendReadinessIntent {
                            request: request.clone(),
                            scope: scope.clone(),
                        })
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::ReportTechnicalReadiness(report) => {
                    control.activate_protocol_connection_generation();
                    control
                        .emit(ClientEffect::ReportTechnicalReadiness(report.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetFile { file } => {
                    control
                        .emit(ClientEffect::SetFile(file.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetPlaylist { files } => {
                    control
                        .emit(ClientEffect::SetPlaylist(files.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetPlaylistIndex { index } => {
                    control
                        .emit(ClientEffect::SetPlaylistIndex(*index))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::RequestControllerAuth { room, password } => {
                    let payload = ControllerAuthPayload::new()
                        .with_room(room.clone())
                        .with_password(password.clone());
                    control
                        .emit(ClientEffect::RequestControllerAuth(payload))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SendChat { message } => {
                    control
                        .emit(ClientEffect::SendChat(message.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyChat(notification) => {
                    control
                        .emit(ClientEffect::NotifyChat(notification.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyControlledRoomCreation(notification) => {
                    control
                        .emit(ClientEffect::NotifyControlledRoomCreation(
                            notification.clone(),
                        ))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyControllerAuthTransition(notification) => {
                    control
                        .emit(ClientEffect::NotifyControllerAuthTransition(
                            notification.clone(),
                        ))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyUserChange(notification) => {
                    control
                        .emit(ClientEffect::NotifyUserChange(notification.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyReconnectTransition(notification) => {
                    control
                        .emit(ClientEffect::NotifyReconnectTransition(
                            notification.clone(),
                        ))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::NotifyAutoplayCountdown(notification) => {
                    control
                        .emit(ClientEffect::NotifyAutoplayCountdown(notification.clone()))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::SetPosition(position) => {
                    player.execute(PlayerCommand::SetPosition(*position))?;
                }
                ClientRuntimeAction::SetPlaybackRate(rate) => {
                    player.execute(PlayerCommand::SetPlaybackRate(*rate))?;
                }
                ClientRuntimeAction::ScheduleReconnect { delay_seconds } => {
                    control
                        .emit(ClientEffect::ScheduleReconnect(*delay_seconds))
                        .map_err(client_effect_player_error)?;
                }
                ClientRuntimeAction::StopReconnect => {
                    control
                        .emit(ClientEffect::StopReconnect)
                        .map_err(client_effect_player_error)?;
                }
            }
        }
        Ok(())
    }

    pub fn apply_hello_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        let message = decode_message_line(json_line)?;
        if !matches!(message, ProtocolMessage::Hello(_)) {
            return Err(ProtocolError::UnexpectedMessageKind {
                expected: "Hello",
                found: message.kind(),
            });
        }
        self.apply_protocol_message(message)
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
        let normalized = normalize_client_protocol_message(message);
        self.retain_compatibility_fallbacks(normalized.fallbacks);
        match normalized.command {
            ClientInboundCommand::Hello(hello) => self.apply_hello(hello),
            ClientInboundCommand::Set(commands) => self.apply_set(commands, now_seconds),
            ClientInboundCommand::List(rooms) => self.apply_list(rooms),
            ClientInboundCommand::State(state) => self.apply_state_at(state, now_seconds),
            ClientInboundCommand::Chat(notification) => self.apply_chat(notification),
            ClientInboundCommand::ServerError(message) => {
                return Err(ProtocolError::ServerError { message });
            }
            ClientInboundCommand::UnexpectedTls(start_tls) => {
                return Err(ProtocolError::UnexpectedTlsMessage { start_tls });
            }
            ClientInboundCommand::Ignore => {}
        }
        Ok(())
    }

    pub fn apply_message_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        for item in decode_message_line_items(json_line)? {
            let message = item.message?;
            self.apply_protocol_message(message)?;
        }
        Ok(())
    }

    pub fn apply_message_json_at(
        &mut self,
        json_line: &str,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        for item in decode_message_line_items(json_line)? {
            let message = item.message?;
            self.apply_protocol_message_at(message, now_seconds)?;
        }
        Ok(())
    }
}
