//! Local transport intent owns staged pause mutations, connection authority and
//! native-play fences. Inputs are explicit gestures, canonical echoes and accepted
//! physical observations; outputs are scoped mutations and causal command records.
//! Room/media changes retire intent; reconnect retains only dormant intent until
//! fresh authority authorizes it. Physical player identity stays with its adapter.
use super::*;

impl RuntimePlaybackCoordination {
    pub(super) fn bind_local_control_authority_context(
        &mut self,
        session: &ClientSession,
        controlled_freshness: LocalControlAuthorityFreshness,
    ) {
        let room = session.room().unwrap_or_default().to_owned();
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            freshness: if controlled_room_name(&room) {
                controlled_freshness
            } else {
                LocalControlAuthorityFreshness::Authorized
            },
            room,
            username: session.username().map(str::to_owned),
            connection_generation: self.connection_generation,
        });
    }

    pub(super) fn local_control_authority_freshness(
        &self,
        session: &ClientSession,
    ) -> Option<LocalControlAuthorityFreshness> {
        let authority = self.local_control_authority.as_ref()?;
        (authority.connection_generation == self.connection_generation
            && Some(authority.room.as_str()) == session.room()
            && authority.username.as_deref() == session.username())
        .then_some(authority.freshness)
    }

    pub(super) fn current_connection_local_control_is_authorized(
        &self,
        session: &ClientSession,
    ) -> bool {
        match self.local_control_authority_freshness(session) {
            Some(LocalControlAuthorityFreshness::Authorized) => true,
            Some(
                LocalControlAuthorityFreshness::Awaiting | LocalControlAuthorityFreshness::Denied,
            ) => false,
            // Coordination used directly in lower-level tests has no
            // connection lifecycle binding. Preserve that compatibility
            // path, but never fall back to a cached session projection once
            // connection-scoped authority tracking has begun.
            None if self.local_control_authority.is_none() => {
                session.local_can_control() == Some(true)
            }
            None => false,
        }
    }

    pub(super) fn project_position_observation_to(
        &self,
        observed_at_seconds: f64,
    ) -> Option<LocalPositionObservation> {
        let observation = self.latest_observation.as_ref()?;
        let mut position = self.latest_position_observation?;
        if observation.media_generation != position.media_generation
            || !observed_at_seconds.is_finite()
            || observed_at_seconds < position.observed_at_seconds
            || observation.seeking == Some(true)
        {
            return None;
        }
        let elapsed_seconds = observed_at_seconds - position.observed_at_seconds;
        let advancing = observation.phase
            == Some(sorotte_player_api::PlayerTransportPhase::Playing)
            && observation.logical_pause == Some(false)
            && observation.paused_for_cache != Some(true)
            && observation.core_idle != Some(true)
            && position.playback_rate.is_finite()
            && position.playback_rate > 0.0;
        let stationary = observation.logical_pause == Some(true)
            || observation.paused_for_cache == Some(true)
            || observation.core_idle == Some(true)
            || matches!(
                observation.phase,
                Some(
                    sorotte_player_api::PlayerTransportPhase::ReadyPaused
                        | sorotte_player_api::PlayerTransportPhase::Prebuffering
                        | sorotte_player_api::PlayerTransportPhase::Rebuffering
                )
            );
        if advancing {
            let actual_sample_age_seconds =
                observed_at_seconds - position.last_actual_position_observed_at_seconds;
            if !actual_sample_age_seconds.is_finite()
                || !(0.0..=MAX_DESYNC_POSITION_SAMPLE_AGE_SECONDS)
                    .contains(&actual_sample_age_seconds)
            {
                return None;
            }
            position.position_seconds += elapsed_seconds * position.playback_rate;
        } else if !stationary {
            return None;
        }
        position.observed_at_seconds = observed_at_seconds;
        Some(position)
    }

    pub(crate) fn projected_local_position_at(
        &self,
        external_now_seconds: f64,
        legacy_position_seconds: Option<f64>,
    ) -> Option<f64> {
        if self.awaiting_ordered_snapshot {
            return None;
        }
        if matches!(
            self.external_player_availability,
            Some(
                ExternalPlayerAvailability::Unavailable
                    | ExternalPlayerAvailability::Disconnected
                    | ExternalPlayerAvailability::Failed,
            )
        ) {
            return None;
        }
        if !self.transport_telemetry_observed {
            // The session-model fallback exists only for adapters that never
            // produced accepted rich transport telemetry. A lifecycle or
            // adapter-epoch fence must not resurrect that retired adapter's
            // cached position through the legacy model. A genuinely legacy
            // adapter may still use the long-standing session-model path.
            return if participant_status_legacy_position_fallback(
                self.external_player_availability,
                self.transport_telemetry_ever_observed,
            ) {
                legacy_position_seconds
            } else {
                None
            };
        }
        let observation = self.latest_observation.as_ref()?;
        let position = self.latest_position_observation?;
        if self.coordinator.current_media_generation() != Some(position.media_generation)
            || observation.media_generation != position.media_generation
            || observation.paused_for_cache == Some(true)
            || observation.seeking == Some(true)
        {
            return None;
        }
        if observation.phase == Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused)
            && observation.logical_pause == Some(true)
        {
            return Some(position.position_seconds);
        }
        if observation.phase != Some(sorotte_player_api::PlayerTransportPhase::Playing)
            || observation.logical_pause == Some(true)
            || observation.core_idle == Some(true)
        {
            return None;
        }
        let playback_rate = position.playback_rate;
        if !playback_rate.is_finite() || playback_rate <= 0.0 {
            return None;
        }
        let sample_age_seconds = self.coordinator_now(external_now_seconds)
            - position.last_actual_position_observed_at_seconds;
        if !sample_age_seconds.is_finite()
            || !(0.0..=MAX_DESYNC_POSITION_SAMPLE_AGE_SECONDS).contains(&sample_age_seconds)
        {
            return None;
        }
        let projection_elapsed_seconds =
            self.coordinator_now(external_now_seconds) - position.observed_at_seconds;
        if !projection_elapsed_seconds.is_finite() || projection_elapsed_seconds < 0.0 {
            return None;
        }
        Some(position.position_seconds + projection_elapsed_seconds * playback_rate)
    }

    /// Returns a position anchor for an already-authorized local Play/Pause
    /// mutation when ordinary transport projection is temporarily blocked.
    ///
    /// Rebuffering and cache-pause telemetry must remain ineligible for
    /// periodic room-state publication, but it must not erase a distinct user
    /// command. The mutation is fenced separately by room, connection, media
    /// generation, and transport revision; this helper additionally requires
    /// the position sample to belong to the current media generation and to be
    /// recent. It never turns the sample into a seek.
    pub(crate) fn local_pause_mutation_position_at(
        &self,
        external_now_seconds: f64,
        legacy_position_seconds: Option<f64>,
    ) -> Option<f64> {
        if self.awaiting_ordered_snapshot {
            return None;
        }
        if matches!(
            self.external_player_availability,
            Some(
                ExternalPlayerAvailability::Unavailable
                    | ExternalPlayerAvailability::Disconnected
                    | ExternalPlayerAvailability::Failed,
            )
        ) {
            return None;
        }
        if !self.transport_telemetry_observed {
            return if participant_status_legacy_position_fallback(
                self.external_player_availability,
                self.transport_telemetry_ever_observed,
            ) {
                legacy_position_seconds.filter(|position| position.is_finite() && *position >= 0.0)
            } else {
                None
            };
        }

        let observation = self.latest_observation.as_ref()?;
        let position = self.latest_position_observation?;
        let current_media_generation = self.coordinator.current_media_generation()?;
        if observation.media_generation != current_media_generation
            || position.media_generation != current_media_generation
            || observation.seeking == Some(true)
        {
            return None;
        }

        let coordinator_now = self.coordinator_now(external_now_seconds);
        let sample_age_seconds =
            coordinator_now - position.last_actual_position_observed_at_seconds;
        if !sample_age_seconds.is_finite()
            || !(0.0..=MAX_DESYNC_POSITION_SAMPLE_AGE_SECONDS).contains(&sample_age_seconds)
        {
            return None;
        }

        self.project_position_observation_to(coordinator_now)
            .map(|position| position.position_seconds)
            .filter(|position| position.is_finite() && *position >= 0.0)
    }

    pub(crate) fn stage_local_pause_intent(&mut self, paused: bool, session: &ClientSession) {
        let Some(room) = session.room() else {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            return;
        };
        let Some(local_media_generation) = self.coordinator.current_media_generation() else {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            return;
        };
        let controlled_room = controlled_room_name(room);
        let authorization =
            if !controlled_room || self.current_connection_local_control_is_authorized(session) {
                LocalIntentAuthorization::Authorized
            } else {
                match self.local_control_authority_freshness(session) {
                    Some(LocalControlAuthorityFreshness::Denied) => {
                        // Fresh, correlated denial is conclusive for this
                        // connection. Do not retain a command that could replay
                        // after an unrelated authority transition.
                        self.pending_local_pause_intent = None;
                        self.last_local_pause_intent_stage_accepted = Some(false);
                        return;
                    }
                    Some(LocalControlAuthorityFreshness::Awaiting) => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None if self.local_control_authority.is_some() => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None if session.local_can_control().is_none() => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None => {
                        // A direct coordination caller with a known
                        // non-controller projection has no connection-scoped
                        // evidence channel, so reject immediately.
                        self.pending_local_pause_intent = None;
                        self.last_local_pause_intent_stage_accepted = Some(false);
                        return;
                    }
                    Some(LocalControlAuthorityFreshness::Authorized) => {
                        unreachable!("authorized authority was handled above")
                    }
                }
            };
        self.pending_local_pause_intent = Some(PendingLocalPauseIntent {
            paused,
            room: room.to_owned(),
            local_media_generation,
            connection_generation: self.connection_generation,
            base_transport_revision: session.current_room_transport_revision(),
            authorization,
            replay_player_after_reauthorization: authorization
                == LocalIntentAuthorization::AwaitingControlledRoomReauthentication,
            last_canonical_playstate_updated_at_seconds: session
                .model
                .room
                .playstate_updated_at_seconds
                .get(room)
                .copied(),
            mismatching_canonical_playstate_updates: 0,
            first_mismatching_canonical_playstate_at_seconds: None,
        });
        self.last_local_pause_intent_stage_accepted = Some(true);
    }

    pub(super) fn has_active_local_pause_intent(
        &self,
        paused: bool,
        session: &ClientSession,
    ) -> bool {
        self.active_local_pause_intent(session) == Some(paused)
    }

    pub(crate) fn active_local_pause_intent(&self, session: &ClientSession) -> Option<bool> {
        self.pending_local_pause_intent
            .as_ref()
            .filter(|intent| {
                intent.connection_generation == self.connection_generation
                    && intent.authorization == LocalIntentAuthorization::Authorized
                    && session.room() == Some(intent.room.as_str())
                    && self.coordinator.current_media_generation()
                        == Some(intent.local_media_generation)
                    && session.current_room_transport_revision() == intent.base_transport_revision
            })
            .map(|intent| intent.paused)
    }

    pub(super) fn room_authority_may_accept_local_pause_intent(
        &self,
        session: &ClientSession,
        authority: RoomPlaystateAuthority,
        paused: bool,
    ) -> bool {
        match authority {
            RoomPlaystateAuthority::LegacyRemoteUser | RoomPlaystateAuthority::LegacyLocalEcho => {
                true
            }
            RoomPlaystateAuthority::ServerBarrier {
                media_generation, ..
            } => {
                let controller_can_decide_awaiting_v1 =
                    session.playback_barrier_status().is_some_and(|status| {
                        status.media_generation == media_generation
                            && status.phase == PlaybackBarrierPhase::AwaitingDecision
                    });
                session.local_can_control().unwrap_or(false)
                    && (controller_can_decide_awaiting_v1
                        || (session.server_readiness_v2_supported()
                            && !self.readiness_gate_holds_current_playback(session)))
            }
            // Room buffering may delay or reject a user Play, but a user Pause
            // is a monotonic safety transition: it cannot violate a server
            // pause and must not be converted back into Play while its player
            // observation and canonical echo are still racing.
            RoomPlaystateAuthority::ServerBufferingPolicy { .. } => paused,
        }
    }

    /// Returns the authorized semantic pause command that may mutate canonical
    /// room state. This is deliberately distinct from the most recent player
    /// observation: mpv can still report the preceding edge while a command is
    /// in flight, and that stale sample must not impersonate the user's intent.
    pub(crate) fn active_local_pause_state_mutation_intent(
        &self,
        session: &ClientSession,
    ) -> Option<crate::session::LocalPauseMutationIntent> {
        self.active_local_pause_state_mutation_intent_for_revision(
            session,
            session.current_room_transport_revision(),
        )
    }

    pub(crate) fn active_local_pause_state_mutation_intent_for_inbound_transport(
        &mut self,
        session: &ClientSession,
        inbound_transport_revision: Option<u64>,
        inbound_do_seek: bool,
    ) -> Option<crate::session::LocalPauseMutationIntent> {
        let current_transport_revision = session.current_room_transport_revision();
        if current_transport_revision.is_none()
            && !inbound_do_seek
            && let Some(inbound_transport_revision) =
                inbound_transport_revision.filter(|revision| *revision != 0)
        {
            let connection_generation = self.connection_generation;
            let local_media_generation = self.coordinator.current_media_generation();
            if let Some(intent) = self.pending_local_pause_intent.as_mut()
                && intent.base_transport_revision.is_none()
                && intent.connection_generation == connection_generation
                && intent.authorization == LocalIntentAuthorization::Authorized
                && session.room() == Some(intent.room.as_str())
                && local_media_generation == Some(intent.local_media_generation)
            {
                // A user can press Play/Pause after joining but before the
                // first tagged State reaches this client. Bind that already
                // staged command to the first non-seek authority revision so
                // it is emitted with an optimistic-concurrency token instead
                // of disappearing when the baseline is applied. A genuine
                // canonical Seek remains system-owned and supersedes it.
                intent.base_transport_revision = Some(inbound_transport_revision);
            }
        }

        self.active_local_pause_state_mutation_intent_for_revision(
            session,
            current_transport_revision.or(inbound_transport_revision),
        )
    }

    pub(super) fn active_local_pause_state_mutation_intent_for_revision(
        &self,
        session: &ClientSession,
        active_transport_revision: Option<u64>,
    ) -> Option<crate::session::LocalPauseMutationIntent> {
        let intent = self.pending_local_pause_intent.as_ref().filter(|intent| {
            intent.connection_generation == self.connection_generation
                && intent.authorization == LocalIntentAuthorization::Authorized
                && session.room() == Some(intent.room.as_str())
                && self.coordinator.current_media_generation()
                    == Some(intent.local_media_generation)
                && active_transport_revision == intent.base_transport_revision
        })?;
        let paused = intent.paused;
        if session
            .current_room_playstate_authority()
            .is_some_and(|authority| {
                !self.room_authority_may_accept_local_pause_intent(session, authority, paused)
            })
        {
            return None;
        }
        Some(crate::session::LocalPauseMutationIntent {
            paused,
            base_transport_revision: intent.base_transport_revision,
        })
    }

    pub(crate) fn rollback_local_pause_intent(&mut self, paused: bool) {
        if self
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.paused == paused)
        {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
        }
    }

    /// Retires the transport-only user pause overlay when newer system
    /// authority issues a player command. Readiness intent remains canonical
    /// in the session model; this only prevents the superseded transport
    /// command from impersonating current player authority.
    pub(super) fn supersede_local_pause_transport(&mut self, at_seconds: f64) {
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.player_transition_classifier
            .supersede_unmatched_commands(PlayerCommandCause::LocalUserPlaybackControl, at_seconds);
    }

    pub(crate) fn observe_local_control_authority(
        &mut self,
        session: &ClientSession,
        room: Option<&str>,
        username: Option<&str>,
        authorized: bool,
    ) {
        let target_room = room.or_else(|| session.room());
        let target_username = username.or_else(|| session.username());
        if target_room != session.room() || target_username != session.username() {
            return;
        }

        let Some(target_room) = target_room else {
            return;
        };
        let authority_allows_control = !controlled_room_name(target_room) || authorized;
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            room: target_room.to_owned(),
            username: target_username.map(str::to_owned),
            connection_generation: self.connection_generation,
            freshness: if authority_allows_control {
                LocalControlAuthorityFreshness::Authorized
            } else {
                LocalControlAuthorityFreshness::Denied
            },
        });

        let pending_context_matches =
            self.pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    target_room == intent.room
                        && self.coordinator.current_media_generation()
                            == Some(intent.local_media_generation)
                });
        if !pending_context_matches {
            return;
        }
        if authority_allows_control {
            let intent = self
                .pending_local_pause_intent
                .as_mut()
                .expect("matching pause intent must still exist");
            intent.connection_generation = self.connection_generation;
            intent.authorization = LocalIntentAuthorization::Authorized;
        } else {
            self.pending_local_pause_intent = None;
        }
    }
}

impl RuntimePlaybackCoordination {
    pub(crate) fn begin_external_pause_command(
        &mut self,
        cause: PlayerCommandCause,
        desired_paused: bool,
        external_now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        let issued_at_seconds = self.standalone_command_issued_at_seconds(external_now_seconds);
        if cause != PlayerCommandCause::LocalUserPlaybackControl {
            self.supersede_local_pause_transport(issued_at_seconds);
        }
        self.register_synthetic_pause_command_completion(
            cause,
            desired_paused,
            issued_at_seconds,
            PlayerCommandCompletion::Pending,
        )
    }

    pub(crate) fn finish_external_pause_command(
        &mut self,
        command_id: PlayerCommandId,
        succeeded: bool,
        external_now_seconds: f64,
    ) -> bool {
        let at_seconds = self.standalone_command_issued_at_seconds(external_now_seconds);
        let completion = if succeeded {
            PlayerCommandCompletion::Completed { at_seconds }
        } else {
            PlayerCommandCompletion::Failed { at_seconds }
        };
        self.player_transition_classifier.update_command_completion(
            self.classifier_adapter_epoch(),
            command_id,
            completion,
        )
    }

    #[cfg(test)]
    pub(crate) fn register_external_pause_command_result(
        &mut self,
        cause: PlayerCommandCause,
        desired_paused: bool,
        succeeded: bool,
        external_now_seconds: f64,
    ) {
        let command_id =
            self.begin_external_pause_command(cause, desired_paused, external_now_seconds);
        if let Some(command_id) = command_id {
            let _ = self.finish_external_pause_command(command_id, succeeded, external_now_seconds);
        }
    }

    pub(super) fn current_native_play_authority_state(
        session: &ClientSession,
    ) -> NativePlayAuthorityState {
        let room = session.room().map(str::to_owned);
        let playstate_updated_at_seconds = room.as_ref().and_then(|room| {
            session
                .model
                .room
                .playstate_updated_at_seconds
                .get(room)
                .copied()
        });
        NativePlayAuthorityState {
            room,
            playstate: session.current_room_playstate().cloned(),
            playstate_updated_at_seconds,
            pause_owner: session
                .readiness_snapshot()
                .map(|snapshot| snapshot.pause_owner.clone()),
        }
    }

    pub(super) fn sync_pending_native_play_authority_fence(&mut self, session: &ClientSession) {
        let Some(first_observed_at_seconds) = self
            .player_transition_classifier
            .pending_native_play_first_observed_at_seconds()
        else {
            self.pending_native_play_authority_fence = None;
            return;
        };
        if self
            .pending_native_play_authority_fence
            .as_ref()
            .is_some_and(|fence| fence.first_observed_at_seconds == first_observed_at_seconds)
        {
            return;
        }
        self.pending_native_play_authority_fence = Some(PendingNativePlayAuthorityFence {
            first_observed_at_seconds,
            authority: Self::current_native_play_authority_state(session),
        });
    }

    pub(super) fn pending_native_play_authority_is_current(&self, session: &ClientSession) -> bool {
        let Some(first_observed_at_seconds) = self
            .player_transition_classifier
            .pending_native_play_first_observed_at_seconds()
        else {
            return false;
        };
        self.pending_native_play_authority_fence
            .as_ref()
            .is_some_and(|fence| {
                fence.first_observed_at_seconds == first_observed_at_seconds
                    && fence.authority == Self::current_native_play_authority_state(session)
            })
    }

    pub(super) fn invalidate_pending_native_play_if_authority_changed(
        &mut self,
        session: &ClientSession,
    ) {
        if self
            .player_transition_classifier
            .pending_native_play_first_observed_at_seconds()
            .is_some()
            && !self.pending_native_play_authority_is_current(session)
        {
            self.invalidate_stale_pending_native_play();
        }
    }

    pub(super) fn invalidate_stale_pending_native_play(&mut self) {
        let _ = self
            .player_transition_classifier
            .invalidate_pending_native_play();
        self.pending_native_play_authority_fence = None;
        self.last_player_transition_classification = None;
    }

    pub(super) fn external_pause_command_registration(
        &self,
        command_id: CoordinatorCommandId,
        issued_at_seconds: f64,
    ) -> Option<(PlayerCommandCause, bool, f64)> {
        let desired_paused = self.coordinator.pending_command_pause_target(command_id)?;
        let cause = if self
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.paused == desired_paused)
        {
            PlayerCommandCause::LocalUserPlaybackControl
        } else {
            self.cause_for_coordinator_command(CoordinatorPlayerCommand::SetPaused(desired_paused))
        };
        Some((cause, desired_paused, issued_at_seconds))
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    /// Stages a user-owned pause/play command before the external player can
    /// synchronously publish its resulting transport observation. The staged
    /// intent overlays ordinary legacy room state until the matching server
    /// echo arrives. Server barrier authority can preempt it; room buffering
    /// can preempt Play but admits Pause as a monotonic safety transition.
    pub fn stage_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .stage_local_pause_intent(paused, &self.session);
        let actions = self
            .playback_coordination
            .update_desired_from_session_with_replay(&self.session, now_seconds, false);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    /// Rolls back a staged pause/play intent when the external player rejects
    /// the command, restoring canonical room authority immediately.
    pub fn rollback_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .rollback_local_pause_intent(paused);
        let actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    /// Registers an attached-player pause/play command before dispatch. This
    /// ordering lets even synchronously published telemetry retain the exact
    /// causal owner. System commands also supersede any transport-only user
    /// overlay without changing the user's canonical readiness intent.
    pub fn begin_external_player_pause_command(
        &mut self,
        paused: bool,
        cause: PlayerCommandCause,
        now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        self.playback_coordination
            .begin_external_pause_command(cause, paused, now_seconds)
    }

    /// Records the terminal dispatch result for a command registered through
    /// [`Self::begin_external_player_pause_command`].
    pub fn finish_external_player_pause_command(
        &mut self,
        command_id: Option<PlayerCommandId>,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        if let Some(command_id) = command_id {
            let _ = self.playback_coordination.finish_external_pause_command(
                command_id,
                succeeded,
                now_seconds,
            );
        }
        if succeeded {
            Ok(())
        } else {
            self.report_player_command_failure_readiness(now_seconds)
        }
    }

    /// Tags the result of a pause/play command issued by an attached player
    /// surface outside the core adapter. The resulting telemetry edge remains
    /// causally owned by the original Sorotte gesture and therefore cannot be
    /// reclassified as a second native-player readiness mutation.
    pub fn record_external_player_pause_command_result(
        &mut self,
        paused: bool,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let command_id = self.begin_external_player_pause_command(
            paused,
            PlayerCommandCause::LocalUserPlaybackControl,
            now_seconds,
        );
        self.finish_external_player_pause_command(command_id, succeeded, now_seconds)
    }

    /// Confirms an explicit native-player action only when the classifier has
    /// already staged the matching same-scope transport edge. Successful
    /// confirmation consumes that edge and dispatches its indirect readiness
    /// intent exactly once.
    pub fn confirm_pending_native_player_play(
        &mut self,
        surface: PlayerInteractionSurface,
    ) -> Result<bool, PlayerError> {
        self.playback_coordination
            .invalidate_pending_native_play_if_authority_changed(&self.session);
        if self
            .playback_coordination
            .player_transition_classifier
            .pending_native_play_first_observed_at_seconds()
            .is_none()
        {
            return Ok(false);
        }
        let Some(classification) = self
            .playback_coordination
            .player_transition_classifier
            .confirm_pending_native_play()
        else {
            return Ok(false);
        };
        self.playback_coordination
            .pending_native_play_authority_fence = None;
        self.playback_coordination
            .last_player_transition_classification = Some(classification);
        self.dispatch_native_player_readiness_action(NativePlayerAction::Play, surface)?;
        Ok(true)
    }

    /// Preserves a user-owned Play edge when its resulting observation races a
    /// not-yet-dispatched pause correction. During Preparing the semantic Play
    /// becomes Ready while the exact gate correction is retained. Everywhere
    /// else, an authorized room/media/connection-scoped Play intent supersedes
    /// the stale correction until canonical authority accepts or rejects it.
    pub(super) fn promote_pending_native_play_before_pause_correction(
        &mut self,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) -> Result<bool, PlayerError> {
        let has_pause_correction = actions.iter().any(|action| {
            matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(true),
                    ..
                }
            )
        });
        if !has_pause_correction {
            return Ok(false);
        }

        let gate_holds = self.readiness_gate_holds_current_playback();
        let controller_can_own_v2_transport = self.session.server_readiness_v2_supported()
            && self.session.local_can_control().unwrap_or(false)
            && !gate_holds;
        let pause_correction_cause = self
            .playback_coordination
            .cause_for_coordinator_command(CoordinatorPlayerCommand::SetPaused(true));
        let native_promotion_allowed = matches!(
            pause_correction_cause,
            PlayerCommandCause::ReadinessGateHold | PlayerCommandCause::RemoteRoomSynchronization
        ) && (gate_holds || controller_can_own_v2_transport);
        let promoted = if native_promotion_allowed {
            self.confirm_pending_native_player_play(PlayerInteractionSurface::NativePlayerControl)?
        } else {
            false
        };
        let local_play_intent_active = self
            .playback_coordination
            .has_active_local_pause_intent(false, &self.session);
        let playlist_transition_holds = self.session.has_pending_playlist_index_reset_intent();
        let room_buffering_holds = matches!(
            self.session.current_room_playstate_authority(),
            Some(RoomPlaystateAuthority::ServerBufferingPolicy { .. })
        );
        if !local_play_intent_active
            || gate_holds
            || playlist_transition_holds
            || room_buffering_holds
        {
            return Ok(promoted);
        }

        let mut superseded_command_ids = Vec::new();
        actions.retain(|action| {
            let PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPaused(true),
            } = action
            else {
                return true;
            };
            superseded_command_ids.push(*command_id);
            false
        });
        for command_id in superseded_command_ids {
            let superseded = self
                .playback_coordination
                .coordinator
                .supersede_unaccepted_command(command_id);
            debug_assert!(
                superseded,
                "a correction removed before dispatch must still be unaccepted"
            );
        }
        Ok(promoted)
    }

    pub(super) fn dispatch_native_player_readiness_action(
        &mut self,
        action: NativePlayerAction,
        surface: PlayerInteractionSurface,
    ) -> Result<(), PlayerError> {
        let paused = action == NativePlayerAction::Pause;
        let readiness_v2_supported = self.session.server_readiness_v2_supported();
        let controller_can_own_v2_transport = readiness_v2_supported
            && self.session.local_can_control().unwrap_or(false)
            && !self.readiness_gate_holds_current_playback();
        if !readiness_v2_supported || controller_can_own_v2_transport {
            // Legacy rooms still need a short transport overlay while the
            // canonical self-echo is in flight. Readiness V2 uses the same
            // overlay only after the classifier proves an authorized native
            // gesture outside the exact Preparing gate.
            self.playback_coordination
                .stage_local_pause_intent(paused, &self.session);
        }
        let actions = self
            .session
            .runtime_actions_for_indirect_player_intent(paused, surface);
        self.dispatch_runtime_actions_with_causal_tracking(&actions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalIntentAuthorization {
    Authorized,
    AwaitingControlledRoomReauthentication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalControlAuthorityFreshness {
    Awaiting,
    Authorized,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConnectionLocalControlAuthority {
    pub(super) room: String,
    pub(super) username: Option<String>,
    pub(super) connection_generation: u64,
    pub(super) freshness: LocalControlAuthorityFreshness,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingLocalPauseIntent {
    pub(super) paused: bool,
    pub(super) room: String,
    pub(super) local_media_generation: u64,
    pub(super) connection_generation: u64,
    pub(super) base_transport_revision: Option<u64>,
    pub(super) authorization: LocalIntentAuthorization,
    pub(super) replay_player_after_reauthorization: bool,
    pub(super) last_canonical_playstate_updated_at_seconds: Option<f64>,
    pub(super) mismatching_canonical_playstate_updates: u8,
    pub(super) first_mismatching_canonical_playstate_at_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct NativePlayAuthorityState {
    pub(super) room: Option<String>,
    pub(super) playstate: Option<RoomPlaystateView>,
    pub(super) playstate_updated_at_seconds: Option<f64>,
    pub(super) pause_owner: Option<RoomPauseOwner>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingNativePlayAuthorityFence {
    pub(super) first_observed_at_seconds: f64,
    pub(super) authority: NativePlayAuthorityState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LocalPositionObservation {
    pub(super) media_generation: u64,
    pub(super) observed_at_seconds: f64,
    pub(super) last_actual_position_observed_at_seconds: f64,
    pub(super) position_seconds: f64,
    pub(super) playback_rate: f64,
}
