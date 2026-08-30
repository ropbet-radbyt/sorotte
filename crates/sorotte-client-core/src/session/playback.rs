use super::*;

impl ClientSession {
    pub fn runtime_actions_for_local_seek(
        &mut self,
        target_position: f64,
    ) -> Vec<ClientRuntimeAction> {
        let previous_position = self
            .model
            .playback
            .local_position
            .or_else(|| {
                self.current_room_playstate()
                    .and_then(|playstate| playstate.position)
            })
            .unwrap_or(0.0);
        self.runtime_actions_for_local_seek_with_previous_position(
            target_position,
            previous_position,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn local_seek_target_allowed(&self, target_position: f64, now_seconds: f64) -> bool {
        self.normalized_local_seek_target(target_position, now_seconds)
            .is_some()
    }

    pub(super) fn normalized_local_seek_target(
        &self,
        target_position: f64,
        now_seconds: f64,
    ) -> Option<f64> {
        if !target_position.is_finite() {
            return None;
        }

        let target_position = target_position.max(0.0);
        if self.recently_rewound(now_seconds, RECENT_REWIND_SEEK_SUPPRESSION_SECONDS)
            && target_position > RECENT_REWIND_SEEK_IGNORE_POSITION_SECONDS
        {
            return None;
        }

        Some(target_position)
    }

    pub(super) fn runtime_actions_for_local_seek_with_previous_position(
        &mut self,
        target_position: f64,
        previous_position: f64,
        now_seconds: f64,
    ) -> Vec<ClientRuntimeAction> {
        let Some(target_position) = self.normalized_local_seek_target(target_position, now_seconds)
        else {
            return Vec::new();
        };

        self.model.playlist.last_seek_position_before_manual_seek = Some(previous_position);
        self.model.playback.local_position = Some(target_position);
        let mut actions = vec![ClientRuntimeAction::SetPosition(target_position)];
        if self.is_active() {
            let paused = self
                .model
                .playback
                .local_paused
                .or_else(|| {
                    self.current_room_playstate()
                        .and_then(|playstate| playstate.paused)
                })
                .unwrap_or(true);
            self.model.playback.client_ignoring_on_the_fly = self
                .model
                .playback
                .client_ignoring_on_the_fly
                .saturating_add(1);
            actions.push(ClientRuntimeAction::SendState(
                StatePayload::new()
                    .with_playstate(
                        PlaystatePayload::new()
                            .with_position(target_position)
                            .with_paused(paused)
                            .with_do_seek(true),
                    )
                    .with_ignoring_on_the_fly(
                        IgnoringOnTheFlyPayload::new()
                            .with_client(self.model.playback.client_ignoring_on_the_fly),
                    ),
            ));
        }
        actions
    }

    pub fn runtime_actions_for_local_seek_offset(
        &mut self,
        offset_seconds: f64,
    ) -> Vec<ClientRuntimeAction> {
        if !offset_seconds.is_finite() {
            return Vec::new();
        }
        let baseline_position = self
            .current_room_playstate()
            .and_then(|playstate| playstate.position)
            .or(self.model.playback.local_position)
            .unwrap_or(0.0);
        self.runtime_actions_for_local_seek(baseline_position + offset_seconds)
    }

    pub fn runtime_actions_for_local_seek_undo(&mut self) -> Vec<ClientRuntimeAction> {
        let Some(target_position) = self.model.playlist.last_seek_position_before_manual_seek
        else {
            return Vec::new();
        };
        let current_position = self
            .model
            .playback
            .local_position
            .or_else(|| {
                self.current_room_playstate()
                    .and_then(|playstate| playstate.position)
            })
            .unwrap_or(target_position);
        self.runtime_actions_for_local_seek_with_previous_position(
            target_position,
            current_position,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub(super) fn evaluate_desync_correction_for_room_playstate(
        &mut self,
        global_playstate: &RoomPlaystateView,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> DesyncCorrectionAction {
        if self.model.playback.local_paused_for_cache == Some(true)
            || self.model.playback.pending_cache_room_playstate_resync
        {
            self.model.playback.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        }

        let (Some(global_position), Some(global_paused)) =
            (global_playstate.position, global_playstate.paused)
        else {
            self.model.playback.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        };

        if global_playstate.do_seek == Some(true) {
            self.model.playback.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        }

        let diff = local_position - global_position;
        let set_by = global_playstate.set_by.clone();
        let set_by_is_self = self
            .model
            .connection
            .username
            .as_deref()
            .zip(set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);
        let self_origin_grace_active = set_by_is_self
            && self
                .model
                .room
                .name
                .as_deref()
                .and_then(|room| {
                    self.model
                        .room
                        .playstate_authority_changed_at_seconds
                        .get(room)
                })
                .is_none_or(|updated_at| {
                    let elapsed_seconds = now_seconds - updated_at;
                    (0.0..=SELF_ORIGIN_CORRECTION_GRACE_SECONDS).contains(&elapsed_seconds)
                });
        let fastforward_sustain_seconds = (self
            .model
            .playback
            .desync_config
            .fastforward_threshold_seconds
            - self
                .model
                .playback
                .desync_config
                .fastforward_behind_threshold_seconds)
            .max(0.0);
        let self_origin_fastforward_grace_active = set_by_is_self
            && self
                .model
                .room
                .name
                .as_deref()
                .and_then(|room| {
                    self.model
                        .room
                        .playstate_authority_changed_at_seconds
                        .get(room)
                })
                .is_none_or(|updated_at| {
                    let elapsed_seconds = now_seconds - updated_at;
                    (0.0..=SELF_ORIGIN_CORRECTION_GRACE_SECONDS + fastforward_sustain_seconds)
                        .contains(&elapsed_seconds)
                });

        if self.model.playback.desync_config.rewind_on_desync
            && diff > self.model.playback.desync_config.rewind_threshold_seconds
        {
            self.model.playback.behind_first_detected_at_seconds = None;
            if self_origin_grace_active {
                return DesyncCorrectionAction::None;
            }
            if self.model.playback.speed_changed {
                self.model.playback.speed_changed = false;
                self.model.playback.speed_correction_rate = None;
                self.model.playback.local_playback_rate = Some(NORMAL_PLAYBACK_RATE);
                return DesyncCorrectionAction::RestoreSpeed {
                    rate: NORMAL_PLAYBACK_RATE,
                };
            }
            return DesyncCorrectionAction::Rewind {
                target_position: global_position,
                set_by,
            };
        }

        if self.model.playback.desync_config.fastforward_on_desync
            && (!local_can_control || dont_slow_down_with_me)
        {
            if diff
                < -self
                    .model
                    .playback
                    .desync_config
                    .fastforward_behind_threshold_seconds
            {
                if let Some(first_detected_at) =
                    self.model.playback.behind_first_detected_at_seconds
                {
                    let duration_behind = now_seconds - first_detected_at;
                    if duration_behind
                        > (self
                            .model
                            .playback
                            .desync_config
                            .fastforward_threshold_seconds
                            - self
                                .model
                                .playback
                                .desync_config
                                .fastforward_behind_threshold_seconds)
                        && diff
                            < -self
                                .model
                                .playback
                                .desync_config
                                .fastforward_threshold_seconds
                    {
                        self.model.playback.behind_first_detected_at_seconds = Some(
                            now_seconds
                                + self
                                    .model
                                    .playback
                                    .desync_config
                                    .fastforward_reset_threshold_seconds,
                        );
                        if self_origin_fastforward_grace_active {
                            return DesyncCorrectionAction::None;
                        }
                        if self.model.playback.speed_changed {
                            self.model.playback.speed_changed = false;
                            self.model.playback.speed_correction_rate = None;
                            self.model.playback.local_playback_rate = Some(NORMAL_PLAYBACK_RATE);
                            return DesyncCorrectionAction::RestoreSpeed {
                                rate: NORMAL_PLAYBACK_RATE,
                            };
                        }
                        return DesyncCorrectionAction::FastForward {
                            target_position: global_position
                                + self.model.playback.desync_config.fastforward_extra_seconds,
                            set_by,
                        };
                    }
                } else {
                    self.model.playback.behind_first_detected_at_seconds = Some(now_seconds);
                }
            } else {
                self.model.playback.behind_first_detected_at_seconds = None;
            }
        } else {
            self.model.playback.behind_first_detected_at_seconds = None;
        }

        if speed_supported
            && self.model.playback.speed_changed
            && !self.model.playback.desync_config.slow_on_desync
        {
            self.model.playback.speed_changed = false;
            self.model.playback.speed_correction_rate = None;
            self.model.playback.local_playback_rate = Some(NORMAL_PLAYBACK_RATE);
            return DesyncCorrectionAction::RestoreSpeed {
                rate: NORMAL_PLAYBACK_RATE,
            };
        }

        if speed_supported && !global_paused && self.model.playback.desync_config.slow_on_desync {
            let threshold = self.model.playback.desync_config.slowdown_threshold_seconds;
            let reset_threshold = self
                .model
                .playback
                .desync_config
                .slowdown_reset_threshold_seconds;
            let active_correction_crossed_target = self
                .model
                .playback
                .speed_correction_rate
                .is_some_and(|rate| rate < NORMAL_PLAYBACK_RATE && diff <= reset_threshold);
            if self.model.playback.speed_changed && active_correction_crossed_target {
                self.model.playback.speed_changed = false;
                self.model.playback.speed_correction_rate = None;
                self.model.playback.local_playback_rate = Some(NORMAL_PLAYBACK_RATE);
                return DesyncCorrectionAction::RestoreSpeed {
                    rate: NORMAL_PLAYBACK_RATE,
                };
            }
            if diff > threshold {
                if self_origin_grace_active {
                    return DesyncCorrectionAction::None;
                }
                let target_rate = self.model.playback.desync_config.slowdown_rate;
                let correction_matches = self
                    .model
                    .playback
                    .speed_correction_rate
                    .is_some_and(|rate| (rate - target_rate).abs() <= 0.001);
                let observed_rate_matches = self
                    .model
                    .playback
                    .local_playback_rate
                    .is_none_or(|rate| (rate - target_rate).abs() <= 0.001);
                if correction_matches && observed_rate_matches {
                    return DesyncCorrectionAction::None;
                }
                self.model.playback.speed_changed = true;
                self.model.playback.speed_correction_rate = Some(target_rate);
                // Treat the accepted command as the current value until fresh player telemetry
                // arrives. If the coordinator or mpv restores 1.0, that observation re-arms this
                // correction instead of leaving the client to drift at the wrong rate.
                self.model.playback.local_playback_rate = Some(target_rate);
                return DesyncCorrectionAction::SlowDown {
                    rate: target_rate,
                    set_by,
                };
            }
            // Ordinary behind-client drift is handled by the existing sustained
            // fast-forward policy for followers. Speeding a client up here is
            // unstable under asymmetric RTT: its samples can become the room
            // clock while other clients respond with the opposite correction.
            // Recovery-owned catch-up remains in PlaybackCoordinator, where it
            // is scoped to a verified cache-recovery episode.
            if self.model.playback.speed_changed && diff.abs() < reset_threshold {
                self.model.playback.speed_changed = false;
                self.model.playback.speed_correction_rate = None;
                self.model.playback.local_playback_rate = Some(NORMAL_PLAYBACK_RATE);
                return DesyncCorrectionAction::RestoreSpeed {
                    rate: NORMAL_PLAYBACK_RATE,
                };
            }
        }

        DesyncCorrectionAction::None
    }

    pub fn evaluate_desync_correction(
        &mut self,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> DesyncCorrectionAction {
        let Some(global_playstate) = self.current_room_playstate().cloned() else {
            self.model.playback.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        };

        self.evaluate_desync_correction_for_room_playstate(
            &global_playstate,
            now_seconds,
            local_position,
            local_can_control,
            dont_slow_down_with_me,
            speed_supported,
        )
    }

    pub fn runtime_actions_for_desync_correction(
        &mut self,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Vec<ClientRuntimeAction> {
        match self.evaluate_desync_correction(
            now_seconds,
            local_position,
            local_can_control,
            dont_slow_down_with_me,
            speed_supported,
        ) {
            DesyncCorrectionAction::None => Vec::new(),
            DesyncCorrectionAction::Rewind {
                target_position, ..
            }
            | DesyncCorrectionAction::FastForward {
                target_position, ..
            } => {
                vec![ClientRuntimeAction::SetPosition(target_position)]
            }
            DesyncCorrectionAction::SlowDown { rate, .. }
            | DesyncCorrectionAction::RestoreSpeed { rate } => {
                vec![ClientRuntimeAction::SetPlaybackRate(rate)]
            }
        }
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
        match self.evaluate_desync_correction_for_room_playstate(
            &room_playstate,
            now_seconds,
            local_position,
            local_can_control,
            dont_slow_down_with_me,
            speed_supported,
        ) {
            DesyncCorrectionAction::None => Vec::new(),
            DesyncCorrectionAction::Rewind {
                target_position, ..
            }
            | DesyncCorrectionAction::FastForward {
                target_position, ..
            } => {
                vec![ClientRuntimeAction::SetPosition(target_position)]
            }
            DesyncCorrectionAction::SlowDown { rate, .. }
            | DesyncCorrectionAction::RestoreSpeed { rate } => {
                vec![ClientRuntimeAction::SetPlaybackRate(rate)]
            }
        }
    }

    pub fn handle_disconnect(&mut self, now_seconds: f64) -> Vec<ClientRuntimeAction> {
        self.mark_disconnected();
        if !self.behavior_config.pause_on_leave {
            return Vec::new();
        }

        self.model.playback.last_paused_on_leave_at_seconds = Some(now_seconds);
        let should_pause = self.model.playback.local_paused != Some(true);
        self.model.playback.local_paused = Some(true);

        if should_pause {
            vec![ClientRuntimeAction::SetPaused(true)]
        } else {
            Vec::new()
        }
    }

    pub fn instaplay_conditions_met(
        &self,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> bool {
        if is_playing_music {
            return true;
        }

        if !local_can_control {
            return false;
        }

        if self.local_user_ready()
            || self.model.readiness.config.unpause_action == UnpauseActionMode::Always
        {
            return true;
        }

        let all_other_users_ready = self.all_other_users_in_current_room_ready();
        match self.model.readiness.config.unpause_action {
            UnpauseActionMode::IfAlreadyReady => false,
            UnpauseActionMode::IfOthersReady => all_other_users_ready,
            UnpauseActionMode::IfMinUsersReady => {
                all_other_users_ready
                    && self
                        .model
                        .readiness
                        .config
                        .auto_play_threshold
                        .is_some_and(|threshold| {
                            self.users_in_current_room_count_for_threshold() >= threshold
                        })
            }
            UnpauseActionMode::Always => true,
        }
    }

    pub fn runtime_actions_for_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_readiness_unpause_attempt_with_gate_hold(
            now_seconds,
            readiness_supported,
            local_can_control,
            is_playing_music,
            None,
        )
    }

    pub(crate) fn runtime_actions_for_readiness_unpause_attempt_with_gate_hold(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        current_gate_holds_play: Option<bool>,
    ) -> Vec<ClientRuntimeAction> {
        if !readiness_supported {
            return Vec::new();
        }
        if self.model.playback.local_paused_for_cache == Some(true) {
            return Vec::new();
        }

        if self.server_readiness_v2_supported() {
            let gate_holds_play =
                current_gate_holds_play.unwrap_or_else(|| self.readiness_gate_holds_room_pause());
            if !local_can_control || gate_holds_play {
                self.model.playback.local_paused = Some(true);
                // This observation-only seam has no proof of a user gesture.
                // Shared causal classification emits any indirect Ready before
                // the gate-hold correction is issued.
                return vec![ClientRuntimeAction::SetPaused(true)];
            }

            // An authorized controller resolves AwaitingDecision or a
            // terminal/precommit barrier with ordinary playback control. V2
            // does not inherit the legacy instaplay preference matrix.
            self.model.playback.local_paused = Some(false);
            return Vec::new();
        }

        let instaplay = self.instaplay_conditions_met(local_can_control, is_playing_music);
        if !instaplay {
            self.model.playback.local_paused = Some(true);
            // This periodic compatibility check observes state but has no
            // proof of a user gesture. The causal player classifier owns
            // native Play detection; a gate correction is system-owned and
            // must not manufacture readiness intent.
            return vec![ClientRuntimeAction::SetPaused(true)];
        }

        if let Some(last_paused_on_leave_at_seconds) =
            self.model.playback.last_paused_on_leave_at_seconds
            && now_seconds - last_paused_on_leave_at_seconds
                < self
                    .model
                    .readiness
                    .config
                    .last_paused_diff_threshold_seconds
        {
            self.model.playback.last_paused_on_leave_at_seconds = None;
            self.model.playback.local_paused = Some(false);
            return Vec::new();
        }

        self.model.playback.local_paused = Some(false);
        Vec::new()
    }

    pub fn autoplay_conditions_met(
        &self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> bool {
        if self.server_readiness_v2_supported() {
            return false;
        }
        if self.model.playback.local_paused_for_cache == Some(true) {
            return false;
        }
        if self.has_pending_playlist_index_reset_intent() {
            return false;
        }
        if is_playing_music {
            return true;
        }

        let threshold_met = self
            .model
            .readiness
            .config
            .auto_play_threshold
            .is_some_and(|threshold| self.users_in_current_room_count_for_threshold() >= threshold);

        self.model.playback.local_paused.unwrap_or(true)
            && (self.model.readiness.autoplay_enabled || recently_advanced)
            && local_can_control
            && readiness_supported
            && self.all_users_in_current_room_ready()
            && (threshold_met || recently_advanced)
    }

    pub fn autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        if is_playing_music {
            return;
        }

        if self.autoplay_conditions_met(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            self.start_autoplay_countdown();
        } else {
            self.stop_autoplay_countdown();
        }
    }

    pub fn autoplay_countdown_tick(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Vec<ClientRuntimeAction> {
        if !self.model.readiness.autoplay_timer_running {
            return Vec::new();
        }

        if !self.autoplay_conditions_met(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            self.stop_autoplay_countdown();
            return Vec::new();
        }

        if self.model.readiness.autoplay_time_left_seconds <= 0.0 {
            self.model.playback.local_paused = Some(false);
            self.stop_autoplay_countdown();
            return vec![ClientRuntimeAction::SetPaused(false)];
        }

        let notification = AutoplayCountdownNotification {
            ready_user_count: self.ready_user_count_in_current_room(),
            seconds_left: self
                .model
                .readiness
                .autoplay_time_left_seconds
                .max(0.0)
                .floor() as u32,
        };
        self.model.readiness.autoplay_time_left_seconds -= AUTOPLAY_COUNTDOWN_STEP_SECONDS;
        vec![ClientRuntimeAction::NotifyAutoplayCountdown(notification)]
    }
}
