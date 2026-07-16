use super::*;
use crate::app::feature_slices::player::Command;
use crate::app::runtime_stack::GuiAttachedPlayerRuntimeAction;
use crate::app::support::system_time_seconds;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn handle_player_command(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        command: Command,
    ) -> bool {
        match command {
            Command::UndoSeek => self.handle_undo_seek_request(handle, projected_state),
            Command::SetOffset(command) => {
                self.handle_set_offset_request(handle, projected_state, command)
            }
            Command::SetAutoplayEnabled(enabled) => {
                self.handle_set_autoplay_enabled_request(handle, projected_state, enabled)
            }
            Command::SetAutoplayThreshold(threshold) => {
                self.handle_set_autoplay_threshold_request(handle, projected_state, threshold)
            }
            Command::RetryLaunch => {
                self.handle_retry_player_launch_request(handle, projected_state)
            }
            Command::SeekOffset(offset_seconds) => {
                self.handle_seek_offset_request(handle, projected_state, offset_seconds)
            }
            Command::SeekToPosition(position_seconds) => {
                self.handle_seek_to_position_request(handle, projected_state, position_seconds)
            }
            Command::KeepWaitingForSeekPreparation => {
                self.handle_keep_waiting_for_seek_preparation_request(handle)
            }
            Command::CancelSeekPreparation => self.handle_cancel_seek_preparation_request(handle),
            Command::JoinNearestBufferedSeekPreparation => {
                self.handle_join_nearest_buffered_seek_preparation_request(handle)
            }
            Command::SetPaused(paused) => {
                self.handle_set_playback_paused_request(handle, projected_state, paused)
            }
            Command::TogglePause => {
                self.handle_toggle_playback_pause_request(handle, projected_state)
            }
        }
    }

    fn finish_seek_preparation_control_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        result: Result<Vec<GuiAttachedPlayerRuntimeAction>, String>,
        description: &str,
    ) -> bool {
        match result {
            Ok(actions) => {
                let state_changed =
                    self.apply_attached_player_runtime_actions_impl(actions, description);
                if state_changed {
                    self.refresh_player_state_impl();
                }
                true
            }
            Err(error) => {
                Self::push_player_error(handle, error);
                true
            }
        }
    }

    fn handle_keep_waiting_for_seek_preparation_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
    ) -> bool {
        let result = self.session.as_mut().map_or_else(
            || Err("No active stream seek preparation is available.".to_owned()),
            |session| session.keep_waiting_for_seek_preparation(system_time_seconds()),
        );
        self.finish_seek_preparation_control_request(handle, result, "keep waiting")
    }

    fn handle_cancel_seek_preparation_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
    ) -> bool {
        let result = self.session.as_mut().map_or_else(
            || Err("No active stream seek preparation is available to cancel.".to_owned()),
            |session| session.cancel_seek_preparation(system_time_seconds()),
        );
        self.finish_seek_preparation_control_request(handle, result, "cancel seek preparation")
    }

    fn handle_join_nearest_buffered_seek_preparation_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
    ) -> bool {
        let result = self.session.as_mut().map_or_else(
            || Err("No safe buffered stream position is available to join.".to_owned()),
            |session| session.join_nearest_buffered_seek_preparation(system_time_seconds()),
        );
        self.finish_seek_preparation_control_request(handle, result, "join nearest buffered")
    }

    pub(super) fn handle_retry_player_launch_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let settings = projected_state.saved_configuration.clone();
        self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
        self.refresh_player_state();
        let stream_helper_snapshot = self.recheck_stream_helper_runtime_snapshot(projected_state);
        let player_settings_applied = self.current_player_launch_state_is_applied();

        let (level, message) = if self.player.is_some() {
            (
                GuiTransientNotificationLevel::Success,
                "mpv is ready with the current player settings.".to_owned(),
            )
        } else if player_settings_applied
            && matches!(self.player_launch_state, GuiPlayerLaunchRuntimeState::None)
        {
            (
                GuiTransientNotificationLevel::Success,
                "The saved configuration has no active player.".to_owned(),
            )
        } else {
            (
                GuiTransientNotificationLevel::Error,
                self.player_unavailability_reason
                    .clone()
                    .unwrap_or_else(|| {
                        "Retrying mpv launch did not attach a playback runtime.".to_owned()
                    }),
            )
        };
        let mut actions = vec![
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(stream_helper_snapshot),
            GuiShellAction::PushTransientNotification {
                level,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ];
        if player_settings_applied {
            self.promote_restart_player_runtime_fields(&settings);
            actions.push(self.pending_apply_requirements_action(projected_state, &settings));
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        true
    }

    pub(super) fn handle_undo_seek_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            Self::push_runtime_unavailable(
                handle,
                "Playback undo seek requires a playback runtime connection.".to_owned(),
            );
            return false;
        }
        match self.undo_seek_target_position_from_detached_session(projected_state) {
            Ok(Some(target_position_seconds)) => {
                let _ = self.interrupt_attached_playback_recovery_impl("manual undo seek");
                let player_target_position_seconds = self
                    .player_target_position_seconds_for_global_position(target_position_seconds);
                let (player_name, undo_result) = {
                    let Some(player) = self.player.as_mut() else {
                        Self::push_runtime_unavailable(
                            handle,
                            "Playback undo seek requires a playback runtime connection.".to_owned(),
                        );
                        return false;
                    };
                    (
                        player.name(),
                        player.set_position(player_target_position_seconds),
                    )
                };
                match undo_result {
                    Ok(()) => {
                        let commit_result = self.commit_undo_seek_into_detached_session(
                            projected_state,
                            target_position_seconds,
                        );
                        self.player_position_seconds = Some(target_position_seconds);
                        self.refresh_player_state();
                        match commit_result {
                            Ok(()) => Self::push_player_success(
                                handle,
                                format!(
                                    "Undo seek applied via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                ),
                            ),
                            Err(error) => Self::push_player_error(handle, error),
                        }
                    }
                    Err(error) => Self::push_player_error(
                        handle,
                        format!(
                            "Playback undo seek through the attached {player_name} player failed: {error}"
                        ),
                    ),
                }
            }
            Ok(None) => Self::push_player_error(
                handle,
                "Playback undo seek is unavailable because no earlier seek target is recorded."
                    .to_owned(),
            ),
            Err(error) => Self::push_player_error(handle, error),
        }
        true
    }

    pub(super) fn handle_set_offset_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        command: sorotte_client_app::app_boundary::commands::LocalOffsetCommand,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
        let dispatch = plan_local_offset_runtime_dispatch_legacy_compatible(
            self.user_offset_seconds,
            previous_position_seconds,
            &command,
            None,
        );
        let Some(PlannedLocalRuntimeAction::SeekToPosition(target_player_position_seconds)) =
            dispatch.action
        else {
            return false;
        };
        self.user_offset_seconds = dispatch
            .updated_user_offset_seconds
            .unwrap_or(self.user_offset_seconds);
        self.player_position_seconds = Some(previous_position_seconds);
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            Self::push_player_error(handle, error);
            return false;
        }
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session
                .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))
        {
            Self::push_player_error(handle, error);
        }
        let message = dispatch.line_to_emit.unwrap_or_else(|| {
            localized_current_offset_message_legacy_compatible(self.user_offset_seconds, None)
        });
        let _ = self.interrupt_attached_playback_recovery_impl("local offset change");
        if let Some(player) = self.player.as_mut() {
            let player_name = player.name();
            match player.set_position(target_player_position_seconds) {
                Ok(()) => {
                    self.refresh_player_state();
                    Self::push_player_success(
                        handle,
                        format!("{message} Applied via the attached {player_name} player."),
                    );
                }
                Err(error) => Self::push_player_error(
                    handle,
                    format!(
                        "{message} Applying it to the attached {player_name} player failed: {error}"
                    ),
                ),
            }
        } else {
            Self::push_player_success(handle, message);
        }
        true
    }

    pub(super) fn handle_set_autoplay_enabled_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            Self::push_player_error(handle, error);
            return false;
        }
        if let Some(session) = self.session.as_mut() {
            match session.set_autoplay_enabled(enabled) {
                Ok(()) => {
                    if let Some(runtime_settings) = self.active_session_settings.as_mut() {
                        runtime_settings.settings.autoplay_initial_state = Some(enabled);
                        runtime_settings.config.readiness.autoplay_initial_state = enabled;
                    }
                }
                Err(error) => Self::push_player_error(handle, error),
            }
        }
        true
    }

    pub(super) fn handle_set_autoplay_threshold_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        threshold: usize,
    ) -> bool {
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            Self::push_player_error(handle, error);
            return false;
        }
        if let Some(session) = self.session.as_mut() {
            match session.set_autoplay_threshold(threshold) {
                Ok(()) => {
                    let threshold =
                        sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(
                            threshold,
                        );
                    if let Some(runtime_settings) = self.active_session_settings.as_mut() {
                        runtime_settings.settings.autoplay_min_users = Some(threshold.clone());
                        runtime_settings.config.readiness.autoplay_min_users = threshold;
                    }
                }
                Err(error) => Self::push_player_error(handle, error),
            }
        }
        true
    }

    pub(super) fn handle_seek_offset_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        offset_seconds: f64,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
            let target_position_seconds = (previous_position_seconds + offset_seconds).max(0.0);
            if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
                Self::push_player_error(handle, error);
                return false;
            }
            if let Some(session) = self.session.as_ref() {
                match session.manual_seek_to_position_allowed(target_position_seconds) {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(error) => {
                        Self::push_player_error(handle, error);
                        return false;
                    }
                }
            }
            let _ = self.interrupt_attached_playback_recovery_impl("manual relative seek");
            let player_target_position_seconds =
                self.player_target_position_seconds_for_global_position(target_position_seconds);
            let (player_name, seek_result) = {
                let Some(player) = self.player.as_mut() else {
                    Self::push_runtime_unavailable(
                        handle,
                        self.seek_unavailable_message(offset_seconds),
                    );
                    return false;
                };
                (
                    player.name(),
                    player.set_position(player_target_position_seconds),
                )
            };
            match seek_result {
                Ok(()) => {
                    self.player_position_seconds = Some(target_position_seconds);
                    self.refresh_player_state();
                    match self.sync_manual_seek_into_detached_session(
                        projected_state,
                        previous_position_seconds,
                        target_position_seconds,
                    ) {
                        Ok(true) => {}
                        Ok(false) => return false,
                        Err(error) => Self::push_player_error(handle, error),
                    }
                    Self::push_player_success(
                        handle,
                        format!(
                            "Applied a {offset_seconds} second seek via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                        ),
                    );
                }
                Err(error) => {
                    Self::push_player_error(
                        handle,
                        format!(
                            "Playback seek through the attached {player_name} player failed: {error}"
                        ),
                    );
                }
            }
        } else {
            Self::push_runtime_unavailable(handle, self.seek_unavailable_message(offset_seconds));
        }
        true
    }

    pub(super) fn handle_seek_to_position_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target_position_seconds: f64,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
            let target_position_seconds = target_position_seconds.max(0.0);
            if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
                Self::push_player_error(handle, error);
                return false;
            }
            if let Some(session) = self.session.as_ref() {
                match session.manual_seek_to_position_allowed(target_position_seconds) {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(error) => {
                        Self::push_player_error(handle, error);
                        return false;
                    }
                }
            }
            let _ = self.interrupt_attached_playback_recovery_impl("manual absolute seek");
            let player_target_position_seconds =
                self.player_target_position_seconds_for_global_position(target_position_seconds);
            let (player_name, seek_result) = {
                let Some(player) = self.player.as_mut() else {
                    Self::push_runtime_unavailable(
                        handle,
                        "Playback absolute seek requires a playback runtime connection.".to_owned(),
                    );
                    return false;
                };
                (
                    player.name(),
                    player.set_position(player_target_position_seconds),
                )
            };
            match seek_result {
                Ok(()) => {
                    self.player_position_seconds = Some(target_position_seconds);
                    self.refresh_player_state();
                    match self.sync_manual_seek_into_detached_session(
                        projected_state,
                        previous_position_seconds,
                        target_position_seconds,
                    ) {
                        Ok(true) => {}
                        Ok(false) => return false,
                        Err(error) => Self::push_player_error(handle, error),
                    }
                    Self::push_player_success(
                        handle,
                        format!(
                            "Applied an absolute seek via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                        ),
                    );
                }
                Err(error) => {
                    Self::push_player_error(
                        handle,
                        format!(
                            "Playback seek through the attached {player_name} player failed: {error}"
                        ),
                    );
                }
            }
        } else {
            Self::push_runtime_unavailable(
                handle,
                "Playback absolute seek requires a playback runtime connection.".to_owned(),
            );
        }
        true
    }

    pub(super) fn handle_toggle_playback_pause_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_paused = self
                .player_paused
                .unwrap_or(projected_state.main_window.playback_paused);
            let target_paused = !previous_paused;
            match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    let actions = if projected_state.main_window.playback_paused == actual_paused {
                        Vec::new()
                    } else {
                        vec![if actual_paused {
                            GuiShellAction::AnnouncePlaybackPaused
                        } else {
                            GuiShellAction::AnnouncePlaybackResumed
                        }]
                    };
                    Self::push_actions_and_project(handle, projected_state, actions);
                }
                Err(error) => Self::push_player_error(handle, error),
            }
        } else {
            Self::push_runtime_unavailable(handle, self.toggle_pause_unavailable_message());
        }
        true
    }

    pub(super) fn handle_set_playback_paused_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target_paused: bool,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_paused = self
                .player_paused
                .unwrap_or(projected_state.main_window.playback_paused);
            match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    let actions = if projected_state.main_window.playback_paused == actual_paused {
                        Vec::new()
                    } else {
                        vec![if actual_paused {
                            GuiShellAction::AnnouncePlaybackPaused
                        } else {
                            GuiShellAction::AnnouncePlaybackResumed
                        }]
                    };
                    Self::push_actions_and_project(handle, projected_state, actions);
                }
                Err(error) => Self::push_player_error(handle, error),
            }
        } else {
            Self::push_runtime_unavailable(handle, self.toggle_pause_unavailable_message());
        }
        true
    }

    pub(super) fn handle_complete_set_playback_pause_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target_paused: bool,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_paused = self
                .player_paused
                .unwrap_or(projected_state.main_window.playback_paused);
            let actions = match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    vec![GuiShellAction::CompletePlaybackPauseState(actual_paused)]
                }
                Err(error) => vec![
                    GuiShellAction::CancelPlaybackPauseState,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    },
                ],
            };
            Self::push_actions_and_project(handle, projected_state, actions);
        } else {
            let actions = vec![
                GuiShellAction::CancelPlaybackPauseState,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: self.toggle_pause_unavailable_message(),
                },
            ];
            Self::push_actions_and_project(handle, projected_state, actions);
        }
        true
    }

    pub(super) fn handle_complete_toggle_playback_pause_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let previous_paused = self
                .player_paused
                .unwrap_or(projected_state.main_window.playback_paused);
            let target_paused = !previous_paused;
            let actions = match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    vec![GuiShellAction::CompletePlaybackPauseState(actual_paused)]
                }
                Err(error) => vec![
                    GuiShellAction::CancelPlaybackPauseToggle,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    },
                ],
            };
            Self::push_actions_and_project(handle, projected_state, actions);
        } else {
            let actions = vec![
                GuiShellAction::CancelPlaybackPauseToggle,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: self.toggle_pause_unavailable_message(),
                },
            ];
            Self::push_actions_and_project(handle, projected_state, actions);
        }
        true
    }
}
