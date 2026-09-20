use sorotte_client_core::PlayerTransportObservation;
use sorotte_client_core::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DesiredRoomPlayback,
    DesiredRoomPlaybackUpdateKind, LogicalMediaId, MediaTransportKind, PlaybackCoordinator,
    PlaybackCoordinatorAction, SeekPreparationDegradedReason, SeekPreparationTerminalOutcome,
};
use sorotte_player_api::{PlayerSeekableRange, PlayerTransportPhase};

fn paused(generation: u64, now: f64, position: f64) -> PlayerTransportObservation {
    PlayerTransportObservation::new(generation, now)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(position)
        .with_logical_pause(true)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
        .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 35.0)])
}

fn desire(generation: u64, revision: u64, now: f64, position: f64) -> DesiredRoomPlayback {
    DesiredRoomPlayback {
        media_generation: generation,
        state_revision: revision,
        paused: false,
        anchor_position_seconds: position,
        anchor_observed_at_seconds: now,
        force_seek: true,
    }
}

fn seek_commands(actions: &[PlaybackCoordinatorAction]) -> Vec<(CoordinatorCommandId, f64)> {
    actions
        .iter()
        .filter_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPosition(position),
            } => Some((*command_id, *position)),
            _ => None,
        })
        .collect()
}

fn nearest_fixture() -> (PlaybackCoordinator, u64, CoordinatorCommandId) {
    let mut coordinator = PlaybackCoordinator::default();
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("buffered-alternative-video").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.observe(paused(generation, 0.1, 5.0));
    coordinator.update_desired_room_state_with_kind(
        desire(generation, 1, 0.2, 40.0),
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    // A slow refill holds the cold seek while the user chooses a nearby point
    // already present in mpv's declared seekable cache range.
    coordinator.observe(
        paused(generation, 0.2, 5.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_cache_pause(true),
    );
    assert!(
        coordinator
            .seek_preparation_snapshot()
            .unwrap()
            .can_join_nearest_buffered
    );
    let nearest = coordinator.join_nearest_buffered_seek_preparation(0.3);
    let commands = seek_commands(&nearest);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].1, 35.0);
    (coordinator, generation, commands[0].0)
}

#[test]
fn failed_nearest_join_does_not_retry_cold_target() {
    let (mut coordinator, generation, nearest_command) = nearest_fixture();
    assert!(coordinator.command_failed(nearest_command, 0.31));
    let after_failure = coordinator.observe(paused(generation, 3.0, 5.0));
    let unrequested_seeks = seek_commands(&after_failure);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
    // A genuinely new user seek must still work after the failed alternative.
    coordinator.update_desired_room_state_with_kind(
        desire(generation, 2, 3.1, 20.0),
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let next = coordinator.observe(paused(generation, 3.2, 5.0));
    assert!(
        seek_commands(&next)
            .iter()
            .any(|(_, position)| (19.0..=21.0).contains(position)),
        "the next explicit cached seek remains usable: {next:?}"
    );
    assert!(
        unrequested_seeks.is_empty(),
        "failed Join nearest must retry its chosen target or degrade, not restart the cold room seek: {unrequested_seeks:?}"
    );
}

#[test]
fn successful_nearest_join_does_not_retry_cold_target() {
    let (mut coordinator, generation, nearest_command) = nearest_fixture();
    assert!(coordinator.command_accepted(nearest_command));
    let applied = coordinator.observe(paused(generation, 0.4, 35.0));
    let follow_on = coordinator.observe(paused(generation, 0.5, 35.0));
    assert!(seek_commands(&applied).is_empty(), "{applied:?}");
    assert!(seek_commands(&follow_on).is_empty(), "{follow_on:?}");
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Superseded)
    );
    coordinator.update_desired_room_state_with_kind(
        desire(generation, 2, 0.6, 20.0),
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let next = coordinator.observe(paused(generation, 0.7, 35.0));
    assert!(
        seek_commands(&next)
            .iter()
            .any(|(_, position)| (19.0..=21.0).contains(position))
    );
}

#[test]
fn timed_out_nearest_join_does_not_retry_cold_target() {
    let (mut coordinator, generation, nearest_command) = nearest_fixture();
    assert!(coordinator.command_accepted(nearest_command));
    let timeout = coordinator.tick(10.4);
    assert!(timeout.iter().any(|action| matches!(action,
        PlaybackCoordinatorAction::CommandTimedOut { command_id } if *command_id == nearest_command)));
    let after_timeout = coordinator.observe(paused(generation, 12.5, 5.0));
    let unrequested_seeks = seek_commands(&after_timeout);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
    coordinator.update_desired_room_state_with_kind(
        desire(generation, 2, 12.6, 20.0),
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let next = coordinator.observe(paused(generation, 12.7, 5.0));
    assert!(
        seek_commands(&next)
            .iter()
            .any(|(_, position)| (19.0..=21.0).contains(position))
    );
    assert!(
        unrequested_seeks.is_empty(),
        "timed-out Join nearest cannot silently restart the cold room seek: {unrequested_seeks:?}"
    );
}
