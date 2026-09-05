//! Room barrier coordination owns request/retry, ready/started deduplication,
//! timeout and room-buffering observations. Inputs are canonical server revisions,
//! local technical evidence and request outcomes; outputs are protocol requests
//! or coordinator intents. Room/media/connection changes invalidate old bindings.
//! This module never turns advisory participant status into room authority.
use super::*;

impl RuntimePlaybackCoordination {
    pub(crate) fn set_barrier_start_config(&mut self, config: PlaybackBarrierStartConfig) {
        self.barrier.barrier_start_config = PlaybackBarrierStartConfig {
            policy: config.policy,
            quorum_percent: config.quorum_percent.clamp(1, 100),
            timeout_seconds: if config.timeout_seconds.is_finite() && config.timeout_seconds > 0.0 {
                config.timeout_seconds
            } else {
                PlaybackBarrierStartConfig::default().timeout_seconds
            },
            timeout_action: config.timeout_action,
        };
    }

    pub(crate) fn set_room_buffering_config(&mut self, config: PlaybackBarrierRoomBufferingConfig) {
        let defaults = PlaybackBarrierRoomBufferingConfig::default();
        self.barrier.room_buffering_config = PlaybackBarrierRoomBufferingConfig {
            policy: config.policy,
            quorum_percent: config.quorum_percent.clamp(1, 100),
            debounce_seconds: normalized_positive_seconds(
                config.debounce_seconds,
                defaults.debounce_seconds,
            ),
            resume_hysteresis_seconds: normalized_positive_seconds(
                config.resume_hysteresis_seconds,
                defaults.resume_hysteresis_seconds,
            ),
            maximum_pause_seconds: normalized_positive_seconds(
                config.maximum_pause_seconds,
                defaults.maximum_pause_seconds,
            ),
        };
    }

    pub(super) fn next_room_barrier_request_nonce(&mut self) -> u64 {
        self.barrier.next_room_barrier_request_nonce = self
            .barrier
            .next_room_barrier_request_nonce
            .saturating_add(1)
            .max(1);
        self.barrier.next_room_barrier_request_nonce
    }

    pub(crate) const fn connection_generation(&self) -> u64 {
        self.connection_generation
    }

    pub(crate) fn playback_barrier_set_for_new_media(
        &mut self,
        plan: &MediaLoadPlan,
        session: &ClientSession,
        now_seconds: f64,
    ) -> Option<PendingPlaybackBarrierRequest> {
        if !plan.playback_episode_changed {
            return None;
        }

        self.playback_barrier_set_for_pending_media(session, now_seconds)
    }

    pub(crate) fn playback_barrier_set_for_pending_media(
        &mut self,
        session: &ClientSession,
        now_seconds: f64,
    ) -> Option<PendingPlaybackBarrierRequest> {
        if let Some(mut recovery) = self.barrier.pending_barrier_recovery.clone() {
            let room = session.room()?.to_owned();
            if recovery.room.as_deref() != Some(room.as_str()) {
                recovery.room = Some(room.clone());
                recovery.recovery_nonce = None;
                self.barrier.pending_barrier_recovery = Some(recovery.clone());
            }
            if recovery.recovery_nonce.is_some()
                || !session.playback_barrier_v1_negotiated()
                || session.local_can_control() != Some(true)
                || self.coordinator.current_media_generation()
                    != Some(recovery.operation.local_media_generation)
                || !self.current_logical_media_matches(&recovery.operation.logical_media_id)
            {
                return None;
            }

            let recovery_nonce = self.next_room_barrier_request_nonce();
            let extension = PlaybackBarrierSetExtension::new().with_recovery(
                PlaybackBarrierRecoveryPayload::query(
                    recovery.operation.request_id.clone(),
                    recovery.operation.request_nonce,
                    recovery_nonce,
                    recovery.operation.logical_media_id.clone(),
                ),
            );
            return Some(PendingPlaybackBarrierRequest {
                extension,
                room,
                local_media_generation: recovery.operation.local_media_generation,
                request_nonce: recovery_nonce,
                operation: recovery.operation,
                recovery_request: true,
            });
        }

        let mut intent = self.barrier.pending_media_coordination.clone()?;
        let room = session.room()?.to_owned();
        if intent.room.as_deref() != Some(room.as_str()) {
            // The old serialized scope is cancelled by the control outbox.
            // The current-media semantic intent remains useful in the new
            // room and is rebound only after its authorization succeeds.
            intent.room = Some(room.clone());
            self.barrier.pending_media_coordination = Some(intent.clone());
        }
        if self
            .barrier
            .initiated_barrier
            .as_ref()
            .is_some_and(|operation| {
                operation.local_media_generation == intent.local_media_generation
            })
            || !session.playback_barrier_v1_negotiated()
            || session.local_can_control() != Some(true)
            || intent
                .retry_not_before_seconds
                .is_some_and(|deadline| !now_seconds.is_finite() || now_seconds < deadline)
        {
            return None;
        }

        let (local_generation, logical_media_id) = self.current_logical_media()?;
        if local_generation != intent.local_media_generation {
            self.barrier.pending_media_coordination = None;
            return None;
        }
        if intent.include_start_barrier
            && intent.load_intent != MediaLoadIntent::Replay
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                logical_media_ids_match(&logical_media_id, &prepare.logical_media_id)
                    && session.playback_barrier_status().is_some_and(|status| {
                        status.media_generation == prepare.media_generation
                            && matches!(
                                status.phase,
                                PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::Committed
                            )
                    })
            })
        {
            // A peer has already established the room generation for this
            // logical source. Loading that source locally is participation,
            // not a competing start request.
            self.barrier.pending_media_coordination = None;
            return None;
        }
        let load_intent = if intent.include_start_barrier
            && intent.load_intent == MediaLoadIntent::NewPlayback
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                logical_media_ids_match(&logical_media_id, &prepare.logical_media_id)
                    && session.playback_barrier_status().is_some_and(|status| {
                        status.media_generation == prepare.media_generation
                            && matches!(
                                status.phase,
                                PlaybackBarrierPhase::AwaitingDecision
                                    | PlaybackBarrierPhase::Complete
                                    | PlaybackBarrierPhase::Degraded
                            )
                    })
            }) {
            // A fresh coordinator has no local load history from which to
            // infer replay intent. Retained terminal room history provides
            // the missing identity without weakening active-generation
            // exclusion above.
            MediaLoadIntent::Replay
        } else {
            intent.load_intent
        };
        let request_nonce = intent
            .retry_request_nonce
            .unwrap_or_else(|| self.next_room_barrier_request_nonce());
        let mut extension = PlaybackBarrierSetExtension::new();
        let mut start_requested = false;
        // `None` preserves the immediate compatibility behavior. Applications
        // opt in to a coordinated start by supplying a barrier policy.
        let effective_start_policy = self.barrier.barrier_start_config.policy;
        if intent.include_start_barrier
            && let Some(policy) = effective_start_policy
        {
            let timeout_ms = (self.barrier.barrier_start_config.timeout_seconds * 1_000.0)
                .round()
                .clamp(1.0, u64::MAX as f64) as u64;
            let mut prepare = PrepareMediaPayload::request(
                request_nonce,
                logical_media_id.clone(),
                0.0,
                policy,
                load_intent,
            )
            .with_request_id(intent.request_id.clone())
            .with_timeout_ms(timeout_ms)
            .with_timeout_action(self.barrier.barrier_start_config.timeout_action);
            if policy == PlaybackBarrierPolicy::Quorum {
                prepare =
                    prepare.with_quorum_percent(self.barrier.barrier_start_config.quorum_percent);
            }
            extension = extension.with_prepare(prepare);
            start_requested = true;
        }

        let room_config = self.barrier.room_buffering_config;
        let mut buffering = RoomBufferingPolicyPayload::new(0, room_config.policy)
            .with_request_nonce(request_nonce)
            .with_request_id(intent.request_id.clone())
            .with_load_intent(load_intent)
            .with_debounce_ms(seconds_to_milliseconds(room_config.debounce_seconds))
            .with_resume_hysteresis_ms(seconds_to_milliseconds(
                room_config.resume_hysteresis_seconds,
            ))
            .with_max_pause_ms(seconds_to_milliseconds(room_config.maximum_pause_seconds));
        if room_config.policy == RoomBufferingPolicy::Quorum {
            buffering = buffering.with_quorum_percent(room_config.quorum_percent);
        }
        let operation = PlaybackBarrierOperation {
            local_media_generation: local_generation,
            load_intent,
            include_start_barrier: start_requested,
            request_id: intent.request_id,
            request_nonce,
            logical_media_id,
            room: room.clone(),
        };
        Some(PendingPlaybackBarrierRequest {
            extension: extension.with_buffering_policy(buffering),
            room,
            local_media_generation: local_generation,
            request_nonce,
            operation,
            recovery_request: false,
        })
    }

    pub(crate) fn confirm_playback_barrier_request_queued(
        &mut self,
        request: &PendingPlaybackBarrierRequest,
    ) {
        if request.recovery_request {
            if let Some(recovery) = self.barrier.pending_barrier_recovery.as_mut()
                && recovery.operation == request.operation
            {
                recovery.recovery_nonce = Some(request.request_nonce);
                recovery.room = Some(request.room.clone());
            }
            return;
        }
        if self
            .barrier
            .pending_media_coordination
            .as_ref()
            .is_some_and(|intent| {
                intent.local_media_generation == request.local_media_generation
                    && intent.request_id == request.operation.request_id
            })
        {
            self.barrier.initiated_barrier = Some(request.operation.clone());
        }
    }

    pub(crate) fn discard_room_scoped_playback_barrier_intent(&mut self) {
        self.barrier.initiated_barrier = None;
        self.barrier.accepted_barrier = None;
        self.barrier.pending_barrier_recovery = None;
        self.barrier.pending_media_coordination = None;
        self.barrier.accepted_barrier_terminal = false;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
    }

    pub(crate) fn handle_authoritative_playback_barrier_room_change(&mut self) {
        self.coordinator.cancel_seek_preparation_for_lifecycle();
        self.coordinator.clear_seek_preparation_terminal();
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.pending_native_play_authority_fence = None;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        self.participant_status
            .pending_participant_status_room_switch_target = None;
        self.participant_status.last_participant_status_fingerprint = None;
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            room: String::new(),
            username: None,
            connection_generation: self.connection_generation,
            freshness: LocalControlAuthorityFreshness::Awaiting,
        });
        if self.barrier.initiated_barrier.is_some()
            || self.barrier.accepted_barrier.is_some()
            || self.barrier.pending_barrier_recovery.is_some()
        {
            self.discard_room_scoped_playback_barrier_intent();
        } else if let Some(pending) = self.barrier.pending_media_coordination.as_mut() {
            // A pre-authentication media intent has never been serialized and
            // may safely bind to the authoritative destination room.
            pending.room = None;
        }
    }

    pub(crate) fn observe_playback_barrier_server_extension(
        &mut self,
        extension: &PlaybackBarrierSetExtension,
        session: &ClientSession,
        now_seconds: f64,
    ) -> bool {
        let retry_scheduled = extension.request_result.as_ref().is_some_and(|result| {
            if result.status != PlaybackBarrierRequestResultStatus::RetryLater
                || !now_seconds.is_finite()
            {
                return false;
            }
            let Some(operation) = self.barrier.initiated_barrier.as_ref() else {
                return false;
            };
            if operation.request_id != result.request_id
                || operation.request_nonce != result.request_nonce
            {
                return false;
            }
            let Some(intent) = self.barrier.pending_media_coordination.as_mut() else {
                return false;
            };
            if intent.local_media_generation != operation.local_media_generation
                || intent.request_id != operation.request_id
            {
                return false;
            }

            intent.retry_attempts = intent.retry_attempts.saturating_add(1);
            let retry_delay_seconds =
                playback_barrier_retry_delay_seconds(result.retry_after_ms, intent.retry_attempts);
            intent.retry_request_nonce = Some(operation.request_nonce);
            intent.retry_not_before_seconds = Some(now_seconds + retry_delay_seconds);
            self.barrier.initiated_barrier = None;
            true
        });

        if let Some(response) = extension.recovery.as_ref()
            && let Some(recovery) = self.barrier.pending_barrier_recovery.clone()
            && recovery.recovery_nonce == Some(response.recovery_nonce)
            && recovery.operation.request_id == response.request_id
            && recovery.operation.request_nonce == response.original_request_nonce
            && logical_media_ids_match(
                &recovery.operation.logical_media_id,
                &response.logical_media_id,
            )
        {
            match response.disposition {
                Some(PlaybackBarrierRecoveryDisposition::Recovered) => {}
                Some(PlaybackBarrierRecoveryDisposition::Existing) => {
                    let existing_lifecycle_applied =
                        session.playback_barrier_prepare().is_some_and(|prepare| {
                            logical_media_ids_match(
                                &recovery.operation.logical_media_id,
                                &prepare.logical_media_id,
                            )
                        });
                    if existing_lifecycle_applied {
                        self.discard_room_scoped_playback_barrier_intent();
                    }
                }
                Some(PlaybackBarrierRecoveryDisposition::Absent) => {
                    let operation = recovery.operation;
                    let was_terminal = self.barrier.accepted_barrier_terminal;
                    self.barrier.initiated_barrier = None;
                    self.barrier.accepted_barrier = None;
                    self.barrier.pending_barrier_recovery = None;
                    self.barrier.accepted_barrier_terminal = false;
                    self.barrier.pending_media_coordination =
                        Some(PendingMediaCoordinationIntent {
                            local_media_generation: operation.local_media_generation,
                            load_intent: if was_terminal {
                                MediaLoadIntent::TransportRefresh
                            } else {
                                operation.load_intent
                            },
                            include_start_barrier: !was_terminal && operation.include_start_barrier,
                            request_id: if was_terminal {
                                new_playback_barrier_request_id(operation.local_media_generation)
                            } else {
                                operation.request_id
                            },
                            retry_request_nonce: (!was_terminal).then_some(operation.request_nonce),
                            retry_not_before_seconds: None,
                            retry_attempts: 0,
                            room: session.room().map(str::to_owned),
                        });
                }
                Some(
                    PlaybackBarrierRecoveryDisposition::Superseded
                    | PlaybackBarrierRecoveryDisposition::Rejected,
                ) => self.discard_room_scoped_playback_barrier_intent(),
                None => {}
            }
        }

        let candidate = self
            .barrier
            .initiated_barrier
            .clone()
            .or_else(|| self.barrier.accepted_barrier.clone())
            .or_else(|| {
                self.barrier
                    .pending_barrier_recovery
                    .as_ref()
                    .map(|recovery| recovery.operation.clone())
            });
        if let Some(operation) = candidate {
            let prepare_matches = extension.prepare.as_ref().is_some_and(|prepare| {
                prepare.request_id.as_deref() == Some(operation.request_id.as_str())
                    && prepare.request_nonce == operation.request_nonce
                    && prepare.media_generation > 0
                    && logical_media_ids_match(
                        &operation.logical_media_id,
                        &prepare.logical_media_id,
                    )
                    && session
                        .playback_barrier_prepare()
                        .is_some_and(|accepted| accepted == prepare)
            });
            let policy_matches = extension.buffering_policy.as_ref().is_some_and(|policy| {
                policy.request_id.as_deref() == Some(operation.request_id.as_str())
                    && policy.request_nonce == operation.request_nonce
                    && policy.media_generation > 0
                    && session
                        .playback_barrier_buffering_policy()
                        .is_some_and(|accepted| accepted == policy)
            });
            if prepare_matches || policy_matches {
                self.barrier.initiated_barrier = Some(operation.clone());
                self.barrier.accepted_barrier = Some(operation);
                self.barrier.pending_media_coordination = None;
                self.barrier.pending_barrier_recovery = None;
            }
        }

        if let Some(operation) = self.barrier.accepted_barrier.as_ref()
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                prepare.request_id.as_deref() == Some(operation.request_id.as_str())
                    && prepare.request_nonce == operation.request_nonce
            })
            && session.playback_barrier_status().is_some_and(|status| {
                matches!(
                    status.phase,
                    PlaybackBarrierPhase::Complete | PlaybackBarrierPhase::Degraded
                )
            })
        {
            self.barrier.accepted_barrier_terminal = true;
        }

        retry_scheduled
    }

    pub(crate) fn pending_playback_barrier_retry_delay_at(&self, now_seconds: f64) -> Option<f64> {
        let intent = self.barrier.pending_media_coordination.as_ref()?;
        let deadline = intent.retry_not_before_seconds?;
        (self.barrier.initiated_barrier.is_none() && now_seconds.is_finite())
            .then_some((deadline - now_seconds).max(0.0))
    }

    pub(super) fn barrier_ready_signature(
        &self,
        session: &ClientSession,
    ) -> Option<BarrierReadySignature> {
        let prepare = session.playback_barrier_prepare()?;
        if session.playback_barrier_status()?.phase != PlaybackBarrierPhase::Preparing {
            return None;
        }
        if !self.current_logical_media_matches(&prepare.logical_media_id) {
            return None;
        }
        let observation = self.latest_observation.as_ref()?;
        if self.coordinator.current_media_generation() != Some(observation.media_generation) {
            return None;
        }
        let phase = observation.phase?;
        let loaded = !matches!(
            phase,
            sorotte_player_api::PlayerTransportPhase::Empty
                | sorotte_player_api::PlayerTransportPhase::Loading
                | sorotte_player_api::PlayerTransportPhase::Failed
        );
        let target_applied = observation
            .position_seconds
            .is_some_and(|position| (position - prepare.target_position).abs() <= 0.5);
        let prepare_revision_applied = self.desired_fingerprint.as_ref().is_some_and(|desired| {
            desired.barrier_media_generation == Some(prepare.media_generation)
                && desired.barrier_state_revision.is_none()
                && self.last_applied_revision == Some(self.desired_revision)
        });
        let buffer_ready = phase == sorotte_player_api::PlayerTransportPhase::ReadyPaused
            && observation.logical_pause == Some(true)
            && target_applied
            && prepare_revision_applied
            && observation.paused_for_cache != Some(true)
            && observation.seeking != Some(true);
        Some(BarrierReadySignature {
            room_media_generation: prepare.media_generation,
            local_media_generation: observation.media_generation,
            loaded,
            seekable: observation.seekable,
            buffer_ready,
        })
    }

    pub(super) fn barrier_started_target(
        &self,
        session: &ClientSession,
        local_media_generation: u64,
        coordinator_state_revision: u64,
    ) -> Option<(u64, u64)> {
        let prepare = session.playback_barrier_prepare()?;
        let commit = session.playback_barrier_active_commit()?;
        let desired = self.desired_fingerprint.as_ref()?;
        if prepare.media_generation != commit.media_generation
            || self.coordinator.current_media_generation() != Some(local_media_generation)
            || !self.current_logical_media_matches(&prepare.logical_media_id)
            || coordinator_state_revision != self.desired_revision
            || desired.barrier_media_generation != Some(commit.media_generation)
            || desired.barrier_state_revision != Some(commit.state_revision)
        {
            return None;
        }
        Some((commit.media_generation, commit.state_revision))
    }

    pub(super) fn capture_barrier_timeout_action(&mut self, session: &ClientSession) {
        let Some(operation) = self.barrier.initiated_barrier.as_ref() else {
            return;
        };
        if self.coordinator.current_media_generation() != Some(operation.local_media_generation) {
            return;
        }
        let Some(prepare) = session.playback_barrier_prepare().filter(|prepare| {
            prepare.request_nonce == operation.request_nonce
                && prepare.request_id.as_deref() == Some(operation.request_id.as_str())
        }) else {
            return;
        };
        let Some(status) = session.playback_barrier_status().filter(|status| {
            status.media_generation == prepare.media_generation
                && status.participants.values().any(|participant| {
                    participant.phase == PlaybackBarrierParticipantPhase::PrepareTimedOut
                })
        }) else {
            return;
        };
        let identity = (status.media_generation, status.state_revision);
        if self.barrier.handled_barrier_timeout == Some(identity) {
            return;
        }
        self.barrier.handled_barrier_timeout = Some(identity);
        let action = prepare.timeout_action.unwrap_or_default();
        if action != PlaybackBarrierTimeoutAction::Continue {
            self.barrier.pending_barrier_timeout_action = Some(action);
        }
    }

    pub(super) fn room_buffering_observation(
        &self,
        session: &ClientSession,
    ) -> Option<RoomBufferingObservation> {
        let policy = session.playback_barrier_buffering_policy()?;
        if policy.policy == RoomBufferingPolicy::Independent {
            return None;
        }
        if let Some(prepare) = session.playback_barrier_prepare()
            && prepare.media_generation == policy.media_generation
            && !self.current_logical_media_matches(&prepare.logical_media_id)
        {
            return None;
        }
        let observation = self.latest_observation.as_ref()?;
        if self.coordinator.current_media_generation() != Some(observation.media_generation) {
            return None;
        }
        let buffering = observation.paused_for_cache == Some(true)
            || observation.phase == Some(sorotte_player_api::PlayerTransportPhase::Rebuffering);
        Some(RoomBufferingObservation {
            report_epoch: session.playback_barrier_buffering_report_epoch(),
            media_generation: policy.media_generation,
            state_revision: policy.state_revision,
            buffering,
            buffered_seconds: observation.buffered_ahead_seconds,
            observed_at: Some(observation.observed_at_seconds),
        })
    }

    pub(super) fn should_report_room_buffering(
        &self,
        report_epoch: u64,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) -> bool {
        self.barrier.last_reported_room_buffering
            != Some((report_epoch, media_generation, state_revision, buffering))
    }

    pub(super) fn mark_room_buffering_reported(
        &mut self,
        report_epoch: u64,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) {
        self.barrier.last_reported_room_buffering =
            Some((report_epoch, media_generation, state_revision, buffering));
    }

    pub(super) fn mark_barrier_ready_reported(&mut self, signature: BarrierReadySignature) {
        self.barrier.last_reported_barrier_ready = Some(signature);
    }

    pub(super) fn mark_barrier_started_reported(
        &mut self,
        media_generation: u64,
        state_revision: u64,
    ) {
        self.barrier.last_reported_barrier_started = Some((media_generation, state_revision));
    }

    pub(super) fn readiness_gate_holds_current_playback(&self, session: &ClientSession) -> bool {
        session.playback_barrier_prepare().is_some_and(|prepare| {
            self.current_logical_media_matches(&prepare.logical_media_id)
                && session.readiness_gate_holds_room_pause_for_generation(prepare.media_generation)
        })
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub(crate) fn emit_pending_playback_barrier_request_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let Some(request) = self
            .playback_coordination
            .playback_barrier_set_for_pending_media(&self.session, now_seconds)
        else {
            return Ok(());
        };
        self.control.activate_protocol_connection_generation();
        let scope = PlaybackBarrierRequestScope::new(
            request.room.clone(),
            request.local_media_generation,
            request.request_nonce,
        );
        self.control
            .emit(ClientEffect::send_playback_barrier_set(
                request.extension.clone(),
                scope,
            ))
            .map_err(client_effect_player_error)?;
        self.playback_coordination
            .confirm_playback_barrier_request_queued(&request);
        Ok(())
    }

    pub fn set_playback_barrier_start_config(&mut self, config: PlaybackBarrierStartConfig) {
        self.playback_coordination.set_barrier_start_config(config);
    }

    pub fn set_playback_barrier_room_buffering_config(
        &mut self,
        config: PlaybackBarrierRoomBufferingConfig,
    ) {
        self.playback_coordination.set_room_buffering_config(config);
    }

    /// Whether the exact currently tracked media is held paused by a
    /// Preparing readiness gate. Front ends consume this shared policy
    /// instead of maintaining their own V2 play rules.
    pub fn readiness_gate_holds_current_playback(&self) -> bool {
        self.playback_coordination
            .readiness_gate_holds_current_playback(&self.session)
    }

    pub fn take_playback_barrier_timeout_action(&mut self) -> Option<PlaybackBarrierTimeoutAction> {
        self.playback_coordination
            .barrier
            .pending_barrier_timeout_action
            .take()
    }

    pub fn report_playback_barrier_media_ready(
        &mut self,
        media_generation: u64,
        loaded: bool,
        seekable: Option<bool>,
        buffer_ready: bool,
    ) -> Result<bool, ClientEffectError> {
        let Some(state) = self.session.playback_barrier_media_ready_observation(
            media_generation,
            loaded,
            seekable,
            buffer_ready,
        ) else {
            return Ok(false);
        };
        self.control.activate_protocol_connection_generation();
        self.control.emit(ClientEffect::SendState(state))?;
        Ok(true)
    }

    pub fn report_playback_barrier_started(
        &mut self,
        media_generation: u64,
        state_revision: u64,
        observed_position: f64,
        position_advancing: bool,
        observed_at: Option<f64>,
    ) -> Result<bool, ClientEffectError> {
        let Some(state) = self.session.playback_barrier_started_observation(
            media_generation,
            state_revision,
            observed_position,
            position_advancing,
            observed_at,
        ) else {
            return Ok(false);
        };
        self.control.activate_protocol_connection_generation();
        self.control.emit(ClientEffect::SendState(state))?;
        Ok(true)
    }

    pub(super) fn report_playback_barrier_observations(
        &mut self,
        actions: &[PlaybackCoordinatorAction],
    ) -> Result<(), ClientEffectError> {
        if let Some(signature) = self
            .playback_coordination
            .barrier_ready_signature(&self.session)
            && self
                .playback_coordination
                .barrier
                .last_reported_barrier_ready
                != Some(signature)
            && self.report_playback_barrier_media_ready(
                signature.room_media_generation,
                signature.loaded,
                signature.seekable,
                signature.buffer_ready,
            )?
        {
            self.playback_coordination
                .mark_barrier_ready_reported(signature);
        }

        if let Some(observation) = self
            .playback_coordination
            .room_buffering_observation(&self.session)
            && self.playback_coordination.should_report_room_buffering(
                observation.report_epoch,
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
            )
            && let Some(state) = self.session.playback_barrier_transport_observation(
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
                observation.buffered_seconds,
                observation.observed_at,
            )
        {
            self.control.activate_protocol_connection_generation();
            self.control.emit(ClientEffect::SendState(state))?;
            self.playback_coordination.mark_room_buffering_reported(
                observation.report_epoch,
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
            );
        }

        for action in actions {
            let PlaybackCoordinatorAction::Started {
                media_generation: local_media_generation,
                state_revision: coordinator_state_revision,
                observed_position_seconds,
            } = action
            else {
                continue;
            };
            let Some((room_media_generation, room_state_revision)) =
                self.playback_coordination.barrier_started_target(
                    &self.session,
                    *local_media_generation,
                    *coordinator_state_revision,
                )
            else {
                continue;
            };
            if self
                .playback_coordination
                .barrier
                .last_reported_barrier_started
                == Some((room_media_generation, room_state_revision))
            {
                continue;
            }
            let observed_at = self.playback_coordination.latest_observed_at_seconds();
            if self.report_playback_barrier_started(
                room_media_generation,
                room_state_revision,
                *observed_position_seconds,
                true,
                observed_at,
            )? {
                self.playback_coordination
                    .mark_barrier_started_reported(room_media_generation, room_state_revision);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(super) struct RoomBarrierState {
    pub(super) last_reported_barrier_ready: Option<BarrierReadySignature>,
    pub(super) last_reported_barrier_started: Option<(u64, u64)>,
    pub(super) barrier_start_config: PlaybackBarrierStartConfig,
    pub(super) room_buffering_config: PlaybackBarrierRoomBufferingConfig,
    pub(super) next_room_barrier_request_nonce: u64,
    pub(super) initiated_barrier: Option<PlaybackBarrierOperation>,
    pub(super) accepted_barrier: Option<PlaybackBarrierOperation>,
    pub(super) pending_barrier_recovery: Option<PendingPlaybackBarrierRecovery>,
    pub(super) accepted_barrier_terminal: bool,
    pub(super) pending_media_coordination: Option<PendingMediaCoordinationIntent>,
    pub(super) handled_barrier_timeout: Option<(u64, Option<u64>)>,
    pub(super) pending_barrier_timeout_action: Option<PlaybackBarrierTimeoutAction>,
    pub(super) last_reported_room_buffering: Option<(u64, u64, Option<u64>, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BarrierReadySignature {
    pub(super) room_media_generation: u64,
    pub(super) local_media_generation: u64,
    pub(super) loaded: bool,
    pub(super) seekable: Option<bool>,
    pub(super) buffer_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RoomBufferingObservation {
    pub(super) report_epoch: u64,
    pub(super) media_generation: u64,
    pub(super) state_revision: Option<u64>,
    pub(super) buffering: bool,
    pub(super) buffered_seconds: Option<f64>,
    pub(super) observed_at: Option<f64>,
}

#[derive(Clone, PartialEq)]
pub(super) struct PendingMediaCoordinationIntent {
    pub(super) local_media_generation: u64,
    pub(super) load_intent: MediaLoadIntent,
    pub(super) include_start_barrier: bool,
    pub(super) request_id: String,
    pub(super) retry_request_nonce: Option<u64>,
    pub(super) retry_not_before_seconds: Option<f64>,
    pub(super) retry_attempts: u32,
    pub(super) room: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct PlaybackBarrierOperation {
    pub(super) local_media_generation: u64,
    pub(super) load_intent: MediaLoadIntent,
    pub(super) include_start_barrier: bool,
    pub(super) request_id: String,
    pub(super) request_nonce: u64,
    pub(super) logical_media_id: String,
    pub(super) room: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingPlaybackBarrierRecovery {
    pub(super) operation: PlaybackBarrierOperation,
    pub(super) recovery_nonce: Option<u64>,
    pub(super) room: Option<String>,
}

impl std::fmt::Debug for PendingMediaCoordinationIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingMediaCoordinationIntent")
            .field("local_media_generation", &self.local_media_generation)
            .field("load_intent", &self.load_intent)
            .field("include_start_barrier", &self.include_start_barrier)
            .field("request_id", &"<redacted>")
            .field("retry_request_nonce", &self.retry_request_nonce)
            .field("retry_not_before_seconds", &self.retry_not_before_seconds)
            .field("retry_attempts", &self.retry_attempts)
            .field("room", &self.room)
            .finish()
    }
}

impl std::fmt::Debug for PlaybackBarrierOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackBarrierOperation")
            .field("local_media_generation", &self.local_media_generation)
            .field("load_intent", &self.load_intent)
            .field("include_start_barrier", &self.include_start_barrier)
            .field("request_id", &"<redacted>")
            .field("request_nonce", &self.request_nonce)
            .field("logical_media_id", &"<redacted>")
            .field("room", &self.room)
            .finish()
    }
}
