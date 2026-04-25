use super::*;

impl ClientSession {
    pub fn runtime_actions_for_local_seek(
        &mut self,
        target_position: f64,
    ) -> Vec<ClientRuntimeAction> {
        let previous_position = self
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

        self.last_seek_position_before_manual_seek = Some(previous_position);
        self.local_position = Some(target_position);
        vec![ClientRuntimeAction::SetPosition(target_position)]
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
            .or(self.local_position)
            .unwrap_or(0.0);
        self.runtime_actions_for_local_seek(baseline_position + offset_seconds)
    }

    pub fn runtime_actions_for_local_seek_undo(&mut self) -> Vec<ClientRuntimeAction> {
        let Some(target_position) = self.last_seek_position_before_manual_seek else {
            return Vec::new();
        };
        let current_position = self
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
        let (Some(global_position), Some(global_paused)) =
            (global_playstate.position, global_playstate.paused)
        else {
            self.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        };

        if global_playstate.do_seek == Some(true) {
            self.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        }

        let diff = local_position - global_position;
        let set_by = global_playstate.set_by.clone();
        let set_by_is_self = self
            .username
            .as_deref()
            .zip(set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);

        if self.desync_config.rewind_on_desync && diff > self.desync_config.rewind_threshold_seconds
        {
            self.behind_first_detected_at_seconds = None;
            if set_by_is_self {
                return DesyncCorrectionAction::None;
            }
            return DesyncCorrectionAction::Rewind {
                target_position: global_position,
                set_by,
            };
        }

        if self.desync_config.fastforward_on_desync
            && (!local_can_control || dont_slow_down_with_me)
        {
            if diff < -self.desync_config.fastforward_behind_threshold_seconds {
                if let Some(first_detected_at) = self.behind_first_detected_at_seconds {
                    let duration_behind = now_seconds - first_detected_at;
                    if duration_behind
                        > (self.desync_config.fastforward_threshold_seconds
                            - self.desync_config.fastforward_behind_threshold_seconds)
                        && diff < -self.desync_config.fastforward_threshold_seconds
                    {
                        self.behind_first_detected_at_seconds = Some(
                            now_seconds + self.desync_config.fastforward_reset_threshold_seconds,
                        );
                        if set_by_is_self {
                            return DesyncCorrectionAction::None;
                        }
                        return DesyncCorrectionAction::FastForward {
                            target_position: global_position
                                + self.desync_config.fastforward_extra_seconds,
                            set_by,
                        };
                    }
                } else {
                    self.behind_first_detected_at_seconds = Some(now_seconds);
                }
            } else {
                self.behind_first_detected_at_seconds = None;
            }
        } else {
            self.behind_first_detected_at_seconds = None;
        }

        if speed_supported && !global_paused && self.desync_config.slow_on_desync {
            if diff > self.desync_config.slowdown_threshold_seconds && !self.speed_changed {
                if set_by_is_self {
                    return DesyncCorrectionAction::None;
                }
                self.speed_changed = true;
                return DesyncCorrectionAction::SlowDown {
                    rate: self.desync_config.slowdown_rate,
                    set_by,
                };
            }
            if self.speed_changed && diff < self.desync_config.slowdown_reset_threshold_seconds {
                self.speed_changed = false;
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
            self.behind_first_detected_at_seconds = None;
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
        self.clear_server_feature_support_state();
        if !self.behavior_config.pause_on_leave {
            return Vec::new();
        }

        self.last_paused_on_leave_at_seconds = Some(now_seconds);
        let should_pause = self.local_paused != Some(true);
        self.local_paused = Some(true);

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
            || self.readiness_autoplay_config.unpause_action == UnpauseActionMode::Always
        {
            return true;
        }

        let all_other_users_ready = self.all_other_users_in_current_room_ready();
        match self.readiness_autoplay_config.unpause_action {
            UnpauseActionMode::IfAlreadyReady => false,
            UnpauseActionMode::IfOthersReady => all_other_users_ready,
            UnpauseActionMode::IfMinUsersReady => {
                all_other_users_ready
                    && self
                        .readiness_autoplay_config
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
        if !readiness_supported {
            return Vec::new();
        }

        let instaplay = self.instaplay_conditions_met(local_can_control, is_playing_music);
        if !instaplay {
            self.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if !self.local_user_ready() {
                self.apply_local_ready_state_optimistically(true);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if let Some(last_paused_on_leave_at_seconds) = self.last_paused_on_leave_at_seconds
            && now_seconds - last_paused_on_leave_at_seconds
                < self
                    .readiness_autoplay_config
                    .last_paused_diff_threshold_seconds
        {
            self.last_paused_on_leave_at_seconds = None;
            self.local_paused = Some(false);
            return Vec::new();
        }

        self.local_paused = Some(false);
        if self.local_user_ready() {
            return Vec::new();
        }

        self.apply_local_ready_state_optimistically(true);
        vec![ClientRuntimeAction::SetReady {
            ready: true,
            manually_initiated: false,
        }]
    }

    pub fn autoplay_conditions_met(
        &self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> bool {
        if is_playing_music {
            return true;
        }

        let threshold_met = self
            .readiness_autoplay_config
            .auto_play_threshold
            .is_some_and(|threshold| self.users_in_current_room_count_for_threshold() >= threshold);

        self.local_paused.unwrap_or(true)
            && (self.autoplay_enabled || recently_advanced)
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
        if !self.autoplay_timer_running {
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

        if self.autoplay_time_left_seconds <= 0.0 {
            self.local_paused = Some(false);
            self.stop_autoplay_countdown();
            return vec![ClientRuntimeAction::SetPaused(false)];
        }

        let notification = AutoplayCountdownNotification {
            ready_user_count: self.ready_user_count_in_current_room(),
            seconds_left: self.autoplay_time_left_seconds.max(0.0).floor() as u32,
        };
        self.autoplay_time_left_seconds -= AUTOPLAY_COUNTDOWN_STEP_SECONDS;
        vec![ClientRuntimeAction::NotifyAutoplayCountdown(notification)]
    }
}
