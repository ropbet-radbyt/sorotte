//! Advisory participant reporting owns report fingerprints, monotonic sequence,
//! heartbeat timing and room-scope bindings. Inputs are accepted transport evidence
//! and the current session; outputs are scoped reports plus commit bookkeeping.
//! Membership, media and attachment resets fence old evidence. Reporting cannot
//! change canonical playback, player ownership, or room-control authority.
use super::*;

impl RuntimePlaybackCoordination {
    pub(super) fn participant_status_phase(&self) -> ParticipantPlaybackPhase {
        if self.latest_observation.as_ref().is_some_and(|observation| {
            observation.phase == Some(sorotte_player_api::PlayerTransportPhase::Ended)
        }) {
            return ParticipantPlaybackPhase::Ended;
        }
        let diagnostic = self.coordinator.diagnostic();
        let starting_is_seeking = self.latest_observation.as_ref().is_some_and(|observation| {
            observation.phase == Some(sorotte_player_api::PlayerTransportPhase::Seeking)
                || observation.seeking == Some(true)
        });
        if diagnostic == PlaybackDiagnostic::Starting && starting_is_seeking {
            return ParticipantPlaybackPhase::Seeking;
        }
        match diagnostic {
            PlaybackDiagnostic::Empty => ParticipantPlaybackPhase::Empty,
            PlaybackDiagnostic::Loading => ParticipantPlaybackPhase::Loading,
            PlaybackDiagnostic::Prebuffering => ParticipantPlaybackPhase::Prebuffering,
            PlaybackDiagnostic::ReadyWaitingForRoom => ParticipantPlaybackPhase::ReadyPaused,
            PlaybackDiagnostic::Starting => ParticipantPlaybackPhase::Loading,
            PlaybackDiagnostic::Playing => ParticipantPlaybackPhase::Playing,
            PlaybackDiagnostic::Rebuffering => ParticipantPlaybackPhase::Rebuffering,
            PlaybackDiagnostic::RecoveringByCatchup => ParticipantPlaybackPhase::Playing,
            PlaybackDiagnostic::RecoveringBySeek => ParticipantPlaybackPhase::Seeking,
            PlaybackDiagnostic::Degraded => ParticipantPlaybackPhase::Unknown,
            PlaybackDiagnostic::Ended => ParticipantPlaybackPhase::Ended,
            PlaybackDiagnostic::Failed => ParticipantPlaybackPhase::Failed,
        }
    }

    pub(super) fn participant_status_state_revision_for_generation(
        session: &ClientSession,
        media_generation: u64,
    ) -> Option<u64> {
        session
            .playback_barrier_active_commit()
            .filter(|commit| commit.media_generation == media_generation)
            .map(|commit| commit.state_revision)
            .or_else(|| {
                session
                    .playback_barrier_buffering_policy()
                    .filter(|policy| policy.media_generation == media_generation)
                    .and_then(|policy| policy.state_revision)
            })
    }

    pub(super) fn accepted_participant_status_media_generation(
        &self,
        session: &ClientSession,
    ) -> Option<u64> {
        let operation = self.barrier.accepted_barrier.as_ref()?;
        if self.coordinator.current_media_generation() != Some(operation.local_media_generation)
            || session.room() != Some(operation.room.as_str())
        {
            return None;
        }

        session
            .playback_barrier_prepare()
            .filter(|prepare| {
                prepare.request_id.as_deref() == Some(operation.request_id.as_str())
                    && prepare.request_nonce == operation.request_nonce
                    && logical_media_ids_match(
                        &prepare.logical_media_id,
                        &operation.logical_media_id,
                    )
            })
            .map(|prepare| prepare.media_generation)
            .or_else(|| {
                session
                    .playback_barrier_buffering_policy()
                    .filter(|policy| {
                        policy.request_id.as_deref() == Some(operation.request_id.as_str())
                            && policy.request_nonce == operation.request_nonce
                    })
                    .map(|policy| policy.media_generation)
            })
    }

    pub(super) fn refresh_participant_status_room_scope(&mut self, session: &ClientSession) {
        let Some(local_media_generation) = self.coordinator.current_media_generation() else {
            self.participant_status.participant_status_room_scope = None;
            return;
        };
        let Some(room) = session.room() else {
            self.participant_status.participant_status_room_scope = None;
            return;
        };
        if self
            .participant_status
            .participant_status_room_scope
            .as_ref()
            .is_some_and(|scope| {
                scope.local_media_generation != local_media_generation || scope.room != room
            })
        {
            self.participant_status.participant_status_room_scope = None;
        }

        if let Some(authoritative) = session.participant_status_authoritative_scope() {
            self.participant_status.participant_status_room_scope =
                Some(ParticipantStatusRoomScope {
                    room: room.to_owned(),
                    local_media_generation,
                    media_generation: authoritative.media_generation,
                    state_revision: authoritative.state_revision,
                    transport_revision: authoritative.transport_revision,
                });
            return;
        }

        let accepted_generation = self.accepted_participant_status_media_generation(session);
        let adopted_generation = self.desired_fingerprint.as_ref().and_then(|desired| {
            desired
                .barrier_media_generation
                .or(desired.buffering_media_generation)
        });
        let adopted_generation = adopted_generation.filter(|media_generation| {
            session.playback_barrier_prepare().is_some_and(|prepare| {
                prepare.media_generation == *media_generation
                    && self.current_logical_media_matches(&prepare.logical_media_id)
            })
        });
        if let Some(media_generation) = accepted_generation.or(adopted_generation) {
            self.participant_status.participant_status_room_scope =
                Some(ParticipantStatusRoomScope {
                    room: room.to_owned(),
                    local_media_generation,
                    media_generation,
                    state_revision: Self::participant_status_state_revision_for_generation(
                        session,
                        media_generation,
                    ),
                    transport_revision: None,
                });
        } else if let Some(scope) = self
            .participant_status
            .participant_status_room_scope
            .as_mut()
        {
            scope.state_revision = Self::participant_status_state_revision_for_generation(
                session,
                scope.media_generation,
            )
            .or(scope.state_revision);
        }
    }

    pub(super) fn participant_status_generation_and_revision(
        &mut self,
        session: &ClientSession,
    ) -> (Option<u64>, Option<u64>, Option<u64>) {
        self.refresh_participant_status_room_scope(session);
        self.participant_status
            .participant_status_room_scope
            .as_ref()
            .zip(
                self.participant_status
                    .participant_status_applied_room_scope
                    .as_ref(),
            )
            .filter(|(current, applied)| current == applied)
            .map_or((None, None, None), |(scope, _)| {
                (
                    Some(scope.media_generation),
                    scope.state_revision,
                    scope.transport_revision,
                )
            })
    }

    pub(super) fn participant_status_telemetry_wait_is_current(
        &mut self,
        now_seconds: f64,
    ) -> bool {
        let waiting_since = *self
            .transport_telemetry_wait_started_at_seconds
            .get_or_insert(now_seconds);
        if !now_seconds.is_finite() || !waiting_since.is_finite() || now_seconds < waiting_since {
            // A lifecycle timer is evidence too. Once its owner clock rolls
            // back, merely catching up to the old timestamp must not revive
            // Starting without a new lifecycle transition or player sample.
            self.participant_status_owner_clock_invalidated = true;
            self.transport_telemetry_wait_started_at_seconds = None;
            return false;
        }
        now_seconds - waiting_since <= PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS
    }

    pub(super) fn participant_status_player_availability(
        &mut self,
        now_seconds: f64,
    ) -> ParticipantPlayerConnection {
        match self.external_player_availability {
            Some(ExternalPlayerAvailability::Unavailable) => {
                return ParticipantPlayerConnection::Unavailable;
            }
            Some(ExternalPlayerAvailability::Disconnected) => {
                return ParticipantPlayerConnection::Disconnected;
            }
            Some(ExternalPlayerAvailability::Failed) => {
                return ParticipantPlayerConnection::Failed;
            }
            Some(
                ExternalPlayerAvailability::Connecting
                | ExternalPlayerAvailability::TelemetryUnavailable,
            )
            | None => {}
        }
        if self.last_external_now_seconds.is_some_and(|last_observed| {
            now_seconds.is_finite() && last_observed.is_finite() && now_seconds < last_observed
        }) {
            // Owner wall-clock rollback is a one-way evidence fence. Merely
            // catching back up cannot make the pre-rollback observation fresh
            // again; only a newly accepted current-epoch sample can rebase it.
            self.participant_status_owner_clock_invalidated = true;
            self.last_transport_telemetry_received_at_seconds = None;
            self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
            self.participant_status.last_participant_status_fingerprint = None;
        }
        if self.participant_status_owner_clock_invalidated {
            return ParticipantPlayerConnection::Unavailable;
        }
        if self.coordinator.diagnostic() == PlaybackDiagnostic::Failed {
            return ParticipantPlayerConnection::Failed;
        }

        let telemetry_is_fresh = self
            .last_transport_telemetry_received_at_seconds
            .is_some_and(|received_at| {
                now_seconds.is_finite()
                    && received_at.is_finite()
                    && now_seconds >= received_at
                    && now_seconds - received_at
                        <= PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS
            });
        if telemetry_is_fresh {
            return ParticipantPlayerConnection::Connected;
        }
        match self.external_player_availability {
            Some(ExternalPlayerAvailability::Connecting) => {
                if self.participant_status_telemetry_wait_is_current(now_seconds) {
                    ParticipantPlayerConnection::Starting
                } else {
                    ParticipantPlayerConnection::Unavailable
                }
            }
            Some(ExternalPlayerAvailability::TelemetryUnavailable) => {
                ParticipantPlayerConnection::Unavailable
            }
            Some(
                ExternalPlayerAvailability::Unavailable
                | ExternalPlayerAvailability::Disconnected
                | ExternalPlayerAvailability::Failed,
            ) => {
                unreachable!("terminal external availability returned above")
            }
            None if self.transport_telemetry_available
                && self.last_transport_telemetry_received_at_seconds.is_none() =>
            {
                if self.participant_status_telemetry_wait_is_current(now_seconds) {
                    ParticipantPlayerConnection::Starting
                } else {
                    ParticipantPlayerConnection::Unavailable
                }
            }
            None => ParticipantPlayerConnection::Unavailable,
        }
    }

    pub(in crate::runtime) fn pending_participant_status_report(
        &mut self,
        session: &ClientSession,
        force: bool,
        now_seconds: f64,
    ) -> Option<PendingParticipantStatusReport> {
        if !now_seconds.is_finite() {
            // Never commit an invalid owner timestamp. In particular, a first
            // transition at infinity must not poison every later unchanged
            // heartbeat by becoming the remembered send time.
            return None;
        }
        if !session.is_active()
            || !session.server_participant_status_v1_supported()
            || session.room().is_none()
            || self
                .participant_status
                .pending_participant_status_room_switch_target
                .is_some()
        {
            self.participant_status.last_participant_status_fingerprint = None;
            return None;
        }

        let player = self.participant_status_player_availability(now_seconds);
        let phase = self.participant_status_phase();
        let timeline_kind = self
            .latest_observation
            .as_ref()
            .and_then(|observation| observation.timeline_kind)
            .map_or(ParticipantTimelineKind::Unknown, |kind| match kind {
                sorotte_player_api::PlayerTimelineKind::Vod => ParticipantTimelineKind::Vod,
                sorotte_player_api::PlayerTimelineKind::SlidingLive => {
                    ParticipantTimelineKind::Live
                }
                sorotte_player_api::PlayerTimelineKind::Unknown => ParticipantTimelineKind::Unknown,
            });
        let paused_for_cache = self
            .latest_observation
            .as_ref()
            .and_then(|observation| observation.paused_for_cache);
        let (media_generation, state_revision, transport_revision) =
            self.participant_status_generation_and_revision(session);
        let fingerprint = ParticipantStatusFingerprint {
            room: session.room().unwrap_or_default().to_owned(),
            player,
            phase,
            timeline_kind,
            paused_for_cache,
            media_generation,
            state_revision,
            transport_revision,
            local_media_generation: self.coordinator.current_media_generation(),
            coordination_revision: self.desired_revision,
        };
        let fingerprint_changed = self
            .participant_status
            .last_participant_status_fingerprint
            .as_ref()
            != Some(&fingerprint);
        if !fingerprint_changed {
            if !force {
                return None;
            }
            let heartbeat_due = self
                .participant_status
                .last_participant_status_sent_at_seconds
                .is_none_or(|last_sent| {
                    now_seconds.is_finite()
                        && last_sent.is_finite()
                        && (now_seconds < last_sent
                            || now_seconds - last_sent >= PARTICIPANT_STATUS_HEARTBEAT_SECONDS)
                });
            if !heartbeat_due {
                return None;
            }
        }

        let sequence = self
            .participant_status
            .next_participant_status_sequence
            .checked_add(1)?;
        let observation = (player == ParticipantPlayerConnection::Connected)
            .then_some(self.latest_observation.as_ref())
            .flatten();
        let mut report =
            ParticipantStatusReport::new(sequence, player, phase).with_timeline_kind(timeline_kind);
        let mut oldest_evidence_at: Option<f64> = None;
        let mut note_evidence = |observed_at: Option<f64>| {
            if let Some(observed_at) = observed_at.filter(|value| value.is_finite()) {
                oldest_evidence_at =
                    Some(oldest_evidence_at.map_or(observed_at, |oldest| oldest.min(observed_at)));
                true
            } else {
                false
            }
        };
        let position_evidence_at = self
            .participant_status_evidence_times
            .position
            .filter(|value| value.is_finite());
        report.position_seconds = observation
            .and_then(|observation| observation.position_seconds)
            .filter(|value| {
                value.is_finite() && (0.0..=PARTICIPANT_STATUS_MAX_POSITION_SECONDS).contains(value)
            })
            .filter(|_| note_evidence(position_evidence_at));
        report.logical_paused = observation
            .and_then(|observation| observation.logical_pause)
            .filter(|_| note_evidence(self.participant_status_evidence_times.logical_pause));
        report.playback_rate = observation
            .and_then(|observation| observation.playback_rate)
            .filter(|value| {
                value.is_finite()
                    && (PARTICIPANT_STATUS_MIN_PLAYBACK_RATE..=PARTICIPANT_STATUS_MAX_PLAYBACK_RATE)
                        .contains(value)
            })
            .filter(|_| note_evidence(self.participant_status_evidence_times.playback_rate));
        report.paused_for_cache = observation
            .and_then(|observation| observation.paused_for_cache)
            .filter(|_| note_evidence(self.participant_status_evidence_times.paused_for_cache));
        report.buffered_ahead_seconds = observation
            .and_then(|observation| observation.buffered_ahead_seconds)
            .filter(|value| {
                value.is_finite()
                    && (0.0..=PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS).contains(value)
            })
            .filter(|_| note_evidence(self.participant_status_evidence_times.buffered_ahead));
        report.cache_percent = observation
            .and_then(|observation| observation.cache_buffering_percent)
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
            .filter(|_| note_evidence(self.participant_status_evidence_times.cache_percent));
        let report_now = self.coordinator_now(now_seconds);
        let evidence_age_ms = |observed_at: f64| {
            let age_seconds = report_now - observed_at;
            (age_seconds.is_finite() && age_seconds >= 0.0).then(|| {
                (age_seconds * 1_000.0)
                    .min(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS as f64)
                    .round() as u64
            })
        };
        report.sample_age_ms = oldest_evidence_at.and_then(evidence_age_ms);
        report.position_sample_age_ms = report
            .position_seconds
            .and(position_evidence_at)
            .and_then(evidence_age_ms);
        if report.position_seconds.is_some() && report.position_sample_age_ms.is_none() {
            report.position_seconds = None;
        }
        if oldest_evidence_at.is_some() && report.sample_age_ms.is_none() {
            // Never serialize precise evidence without a trustworthy age. A
            // rolled-back or inconsistent clock must reduce detail rather
            // than make an old sparse field appear newly sampled.
            report.position_seconds = None;
            report.logical_paused = None;
            report.playback_rate = None;
            report.paused_for_cache = None;
            report.cache_percent = None;
            report.buffered_ahead_seconds = None;
            report.position_sample_age_ms = None;
        }
        report.playback_scope = media_generation.map(|media_generation| {
            let mut scope = ParticipantPlaybackScope::new(media_generation);
            scope.state_revision = state_revision;
            scope.transport_revision = transport_revision;
            scope
        });
        report.redact_ineligible_media_evidence();

        Some(PendingParticipantStatusReport {
            report,
            fingerprint,
            sent_at_seconds: now_seconds,
        })
    }

    pub(in crate::runtime) fn commit_participant_status_report(
        &mut self,
        pending: &PendingParticipantStatusReport,
    ) {
        self.participant_status.next_participant_status_sequence = pending.report.report_sequence;
        self.participant_status.last_participant_status_fingerprint =
            Some(pending.fingerprint.clone());
        self.participant_status
            .last_participant_status_sent_at_seconds = Some(pending.sent_at_seconds);
    }

    #[cfg(test)]
    pub(crate) fn take_participant_status_report(
        &mut self,
        session: &ClientSession,
        force: bool,
        now_seconds: f64,
    ) -> Option<ParticipantStatusReport> {
        let pending = self.pending_participant_status_report(session, force, now_seconds)?;
        self.commit_participant_status_report(&pending);
        Some(pending.report)
    }

    pub(crate) fn begin_participant_status_room_switch(
        &mut self,
        target_room: &str,
        current_room: Option<&str>,
    ) {
        self.participant_status
            .pending_participant_status_room_switch_target =
            (current_room != Some(target_room)).then(|| target_room.to_owned());
        self.participant_status.last_participant_status_fingerprint = None;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
    }

    pub(crate) fn confirm_participant_status_room_membership(&mut self, session: &ClientSession) {
        if self
            .participant_status
            .pending_participant_status_room_switch_target
            .as_deref()
            .is_some_and(|target_room| {
                session.username().is_some_and(|username| {
                    session.room() == Some(target_room)
                        && session.user_room(username) == Some(target_room)
                })
            })
        {
            self.participant_status
                .pending_participant_status_room_switch_target = None;
            self.participant_status.last_participant_status_fingerprint = None;
        }
    }

    pub(super) fn update_participant_status_evidence_times(
        &mut self,
        update: &PlayerTransportObservation,
        replace_previous_state: bool,
    ) {
        if replace_previous_state {
            self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        }
        let observed_at = update.observed_at_seconds;
        if update.position_seconds.is_some() {
            self.participant_status_evidence_times.position = Some(observed_at);
        }
        if update.logical_pause.is_some() {
            self.participant_status_evidence_times.logical_pause = Some(observed_at);
        }
        if update.playback_rate.is_some() {
            self.participant_status_evidence_times.playback_rate = Some(observed_at);
        }
        if update.paused_for_cache.is_some() {
            self.participant_status_evidence_times.paused_for_cache = Some(observed_at);
        }
        if update.cache_buffering_percent.is_some() {
            self.participant_status_evidence_times.cache_percent = Some(observed_at);
        }
        if update.buffered_ahead_seconds.is_some() {
            self.participant_status_evidence_times.buffered_ahead = Some(observed_at);
        }
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub(super) fn emit_participant_status_transition(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        let Some(pending) = self
            .playback_coordination
            .pending_participant_status_report(&self.session, false, now_seconds)
        else {
            return Ok(false);
        };
        self.control.activate_protocol_connection_generation();
        self.control
            .emit(ClientEffect::SendState(
                StatePayload::new().with_participant_status_v1(
                    ParticipantStatusStateExtension::new().with_report(pending.report.clone()),
                ),
            ))
            .map_err(client_effect_player_error)?;
        self.playback_coordination
            .commit_participant_status_report(&pending);
        Ok(true)
    }
}

#[derive(Debug, Default)]
pub(super) struct ParticipantStatusReportingState {
    pub(super) next_participant_status_sequence: u64,
    pub(super) last_participant_status_fingerprint: Option<ParticipantStatusFingerprint>,
    pub(super) last_participant_status_sent_at_seconds: Option<f64>,
    pub(super) participant_status_room_scope: Option<ParticipantStatusRoomScope>,
    pub(super) participant_status_applied_room_scope: Option<ParticipantStatusRoomScope>,
    pub(super) participant_status_desired_scope_bindings:
        BTreeMap<(u64, u64), ParticipantStatusRoomScope>,
    pub(super) pending_participant_status_room_switch_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParticipantStatusFingerprint {
    pub(super) room: String,
    pub(super) player: ParticipantPlayerConnection,
    pub(super) phase: ParticipantPlaybackPhase,
    pub(super) timeline_kind: ParticipantTimelineKind,
    pub(super) paused_for_cache: Option<bool>,
    pub(super) media_generation: Option<u64>,
    pub(super) state_revision: Option<u64>,
    pub(super) transport_revision: Option<u64>,
    pub(super) local_media_generation: Option<u64>,
    pub(super) coordination_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParticipantStatusRoomScope {
    pub(super) room: String,
    pub(super) local_media_generation: u64,
    pub(super) media_generation: u64,
    pub(super) state_revision: Option<u64>,
    pub(super) transport_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct ParticipantStatusEvidenceTimes {
    pub(super) position: Option<f64>,
    pub(super) logical_pause: Option<f64>,
    pub(super) playback_rate: Option<f64>,
    pub(super) paused_for_cache: Option<f64>,
    pub(super) cache_percent: Option<f64>,
    pub(super) buffered_ahead: Option<f64>,
}

#[derive(Debug, Clone)]
pub(in crate::runtime) struct PendingParticipantStatusReport {
    pub(in crate::runtime) report: ParticipantStatusReport,
    pub(super) fingerprint: ParticipantStatusFingerprint,
    pub(super) sent_at_seconds: f64,
}
