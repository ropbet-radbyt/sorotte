//! Ordered player delivery owns attachment/sequence continuity, physical load
//! bindings, transport snapshots and acknowledged semantic outcomes. Inputs are
//! typed player batches, snapshots and attempt identities; outputs are validated
//! ordered deliveries and owned transport projections. New attachment epochs
//! reset all state; acknowledgement compacts only already-applied outcomes.
//! Room authority and advisory participant reporting remain separate owners.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OrderedLoadBinding {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) command_id: Option<PlayerCommandId>,
    pub(super) playlist_entry_id: Option<i64>,
    pub(super) owns_transport: bool,
    pub(super) semantic_load_result: Option<PlayerLoadAttemptResult>,
    pub(super) physical_terminal: bool,
    pub(super) logical_ownership_revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OrderedLoadInstall {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) command_id: Option<PlayerCommandId>,
    pub(super) playlist_entry_id: Option<i64>,
    pub(super) owns_transport: bool,
    pub(super) semantic_load_result: Option<PlayerLoadAttemptResult>,
    pub(super) logical_ownership_revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct OrderedPlayerEventConsumer {
    pub(super) attachment_epoch: Option<PlayerAttachmentEpoch>,
    pub(super) last_sequence: u64,
    pub(super) last_snapshot_boundary: Option<PlayerSequenceBoundary>,
    pub(super) transport: PlayerTransportSnapshot,
    pub(super) attempts: BTreeMap<LoadAttemptId, OrderedLoadBinding>,
    pub(super) transport_owner_attempt: Option<LoadAttemptId>,
    pub(super) acknowledged_semantic_sequence: u64,
    pub(super) applied_semantic_outcomes: BTreeSet<PlayerEventOrder>,
    pub(super) applied_unacknowledged_token: Option<PlayerEventAcknowledgementToken>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum OrderedPlayerDelivery {
    Event(SequencedPlayerEvent),
    SemanticOutcome(SequencedPlayerSemanticOutcome),
}

impl OrderedPlayerDelivery {
    pub(super) fn order(&self) -> PlayerEventOrder {
        match self {
            Self::Event(event) => event.order,
            Self::SemanticOutcome(outcome) => outcome.order,
        }
    }
}

pub(super) fn ordered_batch_error(message: impl Into<String>) -> PlayerError {
    PlayerError::OperationFailed(format!(
        "invalid ordered player event batch: {}",
        message.into()
    ))
}

pub(super) fn snapshot_known_copy<T: Copy>(field: &SnapshotField<T>) -> Option<T> {
    match field {
        SnapshotField::Known(value) => Some(*value),
        SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
    }
}

pub(super) fn snapshot_known_clone<T: Clone>(field: &SnapshotField<T>) -> Option<T> {
    match field {
        SnapshotField::Known(value) => Some(value.clone()),
        SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
    }
}

pub(super) fn local_file_updates_share_identity(
    left: &LocalFileUpdate,
    right: &LocalFileUpdate,
) -> bool {
    match (&left.path, &right.path) {
        (Some(left_path), Some(right_path)) => left_path == right_path,
        _ => left.name == right.name,
    }
}

#[cfg(feature = "test-support")]
pub(super) fn verification_optional_field<T>(value: Option<T>) -> SnapshotField<T> {
    match value {
        Some(value) => SnapshotField::Known(value),
        None => SnapshotField::KnownAbsent,
    }
}

impl OrderedPlayerEventConsumer {
    pub(super) fn reset_for_epoch(&mut self, attachment_epoch: PlayerAttachmentEpoch) {
        self.attachment_epoch = Some(attachment_epoch);
        self.last_sequence = 0;
        self.last_snapshot_boundary = None;
        self.transport = PlayerTransportSnapshot::default();
        self.attempts.clear();
        self.transport_owner_attempt = None;
        self.acknowledged_semantic_sequence = 0;
        self.applied_semantic_outcomes.clear();
        self.applied_unacknowledged_token = None;
    }

    pub(super) fn begin_batch(&mut self, batch: &PlayerEventBatch) -> Result<(), PlayerError> {
        if batch.sequence_boundary.attachment_epoch != batch.attachment_epoch {
            return Err(ordered_batch_error(
                "batch sequence boundary belongs to another attachment",
            ));
        }
        if batch.acknowledgement_token.attachment_epoch() != batch.attachment_epoch {
            return Err(ordered_batch_error(
                "acknowledgement token belongs to another attachment",
            ));
        }
        if let Some(snapshot) = batch.authoritative_snapshot.as_ref()
            && (snapshot.attachment_epoch != batch.attachment_epoch
                || snapshot.sequence_boundary.attachment_epoch != batch.attachment_epoch
                || snapshot.sequence_boundary.through_sequence
                    > batch.sequence_boundary.through_sequence)
        {
            return Err(ordered_batch_error(
                "authoritative snapshot boundary is inconsistent with its batch",
            ));
        }

        match self.attachment_epoch {
            None => self.reset_for_epoch(batch.attachment_epoch),
            Some(current) if batch.attachment_epoch < current => {
                return Err(ordered_batch_error("stale attachment epoch"));
            }
            Some(current) if batch.attachment_epoch > current => {
                let replacement_announced = batch.events.iter().any(|event| {
                    event.order.attachment_epoch == batch.attachment_epoch
                        && matches!(
                            event.event,
                            PlayerEvent::AttachmentReplaced { previous_epoch }
                                if previous_epoch == current
                        )
                });
                if batch.authoritative_snapshot.is_none() && !replacement_announced {
                    return Err(ordered_batch_error(
                        "new attachment epoch lacks replacement evidence",
                    ));
                }
                self.reset_for_epoch(batch.attachment_epoch);
            }
            Some(_) => {}
        }

        let mut previous = None;
        let mut orders = batch
            .events
            .iter()
            .map(|event| event.order)
            .chain(batch.semantic_outcomes.iter().map(|outcome| outcome.order))
            .collect::<Vec<_>>();
        orders.sort_unstable();
        for order in orders {
            if order.attachment_epoch != batch.attachment_epoch {
                return Err(ordered_batch_error(
                    "delivery belongs to another attachment",
                ));
            }
            if order.sequence > batch.sequence_boundary.through_sequence {
                return Err(ordered_batch_error(
                    "delivery exceeds the batch sequence boundary",
                ));
            }
            if previous == Some(order.sequence) {
                return Err(ordered_batch_error("duplicate delivery order"));
            }
            previous = Some(order.sequence);
        }
        Ok(())
    }

    pub(super) fn merged_deliveries(batch: &PlayerEventBatch) -> Vec<OrderedPlayerDelivery> {
        let mut deliveries = batch
            .events
            .iter()
            .cloned()
            .map(OrderedPlayerDelivery::Event)
            .chain(
                batch
                    .semantic_outcomes
                    .iter()
                    .cloned()
                    .map(OrderedPlayerDelivery::SemanticOutcome),
            )
            .collect::<Vec<_>>();
        deliveries.sort_by_key(OrderedPlayerDelivery::order);
        deliveries
    }

    pub(super) fn validate_sequence_continuity(
        &self,
        batch: &PlayerEventBatch,
    ) -> Result<(), PlayerError> {
        let snapshot_boundary = batch
            .authoritative_snapshot
            .as_ref()
            .filter(|snapshot| self.should_rebase_snapshot(snapshot.sequence_boundary))
            .map(|snapshot| snapshot.sequence_boundary.through_sequence);
        let mut cursor = snapshot_boundary.map_or(self.last_sequence, |boundary| {
            self.last_sequence.max(boundary)
        });
        for delivery in Self::merged_deliveries(batch) {
            let order = delivery.order();
            let covered_event = matches!(delivery, OrderedPlayerDelivery::Event(_))
                && snapshot_boundary.is_some_and(|boundary| order.sequence <= boundary);
            let covered_outcome = matches!(delivery, OrderedPlayerDelivery::SemanticOutcome(_))
                && order.sequence <= cursor;
            if covered_event || covered_outcome || order.sequence <= cursor {
                continue;
            }
            let expected = cursor.saturating_add(1);
            if order.sequence != expected {
                return Err(ordered_batch_error(format!(
                    "sequence gap: expected {expected}, received {}",
                    order.sequence
                )));
            }
            cursor = order.sequence;
        }
        Ok(())
    }

    pub(super) fn should_rebase_snapshot(&self, boundary: PlayerSequenceBoundary) -> bool {
        self.last_snapshot_boundary
            .is_none_or(|current| boundary.through_sequence > current.through_sequence)
    }

    pub(super) fn rebase_snapshot(
        &mut self,
        snapshot: &sorotte_player_api::PlayerAuthoritativeSnapshot,
    ) {
        self.transport.rebase(snapshot.transport.clone());
        self.attempts.clear();
        self.transport_owner_attempt = None;
        if let SnapshotField::Known(active) = &snapshot.active_load {
            self.install_active_load(*active);
        }
        self.last_snapshot_boundary = Some(snapshot.sequence_boundary);
        self.last_sequence = self
            .last_sequence
            .max(snapshot.sequence_boundary.through_sequence);
    }

    pub(super) fn install_active_load(&mut self, active: PlayerActiveLoadSnapshot) {
        self.install_attempt(
            active.attempt_id,
            OrderedLoadInstall {
                media_generation: active.media_generation,
                command_id: active.command_id,
                playlist_entry_id: active.playlist_entry_id,
                owns_transport: true,
                semantic_load_result: active.semantic_load_result,
                logical_ownership_revoked: active.logical_ownership_revoked,
            },
        );
    }

    pub(super) fn install_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        install: OrderedLoadInstall,
    ) {
        let OrderedLoadInstall {
            media_generation,
            command_id,
            playlist_entry_id,
            owns_transport,
            semantic_load_result,
            logical_ownership_revoked,
        } = install;
        let existing = self.attempts.get(&attempt_id).copied();
        let physical_terminal = existing.is_some_and(|binding| binding.physical_terminal);
        let owns_transport = owns_transport && !physical_terminal;
        self.attempts.insert(
            attempt_id,
            OrderedLoadBinding {
                media_generation,
                command_id: command_id.or_else(|| existing.and_then(|binding| binding.command_id)),
                playlist_entry_id: playlist_entry_id
                    .or_else(|| existing.and_then(|binding| binding.playlist_entry_id)),
                owns_transport,
                semantic_load_result: semantic_load_result
                    .or_else(|| existing.and_then(|binding| binding.semantic_load_result)),
                physical_terminal,
                logical_ownership_revoked: logical_ownership_revoked
                    || existing.is_some_and(|binding| binding.logical_ownership_revoked),
            },
        );
        if owns_transport {
            for (other_attempt_id, binding) in &mut self.attempts {
                if *other_attempt_id != attempt_id {
                    binding.owns_transport = false;
                }
            }
            self.transport_owner_attempt = Some(attempt_id);
        }
    }

    pub(super) fn ensure_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) {
        if !self.attempts.contains_key(&attempt_id) {
            self.install_attempt(
                attempt_id,
                OrderedLoadInstall {
                    media_generation,
                    command_id,
                    playlist_entry_id: None,
                    owns_transport: false,
                    semantic_load_result: None,
                    logical_ownership_revoked: false,
                },
            );
        }
    }

    pub(super) fn mark_semantic_load_result(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        result: PlayerLoadAttemptResult,
    ) {
        if let Some(binding) = self.attempts.get_mut(&attempt_id)
            && binding.media_generation == media_generation
        {
            binding.semantic_load_result.get_or_insert(result);
        }
    }

    pub(super) fn revoke_logical_ownership(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) {
        if let Some(binding) = self.attempts.get_mut(&attempt_id)
            && binding.media_generation == media_generation
        {
            binding.logical_ownership_revoked = true;
        }
    }

    pub(super) fn attempt_can_clear_timeout_player_failure(
        &self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) -> bool {
        self.attempts.get(&attempt_id).is_some_and(|binding| {
            binding.media_generation == media_generation
                && matches!(
                    binding.semantic_load_result,
                    Some(PlayerLoadAttemptResult::Loaded | PlayerLoadAttemptResult::Indeterminate)
                )
                && !binding.physical_terminal
                && !binding.logical_ownership_revoked
        })
    }

    pub(super) fn attempt_owns_transport(
        &self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) -> bool {
        self.transport_owner_attempt == Some(attempt_id)
            && self.attempts.get(&attempt_id).is_some_and(|binding| {
                binding.media_generation == media_generation
                    && binding.owns_transport
                    && !binding.physical_terminal
            })
    }

    pub(super) fn mark_indeterminate(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) {
        let Some(binding) = self.attempts.get_mut(&attempt_id) else {
            return;
        };
        if binding.media_generation != media_generation {
            return;
        }
        binding
            .semantic_load_result
            .get_or_insert(PlayerLoadAttemptResult::Indeterminate);
    }

    pub(super) fn terminate_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) {
        let Some(binding) = self.attempts.get_mut(&attempt_id) else {
            return;
        };
        if binding.media_generation != media_generation {
            return;
        }
        binding.physical_terminal = true;
        binding.owns_transport = false;
        if self.transport_owner_attempt == Some(attempt_id) {
            self.transport_owner_attempt = None;
        }
    }

    pub(super) fn apply_delta_if_owned(
        &mut self,
        delta: PlayerTransportDelta,
    ) -> Option<PlayerTransportDelta> {
        let mut candidate = self.transport.clone();
        candidate.apply_delta(delta.clone());
        let attempt_id = snapshot_known_copy(&candidate.load_attempt_id)?;
        let media_generation = snapshot_known_copy(&candidate.media_generation)?;
        let binding = self.attempts.get(&attempt_id)?;
        if binding.physical_terminal
            || !binding.owns_transport
            || binding.media_generation != media_generation
            || self.transport_owner_attempt != Some(attempt_id)
        {
            return None;
        }
        self.transport = candidate;
        Some(delta)
    }

    pub(super) fn event_is_covered_by_snapshot(&self, order: PlayerEventOrder) -> bool {
        self.last_snapshot_boundary.is_some_and(|boundary| {
            boundary.attachment_epoch == order.attachment_epoch
                && order.sequence <= boundary.through_sequence
        })
    }

    pub(super) fn require_next_order(&self, order: PlayerEventOrder) -> Result<(), PlayerError> {
        if order.sequence <= self.last_sequence {
            return Ok(());
        }
        let expected = self.last_sequence.saturating_add(1);
        if order.sequence != expected {
            return Err(ordered_batch_error(format!(
                "sequence gap: expected {expected}, received {}",
                order.sequence
            )));
        }
        Ok(())
    }

    pub(super) fn record_order(&mut self, order: PlayerEventOrder) {
        self.last_sequence = self.last_sequence.max(order.sequence);
    }

    pub(super) fn semantic_outcome_was_applied(&self, order: PlayerEventOrder) -> bool {
        order.sequence <= self.acknowledged_semantic_sequence
            || self.applied_semantic_outcomes.contains(&order)
    }

    pub(super) fn record_semantic_outcome(&mut self, order: PlayerEventOrder) {
        self.applied_semantic_outcomes.insert(order);
        self.record_order(order);
    }

    pub(super) fn compact_acknowledged_delivery(
        &mut self,
        acknowledgement_token: PlayerEventAcknowledgementToken,
        sequence_boundary: PlayerSequenceBoundary,
    ) {
        if self.applied_unacknowledged_token != Some(acknowledgement_token) {
            return;
        }
        self.acknowledged_semantic_sequence = self
            .acknowledged_semantic_sequence
            .max(sequence_boundary.through_sequence);
        self.applied_semantic_outcomes.clear();
        let transport_owner_attempt = self.transport_owner_attempt;
        self.attempts.retain(|attempt_id, binding| {
            !binding.physical_terminal || Some(*attempt_id) == transport_owner_attempt
        });
        self.applied_unacknowledged_token = None;
    }

    #[cfg(feature = "test-support")]
    pub(super) fn lifecycle_verification_projection(&self) -> LifecycleVerificationProjection {
        let physical_binding = self
            .transport_owner_attempt
            .and_then(|attempt_id| self.attempts.get(&attempt_id));
        let attempts: BTreeMap<_, _> = self
            .attempts
            .iter()
            .map(|(attempt_id, binding)| {
                (
                    *attempt_id,
                    LifecycleVerificationAttemptProjection {
                        media_generation: binding.media_generation,
                        command_id: binding.command_id,
                        playlist_entry_id: binding.playlist_entry_id,
                        owns_transport: SnapshotField::Known(binding.owns_transport),
                        semantic_load_result: SnapshotField::Known(binding.semantic_load_result),
                        logical_ownership_revoked: SnapshotField::Known(
                            binding.logical_ownership_revoked,
                        ),
                        physical_terminal: SnapshotField::Known(binding.physical_terminal),
                    },
                )
            })
            .collect();
        let terminal_load_results = attempts
            .iter()
            .filter_map(|(attempt_id, attempt)| match attempt.semantic_load_result {
                SnapshotField::Known(Some(result)) => Some((*attempt_id, result)),
                SnapshotField::Known(None)
                | SnapshotField::KnownAbsent
                | SnapshotField::Unavailable => None,
            })
            .collect();

        LifecycleVerificationProjection {
            attachment_epoch: verification_optional_field(self.attachment_epoch),
            sequence_boundary: self.attachment_epoch.map_or(
                SnapshotField::KnownAbsent,
                |attachment_epoch| {
                    SnapshotField::Known(PlayerSequenceBoundary::new(
                        attachment_epoch,
                        self.last_sequence,
                    ))
                },
            ),
            in_flight_acknowledgement: verification_optional_field(
                self.applied_unacknowledged_token,
            ),
            pending_event_count: SnapshotField::Unavailable,
            retained_semantic_outcome_count: SnapshotField::Known(
                self.applied_semantic_outcomes.len(),
            ),
            snapshot_required: SnapshotField::Unavailable,

            physical_transport_owner: verification_optional_field(self.transport_owner_attempt),
            physical_media_generation: verification_optional_field(
                physical_binding.map(|binding| binding.media_generation),
            ),
            physical_playlist_entry_id: match physical_binding {
                Some(binding) => verification_optional_field(binding.playlist_entry_id),
                None => SnapshotField::KnownAbsent,
            },
            physical_path: SnapshotField::Unavailable,
            physical_file_loaded: SnapshotField::Unavailable,
            logical_owner: SnapshotField::Unavailable,

            transport: self.transport.clone(),
            attempts,
            pending_commands: SnapshotField::Unavailable,
            terminal_command_results: SnapshotField::Unavailable,
            terminal_load_results: SnapshotField::Known(terminal_load_results),

            pending_playlist_resolution_attempt: SnapshotField::Unavailable,
            playlist_resolution_state: SnapshotField::Unavailable,
            fallback_pending: SnapshotField::Unavailable,
            player_local_file: SnapshotField::Unavailable,
            player_local_file_placeholder: SnapshotField::Unavailable,
        }
    }
}

impl RuntimePlaybackCoordination {
    pub(super) fn observation_from_ordered_transport(
        &mut self,
        transport: &PlayerTransportSnapshot,
        external_now_seconds: f64,
    ) -> Option<(PlayerTransportObservation, Option<f64>, f64)> {
        if matches!(
            self.external_player_availability,
            Some(
                ExternalPlayerAvailability::Unavailable
                    | ExternalPlayerAvailability::TelemetryUnavailable
                    | ExternalPlayerAvailability::Disconnected
                    | ExternalPlayerAvailability::Failed,
            )
        ) {
            return None;
        }
        let adapter_generation = snapshot_known_copy(&transport.media_generation)?;
        let media_generation =
            self.bind_adapter_generation(adapter_generation, external_now_seconds)?;
        let (observed_at_seconds, candidate_offset_seconds, delivery_reference_seconds) = self
            .map_observation_time(
                snapshot_known_copy(&transport.observed_at),
                external_now_seconds,
            );
        Some((
            PlayerTransportObservation {
                media_generation,
                observed_at_seconds,
                phase: snapshot_known_copy(&transport.phase),
                position_seconds: snapshot_known_copy(&transport.position_seconds),
                playback_rate: snapshot_known_copy(&transport.playback_rate),
                logical_pause: snapshot_known_copy(&transport.logical_pause),
                paused_for_cache: snapshot_known_copy(&transport.paused_for_cache),
                seeking: snapshot_known_copy(&transport.seeking),
                seekable: snapshot_known_copy(&transport.seekable),
                timeline_kind: snapshot_known_copy(&transport.timeline_kind),
                seekable_ranges: snapshot_known_clone(&transport.seekable_ranges),
                known_live_seekable_window: snapshot_known_copy(
                    &transport.known_live_seekable_window,
                ),
                core_idle: snapshot_known_copy(&transport.core_idle),
                playback_restart_sequence: snapshot_known_copy(
                    &transport.playback_restart_sequence,
                ),
                cache_buffering_percent: snapshot_known_copy(&transport.cache_percentage),
                buffered_ahead_seconds: snapshot_known_copy(&transport.buffered_duration_seconds),
                input_rate_bytes_per_second: snapshot_known_copy(
                    &transport.input_rate_bytes_per_second,
                ),
            },
            candidate_offset_seconds,
            delivery_reference_seconds,
        ))
    }

    pub(super) fn position_update_from_ordered_delta(
        delta: &PlayerTransportDelta,
        mapped_observation: &PlayerTransportObservation,
    ) -> PlayerTransportObservation {
        PlayerTransportObservation {
            media_generation: mapped_observation.media_generation,
            observed_at_seconds: mapped_observation.observed_at_seconds,
            phase: delta.phase,
            position_seconds: delta.position_seconds,
            playback_rate: delta.playback_rate,
            logical_pause: delta.logical_pause,
            paused_for_cache: delta.paused_for_cache,
            seeking: delta.seeking,
            seekable: None,
            timeline_kind: None,
            seekable_ranges: None,
            known_live_seekable_window: None,
            core_idle: delta.core_idle,
            playback_restart_sequence: None,
            cache_buffering_percent: None,
            buffered_ahead_seconds: None,
            input_rate_bytes_per_second: None,
        }
    }

    pub(super) fn fence_ordered_transport_until_snapshot(&mut self, external_now_seconds: f64) {
        let lifecycle_fence = self.coordinator_now(external_now_seconds);
        self.awaiting_ordered_snapshot = true;
        self.latest_observation = None;
        self.latest_position_observation = None;
        self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        self.transport_telemetry_observed = false;
        self.last_transport_telemetry_received_at_seconds = None;
        self.external_player_availability = Some(ExternalPlayerAvailability::Connecting);
        self.external_player_lifecycle_observed = true;
        self.transport_telemetry_wait_started_at_seconds = Some(external_now_seconds);
        self.transport_telemetry_lifecycle_fence_at_seconds =
            lifecycle_fence.is_finite().then_some(lifecycle_fence);
        self.participant_status_owner_clock_invalidated = false;
        self.participant_status.last_participant_status_fingerprint = None;
        let coordinator_now = lifecycle_fence;
        self.coordinator
            .reset_transport_adapter_epoch(coordinator_now);
    }

    pub(crate) fn ordered_transport_awaits_snapshot(&self) -> bool {
        self.awaiting_ordered_snapshot
    }

    pub(crate) fn rebase_ordered_transport_snapshot(
        &mut self,
        transport: &PlayerTransportSnapshot,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.awaiting_ordered_snapshot = false;
        self.transport_telemetry_available = true;
        self.latest_observation = None;
        self.latest_position_observation = None;
        self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        self.transport_telemetry_observed = false;
        let Some((observation, candidate_offset_seconds, delivery_reference_seconds)) =
            self.observation_from_ordered_transport(transport, external_now_seconds)
        else {
            return Vec::new();
        };
        let position_update = observation.clone();
        if self.commit_mapped_transport_observation(
            observation.clone(),
            &position_update,
            external_now_seconds,
            delivery_reference_seconds,
            candidate_offset_seconds,
            true,
            true,
        ) != MappedTransportCommitOutcome::Committed
        {
            return Vec::new();
        }
        let actions = self.coordinator.observe(observation);
        self.record_observation_outcomes(&actions);
        actions
    }

    pub(crate) fn observe_ordered_transport_delta(
        &mut self,
        transport: &PlayerTransportSnapshot,
        delta: &PlayerTransportDelta,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if self.awaiting_ordered_snapshot {
            return Vec::new();
        }
        self.transport_telemetry_available = true;
        let Some((observation, candidate_offset_seconds, delivery_reference_seconds)) =
            self.observation_from_ordered_transport(transport, external_now_seconds)
        else {
            return Vec::new();
        };
        let position_update = Self::position_update_from_ordered_delta(delta, &observation);
        if self.commit_mapped_transport_observation(
            observation.clone(),
            &position_update,
            external_now_seconds,
            delivery_reference_seconds,
            candidate_offset_seconds,
            true,
            false,
        ) != MappedTransportCommitOutcome::Committed
        {
            return Vec::new();
        }
        let actions = self.coordinator.observe(observation);
        self.record_observation_outcomes(&actions);
        actions
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub(super) fn record_ordered_playback_projection(&mut self, delta: &PlayerTransportDelta) {
        let update = PlayerPlaybackTelemetryUpdate {
            paused: delta.logical_pause,
            position_seconds: delta.position_seconds,
            playback_rate: delta.playback_rate,
            paused_for_cache: delta.paused_for_cache,
            cache_buffering_percent: delta.cache_percentage,
        };
        self.record_player_playback_projection(update);
    }

    pub(super) fn apply_ordered_coordination_actions(
        &mut self,
        mut actions: Vec<PlaybackCoordinatorAction>,
        now_seconds: f64,
        first_error: &mut Option<PlayerError>,
    ) {
        if let Err(error) = self.handle_latest_player_readiness_observation()
            && first_error.is_none()
        {
            *first_error = Some(error);
        }
        if let Err(error) = self.promote_pending_native_play_before_pause_correction(&mut actions)
            && first_error.is_none()
        {
            *first_error = Some(error);
        }
        if let Err(error) = self.report_playback_barrier_observations(&actions)
            && first_error.is_none()
        {
            *first_error = Some(crate::control::client_effect_player_error(error));
        }
        if let Err(error) = self.execute_playback_coordinator_actions(actions, now_seconds)
            && first_error.is_none()
        {
            *first_error = Some(error);
        }
    }

    pub(super) fn apply_ordered_snapshot(
        &mut self,
        snapshot: &sorotte_player_api::PlayerAuthoritativeSnapshot,
        now_seconds: f64,
        first_error: &mut Option<PlayerError>,
    ) {
        // A reacquisition is a replacement proof boundary. A terminal edge
        // from the discarded prefix cannot safely drive an application side
        // effect after the snapshot has established a possibly newer owner.
        self.pending_natural_playback_completion = None;
        let previous_file = self.last_local_file_update.clone();
        let snapshot_file = match (&snapshot.active_load, &snapshot.current_path) {
            (SnapshotField::Known(active), SnapshotField::Known(path))
                if active.physical_file_loaded =>
            {
                Some(LocalFileUpdate::new(path.clone()).with_path(path.clone()))
            }
            _ => None,
        };
        let matching_pending_file = snapshot_file.as_ref().and_then(|snapshot_file| {
            self.pending_ordered_local_file_updates
                .pending()
                .iter()
                .rev()
                .find(|pending| local_file_updates_share_identity(pending, snapshot_file))
                .cloned()
        });
        let had_matching_pending_file = matching_pending_file.is_some();
        let recovered_file = matching_pending_file.or_else(|| snapshot_file.clone());

        self.ordered_player_events.rebase_snapshot(snapshot);
        self.observe_pending_reconnect_rate_reset(snapshot_known_copy(
            &snapshot.transport.playback_rate,
        ));
        self.pending_player_playback_telemetry_updates = EffectOutbox::default();
        self.pending_ordered_local_file_updates = EffectOutbox::default();
        self.session
            .replace_player_playback_telemetry_from_authoritative_snapshot(
                &PlayerPlaybackTelemetryUpdate {
                    paused: snapshot_known_copy(&snapshot.transport.logical_pause),
                    position_seconds: snapshot_known_copy(&snapshot.transport.position_seconds),
                    playback_rate: snapshot_known_copy(&snapshot.transport.playback_rate),
                    paused_for_cache: snapshot_known_copy(&snapshot.transport.paused_for_cache),
                    cache_buffering_percent: snapshot_known_copy(
                        &snapshot.transport.cache_percentage,
                    ),
                },
            );
        self.last_local_file_update.clone_from(&recovered_file);
        if let Some(recovered_file) = recovered_file
            && (had_matching_pending_file
                || previous_file.as_ref().is_none_or(|previous| {
                    !local_file_updates_share_identity(previous, &recovered_file)
                }))
        {
            self.pending_ordered_local_file_updates
                .push_back(recovered_file);
        }
        let actions = self
            .playback_coordination
            .rebase_ordered_transport_snapshot(&snapshot.transport, now_seconds);
        self.apply_ordered_coordination_actions(actions, now_seconds, first_error);
    }

    pub(super) fn apply_ordered_event(
        &mut self,
        event: SequencedPlayerEvent,
        now_seconds: f64,
        first_error: &mut Option<PlayerError>,
    ) {
        match event.event {
            PlayerEvent::AttachmentReplaced { .. } | PlayerEvent::EventGapDetected => {
                self.pending_natural_playback_completion = None;
                self.pending_player_playback_telemetry_updates = EffectOutbox::default();
                self.session
                    .replace_player_playback_telemetry_from_authoritative_snapshot(
                        &PlayerPlaybackTelemetryUpdate::default(),
                    );
                self.playback_coordination
                    .fence_ordered_transport_until_snapshot(now_seconds);
            }
            PlayerEvent::LocalFileChanged {
                attempt_id,
                media_generation,
                update,
            } => {
                if self
                    .pending_natural_playback_completion
                    .as_ref()
                    .is_some_and(|pending| {
                        pending.attempt_id != Some(attempt_id)
                            || pending.media_generation != Some(media_generation)
                    })
                {
                    self.pending_natural_playback_completion = None;
                }
                if !self
                    .ordered_player_events
                    .attempt_owns_transport(attempt_id, media_generation)
                {
                    return;
                }
                self.last_local_file_update = Some(update.clone());
                self.pending_ordered_local_file_updates.push_back(update);
            }
            PlayerEvent::TransportDelta(delta) => {
                let Some(accepted) = self.ordered_player_events.apply_delta_if_owned(delta) else {
                    return;
                };
                if self.playback_coordination.awaiting_ordered_snapshot {
                    return;
                }
                self.observe_pending_reconnect_rate_reset(accepted.playback_rate);
                self.record_ordered_playback_projection(&accepted);
                let transport = self.ordered_player_events.transport.clone();
                let actions = self.playback_coordination.observe_ordered_transport_delta(
                    &transport,
                    &accepted,
                    now_seconds,
                );
                self.apply_ordered_coordination_actions(actions, now_seconds, first_error);
            }
            PlayerEvent::LoadAttemptBound {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
            } => self.ordered_player_events.install_attempt(
                attempt_id,
                OrderedLoadInstall {
                    media_generation,
                    command_id,
                    playlist_entry_id: Some(playlist_entry_id),
                    owns_transport: false,
                    semantic_load_result: None,
                    logical_ownership_revoked: false,
                },
            ),
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
                owns_transport,
            } => self.ordered_player_events.install_attempt(
                attempt_id,
                OrderedLoadInstall {
                    media_generation,
                    command_id,
                    playlist_entry_id: Some(playlist_entry_id),
                    owns_transport,
                    semantic_load_result: None,
                    logical_ownership_revoked: false,
                },
            ),
            PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
            } => {
                self.ordered_player_events.install_attempt(
                    attempt_id,
                    OrderedLoadInstall {
                        media_generation,
                        command_id,
                        playlist_entry_id: Some(playlist_entry_id),
                        owns_transport: true,
                        semantic_load_result: None,
                        logical_ownership_revoked: false,
                    },
                );
                if self
                    .ordered_player_events
                    .attempt_can_clear_timeout_player_failure(attempt_id, media_generation)
                {
                    self.playback_coordination
                        .clear_timeout_player_failure_for_recovered_load(media_generation);
                }
            }
            PlayerEvent::LoadAttemptLogicalOwnershipRevoked {
                attempt_id,
                media_generation,
                ..
            } => self
                .ordered_player_events
                .revoke_logical_ownership(attempt_id, media_generation),
            PlayerEvent::LoadAttemptTerminal {
                attempt_id,
                media_generation,
                ..
            } => self
                .ordered_player_events
                .terminate_attempt(attempt_id, media_generation),
            PlayerEvent::LogicalPlaybackTerminal {
                attempt_id,
                media_generation,
                outcome,
            } => {
                let terminal_is_owned = self
                    .ordered_player_events
                    .attempts
                    .get(&attempt_id)
                    .is_some_and(|binding| binding.media_generation == media_generation);
                if terminal_is_owned {
                    if outcome == sorotte_player_api::PlayerPhysicalLoadOutcome::Ended {
                        let (playlist_revision, playlist_index) = self
                            .session
                            .current_room_playlist()
                            .map_or((None, None), |playlist| {
                                (Some(playlist.revision), playlist.index)
                            });
                        let playlist_selection_revision =
                            self.session.current_room_playlist_selection_revision();
                        let canonical_playlist_epoch =
                            self.session.current_room_playlist_canonical_epoch();
                        self.pending_natural_playback_completion =
                            Some(PendingNaturalPlaybackCompletion {
                                attempt_id: Some(attempt_id),
                                media_generation: Some(media_generation),
                                playlist_revision,
                                playlist_selection_revision,
                                canonical_playlist_epoch,
                                playlist_index,
                                completed_file: self.last_local_file_update.clone(),
                            });
                    }
                    let mut terminal = self.ordered_player_events.transport.clone();
                    terminal.load_attempt_id = SnapshotField::Known(attempt_id);
                    terminal.media_generation = SnapshotField::Known(media_generation);
                    terminal.phase =
                        SnapshotField::Known(sorotte_player_api::PlayerTransportPhase::Ended);
                    terminal.logical_pause = SnapshotField::Known(true);
                    self.ordered_player_events.transport = terminal.clone();
                    let terminal_delta = PlayerTransportDelta {
                        logical_pause: Some(true),
                        ..PlayerTransportDelta::default()
                    };
                    self.record_ordered_playback_projection(&terminal_delta);
                    let actions = self.playback_coordination.observe_ordered_transport_delta(
                        &terminal,
                        &terminal_delta,
                        now_seconds,
                    );
                    self.apply_ordered_coordination_actions(actions, now_seconds, first_error);
                }
            }
        }
    }

    pub(super) fn apply_ordered_semantic_outcome(
        &mut self,
        outcome: SequencedPlayerSemanticOutcome,
        now_seconds: f64,
        first_error: &mut Option<PlayerError>,
    ) {
        match outcome.outcome {
            PlayerSemanticOutcome::Command(command) => {
                if self
                    .playback_coordination
                    .apply_player_command_outcome(command, now_seconds)
                    && let Err(error) = self.report_player_command_failure_readiness(now_seconds)
                    && first_error.is_none()
                {
                    *first_error = Some(error);
                }
            }
            PlayerSemanticOutcome::LoadAttempt(load) => {
                self.ordered_player_events.ensure_attempt(
                    load.attempt_id,
                    load.media_generation,
                    load.command_id,
                );
                self.ordered_player_events.mark_semantic_load_result(
                    load.attempt_id,
                    load.media_generation,
                    load.result,
                );
                match load.result {
                    PlayerLoadAttemptResult::Loaded => {}
                    PlayerLoadAttemptResult::Superseded => self
                        .ordered_player_events
                        .revoke_logical_ownership(load.attempt_id, load.media_generation),
                    PlayerLoadAttemptResult::Indeterminate => self
                        .ordered_player_events
                        .mark_indeterminate(load.attempt_id, load.media_generation),
                    PlayerLoadAttemptResult::Failed(_)
                    | PlayerLoadAttemptResult::NeverStarted
                    | PlayerLoadAttemptResult::TransportDisconnected => self
                        .ordered_player_events
                        .terminate_attempt(load.attempt_id, load.media_generation),
                }
            }
        }
    }

    pub(super) fn apply_ordered_player_event_batch(
        &mut self,
        batch: &PlayerEventBatch,
        now_seconds: f64,
    ) -> Result<Option<PlayerError>, PlayerError> {
        let mut prepared_consumer = self.ordered_player_events.clone();
        prepared_consumer.begin_batch(batch)?;
        prepared_consumer.validate_sequence_continuity(batch)?;
        self.ordered_player_events = prepared_consumer;
        if let Some(pending) = self.ordered_player_events.applied_unacknowledged_token
            && pending != batch.acknowledgement_token
        {
            return Err(ordered_batch_error(
                "adapter replaced an applied batch before acknowledgement",
            ));
        }
        if self.ordered_player_events.applied_unacknowledged_token
            == Some(batch.acknowledgement_token)
        {
            return Ok(None);
        }

        let mut first_error = None;
        if let Some(snapshot) = batch.authoritative_snapshot.as_ref()
            && self
                .ordered_player_events
                .should_rebase_snapshot(snapshot.sequence_boundary)
        {
            self.apply_ordered_snapshot(snapshot, now_seconds, &mut first_error);
        }

        for delivery in OrderedPlayerEventConsumer::merged_deliveries(batch) {
            let order = delivery.order();
            match delivery {
                OrderedPlayerDelivery::Event(event) => {
                    if self
                        .ordered_player_events
                        .event_is_covered_by_snapshot(order)
                        || order.sequence <= self.ordered_player_events.last_sequence
                    {
                        continue;
                    }
                    self.ordered_player_events.require_next_order(order)?;
                    self.apply_ordered_event(event, now_seconds, &mut first_error);
                    self.ordered_player_events.record_order(order);
                }
                OrderedPlayerDelivery::SemanticOutcome(outcome) => {
                    if self
                        .ordered_player_events
                        .semantic_outcome_was_applied(order)
                    {
                        continue;
                    }
                    if order.sequence > self.ordered_player_events.last_sequence {
                        self.ordered_player_events.require_next_order(order)?;
                    }
                    self.apply_ordered_semantic_outcome(outcome, now_seconds, &mut first_error);
                    self.ordered_player_events.record_semantic_outcome(order);
                }
            }
        }
        self.ordered_player_events.applied_unacknowledged_token = Some(batch.acknowledgement_token);
        Ok(first_error)
    }

    /// Applies an ordered batch through the exact production consumer.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn apply_ordered_player_event_batch_for_verification(
        &mut self,
        batch: &PlayerEventBatch,
        now_seconds: f64,
    ) -> Result<Option<PlayerError>, PlayerError> {
        self.apply_ordered_player_event_batch(batch, now_seconds)
    }

    pub(super) fn drain_ordered_player_events(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        loop {
            let Some(batch) = self.player.take_player_event_batch() else {
                return Ok(());
            };
            let application_error = self.apply_ordered_player_event_batch(&batch, now_seconds)?;
            self.player
                .acknowledge_player_event_batch(batch.acknowledgement_token)?;
            self.ordered_player_events.compact_acknowledged_delivery(
                batch.acknowledgement_token,
                batch.sequence_boundary,
            );
            if let Some(error) = application_error {
                return Err(error);
            }
        }
    }
}
