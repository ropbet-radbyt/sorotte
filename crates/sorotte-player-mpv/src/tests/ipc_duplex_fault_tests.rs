use crate::{
    constants::SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
    ipc::{
        MpvIpcConnectionEvent, MpvIpcNonblockingCommandCompletion, MpvIpcNonblockingRuntimeItem,
        MpvJsonIpcClient, MpvJsonIpcTransport, read_line_from_stream,
    },
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Read},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuplexFaultAction {
    SplitSuccess,
    CoalescedSuccess,
    ServerReject,
    DuplicatePreviousResponse,
    FutureResponseFirst,
    ReadHalfClose,
    WriteDisconnect,
    DelayedSuccess,
}

impl DuplexFaultAction {
    const MODEL_ACTIONS: [Self; 7] = [
        Self::SplitSuccess,
        Self::CoalescedSuccess,
        Self::ServerReject,
        Self::DuplicatePreviousResponse,
        Self::FutureResponseFirst,
        Self::ReadHalfClose,
        Self::WriteDisconnect,
    ];

    fn is_fatal(self) -> bool {
        matches!(
            self,
            Self::DuplicatePreviousResponse
                | Self::FutureResponseFirst
                | Self::ReadHalfClose
                | Self::WriteDisconnect
        )
    }
}

#[derive(Clone, Debug, Default)]
struct DuplexObservations {
    writes: Vec<Value>,
    raw_writes: Vec<String>,
    read_chunks: Vec<(u64, usize)>,
    released_through_request_id: u64,
}

#[derive(Clone, Debug)]
struct DuplexFaultHandle {
    shared: Arc<(Mutex<DuplexObservations>, Condvar)>,
}

impl DuplexFaultHandle {
    fn snapshot(&self) -> DuplexObservations {
        self.shared
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn wait_for_write_count(&self, expected: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (mutex, changed) = &*self.shared;
        let mut observations = mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while observations.writes.len() < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let waited = changed
                .wait_timeout(observations, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            observations = waited.0;
            if waited.1.timed_out() {
                break;
            }
        }
        observations.writes.len() >= expected
    }

    fn release_response(&self, request_id: u64) {
        let (mutex, changed) = &*self.shared;
        let mut observations = mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        observations.released_through_request_id =
            observations.released_through_request_id.max(request_id);
        changed.notify_all();
    }
}

#[derive(Debug)]
struct DuplexWire {
    bytes: VecDeque<u8>,
    max_chunk_bytes: usize,
    request_id: u64,
    shared: Arc<(Mutex<DuplexObservations>, Condvar)>,
}

impl DuplexWire {
    fn new(shared: Arc<(Mutex<DuplexObservations>, Condvar)>) -> Self {
        Self {
            bytes: VecDeque::new(),
            max_chunk_bytes: usize::MAX,
            request_id: 0,
            shared,
        }
    }

    fn replace(&mut self, request_id: u64, bytes: Vec<u8>, max_chunk_bytes: usize) {
        self.bytes = bytes.into();
        self.max_chunk_bytes = max_chunk_bytes;
        self.request_id = request_id;
    }
}

impl Read for DuplexWire {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let bytes_to_read = output.len().min(self.max_chunk_bytes).min(self.bytes.len());
        for slot in &mut output[..bytes_to_read] {
            *slot = self
                .bytes
                .pop_front()
                .expect("the bounded duplex wire length should remain available");
        }
        self.shared
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read_chunks
            .push((self.request_id, bytes_to_read));
        Ok(bytes_to_read)
    }
}

#[derive(Debug)]
struct DuplexFaultTransport {
    actions: VecDeque<DuplexFaultAction>,
    wire: DuplexWire,
    read_buffer: Vec<u8>,
    previous_response: Option<Value>,
    wait_for_release: Option<u64>,
    shared: Arc<(Mutex<DuplexObservations>, Condvar)>,
}

impl DuplexFaultTransport {
    fn new(actions: impl IntoIterator<Item = DuplexFaultAction>) -> (Self, DuplexFaultHandle) {
        let shared = Arc::new((Mutex::new(DuplexObservations::default()), Condvar::new()));
        let handle = DuplexFaultHandle {
            shared: Arc::clone(&shared),
        };
        (
            Self {
                actions: actions.into_iter().collect(),
                wire: DuplexWire::new(Arc::clone(&shared)),
                read_buffer: Vec::new(),
                previous_response: None,
                wait_for_release: None,
                shared,
            },
            handle,
        )
    }

    fn property_event(request_id: u64, ordinal: u64) -> Value {
        json!({
            "event": "property-change",
            "name": format!("duplex-event-{request_id}-{ordinal}"),
            "data": request_id * 10 + ordinal,
        })
    }

    fn control_event(request_id: u64, ordinal: u64) -> Value {
        json!({
            "event": "client-message",
            "args": [
                SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
                json!({
                    "requestId": request_id,
                    "ordinal": ordinal,
                })
                .to_string(),
            ],
        })
    }

    fn encode_lines(lines: &[Value]) -> Vec<u8> {
        let mut wire = String::new();
        for line in lines {
            wire.push_str(&line.to_string());
            wire.push('\n');
        }
        wire.into_bytes()
    }

    fn wait_until_released(&self, request_id: u64, deadline: Instant) -> io::Result<()> {
        let (mutex, changed) = &*self.shared;
        let mut observations = mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while observations.released_through_request_id < request_id {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "duplex peer withheld its response until the command deadline",
                ));
            }
            let waited = changed
                .wait_timeout(observations, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            observations = waited.0;
            if waited.1.timed_out() && observations.released_through_request_id < request_id {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "duplex peer withheld its response until the command deadline",
                ));
            }
        }
        Ok(())
    }
}

impl MpvJsonIpcTransport for DuplexFaultTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        if !self.read_buffer.is_empty() || !self.wire.bytes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the next request arrived before the prior duplex frame was consumed",
            ));
        }
        if !line.ends_with('\n') || line.trim_end_matches('\n').contains('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mpv IPC request was not exactly one newline-delimited frame",
            ));
        }
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request
            .get("request_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request_id missing"))?;
        let property = request
            .pointer("/command/1")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "property missing"))?;
        {
            let (observations, changed) = &*self.shared;
            let mut observations = observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            observations.writes.push(request.clone());
            observations.raw_writes.push(line.to_owned());
            changed.notify_all();
        }

        let action = self.actions.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "duplex peer received an unscripted request",
            )
        })?;
        if action == DuplexFaultAction::WriteDisconnect {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "duplex peer closed its request half",
            ));
        }

        let matching_response = json!({
            "request_id": request_id,
            "error": "success",
            "data": property,
        });
        let previous_response = self.previous_response.replace(matching_response.clone());
        let (lines, max_chunk_bytes) = match action {
            DuplexFaultAction::SplitSuccess => (
                vec![
                    Self::property_event(request_id, 1),
                    Self::property_event(request_id, 2),
                    matching_response,
                ],
                1,
            ),
            DuplexFaultAction::CoalescedSuccess => (
                vec![
                    Self::property_event(request_id, 1),
                    Self::property_event(request_id, 2),
                    matching_response,
                ],
                usize::MAX,
            ),
            DuplexFaultAction::ServerReject => (
                vec![
                    Self::property_event(request_id, 1),
                    json!({
                        "request_id": request_id,
                        "error": "property unavailable",
                    }),
                ],
                3,
            ),
            DuplexFaultAction::DuplicatePreviousResponse => (
                vec![
                    previous_response.unwrap_or_else(|| {
                        json!({
                            "request_id": 0,
                            "error": "success",
                            "data": "stale",
                        })
                    }),
                    matching_response,
                ],
                usize::MAX,
            ),
            DuplexFaultAction::FutureResponseFirst => (
                vec![
                    json!({
                        "request_id": request_id.saturating_add(1),
                        "error": "success",
                        "data": "future",
                    }),
                    matching_response,
                ],
                2,
            ),
            DuplexFaultAction::ReadHalfClose => (vec![Self::property_event(request_id, 1)], 1),
            DuplexFaultAction::DelayedSuccess => {
                self.wait_for_release = Some(request_id);
                (
                    vec![
                        Self::control_event(request_id, 1),
                        Self::control_event(request_id, 2),
                        matching_response,
                    ],
                    2,
                )
            }
            DuplexFaultAction::WriteDisconnect => unreachable!("handled before response setup"),
        };
        self.wire
            .replace(request_id, Self::encode_lines(&lines), max_chunk_bytes);
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, deadline: Instant) -> io::Result<usize> {
        if let Some(request_id) = self.wait_for_release.take() {
            self.wait_until_released(request_id, deadline)?;
        }
        read_line_from_stream(&mut self.wire, &mut self.read_buffer, line)
    }
}

fn expected_property_events(request_id: u64, count: u64) -> Vec<Value> {
    (1..=count)
        .map(|ordinal| DuplexFaultTransport::property_event(request_id, ordinal))
        .collect()
}

fn assert_connection_outcome(
    action: DuplexFaultAction,
    events: &[MpvIpcConnectionEvent],
    history: &[DuplexFaultAction],
    step: usize,
) {
    let command_failures = events
        .iter()
        .filter(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. }))
        .count();
    let timeouts = events
        .iter()
        .filter(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. }))
        .count();
    let disconnects = events
        .iter()
        .filter(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
        .count();
    let expected = if matches!(
        action,
        DuplexFaultAction::SplitSuccess | DuplexFaultAction::CoalescedSuccess
    ) {
        (0, 0, 0)
    } else if action == DuplexFaultAction::ServerReject {
        (1, 0, 0)
    } else {
        (1, 0, 1)
    };
    assert_eq!(
        (command_failures, timeouts, disconnects),
        expected,
        "connection outcome diverged for history {history:?} at step {step}: {events:?}"
    );
}

fn exercise_duplex_history(history: [DuplexFaultAction; 3]) {
    let (transport, handle) = DuplexFaultTransport::new(history);
    let command_timeout = Duration::from_secs(1);
    let mut client =
        MpvJsonIpcClient::new_with_command_timeout(Box::new(transport), command_timeout);
    assert!(matches!(
        client.take_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { .. }]
    ));

    let mut terminal = false;
    let mut expected_write_count = 0;
    for (step, action) in history.into_iter().enumerate() {
        let property = format!("model-property-{step}");
        let started = Instant::now();
        let result = client.get_property(&property);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "history {history:?} step {step} exceeded the bounded command outcome"
        );

        if terminal {
            let error = result.expect_err("a terminal IPC client must reject later commands");
            assert!(
                error.contains("not connected"),
                "{history:?} step {step}: {error}"
            );
            assert_eq!(
                handle.snapshot().writes.len(),
                expected_write_count,
                "a terminal client wrote another request for history {history:?} at step {step}"
            );
            assert!(
                client.take_connection_events().is_empty(),
                "terminal fast-fail must not duplicate disconnect events for {history:?}"
            );
            continue;
        }

        expected_write_count += 1;
        let request_id = u64::try_from(expected_write_count).expect("small model request id");
        let expected_events = match action {
            DuplexFaultAction::SplitSuccess | DuplexFaultAction::CoalescedSuccess => {
                expected_property_events(request_id, 2)
            }
            DuplexFaultAction::ServerReject | DuplexFaultAction::ReadHalfClose => {
                expected_property_events(request_id, 1)
            }
            DuplexFaultAction::DuplicatePreviousResponse
            | DuplexFaultAction::FutureResponseFirst
            | DuplexFaultAction::WriteDisconnect => Vec::new(),
            DuplexFaultAction::DelayedSuccess => unreachable!("not part of model histories"),
        };
        match action {
            DuplexFaultAction::SplitSuccess | DuplexFaultAction::CoalescedSuccess => {
                assert_eq!(
                    result.unwrap_or_else(|error| {
                        panic!("history {history:?} step {step} should succeed: {error}")
                    }),
                    Some(json!(property))
                );
                assert!(client.is_healthy());
            }
            DuplexFaultAction::ServerReject => {
                let error = result.expect_err("an ordinary server rejection must reach the caller");
                assert!(error.contains("property unavailable"), "{error}");
                assert!(
                    client.is_healthy(),
                    "server rejection must allow a later command for {history:?}"
                );
            }
            DuplexFaultAction::DuplicatePreviousResponse => {
                let error = result.expect_err("a stale duplicate must not satisfy a new request");
                assert!(error.contains("request_id mismatch"), "{error}");
                assert!(!client.is_healthy());
            }
            DuplexFaultAction::FutureResponseFirst => {
                let error = result.expect_err("an early future response must fail correlation");
                assert!(error.contains("request_id mismatch"), "{error}");
                assert!(!client.is_healthy());
            }
            DuplexFaultAction::ReadHalfClose => {
                let error =
                    result.expect_err("read half-close must terminate the in-flight command");
                assert!(error.contains("unexpected EOF"), "{error}");
                assert!(!client.is_healthy());
            }
            DuplexFaultAction::WriteDisconnect => {
                let error =
                    result.expect_err("write half-close must terminate the in-flight command");
                assert!(error.contains("failed to write"), "{error}");
                assert!(!client.is_healthy());
            }
            DuplexFaultAction::DelayedSuccess => unreachable!("not part of model histories"),
        }
        assert_eq!(
            client.take_pending_events(),
            expected_events,
            "event ordering or at-most-once delivery diverged for {history:?} at step {step}"
        );
        assert_connection_outcome(action, &client.take_connection_events(), &history, step);

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.writes.len(), expected_write_count);
        assert_eq!(
            snapshot.writes[step]["request_id"],
            json!(request_id),
            "request correlation must be consecutive for {history:?}"
        );
        assert_eq!(
            snapshot.writes[step]["command"],
            json!(["get_property", property]),
            "duplex peer must observe the exact command being modeled"
        );
        assert!(
            snapshot.raw_writes[step].ends_with('\n'),
            "production writer must retain newline framing"
        );
        let nonempty_read_chunks = snapshot
            .read_chunks
            .iter()
            .filter_map(|(observed_request_id, bytes)| {
                (*observed_request_id == request_id && *bytes > 0).then_some(*bytes)
            })
            .collect::<Vec<_>>();
        match action {
            DuplexFaultAction::SplitSuccess => {
                assert!(
                    nonempty_read_chunks.len() > 3
                        && nonempty_read_chunks.iter().all(|bytes| *bytes == 1),
                    "split delivery was not exercised for {history:?}: {nonempty_read_chunks:?}"
                );
            }
            DuplexFaultAction::CoalescedSuccess => assert_eq!(
                nonempty_read_chunks.len(),
                1,
                "coalesced delivery should enter through one stream read for {history:?}"
            ),
            _ => {}
        }
        terminal = action.is_fatal();
    }
}

#[test]
fn duplex_fault_histories_match_independent_correlation_and_terminal_model() {
    let mut histories = 0;
    let mut transitions = 0;
    for first in DuplexFaultAction::MODEL_ACTIONS {
        for second in DuplexFaultAction::MODEL_ACTIONS {
            for third in DuplexFaultAction::MODEL_ACTIONS {
                exercise_duplex_history([first, second, third]);
                histories += 1;
                transitions += 3;
            }
        }
    }
    assert_eq!(histories, 343);
    assert_eq!(transitions, 1_029);
}

#[test]
fn delayed_duplex_response_preserves_event_completion_order_and_at_most_once_delivery() {
    let (transport, handle) = DuplexFaultTransport::new([
        DuplexFaultAction::DelayedSuccess,
        DuplexFaultAction::CoalescedSuccess,
    ]);
    let mut client =
        MpvJsonIpcClient::new_with_command_timeout(Box::new(transport), Duration::from_millis(250));
    client.take_connection_events();

    assert_eq!(
        client.try_get_property_nonblocking("delayed-property", 73),
        Ok(Some(1))
    );
    assert!(handle.wait_for_write_count(1, Duration::from_secs(1)));
    assert_eq!(
        client.try_get_property_nonblocking("must-not-overtake", 74),
        Ok(None),
        "one delayed command must fence a second nonblocking request"
    );
    assert_eq!(
        handle.snapshot().writes.len(),
        1,
        "the fenced request must not reach the duplex writer"
    );
    assert!(
        client
            .take_nonblocking_runtime_items_matching(|_| true)
            .is_empty(),
        "the delayed peer has not released any event or completion yet"
    );

    handle.release_response(1);
    let observation_deadline = Instant::now() + Duration::from_secs(1);
    let mut runtime_items = Vec::new();
    while runtime_items.len() < 3 && Instant::now() < observation_deadline {
        runtime_items.extend(client.take_nonblocking_runtime_items_matching(|_| true));
        std::thread::yield_now();
    }
    assert_eq!(runtime_items.len(), 3);
    assert!(matches!(
        &runtime_items[0],
        MpvIpcNonblockingRuntimeItem::Event(event)
            if event.value == DuplexFaultTransport::control_event(1, 1)
    ));
    assert!(matches!(
        &runtime_items[1],
        MpvIpcNonblockingRuntimeItem::Event(event)
            if event.value == DuplexFaultTransport::control_event(1, 2)
    ));
    assert!(matches!(
        &runtime_items[2],
        MpvIpcNonblockingRuntimeItem::Completion(
            MpvIpcNonblockingCommandCompletion::SucceededWithResponse {
                command_id: 1,
                token: 73,
                response,
            }
        ) if response == &json!({
            "request_id": 1,
            "error": "success",
            "data": "delayed-property",
        })
    ));
    assert!(
        client
            .take_nonblocking_runtime_items_matching(|_| true)
            .is_empty(),
        "events and completion must be delivered at most once"
    );

    assert_eq!(
        client
            .get_property("after-delay")
            .expect("a released delay must preserve connection health"),
        Some(json!("after-delay"))
    );
    assert_eq!(
        handle
            .snapshot()
            .writes
            .iter()
            .map(|request| request["request_id"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2)]
    );
}

#[test]
fn withheld_duplex_response_times_out_once_and_terminally_fences_later_writes() {
    let command_timeout = Duration::from_millis(30);
    let (transport, handle) = DuplexFaultTransport::new([DuplexFaultAction::DelayedSuccess]);
    let mut client =
        MpvJsonIpcClient::new_with_command_timeout(Box::new(transport), command_timeout);
    client.take_connection_events();

    let started = Instant::now();
    let error = client
        .get_property("dropped-response")
        .expect_err("a withheld response must stop at the production command deadline");
    let elapsed = started.elapsed();
    assert!(error.contains("timed out"), "{error}");
    assert!(
        elapsed >= command_timeout.saturating_sub(Duration::from_millis(5)),
        "the duplex peer did not actually withhold the response: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "the command exceeded its bounded termination budget: {elapsed:?}"
    );
    assert!(!client.is_healthy());
    let events = client.take_connection_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
            .count(),
        1
    );

    let fast_fail_started = Instant::now();
    let next_error = client
        .get_property("must-not-write")
        .expect_err("a timed-out connection must remain terminal");
    assert!(next_error.contains("not connected"), "{next_error}");
    assert!(fast_fail_started.elapsed() < Duration::from_millis(100));
    assert_eq!(handle.snapshot().writes.len(), 1);
    assert!(
        client.take_connection_events().is_empty(),
        "terminal reuse must not duplicate the timeout or disconnect"
    );
}
