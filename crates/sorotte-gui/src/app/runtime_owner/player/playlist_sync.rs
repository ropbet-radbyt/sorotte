use super::*;

const PLAYLIST_LOAD_NEXT_FILE_MINIMUM_LENGTH_SECONDS: f64 = 10.0;
const PLAYLIST_LOAD_NEXT_FILE_TIME_FROM_END_THRESHOLD_SECONDS: f64 = 5.0;

impl GuiPersistedConfigRuntimeOwner {
    pub(crate) fn advance_playlist_index_for_attached_player_impl(&mut self) -> Result<(), String> {
        let attached_player_actions = {
            let Some(session) = self.session.as_mut() else {
                return Err(
                    "Advancing the shared playlist requires an active session runtime.".to_owned(),
                );
            };
            session.advance_playlist_index_attached_player_actions()?
        };
        if attached_player_actions.is_empty() {
            let Some(session) = self.session.as_mut() else {
                return Err(
                    "Advancing the shared playlist requires an active session runtime.".to_owned(),
                );
            };
            return session.advance_playlist_index();
        }

        for action in attached_player_actions {
            match action {
                GuiAttachedPlayerRuntimeAction::Paused { paused, cause } => {
                    let command_id = self.begin_attached_player_pause_command(paused, cause)?;
                    if let Some(player) = self.player.as_mut() {
                        let result = player.set_paused(paused);
                        let command_succeeded = result.is_ok();
                        let command_result_error = self
                            .finish_attached_player_pause_command(command_id, command_succeeded)
                            .err();
                        if let Err(error) = result {
                            if let Some(command_result_error) = command_result_error {
                                eprintln!(
                                    "warning: failed to register attached-player shared-playlist pause failure: {command_result_error}"
                                );
                            }
                            return Err(format!(
                                "Attached player shared-playlist advance pause dispatch failed: {error}"
                            ));
                        }
                        if let Some(error) = command_result_error {
                            return Err(format!(
                                "Attached player shared-playlist advance pause registration failed: {error}"
                            ));
                        }
                        self.note_local_attached_player_pause_command(paused);
                    } else {
                        self.finish_attached_player_pause_command(command_id, false)?;
                    }
                    self.player_paused = Some(paused);
                }
                GuiAttachedPlayerRuntimeAction::Position(position_seconds) => {
                    let player_target_position_seconds = self
                        .player_target_position_seconds_for_global_position_impl(position_seconds);
                    if let Some(player) = self.player.as_mut() {
                        player
                            .set_position(player_target_position_seconds)
                            .map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance seek dispatch failed: {error}"
                            )
                            })?;
                    }
                    self.player_position_seconds = Some(position_seconds);
                    self.clamp_player_position_to_file_duration();
                }
                GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate) => {
                    if let Some(player) = self.player.as_mut() {
                        player.set_playback_rate(playback_rate).map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance playback-rate dispatch failed: {error}"
                            )
                        })?;
                    }
                }
                action @ GuiAttachedPlayerRuntimeAction::DesyncPlaybackRate { .. } => {
                    let _ = self.apply_attached_player_runtime_actions_impl(
                        vec![action],
                        "shared-playlist advance",
                    );
                }
                GuiAttachedPlayerRuntimeAction::Coordinator {
                    command_id,
                    command,
                } => {
                    let _ = self.apply_attached_player_runtime_actions_impl(
                        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
                            command_id,
                            command,
                        }],
                        "shared-playlist advance",
                    );
                }
            }
        }

        if let Some(session) = self.session.as_mut() {
            session
                .sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        }
        Ok(())
    }

    pub(crate) fn take_playlist_auto_advance_eof_trigger_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
        playlist_control_available: bool,
        can_auto_advance_to_next_playlist_item: bool,
    ) -> bool {
        let should_trigger = self.runtime_shared_playlist_enabled(state)
            && playlist_control_available
            && can_auto_advance_to_next_playlist_item
            && self.attached_player_observation_is_end_of_file();
        let trigger = should_trigger && !self.playlist_auto_advance_eof_latched;
        self.playlist_auto_advance_eof_latched = should_trigger;
        trigger
    }

    pub(in crate::app) fn attached_player_observation_is_end_of_file(&self) -> bool {
        self.player_paused == Some(true) && self.attached_player_position_is_end_of_file()
    }

    pub(in crate::app) fn attached_player_position_is_end_of_file(&self) -> bool {
        self.current_player_file_duration_seconds()
            .filter(|duration_seconds| {
                *duration_seconds > PLAYLIST_LOAD_NEXT_FILE_MINIMUM_LENGTH_SECONDS
            })
            .zip(
                self.player_position_seconds
                    .filter(|position_seconds| position_seconds.is_finite()),
            )
            .is_some_and(|(duration_seconds, position_seconds)| {
                (position_seconds - duration_seconds).abs()
                    < PLAYLIST_LOAD_NEXT_FILE_TIME_FROM_END_THRESHOLD_SECONDS
            })
    }

    pub(in crate::app::runtime_owner) fn apply_pending_playlist_index_reset_to_attached_player_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
        opened_selected_media: bool,
    ) {
        if !opened_selected_media {
            return;
        }
        if self
            .current_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| self.unresolved_attached_media_target.as_deref() == Some(target))
        {
            return;
        }
        if !self.player_local_file_ready_for_attached_sync() {
            return;
        }
        let Some(pause_before_sync) = self
            .session
            .as_mut()
            .and_then(|session| session.take_pending_playlist_index_reset_intent())
        else {
            return;
        };

        self.suppressed_attached_room_playstate_after_playlist_reset = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playstate());
        let reset_target_position_seconds =
            self.player_target_position_seconds_for_global_position_impl(0.0);

        let Some(position_result) = self
            .player
            .as_mut()
            .map(|player| player.set_position(reset_target_position_seconds))
        else {
            return;
        };

        let mut state_changed = false;
        match position_result {
            Ok(()) => {
                self.player_position_seconds = Some(0.0);
                state_changed = true;
            }
            Err(error) if Self::attached_player_playlist_reset_error_is_transient(&error) => {
                if let Some(session) = self.session.as_mut() {
                    session.note_local_playlist_index_reset_intent(pause_before_sync);
                }
                return;
            }
            Err(error) => {
                eprintln!(
                    "warning: failed to rewind the attached player for a playlist switch reset: {error}"
                );
            }
        }
        if pause_before_sync {
            let command_id = match self
                .begin_attached_player_pause_command(true, PlayerCommandCause::PlaylistTransition)
            {
                Ok(command_id) => command_id,
                Err(error) => {
                    eprintln!(
                        "warning: failed to register attached-player playlist reset pause: {error}"
                    );
                    return;
                }
            };
            let result = self
                .player
                .as_mut()
                .expect("playlist reset player was available for the preceding seek")
                .set_paused(true);
            let command_succeeded = result.is_ok();
            let command_result_error = self
                .finish_attached_player_pause_command(command_id, command_succeeded)
                .err();
            match result {
                Ok(()) => {
                    self.note_local_attached_player_pause_command(true);
                    self.player_paused = Some(true);
                    state_changed = true;
                    if let Some(error) = command_result_error {
                        eprintln!(
                            "warning: failed to register attached-player playlist reset pause: {error}"
                        );
                    }
                }
                Err(error) if Self::attached_player_playlist_reset_error_is_transient(&error) => {
                    if let Some(command_result_error) = command_result_error {
                        eprintln!(
                            "warning: failed to register transient attached-player playlist reset pause failure: {command_result_error}"
                        );
                    }
                    if let Some(session) = self.session.as_mut() {
                        session.note_local_playlist_index_reset_intent(true);
                    }
                    return;
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to pause the attached player for a playlist switch reset: {error}"
                    );
                    if let Some(command_result_error) = command_result_error {
                        eprintln!(
                            "warning: failed to register attached-player playlist reset pause failure: {command_result_error}"
                        );
                    }
                }
            }
        }

        if let Some(session) = self.session.as_mut()
            && let Err(error) =
                session.sync_local_playback_telemetry(pause_before_sync.then_some(true), Some(0.0))
        {
            eprintln!(
                "warning: failed to mirror playlist switch reset telemetry into the session runtime: {error}"
            );
        }

        self.last_applied_attached_room_playstate = None;
        if state_changed {
            self.refresh_player_state_impl();
        }
    }

    fn attached_player_playlist_reset_error_is_transient(
        error: &sorotte_player_api::PlayerError,
    ) -> bool {
        let sorotte_player_api::PlayerError::OperationFailed(message) = error else {
            return false;
        };
        let lower = message.to_ascii_lowercase();
        lower.contains("property unavailable") || lower.contains("no file loaded")
    }
}
