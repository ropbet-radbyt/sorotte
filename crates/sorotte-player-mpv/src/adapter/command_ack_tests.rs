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
