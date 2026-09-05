use crate::MpvAdapter;
use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use serde_json::{Value, json};
use sorotte_player_api::PlayerAdapter;
use std::{
    ffi::OsStr,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
};

const ORDINARY_COMMAND_TIMEOUT: Duration = Duration::from_millis(300);
const TEST_COMPLETION_BUDGET: Duration = Duration::from_secs(1);

static NEXT_PIPE_ID: AtomicU64 = AtomicU64::new(1);

#[test]
fn windows_named_pipe_terminal_cleanup_cancels_pending_heartbeat_and_readback_first() {
    for readback in [false, true] {
        let server = NamedPipeServer::unique("terminal-pending");
        let pipe_name = server.pipe_name().to_owned();
        let (seen_tx, seen_rx) = mpsc::channel();
        let peer = server.spawn("terminal-pending", move |peer| {
            peer.read_request();
            seen_tx.send(()).unwrap();
            // No response to the outstanding five-second command. Terminal
            // writes can arrive only once production native I/O is cancelled.
            peer.read_request();
            peer.read_request();
        });
        let mut client = MpvJsonIpcClient::connect_with_command_timeout(
            Path::new(&pipe_name),
            Duration::from_secs(5),
        )
        .unwrap();
        if readback {
            client
                .try_get_property_nonblocking("pause", 1)
                .unwrap()
                .unwrap();
        } else {
            client
                .try_send_command_expect_success_nonblocking(
                    json!(["script-message-to", "sorotte_network_options", "heartbeat"]),
                    1,
                )
                .unwrap()
                .unwrap();
        }
        seen_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        client.send_final_commands_best_effort(vec![
            json!(["set_property", "pause", false]),
            json!(["script-message", "release"]),
        ]);
        drop(client);
        assert!(
            started.elapsed() < Duration::from_millis(750),
            "shutdown waited for pending command"
        );
        let observed = join_peer(peer);
        assert_eq!(observed.requests.len(), 3);
        assert_eq!(
            observed.requests[1].value["command"],
            json!(["set_property", "pause", false])
        );
        assert_eq!(
            observed.requests[2].value["command"],
            json!(["script-message", "release"])
        );
        let _reused = NamedPipeServer::with_name(pipe_name);
    }
}

#[derive(Clone, Copy, Debug)]
enum DeliveryMode {
    OneByteWrites,
    OneCoalescedWrite,
}

#[derive(Debug)]
struct ObservedRequest {
    raw_line: String,
    value: Value,
}

#[derive(Debug)]
struct PeerObservations {
    requests: Vec<ObservedRequest>,
    write_sizes: Vec<usize>,
}

struct NamedPipeServer {
    pipe_name: String,
    handle: OwnedHandle,
}

impl NamedPipeServer {
    fn unique(scenario: &str) -> Self {
        let pipe_id = NEXT_PIPE_ID.fetch_add(1, Ordering::Relaxed);
        let pipe_name = format!(
            r"\\.\pipe\sorotte-mpv-kernel-{scenario}-{}-{pipe_id}",
            std::process::id()
        );
        Self::with_name(pipe_name)
    }

    fn with_name(pipe_name: String) -> Self {
        let wide_pipe_name = OsStr::new(&pipe_name)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: the pipe name is NUL-terminated, the buffer sizes are
        // bounded test constants, and no optional security attributes are
        // supplied.
        let raw_handle = unsafe {
            CreateNamedPipeW(
                wide_pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(
            raw_handle,
            INVALID_HANDLE_VALUE,
            "failed to create test named pipe {pipe_name}: {}",
            io::Error::last_os_error()
        );
        // SAFETY: ownership of this valid handle transfers from
        // `CreateNamedPipeW` to `OwnedHandle` exactly once.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle as _) };
        Self { pipe_name, handle }
    }

    fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    fn spawn(
        self,
        scenario: &'static str,
        script: impl FnOnce(&mut NamedPipePeer) + Send + 'static,
    ) -> JoinHandle<PeerObservations> {
        std::thread::Builder::new()
            .name(format!("mpv-pipe-{scenario}"))
            .spawn(move || {
                let mut peer = NamedPipePeer {
                    handle: self.handle,
                    read_buffer: Vec::new(),
                    requests: Vec::new(),
                    write_sizes: Vec::new(),
                };
                peer.connect();
                script(&mut peer);
                peer.into_observations()
            })
            .expect("named-pipe peer thread should start")
    }
}

struct NamedPipePeer {
    handle: OwnedHandle,
    read_buffer: Vec<u8>,
    requests: Vec<ObservedRequest>,
    write_sizes: Vec<usize>,
}

impl NamedPipePeer {
    fn raw_handle(&self) -> HANDLE {
        self.handle.as_raw_handle() as HANDLE
    }

    fn connect(&self) {
        // SAFETY: this is a live, synchronous named-pipe server handle. A
        // null `OVERLAPPED` makes this a bounded fixture-side blocking call
        // which is released by the production client opening the pipe.
        let connected = unsafe { ConnectNamedPipe(self.raw_handle(), std::ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_PIPE_CONNECTED as i32),
                "failed to connect test named pipe: {error}"
            );
        }
    }

    fn read_request(&mut self) -> Value {
        loop {
            if let Some(newline_index) = self.read_buffer.iter().position(|byte| *byte == b'\n') {
                let remainder = self.read_buffer.split_off(newline_index + 1);
                let raw_bytes = std::mem::replace(&mut self.read_buffer, remainder);
                let raw_line =
                    String::from_utf8(raw_bytes).expect("client request must be valid UTF-8");
                let value: Value = serde_json::from_str(raw_line.trim_end_matches(['\r', '\n']))
                    .expect("client request must be valid JSON");
                self.requests.push(ObservedRequest {
                    raw_line,
                    value: value.clone(),
                });
                return value;
            }

            let mut chunk = [0_u8; 8 * 1024];
            let mut bytes_read = 0_u32;
            // SAFETY: the server handle is live and synchronous, and both
            // output buffers remain valid for the duration of `ReadFile`.
            let succeeded = unsafe {
                ReadFile(
                    self.raw_handle(),
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(
                succeeded,
                0,
                "test peer failed to read request: {}",
                io::Error::last_os_error()
            );
            assert_ne!(bytes_read, 0, "client closed before sending a request");
            self.read_buffer
                .extend_from_slice(&chunk[..bytes_read as usize]);
        }
    }

    fn write_chunk(&mut self, bytes: &[u8]) {
        assert!(!bytes.is_empty(), "test writes must carry bytes");
        let mut bytes_written = 0_u32;
        // SAFETY: the server handle is live and synchronous, and both input
        // buffers remain valid for the duration of `WriteFile`.
        let succeeded = unsafe {
            WriteFile(
                self.raw_handle(),
                bytes.as_ptr(),
                u32::try_from(bytes.len()).expect("test payload should fit in u32"),
                &mut bytes_written,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            succeeded,
            0,
            "test peer failed to write response: {}",
            io::Error::last_os_error()
        );
        assert_eq!(
            bytes_written as usize,
            bytes.len(),
            "blocking named-pipe fixture must write each scheduled chunk atomically"
        );
        self.write_sizes.push(bytes.len());
    }

    fn write_bytes(&mut self, bytes: &[u8], delivery: DeliveryMode) {
        match delivery {
            DeliveryMode::OneByteWrites => {
                for byte in bytes {
                    self.write_chunk(std::slice::from_ref(byte));
                }
            }
            DeliveryMode::OneCoalescedWrite => self.write_chunk(bytes),
        }
    }

    fn write_lines(&mut self, lines: &[Value], delivery: DeliveryMode) -> usize {
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(
                serde_json::to_string(line)
                    .expect("test response should serialize")
                    .as_bytes(),
            );
            bytes.push(b'\n');
        }
        let payload_len = bytes.len();
        self.write_bytes(&bytes, delivery);
        payload_len
    }

    fn flush(&self) {
        // SAFETY: this is a live named-pipe server handle. The production
        // client is already reading, so the flush completes once the scripted
        // prefix has crossed the kernel boundary.
        let succeeded = unsafe { FlushFileBuffers(self.raw_handle()) };
        assert_ne!(
            succeeded,
            0,
            "test peer failed to flush bytes: {}",
            io::Error::last_os_error()
        );
    }

    fn into_observations(self) -> PeerObservations {
        PeerObservations {
            requests: self.requests,
            write_sizes: self.write_sizes,
        }
    }
}

fn connect_client(pipe_name: &str) -> MpvJsonIpcClient {
    MpvJsonIpcClient::connect_with_command_timeout(Path::new(pipe_name), ORDINARY_COMMAND_TIMEOUT)
        .unwrap_or_else(|error| panic!("production client should open {pipe_name}: {error}"))
}

fn take_initial_connected(client: &mut MpvJsonIpcClient) {
    assert!(matches!(
        client.take_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { generation }]
            if *generation == client.generation()
    ));
}

fn request_id(request: &Value) -> u64 {
    request
        .get("request_id")
        .and_then(Value::as_u64)
        .expect("production request should carry an unsigned request_id")
}

fn success_response(request_id: u64, data: Value) -> Value {
    json!({
        "request_id": request_id,
        "error": "success",
        "data": data,
    })
}

fn assert_protocol_terminal(client: &mut MpvJsonIpcClient) {
    assert!(!client.is_healthy());
    let events = client.take_connection_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. }))
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
            .count(),
        1,
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, MpvIpcConnectionEvent::TimedOut { .. })),
        "{events:?}"
    );
}

fn assert_timeout_terminal(client: &mut MpvJsonIpcClient) {
    assert!(!client.is_healthy());
    let events = client.take_connection_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. }))
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. }))
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
            .count(),
        1,
        "{events:?}"
    );
}

fn assert_terminal_fast_fail(client: &mut MpvJsonIpcClient) {
    let started = Instant::now();
    let error = client
        .get_property("must-not-cross-terminal-fence")
        .expect_err("terminal client reuse must fail");
    assert!(error.contains("not connected"), "{error}");
    assert!(
        started.elapsed() < ORDINARY_COMMAND_TIMEOUT,
        "terminal reuse must not wait for the transport deadline"
    );
    assert!(
        client.take_connection_events().is_empty(),
        "terminal reuse must not emit duplicate failures"
    );
}

fn join_peer(thread: JoinHandle<PeerObservations>) -> PeerObservations {
    thread.join().expect("named-pipe peer should stop cleanly")
}

#[test]
fn windows_named_pipe_fragmentation_and_coalescing_preserve_event_response_order() {
    for (scenario, delivery) in [
        ("fragmented", DeliveryMode::OneByteWrites),
        ("coalesced", DeliveryMode::OneCoalescedWrite),
    ] {
        let server = NamedPipeServer::unique(scenario);
        let pipe_name = server.pipe_name().to_owned();
        let peer_thread = server.spawn(scenario, move |peer| {
            let request = peer.read_request();
            let id = request_id(&request);
            let property = request
                .pointer("/command/1")
                .and_then(Value::as_str)
                .expect("get_property request should carry its property");
            peer.write_lines(
                &[
                    json!({
                        "event": "property-change",
                        "name": "kernel-order",
                        "data": "främé-雪-1",
                    }),
                    json!({
                        "event": "property-change",
                        "name": "kernel-order",
                        "data": "främé-雪-2",
                    }),
                    success_response(id, json!(property)),
                ],
                delivery,
            );
        });

        let mut client = connect_client(&pipe_name);
        take_initial_connected(&mut client);
        assert_eq!(
            client
                .get_property("kernel-framing")
                .unwrap_or_else(|error| panic!("{scenario} response should succeed: {error}")),
            Some(json!("kernel-framing"))
        );
        assert_eq!(
            client.take_pending_events(),
            vec![
                json!({
                    "event": "property-change",
                    "name": "kernel-order",
                    "data": "främé-雪-1",
                }),
                json!({
                    "event": "property-change",
                    "name": "kernel-order",
                    "data": "främé-雪-2",
                }),
            ],
            "{scenario} delivery must preserve event-before-response ordering"
        );
        assert!(client.take_connection_events().is_empty());

        let observations = join_peer(peer_thread);
        assert_eq!(observations.requests.len(), 1);
        assert_eq!(request_id(&observations.requests[0].value), 1);
        assert!(
            observations.requests[0].raw_line.ends_with('\n'),
            "production writes must remain newline framed"
        );
        assert_eq!(
            observations.requests[0]
                .raw_line
                .trim_end_matches(['\r', '\n'])
                .matches('\n')
                .count(),
            0,
            "one request must be exactly one frame"
        );
        match delivery {
            DeliveryMode::OneByteWrites => {
                assert!(
                    observations.write_sizes.len() > 100,
                    "fragmentation must exercise many kernel writes"
                );
                assert!(
                    observations.write_sizes.iter().all(|size| *size == 1),
                    "fragmentation schedule must split every UTF-8 and JSON boundary"
                );
            }
            DeliveryMode::OneCoalescedWrite => assert_eq!(
                observations.write_sizes.len(),
                1,
                "events and response must cross in one kernel write"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CorrelationFault {
    Stale,
    Future,
    Duplicate,
}

#[test]
fn windows_named_pipe_response_correlation_matrix_is_terminal_and_at_most_once() {
    for fault in [
        CorrelationFault::Stale,
        CorrelationFault::Future,
        CorrelationFault::Duplicate,
    ] {
        let scenario = match fault {
            CorrelationFault::Stale => "stale",
            CorrelationFault::Future => "future",
            CorrelationFault::Duplicate => "duplicate",
        };
        let server = NamedPipeServer::unique(scenario);
        let pipe_name = server.pipe_name().to_owned();
        let peer_thread = server.spawn(scenario, move |peer| {
            let first = peer.read_request();
            let first_id = request_id(&first);
            match fault {
                CorrelationFault::Stale => {
                    peer.write_lines(
                        &[
                            success_response(first_id.wrapping_sub(1), json!("stale")),
                            success_response(first_id, json!("matching-but-too-late")),
                        ],
                        DeliveryMode::OneCoalescedWrite,
                    );
                }
                CorrelationFault::Future => {
                    peer.write_lines(
                        &[
                            success_response(first_id.wrapping_add(1), json!("future")),
                            success_response(first_id, json!("matching-but-too-late")),
                        ],
                        DeliveryMode::OneCoalescedWrite,
                    );
                }
                CorrelationFault::Duplicate => {
                    let response = success_response(first_id, json!("first"));
                    peer.write_lines(
                        &[response.clone(), response],
                        DeliveryMode::OneCoalescedWrite,
                    );
                    peer.read_request();
                }
            }
        });

        let mut client = connect_client(&pipe_name);
        take_initial_connected(&mut client);
        let error = match fault {
            CorrelationFault::Stale | CorrelationFault::Future => client
                .get_property("first")
                .expect_err("wrong-generation response must fail correlation"),
            CorrelationFault::Duplicate => {
                assert_eq!(
                    client
                        .get_property("first")
                        .expect("first copy should satisfy the first command"),
                    Some(json!("first"))
                );
                client
                    .get_property("second")
                    .expect_err("duplicate response must not satisfy the next command")
            }
        };
        assert!(error.contains("request_id mismatch"), "{fault:?}: {error}");
        match fault {
            CorrelationFault::Stale => {
                assert!(error.contains("expected 1, received 0"), "{error}");
            }
            CorrelationFault::Future => {
                assert!(error.contains("expected 1, received 2"), "{error}");
            }
            CorrelationFault::Duplicate => {
                assert!(error.contains("expected 2, received 1"), "{error}");
            }
        }
        assert_protocol_terminal(&mut client);
        assert_terminal_fast_fail(&mut client);

        let observations = join_peer(peer_thread);
        let expected_requests = if matches!(fault, CorrelationFault::Duplicate) {
            2
        } else {
            1
        };
        assert_eq!(
            observations.requests.len(),
            expected_requests,
            "terminal fencing must stop every later request for {fault:?}"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum ReadDisconnectFault {
    TruncatedJson,
    CloseBeforeResponse,
}

#[test]
fn windows_named_pipe_truncated_and_closed_responses_fail_boundedly_and_terminally() {
    for fault in [
        ReadDisconnectFault::TruncatedJson,
        ReadDisconnectFault::CloseBeforeResponse,
    ] {
        let scenario = match fault {
            ReadDisconnectFault::TruncatedJson => "truncated-json",
            ReadDisconnectFault::CloseBeforeResponse => "read-close",
        };
        let server = NamedPipeServer::unique(scenario);
        let pipe_name = server.pipe_name().to_owned();
        let peer_thread = server.spawn(scenario, move |peer| {
            peer.read_request();
            if matches!(fault, ReadDisconnectFault::TruncatedJson) {
                peer.write_bytes(br#"{"request_id":1,"error":"#, DeliveryMode::OneByteWrites);
                peer.flush();
            }
            // Dropping the server endpoint makes the client observe the real
            // Windows broken-pipe boundary. Windows named pipes do not expose
            // a socket-style write-half shutdown.
        });

        let mut client = connect_client(&pipe_name);
        take_initial_connected(&mut client);
        let started = Instant::now();
        let error = client
            .get_property("read-disconnect")
            .expect_err("server close must fail the in-flight command");
        assert!(
            started.elapsed() < TEST_COMPLETION_BUDGET,
            "{fault:?} exceeded the bounded disconnect budget"
        );
        assert!(
            error.contains("failed to read")
                || error.contains("unexpected EOF")
                || error.contains("invalid mpv IPC JSON"),
            "{fault:?}: {error}"
        );
        assert_protocol_terminal(&mut client);
        assert_terminal_fast_fail(&mut client);

        let observations = join_peer(peer_thread);
        assert_eq!(observations.requests.len(), 1);
        match fault {
            ReadDisconnectFault::TruncatedJson => {
                assert!(
                    observations.write_sizes.len() > 10
                        && observations.write_sizes.iter().all(|size| *size == 1),
                    "the truncated prefix must cross the kernel boundary byte by byte"
                );
            }
            ReadDisconnectFault::CloseBeforeResponse => {
                assert!(observations.write_sizes.is_empty());
            }
        }
    }
}

#[test]
fn windows_named_pipe_server_disconnect_before_request_fails_the_write_and_fences_reuse() {
    let server = NamedPipeServer::unique("write-close");
    let pipe_name = server.pipe_name().to_owned();
    let (connected_tx, connected_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let peer_thread = server.spawn("write-close", move |_peer| {
        connected_tx
            .send(())
            .expect("test should publish server connection");
        release_rx
            .recv_timeout(TEST_COMPLETION_BUDGET)
            .expect("test should release the connected server");
    });

    let mut client = connect_client(&pipe_name);
    take_initial_connected(&mut client);
    connected_rx
        .recv_timeout(TEST_COMPLETION_BUDGET)
        .expect("server should accept the production client");
    release_tx
        .send(())
        .expect("server endpoint should be released");
    let observations = join_peer(peer_thread);
    assert!(observations.requests.is_empty());

    let started = Instant::now();
    let error = client
        .get_property("write-disconnect")
        .expect_err("closed server endpoint must fail the client write");
    assert!(
        started.elapsed() < TEST_COMPLETION_BUDGET,
        "write disconnect exceeded the bounded failure budget"
    );
    assert!(error.contains("failed to write"), "{error}");
    assert_protocol_terminal(&mut client);
    assert_terminal_fast_fail(&mut client);
}

#[test]
fn windows_named_pipe_withheld_response_honors_deadline_and_fences_later_writes() {
    let server = NamedPipeServer::unique("timeout");
    let pipe_name = server.pipe_name().to_owned();
    let (request_seen_tx, request_seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let peer_thread = server.spawn("timeout", move |peer| {
        peer.read_request();
        request_seen_tx
            .send(())
            .expect("test should publish the observed request");
        release_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test should release the withholding peer");
    });

    let command_timeout = Duration::from_millis(70);
    let mut client =
        MpvJsonIpcClient::connect_with_command_timeout(Path::new(&pipe_name), command_timeout)
            .expect("production client should open timeout pipe");
    take_initial_connected(&mut client);
    let started = Instant::now();
    let error = client
        .get_property("withheld")
        .expect_err("withheld response must reach the command deadline");
    let elapsed = started.elapsed();
    assert!(error.contains("timed out"), "{error}");
    assert!(
        elapsed >= command_timeout.saturating_sub(Duration::from_millis(15)),
        "the peer did not actually withhold the response: {elapsed:?}"
    );
    assert!(
        elapsed < TEST_COMPLETION_BUDGET,
        "kernel read cancellation exceeded its budget: {elapsed:?}"
    );
    request_seen_rx
        .recv_timeout(TEST_COMPLETION_BUDGET)
        .expect("peer should observe exactly one request");
    assert_timeout_terminal(&mut client);
    assert_terminal_fast_fail(&mut client);

    release_tx.send(()).expect("withholding peer should stop");
    let observations = join_peer(peer_thread);
    assert_eq!(observations.requests.len(), 1);
}

#[test]
fn windows_named_pipe_replacement_client_recovers_on_the_same_pipe_name() {
    let first_server = NamedPipeServer::unique("replacement");
    let pipe_name = first_server.pipe_name().to_owned();
    let first_peer = first_server.spawn("replacement-fail", |peer| {
        peer.read_request();
    });

    let mut first_client = connect_client(&pipe_name);
    take_initial_connected(&mut first_client);
    let first_generation = first_client.generation();
    let first_error = first_client
        .get_property("first-generation")
        .expect_err("first peer intentionally disconnects");
    assert!(
        first_error.contains("failed to read") || first_error.contains("unexpected EOF"),
        "{first_error}"
    );
    assert_protocol_terminal(&mut first_client);
    let first_observations = join_peer(first_peer);
    assert_eq!(first_observations.requests.len(), 1);
    drop(first_client);

    let replacement_server = NamedPipeServer::with_name(pipe_name.clone());
    let replacement_peer = replacement_server.spawn("replacement-ok", |peer| {
        let request = peer.read_request();
        peer.write_lines(
            &[success_response(
                request_id(&request),
                json!("replacement-ok"),
            )],
            DeliveryMode::OneCoalescedWrite,
        );
    });
    let mut replacement = connect_client(&pipe_name);
    take_initial_connected(&mut replacement);
    assert_ne!(
        replacement.generation(),
        first_generation,
        "replacement client must own a fresh logical generation"
    );
    assert_eq!(
        replacement
            .get_property("replacement")
            .expect("replacement client should use the new pipe instance"),
        Some(json!("replacement-ok"))
    );
    assert!(replacement.is_healthy());

    let replacement_observations = join_peer(replacement_peer);
    assert_eq!(replacement_observations.requests.len(), 1);
    assert_eq!(
        request_id(&replacement_observations.requests[0].value),
        1,
        "replacement client must restart request correlation independently"
    );
}

#[test]
fn windows_named_pipe_disconnected_adapter_retries_an_explicit_endpoint_when_it_appears() {
    let absent_server = NamedPipeServer::unique("late-explicit-endpoint");
    let pipe_name = absent_server.pipe_name().to_owned();
    drop(absent_server);

    let mut adapter = MpvAdapter::disconnected_with_json_ipc_retry(&pipe_name);
    assert_eq!(adapter.transport_is_connected(), Some(false));

    let replacement_server = NamedPipeServer::with_name(pipe_name);
    let peer = replacement_server.spawn("late-explicit-endpoint", |peer| {
        let request = peer.read_request();
        assert_eq!(
            request.get("command"),
            Some(&json!(["get_property", "mpv-version"]))
        );
        peer.write_lines(
            &[success_response(request_id(&request), json!("0.41.0"))],
            DeliveryMode::OneCoalescedWrite,
        );
    });

    adapter.maintain_runtime_integrations();

    assert_eq!(
        adapter.transport_is_connected(),
        Some(true),
        "the normal maintenance cadence should reattach the retained explicit endpoint"
    );
    let observations = join_peer(peer);
    assert_eq!(observations.requests.len(), 1);
}

#[test]
fn windows_named_pipe_request_ids_wrap_without_losing_response_correlation() {
    let server = NamedPipeServer::unique("request-id-wrap");
    let pipe_name = server.pipe_name().to_owned();
    let peer_thread = server.spawn("request-id-wrap", |peer| {
        for _ in 0..3 {
            let request = peer.read_request();
            let id = request_id(&request);
            let property = request
                .pointer("/command/1")
                .cloned()
                .expect("get_property should carry its property");
            peer.write_lines(
                &[success_response(id, property)],
                DeliveryMode::OneCoalescedWrite,
            );
        }
    });

    let mut client = MpvJsonIpcClient::connect_with_command_timeout_and_initial_request_id(
        Path::new(&pipe_name),
        ORDINARY_COMMAND_TIMEOUT,
        u64::MAX,
    )
    .expect("production client should open rollover pipe");
    take_initial_connected(&mut client);
    for property in ["at-max", "at-zero", "after-zero"] {
        assert_eq!(
            client
                .get_property(property)
                .unwrap_or_else(|error| panic!("{property} should correlate: {error}")),
            Some(json!(property))
        );
    }
    assert!(client.is_healthy());
    assert!(client.take_connection_events().is_empty());

    let observations = join_peer(peer_thread);
    assert_eq!(
        observations
            .requests
            .iter()
            .map(|request| request_id(&request.value))
            .collect::<Vec<_>>(),
        vec![u64::MAX, 0, 1]
    );
}
