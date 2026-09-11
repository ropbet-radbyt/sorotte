use super::command_ack_tests::active_projection;
use super::*;

#[test]
fn authoritative_recovery_preserves_position_and_pause_after_transport_queue_pressure() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter::default();
    active_projection(&mut adapter, generation, 1, "test://pressure");
    adapter.observed_state.position_seconds = Some(12.0);
    adapter.observed_state.logical_pause = Some(true);
    adapter.queue_transport_telemetry_update(
        adapter
            .transport_update_for(generation)
            .with_position_seconds(12.0)
            .with_logical_pause(true),
    );
    for index in 0..128 {
        let phase = if index % 2 == 0 {
            PlayerTransportPhase::Playing
        } else {
            PlayerTransportPhase::ReadyPaused
        };
        adapter.queue_transport_telemetry_update(
            adapter.transport_update_for(generation).with_phase(phase),
        );
    }
    assert!(adapter.player_lifecycle.requires_authoritative_snapshot());
    assert!(
        adapter.player_lifecycle.peek_event_batch().is_none(),
        "a lossy delta stream cannot masquerade as complete"
    );
    adapter.publish_authoritative_lifecycle_snapshot();
    let batch = adapter
        .player_lifecycle
        .peek_event_batch()
        .expect("authoritative recovery");
    let snapshot = batch
        .authoritative_snapshot
        .as_ref()
        .expect("recovery snapshot");
    assert_eq!(
        snapshot.transport.position_seconds,
        SnapshotField::Known(12.0)
    );
    assert_eq!(snapshot.transport.logical_pause, SnapshotField::Known(true));
    adapter
        .acknowledge_player_event_batch(batch.acknowledgement_token)
        .expect("snapshot receipt");
    assert!(!adapter.player_lifecycle.requires_authoritative_snapshot());
}
