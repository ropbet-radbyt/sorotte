use super::*;

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
        r#"{"request_id":10,"error":"success","data":222.5}"#,
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
    assert_eq!(writes.len(), 10);
    let last_payload: Value = serde_json::from_str(writes[9].trim_end()).expect("valid json");
    assert_eq!(
        last_payload,
        json!({
            "command": ["get_property", "time-pos"],
            "request_id": 10
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
        r#"{"event":"client-message","args":["syncplayintf-chat","hello \\ world"]}"#,
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
