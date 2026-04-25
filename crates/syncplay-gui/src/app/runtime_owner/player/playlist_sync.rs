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
                GuiAttachedPlayerRuntimeAction::Paused(paused) => {
                    if let Some(player) = self.player.as_mut() {
                        player.set_paused(paused).map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance pause dispatch failed: {error}"
                            )
                        })?;
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
        state: &SyncplayGuiShellAppState,
        playlist_control_available: bool,
        can_auto_advance_to_next_playlist_item: bool,
    ) -> bool {
        let should_trigger = state.main_window.shared_playlist_enabled
            && playlist_control_available
            && can_auto_advance_to_next_playlist_item
            && self.player_paused == Some(true)
            && self
                .current_player_file_duration_seconds()
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
                });
        let trigger = should_trigger && !self.playlist_auto_advance_eof_latched;
        self.playlist_auto_advance_eof_latched = should_trigger;
        trigger
    }

    pub(in crate::app::runtime_owner) fn apply_pending_playlist_index_reset_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        opened_selected_media: bool,
    ) {
        if !opened_selected_media {
            return;
        }
        if self
            .current_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| !self.current_player_matches_media_target(target))
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

        let Some(player) = self.player.as_mut() else {
            return;
        };

        let mut state_changed = false;
        match player.set_position(reset_target_position_seconds) {
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
            match player.set_paused(true) {
                Ok(()) => {
                    self.player_paused = Some(true);
                    state_changed = true;
                }
                Err(error) if Self::attached_player_playlist_reset_error_is_transient(&error) => {
                    if let Some(session) = self.session.as_mut() {
                        session.note_local_playlist_index_reset_intent(true);
                    }
                    return;
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to pause the attached player for a playlist switch reset: {error}"
                    );
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
        error: &syncplay_player_api::PlayerError,
    ) -> bool {
        let syncplay_player_api::PlayerError::OperationFailed(message) = error else {
            return false;
        };
        let lower = message.to_ascii_lowercase();
        lower.contains("property unavailable") || lower.contains("no file loaded")
    }
}
