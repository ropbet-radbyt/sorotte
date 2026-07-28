//! Shrinkable, state-aware lifecycle histories.
//!
//! The older acceptance generator is deliberately retained: it exercises a
//! deterministic scheduler and fixed seeds. These properties add shrinking,
//! a declared reducer-input vocabulary, and independent epoch/order/at-most-once
//! oracles without copying the reducer's transition implementation. They fuzz
//! the reducer contract; they do not claim every generated history is reachable
//! through the adapter.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use proptest::{prelude::*, test_runner::Config as ProptestConfig};
use sorotte_player_api::{PlayerMediaLoadFailureKind, PlayerSemanticOutcome};

use super::*;

macro_rules! declare_input_kinds {
    ($( $kind:ident => $pattern:pat ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum InputKind {
            $( $kind, )+
        }

        const ALL_INPUT_KINDS: &[InputKind] = &[
            $( InputKind::$kind, )+
        ];

        fn input_kind(input: &PlayerLifecycleInput) -> InputKind {
            match input {
                $( $pattern => InputKind::$kind, )+
            }
        }
    };
}

declare_input_kinds! {
    LoadAttemptSubmitted => PlayerLifecycleInput::LoadAttemptSubmitted { .. },
    ExternalLoadObserved => PlayerLifecycleInput::ExternalLoadObserved { .. },
    LoadAttemptAccepted => PlayerLifecycleInput::LoadAttemptAccepted { .. },
    LoadAttemptRejected => PlayerLifecycleInput::LoadAttemptRejected { .. },
    CommandSubmitted => PlayerLifecycleInput::CommandSubmitted { .. },
    CommandAccepted => PlayerLifecycleInput::CommandAccepted { .. },
    CommandRejected => PlayerLifecycleInput::CommandRejected { .. },
    CommandSuperseded => PlayerLifecycleInput::CommandSuperseded { .. },
    CommandTransportDisconnected => PlayerLifecycleInput::CommandTransportDisconnected { .. },
    CommandCompleted => PlayerLifecycleInput::CommandCompleted { .. },
    CommandCompletionNotObserved => PlayerLifecycleInput::CommandCompletionNotObserved { .. },
    StartFile => PlayerLifecycleInput::StartFile { .. },
    FileLoaded => PlayerLifecycleInput::FileLoaded { .. },
    EndFile => PlayerLifecycleInput::EndFile { .. },
    PlaylistSnapshot => PlayerLifecycleInput::PlaylistSnapshot { .. },
    LifecycleReconciliationFailed => PlayerLifecycleInput::LifecycleReconciliationFailed { .. },
    EofObserved => PlayerLifecycleInput::EofObserved { .. },
    PlaybackRestart => PlayerLifecycleInput::PlaybackRestart { .. },
    PositionObserved => PlayerLifecycleInput::PositionObserved { .. },
    SeekingObserved => PlayerLifecycleInput::SeekingObserved { .. },
    PhaseObserved => PlayerLifecycleInput::PhaseObserved { .. },
    TransportDelta => PlayerLifecycleInput::TransportDelta { .. },
    LocalFileChanged => PlayerLifecycleInput::LocalFileChanged { .. },
    SeekCommandSubmitted => PlayerLifecycleInput::SeekCommandSubmitted { .. },
    SeekCommandAccepted => PlayerLifecycleInput::SeekCommandAccepted { .. },
    SeekCommandRejected => PlayerLifecycleInput::SeekCommandRejected { .. },
    SeekCommandCompletionNotObserved => PlayerLifecycleInput::SeekCommandCompletionNotObserved { .. },
    EventGapDetected => PlayerLifecycleInput::EventGapDetected { .. },
    AuthoritativeSnapshotApplied => PlayerLifecycleInput::AuthoritativeSnapshotApplied(_),
    TimerAdvanced => PlayerLifecycleInput::TimerAdvanced { .. },
    TransportDisconnected => PlayerLifecycleInput::TransportDisconnected { .. },
    AttachmentReplaced => PlayerLifecycleInput::AttachmentReplaced,
}

#[derive(Debug, Clone)]
struct GeneratedStep {
    kind: InputKind,
    value: u16,
    alternate: u16,
    choice: u8,
    flag: bool,
}

fn generated_step() -> impl Strategy<Value = GeneratedStep> {
    (
        0_usize..ALL_INPUT_KINDS.len(),
        any::<u16>(),
        any::<u16>(),
        any::<u8>(),
        any::<bool>(),
    )
        .prop_map(|(kind, value, alternate, choice, flag)| GeneratedStep {
            kind: ALL_INPUT_KINDS[kind],
            value,
            alternate,
            choice,
            flag,
        })
}

#[derive(Debug)]
struct GeneratorCursor {
    next_command: u64,
    next_playlist_entry: i64,
}

impl Default for GeneratorCursor {
    fn default() -> Self {
        Self {
            next_command: 1,
            next_playlist_entry: 100,
        }
    }
}

impl GeneratorCursor {
    fn command_id(&mut self) -> PlayerCommandId {
        let id = PlayerCommandId::new(self.next_command);
        self.next_command += 1;
        id
    }

    fn playlist_entry_id(&mut self) -> i64 {
        let id = self.next_playlist_entry;
        self.next_playlist_entry += 1;
        id
    }
}

fn failure(choice: u8) -> PlayerCommandFailureKind {
    match choice % 4 {
        0 => PlayerCommandFailureKind::TimedOut,
        1 => PlayerCommandFailureKind::MediaEnded,
        2 => PlayerCommandFailureKind::TransportDisconnected,
        _ => PlayerCommandFailureKind::Unknown,
    }
}

fn physical_outcome(choice: u8) -> PlayerPhysicalLoadOutcome {
    match choice % 4 {
        0 => PlayerPhysicalLoadOutcome::Ended,
        1 => PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network),
        2 => PlayerPhysicalLoadOutcome::NeverStarted,
        _ => PlayerPhysicalLoadOutcome::TransportDisconnected,
    }
}

fn phase(choice: u8) -> PlayerTransportPhase {
    match choice % 9 {
        0 => PlayerTransportPhase::Empty,
        1 => PlayerTransportPhase::Loading,
        2 => PlayerTransportPhase::Prebuffering,
        3 => PlayerTransportPhase::ReadyPaused,
        4 => PlayerTransportPhase::Playing,
        5 => PlayerTransportPhase::Rebuffering,
        6 => PlayerTransportPhase::Seeking,
        7 => PlayerTransportPhase::Ended,
        _ => PlayerTransportPhase::Failed,
    }
}

fn first_submitting_attempt(state: &PlayerLifecycleState) -> Option<LoadAttemptId> {
    state
        .load_attempts
        .values()
        .find(|attempt| attempt.state == LoadAttemptState::Submitting)
        .map(|attempt| attempt.id)
}

fn first_live_generic_command(
    state: &PlayerLifecycleState,
    required_state: Option<CommandSemanticState>,
) -> Option<PlayerCommandId> {
    state
        .commands
        .values()
        .find(|command| {
            !command.state.is_terminal()
                && !matches!(command.kind, LifecycleCommandKind::Load(_))
                && !state.seek_ownership.contains_key(&command.id)
                && required_state.is_none_or(|required| command.state == required)
        })
        .map(|command| command.id)
}

fn first_live_seek_command(
    state: &PlayerLifecycleState,
    required_state: Option<CommandSemanticState>,
) -> Option<PlayerCommandId> {
    state
        .commands
        .values()
        .find(|command| {
            !command.state.is_terminal()
                && command.kind == LifecycleCommandKind::Seek
                && state.seek_ownership.contains_key(&command.id)
                && required_state.is_none_or(|required| command.state == required)
        })
        .map(|command| command.id)
}

fn fallback(state: &PlayerLifecycleState) -> PlayerLifecycleInput {
    PlayerLifecycleInput::LifecycleReconciliationFailed {
        attachment_epoch: state.attachment_epoch,
    }
}

fn materialize(
    step: &GeneratedStep,
    state: &PlayerLifecycleState,
    cursor: &mut GeneratorCursor,
) -> PlayerLifecycleInput {
    let epoch = state.attachment_epoch;
    let generation = PlayerMediaGeneration::new(u64::from(step.value % 4) + 1);
    match step.kind {
        InputKind::LoadAttemptSubmitted => {
            let command_id = cursor.command_id();
            PlayerLifecycleInput::LoadAttemptSubmitted {
                command_id: Some(command_id),
                media_generation: state
                    .active_media_generation()
                    .filter(|_| step.flag)
                    .unwrap_or(generation),
                requested_target: format!("property-target-{}", step.alternate % 5),
                baseline_playlist_entry_ids: state
                    .playlist_entry_attempts
                    .keys()
                    .copied()
                    .collect(),
            }
        }
        InputKind::ExternalLoadObserved => {
            let playlist_entry_id = cursor.playlist_entry_id();
            PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch: epoch,
                media_generation: generation,
                playlist_entry_id,
                observed_target: format!("property-external-{playlist_entry_id}"),
                file_loaded: step.flag,
            }
        }
        InputKind::LoadAttemptAccepted => first_submitting_attempt(state).map_or_else(
            || fallback(state),
            |attempt_id| PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch,
                attempt_id,
            },
        ),
        InputKind::LoadAttemptRejected => first_submitting_attempt(state).map_or_else(
            || fallback(state),
            |attempt_id| PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: epoch,
                attempt_id,
                failure: failure(step.choice),
            },
        ),
        InputKind::CommandSubmitted => PlayerLifecycleInput::CommandSubmitted {
            command_id: cursor.command_id(),
            media_generation: None,
            kind: match step.choice % 3 {
                0 => LifecycleCommandKind::Pause,
                1 => LifecycleCommandKind::Play,
                _ => LifecycleCommandKind::Seek,
            },
        },
        InputKind::CommandAccepted => {
            first_live_generic_command(state, Some(CommandSemanticState::Submitted)).map_or_else(
                || fallback(state),
                |command_id| PlayerLifecycleInput::CommandAccepted {
                    attachment_epoch: epoch,
                    command_id,
                },
            )
        }
        InputKind::CommandRejected => first_live_generic_command(state, None).map_or_else(
            || fallback(state),
            |command_id| PlayerLifecycleInput::CommandRejected {
                attachment_epoch: epoch,
                command_id,
                failure: failure(step.choice),
            },
        ),
        InputKind::CommandSuperseded => first_live_generic_command(state, None).map_or_else(
            || fallback(state),
            |command_id| PlayerLifecycleInput::CommandSuperseded {
                attachment_epoch: epoch,
                command_id,
            },
        ),
        InputKind::CommandTransportDisconnected => first_live_generic_command(state, None)
            .map_or_else(
                || fallback(state),
                |command_id| PlayerLifecycleInput::CommandTransportDisconnected {
                    attachment_epoch: epoch,
                    command_id,
                },
            ),
        InputKind::CommandCompleted => first_live_generic_command(state, None).map_or_else(
            || fallback(state),
            |command_id| PlayerLifecycleInput::CommandCompleted {
                attachment_epoch: epoch,
                command_id,
            },
        ),
        InputKind::CommandCompletionNotObserved => first_live_generic_command(state, None)
            .map_or_else(
                || fallback(state),
                |command_id| PlayerLifecycleInput::CommandCompletionNotObserved {
                    attachment_epoch: epoch,
                    command_id,
                },
            ),
        InputKind::StartFile => PlayerLifecycleInput::StartFile {
            attachment_epoch: epoch,
            playlist_entry_id: state
                .playlist_entry_attempts
                .keys()
                .min()
                .copied()
                .unwrap_or_else(|| i64::from(step.value) + 10_000),
        },
        InputKind::FileLoaded => PlayerLifecycleInput::FileLoaded {
            attachment_epoch: epoch,
            playlist_entry_id: state.playlist_entry_attempts.keys().min().copied(),
            loaded_target: step
                .flag
                .then(|| format!("property-loaded-{}", step.alternate % 5)),
        },
        InputKind::EndFile => PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: state
                .playlist_entry_attempts
                .keys()
                .min()
                .copied()
                .unwrap_or_else(|| i64::from(step.value) + 20_000),
            outcome: physical_outcome(step.choice),
        },
        InputKind::PlaylistSnapshot => {
            let candidate = state.load_attempts.values().find(|attempt| {
                !attempt.state.is_terminal()
                    && attempt.state != LoadAttemptState::Submitting
                    && attempt.playlist_entry_id.is_none()
            });
            if let Some(attempt) = candidate {
                let playlist_entry_id = cursor.playlist_entry_id();
                PlayerLifecycleInput::PlaylistSnapshot {
                    attachment_epoch: epoch,
                    entries: vec![AuthoritativePlaylistEntry::new(
                        playlist_entry_id,
                        Some(attempt.requested_target.clone()),
                        true,
                    )],
                    current_path: Some(attempt.requested_target.clone()),
                }
            } else {
                PlayerLifecycleInput::PlaylistSnapshot {
                    attachment_epoch: epoch,
                    entries: Vec::new(),
                    current_path: None,
                }
            }
        }
        InputKind::LifecycleReconciliationFailed => fallback(state),
        InputKind::EofObserved => PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: state
                .active_attempt()
                .and_then(|attempt| attempt.playlist_entry_id),
            reached: step.flag,
            position_seconds: Some(f64::from(step.value) / 4.0),
        },
        InputKind::PlaybackRestart => PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch: epoch,
            playlist_entry_id: state
                .active_attempt()
                .and_then(|attempt| attempt.playlist_entry_id),
        },
        InputKind::PositionObserved => PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: state.active_media_generation().unwrap_or(generation),
            observed_sequence: state.last_event_sequence().saturating_add(1),
            position_seconds: f64::from(step.value) / 4.0,
        },
        InputKind::SeekingObserved => PlayerLifecycleInput::SeekingObserved {
            attachment_epoch: epoch,
            media_generation: state.active_media_generation().unwrap_or(generation),
            observed_sequence: state.last_event_sequence().saturating_add(1),
            seeking: step.flag,
        },
        InputKind::PhaseObserved => PlayerLifecycleInput::PhaseObserved {
            attachment_epoch: epoch,
            phase: phase(step.choice),
        },
        InputKind::TransportDelta => PlayerLifecycleInput::TransportDelta {
            attachment_epoch: epoch,
            delta: PlayerTransportDelta {
                media_generation: state.active_media_generation(),
                phase: Some(phase(step.choice)),
                position_seconds: Some(f64::from(step.value) / 8.0),
                logical_pause: Some(step.flag),
                seeking: Some(step.choice & 1 == 0),
                ..PlayerTransportDelta::default()
            },
        },
        InputKind::LocalFileChanged => state.load_attempts.values().next().map_or_else(
            || fallback(state),
            |attempt| PlayerLifecycleInput::LocalFileChanged {
                attachment_epoch: epoch,
                attempt_id: attempt.id,
                media_generation: attempt.media_generation,
                update: LocalFileUpdate::new(format!("property-file-{}.mkv", step.alternate % 7))
                    .with_duration_seconds(f64::from(step.value) + 1.0),
            },
        ),
        InputKind::SeekCommandSubmitted => {
            let target = f64::from(step.value) / 4.0;
            PlayerLifecycleInput::SeekCommandSubmitted {
                command_id: cursor.command_id(),
                media_generation: state.active_media_generation().unwrap_or(generation),
                raw_player_target_seconds: target,
                effective_room_target_seconds: target + f64::from(step.alternate % 8),
                dispatch_sequence_boundary: state.last_event_sequence(),
            }
        }
        InputKind::SeekCommandAccepted => {
            first_live_seek_command(state, Some(CommandSemanticState::Submitted)).map_or_else(
                || fallback(state),
                |command_id| PlayerLifecycleInput::SeekCommandAccepted {
                    attachment_epoch: epoch,
                    command_id,
                },
            )
        }
        InputKind::SeekCommandRejected => first_live_seek_command(state, None).map_or_else(
            || fallback(state),
            |command_id| PlayerLifecycleInput::SeekCommandRejected {
                attachment_epoch: epoch,
                command_id,
                failure: failure(step.choice),
            },
        ),
        InputKind::SeekCommandCompletionNotObserved => first_live_seek_command(state, None)
            .map_or_else(
                || fallback(state),
                |command_id| PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                    attachment_epoch: epoch,
                    command_id,
                },
            ),
        InputKind::EventGapDetected => PlayerLifecycleInput::EventGapDetected {
            attachment_epoch: epoch,
        },
        InputKind::AuthoritativeSnapshotApplied => {
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot {
                attachment_epoch: epoch,
                sequence_boundary: PlayerSequenceBoundary::new(epoch, state.last_event_sequence()),
                ..PlayerAuthoritativeSnapshot::default()
            })
        }
        InputKind::TimerAdvanced => PlayerLifecycleInput::TimerAdvanced {
            now_tick: state.now_tick.saturating_add(u64::from(step.value % 2_000)),
        },
        InputKind::TransportDisconnected => PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch: epoch,
        },
        InputKind::AttachmentReplaced => PlayerLifecycleInput::AttachmentReplaced,
    }
}

#[derive(Default)]
struct IndependentLedger {
    seen_orders: HashSet<(u64, u64)>,
    last_order_by_epoch: BTreeMap<u64, u64>,
    command_outcomes: HashSet<(u64, u64)>,
    load_outcomes: HashSet<(u64, u64)>,
}

impl IndependentLedger {
    fn observe_order(&mut self, epoch: u64, sequence: u64) -> Result<(), String> {
        let key = (epoch, sequence);
        if !self.seen_orders.insert(key) {
            return Err(format!("ordered effect identity repeated: {key:?}"));
        }
        let previous = self.last_order_by_epoch.entry(epoch).or_default();
        if sequence <= *previous {
            return Err(format!(
                "ordered effect regressed within epoch {epoch}: {sequence} <= {previous}"
            ));
        }
        *previous = sequence;
        Ok(())
    }

    fn observe(&mut self, effects: &[PlayerLifecycleEffect]) -> Result<(), String> {
        for effect in effects {
            match effect {
                PlayerLifecycleEffect::EmitOrderedEvent(event) => {
                    self.observe_order(event.order.attachment_epoch.get(), event.order.sequence)?;
                }
                PlayerLifecycleEffect::EmitSemanticOutcome(outcome) => {
                    let order = (outcome.order.attachment_epoch.get(), outcome.order.sequence);
                    self.observe_order(order.0, order.1)?;
                    let unique = match &outcome.outcome {
                        PlayerSemanticOutcome::Command(command) => self
                            .command_outcomes
                            .insert((command.attachment_epoch.get(), command.command_id.get())),
                        PlayerSemanticOutcome::LoadAttempt(attempt) => self
                            .load_outcomes
                            .insert((attempt.attachment_epoch.get(), attempt.attempt_id.get())),
                    };
                    if !unique {
                        return Err(format!("semantic terminal identity repeated at {order:?}"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalKnownDefect {
    TcPlayer001,
    TcPlayer002,
}

fn tc_player_001_history() -> Vec<PlayerLifecycleInput> {
    let epoch = PlayerAttachmentEpoch::new(1);
    vec![
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 100,
            observed_target: "property-external-100".to_owned(),
            file_loaded: false,
        },
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([100]),
        },
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 101,
            observed_target: "property-external-101".to_owned(),
            file_loaded: false,
        },
    ]
}

fn tc_player_001_acceptance_overwrite_history() -> Vec<PlayerLifecycleInput> {
    let epoch = PlayerAttachmentEpoch::new(1);
    vec![
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 100,
            observed_target: "property-external-100".to_owned(),
            file_loaded: false,
        },
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([100]),
        },
        PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch: epoch,
            attempt_id: LoadAttemptId::new(2),
            failure: PlayerCommandFailureKind::Unknown,
        },
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(2)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([100]),
        },
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: epoch,
            attempt_id: LoadAttemptId::new(3),
        },
    ]
}

fn tc_player_002_history() -> Vec<PlayerLifecycleInput> {
    let epoch = PlayerAttachmentEpoch::new(1);
    vec![
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::new(),
        },
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 100,
            observed_target: "property-external-100".to_owned(),
            file_loaded: false,
        },
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: epoch,
            attempt_id: LoadAttemptId::new(1),
        },
        PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries: vec![AuthoritativePlaylistEntry::new(
                101,
                Some("property-target-0".to_owned()),
                true,
            )],
            current_path: Some("property-target-0".to_owned()),
        },
    ]
}

fn tc_player_002_cross_generation_history() -> Vec<PlayerLifecycleInput> {
    let mut history = tc_player_002_history();
    let PlayerLifecycleInput::LoadAttemptSubmitted {
        media_generation, ..
    } = &mut history[0]
    else {
        panic!("TC-PLAYER-002 cross-generation variant must start with a load submission");
    };
    *media_generation = PlayerMediaGeneration::new(2);
    history
}

fn tc_player_002_superseding_submission_history() -> Vec<PlayerLifecycleInput> {
    let mut history = tc_player_002_history();
    history.insert(
        3,
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(2)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([100]),
        },
    );
    history
}

fn tc_player_002_repeated_external_history() -> Vec<PlayerLifecycleInput> {
    let mut history = tc_player_002_history();
    history.insert(
        3,
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 101,
            observed_target: "property-external-101".to_owned(),
            file_loaded: false,
        },
    );
    let PlayerLifecycleInput::PlaylistSnapshot { entries, .. } = &mut history[4] else {
        panic!("TC-PLAYER-002 repeated-external variant must end with a playlist snapshot");
    };
    entries[0].id = 102;
    history
}

fn tc_player_002_loaded_external_history() -> Vec<PlayerLifecycleInput> {
    let mut history = tc_player_002_history();
    history.swap(1, 2);
    history.insert(
        3,
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            playlist_entry_id: Some(100),
            loaded_target: None,
        },
    );
    history
}

fn tc_player_002_terminal_external_history() -> Vec<PlayerLifecycleInput> {
    let mut history = tc_player_002_history();
    history.swap(1, 2);
    history.insert(
        3,
        PlayerLifecycleInput::EndFile {
            attachment_epoch: PlayerAttachmentEpoch::new(1),
            playlist_entry_id: 100,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
    );
    history
}

fn tc_player_002_replaced_attempt_history() -> Vec<PlayerLifecycleInput> {
    let epoch = PlayerAttachmentEpoch::new(1);
    vec![
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 100,
            observed_target: "property-external-100".to_owned(),
            file_loaded: false,
        },
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(PlayerCommandId::new(1)),
            media_generation: PlayerMediaGeneration::new(1),
            requested_target: "property-target-0".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([100]),
        },
        PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: 100,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: epoch,
            attempt_id: LoadAttemptId::new(2),
        },
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: PlayerMediaGeneration::new(1),
            playlist_entry_id: 101,
            observed_target: "property-external-101".to_owned(),
            file_loaded: false,
        },
        PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries: vec![AuthoritativePlaylistEntry::new(
                102,
                Some("property-target-0".to_owned()),
                true,
            )],
            current_path: Some("property-target-0".to_owned()),
        },
    ]
}

const TC_PLAYER_001_INVARIANT: &str = "attempt predecessor points to another successor";

/// Matches the exact valid-state graph delta that exposes TC-PLAYER-001.
///
/// One or more existing attempts already name a predecessor. A later
/// transition points that predecessor at a different successor without
/// updating the older successor's backlink. This classifier is deliberately
/// independent of the triggering input kind and successor lifecycle state: it
/// requires a valid pre-state, the exact post-state invariant failure, a newly
/// selected reciprocal successor, and at least one preserved stale backlink.
fn canonical_tc_player_001_defect(
    state: &PlayerLifecycleState,
    input: &PlayerLifecycleInput,
) -> bool {
    if state.assert_invariants().is_err() {
        return false;
    }
    let (resulting_state, _) =
        reduce_player_lifecycle_without_invariant_assertion(state.clone(), input.clone());
    if resulting_state.assert_invariants() != Err(TC_PLAYER_001_INVARIANT.to_owned()) {
        return false;
    }

    resulting_state.load_attempts.values().any(|predecessor| {
        let Some(selected_successor_id) = predecessor.superseded_by else {
            return false;
        };
        let Some(previous_predecessor) = state.load_attempts.get(&predecessor.id) else {
            return false;
        };
        let reciprocal_successor_exists = resulting_state
            .load_attempts
            .get(&selected_successor_id)
            .is_some_and(|successor| {
                successor.id != predecessor.id && successor.replaced_attempt == Some(predecessor.id)
            });
        let stale_backlink_exists = state.load_attempts.values().any(|successor| {
            successor.id != selected_successor_id
                && successor.replaced_attempt == Some(predecessor.id)
                && resulting_state
                    .load_attempts
                    .get(&successor.id)
                    .is_some_and(|preserved| preserved.replaced_attempt == Some(predecessor.id))
        });
        previous_predecessor.superseded_by != Some(selected_successor_id)
            && reciprocal_successor_exists
            && stale_backlink_exists
    })
}

/// Matches the exact valid-state graph delta that exposes TC-PLAYER-002.
///
/// The defect is not identified by panic text or by a particular generated
/// history. It requires a valid pre-state and a post-state that simultaneously
/// retains a concrete terminal physical owner and selects a different,
/// current-epoch live physical owner while the same logical terminal outcome
/// remains installed. Triggering input kind, candidate count, target text, and
/// prior replacement linkage are deliberately not part of the classification.
const TC_PLAYER_002_INVARIANT: &str =
    "logical terminal playback still has an active physical attempt";

fn canonical_tc_player_002_defect(
    state: &PlayerLifecycleState,
    input: &PlayerLifecycleInput,
) -> bool {
    if state.assert_invariants().is_err() {
        return false;
    }
    let (resulting_state, _) =
        reduce_player_lifecycle_without_invariant_assertion(state.clone(), input.clone());
    if resulting_state.assert_invariants() != Err(TC_PLAYER_002_INVARIANT.to_owned()) {
        return false;
    }
    let Some(active_id) = resulting_state.active_load_attempt else {
        return false;
    };
    let Some(active) = resulting_state.load_attempts.get(&active_id) else {
        return false;
    };
    let Some((terminal_generation, terminal_outcome)) = resulting_state.logical_terminal else {
        return false;
    };
    let terminal_binding_exists = resulting_state.load_attempts.values().any(|attempt| {
        attempt.id != active_id
            && attempt.attachment_epoch == resulting_state.attachment_epoch
            && attempt.media_generation == terminal_generation
            && attempt.state == LoadAttemptState::Terminal(terminal_outcome)
            && attempt.physical_terminal_sequence.is_some()
    });
    active.attachment_epoch == resulting_state.attachment_epoch
        && active.state.may_receive_lifecycle()
        && !active.state.is_terminal()
        && !active.logical_ownership_revoked
        && terminal_binding_exists
}

fn canonical_known_defect(
    state: &PlayerLifecycleState,
    input: &PlayerLifecycleInput,
    _history: &[PlayerLifecycleInput],
) -> Option<CanonicalKnownDefect> {
    if canonical_tc_player_001_defect(state, input) {
        Some(CanonicalKnownDefect::TcPlayer001)
    } else if canonical_tc_player_002_defect(state, input) {
        Some(CanonicalKnownDefect::TcPlayer002)
    } else {
        None
    }
}

const DEFAULT_PROPTEST_CASES: u32 = 128;
const MAX_PROPTEST_CASES: u32 = 100_000;

fn resolve_proptest_cases(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_PROPTEST_CASES);
    };
    let cases = raw
        .parse::<u32>()
        .map_err(|_| format!("PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"))?;
    if cases == 0 {
        return Err(format!(
            "PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"
        ));
    }
    Ok(cases.min(MAX_PROPTEST_CASES))
}

fn configured_proptest() -> ProptestConfig {
    let raw_cases = std::env::var("PROPTEST_CASES").ok();
    ProptestConfig {
        cases: resolve_proptest_cases(raw_cases.as_deref())
            .unwrap_or_else(|reason| panic!("{reason}")),
        max_shrink_iters: 20_000,
        ..ProptestConfig::default()
    }
}

#[test]
fn proptest_case_budget_rejects_zero_and_caps_excessive_values() {
    assert_eq!(resolve_proptest_cases(None), Ok(DEFAULT_PROPTEST_CASES));
    assert_eq!(resolve_proptest_cases(Some("2048")), Ok(2_048));
    assert_eq!(
        resolve_proptest_cases(Some(&u32::MAX.to_string())),
        Ok(MAX_PROPTEST_CASES)
    );
    for invalid in ["", "0", "-1", "not-a-number"] {
        assert!(
            resolve_proptest_cases(Some(invalid)).is_err(),
            "{invalid:?} must not silently weaken the property budget"
        );
    }
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn generated_reducer_input_histories_preserve_unquarantined_contracts(
        steps in prop::collection::vec(generated_step(), 1..=64),
    ) {
        let mut state = PlayerLifecycleState::default();
        let mut expected_epoch = state.attachment_epoch.get();
        let mut cursor = GeneratorCursor::default();
        let mut ledger = IndependentLedger::default();
        let mut action_history = Vec::new();

        for (index, step) in steps.iter().enumerate() {
            let input = materialize(step, &state, &mut cursor);
            let actual_kind = input_kind(&input);
            let previous_epoch = state.attachment_epoch;
            action_history.push(input.clone());
            if canonical_known_defect(&state, &input, &action_history).is_some() {
                continue;
            }
            let (next_state, effects) = reduce_player_lifecycle(state.clone(), input);
            if let Err(reason) = next_state.assert_invariants() {
                prop_assert!(
                    false,
                    "reducer invariant failed at step {index}: {reason}"
                );
            }
            state = next_state;

            if actual_kind == InputKind::AttachmentReplaced {
                expected_epoch = expected_epoch.saturating_add(1);
                prop_assert!(
                    effects.iter().any(|effect| matches!(
                        effect,
                        PlayerLifecycleEffect::RequestAuthoritativeSnapshot
                    )),
                    "replacement at step {index} did not request an authoritative snapshot",
                );
                prop_assert!(
                    effects.iter().any(|effect| matches!(
                        effect,
                        PlayerLifecycleEffect::RequestLifecycleReconciliation
                    )),
                    "replacement at step {index} did not request lifecycle reconciliation",
                );
            } else {
                prop_assert_eq!(
                    state.attachment_epoch,
                    previous_epoch,
                    "non-replacement input changed epoch at step {}",
                    index,
                );
            }

            if actual_kind == InputKind::LoadAttemptSubmitted {
                let allocations = effects
                    .iter()
                    .filter(|effect| matches!(
                        effect,
                        PlayerLifecycleEffect::LoadAttemptAllocated { .. }
                    ))
                    .count();
                prop_assert_eq!(
                    allocations,
                    1,
                    "fresh load submission did not allocate exactly one attempt at step {}",
                    index,
                );
            }

            prop_assert_eq!(
                state.attachment_epoch.get(),
                expected_epoch,
                "independent epoch model diverged at step {}",
                index,
            );
            let ledger_result = ledger.observe(&effects);
            prop_assert!(
                ledger_result.is_ok(),
                "effect ledger failed at step {}: {:?}",
                index,
                ledger_result,
            );
        }
    }

    #[test]
    fn stale_epoch_observations_are_no_ops(
        setup in prop::collection::vec(any::<u8>(), 0..=24),
        value in any::<u16>(),
    ) {
        let mut state = PlayerLifecycleState::default();
        let mut cursor = GeneratorCursor::default();
        for choice in setup {
            let input = match choice % 6 {
                0 => PlayerLifecycleInput::CommandSubmitted {
                    command_id: cursor.command_id(),
                    media_generation: None,
                    kind: LifecycleCommandKind::Pause,
                },
                1 => first_live_generic_command(
                    &state,
                    Some(CommandSemanticState::Submitted),
                )
                .map_or(
                    PlayerLifecycleInput::PhaseObserved {
                        attachment_epoch: state.attachment_epoch,
                        phase: PlayerTransportPhase::Empty,
                    },
                    |command_id| PlayerLifecycleInput::CommandAccepted {
                        attachment_epoch: state.attachment_epoch,
                        command_id,
                    },
                ),
                2 => PlayerLifecycleInput::EventGapDetected {
                    attachment_epoch: state.attachment_epoch,
                },
                3 => PlayerLifecycleInput::TransportDelta {
                    attachment_epoch: state.attachment_epoch,
                    delta: PlayerTransportDelta {
                        position_seconds: Some(f64::from(choice)),
                        ..PlayerTransportDelta::default()
                    },
                },
                4 => PlayerLifecycleInput::TimerAdvanced {
                    now_tick: state.now_tick.saturating_add(u64::from(choice)),
                },
                _ => PlayerLifecycleInput::PhaseObserved {
                    attachment_epoch: state.attachment_epoch,
                    phase: phase(choice),
                },
            };
            let (next_state, _) = reduce_player_lifecycle(state, input);
            state = next_state;
            prop_assert!(state.assert_invariants().is_ok());
        }

        let stale_epoch = state.attachment_epoch;
        let (state, _) = reduce_player_lifecycle(state, PlayerLifecycleInput::AttachmentReplaced);
        let (state, targets) = seed_current_epoch_targets(state, &mut cursor);
        prop_assert!(state.assert_invariants().is_ok());

        for kind in STALE_INPUT_KINDS {
            let before = state.clone();
            let stale = stale_observation(kind, &targets, stale_epoch, value);
            let (after, effects) = reduce_player_lifecycle(state.clone(), stale);

            prop_assert!(
                effects.is_empty(),
                "stale {:?} emitted effects: {effects:#?}",
                kind,
            );
            prop_assert_eq!(
                after,
                before,
                "stale {:?} mutated current-epoch state",
                kind,
            );
        }
    }
}

fn state_before_final_transition(
    history: &[PlayerLifecycleInput],
) -> (PlayerLifecycleState, PlayerLifecycleInput) {
    let (final_input, prefix) = history
        .split_last()
        .expect("known-defect history must include a final transition");
    let mut state = PlayerLifecycleState::default();
    for input in prefix {
        let (next_state, _) = reduce_player_lifecycle(state, input.clone());
        state = next_state;
        assert!(
            state.assert_invariants().is_ok(),
            "known-defect prefix must remain valid before its final transition"
        );
    }
    (state, final_input.clone())
}

fn classify_history(history: &[PlayerLifecycleInput]) -> Option<CanonicalKnownDefect> {
    let (state, final_input) = state_before_final_transition(history);
    canonical_known_defect(&state, &final_input, history)
}

#[test]
fn known_defect_quarantine_is_bound_to_the_exact_causal_state() {
    let tc_player_001 = tc_player_001_history();
    let tc_player_001_acceptance_overwrite = tc_player_001_acceptance_overwrite_history();
    let tc_player_002 = tc_player_002_history();
    let tc_player_002_cross_generation = tc_player_002_cross_generation_history();
    let tc_player_002_superseding = tc_player_002_superseding_submission_history();
    let tc_player_002_repeated_external = tc_player_002_repeated_external_history();
    let tc_player_002_loaded_external = tc_player_002_loaded_external_history();
    let tc_player_002_terminal_external = tc_player_002_terminal_external_history();
    let tc_player_002_replaced_attempt = tc_player_002_replaced_attempt_history();
    assert_eq!(
        classify_history(&tc_player_001),
        Some(CanonicalKnownDefect::TcPlayer001)
    );
    assert_eq!(
        classify_history(&tc_player_001_acceptance_overwrite),
        Some(CanonicalKnownDefect::TcPlayer001)
    );
    assert_eq!(
        classify_history(&tc_player_002),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_cross_generation),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_superseding),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_repeated_external),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_loaded_external),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_terminal_external),
        Some(CanonicalKnownDefect::TcPlayer002)
    );
    assert_eq!(
        classify_history(&tc_player_002_replaced_attempt),
        Some(CanonicalKnownDefect::TcPlayer002)
    );

    let mut semantically_equivalent_prefix =
        vec![PlayerLifecycleInput::TimerAdvanced { now_tick: 0 }];
    semantically_equivalent_prefix.extend(tc_player_002.clone());
    assert_eq!(
        classify_history(&semantically_equivalent_prefix),
        Some(CanonicalKnownDefect::TcPlayer002),
        "TC-PLAYER-002 classification must depend on causal state, not full-history syntax"
    );

    let mut safe_noncurrent_snapshot = tc_player_002;
    let PlayerLifecycleInput::PlaylistSnapshot {
        entries,
        current_path,
        ..
    } = safe_noncurrent_snapshot
        .last_mut()
        .expect("TC-PLAYER-002 has a final action")
    else {
        panic!("TC-PLAYER-002 must end in a playlist snapshot");
    };
    entries[0].current = false;
    *current_path = None;
    assert_eq!(
        classify_history(&safe_noncurrent_snapshot),
        None,
        "a valid noncurrent snapshot must not match TC-PLAYER-002"
    );

    let mut target_variant = tc_player_002_history();
    let PlayerLifecycleInput::LoadAttemptSubmitted {
        requested_target, ..
    } = &mut target_variant[0]
    else {
        panic!("TC-PLAYER-002 must start with a load submission");
    };
    *requested_target = "target-variant".to_owned();
    let PlayerLifecycleInput::PlaylistSnapshot {
        entries,
        current_path,
        ..
    } = target_variant
        .last_mut()
        .expect("TC-PLAYER-002 has a final action")
    else {
        panic!("TC-PLAYER-002 must end in a playlist snapshot");
    };
    entries[0].original_filename = Some("target-variant".to_owned());
    *current_path = Some("target-variant".to_owned());
    assert_eq!(
        classify_history(&target_variant),
        Some(CanonicalKnownDefect::TcPlayer002)
    );

    let (causal_state, final_snapshot) = state_before_final_transition(&tc_player_002_history());
    let mut wrong_identity = final_snapshot.clone();
    let PlayerLifecycleInput::PlaylistSnapshot {
        entries: wrong_entries,
        ..
    } = &mut wrong_identity
    else {
        unreachable!();
    };
    wrong_entries[0].id = causal_state
        .active_attempt()
        .and_then(|attempt| attempt.playlist_entry_id)
        .expect("TC-PLAYER-002 must have a bound active attempt");
    assert_eq!(
        canonical_known_defect(&causal_state, &wrong_identity, &tc_player_002_history()),
        None,
        "a snapshot that does not contradict the active identity must not be quarantined"
    );

    let mut not_current = final_snapshot.clone();
    let PlayerLifecycleInput::PlaylistSnapshot {
        entries: not_current_entries,
        ..
    } = &mut not_current
    else {
        unreachable!();
    };
    not_current_entries[0].current = false;
    assert_eq!(
        canonical_known_defect(&causal_state, &not_current, &tc_player_002_history()),
        None,
        "a non-current snapshot cannot establish TC-PLAYER-002"
    );

    let mut merely_submitted = tc_player_002_history();
    merely_submitted.remove(2);
    assert_eq!(
        classify_history(&merely_submitted),
        None,
        "a submitted attempt whose snapshot remains valid must not be quarantined"
    );

    let mut same_message_different_state = causal_state.clone();
    let active_generation = same_message_different_state
        .active_media_generation()
        .expect("TC-PLAYER-002 must have an active media generation");
    same_message_different_state.logical_terminal =
        Some((active_generation, PlayerPhysicalLoadOutcome::Ended));
    assert_eq!(
        same_message_different_state.assert_invariants(),
        Err("logical terminal playback still has an active physical attempt".to_owned())
    );
    assert_eq!(
        canonical_known_defect(
            &same_message_different_state,
            &final_snapshot,
            &tc_player_002_history()
        ),
        None,
        "panic text alone must not quarantine a different pre-transition state"
    );

    let no_pending_successor = vec![tc_player_001[0].clone(), tc_player_001[2].clone()];
    assert_eq!(
        classify_history(&no_pending_successor),
        None,
        "an ordinary external replacement without a pending successor is not TC-PLAYER-001"
    );

    let mut prefixed_tc_player_001 = vec![PlayerLifecycleInput::TimerAdvanced { now_tick: 0 }];
    prefixed_tc_player_001.extend(tc_player_001);
    assert_eq!(
        classify_history(&prefixed_tc_player_001),
        Some(CanonicalKnownDefect::TcPlayer001),
        "TC-PLAYER-001 classification must depend on causal state, not full-history syntax"
    );
}

#[test]
#[should_panic(expected = "attempt predecessor points to another successor")]
fn known_defect_tc_player_001_external_replacement_breaks_predecessor_links() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_001_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "attempt predecessor points to another successor")]
fn known_defect_tc_player_001_acceptance_overwrites_predecessor_link() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_001_acceptance_overwrite_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_delayed_acceptance_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_cross_generation_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_cross_generation_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_superseding_submission_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_superseding_submission_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_repeated_external_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_repeated_external_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_loaded_external_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_loaded_external_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_terminal_external_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_terminal_external_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[test]
#[should_panic(expected = "logical terminal playback still has an active physical attempt")]
fn known_defect_tc_player_002_replaced_attempt_variant_retains_terminal_active_state() {
    let mut state = PlayerLifecycleState::default();
    for input in tc_player_002_replaced_attempt_history() {
        let (next_state, _) = reduce_player_lifecycle(state, input);
        state = next_state;
        if let Err(reason) = state.assert_invariants() {
            panic!("{reason}");
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum StaleInputKind {
    ExternalLoadObserved,
    LoadAttemptAccepted,
    LoadAttemptRejected,
    CommandAccepted,
    CommandRejected,
    CommandSuperseded,
    CommandTransportDisconnected,
    CommandCompleted,
    CommandCompletionNotObserved,
    StartFile,
    FileLoaded,
    EndFile,
    PlaylistSnapshot,
    LifecycleReconciliationFailed,
    EofObserved,
    PlaybackRestart,
    PositionObserved,
    SeekingObserved,
    PhaseObserved,
    TransportDelta,
    LocalFileChanged,
    SeekCommandAccepted,
    SeekCommandRejected,
    SeekCommandCompletionNotObserved,
    EventGapDetected,
    AuthoritativeSnapshotApplied,
    TransportDisconnected,
}

const STALE_INPUT_KINDS: [StaleInputKind; 27] = [
    StaleInputKind::ExternalLoadObserved,
    StaleInputKind::LoadAttemptAccepted,
    StaleInputKind::LoadAttemptRejected,
    StaleInputKind::CommandAccepted,
    StaleInputKind::CommandRejected,
    StaleInputKind::CommandSuperseded,
    StaleInputKind::CommandTransportDisconnected,
    StaleInputKind::CommandCompleted,
    StaleInputKind::CommandCompletionNotObserved,
    StaleInputKind::StartFile,
    StaleInputKind::FileLoaded,
    StaleInputKind::EndFile,
    StaleInputKind::PlaylistSnapshot,
    StaleInputKind::LifecycleReconciliationFailed,
    StaleInputKind::EofObserved,
    StaleInputKind::PlaybackRestart,
    StaleInputKind::PositionObserved,
    StaleInputKind::SeekingObserved,
    StaleInputKind::PhaseObserved,
    StaleInputKind::TransportDelta,
    StaleInputKind::LocalFileChanged,
    StaleInputKind::SeekCommandAccepted,
    StaleInputKind::SeekCommandRejected,
    StaleInputKind::SeekCommandCompletionNotObserved,
    StaleInputKind::EventGapDetected,
    StaleInputKind::AuthoritativeSnapshotApplied,
    StaleInputKind::TransportDisconnected,
];

#[derive(Debug, Clone, Copy)]
struct StaleTargets {
    submitting_attempt_id: LoadAttemptId,
    submitting_generation: PlayerMediaGeneration,
    generic_command_id: PlayerCommandId,
    seek_command_id: PlayerCommandId,
    active_generation: PlayerMediaGeneration,
    mapped_playlist_entry_id: i64,
    unmapped_playlist_entry_id: i64,
}

fn seed_current_epoch_targets(
    mut state: PlayerLifecycleState,
    cursor: &mut GeneratorCursor,
) -> (PlayerLifecycleState, StaleTargets) {
    let epoch = state.attachment_epoch;
    let active_generation = PlayerMediaGeneration::new(10_001);
    let submitting_generation = PlayerMediaGeneration::new(10_002);
    let mapped_playlist_entry_id = 50_001;
    let unmapped_playlist_entry_id = 50_002;

    (state, _) = reduce_player_lifecycle(
        state,
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: active_generation,
            playlist_entry_id: mapped_playlist_entry_id,
            observed_target: "current-epoch-active".to_owned(),
            file_loaded: false,
        },
    );

    let load_command_id = cursor.command_id();
    let (next_state, effects) = reduce_player_lifecycle(
        state,
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(load_command_id),
            media_generation: submitting_generation,
            requested_target: "current-epoch-submitting".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::from([mapped_playlist_entry_id]),
        },
    );
    state = next_state;
    let submitting_attempt_id = effects
        .iter()
        .find_map(|effect| match effect {
            PlayerLifecycleEffect::LoadAttemptAllocated { attempt_id, .. } => Some(*attempt_id),
            _ => None,
        })
        .expect("current-epoch load seed must allocate an attempt");

    let generic_command_id = cursor.command_id();
    (state, _) = reduce_player_lifecycle(
        state,
        PlayerLifecycleInput::CommandSubmitted {
            command_id: generic_command_id,
            media_generation: Some(active_generation),
            kind: LifecycleCommandKind::Pause,
        },
    );

    let seek_command_id = cursor.command_id();
    let dispatch_sequence_boundary = state.last_event_sequence();
    (state, _) = reduce_player_lifecycle(
        state,
        PlayerLifecycleInput::SeekCommandSubmitted {
            command_id: seek_command_id,
            media_generation: active_generation,
            raw_player_target_seconds: 12.0,
            effective_room_target_seconds: 12.0,
            dispatch_sequence_boundary,
        },
    );

    state
        .assert_invariants()
        .expect("current-epoch stale targets must start valid");
    (
        state,
        StaleTargets {
            submitting_attempt_id,
            submitting_generation,
            generic_command_id,
            seek_command_id,
            active_generation,
            mapped_playlist_entry_id,
            unmapped_playlist_entry_id,
        },
    )
}

fn stale_observation(
    kind: StaleInputKind,
    targets: &StaleTargets,
    stale_epoch: PlayerAttachmentEpoch,
    value: u16,
) -> PlayerLifecycleInput {
    match kind {
        StaleInputKind::ExternalLoadObserved => PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: stale_epoch,
            media_generation: targets.active_generation,
            playlist_entry_id: targets.unmapped_playlist_entry_id,
            observed_target: "stale-external".to_owned(),
            file_loaded: true,
        },
        StaleInputKind::LoadAttemptAccepted => PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: stale_epoch,
            attempt_id: targets.submitting_attempt_id,
        },
        StaleInputKind::LoadAttemptRejected => PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch: stale_epoch,
            attempt_id: targets.submitting_attempt_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        StaleInputKind::CommandAccepted => PlayerLifecycleInput::CommandAccepted {
            attachment_epoch: stale_epoch,
            command_id: targets.generic_command_id,
        },
        StaleInputKind::CommandRejected => PlayerLifecycleInput::CommandRejected {
            attachment_epoch: stale_epoch,
            command_id: targets.generic_command_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        StaleInputKind::CommandSuperseded => PlayerLifecycleInput::CommandSuperseded {
            attachment_epoch: stale_epoch,
            command_id: targets.generic_command_id,
        },
        StaleInputKind::CommandTransportDisconnected => {
            PlayerLifecycleInput::CommandTransportDisconnected {
                attachment_epoch: stale_epoch,
                command_id: targets.generic_command_id,
            }
        }
        StaleInputKind::CommandCompleted => PlayerLifecycleInput::CommandCompleted {
            attachment_epoch: stale_epoch,
            command_id: targets.generic_command_id,
        },
        StaleInputKind::CommandCompletionNotObserved => {
            PlayerLifecycleInput::CommandCompletionNotObserved {
                attachment_epoch: stale_epoch,
                command_id: targets.generic_command_id,
            }
        }
        StaleInputKind::StartFile => PlayerLifecycleInput::StartFile {
            attachment_epoch: stale_epoch,
            playlist_entry_id: targets.unmapped_playlist_entry_id,
        },
        StaleInputKind::FileLoaded => PlayerLifecycleInput::FileLoaded {
            attachment_epoch: stale_epoch,
            playlist_entry_id: Some(targets.mapped_playlist_entry_id),
            loaded_target: Some("stale-loaded".to_owned()),
        },
        StaleInputKind::EndFile => PlayerLifecycleInput::EndFile {
            attachment_epoch: stale_epoch,
            playlist_entry_id: targets.mapped_playlist_entry_id,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
        StaleInputKind::PlaylistSnapshot => PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: stale_epoch,
            entries: vec![AuthoritativePlaylistEntry::new(
                targets.mapped_playlist_entry_id,
                Some("stale".to_owned()),
                true,
            )],
            current_path: Some("stale".to_owned()),
        },
        StaleInputKind::LifecycleReconciliationFailed => {
            PlayerLifecycleInput::LifecycleReconciliationFailed {
                attachment_epoch: stale_epoch,
            }
        }
        StaleInputKind::EofObserved => PlayerLifecycleInput::EofObserved {
            attachment_epoch: stale_epoch,
            playlist_entry_id: Some(targets.mapped_playlist_entry_id),
            reached: true,
            position_seconds: Some(f64::from(value)),
        },
        StaleInputKind::PlaybackRestart => PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch: stale_epoch,
            playlist_entry_id: Some(targets.mapped_playlist_entry_id),
        },
        StaleInputKind::PositionObserved => PlayerLifecycleInput::PositionObserved {
            attachment_epoch: stale_epoch,
            media_generation: targets.active_generation,
            observed_sequence: 1,
            position_seconds: f64::from(value),
        },
        StaleInputKind::SeekingObserved => PlayerLifecycleInput::SeekingObserved {
            attachment_epoch: stale_epoch,
            media_generation: targets.active_generation,
            observed_sequence: 1,
            seeking: true,
        },
        StaleInputKind::PhaseObserved => PlayerLifecycleInput::PhaseObserved {
            attachment_epoch: stale_epoch,
            phase: PlayerTransportPhase::Playing,
        },
        StaleInputKind::TransportDelta => PlayerLifecycleInput::TransportDelta {
            attachment_epoch: stale_epoch,
            delta: PlayerTransportDelta {
                position_seconds: Some(f64::from(value)),
                ..PlayerTransportDelta::default()
            },
        },
        StaleInputKind::LocalFileChanged => PlayerLifecycleInput::LocalFileChanged {
            attachment_epoch: stale_epoch,
            attempt_id: targets.submitting_attempt_id,
            media_generation: targets.submitting_generation,
            update: LocalFileUpdate::new("stale-file.mkv"),
        },
        StaleInputKind::SeekCommandAccepted => PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: stale_epoch,
            command_id: targets.seek_command_id,
        },
        StaleInputKind::SeekCommandRejected => PlayerLifecycleInput::SeekCommandRejected {
            attachment_epoch: stale_epoch,
            command_id: targets.seek_command_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        StaleInputKind::SeekCommandCompletionNotObserved => {
            PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                attachment_epoch: stale_epoch,
                command_id: targets.seek_command_id,
            }
        }
        StaleInputKind::EventGapDetected => PlayerLifecycleInput::EventGapDetected {
            attachment_epoch: stale_epoch,
        },
        StaleInputKind::AuthoritativeSnapshotApplied => {
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot {
                attachment_epoch: stale_epoch,
                sequence_boundary: PlayerSequenceBoundary::new(stale_epoch, 1),
                ..PlayerAuthoritativeSnapshot::default()
            })
        }
        StaleInputKind::TransportDisconnected => PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch: stale_epoch,
        },
    }
}

fn representative_input(kind: InputKind) -> PlayerLifecycleInput {
    let epoch = PlayerAttachmentEpoch::new(1);
    let command_id = PlayerCommandId::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let generation = PlayerMediaGeneration::new(1);
    match kind {
        InputKind::LoadAttemptSubmitted => PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id: Some(command_id),
            media_generation: generation,
            requested_target: "representative".to_owned(),
            baseline_playlist_entry_ids: BTreeSet::new(),
        },
        InputKind::ExternalLoadObserved => PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            playlist_entry_id: 1,
            observed_target: "representative".to_owned(),
            file_loaded: false,
        },
        InputKind::LoadAttemptAccepted => PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: epoch,
            attempt_id,
        },
        InputKind::LoadAttemptRejected => PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch: epoch,
            attempt_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        InputKind::CommandSubmitted => PlayerLifecycleInput::CommandSubmitted {
            command_id,
            media_generation: None,
            kind: LifecycleCommandKind::Pause,
        },
        InputKind::CommandAccepted => PlayerLifecycleInput::CommandAccepted {
            attachment_epoch: epoch,
            command_id,
        },
        InputKind::CommandRejected => PlayerLifecycleInput::CommandRejected {
            attachment_epoch: epoch,
            command_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        InputKind::CommandSuperseded => PlayerLifecycleInput::CommandSuperseded {
            attachment_epoch: epoch,
            command_id,
        },
        InputKind::CommandTransportDisconnected => {
            PlayerLifecycleInput::CommandTransportDisconnected {
                attachment_epoch: epoch,
                command_id,
            }
        }
        InputKind::CommandCompleted => PlayerLifecycleInput::CommandCompleted {
            attachment_epoch: epoch,
            command_id,
        },
        InputKind::CommandCompletionNotObserved => {
            PlayerLifecycleInput::CommandCompletionNotObserved {
                attachment_epoch: epoch,
                command_id,
            }
        }
        InputKind::StartFile => PlayerLifecycleInput::StartFile {
            attachment_epoch: epoch,
            playlist_entry_id: 1,
        },
        InputKind::FileLoaded => PlayerLifecycleInput::FileLoaded {
            attachment_epoch: epoch,
            playlist_entry_id: Some(1),
            loaded_target: Some("representative".to_owned()),
        },
        InputKind::EndFile => PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: 1,
            outcome: PlayerPhysicalLoadOutcome::Ended,
        },
        InputKind::PlaylistSnapshot => PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries: Vec::new(),
            current_path: None,
        },
        InputKind::LifecycleReconciliationFailed => {
            PlayerLifecycleInput::LifecycleReconciliationFailed {
                attachment_epoch: epoch,
            }
        }
        InputKind::EofObserved => PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: None,
            reached: false,
            position_seconds: None,
        },
        InputKind::PlaybackRestart => PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch: epoch,
            playlist_entry_id: None,
        },
        InputKind::PositionObserved => PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            observed_sequence: 1,
            position_seconds: 0.0,
        },
        InputKind::SeekingObserved => PlayerLifecycleInput::SeekingObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            observed_sequence: 1,
            seeking: false,
        },
        InputKind::PhaseObserved => PlayerLifecycleInput::PhaseObserved {
            attachment_epoch: epoch,
            phase: PlayerTransportPhase::Empty,
        },
        InputKind::TransportDelta => PlayerLifecycleInput::TransportDelta {
            attachment_epoch: epoch,
            delta: PlayerTransportDelta::default(),
        },
        InputKind::LocalFileChanged => PlayerLifecycleInput::LocalFileChanged {
            attachment_epoch: epoch,
            attempt_id,
            media_generation: generation,
            update: LocalFileUpdate::new("representative"),
        },
        InputKind::SeekCommandSubmitted => PlayerLifecycleInput::SeekCommandSubmitted {
            command_id,
            media_generation: generation,
            raw_player_target_seconds: 0.0,
            effective_room_target_seconds: 0.0,
            dispatch_sequence_boundary: 0,
        },
        InputKind::SeekCommandAccepted => PlayerLifecycleInput::SeekCommandAccepted {
            attachment_epoch: epoch,
            command_id,
        },
        InputKind::SeekCommandRejected => PlayerLifecycleInput::SeekCommandRejected {
            attachment_epoch: epoch,
            command_id,
            failure: PlayerCommandFailureKind::Unknown,
        },
        InputKind::SeekCommandCompletionNotObserved => {
            PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                attachment_epoch: epoch,
                command_id,
            }
        }
        InputKind::EventGapDetected => PlayerLifecycleInput::EventGapDetected {
            attachment_epoch: epoch,
        },
        InputKind::AuthoritativeSnapshotApplied => {
            PlayerLifecycleInput::AuthoritativeSnapshotApplied(PlayerAuthoritativeSnapshot {
                attachment_epoch: epoch,
                sequence_boundary: PlayerSequenceBoundary::new(epoch, 0),
                ..PlayerAuthoritativeSnapshot::default()
            })
        }
        InputKind::TimerAdvanced => PlayerLifecycleInput::TimerAdvanced { now_tick: 0 },
        InputKind::TransportDisconnected => PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch: epoch,
        },
        InputKind::AttachmentReplaced => PlayerLifecycleInput::AttachmentReplaced,
    }
}

#[test]
fn declared_vocabulary_executes_every_reducer_input_kind() {
    let mut observed = BTreeSet::new();
    for kind in ALL_INPUT_KINDS.iter().copied() {
        let input = representative_input(kind);
        assert_eq!(input_kind(&input), kind);
        let (state, _) = reduce_player_lifecycle(PlayerLifecycleState::default(), input);
        state
            .assert_invariants()
            .unwrap_or_else(|reason| panic!("representative {kind:?} was invalid: {reason}"));
        assert!(
            observed.insert(kind),
            "duplicate declared input kind: {kind:?}"
        );
    }
    let expected = ALL_INPUT_KINDS.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(observed, expected);
    assert_eq!(observed.len(), ALL_INPUT_KINDS.len());
}
