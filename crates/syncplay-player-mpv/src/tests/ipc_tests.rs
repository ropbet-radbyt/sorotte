use super::*;

#[test]
fn buffered_read_line_from_stream_reuses_remaining_bytes_across_calls() {
    let mut stream = io::Cursor::new(
        b"{\"request_id\":1,\"error\":\"success\"}\n{\"request_id\":2,\"error\":\"success\"}\n"
            .to_vec(),
    );
    let mut read_buffer = Vec::new();
    let mut line = String::new();

    let first_bytes =
        read_line_from_stream(&mut stream, &mut read_buffer, &mut line).expect("first line");
    assert_eq!(first_bytes, line.len());
    assert_eq!(line, "{\"request_id\":1,\"error\":\"success\"}\n");

    let second_bytes =
        read_line_from_stream(&mut stream, &mut read_buffer, &mut line).expect("second line");
    assert_eq!(second_bytes, line.len());
    assert_eq!(line, "{\"request_id\":2,\"error\":\"success\"}\n");

    let eof_bytes = read_line_from_stream(&mut stream, &mut read_buffer, &mut line).expect("eof");
    assert_eq!(eof_bytes, 0);
    assert!(line.is_empty());
}

#[test]
fn buffered_read_line_from_stream_returns_partial_final_line_on_eof() {
    let mut stream = io::Cursor::new(b"{\"request_id\":1,\"error\":\"success\"}".to_vec());
    let mut read_buffer = Vec::new();
    let mut line = String::new();

    let bytes = read_line_from_stream(&mut stream, &mut read_buffer, &mut line).expect("line");
    assert_eq!(bytes, line.len());
    assert_eq!(line, "{\"request_id\":1,\"error\":\"success\"}");

    let eof_bytes = read_line_from_stream(&mut stream, &mut read_buffer, &mut line).expect("eof");
    assert_eq!(eof_bytes, 0);
    assert!(line.is_empty());
}

#[test]
fn open_file_collects_filesystem_size_for_local_paths() {
    let temp_path = std::env::temp_dir().join("syncplay_mpv_adapter_size_probe.tmp");
    let mut temp_file = File::create(&temp_path).expect("temp file should be creatable");
    writeln!(temp_file, "12345").expect("temp file should be writable");
    drop(temp_file);

    let mut adapter = MpvAdapter::default();
    adapter
        .open_file(temp_path.to_string_lossy().as_ref())
        .expect("mpv stub should accept local temp file");

    let file_update = adapter
        .take_local_file_update()
        .expect("open file should queue local file metadata update");
    assert_eq!(
        file_update.path.as_deref(),
        Some(temp_path.to_string_lossy().as_ref())
    );
    assert!(
        file_update.size_bytes.is_some_and(|size| size >= 6),
        "expected local file metadata size"
    );

    std::fs::remove_file(temp_path).expect("temp file should be removable");
}

#[test]
fn set_paused_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_paused(true)
        .expect("attached mpv transport should accept pause command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let sent = &writes[0];
    assert!(sent.ends_with('\n'), "expected newline-delimited mpv IPC");
    let payload: Value = serde_json::from_str(sent.trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "pause", true],
            "request_id": 1
        })
    );
    assert!(adapter.paused());
}

#[test]
fn set_muted_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_muted(true)
        .expect("attached mpv transport should accept mute command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "mute", true],
            "request_id": 1
        })
    );
    assert!(adapter.muted());
}

#[test]
fn set_volume_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_volume(33.5)
        .expect("attached mpv transport should accept volume command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "volume", 33.5],
            "request_id": 1
        })
    );
    assert_eq!(adapter.volume(), 33.5);
}

#[test]
fn set_fullscreen_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_fullscreen(true)
        .expect("attached mpv transport should accept fullscreen command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "fullscreen", true],
            "request_id": 1
        })
    );
    assert!(adapter.fullscreen());
}

#[test]
fn set_ontop_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_ontop(true)
        .expect("attached mpv transport should accept ontop command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "ontop", true],
            "request_id": 1
        })
    );
    assert!(adapter.ontop());
}

#[test]
fn set_border_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_border(true)
        .expect("attached mpv transport should accept border command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "border", true],
            "request_id": 1
        })
    );
    assert!(adapter.border());
}

#[test]
fn set_keep_open_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_keep_open(true)
        .expect("attached mpv transport should accept keep-open command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "keep-open", true],
            "request_id": 1
        })
    );
    assert!(adapter.keep_open());
}

#[test]
fn set_force_window_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_force_window(true)
        .expect("attached mpv transport should accept force-window command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "force-window", true],
            "request_id": 1
        })
    );
    assert!(adapter.force_window());
}

#[test]
fn set_deinterlace_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_deinterlace(true)
        .expect("attached mpv transport should accept deinterlace command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "deinterlace", true],
            "request_id": 1
        })
    );
    assert!(adapter.deinterlace());
}

#[test]
fn set_keepaspect_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_keepaspect(true)
        .expect("attached mpv transport should accept keepaspect command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "keepaspect", true],
            "request_id": 1
        })
    );
    assert!(adapter.keepaspect());
}

#[test]
fn set_keepaspect_window_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_keepaspect_window(true)
        .expect("attached mpv transport should accept keepaspect-window command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "keepaspect-window", true],
            "request_id": 1
        })
    );
    assert!(adapter.keepaspect_window());
}

#[test]
fn set_keep_open_pause_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_keep_open_pause(true)
        .expect("attached mpv transport should accept keep-open-pause command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "keep-open-pause", true],
            "request_id": 1
        })
    );
    assert!(adapter.keep_open_pause());
}

#[test]
fn set_cursor_autohide_fs_only_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_cursor_autohide_fs_only(true)
        .expect("attached mpv transport should accept cursor-autohide-fs-only command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "cursor-autohide-fs-only", true],
            "request_id": 1
        })
    );
    assert!(adapter.cursor_autohide_fs_only());
}

#[test]
fn set_stop_screensaver_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_stop_screensaver(true)
        .expect("attached mpv transport should accept stop-screensaver command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "stop-screensaver", true],
            "request_id": 1
        })
    );
    assert!(adapter.stop_screensaver());
}

#[test]
fn set_sub_visibility_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_sub_visibility(true)
        .expect("attached mpv transport should accept sub-visibility command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "sub-visibility", true],
            "request_id": 1
        })
    );
    assert!(adapter.sub_visibility());
}

#[test]
fn set_osd_bar_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_osd_bar(true)
        .expect("attached mpv transport should accept osd-bar command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "osd-bar", true],
            "request_id": 1
        })
    );
    assert!(adapter.osd_bar());
}

#[test]
fn set_window_maximized_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_window_maximized(true)
        .expect("attached mpv transport should accept window-maximized command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "window-maximized", true],
            "request_id": 1
        })
    );
    assert!(adapter.window_maximized());
}

#[test]
fn set_window_minimized_sends_json_ipc_set_property_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_window_minimized(true)
        .expect("attached mpv transport should accept window-minimized command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "window-minimized", true],
            "request_id": 1
        })
    );
    assert!(adapter.window_minimized());
}

#[test]
fn set_position_waits_for_matching_response_and_ignores_async_events() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"request_id":999,"error":"success"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_position(24.5)
        .expect("attached mpv transport should accept seek command");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set_property", "time-pos", 24.5],
            "request_id": 1
        })
    );
    assert_eq!(adapter.position_seconds(), 24.5);
}

#[test]
fn mpv_error_response_is_reported_and_local_state_is_not_updated() {
    let (transport, _state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"property unavailable"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    let err = adapter
        .set_position(42.0)
        .expect_err("mpv error response should fail operation");
    match err {
        syncplay_player_api::PlayerError::OperationFailed(message) => {
            assert!(
                message.contains("property unavailable"),
                "unexpected message: {message}"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
    assert_eq!(adapter.position_seconds(), 0.0);
}

#[test]
fn open_file_sends_mpv_loadfile_replace_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("movie.mkv")
        .expect("attached mpv transport should accept loadfile");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["loadfile", "movie.mkv", "replace"],
            "request_id": 1
        })
    );
}

#[test]
fn attached_open_file_waits_for_file_loaded_before_emitting_local_file_update() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":"movie.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":24.5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success","data":"movie.mkv"}"#,
        r#"{"request_id":3,"error":"success","data":24.5}"#,
        r#"{"request_id":4,"error":"success","data":1000}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("movie.mkv")
        .expect("attached mpv transport should accept loadfile");

    let outcome = adapter
        .take_media_load_outcome()
        .expect("file-loaded should emit a success outcome");
    assert_eq!(
        outcome,
        PlayerMediaLoadOutcome::success("movie.mkv", Some("movie.mkv".to_owned()))
    );
    let update = adapter
        .take_local_file_update()
        .expect("file-loaded should emit a local file update");
    assert_eq!(update.path.as_deref(), Some("movie.mkv"));
}

#[test]
fn attached_open_file_emits_failure_outcome_when_end_file_reports_error() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"end-file","reason":"error","file_error":"Failed to recognize file format."}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://www.youtube.com/watch?v=test")
        .expect("attached mpv transport should accept loadfile");

    let outcome = adapter
        .take_media_load_outcome()
        .expect("end-file error should emit a failure outcome");
    assert_eq!(
        outcome.requested_target,
        "https://www.youtube.com/watch?v=test"
    );
    assert_eq!(outcome.loaded_target, None);
    assert_eq!(
        outcome.failure.as_ref().map(|failure| failure.kind),
        Some(PlayerMediaLoadFailureKind::FormatUnsupported)
    );
    assert_eq!(adapter.take_local_file_update(), None);
}

#[test]
fn set_option_string_sends_json_ipc_set_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .set_option_string("script-opts", "osc=no")
        .expect("attached mpv transport should accept generic option updates");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["set", "script-opts", "osc=no"],
            "request_id": 1
        })
    );
}

#[test]
fn apply_profile_sends_json_ipc_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .apply_profile("fast")
        .expect("attached mpv transport should accept apply-profile");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["apply-profile", "fast"],
            "request_id": 1
        })
    );
}

#[test]
fn show_text_sends_json_ipc_command_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .show_text("syncplay notice", 4_000, 1)
        .expect("attached mpv transport should accept show-text");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "syncplay notice", 4_000, 1],
            "request_id": 1
        })
    );
}
