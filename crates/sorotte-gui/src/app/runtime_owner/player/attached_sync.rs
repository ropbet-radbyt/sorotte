use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn apply_attached_player_runtime_actions_impl(
        &mut self,
        actions: Vec<GuiAttachedPlayerRuntimeAction>,
        action_description: &str,
    ) -> bool {
        let mut state_changed = false;
        let user_offset_seconds = self.user_offset_seconds;
        for action in actions {
            match action {
                GuiAttachedPlayerRuntimeAction::Paused(paused) => {
                    if self.player_paused_for_cache == Some(true) && !paused {
                        self.pending_attached_cache_unpause = true;
                        continue;
                    }
                    let Some(player) = self.player.as_mut() else {
                        return state_changed;
                    };
                    match player.set_paused(paused) {
                        Ok(()) => {
                            self.player_paused = Some(paused);
                            if !paused {
                                self.pending_attached_cache_unpause = false;
                            }
                            state_changed = true;
                            if let Some(session) = self.session.as_mut()
                                && let Err(error) = session.sync_local_playback_telemetry(
                                    Some(paused),
                                    self.player_position_seconds,
                                )
                            {
                                eprintln!(
                                    "warning: failed to mirror attached-player {action_description} pause action into the session runtime: {error}"
                                );
                            }
                        }
                        Err(error) => {
                            eprintln!(
                                "warning: failed to apply attached-player {action_description} pause action: {error}"
                            );
                        }
                    }
                }
                GuiAttachedPlayerRuntimeAction::Position(position_seconds) => {
                    let sync_position_seconds = (position_seconds + user_offset_seconds).max(0.0);
                    let Some(player) = self.player.as_mut() else {
                        return state_changed;
                    };
                    match player.set_position(sync_position_seconds) {
                        Ok(()) => {
                            self.player_position_seconds = Some(position_seconds);
                            state_changed = true;
                            if let Some(session) = self.session.as_mut()
                                && let Err(error) = session.sync_local_playback_telemetry(
                                    self.player_paused,
                                    Some(position_seconds),
                                )
                            {
                                eprintln!(
                                    "warning: failed to mirror attached-player {action_description} position action into the session runtime: {error}"
                                );
                            }
                        }
                        Err(error) => {
                            eprintln!(
                                "warning: failed to apply attached-player {action_description} position action: {error}"
                            );
                        }
                    }
                }
                GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate) => {
                    let Some(player) = self.player.as_mut() else {
                        return state_changed;
                    };
                    if let Err(error) = player.set_playback_rate(playback_rate) {
                        eprintln!(
                            "warning: failed to apply attached-player {action_description} playback-rate action: {error}"
                        );
                    }
                }
            }
        }
        state_changed
    }

    pub(in crate::app::runtime_owner) fn sync_session_playstate_to_attached_player_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
        force: bool,
    ) {
        if self
            .current_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| self.unresolved_attached_media_target.as_deref() == Some(target))
        {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        if self.player.is_none() {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        if !self.player_local_file_ready_for_attached_sync() {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        let local_runtime_actions = self
            .session
            .as_mut()
            .map(|session| session.take_attached_player_local_runtime_actions());
        let mut state_changed = match local_runtime_actions {
            Some(Ok(actions)) => {
                self.apply_attached_player_runtime_actions_impl(actions, "local runtime")
            }
            Some(Err(error)) => {
                eprintln!(
                    "warning: failed to drain attached-player local runtime actions: {error}"
                );
                false
            }
            None => false,
        };
        let Some((playstate, raw_playstate, local_username)) =
            self.session.as_ref().and_then(|session| {
                session
                    .current_room_playstate_for_attached_player_sync()
                    .map(|playstate| {
                        (
                            playstate,
                            session.current_room_playstate(),
                            session.local_username().map(str::to_owned),
                        )
                    })
            })
        else {
            self.last_applied_attached_room_playstate = None;
            if state_changed {
                self.refresh_player_state_impl();
            }
            return;
        };
        if let Some(suppressed_playstate) = self
            .suppressed_attached_room_playstate_after_playlist_reset
            .as_ref()
        {
            if raw_playstate.as_ref() == Some(suppressed_playstate) {
                return;
            }
            self.suppressed_attached_room_playstate_after_playlist_reset = None;
        }
        let set_by_is_local_user = playstate
            .set_by
            .as_deref()
            .zip(local_username.as_deref())
            .is_some_and(|(set_by, local_username)| set_by == local_username);
        if self.pending_local_attached_pause_override == playstate.paused {
            self.pending_local_attached_pause_override = None;
        }
        let suppress_stale_room_pause_sync = self
            .pending_local_attached_pause_override
            .is_some_and(|pending_paused| playstate.paused != Some(pending_paused));
        let requested_sync_paused_state = (!suppress_stale_room_pause_sync)
            .then_some(playstate.paused)
            .flatten();
        let pending_cache_unpause_ready = self.pending_attached_cache_unpause
            && self.player_paused_for_cache != Some(true)
            && requested_sync_paused_state == Some(false);
        let mut sync_paused_state = requested_sync_paused_state;
        if self.player_paused_for_cache == Some(true) && sync_paused_state == Some(false) {
            self.pending_attached_cache_unpause = true;
            sync_paused_state = None;
        } else if sync_paused_state == Some(true) {
            self.pending_attached_cache_unpause = false;
        }
        let playstate_unchanged = !force
            && !pending_cache_unpause_ready
            && self.last_applied_attached_room_playstate.as_ref() == Some(&playstate);
        let initial_room_playstate_sync = self.last_applied_attached_room_playstate.is_none();
        let allow_initial_self_origin_position_sync =
            force && self.player_position_seconds.is_none() && initial_room_playstate_sync;
        let allow_initial_remote_position_sync =
            initial_room_playstate_sync && !set_by_is_local_user;
        let user_offset_seconds = self.user_offset_seconds;
        let should_seek_for_room_playstate = force
            || pending_cache_unpause_ready
            || playstate.do_seek == Some(true)
            || sync_paused_state == Some(true)
            || allow_initial_remote_position_sync;

        let mut room_playstate_sync_failed = false;
        if !playstate_unchanged {
            if let Some(position_seconds) = playstate.position_seconds
                && (!set_by_is_local_user || allow_initial_self_origin_position_sync)
                && should_seek_for_room_playstate
                && (force
                    || self
                        .player_position_seconds
                        .map(|current_position_seconds| {
                            (current_position_seconds - position_seconds).abs() > f64::EPSILON
                        })
                        .unwrap_or(true))
            {
                let sync_position_seconds = (position_seconds + user_offset_seconds).max(0.0);
                let Some(player) = self.player.as_mut() else {
                    self.last_applied_attached_room_playstate = None;
                    return;
                };
                match player.set_position(sync_position_seconds) {
                    Ok(()) => {
                        self.player_position_seconds = Some(position_seconds);
                        state_changed = true;
                    }
                    Err(error) => {
                        room_playstate_sync_failed = true;
                        eprintln!(
                            "warning: failed to sync session playback position to the attached player: {error}"
                        );
                    }
                }
            }

            if let Some(paused) = sync_paused_state
                && (force || pending_cache_unpause_ready || self.player_paused != Some(paused))
            {
                let Some(player) = self.player.as_mut() else {
                    self.last_applied_attached_room_playstate = None;
                    return;
                };
                match player.set_paused(paused) {
                    Ok(()) => {
                        self.player_paused = Some(paused);
                        if !paused {
                            self.pending_attached_cache_unpause = false;
                        }
                        state_changed = true;
                        // Remote room sync is not local playback intent; mirror it as telemetry
                        // so later pumps do not reinterpret it as a readiness-changing action.
                        if let Some(session) = self.session.as_mut()
                            && let Err(error) = session.sync_local_playback_telemetry(
                                Some(paused),
                                self.player_position_seconds,
                            )
                        {
                            eprintln!(
                                "warning: failed to mirror remote room pause sync into the session runtime: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        room_playstate_sync_failed = true;
                        eprintln!(
                            "warning: failed to sync session playback pause state to the attached player: {error}"
                        );
                    }
                }
            }

            if !room_playstate_sync_failed {
                self.last_applied_attached_room_playstate = Some(playstate.clone());
            }
        }

        if !state_changed {
            let attached_runtime_actions = self
                .session
                .as_mut()
                .map(|session| session.attached_player_runtime_actions(system_time_seconds()));
            match attached_runtime_actions {
                Some(Ok(actions)) => {
                    state_changed |=
                        self.apply_attached_player_runtime_actions_impl(actions, "correction");
                }
                Some(Err(error)) => {
                    eprintln!(
                        "warning: failed to evaluate attached-player desync correction actions: {error}"
                    );
                }
                None => {}
            }
        }

        if !room_playstate_sync_failed && self.last_applied_attached_room_playstate.is_none() {
            self.last_applied_attached_room_playstate = Some(playstate);
        }
        if state_changed {
            self.refresh_player_state_impl();
        }
    }
}
