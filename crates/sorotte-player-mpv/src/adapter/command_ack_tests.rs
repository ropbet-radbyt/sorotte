use super::*;
use crate::ipc::MpvJsonIpcTransport;
use crate::lifecycle::{CommandSemanticState, LoadAttemptState};
use sorotte_player_api::{
    PlayerCommandSemanticResult, PlayerEvent, PlayerEventAcknowledgementToken,
    PlayerLoadAttemptResult, PlayerSemanticOutcome,
};
use std::{collections::BTreeSet, io};

pub(super) fn active_projection(
    adapter: &mut MpvAdapter,
    generation: PlayerMediaGeneration,
    playlist_entry_id: i64,
    path: &str,
) -> LoadAttemptId {
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: generation,
        playlist_entry_id,
        observed_target: path.to_owned(),
        file_loaded: true,
    });
    let attempt_id = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("external fixture should establish an active attempt");
    adapter.install_physical_projection(
        attempt_id,
        generation,
        Some(playlist_entry_id),
        Some(path.to_owned()),
        true,
    );
    adapter.observed_state.path = Some(path.to_owned());
    adapter.path_metadata_generation = Some(generation);
    adapter.active_generation_has_restarted = true;
    adapter.transport_phase = PlayerTransportPhase::Playing;
    attempt_id
}

fn timeout_unstarted_replacement(
    adapter: &mut MpvAdapter,
    generation: PlayerMediaGeneration,
    target: &str,
) -> LoadAttemptId {
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Load {
            file_loaded: false,
            ready: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    let baseline = adapter
        .active_playlist_entry_id
        .and_then(|entry_id| i64::try_from(entry_id).ok())
        .into_iter()
        .collect();
    let attempt_id = adapter.submit_lifecycle_load(Some(command_id), generation, target, baseline);
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch: adapter.lifecycle_epoch(),
        attempt_id,
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);
    adapter.finish_tracked_command(
        command_id,
        PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
    );
    assert!(matches!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::MayStillEmitQuiescent { .. }
    ));
    attempt_id
}

#[test]
fn timed_out_unbound_load_clears_pending_ui_and_stops_property_query_scheduling() {
    let generation = PlayerMediaGeneration::new(1);
    let target = "https://media.invalid/never-observed";
    let mut adapter = MpvAdapter::default();
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Load {
            file_loaded: false,
            ready: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    let attempt_id =
        adapter.submit_lifecycle_load(Some(command_id), generation, target, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);
    adapter.transport_phase = PlayerTransportPhase::Loading;
    adapter.lifecycle_reconciliation_due = true;

    adapter.finish_tracked_command(
        command_id,
        PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
    );

    assert_eq!(adapter.pending_load_request(), None);
    assert_eq!(adapter.pending_load_generation(), None);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
    assert!(
        adapter.lifecycle_reconciliation_due,
        "quiescence requests exactly one authoritative rebase"
    );
    assert!(!adapter.player_lifecycle.reconciliation_required);
    assert!(
        adapter
            .pending_load_transition_generations_for_test()
            .is_empty()
    );
    assert!(matches!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::MayStillEmitQuiescent { .. }
    ));

    let mut reconciliation_requests = 0;
    for now_tick in [100, 250, 500, 1_000, 2_000, 10_000, 60_000] {
        reconciliation_requests += adapter
            .apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced { now_tick })
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    PlayerLifecycleEffect::RequestLifecycleReconciliation
                )
            })
            .count();
    }
    assert_eq!(
        reconciliation_requests, 0,
        "quiescent ownership must not schedule another synchronous property-query group"
    );

    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("timeout outcomes should remain acknowledged delivery");
    assert!(batch.semantic_outcomes.iter().any(|outcome| matches!(
        &outcome.outcome,
        PlayerSemanticOutcome::LoadAttempt(attempt)
            if attempt.attempt_id == attempt_id
                && attempt.result == PlayerLoadAttemptResult::Indeterminate
    )));
}

#[test]
fn timed_out_started_load_retains_loading_projection_and_late_physical_ownership() {
    let generation = PlayerMediaGeneration::new(3);
    let target = "https://media.invalid/bound-without-file-loaded";
    let mut adapter = MpvAdapter::default();
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Load {
            file_loaded: false,
            ready: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    let attempt_id =
        adapter.submit_lifecycle_load(Some(command_id), generation, target, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![crate::lifecycle::AuthoritativePlaylistEntry::new(
            77,
            Some(target.to_owned()),
            true,
        )],
        current_path: Some(target.to_owned()),
    });
    adapter.handle_start_file_observation(77);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Loading);
    assert_eq!(adapter.active_media_generation, Some(generation));

    let effects = adapter.apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced {
        now_tick: adapter.player_lifecycle.now_tick.saturating_add(60_000),
    });

    assert_eq!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::Starting
    );
    assert_eq!(
        adapter.player_lifecycle.attempt_for_playlist_entry(77),
        Some(attempt_id)
    );
    assert_eq!(adapter.pending_load_request(), Some(target));
    assert_eq!(adapter.pending_load_generation(), Some(generation));
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Loading);
    assert_eq!(adapter.active_media_generation, Some(generation));
    assert_eq!(adapter.active_playlist_entry_id, Some(77));
    assert!(!adapter.paused_for_cache());
    assert_eq!(adapter.cache_buffering_percent(), None);
    assert!(
        adapter.lifecycle_reconciliation_due,
        "the already-requested authoritative rebase remains pending"
    );
    assert!(!adapter.player_lifecycle.reconciliation_required);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(
                effect,
                PlayerLifecycleEffect::EmitSemanticOutcome(outcome)
                    if matches!(
                        outcome.outcome,
                        PlayerSemanticOutcome::LoadAttempt(ref load)
                            if load.attempt_id == attempt_id
                                && load.result == PlayerLoadAttemptResult::Indeterminate
                    )
            ))
            .count(),
        1
    );

    adapter.handle_file_loaded_observation(Some(target.to_owned()));
    assert_eq!(
        adapter.player_lifecycle.active_load_attempt,
        Some(attempt_id)
    );
    assert_eq!(adapter.active_media_generation, Some(generation));
    assert!(adapter.active_file_loaded);
    assert_eq!(adapter.pending_load_generation(), None);
    let semantic_outcomes = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("retained timeout delivery")
        .semantic_outcomes;
    assert_eq!(
        semantic_outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(ref load) if load.attempt_id == attempt_id
            ))
            .count(),
        1,
        "late file-loaded must not emit a second success outcome"
    );
}

#[test]
fn commandless_recovery_load_deadline_retains_started_physical_state() {
    let generation = PlayerMediaGeneration::new(9);
    let mut adapter = MpvAdapter::default();
    let attempt_id = adapter.submit_lifecycle_load(
        None,
        generation,
        "https://media.invalid/recovery",
        BTreeSet::new(),
    );
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![crate::lifecycle::AuthoritativePlaylistEntry::new(
            77,
            Some("https://media.invalid/recovery".to_owned()),
            true,
        )],
        current_path: Some("https://media.invalid/recovery".to_owned()),
    });
    adapter.handle_start_file_observation(77);
    adapter.stream_recovery.interrupted_network_stream_recovery =
        Some(InterruptedNetworkStreamRecovery {
            media_generation: generation,
            latest_attempt_id: attempt_id,
            resume_position_seconds: 42.0,
            consecutive_attempts: 1,
            total_attempts: 1,
        });
    adapter.lifecycle_reconciliation_due = true;
    let accepted_at_tick = adapter.player_lifecycle.now_tick;

    let effects = adapter.apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced {
        now_tick: accepted_at_tick.saturating_add(60_000),
    });

    assert_eq!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::Starting
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        PlayerLifecycleEffect::EmitSemanticOutcome(outcome)
            if matches!(
                outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(ref load)
                    if load.attempt_id == attempt_id
                        && load.result == PlayerLoadAttemptResult::Indeterminate
            )
    )));
    assert_eq!(
        adapter.stream_recovery.interrupted_network_stream_recovery,
        Some(InterruptedNetworkStreamRecovery {
            media_generation: generation,
            latest_attempt_id: attempt_id,
            resume_position_seconds: 42.0,
            consecutive_attempts: 1,
            total_attempts: 1,
        })
    );
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Loading);
    assert_eq!(adapter.active_media_generation, Some(generation));
    assert_eq!(adapter.active_playlist_entry_id, Some(77));
    assert_eq!(adapter.current_path, None);
    assert!(
        adapter.lifecycle_reconciliation_due,
        "the already-requested authoritative rebase remains pending"
    );
    assert!(!adapter.player_lifecycle.reconciliation_required);
}

#[test]
fn delayed_file_loaded_from_superseded_attempt_cannot_replace_active_adapter_projection() {
    let mut adapter = MpvAdapter::default();
    let attachment_epoch = adapter.lifecycle_epoch();
    let generation_a = PlayerMediaGeneration::new(11);
    let attempt_a = adapter.submit_lifecycle_load(
        None,
        generation_a,
        "https://media.invalid/a",
        BTreeSet::new(),
    );
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id: attempt_a,
    });
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![crate::lifecycle::AuthoritativePlaylistEntry::new(
            10,
            Some("https://media.invalid/a".to_owned()),
            true,
        )],
        current_path: Some("https://media.invalid/a".to_owned()),
    });
    adapter.handle_start_file_observation(10);

    let generation_b = PlayerMediaGeneration::new(12);
    let attempt_b = adapter.submit_lifecycle_load(
        None,
        generation_b,
        "https://media.invalid/b",
        BTreeSet::from([10]),
    );
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id: attempt_b,
    });
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![
            crate::lifecycle::AuthoritativePlaylistEntry::new(
                10,
                Some("https://media.invalid/a".to_owned()),
                false,
            ),
            crate::lifecycle::AuthoritativePlaylistEntry::new(
                20,
                Some("https://media.invalid/b".to_owned()),
                true,
            ),
        ],
        current_path: Some("https://media.invalid/b".to_owned()),
    });
    adapter.handle_start_file_observation(20);
    adapter.handle_file_loaded_observation(Some("https://media.invalid/b".to_owned()));
    assert_eq!(
        adapter.player_lifecycle.active_load_attempt,
        Some(attempt_b)
    );
    assert_eq!(adapter.active_media_generation, Some(generation_b));
    assert_eq!(adapter.active_playlist_entry_id, Some(20));
    assert!(adapter.active_file_loaded);

    adapter.handle_start_file_observation(10);
    adapter.handle_file_loaded_observation(Some("https://media.invalid/a".to_owned()));

    assert_eq!(
        adapter.player_lifecycle.active_load_attempt,
        Some(attempt_b)
    );
    assert_eq!(adapter.active_media_generation, Some(generation_b));
    assert_eq!(adapter.active_playlist_entry_id, Some(20));
    assert!(adapter.active_file_loaded);
    assert_eq!(adapter.current_path, None);
    assert!(adapter.player_lifecycle.load_attempts[&attempt_a].logical_ownership_revoked);
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("supersession outcomes");
    assert!(batch.semantic_outcomes.iter().any(|outcome| matches!(
        outcome.outcome,
        PlayerSemanticOutcome::LoadAttempt(ref load)
            if load.attempt_id == attempt_a
                && load.result == PlayerLoadAttemptResult::Superseded
    )));
    assert!(!batch.semantic_outcomes.iter().any(|outcome| matches!(
        outcome.outcome,
        PlayerSemanticOutcome::LoadAttempt(ref load)
            if load.attempt_id == attempt_a
                && load.result == PlayerLoadAttemptResult::Loaded
    )));
}

#[test]
fn unstarted_replacement_timeout_rebases_to_authoritative_predecessor_without_mixing_identity() {
    for (active_path, replacement_path) in [
        ("C:/media/local-a.mkv", "https://media.invalid/network-b"),
        ("https://media.invalid/network-a", "C:/media/local-b.mkv"),
    ] {
        let generation_a = PlayerMediaGeneration::new(11);
        let generation_b = PlayerMediaGeneration::new(12);
        let mut adapter = MpvAdapter::default();
        let attempt_a = active_projection(&mut adapter, generation_a, 10, active_path);
        let attempt_b = timeout_unstarted_replacement(&mut adapter, generation_b, replacement_path);

        assert_eq!(adapter.active_load_attempt_id, Some(attempt_a));
        assert_eq!(adapter.active_media_generation, Some(generation_a));
        assert_eq!(adapter.active_playlist_entry_id, Some(10));
        assert_eq!(adapter.current_path(), Some(active_path));
        assert!(adapter.active_file_loaded);
        assert!(adapter.physical_projection_is_coherent());
        assert_ne!(adapter.active_load_attempt_id, Some(attempt_b));

        adapter.inject_authoritative_playlist_snapshot_for_test(
            [(10, Some(active_path.to_owned()), true)],
            Some(active_path.to_owned()),
        );

        assert_eq!(adapter.active_load_attempt_id, Some(attempt_a));
        assert_eq!(adapter.active_media_generation, Some(generation_a));
        assert_eq!(adapter.active_playlist_entry_id, Some(10));
        assert_eq!(adapter.current_path(), Some(active_path));
        assert!(adapter.active_file_loaded);
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
        assert!(adapter.physical_projection_is_coherent());
    }
}

#[test]
fn unstarted_replacement_timeout_rebases_to_authoritative_empty_player() {
    let generation_a = PlayerMediaGeneration::new(11);
    let generation_b = PlayerMediaGeneration::new(12);
    let mut adapter = MpvAdapter::default();
    let attempt_a = active_projection(&mut adapter, generation_a, 10, "https://media.invalid/a");
    timeout_unstarted_replacement(&mut adapter, generation_b, "C:/media/b.mkv");

    adapter.inject_authoritative_playlist_snapshot_for_test([], None);

    assert_eq!(adapter.active_load_attempt_id, None);
    assert_eq!(adapter.active_media_generation, None);
    assert_eq!(adapter.active_playlist_entry_id, None);
    assert_eq!(adapter.current_path(), None);
    assert!(!adapter.active_file_loaded);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
    assert!(adapter.physical_projection_is_coherent());
    assert!(
        adapter.player_lifecycle.load_attempts[&attempt_a]
            .state
            .is_terminal()
    );
}

#[test]
fn quiescent_replacement_appearing_after_timeout_never_mixes_predecessor_and_successor() {
    let generation_a = PlayerMediaGeneration::new(11);
    let generation_b = PlayerMediaGeneration::new(12);
    let target_a = "C:/media/a.mkv";
    let target_b = "https://media.invalid/b";
    let mut adapter = MpvAdapter::default();
    let attempt_a = active_projection(&mut adapter, generation_a, 10, target_a);
    let attempt_b = timeout_unstarted_replacement(&mut adapter, generation_b, target_b);

    adapter.inject_authoritative_playlist_snapshot_for_test(
        [(20, Some(target_b.to_owned()), true)],
        Some(target_b.to_owned()),
    );
    assert_eq!(adapter.active_load_attempt_id, None);
    assert_eq!(adapter.active_media_generation, None);
    assert_eq!(adapter.current_path(), None);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
    assert!(adapter.physical_projection_is_coherent());
    assert!(
        adapter.player_lifecycle.load_attempts[&attempt_a]
            .state
            .is_terminal()
    );

    adapter.handle_start_file_observation(20);
    assert_eq!(
        adapter.active_load_attempt_id, None,
        "a quiescent late start remains fail-closed until file-loaded"
    );
    assert!(adapter.physical_projection_is_coherent());

    adapter.handle_file_loaded_observation(Some(target_b.to_owned()));
    assert_eq!(adapter.active_load_attempt_id, Some(attempt_b));
    assert_eq!(adapter.active_media_generation, Some(generation_b));
    assert_eq!(adapter.active_playlist_entry_id, Some(20));
    assert_eq!(adapter.current_path(), Some(target_b));
    assert!(adapter.active_file_loaded);
    assert!(adapter.physical_projection_is_coherent());
}

#[test]
fn predecessor_end_file_clears_projection_while_replacement_remains_unstarted() {
    let generation_a = PlayerMediaGeneration::new(11);
    let generation_b = PlayerMediaGeneration::new(12);
    let mut adapter = MpvAdapter::default();
    active_projection(&mut adapter, generation_a, 10, "C:/media/a.mkv");
    let attempt_b =
        timeout_unstarted_replacement(&mut adapter, generation_b, "https://media.invalid/b");

    adapter.handle_end_file_event(&serde_json::json!({
        "reason": "stop",
        "playlist_entry_id": 10,
    }));

    assert_eq!(adapter.active_load_attempt_id, None);
    assert_eq!(adapter.active_media_generation, None);
    assert_eq!(adapter.active_playlist_entry_id, None);
    assert_eq!(adapter.current_path(), None);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
    assert!(adapter.physical_projection_is_coherent());
    assert!(matches!(
        adapter.player_lifecycle.load_attempts[&attempt_b].state,
        LoadAttemptState::MayStillEmitQuiescent { .. }
    ));
}

#[test]
fn accepted_load_deadline_is_anchored_when_the_command_is_accepted() {
    let generation = PlayerMediaGeneration::new(4);
    let mut adapter = MpvAdapter {
        observation_clock_origin: Instant::now() - Duration::from_secs(120),
        ..MpvAdapter::default()
    };
    let attempt_id = adapter.submit_lifecycle_load(
        None,
        generation,
        "https://media.invalid/fresh-acceptance",
        BTreeSet::new(),
    );
    let attachment_epoch = adapter.lifecycle_epoch();

    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    let accepted_at_tick = adapter.player_lifecycle.now_tick;
    adapter.apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced {
        now_tick: accepted_at_tick.saturating_add(59_999),
    });
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::AcceptedUnbound,
        "an adapter that was idle before dispatch must still grant a fresh reconciliation window"
    );

    adapter.apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced {
        now_tick: accepted_at_tick.saturating_add(60_000),
    });
    assert!(matches!(
        adapter.player_lifecycle.load_attempts[&attempt_id].state,
        LoadAttemptState::MayStillEmitQuiescent { .. }
    ));
}

#[test]
fn stale_generation_observations_cannot_complete_a_tracked_seek() {
    let stale_generation = PlayerMediaGeneration::new(1);
    let command_generation = PlayerMediaGeneration::new(2);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(command_generation),
        ..MpvAdapter::default()
    };
    let command_id = adapter.register_tracked_command(
        Some(command_generation),
        TrackedCommandKind::Seek {
            target_seconds: 40.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    assert_eq!(
        adapter.player_lifecycle.commands[&command_id].state,
        CommandSemanticState::Accepted
    );

    adapter.observe_tracked_commands(
        Some(stale_generation),
        TrackedCommandObservation::Seeking(false),
    );
    adapter.observe_tracked_commands(
        Some(stale_generation),
        TrackedCommandObservation::Position(40.0),
    );
    assert_eq!(
        adapter.player_lifecycle.commands[&command_id].state,
        CommandSemanticState::Accepted,
        "matching values from old media must not complete a new-generation command"
    );

    adapter.observe_tracked_commands(
        Some(command_generation),
        TrackedCommandObservation::Seeking(false),
    );
    adapter.observe_tracked_commands(
        Some(command_generation),
        TrackedCommandObservation::Position(40.0),
    );
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("terminal command batch");
    assert!(
        batch
            .semantic_outcomes
            .iter()
            .any(|item| matches!(item.outcome,
                PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
                    && outcome.media_generation == Some(command_generation)
                    && outcome.result == PlayerCommandSemanticResult::Completed
            ))
    );
}

#[test]
fn accepted_seek_expires_with_a_typed_timeout_failure() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        ..MpvAdapter::default()
    };
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Seek {
            target_seconds: 40.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    adapter
        .pending_tracked_commands
        .front_mut()
        .expect("command should remain pending")
        .accepted_at = Some(Instant::now() - PLAYER_COMMAND_TIMEOUT);

    adapter.expire_tracked_commands();

    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("terminal command batch");
    assert!(
        batch
            .semantic_outcomes
            .iter()
            .any(|item| matches!(item.outcome,
                PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
                    && outcome.media_generation == Some(generation)
                    && outcome.result == PlayerCommandSemanticResult::CompletionNotObserved
            ))
    );
    assert!(adapter.pending_tracked_commands.is_empty());
}

#[test]
fn event_gap_preserves_an_accepted_pending_seek_until_completion() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter::default();
    active_projection(&mut adapter, generation, 1, "test://pending-seek");
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Seek {
            target_seconds: 40.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    adapter.apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected {
        attachment_epoch: adapter.lifecycle_epoch(),
    });
    assert!(adapter.player_lifecycle.requires_authoritative_snapshot());
    adapter.publish_authoritative_lifecycle_snapshot();
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("snapshot closes the event gap");
    assert!(batch.authoritative_snapshot.is_some());
    assert_eq!(
        adapter.player_lifecycle.commands[&command_id].state,
        CommandSemanticState::Accepted
    );
    assert!(
        adapter
            .pending_tracked_commands
            .iter()
            .any(|pending| pending.id == command_id)
    );
    assert!(
        adapter
            .player_lifecycle
            .acknowledge_event_batch(batch.acknowledgement_token)
    );
    adapter.finish_tracked_command(command_id, PlayerCommandResult::Completed);
    let completed = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("completion after gap");
    assert!(
        completed
            .semantic_outcomes
            .iter()
            .any(|item| matches!(item.outcome,
                PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
                    && outcome.result == PlayerCommandSemanticResult::Completed
            ))
    );
}

#[test]
fn rejected_acknowledgement_replays_the_exact_terminal_batch() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter::default();
    active_projection(&mut adapter, generation, 1, "test://completed-seek");
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Seek {
            target_seconds: 40.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    adapter.finish_tracked_command(command_id, PlayerCommandResult::Completed);
    let rejected = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("terminal batch");
    assert!(
        rejected
            .semantic_outcomes
            .iter()
            .any(|item| matches!(item.outcome,
                PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
                    && outcome.result == PlayerCommandSemanticResult::Completed
            ))
    );
    let wrong_token = PlayerEventAcknowledgementToken::new(
        adapter.lifecycle_epoch(),
        rejected.acknowledgement_token.get() + 1,
    );
    assert!(adapter.acknowledge_player_event_batch(wrong_token).is_err());
    assert_eq!(
        adapter.player_lifecycle.peek_event_batch(),
        Some(rejected.clone())
    );
    adapter
        .acknowledge_player_event_batch(rejected.acknowledgement_token)
        .expect("matching receipt");
    assert!(adapter.player_lifecycle.peek_event_batch().is_none());
}

#[test]
fn early_tracked_load_failure_survives_reacquisition_without_an_active_generation() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        transport_phase: PlayerTransportPhase::Loading,
        ..MpvAdapter::default()
    };
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Load {
            file_loaded: false,
            ready: false,
        },
    );
    adapter.accept_tracked_command(command_id);
    let target = "https://media.invalid/fail";
    let attempt_id =
        adapter.submit_lifecycle_load(Some(command_id), generation, target, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);

    adapter.handle_end_file_event(&serde_json::json!({
        "reason": "error",
        "file_error": "network failed before start-file"
    }));
    assert_eq!(adapter.active_media_generation, None);
    assert_eq!(adapter.pending_load_generation(), None);
    adapter.apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected { attachment_epoch });
    adapter.publish_authoritative_lifecycle_snapshot();
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("snapshot and retained failures");
    assert_eq!(
        batch
            .authoritative_snapshot
            .as_ref()
            .expect("snapshot")
            .active_load,
        SnapshotField::KnownAbsent
    );
    assert!(batch.semantic_outcomes.iter().any(|item| matches!(item.outcome,
        PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
            && outcome.media_generation == Some(generation)
            && outcome.result == PlayerCommandSemanticResult::Failed(PlayerCommandFailureKind::MediaEnded)
    )));
    assert!(batch.semantic_outcomes.iter().any(|item| matches!(&item.outcome,
        PlayerSemanticOutcome::LoadAttempt(outcome) if outcome.attempt_id == attempt_id
            && outcome.media_generation == generation
            && outcome.requested_target == "https://media.invalid/fail"
            && outcome.result == PlayerLoadAttemptResult::Failed(PlayerMediaLoadFailureKind::Network)
    )));
}

#[test]
fn event_gap_retains_all_terminal_commands_until_acknowledged() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter::default();
    active_projection(&mut adapter, generation, 1, "test://many-commands");
    let mut command_ids = BTreeSet::new();
    for _ in 0..160 {
        let id = adapter.register_tracked_command(
            Some(generation),
            TrackedCommandKind::Seek {
                target_seconds: 40.0,
                seeking_finished: false,
                position_in_tolerance: false,
            },
        );
        adapter.accept_tracked_command(id);
        adapter.finish_tracked_command(id, PlayerCommandResult::Completed);
        command_ids.insert(id);
    }
    adapter.apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected {
        attachment_epoch: adapter.lifecycle_epoch(),
    });
    adapter.publish_authoritative_lifecycle_snapshot();
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("snapshot and retained outcomes");
    assert!(batch.authoritative_snapshot.is_some());
    let replayed = batch
        .semantic_outcomes
        .iter()
        .filter_map(|item| match item.outcome {
            PlayerSemanticOutcome::Command(outcome)
                if outcome.result == PlayerCommandSemanticResult::Completed =>
            {
                Some(outcome.command_id)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(replayed, command_ids);
    assert!(
        adapter
            .player_lifecycle
            .acknowledge_event_batch(batch.acknowledgement_token)
    );
    assert_eq!(adapter.player_lifecycle.pending_semantic_outcome_count(), 0);
}

#[test]
fn accepted_seek_disconnects_with_a_distinct_transport_failure() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        ..MpvAdapter::default()
    };
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Seek {
            target_seconds: 40.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);

    adapter.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);

    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("terminal command batch");
    assert!(
        batch
            .semantic_outcomes
            .iter()
            .any(|item| matches!(item.outcome,
                PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
                    && outcome.media_generation == Some(generation)
                    && outcome.result == PlayerCommandSemanticResult::TransportDisconnected
            ))
    );
    assert!(adapter.pending_tracked_commands.is_empty());
}

#[derive(Debug)]
struct DisconnectingTransport;

impl MpvJsonIpcTransport for DisconnectingTransport {
    fn send_line_until(&mut self, _line: &str, _deadline: Instant) -> io::Result<()> {
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        line.clear();
        Ok(0)
    }
}

#[test]
fn unhealthy_ipc_emits_one_generation_scoped_transport_failure() {
    let mut ipc_client = MpvJsonIpcClient::new(Box::new(DisconnectingTransport));
    assert!(
        ipc_client
            .send_command_expect_success(serde_json::json!(["get_property", "pause"]))
            .is_err()
    );
    assert!(!ipc_client.is_healthy());

    let generation = PlayerMediaGeneration::new(7);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        active_file_loaded: true,
        transport_phase: PlayerTransportPhase::Seeking,
        ipc_client: Some(ipc_client),
        ..MpvAdapter::default()
    };
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: generation,
        playlist_entry_id: 1,
        observed_target: "test://unhealthy-transport".to_owned(),
        file_loaded: true,
    });
    adapter.observe_unhealthy_ipc_transport();
    adapter.observe_unhealthy_ipc_transport();

    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("transport failure batch");
    let failures = batch
        .events
        .iter()
        .filter_map(|item| match &item.event {
            PlayerEvent::TransportDelta(delta)
                if delta.phase == Some(PlayerTransportPhase::Failed) =>
            {
                Some(delta)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].media_generation, Some(generation));
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Failed);
}
