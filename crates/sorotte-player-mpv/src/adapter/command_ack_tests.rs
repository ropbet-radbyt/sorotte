use super::*;
use sorotte_player_api::PlayerCommandProgressState;

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
fn accepted_commands_expire_with_a_typed_timeout_failure() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        ..MpvAdapter::default()
    };
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Pause {
            logical_pause_observed: false,
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
