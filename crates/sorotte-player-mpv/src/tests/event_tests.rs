use super::*;
use sorotte_client_core::{
    CoordinatorPlayerCommand, DesiredRoomPlayback, DesiredRoomPlaybackUpdateKind, LogicalMediaId,
    MediaTransportKind, PlaybackCoordinator, PlaybackCoordinatorAction, PlayerTransportObservation,
    SeekPreparationTerminalOutcome,
};
use sorotte_player_api::PlayerTransportTelemetryUpdate;

fn coordinator_observation(
    update: PlayerTransportTelemetryUpdate,
    media_generation: u64,
) -> PlayerTransportObservation {
    PlayerTransportObservation {
        media_generation,
        observed_at_seconds: update
            .observed_at
            .expect("mpv transport updates should be timestamped")
            .elapsed_since_adapter_start()
            .as_secs_f64(),
        phase: update.phase,
        position_seconds: update.position_seconds,
        playback_rate: update.playback_rate,
        logical_pause: update.logical_pause,
        paused_for_cache: update.paused_for_cache,
        seeking: update.seeking,
        seekable: update.seekable,
        timeline_kind: update.timeline_kind,
        seekable_ranges: update.seekable_ranges,
        known_live_seekable_window: update.known_live_seekable_window,
        core_idle: update.core_idle,
        playback_restart_sequence: update.playback_restart_sequence,
        cache_buffering_percent: update.cache_buffering_percent,
        buffered_ahead_seconds: update.buffered_ahead_seconds,
        input_rate_bytes_per_second: update.input_rate_bytes_per_second,
    }
}

#[test]
fn take_local_file_update_polls_mpv_properties_and_emits_changes_once() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success","data":"C:/media/movie.mkv"}"#,
        r#"{"request_id":10,"error":"success","data":1439.5}"#,
        r#"{"request_id":11,"error":"success","data":123456}"#,
        r#"{"request_id":12,"error":"success","data":"C:/media/movie.mkv"}"#,
        r#"{"request_id":13,"error":"success","data":1439.5}"#,
        r#"{"request_id":14,"error":"success","data":123456}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    let first = adapter
        .take_local_file_update()
        .expect("first poll should emit local file update");
    assert_eq!(first.name, "movie.mkv");
    assert_eq!(first.path.as_deref(), Some("C:/media/movie.mkv"));
    assert_eq!(first.duration_seconds, Some(1439.5));
    assert_eq!(first.size_bytes, Some(123456));

    assert_eq!(
        adapter.take_local_file_update(),
        None,
        "unchanged telemetry should not re-emit a file update"
    );

    let writes = state.writes();
    assert_eq!(writes.len(), 14);
    let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    let third_payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
    let seventh_payload: Value = serde_json::from_str(writes[6].trim_end()).expect("valid json");
    let eighth_payload: Value = serde_json::from_str(writes[7].trim_end()).expect("valid json");
    let ninth_payload: Value = serde_json::from_str(writes[8].trim_end()).expect("valid json");
    let tenth_payload: Value = serde_json::from_str(writes[9].trim_end()).expect("valid json");
    let eleventh_payload: Value = serde_json::from_str(writes[10].trim_end()).expect("valid json");
    assert_eq!(
        first_payload,
        json!({
            "command": ["observe_property", 1, "path"],
            "request_id": 1
        })
    );
    assert_eq!(
        second_payload,
        json!({
            "command": ["observe_property", 2, "duration"],
            "request_id": 2
        })
    );
    assert_eq!(
        third_payload,
        json!({
            "command": ["observe_property", 3, "file-size"],
            "request_id": 3
        })
    );
    assert_eq!(
        seventh_payload,
        json!({
            "command": ["observe_property", 7, "paused-for-cache"],
            "request_id": 7
        })
    );
    assert_eq!(
        eighth_payload,
        json!({
            "command": ["observe_property", 8, "cache-buffering-state"],
            "request_id": 8
        })
    );
    assert_eq!(
        ninth_payload,
        json!({
            "command": ["get_property", "path"],
            "request_id": 9
        })
    );
    assert_eq!(
        tenth_payload,
        json!({
            "command": ["get_property", "duration"],
            "request_id": 10
        })
    );
    assert_eq!(
        eleventh_payload,
        json!({
            "command": ["get_property", "file-size"],
            "request_id": 11
        })
    );
}

#[test]
fn take_local_file_update_ignores_missing_path_until_file_is_available() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"property unavailable"}"#,
        r#"{"request_id":10,"error":"success","data":"C:/media/movie2.mkv"}"#,
        r#"{"request_id":11,"error":"success","data":42}"#,
        r#"{"request_id":12,"error":"success","data":1000}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    assert_eq!(adapter.take_local_file_update(), None);

    let update = adapter
        .take_local_file_update()
        .expect("file should emit after path becomes available");
    assert_eq!(update.name, "movie2.mkv");
    assert_eq!(update.path.as_deref(), Some("C:/media/movie2.mkv"));
    assert_eq!(update.duration_seconds, Some(42.0));
    assert_eq!(update.size_bytes, Some(1000));
}

#[test]
fn async_property_change_events_from_mpv_queue_local_file_update() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"property unavailable"}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/from-event.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":120.0}"#,
        r#"{"event":"property-change","name":"file-size","data":987654}"#,
        r#"{"request_id":10,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    assert_eq!(adapter.take_local_file_update(), None);

    adapter.set_paused(true).expect("command should succeed");

    let update = adapter
        .take_local_file_update()
        .expect("async property-change events should queue a local file update");
    assert_eq!(update.name, "from-event.mkv");
    assert_eq!(update.path.as_deref(), Some("C:/media/from-event.mkv"));
    assert_eq!(update.duration_seconds, Some(120.0));
    assert_eq!(update.size_bytes, Some(987654));

    let writes = state.writes();
    assert_eq!(writes.len(), 10);
    let last_payload: Value = serde_json::from_str(writes[9].trim_end()).expect("valid json");
    assert_eq!(
        last_payload,
        json!({
            "command": ["set_property", "pause", true],
            "request_id": 10
        })
    );
}

#[test]
fn async_property_change_events_queue_playback_telemetry_update() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"property unavailable"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":123.25}"#,
        r#"{"event":"property-change","name":"speed","data":1.10}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    assert_eq!(adapter.take_local_file_update(), None);

    adapter
        .set_position(10.0)
        .expect("command should drain and process queued events");

    let telemetry = adapter
        .take_playback_telemetry_update()
        .expect("expected merged playback telemetry update from async events");
    assert_eq!(
        telemetry,
        PlayerPlaybackTelemetryUpdate {
            paused: Some(true),
            position_seconds: Some(123.25),
            playback_rate: Some(1.10),
            paused_for_cache: None,
            cache_buffering_percent: None,
        }
    );
    assert_eq!(adapter.take_playback_telemetry_update(), None);
    assert!(adapter.paused());
    assert_eq!(
        adapter.position_seconds(),
        10.0,
        "commanded local state currently wins over earlier async time-pos event in this slice"
    );
    assert_eq!(adapter.playback_rate(), 1.10);
}

#[test]
fn paused_playback_telemetry_polls_time_pos_without_async_seek_event() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
        r#"{"request_id":17,"error":"success"}"#,
        r#"{"request_id":18,"error":"success","data":222.5}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_paused(true)
        .expect("attached mpv transport should accept pause command");

    let telemetry = adapter
        .take_playback_telemetry_update()
        .expect("paused telemetry should poll current mpv position");
    assert_eq!(
        telemetry,
        PlayerPlaybackTelemetryUpdate {
            paused: None,
            position_seconds: Some(222.5),
            playback_rate: None,
            paused_for_cache: None,
            cache_buffering_percent: None,
        }
    );
    assert_eq!(adapter.position_seconds(), 222.5);

    let writes = state.writes();
    assert_eq!(writes.len(), 18);
    let last_payload: Value = serde_json::from_str(writes[17].trim_end()).expect("valid json");
    assert_eq!(
        last_payload,
        json!({
            "command": ["get_property", "time-pos"],
            "request_id": 18
        })
    );
}

#[test]
fn cache_property_change_events_queue_cache_playback_telemetry_without_manual_pause() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"property unavailable"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":42.5}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    assert_eq!(adapter.take_local_file_update(), None);
    adapter
        .set_position(10.0)
        .expect("command should drain and process queued events");

    let telemetry = adapter
        .take_playback_telemetry_update()
        .expect("expected cache buffering telemetry from async events");
    assert_eq!(
        telemetry,
        PlayerPlaybackTelemetryUpdate {
            paused: None,
            position_seconds: None,
            playback_rate: None,
            paused_for_cache: Some(true),
            cache_buffering_percent: Some(42.5),
        }
    );
    assert!(adapter.paused());
    assert!(adapter.paused_for_cache());
    assert_eq!(adapter.cache_buffering_percent(), Some(42.5));
    let mut transport_updates = Vec::new();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        transport_updates.push(update);
    }
    assert!(
        transport_updates
            .iter()
            .all(|update| update.logical_pause != Some(true)),
        "a cache-induced mpv pause must not be reported as logical pause intent"
    );
    assert!(transport_updates.iter().any(|update| {
        update.paused_for_cache == Some(true) && update.cache_buffering_percent == Some(42.5)
    }));
}

#[test]
fn transport_lifecycle_and_cache_hints_are_generation_correlated() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":41}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"property-change","name":"seekable","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":10.0,"end":80.0},{"start":90.0,"end":85.0}],"cache-duration":5.25,"fw-bytes":345678,"raw-input-rate":2000000}}"#,
        r#"{"event":"property-change","name":"demuxer-cache-idle","data":false}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":42.5}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
        r#"{"request_id":17,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"request_id":18,"error":"success"}"#,
        r#"{"event":"property-change","name":"core-idle","data":false}"#,
        r#"{"request_id":19,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://media.invalid/watch?v=generation-test")
        .expect("load command should be accepted");

    let mut updates = Vec::new();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        updates.push(update);
    }

    let generation = adapter
        .media_generation()
        .expect("start-file should establish a media generation");
    assert_eq!(generation.get(), 1);
    assert!(
        updates
            .iter()
            .all(|update| update.media_generation == Some(generation))
    );
    assert!(updates.iter().all(|update| update.observed_at.is_some()));
    assert!(updates.windows(2).all(|window| {
        window[0].observed_at.expect("timestamp should be present")
            <= window[1].observed_at.expect("timestamp should be present")
    }));
    assert!(
        updates
            .iter()
            .any(|update| update.phase == Some(PlayerTransportPhase::Loading))
    );
    let restarted = updates
        .iter()
        .find(|update| update.playback_restart_sequence == Some(1))
        .expect("playback-restart should be preserved as a lifecycle boundary");
    assert_eq!(restarted.phase, Some(PlayerTransportPhase::Playing));
    let rebuffering = updates
        .iter()
        .find(|update| update.phase == Some(PlayerTransportPhase::Rebuffering))
        .expect("cache pause after restart should be classified as rebuffering");
    assert_eq!(rebuffering.paused_for_cache, Some(true));
    assert_eq!(rebuffering.core_idle, Some(true));
    assert_eq!(rebuffering.cache_buffering_percent, Some(42.5));
    assert_eq!(rebuffering.logical_pause, None);
    assert!(
        updates
            .iter()
            .all(|update| update.phase != Some(PlayerTransportPhase::ReadyPaused))
    );
    assert_eq!(
        rebuffering.seekable_ranges,
        Some(vec![PlayerSeekableRange::new(10.0, 80.0)])
    );
    assert_eq!(rebuffering.buffered_ahead_seconds, Some(5.25));
    assert_eq!(rebuffering.buffered_ahead_bytes, Some(345_678));
    assert_eq!(rebuffering.input_rate_bytes_per_second, Some(2_000_000));
    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Rebuffering);

    adapter
        .set_playback_rate(1.0)
        .expect("command should drain cache-release observations");
    let settling = adapter
        .take_transport_telemetry_update()
        .expect("cache release should emit a settling observation");
    assert_eq!(settling.paused_for_cache, Some(false));
    assert_eq!(settling.core_idle, Some(true));
    assert_eq!(settling.phase, Some(PlayerTransportPhase::Rebuffering));
    adapter
        .set_playback_rate(1.0)
        .expect("command should drain resumed core observation");
    let resumed = adapter
        .take_transport_telemetry_update()
        .expect("core-idle false should release rebuffering");
    assert_eq!(resumed.core_idle, Some(false));
    assert_eq!(resumed.phase, Some(PlayerTransportPhase::Playing));

    let writes = state.writes();
    assert_eq!(writes.len(), 19);
    let seeking_observer: Value =
        serde_json::from_str(writes[9].trim_end()).expect("valid seeking observer json");
    let cache_state_observer: Value =
        serde_json::from_str(writes[12].trim_end()).expect("valid cache observer json");
    let ytdl_live_observer: Value =
        serde_json::from_str(writes[14].trim_end()).expect("valid ytdl live observer json");
    let metadata_observer: Value =
        serde_json::from_str(writes[15].trim_end()).expect("valid metadata observer json");
    assert_eq!(
        seeking_observer,
        json!({
            "command": ["observe_property", 9, "seeking"],
            "request_id": 10
        })
    );
    assert_eq!(
        cache_state_observer,
        json!({
            "command": ["observe_property", 12, "demuxer-cache-state"],
            "request_id": 13
        })
    );
    assert_eq!(
        ytdl_live_observer,
        json!({
            "command": ["observe_property", 15, "metadata/by-key/ytdl_is_live"],
            "request_id": 15
        })
    );
    assert_eq!(
        metadata_observer,
        json!({
            "command": ["observe_property", 16, "metadata"],
            "request_id": 16
        })
    );
}

#[test]
fn playback_restart_preserves_observed_core_idle_for_intentionally_paused_media() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":42}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_playback_rate(1.0)
        .expect("paused playback lifecycle should be observed");

    let mut latest = PlayerTransportTelemetryUpdate::default();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        latest.merge_from(update);
    }
    assert_eq!(latest.phase, Some(PlayerTransportPhase::ReadyPaused));
    assert_eq!(latest.logical_pause, Some(true));
    assert_eq!(latest.paused_for_cache, Some(false));
    assert_eq!(latest.core_idle, Some(true));
    assert_eq!(latest.playback_restart_sequence, Some(1));
}

#[test]
fn paused_load_binds_global_pause_and_core_idle_and_restores_pause_after_cache_release() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"start-file","playlist_entry_id":43}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_playback_rate(1.0)
        .expect("paused load and cache lifecycle should be observed");

    let generation = adapter
        .media_generation()
        .expect("start-file should bind a media generation");
    let mut latest = PlayerTransportTelemetryUpdate::default();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        if update.media_generation == Some(generation) {
            latest.merge_from(update);
        }
    }
    assert_eq!(latest.phase, Some(PlayerTransportPhase::ReadyPaused));
    assert_eq!(latest.logical_pause, Some(true));
    assert_eq!(latest.paused_for_cache, Some(false));
    assert_eq!(latest.core_idle, Some(true));
    assert_eq!(latest.playback_restart_sequence, Some(1));
}

#[test]
fn drained_ready_paused_cannot_finish_preparation_before_delayed_cache_pause() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":44}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.invalid/race.wav"}"#,
        r#"{"event":"property-change","name":"duration","data":60.0}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"property-change","name":"seekable","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":5.0}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":5000000}}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":100.0}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"property-change","name":"time-pos","data":5.0}"#,
        r#"{"request_id":2,"error":"success"}"#,
        // These delayed pre-seek cache values arrive after command dispatch.
        // A later target-position event must not inherit them.
        r#"{"event":"property-change","name":"cache-buffering-state","data":100.0}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":9000000}}"#,
        r#"{"event":"property-change","name":"seeking","data":true}"#,
        // Input-rate/byte-only cache telemetry must also form a position boundary.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"fw-bytes":123456,"raw-input-rate":8000000}}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.0}"#,
        // The reverse ordering must not merge old cache evidence back into
        // the already queued target-position update.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"fw-bytes":654321,"raw-input-rate":9000000}}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":5,"error":"success"}"#,
        // A fresh but delayed old-position sample after Ready must not change
        // the recovery decision inherited from the target epoch.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":9000000}}"#,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    let mut coordinator = PlaybackCoordinator::default();
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("actual-mpv-ready-cache-race").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;

    adapter
        .set_playback_rate(1.0)
        .expect("initial paused media observations should drain");
    while let Some(update) = adapter.take_transport_telemetry_update() {
        coordinator.observe(coordinator_observation(update, generation));
    }
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 40.0,
            anchor_observed_at_seconds: 0.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    adapter
        .set_playback_rate(1.0)
        .expect("pre-seek position should trigger coordinator dispatch");
    let mut dispatch_actions = Vec::new();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        dispatch_actions.extend(coordinator.observe(coordinator_observation(update, generation)));
    }
    assert!(dispatch_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));

    adapter
        .execute_tracked(PlayerCommand::SetPosition(40.0))
        .expect("mpv should accept the primary seek");
    let mut transient_updates = Vec::new();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        transient_updates.push(update);
    }
    assert!(
        transient_updates.iter().any(|update| {
            update.phase == Some(PlayerTransportPhase::ReadyPaused)
                && update.playback_restart_sequence.is_some()
        }),
        "transient updates: {transient_updates:#?}"
    );
    assert!(
        transient_updates
            .iter()
            .filter(|update| {
                update
                    .position_seconds
                    .is_some_and(|position| (position - 40.0).abs() <= f64::EPSILON)
            })
            .all(|update| {
                update.cache_buffering_percent.is_none()
                    && update.buffered_ahead_seconds.is_none()
                    && update.buffered_ahead_bytes.is_none()
                    && update.input_rate_bytes_per_second.is_none()
            })
    );
    for update in transient_updates {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("delayed cache pause should drain separately");
    while let Some(update) = adapter.take_transport_telemetry_update() {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should drain separately");
    while let Some(update) = adapter.take_transport_telemetry_update() {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("post-ready delayed old cache state should drain separately");
    while let Some(update) = adapter.take_transport_telemetry_update() {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);
}

#[test]
fn end_file_error_is_classified_for_the_matching_generation() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":900}"#,
        r#"{"event":"end-file","playlist_entry_id":900,"reason":"error","file_error":"Failed to recognize file format."}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://media.invalid/unsupported")
        .expect("load command should be accepted");

    let mut updates = Vec::new();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        updates.push(update);
    }
    let failed = updates
        .iter()
        .find(|update| update.phase == Some(PlayerTransportPhase::Failed))
        .expect("end-file error should emit a failed transport observation");
    assert_eq!(
        failed.media_generation.map(|generation| generation.get()),
        Some(1)
    );
    assert_eq!(failed.eof_reached, Some(true));
    assert_eq!(
        failed.error_kind,
        Some(PlayerMediaLoadFailureKind::FormatUnsupported)
    );
    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Failed);

    let outcome = adapter
        .take_media_load_outcome()
        .expect("matching pending load should retain its detailed failure outcome");
    assert_eq!(
        outcome.failure.map(|failure| failure.kind),
        Some(PlayerMediaLoadFailureKind::FormatUnsupported)
    );
}

#[test]
fn stale_end_file_keeps_the_new_generation_loading() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":100}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
        r#"{"request_id":17,"error":"success"}"#,
        r#"{"event":"end-file","playlist_entry_id":100,"reason":"stop"}"#,
        r#"{"request_id":18,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://media.invalid/first")
        .expect("first load should be accepted");
    while adapter.take_transport_telemetry_update().is_some() {}

    adapter
        .open_file("https://media.invalid/second")
        .expect("replacement load should be accepted");

    assert_eq!(
        adapter.transport_phase(),
        PlayerTransportPhase::Loading,
        "the old end-file event must not overwrite the replacement generation phase"
    );
    let replacement_loading = adapter
        .take_transport_telemetry_update()
        .expect("replacement generation should emit loading telemetry");
    assert_eq!(
        replacement_loading
            .media_generation
            .map(|generation| generation.get()),
        Some(2)
    );
    assert_eq!(
        replacement_loading.phase,
        Some(PlayerTransportPhase::Loading)
    );
    let stale_end = adapter
        .take_transport_telemetry_update()
        .expect("old end-file should remain available with its original generation");
    assert_eq!(
        stale_end
            .media_generation
            .map(|generation| generation.get()),
        Some(1)
    );
    assert_eq!(stale_end.phase, Some(PlayerTransportPhase::Ended));
}

#[test]
fn client_message_events_from_syncplayintf_queue_pending_chat_requests() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"event":"client-message","args":["syncplayintf-chat","{\"protocol\":\"sorotte-syncplayintf-v1\",\"bridgeInstanceId\":\"test-bridge\",\"ownerId\":\"test-owner\",\"attachmentId\":\"test-attachment\",\"text\":\"hello \\\\ world\"}"]}"#,
        r#"{"request_id":9,"error":"success","data":false}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.enable_test_legacy_chat_input();

    assert_eq!(
        adapter.take_pending_chat_request(),
        Some("hello \\ world".to_owned())
    );
    assert_eq!(adapter.take_pending_chat_request(), None);
}

#[test]
fn endpoint_attachment_reset_discards_queued_syncplayintf_chat() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"client-message","args":["syncplayintf-chat","{\"protocol\":\"sorotte-syncplayintf-v1\",\"bridgeInstanceId\":\"test-bridge\",\"ownerId\":\"test-owner\",\"attachmentId\":\"test-attachment\",\"text\":\"old endpoint\"}"]}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.enable_test_legacy_chat_input();

    adapter
        .set_paused(false)
        .expect("a command should pump the queued client-message event");
    adapter.reset_test_legacy_syncplayintf_attachment();

    assert_eq!(adapter.take_pending_chat_request(), None);
}
