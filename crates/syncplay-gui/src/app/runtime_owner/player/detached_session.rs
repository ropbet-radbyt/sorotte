use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn sync_pending_local_attached_pause_override_from_session(&mut self) {
        let session_pause_state = self
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state());
        let room_pause_state = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playstate_for_attached_player_sync())
            .and_then(|playstate| playstate.paused);
        self.pending_local_attached_pause_override = match (session_pause_state, room_pause_state) {
            (Some(session_pause_state), Some(room_pause_state))
                if room_pause_state != session_pause_state =>
            {
                Some(session_pause_state)
            }
            _ => None,
        };
    }

    pub(in crate::app::runtime_owner) fn sync_manual_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<bool, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(true);
        };
        session
            .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))?;
        let seek_recorded = session.record_manual_seek_to_position(target_position_seconds)?;
        if !seek_recorded {
            return Ok(false);
        }
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(true)
    }

    fn sync_playback_pause_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(), String> {
        self.pending_attached_player_pause_confirmation_pump = None;
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let _ = session.set_playback_paused(target_paused)?;
        session.sync_local_playback_telemetry(Some(target_paused), self.player_position_seconds)?;
        self.sync_pending_local_attached_pause_override_from_session();
        Ok(())
    }

    pub(in crate::app::runtime_owner) fn apply_playback_pause_change_with_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(bool, Option<String>), String> {
        self.pending_attached_player_pause_confirmation_pump = None;
        let mut sync_error = None;
        if !target_paused {
            if self.player_paused_for_cache == Some(true) {
                self.refresh_player_state_impl();
                return Ok((true, None));
            }
            match self.preflight_local_player_unpause_against_detached_session_impl(
                state,
                previous_paused,
            ) {
                Ok(GuiLocalPlayerUnpauseDecision::Block) => {
                    self.player_paused = Some(true);
                    self.refresh_player_state_impl();
                    return Ok((true, None));
                }
                Ok(GuiLocalPlayerUnpauseDecision::Allow) => {
                    let Some(player) = self.player.as_mut() else {
                        return Err(
                            "Playback pause toggle requires a playback runtime connection."
                                .to_owned(),
                        );
                    };
                    player.set_paused(false).map_err(|error| {
                            format!(
                                "Playback pause toggle through the attached player failed while resuming playback: {error}"
                            )
                        })?;
                    self.player_paused = Some(false);
                    self.refresh_player_state_impl();
                    let mut telemetry_synced = false;
                    if let Some(session) = self.session.as_mut() {
                        match session.sync_local_playback_telemetry(
                            Some(false),
                            self.player_position_seconds,
                        ) {
                            Ok(()) => {
                                telemetry_synced = true;
                                if let Err(error) = session.finalize_local_player_unpause_attempt()
                                {
                                    sync_error = Some(error);
                                } else if let Err(error) =
                                    session.emit_immediate_playback_state_update()
                                {
                                    sync_error = Some(error);
                                }
                            }
                            Err(error) => sync_error = Some(error),
                        }
                    }
                    if telemetry_synced {
                        self.sync_pending_local_attached_pause_override_from_session();
                    }
                    return Ok((false, sync_error));
                }
                Ok(GuiLocalPlayerUnpauseDecision::NotApplicable) => {}
                Err(error) => sync_error = Some(error),
            }
        }

        let Some(player) = self.player.as_mut() else {
            return Err("Playback pause toggle requires a playback runtime connection.".to_owned());
        };
        player.set_paused(target_paused).map_err(|error| {
            format!("Playback pause toggle through the attached player failed: {error}")
        })?;
        self.player_paused = Some(target_paused);
        self.refresh_player_state_impl();
        if let Err(error) = self.sync_playback_pause_into_detached_session_impl(
            state,
            previous_paused,
            target_paused,
        ) && sync_error.is_none()
        {
            sync_error = Some(error);
        }
        Ok((target_paused, sync_error))
    }

    fn preflight_local_player_unpause_against_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
    ) -> Result<GuiLocalPlayerUnpauseDecision, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let decision = session.handle_local_player_unpause_attempt()?;
        if decision == GuiLocalPlayerUnpauseDecision::Block {
            session.sync_local_playback_telemetry(Some(true), self.player_position_seconds)?;
        }
        Ok(decision)
    }

    pub(in crate::app::runtime_owner) fn undo_seek_target_position_from_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        Ok(session.pending_undo_seek_target_position())
    }

    pub(in crate::app::runtime_owner) fn commit_undo_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        if !session.commit_undo_seek()? {
            return Err(
                "Playback undo seek is unavailable because no earlier seek target is recorded."
                    .to_owned(),
            );
        }
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(())
    }
}
