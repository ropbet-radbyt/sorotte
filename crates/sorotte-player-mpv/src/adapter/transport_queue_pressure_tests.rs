use super::*;

#[test]
fn legacy_snapshot_survives_eviction_of_rich_position_and_pause_fields() {
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter {
        active_media_generation: Some(generation),
        ..MpvAdapter::default()
    };
    adapter.queue_playback_telemetry_update(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(12.0)
            .with_paused(true),
    );
    adapter.queue_transport_telemetry_update(
        PlayerTransportTelemetryUpdate::default()
            .with_position_seconds(12.0)
            .with_logical_pause(true),
    );

    for index in 0..=MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES {
        let phase = if index % 2 == 0 {
            PlayerTransportPhase::Playing
        } else {
            PlayerTransportPhase::ReadyPaused
        };
        adapter.queue_transport_telemetry_update(
            PlayerTransportTelemetryUpdate::default().with_phase(phase),
        );
    }

    let mut rich_position_seen = false;
    let mut rich_pause_seen = false;
    while let Some(update) = adapter.take_transport_telemetry_update() {
        rich_position_seen |= update.position_seconds.is_some();
        rich_pause_seen |= update.logical_pause.is_some();
    }
    assert!(!rich_position_seen);
    assert!(!rich_pause_seen);
    let legacy = adapter
        .take_playback_telemetry_update()
        .expect("coalesced legacy telemetry remains available after rich queue pressure");
    assert_eq!(legacy.position_seconds, Some(12.0));
    assert_eq!(legacy.paused, Some(true));
}
