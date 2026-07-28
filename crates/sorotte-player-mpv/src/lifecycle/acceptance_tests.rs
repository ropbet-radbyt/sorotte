use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sorotte_player_api::{
    PlayerActiveLoadSnapshot, PlayerCommandSemanticResult, PlayerLoadAttemptResult,
    PlayerMediaLoadFailureKind, PlayerTransportSnapshot, SnapshotField,
};

use super::*;

const PARTITION_SEEDS: [u64; 4] = [0x5eed_cafe, 0x1bad_b002, 0x0d15_ca11, 0xa11c_e55e];
const HISTORY_SEEDS: [u64; 4] = [0x00dd_5eed, 0xc0ff_ee42, 0xdec0_de01, 0x51a7_e123];

fn baseline(ids: &[i64]) -> BTreeSet<i64> {
    ids.iter().copied().collect()
}

fn reduce_checked(
    state: &mut PlayerLifecycleState,
    input: PlayerLifecycleInput,
) -> Vec<PlayerLifecycleEffect> {
    let current = std::mem::take(state);
    let (next, effects) = reduce_player_lifecycle(current, input);
    next.assert_invariants()
        .unwrap_or_else(|error| panic!("{error}; state={next:#?}"));
    *state = next;
    effects
}

fn submit_attempt(
    state: &mut PlayerLifecycleState,
    command_id: Option<u64>,
    generation: u64,
    target: &str,
    baseline_ids: &[i64],
) -> LoadAttemptId {
    let effects = reduce_checked(
        state,
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: command_id.map(PlayerCommandId::new),
            media_generation: PlayerMediaGeneration::new(generation),
            requested_target: target.to_owned(),
            baseline_playlist_entry_ids: baseline(baseline_ids),
        },
    );
    effects
        .into_iter()
        .find_map(|effect| match effect {
            PlayerLifecycleEffect::LoadAttemptAllocated { attempt_id, .. } => Some(attempt_id),
            _ => None,
        })
        .expect("submission should allocate one attempt")
}

fn accept_attempt(state: &mut PlayerLifecycleState, attempt_id: LoadAttemptId) {
    let epoch = state.attachment_epoch;
    reduce_checked(
        state,
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: epoch,
            attempt_id,
        },
    );
}

fn playlist_entry(id: i64, target: &str, current: bool) -> AuthoritativePlaylistEntry {
    AuthoritativePlaylistEntry::new(id, Some(target.to_owned()), current)
}

fn reconcile(
    state: &mut PlayerLifecycleState,
    entries: Vec<AuthoritativePlaylistEntry>,
    current_path: Option<&str>,
) {
    let epoch = state.attachment_epoch;
    reduce_checked(
        state,
        PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries,
            current_path: current_path.map(str::to_owned),
        },
    );
}

fn load_active(
    state: &mut PlayerLifecycleState,
    command_id: u64,
    generation: u64,
    target: &str,
    playlist_entry_id: i64,
) -> LoadAttemptId {
    let attempt = submit_attempt(state, Some(command_id), generation, target, &[]);
    accept_attempt(state, attempt);
    reconcile(
        state,
        vec![playlist_entry(playlist_entry_id, target, true)],
        Some(target),
    );
    let epoch = state.attachment_epoch;
    reduce_checked(
        state,
        PlayerLifecycleInput::StartFile {
            attachment_epoch: epoch,
            playlist_entry_id,
        },
    );
    reduce_checked(
        state,
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch: epoch,
            playlist_entry_id: Some(playlist_entry_id),
            loaded_target: Some(target.to_owned()),
        },
    );
    attempt
}

fn authoritative_snapshot(
    state: &PlayerLifecycleState,
    generation: Option<PlayerMediaGeneration>,
    position_seconds: Option<f64>,
) -> PlayerAuthoritativeSnapshot {
    let active = state.active_attempt();
    let mut transport = PlayerTransportSnapshot {
        media_generation: generation.map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
        phase: active.map_or(SnapshotField::Known(PlayerTransportPhase::Empty), |_| {
            SnapshotField::Known(PlayerTransportPhase::Playing)
        }),
        position_seconds: position_seconds.map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
        playback_rate: SnapshotField::Known(1.0),
        logical_pause: SnapshotField::Known(false),
        paused_for_cache: SnapshotField::Known(false),
        seeking: SnapshotField::Known(false),
        core_idle: SnapshotField::Known(active.is_none()),
        eof_reached: SnapshotField::Known(false),
        error_kind: SnapshotField::KnownAbsent,
        ..PlayerTransportSnapshot::default()
    };
    if generation.is_none() {
        transport.observed_at = SnapshotField::KnownAbsent;
    }
    PlayerAuthoritativeSnapshot {
        attachment_epoch: state.attachment_epoch,
        sequence_boundary: PlayerSequenceBoundary::new(
            state.attachment_epoch,
            state.last_event_sequence(),
        ),
        active_load: active.map_or(SnapshotField::KnownAbsent, |attempt| {
            SnapshotField::Known(PlayerActiveLoadSnapshot {
                attempt_id: attempt.id,
                media_generation: attempt.media_generation,
                command_id: attempt.command_id,
                playlist_entry_id: attempt.playlist_entry_id,
                physical_file_loaded: attempt.physical_file_loaded(),
                semantic_load_result: attempt.semantic_load_result,
                logical_ownership_revoked: attempt.logical_ownership_revoked,
            })
        }),
        current_playlist_entry_id: active
            .and_then(|attempt| attempt.playlist_entry_id)
            .map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
        current_path: active.map_or(SnapshotField::KnownAbsent, |attempt| {
            SnapshotField::Known(attempt.requested_target.clone())
        }),
        transport,
    }
}

#[derive(Debug, Clone)]
enum SimulatedPlayerEvent {
    Input(Box<PlayerLifecycleInput>),
    StartFile {
        playlist_entry_id: i64,
    },
    FileLoaded {
        playlist_entry_id: Option<i64>,
        target: Option<String>,
    },
    EndFile {
        playlist_entry_id: i64,
        outcome: PlayerPhysicalLoadOutcome,
    },
    Position {
        generation: PlayerMediaGeneration,
        observed_sequence: u64,
        position_seconds: f64,
    },
    Seeking {
        generation: PlayerMediaGeneration,
        observed_sequence: u64,
        seeking: bool,
    },
    Eof {
        playlist_entry_id: Option<i64>,
        reached: bool,
        position_seconds: Option<f64>,
    },
    PlaybackRestart {
        playlist_entry_id: Option<i64>,
    },
    Gap,
    Snapshot {
        generation: Option<PlayerMediaGeneration>,
        position_seconds: Option<f64>,
    },
    Timer(u64),
    Disconnect,
    Reattach,
}

impl SimulatedPlayerEvent {
    fn input(input: PlayerLifecycleInput) -> Self {
        Self::Input(Box::new(input))
    }

    fn into_input(self, state: &PlayerLifecycleState) -> PlayerLifecycleInput {
        let attachment_epoch = state.attachment_epoch;
        match self {
            Self::Input(input) => *input,
            Self::StartFile { playlist_entry_id } => PlayerLifecycleInput::StartFile {
                attachment_epoch,
                playlist_entry_id,
            },
            Self::FileLoaded {
                playlist_entry_id,
                target,
            } => PlayerLifecycleInput::FileLoaded {
                attachment_epoch,
                playlist_entry_id,
                loaded_target: target,
            },
            Self::EndFile {
                playlist_entry_id,
                outcome,
            } => PlayerLifecycleInput::EndFile {
                attachment_epoch,
                playlist_entry_id,
                outcome,
            },
            Self::Position {
                generation,
                observed_sequence,
                position_seconds,
            } => PlayerLifecycleInput::PositionObserved {
                attachment_epoch,
                media_generation: generation,
                observed_sequence,
                position_seconds,
            },
            Self::Seeking {
                generation,
                observed_sequence,
                seeking,
            } => PlayerLifecycleInput::SeekingObserved {
                attachment_epoch,
                media_generation: generation,
                observed_sequence,
                seeking,
            },
            Self::Eof {
                playlist_entry_id,
                reached,
                position_seconds,
            } => PlayerLifecycleInput::EofObserved {
                attachment_epoch,
                playlist_entry_id,
                reached,
                position_seconds,
            },
            Self::PlaybackRestart { playlist_entry_id } => PlayerLifecycleInput::PlaybackRestart {
                attachment_epoch,
                playlist_entry_id,
            },
            Self::Gap => PlayerLifecycleInput::EventGapDetected { attachment_epoch },
            Self::Snapshot {
                generation,
                position_seconds,
            } => PlayerLifecycleInput::AuthoritativeSnapshotApplied(authoritative_snapshot(
                state,
                generation,
                position_seconds,
            )),
            Self::Timer(now_tick) => PlayerLifecycleInput::TimerAdvanced { now_tick },
            Self::Disconnect => PlayerLifecycleInput::TransportDisconnected { attachment_epoch },
            Self::Reattach => PlayerLifecycleInput::AttachmentReplaced,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct ConsumerState {
    attachment_epoch: PlayerAttachmentEpoch,
    snapshot_boundaries: BTreeMap<PlayerAttachmentEpoch, u64>,
    seen: BTreeSet<(PlayerAttachmentEpoch, u64)>,
    position_seconds: Option<f64>,
    active_attempt: Option<LoadAttemptId>,
    logical_terminal: Option<(PlayerMediaGeneration, PlayerPhysicalLoadOutcome)>,
    command_outcomes:
        BTreeMap<(PlayerAttachmentEpoch, PlayerCommandId), PlayerCommandSemanticResult>,
    load_outcomes: BTreeMap<(PlayerAttachmentEpoch, LoadAttemptId), PlayerLoadAttemptResult>,
}

#[derive(Debug, Clone)]
enum BatchItem {
    Event(SequencedPlayerEvent),
    Outcome(SequencedPlayerSemanticOutcome),
}

impl BatchItem {
    fn order(&self) -> PlayerEventOrder {
        match self {
            Self::Event(event) => event.order,
            Self::Outcome(outcome) => outcome.order,
        }
    }
}

impl ConsumerState {
    fn apply_batch(&mut self, batch: &PlayerEventBatch) {
        assert_eq!(
            batch.sequence_boundary.attachment_epoch, batch.attachment_epoch,
            "batch boundary must belong to its header epoch"
        );
        assert_eq!(
            batch.acknowledgement_token.attachment_epoch(),
            batch.attachment_epoch,
            "batch token must belong to its header epoch"
        );
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
        assert!(
            batch
                .authoritative_snapshot
                .as_ref()
                .is_none_or(|snapshot| {
                    snapshot.attachment_epoch == batch.attachment_epoch
                        && snapshot.sequence_boundary.attachment_epoch == batch.attachment_epoch
                })
        );
        if let Some(snapshot) = &batch.authoritative_snapshot {
            self.attachment_epoch = snapshot.attachment_epoch;
            self.snapshot_boundaries.insert(
                snapshot.attachment_epoch,
                snapshot.sequence_boundary.through_sequence,
            );
            self.position_seconds = match snapshot.transport.position_seconds {
                SnapshotField::Known(position) => Some(position),
                SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
            };
            self.active_attempt = match snapshot.active_load {
                SnapshotField::Known(active) => Some(active.attempt_id),
                SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
            };
        }

        let mut items = batch
            .events
            .iter()
            .cloned()
            .map(BatchItem::Event)
            .chain(
                batch
                    .semantic_outcomes
                    .iter()
                    .cloned()
                    .map(BatchItem::Outcome),
            )
            .collect::<Vec<_>>();
        items.sort_by_key(|item| {
            let order = item.order();
            (order.attachment_epoch, order.sequence)
        });
        for item in items {
            let order = item.order();
            if matches!(item, BatchItem::Event(_))
                && self
                    .snapshot_boundaries
                    .get(&order.attachment_epoch)
                    .is_some_and(|boundary| order.sequence <= *boundary)
            {
                continue;
            }
            if !self.seen.insert((order.attachment_epoch, order.sequence)) {
                continue;
            }
            match item {
                BatchItem::Event(event) => self.apply_event(event),
                BatchItem::Outcome(outcome) => self.apply_outcome(outcome),
            }
        }
    }

    fn apply_event(&mut self, event: SequencedPlayerEvent) {
        self.attachment_epoch = event.order.attachment_epoch;
        match event.event {
            PlayerEvent::AttachmentReplaced { .. } => {
                self.active_attempt = None;
                self.logical_terminal = None;
                self.position_seconds = None;
            }
            PlayerEvent::TransportDelta(delta) => {
                if let Some(position) = delta.position_seconds {
                    self.position_seconds = Some(position);
                }
            }
            PlayerEvent::LoadAttemptActive { attempt_id, .. }
            | PlayerEvent::LoadAttemptStarting { attempt_id, .. } => {
                self.active_attempt = Some(attempt_id);
                self.logical_terminal = None;
            }
            PlayerEvent::LoadAttemptTerminal { attempt_id, .. } => {
                if self.active_attempt == Some(attempt_id) {
                    self.active_attempt = None;
                }
            }
            PlayerEvent::LogicalPlaybackTerminal {
                media_generation,
                outcome,
                ..
            } => {
                self.active_attempt = None;
                self.logical_terminal = Some((media_generation, outcome));
            }
            PlayerEvent::LocalFileChanged { .. }
            | PlayerEvent::LoadAttemptBound { .. }
            | PlayerEvent::LoadAttemptLogicalOwnershipRevoked { .. }
            | PlayerEvent::EventGapDetected => {}
        }
    }

    fn apply_outcome(&mut self, outcome: SequencedPlayerSemanticOutcome) {
        match outcome.outcome {
            PlayerSemanticOutcome::Command(command) => {
                let key = (command.attachment_epoch, command.command_id);
                if let Some(existing) = self.command_outcomes.insert(key, command.result) {
                    assert_eq!(existing, command.result);
                }
            }
            PlayerSemanticOutcome::LoadAttempt(attempt) => {
                let key = (attempt.attachment_epoch, attempt.attempt_id);
                if let Some(existing) = self.load_outcomes.insert(key, attempt.result) {
                    assert_eq!(existing, attempt.result);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EffectDigest {
    system_seeks: usize,
    native_seeks: usize,
    logical_terminals: usize,
}

impl EffectDigest {
    fn observe(&mut self, effects: &[PlayerLifecycleEffect]) {
        for effect in effects {
            match effect {
                PlayerLifecycleEffect::ConsumeSystemSeek { .. } => self.system_seeks += 1,
                PlayerLifecycleEffect::NativeSeekCandidate { .. } => self.native_seeks += 1,
                PlayerLifecycleEffect::LogicalPlaybackTerminal { .. } => {
                    self.logical_terminals += 1;
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
struct DeterministicScheduler {
    state: PlayerLifecycleState,
    delayed: VecDeque<SimulatedPlayerEvent>,
    consumer: ConsumerState,
    effects: EffectDigest,
}

impl DeterministicScheduler {
    fn new(history: impl IntoIterator<Item = SimulatedPlayerEvent>) -> Self {
        Self {
            state: PlayerLifecycleState::default(),
            delayed: history.into_iter().collect(),
            consumer: ConsumerState::default(),
            effects: EffectDigest::default(),
        }
    }

    fn deliver_at(&mut self, index: usize) {
        let event = self
            .delayed
            .remove(index)
            .expect("scheduled event index should exist");
        let input = event.into_input(&self.state);
        let effects = reduce_checked(&mut self.state, input);
        self.effects.observe(&effects);
    }

    fn deliver_next(&mut self) {
        self.deliver_at(0);
    }

    fn duplicate_at(&mut self, index: usize) {
        let event = self
            .delayed
            .get(index)
            .expect("scheduled event index should exist")
            .clone();
        let input = event.clone().into_input(&self.state);
        let effects = reduce_checked(&mut self.state, input.clone());
        self.effects.observe(&effects);
        let effects = reduce_checked(&mut self.state, input);
        self.effects.observe(&effects);
    }

    fn drop_at(&mut self, index: usize) {
        self.delayed
            .remove(index)
            .expect("scheduled event index should exist");
    }

    fn pump(&mut self, count: usize) {
        for _ in 0..count.min(self.delayed.len()) {
            self.deliver_next();
        }
        self.apply_and_acknowledge_batch();
    }

    fn apply_and_acknowledge_batch(&mut self) {
        let Some(batch) = self.state.peek_event_batch() else {
            return;
        };
        assert_eq!(
            self.state.peek_event_batch(),
            Some(batch.clone()),
            "an unacknowledged batch must replay byte-for-byte"
        );
        self.consumer.apply_batch(&batch);
        let applied_once = self.consumer.clone();
        self.consumer.apply_batch(&batch);
        assert_eq!(
            self.consumer, applied_once,
            "replaying an unacknowledged batch must be idempotent"
        );
        assert!(
            self.state
                .acknowledge_event_batch(batch.acknowledgement_token)
        );
    }
}

type AttemptDigest = (
    LoadAttemptId,
    PlayerMediaGeneration,
    LoadAttemptState,
    Option<i64>,
    Option<LoadAttemptId>,
);

#[derive(Debug, Clone, PartialEq)]
struct LifecycleDigest {
    epoch: PlayerAttachmentEpoch,
    attempts: Vec<AttemptDigest>,
    mappings: Vec<(i64, LoadAttemptId)>,
    active: Option<LoadAttemptId>,
    commands: Vec<(PlayerCommandId, CommandSemanticState)>,
    seeks: Vec<(PlayerCommandId, SystemSeekOwnershipState)>,
    logical_terminal: Option<(PlayerMediaGeneration, PlayerPhysicalLoadOutcome)>,
}

impl LifecycleDigest {
    fn from_state(state: &PlayerLifecycleState) -> Self {
        let mut mappings = state
            .playlist_entry_attempts
            .iter()
            .map(|(entry, attempt)| (*entry, *attempt))
            .collect::<Vec<_>>();
        mappings.sort_unstable();
        Self {
            epoch: state.attachment_epoch,
            attempts: state
                .load_attempts
                .values()
                .map(|attempt| {
                    (
                        attempt.id,
                        attempt.media_generation,
                        attempt.state,
                        attempt.playlist_entry_id,
                        attempt.superseded_by,
                    )
                })
                .collect(),
            mappings,
            active: state.active_load_attempt,
            commands: state
                .commands
                .values()
                .map(|command| (command.id, command.state))
                .collect(),
            seeks: state
                .seek_ownership
                .values()
                .map(|owner| (owner.command_id, owner.state))
                .collect(),
            logical_terminal: state.logical_terminal,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RunDigest {
    lifecycle: LifecycleDigest,
    consumer: ConsumerDigest,
    effects: EffectDigest,
}

#[derive(Debug, Clone, PartialEq)]
struct ConsumerDigest {
    attachment_epoch: PlayerAttachmentEpoch,
    position_seconds: Option<f64>,
    active_attempt: Option<LoadAttemptId>,
    logical_terminal: Option<(PlayerMediaGeneration, PlayerPhysicalLoadOutcome)>,
    command_outcomes:
        BTreeMap<(PlayerAttachmentEpoch, PlayerCommandId), PlayerCommandSemanticResult>,
    load_outcomes: BTreeMap<(PlayerAttachmentEpoch, LoadAttemptId), PlayerLoadAttemptResult>,
}

impl From<ConsumerState> for ConsumerDigest {
    fn from(consumer: ConsumerState) -> Self {
        Self {
            attachment_epoch: consumer.attachment_epoch,
            position_seconds: consumer.position_seconds,
            active_attempt: consumer.active_attempt,
            logical_terminal: consumer.logical_terminal,
            command_outcomes: consumer.command_outcomes,
            load_outcomes: consumer.load_outcomes,
        }
    }
}

fn run_partitioned(history: &[SimulatedPlayerEvent], partitions: &[usize]) -> RunDigest {
    let mut scheduler = DeterministicScheduler::new(history.iter().cloned());
    for partition in partitions {
        scheduler.pump(*partition);
    }
    assert!(
        scheduler.delayed.is_empty(),
        "partitions must cover the complete history"
    );
    scheduler.apply_and_acknowledge_batch();
    scheduler.state.assert_invariants().unwrap();
    RunDigest {
        lifecycle: LifecycleDigest::from_state(&scheduler.state),
        consumer: scheduler.consumer.into(),
        effects: scheduler.effects,
    }
}

fn random_partitions(length: usize, seed: u64) -> Vec<usize> {
    let mut random = seed;
    let mut remaining = length;
    let mut partitions = Vec::new();
    while remaining > 0 {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let width = ((random >> 32) as usize % 11 + 1).min(remaining);
        partitions.push(width);
        remaining -= width;
    }
    partitions
}

#[test]
fn external_current_item_never_binds_the_only_pending_attempt() {
    let mut state = PlayerLifecycleState::default();
    let pending = submit_attempt(&mut state, Some(1), 7, "C", &[10]);
    accept_attempt(&mut state, pending);
    let epoch = state.attachment_epoch;

    reduce_checked(
        &mut state,
        PlayerLifecycleInput::StartFile {
            attachment_epoch: epoch,
            playlist_entry_id: 99,
        },
    );
    reconcile(
        &mut state,
        vec![playlist_entry(99, "external-X", true)],
        Some("external-X"),
    );

    assert_eq!(state.load_attempts[&pending].playlist_entry_id, None);
    assert_eq!(state.attempt_for_playlist_entry(99), None);
    assert!(state.reconciliation_required);

    reconcile(
        &mut state,
        vec![
            playlist_entry(99, "external-X", false),
            playlist_entry(20, "C", true),
        ],
        Some("resolved-C"),
    );
    assert_eq!(state.load_attempts[&pending].playlist_entry_id, Some(20));
    assert_eq!(state.active_load_attempt, Some(pending));
    assert_eq!(state.attempt_for_playlist_entry(99), None);
}

#[test]
fn same_target_attempts_remain_ambiguous_until_unique_causal_evidence_exists() {
    let mut state = PlayerLifecycleState::default();
    let first = submit_attempt(&mut state, Some(1), 7, "stream", &[]);
    accept_attempt(&mut state, first);
    let second = submit_attempt(&mut state, Some(2), 7, "stream", &[]);
    accept_attempt(&mut state, second);

    reconcile(
        &mut state,
        vec![playlist_entry(20, "stream", true)],
        Some("resolved-stream"),
    );

    assert_eq!(state.load_attempts[&first].playlist_entry_id, None);
    assert_eq!(state.load_attempts[&second].playlist_entry_id, None);
    assert_eq!(state.attempt_for_playlist_entry(20), None);
    assert!(state.reconciliation_required);
}

#[test]
fn duplicate_lifecycle_events_are_idempotent() {
    let mut state = PlayerLifecycleState::default();
    let attempt = load_active(&mut state, 1, 1, "A", 10);
    let epoch = state.attachment_epoch;
    let before_duplicates = LifecycleDigest::from_state(&state);
    let sequence = state.last_event_sequence();

    let effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::StartFile {
            attachment_epoch: epoch,
            playlist_entry_id: 10,
        },
    );
    assert!(effects.is_empty());
    let effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch: epoch,
            playlist_entry_id: Some(10),
            loaded_target: Some("A".to_owned()),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(LifecycleDigest::from_state(&state), before_duplicates);
    assert_eq!(state.last_event_sequence(), sequence);

    reduce_checked(
        &mut state,
        PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: 10,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
    );
    let after_terminal = LifecycleDigest::from_state(&state);
    let sequence = state.last_event_sequence();
    let effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: 10,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(LifecycleDigest::from_state(&state), after_terminal);
    assert_eq!(state.last_event_sequence(), sequence);
    assert!(state.load_attempts[&attempt].state.is_terminal());
}

#[test]
fn overlapping_and_offset_changed_system_seeks_never_become_native() {
    let mut state = PlayerLifecycleState::default();
    let epoch = state.attachment_epoch;
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandSubmitted {
            command_id: PlayerCommandId::new(10),
            media_generation: PlayerMediaGeneration::new(7),
            raw_player_target_seconds: 10.0,
            effective_room_target_seconds: 15.0,
            dispatch_sequence_boundary: 1,
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: epoch,
            command_id: PlayerCommandId::new(10),
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandSubmitted {
            command_id: PlayerCommandId::new(11),
            media_generation: PlayerMediaGeneration::new(7),
            raw_player_target_seconds: 20.0,
            // The effective target changed with the user's offset. Matching
            // must continue to use raw player coordinates.
            effective_room_target_seconds: 80.0,
            dispatch_sequence_boundary: 2,
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: epoch,
            command_id: PlayerCommandId::new(11),
        },
    );

    assert_eq!(
        state.commands[&PlayerCommandId::new(10)].state,
        CommandSemanticState::Superseded
    );
    assert_eq!(
        state.seek_ownership[&PlayerCommandId::new(10)].state,
        SystemSeekOwnershipState::MayStillArrive
    );

    let old_effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(7),
            observed_sequence: 3,
            position_seconds: 10.1,
        },
    );
    assert!(old_effects.iter().any(|effect| matches!(
        effect,
        PlayerLifecycleEffect::ConsumeSystemSeek { command_id, .. }
            if *command_id == PlayerCommandId::new(10)
    )));
    assert!(
        !old_effects
            .iter()
            .any(|effect| matches!(effect, PlayerLifecycleEffect::NativeSeekCandidate { .. }))
    );

    let new_effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(7),
            observed_sequence: 4,
            position_seconds: 20.1,
        },
    );
    assert!(new_effects.iter().any(|effect| matches!(
        effect,
        PlayerLifecycleEffect::ConsumeSystemSeek { command_id, .. }
            if *command_id == PlayerCommandId::new(11)
    )));
    assert!(
        !new_effects
            .iter()
            .any(|effect| matches!(effect, PlayerLifecycleEffect::NativeSeekCandidate { .. }))
    );
}

#[test]
fn gap_snapshot_preserves_late_system_seek_ownership() {
    let mut state = PlayerLifecycleState::default();
    let epoch = state.attachment_epoch;
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandSubmitted {
            command_id: PlayerCommandId::new(9),
            media_generation: PlayerMediaGeneration::new(7),
            raw_player_target_seconds: 30.0,
            effective_room_target_seconds: 35.0,
            dispatch_sequence_boundary: 4,
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: epoch,
            command_id: PlayerCommandId::new(9),
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::SeekCommandCompletionNotObserved {
            attachment_epoch: epoch,
            command_id: PlayerCommandId::new(9),
        },
    );
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::EventGapDetected {
            attachment_epoch: epoch,
        },
    );
    assert!(state.requires_authoritative_snapshot());
    let snapshot = authoritative_snapshot(&state, Some(PlayerMediaGeneration::new(7)), Some(0.0));
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::AuthoritativeSnapshotApplied(snapshot),
    );
    assert!(!state.requires_authoritative_snapshot());

    let effects = reduce_checked(
        &mut state,
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(7),
            observed_sequence: 5,
            position_seconds: 30.1,
        },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        PlayerLifecycleEffect::ConsumeSystemSeek { command_id, .. }
            if *command_id == PlayerCommandId::new(9)
    )));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, PlayerLifecycleEffect::NativeSeekCandidate { .. }))
    );
    assert_eq!(
        state.seek_ownership[&PlayerCommandId::new(9)].state,
        SystemSeekOwnershipState::Observed
    );
}

#[test]
fn rejected_third_load_preserves_the_accepted_second_load() {
    let mut state = PlayerLifecycleState::default();
    load_active(&mut state, 1, 1, "A", 10);
    let second = submit_attempt(&mut state, Some(2), 2, "B", &[10]);
    accept_attempt(&mut state, second);
    let third = submit_attempt(&mut state, Some(3), 3, "C", &[10]);
    let epoch = state.attachment_epoch;
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch: epoch,
            attempt_id: third,
            failure: PlayerCommandFailureKind::Unknown,
        },
    );

    assert!(!state.load_attempts[&second].state.is_terminal());
    assert_eq!(
        state.commands[&PlayerCommandId::new(3)].state,
        CommandSemanticState::Failed(PlayerCommandFailureKind::Unknown)
    );
    reconcile(&mut state, vec![playlist_entry(20, "B", true)], Some("B"));
    reduce_checked(
        &mut state,
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch: epoch,
            playlist_entry_id: Some(20),
            loaded_target: Some("B".to_owned()),
        },
    );
    assert_eq!(state.active_load_attempt, Some(second));
    assert_eq!(
        state.commands[&PlayerCommandId::new(2)].state,
        CommandSemanticState::Completed
    );
}

#[test]
fn semantic_outcomes_share_the_authoritative_event_order() {
    let mut state = PlayerLifecycleState::default();
    load_active(&mut state, 1, 1, "A", 10);
    let batch = state.peek_event_batch().expect("ordered lifecycle batch");
    let mut orders = batch
        .events
        .iter()
        .map(|event| event.order)
        .chain(batch.semantic_outcomes.iter().map(|outcome| outcome.order))
        .collect::<Vec<_>>();
    orders.sort();
    assert!(
        orders
            .windows(2)
            .all(|pair| pair[0].attachment_epoch != pair[1].attachment_epoch
                || pair[0].sequence < pair[1].sequence)
    );
    assert_eq!(
        orders.iter().copied().collect::<BTreeSet<_>>().len(),
        orders.len()
    );
    assert!(
        batch
            .semantic_outcomes
            .iter()
            .any(|outcome| matches!(outcome.outcome, PlayerSemanticOutcome::Command(_)))
    );
    assert!(
        batch
            .semantic_outcomes
            .iter()
            .any(|outcome| matches!(outcome.outcome, PlayerSemanticOutcome::LoadAttempt(_)))
    );
}

fn partition_history() -> Vec<SimulatedPlayerEvent> {
    let mut history = vec![
        SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(7),
            requested_target: "stream".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::new(),
        }),
        SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            attempt_id: LoadAttemptId::new(1),
        }),
        SimulatedPlayerEvent::input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            entries: vec![playlist_entry(10, "stream", true)],
            current_path: Some("resolved-stream".to_owned()),
        }),
        SimulatedPlayerEvent::StartFile {
            playlist_entry_id: 10,
        },
        SimulatedPlayerEvent::FileLoaded {
            playlist_entry_id: Some(10),
            target: Some("resolved-stream".to_owned()),
        },
        SimulatedPlayerEvent::input(PlayerLifecycleInput::SeekCommandSubmitted {
            command_id: PlayerCommandId::new(2),
            media_generation: PlayerMediaGeneration::new(7),
            raw_player_target_seconds: 30.0,
            effective_room_target_seconds: 35.0,
            dispatch_sequence_boundary: 0,
        }),
        SimulatedPlayerEvent::input(PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            command_id: PlayerCommandId::new(2),
        }),
        SimulatedPlayerEvent::Seeking {
            generation: PlayerMediaGeneration::new(7),
            observed_sequence: 100,
            seeking: true,
        },
        SimulatedPlayerEvent::Position {
            generation: PlayerMediaGeneration::new(7),
            observed_sequence: 101,
            position_seconds: 30.1,
        },
    ];
    for position in 0..70 {
        history.push(SimulatedPlayerEvent::input(
            PlayerLifecycleInput::TransportDelta {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                delta: PlayerTransportDelta {
                    media_generation: Some(PlayerMediaGeneration::new(7)),
                    position_seconds: Some(position as f64),
                    phase: Some(PlayerTransportPhase::Playing),
                    ..PlayerTransportDelta::default()
                },
            },
        ));
    }
    history.extend([
        SimulatedPlayerEvent::Snapshot {
            generation: Some(PlayerMediaGeneration::new(7)),
            position_seconds: Some(69.0),
        },
        SimulatedPlayerEvent::Eof {
            playlist_entry_id: Some(10),
            reached: true,
            position_seconds: Some(69.0),
        },
        SimulatedPlayerEvent::PlaybackRestart {
            playlist_entry_id: Some(10),
        },
    ]);
    history
}

#[test]
fn randomized_gui_pump_partitions_converge_through_real_batch_application() {
    let history = partition_history();
    let expected = run_partitioned(&history, &vec![1; history.len()]);
    assert_eq!(run_partitioned(&history, &[history.len()]), expected);

    for seed in PARTITION_SEEDS {
        let partitions = random_partitions(history.len(), seed);
        let actual = run_partitioned(&history, &partitions);
        assert_eq!(
            actual, expected,
            "pump partition divergence for seed {seed:#x}; partitions={partitions:?}"
        );
    }
    assert_eq!(expected.effects.system_seeks, 1);
    assert_eq!(expected.effects.native_seeks, 0);
    assert_eq!(expected.consumer.position_seconds, Some(69.0));
}

#[test]
fn scheduler_supports_delay_duplicate_drop_and_fake_clock() {
    let history = vec![
        SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "A".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::new(),
        }),
        SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            attempt_id: LoadAttemptId::new(1),
        }),
        SimulatedPlayerEvent::StartFile {
            playlist_entry_id: 99,
        },
        SimulatedPlayerEvent::Timer(1),
        SimulatedPlayerEvent::PlaybackRestart {
            playlist_entry_id: None,
        },
    ];
    let mut scheduler = DeterministicScheduler::new(history);
    scheduler.deliver_next();
    scheduler.deliver_next();
    // Drop an irrelevant restart, duplicate the unknown start, then deliver
    // the fake clock before the delayed start. No real sleep is involved.
    scheduler.drop_at(2);
    scheduler.duplicate_at(0);
    scheduler.deliver_at(1);
    scheduler.deliver_next();
    scheduler.apply_and_acknowledge_batch();

    assert!(scheduler.delayed.is_empty());
    assert!(scheduler.state.reconciliation_required);
    assert_eq!(scheduler.state.now_tick, 1);
    assert_eq!(
        scheduler.state.load_attempts[&LoadAttemptId::new(1)].playlist_entry_id,
        None
    );
}

fn next_random(random: &mut u64) -> u64 {
    *random = random
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *random
}

fn first_attempt_in_state(
    state: &PlayerLifecycleState,
    predicate: impl Fn(&LoadAttempt) -> bool,
) -> Option<LoadAttemptId> {
    state
        .load_attempts
        .values()
        .find(|attempt| predicate(attempt))
        .map(|attempt| attempt.id)
}

fn generated_input(
    state: &PlayerLifecycleState,
    random: &mut u64,
    next_command: &mut u64,
    next_entry: &mut i64,
    step: u64,
) -> SimulatedPlayerEvent {
    let choice = next_random(random) % 24;
    let epoch = state.attachment_epoch;
    match choice {
        0 | 1 => {
            let command = *next_command;
            *next_command += 1;
            let same_generation = state.active_media_generation().filter(|_| *random & 1 == 0);
            let generation =
                same_generation.unwrap_or_else(|| PlayerMediaGeneration::new(command % 4 + 1));
            let target = ["A", "B", "C", "stream"][(next_random(random) as usize) % 4];
            SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(command)),
                media_generation: generation,
                requested_target: target.to_owned(),
                baseline_playlist_entry_ids: state
                    .playlist_entry_attempts
                    .keys()
                    .copied()
                    .collect(),
            })
        }
        2 => first_attempt_in_state(state, |attempt| {
            attempt.state == LoadAttemptState::Submitting
        })
        .map_or(SimulatedPlayerEvent::Timer(step), |attempt_id| {
            SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch,
                attempt_id,
            })
        }),
        3 => first_attempt_in_state(state, |attempt| {
            attempt.state == LoadAttemptState::Submitting
        })
        .map_or(SimulatedPlayerEvent::Timer(step), |attempt_id| {
            SimulatedPlayerEvent::input(PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: epoch,
                attempt_id,
                failure: PlayerCommandFailureKind::Unknown,
            })
        }),
        4 => {
            let candidate = state.load_attempts.values().find(|attempt| {
                !attempt.state.is_terminal()
                    && !matches!(attempt.state, LoadAttemptState::Submitting)
                    && attempt.playlist_entry_id.is_none()
            });
            if let Some(attempt) = candidate {
                let entry_id = *next_entry;
                *next_entry += 1;
                SimulatedPlayerEvent::input(PlayerLifecycleInput::PlaylistSnapshot {
                    attachment_epoch: epoch,
                    entries: vec![playlist_entry(
                        entry_id,
                        &attempt.requested_target,
                        *random & 1 == 0,
                    )],
                    current_path: Some(format!("resolved-{entry_id}")),
                })
            } else {
                SimulatedPlayerEvent::input(PlayerLifecycleInput::PlaylistSnapshot {
                    attachment_epoch: epoch,
                    entries: Vec::new(),
                    current_path: None,
                })
            }
        }
        5 => state.playlist_entry_attempts.keys().next().copied().map_or(
            SimulatedPlayerEvent::StartFile {
                playlist_entry_id: *next_entry,
            },
            |playlist_entry_id| SimulatedPlayerEvent::StartFile { playlist_entry_id },
        ),
        6 => state.playlist_entry_attempts.keys().next().copied().map_or(
            SimulatedPlayerEvent::FileLoaded {
                playlist_entry_id: None,
                target: None,
            },
            |playlist_entry_id| SimulatedPlayerEvent::FileLoaded {
                playlist_entry_id: Some(playlist_entry_id),
                target: Some(format!("loaded-{playlist_entry_id}")),
            },
        ),
        7 => state.playlist_entry_attempts.keys().next().copied().map_or(
            SimulatedPlayerEvent::Timer(step),
            |playlist_entry_id| SimulatedPlayerEvent::EndFile {
                playlist_entry_id,
                outcome: if *random & 1 == 0 {
                    PlayerPhysicalLoadOutcome::Ended
                } else {
                    PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network)
                },
            },
        ),
        8 => {
            let entry_id = *next_entry;
            *next_entry += 1;
            SimulatedPlayerEvent::input(PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch: epoch,
                media_generation: PlayerMediaGeneration::new((*random % 4) + 1),
                playlist_entry_id: entry_id,
                observed_target: format!("external-{entry_id}"),
                file_loaded: *random & 1 == 0,
            })
        }
        9 => SimulatedPlayerEvent::Eof {
            playlist_entry_id: state
                .active_attempt()
                .and_then(|attempt| attempt.playlist_entry_id),
            reached: true,
            position_seconds: Some(step as f64),
        },
        10 => SimulatedPlayerEvent::PlaybackRestart {
            playlist_entry_id: state
                .active_attempt()
                .and_then(|attempt| attempt.playlist_entry_id),
        },
        11 => {
            let command = *next_command;
            *next_command += 1;
            SimulatedPlayerEvent::input(PlayerLifecycleInput::SeekCommandSubmitted {
                command_id: PlayerCommandId::new(command),
                media_generation: state
                    .active_media_generation()
                    .unwrap_or(PlayerMediaGeneration::new(1)),
                raw_player_target_seconds: (step % 60) as f64,
                effective_room_target_seconds: (step % 60) as f64 + 5.0,
                dispatch_sequence_boundary: state.last_event_sequence(),
            })
        }
        12 => state
            .commands
            .values()
            .find(|command| {
                command.kind == LifecycleCommandKind::Seek
                    && command.state == CommandSemanticState::Submitted
            })
            .map_or(SimulatedPlayerEvent::Timer(step), |command| {
                SimulatedPlayerEvent::input(PlayerLifecycleInput::SeekCommandAccepted {
                    attachment_epoch: epoch,
                    command_id: command.id,
                })
            }),
        13 => state
            .commands
            .values()
            .find(|command| {
                command.kind == LifecycleCommandKind::Seek && !command.state.is_terminal()
            })
            .map_or(SimulatedPlayerEvent::Timer(step), |command| {
                SimulatedPlayerEvent::input(
                    PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                        attachment_epoch: epoch,
                        command_id: command.id,
                    },
                )
            }),
        14 => {
            let generation = state
                .active_media_generation()
                .unwrap_or(PlayerMediaGeneration::new(1));
            SimulatedPlayerEvent::Seeking {
                generation,
                observed_sequence: state.last_event_sequence().saturating_add(1),
                seeking: true,
            }
        }
        15 => {
            let owner = state.seek_ownership.values().find(|owner| {
                matches!(
                    owner.state,
                    SystemSeekOwnershipState::Accepted | SystemSeekOwnershipState::MayStillArrive
                )
            });
            owner.map_or(
                SimulatedPlayerEvent::Position {
                    generation: state
                        .active_media_generation()
                        .unwrap_or(PlayerMediaGeneration::new(1)),
                    observed_sequence: state.last_event_sequence().saturating_add(1),
                    position_seconds: step as f64,
                },
                |owner| SimulatedPlayerEvent::Position {
                    generation: owner.media_generation,
                    observed_sequence: owner.dispatch_sequence_boundary.saturating_add(1),
                    position_seconds: owner.raw_player_target_seconds,
                },
            )
        }
        16 => SimulatedPlayerEvent::Gap,
        17 => SimulatedPlayerEvent::Snapshot {
            generation: state.active_media_generation(),
            position_seconds: Some(step as f64),
        },
        18 => SimulatedPlayerEvent::Disconnect,
        19 => SimulatedPlayerEvent::Reattach,
        20 => SimulatedPlayerEvent::Timer(step.saturating_mul(64)),
        21 => SimulatedPlayerEvent::input(PlayerLifecycleInput::LifecycleReconciliationFailed {
            attachment_epoch: epoch,
        }),
        22 => SimulatedPlayerEvent::input(PlayerLifecycleInput::TransportDelta {
            attachment_epoch: epoch,
            delta: PlayerTransportDelta {
                media_generation: state.active_media_generation(),
                position_seconds: Some(step as f64),
                ..PlayerTransportDelta::default()
            },
        }),
        _ => SimulatedPlayerEvent::StartFile {
            playlist_entry_id: *next_entry + 10_000,
        },
    }
}

#[test]
fn generated_histories_cover_delay_drop_duplicate_gap_recovery_and_reattachment() {
    for seed in HISTORY_SEEDS {
        let mut random = seed;
        let mut next_command = 1;
        let mut next_entry = 100;
        let mut scheduler = DeterministicScheduler::new(Vec::new());
        for step in 0..384_u64 {
            let event = generated_input(
                &scheduler.state,
                &mut random,
                &mut next_command,
                &mut next_entry,
                step,
            );
            match next_random(&mut random) % 8 {
                0 => {
                    // Explicitly drop the event.
                }
                1 | 2 => scheduler.delayed.push_back(event),
                3 if !scheduler.delayed.is_empty() => {
                    scheduler.delayed.push_back(event);
                    let index = (next_random(&mut random) as usize) % scheduler.delayed.len();
                    scheduler.deliver_at(index);
                }
                4 => {
                    scheduler.delayed.push_back(event);
                    let index = scheduler.delayed.len() - 1;
                    scheduler.duplicate_at(index);
                    scheduler.drop_at(index);
                }
                _ => {
                    scheduler.delayed.push_back(event);
                    scheduler.deliver_at(scheduler.delayed.len() - 1);
                }
            }
            if step % 17 == 0 {
                scheduler.apply_and_acknowledge_batch();
            }
            scheduler.state.assert_invariants().unwrap_or_else(|error| {
                panic!(
                    "generated history invariant failed for seed {seed:#x}, step {step}: \
                     {error}; state={:#?}",
                    scheduler.state
                )
            });
        }
        while !scheduler.delayed.is_empty() {
            let index = (next_random(&mut random) as usize) % scheduler.delayed.len();
            scheduler.deliver_at(index);
        }
        scheduler.apply_and_acknowledge_batch();
        scheduler.state.assert_invariants().unwrap_or_else(|error| {
            panic!("generated history final invariant failed for seed {seed:#x}: {error}")
        });
    }
}
