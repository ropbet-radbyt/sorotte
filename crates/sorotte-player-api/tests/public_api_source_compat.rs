use sorotte_player_api::{
    PlayerMediaLoadFailureKind, PlayerSeekableRange, PlayerTimelineKind, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};

#[test]
fn legacy_transport_telemetry_struct_literal_remains_source_compatible() {
    let update = PlayerTransportTelemetryUpdate {
        media_generation: None,
        observed_at: None,
        phase: Some(PlayerTransportPhase::Playing),
        position_seconds: Some(12.5),
        playback_rate: Some(1.0),
        logical_pause: Some(false),
        paused_for_cache: Some(false),
        cache_buffering_percent: Some(100.0),
        seeking: Some(false),
        seekable: Some(true),
        timeline_kind: Some(PlayerTimelineKind::Vod),
        core_idle: Some(false),
        demuxer_cache_idle: Some(false),
        playback_restart_sequence: Some(1),
        eof_reached: Some(false),
        seekable_ranges: Some(vec![PlayerSeekableRange::new(0.0, 12.5)]),
        known_live_seekable_window: None,
        buffered_ahead_seconds: Some(5.0),
        buffered_ahead_bytes: Some(1024),
        input_rate_bytes_per_second: Some(2048),
        error_kind: None::<PlayerMediaLoadFailureKind>,
    };

    assert_eq!(update.phase, Some(PlayerTransportPhase::Playing));
}
