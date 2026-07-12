use super::*;

const ATTACHED_UNPAUSE_MIN_OBSERVED_ADVANCEMENT_SECONDS: f64 = 0.01;

impl GuiPersistedConfigRuntimeOwner {
    fn update_pending_attached_room_unpause_observation(
        &mut self,
        requested_paused_state: Option<bool>,
    ) {
        if requested_paused_state == Some(true) {
            self.pending_attached_room_unpause_observation = None;
            return;
        }
        if requested_paused_state != Some(false) {
            return;
        }

        if self.player_paused_for_cache == Some(true) {
            self.pending_attached_room_unpause_observation =
                Some(GuiPendingAttachedRoomUnpauseObservation::CachePaused);
            return;
        }

        let Some(pending_observation) = self.pending_attached_room_unpause_observation else {
            return;
        };
        match pending_observation {
            GuiPendingAttachedRoomUnpauseObservation::CachePaused => {
                self.pending_attached_room_unpause_observation = Some(
                    GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                        baseline_position_seconds: self.player_position_seconds,
                    },
                );
            }
            GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                baseline_position_seconds: None,
            } => {
                if let Some(position_seconds) = self.player_position_seconds {
                    self.pending_attached_room_unpause_observation = Some(
                        GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                            baseline_position_seconds: Some(position_seconds),
                        },
                    );
                }
            }
            GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                baseline_position_seconds: Some(baseline_position_seconds),
            } => {
                if self.player_paused == Some(false)
                    && self
                        .player_position_seconds
                        .is_some_and(|position_seconds| {
                            position_seconds - baseline_position_seconds
                                > ATTACHED_UNPAUSE_MIN_OBSERVED_ADVANCEMENT_SECONDS
                        })
                {
                    self.pending_attached_room_unpause_observation = None;
                }
            }
        }
    }

    pub(super) fn apply_attached_player_runtime_actions_impl(
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
                        self.pending_attached_room_unpause_observation =
                            Some(GuiPendingAttachedRoomUnpauseObservation::CachePaused);
                        continue;
                    }
                    let Some(player) = self.player.as_mut() else {
                        return state_changed;
                    };
                    match player.set_paused(paused) {
                        Ok(()) => {
                            self.player_paused = Some(paused);
                            self.pending_attached_room_unpause_observation = None;
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
                GuiAttachedPlayerRuntimeAction::Coordinator {
                    command_id,
                    command,
                } => {
                    let Some(player) = self.player.as_mut() else {
                        return state_changed;
                    };
                    let result = match command {
                        CoordinatorPlayerCommand::SetPaused(paused) => player.set_paused(paused),
                        CoordinatorPlayerCommand::SetPosition(position_seconds) => {
                            player.set_position((position_seconds + user_offset_seconds).max(0.0))
                        }
                        CoordinatorPlayerCommand::SetPlaybackRate(rate) => {
                            player.set_playback_rate(rate)
                        }
                    };
                    let accepted = result.is_ok();
                    if let Err(error) = result {
                        eprintln!(
                            "warning: attached-player coordinator command failed during {action_description}: {error}"
                        );
                    }
                    if let Some(session) = self.session.as_mut() {
                        session.report_attached_coordinator_command_dispatch(
                            command_id,
                            accepted,
                            system_time_seconds(),
                        );
                    }
                    // Refreshing is safe: only resulting player observations,
                    // never command acceptance, update the visible/applied state.
                    state_changed = true;
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
        let coordinator_owns_sync = self
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .is_some_and(|snapshot| snapshot.transport_telemetry_observed);
        if coordinator_owns_sync {
            let coordinator_actions = self
                .session
                .as_mut()
                .map(|session| session.attached_player_runtime_actions(system_time_seconds()));
            match coordinator_actions {
                Some(Ok(actions)) => {
                    state_changed |= self.apply_attached_player_runtime_actions_impl(
                        actions,
                        "coordinator reconciliation",
                    );
                }
                Some(Err(error)) => eprintln!(
                    "warning: failed to reconcile attached player through client-core coordinator: {error}"
                ),
                None => {}
            }
            // Coordinator revisions are acknowledged only by transport
            // observations; the legacy optimistic playstate cache is disabled.
            self.last_applied_attached_room_playstate = None;
            if state_changed {
                self.refresh_player_state_impl();
            }
            return;
        }
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
        let room_unpause_observation_was_pending =
            self.pending_attached_room_unpause_observation.is_some();
        self.update_pending_attached_room_unpause_observation(requested_sync_paused_state);
        let pending_room_unpause_observation = requested_sync_paused_state == Some(false)
            && self.pending_attached_room_unpause_observation.is_some();
        let room_unpause_recovery_in_progress = requested_sync_paused_state == Some(false)
            && (room_unpause_observation_was_pending || pending_room_unpause_observation);
        let mut sync_paused_state = requested_sync_paused_state;
        if pending_room_unpause_observation {
            sync_paused_state = None;
        }
        let playstate_unchanged = !force
            && !pending_room_unpause_observation
            && self.last_applied_attached_room_playstate.as_ref() == Some(&playstate);
        let initial_room_playstate_sync = self.last_applied_attached_room_playstate.is_none();
        let allow_initial_self_origin_position_sync = force
            && self.player_position_seconds.is_none()
            && initial_room_playstate_sync
            && !room_unpause_recovery_in_progress;
        let allow_initial_remote_position_sync = initial_room_playstate_sync
            && !set_by_is_local_user
            && !room_unpause_recovery_in_progress;
        let user_offset_seconds = self.user_offset_seconds;
        let should_seek_for_room_playstate = force
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
                && (force || self.player_paused != Some(paused))
            {
                let Some(player) = self.player.as_mut() else {
                    self.last_applied_attached_room_playstate = None;
                    return;
                };
                match player.set_paused(paused) {
                    Ok(()) => {
                        if paused {
                            self.player_paused = Some(true);
                            self.pending_attached_room_unpause_observation = None;
                        } else {
                            self.pending_attached_room_unpause_observation = Some(
                                GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                                    baseline_position_seconds: self.player_position_seconds,
                                },
                            );
                        }
                        state_changed = true;
                        // Remote room sync is not local playback intent; mirror it as telemetry
                        // so later pumps do not reinterpret it as a readiness-changing action.
                        if paused
                            && let Some(session) = self.session.as_mut()
                            && let Err(error) = session.sync_local_playback_telemetry(
                                Some(true),
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
        }

        if !state_changed
            && !room_unpause_observation_was_pending
            && self.pending_attached_room_unpause_observation.is_none()
        {
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

        let desired_pause_state_observed = match requested_sync_paused_state {
            Some(false) => {
                self.player_paused_for_cache != Some(true)
                    && self.pending_attached_room_unpause_observation.is_none()
                    && self.player_paused == Some(false)
            }
            Some(true) => self.player_paused == Some(true),
            None => true,
        };
        if !room_playstate_sync_failed && desired_pause_state_observed {
            self.last_applied_attached_room_playstate = Some(playstate);
        }
        if state_changed {
            self.refresh_player_state_impl();
        }
    }
}
