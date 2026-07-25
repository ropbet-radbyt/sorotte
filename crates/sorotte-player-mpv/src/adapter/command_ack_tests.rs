use super::*;
use crate::ipc::MpvJsonIpcTransport;
use sorotte_player_api::PlayerCommandProgressState;
use std::io;

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
        adapter
            .pending_command_progress_updates
            .pop_front()
            .map(|progress| progress.state),
        Some(PlayerCommandProgressState::Accepted)
    );

    adapter.observe_tracked_commands(
        Some(stale_generation),
        TrackedCommandObservation::Seeking(false),
    );
    adapter.observe_tracked_commands(
        Some(stale_generation),
        TrackedCommandObservation::Position(40.0),
    );
    assert!(
        adapter.pending_command_progress_updates.is_empty(),
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
    assert_eq!(
        adapter
            .pending_command_progress_updates
            .pop_front()
            .map(|progress| progress.state),
        Some(PlayerCommandProgressState::Finished(
            PlayerCommandResult::Completed
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
    adapter.pending_command_progress_updates.clear();
    adapter
        .pending_tracked_commands
        .front_mut()
        .expect("command should remain pending")
        .accepted_at = Some(Instant::now() - PLAYER_COMMAND_TIMEOUT);

    adapter.expire_tracked_commands();

    assert_eq!(
        adapter
            .pending_command_progress_updates
            .pop_front()
            .map(|progress| progress.state),
        Some(PlayerCommandProgressState::Finished(
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut)
        ))
    );
    assert!(adapter.pending_tracked_commands.is_empty());
}

#[test]
fn ordered_event_reacquisition_replays_an_accepted_pending_seek_lifecycle() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        active_file_loaded: true,
        transport_phase: PlayerTransportPhase::Seeking,
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
    adapter.ordered_player_event_reacquisition_required = true;

    let batch = adapter
        .take_ordered_event_batch()
        .expect("mpv supports ordered event batches");

    assert!(batch.dropped_events_through.is_some());
    assert!(batch.ordered_events.iter().any(|event| matches!(
        event.kind,
        PlayerOrderedEventKind::CommandProgress(progress)
            if progress.command_id == command_id
                && progress.state == PlayerCommandProgressState::Accepted
    )));
    assert!(
        adapter
            .pending_tracked_commands
            .iter()
            .any(|pending| pending.id == command_id)
    );
}

#[test]
fn consumer_reacquisition_replays_the_exact_terminal_from_the_rejected_batch() {
    let generation = PlayerMediaGeneration::new(1);
    let command_id = PlayerCommandId::new(91);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        active_file_loaded: true,
        transport_phase: PlayerTransportPhase::Playing,
        ..MpvAdapter::default()
    };
    adapter.queue_command_progress(PlayerCommandProgress::finished(
        command_id,
        Some(generation),
        Some(adapter.observation_timestamp()),
        Some(40.0),
        PlayerCommandResult::Completed,
    ));
    let rejected = adapter
        .take_ordered_event_batch()
        .expect("mpv supports ordered event batches");
    assert!(rejected.ordered_events.iter().any(|event| matches!(
        event.kind,
        PlayerOrderedEventKind::CommandProgress(progress)
            if progress.command_id == command_id
                && progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
    )));

    adapter.request_ordered_event_reacquisition();
    let reacquired = adapter
        .take_ordered_event_batch()
        .expect("mpv supports ordered event batches");

    assert!(reacquired.dropped_events_through.is_some());
    assert!(reacquired.ordered_events.iter().any(|event| matches!(
        event.kind,
        PlayerOrderedEventKind::CommandProgress(progress)
            if progress.command_id == command_id
                && progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
    )));
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
    adapter.ordered_player_event_reacquisition_required = true;

    let batch = adapter
        .take_ordered_event_batch()
        .expect("mpv supports ordered event batches");

    assert!(batch.dropped_events_through.is_some());
    assert!(batch.ordered_events.iter().any(|event| matches!(
        &event.kind,
        PlayerOrderedEventKind::CommandProgress(progress)
            if progress.command_id == command_id
                && progress.media_generation == Some(generation)
                && progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                        PlayerCommandFailureKind::MediaEnded
                    ))
    )));
    assert!(batch.ordered_events.iter().any(|event| matches!(
        &event.kind,
        PlayerOrderedEventKind::MediaLoad(observation)
            if observation.media_generation == Some(generation)
                && observation.outcome.requested_target == "https://media.invalid/fail"
                && observation.outcome.failure.as_ref().is_some_and(|failure| {
                    failure.kind == PlayerMediaLoadFailureKind::Network
                })
    )));
    assert!(batch.ordered_events.iter().any(|event| matches!(
        &event.kind,
        PlayerOrderedEventKind::Transport(update)
            if update.media_generation == Some(generation)
                && update.phase == Some(PlayerTransportPhase::Failed)
    )));
}

#[test]
fn reacquisition_replays_more_terminal_commands_than_the_legacy_progress_queue() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        active_file_loaded: true,
        transport_phase: PlayerTransportPhase::Playing,
        ..MpvAdapter::default()
    };
    let command_ids = (1..=160).map(PlayerCommandId::new).collect::<Vec<_>>();
    for command_id in &command_ids {
        adapter.queue_command_progress(PlayerCommandProgress::finished(
            *command_id,
            Some(generation),
            Some(adapter.observation_timestamp()),
            None,
            PlayerCommandResult::Completed,
        ));
    }
    for position in 0..120 {
        let update = adapter
            .transport_update_for(generation)
            .with_position_seconds(f64::from(position));
        adapter.queue_ordered_player_event(PlayerOrderedEventKind::Transport(update));
    }
    assert!(adapter.ordered_player_event_reacquisition_required);
    assert_eq!(
        adapter.pending_command_progress_updates.len(),
        MAX_PENDING_COMMAND_PROGRESS_UPDATES
    );

    let batch = adapter
        .take_ordered_event_batch()
        .expect("mpv supports ordered event batches");
    let replayed = batch
        .ordered_events
        .iter()
        .filter_map(|event| match event.kind {
            PlayerOrderedEventKind::CommandProgress(progress)
                if progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) =>
            {
                Some(progress.command_id)
            }
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(replayed.len(), command_ids.len());
    assert!(
        command_ids
            .iter()
            .all(|command_id| replayed.contains(command_id))
    );
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
    adapter.pending_command_progress_updates.clear();

    adapter.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);

    assert_eq!(
        adapter
            .pending_command_progress_updates
            .pop_front()
            .map(|progress| progress.state),
        Some(PlayerCommandProgressState::Finished(
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TransportDisconnected)
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

    let failures = adapter
        .pending_transport_telemetry_updates
        .iter()
        .filter(|update| update.phase == Some(PlayerTransportPhase::Failed))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].media_generation, Some(generation));
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Failed);
}
