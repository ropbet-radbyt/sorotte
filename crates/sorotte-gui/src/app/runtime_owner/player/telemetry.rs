use super::*;
use crate::app::runtime_owner::{
    GuiAttachedNativeSeekTracker, GuiAttachedPlayerPositionObservation,
    GuiAttachedSystemSeekFailClosedGuard, GuiAttachedSystemSeekOwnership,
    GuiAttachedSystemSeekOwnershipState, GuiAttachedSystemSeekSource,
    GuiCorePlayerConfigurationHealth, GuiStreamingDegradationOrigin,
};
use sorotte_player_api::{
    PlayerCommandFailureKind, PlayerEventSequence, PlayerLocalFileObservation,
    PlayerMediaLoadObservation, PlayerObservationTimestamp, PlayerOrderedEventKind,
    PlayerTransportPhase, PlayerTransportTelemetryUpdate,
};
use sorotte_player_mpv::{
    MPV_SEEK_COMPLETION_TOLERANCE_SECONDS, MpvNetworkMediaPolicyOutcome,
    MpvNetworkMediaPolicyState, MpvNetworkOptionsHookHealth, MpvNetworkOptionsHookHealthTransition,
    MpvNetworkOptionsRuntimeHealthSnapshot,
};

const ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS: f64 = 1.0;
const ATTACHED_NATIVE_SEEK_MAX_OBSERVATION_AGE_SECONDS: f64 = 2.0;
const ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIFETIME: Duration = Duration::from_secs(65);
const ATTACHED_SYSTEM_SEEK_TIMEOUT_EXTENSION: Duration = Duration::from_secs(60);
const ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIMIT: usize = 8;

#[cfg(test)]
mod lifecycle_verification_tests;

#[derive(Debug, Clone, PartialEq)]
enum GuiAttachedTransportObservationDisposition {
    Accepted {
        update: Box<PlayerTransportTelemetryUpdate>,
        native_seek_classification: Option<bool>,
    },
    Rejected,
}

#[derive(Debug)]
enum GuiAttachedOrderedPlayerEvent {
    CommandProgress(PlayerCommandProgress),
    LocalFile(PlayerLocalFileObservation),
    MediaLoad(PlayerMediaLoadObservation),
    Transport(PlayerTransportTelemetryUpdate),
}

#[derive(Debug)]
struct GuiAttachedSequencedPlayerEvent {
    sequence: Option<PlayerEventSequence>,
    authoritative_reacquisition: bool,
    kind: GuiAttachedOrderedPlayerEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::runtime_owner) struct GuiMediaBoundary {
    previous_media_generation: Option<u64>,
    unsequenced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuiAcceptedMediaObservation {
    previous_media_generation: Option<u64>,
    generation_advanced: bool,
}

impl GuiAttachedOrderedPlayerEvent {
    fn observed_at(&self) -> Option<PlayerObservationTimestamp> {
        match self {
            Self::CommandProgress(progress) => progress.observed_at,
            Self::LocalFile(observation) => observation.observed_at,
            Self::MediaLoad(observation) => observation.observed_at,
            Self::Transport(update) => update.observed_at,
        }
    }

    fn same_instant_rank(&self) -> u8 {
        match self {
            Self::CommandProgress(progress) if !progress.is_terminal() => 0,
            Self::LocalFile(_) => 1,
            Self::Transport(_) => 2,
            Self::MediaLoad(_) => 3,
            Self::CommandProgress(_) => 4,
        }
    }
}

fn compare_attached_ordered_player_events(
    left: &GuiAttachedOrderedPlayerEvent,
    right: &GuiAttachedOrderedPlayerEvent,
) -> std::cmp::Ordering {
    let key = |event: &GuiAttachedOrderedPlayerEvent| {
        (
            event
                .observed_at()
                .map(|timestamp| timestamp.elapsed_since_adapter_start()),
            event.same_instant_rank(),
        )
    };
    key(left).cmp(&key(right))
}

#[cfg(test)]
mod ordered_event_tests {
    use super::*;

    #[test]
    fn delivery_reference_does_not_reorder_same_instant_media_boundary() {
        let generation = PlayerMediaGeneration::new(2);
        let observed_at = Duration::from_secs(5);
        let local_file = GuiAttachedOrderedPlayerEvent::LocalFile(PlayerLocalFileObservation::new(
            LocalFileUpdate::new("new.mkv"),
            Some(generation),
            Some(PlayerObservationTimestamp::from_adapter_observation(
                observed_at,
                Duration::from_secs(10),
            )),
        ));
        let transport =
            GuiAttachedOrderedPlayerEvent::Transport(PlayerTransportTelemetryUpdate::new(
                generation,
                PlayerObservationTimestamp::from_adapter_observation(
                    observed_at,
                    Duration::from_secs(9),
                ),
            ));

        assert_eq!(
            compare_attached_ordered_player_events(&local_file, &transport),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_attached_ordered_player_events(&transport, &local_file),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn legacy_event_comparator_is_antisymmetric_and_transitive() {
        let timestamp = |seconds| {
            Some(PlayerObservationTimestamp::from_adapter_start(
                Duration::from_secs(seconds),
            ))
        };
        let generation = Some(PlayerMediaGeneration::new(7));
        let events = vec![
            GuiAttachedOrderedPlayerEvent::CommandProgress(PlayerCommandProgress::accepted(
                PlayerCommandId::new(1),
                generation,
                None,
            )),
            GuiAttachedOrderedPlayerEvent::CommandProgress(PlayerCommandProgress::finished(
                PlayerCommandId::new(1),
                generation,
                timestamp(3),
                None,
                sorotte_player_api::PlayerCommandResult::Completed,
            )),
            GuiAttachedOrderedPlayerEvent::LocalFile(PlayerLocalFileObservation::new(
                LocalFileUpdate::new("ordered.mkv"),
                generation,
                timestamp(1),
            )),
            GuiAttachedOrderedPlayerEvent::MediaLoad(PlayerMediaLoadObservation::new(
                sorotte_player_api::PlayerMediaLoadOutcome::success("ordered.mkv", None),
                generation,
                timestamp(2),
            )),
            GuiAttachedOrderedPlayerEvent::Transport(PlayerTransportTelemetryUpdate::new(
                PlayerMediaGeneration::new(7),
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
            )),
        ];

        for left in &events {
            for right in &events {
                assert_eq!(
                    compare_attached_ordered_player_events(left, right),
                    compare_attached_ordered_player_events(right, left).reverse()
                );
            }
        }
        for left in &events {
            for middle in &events {
                for right in &events {
                    let left_before_middle = compare_attached_ordered_player_events(left, middle)
                        != std::cmp::Ordering::Greater;
                    let middle_before_right = compare_attached_ordered_player_events(middle, right)
                        != std::cmp::Ordering::Greater;
                    if left_before_middle && middle_before_right {
                        assert_ne!(
                            compare_attached_ordered_player_events(left, right),
                            std::cmp::Ordering::Greater
                        );
                    }
                }
            }
        }
    }
}

impl GuiAttachedNativeSeekTracker {
    fn disarm_untrusted_position_evidence(&mut self) {
        self.position_anchor = None;
        self.interval_disarmed = true;
        self.seeking_since_anchor = false;
    }

    fn observe(
        &mut self,
        mut update: PlayerTransportTelemetryUpdate,
        ordered_sequence_is_authoritative: bool,
    ) -> GuiAttachedTransportObservationDisposition {
        let Some(media_generation) = update.media_generation.map(|generation| generation.get())
        else {
            if update.position_seconds.is_some() {
                self.disarm_untrusted_position_evidence();
            }
            return GuiAttachedTransportObservationDisposition::Rejected;
        };
        let same_media_generation = self.media_generation == Some(media_generation);
        if self
            .media_generation
            .is_some_and(|current_generation| media_generation < current_generation)
        {
            return GuiAttachedTransportObservationDisposition::Rejected;
        }
        let observed_at_seconds = update
            .observed_at
            .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
        let timestamp_regressed = same_media_generation
            && observed_at_seconds.is_some_and(|observed| {
                !observed.is_finite()
                    || self
                        .last_observed_at_seconds
                        .is_some_and(|latest| observed < latest)
            });
        if timestamp_regressed && !ordered_sequence_is_authoritative {
            if update.position_seconds.is_some() {
                self.disarm_untrusted_position_evidence();
            }
            return GuiAttachedTransportObservationDisposition::Rejected;
        }
        if timestamp_regressed && update.position_seconds.is_some() {
            self.disarm_untrusted_position_evidence();
            update.position_seconds = None;
        }
        if !same_media_generation {
            *self = Self {
                media_generation: Some(media_generation),
                ..Self::default()
            };
        }
        if let Some(observed_at_seconds) = observed_at_seconds.filter(|seconds| seconds.is_finite())
        {
            self.last_observed_at_seconds = Some(
                self.last_observed_at_seconds
                    .map_or(observed_at_seconds, |latest| {
                        latest.max(observed_at_seconds)
                    }),
            );
        }

        let observation_is_fresh = update.observed_at.is_some_and(|timestamp| {
            let observed = timestamp.elapsed_since_adapter_start().as_secs_f64();
            let delivered = timestamp
                .delivery_reference_since_adapter_start()
                .as_secs_f64();
            let age = delivered - observed;
            observed.is_finite()
                && delivered.is_finite()
                && (0.0..=ATTACHED_NATIVE_SEEK_MAX_OBSERVATION_AGE_SECONDS).contains(&age)
        });
        let position_is_fresh =
            update.position_seconds.is_some_and(f64::is_finite) && observation_is_fresh;
        if update.position_seconds.is_some() && !position_is_fresh {
            self.disarm_untrusted_position_evidence();
            update.position_seconds = None;
        }
        if update.playback_rate.is_some() && !observation_is_fresh {
            update.playback_rate = None;
        }

        let previously_seeking = self.seeking == Some(true)
            || self.phase == Some(PlayerTransportPhase::Seeking)
            || self.seeking_since_anchor;
        let state_transition = update.phase.is_some_and(|phase| self.phase != Some(phase))
            || update
                .playback_rate
                .is_some_and(|rate| self.playback_rate != Some(rate))
            || update
                .logical_pause
                .is_some_and(|paused| self.logical_pause != Some(paused))
            || update
                .paused_for_cache
                .is_some_and(|paused| self.paused_for_cache != Some(paused))
            || update
                .core_idle
                .is_some_and(|core_idle| self.core_idle != Some(core_idle));

        if let Some(phase) = update.phase {
            self.phase = Some(phase);
        }
        if let Some(playback_rate) = update.playback_rate {
            self.playback_rate =
                (playback_rate.is_finite() && playback_rate > 0.0).then_some(playback_rate);
        }
        if let Some(logical_pause) = update.logical_pause {
            self.logical_pause = Some(logical_pause);
        }
        if let Some(paused_for_cache) = update.paused_for_cache {
            self.paused_for_cache = Some(paused_for_cache);
        }
        if let Some(seeking) = update.seeking {
            self.seeking = Some(seeking);
        }
        if let Some(core_idle) = update.core_idle {
            self.core_idle = Some(core_idle);
        }

        if self.phase == Some(PlayerTransportPhase::Loading) {
            self.disarm_untrusted_position_evidence();
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: None,
            };
        }

        let currently_seeking =
            self.seeking == Some(true) || self.phase == Some(PlayerTransportPhase::Seeking);
        if currently_seeking {
            self.seeking_since_anchor = true;
            if let Some(anchor) = self.position_anchor.as_mut()
                && let Some(observed_at_seconds) = observed_at_seconds
            {
                anchor.observed_at_seconds = anchor.observed_at_seconds.max(observed_at_seconds);
            }
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: None,
            };
        }
        if state_transition && !previously_seeking {
            self.interval_disarmed = true;
        }

        let Some(position_seconds) = update.position_seconds else {
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: None,
            };
        };
        let observed_at_seconds = observed_at_seconds
            .expect("fresh position evidence always has a finite observation timestamp");
        let Some((phase, playback_rate, logical_pause, paused_for_cache, false, core_idle)) = self
            .phase
            .zip(self.playback_rate)
            .zip(self.logical_pause)
            .zip(self.paused_for_cache)
            .zip(self.seeking)
            .zip(self.core_idle)
            .map(
                |(
                    ((((phase, playback_rate), logical_pause), paused_for_cache), seeking),
                    core_idle,
                )| {
                    (
                        phase,
                        playback_rate,
                        logical_pause,
                        paused_for_cache,
                        seeking,
                        core_idle,
                    )
                },
            )
        else {
            self.position_anchor = None;
            self.interval_disarmed = true;
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: Some(false),
            };
        };
        let current = GuiAttachedPlayerPositionObservation {
            media_generation,
            observed_at_seconds,
            position_seconds,
            phase,
            playback_rate,
            logical_pause,
            paused_for_cache,
            core_idle,
        };
        if !current.is_stable() {
            self.position_anchor = Some(current);
            self.interval_disarmed = true;
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: Some(false),
            };
        }
        if self.interval_disarmed && !self.seeking_since_anchor {
            self.position_anchor = Some(current);
            self.interval_disarmed = false;
            return GuiAttachedTransportObservationDisposition::Accepted {
                update: Box::new(update),
                native_seek_classification: Some(false),
            };
        }

        let unexpected_position_jump = self.position_anchor.is_some_and(|previous| {
            previous.media_generation == current.media_generation
                && previous.is_stable()
                && current.observed_at_seconds >= previous.observed_at_seconds
                && (self.seeking_since_anchor || previous.same_motion_regime(current))
                && {
                    let elapsed_seconds =
                        current.observed_at_seconds - previous.observed_at_seconds;
                    let expected_advance = if previous.logical_pause {
                        0.0
                    } else {
                        elapsed_seconds * previous.playback_rate
                    };
                    let actual_advance = current.position_seconds - previous.position_seconds;
                    actual_advance < expected_advance - ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS
                        || actual_advance
                            > expected_advance + ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS
                }
        });
        self.position_anchor = Some(current);
        self.interval_disarmed = false;
        self.seeking_since_anchor = false;
        GuiAttachedTransportObservationDisposition::Accepted {
            update: Box::new(update),
            native_seek_classification: Some(unexpected_position_jump),
        }
    }

    fn reanchor_after_owned_seek(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
        position_seconds: Option<f64>,
    ) -> bool {
        let Some(media_generation) = media_generation.map(PlayerMediaGeneration::get) else {
            return false;
        };
        if self.media_generation != Some(media_generation) {
            return false;
        }
        let Some(observed_at_seconds) = observed_at
            .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64())
            .filter(|seconds| seconds.is_finite())
        else {
            return false;
        };
        if self
            .last_observed_at_seconds
            .is_some_and(|latest| observed_at_seconds < latest)
        {
            return false;
        }
        let Some(position_seconds) = position_seconds.filter(|position| position.is_finite())
        else {
            return false;
        };
        let Some((phase, playback_rate, logical_pause, paused_for_cache, false, core_idle)) = self
            .phase
            .zip(self.playback_rate)
            .zip(self.logical_pause)
            .zip(self.paused_for_cache)
            .zip(self.seeking)
            .zip(self.core_idle)
            .map(
                |(
                    ((((phase, playback_rate), logical_pause), paused_for_cache), seeking),
                    core_idle,
                )| {
                    (
                        phase,
                        playback_rate,
                        logical_pause,
                        paused_for_cache,
                        seeking,
                        core_idle,
                    )
                },
            )
        else {
            return false;
        };
        let observation = GuiAttachedPlayerPositionObservation {
            media_generation,
            observed_at_seconds,
            position_seconds,
            phase,
            playback_rate,
            logical_pause,
            paused_for_cache,
            core_idle,
        };
        if !observation.is_stable() {
            return false;
        }
        self.last_observed_at_seconds = Some(observed_at_seconds);
        self.position_anchor = Some(observation);
        self.interval_disarmed = false;
        self.seeking_since_anchor = false;
        true
    }
}

impl GuiAttachedPlayerPositionObservation {
    fn is_stable(self) -> bool {
        !self.paused_for_cache
            && self.playback_rate.is_finite()
            && self.playback_rate > 0.0
            && matches!(
                (self.phase, self.logical_pause, self.core_idle),
                (PlayerTransportPhase::Playing, false, false)
                    | (PlayerTransportPhase::ReadyPaused, true, _)
            )
    }

    fn same_motion_regime(self, other: Self) -> bool {
        self.phase == other.phase
            && self.playback_rate == other.playback_rate
            && self.logical_pause == other.logical_pause
            && self.paused_for_cache == other.paused_for_cache
            && self.core_idle == other.core_idle
    }
}

#[derive(Debug, Clone, PartialEq)]
enum GuiOrderedPlayerDelivery {
    Event(SequencedPlayerEvent),
    SemanticOutcome(SequencedPlayerSemanticOutcome),
}

impl GuiOrderedPlayerDelivery {
    fn order(&self) -> PlayerEventOrder {
        match self {
            Self::Event(event) => event.order,
            Self::SemanticOutcome(outcome) => outcome.order,
        }
    }
}

fn ordered_batch_error(message: impl Into<String>) -> sorotte_player_api::PlayerError {
    sorotte_player_api::PlayerError::OperationFailed(format!(
        "invalid ordered player event batch: {}",
        message.into()
    ))
}

fn snapshot_known_copy<T: Copy>(field: &SnapshotField<T>) -> Option<T> {
    match field {
        SnapshotField::Known(value) => Some(*value),
        SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
    }
}

fn snapshot_known_clone<T: Clone>(field: &SnapshotField<T>) -> Option<T> {
    match field {
        SnapshotField::Known(value) => Some(value.clone()),
        SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
    }
}

impl GuiOrderedPlayerEventConsumer {
    fn reset_for_epoch(&mut self, attachment_epoch: PlayerAttachmentEpoch) {
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

    fn begin_batch(
        &mut self,
        batch: &PlayerEventBatch,
    ) -> Result<(), sorotte_player_api::PlayerError> {
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
        if let Some(snapshot) = batch.authoritative_snapshot.as_ref()
            && let SnapshotField::Known(active) = snapshot.active_load
            && (snapshot_known_copy(&snapshot.transport.load_attempt_id)
                .is_some_and(|attempt_id| attempt_id != active.attempt_id)
                || snapshot_known_copy(&snapshot.transport.media_generation)
                    .is_some_and(|generation| generation != active.media_generation)
                || active
                    .playlist_entry_id
                    .zip(snapshot_known_copy(&snapshot.current_playlist_entry_id))
                    .is_some_and(|(active, current)| active != current))
        {
            return Err(ordered_batch_error(
                "authoritative snapshot transport disagrees with its active load",
            ));
        }
        for outcome in &batch.semantic_outcomes {
            let payload_epoch = match &outcome.outcome {
                PlayerSemanticOutcome::Command(command) => command.attachment_epoch,
                PlayerSemanticOutcome::LoadAttempt(load) => load.attachment_epoch,
            };
            if payload_epoch != batch.attachment_epoch {
                return Err(ordered_batch_error(
                    "semantic outcome payload belongs to another attachment",
                ));
            }
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

        let mut previous_sequence = None;
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
            if previous_sequence == Some(order.sequence) {
                return Err(ordered_batch_error("duplicate delivery order"));
            }
            previous_sequence = Some(order.sequence);
        }
        Ok(())
    }

    fn validate_sequence_continuity(
        &self,
        batch: &PlayerEventBatch,
    ) -> Result<(), sorotte_player_api::PlayerError> {
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
            if order.sequence <= cursor {
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

    fn merged_deliveries(batch: &PlayerEventBatch) -> Vec<GuiOrderedPlayerDelivery> {
        let mut deliveries = batch
            .events
            .iter()
            .cloned()
            .map(GuiOrderedPlayerDelivery::Event)
            .chain(
                batch
                    .semantic_outcomes
                    .iter()
                    .cloned()
                    .map(GuiOrderedPlayerDelivery::SemanticOutcome),
            )
            .collect::<Vec<_>>();
        deliveries.sort_by_key(GuiOrderedPlayerDelivery::order);
        deliveries
    }

    fn should_rebase_snapshot(&self, boundary: PlayerSequenceBoundary) -> bool {
        self.last_snapshot_boundary
            .is_none_or(|current| boundary.through_sequence > current.through_sequence)
    }

    fn rebase_snapshot(&mut self, snapshot: &PlayerAuthoritativeSnapshot) {
        self.transport.rebase(snapshot.transport.clone());
        self.attempts.clear();
        self.transport_owner_attempt = None;
        if let SnapshotField::Known(active) = snapshot.active_load {
            self.install_active_load(active);
        }
        self.last_snapshot_boundary = Some(snapshot.sequence_boundary);
        self.last_sequence = self
            .last_sequence
            .max(snapshot.sequence_boundary.through_sequence);
    }

    fn install_active_load(&mut self, active: PlayerActiveLoadSnapshot) {
        self.install_attempt(
            active.attempt_id,
            GuiOrderedLoadInstall {
                media_generation: active.media_generation,
                command_id: active.command_id,
                playlist_entry_id: active.playlist_entry_id,
                owns_transport: true,
                semantic_load_result: active.semantic_load_result,
                logical_ownership_revoked: active.logical_ownership_revoked,
            },
        );
    }

    fn install_attempt(&mut self, attempt_id: LoadAttemptId, install: GuiOrderedLoadInstall) {
        let GuiOrderedLoadInstall {
            media_generation,
            mut command_id,
            mut playlist_entry_id,
            owns_transport,
            semantic_load_result,
            logical_ownership_revoked,
        } = install;
        let existing = self.attempts.get(&attempt_id).copied();
        if let Some(existing) = existing {
            command_id = command_id.or(existing.command_id);
            playlist_entry_id = playlist_entry_id.or(existing.playlist_entry_id);
        }
        let physical_terminal = existing.is_some_and(|binding| binding.physical_terminal);
        let owns_transport = owns_transport && !physical_terminal;
        self.attempts.insert(
            attempt_id,
            GuiOrderedLoadBinding {
                media_generation,
                command_id,
                playlist_entry_id,
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

    fn ensure_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) {
        if !self.attempts.contains_key(&attempt_id) {
            self.install_attempt(
                attempt_id,
                GuiOrderedLoadInstall {
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

    fn mark_semantic_load_result(
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

    fn revoke_logical_ownership(
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

    fn mark_indeterminate(
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

    fn validate_attempt_binding(
        &self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
        playlist_entry_id: Option<i64>,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        let Some(binding) = self.attempts.get(&attempt_id) else {
            return Ok(());
        };
        if binding.media_generation != media_generation
            || command_id
                .zip(binding.command_id)
                .is_some_and(|(incoming, current)| incoming != current)
            || playlist_entry_id
                .zip(binding.playlist_entry_id)
                .is_some_and(|(incoming, current)| incoming != current)
            || binding.physical_terminal
        {
            return Err(ordered_batch_error(
                "load attempt was rebound to incompatible ownership",
            ));
        }
        Ok(())
    }

    fn terminate_attempt(
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

    fn attempt_is_owned(
        &self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) -> bool {
        self.transport_owner_attempt == Some(attempt_id)
            && self.attempts.get(&attempt_id).is_some_and(|binding| {
                binding.owns_transport
                    && !binding.physical_terminal
                    && binding.media_generation == media_generation
            })
    }

    fn outcome_matches_attempt(
        &self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) -> bool {
        self.attempts.get(&attempt_id).is_some_and(|binding| {
            binding.media_generation == media_generation
                && command_id.is_none_or(|command_id| binding.command_id == Some(command_id))
        })
    }

    fn apply_delta_if_owned(
        &mut self,
        delta: PlayerTransportDelta,
    ) -> Option<PlayerTransportDelta> {
        let mut candidate = self.transport.clone();
        candidate.apply_delta(delta.clone());
        let attempt_id = snapshot_known_copy(&candidate.load_attempt_id)?;
        let media_generation = snapshot_known_copy(&candidate.media_generation)?;
        if !self.attempt_is_owned(attempt_id, media_generation) {
            return None;
        }
        self.transport = candidate;
        Some(delta)
    }

    fn event_is_covered_by_snapshot(&self, order: PlayerEventOrder) -> bool {
        self.last_snapshot_boundary.is_some_and(|boundary| {
            boundary.attachment_epoch == order.attachment_epoch
                && order.sequence <= boundary.through_sequence
        })
    }

    fn require_next_order(
        &self,
        order: PlayerEventOrder,
    ) -> Result<(), sorotte_player_api::PlayerError> {
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

    fn record_order(&mut self, order: PlayerEventOrder) {
        self.last_sequence = self.last_sequence.max(order.sequence);
    }

    fn semantic_outcome_was_applied(&self, order: PlayerEventOrder) -> bool {
        order.sequence <= self.acknowledged_semantic_sequence
            || self.applied_semantic_outcomes.contains(&order)
    }

    fn record_semantic_outcome(&mut self, order: PlayerEventOrder) {
        self.applied_semantic_outcomes.insert(order);
        self.record_order(order);
    }

    fn compact_acknowledged_delivery(
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
}

impl GuiPersistedConfigRuntimeOwner {
    fn current_attached_system_seek_room_name(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|session| session.current_room_name())
            .map(str::to_owned)
    }

    pub(in crate::app::runtime_owner) fn prune_attached_system_seek_ownership(
        &mut self,
        now: Instant,
    ) {
        let player_attachment_epoch = self.player_attachment_epoch;
        let session_generation = self.session_generation;
        let room_name = self.current_attached_system_seek_room_name();
        self.attached_system_seek_ownership.retain(|ownership| {
            ownership.player_attachment_epoch == player_attachment_epoch
                && ownership.session_generation == session_generation
                && ownership.room_name == room_name
                && ownership.retire_after > now
        });
        if self
            .attached_system_seek_fail_closed
            .as_ref()
            .is_some_and(|guard| {
                guard.player_attachment_epoch != player_attachment_epoch
                    || guard.session_generation != session_generation
                    || guard.room_name != room_name
                    || guard.retire_after <= now
            })
        {
            self.attached_system_seek_fail_closed = None;
        }
    }

    pub(in crate::app::runtime_owner) fn extend_attached_system_seek_ownership_after_keep_waiting(
        &mut self,
        now: Instant,
    ) {
        self.prune_attached_system_seek_ownership(now);
        let Some(preparation) = self
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .and_then(|snapshot| snapshot.seek_preparation)
            .filter(|preparation| {
                preparation.terminal_outcome.is_none() && preparation.can_keep_waiting
            })
        else {
            return;
        };
        let retire_after = now + ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIFETIME;
        for ownership in &mut self.attached_system_seek_ownership {
            let target_matches = (ownership.requested_target_position_seconds
                - preparation.requested_target_seconds)
                .abs()
                <= ownership.tolerance_seconds
                || (ownership.target_position_seconds - preparation.frozen_target_seconds).abs()
                    <= ownership.tolerance_seconds;
            if ownership.logical_media_generation == Some(preparation.media_generation)
                && target_matches
            {
                ownership.retire_after = ownership.retire_after.max(retire_after);
            }
        }
        if let Some(guard) = self.attached_system_seek_fail_closed.as_mut()
            && guard.logical_media_generation == Some(preparation.media_generation)
        {
            guard.retire_after = guard.retire_after.max(retire_after);
        }
    }

    fn note_attached_system_seek_dispatched(
        &mut self,
        source: GuiAttachedSystemSeekSource,
        adapter_player_command_id: Option<PlayerCommandId>,
        requested_target_position_seconds: f64,
        player_target_position_seconds: f64,
    ) {
        if !requested_target_position_seconds.is_finite()
            || !player_target_position_seconds.is_finite()
        {
            return;
        }
        let dispatch_offset_seconds = self.user_offset_seconds;
        let target_position_seconds = player_target_position_seconds - dispatch_offset_seconds;
        let now = Instant::now();
        self.prune_attached_system_seek_ownership(now);
        for ownership in &mut self.attached_system_seek_ownership {
            if ownership.state == GuiAttachedSystemSeekOwnershipState::Active {
                ownership.state = GuiAttachedSystemSeekOwnershipState::SupersededMayArrive;
            }
        }
        let retire_after = now + ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIFETIME;
        let media_generation = self.attached_native_seek_tracker.media_generation;
        let logical_media_generation = media_generation.and_then(|generation| {
            self.session.as_ref().and_then(|session| {
                session.logical_generation_for_adapter_generation(PlayerMediaGeneration::new(
                    generation,
                ))
            })
        });
        if self.attached_system_seek_ownership.len() >= ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIMIT {
            let guard = GuiAttachedSystemSeekFailClosedGuard {
                player_attachment_epoch: self.player_attachment_epoch,
                session_generation: self.session_generation,
                room_name: self.current_attached_system_seek_room_name(),
                media_generation,
                logical_media_generation,
                retire_after,
            };
            match self.attached_system_seek_fail_closed.as_mut() {
                Some(existing)
                    if existing.media_generation == guard.media_generation
                        && existing.logical_media_generation == guard.logical_media_generation =>
                {
                    existing.retire_after = existing.retire_after.max(retire_after);
                }
                Some(existing) => *existing = guard,
                None => self.attached_system_seek_fail_closed = Some(guard),
            }
            return;
        }
        self.attached_system_seek_ownership
            .push_back(GuiAttachedSystemSeekOwnership {
                source,
                adapter_player_command_id,
                player_attachment_epoch: self.player_attachment_epoch,
                session_generation: self.session_generation,
                room_name: self.current_attached_system_seek_room_name(),
                media_generation,
                logical_media_generation,
                issued_after_observed_at_seconds: self
                    .attached_native_seek_tracker
                    .last_observed_at_seconds,
                requested_target_position_seconds,
                player_target_position_seconds,
                dispatch_offset_seconds,
                target_position_seconds,
                tolerance_seconds: MPV_SEEK_COMPLETION_TOLERANCE_SECONDS,
                retire_after,
                state: GuiAttachedSystemSeekOwnershipState::Active,
            });
    }

    pub(in crate::app::runtime_owner) fn note_attached_coordinator_seek_dispatched(
        &mut self,
        coordinator_command_id: CoordinatorCommandId,
        adapter_player_command_id: Option<PlayerCommandId>,
        requested_target_position_seconds: f64,
        player_target_position_seconds: f64,
    ) {
        self.note_attached_system_seek_dispatched(
            GuiAttachedSystemSeekSource::Coordinator(coordinator_command_id),
            adapter_player_command_id,
            requested_target_position_seconds,
            player_target_position_seconds,
        );
    }

    pub(in crate::app::runtime_owner) fn note_attached_runtime_position_dispatched(
        &mut self,
        adapter_player_command_id: Option<PlayerCommandId>,
        requested_target_position_seconds: f64,
        player_target_position_seconds: f64,
    ) {
        self.note_attached_system_seek_dispatched(
            GuiAttachedSystemSeekSource::RuntimeAction,
            adapter_player_command_id,
            requested_target_position_seconds,
            player_target_position_seconds,
        );
    }

    pub(in crate::app::runtime_owner) fn reconcile_attached_system_seek_command_progress(
        &mut self,
        progress: PlayerCommandProgress,
    ) {
        let Some(index) = self
            .attached_system_seek_ownership
            .iter()
            .position(|ownership| ownership.adapter_player_command_id == Some(progress.command_id))
        else {
            if matches!(
                progress.state,
                PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                    PlayerCommandFailureKind::TimedOut | PlayerCommandFailureKind::Unknown
                ))
            ) && let Some(guard) = self.attached_system_seek_fail_closed.as_mut()
            {
                guard.retire_after = guard
                    .retire_after
                    .max(Instant::now() + ATTACHED_SYSTEM_SEEK_TIMEOUT_EXTENSION);
            }
            return;
        };
        if let Some(ownership) = self.attached_system_seek_ownership.get_mut(index) {
            if ownership.media_generation.is_none() {
                ownership.media_generation =
                    progress.media_generation.map(PlayerMediaGeneration::get);
            }
            if ownership.issued_after_observed_at_seconds.is_none() {
                ownership.issued_after_observed_at_seconds = progress
                    .observed_at
                    .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64())
                    .filter(|seconds| seconds.is_finite());
            }
        }
        match progress.state {
            PlayerCommandProgressState::Accepted => {}
            PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                let ownership = &self.attached_system_seek_ownership[index];
                let player_position_seconds = progress.observed_position_seconds;
                let position_matches = player_position_seconds.is_some_and(|position| {
                    (ownership.player_target_position_seconds - position).abs()
                        <= ownership.tolerance_seconds
                });
                let observed_position_seconds =
                    player_position_seconds.map(|position| position - self.user_offset_seconds);
                if position_matches
                    && self.attached_native_seek_tracker.reanchor_after_owned_seek(
                        progress.media_generation,
                        progress.observed_at,
                        observed_position_seconds,
                    )
                {
                    self.attached_system_seek_ownership.remove(index);
                } else if let Some(ownership) = self.attached_system_seek_ownership.get_mut(index) {
                    ownership.state =
                        GuiAttachedSystemSeekOwnershipState::CompletedAwaitingStablePosition;
                }
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Superseded) => {
                if let Some(ownership) = self.attached_system_seek_ownership.get_mut(index) {
                    ownership.state = GuiAttachedSystemSeekOwnershipState::SupersededMayArrive;
                }
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                PlayerCommandFailureKind::TimedOut | PlayerCommandFailureKind::Unknown,
            )) => {
                if let Some(ownership) = self.attached_system_seek_ownership.get_mut(index) {
                    ownership.state = GuiAttachedSystemSeekOwnershipState::MayStillArrive;
                    ownership.retire_after = ownership
                        .retire_after
                        .max(Instant::now() + ATTACHED_SYSTEM_SEEK_TIMEOUT_EXTENSION);
                }
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                PlayerCommandFailureKind::MediaEnded
                | PlayerCommandFailureKind::TransportDisconnected,
            )) => {
                self.attached_system_seek_ownership.remove(index);
            }
        }
    }

    fn consume_matching_attached_system_seek(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
        player_position_seconds: f64,
    ) -> bool {
        self.prune_attached_system_seek_ownership(Instant::now());
        let observed_generation = media_generation.map(PlayerMediaGeneration::get);
        let observed_at_seconds =
            observed_at.map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
        let matching_index = self
            .attached_system_seek_ownership
            .iter()
            .position(|ownership| {
                ownership
                    .media_generation
                    .zip(observed_generation)
                    .is_none_or(|(expected, observed)| expected == observed)
                    && ownership
                        .issued_after_observed_at_seconds
                        .zip(observed_at_seconds)
                        .is_none_or(|(issued_after, observed)| observed > issued_after)
                    && (ownership.player_target_position_seconds - player_position_seconds).abs()
                        <= ownership.tolerance_seconds
            });
        if let Some(index) = matching_index {
            self.attached_system_seek_ownership.remove(index);
            true
        } else {
            false
        }
    }

    fn attached_system_seek_classification_is_fail_closed(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
    ) -> bool {
        self.prune_attached_system_seek_ownership(Instant::now());
        let observed_generation = media_generation.map(PlayerMediaGeneration::get);
        self.attached_system_seek_fail_closed
            .as_ref()
            .is_some_and(|guard| {
                guard
                    .media_generation
                    .zip(observed_generation)
                    .is_none_or(|(expected, observed)| expected == observed)
            })
    }

    fn sync_attached_player_position_observation(
        &mut self,
        position_seconds: f64,
        unexpected_position_jump: bool,
    ) -> bool {
        let position_already_owned_by_session = self.session.as_ref().is_some_and(|session| {
            session
                .local_position_seconds()
                .is_some_and(|known_position| {
                    (known_position - position_seconds).abs()
                        <= MPV_SEEK_COMPLETION_TOLERANCE_SECONDS
                })
        });

        let mut publish_succeeded = true;
        if unexpected_position_jump && !position_already_owned_by_session {
            let _ = self.interrupt_attached_playback_recovery_impl("native player seek");
            publish_succeeded = match self
                .session
                .as_mut()
                .map(|session| session.record_manual_seek_to_position(position_seconds))
            {
                Some(Ok(true)) | None => true,
                Some(Ok(false)) => false,
                Some(Err(error)) => {
                    eprintln!(
                        "warning: failed to publish native attached-player seek to the room: {error}"
                    );
                    false
                }
            };
        }

        if publish_succeeded
            && let Some(session) = self.session.as_mut()
            && let Err(error) = session.sync_local_playback_telemetry(
                // Pause edges have their own causal classifier. Mirroring the
                // just-observed pause value here would erase a native Play or
                // Pause edge before transport telemetry can classify it.
                None,
                Some(position_seconds),
            )
        {
            eprintln!(
                "warning: failed to ground the session position in attached-player telemetry: {error}"
            );
        }
        publish_succeeded
    }

    fn accept_attached_media_observation(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
        sequence: Option<PlayerEventSequence>,
    ) -> Option<GuiAcceptedMediaObservation> {
        let previous_media_generation = self
            .attached_media_observation_cursor
            .media_generation
            .max(self.attached_native_seek_tracker.media_generation);
        let Some(media_generation) = media_generation.map(PlayerMediaGeneration::get) else {
            return Some(GuiAcceptedMediaObservation {
                previous_media_generation,
                generation_advanced: false,
            });
        };
        if previous_media_generation.is_some_and(|current| media_generation < current) {
            return None;
        }
        let observed_at_seconds = observed_at
            .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64())
            .filter(|seconds| seconds.is_finite());
        let latest_observed_at_seconds = match (
            self.attached_media_observation_cursor
                .last_observed_at_seconds,
            self.attached_native_seek_tracker.last_observed_at_seconds,
        ) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        if previous_media_generation == Some(media_generation)
            && sequence.is_none()
            && observed_at_seconds.is_some_and(|observed| {
                latest_observed_at_seconds.is_some_and(|latest| observed < latest)
            })
        {
            return None;
        }
        let generation_advanced =
            previous_media_generation.is_none_or(|current| media_generation > current);
        if generation_advanced {
            let last_ordered_event_sequence = self
                .attached_media_observation_cursor
                .last_ordered_event_sequence;
            self.attached_media_observation_cursor =
                crate::app::runtime_owner::GuiAttachedMediaObservationCursor {
                    media_generation: Some(media_generation),
                    last_observed_at_seconds: observed_at_seconds,
                    last_ordered_event_sequence,
                };
        } else if let Some(observed_at_seconds) = observed_at_seconds {
            self.attached_media_observation_cursor
                .last_observed_at_seconds = Some(
                self.attached_media_observation_cursor
                    .last_observed_at_seconds
                    .map_or(observed_at_seconds, |latest| {
                        latest.max(observed_at_seconds)
                    }),
            );
        }
        if generation_advanced {
            self.attached_native_seek_tracker = GuiAttachedNativeSeekTracker {
                media_generation: Some(media_generation),
                last_observed_at_seconds: observed_at_seconds,
                ..GuiAttachedNativeSeekTracker::default()
            };
        }
        Some(GuiAcceptedMediaObservation {
            previous_media_generation,
            generation_advanced,
        })
    }

    fn reset_attached_media_boundary_state(&mut self) {
        self.pending_local_attached_pause_override = None;
        self.attached_system_seek_ownership.clear();
        self.attached_system_seek_fail_closed = None;
        self.attached_transport_telemetry_authority = Default::default();
    }

    fn rebase_attached_ordered_player_inference_for_reacquisition(&mut self) {
        let media_generation = self
            .attached_media_observation_cursor
            .media_generation
            .max(self.attached_native_seek_tracker.media_generation);
        let last_observed_at_seconds = match (
            self.attached_media_observation_cursor
                .last_observed_at_seconds,
            self.attached_native_seek_tracker.last_observed_at_seconds,
        ) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        let now = Instant::now();
        self.prune_attached_system_seek_ownership(now);
        let retire_after = now + ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIFETIME;
        for ownership in &mut self.attached_system_seek_ownership {
            ownership.state = GuiAttachedSystemSeekOwnershipState::MayStillArrive;
            ownership.retire_after = ownership.retire_after.max(retire_after);
        }
        if !self.attached_system_seek_ownership.is_empty() {
            let guard = GuiAttachedSystemSeekFailClosedGuard {
                player_attachment_epoch: self.player_attachment_epoch,
                session_generation: self.session_generation,
                room_name: self.current_attached_system_seek_room_name(),
                media_generation,
                logical_media_generation: self
                    .attached_system_seek_ownership
                    .iter()
                    .find_map(|ownership| ownership.logical_media_generation),
                retire_after,
            };
            match self.attached_system_seek_fail_closed.as_mut() {
                Some(existing)
                    if existing.media_generation == guard.media_generation
                        && existing.logical_media_generation == guard.logical_media_generation =>
                {
                    existing.retire_after = existing.retire_after.max(retire_after);
                }
                Some(existing) => *existing = guard,
                None => self.attached_system_seek_fail_closed = Some(guard),
            }
        }
        self.attached_transport_telemetry_authority = Default::default();
        self.attached_native_seek_tracker = GuiAttachedNativeSeekTracker {
            media_generation,
            last_observed_at_seconds,
            ..GuiAttachedNativeSeekTracker::default()
        };
    }

    pub(in crate::app::runtime_owner) fn process_attached_local_file_observation(
        &mut self,
        observation: PlayerLocalFileObservation,
        sequence: Option<PlayerEventSequence>,
        authoritative_reacquisition: bool,
    ) -> Option<GuiMediaBoundary> {
        let PlayerLocalFileObservation {
            mut update,
            media_generation,
            observed_at,
        } = observation;
        let accepted =
            self.accept_attached_media_observation(media_generation, observed_at, sequence)?;
        let mut logical_override_confirmed = None;
        if let Some((override_update, confirmed)) =
            self.logical_media_override_for_loaded_target(&update, media_generation)
        {
            update = override_update;
            logical_override_confirmed = Some(confirmed);
        }
        let file_changed =
            Self::local_file_update_replaces_current_file(self.player_local_file.as_ref(), &update);
        // Identity confirmation is valid during an authoritative replay even when no new media
        // episode exists. Keep it separate from the file-change side effects below so ordered
        // adapters can activate an untracked playlist attempt and Plex can retire a confirmed
        // logical placeholder without preparing the same media a second time.
        if authoritative_reacquisition {
            // `apply_ordered_snapshot` only supplies a local-file replay here
            // when the active-load snapshot explicitly proves
            // `physical_file_loaded`. Ordinary path observations do not carry
            // that authority.
            self.handle_authoritative_playlist_local_file_observation(&update);
        } else {
            self.handle_untracked_playlist_local_file_observation(&update);
        }
        let tracked_playlist_load_unconfirmed =
            self.tracked_playlist_resolution_load_matches_local_file(&update);
        let identity_remains_placeholder = tracked_playlist_load_unconfirmed
            || logical_override_confirmed.is_some_and(|confirmed| !confirmed);
        if authoritative_reacquisition && !file_changed && !accepted.generation_advanced {
            self.player_local_file = Some(update);
            self.player_local_file_placeholder = identity_remains_placeholder;
            return None;
        }
        if file_changed || accepted.generation_advanced {
            self.reset_attached_media_boundary_state();
        }
        if file_changed && media_generation.is_none() {
            let last_ordered_event_sequence = self
                .attached_media_observation_cursor
                .last_ordered_event_sequence;
            self.attached_media_observation_cursor =
                crate::app::runtime_owner::GuiAttachedMediaObservationCursor {
                    last_ordered_event_sequence,
                    ..Default::default()
                };
            self.attached_native_seek_tracker = GuiAttachedNativeSeekTracker::default();
        }
        if file_changed {
            let _ =
                self.interrupt_attached_playback_recovery_impl("observed media transport change");
            let logical_id = logical_media_id_for_local_file_update(&update);
            let kind = if update.path.as_deref().is_some_and(browser_is_url)
                || browser_is_url(&update.name)
            {
                MediaTransportKind::NetworkVod
            } else {
                MediaTransportKind::LocalFile
            };
            if let Some(session) = self.session.as_mut()
                && let Err(error) = session.prepare_attached_playback_media(
                    logical_id,
                    kind,
                    MediaLoadIntent::TransportRefresh,
                    system_time_seconds(),
                )
            {
                eprintln!(
                    "warning: failed to prepare attached-player logical media generation: {error}"
                );
            }
        }
        self.player_local_file = Some(update);
        self.player_local_file_placeholder = identity_remains_placeholder;
        if file_changed || accepted.generation_advanced || self.player_position_seconds.is_none() {
            self.player_position_seconds = Some(0.0);
        }
        (file_changed || accepted.generation_advanced).then_some(GuiMediaBoundary {
            previous_media_generation: accepted.previous_media_generation,
            unsequenced: media_generation.is_none(),
        })
    }

    fn process_attached_ordered_player_event(
        &mut self,
        event: GuiAttachedSequencedPlayerEvent,
        user_offset_seconds: f64,
    ) -> Option<GuiMediaBoundary> {
        let GuiAttachedSequencedPlayerEvent {
            sequence,
            authoritative_reacquisition,
            kind,
        } = event;
        if let Some(sequence) = sequence {
            self.attached_media_observation_cursor
                .last_ordered_event_sequence = Some(sequence);
        }
        match kind {
            GuiAttachedOrderedPlayerEvent::CommandProgress(progress) => {
                self.handle_playlist_resolution_command_progress(progress);
                self.reconcile_attached_system_seek_command_progress(progress);
                None
            }
            GuiAttachedOrderedPlayerEvent::LocalFile(observation) => self
                .process_attached_local_file_observation(
                    observation,
                    sequence,
                    authoritative_reacquisition,
                ),
            GuiAttachedOrderedPlayerEvent::MediaLoad(observation) => {
                let accepted = self.accept_attached_media_observation(
                    observation.media_generation,
                    observation.observed_at,
                    sequence,
                )?;
                if accepted.generation_advanced {
                    self.reset_attached_media_boundary_state();
                }
                self.handle_playlist_media_load_outcome(&observation.outcome);
                self.handle_player_media_load_outcome(observation.outcome);
                accepted.generation_advanced.then_some(GuiMediaBoundary {
                    previous_media_generation: accepted.previous_media_generation,
                    unsequenced: false,
                })
            }
            GuiAttachedOrderedPlayerEvent::Transport(update) => {
                let player_position_seconds = update.position_seconds;
                let Some(accepted) = self.accept_attached_media_observation(
                    update.media_generation,
                    update.observed_at,
                    sequence,
                ) else {
                    if player_position_seconds.is_some() {
                        self.attached_native_seek_tracker
                            .disarm_untrusted_position_evidence();
                    }
                    return None;
                };
                let established_generation_advanced =
                    accepted.generation_advanced && accepted.previous_media_generation.is_some();
                if established_generation_advanced {
                    self.reset_attached_media_boundary_state();
                    self.player_local_file = None;
                    self.player_local_file_placeholder = false;
                    self.player_position_seconds = None;
                    self.player_paused = None;
                    self.player_paused_for_cache = None;
                    self.player_cache_buffering_percent = None;
                } else if authoritative_reacquisition {
                    self.player_position_seconds = None;
                    self.player_paused = None;
                    self.player_paused_for_cache = None;
                    self.player_cache_buffering_percent = None;
                }
                let update = transport_update_on_room_timeline(update, user_offset_seconds);
                let previous_native_seek_tracker = self.attached_native_seek_tracker;
                let GuiAttachedTransportObservationDisposition::Accepted {
                    update,
                    native_seek_classification,
                } = self
                    .attached_native_seek_tracker
                    .observe(update, sequence.is_some())
                else {
                    return None;
                };
                let update = *update;
                self.reconcile_pending_logical_override_media_generation(update.media_generation);
                self.attached_transport_telemetry_authority.position |=
                    update.position_seconds.is_some();
                self.attached_transport_telemetry_authority.logical_pause |=
                    update.logical_pause.is_some();
                self.attached_transport_telemetry_authority.paused_for_cache |=
                    update.paused_for_cache.is_some();
                self.attached_transport_telemetry_authority
                    .cache_buffering_percent |= update.cache_buffering_percent.is_some();
                if let Some(paused_for_cache) = update.paused_for_cache {
                    self.player_paused_for_cache = Some(paused_for_cache);
                }
                if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                    self.player_cache_buffering_percent = Some(cache_buffering_percent);
                }
                if let Some(position_seconds) = update.position_seconds
                    && let Some(unexpected_position_jump) = native_seek_classification
                {
                    let system_seek_owned = player_position_seconds.is_some_and(|position| {
                        self.consume_matching_attached_system_seek(
                            update.media_generation,
                            update.observed_at,
                            position,
                        )
                    });
                    let fail_closed = unexpected_position_jump
                        && self.attached_system_seek_classification_is_fail_closed(
                            update.media_generation,
                        );
                    let position_accepted = self.sync_attached_player_position_observation(
                        position_seconds,
                        unexpected_position_jump && !system_seek_owned && !fail_closed,
                    );
                    if unexpected_position_jump && !position_accepted {
                        self.attached_native_seek_tracker.position_anchor =
                            previous_native_seek_tracker.position_anchor;
                        self.attached_native_seek_tracker.interval_disarmed =
                            previous_native_seek_tracker.interval_disarmed;
                        self.attached_native_seek_tracker.seeking_since_anchor =
                            previous_native_seek_tracker.seeking_since_anchor;
                    }
                    if position_accepted {
                        self.player_position_seconds = Some(position_seconds);
                    }
                }
                if let Some(logical_pause) = update.logical_pause
                    && self.player_paused_for_cache != Some(true)
                {
                    self.player_paused = Some(logical_pause);
                }
                let actions = self.session.as_mut().and_then(|session| {
                    let result = if authoritative_reacquisition {
                        session.rebase_attached_player_transport_telemetry(
                            update,
                            system_time_seconds(),
                        )
                    } else {
                        session.sync_attached_player_transport_telemetry(
                            update,
                            system_time_seconds(),
                        )
                    };
                    match result {
                        Ok(actions) => Some(actions),
                        Err(error) => {
                            eprintln!(
                                "warning: failed to feed attached-player transport telemetry to client-core coordinator: {error}"
                            );
                            None
                        }
                    }
                });
                if let Some(actions) = actions {
                    let _ = self.apply_attached_player_runtime_actions_impl(
                        actions,
                        "transport observation",
                    );
                }
                established_generation_advanced.then_some(GuiMediaBoundary {
                    previous_media_generation: accepted.previous_media_generation,
                    unsequenced: false,
                })
            }
        }
    }

    fn refresh_legacy_player_state_impl(&mut self) {
        self.prune_attached_system_seek_ownership(Instant::now());
        self.attached_transport_telemetry_authority = Default::default();
        let user_offset_seconds = self.user_offset_seconds;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let mut playback_updates = Vec::new();
        let mut transport_updates = VecDeque::new();
        let mut command_progress_updates = VecDeque::new();
        let mut media_load_observations = VecDeque::new();
        let mut local_file_observations = VecDeque::new();
        let mut ordered_events = Vec::new();
        let mut ordered_reacquisition_boundary = None;
        let mut unannounced_ordered_sequence_gap = false;
        if let Some(mut batch) = player.take_ordered_event_batch() {
            batch.ordered_events.sort_by_key(|event| event.sequence);
            let previous_sequence = self
                .attached_media_observation_cursor
                .last_ordered_event_sequence;
            let dropped_events_through = batch.dropped_events_through;
            let marker_precedes_consumed_state = dropped_events_through
                .zip(previous_sequence)
                .is_some_and(|(dropped, consumed)| dropped < consumed);
            let expected_predecessor = dropped_events_through.or(previous_sequence);
            let first_event_is_contiguous = expected_predecessor
                .zip(batch.ordered_events.first().map(|event| event.sequence))
                .is_none_or(|(previous, first)| {
                    previous
                        .get()
                        .checked_add(1)
                        .is_some_and(|expected| first.get() == expected)
                });
            let batch_is_internally_contiguous = batch.ordered_events.windows(2).all(|events| {
                events[0]
                    .sequence
                    .get()
                    .checked_add(1)
                    .is_some_and(|expected| events[1].sequence.get() == expected)
            });
            if marker_precedes_consumed_state
                || !first_event_is_contiguous
                || !batch_is_internally_contiguous
            {
                player.request_ordered_event_reacquisition();
                unannounced_ordered_sequence_gap = true;
            } else {
                ordered_reacquisition_boundary = dropped_events_through;
                ordered_events.extend(batch.ordered_events.into_iter().map(|event| {
                    let kind = match event.kind {
                        PlayerOrderedEventKind::CommandProgress(progress) => {
                            GuiAttachedOrderedPlayerEvent::CommandProgress(progress)
                        }
                        PlayerOrderedEventKind::LocalFile(observation) => {
                            GuiAttachedOrderedPlayerEvent::LocalFile(observation)
                        }
                        PlayerOrderedEventKind::MediaLoad(observation) => {
                            GuiAttachedOrderedPlayerEvent::MediaLoad(observation)
                        }
                        PlayerOrderedEventKind::Transport(update) => {
                            GuiAttachedOrderedPlayerEvent::Transport(update)
                        }
                    };
                    GuiAttachedSequencedPlayerEvent {
                        sequence: Some(event.sequence),
                        authoritative_reacquisition: dropped_events_through.is_some(),
                        kind,
                    }
                }));
                if dropped_events_through.is_none()
                    && let Some(update) = batch.legacy_playback_telemetry
                {
                    playback_updates.push(update);
                }
            }
        } else {
            while let Some(progress) = player.take_command_progress() {
                command_progress_updates.push_back(progress);
            }
            while let Some(update) = player.take_playback_telemetry_update() {
                playback_updates.push(update);
            }
            while let Some(update) = player.take_transport_telemetry_update() {
                transport_updates.push_back(update);
            }
            while let Some(observation) = player.take_media_load_observation() {
                media_load_observations.push_back(observation);
            }
            while let Some(observation) = player.take_local_file_observation() {
                local_file_observations.push_back(observation);
            }
        }
        let mut hook_health_transitions = Vec::new();
        let mut media_policy_outcomes = Vec::new();
        let mut network_options_snapshot = None;
        let mut mpv_connected = true;
        if let Some(player) = player.as_mpv_mut() {
            while let Some(transition) = player.take_network_options_hook_health_transition() {
                hook_health_transitions.push(transition);
            }
            while let Some(outcome) = player.take_network_media_policy_outcome() {
                media_policy_outcomes.push(outcome);
            }
            network_options_snapshot = Some(player.network_options_runtime_health_snapshot());
            mpv_connected = player.is_connected();
        }
        for transition in hook_health_transitions {
            match transition {
                MpvNetworkOptionsHookHealthTransition::Recovered => {
                    self.record_network_options_hook_recovered();
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) if mpv_connected => {
                    self.mark_network_options_hook_degraded(format!(
                        "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry or player restart: {error}"
                    ));
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while maintaining Sorotte's core streaming-settings hook: {error}"
                    ));
                    return;
                }
            }
        }
        for outcome in media_policy_outcomes {
            match outcome {
                MpvNetworkMediaPolicyOutcome::NoActiveMedia
                | MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged
                | MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                    self.record_network_media_transition_recovered();
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) if mpv_connected => {
                    self.mark_network_media_transition_apply_failed(format!(
                        "mpv switched to network media, but configured streaming settings could not be applied to the new file: {error}"
                    ));
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while applying configured streaming settings to newly active network media: {error}"
                    ));
                    return;
                }
            }
        }
        if let Some(snapshot) = network_options_snapshot
            && !self.reconcile_network_options_runtime_health_snapshot(snapshot, mpv_connected)
        {
            return;
        }
        if unannounced_ordered_sequence_gap {
            self.rebase_attached_ordered_player_inference_for_reacquisition();
        } else if let Some(dropped_events_through) = ordered_reacquisition_boundary {
            self.attached_media_observation_cursor
                .last_ordered_event_sequence = Some(dropped_events_through);
            self.rebase_attached_ordered_player_inference_for_reacquisition();
        }
        let now = Instant::now();
        if self
            .pending_attached_player_pause_command
            .is_some_and(|pending| pending.suppress_until <= now)
        {
            self.pending_attached_player_pause_command = None;
        }
        ordered_events.reserve(
            command_progress_updates.len()
                + local_file_observations.len()
                + media_load_observations.len()
                + transport_updates.len(),
        );
        while !command_progress_updates.is_empty()
            || !local_file_observations.is_empty()
            || !media_load_observations.is_empty()
            || !transport_updates.is_empty()
        {
            let mut candidates = Vec::with_capacity(4);
            if let Some(progress) = command_progress_updates.front() {
                candidates.push((
                    0_u8,
                    GuiAttachedOrderedPlayerEvent::CommandProgress(*progress),
                ));
            }
            if let Some(observation) = local_file_observations.front() {
                candidates.push((
                    1_u8,
                    GuiAttachedOrderedPlayerEvent::LocalFile(observation.clone()),
                ));
            }
            if let Some(observation) = media_load_observations.front() {
                candidates.push((
                    2_u8,
                    GuiAttachedOrderedPlayerEvent::MediaLoad(observation.clone()),
                ));
            }
            if let Some(update) = transport_updates.front() {
                candidates.push((
                    3_u8,
                    GuiAttachedOrderedPlayerEvent::Transport(update.clone()),
                ));
            }
            candidates.sort_by(|(left_source, left), (right_source, right)| {
                compare_attached_ordered_player_events(left, right)
                    .then_with(|| left_source.cmp(right_source))
            });
            let (source, _) = candidates
                .first()
                .expect("at least one ordered player event queue is non-empty");
            let event = match source {
                0 => GuiAttachedOrderedPlayerEvent::CommandProgress(
                    command_progress_updates
                        .pop_front()
                        .expect("command progress candidate"),
                ),
                1 => GuiAttachedOrderedPlayerEvent::LocalFile(
                    local_file_observations
                        .pop_front()
                        .expect("local file candidate"),
                ),
                2 => GuiAttachedOrderedPlayerEvent::MediaLoad(
                    media_load_observations
                        .pop_front()
                        .expect("media load candidate"),
                ),
                3 => GuiAttachedOrderedPlayerEvent::Transport(
                    transport_updates.pop_front().expect("transport candidate"),
                ),
                _ => unreachable!("known ordered player event source"),
            };
            ordered_events.push(GuiAttachedSequencedPlayerEvent {
                sequence: None,
                authoritative_reacquisition: false,
                kind: event,
            });
        }
        let mut unsequenced_media_boundary = None;
        let mut media_boundary_observed = false;
        for event in ordered_events {
            if let (
                Some(GuiMediaBoundary {
                    previous_media_generation,
                    ..
                }),
                GuiAttachedOrderedPlayerEvent::Transport(update),
            ) = (unsequenced_media_boundary, &event.kind)
            {
                let proves_new_media_generation =
                    update.media_generation.is_some_and(|generation| {
                        previous_media_generation
                            .is_some_and(|previous| generation.get() > previous)
                    });
                if !proves_new_media_generation {
                    continue;
                }
            }
            if let Some(boundary) =
                self.process_attached_ordered_player_event(event, user_offset_seconds)
            {
                media_boundary_observed = true;
                unsequenced_media_boundary = boundary.unsequenced.then_some(boundary);
            }
        }
        if media_boundary_observed {
            playback_updates.clear();
        }
        for update in playback_updates {
            let legacy_paused_for_cache =
                (!self.attached_transport_telemetry_authority.paused_for_cache)
                    .then_some(update.paused_for_cache)
                    .flatten();
            let legacy_cache_buffering_percent = (!self
                .attached_transport_telemetry_authority
                .cache_buffering_percent)
                .then_some(update.cache_buffering_percent)
                .flatten();
            if let Some(paused_for_cache) = legacy_paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(cache_buffering_percent) = legacy_cache_buffering_percent {
                self.player_cache_buffering_percent = Some(cache_buffering_percent);
            }
            if (legacy_paused_for_cache.is_some() || legacy_cache_buffering_percent.is_some())
                && let Some(session) = self.session.as_mut()
                && let Err(error) = session.sync_local_playback_cache_state(
                    legacy_paused_for_cache,
                    legacy_cache_buffering_percent,
                )
            {
                eprintln!(
                    "warning: failed to mirror attached-player cache buffering state into the session runtime: {error}"
                );
            }
            if !self.attached_transport_telemetry_authority.position
                && let Some(position_seconds) = update.position_seconds
            {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
            if !self.attached_transport_telemetry_authority.logical_pause
                && let Some(paused) = update.paused
                && self.player_paused_for_cache != Some(true)
            {
                let application_pause_command_active = self
                    .pending_attached_player_pause_command
                    .is_some_and(|pending| pending.suppress_until > now);
                let previous_paused = self.player_paused;
                let accept_paused = match self.pending_attached_player_pause_command {
                    Some(pending) if pending.suppress_until > now => {
                        self.player_paused = Some(pending.target_paused);
                        paused == pending.target_paused
                    }
                    _ => true,
                };
                if accept_paused {
                    if !application_pause_command_active
                        && previous_paused != Some(paused)
                        && paused
                        && self.attached_player_position_is_end_of_file()
                        && let Some(session) = self.session.as_mut()
                        && let Err(error) =
                            session.observe_external_player_end_of_file(system_time_seconds())
                    {
                        eprintln!(
                            "warning: failed to classify attached-player EOF as a technical transition: {error}"
                        );
                    }
                    self.player_paused = Some(paused);
                }
            }
        }
        let quality_suggestion = self
            .session
            .as_mut()
            .and_then(|session| session.take_streaming_quality_downgrade_suggestion());
        if let Some(suggestion) = quality_suggestion {
            let reason = match suggestion.reason {
                StreamingQualitySuggestionReason::RepeatedRebuffering => {
                    "repeated buffering was observed"
                }
                StreamingQualitySuggestionReason::InsufficientObservedInputRate => {
                    "the observed input rate is below the selected stream's needs"
                }
            };
            self.queue_stream_warning(format!(
                "Stream quality suggestion: change from '{}' to '{}' because {reason}. Sorotte did not change quality automatically.",
                suggestion.current.config_value(),
                suggestion.recommended.config_value(),
            ));
        }
        let timeout_action = self
            .session
            .as_mut()
            .and_then(|session| session.take_playback_barrier_timeout_action());
        match timeout_action {
            Some(PlaybackBarrierTimeoutAction::RemainPaused) => self.queue_stream_warning(
                "Playback start timed out and the room was kept paused. The controller can start it manually when ready."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::AskController) => self.queue_stream_warning(
                "Playback start timed out. The room is paused and waiting for the controller to decide whether to continue."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::Continue) | None => {}
        }
        self.clamp_player_position_to_file_duration();
    }

    pub(in crate::app::runtime_owner) fn emit_gui_actions_to_attached_player_impl(
        &mut self,
        actions: &[GuiShellAction],
    ) {
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return;
        };
        let mut already_emitted_osd_messages = BTreeSet::new();
        for action in actions {
            match action {
                GuiShellAction::PushChatMessage { sender, message } => {
                    if let Err(error) =
                        player.show_syncplay_legacy_chat_message(&format!("<{sender}> {message}"))
                    {
                        eprintln!(
                            "warning: failed to display GUI chat notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::PushTransientNotification { level, message } => {
                    already_emitted_osd_messages.insert(message.clone());
                    let kind = match level {
                        GuiTransientNotificationLevel::Info
                        | GuiTransientNotificationLevel::Success => {
                            LegacySyncplayOsdKind::Notification
                        }
                        GuiTransientNotificationLevel::Warning
                        | GuiTransientNotificationLevel::Error => LegacySyncplayOsdKind::Alert,
                    };
                    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
                        eprintln!(
                            "warning: failed to display GUI notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if already_emitted_osd_messages.insert(message.clone()) =>
                {
                    if let Err(error) = player
                        .show_syncplay_legacy_message(message, LegacySyncplayOsdKind::Notification)
                    {
                        eprintln!(
                            "warning: failed to display GUI system-chat event via mpv OSD: {error}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    pub(in crate::app::runtime_owner) fn drain_player_chat_input_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let mut errors = Vec::new();
        let chat_ready = self
            .session
            .as_ref()
            .is_some_and(|session| session.attached_player_chat_input_ready());
        let unavailable_message = self
            .session
            .as_ref()
            .map(|session| session.attached_player_chat_input_unavailable_message())
            .unwrap_or_else(|| {
                "Chat input from the attached player requires an active session with chat support."
                    .to_owned()
            });
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
            if !chat_ready {
                errors.push(unavailable_message.clone());
                continue;
            }
            let Some(session) = self.session.as_mut() else {
                errors.push(unavailable_message.clone());
                continue;
            };
            let send_result = session.send_chat_message(message.clone());
            if let Err(error) = send_result {
                errors.push(format!(
                    "Chat input from the attached player could not be sent: {error}"
                ));
            }
        }

        if !errors.is_empty() {
            Self::push_actions_and_project(
                handle,
                projected_state,
                errors
                    .into_iter()
                    .flat_map(|message| {
                        [
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: message.clone(),
                            },
                            GuiShellAction::AnnounceSystemChatEvent(message),
                        ]
                    })
                    .collect(),
            );
        }
    }

    fn apply_ordered_transport_projection(
        &mut self,
        transport: &PlayerTransportSnapshot,
        user_offset_seconds: f64,
    ) {
        self.player_position_seconds = snapshot_known_copy(&transport.position_seconds)
            .map(|position| position - user_offset_seconds);
        self.player_paused = snapshot_known_copy(&transport.logical_pause);
        self.player_paused_for_cache = snapshot_known_copy(&transport.paused_for_cache);
        self.player_cache_buffering_percent = snapshot_known_copy(&transport.cache_percentage);
    }

    fn forward_ordered_transport_update(
        &mut self,
        update: sorotte_player_api::PlayerTransportTelemetryUpdate,
    ) {
        let player_position_seconds = update
            .position_seconds
            .map(|position| position + self.user_offset_seconds);
        let previous_native_seek_tracker = self.attached_native_seek_tracker;
        let GuiAttachedTransportObservationDisposition::Accepted {
            update,
            native_seek_classification,
        } = self.attached_native_seek_tracker.observe(update, true)
        else {
            return;
        };
        let update = *update;
        self.reconcile_pending_logical_override_media_generation(update.media_generation);
        self.attached_transport_telemetry_authority.position |= update.position_seconds.is_some();
        self.attached_transport_telemetry_authority.logical_pause |= update.logical_pause.is_some();
        self.attached_transport_telemetry_authority.paused_for_cache |=
            update.paused_for_cache.is_some();
        self.attached_transport_telemetry_authority
            .cache_buffering_percent |= update.cache_buffering_percent.is_some();
        if let Some(position_seconds) = update.position_seconds
            && let Some(unexpected_position_jump) = native_seek_classification
        {
            let system_seek_owned = player_position_seconds.is_some_and(|position| {
                self.consume_matching_attached_system_seek(
                    update.media_generation,
                    update.observed_at,
                    position,
                )
            });
            let fail_closed = unexpected_position_jump
                && self.attached_system_seek_classification_is_fail_closed(update.media_generation);
            let position_accepted = self.sync_attached_player_position_observation(
                position_seconds,
                unexpected_position_jump && !system_seek_owned && !fail_closed,
            );
            if unexpected_position_jump && !position_accepted {
                self.attached_native_seek_tracker.position_anchor =
                    previous_native_seek_tracker.position_anchor;
                self.attached_native_seek_tracker.interval_disarmed =
                    previous_native_seek_tracker.interval_disarmed;
                self.attached_native_seek_tracker.seeking_since_anchor =
                    previous_native_seek_tracker.seeking_since_anchor;
            }
            if position_accepted {
                self.player_position_seconds = Some(position_seconds);
            }
        }
        let actions = self.session.as_mut().and_then(|session| {
            match session.sync_attached_player_transport_telemetry(
                update,
                system_time_seconds(),
            ) {
                Ok(actions) => Some(actions),
                Err(error) => {
                    eprintln!(
                        "warning: failed to feed ordered attached-player transport telemetry to client-core coordinator: {error}"
                    );
                    None
                }
            }
        });
        if let Some(actions) = actions {
            let _ =
                self.apply_attached_player_runtime_actions_impl(actions, "transport observation");
        }
    }

    fn apply_ordered_snapshot(
        &mut self,
        snapshot: &PlayerAuthoritativeSnapshot,
        user_offset_seconds: f64,
    ) {
        self.ordered_player_events.rebase_snapshot(snapshot);
        self.pending_attached_player_pause_command = None;
        self.pending_local_attached_pause_override = None;
        match snapshot.active_load {
            SnapshotField::Known(active) if active.physical_file_loaded => {
                if let Some(path) = snapshot_known_clone(&snapshot.current_path) {
                    let sequence =
                        PlayerEventSequence::new(snapshot.sequence_boundary.through_sequence);
                    self.attached_media_observation_cursor
                        .last_ordered_event_sequence = Some(sequence);
                    let _ = self.process_attached_local_file_observation(
                        PlayerLocalFileObservation::new(
                            local_file_update_for_player_path(&path),
                            Some(active.media_generation),
                            snapshot_known_copy(&snapshot.transport.observed_at),
                        ),
                        Some(sequence),
                        true,
                    );
                } else {
                    self.player_local_file = None;
                    self.player_local_file_placeholder = false;
                }
            }
            SnapshotField::Known(_) => {
                if !self.player_local_file_placeholder {
                    self.player_local_file = None;
                }
            }
            SnapshotField::KnownAbsent => {
                self.player_local_file = None;
                self.player_local_file_placeholder = false;
            }
            SnapshotField::Unavailable => {}
        }
        let transport = self.ordered_player_events.transport.clone();
        self.apply_ordered_transport_projection(&transport, user_offset_seconds);
        self.playlist_auto_advance_eof_latched =
            snapshot_known_copy(&transport.eof_reached).unwrap_or(false);

        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.sync_local_playback_cache_state(
                snapshot_known_copy(&transport.paused_for_cache),
                snapshot_known_copy(&transport.cache_percentage),
            )
        {
            eprintln!(
                "warning: failed to replace attached-player cache buffering state in the session runtime: {error}"
            );
        }
        self.forward_ordered_transport_update(transport_update_from_snapshot(
            &transport,
            user_offset_seconds,
        ));
    }

    fn apply_ordered_transport_delta(
        &mut self,
        delta: PlayerTransportDelta,
        user_offset_seconds: f64,
    ) {
        if let Some(paused_for_cache) = delta.paused_for_cache {
            self.player_paused_for_cache = Some(paused_for_cache);
        }
        if let Some(cache_percentage) = delta.cache_percentage {
            self.player_cache_buffering_percent = Some(cache_percentage);
        }
        if delta.observed_at.is_none()
            && let Some(position_seconds) = delta.position_seconds
        {
            // Ordered third-party adapters may omit a sample clock. The value is still valid
            // for authoritative UI projection, but the native-seek classifier below refuses to
            // use it as motion evidence.
            self.player_position_seconds = Some(position_seconds - user_offset_seconds);
        }
        if let Some(logical_pause) = delta.logical_pause
            && self.player_paused_for_cache != Some(true)
        {
            self.player_paused = Some(logical_pause);
        }
        if delta.eof_reached == Some(false) {
            self.playlist_auto_advance_eof_latched = false;
        }
        if (delta.paused_for_cache.is_some() || delta.cache_percentage.is_some())
            && let Some(session) = self.session.as_mut()
            && let Err(error) = session
                .sync_local_playback_cache_state(delta.paused_for_cache, delta.cache_percentage)
        {
            eprintln!(
                "warning: failed to mirror ordered attached-player cache buffering state into the session runtime: {error}"
            );
        }
        self.forward_ordered_transport_update(transport_update_from_delta(
            delta,
            user_offset_seconds,
        ));
    }

    fn apply_ordered_event(
        &mut self,
        event: SequencedPlayerEvent,
        user_offset_seconds: f64,
    ) -> Result<Option<GuiMediaBoundary>, sorotte_player_api::PlayerError> {
        let sequence = PlayerEventSequence::new(event.order.sequence);
        match event.event {
            PlayerEvent::AttachmentReplaced { .. } | PlayerEvent::EventGapDetected => Ok(None),
            PlayerEvent::LocalFileChanged {
                attempt_id,
                media_generation,
                update,
            } => {
                if !self
                    .ordered_player_events
                    .attempt_is_owned(attempt_id, media_generation)
                {
                    return Ok(None);
                }
                Ok(self.process_attached_local_file_observation(
                    PlayerLocalFileObservation::new(update, Some(media_generation), None),
                    Some(sequence),
                    false,
                ))
            }
            PlayerEvent::TransportDelta(delta) => {
                let Some(accepted) = self.ordered_player_events.apply_delta_if_owned(delta) else {
                    return Ok(None);
                };
                self.apply_ordered_transport_delta(accepted, user_offset_seconds);
                Ok(None)
            }
            PlayerEvent::LoadAttemptBound {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
            } => {
                self.ordered_player_events.validate_attempt_binding(
                    attempt_id,
                    media_generation,
                    command_id,
                    Some(playlist_entry_id),
                )?;
                self.ordered_player_events.install_attempt(
                    attempt_id,
                    GuiOrderedLoadInstall {
                        media_generation,
                        command_id,
                        playlist_entry_id: Some(playlist_entry_id),
                        owns_transport: false,
                        semantic_load_result: None,
                        logical_ownership_revoked: false,
                    },
                );
                self.track_playlist_resolution_load_attempt(
                    attempt_id,
                    media_generation,
                    command_id,
                );
                Ok(None)
            }
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
                owns_transport,
            } => {
                self.ordered_player_events.validate_attempt_binding(
                    attempt_id,
                    media_generation,
                    command_id,
                    Some(playlist_entry_id),
                )?;
                self.ordered_player_events.install_attempt(
                    attempt_id,
                    GuiOrderedLoadInstall {
                        media_generation,
                        command_id,
                        playlist_entry_id: Some(playlist_entry_id),
                        owns_transport,
                        semantic_load_result: None,
                        logical_ownership_revoked: false,
                    },
                );
                self.track_playlist_resolution_load_attempt(
                    attempt_id,
                    media_generation,
                    command_id,
                );
                Ok(None)
            }
            PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
            } => {
                self.ordered_player_events.validate_attempt_binding(
                    attempt_id,
                    media_generation,
                    command_id,
                    Some(playlist_entry_id),
                )?;
                self.ordered_player_events.install_attempt(
                    attempt_id,
                    GuiOrderedLoadInstall {
                        media_generation,
                        command_id,
                        playlist_entry_id: Some(playlist_entry_id),
                        owns_transport: true,
                        semantic_load_result: None,
                        logical_ownership_revoked: false,
                    },
                );
                self.recover_playlist_resolution_from_active_load(
                    attempt_id,
                    media_generation,
                    command_id,
                );
                Ok(None)
            }
            PlayerEvent::LoadAttemptLogicalOwnershipRevoked {
                attempt_id,
                media_generation,
                ..
            } => {
                self.ordered_player_events
                    .revoke_logical_ownership(attempt_id, media_generation);
                Ok(None)
            }
            PlayerEvent::LoadAttemptTerminal {
                attempt_id,
                media_generation,
                ..
            } => {
                self.ordered_player_events
                    .terminate_attempt(attempt_id, media_generation);
                Ok(None)
            }
            PlayerEvent::LogicalPlaybackTerminal {
                media_generation,
                attempt_id,
                ..
            } => {
                let transport_owns_attempt =
                    snapshot_known_copy(&self.ordered_player_events.transport.load_attempt_id)
                        == Some(attempt_id)
                        && snapshot_known_copy(
                            &self.ordered_player_events.transport.media_generation,
                        ) == Some(media_generation);
                if self.ordered_player_events.outcome_matches_attempt(
                    attempt_id,
                    media_generation,
                    None,
                ) && transport_owns_attempt
                {
                    self.ordered_player_events.transport.phase =
                        SnapshotField::Known(sorotte_player_api::PlayerTransportPhase::Ended);
                    self.ordered_player_events.transport.logical_pause = SnapshotField::Known(true);
                    self.ordered_player_events.transport.eof_reached = SnapshotField::Known(true);
                    self.player_paused = Some(true);
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) =
                            session.observe_external_player_end_of_file(system_time_seconds())
                    {
                        eprintln!(
                            "warning: failed to route ordered attached-player terminal playback to the session runtime: {error}"
                        );
                    }
                }
                Ok(None)
            }
        }
    }

    fn apply_ordered_semantic_outcome(
        &mut self,
        outcome: SequencedPlayerSemanticOutcome,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        match outcome.outcome {
            PlayerSemanticOutcome::Command(command) => {
                if self.ordered_player_events.attachment_epoch != Some(command.attachment_epoch) {
                    return Err(ordered_batch_error(
                        "command outcome belongs to another attachment",
                    ));
                }
                let result = match command.result {
                    PlayerCommandSemanticResult::Completed => PlayerCommandResult::Completed,
                    PlayerCommandSemanticResult::Superseded => PlayerCommandResult::Superseded,
                    PlayerCommandSemanticResult::Failed(kind) => PlayerCommandResult::Failed(kind),
                    PlayerCommandSemanticResult::CompletionNotObserved => {
                        PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut)
                    }
                    PlayerCommandSemanticResult::TransportDisconnected => {
                        PlayerCommandResult::Failed(PlayerCommandFailureKind::TransportDisconnected)
                    }
                };
                let progress = PlayerCommandProgress::finished(
                    command.command_id,
                    command.media_generation,
                    None,
                    None,
                    result,
                );
                self.handle_playlist_resolution_command_progress(progress);
                self.reconcile_attached_system_seek_command_progress(progress);
            }
            PlayerSemanticOutcome::LoadAttempt(load) => {
                if self.ordered_player_events.attachment_epoch != Some(load.attachment_epoch) {
                    return Err(ordered_batch_error(
                        "load-attempt outcome belongs to another attachment",
                    ));
                }
                self.ordered_player_events.ensure_attempt(
                    load.attempt_id,
                    load.media_generation,
                    load.command_id,
                );
                if !self.ordered_player_events.outcome_matches_attempt(
                    load.attempt_id,
                    load.media_generation,
                    load.command_id,
                ) {
                    return Ok(());
                }
                self.ordered_player_events.mark_semantic_load_result(
                    load.attempt_id,
                    load.media_generation,
                    load.result,
                );
                match load.result {
                    PlayerLoadAttemptResult::Loaded => {}
                    PlayerLoadAttemptResult::Superseded => {
                        self.ordered_player_events
                            .revoke_logical_ownership(load.attempt_id, load.media_generation);
                        self.supersede_playlist_resolution_load_attempt(
                            load.attempt_id,
                            load.media_generation,
                        );
                    }
                    PlayerLoadAttemptResult::Indeterminate => {
                        self.ordered_player_events
                            .mark_indeterminate(load.attempt_id, load.media_generation);
                        self.mark_playlist_resolution_load_indeterminate(
                            load.attempt_id,
                            load.media_generation,
                            load.command_id,
                        );
                    }
                    PlayerLoadAttemptResult::Failed(_)
                    | PlayerLoadAttemptResult::NeverStarted
                    | PlayerLoadAttemptResult::TransportDisconnected => self
                        .ordered_player_events
                        .terminate_attempt(load.attempt_id, load.media_generation),
                }
                let attempt_is_owned = self
                    .ordered_player_events
                    .attempt_is_owned(load.attempt_id, load.media_generation);
                let legacy_outcome = match load.result {
                    PlayerLoadAttemptResult::Loaded if attempt_is_owned => Some(
                        PlayerMediaLoadOutcome::success(load.requested_target, load.loaded_target),
                    ),
                    PlayerLoadAttemptResult::Loaded | PlayerLoadAttemptResult::Superseded => None,
                    PlayerLoadAttemptResult::Failed(kind) => Some(PlayerMediaLoadOutcome::failure(
                        load.requested_target,
                        load.loaded_target,
                        kind,
                        "ordered player load attempt failed",
                    )),
                    PlayerLoadAttemptResult::NeverStarted => Some(PlayerMediaLoadOutcome::failure(
                        load.requested_target,
                        load.loaded_target,
                        PlayerMediaLoadFailureKind::Unknown,
                        "ordered player load attempt never started",
                    )),
                    PlayerLoadAttemptResult::TransportDisconnected => {
                        Some(PlayerMediaLoadOutcome::failure(
                            load.requested_target,
                            load.loaded_target,
                            PlayerMediaLoadFailureKind::Unknown,
                            "player transport disconnected during ordered load attempt",
                        ))
                    }
                    PlayerLoadAttemptResult::Indeterminate => None,
                };
                if let Some(legacy_outcome) = legacy_outcome {
                    self.handle_playlist_media_load_outcome(&legacy_outcome);
                    self.handle_player_media_load_outcome(legacy_outcome);
                }
            }
        }
        Ok(())
    }

    fn apply_ordered_player_event_batch(
        &mut self,
        batch: &PlayerEventBatch,
        user_offset_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        let mut prepared_consumer = self.ordered_player_events.clone();
        prepared_consumer.begin_batch(batch)?;
        prepared_consumer.validate_sequence_continuity(batch)?;
        if let Some(pending) = prepared_consumer.applied_unacknowledged_token
            && pending != batch.acknowledgement_token
        {
            return Err(ordered_batch_error(
                "adapter replaced an applied batch before acknowledgement",
            ));
        }
        if prepared_consumer.applied_unacknowledged_token == Some(batch.acknowledgement_token) {
            return Ok(());
        }
        self.ordered_player_events = prepared_consumer;

        if let Some(snapshot) = batch.authoritative_snapshot.as_ref()
            && self
                .ordered_player_events
                .should_rebase_snapshot(snapshot.sequence_boundary)
        {
            self.apply_ordered_snapshot(snapshot, user_offset_seconds);
        }

        for delivery in GuiOrderedPlayerEventConsumer::merged_deliveries(batch) {
            let order = delivery.order();
            match delivery {
                GuiOrderedPlayerDelivery::Event(event) => {
                    if self
                        .ordered_player_events
                        .event_is_covered_by_snapshot(order)
                        || order.sequence <= self.ordered_player_events.last_sequence
                    {
                        continue;
                    }
                    self.ordered_player_events.require_next_order(order)?;
                    self.apply_ordered_event(event, user_offset_seconds)?;
                    self.ordered_player_events.record_order(order);
                }
                GuiOrderedPlayerDelivery::SemanticOutcome(outcome) => {
                    if self
                        .ordered_player_events
                        .semantic_outcome_was_applied(order)
                    {
                        continue;
                    }
                    if order.sequence > self.ordered_player_events.last_sequence {
                        self.ordered_player_events.require_next_order(order)?;
                    }
                    self.apply_ordered_semantic_outcome(outcome)?;
                    self.ordered_player_events.record_semantic_outcome(order);
                }
            }
        }
        self.ordered_player_events.applied_unacknowledged_token = Some(batch.acknowledgement_token);
        Ok(())
    }

    fn drain_ordered_player_events(&mut self, user_offset_seconds: f64) {
        loop {
            let batch = self
                .player
                .as_mut()
                .and_then(PlayerAdapter::take_player_event_batch);
            let Some(batch) = batch else {
                return;
            };
            if let Err(error) = self.apply_ordered_player_event_batch(&batch, user_offset_seconds) {
                eprintln!("warning: rejected ordered player event batch: {error}");
                return;
            }
            let acknowledgement = self
                .player
                .as_mut()
                .map(|player| player.acknowledge_player_event_batch(batch.acknowledgement_token));
            match acknowledgement {
                Some(Ok(())) => {
                    self.ordered_player_events.compact_acknowledged_delivery(
                        batch.acknowledgement_token,
                        batch.sequence_boundary,
                    );
                }
                Some(Err(error)) => {
                    eprintln!("warning: failed to acknowledge ordered player event batch: {error}");
                    return;
                }
                None => return,
            }
        }
    }

    pub(in crate::app::runtime_owner) fn refresh_player_state_impl(&mut self) {
        if self.player.as_ref().is_some_and(|player| {
            player.player_event_delivery_mode()
                != PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        }) {
            self.refresh_legacy_player_state_impl();
            return;
        }
        let user_offset_seconds = self.user_offset_seconds;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let delivery_mode = player.player_event_delivery_mode();
        let mut hook_health_transitions = Vec::new();
        let mut media_policy_outcomes = Vec::new();
        let mut network_options_snapshot = None;
        let mut mpv_connected = true;
        if let Some(player) = player.as_mpv_mut() {
            while let Some(transition) = player.take_network_options_hook_health_transition() {
                hook_health_transitions.push(transition);
            }
            while let Some(outcome) = player.take_network_media_policy_outcome() {
                media_policy_outcomes.push(outcome);
            }
            network_options_snapshot = Some(player.network_options_runtime_health_snapshot());
            mpv_connected = player.is_connected();
        }
        if !mpv_connected {
            // A queued sample from before IPC loss must not re-establish an
            // observation-derived Connected status after lifecycle evidence
            // has declared the player unavailable.
            self.report_external_player_availability(ExternalPlayerAvailability::Disconnected);
        }
        for transition in hook_health_transitions {
            match transition {
                MpvNetworkOptionsHookHealthTransition::Recovered => {
                    self.record_network_options_hook_recovered();
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) if mpv_connected => {
                    self.mark_network_options_hook_degraded(format!(
                        "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry or player restart: {error}"
                    ));
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while maintaining Sorotte's core streaming-settings hook: {error}"
                    ));
                    return;
                }
            }
        }
        for outcome in media_policy_outcomes {
            match outcome {
                MpvNetworkMediaPolicyOutcome::NoActiveMedia
                | MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged
                | MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                    self.record_network_media_transition_recovered();
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) if mpv_connected => {
                    self.mark_network_media_transition_apply_failed(format!(
                        "mpv switched to network media, but configured streaming settings could not be applied to the new file: {error}"
                    ));
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while applying configured streaming settings to newly active network media: {error}"
                    ));
                    return;
                }
            }
        }
        if let Some(snapshot) = network_options_snapshot
            && !self.reconcile_network_options_runtime_health_snapshot(snapshot, mpv_connected)
        {
            return;
        }
        let now = Instant::now();
        if self
            .pending_attached_player_pause_command
            .is_some_and(|pending| pending.suppress_until <= now)
        {
            self.pending_attached_player_pause_command = None;
        }
        if delivery_mode == PlayerEventDeliveryMode::OrderedAcknowledgedBatches {
            self.drain_ordered_player_events(user_offset_seconds);
            self.finish_player_state_refresh();
            return;
        }

        let Some(player) = self.player.as_mut() else {
            return;
        };
        let mut playback_updates = Vec::new();
        let mut transport_updates = Vec::new();
        let mut command_progress_updates = Vec::new();
        let mut media_load_outcomes = Vec::new();
        let mut local_file_updates = Vec::new();
        while let Some(progress) = player.take_command_progress() {
            command_progress_updates.push(progress);
        }
        while let Some(update) = player.take_playback_telemetry_update() {
            playback_updates.push(update);
        }
        while let Some(update) = player.take_transport_telemetry_update() {
            transport_updates.push(update);
        }
        if !mpv_connected {
            transport_updates.clear();
        }
        while let Some(outcome) = player.take_media_load_outcome() {
            media_load_outcomes.push(outcome);
        }
        while let Some(update) = player.take_local_file_update() {
            local_file_updates.push(update);
        }
        for update in playback_updates {
            if let Some(paused_for_cache) = update.paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                self.player_cache_buffering_percent = Some(cache_buffering_percent);
            }
            if (update.paused_for_cache.is_some() || update.cache_buffering_percent.is_some())
                && let Some(session) = self.session.as_mut()
                && let Err(error) = session.sync_local_playback_cache_state(
                    update.paused_for_cache,
                    update.cache_buffering_percent,
                )
            {
                eprintln!(
                    "warning: failed to mirror attached-player cache buffering state into the session runtime: {error}"
                );
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
            if let Some(paused) = update.paused
                && self.player_paused_for_cache != Some(true)
            {
                let application_pause_command_active = self
                    .pending_attached_player_pause_command
                    .is_some_and(|pending| pending.suppress_until > now);
                let previous_paused = self.player_paused;
                let accept_paused = match self.pending_attached_player_pause_command {
                    Some(pending) if pending.suppress_until > now => {
                        self.player_paused = Some(pending.target_paused);
                        paused == pending.target_paused
                    }
                    _ => true,
                };
                if accept_paused {
                    if !application_pause_command_active
                        && previous_paused != Some(paused)
                        && paused
                        && self.attached_player_position_is_end_of_file()
                        && let Some(session) = self.session.as_mut()
                        && let Err(error) =
                            session.observe_external_player_end_of_file(system_time_seconds())
                    {
                        eprintln!(
                            "warning: failed to classify attached-player EOF as a technical transition: {error}"
                        );
                    }
                    self.player_paused = Some(paused);
                }
            }
        }
        for outcome in media_load_outcomes {
            self.handle_playlist_media_load_outcome(&outcome);
            self.handle_player_media_load_outcome(outcome);
        }
        for mut update in local_file_updates {
            let mut logical_override_confirmed = None;
            if let Some((override_update, confirmed)) =
                self.logical_media_override_for_loaded_target(&update, None)
            {
                update = override_update;
                logical_override_confirmed = Some(confirmed);
            }
            self.handle_untracked_playlist_local_file_observation(&update);
            let tracked_playlist_load_unconfirmed =
                self.tracked_playlist_resolution_load_matches_local_file(&update);
            let file_changed = Self::local_file_update_replaces_current_file(
                self.player_local_file.as_ref(),
                &update,
            );
            if file_changed {
                self.pending_local_attached_pause_override = None;
                let _ = self
                    .interrupt_attached_playback_recovery_impl("observed media transport change");
                let logical_id = logical_media_id_for_local_file_update(&update);
                let kind = if update.path.as_deref().is_some_and(browser_is_url)
                    || browser_is_url(&update.name)
                {
                    MediaTransportKind::NetworkVod
                } else {
                    MediaTransportKind::LocalFile
                };
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.prepare_attached_playback_media(
                        logical_id,
                        kind,
                        MediaLoadIntent::TransportRefresh,
                        system_time_seconds(),
                    )
                {
                    eprintln!(
                        "warning: failed to prepare attached-player logical media generation: {error}"
                    );
                }
            }
            self.player_local_file = Some(update);
            self.player_local_file_placeholder = tracked_playlist_load_unconfirmed
                || logical_override_confirmed.is_some_and(|confirmed| !confirmed);
            if file_changed || self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
        // A tracked load's terminal result is the final authority for the
        // provisional identity observed in the queues above. Processing it
        // last prevents an earlier file-loaded observation from resurrecting
        // media that the same command subsequently rejected.
        for progress in command_progress_updates {
            self.handle_playlist_resolution_command_progress(progress);
        }
        for update in transport_updates {
            self.reconcile_pending_logical_override_media_generation(update.media_generation);
            let update = transport_update_on_room_timeline(update, user_offset_seconds);
            if let Some(paused_for_cache) = update.paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds);
            }
            if let Some(logical_pause) = update.logical_pause
                && self.player_paused_for_cache != Some(true)
            {
                self.player_paused = Some(logical_pause);
            }
            let actions = self.session.as_mut().and_then(|session| {
                match session.sync_attached_player_transport_telemetry(
                    update,
                    system_time_seconds(),
                ) {
                    Ok(actions) => Some(actions),
                    Err(error) => {
                        eprintln!(
                            "warning: failed to feed attached-player transport telemetry to client-core coordinator: {error}"
                        );
                        None
                    }
                }
            });
            if let Some(actions) = actions {
                let _ = self
                    .apply_attached_player_runtime_actions_impl(actions, "transport observation");
            }
        }
        self.finish_player_state_refresh();
    }

    fn finish_player_state_refresh(&mut self) {
        let quality_suggestion = self
            .session
            .as_mut()
            .and_then(|session| session.take_streaming_quality_downgrade_suggestion());
        if let Some(suggestion) = quality_suggestion {
            let reason = match suggestion.reason {
                StreamingQualitySuggestionReason::RepeatedRebuffering => {
                    "repeated buffering was observed"
                }
                StreamingQualitySuggestionReason::InsufficientObservedInputRate => {
                    "the observed input rate is below the selected stream's needs"
                }
            };
            self.queue_stream_warning(format!(
                "Stream quality suggestion: change from '{}' to '{}' because {reason}. Sorotte did not change quality automatically.",
                suggestion.current.config_value(),
                suggestion.recommended.config_value(),
            ));
        }
        let timeout_action = self
            .session
            .as_mut()
            .and_then(|session| session.take_playback_barrier_timeout_action());
        match timeout_action {
            Some(PlaybackBarrierTimeoutAction::RemainPaused) => self.queue_stream_warning(
                "Playback start timed out and the room was kept paused. The controller can start it manually when ready."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::AskController) => self.queue_stream_warning(
                "Playback start timed out. The room is paused and waiting for the controller to decide whether to continue."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::Continue) | None => {}
        }
        self.clamp_player_position_to_file_duration();
    }

    pub(in crate::app::runtime_owner) fn mark_network_media_transition_apply_failed(
        &mut self,
        reason: String,
    ) {
        self.player_apply_state.mark_streaming_apply_failed();
        self.pending_apply_requirements_refresh_required = true;
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place: true,
                origin: GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
            };
        self.player_unavailability_reason = Some(reason);
    }

    fn reconcile_network_options_runtime_health_snapshot(
        &mut self,
        snapshot: MpvNetworkOptionsRuntimeHealthSnapshot,
        mpv_connected: bool,
    ) -> bool {
        // Apply the snapshot after both event queues every time. A transition can be enqueued by
        // the maintenance performed while draining the other channel, so revision equality alone
        // is not sufficient to skip this final authoritative reconciliation.
        self.network_options_runtime_health_revision = Some(snapshot.revision);
        match snapshot.hook_health {
            MpvNetworkOptionsHookHealth::Ready => self.record_network_options_hook_recovered(),
            MpvNetworkOptionsHookHealth::Degraded(reason) if mpv_connected => {
                self.mark_network_options_hook_degraded(format!(
                    "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry or player restart: {reason}"
                ));
            }
            MpvNetworkOptionsHookHealth::Degraded(reason) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.detach_player();
                self.player_unavailability_reason = Some(format!(
                    "mpv JSON IPC became unavailable while maintaining Sorotte's core streaming-settings hook: {reason}"
                ));
                return false;
            }
            MpvNetworkOptionsHookHealth::Pending => {}
        }
        match snapshot.media_policy {
            MpvNetworkMediaPolicyState::NoActiveMedia
            | MpvNetworkMediaPolicyState::LocalMediaUnchanged
            | MpvNetworkMediaPolicyState::NetworkMediaUpdated => {
                self.record_network_media_transition_recovered();
            }
            MpvNetworkMediaPolicyState::Failed(reason) if mpv_connected => {
                if !matches!(
                    self.core_player_configuration_health,
                    GuiCorePlayerConfigurationHealth::StreamingDegraded {
                        origin: GuiStreamingDegradationOrigin::ExplicitApply,
                        ..
                    }
                ) {
                    self.mark_network_media_transition_apply_failed(format!(
                        "mpv switched to network media, but configured streaming settings could not be applied to the new file: {reason}"
                    ));
                }
            }
            MpvNetworkMediaPolicyState::Failed(reason) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.detach_player();
                self.player_unavailability_reason = Some(format!(
                    "mpv JSON IPC became unavailable while applying configured streaming settings to newly active network media: {reason}"
                ));
                return false;
            }
            MpvNetworkMediaPolicyState::Unknown
            | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad => {}
        }
        true
    }

    fn mark_network_options_hook_degraded(&mut self, reason: String) {
        self.network_options_hook_failure_reason = Some(reason.clone());
        // Hook health is independent of an explicit media-policy apply that is still awaiting
        // its authoritative load result. Preserve that latch so NoActive/Local/Network/Failed can
        // resolve the policy baseline even while future hook transitions remain unprotected.
        self.player_apply_state.core_reapply_required = true;
        self.pending_apply_requirements_refresh_required = true;
        if matches!(
            self.core_player_configuration_health,
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                origin: GuiStreamingDegradationOrigin::ExplicitApply
                    | GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
                ..
            }
        ) {
            return;
        }
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place: true,
                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
            };
        self.player_unavailability_reason = Some(reason);
    }

    fn record_network_options_hook_recovered(&mut self) {
        let Some(hook_failure_reason) = self.network_options_hook_failure_reason.take() else {
            return;
        };
        let hook_issue_is_projected = matches!(
            self.core_player_configuration_health,
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
                ..
            }
        );
        self.pending_apply_requirements_refresh_required = true;
        if !hook_issue_is_projected {
            return;
        }
        self.player_apply_state.core_reapply_required = false;
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        if self.player_unavailability_reason.as_deref() == Some(hook_failure_reason.as_str()) {
            self.player_unavailability_reason = None;
        }
    }

    pub(in crate::app::runtime_owner) fn record_network_media_transition_recovered(&mut self) {
        if self.player_apply_state.streaming_apply_awaiting_transition
            && self
                .player_apply_state
                .process_target_is_applied(&self.player_launch_state)
        {
            self.player_apply_state
                .record_streaming_options_applied(&self.player_launch_state);
            self.pending_apply_requirements_refresh_required = true;
            self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
            if !self.restore_network_options_hook_degradation() {
                self.player_unavailability_reason = None;
            }
            return;
        }
        let transition_failure_reason = match &self.core_player_configuration_health {
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason,
                origin: GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
                ..
            } => reason.clone(),
            GuiCorePlayerConfigurationHealth::Ready
            | GuiCorePlayerConfigurationHealth::StreamingDegraded { .. } => return,
        };
        if !self
            .player_apply_state
            .process_target_is_applied(&self.player_launch_state)
            || !self
                .player_apply_state
                .streaming_options_are_applied(&self.player_launch_state)
        {
            return;
        }

        self.player_apply_state.core_reapply_required = false;
        self.pending_apply_requirements_refresh_required = true;
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        if !self.restore_network_options_hook_degradation()
            && self.player_unavailability_reason.as_deref()
                == Some(transition_failure_reason.as_str())
        {
            self.player_unavailability_reason = None;
        }
    }

    pub(in crate::app::runtime_owner) fn player_local_file_ready_for_attached_sync(&self) -> bool {
        self.player_local_file.is_some()
            && self.player_local_file_identity_confirmed_for_shared_sync()
    }

    fn logical_media_override_for_loaded_target(
        &mut self,
        update: &LocalFileUpdate,
        observed_media_generation: Option<PlayerMediaGeneration>,
    ) -> Option<(LocalFileUpdate, bool)> {
        let scope_matches = self
            .pending_logical_media_override
            .as_ref()
            .is_some_and(|pending| {
                pending.playlist_row_id.is_none()
                    || (pending.playlist_generation == self.playlist_resolution.generation
                        && self
                            .playlist_resolution_attempt
                            .as_ref()
                            .is_some_and(|attempt| {
                                Some(attempt.row_id) == pending.playlist_row_id
                                    && attempt.playlist_generation == pending.playlist_generation
                                    && (pending.load_completed
                                        || attempt.player_command_id == pending.player_command_id)
                            }))
            });
        if !scope_matches {
            self.pending_logical_media_override = None;
            return None;
        }
        let (expected_media_generation, exact_target_match) = self
            .pending_logical_media_override
            .as_ref()
            .map(|pending| {
                let loaded_target = pending.loaded_target_secret.as_str();
                (
                    pending.player_media_generation,
                    update
                        .path
                        .as_deref()
                        .is_some_and(|path| path == loaded_target)
                        || update.name == loaded_target,
                )
            })
            .expect("scoped pending logical override should exist");
        let generation_matches = match (expected_media_generation, observed_media_generation) {
            (Some(expected), Some(observed)) if observed == expected => true,
            (Some(expected), Some(observed)) if observed > expected => {
                // A newer physical media generation is an authoritative
                // external/superseding load. It must not inherit the Plex
                // identity from the generation it replaced.
                self.pending_logical_media_override = None;
                return None;
            }
            (Some(_), Some(_)) => return None,
            (None, Some(observed)) if exact_target_match => {
                self.pending_logical_media_override
                    .as_mut()
                    .expect("scoped pending logical override should exist")
                    .player_media_generation = Some(observed);
                true
            }
            (None, Some(_)) => {
                self.pending_logical_media_override = None;
                return None;
            }
            (_, None) => exact_target_match,
        };
        if !generation_matches {
            // Generation-less adapters can only prove ownership by reporting
            // the exact target Sorotte asked them to load.
            self.pending_logical_media_override = None;
            return None;
        }

        let (logical_file, confirmed) = {
            let pending = self
                .pending_logical_media_override
                .as_mut()
                .expect("matching pending logical override should exist");
            pending.logical_file_observed = true;
            (pending.logical_file.clone(), pending.load_completed)
        };
        Some((logical_file, confirmed))
    }
}

#[cfg(test)]
mod logical_media_projection_tests {
    use super::*;
    use crate::app::runtime_owner::GuiPendingLogicalMediaOverride;

    const STREAM_TARGET: &str = "https://plex.example/stream?token=secret";
    const LOGICAL_TARGET: &str = "plex://machine/metadata/123";

    fn pending_override(
        player_command_id: Option<PlayerCommandId>,
        player_media_generation: Option<PlayerMediaGeneration>,
        playlist_row_id: Option<GuiPlaylistEntryId>,
        playlist_generation: u64,
    ) -> GuiPendingLogicalMediaOverride {
        GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(STREAM_TARGET),
            logical_file: LocalFileUpdate::new("episode.mkv").with_path(LOGICAL_TARGET),
            user_initiated: false,
            player_command_id,
            player_media_generation,
            playlist_row_id,
            playlist_generation,
            load_completed: false,
            logical_file_observed: false,
        }
    }

    #[test]
    fn command_scoped_exact_observation_binds_the_authoritative_media_generation() {
        let row_id = GuiPlaylistEntryId::next();
        let command_id = PlayerCommandId::new(22);
        let media_generation = PlayerMediaGeneration::new(7);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner
            .playlist_resolution_attempt
            .as_mut()
            .expect("playlist resolution attempt")
            .player_command_id = Some(command_id);
        owner.pending_logical_media_override =
            Some(pending_override(Some(command_id), None, Some(row_id), 4));

        let projection = owner.logical_media_override_for_loaded_target(
            &LocalFileUpdate::new(STREAM_TARGET).with_path(STREAM_TARGET),
            Some(media_generation),
        );

        assert_eq!(
            projection,
            Some((
                LocalFileUpdate::new("episode.mkv").with_path(LOGICAL_TARGET),
                false,
            ))
        );
        let pending = owner
            .pending_logical_media_override
            .as_ref()
            .expect("matching projection should remain active");
        assert_eq!(pending.player_media_generation, Some(media_generation));
        assert!(pending.logical_file_observed);
    }

    #[test]
    fn older_observations_are_ignored_and_unbound_mismatches_clear_the_projection() {
        let expected_generation = PlayerMediaGeneration::new(7);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.pending_logical_media_override = Some(pending_override(
            Some(PlayerCommandId::new(22)),
            Some(expected_generation),
            None,
            0,
        ));

        let delayed_old_projection = owner.logical_media_override_for_loaded_target(
            &LocalFileUpdate::new(STREAM_TARGET).with_path(STREAM_TARGET),
            Some(PlayerMediaGeneration::new(expected_generation.get() - 1)),
        );

        assert_eq!(delayed_old_projection, None);
        assert!(
            owner.pending_logical_media_override.is_some(),
            "an older observation cannot supersede the active Plex projection"
        );

        owner
            .pending_logical_media_override
            .as_mut()
            .expect("older observation should leave the projection intact")
            .player_media_generation = None;
        let mismatched_projection = owner.logical_media_override_for_loaded_target(
            &LocalFileUpdate::new("https://media.example/new-video.mkv")
                .with_path("https://media.example/new-video.mkv"),
            Some(PlayerMediaGeneration::new(expected_generation.get() + 1)),
        );

        assert_eq!(mismatched_projection, None);
        assert!(
            owner.pending_logical_media_override.is_none(),
            "a different authoritative target must retire an unbound Plex projection"
        );
    }
}

fn transport_update_from_delta(
    delta: PlayerTransportDelta,
    user_offset_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    let mut update = sorotte_player_api::PlayerTransportTelemetryUpdate {
        media_generation: delta.media_generation,
        observed_at: delta.observed_at,
        phase: delta.phase,
        position_seconds: delta.position_seconds,
        playback_rate: delta.playback_rate,
        logical_pause: delta.logical_pause,
        paused_for_cache: delta.paused_for_cache,
        cache_buffering_percent: delta.cache_percentage,
        seeking: delta.seeking,
        seekable: delta.seekable,
        timeline_kind: delta.timeline_kind,
        core_idle: delta.core_idle,
        demuxer_cache_idle: delta.demuxer_cache_idle,
        playback_restart_sequence: delta.playback_restart_sequence,
        eof_reached: delta.eof_reached,
        seekable_ranges: delta.seekable_ranges,
        known_live_seekable_window: delta.known_live_seekable_window,
        buffered_ahead_seconds: delta.buffered_duration_seconds,
        buffered_ahead_bytes: delta.buffered_bytes,
        input_rate_bytes_per_second: delta.input_rate_bytes_per_second,
        error_kind: delta.error_kind,
    };
    update.position_seconds = update
        .position_seconds
        .map(|position| position - user_offset_seconds);
    update.seekable_ranges = update.seekable_ranges.map(|ranges| {
        ranges
            .into_iter()
            .map(|range| range.shifted(-user_offset_seconds))
            .collect()
    });
    update.known_live_seekable_window = update
        .known_live_seekable_window
        .map(|range| range.shifted(-user_offset_seconds));
    update
}

fn transport_update_from_snapshot(
    snapshot: &PlayerTransportSnapshot,
    user_offset_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    transport_update_from_delta(
        PlayerTransportDelta {
            load_attempt_id: snapshot_known_copy(&snapshot.load_attempt_id),
            media_generation: snapshot_known_copy(&snapshot.media_generation),
            observed_at: snapshot_known_copy(&snapshot.observed_at),
            phase: snapshot_known_copy(&snapshot.phase),
            position_seconds: snapshot_known_copy(&snapshot.position_seconds),
            playback_rate: snapshot_known_copy(&snapshot.playback_rate),
            logical_pause: snapshot_known_copy(&snapshot.logical_pause),
            paused_for_cache: snapshot_known_copy(&snapshot.paused_for_cache),
            cache_percentage: snapshot_known_copy(&snapshot.cache_percentage),
            seeking: snapshot_known_copy(&snapshot.seeking),
            seekable: snapshot_known_copy(&snapshot.seekable),
            timeline_kind: snapshot_known_copy(&snapshot.timeline_kind),
            core_idle: snapshot_known_copy(&snapshot.core_idle),
            demuxer_cache_idle: snapshot_known_copy(&snapshot.demuxer_cache_idle),
            playback_restart_sequence: snapshot_known_copy(&snapshot.playback_restart_sequence),
            eof_reached: snapshot_known_copy(&snapshot.eof_reached),
            seekable_ranges: snapshot_known_clone(&snapshot.seekable_ranges),
            known_live_seekable_window: snapshot_known_copy(&snapshot.known_live_seekable_window),
            buffered_duration_seconds: snapshot_known_copy(&snapshot.buffered_duration_seconds),
            buffered_bytes: snapshot_known_copy(&snapshot.buffered_bytes),
            input_rate_bytes_per_second: snapshot_known_copy(&snapshot.input_rate_bytes_per_second),
            error_kind: snapshot_known_copy(&snapshot.error_kind),
        },
        user_offset_seconds,
    )
}

fn transport_update_on_room_timeline(
    mut update: sorotte_player_api::PlayerTransportTelemetryUpdate,
    user_offset_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    update.position_seconds = update
        .position_seconds
        .map(|position| position - user_offset_seconds);
    update.seekable_ranges = update.seekable_ranges.map(|ranges| {
        ranges
            .into_iter()
            .map(|range| range.shifted(-user_offset_seconds))
            .collect()
    });
    update
}

#[cfg(test)]
mod transport_timeline_tests {
    use super::transport_update_on_room_timeline;
    use sorotte_player_api::{
        PlayerMediaGeneration, PlayerObservationTimestamp, PlayerSeekableRange,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
    };
    use std::time::Duration;

    fn update(phase: PlayerTransportPhase, player_position: f64) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
        )
        .with_phase(phase)
        .with_position_seconds(player_position);
        update.seekable_ranges = Some(vec![PlayerSeekableRange::new(
            player_position - 10.0,
            player_position + 30.0,
        )]);
        update
    }

    #[test]
    fn positive_offset_is_removed_for_barrier_and_normal_sync_observations() {
        let normalized =
            transport_update_on_room_timeline(update(PlayerTransportPhase::ReadyPaused, 15.0), 5.0);
        assert_eq!(normalized.position_seconds, Some(10.0));
        assert_eq!(
            normalized.seekable_ranges,
            Some(vec![PlayerSeekableRange::new(0.0, 40.0)])
        );
    }

    #[test]
    fn negative_offset_is_removed_for_rebuffer_recovery_observations() {
        let normalized =
            transport_update_on_room_timeline(update(PlayerTransportPhase::Rebuffering, 5.0), -5.0);
        assert_eq!(normalized.position_seconds, Some(10.0));
        assert_eq!(
            normalized.seekable_ranges,
            Some(vec![PlayerSeekableRange::new(0.0, 40.0)])
        );
    }
}

#[cfg(test)]
mod ordered_delivery_tests {
    use super::*;
    use crate::app::runtime_stack::GuiSessionRuntimeAdapter;
    use sorotte_player_api::PlayerPhysicalLoadOutcome;
    use sorotte_player_mpv::lifecycle::{
        AuthoritativePlaylistEntry, PlayerLifecycleEffect, PlayerLifecycleInput,
        PlayerLifecycleState, SystemSeekOwnershipState, reduce_player_lifecycle,
    };
    use std::{
        collections::{BTreeSet, VecDeque},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    struct OrderedBatchPlayer {
        batches: VecDeque<PlayerEventBatch>,
        fail_next_ack: bool,
        acknowledgement_calls: Arc<AtomicUsize>,
        legacy_drain_calls: Arc<AtomicUsize>,
    }

    impl PlayerAdapter for OrderedBatchPlayer {
        fn name(&self) -> &'static str {
            "ordered-batch-test"
        }

        fn player_event_delivery_mode(&self) -> PlayerEventDeliveryMode {
            PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        }

        fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
            self.batches.front().cloned()
        }

        fn acknowledge_player_event_batch(
            &mut self,
            token: PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.acknowledgement_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_next_ack {
                self.fail_next_ack = false;
                return Err(sorotte_player_api::PlayerError::OperationFailed(
                    "synthetic acknowledgement failure".to_owned(),
                ));
            }
            let expected = self
                .batches
                .front()
                .map(|batch| batch.acknowledgement_token);
            if expected != Some(token) {
                return Err(sorotte_player_api::PlayerError::OperationFailed(
                    "unexpected acknowledgement token".to_owned(),
                ));
            }
            self.batches.pop_front();
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.legacy_drain_calls.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    struct LifecycleBatchPlayer {
        state: PlayerLifecycleState,
        acknowledged_epochs: Arc<Mutex<Vec<PlayerAttachmentEpoch>>>,
    }

    impl PlayerAdapter for LifecycleBatchPlayer {
        fn name(&self) -> &'static str {
            "lifecycle-batch-test"
        }

        fn player_event_delivery_mode(&self) -> PlayerEventDeliveryMode {
            PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        }

        fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
            let batch = self.state.peek_event_batch()?;
            assert!(
                batch
                    .events
                    .iter()
                    .all(|event| { event.order.attachment_epoch == batch.attachment_epoch })
            );
            assert!(batch.semantic_outcomes.iter().all(|outcome| {
                outcome.order.attachment_epoch == batch.attachment_epoch
                    && match &outcome.outcome {
                        PlayerSemanticOutcome::Command(command) => {
                            command.attachment_epoch == batch.attachment_epoch
                        }
                        PlayerSemanticOutcome::LoadAttempt(attempt) => {
                            attempt.attachment_epoch == batch.attachment_epoch
                        }
                    }
            }));
            Some(batch)
        }

        fn acknowledge_player_event_batch(
            &mut self,
            token: PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            if !self.state.acknowledge_event_batch(token) {
                return Err(sorotte_player_api::PlayerError::OperationFailed(
                    "lifecycle batch acknowledgement failed".to_owned(),
                ));
            }
            self.acknowledged_epochs
                .lock()
                .expect("acknowledgement epoch lock")
                .push(token.attachment_epoch());
            Ok(())
        }
    }

    struct CountingSession {
        transport_updates: Arc<AtomicUsize>,
    }

    impl GuiSessionRuntimeAdapter for CountingSession {
        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(current_servers)
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn sync_attached_player_transport_telemetry(
            &mut self,
            _update: sorotte_player_api::PlayerTransportTelemetryUpdate,
            _now_seconds: f64,
        ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
            self.transport_updates.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    struct MediaBoundaryRecordingSession {
        prepared_media: Arc<AtomicUsize>,
        interrupted_recovery: Arc<AtomicUsize>,
    }

    impl GuiSessionRuntimeAdapter for MediaBoundaryRecordingSession {
        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(current_servers)
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn prepare_attached_playback_media(
            &mut self,
            _logical_id: sorotte_client_core::LogicalMediaId,
            _kind: MediaTransportKind,
            _intent: MediaLoadIntent,
            _now_seconds: f64,
        ) -> Result<Option<sorotte_client_core::MediaLoadPlan>, String> {
            self.prepared_media.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }

        fn interrupt_attached_playback_recovery(
            &mut self,
        ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
            self.interrupted_recovery.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    fn epoch() -> PlayerAttachmentEpoch {
        PlayerAttachmentEpoch::new(1)
    }

    fn active_snapshot(
        through_sequence: u64,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        position_seconds: f64,
    ) -> PlayerAuthoritativeSnapshot {
        let attachment_epoch = epoch();
        let transport = PlayerTransportSnapshot {
            load_attempt_id: SnapshotField::Known(attempt_id),
            media_generation: SnapshotField::Known(media_generation),
            position_seconds: SnapshotField::Known(position_seconds),
            logical_pause: SnapshotField::Known(false),
            paused_for_cache: SnapshotField::Known(false),
            cache_percentage: SnapshotField::Known(100.0),
            playback_rate: SnapshotField::Known(1.0),
            seeking: SnapshotField::Known(false),
            seekable: SnapshotField::Known(true),
            eof_reached: SnapshotField::Known(false),
            error_kind: SnapshotField::KnownAbsent,
            ..PlayerTransportSnapshot::default()
        };
        PlayerAuthoritativeSnapshot {
            attachment_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(attachment_epoch, through_sequence),
            transport,
            active_load: SnapshotField::Known(PlayerActiveLoadSnapshot {
                attempt_id,
                media_generation,
                command_id: Some(PlayerCommandId::new(40)),
                playlist_entry_id: Some(400),
                physical_file_loaded: true,
                semantic_load_result: Some(PlayerLoadAttemptResult::Loaded),
                logical_ownership_revoked: false,
            }),
            current_playlist_entry_id: SnapshotField::Known(400),
            current_path: SnapshotField::Known("synthetic-current.mkv".to_owned()),
        }
    }

    fn delta_event(
        sequence: u64,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        position_seconds: f64,
        paused_for_cache: bool,
    ) -> SequencedPlayerEvent {
        SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch(), sequence),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                load_attempt_id: Some(attempt_id),
                media_generation: Some(media_generation),
                position_seconds: Some(position_seconds),
                paused_for_cache: Some(paused_for_cache),
                cache_percentage: Some(if paused_for_cache { 25.0 } else { 100.0 }),
                ..PlayerTransportDelta::default()
            }),
        }
    }

    fn batch(
        through_sequence: u64,
        token: u64,
        snapshot: Option<PlayerAuthoritativeSnapshot>,
        events: Vec<SequencedPlayerEvent>,
    ) -> PlayerEventBatch {
        PlayerEventBatch {
            attachment_epoch: epoch(),
            sequence_boundary: PlayerSequenceBoundary::new(epoch(), through_sequence),
            authoritative_snapshot: snapshot,
            events,
            semantic_outcomes: Vec::new(),
            acknowledgement_token: PlayerEventAcknowledgementToken::new(epoch(), token),
        }
    }

    fn owner_with_batches(
        batches: Vec<PlayerEventBatch>,
        fail_next_ack: bool,
        acknowledgement_calls: Arc<AtomicUsize>,
        legacy_drain_calls: Arc<AtomicUsize>,
        transport_updates: Arc<AtomicUsize>,
    ) -> GuiPersistedConfigRuntimeOwner {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OrderedBatchPlayer {
            batches: batches.into(),
            fail_next_ack,
            acknowledgement_calls,
            legacy_drain_calls,
        })));
        owner.session = Some(Box::new(CountingSession { transport_updates }));
        owner
    }

    fn reduce_lifecycle(state: &mut PlayerLifecycleState, input: PlayerLifecycleInput) {
        let _ = reduce_lifecycle_with_effects(state, input);
    }

    fn reduce_lifecycle_with_effects(
        state: &mut PlayerLifecycleState,
        input: PlayerLifecycleInput,
    ) -> Vec<PlayerLifecycleEffect> {
        let current = std::mem::take(state);
        let (next, effects) = reduce_player_lifecycle(current, input);
        next.assert_invariants().expect("lifecycle invariants");
        *state = next;
        effects
    }

    fn acknowledge_all_lifecycle_batches(state: &mut PlayerLifecycleState) {
        while let Some(batch) = state.peek_event_batch() {
            assert!(
                state.acknowledge_event_batch(batch.acknowledgement_token),
                "generated lifecycle batch should acknowledge"
            );
        }
    }

    #[test]
    fn repro_acknowledged_local_file_change_prepares_new_logical_media_boundary() {
        let attempt_id = LoadAttemptId::new(19);
        let media_generation = PlayerMediaGeneration::new(2);
        let prepared_media = Arc::new(AtomicUsize::new(0));
        let interrupted_recovery = Arc::new(AtomicUsize::new(0));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player_local_file =
            Some(LocalFileUpdate::new("old.mkv").with_path("C:\\media\\old.mkv"));
        owner.session = Some(Box::new(MediaBoundaryRecordingSession {
            prepared_media: prepared_media.clone(),
            interrupted_recovery: interrupted_recovery.clone(),
        }));
        let event_batch = batch(
            2,
            1,
            None,
            vec![
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 1),
                    event: PlayerEvent::LoadAttemptActive {
                        attempt_id,
                        media_generation,
                        command_id: None,
                        playlist_entry_id: 19,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 2),
                    event: PlayerEvent::LocalFileChanged {
                        attempt_id,
                        media_generation,
                        update: LocalFileUpdate::new("new.mkv").with_path("C:\\media\\new.mkv"),
                    },
                },
            ],
        );

        owner
            .apply_ordered_player_event_batch(&event_batch, 0.0)
            .expect("valid acknowledged player batch");

        assert_eq!(
            prepared_media.load(Ordering::SeqCst),
            1,
            "a confirmed acknowledged local-file boundary must prepare client-core for the new logical media"
        );
        assert_eq!(
            interrupted_recovery.load(Ordering::SeqCst),
            1,
            "a confirmed acknowledged local-file boundary must interrupt recovery for the previous media"
        );
    }

    #[test]
    fn repro_authoritative_snapshot_uses_local_basename_for_player_identity() {
        let attempt_id = LoadAttemptId::new(23);
        let media_generation = PlayerMediaGeneration::new(23);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut snapshot = active_snapshot(0, attempt_id, media_generation, 0.0);
        snapshot.current_path = SnapshotField::Known("C:\\private\\shows\\episode.mkv".to_owned());

        owner
            .apply_ordered_player_event_batch(&batch(0, 1, Some(snapshot), Vec::new()), 0.0)
            .expect("valid authoritative snapshot batch");

        assert_eq!(
            (
                owner
                    .player_local_file
                    .as_ref()
                    .map(|file| file.name.as_str()),
                owner.current_player_matches_media_target("episode.mkv"),
            ),
            (Some("episode.mkv"), true),
            "snapshot recovery must retain a basename display identity that matches a basename-only shared-playlist target"
        );
    }

    #[test]
    fn repro_authoritative_snapshot_media_boundary_prepares_client_core() {
        let attempt_id = LoadAttemptId::new(24);
        let media_generation = PlayerMediaGeneration::new(24);
        let prepared_media = Arc::new(AtomicUsize::new(0));
        let interrupted_recovery = Arc::new(AtomicUsize::new(0));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player_local_file =
            Some(LocalFileUpdate::new("old.mkv").with_path("C:\\media\\old.mkv"));
        owner.session = Some(Box::new(MediaBoundaryRecordingSession {
            prepared_media: prepared_media.clone(),
            interrupted_recovery: interrupted_recovery.clone(),
        }));
        let mut snapshot = active_snapshot(2, attempt_id, media_generation, 0.0);
        snapshot.current_path = SnapshotField::Known("C:\\media\\new.mkv".to_owned());

        owner
            .apply_ordered_player_event_batch(&batch(2, 1, Some(snapshot), Vec::new()), 0.0)
            .expect("valid authoritative reacquisition batch");

        assert_eq!(
            prepared_media.load(Ordering::SeqCst),
            1,
            "a snapshot-only authoritative media boundary must prepare client-core for the newly recovered logical media"
        );
        assert_eq!(
            interrupted_recovery.load(Ordering::SeqCst),
            1,
            "a snapshot-only authoritative media boundary must retire recovery for the previous media"
        );
    }

    #[test]
    fn stale_old_attempt_delta_cannot_replace_authoritative_gui_position() {
        let current_attempt = LoadAttemptId::new(2);
        let current_generation = PlayerMediaGeneration::new(2);
        let stale_attempt = LoadAttemptId::new(1);
        let stale_generation = PlayerMediaGeneration::new(1);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let stale = batch(
            1,
            1,
            Some(active_snapshot(
                0,
                current_attempt,
                current_generation,
                20.0,
            )),
            vec![delta_event(1, stale_attempt, stale_generation, 999.0, true)],
        );

        owner
            .apply_ordered_player_event_batch(&stale, 0.0)
            .expect("structurally valid stale evidence should be consumed");

        assert_eq!(owner.player_position_seconds, Some(20.0));
        assert_eq!(owner.player_paused_for_cache, Some(false));
        assert_eq!(
            snapshot_known_copy(&owner.ordered_player_events.transport.load_attempt_id),
            Some(current_attempt)
        );
    }

    #[test]
    fn gui_accepts_starting_transport_only_for_explicit_physical_owner() {
        for (owns_transport, expected_phase) in
            [(true, Some(PlayerTransportPhase::Loading)), (false, None)]
        {
            let attempt_id = LoadAttemptId::new(1);
            let generation = PlayerMediaGeneration::new(1);
            let acknowledgement_calls = Arc::new(AtomicUsize::new(0));
            let legacy_drain_calls = Arc::new(AtomicUsize::new(0));
            let transport_updates = Arc::new(AtomicUsize::new(0));
            let events = vec![
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 1),
                    event: PlayerEvent::LoadAttemptStarting {
                        attempt_id,
                        media_generation: generation,
                        command_id: Some(PlayerCommandId::new(9)),
                        playlist_entry_id: 10,
                        owns_transport,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 2),
                    event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                        load_attempt_id: Some(attempt_id),
                        media_generation: Some(generation),
                        phase: Some(PlayerTransportPhase::Loading),
                        paused_for_cache: Some(true),
                        cache_percentage: Some(25.0),
                        ..PlayerTransportDelta::default()
                    }),
                },
            ];
            let mut owner = owner_with_batches(
                vec![batch(2, 30 + u64::from(owns_transport), None, events)],
                false,
                acknowledgement_calls.clone(),
                legacy_drain_calls.clone(),
                transport_updates.clone(),
            );

            owner.refresh_player_state_impl();

            assert_eq!(
                owner.ordered_player_events.transport_owner_attempt,
                owns_transport.then_some(attempt_id)
            );
            assert_eq!(
                snapshot_known_copy(&owner.ordered_player_events.transport.phase),
                expected_phase
            );
            assert_eq!(
                snapshot_known_copy(&owner.ordered_player_events.transport.paused_for_cache),
                owns_transport.then_some(true)
            );
            assert_eq!(
                transport_updates.load(Ordering::SeqCst),
                usize::from(owns_transport)
            );
            assert_eq!(acknowledgement_calls.load(Ordering::SeqCst), 1);
            assert_eq!(legacy_drain_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn gui_accepts_same_generation_successor_transport_before_old_terminal() {
        let generation = PlayerMediaGeneration::new(7);
        let predecessor = LoadAttemptId::new(1);
        let successor = LoadAttemptId::new(2);
        let ordered = PlayerEventBatch {
            attachment_epoch: epoch(),
            sequence_boundary: PlayerSequenceBoundary::new(epoch(), 4),
            authoritative_snapshot: None,
            events: vec![
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 1),
                    event: PlayerEvent::LoadAttemptActive {
                        attempt_id: predecessor,
                        media_generation: generation,
                        command_id: None,
                        playlist_entry_id: 10,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 3),
                    event: PlayerEvent::LoadAttemptStarting {
                        attempt_id: successor,
                        media_generation: generation,
                        command_id: None,
                        playlist_entry_id: 20,
                        owns_transport: true,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 4),
                    event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                        load_attempt_id: Some(successor),
                        media_generation: Some(generation),
                        phase: Some(PlayerTransportPhase::Prebuffering),
                        ..PlayerTransportDelta::default()
                    }),
                },
            ],
            semantic_outcomes: vec![SequencedPlayerSemanticOutcome {
                order: PlayerEventOrder::new(epoch(), 2),
                outcome: PlayerSemanticOutcome::LoadAttempt(
                    sorotte_player_api::LoadAttemptOutcome {
                        attachment_epoch: epoch(),
                        attempt_id: predecessor,
                        media_generation: generation,
                        command_id: None,
                        requested_target: "stream".to_owned(),
                        loaded_target: Some("stream".to_owned()),
                        result: PlayerLoadAttemptResult::Superseded,
                    },
                ),
            }],
            acknowledgement_token: PlayerEventAcknowledgementToken::new(epoch(), 32),
        };
        let mut owner = owner_with_batches(
            vec![ordered],
            false,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );

        owner.refresh_player_state_impl();

        assert_eq!(
            owner.ordered_player_events.transport_owner_attempt,
            Some(successor)
        );
        assert_eq!(
            snapshot_known_copy(&owner.ordered_player_events.transport.phase),
            Some(PlayerTransportPhase::Prebuffering)
        );
        assert_eq!(
            owner
                .ordered_player_events
                .attempts
                .get(&predecessor)
                .map(|binding| (binding.logical_ownership_revoked, binding.physical_terminal,)),
            Some((true, false))
        );
    }

    #[test]
    fn authoritative_snapshot_replacement_clears_stale_gui_and_transport_fields() {
        let attempt_id = LoadAttemptId::new(7);
        let media_generation = PlayerMediaGeneration::new(7);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player_local_file =
            Some(LocalFileUpdate::new("stale.mkv").with_path("C:\\synthetic\\stale.mkv"));
        owner.player_position_seconds = Some(99.0);
        owner.player_paused = Some(true);
        owner.player_paused_for_cache = Some(true);
        owner.player_cache_buffering_percent = Some(12.0);
        owner.playlist_auto_advance_eof_latched = true;
        let mut replacement = active_snapshot(0, attempt_id, media_generation, 0.0);
        replacement.current_path = SnapshotField::KnownAbsent;
        replacement.transport.position_seconds = SnapshotField::Unavailable;
        replacement.transport.logical_pause = SnapshotField::KnownAbsent;
        replacement.transport.paused_for_cache = SnapshotField::Unavailable;
        replacement.transport.cache_percentage = SnapshotField::KnownAbsent;
        replacement.transport.playback_rate = SnapshotField::KnownAbsent;
        replacement.transport.seeking = SnapshotField::Unavailable;
        replacement.transport.seekable = SnapshotField::KnownAbsent;
        replacement.transport.eof_reached = SnapshotField::Unavailable;
        replacement.transport.error_kind = SnapshotField::KnownAbsent;

        owner
            .apply_ordered_player_event_batch(&batch(0, 1, Some(replacement), Vec::new()), 0.0)
            .expect("replacement snapshot");

        assert_eq!(owner.player_local_file, None);
        assert_eq!(owner.player_position_seconds, None);
        assert_eq!(owner.player_paused, None);
        assert_eq!(owner.player_paused_for_cache, None);
        assert_eq!(owner.player_cache_buffering_percent, None);
        assert!(!owner.playlist_auto_advance_eof_latched);
        let transport = &owner.ordered_player_events.transport;
        assert_eq!(transport.playback_rate, SnapshotField::KnownAbsent);
        assert_eq!(transport.seeking, SnapshotField::Unavailable);
        assert_eq!(transport.seekable, SnapshotField::KnownAbsent);
        assert_eq!(transport.eof_reached, SnapshotField::Unavailable);
        assert_eq!(transport.error_kind, SnapshotField::KnownAbsent);
    }

    #[test]
    fn unacknowledged_replay_is_idempotent_and_legacy_queues_are_not_drained() {
        let attempt_id = LoadAttemptId::new(4);
        let media_generation = PlayerMediaGeneration::new(4);
        let acknowledgement_calls = Arc::new(AtomicUsize::new(0));
        let legacy_drain_calls = Arc::new(AtomicUsize::new(0));
        let transport_updates = Arc::new(AtomicUsize::new(0));
        let event_batch = batch(
            1,
            1,
            Some(active_snapshot(0, attempt_id, media_generation, 5.0)),
            vec![delta_event(1, attempt_id, media_generation, 7.0, false)],
        );
        let mut owner = owner_with_batches(
            vec![event_batch],
            true,
            acknowledgement_calls.clone(),
            legacy_drain_calls.clone(),
            transport_updates.clone(),
        );

        owner.refresh_player_state_impl();
        assert_eq!(owner.player_position_seconds, Some(7.0));
        assert_eq!(transport_updates.load(Ordering::SeqCst), 2);
        owner.refresh_player_state_impl();
        owner.refresh_player_state_impl();

        assert_eq!(owner.player_position_seconds, Some(7.0));
        assert_eq!(transport_updates.load(Ordering::SeqCst), 2);
        assert_eq!(acknowledgement_calls.load(Ordering::SeqCst), 2);
        assert_eq!(legacy_drain_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            owner.ordered_player_events.applied_unacknowledged_token,
            None
        );
    }

    #[test]
    fn ordered_projection_is_invariant_to_gui_pump_partitioning() {
        let attempt_id = LoadAttemptId::new(9);
        let media_generation = PlayerMediaGeneration::new(9);
        let first = delta_event(1, attempt_id, media_generation, 10.0, false);
        let second = delta_event(2, attempt_id, media_generation, 20.0, true);
        let snapshot = active_snapshot(0, attempt_id, media_generation, 0.0);
        let one_batch = vec![batch(
            2,
            1,
            Some(snapshot.clone()),
            vec![first.clone(), second.clone()],
        )];
        let split_batches = vec![
            batch(1, 1, Some(snapshot), vec![first]),
            batch(2, 2, None, vec![second]),
        ];
        let build = |batches| {
            owner_with_batches(
                batches,
                false,
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            )
        };
        let mut combined = build(one_batch);
        let mut partitioned = build(split_batches);

        combined.refresh_player_state_impl();
        partitioned.refresh_player_state_impl();

        assert_eq!(
            combined.ordered_player_events,
            partitioned.ordered_player_events
        );
        assert_eq!(
            combined.player_position_seconds,
            partitioned.player_position_seconds
        );
        assert_eq!(combined.player_paused, partitioned.player_paused);
        assert_eq!(
            combined.player_paused_for_cache,
            partitioned.player_paused_for_cache
        );
        assert_eq!(
            combined.player_cache_buffering_percent,
            partitioned.player_cache_buffering_percent
        );
        assert_eq!(combined.player_local_file, partitioned.player_local_file);
    }

    #[test]
    fn attachment_replacement_drains_old_terminals_before_new_epoch_snapshot() {
        let mut state = PlayerLifecycleState::default();
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(1)),
                media_generation: PlayerMediaGeneration::new(1),
                requested_target: "old-core.mkv".to_owned(),
                baseline_playlist_entry_ids: BTreeSet::new(),
            },
        );
        let old_attempt_id = state
            .attempt_for_command(PlayerCommandId::new(1))
            .expect("old load attempt");
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                attempt_id: old_attempt_id,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: vec![AuthoritativePlaylistEntry::new(
                    77,
                    Some("old-core.mkv".to_owned()),
                    true,
                )],
                current_path: Some("old-core.mkv".to_owned()),
            },
        );
        let seek_dispatch_boundary = state.last_event_sequence();
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::SeekCommandSubmitted {
                command_id: PlayerCommandId::new(2),
                media_generation: PlayerMediaGeneration::new(1),
                raw_player_target_seconds: 12.0,
                effective_room_target_seconds: 12.0,
                dispatch_sequence_boundary: seek_dispatch_boundary,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::SeekCommandAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id: PlayerCommandId::new(2),
            },
        );
        reduce_lifecycle(&mut state, PlayerLifecycleInput::AttachmentReplaced);
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(2),
                media_generation: PlayerMediaGeneration::new(2),
                playlist_entry_id: 77,
                observed_target: "new-core.mkv".to_owned(),
                file_loaded: true,
            },
        );
        let new_attempt_id = state.active_load_attempt.expect("new active attempt");
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(2),
                sequence_boundary: PlayerSequenceBoundary::new(PlayerAttachmentEpoch::new(2), 0),
                transport: PlayerTransportSnapshot {
                    load_attempt_id: SnapshotField::Known(new_attempt_id),
                    media_generation: SnapshotField::Known(PlayerMediaGeneration::new(2)),
                    phase: SnapshotField::Known(PlayerTransportPhase::ReadyPaused),
                    logical_pause: SnapshotField::Known(true),
                    ..PlayerTransportSnapshot::default()
                },
                active_load: SnapshotField::Known(PlayerActiveLoadSnapshot {
                    attempt_id: new_attempt_id,
                    media_generation: PlayerMediaGeneration::new(2),
                    command_id: None,
                    playlist_entry_id: Some(77),
                    physical_file_loaded: true,
                    semantic_load_result: Some(PlayerLoadAttemptResult::Loaded),
                    logical_ownership_revoked: false,
                }),
                current_playlist_entry_id: SnapshotField::Known(77),
                current_path: SnapshotField::Known("new-core.mkv".to_owned()),
            }),
        );

        let old_batch = state.peek_event_batch().expect("old handoff batch");
        assert_eq!(old_batch.attachment_epoch, PlayerAttachmentEpoch::new(1));
        let old_command_disconnects = old_batch
            .semantic_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    &outcome.outcome,
                    PlayerSemanticOutcome::Command(command)
                        if command.result
                            == PlayerCommandSemanticResult::TransportDisconnected
                )
            })
            .count();
        let old_attempt_disconnects = old_batch
            .semantic_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    &outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(attempt)
                        if attempt.result == PlayerLoadAttemptResult::TransportDisconnected
                )
            })
            .count();
        assert_eq!(old_command_disconnects, 2);
        assert_eq!(old_attempt_disconnects, 1);

        let acknowledged_epochs = Arc::new(Mutex::new(Vec::new()));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(LifecycleBatchPlayer {
            state,
            acknowledged_epochs: acknowledged_epochs.clone(),
        })));
        owner.session = Some(Box::new(CountingSession {
            transport_updates: Arc::new(AtomicUsize::new(0)),
        }));

        owner.refresh_player_state_impl();
        assert_eq!(
            *acknowledged_epochs
                .lock()
                .expect("acknowledgement epoch lock"),
            vec![PlayerAttachmentEpoch::new(1), PlayerAttachmentEpoch::new(2)]
        );
        assert_eq!(
            owner.ordered_player_events.attachment_epoch,
            Some(PlayerAttachmentEpoch::new(2))
        );
        assert_eq!(
            owner.ordered_player_events.transport_owner_attempt,
            Some(new_attempt_id)
        );
        assert_eq!(
            owner
                .ordered_player_events
                .attempts
                .get(&new_attempt_id)
                .map(|binding| (binding.media_generation, binding.playlist_entry_id)),
            Some((PlayerMediaGeneration::new(2), Some(77)))
        );
        assert!(
            !owner
                .ordered_player_events
                .attempts
                .contains_key(&old_attempt_id)
        );
        assert_eq!(
            snapshot_known_clone(&owner.ordered_player_events.transport.phase),
            Some(PlayerTransportPhase::ReadyPaused)
        );
        assert!(
            owner
                .ordered_player_events
                .applied_semantic_outcomes
                .is_empty(),
            "successful acknowledgements must compact replay-only semantic identity"
        );
    }

    #[test]
    fn gui_consumer_never_reactivates_predecessor_after_successor_compaction() {
        let mut state = PlayerLifecycleState::default();
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(1)),
                media_generation: PlayerMediaGeneration::new(1),
                requested_target: "A".to_owned(),
                baseline_playlist_entry_ids: BTreeSet::new(),
            },
        );
        let predecessor = state
            .attempt_for_command(PlayerCommandId::new(1))
            .expect("predecessor");
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch(),
                attempt_id: predecessor,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(2)),
                media_generation: PlayerMediaGeneration::new(2),
                requested_target: "B".to_owned(),
                baseline_playlist_entry_ids: BTreeSet::new(),
            },
        );
        let successor = state
            .attempt_for_command(PlayerCommandId::new(2))
            .expect("successor");
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch(),
                attempt_id: successor,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: epoch(),
                entries: vec![AuthoritativePlaylistEntry::new(
                    20,
                    Some("B".to_owned()),
                    true,
                )],
                current_path: Some("B".to_owned()),
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: epoch(),
                playlist_entry_id: Some(20),
                loaded_target: Some("B".to_owned()),
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch(),
                playlist_entry_id: 20,
                outcome: PlayerPhysicalLoadOutcome::Ended,
            },
        );
        let successor_batch = state.peek_event_batch().expect("successor batch");
        assert!(
            state.acknowledge_event_batch(successor_batch.acknowledgement_token),
            "successor batch should compact"
        );
        assert!(!state.load_attempts.contains_key(&successor));
        assert!(state.load_attempts[&predecessor].logical_ownership_revoked);

        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: epoch(),
                entries: vec![AuthoritativePlaylistEntry::new(
                    10,
                    Some("A".to_owned()),
                    false,
                )],
                current_path: None,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::StartFile {
                attachment_epoch: epoch(),
                playlist_entry_id: 10,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: epoch(),
                playlist_entry_id: Some(10),
                loaded_target: Some("A".to_owned()),
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch(),
                playlist_entry_id: 10,
                outcome: PlayerPhysicalLoadOutcome::Ended,
            },
        );
        let late_predecessor_batch = state.peek_event_batch().expect("late predecessor batch");
        assert!(late_predecessor_batch.events.iter().all(|event| {
            !matches!(
                event.event,
                PlayerEvent::LoadAttemptStarting { attempt_id, .. }
                    | PlayerEvent::LoadAttemptActive { attempt_id, .. }
                    | PlayerEvent::LogicalPlaybackTerminal { attempt_id, .. }
                    if attempt_id == predecessor
            )
        }));

        let acknowledgement_calls = Arc::new(AtomicUsize::new(0));
        let legacy_drain_calls = Arc::new(AtomicUsize::new(0));
        let transport_updates = Arc::new(AtomicUsize::new(0));
        let mut owner = owner_with_batches(
            vec![successor_batch, late_predecessor_batch],
            false,
            acknowledgement_calls.clone(),
            legacy_drain_calls.clone(),
            transport_updates,
        );
        owner.refresh_player_state_impl();

        assert_eq!(acknowledgement_calls.load(Ordering::SeqCst), 2);
        assert_eq!(legacy_drain_calls.load(Ordering::SeqCst), 0);
        assert_eq!(owner.ordered_player_events.transport_owner_attempt, None);
        assert!(
            !owner
                .ordered_player_events
                .attempts
                .contains_key(&predecessor)
        );
    }

    #[test]
    fn gui_accepts_late_active_event_after_indeterminate_load_deadline() {
        let mut state = PlayerLifecycleState::default();
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(1)),
                media_generation: PlayerMediaGeneration::new(1),
                requested_target: "late-load.mkv".to_owned(),
                baseline_playlist_entry_ids: BTreeSet::new(),
            },
        );
        let attempt_id = state
            .attempt_for_command(PlayerCommandId::new(1))
            .expect("load attempt");
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch(),
                attempt_id,
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: epoch(),
                entries: vec![AuthoritativePlaylistEntry::new(
                    10,
                    Some("late-load.mkv".to_owned()),
                    true,
                )],
                current_path: Some("late-load.mkv".to_owned()),
            },
        );
        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::TimerAdvanced { now_tick: 60_000 },
        );
        assert_eq!(
            state.active_load_attempt, None,
            "quiescent ownership must not appear active in a snapshot"
        );
        assert!(state.peek_event_batch().is_some_and(|batch| {
            batch.semantic_outcomes.iter().any(|outcome| {
                matches!(
                    outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(ref load)
                        if load.attempt_id == attempt_id
                            && load.result == PlayerLoadAttemptResult::Indeterminate
                )
            })
        }));

        reduce_lifecycle(
            &mut state,
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: epoch(),
                playlist_entry_id: Some(10),
                loaded_target: Some("late-load.mkv".to_owned()),
            },
        );
        assert_eq!(state.active_load_attempt, Some(attempt_id));

        let acknowledged_epochs = Arc::new(Mutex::new(Vec::new()));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(LifecycleBatchPlayer {
            state,
            acknowledged_epochs: acknowledged_epochs.clone(),
        })));
        owner.session = Some(Box::new(CountingSession {
            transport_updates: Arc::new(AtomicUsize::new(0)),
        }));

        owner.refresh_player_state_impl();

        assert_eq!(
            *acknowledged_epochs
                .lock()
                .expect("acknowledgement epoch lock"),
            vec![epoch(), epoch()]
        );
        assert_eq!(
            owner.ordered_player_events.transport_owner_attempt,
            Some(attempt_id)
        );
        assert_eq!(
            owner
                .ordered_player_events
                .attempts
                .get(&attempt_id)
                .map(|binding| binding.physical_terminal),
            Some(false)
        );
    }

    #[test]
    fn gui_ignores_loaded_success_for_an_attempt_that_is_not_currently_owned() {
        let predecessor = LoadAttemptId::new(1);
        let successor = LoadAttemptId::new(2);
        let predecessor_generation = PlayerMediaGeneration::new(1);
        let successor_generation = PlayerMediaGeneration::new(2);
        let batch = PlayerEventBatch {
            attachment_epoch: epoch(),
            sequence_boundary: PlayerSequenceBoundary::new(epoch(), 3),
            authoritative_snapshot: None,
            events: vec![
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 1),
                    event: PlayerEvent::LoadAttemptBound {
                        attempt_id: predecessor,
                        media_generation: predecessor_generation,
                        command_id: None,
                        playlist_entry_id: 10,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch(), 2),
                    event: PlayerEvent::LoadAttemptActive {
                        attempt_id: successor,
                        media_generation: successor_generation,
                        command_id: None,
                        playlist_entry_id: 20,
                    },
                },
            ],
            semantic_outcomes: vec![SequencedPlayerSemanticOutcome {
                order: PlayerEventOrder::new(epoch(), 3),
                outcome: PlayerSemanticOutcome::LoadAttempt(
                    sorotte_player_api::LoadAttemptOutcome {
                        attachment_epoch: epoch(),
                        attempt_id: predecessor,
                        media_generation: predecessor_generation,
                        command_id: None,
                        requested_target: "A".to_owned(),
                        loaded_target: Some("A".to_owned()),
                        result: PlayerLoadAttemptResult::Loaded,
                    },
                ),
            }],
            acknowledgement_token: PlayerEventAcknowledgementToken::new(epoch(), 1),
        };
        let acknowledgement_calls = Arc::new(AtomicUsize::new(0));
        let legacy_drain_calls = Arc::new(AtomicUsize::new(0));
        let mut owner = owner_with_batches(
            vec![batch],
            false,
            acknowledgement_calls.clone(),
            legacy_drain_calls.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        owner.player_local_file = Some(LocalFileUpdate::new("A").with_path("A"));
        owner.player_local_file_placeholder = true;

        owner.refresh_player_state_impl();

        assert_eq!(
            owner.ordered_player_events.transport_owner_attempt,
            Some(successor)
        );
        assert!(
            owner.player_local_file_placeholder,
            "a stale Loaded outcome must not confirm the predecessor's placeholder"
        );
        assert_eq!(acknowledgement_calls.load(Ordering::SeqCst), 1);
        assert_eq!(legacy_drain_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn generated_ordered_seek_histories_keep_reducer_and_gui_legacy_decisions_equal() {
        const SEEDS: [u64; 4] = [0x00dd_5eed, 0xc0ff_ee42, 0xdec0_de01, 0x51a7_e123];
        const GENERATION: PlayerMediaGeneration = PlayerMediaGeneration::new(7);

        fn next_random(random: &mut u64) -> u64 {
            *random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *random
        }

        for seed in SEEDS {
            let mut random = seed;
            let mut state = PlayerLifecycleState::default();
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
            owner.player_attachment_epoch = state.attachment_epoch.get();
            owner.attached_native_seek_tracker.media_generation = Some(GENERATION.get());
            let mut next_command = 1_u64;
            let mut observed_sequence = 0_u64;

            for step in 0..1_024_u64 {
                let choice = next_random(&mut random) % 8;
                match choice {
                    0 | 1 => {
                        let command_id = PlayerCommandId::new(next_command);
                        next_command += 1;
                        let raw_target = (next_random(&mut random) % 120) as f64 + 0.25;
                        let room_target =
                            raw_target + ((next_random(&mut random) % 21) as f64 - 10.0);

                        reduce_lifecycle(
                            &mut state,
                            PlayerLifecycleInput::SeekCommandSubmitted {
                                command_id,
                                media_generation: GENERATION,
                                raw_player_target_seconds: raw_target,
                                effective_room_target_seconds: room_target,
                                dispatch_sequence_boundary: observed_sequence,
                            },
                        );
                        owner.note_attached_runtime_position_dispatched(
                            Some(command_id),
                            room_target,
                            raw_target,
                        );
                        let attachment_epoch = state.attachment_epoch;
                        reduce_lifecycle(
                            &mut state,
                            PlayerLifecycleInput::SeekCommandAccepted {
                                attachment_epoch,
                                command_id,
                            },
                        );
                        owner.reconcile_attached_system_seek_command_progress(
                            PlayerCommandProgress::accepted(command_id, Some(GENERATION), None),
                        );
                    }
                    2 => {
                        if let Some(command_id) =
                            state.seek_ownership.values().find_map(|ownership| {
                                matches!(
                                    ownership.state,
                                    SystemSeekOwnershipState::Accepted
                                        | SystemSeekOwnershipState::Submitted
                                )
                                .then_some(ownership.command_id)
                            })
                        {
                            let attachment_epoch = state.attachment_epoch;
                            reduce_lifecycle(
                                &mut state,
                                PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                                    attachment_epoch,
                                    command_id,
                                },
                            );
                            owner.reconcile_attached_system_seek_command_progress(
                                PlayerCommandProgress::finished(
                                    command_id,
                                    Some(GENERATION),
                                    None,
                                    None,
                                    PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
                                ),
                            );
                        }
                    }
                    3..=6 => {
                        let expected_gui_command = owner
                            .attached_system_seek_ownership
                            .front()
                            .map(|ownership| ownership.adapter_player_command_id);
                        let position_seconds =
                            owner.attached_system_seek_ownership.front().map_or_else(
                                || (next_random(&mut random) % 120) as f64 + 0.75,
                                |ownership| ownership.player_target_position_seconds,
                            );

                        observed_sequence += 1;
                        let attachment_epoch = state.attachment_epoch;
                        let seeking_effects = reduce_lifecycle_with_effects(
                            &mut state,
                            PlayerLifecycleInput::SeekingObserved {
                                attachment_epoch,
                                media_generation: GENERATION,
                                observed_sequence,
                                seeking: true,
                            },
                        );
                        assert!(
                            seeking_effects.iter().all(|effect| !matches!(
                                effect,
                                PlayerLifecycleEffect::ConsumeSystemSeek { .. }
                                    | PlayerLifecycleEffect::NativeSeekCandidate { .. }
                            )),
                            "seeking evidence alone must not classify a seek"
                        );

                        observed_sequence += 1;
                        let attachment_epoch = state.attachment_epoch;
                        let reducer_effects = reduce_lifecycle_with_effects(
                            &mut state,
                            PlayerLifecycleInput::PositionObserved {
                                attachment_epoch,
                                media_generation: GENERATION,
                                observed_sequence,
                                position_seconds,
                            },
                        );
                        let reducer_system_command =
                            reducer_effects.iter().find_map(|effect| match effect {
                                PlayerLifecycleEffect::ConsumeSystemSeek { command_id, .. } => {
                                    Some(*command_id)
                                }
                                _ => None,
                            });
                        let reducer_native = reducer_effects.iter().any(|effect| {
                            matches!(effect, PlayerLifecycleEffect::NativeSeekCandidate { .. })
                        });
                        assert!(
                            reducer_system_command.is_none() || !reducer_native,
                            "one position cannot be both a system and native seek"
                        );

                        let observed_at = Some(PlayerObservationTimestamp::from_adapter_start(
                            Duration::from_millis(observed_sequence.saturating_mul(10)),
                        ));
                        let gui_system = owner.consume_matching_attached_system_seek(
                            Some(GENERATION),
                            observed_at,
                            position_seconds,
                        );
                        let gui_fail_closed = owner
                            .attached_system_seek_classification_is_fail_closed(Some(GENERATION));
                        let gui_native = !gui_system && !gui_fail_closed;

                        assert_eq!(
                            reducer_system_command.is_some(),
                            gui_system,
                            "system-seek divergence for seed {seed:#x}, step {step}, \
                             position {position_seconds}"
                        );
                        assert_eq!(
                            reducer_system_command.map(Some),
                            expected_gui_command.filter(|_| gui_system),
                            "system-seek owner divergence for seed {seed:#x}, step {step}"
                        );
                        assert_eq!(
                            reducer_native, gui_native,
                            "native-seek divergence for seed {seed:#x}, step {step}, \
                             position {position_seconds}"
                        );
                    }
                    _ => {
                        reduce_lifecycle(&mut state, PlayerLifecycleInput::AttachmentReplaced);
                        owner.player_attachment_epoch = state.attachment_epoch.get();
                        owner.reset_attached_media_boundary_state();
                        owner.attached_native_seek_tracker.media_generation =
                            Some(GENERATION.get());
                    }
                }

                acknowledge_all_lifecycle_batches(&mut state);
                assert!(
                    owner.attached_system_seek_ownership.len()
                        <= ATTACHED_SYSTEM_SEEK_OWNERSHIP_LIMIT,
                    "generated GUI ownership must remain bounded"
                );
            }
        }
    }
}
