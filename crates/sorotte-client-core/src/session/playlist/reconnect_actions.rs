use super::super::*;

impl ClientSession {
    pub fn recently_advanced(&self, now_seconds: f64) -> bool {
        let threshold_seconds =
            self.model.readiness.config.autoplay_delay_seconds + RECENTLY_ADVANCED_GRACE_SECONDS;
        self.model
            .playback
            .last_advanced_at_seconds
            .is_some_and(|last_advanced_at_seconds| {
                let elapsed = now_seconds - last_advanced_at_seconds;
                elapsed >= 0.0 && elapsed < threshold_seconds
            })
    }

    pub fn plan_reconnect_retry(&mut self, retries: u32) -> ReconnectRetryDecision {
        self.reset_sync_state_for_reconnect_with_attempt(retries);

        if retries > self.model.reconnect.policy.max_retries {
            self.model.reconnect.in_progress = false;
            self.model.reconnect.connected_intent = false;
            self.mark_disconnected();
            return ReconnectRetryDecision {
                should_retry: false,
                delay_seconds: None,
                should_reset_state: true,
            };
        }

        let exponent = retries.min(self.model.reconnect.policy.max_backoff_exponent);
        let delay_seconds =
            self.model.reconnect.policy.base_delay_seconds * 2_f64.powi(exponent as i32);
        self.model.reconnect.in_progress = true;

        ReconnectRetryDecision {
            should_retry: true,
            delay_seconds: Some(delay_seconds),
            should_reset_state: true,
        }
    }

    pub fn runtime_actions_for_reconnect_retry(
        &mut self,
        retries: u32,
    ) -> Vec<ClientRuntimeAction> {
        let decision = self.plan_reconnect_retry(retries);
        if decision.should_retry {
            if let Some(delay_seconds) = decision.delay_seconds {
                return vec![
                    ClientRuntimeAction::NotifyReconnectTransition(
                        ReconnectTransitionNotification::Attempting {
                            retries,
                            delay_seconds,
                        },
                    ),
                    ClientRuntimeAction::ScheduleReconnect { delay_seconds },
                ];
            }
            return Vec::new();
        }
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Disconnected,
            ),
            ClientRuntimeAction::StopReconnect,
        ]
    }

    pub fn runtime_actions_for_reconnect_transition_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.model.reconnect.connected_intent {
            return Vec::new();
        }
        self.model.reconnect.connected_intent = false;
        vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Connected,
        )]
    }

    pub fn runtime_actions_for_controller_auth_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_controller_auth_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyControllerAuthTransition)
            .collect()
    }

    pub fn runtime_actions_for_controlled_room_creation_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_controlled_room_creation_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyControlledRoomCreation)
            .collect()
    }

    pub fn runtime_actions_for_chat_notifications_if_needed(&mut self) -> Vec<ClientRuntimeAction> {
        self.pending_chat_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyChat)
            .collect()
    }

    pub fn runtime_actions_for_user_change_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_user_change_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyUserChange)
            .collect()
    }

    pub fn runtime_actions_for_reconnect_state_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let mut actions = Vec::new();

        if let Some(ready) = self.model.reconnect.ready_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: false,
            });
        }

        if let Some(file_payload) = self.model.reconnect.file_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetFile { file: file_payload });
            actions.push(ClientRuntimeAction::RequestUserList);
        }

        if !actions.is_empty() {
            self.model.reconnect.state_restore_validation_pending = true;
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
            self.begin_reconnect_state_restore_validation_cycle();
            actions.insert(
                0,
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringState,
                ),
            );
        }

        actions
    }

    pub fn runtime_actions_for_reconnect_state_restore_validation_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.model.reconnect.state_restore_validation_pending {
            return Vec::new();
        }
        if self
            .model
            .reconnect
            .state_restore_validation_retry_cooldown_ticks
            > 0
        {
            self.model
                .reconnect
                .state_restore_validation_retry_cooldown_ticks = self
                .model
                .reconnect
                .state_restore_validation_retry_cooldown_ticks
                .saturating_sub(1);
            return Vec::new();
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let Some(room_playstate) = self.current_room_playstate_at(now_seconds) else {
            return Vec::new();
        };
        let (Some(room_paused), Some(room_position)) =
            (room_playstate.paused, room_playstate.position)
        else {
            return Vec::new();
        };
        let (Some(local_paused), Some(local_position)) = (
            self.model.playback.local_paused,
            self.model.playback.local_position,
        ) else {
            return Vec::new();
        };

        let position_diff_seconds = (local_position - room_position).abs();
        let pause_matches = local_paused == room_paused;
        let position_tolerance_seconds =
            self.reconnect_state_restore_position_tolerance_seconds_effective();
        let position_matches = position_diff_seconds <= position_tolerance_seconds;
        if pause_matches && position_matches {
            self.model
                .reconnect
                .state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch
                .saturating_add(1);
            self.model
                .reconnect
                .state_restore_correction_consecutive_mismatch_cycles = 0;
            self.reset_reconnect_state_restore_correction_retry_exhaustions();
            self.model
                .reconnect
                .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 0;
            self.model
                .reconnect
                .state_restore_correction_recovery_reenable_notification_pending = false;
            self.clear_reconnect_state_restore_validation_state();
            return Vec::new();
        }

        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        let correction_suppressed_for_recovery_cycle = self
            .model
            .reconnect
            .state_restore_correction_recovery_suppressed_this_cycle;
        let correction_reenabled_for_this_cycle = self
            .model
            .reconnect
            .state_restore_correction_recovery_reenabled_this_cycle;
        if !self
            .model
            .reconnect
            .state_restore_validation_mismatch_seen_in_cycle
            && !correction_suppressed_for_recovery_cycle
        {
            self.model
                .reconnect
                .state_restore_validation_mismatch_seen_in_cycle = true;
            self.model
                .reconnect
                .state_restore_correction_metrics
                .mismatch_cycles_detected = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .mismatch_cycles_detected
                .saturating_add(1);
            self.model
                .reconnect
                .state_restore_correction_consecutive_mismatch_cycles = self
                .model
                .reconnect
                .state_restore_correction_consecutive_mismatch_cycles
                .saturating_add(1);
        }
        let consecutive_mismatch_cycles = self
            .model
            .reconnect
            .state_restore_correction_consecutive_mismatch_cycles;
        let disable_after_mismatch_cycles = self
            .behavior_config
            .reconnect_state_restore_correction_disable_after_mismatch_cycles;
        let disable_correction_due_to_repeated_mismatches = matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches
        ) && disable_after_mismatch_cycles > 0
            && consecutive_mismatch_cycles >= disable_after_mismatch_cycles;
        let mut actions = Vec::new();
        let should_emit_mismatch_notification = !matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion
        ) && !disable_correction_due_to_repeated_mismatches;
        if correction_reenabled_for_this_cycle {
            self.model
                .reconnect
                .state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
            ));
        }
        if should_emit_mismatch_notification
            && !self
                .model
                .reconnect
                .state_restore_validation_mismatch_notified
        {
            self.model
                .reconnect
                .state_restore_validation_mismatch_notified = true;
            self.model
                .reconnect
                .state_restore_correction_metrics
                .mismatch_notifications_emitted = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .mismatch_notifications_emitted
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused,
                    room_paused,
                    local_position,
                    room_position,
                    position_diff_seconds,
                },
            ));
        }

        if correction_suppressed_for_recovery_cycle {
            self.model
                .reconnect
                .state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: self
                        .model.reconnect.state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
                },
            ));
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if disable_correction_due_to_repeated_mismatches {
            if self.activate_reconnect_state_restore_correction_recovery_cooldown_if_configured() {
                self.model
                    .reconnect
                    .state_restore_correction_consecutive_mismatch_cycles = 0;
            }
            self.model
                .reconnect
                .state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches = self
                .model
                .reconnect
                .state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches
                .saturating_add(1);
            self.clear_reconnect_state_restore_validation_state();
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles,
                    disable_after_mismatch_cycles,
                },
            ));
            return actions;
        }

        if matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
        ) {
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if !pause_matches {
            actions.push(ClientRuntimeAction::SetPaused(room_paused));
        }
        if !position_matches {
            actions.push(ClientRuntimeAction::SetPosition(room_position));
        }
        actions
    }

    pub fn runtime_actions_for_reconnect_playlist_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.is_active() {
            return Vec::new();
        }
        let Some(restore_intent) = self.model.reconnect.playlist_restore_intent.take() else {
            return Vec::new();
        };
        if !self.server_shared_playlists_supported() {
            return Vec::new();
        }

        let mut actions = vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringPlaylist,
            ),
            ClientRuntimeAction::SetPlaylist {
                files: restore_intent.files,
            },
        ];
        if let Some(index) = restore_intent.index {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index });
        }
        actions
    }

    pub fn runtime_actions_for_controller_reidentify_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.server_managed_rooms_supported() {
            self.model.controller.controlled_room_switch_intent = None;
            self.model.controller.reidentify_intent = None;
            return Vec::new();
        }

        let mut actions = Vec::new();
        if let Some(room) = self.model.controller.controlled_room_switch_intent.take() {
            actions.push(ClientRuntimeAction::SetRoom { room });
            actions.push(ClientRuntimeAction::RequestUserList);
        }
        if let Some((room, password)) = self.model.controller.reidentify_intent.take() {
            self.model.controller.last_auth_password_attempt = Some(password.clone());
            actions.push(ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ));
            actions.push(ClientRuntimeAction::RequestControllerAuth { room, password });
        }
        actions
    }
}
