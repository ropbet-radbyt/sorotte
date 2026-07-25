//! Pure player lifecycle decision model.
//!
//! The reducer in this module deliberately has no IPC, GUI, filesystem, sleep,
//! or wall-clock dependency. The mpv adapter translates raw ingress into these
//! inputs and executes the returned effects.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fmt::Write as _,
};

use sorotte_player_api::{
    LoadAttemptId, LoadAttemptOutcome, LocalFileUpdate, PlayerActiveLoadSnapshot,
    PlayerAttachmentEpoch, PlayerAuthoritativeSnapshot, PlayerCommandFailureKind, PlayerCommandId,
    PlayerCommandOutcome, PlayerCommandSemanticResult, PlayerEvent,
    PlayerEventAcknowledgementToken, PlayerEventBatch, PlayerEventOrder, PlayerLoadAttemptResult,
    PlayerMediaGeneration, PlayerPhysicalLoadOutcome, PlayerSemanticOutcome,
    PlayerSequenceBoundary, PlayerTransportDelta, PlayerTransportPhase, PlayerTransportSnapshot,
    SequencedPlayerEvent, SequencedPlayerSemanticOutcome, SnapshotField,
};

const MAX_PENDING_TELEMETRY_EVENTS: usize = 64;
const INITIAL_RECONCILIATION_BACKOFF_TICKS: u64 = 100;
const MAX_RECONCILIATION_BACKOFF_TICKS: u64 = 2_000;
const ACCEPTED_UNBOUND_RECONCILIATION_TICKS: u64 = 60 * 1_000;
const QUIESCENT_LOAD_ATTEMPT_RETENTION_TICKS: u64 = 10 * 60 * 1_000;
const SEEK_TOMBSTONE_RETENTION_TICKS: u64 = 60_000;
const MAX_TERMINAL_ATTEMPT_TOMBSTONES: usize = 256;
const MAX_RETIRED_COMMAND_TOMBSTONES: usize = 512;
const MAX_RETIRED_SEEK_TOMBSTONES: usize = 256;
const TERMINAL_TOMBSTONE_SEQUENCE_WINDOW: u64 = 4_096;
const SEEK_MATCH_TOLERANCE_SECONDS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSemanticState {
    Submitted,
    Accepted,
    Completed,
    Superseded,
    Failed(PlayerCommandFailureKind),
    CompletionNotObserved,
    TransportDisconnected,
}

impl CommandSemanticState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Superseded
                | Self::Failed(_)
                | Self::CompletionNotObserved
                | Self::TransportDisconnected
        )
    }

    const fn public_result(self) -> Option<PlayerCommandSemanticResult> {
        match self {
            Self::Submitted | Self::Accepted => None,
            Self::Completed => Some(PlayerCommandSemanticResult::Completed),
            Self::Superseded => Some(PlayerCommandSemanticResult::Superseded),
            Self::Failed(kind) => Some(PlayerCommandSemanticResult::Failed(kind)),
            Self::CompletionNotObserved => Some(PlayerCommandSemanticResult::CompletionNotObserved),
            Self::TransportDisconnected => Some(PlayerCommandSemanticResult::TransportDisconnected),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleCommandKind {
    Load(LoadAttemptId),
    Seek,
    Pause,
    Play,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleCommand {
    pub id: PlayerCommandId,
    pub attachment_epoch: PlayerAttachmentEpoch,
    pub media_generation: Option<PlayerMediaGeneration>,
    pub kind: LifecycleCommandKind,
    pub state: CommandSemanticState,
    outcome_emitted: bool,
    terminal_sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadAttemptState {
    Submitting,
    AcceptedUnbound,
    Bound,
    Starting,
    Active,
    SupersededMayStillEmit { successor: LoadAttemptId },
    MayStillEmit,
    MayStillEmitQuiescent { retire_after_tick: u64 },
    Terminal(PlayerPhysicalLoadOutcome),
}

impl LoadAttemptState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal(_))
    }

    const fn may_receive_lifecycle(self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadAttempt {
    pub id: LoadAttemptId,
    pub attachment_epoch: PlayerAttachmentEpoch,
    pub media_generation: PlayerMediaGeneration,
    pub command_id: Option<PlayerCommandId>,
    pub requested_target: String,
    pub playlist_entry_id: Option<i64>,
    pub baseline_playlist_entry_ids: BTreeSet<i64>,
    pub replaced_attempt: Option<LoadAttemptId>,
    pub superseded_by: Option<LoadAttemptId>,
    pub state: LoadAttemptState,
    pub semantic_outcome_emitted: bool,
    reconcile_until_tick: Option<u64>,
    semantic_outcome_sequence: Option<u64>,
    physical_terminal_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativePlaylistEntry {
    pub id: i64,
    pub original_filename: Option<String>,
    pub current: bool,
}

impl AuthoritativePlaylistEntry {
    pub fn new(id: i64, original_filename: Option<String>, current: bool) -> Self {
        Self {
            id,
            original_filename,
            current,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadLifecycleReconciliation {
    Resolved,
    AuthoritativeIdle,
    AwaitingAcceptedAttempt,
    IncompleteSnapshot,
    TransportFailure,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemSeekOwnership {
    pub attachment_epoch: PlayerAttachmentEpoch,
    pub media_generation: PlayerMediaGeneration,
    pub raw_player_target_seconds: f64,
    pub effective_room_target_seconds: f64,
    pub command_id: PlayerCommandId,
    pub dispatch_sequence_boundary: u64,
    pub state: SystemSeekOwnershipState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemSeekOwnershipState {
    Submitted,
    Accepted,
    MayStillArrive,
    Observed,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ProvisionalEofCandidate {
    attempt_id: LoadAttemptId,
    last_position_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeferredStartFile {
    attachment_epoch: PlayerAttachmentEpoch,
    playlist_entry_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalAttemptTombstone {
    attachment_epoch: PlayerAttachmentEpoch,
    attempt_id: LoadAttemptId,
    media_generation: PlayerMediaGeneration,
    command_id: Option<PlayerCommandId>,
    playlist_entry_id: Option<i64>,
    terminal_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetiredCommandTombstone {
    attachment_epoch: PlayerAttachmentEpoch,
    command_id: PlayerCommandId,
    terminal_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RetiredSeekTombstone {
    ownership: SystemSeekOwnership,
    retire_after_tick: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct CachedBatch {
    batch: PlayerEventBatch,
    event_count: usize,
    outcome_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct EpochDeliveryBuffer {
    attachment_epoch: PlayerAttachmentEpoch,
    pending_events: VecDeque<SequencedPlayerEvent>,
    retained_semantic_outcomes: VecDeque<SequencedPlayerSemanticOutcome>,
    recovery_snapshot: Option<PlayerAuthoritativeSnapshot>,
    gap_detected: bool,
}

impl EpochDeliveryBuffer {
    fn is_empty(&self) -> bool {
        self.pending_events.is_empty()
            && self.retained_semantic_outcomes.is_empty()
            && self.recovery_snapshot.is_none()
            && !self.gap_detected
    }

    fn has_deliverable_content(&self) -> bool {
        !self.pending_events.is_empty()
            || !self.retained_semantic_outcomes.is_empty()
            || self.recovery_snapshot.is_some()
    }

    fn prune_snapshot_covered_events(&mut self) {
        let Some(snapshot) = &self.recovery_snapshot else {
            return;
        };
        let attachment_epoch = snapshot.attachment_epoch;
        let boundary = snapshot.sequence_boundary.through_sequence;
        self.pending_events.retain(|event| {
            event.order.attachment_epoch != attachment_epoch || event.order.sequence > boundary
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLifecycleState {
    pub attachment_epoch: PlayerAttachmentEpoch,
    pub load_attempts: BTreeMap<LoadAttemptId, LoadAttempt>,
    pub playlist_entry_attempts: HashMap<i64, LoadAttemptId>,
    pub active_load_attempt: Option<LoadAttemptId>,
    pub commands: BTreeMap<PlayerCommandId, LifecycleCommand>,
    pub seek_ownership: BTreeMap<PlayerCommandId, SystemSeekOwnership>,
    pub reconciliation_required: bool,
    pub last_reconciliation: Option<LoadLifecycleReconciliation>,
    pub next_reconciliation_tick: Option<u64>,
    pub reconciliation_backoff_ticks: u64,
    pub now_tick: u64,
    pub logical_terminal: Option<(PlayerMediaGeneration, PlayerPhysicalLoadOutcome)>,
    next_load_attempt_id: u64,
    next_event_sequence: u64,
    next_acknowledgement_token: u64,
    deferred_start_files: VecDeque<DeferredStartFile>,
    provisional_eof: Option<ProvisionalEofCandidate>,
    pending_events: VecDeque<SequencedPlayerEvent>,
    retained_semantic_outcomes: VecDeque<SequencedPlayerSemanticOutcome>,
    recovery_snapshot: Option<PlayerAuthoritativeSnapshot>,
    gap_detected: bool,
    pending_native_seek_generation: Option<PlayerMediaGeneration>,
    retired_epoch_deliveries: VecDeque<EpochDeliveryBuffer>,
    terminal_attempt_tombstones: VecDeque<TerminalAttemptTombstone>,
    retired_command_tombstones: VecDeque<RetiredCommandTombstone>,
    retired_seek_tombstones: VecDeque<RetiredSeekTombstone>,
    uncertain_seek_generations: BTreeMap<PlayerMediaGeneration, u64>,
    cached_batch: Option<CachedBatch>,
}

impl Default for PlayerLifecycleState {
    fn default() -> Self {
        Self::new(PlayerAttachmentEpoch::new(1))
    }
}

impl PlayerLifecycleState {
    pub fn new(attachment_epoch: PlayerAttachmentEpoch) -> Self {
        Self {
            attachment_epoch,
            load_attempts: BTreeMap::new(),
            playlist_entry_attempts: HashMap::new(),
            active_load_attempt: None,
            commands: BTreeMap::new(),
            seek_ownership: BTreeMap::new(),
            reconciliation_required: false,
            last_reconciliation: None,
            next_reconciliation_tick: None,
            reconciliation_backoff_ticks: INITIAL_RECONCILIATION_BACKOFF_TICKS,
            now_tick: 0,
            logical_terminal: None,
            next_load_attempt_id: 1,
            next_event_sequence: 1,
            next_acknowledgement_token: 1,
            deferred_start_files: VecDeque::new(),
            provisional_eof: None,
            pending_events: VecDeque::new(),
            retained_semantic_outcomes: VecDeque::new(),
            recovery_snapshot: None,
            gap_detected: false,
            pending_native_seek_generation: None,
            retired_epoch_deliveries: VecDeque::new(),
            terminal_attempt_tombstones: VecDeque::new(),
            retired_command_tombstones: VecDeque::new(),
            retired_seek_tombstones: VecDeque::new(),
            uncertain_seek_generations: BTreeMap::new(),
            cached_batch: None,
        }
    }

    pub fn allocate_load_attempt_id(&mut self) -> LoadAttemptId {
        let value = self.next_load_attempt_id.max(1);
        self.next_load_attempt_id = value.saturating_add(1).max(1);
        LoadAttemptId::new(value)
    }

    pub fn attempt_for_command(&self, command_id: PlayerCommandId) -> Option<LoadAttemptId> {
        match self.commands.get(&command_id).map(|command| command.kind) {
            Some(LifecycleCommandKind::Load(attempt_id)) => Some(attempt_id),
            _ => None,
        }
    }

    pub fn active_attempt(&self) -> Option<&LoadAttempt> {
        self.active_load_attempt
            .and_then(|attempt_id| self.load_attempts.get(&attempt_id))
    }

    pub fn active_media_generation(&self) -> Option<PlayerMediaGeneration> {
        self.active_attempt()
            .map(|attempt| attempt.media_generation)
    }

    /// Returns the logical generation owned by the accepted successor chain.
    ///
    /// This is a projection of explicit reducer relationships, not an event
    /// ownership heuristic: physical lifecycle events still require a bound
    /// playlist-entry ID before they may mutate an attempt.
    pub fn current_media_generation(&self) -> Option<PlayerMediaGeneration> {
        let mut attempt_id = self.active_load_attempt;
        while let Some(current_id) = attempt_id {
            let attempt = self.load_attempts.get(&current_id)?;
            let Some(successor_id) = attempt.superseded_by else {
                return (!attempt.state.is_terminal()).then_some(attempt.media_generation);
            };
            attempt_id = Some(successor_id);
        }
        self.load_attempts
            .values()
            .rev()
            .find(|attempt| {
                !attempt.state.is_terminal()
                    && attempt.superseded_by.is_none()
                    && matches!(
                        attempt.state,
                        LoadAttemptState::Submitting | LoadAttemptState::AcceptedUnbound
                    )
            })
            .map(|attempt| attempt.media_generation)
    }

    pub fn attempt_for_playlist_entry(&self, playlist_entry_id: i64) -> Option<LoadAttemptId> {
        self.playlist_entry_attempts
            .get(&playlist_entry_id)
            .copied()
    }

    pub fn is_known_terminal_playlist_entry(&self, playlist_entry_id: i64) -> bool {
        self.terminal_attempt_tombstones
            .iter()
            .any(|tombstone| tombstone.playlist_entry_id == Some(playlist_entry_id))
    }

    fn is_retired_command(&self, command_id: PlayerCommandId) -> bool {
        self.retired_command_tombstones
            .iter()
            .any(|tombstone| tombstone.command_id == command_id)
    }

    pub fn pending_semantic_outcome_count(&self) -> usize {
        self.retained_semantic_outcomes.len()
            + self
                .retired_epoch_deliveries
                .iter()
                .map(|delivery| delivery.retained_semantic_outcomes.len())
                .sum::<usize>()
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending_events.len()
            + self
                .retired_epoch_deliveries
                .iter()
                .map(|delivery| delivery.pending_events.len())
                .sum::<usize>()
    }

    pub fn last_event_sequence(&self) -> u64 {
        self.next_event_sequence.saturating_sub(1)
    }

    pub fn provisional_eof_attempt(&self) -> Option<LoadAttemptId> {
        self.provisional_eof.map(|candidate| candidate.attempt_id)
    }

    pub fn requires_authoritative_snapshot(&self) -> bool {
        self.gap_detected && self.recovery_snapshot.is_none()
    }

    /// Emits a compact deterministic lifecycle dump suitable for bug reports.
    ///
    /// Media targets are represented only by a coarse kind. Paths, URLs,
    /// credentials, and query strings never enter the output.
    pub fn redacted_debug_dump(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(
            output,
            "epoch={} active_attempt={} logical_terminal={:?} \
             reconciliation={:?} required={} next_tick={} backoff={} \
             event_sequence={} pending_events={} pending_outcomes={} gap={}",
            self.attachment_epoch.get(),
            optional_id(self.active_load_attempt.map(LoadAttemptId::get)),
            self.logical_terminal,
            self.last_reconciliation,
            self.reconciliation_required,
            optional_id(self.next_reconciliation_tick),
            self.reconciliation_backoff_ticks,
            self.last_event_sequence(),
            self.pending_event_count(),
            self.pending_semantic_outcome_count(),
            self.gap_detected,
        );
        for attempt in self.load_attempts.values() {
            let ownership = if self.active_load_attempt == Some(attempt.id) {
                "active"
            } else if attempt.state.is_terminal() {
                "terminal"
            } else if attempt.superseded_by.is_some() {
                "superseded"
            } else if attempt.playlist_entry_id.is_some() {
                "bound"
            } else {
                "unbound"
            };
            let _ = writeln!(
                output,
                "attempt={} epoch={} generation={} command={} playlist={} \
                 target_kind={} state={:?} ownership={} replaced={} successor={}",
                attempt.id.get(),
                attempt.attachment_epoch.get(),
                attempt.media_generation.get(),
                optional_id(attempt.command_id.map(PlayerCommandId::get)),
                optional_signed_id(attempt.playlist_entry_id),
                redacted_target_kind(&attempt.requested_target),
                attempt.state,
                ownership,
                optional_id(attempt.replaced_attempt.map(LoadAttemptId::get)),
                optional_id(attempt.superseded_by.map(LoadAttemptId::get)),
            );
        }
        for command in self.commands.values() {
            let _ = writeln!(
                output,
                "command={} epoch={} generation={} kind={:?} state={:?}",
                command.id.get(),
                command.attachment_epoch.get(),
                optional_id(command.media_generation.map(PlayerMediaGeneration::get)),
                command.kind,
                command.state,
            );
        }
        for ownership in self.seek_ownership.values() {
            let _ = writeln!(
                output,
                "seek command={} epoch={} generation={} boundary={} state={:?}",
                ownership.command_id.get(),
                ownership.attachment_epoch.get(),
                ownership.media_generation.get(),
                ownership.dispatch_sequence_boundary,
                ownership.state,
            );
        }
        output
    }

    pub fn peek_event_batch(&mut self) -> Option<PlayerEventBatch> {
        if let Some(cached) = &self.cached_batch {
            return Some(cached.batch.clone());
        }
        self.prune_snapshot_covered_events();
        while self
            .retired_epoch_deliveries
            .front()
            .is_some_and(EpochDeliveryBuffer::is_empty)
        {
            self.retired_epoch_deliveries.pop_front();
        }

        let delivery = self.retired_epoch_deliveries.front().map_or_else(
            || EpochDeliveryBuffer {
                attachment_epoch: self.attachment_epoch,
                pending_events: self.pending_events.clone(),
                retained_semantic_outcomes: self.retained_semantic_outcomes.clone(),
                recovery_snapshot: self.recovery_snapshot.clone(),
                gap_detected: self.gap_detected,
            },
            Clone::clone,
        );
        if !delivery.has_deliverable_content() {
            return None;
        }
        let token_value = self.next_acknowledgement_token.max(1);
        self.next_acknowledgement_token = token_value.saturating_add(1).max(1);
        let through_sequence = delivery
            .pending_events
            .iter()
            .map(|event| event.order.sequence)
            .chain(
                delivery
                    .retained_semantic_outcomes
                    .iter()
                    .map(|outcome| outcome.order.sequence),
            )
            .chain(
                delivery
                    .recovery_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.sequence_boundary.through_sequence),
            )
            .max()
            .unwrap_or(0);
        let batch = PlayerEventBatch {
            attachment_epoch: delivery.attachment_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(
                delivery.attachment_epoch,
                through_sequence,
            ),
            authoritative_snapshot: delivery.recovery_snapshot,
            events: delivery.pending_events.into_iter().collect(),
            semantic_outcomes: delivery.retained_semantic_outcomes.into_iter().collect(),
            acknowledgement_token: PlayerEventAcknowledgementToken::new(
                delivery.attachment_epoch,
                token_value,
            ),
        };
        self.cached_batch = Some(CachedBatch {
            event_count: batch.events.len(),
            outcome_count: batch.semantic_outcomes.len(),
            batch: batch.clone(),
        });
        Some(batch)
    }

    pub fn acknowledge_event_batch(&mut self, token: PlayerEventAcknowledgementToken) -> bool {
        let Some(cached) = self.cached_batch.take() else {
            return false;
        };
        if cached.batch.acknowledgement_token != token {
            self.cached_batch = Some(cached);
            return false;
        }
        let acknowledged_current_epoch = cached.batch.attachment_epoch == self.attachment_epoch;
        let acknowledged_through_sequence = cached.batch.sequence_boundary.through_sequence;
        if self
            .retired_epoch_deliveries
            .front()
            .is_some_and(|delivery| delivery.attachment_epoch == cached.batch.attachment_epoch)
        {
            let delivery = self
                .retired_epoch_deliveries
                .front_mut()
                .expect("retired delivery was present");
            for _ in 0..cached.event_count {
                delivery.pending_events.pop_front();
            }
            for _ in 0..cached.outcome_count {
                delivery.retained_semantic_outcomes.pop_front();
            }
            if cached.batch.authoritative_snapshot.is_some() {
                delivery.recovery_snapshot = None;
                delivery.gap_detected = false;
            }
            delivery.prune_snapshot_covered_events();
            if delivery.is_empty() {
                self.retired_epoch_deliveries.pop_front();
            }
        } else if cached.batch.attachment_epoch == self.attachment_epoch {
            for _ in 0..cached.event_count {
                self.pending_events.pop_front();
            }
            for _ in 0..cached.outcome_count {
                self.retained_semantic_outcomes.pop_front();
            }
            if cached.batch.authoritative_snapshot.is_some() {
                self.recovery_snapshot = None;
                self.gap_detected = false;
            }
        } else {
            self.cached_batch = Some(cached);
            return false;
        }
        self.prune_snapshot_covered_events();
        if acknowledged_current_epoch {
            self.compact_acknowledged_lifecycle(acknowledged_through_sequence);
        }
        true
    }

    fn prune_snapshot_covered_events(&mut self) {
        for delivery in &mut self.retired_epoch_deliveries {
            delivery.prune_snapshot_covered_events();
        }
        let Some(snapshot) = &self.recovery_snapshot else {
            return;
        };
        let attachment_epoch = snapshot.attachment_epoch;
        let boundary = snapshot.sequence_boundary.through_sequence;
        self.pending_events.retain(|event| {
            event.order.attachment_epoch != attachment_epoch || event.order.sequence > boundary
        });
    }

    fn closing_snapshot_for_current_epoch(&self) -> PlayerAuthoritativeSnapshot {
        let active = self.active_attempt();
        let active_load = active.map(|attempt| PlayerActiveLoadSnapshot {
            attempt_id: attempt.id,
            media_generation: attempt.media_generation,
            command_id: attempt.command_id,
            playlist_entry_id: attempt.playlist_entry_id,
        });
        PlayerAuthoritativeSnapshot {
            attachment_epoch: self.attachment_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(
                self.attachment_epoch,
                self.last_event_sequence(),
            ),
            transport: PlayerTransportSnapshot {
                load_attempt_id: active
                    .map(|attempt| SnapshotField::Known(attempt.id))
                    .unwrap_or(SnapshotField::KnownAbsent),
                media_generation: active
                    .map(|attempt| SnapshotField::Known(attempt.media_generation))
                    .unwrap_or(SnapshotField::KnownAbsent),
                ..PlayerTransportSnapshot::default()
            },
            active_load: active_load
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::KnownAbsent),
            current_playlist_entry_id: active
                .and_then(|attempt| attempt.playlist_entry_id)
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::KnownAbsent),
            current_path: SnapshotField::Unavailable,
        }
    }

    fn retire_current_epoch_delivery(&mut self) {
        let delivery = EpochDeliveryBuffer {
            attachment_epoch: self.attachment_epoch,
            pending_events: std::mem::take(&mut self.pending_events),
            retained_semantic_outcomes: std::mem::take(&mut self.retained_semantic_outcomes),
            recovery_snapshot: self.recovery_snapshot.take(),
            gap_detected: std::mem::take(&mut self.gap_detected),
        };
        if !delivery.is_empty() {
            self.retired_epoch_deliveries.push_back(delivery);
        }
    }

    fn retire_command_record(&mut self, command_id: PlayerCommandId) {
        let Some(command) = self.commands.remove(&command_id) else {
            return;
        };
        let Some(terminal_sequence) = command.terminal_sequence else {
            self.commands.insert(command_id, command);
            return;
        };
        self.retired_command_tombstones
            .push_back(RetiredCommandTombstone {
                attachment_epoch: command.attachment_epoch,
                command_id,
                terminal_sequence,
            });
    }

    fn push_retired_seek_tombstone(&mut self, ownership: SystemSeekOwnership) {
        let retire_after_tick = self.now_tick.saturating_add(SEEK_TOMBSTONE_RETENTION_TICKS);
        if self.retired_seek_tombstones.len() >= MAX_RETIRED_SEEK_TOMBSTONES
            && let Some(evicted) = self.retired_seek_tombstones.pop_front()
        {
            self.uncertain_seek_generations
                .entry(evicted.ownership.media_generation)
                .and_modify(|deadline| *deadline = (*deadline).max(evicted.retire_after_tick))
                .or_insert(evicted.retire_after_tick);
        }
        self.retired_seek_tombstones
            .push_back(RetiredSeekTombstone {
                ownership,
                retire_after_tick,
            });
    }

    fn compact_acknowledged_lifecycle(&mut self, through_sequence: u64) {
        let terminal_attempt_ids = self
            .load_attempts
            .values()
            .filter(|attempt| {
                attempt.state.is_terminal()
                    && attempt
                        .semantic_outcome_sequence
                        .is_some_and(|sequence| sequence <= through_sequence)
                    && attempt
                        .physical_terminal_sequence
                        .is_some_and(|sequence| sequence <= through_sequence)
            })
            .map(|attempt| attempt.id)
            .collect::<Vec<_>>();
        for attempt_id in terminal_attempt_ids {
            let Some(attempt) = self.load_attempts.remove(&attempt_id) else {
                continue;
            };
            if let Some(successor_id) = attempt.superseded_by
                && let Some(successor) = self.load_attempts.get_mut(&successor_id)
                && successor.replaced_attempt == Some(attempt_id)
            {
                successor.replaced_attempt = None;
            }
            if let Some(predecessor_id) = attempt.replaced_attempt
                && let Some(predecessor) = self.load_attempts.get_mut(&predecessor_id)
                && predecessor.superseded_by == Some(attempt_id)
            {
                predecessor.superseded_by = None;
            }
            let terminal_sequence = attempt
                .physical_terminal_sequence
                .unwrap_or(through_sequence)
                .max(
                    attempt
                        .semantic_outcome_sequence
                        .unwrap_or(through_sequence),
                );
            self.terminal_attempt_tombstones
                .push_back(TerminalAttemptTombstone {
                    attachment_epoch: attempt.attachment_epoch,
                    attempt_id,
                    media_generation: attempt.media_generation,
                    command_id: attempt.command_id,
                    playlist_entry_id: attempt.playlist_entry_id,
                    terminal_sequence,
                });
            if let Some(command_id) = attempt.command_id
                && self.commands.get(&command_id).is_some_and(|command| {
                    command.state.is_terminal()
                        && command
                            .terminal_sequence
                            .is_some_and(|sequence| sequence <= through_sequence)
                })
            {
                self.retire_command_record(command_id);
            }
        }

        let retired_seek_ids = self
            .seek_ownership
            .iter()
            .filter(|(command_id, ownership)| {
                matches!(
                    ownership.state,
                    SystemSeekOwnershipState::Observed
                        | SystemSeekOwnershipState::Invalidated
                        | SystemSeekOwnershipState::MayStillArrive
                ) && self.commands.get(command_id).is_some_and(|command| {
                    command.state.is_terminal()
                        && command
                            .terminal_sequence
                            .is_some_and(|sequence| sequence <= through_sequence)
                })
            })
            .map(|(command_id, ownership)| (*command_id, *ownership))
            .collect::<Vec<_>>();
        for (command_id, ownership) in retired_seek_ids {
            self.seek_ownership.remove(&command_id);
            if ownership.state == SystemSeekOwnershipState::MayStillArrive {
                self.push_retired_seek_tombstone(ownership);
            }
            self.retire_command_record(command_id);
        }

        let referenced_commands = self
            .load_attempts
            .values()
            .filter_map(|attempt| attempt.command_id)
            .chain(self.seek_ownership.keys().copied())
            .collect::<BTreeSet<_>>();
        let retired_command_ids = self
            .commands
            .values()
            .filter(|command| {
                command.state.is_terminal()
                    && command
                        .terminal_sequence
                        .is_some_and(|sequence| sequence <= through_sequence)
                    && !referenced_commands.contains(&command.id)
            })
            .map(|command| command.id)
            .collect::<Vec<_>>();
        for command_id in retired_command_ids {
            self.retire_command_record(command_id);
        }

        while self.terminal_attempt_tombstones.len() > MAX_TERMINAL_ATTEMPT_TOMBSTONES
            || self
                .terminal_attempt_tombstones
                .front()
                .is_some_and(|tombstone| {
                    tombstone
                        .terminal_sequence
                        .saturating_add(TERMINAL_TOMBSTONE_SEQUENCE_WINDOW)
                        < through_sequence
                })
        {
            self.terminal_attempt_tombstones.pop_front();
        }
        while self.retired_command_tombstones.len() > MAX_RETIRED_COMMAND_TOMBSTONES
            || self
                .retired_command_tombstones
                .front()
                .is_some_and(|tombstone| {
                    tombstone
                        .terminal_sequence
                        .saturating_add(TERMINAL_TOMBSTONE_SEQUENCE_WINDOW)
                        < through_sequence
                })
        {
            self.retired_command_tombstones.pop_front();
        }
    }

    fn prune_expired_seek_tombstones(&mut self) {
        self.retired_seek_tombstones
            .retain(|tombstone| self.now_tick < tombstone.retire_after_tick);
        self.uncertain_seek_generations
            .retain(|_, deadline| self.now_tick < *deadline);
    }

    pub fn assert_invariants(&self) -> Result<(), String> {
        if let Some(active_id) = self.active_load_attempt {
            let active = self
                .load_attempts
                .get(&active_id)
                .ok_or_else(|| "active_load_attempt is missing".to_owned())?;
            if active.state.is_terminal() {
                return Err("active_load_attempt is terminal".to_owned());
            }
            if active.attachment_epoch != self.attachment_epoch {
                return Err("active_load_attempt belongs to another attachment".to_owned());
            }
        }

        let active_attempts = self
            .load_attempts
            .values()
            .filter(|attempt| attempt.state == LoadAttemptState::Active)
            .map(|attempt| attempt.id)
            .collect::<Vec<_>>();
        if active_attempts.len() > 1 {
            return Err("more than one physical attempt is active".to_owned());
        }
        if let [active_id] = active_attempts.as_slice()
            && self.active_load_attempt != Some(*active_id)
        {
            return Err("active physical state disagrees with active_load_attempt".to_owned());
        }
        if self.logical_terminal.is_some() && self.active_load_attempt.is_some() {
            return Err(
                "logical terminal playback still has an active physical attempt".to_owned(),
            );
        }

        let mut mapped_attempts = BTreeSet::new();
        for (entry_id, attempt_id) in &self.playlist_entry_attempts {
            let attempt = self
                .load_attempts
                .get(attempt_id)
                .ok_or_else(|| format!("playlist entry {entry_id} maps to a missing attempt"))?;
            if attempt.attachment_epoch != self.attachment_epoch {
                return Err("playlist mapping crosses attachment epoch".to_owned());
            }
            if attempt.playlist_entry_id != Some(*entry_id) {
                return Err("playlist mapping and attempt disagree".to_owned());
            }
            if attempt.state.is_terminal() {
                return Err("playlist entry maps to a terminal attempt".to_owned());
            }
            if !mapped_attempts.insert(*attempt_id) {
                return Err("one attempt is bound to multiple playlist entries".to_owned());
            }
        }
        for attempt in self.load_attempts.values() {
            if attempt.attachment_epoch != self.attachment_epoch {
                return Err("attempt crosses attachment epoch".to_owned());
            }
            if !attempt.state.is_terminal()
                && let Some(entry_id) = attempt.playlist_entry_id
                && self.playlist_entry_attempts.get(&entry_id) != Some(&attempt.id)
            {
                return Err("live attempt playlist ID is not reverse-mapped".to_owned());
            }
            if attempt.semantic_outcome_emitted != attempt.semantic_outcome_sequence.is_some() {
                return Err("load outcome sequence invariant is violated".to_owned());
            }
            if attempt.state.is_terminal() != attempt.physical_terminal_sequence.is_some() {
                return Err("physical terminal sequence invariant is violated".to_owned());
            }
            if attempt.state == LoadAttemptState::AcceptedUnbound
                && (attempt.playlist_entry_id.is_some() || attempt.reconcile_until_tick.is_none())
            {
                return Err("accepted unbound attempt has no reconciliation deadline".to_owned());
            }
            if attempt.reconcile_until_tick.is_some()
                && (attempt.playlist_entry_id.is_some()
                    || !matches!(
                        attempt.state,
                        LoadAttemptState::AcceptedUnbound
                            | LoadAttemptState::MayStillEmit
                            | LoadAttemptState::SupersededMayStillEmit { .. }
                    ))
            {
                return Err("load attempt reconciliation deadline is inconsistent".to_owned());
            }
            if let Some(successor) = attempt.superseded_by {
                let successor = self
                    .load_attempts
                    .get(&successor)
                    .ok_or_else(|| "attempt successor is missing".to_owned())?;
                if successor.id == attempt.id {
                    return Err("attempt supersedes itself".to_owned());
                }
                if successor.replaced_attempt != Some(attempt.id) {
                    return Err("attempt successor does not point back to predecessor".to_owned());
                }
            }
            if let Some(predecessor) = attempt.replaced_attempt {
                let predecessor = self
                    .load_attempts
                    .get(&predecessor)
                    .ok_or_else(|| "attempt predecessor is missing".to_owned())?;
                if predecessor.id == attempt.id {
                    return Err("attempt replaces itself".to_owned());
                }
                if predecessor.superseded_by.is_some()
                    && predecessor.superseded_by != Some(attempt.id)
                {
                    return Err("attempt predecessor points to another successor".to_owned());
                }
            }
            if let Some(command_id) = attempt.command_id {
                let command = self
                    .commands
                    .get(&command_id)
                    .ok_or_else(|| "load attempt command is missing".to_owned())?;
                if command.kind != LifecycleCommandKind::Load(attempt.id) {
                    return Err("load attempt command points to another effect".to_owned());
                }
                if command.media_generation != Some(attempt.media_generation) {
                    return Err("load attempt command has another media generation".to_owned());
                }
            }
        }
        for (command_id, command) in &self.commands {
            if *command_id != command.id {
                return Err("command map key disagrees with command identity".to_owned());
            }
            if command.attachment_epoch != self.attachment_epoch {
                return Err("command crosses attachment epoch".to_owned());
            }
            if command.outcome_emitted != command.state.is_terminal() {
                return Err("command terminal/outcome invariant is violated".to_owned());
            }
            if command.outcome_emitted != command.terminal_sequence.is_some() {
                return Err("command outcome sequence invariant is violated".to_owned());
            }
            if let LifecycleCommandKind::Load(attempt_id) = command.kind {
                let attempt = self
                    .load_attempts
                    .get(&attempt_id)
                    .ok_or_else(|| "load command attempt is missing".to_owned())?;
                if attempt.command_id != Some(command.id) {
                    return Err("load command attempt does not point back to command".to_owned());
                }
                if command.media_generation != Some(attempt.media_generation) {
                    return Err("load command and attempt generations disagree".to_owned());
                }
            }
        }

        for (command_id, owner) in &self.seek_ownership {
            if *command_id != owner.command_id {
                return Err("seek ownership key disagrees with command identity".to_owned());
            }
            if owner.attachment_epoch != self.attachment_epoch {
                return Err("seek ownership crosses attachment epoch".to_owned());
            }
            if !owner.raw_player_target_seconds.is_finite()
                || !owner.effective_room_target_seconds.is_finite()
            {
                return Err("seek ownership contains a non-finite target".to_owned());
            }
            let command = self
                .commands
                .get(command_id)
                .ok_or_else(|| "seek ownership command is missing".to_owned())?;
            if command.kind != LifecycleCommandKind::Seek {
                return Err("seek ownership belongs to a non-seek command".to_owned());
            }
            if command.media_generation != Some(owner.media_generation) {
                return Err("seek ownership generation disagrees with command".to_owned());
            }
            let compatible = match owner.state {
                SystemSeekOwnershipState::Submitted => {
                    command.state == CommandSemanticState::Submitted
                }
                SystemSeekOwnershipState::Accepted => {
                    command.state == CommandSemanticState::Accepted
                }
                SystemSeekOwnershipState::MayStillArrive => matches!(
                    command.state,
                    CommandSemanticState::Superseded | CommandSemanticState::CompletionNotObserved
                ),
                SystemSeekOwnershipState::Observed => matches!(
                    command.state,
                    CommandSemanticState::Completed
                        | CommandSemanticState::Superseded
                        | CommandSemanticState::CompletionNotObserved
                ),
                SystemSeekOwnershipState::Invalidated => {
                    matches!(
                        command.state,
                        CommandSemanticState::Failed(_)
                            | CommandSemanticState::Superseded
                            | CommandSemanticState::CompletionNotObserved
                            | CommandSemanticState::TransportDisconnected
                    )
                }
            };
            if !compatible {
                return Err("seek ownership state disagrees with command state".to_owned());
            }
        }

        if self.terminal_attempt_tombstones.len() > MAX_TERMINAL_ATTEMPT_TOMBSTONES
            || self.retired_command_tombstones.len() > MAX_RETIRED_COMMAND_TOMBSTONES
            || self.retired_seek_tombstones.len() > MAX_RETIRED_SEEK_TOMBSTONES
        {
            return Err("lifecycle tombstone bound is violated".to_owned());
        }
        if self.terminal_attempt_tombstones.iter().any(|tombstone| {
            tombstone.attachment_epoch != self.attachment_epoch
                || tombstone.attempt_id.get() == 0
                || tombstone.media_generation.get() == 0
                || tombstone
                    .command_id
                    .is_some_and(|command_id| command_id.get() == 0)
                || tombstone.terminal_sequence == 0
        }) || self.retired_command_tombstones.iter().any(|tombstone| {
            tombstone.attachment_epoch != self.attachment_epoch || tombstone.terminal_sequence == 0
        }) || self.retired_seek_tombstones.iter().any(|tombstone| {
            tombstone.ownership.attachment_epoch != self.attachment_epoch
                || tombstone.retire_after_tick <= self.now_tick
        }) {
            return Err("lifecycle tombstone identity is invalid".to_owned());
        }

        let mut ordered_items = BTreeSet::new();
        let mut previous_retired_epoch = None;
        for delivery in &self.retired_epoch_deliveries {
            if delivery.attachment_epoch == self.attachment_epoch
                || previous_retired_epoch
                    .is_some_and(|previous| previous >= delivery.attachment_epoch)
            {
                return Err("retired epoch delivery order is invalid".to_owned());
            }
            previous_retired_epoch = Some(delivery.attachment_epoch);
            if let Some(snapshot) = &delivery.recovery_snapshot
                && (snapshot.attachment_epoch != delivery.attachment_epoch
                    || snapshot.sequence_boundary.attachment_epoch != delivery.attachment_epoch)
            {
                return Err("retired recovery snapshot crosses attachment epoch".to_owned());
            }
            let mut previous_event_sequence = None;
            for event in &delivery.pending_events {
                if event.order.attachment_epoch != delivery.attachment_epoch
                    || event.order.sequence == 0
                    || previous_event_sequence
                        .is_some_and(|previous| previous >= event.order.sequence)
                    || !ordered_items.insert((event.order.attachment_epoch, event.order.sequence))
                {
                    return Err("retired event order is invalid".to_owned());
                }
                previous_event_sequence = Some(event.order.sequence);
            }
            let mut previous_outcome_sequence = None;
            for outcome in &delivery.retained_semantic_outcomes {
                if outcome.order.attachment_epoch != delivery.attachment_epoch
                    || outcome.order.sequence == 0
                    || previous_outcome_sequence
                        .is_some_and(|previous| previous >= outcome.order.sequence)
                    || !ordered_items
                        .insert((outcome.order.attachment_epoch, outcome.order.sequence))
                {
                    return Err("retired semantic outcome order is invalid".to_owned());
                }
                let payload_epoch = match &outcome.outcome {
                    PlayerSemanticOutcome::Command(command) => command.attachment_epoch,
                    PlayerSemanticOutcome::LoadAttempt(attempt) => attempt.attachment_epoch,
                };
                if payload_epoch != delivery.attachment_epoch {
                    return Err("retired semantic payload crosses attachment epoch".to_owned());
                }
                previous_outcome_sequence = Some(outcome.order.sequence);
            }
        }

        let mut previous = None;
        for event in &self.pending_events {
            if event.order.attachment_epoch != self.attachment_epoch {
                return Err("pending event crosses attachment epoch".to_owned());
            }
            if previous.is_some_and(|(epoch, sequence)| {
                epoch == event.order.attachment_epoch && sequence >= event.order.sequence
            }) {
                return Err("event order is not strictly monotonic".to_owned());
            }
            if event.order.sequence == 0
                || !ordered_items.insert((event.order.attachment_epoch, event.order.sequence))
            {
                return Err("event order is zero or duplicated".to_owned());
            }
            previous = Some((event.order.attachment_epoch, event.order.sequence));
        }

        let mut previous_outcome = None;
        let mut command_outcomes = BTreeSet::new();
        let mut load_outcomes = BTreeSet::new();
        for outcome in &self.retained_semantic_outcomes {
            if outcome.order.attachment_epoch != self.attachment_epoch {
                return Err("semantic outcome crosses attachment epoch".to_owned());
            }
            if outcome.order.sequence == 0
                || !ordered_items.insert((outcome.order.attachment_epoch, outcome.order.sequence))
            {
                return Err("semantic outcome order is zero or duplicated".to_owned());
            }
            if previous_outcome.is_some_and(|(epoch, sequence)| {
                epoch == outcome.order.attachment_epoch && sequence >= outcome.order.sequence
            }) {
                return Err("semantic outcome order is not strictly monotonic".to_owned());
            }
            previous_outcome = Some((outcome.order.attachment_epoch, outcome.order.sequence));
            match &outcome.outcome {
                PlayerSemanticOutcome::Command(command) => {
                    if command.attachment_epoch != outcome.order.attachment_epoch {
                        return Err("command outcome order crosses attachment epoch".to_owned());
                    }
                    if !command_outcomes.insert((command.attachment_epoch, command.command_id)) {
                        return Err(
                            "command has more than one retained terminal outcome".to_owned()
                        );
                    }
                }
                PlayerSemanticOutcome::LoadAttempt(attempt) => {
                    if attempt.attachment_epoch != outcome.order.attachment_epoch {
                        return Err("load outcome order crosses attachment epoch".to_owned());
                    }
                    if !load_outcomes.insert((attempt.attachment_epoch, attempt.attempt_id)) {
                        return Err(
                            "load attempt has more than one retained semantic outcome".to_owned()
                        );
                    }
                }
            }
        }

        if let Some(snapshot) = &self.recovery_snapshot
            && (snapshot.attachment_epoch != self.attachment_epoch
                || snapshot.sequence_boundary.attachment_epoch != self.attachment_epoch)
        {
            return Err("recovery snapshot crosses attachment epoch".to_owned());
        }
        if let Some(cached) = &self.cached_batch {
            let epoch = cached.batch.attachment_epoch;
            if cached.batch.sequence_boundary.attachment_epoch != epoch
                || cached.batch.acknowledgement_token.attachment_epoch() != epoch
                || cached
                    .batch
                    .authoritative_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| {
                        snapshot.attachment_epoch != epoch
                            || snapshot.sequence_boundary.attachment_epoch != epoch
                    })
                || cached
                    .batch
                    .events
                    .iter()
                    .any(|event| event.order.attachment_epoch != epoch)
                || cached.batch.semantic_outcomes.iter().any(|outcome| {
                    outcome.order.attachment_epoch != epoch
                        || match &outcome.outcome {
                            PlayerSemanticOutcome::Command(command) => {
                                command.attachment_epoch != epoch
                            }
                            PlayerSemanticOutcome::LoadAttempt(attempt) => {
                                attempt.attachment_epoch != epoch
                            }
                        }
                })
            {
                return Err("cached batch is not single-epoch".to_owned());
            }
        }
        Ok(())
    }

    fn next_order(&mut self) -> PlayerEventOrder {
        let sequence = self.next_event_sequence.max(1);
        self.next_event_sequence = sequence.saturating_add(1).max(1);
        PlayerEventOrder::new(self.attachment_epoch, sequence)
    }

    fn queue_event(
        &mut self,
        event: PlayerEvent,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) -> PlayerEventOrder {
        if matches!(event, PlayerEvent::TransportDelta(_)) {
            let telemetry_count = self
                .pending_events
                .iter()
                .filter(|queued| matches!(queued.event, PlayerEvent::TransportDelta(_)))
                .count();
            if telemetry_count >= MAX_PENDING_TELEMETRY_EVENTS {
                if let Some(index) = self
                    .pending_events
                    .iter()
                    .position(|queued| matches!(queued.event, PlayerEvent::TransportDelta(_)))
                {
                    self.pending_events.remove(index);
                }
                self.mark_gap(effects);
            }
        }
        let sequenced = SequencedPlayerEvent {
            order: self.next_order(),
            event,
        };
        let order = sequenced.order;
        effects.push(PlayerLifecycleEffect::EmitOrderedEvent(sequenced.clone()));
        self.pending_events.push_back(sequenced);
        order
    }

    fn retain_outcome(
        &mut self,
        outcome: PlayerSemanticOutcome,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) -> PlayerEventOrder {
        let outcome = SequencedPlayerSemanticOutcome {
            order: self.next_order(),
            outcome,
        };
        let order = outcome.order;
        effects.push(PlayerLifecycleEffect::EmitSemanticOutcome(outcome.clone()));
        self.retained_semantic_outcomes.push_back(outcome);
        order
    }

    fn mark_gap(&mut self, effects: &mut Vec<PlayerLifecycleEffect>) {
        if !self.gap_detected {
            self.gap_detected = true;
            let event = SequencedPlayerEvent {
                order: self.next_order(),
                event: PlayerEvent::EventGapDetected,
            };
            self.pending_events.push_back(event.clone());
            effects.push(PlayerLifecycleEffect::EmitOrderedEvent(event));
        }
        self.reconciliation_required = true;
        self.recovery_snapshot = None;
        self.schedule_reconciliation();
        effects.push(PlayerLifecycleEffect::RequestAuthoritativeSnapshot);
    }

    fn schedule_reconciliation(&mut self) {
        self.reconciliation_required = true;
        self.next_reconciliation_tick = Some(
            self.now_tick
                .saturating_add(self.reconciliation_backoff_ticks),
        );
        self.reconciliation_backoff_ticks = match self.reconciliation_backoff_ticks {
            0..=100 => 250,
            101..=250 => 500,
            251..=500 => 1_000,
            _ => MAX_RECONCILIATION_BACKOFF_TICKS,
        };
    }

    fn command_terminal(
        &mut self,
        command_id: PlayerCommandId,
        terminal: CommandSemanticState,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) {
        let Some(command) = self.commands.get_mut(&command_id) else {
            return;
        };
        if command.state.is_terminal() {
            return;
        }
        let Some(result) = terminal.public_result() else {
            return;
        };
        command.state = terminal;
        command.outcome_emitted = true;
        let outcome = PlayerCommandOutcome {
            attachment_epoch: command.attachment_epoch,
            command_id,
            media_generation: command.media_generation,
            result,
        };
        let order = self.retain_outcome(PlayerSemanticOutcome::Command(outcome), effects);
        if let Some(command) = self.commands.get_mut(&command_id) {
            command.terminal_sequence = Some(order.sequence);
        }
    }

    fn emit_load_outcome(
        &mut self,
        attempt_id: LoadAttemptId,
        result: PlayerLoadAttemptResult,
        loaded_target: Option<String>,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) {
        let Some(attempt) = self.load_attempts.get_mut(&attempt_id) else {
            return;
        };
        if attempt.semantic_outcome_emitted {
            return;
        }
        attempt.semantic_outcome_emitted = true;
        let outcome = LoadAttemptOutcome {
            attachment_epoch: attempt.attachment_epoch,
            attempt_id,
            media_generation: attempt.media_generation,
            command_id: attempt.command_id,
            requested_target: attempt.requested_target.clone(),
            loaded_target,
            result,
        };
        let order = self.retain_outcome(PlayerSemanticOutcome::LoadAttempt(outcome), effects);
        if let Some(attempt) = self.load_attempts.get_mut(&attempt_id) {
            attempt.semantic_outcome_sequence = Some(order.sequence);
        }
    }

    fn bind_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        playlist_entry_id: i64,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) -> bool {
        if self
            .playlist_entry_attempts
            .get(&playlist_entry_id)
            .is_some_and(|existing| *existing != attempt_id)
        {
            self.schedule_reconciliation();
            effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
            return false;
        }
        let Some(attempt) = self.load_attempts.get_mut(&attempt_id) else {
            return false;
        };
        if attempt.attachment_epoch != self.attachment_epoch
            || attempt.state.is_terminal()
            || attempt
                .playlist_entry_id
                .is_some_and(|existing| existing != playlist_entry_id)
        {
            return false;
        }
        let was_unbound = attempt.playlist_entry_id.is_none();
        attempt.playlist_entry_id = Some(playlist_entry_id);
        attempt.reconcile_until_tick = None;
        if matches!(
            attempt.state,
            LoadAttemptState::AcceptedUnbound
                | LoadAttemptState::MayStillEmit
                | LoadAttemptState::MayStillEmitQuiescent { .. }
        ) {
            attempt.state = LoadAttemptState::Bound;
        }
        self.playlist_entry_attempts
            .insert(playlist_entry_id, attempt_id);
        if was_unbound {
            let media_generation = attempt.media_generation;
            let command_id = attempt.command_id;
            self.queue_event(
                PlayerEvent::LoadAttemptBound {
                    attempt_id,
                    media_generation,
                    command_id,
                    playlist_entry_id,
                },
                effects,
            );
        }
        true
    }

    fn start_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        playlist_entry_id: i64,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) {
        if !self.bind_attempt(attempt_id, playlist_entry_id, effects) {
            return;
        }
        let Some(attempt) = self.load_attempts.get_mut(&attempt_id) else {
            return;
        };
        if attempt.state.is_terminal() {
            return;
        }
        if matches!(
            attempt.state,
            LoadAttemptState::Starting | LoadAttemptState::Active
        ) {
            self.active_load_attempt = Some(attempt_id);
            return;
        }
        if let Some(previous_id) = self.active_load_attempt
            && previous_id != attempt_id
            && let Some(previous) = self.load_attempts.get_mut(&previous_id)
            && matches!(
                previous.state,
                LoadAttemptState::Starting | LoadAttemptState::Active
            )
        {
            previous.state = previous
                .superseded_by
                .map_or(LoadAttemptState::MayStillEmit, |successor| {
                    LoadAttemptState::SupersededMayStillEmit { successor }
                });
        }
        let Some(attempt) = self.load_attempts.get_mut(&attempt_id) else {
            return;
        };
        attempt.state = LoadAttemptState::Starting;
        let media_generation = attempt.media_generation;
        let command_id = attempt.command_id;
        self.active_load_attempt = Some(attempt_id);
        self.provisional_eof = None;
        self.queue_event(
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                command_id,
                playlist_entry_id,
            },
            effects,
        );
    }

    fn strict_start_owner(&self, playlist_entry_id: i64) -> Option<LoadAttemptId> {
        self.playlist_entry_attempts
            .get(&playlist_entry_id)
            .copied()
    }

    fn try_deferred_starts(&mut self, effects: &mut Vec<PlayerLifecycleEffect>) {
        let mut retained = VecDeque::new();
        while let Some(deferred) = self.deferred_start_files.pop_front() {
            if deferred.attachment_epoch != self.attachment_epoch {
                continue;
            }
            if let Some(attempt_id) = self.strict_start_owner(deferred.playlist_entry_id) {
                self.start_attempt(attempt_id, deferred.playlist_entry_id, effects);
            } else {
                retained.push_back(deferred);
            }
        }
        self.deferred_start_files = retained;
        if !self.deferred_start_files.is_empty() && self.has_proactive_unbound_load_attempt() {
            self.schedule_reconciliation();
            effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
        }
    }

    fn has_proactive_unbound_load_attempt(&self) -> bool {
        self.load_attempts.values().any(|attempt| {
            attempt.attachment_epoch == self.attachment_epoch
                && attempt.playlist_entry_id.is_none()
                && matches!(
                    attempt.state,
                    LoadAttemptState::Submitting
                        | LoadAttemptState::AcceptedUnbound
                        | LoadAttemptState::MayStillEmit
                )
        })
    }

    fn stop_reconciliation_without_proactive_work(&mut self) {
        if self.has_proactive_unbound_load_attempt() || !self.deferred_start_files.is_empty() {
            return;
        }
        self.reconciliation_required = false;
        self.next_reconciliation_tick = None;
        self.reconciliation_backoff_ticks = INITIAL_RECONCILIATION_BACKOFF_TICKS;
    }

    fn accepted_successor_exists(&self, attempt_id: LoadAttemptId) -> bool {
        let Some(attempt) = self.load_attempts.get(&attempt_id) else {
            return false;
        };
        let mut successor = attempt.superseded_by;
        while let Some(successor_id) = successor {
            let Some(candidate) = self.load_attempts.get(&successor_id) else {
                break;
            };
            if candidate.media_generation == attempt.media_generation
                && !candidate.state.is_terminal()
                && !matches!(candidate.state, LoadAttemptState::Submitting)
            {
                return true;
            }
            successor = candidate.superseded_by;
        }
        false
    }

    fn commit_physical_attempt_terminal(
        &mut self,
        attempt_id: LoadAttemptId,
        outcome: PlayerPhysicalLoadOutcome,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) {
        let Some(before) = self.load_attempts.get(&attempt_id).cloned() else {
            return;
        };
        if before.attachment_epoch != self.attachment_epoch || before.state.is_terminal() {
            return;
        }
        let logical_terminal_allowed = self.active_load_attempt == Some(attempt_id)
            && !self.accepted_successor_exists(attempt_id)
            && before.superseded_by.is_none();
        if let Some(entry_id) = before.playlist_entry_id {
            self.playlist_entry_attempts.remove(&entry_id);
        }
        if self.provisional_eof_attempt() == Some(attempt_id) {
            self.provisional_eof = None;
        }
        if let Some(attempt) = self.load_attempts.get_mut(&attempt_id) {
            attempt.state = LoadAttemptState::Terminal(outcome);
            attempt.reconcile_until_tick = None;
        }
        if self.active_load_attempt == Some(attempt_id) {
            self.active_load_attempt = None;
        }
        let terminal_order = self.queue_event(
            PlayerEvent::LoadAttemptTerminal {
                attempt_id,
                media_generation: before.media_generation,
                outcome,
            },
            effects,
        );
        if let Some(attempt) = self.load_attempts.get_mut(&attempt_id) {
            attempt.physical_terminal_sequence = Some(terminal_order.sequence);
        }

        if !before.semantic_outcome_emitted {
            let result = match outcome {
                PlayerPhysicalLoadOutcome::Ended => PlayerLoadAttemptResult::Indeterminate,
                PlayerPhysicalLoadOutcome::Failed(kind) => PlayerLoadAttemptResult::Failed(kind),
                PlayerPhysicalLoadOutcome::NeverStarted => PlayerLoadAttemptResult::NeverStarted,
                PlayerPhysicalLoadOutcome::TransportDisconnected => {
                    PlayerLoadAttemptResult::TransportDisconnected
                }
            };
            self.emit_load_outcome(attempt_id, result, None, effects);
        }
        if let Some(command_id) = before.command_id
            && self
                .commands
                .get(&command_id)
                .is_some_and(|command| !command.state.is_terminal())
        {
            let terminal = match outcome {
                PlayerPhysicalLoadOutcome::TransportDisconnected => {
                    CommandSemanticState::TransportDisconnected
                }
                PlayerPhysicalLoadOutcome::NeverStarted => {
                    CommandSemanticState::Failed(PlayerCommandFailureKind::Unknown)
                }
                PlayerPhysicalLoadOutcome::Ended | PlayerPhysicalLoadOutcome::Failed(_) => {
                    CommandSemanticState::Failed(PlayerCommandFailureKind::MediaEnded)
                }
            };
            self.command_terminal(command_id, terminal, effects);
        }
        if logical_terminal_allowed {
            self.logical_terminal = Some((before.media_generation, outcome));
            self.queue_event(
                PlayerEvent::LogicalPlaybackTerminal {
                    media_generation: before.media_generation,
                    attempt_id,
                    outcome,
                },
                effects,
            );
            effects.push(PlayerLifecycleEffect::LogicalPlaybackTerminal {
                media_generation: before.media_generation,
                attempt_id,
                outcome,
            });
        }
    }

    fn reconcile_playlist(
        &mut self,
        entries: Vec<AuthoritativePlaylistEntry>,
        current_path: Option<String>,
        effects: &mut Vec<PlayerLifecycleEffect>,
    ) {
        if entries.is_empty() {
            if current_path.is_some() {
                self.last_reconciliation = Some(LoadLifecycleReconciliation::IncompleteSnapshot);
                self.schedule_reconciliation();
                effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                return;
            }
            let awaiting = self.load_attempts.values().any(|attempt| {
                attempt.attachment_epoch == self.attachment_epoch
                    && !attempt.state.is_terminal()
                    && matches!(
                        attempt.state,
                        LoadAttemptState::Submitting
                            | LoadAttemptState::AcceptedUnbound
                            | LoadAttemptState::MayStillEmit
                    )
            });
            if awaiting {
                self.last_reconciliation =
                    Some(LoadLifecycleReconciliation::AwaitingAcceptedAttempt);
                self.schedule_reconciliation();
                effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                return;
            }
            let terminal_attempts = self
                .load_attempts
                .values()
                .filter(|attempt| {
                    matches!(
                        attempt.state,
                        LoadAttemptState::Bound
                            | LoadAttemptState::Starting
                            | LoadAttemptState::Active
                            | LoadAttemptState::SupersededMayStillEmit { .. }
                    )
                })
                .map(|attempt| attempt.id)
                .collect::<Vec<_>>();
            for attempt_id in terminal_attempts {
                self.commit_physical_attempt_terminal(
                    attempt_id,
                    PlayerPhysicalLoadOutcome::Ended,
                    effects,
                );
            }
            self.deferred_start_files.clear();
            self.last_reconciliation = Some(LoadLifecycleReconciliation::AuthoritativeIdle);
            self.reconciliation_required = false;
            self.next_reconciliation_tick = None;
            self.reconciliation_backoff_ticks = INITIAL_RECONCILIATION_BACKOFF_TICKS;
            return;
        }

        let unbound_attempt_ids = self
            .load_attempts
            .values()
            .filter(|attempt| {
                attempt.attachment_epoch == self.attachment_epoch
                    && attempt.state.may_receive_lifecycle()
                    && !matches!(attempt.state, LoadAttemptState::Submitting)
                    && attempt.playlist_entry_id.is_none()
            })
            .map(|attempt| attempt.id)
            .collect::<Vec<_>>();
        let candidate_edges = unbound_attempt_ids
            .iter()
            .filter_map(|attempt_id| {
                let attempt = self.load_attempts.get(attempt_id)?;
                let candidates = entries
                    .iter()
                    .filter(|entry| {
                        !self.playlist_entry_attempts.contains_key(&entry.id)
                            && !attempt.baseline_playlist_entry_ids.contains(&entry.id)
                            && entry.original_filename.as_deref()
                                == Some(attempt.requested_target.as_str())
                    })
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>();
                Some((*attempt_id, candidates))
            })
            .collect::<Vec<_>>();
        for (attempt_id, candidates) in &candidate_edges {
            let [entry_id] = candidates.as_slice() else {
                continue;
            };
            let uniquely_owned = candidate_edges
                .iter()
                .filter(|(_, other_candidates)| other_candidates.contains(entry_id))
                .count()
                == 1;
            if uniquely_owned {
                self.bind_attempt(*attempt_id, *entry_id, effects);
            }
        }

        if let Some(current) = entries.iter().find(|entry| entry.current)
            && let Some(attempt_id) = self.playlist_entry_attempts.get(&current.id).copied()
        {
            self.start_attempt(attempt_id, current.id, effects);
        }
        self.try_deferred_starts(effects);
        let unresolved = self.load_attempts.values().any(|attempt| {
            attempt.attachment_epoch == self.attachment_epoch
                && attempt.playlist_entry_id.is_none()
                && matches!(
                    attempt.state,
                    LoadAttemptState::AcceptedUnbound | LoadAttemptState::MayStillEmit
                )
        });
        if unresolved {
            self.last_reconciliation = Some(LoadLifecycleReconciliation::AwaitingAcceptedAttempt);
            self.schedule_reconciliation();
            effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
        } else {
            self.deferred_start_files.clear();
            self.last_reconciliation = Some(LoadLifecycleReconciliation::Resolved);
            self.reconciliation_required = false;
            self.next_reconciliation_tick = None;
            self.reconciliation_backoff_ticks = INITIAL_RECONCILIATION_BACKOFF_TICKS;
        }
    }
}

fn redacted_target_kind(target: &str) -> &'static str {
    let normalized = target.trim().to_ascii_lowercase();
    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        "network"
    } else if normalized.contains("://") {
        "other-url"
    } else if normalized.is_empty() {
        "empty"
    } else {
        "local-or-opaque"
    }
}

fn optional_id(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn optional_signed_id(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerLifecycleInput {
    LoadAttemptSubmitted {
        command_id: Option<PlayerCommandId>,
        media_generation: PlayerMediaGeneration,
        requested_target: String,
        baseline_playlist_entry_ids: BTreeSet<i64>,
    },
    ExternalLoadObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        media_generation: PlayerMediaGeneration,
        playlist_entry_id: i64,
        observed_target: String,
        file_loaded: bool,
    },
    LoadAttemptAccepted {
        attachment_epoch: PlayerAttachmentEpoch,
        attempt_id: LoadAttemptId,
    },
    LoadAttemptRejected {
        attachment_epoch: PlayerAttachmentEpoch,
        attempt_id: LoadAttemptId,
        failure: PlayerCommandFailureKind,
    },
    CommandSubmitted {
        command_id: PlayerCommandId,
        media_generation: Option<PlayerMediaGeneration>,
        kind: LifecycleCommandKind,
    },
    CommandAccepted {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    CommandRejected {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
        failure: PlayerCommandFailureKind,
    },
    CommandSuperseded {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    CommandTransportDisconnected {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    CommandCompleted {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    CommandCompletionNotObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    StartFile {
        attachment_epoch: PlayerAttachmentEpoch,
        playlist_entry_id: i64,
    },
    FileLoaded {
        attachment_epoch: PlayerAttachmentEpoch,
        playlist_entry_id: Option<i64>,
        loaded_target: Option<String>,
    },
    EndFile {
        attachment_epoch: PlayerAttachmentEpoch,
        playlist_entry_id: i64,
        outcome: PlayerPhysicalLoadOutcome,
    },
    PlaylistSnapshot {
        attachment_epoch: PlayerAttachmentEpoch,
        entries: Vec<AuthoritativePlaylistEntry>,
        current_path: Option<String>,
    },
    LifecycleReconciliationFailed {
        attachment_epoch: PlayerAttachmentEpoch,
    },
    EofObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        playlist_entry_id: Option<i64>,
        reached: bool,
        position_seconds: Option<f64>,
    },
    PlaybackRestart {
        attachment_epoch: PlayerAttachmentEpoch,
        playlist_entry_id: Option<i64>,
    },
    PositionObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        media_generation: PlayerMediaGeneration,
        observed_sequence: u64,
        position_seconds: f64,
    },
    SeekingObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        media_generation: PlayerMediaGeneration,
        observed_sequence: u64,
        seeking: bool,
    },
    PhaseObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        phase: PlayerTransportPhase,
    },
    TransportDelta {
        attachment_epoch: PlayerAttachmentEpoch,
        delta: PlayerTransportDelta,
    },
    LocalFileChanged {
        attachment_epoch: PlayerAttachmentEpoch,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        update: LocalFileUpdate,
    },
    SeekCommandSubmitted {
        command_id: PlayerCommandId,
        media_generation: PlayerMediaGeneration,
        raw_player_target_seconds: f64,
        effective_room_target_seconds: f64,
        dispatch_sequence_boundary: u64,
    },
    SeekCommandAccepted {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    SeekCommandRejected {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
        failure: PlayerCommandFailureKind,
    },
    SeekCommandCompletionNotObserved {
        attachment_epoch: PlayerAttachmentEpoch,
        command_id: PlayerCommandId,
    },
    EventGapDetected {
        attachment_epoch: PlayerAttachmentEpoch,
    },
    AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot),
    TimerAdvanced {
        now_tick: u64,
    },
    TransportDisconnected {
        attachment_epoch: PlayerAttachmentEpoch,
    },
    AttachmentReplaced,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerLifecycleEffect {
    LoadAttemptAllocated {
        attempt_id: LoadAttemptId,
        command_id: Option<PlayerCommandId>,
    },
    RequestLifecycleReconciliation,
    RequestAuthoritativeSnapshot,
    EmitOrderedEvent(SequencedPlayerEvent),
    EmitSemanticOutcome(SequencedPlayerSemanticOutcome),
    LogicalPlaybackTerminal {
        media_generation: PlayerMediaGeneration,
        attempt_id: LoadAttemptId,
        outcome: PlayerPhysicalLoadOutcome,
    },
    ConsumeSystemSeek {
        command_id: PlayerCommandId,
        position_seconds: f64,
    },
    NativeSeekCandidate {
        media_generation: PlayerMediaGeneration,
        position_seconds: f64,
    },
}

pub fn reduce_player_lifecycle(
    mut state: PlayerLifecycleState,
    input: PlayerLifecycleInput,
) -> (PlayerLifecycleState, Vec<PlayerLifecycleEffect>) {
    let mut effects = Vec::new();
    match input {
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id,
            media_generation,
            requested_target,
            baseline_playlist_entry_ids,
        } => {
            if command_id.is_none_or(|command_id| {
                !state.commands.contains_key(&command_id) && !state.is_retired_command(command_id)
            }) {
                let attempt_id = state.allocate_load_attempt_id();
                let replaced_attempt = state
                    .load_attempts
                    .values()
                    .rev()
                    .find(|attempt| {
                        attempt.attachment_epoch == state.attachment_epoch
                            && !attempt.state.is_terminal()
                            && attempt.superseded_by.is_none()
                    })
                    .map(|attempt| attempt.id);
                state.load_attempts.insert(
                    attempt_id,
                    LoadAttempt {
                        id: attempt_id,
                        attachment_epoch: state.attachment_epoch,
                        media_generation,
                        command_id,
                        requested_target,
                        playlist_entry_id: None,
                        baseline_playlist_entry_ids,
                        replaced_attempt,
                        superseded_by: None,
                        state: LoadAttemptState::Submitting,
                        semantic_outcome_emitted: false,
                        reconcile_until_tick: None,
                        semantic_outcome_sequence: None,
                        physical_terminal_sequence: None,
                    },
                );
                if let Some(command_id) = command_id {
                    state.commands.insert(
                        command_id,
                        LifecycleCommand {
                            id: command_id,
                            attachment_epoch: state.attachment_epoch,
                            media_generation: Some(media_generation),
                            kind: LifecycleCommandKind::Load(attempt_id),
                            state: CommandSemanticState::Submitted,
                            outcome_emitted: false,
                            terminal_sequence: None,
                        },
                    );
                }
                effects.push(PlayerLifecycleEffect::LoadAttemptAllocated {
                    attempt_id,
                    command_id,
                });
            }
        }
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation,
            playlist_entry_id,
            observed_target,
            file_loaded,
        } => {
            if attachment_epoch != state.attachment_epoch
                || state
                    .playlist_entry_attempts
                    .contains_key(&playlist_entry_id)
                || state.is_known_terminal_playlist_entry(playlist_entry_id)
            {
                return (state, effects);
            }
            let predecessor_id = state.active_load_attempt;
            let attempt_id = state.allocate_load_attempt_id();
            state.load_attempts.insert(
                attempt_id,
                LoadAttempt {
                    id: attempt_id,
                    attachment_epoch,
                    media_generation,
                    command_id: None,
                    requested_target: observed_target.clone(),
                    playlist_entry_id: Some(playlist_entry_id),
                    baseline_playlist_entry_ids: state
                        .playlist_entry_attempts
                        .keys()
                        .copied()
                        .collect(),
                    replaced_attempt: predecessor_id,
                    superseded_by: None,
                    state: if file_loaded {
                        LoadAttemptState::Active
                    } else {
                        LoadAttemptState::Starting
                    },
                    semantic_outcome_emitted: false,
                    reconcile_until_tick: None,
                    semantic_outcome_sequence: None,
                    physical_terminal_sequence: None,
                },
            );
            if let Some(predecessor_id) = predecessor_id
                && let Some(predecessor) = state.load_attempts.get_mut(&predecessor_id)
                && !predecessor.state.is_terminal()
            {
                predecessor.superseded_by = Some(attempt_id);
                predecessor.state = LoadAttemptState::SupersededMayStillEmit {
                    successor: attempt_id,
                };
                if let Some(command_id) = predecessor.command_id {
                    state.command_terminal(
                        command_id,
                        CommandSemanticState::Superseded,
                        &mut effects,
                    );
                }
            }
            state
                .playlist_entry_attempts
                .insert(playlist_entry_id, attempt_id);
            state.active_load_attempt = Some(attempt_id);
            state.provisional_eof = None;
            state.logical_terminal = None;
            state.queue_event(
                PlayerEvent::LoadAttemptBound {
                    attempt_id,
                    media_generation,
                    command_id: None,
                    playlist_entry_id,
                },
                &mut effects,
            );
            state.queue_event(
                PlayerEvent::LoadAttemptStarting {
                    attempt_id,
                    media_generation,
                    command_id: None,
                    playlist_entry_id,
                },
                &mut effects,
            );
            if file_loaded {
                state.queue_event(
                    PlayerEvent::LoadAttemptActive {
                        attempt_id,
                        media_generation,
                        command_id: None,
                        playlist_entry_id,
                    },
                    &mut effects,
                );
                state.emit_load_outcome(
                    attempt_id,
                    PlayerLoadAttemptResult::Loaded,
                    Some(observed_target),
                    &mut effects,
                );
            }
        }
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch,
            attempt_id,
        } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            let replaced_attempt = state
                .load_attempts
                .get(&attempt_id)
                .and_then(|attempt| attempt.replaced_attempt);
            let command_id = state
                .load_attempts
                .get(&attempt_id)
                .and_then(|attempt| attempt.command_id);
            let transitioned = state.load_attempts.get(&attempt_id).is_some_and(|attempt| {
                attempt.attachment_epoch == attachment_epoch
                    && attempt.state == LoadAttemptState::Submitting
            });
            if !transitioned {
                return (state, effects);
            }
            if let Some(command_id) = command_id
                && let Some(command) = state.commands.get_mut(&command_id)
                && command.attachment_epoch == attachment_epoch
                && command.state == CommandSemanticState::Submitted
            {
                command.state = CommandSemanticState::Accepted;
            }
            let reconcile_until_tick = state
                .now_tick
                .saturating_add(ACCEPTED_UNBOUND_RECONCILIATION_TICKS);
            if let Some(attempt) = state.load_attempts.get_mut(&attempt_id) {
                attempt.state = LoadAttemptState::AcceptedUnbound;
                attempt.reconcile_until_tick = Some(reconcile_until_tick);
            }
            if let Some(predecessor_id) = replaced_attempt {
                let predecessor_command = state
                    .load_attempts
                    .get(&predecessor_id)
                    .and_then(|attempt| attempt.command_id);
                if let Some(predecessor) = state.load_attempts.get_mut(&predecessor_id)
                    && !predecessor.state.is_terminal()
                {
                    predecessor.superseded_by = Some(attempt_id);
                    predecessor.state = LoadAttemptState::SupersededMayStillEmit {
                        successor: attempt_id,
                    };
                }
                if let Some(predecessor_command) = predecessor_command {
                    state.command_terminal(
                        predecessor_command,
                        CommandSemanticState::Superseded,
                        &mut effects,
                    );
                }
            }
            state.logical_terminal = None;
            state.provisional_eof = None;
            state.try_deferred_starts(&mut effects);
            state.schedule_reconciliation();
            effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
        }
        PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch,
            attempt_id,
            failure,
        } => {
            if attachment_epoch != state.attachment_epoch
                || !state.load_attempts.get(&attempt_id).is_some_and(|attempt| {
                    attempt.attachment_epoch == attachment_epoch
                        && attempt.state == LoadAttemptState::Submitting
                })
            {
                return (state, effects);
            }
            let command_id = state
                .load_attempts
                .get(&attempt_id)
                .and_then(|attempt| attempt.command_id);
            if let Some(command_id) = command_id {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::Failed(failure),
                    &mut effects,
                );
            }
            state.emit_load_outcome(
                attempt_id,
                PlayerLoadAttemptResult::NeverStarted,
                None,
                &mut effects,
            );
            state.commit_physical_attempt_terminal(
                attempt_id,
                PlayerPhysicalLoadOutcome::NeverStarted,
                &mut effects,
            );
        }
        PlayerLifecycleInput::CommandSubmitted {
            command_id,
            media_generation,
            kind,
        } => {
            if state.is_retired_command(command_id) {
                return (state, effects);
            }
            state
                .commands
                .entry(command_id)
                .or_insert(LifecycleCommand {
                    id: command_id,
                    attachment_epoch: state.attachment_epoch,
                    media_generation,
                    kind,
                    state: CommandSemanticState::Submitted,
                    outcome_emitted: false,
                    terminal_sequence: None,
                });
        }
        PlayerLifecycleInput::CommandAccepted {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            let accepted_kind = state.commands.get(&command_id).and_then(|command| {
                (command.attachment_epoch == attachment_epoch
                    && command.state == CommandSemanticState::Submitted)
                    .then_some(command.kind)
            });
            if let Some(accepted_kind) = accepted_kind {
                let superseded = state
                    .commands
                    .values()
                    .filter(|command| {
                        command.id != command_id
                            && command.attachment_epoch == attachment_epoch
                            && !command.state.is_terminal()
                            && matches!(
                                (accepted_kind, command.kind),
                                (
                                    LifecycleCommandKind::Pause | LifecycleCommandKind::Play,
                                    LifecycleCommandKind::Pause | LifecycleCommandKind::Play
                                )
                            )
                    })
                    .map(|command| command.id)
                    .collect::<Vec<_>>();
                for superseded_id in superseded {
                    state.command_terminal(
                        superseded_id,
                        CommandSemanticState::Superseded,
                        &mut effects,
                    );
                }
                if let Some(command) = state.commands.get_mut(&command_id) {
                    command.state = CommandSemanticState::Accepted;
                }
            }
        }
        PlayerLifecycleInput::CommandRejected {
            attachment_epoch,
            command_id,
            failure,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::Failed(failure),
                    &mut effects,
                );
                if let Some(owner) = state.seek_ownership.get_mut(&command_id) {
                    owner.state = SystemSeekOwnershipState::Invalidated;
                }
            }
        }
        PlayerLifecycleInput::CommandSuperseded {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.command_terminal(command_id, CommandSemanticState::Superseded, &mut effects);
                if let Some(owner) = state.seek_ownership.get_mut(&command_id)
                    && matches!(
                        owner.state,
                        SystemSeekOwnershipState::Submitted | SystemSeekOwnershipState::Accepted
                    )
                {
                    owner.state = SystemSeekOwnershipState::MayStillArrive;
                }
            }
        }
        PlayerLifecycleInput::CommandTransportDisconnected {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::TransportDisconnected,
                    &mut effects,
                );
                if let Some(owner) = state.seek_ownership.get_mut(&command_id) {
                    owner.state = SystemSeekOwnershipState::Invalidated;
                }
            }
        }
        PlayerLifecycleInput::CommandCompleted {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch == state.attachment_epoch {
                if let Some(owner) = state.seek_ownership.get_mut(&command_id)
                    && matches!(
                        owner.state,
                        SystemSeekOwnershipState::Submitted
                            | SystemSeekOwnershipState::Accepted
                            | SystemSeekOwnershipState::MayStillArrive
                    )
                {
                    owner.state = SystemSeekOwnershipState::Observed;
                }
                state.command_terminal(command_id, CommandSemanticState::Completed, &mut effects);
            }
        }
        PlayerLifecycleInput::CommandCompletionNotObserved {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            state.command_terminal(
                command_id,
                CommandSemanticState::CompletionNotObserved,
                &mut effects,
            );
            if let Some(owner) = state.seek_ownership.get_mut(&command_id)
                && matches!(
                    owner.state,
                    SystemSeekOwnershipState::Submitted | SystemSeekOwnershipState::Accepted
                )
            {
                owner.state = SystemSeekOwnershipState::MayStillArrive;
            }
            let mut quiesced_attempt = None;
            if let Some(attempt_id) = state.attempt_for_command(command_id)
                && let Some(attempt) = state.load_attempts.get_mut(&attempt_id)
                && !attempt.state.is_terminal()
                && !matches!(
                    attempt.state,
                    LoadAttemptState::SupersededMayStillEmit { .. }
                        | LoadAttemptState::MayStillEmitQuiescent { .. }
                )
            {
                if attempt.playlist_entry_id.is_none() {
                    attempt.state = LoadAttemptState::MayStillEmitQuiescent {
                        retire_after_tick: state
                            .now_tick
                            .saturating_add(QUIESCENT_LOAD_ATTEMPT_RETENTION_TICKS),
                    };
                    attempt.reconcile_until_tick = None;
                    quiesced_attempt = Some(attempt_id);
                } else {
                    attempt.state = LoadAttemptState::MayStillEmit;
                }
            }
            if let Some(attempt_id) = quiesced_attempt {
                state.emit_load_outcome(
                    attempt_id,
                    PlayerLoadAttemptResult::Indeterminate,
                    None,
                    &mut effects,
                );
                state.stop_reconciliation_without_proactive_work();
            }
        }
        PlayerLifecycleInput::StartFile {
            attachment_epoch,
            playlist_entry_id,
        } => {
            if attachment_epoch == state.attachment_epoch {
                if state.is_known_terminal_playlist_entry(playlist_entry_id) {
                    return (state, effects);
                }
                if let Some(attempt_id) = state.strict_start_owner(playlist_entry_id) {
                    state.start_attempt(attempt_id, playlist_entry_id, &mut effects);
                } else {
                    if !state.deferred_start_files.iter().any(|event| {
                        event.attachment_epoch == attachment_epoch
                            && event.playlist_entry_id == playlist_entry_id
                    }) {
                        state.deferred_start_files.push_back(DeferredStartFile {
                            attachment_epoch,
                            playlist_entry_id,
                        });
                    }
                    state.schedule_reconciliation();
                    effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                }
            }
        }
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch,
            playlist_entry_id,
            loaded_target,
        } => {
            if attachment_epoch == state.attachment_epoch {
                let attempt_id = playlist_entry_id
                    .and_then(|entry_id| state.playlist_entry_attempts.get(&entry_id).copied());
                if let Some(attempt_id) = attempt_id {
                    let entry_id = state
                        .load_attempts
                        .get(&attempt_id)
                        .and_then(|attempt| attempt.playlist_entry_id);
                    let transitioned =
                        state.load_attempts.get(&attempt_id).is_some_and(|attempt| {
                            matches!(
                                attempt.state,
                                LoadAttemptState::Bound | LoadAttemptState::Starting
                            )
                        });
                    if transitioned && let Some(attempt) = state.load_attempts.get_mut(&attempt_id)
                    {
                        attempt.state = LoadAttemptState::Active;
                    }
                    if transitioned {
                        if let Some(previous_id) = state.active_load_attempt
                            && previous_id != attempt_id
                            && let Some(previous) = state.load_attempts.get_mut(&previous_id)
                            && previous.state == LoadAttemptState::Active
                        {
                            previous.state = previous
                                .superseded_by
                                .map_or(LoadAttemptState::MayStillEmit, |successor| {
                                    LoadAttemptState::SupersededMayStillEmit { successor }
                                });
                        }
                        state.active_load_attempt = Some(attempt_id);
                    }
                    if transitioned && let Some(entry_id) = entry_id {
                        state.queue_event(
                            PlayerEvent::LoadAttemptActive {
                                attempt_id,
                                media_generation: state.load_attempts[&attempt_id].media_generation,
                                command_id: state.load_attempts[&attempt_id].command_id,
                                playlist_entry_id: entry_id,
                            },
                            &mut effects,
                        );
                    }
                    if transitioned {
                        state.emit_load_outcome(
                            attempt_id,
                            PlayerLoadAttemptResult::Loaded,
                            loaded_target,
                            &mut effects,
                        );
                        if let Some(command_id) = state
                            .load_attempts
                            .get(&attempt_id)
                            .and_then(|attempt| attempt.command_id)
                        {
                            state.command_terminal(
                                command_id,
                                CommandSemanticState::Completed,
                                &mut effects,
                            );
                        }
                    }
                } else {
                    state.schedule_reconciliation();
                    effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                }
            }
        }
        PlayerLifecycleInput::EndFile {
            attachment_epoch,
            playlist_entry_id,
            outcome,
        } => {
            if attachment_epoch == state.attachment_epoch {
                if let Some(attempt_id) = state
                    .playlist_entry_attempts
                    .get(&playlist_entry_id)
                    .copied()
                {
                    state.commit_physical_attempt_terminal(attempt_id, outcome, &mut effects);
                } else if state.load_attempts.values().any(|attempt| {
                    attempt.attachment_epoch == attachment_epoch
                        && attempt.playlist_entry_id == Some(playlist_entry_id)
                        && attempt.state.is_terminal()
                }) {
                    // Duplicate terminal notifications are idempotent.
                } else {
                    state.schedule_reconciliation();
                    effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                }
            }
        }
        PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch,
            entries,
            current_path,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.reconcile_playlist(entries, current_path, &mut effects);
            }
        }
        PlayerLifecycleInput::LifecycleReconciliationFailed { attachment_epoch } => {
            if attachment_epoch == state.attachment_epoch {
                state.last_reconciliation = Some(LoadLifecycleReconciliation::TransportFailure);
                state.schedule_reconciliation();
            }
        }
        PlayerLifecycleInput::EofObserved {
            attachment_epoch,
            playlist_entry_id,
            reached,
            position_seconds,
        } => {
            if attachment_epoch == state.attachment_epoch {
                let attempt_id = playlist_entry_id
                    .and_then(|entry_id| state.playlist_entry_attempts.get(&entry_id).copied());
                if reached {
                    if let Some(attempt_id) = attempt_id {
                        state.provisional_eof = Some(ProvisionalEofCandidate {
                            attempt_id,
                            last_position_seconds: position_seconds,
                        });
                    }
                } else if state
                    .provisional_eof
                    .is_some_and(|candidate| Some(candidate.attempt_id) == attempt_id)
                {
                    state.provisional_eof = None;
                }
            }
        }
        PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch,
            playlist_entry_id,
        } => {
            if attachment_epoch == state.attachment_epoch {
                let attempt_id = playlist_entry_id
                    .and_then(|entry_id| state.playlist_entry_attempts.get(&entry_id).copied());
                if state
                    .provisional_eof
                    .is_some_and(|candidate| Some(candidate.attempt_id) == attempt_id)
                {
                    state.provisional_eof = None;
                }
            }
        }
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch,
            media_generation,
            observed_sequence,
            position_seconds,
        } => {
            if attachment_epoch == state.attachment_epoch {
                if state.provisional_eof.is_some_and(|candidate| {
                    state
                        .load_attempts
                        .get(&candidate.attempt_id)
                        .is_some_and(|attempt| attempt.media_generation == media_generation)
                        && candidate
                            .last_position_seconds
                            .is_some_and(|previous| position_seconds > previous + f64::EPSILON)
                }) {
                    state.provisional_eof = None;
                }
                let matching_seek = state.seek_ownership.iter().find_map(|(command_id, owner)| {
                    (owner.attachment_epoch == attachment_epoch
                        && owner.media_generation == media_generation
                        && matches!(
                            owner.state,
                            SystemSeekOwnershipState::Accepted
                                | SystemSeekOwnershipState::MayStillArrive
                        )
                        && observed_sequence > owner.dispatch_sequence_boundary
                        && (owner.raw_player_target_seconds - position_seconds).abs()
                            <= SEEK_MATCH_TOLERANCE_SECONDS)
                        .then_some(*command_id)
                });
                if let Some(command_id) = matching_seek {
                    if let Some(owner) = state.seek_ownership.get_mut(&command_id) {
                        owner.state = SystemSeekOwnershipState::Observed;
                    }
                    state.command_terminal(
                        command_id,
                        CommandSemanticState::Completed,
                        &mut effects,
                    );
                    effects.push(PlayerLifecycleEffect::ConsumeSystemSeek {
                        command_id,
                        position_seconds,
                    });
                    state.pending_native_seek_generation = None;
                } else if let Some(index) =
                    state.retired_seek_tombstones.iter().position(|tombstone| {
                        let owner = tombstone.ownership;
                        owner.attachment_epoch == attachment_epoch
                            && owner.media_generation == media_generation
                            && observed_sequence > owner.dispatch_sequence_boundary
                            && (owner.raw_player_target_seconds - position_seconds).abs()
                                <= SEEK_MATCH_TOLERANCE_SECONDS
                    })
                {
                    let tombstone = state
                        .retired_seek_tombstones
                        .remove(index)
                        .expect("matched retired seek tombstone");
                    effects.push(PlayerLifecycleEffect::ConsumeSystemSeek {
                        command_id: tombstone.ownership.command_id,
                        position_seconds,
                    });
                    state.pending_native_seek_generation = None;
                } else if state.pending_native_seek_generation == Some(media_generation) {
                    state.pending_native_seek_generation = None;
                    if !state
                        .uncertain_seek_generations
                        .contains_key(&media_generation)
                    {
                        effects.push(PlayerLifecycleEffect::NativeSeekCandidate {
                            media_generation,
                            position_seconds,
                        });
                    }
                }
            }
        }
        PlayerLifecycleInput::SeekingObserved {
            attachment_epoch,
            media_generation,
            observed_sequence,
            seeking,
        } => {
            if attachment_epoch == state.attachment_epoch && seeking {
                state.provisional_eof = None;
                let may_be_system_seek =
                    state.seek_ownership.values().any(|owner| {
                        owner.attachment_epoch == attachment_epoch
                            && owner.media_generation == media_generation
                            && observed_sequence > owner.dispatch_sequence_boundary
                            && matches!(
                                owner.state,
                                SystemSeekOwnershipState::Accepted
                                    | SystemSeekOwnershipState::MayStillArrive
                            )
                    }) || state.retired_seek_tombstones.iter().any(|tombstone| {
                        let owner = tombstone.ownership;
                        owner.attachment_epoch == attachment_epoch
                            && owner.media_generation == media_generation
                            && observed_sequence > owner.dispatch_sequence_boundary
                    }) || state
                        .uncertain_seek_generations
                        .contains_key(&media_generation);
                state.pending_native_seek_generation =
                    (!may_be_system_seek).then_some(media_generation);
            }
        }
        PlayerLifecycleInput::PhaseObserved {
            attachment_epoch,
            phase,
        } => {
            if attachment_epoch == state.attachment_epoch
                && matches!(
                    phase,
                    PlayerTransportPhase::Playing
                        | PlayerTransportPhase::Prebuffering
                        | PlayerTransportPhase::Rebuffering
                        | PlayerTransportPhase::Seeking
                )
            {
                state.provisional_eof = None;
            }
        }
        PlayerLifecycleInput::TransportDelta {
            attachment_epoch,
            delta,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.queue_event(PlayerEvent::TransportDelta(delta), &mut effects);
            }
        }
        PlayerLifecycleInput::LocalFileChanged {
            attachment_epoch,
            attempt_id,
            media_generation,
            update,
        } => {
            if attachment_epoch == state.attachment_epoch
                && state.load_attempts.get(&attempt_id).is_some_and(|attempt| {
                    attempt.attachment_epoch == attachment_epoch
                        && attempt.media_generation == media_generation
                        && !attempt.state.is_terminal()
                })
            {
                state.queue_event(
                    PlayerEvent::LocalFileChanged {
                        attempt_id,
                        media_generation,
                        update,
                    },
                    &mut effects,
                );
            }
        }
        PlayerLifecycleInput::SeekCommandSubmitted {
            command_id,
            media_generation,
            raw_player_target_seconds,
            effective_room_target_seconds,
            dispatch_sequence_boundary,
        } => {
            if state.is_retired_command(command_id) {
                return (state, effects);
            }
            let superseded = state
                .commands
                .values()
                .filter(|command| {
                    command.attachment_epoch == state.attachment_epoch
                        && command.id != command_id
                        && command.kind == LifecycleCommandKind::Seek
                        && !command.state.is_terminal()
                })
                .map(|command| command.id)
                .collect::<Vec<_>>();
            for superseded_id in superseded {
                state.command_terminal(
                    superseded_id,
                    CommandSemanticState::Superseded,
                    &mut effects,
                );
                if let Some(owner) = state.seek_ownership.get_mut(&superseded_id)
                    && matches!(
                        owner.state,
                        SystemSeekOwnershipState::Submitted | SystemSeekOwnershipState::Accepted
                    )
                {
                    owner.state = SystemSeekOwnershipState::MayStillArrive;
                }
            }
            state
                .commands
                .entry(command_id)
                .or_insert(LifecycleCommand {
                    id: command_id,
                    attachment_epoch: state.attachment_epoch,
                    media_generation: Some(media_generation),
                    kind: LifecycleCommandKind::Seek,
                    state: CommandSemanticState::Submitted,
                    outcome_emitted: false,
                    terminal_sequence: None,
                });
            state.seek_ownership.insert(
                command_id,
                SystemSeekOwnership {
                    attachment_epoch: state.attachment_epoch,
                    media_generation,
                    raw_player_target_seconds,
                    effective_room_target_seconds,
                    command_id,
                    dispatch_sequence_boundary,
                    state: SystemSeekOwnershipState::Submitted,
                },
            );
        }
        PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            if let Some(command) = state.commands.get_mut(&command_id)
                && command.attachment_epoch == attachment_epoch
                && command.state == CommandSemanticState::Submitted
            {
                command.state = CommandSemanticState::Accepted;
            }
            if let Some(owner) = state.seek_ownership.get_mut(&command_id)
                && owner.state == SystemSeekOwnershipState::Submitted
            {
                owner.state = SystemSeekOwnershipState::Accepted;
            }
        }
        PlayerLifecycleInput::SeekCommandRejected {
            attachment_epoch,
            command_id,
            failure,
        } => {
            if attachment_epoch == state.attachment_epoch {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::Failed(failure),
                    &mut effects,
                );
                if let Some(owner) = state.seek_ownership.get_mut(&command_id) {
                    owner.state = SystemSeekOwnershipState::Invalidated;
                }
            }
        }
        PlayerLifecycleInput::SeekCommandCompletionNotObserved {
            attachment_epoch,
            command_id,
        } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            state.command_terminal(
                command_id,
                CommandSemanticState::CompletionNotObserved,
                &mut effects,
            );
            if let Some(owner) = state.seek_ownership.get_mut(&command_id)
                && matches!(
                    owner.state,
                    SystemSeekOwnershipState::Submitted | SystemSeekOwnershipState::Accepted
                )
            {
                owner.state = SystemSeekOwnershipState::MayStillArrive;
            }
        }
        PlayerLifecycleInput::EventGapDetected { attachment_epoch } => {
            if attachment_epoch == state.attachment_epoch {
                state.mark_gap(&mut effects);
                state.pending_native_seek_generation = None;
            }
        }
        PlayerLifecycleInput::AuthoritativeSnapshotApplied(snapshot) => {
            if snapshot.attachment_epoch == state.attachment_epoch
                && snapshot.sequence_boundary.attachment_epoch == state.attachment_epoch
                && snapshot.sequence_boundary.through_sequence <= state.last_event_sequence()
            {
                let boundary = snapshot.sequence_boundary.through_sequence;
                let attachment_epoch = state.attachment_epoch;
                state.recovery_snapshot = Some(snapshot);
                state.gap_detected = true;
                if state.cached_batch.is_none() {
                    state.pending_events.retain(|event| {
                        event.order.attachment_epoch != attachment_epoch
                            || event.order.sequence > boundary
                    });
                }
            }
        }
        PlayerLifecycleInput::TimerAdvanced { now_tick } => {
            state.now_tick = state.now_tick.max(now_tick);
            state.prune_expired_seek_tombstones();
            let expired_active_reconciliation_attempts = state
                .load_attempts
                .values()
                .filter(|attempt| {
                    attempt.playlist_entry_id.is_none()
                        && attempt
                            .reconcile_until_tick
                            .is_some_and(|deadline| state.now_tick >= deadline)
                        && matches!(
                            attempt.state,
                            LoadAttemptState::AcceptedUnbound
                                | LoadAttemptState::MayStillEmit
                                | LoadAttemptState::SupersededMayStillEmit { .. }
                        )
                })
                .map(|attempt| attempt.id)
                .collect::<Vec<_>>();
            let quiescent_retire_after_tick = state
                .now_tick
                .saturating_add(QUIESCENT_LOAD_ATTEMPT_RETENTION_TICKS);
            for attempt_id in expired_active_reconciliation_attempts {
                if let Some(attempt) = state.load_attempts.get_mut(&attempt_id) {
                    attempt.state = LoadAttemptState::MayStillEmitQuiescent {
                        retire_after_tick: quiescent_retire_after_tick,
                    };
                    attempt.reconcile_until_tick = None;
                }
                state.emit_load_outcome(
                    attempt_id,
                    PlayerLoadAttemptResult::Indeterminate,
                    None,
                    &mut effects,
                );
            }
            let expired_quiescent_attempts = state
                .load_attempts
                .values()
                .filter_map(|attempt| match attempt.state {
                    LoadAttemptState::MayStillEmitQuiescent { retire_after_tick }
                        if state.now_tick >= retire_after_tick =>
                    {
                        Some(attempt.id)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            for attempt_id in expired_quiescent_attempts {
                state.commit_physical_attempt_terminal(
                    attempt_id,
                    PlayerPhysicalLoadOutcome::NeverStarted,
                    &mut effects,
                );
            }
            state.stop_reconciliation_without_proactive_work();
            if state.reconciliation_required
                && state
                    .next_reconciliation_tick
                    .is_none_or(|deadline| state.now_tick >= deadline)
            {
                effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
                state.schedule_reconciliation();
            }
        }
        PlayerLifecycleInput::TransportDisconnected { attachment_epoch } => {
            if attachment_epoch != state.attachment_epoch {
                return (state, effects);
            }
            let command_ids = state
                .commands
                .values()
                .filter(|command| !command.state.is_terminal())
                .map(|command| command.id)
                .collect::<Vec<_>>();
            for command_id in command_ids {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::TransportDisconnected,
                    &mut effects,
                );
                if let Some(owner) = state.seek_ownership.get_mut(&command_id) {
                    owner.state = SystemSeekOwnershipState::Invalidated;
                }
            }
            let attempt_ids = state
                .load_attempts
                .values()
                .filter(|attempt| !attempt.state.is_terminal())
                .map(|attempt| attempt.id)
                .collect::<Vec<_>>();
            for attempt_id in attempt_ids {
                state.commit_physical_attempt_terminal(
                    attempt_id,
                    PlayerPhysicalLoadOutcome::TransportDisconnected,
                    &mut effects,
                );
            }
        }
        PlayerLifecycleInput::AttachmentReplaced => {
            let previous_epoch = state.attachment_epoch;
            if state.gap_detected && state.recovery_snapshot.is_none() {
                state.recovery_snapshot = Some(state.closing_snapshot_for_current_epoch());
            }
            let command_ids = state
                .commands
                .values()
                .filter(|command| !command.state.is_terminal())
                .map(|command| command.id)
                .collect::<Vec<_>>();
            for command_id in command_ids {
                state.command_terminal(
                    command_id,
                    CommandSemanticState::TransportDisconnected,
                    &mut effects,
                );
            }
            let attempt_ids = state
                .load_attempts
                .values()
                .filter(|attempt| !attempt.state.is_terminal())
                .map(|attempt| attempt.id)
                .collect::<Vec<_>>();
            for attempt_id in attempt_ids {
                state.commit_physical_attempt_terminal(
                    attempt_id,
                    PlayerPhysicalLoadOutcome::TransportDisconnected,
                    &mut effects,
                );
            }
            state.retire_current_epoch_delivery();
            state.attachment_epoch = state.attachment_epoch.next();
            state.load_attempts.clear();
            state.playlist_entry_attempts.clear();
            state.active_load_attempt = None;
            state.commands.clear();
            state.seek_ownership.clear();
            state.terminal_attempt_tombstones.clear();
            state.retired_command_tombstones.clear();
            state.retired_seek_tombstones.clear();
            state.uncertain_seek_generations.clear();
            state.deferred_start_files.clear();
            state.provisional_eof = None;
            state.logical_terminal = None;
            state.next_event_sequence = 1;
            state.reconciliation_required = true;
            state.next_reconciliation_tick = None;
            state.reconciliation_backoff_ticks = INITIAL_RECONCILIATION_BACKOFF_TICKS;
            state.gap_detected = true;
            state.recovery_snapshot = None;
            state.pending_native_seek_generation = None;
            state.queue_event(
                PlayerEvent::AttachmentReplaced { previous_epoch },
                &mut effects,
            );
            effects.push(PlayerLifecycleEffect::RequestAuthoritativeSnapshot);
            effects.push(PlayerLifecycleEffect::RequestLifecycleReconciliation);
        }
    }
    if let Err(reason) = state.assert_invariants() {
        debug_assert!(false, "player lifecycle invariant violated: {reason}");
    }
    (state, effects)
}

#[cfg(test)]
mod acceptance_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_player_api::PlayerMediaLoadFailureKind;

    fn baseline(ids: &[i64]) -> BTreeSet<i64> {
        ids.iter().copied().collect()
    }

    fn reduce(state: &mut PlayerLifecycleState, input: PlayerLifecycleInput) {
        let current = std::mem::take(state);
        let (next, _) = reduce_player_lifecycle(current, input);
        next.assert_invariants().expect("lifecycle invariants");
        *state = next;
    }

    fn submit(
        state: &mut PlayerLifecycleState,
        command: u64,
        generation: u64,
        target: &str,
        baseline_ids: &[i64],
    ) -> LoadAttemptId {
        reduce(
            state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(command)),
                media_generation: PlayerMediaGeneration::new(generation),
                requested_target: target.to_owned(),
                baseline_playlist_entry_ids: baseline(baseline_ids),
            },
        );
        state
            .attempt_for_command(PlayerCommandId::new(command))
            .expect("submitted load attempt")
    }

    fn accept(state: &mut PlayerLifecycleState, command: u64) {
        let attempt_id = state
            .attempt_for_command(PlayerCommandId::new(command))
            .expect("submitted load attempt");
        let attachment_epoch = state.attachment_epoch;
        reduce(
            state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch,
                attempt_id,
            },
        );
    }

    fn bind_with_snapshot(state: &mut PlayerLifecycleState, id: i64, target: &str, current: bool) {
        reduce(
            state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: state.attachment_epoch,
                entries: vec![AuthoritativePlaylistEntry::new(
                    id,
                    Some(target.to_owned()),
                    current,
                )],
                current_path: current.then(|| target.to_owned()),
            },
        );
    }

    #[test]
    fn lifecycle_debug_dump_is_deterministic_and_redacts_media_targets() {
        let mut state = PlayerLifecycleState::default();
        let attempt_id = submit(
            &mut state,
            41,
            7,
            "https://media.invalid/video?token=private-value",
            &[],
        );
        accept(&mut state, 41);
        bind_with_snapshot(
            &mut state,
            71,
            "https://media.invalid/video?token=private-value",
            true,
        );

        let dump = state.redacted_debug_dump();

        assert_eq!(dump, state.redacted_debug_dump());
        assert!(dump.contains("epoch=1"));
        assert!(dump.contains(&format!("attempt={}", attempt_id.get())));
        assert!(dump.contains("generation=7"));
        assert!(dump.contains("command=41"));
        assert!(dump.contains("playlist=71"));
        assert!(dump.contains("target_kind=network"));
        assert!(!dump.contains("media.invalid"));
        assert!(!dump.contains("private-value"));
        assert!(!dump.contains("token="));
    }

    #[test]
    fn rapid_replacement_delayed_middle_terminal_cannot_end_successor() {
        for outcome in [
            PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network),
            PlayerPhysicalLoadOutcome::Ended,
        ] {
            let mut state = PlayerLifecycleState::default();
            submit(&mut state, 1, 1, "A", &[]);
            accept(&mut state, 1);
            bind_with_snapshot(&mut state, 10, "A", true);
            reduce(
                &mut state,
                PlayerLifecycleInput::FileLoaded {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: Some(10),
                    loaded_target: Some("A".to_owned()),
                },
            );

            submit(&mut state, 2, 2, "B", &[10]);
            accept(&mut state, 2);
            submit(&mut state, 3, 3, "C", &[10]);
            accept(&mut state, 3);
            bind_with_snapshot(&mut state, 20, "B", false);
            reduce(
                &mut state,
                PlayerLifecycleInput::StartFile {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: 20,
                },
            );
            reduce(
                &mut state,
                PlayerLifecycleInput::EndFile {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: 20,
                    outcome,
                },
            );
            assert_eq!(state.logical_terminal, None);

            bind_with_snapshot(&mut state, 30, "C", true);
            reduce(
                &mut state,
                PlayerLifecycleInput::FileLoaded {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: Some(30),
                    loaded_target: Some("C".to_owned()),
                },
            );
            assert_eq!(
                state
                    .active_attempt()
                    .map(|attempt| attempt.requested_target.as_str()),
                Some("C")
            );
            assert_eq!(state.logical_terminal, None);
        }
    }

    #[test]
    fn synchronous_rejection_preserves_prior_accepted_attempt() {
        let mut state = PlayerLifecycleState::default();
        submit(&mut state, 1, 1, "A", &[]);
        accept(&mut state, 1);
        bind_with_snapshot(&mut state, 10, "A", true);
        submit(&mut state, 2, 2, "B", &[10]);
        accept(&mut state, 2);
        let b = state
            .attempt_for_command(PlayerCommandId::new(2))
            .expect("B");
        submit(&mut state, 3, 3, "C", &[10]);
        let c = state
            .attempt_for_command(PlayerCommandId::new(3))
            .expect("C attempt");
        reduce(
            &mut state,
            PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                attempt_id: c,
                failure: PlayerCommandFailureKind::Unknown,
            },
        );
        assert!(!state.load_attempts[&b].state.is_terminal());
        bind_with_snapshot(&mut state, 20, "B", true);
        reduce(
            &mut state,
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                playlist_entry_id: Some(20),
                loaded_target: Some("B".to_owned()),
            },
        );
        assert_eq!(state.active_load_attempt, Some(b));
    }

    #[test]
    fn same_generation_recovery_old_terminal_never_ends_logical_media() {
        for terminal_after_start in [false, true] {
            for outcome in [
                PlayerPhysicalLoadOutcome::Ended,
                PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network),
            ] {
                let mut state = PlayerLifecycleState::default();
                let old = submit(&mut state, 1, 7, "stream", &[]);
                accept(&mut state, 1);
                bind_with_snapshot(&mut state, 10, "stream", true);
                reduce(
                    &mut state,
                    PlayerLifecycleInput::FileLoaded {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        playlist_entry_id: Some(10),
                        loaded_target: Some("stream".to_owned()),
                    },
                );
                let replacement = submit(&mut state, 2, 7, "stream", &[10]);
                accept(&mut state, 2);
                if terminal_after_start {
                    bind_with_snapshot(&mut state, 20, "stream", true);
                }
                reduce(
                    &mut state,
                    PlayerLifecycleInput::EndFile {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        playlist_entry_id: 10,
                        outcome,
                    },
                );
                if !terminal_after_start {
                    bind_with_snapshot(&mut state, 20, "stream", true);
                }
                reduce(
                    &mut state,
                    PlayerLifecycleInput::FileLoaded {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        playlist_entry_id: Some(20),
                        loaded_target: Some("stream".to_owned()),
                    },
                );
                assert!(state.load_attempts[&old].state.is_terminal());
                assert_eq!(state.active_load_attempt, Some(replacement));
                assert_eq!(state.logical_terminal, None);
            }
        }
    }

    #[test]
    fn ambiguous_start_mutates_no_attempt_and_strict_snapshot_resolves_it() {
        let mut state = PlayerLifecycleState::default();
        let b = submit(&mut state, 1, 1, "B", &[]);
        accept(&mut state, 1);
        let c = submit(&mut state, 2, 2, "C", &[]);
        accept(&mut state, 2);
        reduce(
            &mut state,
            PlayerLifecycleInput::StartFile {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                playlist_entry_id: 99,
            },
        );
        assert_eq!(state.load_attempts[&b].playlist_entry_id, None);
        assert_eq!(state.load_attempts[&c].playlist_entry_id, None);
        assert!(state.reconciliation_required);

        reduce(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: vec![
                    AuthoritativePlaylistEntry::new(98, Some("B".to_owned()), false),
                    AuthoritativePlaylistEntry::new(99, Some("C".to_owned()), true),
                ],
                current_path: Some("resolved-C".to_owned()),
            },
        );
        assert_eq!(state.load_attempts[&b].playlist_entry_id, Some(98));
        assert_eq!(state.load_attempts[&c].playlist_entry_id, Some(99));
    }

    #[test]
    fn empty_playlist_is_bounded_and_never_retires_recent_acceptance() {
        let mut state = PlayerLifecycleState::default();
        let attempt = submit(&mut state, 1, 1, "C", &[]);
        accept(&mut state, 1);
        reduce(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: Vec::new(),
                current_path: None,
            },
        );
        assert_eq!(
            state.last_reconciliation,
            Some(LoadLifecycleReconciliation::AwaitingAcceptedAttempt)
        );
        assert!(!state.load_attempts[&attempt].state.is_terminal());
        let first_deadline = state.next_reconciliation_tick.expect("scheduled retry");
        reduce(
            &mut state,
            PlayerLifecycleInput::TimerAdvanced {
                now_tick: first_deadline,
            },
        );
        assert!(state.next_reconciliation_tick.expect("bounded next retry") > first_deadline);
    }

    #[test]
    fn commandless_accepted_load_quiesces_at_its_physical_reconciliation_deadline() {
        let state = PlayerLifecycleState::default();
        let (mut state, effects) = reduce_player_lifecycle(
            state,
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: None,
                media_generation: PlayerMediaGeneration::new(7),
                requested_target: "same-generation-recovery".to_owned(),
                baseline_playlist_entry_ids: baseline(&[10]),
            },
        );
        let attempt_id = effects
            .iter()
            .find_map(|effect| match effect {
                PlayerLifecycleEffect::LoadAttemptAllocated { attempt_id, .. } => Some(*attempt_id),
                _ => None,
            })
            .expect("commandless load attempt");
        reduce(
            &mut state,
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                attempt_id,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: Vec::new(),
                current_path: None,
            },
        );

        let current = std::mem::take(&mut state);
        let (next, effects) = reduce_player_lifecycle(
            current,
            PlayerLifecycleInput::TimerAdvanced {
                now_tick: ACCEPTED_UNBOUND_RECONCILIATION_TICKS,
            },
        );
        state = next;
        state
            .assert_invariants()
            .expect("commandless quiescent invariants");
        assert!(matches!(
            state.load_attempts[&attempt_id].state,
            LoadAttemptState::MayStillEmitQuiescent { .. }
        ));
        assert!(!state.reconciliation_required);
        assert_eq!(state.next_reconciliation_tick, None);
        assert!(effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::EmitSemanticOutcome(outcome)
                if matches!(
                    outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(LoadAttemptOutcome {
                        attempt_id: observed,
                        result: PlayerLoadAttemptResult::Indeterminate,
                        ..
                    }) if observed == attempt_id
                )
        )));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::RequestLifecycleReconciliation
        )));
    }

    #[test]
    fn timed_out_unbound_load_quiesces_without_losing_strict_late_ownership() {
        let mut state = PlayerLifecycleState::default();
        let attempt_id = submit(&mut state, 1, 1, "late-target", &[]);
        accept(&mut state, 1);
        reduce(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: Vec::new(),
                current_path: None,
            },
        );
        assert!(state.reconciliation_required);

        let current = std::mem::take(&mut state);
        let (next, effects) = reduce_player_lifecycle(
            current,
            PlayerLifecycleInput::CommandCompletionNotObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id: PlayerCommandId::new(1),
            },
        );
        state = next;
        state.assert_invariants().expect("quiescent invariants");
        assert!(matches!(
            state.load_attempts[&attempt_id].state,
            LoadAttemptState::MayStillEmitQuiescent { .. }
        ));
        assert!(!state.reconciliation_required);
        assert_eq!(state.next_reconciliation_tick, None);
        assert_eq!(state.current_media_generation(), None);
        assert!(effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::EmitSemanticOutcome(outcome)
                if matches!(
                    outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(LoadAttemptOutcome {
                        result: PlayerLoadAttemptResult::Indeterminate,
                        ..
                    })
                )
        )));

        let current = std::mem::take(&mut state);
        let (next, effects) = reduce_player_lifecycle(
            current,
            PlayerLifecycleInput::TimerAdvanced { now_tick: 120_000 },
        );
        state = next;
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::RequestLifecycleReconciliation
        )));

        let attachment_epoch = state.attachment_epoch;
        reduce(
            &mut state,
            PlayerLifecycleInput::StartFile {
                attachment_epoch,
                playlist_entry_id: 77,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch,
                entries: vec![AuthoritativePlaylistEntry::new(
                    77,
                    Some("late-target".to_owned()),
                    true,
                )],
                current_path: Some("late-target".to_owned()),
            },
        );
        assert_eq!(state.attempt_for_playlist_entry(77), Some(attempt_id));
        assert_eq!(state.active_load_attempt, Some(attempt_id));
        assert_eq!(
            state.load_attempts[&attempt_id].state,
            LoadAttemptState::Starting
        );
    }

    #[test]
    fn eof_is_provisional_and_contradictory_evidence_cancels_it() {
        for contradiction in 0..3 {
            let mut state = PlayerLifecycleState::default();
            submit(&mut state, 1, 1, "A", &[]);
            accept(&mut state, 1);
            bind_with_snapshot(&mut state, 10, "A", true);
            reduce(
                &mut state,
                PlayerLifecycleInput::FileLoaded {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: Some(10),
                    loaded_target: Some("A".to_owned()),
                },
            );
            reduce(
                &mut state,
                PlayerLifecycleInput::EofObserved {
                    attachment_epoch: PlayerAttachmentEpoch::new(1),
                    playlist_entry_id: Some(10),
                    reached: true,
                    position_seconds: Some(20.0),
                },
            );
            assert!(state.provisional_eof_attempt().is_some());
            match contradiction {
                0 => reduce(
                    &mut state,
                    PlayerLifecycleInput::PlaybackRestart {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        playlist_entry_id: Some(10),
                    },
                ),
                1 => reduce(
                    &mut state,
                    PlayerLifecycleInput::PositionObserved {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        media_generation: PlayerMediaGeneration::new(1),
                        observed_sequence: 1,
                        position_seconds: 21.0,
                    },
                ),
                _ => reduce(
                    &mut state,
                    PlayerLifecycleInput::SeekingObserved {
                        attachment_epoch: PlayerAttachmentEpoch::new(1),
                        media_generation: PlayerMediaGeneration::new(1),
                        observed_sequence: 1,
                        seeking: true,
                    },
                ),
            }
            reduce(
                &mut state,
                PlayerLifecycleInput::TimerAdvanced { now_tick: 10_000 },
            );
            assert_eq!(state.provisional_eof_attempt(), None);
            assert_eq!(state.logical_terminal, None);
        }
    }

    #[test]
    fn reattachment_reuses_playlist_id_without_cross_core_identity() {
        let mut state = PlayerLifecycleState::default();
        submit(&mut state, 1, 1, "old", &[]);
        accept(&mut state, 1);
        bind_with_snapshot(&mut state, 1, "old", true);
        reduce(&mut state, PlayerLifecycleInput::AttachmentReplaced);
        assert_eq!(state.attachment_epoch, PlayerAttachmentEpoch::new(2));
        assert!(state.load_attempts.is_empty());
        assert!(state.playlist_entry_attempts.is_empty());

        submit(&mut state, 2, 2, "new", &[]);
        accept(&mut state, 2);
        bind_with_snapshot(&mut state, 1, "new", true);
        let new_attempt_id = state.active_load_attempt.expect("new active attempt");
        reduce(
            &mut state,
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(2),
                sequence_boundary: PlayerSequenceBoundary::new(PlayerAttachmentEpoch::new(2), 0),
                transport: PlayerTransportSnapshot::default(),
                active_load: SnapshotField::Known(PlayerActiveLoadSnapshot {
                    attempt_id: new_attempt_id,
                    media_generation: PlayerMediaGeneration::new(2),
                    command_id: Some(PlayerCommandId::new(2)),
                    playlist_entry_id: Some(1),
                }),
                current_playlist_entry_id: SnapshotField::Known(1),
                current_path: SnapshotField::Known("new".to_owned()),
            }),
        );
        assert_eq!(
            state
                .active_attempt()
                .map(|attempt| attempt.requested_target.as_str()),
            Some("new")
        );

        let old_batch = state.peek_event_batch().expect("old-epoch handoff batch");
        assert_eq!(old_batch.attachment_epoch, PlayerAttachmentEpoch::new(1));
        assert_eq!(state.peek_event_batch(), Some(old_batch.clone()));
        assert!(
            old_batch
                .events
                .iter()
                .all(|event| { event.order.attachment_epoch == PlayerAttachmentEpoch::new(1) })
        );
        assert!(old_batch.semantic_outcomes.iter().all(|outcome| {
            outcome.order.attachment_epoch == PlayerAttachmentEpoch::new(1)
                && match &outcome.outcome {
                    PlayerSemanticOutcome::Command(command) => {
                        command.attachment_epoch == PlayerAttachmentEpoch::new(1)
                    }
                    PlayerSemanticOutcome::LoadAttempt(attempt) => {
                        attempt.attachment_epoch == PlayerAttachmentEpoch::new(1)
                    }
                }
        }));
        assert!(old_batch.semantic_outcomes.iter().any(|outcome| matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::Command(command)
                if command.result == PlayerCommandSemanticResult::TransportDisconnected
        )));
        assert!(
            state.acknowledge_event_batch(old_batch.acknowledgement_token),
            "old epoch must acknowledge before the successor is visible"
        );

        let new_batch = state
            .peek_event_batch()
            .expect("new-epoch replacement batch");
        assert_eq!(new_batch.attachment_epoch, PlayerAttachmentEpoch::new(2));
        assert!(
            new_batch
                .events
                .iter()
                .all(|event| { event.order.attachment_epoch == PlayerAttachmentEpoch::new(2) })
        );
        assert!(
            new_batch
                .semantic_outcomes
                .iter()
                .all(|outcome| { outcome.order.attachment_epoch == PlayerAttachmentEpoch::new(2) })
        );
        assert!(new_batch.events.iter().any(|event| matches!(
            event.event,
            PlayerEvent::AttachmentReplaced {
                previous_epoch
            } if previous_epoch == PlayerAttachmentEpoch::new(1)
        )));
        assert_eq!(
            new_batch
                .authoritative_snapshot
                .as_ref()
                .and_then(|snapshot| match snapshot.active_load {
                    SnapshotField::Known(active) => Some(active),
                    SnapshotField::KnownAbsent | SnapshotField::Unavailable => None,
                })
                .map(|active| (active.media_generation, active.playlist_entry_id)),
            Some((PlayerMediaGeneration::new(2), Some(1)))
        );
    }

    #[test]
    fn cached_old_epoch_batch_replays_before_terminal_handoff_and_replacement_epoch() {
        let mut state = PlayerLifecycleState::default();
        submit(&mut state, 1, 1, "old", &[]);
        accept(&mut state, 1);
        bind_with_snapshot(&mut state, 10, "old", true);
        let cached = state.peek_event_batch().expect("initial cached batch");

        reduce(&mut state, PlayerLifecycleInput::AttachmentReplaced);
        assert_eq!(
            state.peek_event_batch(),
            Some(cached.clone()),
            "replacement must not mutate an already returned batch"
        );
        assert!(state.acknowledge_event_batch(cached.acknowledgement_token));

        let terminal_handoff = state.peek_event_batch().expect("terminal handoff");
        assert_eq!(
            terminal_handoff.attachment_epoch,
            PlayerAttachmentEpoch::new(1)
        );
        assert!(terminal_handoff.semantic_outcomes.iter().any(|outcome| {
            matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::Command(command)
                    if command.result
                        == PlayerCommandSemanticResult::TransportDisconnected
            )
        }));
        assert!(state.acknowledge_event_batch(terminal_handoff.acknowledgement_token));

        let replacement = state.peek_event_batch().expect("replacement epoch");
        assert_eq!(replacement.attachment_epoch, PlayerAttachmentEpoch::new(2));
        assert!(
            replacement
                .events
                .iter()
                .any(|event| matches!(event.event, PlayerEvent::AttachmentReplaced { .. }))
        );
    }

    #[test]
    fn retiring_epoch_with_gap_freezes_snapshot_before_terminal_outcomes() {
        let mut state = PlayerLifecycleState::default();
        submit(&mut state, 1, 1, "old", &[]);
        accept(&mut state, 1);
        reduce(
            &mut state,
            PlayerLifecycleInput::EventGapDetected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
            },
        );
        reduce(&mut state, PlayerLifecycleInput::AttachmentReplaced);

        let handoff = state.peek_event_batch().expect("gap-closing handoff");
        let snapshot = handoff
            .authoritative_snapshot
            .as_ref()
            .expect("retiring gap must have a closing snapshot");
        assert_eq!(snapshot.attachment_epoch, PlayerAttachmentEpoch::new(1));
        assert!(handoff.semantic_outcomes.iter().all(|outcome| {
            outcome.order.sequence > snapshot.sequence_boundary.through_sequence
        }));
        assert!(handoff.semantic_outcomes.iter().any(|outcome| matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::LoadAttempt(attempt)
                if attempt.result == PlayerLoadAttemptResult::TransportDisconnected
        )));
    }

    #[test]
    fn acknowledged_gap_marker_waits_for_snapshot_without_emitting_empty_batches() {
        let mut state = PlayerLifecycleState::default();
        reduce(
            &mut state,
            PlayerLifecycleInput::EventGapDetected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
            },
        );

        let gap_batch = state.peek_event_batch().expect("gap marker batch");
        assert!(gap_batch.authoritative_snapshot.is_none());
        assert!(
            gap_batch
                .events
                .iter()
                .any(|event| { matches!(event.event, PlayerEvent::EventGapDetected) })
        );
        assert!(state.acknowledge_event_batch(gap_batch.acknowledgement_token));
        assert!(state.requires_authoritative_snapshot());
        assert_eq!(
            state.peek_event_batch(),
            None,
            "a pending snapshot latch has no deliverable payload of its own"
        );

        let snapshot = state.closing_snapshot_for_current_epoch();
        reduce(
            &mut state,
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(snapshot),
        );
        let snapshot_batch = state.peek_event_batch().expect("snapshot batch");
        assert!(snapshot_batch.authoritative_snapshot.is_some());
        assert!(state.acknowledge_event_batch(snapshot_batch.acknowledgement_token));
        assert!(!state.requires_authoritative_snapshot());
        assert_eq!(state.peek_event_batch(), None);
    }

    #[test]
    fn semantic_outcomes_survive_gap_and_batch_replay_until_acknowledged() {
        let mut state = PlayerLifecycleState::default();
        let attempt_id = submit(&mut state, 1, 1, "A", &[]);
        reduce(
            &mut state,
            PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                attempt_id,
                failure: PlayerCommandFailureKind::Unknown,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::EventGapDetected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
            },
        );
        let first = state.peek_event_batch().expect("batch");
        let repeated = state.peek_event_batch().expect("repeated batch");
        assert_eq!(first, repeated);
        assert!(!first.semantic_outcomes.is_empty());
        assert!(state.load_attempts.contains_key(&attempt_id));
        assert!(
            !state.acknowledge_event_batch(PlayerEventAcknowledgementToken::new(
                state.attachment_epoch,
                first.acknowledgement_token.get().saturating_add(1),
            ))
        );
        assert_eq!(state.peek_event_batch(), Some(first.clone()));
        assert!(state.load_attempts.contains_key(&attempt_id));
        assert!(state.acknowledge_event_batch(first.acknowledgement_token));
        assert_eq!(state.pending_semantic_outcome_count(), 0);
        assert!(!state.load_attempts.contains_key(&attempt_id));
    }

    #[test]
    fn acknowledged_timed_out_seek_compacts_to_bounded_late_ownership() {
        let mut state = PlayerLifecycleState::default();
        let command_id = PlayerCommandId::new(9);
        let generation = PlayerMediaGeneration::new(3);
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandSubmitted {
                command_id,
                media_generation: generation,
                raw_player_target_seconds: 42.0,
                effective_room_target_seconds: 42.0,
                dispatch_sequence_boundary: 0,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id,
            },
        );
        let batch = state.peek_event_batch().expect("seek timeout batch");
        assert!(state.acknowledge_event_batch(batch.acknowledgement_token));
        assert!(!state.commands.contains_key(&command_id));
        assert!(!state.seek_ownership.contains_key(&command_id));
        assert_eq!(state.retired_seek_tombstones.len(), 1);

        reduce(
            &mut state,
            PlayerLifecycleInput::SeekingObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                media_generation: generation,
                observed_sequence: 1,
                seeking: true,
            },
        );
        let current = std::mem::take(&mut state);
        let (next, effects) = reduce_player_lifecycle(
            current,
            PlayerLifecycleInput::PositionObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                media_generation: generation,
                observed_sequence: 2,
                position_seconds: 42.0,
            },
        );
        state = next;
        state.assert_invariants().expect("late seek invariants");
        assert!(effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::ConsumeSystemSeek {
                command_id: observed,
                ..
            } if *observed == command_id
        )));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, PlayerLifecycleEffect::NativeSeekCandidate { .. }))
        );
        assert!(state.retired_seek_tombstones.is_empty());
    }

    #[test]
    fn acknowledged_lifecycle_history_compacts_under_one_hundred_thousand_operations() {
        let mut state = PlayerLifecycleState::default();
        let first_private_target = "https://media.invalid/private-token-first";

        for operation in 0_u64..100_000 {
            let command_id = PlayerCommandId::new(operation + 1);
            let generation = PlayerMediaGeneration::new(operation + 1);
            let attachment_epoch = state.attachment_epoch;
            match operation % 4 {
                0 | 1 => {
                    let kind = if operation % 4 == 0 {
                        LifecycleCommandKind::Pause
                    } else {
                        LifecycleCommandKind::Play
                    };
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::CommandSubmitted {
                            command_id,
                            media_generation: Some(generation),
                            kind,
                        },
                    );
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::CommandAccepted {
                            attachment_epoch,
                            command_id,
                        },
                    );
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::CommandCompleted {
                            attachment_epoch,
                            command_id,
                        },
                    );
                }
                2 => {
                    let dispatch_sequence_boundary = state.last_event_sequence();
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::SeekCommandSubmitted {
                            command_id,
                            media_generation: generation,
                            raw_player_target_seconds: operation as f64,
                            effective_room_target_seconds: operation as f64,
                            dispatch_sequence_boundary,
                        },
                    );
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::SeekCommandAccepted {
                            attachment_epoch,
                            command_id,
                        },
                    );
                    let observed_sequence = state.last_event_sequence().saturating_add(1);
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::PositionObserved {
                            attachment_epoch,
                            media_generation: generation,
                            observed_sequence,
                            position_seconds: operation as f64,
                        },
                    );
                }
                _ => {
                    let target = if operation == 3 {
                        first_private_target.to_owned()
                    } else {
                        format!("target-{operation}")
                    };
                    let attempt_id =
                        submit(&mut state, command_id.get(), generation.get(), &target, &[]);
                    accept(&mut state, command_id.get());
                    let playlist_entry_id =
                        i64::try_from(operation + 1).expect("stress ID fits i64");
                    bind_with_snapshot(&mut state, playlist_entry_id, &target, true);
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::FileLoaded {
                            attachment_epoch,
                            playlist_entry_id: Some(playlist_entry_id),
                            loaded_target: Some(target),
                        },
                    );
                    reduce(
                        &mut state,
                        PlayerLifecycleInput::EndFile {
                            attachment_epoch,
                            playlist_entry_id,
                            outcome: PlayerPhysicalLoadOutcome::Ended,
                        },
                    );
                    assert_ne!(
                        state.load_attempts[&attempt_id].physical_terminal_sequence,
                        None
                    );
                }
            }

            let batch = state.peek_event_batch().expect("operation delivery batch");
            assert!(state.acknowledge_event_batch(batch.acknowledgement_token));
            assert!(state.load_attempts.len() <= 1);
            assert!(state.commands.len() <= 1);
            assert!(state.seek_ownership.len() <= 1);
            assert!(state.terminal_attempt_tombstones.len() <= MAX_TERMINAL_ATTEMPT_TOMBSTONES);
            assert!(state.retired_command_tombstones.len() <= MAX_RETIRED_COMMAND_TOMBSTONES);
        }

        assert!(state.load_attempts.is_empty());
        assert!(state.commands.is_empty());
        assert!(state.seek_ownership.is_empty());
        assert!(
            !format!("{state:?}").contains(first_private_target),
            "acknowledged compaction must not retain the retired target anywhere"
        );
        let last_entry_id = 100_000_i64;
        let before = state.clone();
        let attachment_epoch = state.attachment_epoch;
        reduce(
            &mut state,
            PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch,
                media_generation: PlayerMediaGeneration::new(200_000),
                playlist_entry_id: last_entry_id,
                observed_target: "duplicate-must-not-revive".to_owned(),
                file_loaded: true,
            },
        );
        assert_eq!(state, before);
    }

    #[test]
    fn late_system_seek_after_gap_is_consumed_in_raw_coordinates() {
        let mut state = PlayerLifecycleState::default();
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandSubmitted {
                command_id: PlayerCommandId::new(9),
                media_generation: PlayerMediaGeneration::new(7),
                raw_player_target_seconds: 30.0,
                effective_room_target_seconds: 35.0,
                dispatch_sequence_boundary: 4,
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id: PlayerCommandId::new(9),
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                command_id: PlayerCommandId::new(9),
            },
        );
        reduce(
            &mut state,
            PlayerLifecycleInput::EventGapDetected {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
            },
        );
        let current = std::mem::take(&mut state);
        let (next, effects) = reduce_player_lifecycle(
            current,
            PlayerLifecycleInput::PositionObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                media_generation: PlayerMediaGeneration::new(7),
                observed_sequence: 5,
                position_seconds: 30.1,
            },
        );
        state = next;
        assert!(effects.iter().any(|effect| matches!(
            effect,
            PlayerLifecycleEffect::ConsumeSystemSeek {
                command_id,
                ..
            } if *command_id == PlayerCommandId::new(9)
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Digest {
        epoch: PlayerAttachmentEpoch,
        attempts: Vec<(LoadAttemptId, LoadAttemptState, Option<i64>)>,
        active: Option<LoadAttemptId>,
        logical_terminal: Option<(PlayerMediaGeneration, PlayerPhysicalLoadOutcome)>,
        outcomes: usize,
    }

    fn digest(state: &PlayerLifecycleState) -> Digest {
        Digest {
            epoch: state.attachment_epoch,
            attempts: state
                .load_attempts
                .values()
                .map(|attempt| (attempt.id, attempt.state, attempt.playlist_entry_id))
                .collect(),
            active: state.active_load_attempt,
            logical_terminal: state.logical_terminal,
            outcomes: state.retained_semantic_outcomes.len(),
        }
    }

    #[test]
    fn pump_partitions_do_not_change_final_lifecycle_state() {
        let history = [
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(PlayerCommandId::new(1)),
                media_generation: PlayerMediaGeneration::new(7),
                requested_target: "stream".to_owned(),
                baseline_playlist_entry_ids: baseline(&[]),
            },
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                attempt_id: LoadAttemptId::new(1),
            },
            PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                entries: vec![AuthoritativePlaylistEntry::new(
                    10,
                    Some("stream".to_owned()),
                    true,
                )],
                current_path: Some("resolved".to_owned()),
            },
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                playlist_entry_id: Some(10),
                loaded_target: Some("resolved".to_owned()),
            },
            PlayerLifecycleInput::EofObserved {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                playlist_entry_id: Some(10),
                reached: true,
                position_seconds: Some(10.0),
            },
            PlayerLifecycleInput::PlaybackRestart {
                attachment_epoch: PlayerAttachmentEpoch::new(1),
                playlist_entry_id: Some(10),
            },
        ];
        let run = |partitions: &[usize]| {
            let mut state = PlayerLifecycleState::default();
            let mut cursor = 0;
            for partition in partitions {
                for input in history[cursor..cursor + partition].iter().cloned() {
                    reduce(&mut state, input);
                }
                cursor += partition;
            }
            assert_eq!(cursor, history.len());
            digest(&state)
        };
        let expected = run(&[1, 1, 1, 1, 1, 1]);
        assert_eq!(run(&[6]), expected);
        assert_eq!(run(&[2, 3, 1]), expected);
        assert_eq!(run(&[3, 1, 2]), expected);
    }

    #[test]
    fn seeded_generated_histories_preserve_invariants_after_every_input() {
        for seed in 0_u64..64 {
            let mut random = seed ^ 0x9e37_79b9_7f4a_7c15;
            let mut state = PlayerLifecycleState::default();
            let mut next_command = 1_u64;
            for step in 0..128_u64 {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let input = match random % 10 {
                    0 | 1 => {
                        let command = next_command;
                        next_command += 1;
                        PlayerLifecycleInput::LoadAttemptSubmitted {
                            command_id: Some(PlayerCommandId::new(command)),
                            media_generation: PlayerMediaGeneration::new((command % 4) + 1),
                            requested_target: format!("target-{}", command % 3),
                            baseline_playlist_entry_ids: state
                                .playlist_entry_attempts
                                .keys()
                                .copied()
                                .collect(),
                        }
                    }
                    2 if next_command > 1 => {
                        let command_id = PlayerCommandId::new((random % (next_command - 1)) + 1);
                        state
                            .attempt_for_command(command_id)
                            .map(|attempt_id| PlayerLifecycleInput::LoadAttemptAccepted {
                                attachment_epoch: state.attachment_epoch,
                                attempt_id,
                            })
                            .unwrap_or(PlayerLifecycleInput::TimerAdvanced { now_tick: step })
                    }
                    3 => PlayerLifecycleInput::StartFile {
                        attachment_epoch: state.attachment_epoch,
                        playlist_entry_id: (random % 8) as i64,
                    },
                    4 => PlayerLifecycleInput::EndFile {
                        attachment_epoch: state.attachment_epoch,
                        playlist_entry_id: (random % 8) as i64,
                        outcome: PlayerPhysicalLoadOutcome::Ended,
                    },
                    5 => PlayerLifecycleInput::EventGapDetected {
                        attachment_epoch: state.attachment_epoch,
                    },
                    6 => PlayerLifecycleInput::EofObserved {
                        attachment_epoch: state.attachment_epoch,
                        playlist_entry_id: state
                            .active_attempt()
                            .and_then(|attempt| attempt.playlist_entry_id),
                        reached: random & 1 == 0,
                        position_seconds: Some(step as f64),
                    },
                    7 => PlayerLifecycleInput::PlaybackRestart {
                        attachment_epoch: state.attachment_epoch,
                        playlist_entry_id: state
                            .active_attempt()
                            .and_then(|attempt| attempt.playlist_entry_id),
                    },
                    8 => PlayerLifecycleInput::TimerAdvanced { now_tick: step },
                    _ if random & 0x100 != 0 => PlayerLifecycleInput::AttachmentReplaced,
                    _ => PlayerLifecycleInput::PlaylistSnapshot {
                        attachment_epoch: state.attachment_epoch,
                        entries: Vec::new(),
                        current_path: None,
                    },
                };
                reduce(&mut state, input);
                state.assert_invariants().unwrap_or_else(|error| {
                    panic!("seed {seed}, step {step}: {error}; state={state:#?}")
                });
            }
        }
    }
}
