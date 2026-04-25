use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_retry_player_launch_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        let settings = projected_state.configuration.to_stored_settings();
        self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
        self.refresh_player_state();
        let stream_helper_snapshot = self.recheck_stream_helper_runtime_snapshot(projected_state);

        let (level, message) = if self.player.is_some() {
            (
                GuiTransientNotificationLevel::Success,
                "mpv is ready with the current player settings.".to_owned(),
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
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(stream_helper_snapshot),
                GuiShellAction::PushTransientNotification {
                    level,
                    message: message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(message),
            ],
        );
        true
    }

    pub(super) fn handle_undo_seek_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
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
        projected_state: &mut SyncplayGuiShellAppState,
        command: syncplay_client_app::app_boundary::commands::LocalOffsetCommand,
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
        projected_state: &mut SyncplayGuiShellAppState,
        enabled: bool,
    ) -> bool {
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            Self::push_player_error(handle, error);
            return false;
        }
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.set_autoplay_enabled(enabled)
        {
            Self::push_player_error(handle, error);
        }
        true
    }

    pub(super) fn handle_set_autoplay_threshold_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        threshold: usize,
    ) -> bool {
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            Self::push_player_error(handle, error);
            return false;
        }
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.set_autoplay_threshold(threshold)
        {
            Self::push_player_error(handle, error);
        }
        true
    }

    pub(super) fn handle_seek_offset_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
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
        projected_state: &mut SyncplayGuiShellAppState,
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
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let target_paused = !projected_state.main_window.playback_paused;
            let previous_paused = projected_state.main_window.playback_paused;
            match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    let actions = if actual_paused == previous_paused {
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

    pub(super) fn handle_complete_toggle_playback_pause_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        self.refresh_player_state();
        self.ensure_configured_player_attached();
        if self.player.is_some() {
            let target_paused = !projected_state.main_window.playback_paused;
            let previous_paused = projected_state.main_window.playback_paused;
            let actions = match self.apply_playback_pause_change_with_detached_session(
                projected_state,
                previous_paused,
                target_paused,
            ) {
                Ok((_actual_paused, sync_error)) => {
                    if let Some(error) = sync_error {
                        Self::push_player_error(handle, error);
                    }
                    vec![GuiShellAction::CompletePlaybackPauseToggle]
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
