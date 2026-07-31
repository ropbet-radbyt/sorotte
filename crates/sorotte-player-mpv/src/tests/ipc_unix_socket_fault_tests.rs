use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const ORDINARY_COMMAND_TIMEOUT: Duration = Duration::from_millis(300);
const TEST_COMPLETION_BUDGET: Duration = Duration::from_secs(1);

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

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

struct UnixSocketFixture {
    root: PathBuf,
    socket_path: PathBuf,
}

impl UnixSocketFixture {
    fn unique() -> Self {
        loop {
            let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("sorotte-mpv-uds-{}-{id}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let socket_path = root.join("mpv.sock");
                    return Self { root, socket_path };
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!(
                    "failed to create nonce-owned Unix socket root {}: {error}",
                    root.display()
                ),
            }
        }
    }

    fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn root_path(&self) -> &Path {
        &self.root
    }

    fn bind(&self) -> UnixSocketServer {
        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path).unwrap_or_else(|error| {
                panic!(
                    "failed to remove owned stale socket {}: {error}",
                    self.socket_path.display()
                )
            });
        }
        let listener = UnixListener::bind(&self.socket_path).unwrap_or_else(|error| {
            panic!(
                "failed to bind nonce-owned Unix socket {}: {error}",
                self.socket_path.display()
            )
        });
        UnixSocketServer { listener }
    }
}

impl Drop for UnixSocketFixture {
    fn drop(&mut self) {
        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }
        let _ = fs::remove_dir(&self.root);
    }
}

struct UnixSocketServer {
    listener: UnixListener,
}

impl UnixSocketServer {
    fn spawn(
        self,
        scenario: &'static str,
        script: impl FnOnce(&mut UnixSocketPeer) + Send + 'static,
    ) -> JoinHandle<PeerObservations> {
        std::thread::Builder::new()
            .name(format!("mpv-uds-{scenario}"))
            .spawn(move || {
                let (stream, _) = self
                    .listener
                    .accept()
                    .expect("Unix socket peer should accept the production client");
                stream
                    .set_read_timeout(Some(TEST_COMPLETION_BUDGET))
                    .expect("test peer should set a bounded read timeout");
                stream
                    .set_write_timeout(Some(TEST_COMPLETION_BUDGET))
                    .expect("test peer should set a bounded write timeout");
                let mut peer = UnixSocketPeer {
                    stream,
                    read_buffer: Vec::new(),
                    requests: Vec::new(),
                    write_sizes: Vec::new(),
                };
                script(&mut peer);
                peer.into_observations()
            })
            .expect("Unix socket peer thread should start")
    }
}

struct UnixSocketPeer {
    stream: UnixStream,
    read_buffer: Vec<u8>,
    requests: Vec<ObservedRequest>,
    write_sizes: Vec<usize>,
}

impl UnixSocketPeer {
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
            let bytes_read = self
                .stream
                .read(&mut chunk)
                .expect("test peer should read a production request");
            assert_ne!(bytes_read, 0, "client closed before sending a request");
            self.read_buffer.extend_from_slice(&chunk[..bytes_read]);
        }
    }

    fn write_chunk(&mut self, bytes: &[u8]) {
        assert!(!bytes.is_empty(), "test writes must carry bytes");
        self.stream
            .write_all(bytes)
            .expect("test peer should write its complete scheduled chunk");
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

    fn shutdown_write(&self) {
        self.stream
            .shutdown(std::net::Shutdown::Write)
            .expect("test peer should half-close its write side");
    }

    fn shutdown_both(&self) {
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .expect("test peer should close both socket directions");
    }

    fn expect_client_eof(&mut self) {
        let mut byte = [0_u8; 1];
        let bytes = self
            .stream
            .read(&mut byte)
            .expect("idle production client should close cleanly");
        assert_eq!(bytes, 0, "client drop must release the kernel stream");
    }

    fn into_observations(self) -> PeerObservations {
        PeerObservations {
            requests: self.requests,
            write_sizes: self.write_sizes,
        }
    }
}

fn connect_client(socket_path: &Path) -> MpvJsonIpcClient {
    MpvJsonIpcClient::connect_with_command_timeout(socket_path, ORDINARY_COMMAND_TIMEOUT)
        .unwrap_or_else(|error| {
            panic!(
                "production client should open {}: {error}",
                socket_path.display()
            )
        })
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

fn assert_non_timeout_terminal(client: &mut MpvJsonIpcClient) {
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
    thread.join().expect("Unix socket peer should stop cleanly")
}

#[test]
fn unix_socket_fragmentation_and_coalescing_preserve_event_response_order() {
    for (scenario, delivery) in [
        ("fragmented", DeliveryMode::OneByteWrites),
        ("coalesced", DeliveryMode::OneCoalescedWrite),
    ] {
        let fixture = UnixSocketFixture::unique();
        let peer_thread = fixture.bind().spawn(scenario, move |peer| {
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

        let mut client = connect_client(fixture.socket_path());
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
                    "fragmentation must exercise many socket writes"
                );
                assert!(
                    observations.write_sizes.iter().all(|size| *size == 1),
                    "fragmentation must split every UTF-8 and JSON boundary"
                );
            }
            DeliveryMode::OneCoalescedWrite => assert_eq!(
                observations.write_sizes.len(),
                1,
                "events and response must cross in one scheduled socket write"
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
fn unix_socket_response_correlation_reordering_is_terminal_and_at_most_once() {
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
        let fixture = UnixSocketFixture::unique();
        let peer_thread = fixture.bind().spawn(scenario, move |peer| {
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

        let mut client = connect_client(fixture.socket_path());
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
        assert_non_timeout_terminal(&mut client);
        assert_terminal_fast_fail(&mut client);

        let observations = join_peer(peer_thread);
        assert_eq!(
            observations.requests.len(),
            if matches!(fault, CorrelationFault::Duplicate) {
                2
            } else {
                1
            },
            "terminal fencing must stop every later request for {fault:?}"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum ReadFault {
    MalformedJson,
    TruncatedJson,
    CloseBeforeResponse,
}

#[test]
fn unix_socket_malformed_truncated_and_closed_frames_fail_boundedly() {
    for fault in [
        ReadFault::MalformedJson,
        ReadFault::TruncatedJson,
        ReadFault::CloseBeforeResponse,
    ] {
        let scenario = match fault {
            ReadFault::MalformedJson => "malformed-json",
            ReadFault::TruncatedJson => "truncated-json",
            ReadFault::CloseBeforeResponse => "read-close",
        };
        let fixture = UnixSocketFixture::unique();
        let peer_thread = fixture.bind().spawn(scenario, move |peer| {
            peer.read_request();
            match fault {
                ReadFault::MalformedJson => peer.write_bytes(
                    br#"{"request_id":1,"error":invalid}"#,
                    DeliveryMode::OneByteWrites,
                ),
                ReadFault::TruncatedJson => {
                    peer.write_bytes(br#"{"request_id":1,"error":"#, DeliveryMode::OneByteWrites)
                }
                ReadFault::CloseBeforeResponse => {}
            }
            if matches!(fault, ReadFault::MalformedJson) {
                peer.write_bytes(b"\n", DeliveryMode::OneCoalescedWrite);
            }
        });

        let mut client = connect_client(fixture.socket_path());
        take_initial_connected(&mut client);
        let started = Instant::now();
        let error = client
            .get_property("read-fault")
            .expect_err("invalid or closed response must fail the in-flight command");
        assert!(
            started.elapsed() < TEST_COMPLETION_BUDGET,
            "{fault:?} exceeded the bounded disconnect budget"
        );
        assert!(
            error.contains("invalid mpv IPC JSON") || error.contains("unexpected EOF"),
            "{fault:?}: {error}"
        );
        assert_non_timeout_terminal(&mut client);
        assert_terminal_fast_fail(&mut client);

        let observations = join_peer(peer_thread);
        assert_eq!(observations.requests.len(), 1);
        match fault {
            ReadFault::MalformedJson | ReadFault::TruncatedJson => {
                assert!(
                    observations.write_sizes.len() > 10,
                    "invalid frame must cross the socket boundary in small pieces"
                );
            }
            ReadFault::CloseBeforeResponse => {
                assert!(observations.write_sizes.is_empty());
            }
        }
    }
}

#[test]
fn unix_socket_server_disconnect_before_request_fences_client_reuse() {
    let fixture = UnixSocketFixture::unique();
    let (connected_tx, connected_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let peer_thread = fixture.bind().spawn("write-close", move |peer| {
        connected_tx
            .send(())
            .expect("test should publish server connection");
        release_rx
            .recv_timeout(TEST_COMPLETION_BUDGET)
            .expect("test should release the connected server");
        peer.shutdown_both();
    });

    let mut client = connect_client(fixture.socket_path());
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
        .expect_err("closed server endpoint must fail the command");
    assert!(
        started.elapsed() < TEST_COMPLETION_BUDGET,
        "disconnect exceeded the bounded failure budget"
    );
    assert!(
        error.contains("failed to write")
            || error.contains("failed to read")
            || error.contains("unexpected EOF"),
        "{error}"
    );
    assert_non_timeout_terminal(&mut client);
    assert_terminal_fast_fail(&mut client);
}

#[test]
fn unix_socket_write_half_close_preserves_prior_event_then_disconnects() {
    let fixture = UnixSocketFixture::unique();
    let event = json!({
        "event": "property-change",
        "name": "half-close",
        "data": "preserved",
    });
    let expected_event = event.clone();
    let peer_thread = fixture.bind().spawn("half-close", move |peer| {
        peer.read_request();
        peer.write_lines(&[event], DeliveryMode::OneCoalescedWrite);
        peer.shutdown_write();
    });

    let mut client = connect_client(fixture.socket_path());
    take_initial_connected(&mut client);
    let error = client
        .get_property("half-close")
        .expect_err("write-half close before the response must fail");
    assert!(error.contains("unexpected EOF"), "{error}");
    assert_eq!(
        client.take_pending_events(),
        vec![expected_event],
        "a complete event preceding the half-close must remain observable"
    );
    assert_non_timeout_terminal(&mut client);
    assert_terminal_fast_fail(&mut client);

    let observations = join_peer(peer_thread);
    assert_eq!(observations.requests.len(), 1);
    assert_eq!(observations.write_sizes.len(), 1);
}

#[test]
fn unix_socket_withheld_response_honors_deadline_and_fences_later_writes() {
    let fixture = UnixSocketFixture::unique();
    let (request_seen_tx, request_seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let peer_thread = fixture.bind().spawn("timeout", move |peer| {
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
        MpvJsonIpcClient::connect_with_command_timeout(fixture.socket_path(), command_timeout)
            .expect("production client should open timeout socket");
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
        "Unix socket read deadline exceeded its budget: {elapsed:?}"
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
fn unix_socket_replacement_client_recovers_on_the_same_owned_path() {
    let fixture = UnixSocketFixture::unique();
    let first_peer = fixture.bind().spawn("replacement-fail", |peer| {
        peer.read_request();
    });

    let mut first_client = connect_client(fixture.socket_path());
    take_initial_connected(&mut first_client);
    let first_generation = first_client.generation();
    let first_error = first_client
        .get_property("first-generation")
        .expect_err("first peer intentionally disconnects");
    assert!(first_error.contains("unexpected EOF"), "{first_error}");
    assert_non_timeout_terminal(&mut first_client);
    let first_observations = join_peer(first_peer);
    assert_eq!(first_observations.requests.len(), 1);
    drop(first_client);

    let replacement_peer = fixture.bind().spawn("replacement-ok", |peer| {
        let request = peer.read_request();
        peer.write_lines(
            &[success_response(
                request_id(&request),
                json!("replacement-ok"),
            )],
            DeliveryMode::OneCoalescedWrite,
        );
    });
    let mut replacement = connect_client(fixture.socket_path());
    take_initial_connected(&mut replacement);
    assert_ne!(
        replacement.generation(),
        first_generation,
        "replacement client must own a fresh logical generation"
    );
    assert_eq!(
        replacement
            .get_property("replacement")
            .expect("replacement client should use the rebound socket"),
        Some(json!("replacement-ok"))
    );
    assert!(replacement.is_healthy());

    let observations = join_peer(replacement_peer);
    assert_eq!(observations.requests.len(), 1);
    assert_eq!(
        request_id(&observations.requests[0].value),
        1,
        "replacement client must restart request correlation independently"
    );
}

#[test]
fn unix_socket_request_ids_wrap_without_losing_response_correlation() {
    let fixture = UnixSocketFixture::unique();
    let peer_thread = fixture.bind().spawn("request-id-wrap", |peer| {
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
        fixture.socket_path(),
        ORDINARY_COMMAND_TIMEOUT,
        u64::MAX,
    )
    .expect("production client should open rollover socket");
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

#[test]
fn unix_socket_idle_client_drop_releases_stream_and_fixture_paths() {
    let (root_path, socket_path) = {
        let fixture = UnixSocketFixture::unique();
        let root_path = fixture.root_path().to_owned();
        let socket_path = fixture.socket_path().to_owned();
        let peer_thread = fixture.bind().spawn("idle-drop", |peer| {
            peer.expect_client_eof();
        });
        assert!(root_path.is_dir());
        assert!(socket_path.exists());

        let mut client = connect_client(fixture.socket_path());
        take_initial_connected(&mut client);
        drop(client);

        let observations = join_peer(peer_thread);
        assert!(observations.requests.is_empty());
        assert!(observations.write_sizes.is_empty());
        (root_path, socket_path)
    };

    assert!(
        !socket_path.exists(),
        "fixture drop must remove its owned socket path"
    );
    assert!(
        !root_path.exists(),
        "fixture drop must remove its nonce-owned temp root"
    );
}
