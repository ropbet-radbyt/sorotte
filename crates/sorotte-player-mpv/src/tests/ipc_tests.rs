use super::*;
use crate::ipc::{MPV_IPC_MAX_LINE_BYTES, MpvIpcConnectionEvent, MpvJsonIpcClient};
use sorotte_player_api::PlayerCapabilities;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
struct NeverRespondingTransport;

impl MpvJsonIpcTransport for NeverRespondingTransport {
    fn send_line_until(&mut self, _line: &str, _deadline: Instant) -> io::Result<()> {
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, deadline: Instant) -> io::Result<usize> {
        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(remaining);
        }
        line.clear();
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "test transport never produces a response",
        ))
    }
}

#[derive(Debug)]
struct VersionResponseThenTimeoutTransport {
    writes: Arc<Mutex<Vec<String>>>,
    version_response_sent: bool,
}

impl MpvJsonIpcTransport for VersionResponseThenTimeoutTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("timeout transport mutex should not be poisoned")
            .push(line.to_owned());
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        if !self.version_response_sent {
            self.version_response_sent = true;
            line.clear();
            line.push_str(r#"{"request_id":1,"error":"success","data":"custom-build"}"#);
            line.push('\n');
            return Ok(line.len());
        }
        line.clear();
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "compatibility probe timed out",
        ))
    }
}

#[derive(Debug)]
struct DropObservedTransport {
    dropped: Arc<AtomicBool>,
}

impl Drop for DropObservedTransport {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl MpvJsonIpcTransport for DropObservedTransport {
    fn send_line_until(&mut self, _line: &str, _deadline: Instant) -> io::Result<()> {
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        line.clear();
        Ok(0)
    }
}

#[test]
fn mpv_ipc_client_joins_worker_during_shutdown() {
    let dropped = Arc::new(AtomicBool::new(false));
    let client = MpvJsonIpcClient::new(Box::new(DropObservedTransport {
        dropped: Arc::clone(&dropped),
    }));

    drop(client);

    assert!(
        dropped.load(Ordering::SeqCst),
        "transport should be dropped before client shutdown returns"
    );
}

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
fn mpv_ipc_rejects_line_over_max_bytes() {
    let oversized_response = format!(
        r#"{{"request_id":1,"error":"success","data":"{}"}}"#,
        "a".repeat(MPV_IPC_MAX_LINE_BYTES)
    );
    let mut stream = io::Cursor::new(format!("{oversized_response}\n").into_bytes());
    let mut read_buffer = Vec::new();
    let mut line = String::new();
    let stream_error = read_line_from_stream(&mut stream, &mut read_buffer, &mut line)
        .expect_err("oversized stream line should fail before decoding");
    assert!(
        stream_error.to_string().contains("mpv IPC line too long"),
        "unexpected stream error: {stream_error}"
    );

    let lines = [oversized_response.as_str()];
    let (transport, _state) = fake_transport_with_reads(&lines);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    let client_error = client
        .get_property("path")
        .expect_err("oversized IPC client line should fail");
    assert!(
        client_error.contains("mpv IPC line too long"),
        "unexpected client error: {client_error}"
    );
}

#[test]
fn mpv_ipc_timeout_marks_connection_dead_and_next_command_fails_immediately() {
    let command_timeout = Duration::from_millis(20);
    let mut client = MpvJsonIpcClient::new_with_command_timeout(
        Box::new(NeverRespondingTransport),
        command_timeout,
    );

    let first_error = client
        .get_property("path")
        .expect_err("missing matching response should time out");

    assert!(
        first_error.contains("mpv IPC command timed out"),
        "unexpected timeout error: {first_error}"
    );

    let second_started_at = Instant::now();
    let second_error = client
        .get_property("pause")
        .expect_err("dead connection should reject a second command");
    assert!(
        second_error.contains("not connected"),
        "unexpected second-command error: {second_error}"
    );
    assert!(
        second_started_at.elapsed() < command_timeout,
        "second command waited behind the timed-out command"
    );

    let events = client.take_connection_events();
    assert!(matches!(
        events.as_slice(),
        [
            MpvIpcConnectionEvent::Connected { .. },
            MpvIpcConnectionEvent::CommandFailed { .. },
            MpvIpcConnectionEvent::TimedOut { timeout, .. },
            MpvIpcConnectionEvent::Disconnected { .. },
        ] if *timeout == command_timeout
    ));
}

#[cfg(windows)]
#[test]
fn windows_named_pipe_read_is_cancelled_at_command_deadline() {
    use std::{
        ffi::OsStr,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        path::Path,
        sync::mpsc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use windows_sys::Win32::{
        Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile},
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let pipe_name = format!(
        r"\\.\pipe\sorotte-mpv-timeout-{}-{unique}",
        std::process::id()
    );
    let wide_pipe_name = OsStr::new(&pipe_name)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: the pipe name is NUL-terminated and all optional security
    // attributes are null.
    let raw_server_handle = unsafe {
        CreateNamedPipeW(
            wide_pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            8 * 1024,
            8 * 1024,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(raw_server_handle, INVALID_HANDLE_VALUE);
    // SAFETY: ownership of the valid handle returned by `CreateNamedPipeW`
    // transfers to `OwnedHandle` exactly once.
    let server_handle = unsafe { OwnedHandle::from_raw_handle(raw_server_handle as _) };
    let (command_seen_tx, command_seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let handle = server_handle.as_raw_handle() as HANDLE;
        // SAFETY: `handle` is a live named-pipe server handle and this test
        // intentionally performs a synchronous server-side connection.
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_PIPE_CONNECTED as i32),
                "failed to connect test named pipe: {error}"
            );
        }

        let mut request = [0_u8; 8 * 1024];
        let mut bytes_read = 0_u32;
        // SAFETY: the request buffer and byte count remain valid for this
        // synchronous read, and the server handle was not opened overlapped.
        let read_succeeded = unsafe {
            ReadFile(
                handle,
                request.as_mut_ptr(),
                request.len() as u32,
                &mut bytes_read,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            read_succeeded,
            0,
            "test server failed to read request: {}",
            io::Error::last_os_error()
        );
        command_seen_tx
            .send(())
            .expect("test should observe the command");
        let _ = release_rx.recv_timeout(Duration::from_secs(2));
    });

    let command_timeout = Duration::from_millis(50);
    let mut client =
        MpvJsonIpcClient::connect_with_command_timeout(Path::new(&pipe_name), command_timeout)
            .expect("test client should connect to named pipe");
    let first_error = client
        .get_property("path")
        .expect_err("server intentionally never sends a response");
    assert!(first_error.contains("timed out"), "{first_error}");
    command_seen_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test server should receive the command");

    let second_started_at = Instant::now();
    let second_error = client
        .get_property("pause")
        .expect_err("timed-out named pipe should be disconnected");
    assert!(second_error.contains("not connected"), "{second_error}");
    assert!(second_started_at.elapsed() < command_timeout);

    release_tx.send(()).expect("test server should be released");
    server_thread.join().expect("test server should stop");
}

#[test]
fn mpv_ipc_preserves_unrelated_events_while_waiting() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"request_id":1,"error":"success","data":false}"#,
    ]);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let value = client
        .get_property("pause")
        .expect("matching response should succeed");

    assert_eq!(value, Some(json!(false)));
    assert_eq!(
        client.take_pending_events(),
        vec![json!({"event":"property-change","name":"pause","data":true})]
    );
}

#[test]
fn mpv_command_failure_is_observable_without_killing_connection() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"property unavailable"}"#,
        r#"{"request_id":2,"error":"success","data":"movie.mkv"}"#,
    ]);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let first_error = client
        .get_property("missing")
        .expect_err("mpv command error should be returned");
    assert!(first_error.contains("property unavailable"));

    let path = client
        .get_property_string("path")
        .expect("ordinary command failure should leave connection healthy");
    assert_eq!(path.as_deref(), Some("movie.mkv"));

    let events = client.take_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. }))
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        MpvIpcConnectionEvent::Disconnected { .. } | MpvIpcConnectionEvent::TimedOut { .. }
    )));
}

#[test]
fn malformed_mpv_response_does_not_leak_tokenized_target() {
    let secret = "mpv-malformed-response-token-canary";
    let malformed = format!("not-json X-Plex-Token={secret}");
    let (transport, _state) = fake_transport_with_reads(&[&malformed]);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let error = client
        .get_property_string("path")
        .expect_err("malformed response should fail and disconnect");
    let events = client.take_connection_events();

    assert!(!error.contains(secret));
    assert!(!format!("{events:?}").contains(secret));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
    );
}

#[test]
fn mpv_ipc_request_id_mismatch_marks_connection_dead() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":999,"error":"success","data":"old.mkv"}"#,
        r#"{"request_id":1,"error":"success","data":"movie.mkv"}"#,
    ]);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let error = client
        .get_property_string("path")
        .expect_err("mismatched request id should corrupt the connection");

    assert!(
        error.contains("request_id mismatch"),
        "unexpected mismatch error: {error}"
    );

    let second_error = client
        .get_property_string("path")
        .expect_err("corrupt connection should reject later commands");
    assert!(second_error.contains("not connected"));
}

#[test]
fn mpv_adapter_surfaces_timeout_as_player_error() {
    let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
        NeverRespondingTransport,
        Duration::from_millis(20),
    );

    let error = adapter
        .set_paused(true)
        .expect_err("adapter command should surface IPC timeout");

    match error {
        sorotte_player_api::PlayerError::OperationFailed(message) => {
            assert!(
                message.contains("mpv IPC command timed out"),
                "unexpected adapter timeout message: {message}"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    assert!(!adapter.is_connected());
    assert_eq!(adapter.capabilities(), PlayerCapabilities::NONE);

    let events = adapter.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. })),
        "adapter should surface the typed timeout event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. })),
        "adapter should surface the typed disconnect event: {events:?}"
    );
}

#[test]
fn connected_mpv_player_capabilities_follow_ipc_health() {
    let adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
        NeverRespondingTransport,
        Duration::from_millis(20),
    );
    let mut player = ConnectedMpvPlayer::from_test_adapter(adapter);

    assert!(player.is_connected());
    assert_eq!(player.capabilities(), PlayerCapabilities::ALL);

    let error = player
        .execute(PlayerCommand::SetPaused(true))
        .expect_err("connected wrapper should surface the IPC timeout");
    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("mpv IPC command timed out")),
        "unexpected connected-wrapper error: {error:?}"
    );

    assert!(!player.is_connected());
    assert_eq!(player.capabilities(), PlayerCapabilities::NONE);
    let events = player.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. })),
        "connected wrapper should surface its timeout event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. })),
        "connected wrapper should surface its disconnect event: {events:?}"
    );
}

#[test]
fn simulated_player_keeps_all_capabilities_without_ipc() {
    let player = SimulatedPlayer::new();

    assert!(!player.is_connected());
    assert_eq!(player.capabilities(), PlayerCapabilities::ALL);
}

#[test]
fn mpv_adapter_property_polling_emits_connection_failure_events() {
    let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
        NeverRespondingTransport,
        Duration::from_millis(20),
    );

    assert_eq!(adapter.take_local_file_update(), None);

    let events = adapter.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. })),
        "property polling should expose its command failure: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. })),
        "property polling should expose its timeout: {events:?}"
    );
}

#[test]
fn open_file_collects_filesystem_size_for_local_paths() {
    let temp_path = std::env::temp_dir().join("sorotte_mpv_adapter_size_probe.tmp");
    let mut temp_file = File::create(&temp_path).expect("temp file should be creatable");
    writeln!(temp_file, "12345").expect("temp file should be writable");
    drop(temp_file);

    let mut adapter = SimulatedPlayer::new();
    adapter
        .execute(PlayerCommand::OpenFile(
            temp_path.to_string_lossy().into_owned(),
        ))
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
fn set_position_waits_for_matching_response_and_preserves_async_events() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"property-change","name":"pause","data":false}"#,
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
        sorotte_player_api::PlayerError::OperationFailed(message) => {
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
fn open_network_file_scopes_configured_cache_options_to_the_load() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"0.40.0"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30"), ("cache-pause-wait", "5")]);

    adapter
        .open_file("https://media.example/video.m3u8")
        .expect("attached mpv transport should accept network loadfile");

    let writes = state.writes();
    let version_query: Value =
        serde_json::from_str(writes[0].trim_end()).expect("valid version query");
    assert_eq!(
        version_query,
        json!({
            "command": ["get_property", "mpv-version"],
            "request_id": 1
        })
    );
    let payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": [
                "loadfile",
                "https://media.example/video.m3u8",
                "replace",
                -1,
                {
                    "cache-pause-wait": "5",
                    "cache-secs": "30"
                }
            ],
            "request_id": 2
        })
    );
}

#[test]
fn open_network_file_uses_legacy_options_position_for_mpv_before_0_38() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"0.37.0"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);

    adapter
        .open_file("https://media.example/legacy.m3u8")
        .expect("legacy mpv should accept its four-argument loadfile shape");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": [
                "loadfile",
                "https://media.example/legacy.m3u8",
                "replace",
                {"cache-secs": "30"}
            ],
            "request_id": 2
        })
    );
}

#[test]
fn open_network_file_unknown_version_falls_back_to_legacy_and_caches_result() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"custom-build"}"#,
        r#"{"request_id":2,"error":"invalid parameter"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);

    adapter
        .open_file("https://media.example/unknown.m3u8")
        .expect("unknown old mpv should accept the fallback command");
    adapter
        .open_file("https://media.example/next.m3u8")
        .expect("detected legacy syntax should be cached");

    let writes = state.writes();
    assert_eq!(writes.len(), 4, "the version must be queried only once");
    let modern_attempt: Value =
        serde_json::from_str(writes[1].trim_end()).expect("valid modern attempt");
    assert_eq!(modern_attempt["command"][3], json!(-1));
    let legacy_fallback: Value =
        serde_json::from_str(writes[2].trim_end()).expect("valid legacy fallback");
    assert!(legacy_fallback["command"][3].is_object());
    let cached_legacy: Value =
        serde_json::from_str(writes[3].trim_end()).expect("valid cached legacy command");
    assert!(cached_legacy["command"][3].is_object());
    assert_eq!(cached_legacy["request_id"], json!(4));

    let connection_events = adapter.take_ipc_connection_events();
    assert!(
        matches!(
            connection_events.as_slice(),
            [MpvIpcConnectionEvent::Connected { .. }]
        ),
        "a successful compatibility fallback must not surface its expected modern-shape rejection: {connection_events:?}"
    );
}

#[test]
fn unknown_loadfile_syntax_does_not_fallback_after_primary_disconnect() {
    let (transport, state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"success","data":"custom-build"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);

    let error = adapter
        .open_file("https://media.example/disconnect.m3u8")
        .expect_err("EOF during the modern probe must fail without a legacy retry");

    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("unexpected EOF")),
        "the primary disconnect error must be preserved: {error:?}"
    );
    assert_eq!(
        state.writes().len(),
        2,
        "only the version query and primary loadfile probe may be written"
    );
    let events = adapter.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. })),
        "the primary disconnect must remain observable: {events:?}"
    );
}

#[test]
fn unknown_loadfile_syntax_does_not_fallback_after_primary_timeout() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = VersionResponseThenTimeoutTransport {
        writes: Arc::clone(&writes),
        version_response_sent: false,
    };
    let mut adapter =
        MpvAdapter::with_test_transport_and_ipc_timeout(transport, Duration::from_millis(20));
    adapter.configure_network_media_options([("cache-secs", "30")]);

    let error = adapter
        .open_file("https://media.example/timeout.m3u8")
        .expect_err("timeout during the modern probe must fail without a legacy retry");

    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("timed out")),
        "the primary timeout error must be preserved: {error:?}"
    );
    assert_eq!(
        writes
            .lock()
            .expect("timeout transport mutex should not be poisoned")
            .len(),
        2,
        "only the version query and primary loadfile probe may be written"
    );
    let events = adapter.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. })),
        "the primary timeout must remain observable: {events:?}"
    );
}

#[test]
fn unknown_loadfile_syntax_does_not_fallback_after_primary_protocol_corruption() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"custom-build"}"#,
        "not-json",
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);

    let error = adapter
        .open_file("https://media.example/corrupt.m3u8")
        .expect_err("protocol corruption during the modern probe must not retry");

    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("invalid mpv IPC JSON")),
        "the primary protocol error must be preserved: {error:?}"
    );
    assert_eq!(
        state.writes().len(),
        2,
        "only the version query and primary loadfile probe may be written"
    );
}

#[test]
fn unknown_loadfile_syntax_reports_both_probe_and_fallback_rejections() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"custom-build"}"#,
        r#"{"request_id":2,"error":"invalid parameter"}"#,
        r#"{"request_id":3,"error":"property unavailable"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);

    let error = adapter
        .open_file("https://media.example/rejected.m3u8")
        .expect_err("both rejected command shapes must fail the load");

    assert!(
        matches!(error, PlayerError::OperationFailed(ref message)
            if message.contains("invalid parameter") && message.contains("property unavailable")),
        "the final error must retain both compatibility failures: {error:?}"
    );
    assert_eq!(state.writes().len(), 3);
}

#[test]
fn open_local_file_preserves_user_cache_options_when_network_options_are_configured() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30")]);
    assert!(
        state.writes().is_empty(),
        "configuring network media must not mutate mpv's global options"
    );

    adapter
        .open_file("C:/Media/movie.mkv")
        .expect("attached mpv transport should accept local loadfile");
    adapter
        .open_file("file:///C:/Media/movie.mkv")
        .expect("attached mpv transport should preserve options for file URLs");

    let writes = state.writes();
    let payloads = writes
        .iter()
        .map(|write| serde_json::from_str::<Value>(write.trim_end()).expect("valid json"))
        .collect::<Vec<_>>();
    assert_eq!(
        payloads,
        vec![
            json!({
                "command": ["loadfile", "C:/Media/movie.mkv", "replace"],
                "request_id": 1
            }),
            json!({
                "command": ["loadfile", "file:///C:/Media/movie.mkv", "replace"],
                "request_id": 2
            }),
        ]
    );
}

#[test]
fn active_network_option_reapply_uses_authoritative_network_path_over_stale_local_cache() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.example/live.m3u8"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .open_file("C:/Media/stale-local.mkv")
        .expect("stale local path should be cached from an earlier request");
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);

    adapter
        .apply_network_media_options_to_active_media()
        .expect("an attached network file should accept file-local options");
    assert!(adapter.is_connected());

    let payloads = state
        .writes()
        .iter()
        .map(|write| serde_json::from_str::<Value>(write.trim_end()).expect("valid json"))
        .collect::<Vec<_>>();
    assert_eq!(
        payloads,
        vec![
            json!({
                "command": ["loadfile", "C:/Media/stale-local.mkv", "replace"],
                "request_id": 1
            }),
            json!({
                "command": ["get_property", "path"],
                "request_id": 2
            }),
            json!({
                "command": ["set_property", "file-local-options/cache-pause-wait", "5"],
                "request_id": 3
            }),
            json!({
                "command": ["set_property", "file-local-options/cache-secs", "75"],
                "request_id": 4
            }),
        ]
    );
}

#[test]
fn active_network_option_reapply_uses_authoritative_local_path_over_stale_network_cache() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"C:/Media/movie.mkv"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .open_file("https://media.example/stale-network.m3u8")
        .expect("stale network path should be cached from an earlier request");
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .apply_network_media_options_to_active_media()
        .expect("an attached local file should be left unchanged");
    assert!(adapter.is_connected());

    let writes = state.writes();
    assert_eq!(writes.len(), 2);
    let load_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid load json");
    assert_eq!(
        load_payload,
        json!({
            "command": ["loadfile", "https://media.example/stale-network.m3u8", "replace"],
            "request_id": 1
        })
    );
    let path_payload: Value =
        serde_json::from_str(writes[1].trim_end()).expect("valid path query json");
    assert_eq!(
        path_payload,
        json!({
            "command": ["get_property", "path"],
            "request_id": 2
        })
    );
}

#[test]
fn active_network_option_reapply_treats_null_path_as_healthy_idle_player() {
    let (transport, state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"success","data":null}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .apply_network_media_options_to_active_media()
        .expect("an idle attached player should need no option changes");

    assert!(adapter.is_connected());
    assert_eq!(state.writes().len(), 1, "only the path should be queried");
}

#[test]
fn active_network_option_reapply_treats_property_unavailable_as_healthy_idle_player() {
    let (transport, state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"property unavailable"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .apply_network_media_options_to_active_media()
        .expect("mpv's canonical unavailable-path response should mean no active file");

    assert!(adapter.is_connected());
    assert_eq!(state.writes().len(), 1, "only the path should be queried");
    assert!(matches!(
        adapter.take_ipc_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { .. }]
    ));
}

#[test]
fn active_network_option_reapply_does_not_swallow_other_server_rejection() {
    let (transport, state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"property not found"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media()
        .expect_err("only mpv's exact property-unavailable token should mean idle");

    assert!(error.to_string().contains("property not found"));
    assert!(
        adapter.is_connected(),
        "ordinary server rejection is nonfatal"
    );
    assert_eq!(state.writes().len(), 1, "only the path should be queried");
}

#[test]
fn active_network_option_reapply_surfaces_path_query_timeout_and_disconnects() {
    let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
        NeverRespondingTransport,
        Duration::from_millis(20),
    );
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media()
        .expect_err("path-query timeout must not look like an idle player");

    assert!(error.to_string().contains("timed out"));
    assert!(!adapter.is_connected());
}

#[test]
fn active_network_option_reapply_surfaces_path_query_disconnect() {
    let (transport, _state) = fake_transport_with_reads(&[]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media()
        .expect_err("path-query EOF must not look like an idle player");

    assert!(error.to_string().contains("unexpected EOF"));
    assert!(!adapter.is_connected());
}

#[test]
fn active_network_option_reapply_surfaces_malformed_path_response() {
    let (transport, _state) = fake_transport_with_reads(&["not-json"]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media()
        .expect_err("malformed path response must not look like an idle player");

    assert!(error.to_string().contains("invalid mpv IPC JSON"));
    assert!(!adapter.is_connected());
}

#[test]
fn active_network_option_reapply_surfaces_mismatched_path_response() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":999,"error":"success","data":"https://media.example/live.m3u8"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media()
        .expect_err("mismatched path response must not look like an idle player");

    assert!(error.to_string().contains("request_id mismatch"));
    assert!(!adapter.is_connected());
}

#[test]
fn attached_open_file_waits_for_file_loaded_before_emitting_local_file_update() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":"movie.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":24.5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success","data":"movie.mkv"}"#,
        r#"{"request_id":4,"error":"success","data":24.5}"#,
        r#"{"request_id":5,"error":"success","data":1000}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
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
    assert_eq!(update.duration_seconds, Some(24.5));
    assert_eq!(update.size_bytes, Some(1000));
}

#[test]
fn attached_open_file_completes_pending_load_from_polled_properties_without_file_loaded_event() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success","data":"C:/media/movie.mkv"}"#,
        r#"{"request_id":11,"error":"success","data":24.5}"#,
        r#"{"request_id":12,"error":"success","data":1000}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("C:/media/movie.mkv")
        .expect("attached mpv transport should accept loadfile");

    assert_eq!(
        adapter.take_media_load_outcome(),
        None,
        "no async file-loaded event has been observed yet"
    );
    let update = adapter
        .take_local_file_update()
        .expect("loaded file metadata should be recovered by polling mpv properties");
    assert_eq!(update.path.as_deref(), Some("C:/media/movie.mkv"));
    assert_eq!(update.duration_seconds, Some(24.5));
    assert_eq!(update.size_bytes, Some(1000));

    let outcome = adapter
        .take_media_load_outcome()
        .expect("poll completion should also finish the pending media load");
    assert_eq!(
        outcome,
        PlayerMediaLoadOutcome::success(
            "C:/media/movie.mkv",
            Some("C:/media/movie.mkv".to_owned())
        )
    );
}

#[test]
fn pending_open_file_poll_ignores_stale_previous_file_until_requested_target_loads() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success","data":"C:/media/old.mkv"}"#,
        r#"{"request_id":11,"error":"success","data":10.0}"#,
        r#"{"request_id":12,"error":"success","data":500}"#,
        r#"{"request_id":13,"error":"success","data":"C:/media/movie.mkv"}"#,
        r#"{"request_id":14,"error":"success","data":24.5}"#,
        r#"{"request_id":15,"error":"success","data":1000}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("C:/media/movie.mkv")
        .expect("attached mpv transport should accept loadfile");

    assert_eq!(
        adapter.take_local_file_update(),
        None,
        "a pending load should not publish metadata for the previous mpv file"
    );
    let update = adapter
        .take_local_file_update()
        .expect("requested target should publish once mpv reports it");
    assert_eq!(update.path.as_deref(), Some("C:/media/movie.mkv"));
    assert_eq!(update.duration_seconds, Some(24.5));
    assert_eq!(update.size_bytes, Some(1000));
}

#[test]
fn attached_open_file_defers_local_file_update_until_duration_is_available() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success","data":"movie.mkv"}"#,
        r#"{"request_id":4,"error":"success","data":null}"#,
        r#"{"request_id":5,"error":"success","data":1000}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success","data":"movie.mkv"}"#,
        r#"{"request_id":14,"error":"success","data":null}"#,
        r#"{"request_id":15,"error":"success","data":1000}"#,
        r#"{"request_id":16,"error":"success","data":"movie.mkv"}"#,
        r#"{"request_id":17,"error":"success","data":24.5}"#,
        r#"{"request_id":18,"error":"success","data":1000}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("movie.mkv")
        .expect("attached mpv transport should accept loadfile");

    let outcome = adapter
        .take_media_load_outcome()
        .expect("file-loaded should still emit a success outcome");
    assert_eq!(
        outcome,
        PlayerMediaLoadOutcome::success("movie.mkv", Some("movie.mkv".to_owned()))
    );
    assert_eq!(
        adapter.take_local_file_update(),
        None,
        "local file metadata should not publish a transient zero duration while mpv is still probing"
    );

    let update = adapter
        .take_local_file_update()
        .expect("duration availability should release the local file update");
    assert_eq!(update.path.as_deref(), Some("movie.mkv"));
    assert_eq!(update.duration_seconds, Some(24.5));
    assert_eq!(update.size_bytes, Some(1000));
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
        .show_text("sorotte notice", 4_000, 1)
        .expect("attached mpv transport should accept show-text");

    let writes = state.writes();
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "sorotte notice", 4_000, 1],
            "request_id": 1
        })
    );
}
