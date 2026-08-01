use super::*;
use crate::constants::SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT;
use crate::ipc::{MPV_IPC_MAX_LINE_BYTES, MpvIpcConnectionEvent, MpvJsonIpcClient};
use crate::{
    MpvNetworkMediaPolicyApplicationState, MpvNetworkOptionApplyResult, MpvNetworkOptionApplyStatus,
};
use sorotte_player_api::PlayerCapabilities;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
struct NeverRespondingTransport;

#[derive(Debug)]
struct FragmentingReader {
    bytes: io::Cursor<Vec<u8>>,
    max_chunk_bytes: usize,
}

impl FragmentingReader {
    fn new(bytes: Vec<u8>, max_chunk_bytes: usize) -> Self {
        assert!(max_chunk_bytes > 0);
        Self {
            bytes: io::Cursor::new(bytes),
            max_chunk_bytes,
        }
    }
}

impl io::Read for FragmentingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let bounded_len = output.len().min(self.max_chunk_bytes);
        io::Read::read(&mut self.bytes, &mut output[..bounded_len])
    }
}

#[derive(Debug)]
struct FramedWireTransport {
    reader: FragmentingReader,
    read_buffer: Vec<u8>,
    writes: Arc<Mutex<Vec<String>>>,
}

impl FramedWireTransport {
    fn new(wire: Vec<u8>, max_chunk_bytes: usize) -> (Self, Arc<Mutex<Vec<String>>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                reader: FragmentingReader::new(wire, max_chunk_bytes),
                read_buffer: Vec::new(),
                writes: Arc::clone(&writes),
            },
            writes,
        )
    }
}

impl MpvJsonIpcTransport for FramedWireTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("framed-wire writes should not be poisoned")
            .push(line.to_owned());
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        read_line_from_stream(&mut self.reader, &mut self.read_buffer, line)
    }
}

#[derive(Debug)]
struct NetworkOptionsHookSupersessionTransport {
    writes: Arc<Mutex<Vec<String>>>,
    responses: VecDeque<String>,
}

#[derive(Clone, Copy, Debug)]
enum HookSupersessionTarget {
    StableNetwork,
    Local,
    Idle,
    NetworkSuccess,
    NetworkFailure,
    NetworkAwaitingResult,
}

#[derive(Debug)]
struct NetworkOptionsHookScenarioTransport {
    writes: Arc<Mutex<Vec<String>>>,
    responses: VecDeque<String>,
    old_network_succeeds: bool,
    target: HookSupersessionTarget,
    lose_ownership_on_heartbeat: bool,
    acknowledge_heartbeats: bool,
    expire_ownership_on_second_active_apply: bool,
    active_apply_count: usize,
}

impl NetworkOptionsHookScenarioTransport {
    fn push(&mut self, value: Value) {
        self.responses.push_back(value.to_string() + "\n");
    }

    fn client_message(name: &str, payload: Value) -> Value {
        json!({
            "event": "client-message",
            "args": [name, payload.to_string()],
        })
    }
}

impl MpvJsonIpcTransport for NetworkOptionsHookScenarioTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("network-options scenario writes should not be poisoned")
            .push(line.to_owned());
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request["request_id"]
            .as_u64()
            .expect("test request should contain an id");
        let command = request["command"]
            .as_array()
            .expect("test request should contain a command");
        match command.first().and_then(Value::as_str) {
            Some("get_property") if command.get(1).and_then(Value::as_str) == Some("path") => {
                self.push(json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": "https://media.example.test/a.m3u8",
                }));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_configure") =>
            {
                let payload: Value = serde_json::from_str(
                    command
                        .get(3)
                        .and_then(Value::as_str)
                        .expect("configure command should contain JSON"),
                )
                .expect("configure payload should be valid");
                let current_load_sequence = if self.active_apply_count == 0 {
                    0
                } else if matches!(self.target, HookSupersessionTarget::StableNetwork) {
                    1
                } else {
                    2
                };
                self.push(Self::client_message(
                    "sorotte-network-options-configured",
                    json!({
                        "protocol": "sorotte-network-options-v3",
                        "ownerId": payload["ownerId"],
                        "attachmentId": payload["attachmentId"],
                        "configurationGeneration": payload["configurationGeneration"],
                        "hookInstanceId": "scenario-hook-instance",
                        "currentLoadSequence": current_load_sequence,
                        "status": "configured",
                    }),
                ));
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_apply_active") =>
            {
                let payload: Value = serde_json::from_str(
                    command
                        .get(3)
                        .and_then(Value::as_str)
                        .expect("apply command should contain JSON"),
                )
                .expect("apply payload should be valid");
                self.active_apply_count += 1;
                if self.expire_ownership_on_second_active_apply && self.active_apply_count == 2 {
                    self.push(Self::client_message(
                        "sorotte-network-options-ownership",
                        json!({
                            "protocol": "sorotte-network-options-v3",
                            "ownerId": payload["ownerId"],
                            "attachmentId": payload["attachmentId"],
                            "configurationGeneration": payload["configurationGeneration"],
                            "hookInstanceId": "scenario-hook-instance",
                            "status": "lease-expired",
                        }),
                    ));
                    self.push(json!({"request_id": request_id, "error": "success"}));
                    return Ok(());
                }
                let base = json!({
                    "protocol": "sorotte-network-options-v3",
                    "ownerId": payload["ownerId"],
                    "attachmentId": payload["attachmentId"],
                    "configurationGeneration": payload["configurationGeneration"],
                    "hookInstanceId": "scenario-hook-instance",
                });
                let mut active_result = base.clone();
                active_result["attempt"] = payload["attempt"].clone();
                active_result["loadSequence"] = json!(1);
                active_result["sourcePath"] = json!("https://media.example.test/a.m3u8");
                active_result["streamOpenFilename"] = json!("https://media.example.test/a.m3u8");
                if self.old_network_succeeds {
                    active_result["status"] = json!("network-updated");
                } else {
                    active_result["status"] = json!("failed");
                    active_result["error"] = json!("A rejected cache-secs");
                }
                self.push(Self::client_message(
                    "sorotte-network-options-active-result",
                    active_result,
                ));

                match self.target {
                    HookSupersessionTarget::StableNetwork => {}
                    HookSupersessionTarget::Local => {
                        self.push(json!({"event": "start-file", "playlist_entry_id": 102}));
                        self.push(json!({
                            "event": "property-change",
                            "name": "path",
                            "data": "C:/media/local-b.mkv",
                        }));
                        let mut transition = base;
                        transition["loadSequence"] = json!(2);
                        transition["sourcePath"] = json!("C:/media/local-b.mkv");
                        transition["streamOpenFilename"] = json!("C:/media/local-b.mkv");
                        transition["status"] = json!("local");
                        self.push(Self::client_message(
                            "sorotte-network-options-transition-result",
                            transition,
                        ));
                    }
                    HookSupersessionTarget::Idle => {
                        self.push(json!({
                            "event": "property-change",
                            "name": "path",
                            "data": null,
                        }));
                    }
                    HookSupersessionTarget::NetworkSuccess
                    | HookSupersessionTarget::NetworkFailure
                    | HookSupersessionTarget::NetworkAwaitingResult => {
                        self.push(json!({"event": "start-file", "playlist_entry_id": 103}));
                        self.push(json!({
                            "event": "property-change",
                            "name": "path",
                            "data": "https://media.example.test/b.m3u8",
                        }));
                        if !matches!(self.target, HookSupersessionTarget::NetworkAwaitingResult) {
                            let mut transition = base;
                            transition["loadSequence"] = json!(2);
                            transition["sourcePath"] = json!("https://media.example.test/b.m3u8");
                            transition["streamOpenFilename"] =
                                json!("https://media.example.test/b.m3u8");
                            if matches!(self.target, HookSupersessionTarget::NetworkSuccess) {
                                transition["status"] = json!("network-updated");
                            } else {
                                transition["status"] = json!("failed");
                                transition["error"] = json!("B rejected cache-secs");
                            }
                            self.push(Self::client_message(
                                "sorotte-network-options-transition-result",
                                transition,
                            ));
                        }
                    }
                }
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_heartbeat") =>
            {
                let payload: Value =
                    serde_json::from_str(command.get(3).and_then(Value::as_str).unwrap()).unwrap();
                if self.lose_ownership_on_heartbeat {
                    self.push(Self::client_message(
                        "sorotte-network-options-ownership",
                        json!({
                            "protocol": "sorotte-network-options-v3",
                            "ownerId": payload["ownerId"],
                            "attachmentId": payload["attachmentId"],
                            "configurationGeneration": payload["configurationGeneration"],
                            "hookInstanceId": "scenario-hook-instance",
                            "status": "ownership-lost",
                        }),
                    ));
                } else if self.acknowledge_heartbeats {
                    self.push(Self::client_message(
                        "sorotte-network-options-heartbeat",
                        json!({
                            "protocol": "sorotte-network-options-v3",
                            "ownerId": payload["ownerId"],
                            "attachmentId": payload["attachmentId"],
                            "configurationGeneration": payload["configurationGeneration"],
                            "hookInstanceId": "scenario-hook-instance",
                            "heartbeatNonce": payload["heartbeatNonce"],
                            "status": "renewed",
                        }),
                    ));
                }
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            _ => self.push(json!({"request_id": request_id, "error": "success"})),
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "network-options scenario omitted a response",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

#[derive(Debug)]
struct NetworkOptionsHookConfigurationTimeoutTransport {
    writes: Arc<Mutex<Vec<String>>>,
    responses: VecDeque<String>,
    transition_injected: bool,
}

impl NetworkOptionsHookConfigurationTimeoutTransport {
    fn push(&mut self, value: Value) {
        self.responses.push_back(value.to_string() + "\n");
    }
}

impl MpvJsonIpcTransport for NetworkOptionsHookConfigurationTimeoutTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("hook timeout writes should not be poisoned")
            .push(line.to_owned());
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request["request_id"].as_u64().unwrap();
        let command = request["command"].as_array().unwrap();
        if command.first().and_then(Value::as_str) == Some("script-message-to")
            && command.get(2).and_then(Value::as_str) == Some("sorotte_network_options_configure")
            && !self.transition_injected
        {
            self.transition_injected = true;
            self.push(json!({"event": "start-file", "playlist_entry_id": 201}));
            self.push(json!({
                "event": "property-change",
                "name": "path",
                "data": "https://media.example.test/a.m3u8",
            }));
            self.push(json!({"event": "start-file", "playlist_entry_id": 202}));
            self.push(json!({
                "event": "property-change",
                "name": "path",
                "data": "C:/media/local-b.mkv",
            }));
        }
        self.push(json!({"request_id": request_id, "error": "success"}));
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "hook timeout response missing",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

#[derive(Debug)]
struct NetworkOptionsHookReloadTransport {
    writes: Arc<Mutex<Vec<String>>>,
    responses: VecDeque<String>,
    configure_attempts: usize,
}

impl NetworkOptionsHookReloadTransport {
    fn push(&mut self, value: Value) {
        self.responses.push_back(value.to_string() + "\n");
    }
}

impl MpvJsonIpcTransport for NetworkOptionsHookReloadTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("hook reload writes should not be poisoned")
            .push(line.to_owned());
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request["request_id"].as_u64().unwrap();
        let command = request["command"].as_array().unwrap();
        match command.first().and_then(Value::as_str) {
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_configure") =>
            {
                self.configure_attempts += 1;
                if self.configure_attempts == 1 {
                    self.push(json!({
                        "request_id": request_id,
                        "error": "target client not found",
                    }));
                } else {
                    let payload: Value =
                        serde_json::from_str(command.get(3).and_then(Value::as_str).unwrap())
                            .unwrap();
                    self.push(NetworkOptionsHookScenarioTransport::client_message(
                        "sorotte-network-options-configured",
                        json!({
                            "protocol": "sorotte-network-options-v3",
                            "ownerId": payload["ownerId"],
                            "attachmentId": payload["attachmentId"],
                            "configurationGeneration": payload["configurationGeneration"],
                            "hookInstanceId": "reload-hook-instance",
                            "currentLoadSequence": 0,
                            "status": "configured",
                        }),
                    ));
                    self.push(json!({"request_id": request_id, "error": "success"}));
                }
            }
            Some("get_property") if command.get(1).and_then(Value::as_str) == Some("path") => {
                self.push(json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": "https://media.example.test/recovered.m3u8",
                }));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_apply_active") =>
            {
                let payload: Value =
                    serde_json::from_str(command.get(3).and_then(Value::as_str).unwrap()).unwrap();
                self.push(NetworkOptionsHookScenarioTransport::client_message(
                    "sorotte-network-options-active-result",
                    json!({
                        "protocol": "sorotte-network-options-v3",
                        "ownerId": payload["ownerId"],
                        "attachmentId": payload["attachmentId"],
                        "configurationGeneration": payload["configurationGeneration"],
                        "hookInstanceId": "reload-hook-instance",
                        "attempt": payload["attempt"],
                        "loadSequence": 0,
                        "sourcePath": "https://media.example.test/recovered.m3u8",
                        "streamOpenFilename": "https://media.example.test/recovered.m3u8",
                        "status": "network-updated",
                    }),
                ));
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            _ => self.push(json!({"request_id": request_id, "error": "success"})),
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "hook reload response missing")
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

impl NetworkOptionsHookSupersessionTransport {
    fn push(&mut self, value: Value) {
        self.responses.push_back(value.to_string() + "\n");
    }
}

impl MpvJsonIpcTransport for NetworkOptionsHookSupersessionTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        self.writes
            .lock()
            .expect("network-options hook writes should not be poisoned")
            .push(line.to_owned());
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request["request_id"]
            .as_u64()
            .expect("test request should contain an id");
        let command = request["command"]
            .as_array()
            .expect("test request should contain a command");
        match command.first().and_then(Value::as_str) {
            Some("get_property") if command.get(1).and_then(Value::as_str) == Some("path") => {
                self.push(json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": "https://media.example.test/a.m3u8",
                }));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_configure") =>
            {
                let payload: Value = serde_json::from_str(
                    command
                        .get(3)
                        .and_then(Value::as_str)
                        .expect("configure command should contain JSON"),
                )
                .expect("configure payload should be valid");
                self.push(json!({
                    "event": "client-message",
                    "args": ["sorotte-network-options-configured", json!({
                        "protocol": "sorotte-network-options-v3",
                        "ownerId": payload["ownerId"],
                        "attachmentId": payload["attachmentId"],
                        "configurationGeneration": payload["configurationGeneration"],
                        "hookInstanceId": "supersession-hook-instance",
                        "currentLoadSequence": 0,
                        "status": "configured",
                    }).to_string()],
                }));
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            Some("script-message-to")
                if command.get(2).and_then(Value::as_str)
                    == Some("sorotte_network_options_apply_active") =>
            {
                let payload: Value = serde_json::from_str(
                    command
                        .get(3)
                        .and_then(Value::as_str)
                        .expect("apply command should contain JSON"),
                )
                .expect("apply payload should be valid");
                let result = json!({
                    "protocol": "sorotte-network-options-v3",
                    "ownerId": payload["ownerId"],
                    "attachmentId": payload["attachmentId"],
                    "configurationGeneration": payload["configurationGeneration"],
                    "hookInstanceId": "supersession-hook-instance",
                    "loadSequence": 2,
                    "sourcePath": "https://media.example.test/b.m3u8",
                    "streamOpenFilename": "https://media.example.test/b.m3u8",
                    "status": "network-updated",
                });
                self.push(json!({
                    "event": "start-file",
                    "playlist_entry_id": 92,
                }));
                self.push(json!({
                    "event": "property-change",
                    "name": "path",
                    "data": "https://media.example.test/b.m3u8",
                }));
                self.push(json!({
                    "event": "client-message",
                    "args": ["sorotte-network-options-transition-result", result.to_string()],
                }));
                let mut active_result = result;
                active_result["attempt"] = payload["attempt"].clone();
                self.push(json!({
                    "event": "client-message",
                    "args": ["sorotte-network-options-active-result", active_result.to_string()],
                }));
                self.push(json!({"request_id": request_id, "error": "success"}));
            }
            _ => self.push(json!({"request_id": request_id, "error": "success"})),
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "network-options hook test omitted a response",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

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

#[derive(Debug)]
struct FailFirstFinalWriteTransport {
    writes: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
    sends: usize,
}

impl Drop for FailFirstFinalWriteTransport {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl MpvJsonIpcTransport for FailFirstFinalWriteTransport {
    fn send_line_until(&mut self, line: &str, deadline: Instant) -> io::Result<()> {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "cleanup command received no independent write budget",
            ));
        }
        self.writes
            .lock()
            .expect("final-write transport mutex should not be poisoned")
            .push(line.to_owned());
        self.sends += 1;
        if self.sends == 1 {
            if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "first cleanup write intentionally exhausted its deadline",
            ));
        }
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
fn terminal_cleanup_preserves_order_and_attempts_release_after_restore_failure() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let mut client = MpvJsonIpcClient::new(Box::new(FailFirstFinalWriteTransport {
        writes: Arc::clone(&writes),
        dropped: Arc::clone(&dropped),
        sends: 0,
    }));

    client.send_final_commands_best_effort(vec![
        json!(["set_property", "osd-align-y", "top"]),
        json!(["set_property", "osd-margin-y", 16]),
        json!([
            "script-message-to",
            "sorotte_syncplayintf",
            "release",
            "owner"
        ]),
    ]);

    assert!(
        dropped.load(Ordering::SeqCst),
        "terminal cleanup must finish its bounded attempts before returning"
    );
    let commands = writes
        .lock()
        .expect("final-write transport mutex should not be poisoned")
        .iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .expect("terminal command should be valid JSON")["command"]
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            json!(["set_property", "osd-align-y", "top"]),
            json!(["set_property", "osd-margin-y", 16]),
            json!([
                "script-message-to",
                "sorotte_syncplayintf",
                "release",
                "owner"
            ]),
        ],
        "a failed restoration write must not suppress the later release attempt"
    );
    assert!(!client.is_healthy());
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
fn buffered_read_line_from_stream_is_invariant_to_every_chunk_size() {
    let expected_lines = [
        "{\"event\":\"client-message\",\"args\":[\"snowman\",\"☃\"]}\r\n",
        "\n",
        "{\"request_id\":7,\"error\":\"success\"}\n",
        "{\"event\":\"shutdown\"}",
    ];
    let wire = expected_lines.concat().into_bytes();

    for max_chunk_bytes in 1..=wire.len() {
        let mut stream = FragmentingReader::new(wire.clone(), max_chunk_bytes);
        let mut read_buffer = Vec::new();
        let mut line = String::new();

        for expected in expected_lines {
            let bytes = read_line_from_stream(&mut stream, &mut read_buffer, &mut line)
                .unwrap_or_else(|error| {
                    panic!("chunk size {max_chunk_bytes} should decode {expected:?}: {error}")
                });
            assert_eq!(
                bytes,
                expected.len(),
                "chunk size {max_chunk_bytes} returned the wrong byte count"
            );
            assert_eq!(
                line, expected,
                "chunk size {max_chunk_bytes} changed frame boundaries"
            );
        }

        assert_eq!(
            read_line_from_stream(&mut stream, &mut read_buffer, &mut line)
                .expect("the fragmented stream should reach a clean EOF"),
            0,
            "chunk size {max_chunk_bytes} should be exhausted"
        );
        assert!(line.is_empty());
        assert!(read_buffer.is_empty());
    }
}

#[test]
fn buffered_read_line_from_stream_enforces_utf8_and_exact_size_boundaries() {
    let exact_line = format!("{}\r\n", "x".repeat(MPV_IPC_MAX_LINE_BYTES));
    let mut exact_stream = FragmentingReader::new(exact_line.as_bytes().to_vec(), 8 * 1024);
    let mut read_buffer = Vec::new();
    let mut line = String::new();
    assert_eq!(
        read_line_from_stream(&mut exact_stream, &mut read_buffer, &mut line)
            .expect("a line exactly at the content limit should be accepted"),
        exact_line.len()
    );
    assert_eq!(line, exact_line);

    let oversized_line = format!("{}\n", "x".repeat(MPV_IPC_MAX_LINE_BYTES + 1));
    let mut oversized_stream = FragmentingReader::new(oversized_line.as_bytes().to_vec(), 8 * 1024);
    let oversized_error = read_line_from_stream(&mut oversized_stream, &mut read_buffer, &mut line)
        .expect_err("one content byte over the limit must fail");
    assert_eq!(oversized_error.kind(), io::ErrorKind::InvalidData);
    assert!(oversized_error.to_string().contains("line too long"));

    for max_chunk_bytes in 1..=4 {
        let mut invalid_utf8 =
            FragmentingReader::new(vec![b'{', 0xff, b'}', b'\n'], max_chunk_bytes);
        let mut invalid_buffer = Vec::new();
        let mut invalid_line = String::new();
        let error =
            read_line_from_stream(&mut invalid_utf8, &mut invalid_buffer, &mut invalid_line)
                .expect_err("invalid UTF-8 must fail before JSON decoding");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not valid UTF-8"));
    }
}

#[test]
fn framed_worker_is_invariant_to_every_chunk_size_and_preserves_event_order() {
    let first_event = json!({
        "event": "property-change",
        "name": "pause",
        "data": true,
    });
    let second_event = json!({
        "event": "property-change",
        "name": "time-pos",
        "data": 12.5,
    });
    let response = json!({
        "request_id": 1,
        "error": "success",
        "data": false,
    });
    let wire = format!("\n{first_event}\r\n{second_event}\n{response}").into_bytes();

    for max_chunk_bytes in 1..=wire.len() {
        let (transport, writes) = FramedWireTransport::new(wire.clone(), max_chunk_bytes);
        let mut client = MpvJsonIpcClient::new(Box::new(transport));

        assert_eq!(
            client.get_property("pause").unwrap_or_else(|error| panic!(
                "chunk size {max_chunk_bytes} should reach the matching response: {error}"
            )),
            Some(json!(false))
        );
        assert_eq!(
            client.take_pending_events(),
            vec![first_event.clone(), second_event.clone()],
            "chunk size {max_chunk_bytes} changed event ordering"
        );

        let writes = writes
            .lock()
            .expect("framed-wire writes should not be poisoned");
        assert_eq!(writes.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(writes[0].trim_end())
                .expect("the production worker should emit valid JSON"),
            json!({
                "command": ["get_property", "pause"],
                "request_id": 1,
            })
        );
    }
}

#[test]
fn framed_worker_carries_coalesced_events_across_command_boundaries() {
    let first_response = json!({
        "request_id": 1,
        "error": "success",
        "data": false,
    });
    let intervening_event = json!({
        "event": "property-change",
        "name": "path",
        "data": "movie.mkv",
    });
    let second_response = json!({
        "request_id": 2,
        "error": "success",
        "data": "movie.mkv",
    });
    let wire = format!("{first_response}\n{intervening_event}\n{second_response}\n").into_bytes();
    let (transport, _writes) = FramedWireTransport::new(wire, usize::MAX);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client
            .get_property("pause")
            .expect("the first coalesced response should be consumed"),
        Some(json!(false))
    );
    assert_eq!(
        client
            .get_property_string("path")
            .expect("the next command should cross the buffered event"),
        Some("movie.mkv".to_owned())
    );
    assert_eq!(client.take_pending_events(), vec![intervening_event]);
}

#[test]
fn framed_worker_half_close_preserves_prior_events_and_disconnects() {
    let prior_event = json!({
        "event": "property-change",
        "name": "pause",
        "data": true,
    });
    let wire = format!("{prior_event}\n").into_bytes();
    let (transport, _writes) = FramedWireTransport::new(wire, 1);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let error = client
        .get_property("pause")
        .expect_err("EOF after an event must not impersonate a command response");

    assert!(error.contains("unexpected EOF"), "{error}");
    assert_eq!(client.take_pending_events(), vec![prior_event]);
    assert!(!client.is_healthy());
    assert!(
        client
            .take_connection_events()
            .iter()
            .any(|event| { matches!(event, MpvIpcConnectionEvent::Disconnected { .. }) })
    );
}

#[test]
fn framed_worker_rejects_truncated_json_without_echoing_credential_bytes() {
    let secret = "framed-partial-credential-canary";
    let wire =
        format!(r#"{{"request_id":1,"error":"success","X-Plex-Token":"{secret}""#).into_bytes();
    let (transport, _writes) = FramedWireTransport::new(wire, 2);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let error = client
        .get_property("path")
        .expect_err("a truncated final frame must fail JSON decoding");
    let connection_events = client.take_connection_events();

    assert!(error.contains("invalid mpv IPC JSON line"), "{error}");
    assert!(!error.contains(secret), "{error}");
    assert!(!format!("{connection_events:?}").contains(secret));
    assert!(!client.is_healthy());
}

#[test]
fn framed_worker_rejects_duplicate_stale_response_on_the_next_command() {
    let response_one = json!({
        "request_id": 1,
        "error": "success",
        "data": false,
    });
    let response_two = json!({
        "request_id": 2,
        "error": "success",
        "data": "movie.mkv",
    });
    let wire = format!("{response_one}\n{response_one}\n{response_two}\n").into_bytes();
    let (transport, _writes) = FramedWireTransport::new(wire, usize::MAX);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client
            .get_property("pause")
            .expect("the first response should succeed"),
        Some(json!(false))
    );
    let error = client
        .get_property("path")
        .expect_err("a duplicate response must not satisfy a later command");
    assert!(error.contains("request_id mismatch"), "{error}");
    assert!(!client.is_healthy());
}

#[test]
fn framed_worker_rejects_a_future_response_before_the_matching_response() {
    let wire = concat!(
        "{\"request_id\":2,\"error\":\"success\",\"data\":\"future\"}\n",
        "{\"request_id\":1,\"error\":\"success\",\"data\":\"current\"}\n",
    )
    .as_bytes()
    .to_vec();
    let (transport, _writes) = FramedWireTransport::new(wire, 3);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    let error = client
        .get_property("path")
        .expect_err("a reordered future response must fail closed");
    assert!(error.contains("expected 1, received 2"), "{error}");
    assert!(!client.is_healthy());
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

#[test]
fn mpv_ipc_nonblocking_command_never_waits_for_transport_timeout() {
    let command_timeout = Duration::from_millis(100);
    let mut client = MpvJsonIpcClient::new_with_command_timeout(
        Box::new(NeverRespondingTransport),
        command_timeout,
    );

    let queued_at = Instant::now();
    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "path"]), 1),
        Ok(Some(1))
    );
    assert!(
        queued_at.elapsed() < command_timeout / 2,
        "queueing nonblocking maintenance must not wait for the IPC response timeout"
    );

    let observation_deadline = Instant::now() + Duration::from_secs(1);
    let mut events = Vec::new();
    while Instant::now() < observation_deadline
        && !events.iter().any(|event| {
            matches!(
                event,
                MpvIpcConnectionEvent::Disconnected { .. } | MpvIpcConnectionEvent::TimedOut { .. }
            )
        })
    {
        events.extend(client.take_connection_events());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(events.iter().any(|event| matches!(
        event,
        MpvIpcConnectionEvent::TimedOut { timeout, .. } if *timeout == command_timeout
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
    );
}

#[test]
fn mpv_ipc_nonblocking_command_harvests_selected_events_without_dropping_others() {
    let property_event = json!({
        "event": "property-change",
        "name": "path",
        "data": "network.mkv",
    });
    let lease_event = json!({
        "event": "client-message",
        "args": [SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT, "{}"],
    });
    let response = json!({"request_id": 1, "error": "success", "data": false});
    let reads = [
        property_event.to_string(),
        lease_event.to_string(),
        response.to_string(),
    ];
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 1),
        Ok(Some(1))
    );
    let observation_deadline = Instant::now() + Duration::from_secs(1);
    let selected = loop {
        let events = client.take_pending_events_matching(|event| {
            event.get("event").and_then(Value::as_str) == Some("client-message")
        });
        if !events.is_empty() || Instant::now() >= observation_deadline {
            break events;
        }
        std::thread::yield_now();
    };

    assert_eq!(selected, vec![lease_event]);
    assert_eq!(client.take_pending_events(), vec![property_event]);
}

#[test]
fn mpv_ipc_events_preserve_ingress_time_across_ordinary_and_control_lanes() {
    let (transport, _state) = fake_transport_with_reads(&[]);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    let ordinary_received_at = Instant::now() - Duration::from_secs(4);
    let control_received_at = ordinary_received_at + Duration::from_secs(1);
    client.inject_test_event_received_at(
        json!({
            "event": "property-change",
            "name": "time-pos",
            "data": 10.0,
        }),
        ordinary_received_at,
    );
    client.inject_test_event_received_at(
        json!({
            "event": "client-message",
            "args": [SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT, "{}"],
        }),
        control_received_at,
    );

    let control = client.take_nonblocking_runtime_items_matching(|event| {
        event.get("event").and_then(Value::as_str) == Some("client-message")
    });
    assert!(matches!(
        control.as_slice(),
        [crate::ipc::MpvIpcNonblockingRuntimeItem::Event(event)]
            if event.received_at == control_received_at
    ));
    let ordinary = client.take_pending_timed_events();
    assert_eq!(ordinary.len(), 1);
    assert_eq!(ordinary[0].received_at, ordinary_received_at);
    assert_eq!(
        ordinary[0].value.get("name").and_then(Value::as_str),
        Some("time-pos")
    );
}

#[test]
fn mpv_ipc_nonblocking_runtime_items_bypass_an_earlier_unselected_event() {
    let property_event = json!({
        "event": "property-change",
        "name": "path",
        "data": "network.mkv",
    });
    let lease_event = json!({
        "event": "client-message",
        "args": [SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT, "{}"],
    });
    let response = json!({"request_id": 1, "error": "success", "data": false});
    let reads = [
        property_event.to_string(),
        lease_event.to_string(),
        response.to_string(),
    ];
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 7),
        Ok(Some(1))
    );

    let observation_deadline = Instant::now() + Duration::from_secs(1);
    let mut runtime_items = Vec::new();
    while runtime_items.len() < 2 && Instant::now() < observation_deadline {
        runtime_items.extend(client.take_nonblocking_runtime_items_matching(|event| {
            event.get("event").and_then(Value::as_str) == Some("client-message")
        }));
        std::thread::yield_now();
    }

    assert_eq!(runtime_items.len(), 2);
    assert!(matches!(
        &runtime_items[0],
        crate::ipc::MpvIpcNonblockingRuntimeItem::Event(event) if event.value == lease_event
    ));
    assert!(matches!(
        &runtime_items[1],
        crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                command_id: 1,
                token: 7,
            }
        )
    ));
    assert_eq!(
        client.take_pending_events(),
        vec![property_event],
        "the bypassed ordinary event must remain available to the full pump"
    );
}

#[test]
fn each_nonblocking_command_receives_a_unique_completion_identity() {
    let responses = [
        json!({"request_id": 1, "error": "success"}).to_string(),
        json!({"request_id": 2, "error": "success"}).to_string(),
    ];
    let response_refs = responses.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&response_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 7),
        Ok(Some(1))
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let first = client.take_nonblocking_runtime_items_matching(|_| true);
    assert!(matches!(
        first.as_slice(),
        [crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                command_id: 1,
                token: 7,
            }
        )]
    ));
    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 7),
        Ok(Some(2))
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let second = client.take_nonblocking_runtime_items_matching(|_| true);
    assert!(matches!(
        second.as_slice(),
        [crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                command_id: 2,
                token: 7,
            }
        )]
    ));
}

#[test]
fn nonblocking_property_read_retains_its_response_for_scoped_consumers() {
    let response = json!({
        "request_id": 1,
        "error": "success",
        "data": false,
    });
    let reads = [response.to_string()];
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client.try_get_property_nonblocking("paused-for-cache", 5),
        Ok(Some(1))
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let completions = client.take_nonblocking_runtime_items_matching(|_| true);
    assert!(matches!(
        completions.as_slice(),
        [crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::SucceededWithResponse {
                command_id: 1,
                token: 5,
                response: observed,
            }
        )] if observed == &response
    ));
}

#[test]
fn unavailable_nonblocking_property_read_emits_no_connection_failure() {
    let response = json!({
        "request_id": 1,
        "error": crate::constants::MPV_RESPONSE_PROPERTY_UNAVAILABLE,
    });
    let reads = [response.to_string()];
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    assert!(matches!(
        client.take_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { .. }]
    ));

    assert_eq!(
        client.try_get_property_nonblocking("paused-for-cache", 5),
        Ok(Some(1))
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let completions = client.take_nonblocking_runtime_items_matching(|_| true);
    assert!(matches!(
        completions.as_slice(),
        [crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                command_id: 1,
                token: 5,
                ..
            }
        )]
    ));
    assert!(client.take_connection_events().is_empty());
    assert!(client.is_healthy());
}

#[test]
fn unrelated_client_message_stays_on_the_ordinary_full_pump_lane() {
    let unrelated = json!({
        "event": "client-message",
        "args": ["third-party-script-message", "payload"],
    });
    let response = json!({"request_id": 1, "error": "success"});
    let reads = [unrelated.to_string(), response.to_string()];
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 9),
        Ok(Some(1))
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while client.test_runtime_queue_sizes().0 == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let (ordinary_count, control_count) = client.test_runtime_queue_sizes();
    assert_eq!(ordinary_count, 1);
    assert!(
        control_count <= 1,
        "only the command completion may use the control lane"
    );

    let runtime_items = client.take_nonblocking_runtime_items_matching(|_| false);
    assert!(
        runtime_items
            .iter()
            .all(|item| !matches!(item, crate::ipc::MpvIpcNonblockingRuntimeItem::Event(_)))
    );
    assert_eq!(client.take_pending_events(), vec![unrelated]);
}

#[test]
fn ipc_runtime_queues_are_bounded_and_preserve_structural_ordinary_events() {
    let (ordinary_capacity, control_capacity) = MpvJsonIpcClient::test_runtime_queue_capacities();
    let start_file = json!({"event": "start-file", "playlist_entry_id": 41});
    let path = json!({
        "event": "property-change",
        "name": "path",
        "data": "https://media.example.test/live.m3u8",
    });
    let mut reads = vec![start_file.to_string(), path.to_string()];
    for tick in 0..(ordinary_capacity + 32) {
        reads.push(
            json!({"event": "property-change", "name": "time-pos", "data": tick}).to_string(),
        );
    }
    for nonce in 0..(control_capacity + 32) {
        reads.push(
            json!({
                "event": "client-message",
                "args": [SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT, nonce.to_string()],
            })
            .to_string(),
        );
    }
    reads.push(json!({"request_id": 1, "error": "success"}).to_string());
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));

    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 11),
        Ok(Some(1))
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!client.test_nonblocking_command_is_pending());
    let (ordinary_count, control_count) = client.test_runtime_queue_sizes();
    assert_eq!(ordinary_count, ordinary_capacity);
    assert_eq!(control_count, control_capacity);

    let control_items = client.take_nonblocking_runtime_items_matching(|_| true);
    assert!(control_items.iter().any(|item| matches!(
        item,
        crate::ipc::MpvIpcNonblockingRuntimeItem::ControlQueueOverflow
    )));
    let ordinary_events = client.take_pending_events();
    assert!(ordinary_events.contains(&start_file));
    assert!(ordinary_events.contains(&path));
    assert!(ordinary_events.len() <= ordinary_capacity);
}

#[test]
fn noisy_incoming_event_cannot_evict_a_full_structural_event_window() {
    let (ordinary_capacity, _) = MpvJsonIpcClient::test_runtime_queue_capacities();
    let mut reads = Vec::with_capacity(ordinary_capacity + 2);
    for playlist_entry_id in 0..ordinary_capacity {
        reads.push(
            json!({"event": "start-file", "playlist_entry_id": playlist_entry_id}).to_string(),
        );
    }
    reads.push(json!({"event": "property-change", "name": "time-pos", "data": 99}).to_string());
    reads.push(json!({"request_id": 1, "error": "success"}).to_string());
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 12),
        Ok(Some(1))
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut control_items = Vec::new();
    while Instant::now() < deadline {
        control_items.extend(client.take_nonblocking_runtime_items_matching(|_| true));
        if control_items.iter().any(|item| {
            matches!(
                item,
                crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(_)
            )
        }) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(control_items.iter().all(|item| !matches!(
        item,
        crate::ipc::MpvIpcNonblockingRuntimeItem::OrdinaryQueueOverflow
    )));
    let ordinary_events = client.take_pending_events();
    assert_eq!(ordinary_events.len(), ordinary_capacity);
    assert!(
        ordinary_events
            .iter()
            .all(|event| { event.get("event").and_then(Value::as_str) == Some("start-file") })
    );
}

#[test]
fn queue_pressure_preserves_transient_seek_and_playback_restart_lifecycle_edges() {
    let (ordinary_capacity, _) = MpvJsonIpcClient::test_runtime_queue_capacities();
    let seek = json!({"event": "seek"});
    let playback_restart = json!({"event": "playback-restart"});
    let mut reads = vec![seek.to_string(), playback_restart.to_string()];
    for tick in 0..ordinary_capacity {
        reads.push(
            json!({"event": "property-change", "name": "time-pos", "data": tick}).to_string(),
        );
    }
    reads.push(json!({"request_id": 1, "error": "success"}).to_string());
    let read_refs = reads.iter().map(String::as_str).collect::<Vec<_>>();
    let (transport, _state) = fake_transport_with_reads(&read_refs);
    let mut client = MpvJsonIpcClient::new(Box::new(transport));
    assert_eq!(
        client.try_send_command_expect_success_nonblocking(json!(["get_property", "pause"]), 13),
        Ok(Some(1))
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while client.test_nonblocking_command_is_pending() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!client.test_nonblocking_command_is_pending());

    let ordinary_events = client.take_pending_events();
    assert!(
        ordinary_events.contains(&seek),
        "seek is a one-shot lifecycle edge required to track active seek ownership"
    );
    assert!(
        ordinary_events.contains(&playback_restart),
        "playback-restart is a one-shot lifecycle edge required to complete StartAfterLoad and StartAfterSeek commands"
    );
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
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "30"), ("cache-pause-wait", "5")]);

    adapter
        .open_file("https://media.example/video.m3u8")
        .expect("attached mpv transport should accept network loadfile");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
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
            "request_id": 1
        })
    );
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

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("an attached network file should accept file-local options"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
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

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("an attached local file should be left unchanged"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );
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

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("an idle attached player should need no option changes"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia
    );

    assert!(adapter.is_connected());
    assert_eq!(state.writes().len(), 1, "only the path should be queried");
}

#[test]
fn active_network_option_reapply_treats_property_unavailable_as_healthy_idle_player() {
    let (transport, state) =
        fake_transport_with_reads(&[r#"{"request_id":1,"error":"property unavailable"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("mpv's canonical unavailable-path response should mean no active file"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia
    );

    assert!(adapter.is_connected());
    assert_eq!(state.writes().len(), 1, "only the path should be queried");
    assert!(matches!(
        adapter.take_ipc_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { .. }]
    ));
}

#[cfg(feature = "test-support")]
#[test]
fn delayed_active_network_media_fixture_reports_idle_then_applies_network_options() {
    let (mut adapter, commands) =
        MpvAdapter::with_delayed_active_network_media_test_ipc(LegacySyncplayUiSettings::default());
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("the first authoritative path query should be healthy and idle"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia
    );
    assert!(
        commands
            .lock()
            .expect("active-network command log should not be poisoned")
            .is_empty(),
        "no file-local option may be written before network media becomes active"
    );

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("the later network path should accept every configured option"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    assert_eq!(
        *commands
            .lock()
            .expect("active-network command log should not be poisoned"),
        vec![
            json!(["set_property", "file-local-options/cache-pause-wait", "5"]),
            json!(["set_property", "file-local-options/cache-secs", "75"]),
        ]
    );
}

#[test]
fn external_local_to_network_transition_applies_options_once_per_generation_and_path() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":42}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/main-stream.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/main-stream.m3u8"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":43}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/next-local.mkv"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":44}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/main-stream.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local media query should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );
    adapter
        .set_playback_rate(1.0)
        .expect("the first external network transition should remain healthy");
    adapter
        .set_playback_rate(1.0)
        .expect("a duplicate path observation should remain healthy");
    adapter
        .set_playback_rate(1.0)
        .expect("a later local file must remain untouched");
    adapter
        .set_playback_rate(1.0)
        .expect("the same URL in a new media generation must reapply options");

    let file_local_commands = state
        .writes()
        .iter()
        .map(|write| serde_json::from_str::<Value>(write.trim_end()).expect("valid command json"))
        .filter(|command| {
            command
                .pointer("/command/1")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with("file-local-options/"))
        })
        .map(|command| command["command"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        file_local_commands,
        vec![
            json!(["set_property", "file-local-options/cache-pause-wait", "5"]),
            json!(["set_property", "file-local-options/cache-secs", "75"]),
            json!(["set_property", "file-local-options/cache-pause-wait", "5"]),
            json!(["set_property", "file-local-options/cache-secs", "75"]),
        ],
        "the duplicate path and intervening local generation must not receive option writes"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn explicit_active_network_apply_suppresses_initial_path_echo_but_not_same_url_reload() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"https://media.example.test/live.m3u8"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/live.m3u8"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":91}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/live.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial active network file should accept its options"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    adapter
        .set_playback_rate(1.0)
        .expect("the initial observer echo should not duplicate options");
    adapter
        .set_playback_rate(1.0)
        .expect("a genuine same-URL reload should accept options");

    let file_local_writes = state
        .writes()
        .iter()
        .filter(|write| write.contains("file-local-options/cache-secs"))
        .count();
    assert_eq!(
        file_local_writes, 2,
        "one explicit apply and one new-generation apply are expected"
    );
}

#[test]
fn local_transition_during_first_option_write_supersedes_remaining_network_writes() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":81}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/a.m3u8"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":82}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/replaced-local.mkv"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":3,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local path should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );

    adapter
        .set_playback_rate(1.0)
        .expect("the triggering command and superseding local transition should remain healthy");

    let file_local_writes = state
        .writes()
        .iter()
        .filter(|write| write.contains("file-local-options/"))
        .count();
    assert_eq!(
        file_local_writes, 1,
        "the stale network attempt must stop before writing its remaining option to local media"
    );
    assert_eq!(adapter.current_path(), Some("C:/media/replaced-local.mkv"));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn core_hook_keeps_network_option_writes_inside_mpv_and_classifies_a_to_b_as_superseded() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookSupersessionTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("the hook should accept the complete map for superseding network B"),
        MpvActiveNetworkMediaOptionsApplyOutcome::Superseded
    );
    assert!(adapter.is_connected());
    assert!(
        writes
            .lock()
            .expect("network-options hook writes should not be poisoned")
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "Rust JSON IPC must never issue per-option writes once the core hook is active"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "network B's ordered result should own the final transition state"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "the explicit result and on-load result should converge without duplicate recovery"
    );
}

fn run_core_hook_supersession_scenario(
    old_network_succeeds: bool,
    target: HookSupersessionTarget,
) -> (
    MpvActiveNetworkMediaOptionsApplyOutcome,
    Option<MpvNetworkMediaOptionsTransitionOutcome>,
    Arc<Mutex<Vec<String>>>,
) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        old_network_succeeds,
        target,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: true,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    let apply = adapter
        .apply_network_media_options_to_active_media_classified()
        .expect("the explicit A apply should defer to the authoritative successor");
    let outcome = adapter.take_network_media_options_transition_outcome();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.is_connected());
    assert!(
        writes
            .lock()
            .expect("scenario writes should not be poisoned")
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "production hook scenarios must never use direct file-local writes"
    );
    (apply, outcome, writes)
}

#[cfg(feature = "test-support")]
fn superseded_network_options_adapter_awaiting_hook_result() -> MpvAdapter {
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::new(Mutex::new(Vec::new())),
        responses: VecDeque::new(),
        old_network_succeeds: false,
        target: HookSupersessionTarget::NetworkAwaitingResult,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: true,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("network B should supersede the explicit apply while awaiting its hook result"),
        MpvActiveNetworkMediaOptionsApplyOutcome::Superseded
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    adapter
}

fn configured_v3_hook_reducer_adapter() -> MpvAdapter {
    let mut adapter =
        MpvAdapter::with_network_options_hook_test_transport(NeverRespondingTransport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter.prepare_test_network_options_hook_v3_reducer();
    adapter
}

fn defer_v3_transition(
    adapter: &mut MpvAdapter,
    load_sequence: u64,
    source_path: &str,
    stream_open_filename: &str,
    status: &str,
    error: Option<&str>,
) {
    adapter.defer_test_network_options_hook_v3_transition(
        load_sequence,
        source_path,
        stream_open_filename,
        status,
        error,
    );
}

fn apply_deferred_v3_transition(adapter: &mut MpvAdapter) {
    adapter.flush_test_network_options_hook_v3_transition();
}

#[test]
fn verified_network_policy_reports_partial_rejection_and_privacy_safe_readback() {
    let secret = "advanced-format-token-canary";
    let source = "https://media.example.test/watch?access_token=must-not-be-retained";
    let mut adapter =
        MpvAdapter::with_network_options_hook_test_transport(NeverRespondingTransport);
    adapter.configure_network_media_options([
        ("cache-pause-wait", "5"),
        ("cache-secs", "75"),
        ("ytdl-format", secret),
    ]);
    adapter.prepare_test_network_options_hook_v3_reducer();
    adapter.handle_test_network_options_start_file(501);
    let generation = adapter
        .media_generation()
        .expect("start-file should establish a diagnostic generation");
    adapter.defer_test_network_options_hook_verified_transition(
        1,
        source,
        "partially-applied",
        &[
            ("cache-pause-wait", "applied"),
            ("cache-secs", "rejected"),
            ("ytdl-format", "applied"),
        ],
        &[
            ("cache-pause-wait", "5.0"),
            ("cache-secs", "30"),
            ("ytdl-format", secret),
        ],
    );
    apply_deferred_v3_transition(&mut adapter);

    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::Failed(error))
            if error.to_string().contains("partially applied")
    ));
    let snapshot = adapter.network_media_diagnostic_snapshot();
    assert_eq!(snapshot.media_generation, Some(generation));
    assert_eq!(snapshot.load_sequence, Some(1));
    assert_eq!(
        snapshot.application_state,
        Some(MpvNetworkMediaPolicyApplicationState::PartiallyApplied)
    );
    assert!(snapshot.verification_complete);
    assert_eq!(
        snapshot.option_results,
        vec![
            MpvNetworkOptionApplyResult {
                name: "cache-pause-wait".to_owned(),
                status: MpvNetworkOptionApplyStatus::Applied,
            },
            MpvNetworkOptionApplyResult {
                name: "cache-secs".to_owned(),
                status: MpvNetworkOptionApplyStatus::Rejected,
            },
            MpvNetworkOptionApplyResult {
                name: "ytdl-format".to_owned(),
                status: MpvNetworkOptionApplyStatus::Applied,
            },
        ]
    );
    assert_eq!(
        snapshot.desired_cache_options,
        BTreeMap::from([
            ("cache-pause-wait".to_owned(), "5".to_owned()),
            ("cache-secs".to_owned(), "75".to_owned()),
        ])
    );
    assert_eq!(
        snapshot.effective_cache_options,
        BTreeMap::from([
            ("cache-pause-wait".to_owned(), "5".to_owned()),
            ("cache-secs".to_owned(), "30".to_owned()),
        ])
    );
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains(secret));
    assert!(!debug.contains("must-not-be-retained"));
}

#[test]
fn verified_network_policy_treats_critical_readback_mismatch_as_partial() {
    let mut adapter =
        MpvAdapter::with_network_options_hook_test_transport(NeverRespondingTransport);
    adapter.configure_network_media_options([("cache-pause-wait", "5"), ("cache-secs", "75")]);
    adapter.prepare_test_network_options_hook_v3_reducer();
    adapter.handle_test_network_options_start_file(502);
    adapter.defer_test_network_options_hook_verified_transition(
        1,
        "https://media.example.test/mismatch.m3u8",
        "network-updated",
        &[("cache-pause-wait", "applied"), ("cache-secs", "applied")],
        &[("cache-pause-wait", "5"), ("cache-secs", "30")],
    );
    apply_deferred_v3_transition(&mut adapter);

    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::Failed(_))
    ));
    let snapshot = adapter.network_media_diagnostic_snapshot();
    assert_eq!(
        snapshot.application_state,
        Some(MpvNetworkMediaPolicyApplicationState::PartiallyApplied)
    );
    assert_eq!(
        snapshot.option_results,
        vec![
            MpvNetworkOptionApplyResult {
                name: "cache-pause-wait".to_owned(),
                status: MpvNetworkOptionApplyStatus::Applied,
            },
            MpvNetworkOptionApplyResult {
                name: "cache-secs".to_owned(),
                status: MpvNetworkOptionApplyStatus::Mismatched,
            },
        ]
    );
}

#[test]
fn new_media_generation_clears_diagnostics_and_ignores_stale_hook_result() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.handle_test_network_options_start_file(503);
    adapter.defer_test_network_options_hook_verified_transition(
        1,
        "https://media.example.test/first.m3u8",
        "network-updated",
        &[("cache-secs", "applied")],
        &[("cache-secs", "75")],
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter
            .network_media_diagnostic_snapshot()
            .application_state,
        Some(MpvNetworkMediaPolicyApplicationState::Applied)
    );
    let first_generation = adapter.media_generation();

    adapter.handle_test_network_options_start_file(504);
    let second_generation = adapter.media_generation();
    assert_ne!(second_generation, first_generation);
    let reset = adapter.network_media_diagnostic_snapshot();
    assert_eq!(reset.media_generation, second_generation);
    assert_eq!(reset.application_state, None);
    assert!(reset.option_results.is_empty());
    assert!(reset.effective_cache_options.is_empty());

    adapter.defer_test_network_options_hook_verified_transition(
        1,
        "https://media.example.test/stale-first.m3u8",
        "network-updated",
        &[("cache-secs", "applied")],
        &[("cache-secs", "75")],
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter
            .network_media_diagnostic_snapshot()
            .application_state,
        None,
        "a completed older hook load must not repopulate the next generation"
    );
}

#[test]
fn unaccepted_delayed_load_result_cannot_rebind_to_the_next_media_generation() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.handle_test_network_options_start_file(505);
    let first_generation = adapter.media_generation();

    adapter.handle_test_network_options_start_file(506);
    let second_generation = adapter.media_generation();
    assert_ne!(second_generation, first_generation);

    adapter.defer_test_network_options_hook_verified_transition(
        1,
        "https://media.example.test/delayed-first.m3u8",
        "network-updated",
        &[("cache-secs", "applied")],
        &[("cache-secs", "75")],
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter
            .network_media_diagnostic_snapshot()
            .application_state,
        None,
        "an unaccepted result below the current generation's expected load sequence is stale"
    );

    adapter.defer_test_network_options_hook_verified_transition(
        2,
        "https://media.example.test/current-second.m3u8",
        "network-updated",
        &[("cache-secs", "applied")],
        &[("cache-secs", "75")],
    );
    apply_deferred_v3_transition(&mut adapter);
    let snapshot = adapter.network_media_diagnostic_snapshot();
    assert_eq!(snapshot.media_generation, second_generation);
    assert_eq!(snapshot.load_sequence, Some(2));
    assert_eq!(
        snapshot.application_state,
        Some(MpvNetworkMediaPolicyApplicationState::Applied)
    );
}

#[test]
fn diagnostic_snapshot_drops_invalid_values_even_under_allowlisted_cache_keys() {
    let secret = "https://private.example/cache?access_token=allowlist-canary";
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.configure_network_media_options([
        ("cache", secret),
        ("cache-secs", secret),
        ("demuxer-max-bytes", secret),
        ("ytdl-format", secret),
    ]);
    adapter.prepare_test_network_options_hook_v3_reducer();
    adapter.handle_test_network_options_start_file(507);
    adapter.defer_test_network_options_hook_verified_transition(
        1,
        "https://media.example.test/privacy.m3u8",
        "network-updated",
        &[
            ("cache", "applied"),
            ("cache-secs", "applied"),
            ("demuxer-max-bytes", "applied"),
            ("ytdl-format", "applied"),
        ],
        &[
            ("cache", secret),
            ("cache-secs", secret),
            ("demuxer-max-bytes", secret),
            ("ytdl-format", secret),
        ],
    );
    apply_deferred_v3_transition(&mut adapter);

    let snapshot = adapter.network_media_diagnostic_snapshot();
    assert!(snapshot.desired_cache_options.is_empty());
    assert!(snapshot.effective_cache_options.is_empty());
    assert!(!format!("{snapshot:?}").contains(secret));
}

fn assert_sanitized_hook_failure(
    outcome: &MpvNetworkMediaOptionsTransitionOutcome,
    load_sequence: u64,
    source_kind: &str,
    resolved_target_kind: &str,
    forbidden: &[&str],
) {
    let MpvNetworkMediaOptionsTransitionOutcome::Failed(error) = outcome else {
        panic!("expected a failed network-options transition, got {outcome:?}");
    };
    let display = error.to_string();
    let debug = format!("{outcome:?}");
    let expected_load = format!("hook load {load_sequence}");
    assert!(
        display.contains(&expected_load),
        "sanitized failure should preserve its load sequence: {display}"
    );
    assert!(
        display.contains(&format!("source: {source_kind}")),
        "sanitized failure should preserve only the source kind: {display}"
    );
    assert!(
        display.contains(&format!("resolved target: {resolved_target_kind}")),
        "sanitized failure should preserve only the resolved-target kind: {display}"
    );
    for secret in forbidden {
        assert!(
            !display.contains(secret),
            "Display leaked credential-bearing hook data {secret:?}: {display}"
        );
        assert!(
            !debug.contains(secret),
            "Debug leaked credential-bearing hook data {secret:?}: {debug}"
        );
    }
}

#[test]
fn rewritten_stream_result_is_accepted_and_rejection_remains_visible_and_retryable() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    let source = "https://service.example/watch/123";

    defer_v3_transition(
        &mut adapter,
        1,
        source,
        "edl://resolved-stream-a",
        "network-updated",
        None,
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert_eq!(
        adapter.test_network_options_policy_source_path(),
        Some(source),
        "logical source identity must not be replaced by the rewritten stream target"
    );

    defer_v3_transition(
        &mut adapter,
        2,
        source,
        "edl://resolved-stream-b",
        "failed",
        Some("mpv rejected cache-secs"),
    );
    apply_deferred_v3_transition(&mut adapter);
    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("rewritten target rejection should remain visible");
    assert_sanitized_hook_failure(
        &outcome,
        2,
        "HTTPS",
        "EDL",
        &[source, "edl://resolved-stream-b", "mpv rejected cache-secs"],
    );

    defer_v3_transition(
        &mut adapter,
        3,
        source,
        "edl://resolved-stream-c",
        "network-updated",
        None,
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "a later load sequence must recover a retryable rejection"
    );
}

#[test]
fn hook_failures_never_expose_raw_sources_resolved_targets_or_lua_errors() {
    let cases = [
        (
            "https://CANARY_USER:CANARY_PASS@media.example.test/live.m3u8",
            "https://CANARY_USER:CANARY_PASS@media.example.test/live.m3u8",
            "CANARY_LUA_ERROR_USERINFO",
            "HTTPS",
            "HTTPS",
            &["CANARY_USER", "CANARY_PASS"][..],
        ),
        (
            "https://media.example.test/live.m3u8?sig=CANARY_SIG&auth=CANARY_AUTH&X-Amz-Credential=CANARY_AWS",
            "https://cdn.example.test/live.m3u8?sig=CANARY_RESOLVED_SIG",
            "CANARY_LUA_ERROR_QUERY",
            "HTTPS",
            "HTTPS",
            &[
                "CANARY_SIG",
                "CANARY_AUTH",
                "CANARY_AWS",
                "CANARY_RESOLVED_SIG",
            ][..],
        ),
        (
            "https://service.example.test/watch/CANARY_EDL_SOURCE",
            "edl://https://cdn.example.test/a.ts?sig=CANARY_EDL_SIG;https://cdn.example.test/b.ts?auth=CANARY_EDL_AUTH",
            "CANARY_LUA_ERROR_EDL",
            "HTTPS",
            "EDL",
            &["CANARY_EDL_SOURCE", "CANARY_EDL_SIG", "CANARY_EDL_AUTH"][..],
        ),
        (
            "C:/Users/private/CANARY_LOCAL_SOURCE.mkv",
            "https://cdn.example.test/rewritten.m3u8?auth=CANARY_REWRITE_AUTH",
            "CANARY_LUA_ERROR_REWRITE",
            "local path",
            "HTTPS",
            &["CANARY_LOCAL_SOURCE", "CANARY_REWRITE_AUTH"][..],
        ),
    ];

    for (source, resolved_target, lua_error, source_kind, target_kind, canaries) in cases {
        let mut adapter = configured_v3_hook_reducer_adapter();
        defer_v3_transition(
            &mut adapter,
            41,
            source,
            resolved_target,
            "failed",
            Some(lua_error),
        );
        apply_deferred_v3_transition(&mut adapter);
        let outcome = adapter
            .take_network_media_options_transition_outcome()
            .expect("each hook failure should remain observable");
        let mut forbidden = vec![source, resolved_target, lua_error];
        forbidden.extend_from_slice(canaries);
        assert_sanitized_hook_failure(&outcome, 41, source_kind, target_kind, &forbidden);
        assert_eq!(
            adapter.take_network_media_options_transition_outcome(),
            None,
            "one raw hook failure must produce exactly one sanitized outcome"
        );
    }
}

#[test]
fn same_url_higher_sequence_is_final_for_success_failure_and_delayed_results() {
    let source = "https://service.example/watch/123";
    let stream = "edl://resolved-stream";
    let mut adapter = configured_v3_hook_reducer_adapter();

    defer_v3_transition(&mut adapter, 1, source, stream, "network-updated", None);
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_path(source);
    defer_v3_transition(&mut adapter, 1, source, stream, "network-updated", None);
    defer_v3_transition(
        &mut adapter,
        2,
        source,
        stream,
        "failed",
        Some("same-URL B rejected cache-secs"),
    );
    adapter.end_test_network_options_event_batch();
    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("same-URL B failure should supersede delayed A success");
    assert_sanitized_hook_failure(
        &outcome,
        2,
        "HTTPS",
        "EDL",
        &[source, stream, "same-URL B rejected cache-secs"],
    );
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(2)
    );

    let mut adapter = configured_v3_hook_reducer_adapter();
    defer_v3_transition(
        &mut adapter,
        1,
        source,
        stream,
        "failed",
        Some("same-URL A rejected cache-secs"),
    );
    apply_deferred_v3_transition(&mut adapter);
    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::Failed(_))
    ));

    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    defer_v3_transition(&mut adapter, 2, source, stream, "network-updated", None);
    defer_v3_transition(
        &mut adapter,
        1,
        source,
        stream,
        "failed",
        Some("delayed A rejection"),
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "same-URL B success must supersede delayed A failure"
    );
    assert!(
        !adapter.test_network_options_awaiting_authoritative_transition(),
        "B success must clear the stale explicit-apply requirement"
    );
}

#[test]
fn higher_sequence_result_before_same_url_path_observation_completes_once() {
    let source = "https://service.example/watch/123";
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.begin_test_network_options_event_batch();
    defer_v3_transition(
        &mut adapter,
        2,
        source,
        "edl://resolved-stream-b",
        "network-updated",
        None,
    );
    adapter.observe_test_network_options_path(source);
    adapter.end_test_network_options_event_batch();

    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(!adapter.test_network_options_awaiting_authoritative_transition());
}

#[test]
fn production_path_observations_wait_for_authoritative_hook_completion() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_pending_start();
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_path("C:/media/local-b.mkv");
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());

    defer_v3_transition(
        &mut adapter,
        1,
        "C:/media/local-b.mkv",
        "C:/media/local-b.mkv",
        "local",
        None,
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(!adapter.test_network_options_awaiting_authoritative_transition());
}

#[test]
fn rewritten_network_failure_is_not_masked_by_local_logical_path() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_path("C:/logical/source.mkv");
    defer_v3_transition(
        &mut adapter,
        1,
        "C:/logical/source.mkv",
        "https://cdn.example.test/rewritten.m3u8",
        "failed",
        Some("rewritten stream rejected cache-secs"),
    );
    adapter.end_test_network_options_event_batch();

    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("the sequenced rewritten-stream failure should be the only outcome");
    assert_sanitized_hook_failure(
        &outcome,
        1,
        "local path",
        "HTTPS",
        &[
            "C:/logical/source.mkv",
            "https://cdn.example.test/rewritten.m3u8",
            "rewritten stream rejected cache-secs",
        ],
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn only_terminal_end_file_can_complete_hook_policy_without_a_hook_result() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_null_path();
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_terminal_end();
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn successor_start_supersedes_terminal_idle_completion_in_the_same_batch() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_terminal_end();
    adapter.observe_test_network_options_pending_start();
    adapter.end_test_network_options_event_batch();

    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());
}

#[test]
fn sequenced_failure_precedes_terminal_idle_fallback_in_the_same_batch() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.begin_test_network_options_event_batch();
    defer_v3_transition(
        &mut adapter,
        1,
        "https://media.example.test/ended.m3u8",
        "https://media.example.test/ended.m3u8",
        "failed",
        Some("ended load rejected cache-secs"),
    );
    adapter.observe_test_network_options_terminal_end();
    adapter.end_test_network_options_event_batch();

    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("the hook's sequenced failure must outrank terminal-idle fallback");
    assert_sanitized_hook_failure(
        &outcome,
        1,
        "HTTPS",
        "HTTPS",
        &[
            "https://media.example.test/ended.m3u8",
            "ended load rejected cache-secs",
        ],
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn matching_real_end_file_completes_idle_but_successor_start_does_not() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.handle_test_network_options_start_file(701);
    adapter.observe_test_network_options_path("https://media.example.test/a.m3u8");
    adapter.handle_test_network_options_end_file(701);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );

    adapter.set_test_network_options_awaiting_authoritative_transition(true);
    adapter.begin_test_network_options_event_batch();
    adapter.handle_test_network_options_start_file(702);
    adapter.observe_test_network_options_path("https://media.example.test/b.m3u8");
    adapter.handle_test_network_options_end_file(702);
    adapter.handle_test_network_options_start_file(703);
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert!(adapter.test_network_options_awaiting_authoritative_transition());
}

#[test]
fn hook_instance_ack_fails_closed_on_sequence_regression_and_resets_for_new_instance() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    defer_v3_transition(
        &mut adapter,
        3,
        "https://media.example.test/a.m3u8",
        "https://media.example.test/a.m3u8",
        "network-updated",
        None,
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );

    adapter.invalidate_test_network_options_hook_delivery();
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(3),
        "delivery invalidation must preserve the canonical hook cursor"
    );
    adapter.configure_test_network_options_hook_instance("test-hook-instance", 5);
    adapter.defer_test_network_options_hook_v3_transition_for_instance(
        "test-hook-instance",
        4,
        "https://media.example.test/delayed.m3u8",
        "failed",
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(5)
    );
    adapter.defer_test_network_options_hook_v3_transition_for_instance(
        "test-hook-instance",
        6,
        "https://media.example.test/must-not-survive-regression.m3u8",
        "failed",
    );
    adapter.configure_test_network_options_hook_instance("test-hook-instance", 2);
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        adapter.take_network_media_options_transition_outcome()
    else {
        panic!("same-instance sequence regression must degrade hook delivery");
    };
    assert!(error.to_string().contains("regressed load sequence"));
    assert!(
        !adapter.test_network_media_options_hook_is_ready(),
        "a regressed same-instance acknowledgement must fail closed"
    );
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(5),
        "same-instance acknowledgement must never move the sequence floor backward"
    );

    adapter.configure_test_network_options_hook_instance("test-hook-instance", 5);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "the same instance may recover only after acknowledging the retained floor"
    );
    assert!(adapter.test_network_media_options_hook_is_ready());
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "sequence rollback must discard results deferred under the invalid delivery state"
    );

    adapter.configure_test_network_options_hook_instance("fresh-hook-instance", 0);
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(0),
        "a genuinely new hook instance owns a fresh sequence namespace"
    );
    adapter.defer_test_network_options_hook_v3_transition_for_instance(
        "test-hook-instance",
        6,
        "https://media.example.test/stale-instance.m3u8",
        "failed",
    );
    adapter.defer_test_network_options_hook_v3_transition_for_instance(
        "fresh-hook-instance",
        1,
        "https://media.example.test/fresh.m3u8",
        "network-updated",
    );
    apply_deferred_v3_transition(&mut adapter);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(1)
    );
}

#[test]
fn incomplete_hook_instance_ack_never_marks_delivery_ready() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.invalidate_test_network_options_hook_delivery();

    adapter.configure_test_network_options_hook_instance_fields(None, Some(0));
    assert!(!adapter.test_network_media_options_hook_is_ready());
    adapter.configure_test_network_options_hook_instance_fields(Some("test-hook-instance"), None);
    assert!(!adapter.test_network_media_options_hook_is_ready());

    adapter.configure_test_network_options_hook_instance("test-hook-instance", 0);
    assert!(adapter.test_network_media_options_hook_is_ready());
}

#[test]
fn failed_network_a_superseded_by_local_b_recovers_without_stale_failure() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(false, HookSupersessionTarget::Local);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    assert_eq!(
        outcome,
        Some(MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged)
    );
}

#[test]
fn failed_network_a_superseded_by_raw_null_waits_for_terminal_end_or_hook_result() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(false, HookSupersessionTarget::Idle);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    assert_eq!(outcome, None);
}

#[test]
fn failed_network_a_superseded_by_successful_network_b_applies_b() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(false, HookSupersessionTarget::NetworkSuccess);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    assert_eq!(
        outcome,
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
}

#[test]
fn failed_network_a_superseded_by_failed_network_b_reports_only_b_failure() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(false, HookSupersessionTarget::NetworkFailure);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    let outcome = outcome.expect("authoritative B failure should remain observable");
    assert_sanitized_hook_failure(
        &outcome,
        2,
        "HTTPS",
        "HTTPS",
        &[
            "https://media.example.test/b.m3u8",
            "B rejected cache-secs",
            "A rejected cache-secs",
        ],
    );
}

#[test]
fn successful_network_a_superseded_by_failed_network_b_reports_b_failure() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(true, HookSupersessionTarget::NetworkFailure);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    let outcome = outcome.expect("authoritative B failure should remain observable");
    assert_sanitized_hook_failure(
        &outcome,
        2,
        "HTTPS",
        "HTTPS",
        &["https://media.example.test/b.m3u8", "B rejected cache-secs"],
    );
}

#[test]
fn superseded_apply_emits_no_outcome_before_network_b_reports_its_result() {
    let (apply, outcome, _) =
        run_core_hook_supersession_scenario(false, HookSupersessionTarget::NetworkAwaitingResult);
    assert_eq!(apply, MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
    assert_eq!(outcome, None);
}

#[test]
fn hook_configuration_timeout_never_reenters_direct_writes_across_network_to_local() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookConfigurationTimeoutTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        transition_injected: false,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    let error = adapter
        .apply_network_media_options_to_active_media_classified()
        .expect_err("a missing configuration acknowledgement must remain retryable");
    assert!(error.to_string().contains("did not acknowledge generation"));
    assert_eq!(adapter.current_path(), Some("C:/media/local-b.mkv"));
    assert!(adapter.is_connected());
    assert!(
        writes
            .lock()
            .expect("hook timeout writes should not be poisoned")
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "an unavailable production hook must never re-enable direct per-option writes"
    );
}

#[test]
fn missing_canonical_hook_is_loaded_again_and_recovers_on_retry() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookReloadTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        configure_attempts: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .apply_network_media_options_to_active_media_classified()
        .expect_err("the missing canonical target should fail the first configure delivery");
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("retry should reload and configure the canonical hook"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );

    let writes = writes
        .lock()
        .expect("hook reload writes should not be poisoned");
    assert_eq!(
        writes
            .iter()
            .filter(|write| write.contains("\"load-script\""))
            .count(),
        2,
        "a rejected canonical target must clear the loaded assumption"
    );
    assert!(
        writes
            .iter()
            .all(|write| !write.contains("file-local-options/"))
    );
}

#[test]
fn successful_explicit_retry_rearms_independent_hook_degradation_reporting() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes,
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::StableNetwork,
        lose_ownership_on_heartbeat: true,
        acknowledge_heartbeats: false,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial explicit hook apply should succeed"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    adapter.force_test_network_media_options_hook_heartbeat_due();
    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert!(adapter.is_connected());

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("same-map retry should restore authoritative policy health"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    assert!(!adapter.test_network_options_awaiting_authoritative_transition());
    assert_eq!(
        adapter.test_network_options_last_accepted_load_sequence(),
        Some(1)
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "a successful explicit retry must positively recover hook health"
    );

    adapter.force_test_network_media_options_hook_heartbeat_due();
    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert!(adapter.is_connected());
}

#[test]
fn active_network_options_apply_reacquires_an_expired_hook_lease() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::StableNetwork,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: true,
        expire_ownership_on_second_active_apply: true,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial hook ownership should configure"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("an expired lease should be reacquired within the explicit apply"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    assert!(adapter.test_network_media_options_hook_is_ready());

    let configure_count = writes
        .lock()
        .expect("lease recovery writes should not be poisoned")
        .iter()
        .filter(|write| write.contains("sorotte_network_options_configure"))
        .count();
    assert_eq!(
        configure_count, 2,
        "the expired owner should perform exactly one fresh configuration"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn continuous_hook_degradation_is_emitted_only_once() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.inject_test_network_media_options_hook_degradation("first lease failure");
    adapter.inject_test_network_media_options_hook_degradation("duplicate lease failure");

    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[cfg(feature = "test-support")]
#[test]
fn queued_hook_recovery_survives_successful_explicit_policy_apply() {
    let (mut adapter, _) = MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
        LegacySyncplayUiSettings::default(),
        usize::MAX,
    );
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter.inject_test_network_media_options_hook_degradation("consumed hook lease loss");
    assert!(matches!(
        adapter.take_network_options_hook_health_transition(),
        Some(MpvNetworkOptionsHookHealthTransition::Degraded(_))
    ));

    adapter.inject_test_network_media_options_hook_recovery();
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("the explicit policy retry should succeed"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );

    assert_eq!(
        adapter.take_network_options_hook_health_transition(),
        Some(MpvNetworkOptionsHookHealthTransition::Recovered),
        "an explicit media-policy apply must not erase queued hook recovery"
    );
    let snapshot = adapter.network_options_runtime_health_snapshot();
    assert_eq!(snapshot.hook_health, MpvNetworkOptionsHookHealth::Ready);
    assert_eq!(
        snapshot.media_policy,
        MpvNetworkMediaPolicyState::NetworkMediaUpdated
    );
}

#[cfg(feature = "test-support")]
#[test]
fn queued_hook_degradation_survives_failed_explicit_policy_apply() {
    let (mut adapter, _) = MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
        LegacySyncplayUiSettings::default(),
        1,
    );
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter.inject_test_network_media_options_hook_degradation("unconsumed hook lease loss");

    adapter
        .apply_network_media_options_to_active_media_classified()
        .expect_err("the first explicit file-local option should be rejected");

    let Some(MpvNetworkOptionsHookHealthTransition::Degraded(error)) =
        adapter.take_network_options_hook_health_transition()
    else {
        panic!("the unconsumed hook degradation must remain observable");
    };
    assert!(error.to_string().contains("unconsumed hook lease loss"));
    let snapshot = adapter.network_options_runtime_health_snapshot();
    assert!(matches!(
        snapshot.hook_health,
        MpvNetworkOptionsHookHealth::Degraded(reason)
            if reason.contains("unconsumed hook lease loss")
    ));
    assert!(matches!(
        snapshot.media_policy,
        MpvNetworkMediaPolicyState::Failed(_)
    ));
}

#[cfg(feature = "test-support")]
#[test]
fn explicit_apply_preserves_pending_hook_recovery_and_authoritative_policy_failure() {
    let (mut adapter, _) = MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
        LegacySyncplayUiSettings::default(),
        usize::MAX,
    );
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter.inject_test_network_media_options_hook_degradation("prior hook lease loss");
    assert!(matches!(
        adapter.take_network_options_hook_health_transition(),
        Some(MpvNetworkOptionsHookHealthTransition::Degraded(_))
    ));
    adapter.inject_test_network_media_options_hook_recovery();
    adapter.inject_test_network_media_options_policy_failure(
        42,
        "https://media.example.test/source.m3u8",
        "https://cdn.example.test/resolved.m3u8",
    );

    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("the later explicit apply should succeed"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    assert_eq!(
        adapter.take_network_options_hook_health_transition(),
        Some(MpvNetworkOptionsHookHealthTransition::Recovered)
    );
    assert!(matches!(
        adapter.take_network_media_policy_outcome(),
        Some(MpvNetworkMediaPolicyOutcome::Failed(error))
            if error.to_string().contains("hook load 42")
    ));
    let snapshot = adapter.network_options_runtime_health_snapshot();
    assert_eq!(snapshot.hook_health, MpvNetworkOptionsHookHealth::Ready);
    assert_eq!(
        snapshot.media_policy,
        MpvNetworkMediaPolicyState::NetworkMediaUpdated,
        "snapshot-last reconciliation must identify the later explicit success as current"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn network_options_map_change_preserves_queued_hook_degradation_and_dedup_state() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.inject_test_network_media_options_hook_degradation("original lease failure");

    adapter.configure_network_media_options([("cache-secs", "90"), ("cache-pause-wait", "7")]);
    adapter
        .inject_test_network_media_options_hook_degradation("duplicate after settings-map change");

    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        adapter.take_network_media_options_transition_outcome()
    else {
        panic!("the original queued hook degradation must survive a settings-map change");
    };
    assert!(error.to_string().contains("original lease failure"));
    assert!(
        !error
            .to_string()
            .contains("duplicate after settings-map change"),
        "continuous degradation must retain the original observable failure"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "a map change must not rearm duplicate degradation while health remains degraded"
    );

    adapter.configure_test_network_options_hook_instance("test-hook-instance", 0);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered)
    );
    assert!(adapter.test_network_media_options_hook_is_ready());

    adapter.inject_test_network_media_options_hook_degradation("lease failed after recovery");
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        adapter.take_network_media_options_transition_outcome()
    else {
        panic!("positive recovery must rearm later degradation reporting");
    };
    assert!(error.to_string().contains("lease failed after recovery"));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );

    let mut ordered = configured_v3_hook_reducer_adapter();
    ordered.inject_test_network_media_options_hook_degradation("first independent failure");
    ordered.inject_test_network_media_options_hook_recovery();
    ordered.inject_test_network_media_options_hook_degradation("latest independent failure");

    ordered.configure_network_media_options([("cache-secs", "120")]);
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        ordered.take_network_media_options_transition_outcome()
    else {
        panic!("the pre-recovery degradation must survive a settings-map change");
    };
    assert!(error.to_string().contains("first independent failure"));
    assert_eq!(
        ordered.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "the recovery separating two independent degradations must remain observable"
    );
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        ordered.take_network_media_options_transition_outcome()
    else {
        panic!("the current post-recovery degradation must survive a settings-map change");
    };
    assert!(error.to_string().contains("latest independent failure"));
    assert_eq!(
        ordered.take_network_media_options_transition_outcome(),
        None,
        "D1, HookRecovered, D2 must remain exactly ordered after a map change"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn idle_and_local_observations_cannot_clear_hook_degradation_without_positive_recovery() {
    let mut adapter = configured_v3_hook_reducer_adapter();
    adapter.inject_test_network_media_options_hook_degradation("lease unavailable");
    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert!(!adapter.test_network_media_options_hook_is_ready());

    adapter.inject_test_network_media_options_no_active_media();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia),
        "terminal idle may resolve media policy without claiming the hook recovered"
    );
    assert!(
        !adapter.test_network_media_options_hook_is_ready(),
        "idle policy success must leave independent hook degradation intact"
    );

    adapter.begin_test_network_options_event_batch();
    adapter.observe_test_network_options_path("C:/media/local-after-degradation.mkv");
    adapter.end_test_network_options_event_batch();
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "an unsequenced local observation is not positive hook recovery"
    );
    assert!(!adapter.test_network_media_options_hook_is_ready());

    adapter.configure_test_network_options_hook_instance("test-hook-instance", 0);
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered)
    );
    assert!(adapter.test_network_media_options_hook_is_ready());
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[cfg(feature = "test-support")]
#[test]
fn superseded_policy_and_hook_health_resolve_independently_in_both_event_orders() {
    let mut idle_then_hook = superseded_network_options_adapter_awaiting_hook_result();
    idle_then_hook.inject_test_network_media_options_hook_degradation("lease unavailable");
    assert!(matches!(
        idle_then_hook.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert!(
        idle_then_hook.test_network_options_awaiting_authoritative_transition(),
        "hook degradation must preserve the successor's pending authoritative policy"
    );
    assert!(!idle_then_hook.test_network_media_options_hook_is_ready());

    idle_then_hook.inject_test_network_media_options_no_active_media();
    assert_eq!(
        idle_then_hook.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia)
    );
    assert!(
        !idle_then_hook.test_network_options_awaiting_authoritative_transition(),
        "terminal idle should resolve only the pending media policy"
    );
    assert!(
        !idle_then_hook.test_network_media_options_hook_is_ready(),
        "terminal idle must not recover the independent hook failure"
    );

    idle_then_hook.inject_test_network_media_options_hook_recovery();
    assert_eq!(
        idle_then_hook.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered)
    );
    assert!(idle_then_hook.test_network_media_options_hook_is_ready());
    assert_eq!(
        idle_then_hook.take_network_media_options_transition_outcome(),
        None
    );

    let mut hook_then_local = superseded_network_options_adapter_awaiting_hook_result();
    hook_then_local.inject_test_network_media_options_hook_degradation("lease unavailable");
    assert!(matches!(
        hook_then_local.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(_))
    ));
    assert!(hook_then_local.test_network_options_awaiting_authoritative_transition());

    hook_then_local.inject_test_network_media_options_hook_recovery();
    assert_eq!(
        hook_then_local.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "positive hook recovery must be reported before the later policy result"
    );
    assert!(
        hook_then_local.test_network_options_awaiting_authoritative_transition(),
        "hook recovery must not fabricate completion for the pending successor policy"
    );
    assert!(hook_then_local.test_network_media_options_hook_is_ready());

    hook_then_local.inject_test_network_media_options_local_media_unchanged();
    assert_eq!(
        hook_then_local.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged)
    );
    assert!(!hook_then_local.test_network_options_awaiting_authoritative_transition());
    assert!(hook_then_local.test_network_media_options_hook_is_ready());
    assert_eq!(
        hook_then_local.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn ownership_loss_is_typed_and_keeps_playback_attached() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes,
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::NetworkSuccess,
        lose_ownership_on_heartbeat: true,
        acknowledge_heartbeats: false,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial hook ownership should configure"),
        MpvActiveNetworkMediaOptionsApplyOutcome::Superseded
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );
    assert!(
        adapter.test_network_media_options_hook_is_ready(),
        "configured hook should remain ready before heartbeat testing"
    );

    adapter.force_test_network_media_options_hook_heartbeat_due();
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) =
        adapter.take_network_media_options_transition_outcome()
    else {
        panic!("ownership loss should publish a typed hook degradation");
    };
    assert!(error.to_string().contains("ownership was replaced"));
    assert!(
        adapter.is_connected(),
        "hook ownership loss is not IPC loss"
    );
}

#[test]
fn full_maintenance_reacquires_lost_hook_ownership_without_explicit_retry() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::StableNetwork,
        lose_ownership_on_heartbeat: true,
        acknowledge_heartbeats: false,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial hook ownership should configure"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated
    );
    adapter.force_test_network_media_options_hook_heartbeat_due();
    adapter.maintain_runtime_integrations();

    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error))
            if error.to_string().contains("ownership was replaced")
    ));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "bounded full maintenance should positively recover the transient ownership loss"
    );
    assert!(adapter.test_network_media_options_hook_is_ready());
    assert!(adapter.is_connected());

    let configure_count = writes
        .lock()
        .expect("lease recovery writes should not be poisoned")
        .iter()
        .filter(|write| write.contains("sorotte_network_options_configure"))
        .count();
    assert_eq!(
        configure_count, 2,
        "maintenance should reacquire exactly once"
    );
}

#[test]
fn transport_telemetry_only_pump_keeps_core_hook_ownership_live_past_the_lease() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::NetworkSuccess,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: true,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .apply_network_media_options_to_active_media_classified()
        .expect("initial hook ownership should configure");
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );

    let deadline = Instant::now() + Duration::from_millis(2_200);
    while Instant::now() < deadline {
        let _ = adapter.take_transport_telemetry_update();
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = adapter.take_transport_telemetry_update();

    assert!(adapter.is_connected());
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    let heartbeat_count = writes
        .lock()
        .expect("transport-only heartbeat writes should not be poisoned")
        .iter()
        .filter(|line| line.contains("sorotte_network_options_heartbeat"))
        .count();
    assert!(
        heartbeat_count >= 3,
        "transport-only polling should positively renew the two-second lease"
    );
}

#[test]
fn accepted_but_unacknowledged_heartbeat_recovers_without_explicit_retry() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes,
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::NetworkSuccess,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: false,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .apply_network_media_options_to_active_media_classified()
        .expect("initial hook ownership should configure");
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated)
    );

    adapter.force_test_network_media_options_hook_heartbeat_due();
    assert!(
        adapter.test_network_media_options_hook_heartbeat_pending(),
        "accepted heartbeat should remain pending until a positive acknowledgement"
    );
    adapter.force_test_network_media_options_hook_heartbeat_ack_timeout();
    adapter.maintain_runtime_integrations();
    assert_eq!(
        adapter
            .network_options_runtime_health_snapshot()
            .hook_health,
        crate::MpvNetworkOptionsHookHealth::Ready,
        "missed heartbeat recovery should finish before the GUI consumes health transitions"
    );
    let outcome = adapter.take_network_media_options_transition_outcome();
    let Some(MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)) = outcome else {
        panic!(
            "missing positive heartbeat acknowledgement should degrade the hook, got {outcome:?}"
        );
    };
    assert!(
        error
            .to_string()
            .contains("did not acknowledge heartbeat nonce")
    );
    assert!(
        adapter.is_connected(),
        "hook degradation must remain scoped"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::HookRecovered),
        "full maintenance should reconfigure after a missed heartbeat acknowledgement"
    );
    assert!(adapter.test_network_media_options_hook_is_ready());
}

#[test]
fn graceful_cleanup_releases_the_core_hook_before_optional_bridge_release() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let transport = NetworkOptionsHookScenarioTransport {
        writes: Arc::clone(&writes),
        responses: VecDeque::new(),
        old_network_succeeds: true,
        target: HookSupersessionTarget::NetworkSuccess,
        lose_ownership_on_heartbeat: false,
        acknowledge_heartbeats: true,
        expire_ownership_on_second_active_apply: false,
        active_apply_count: 0,
    };
    let mut adapter = MpvAdapter::with_network_options_hook_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .apply_network_media_options_to_active_media_classified()
        .expect("hook ownership should configure before cleanup");
    adapter.release_sorotte_bridge_best_effort();

    let commands = writes
        .lock()
        .expect("cleanup writes should not be poisoned")
        .iter()
        .map(|line| serde_json::from_str::<Value>(line.trim_end()).unwrap())
        .collect::<Vec<_>>();
    let release = commands
        .iter()
        .find(|request| {
            request.pointer("/command/2").and_then(Value::as_str)
                == Some("sorotte_network_options_release")
        })
        .expect("terminal cleanup must release core network-hook ownership");
    assert_eq!(
        release.pointer("/command/1").and_then(Value::as_str),
        Some("sorotte_network_options")
    );
}

#[test]
fn newer_network_success_supersedes_rejected_older_attempt_and_remains_final_outcome() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":83}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/a.m3u8"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":84}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/c.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":3,"error":"invalid parameter"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local path should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );

    adapter
        .set_playback_rate(1.0)
        .expect("the triggering command should remain accepted");

    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "the newer complete C application must be the only observable outcome"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    assert_eq!(
        adapter.current_path(),
        Some("https://media.example.test/c.m3u8")
    );
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/"))
            .count(),
        3,
        "A writes once before supersession and C receives the complete two-option set"
    );
}

#[test]
fn authoritative_null_rearms_same_url_transition_without_mutating_idle_media() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"https://media.example.test/live.m3u8"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":null}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/live.m3u8"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .apply_network_media_options_to_active_media()
        .expect("initial active network file should accept its options");

    adapter
        .set_playback_rate(1.0)
        .expect("the authoritative idle transition should remain healthy");
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        1,
        "idle media must not receive a file-local option write"
    );
    adapter
        .set_playback_rate(1.0)
        .expect("the same URL after idle should be treated as a new authoritative transition");
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        2
    );
}

#[test]
fn sorotte_network_loadfile_path_echo_does_not_double_apply_embedded_options() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":72}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/rejected.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"invalid parameter"}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/stale-local.mkv"}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/pre-start-network.m3u8"}"#,
        r#"{"event":"start-file","playlist_entry_id":999}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/stale-network.m3u8"}"#,
        r#"{"event":"start-file","playlist_entry_id":73}"#,
        r#"{"event":"property-change","name":"path","data":null}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/sorotte.m3u8"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local path should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );
    adapter
        .set_playback_rate(1.0)
        .expect("the command that observes the external transition should succeed");
    let file_local_writes_before_sorotte_load = state
        .writes()
        .iter()
        .filter(|write| write.contains("file-local-options/"))
        .count();
    assert_eq!(file_local_writes_before_sorotte_load, 1);

    adapter
        .open_file("https://media.example.test/sorotte.m3u8")
        .expect("Sorotte network loadfile should be accepted");

    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/"))
            .count(),
        file_local_writes_before_sorotte_load,
        "stale local/network/null observations and the final path echo must not duplicate loadfile's embedded options"
    );
    let loadfile = state
        .writes()
        .iter()
        .map(|write| serde_json::from_str::<Value>(write.trim_end()).expect("valid command json"))
        .find(|command| command.pointer("/command/0").and_then(Value::as_str) == Some("loadfile"))
        .expect("loadfile command should be present");
    assert_eq!(loadfile["command"][4]["cache-secs"], json!("75"));
    assert!(matches!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::Failed(_))
    ));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "embedded Sorotte options must signal recovery from the earlier external rejection"
    );
}

#[test]
fn pending_sorotte_load_poll_applies_mismatched_network_path_and_retains_target_marker() {
    let requested_target = "https://media.example.test/requested-a.m3u8";
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":701}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/external-b.m3u8"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success","data":"https://media.example.test/external-b.m3u8"}"#,
        r#"{"request_id":11,"error":"success","data":10.0}"#,
        r#"{"request_id":12,"error":"success","data":0}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"request_id":15,"error":"success","data":20.0}"#,
        r#"{"request_id":16,"error":"success","data":0}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .open_file(requested_target)
        .expect("Sorotte network loadfile should be accepted");

    assert_eq!(
        adapter.take_local_file_update(),
        None,
        "the mismatched poll must not complete Sorotte's pending A load"
    );
    assert_eq!(
        adapter.current_path(),
        None,
        "an uncorrelated authoritative path must not be mixed into the pending attempt's physical projection"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "fresh polling must apply policy to authoritative external B"
    );
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        1
    );

    let update = adapter
        .take_local_file_update()
        .expect("a later matching A poll should complete the pending Sorotte load");
    assert_eq!(update.path.as_deref(), Some(requested_target));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "matching A should consume its retained embedded marker without another write"
    );
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        1
    );
}

#[test]
fn pending_sorotte_load_drains_matching_start_and_path_events_before_poll_response() {
    let requested_target = "https://media.example.test/requested-a.m3u8";
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
        r#"{"event":"start-file","playlist_entry_id":702}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"request_id":10,"error":"success","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"request_id":11,"error":"success","data":20.0}"#,
        r#"{"request_id":12,"error":"success","data":0}"#,
        r#"{"request_id":13,"error":"success","data":"https://media.example.test/requested-a.m3u8"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .open_file(requested_target)
        .expect("Sorotte network loadfile should be accepted");

    let update = adapter
        .take_local_file_update()
        .expect("the matching poll should complete the pending Sorotte load");
    assert_eq!(update.path.as_deref(), Some(requested_target));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "the queued path echo should consume the embedded-options marker"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "the matching poll must not report a second application"
    );
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        0,
        "the queued path echo must not duplicate loadfile's embedded options after polling"
    );
    let loadfile = state
        .writes()
        .iter()
        .map(|write| serde_json::from_str::<Value>(write.trim_end()).expect("valid command json"))
        .find(|command| command.pointer("/command/0").and_then(Value::as_str) == Some("loadfile"))
        .expect("loadfile command should be present");
    assert_eq!(loadfile["command"][4]["cache-secs"], json!("75"));
}

#[test]
fn buffered_network_then_local_batch_never_writes_network_options_to_local_media() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":801}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/transient.m3u8"}"#,
        r#"{"event":"start-file","playlist_entry_id":802}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/final-local.mkv"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75"), ("cache-pause-wait", "5")]);

    adapter
        .set_playback_rate(1.0)
        .expect("the command that collects the transition batch should succeed");

    assert_eq!(adapter.current_path(), Some("C:/media/final-local.mkv"));
    assert!(
        state
            .writes()
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "no option write may begin until the whole buffered batch resolves to local media"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None,
        "a transient network path superseded in the same batch has no apply outcome"
    );
}

#[test]
fn trailing_start_in_buffered_batch_invalidates_earlier_network_path_before_write() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":803}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/transient.m3u8"}"#,
        r#"{"event":"start-file","playlist_entry_id":804}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .set_playback_rate(1.0)
        .expect("the command that collects the transition batch should succeed");

    assert_eq!(adapter.current_path(), None);
    assert!(
        state
            .writes()
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "a later start without a path must invalidate the earlier network candidate"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn matching_end_file_in_buffered_batch_invalidates_earlier_network_path_before_write() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":805}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/ended.m3u8"}"#,
        r#"{"event":"end-file","playlist_entry_id":805,"reason":"eof"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .set_playback_rate(1.0)
        .expect("the command that collects the lifecycle batch should succeed");

    assert!(
        state
            .writes()
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "a matching end-file must cancel the earlier network candidate before any write"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn composite_poll_revalidates_path_after_newer_local_events_during_metadata_reads() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success","data":"https://media.example.test/stale-a.m3u8"}"#,
        r#"{"event":"start-file","playlist_entry_id":811}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/newer-b.mkv"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":10,"error":"success","data":10.0}"#,
        r#"{"request_id":11,"error":"success","data":0}"#,
        r#"{"request_id":12,"error":"success","data":"C:/media/newer-b.mkv"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    assert_eq!(
        adapter.take_local_file_update(),
        None,
        "metadata from stale network A must not be published as local B metadata"
    );
    assert_eq!(adapter.current_path(), Some("C:/media/newer-b.mkv"));
    assert!(
        state
            .writes()
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "the final path revalidation must prevent a stale network-A option write"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn nested_poll_from_final_query_events_outranks_captured_outer_path() {
    let requested_target = "https://media.example.test/requested-a.m3u8";
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
        r#"{"request_id":10,"error":"success","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"request_id":11,"error":"success","data":20.0}"#,
        r#"{"request_id":12,"error":"success","data":0}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":13,"error":"success","data":"https://media.example.test/requested-a.m3u8"}"#,
        r#"{"request_id":14,"error":"success","data":"C:/media/newer-b.mkv"}"#,
        r#"{"request_id":15,"error":"success","data":10.0}"#,
        r#"{"request_id":16,"error":"success","data":0}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    adapter
        .open_file(requested_target)
        .expect("Sorotte network loadfile should be accepted");

    let update = adapter
        .take_local_file_update()
        .expect("the nested newer poll should publish its local target");
    assert_eq!(update.path.as_deref(), Some("C:/media/newer-b.mkv"));
    assert_eq!(adapter.current_path(), Some("C:/media/newer-b.mkv"));
    assert!(
        state
            .writes()
            .iter()
            .all(|write| !write.contains("file-local-options/")),
        "a nested newer poll must prevent the captured outer network path from being applied"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn superseded_option_write_still_reports_fatal_transport_loss() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":821}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/failing-a.m3u8"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":822}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/superseding-b.mkv"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);

    adapter
        .set_playback_rate(1.0)
        .expect("the outer triggering command should be acknowledged before transport loss");

    assert!(!adapter.is_connected());
    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("transport loss must remain observable after path supersession");
    let MpvNetworkMediaOptionsTransitionOutcome::Failed(error) = outcome else {
        panic!("expected a failed transition outcome, got {outcome:?}");
    };
    assert!(error.to_string().contains("unexpected EOF"));
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
}

#[test]
fn external_network_option_rejection_is_queued_while_healthy_and_only_once() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":52}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/main.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"invalid parameter"}"#,
        r#"{"event":"start-file","playlist_entry_id":53}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/recovery-local.mkv"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"start-file","playlist_entry_id":54}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/recovered.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local path should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );

    adapter
        .set_playback_rate(1.0)
        .expect("the triggering player command itself should remain accepted");
    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("the transition-time option rejection must be observable");
    let MpvNetworkMediaOptionsTransitionOutcome::Failed(error) = outcome else {
        panic!("expected failed transition outcome, got {outcome:?}");
    };
    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("invalid parameter")),
        "unexpected transition error: {error:?}"
    );
    assert!(
        adapter.is_connected(),
        "server rejection must leave IPC healthy"
    );
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        None
    );
    adapter
        .set_playback_rate(1.0)
        .expect("local interlude and later network recovery should remain healthy");
    assert_eq!(
        adapter.take_network_media_options_transition_outcome(),
        Some(MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated),
        "a later successful network generation must clear higher-layer degradation"
    );
    assert_eq!(
        state
            .writes()
            .iter()
            .filter(|write| write.contains("file-local-options/cache-secs"))
            .count(),
        2
    );
}

#[test]
fn external_network_option_transport_loss_is_queued_and_marks_ipc_unhealthy() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"C:/media/local-intro.mkv"}"#,
        r#"{"event":"start-file","playlist_entry_id":62}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.example.test/main.m3u8"}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        adapter
            .apply_network_media_options_to_active_media_classified()
            .expect("initial local path should remain healthy"),
        MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged
    );

    adapter
        .set_playback_rate(1.0)
        .expect("the triggering player command was acknowledged before transport loss");
    let outcome = adapter
        .take_network_media_options_transition_outcome()
        .expect("the transition-time transport loss must be observable");
    let MpvNetworkMediaOptionsTransitionOutcome::Failed(error) = outcome else {
        panic!("expected failed transition outcome, got {outcome:?}");
    };
    assert!(
        matches!(error, PlayerError::OperationFailed(ref message) if message.contains("unexpected EOF")),
        "unexpected transition error: {error:?}"
    );
    assert!(!adapter.is_connected());
    assert!(
        adapter
            .take_ipc_connection_events()
            .iter()
            .any(|event| { matches!(event, MpvIpcConnectionEvent::Disconnected { .. }) })
    );
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

    let observation = adapter
        .take_media_load_observation()
        .expect("file-loaded should emit a sequenced success outcome");
    assert_eq!(
        observation.outcome,
        PlayerMediaLoadOutcome::success("movie.mkv", Some("movie.mkv".to_owned()))
    );
    assert_eq!(
        observation
            .media_generation
            .map(sorotte_player_api::PlayerMediaGeneration::get),
        Some(1)
    );
    assert!(observation.observed_at.is_some());
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
