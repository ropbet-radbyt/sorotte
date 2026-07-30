use super::{
    ConnectedMpvPlayer, LegacySyncplayOsdKind, LegacySyncplayUiSettings,
    MpvActiveNetworkMediaOptionsApplyOutcome, MpvAdapter, MpvNetworkMediaOptionsTransitionOutcome,
    SimulatedPlayer, SorotteBridgeFailureKind, SorotteBridgeHealth,
};
#[cfg(feature = "test-support")]
use super::{
    MpvNetworkMediaPolicyOutcome, MpvNetworkMediaPolicyState, MpvNetworkOptionsHookHealth,
    MpvNetworkOptionsHookHealthTransition,
};
use crate::constants::{
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED,
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED, LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG,
    LEGACY_SYNCPLAYINTF_PROTOCOL, LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE,
    LEGACY_SYNCPLAYINTF_SCRIPT_NAME, MPV_COMMAND_GET_PROPERTY, MPV_COMMAND_LOADFILE,
    MPV_EVENT_START_FILE, MPV_PROPERTY_DURATION, MPV_PROPERTY_FILE_SIZE, MPV_PROPERTY_PATH,
    MPV_PROPERTY_PLAYLIST,
};
use crate::ipc::{MpvJsonIpcTransport, read_line_from_stream};
use serde_json::{Value, json};
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCommand, PlayerError, PlayerMediaLoadFailureKind,
    PlayerMediaLoadOutcome, PlayerPlaybackTelemetryUpdate, PlayerSeekableRange,
    PlayerTransportPhase,
};
use std::{
    collections::VecDeque,
    fs::File,
    io,
    io::Write,
    sync::{Arc, Mutex},
};

mod command_tests;
mod event_tests;
mod ipc_duplex_fault_tests;
#[cfg(windows)]
mod ipc_named_pipe_fault_tests;
mod ipc_tests;
mod legacy_ui_tests;
mod network_options_lua_tests;
mod smoke_tests;
mod state_tests;
mod syncplayintf_lua_tests;

const FAKE_SYNCPLAYINTF_PONG_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_PONG__";
const FAKE_SYNCPLAYINTF_RELOADED_PONG_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_RELOADED_PONG__";
const FAKE_SYNCPLAYINTF_ACK_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_ACK__";
const FAKE_SYNCPLAYINTF_STALE_ACK_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_STALE_ACK__";
const FAKE_SYNCPLAYINTF_FUTURE_ACK_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_FUTURE_ACK__";
const FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_REJECTED_ACK__";
const FAKE_SYNCPLAYINTF_SETTINGS_REJECTED_ACK_EVENT: &str =
    "__SOROTTE_SYNCPLAYINTF_SETTINGS_REJECTED_ACK__";
const FAKE_SYNCPLAYINTF_MALFORMED_ACK_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_MALFORMED_ACK__";
const FAKE_SYNCPLAYINTF_LEASE_EXPIRED_EVENT: &str = "__SOROTTE_SYNCPLAYINTF_LEASE_EXPIRED__";

fn fake_transport_with_reads(lines: &[&str]) -> (FakeTransport, FakeTransportStateHandle) {
    let shared = Arc::new(Mutex::new(FakeTransportState {
        reads: lines
            .iter()
            .map(|line| {
                let mut owned = (*line).to_owned();
                owned.push('\n');
                owned
            })
            .collect(),
        writes: Vec::new(),
        current_playlist_entry_id: None,
        current_playlist_path: None,
        playlist_query_overrides: VecDeque::new(),
        synthesize_path_queries: false,
    }));
    (
        FakeTransport {
            shared: Arc::clone(&shared),
        },
        FakeTransportStateHandle { shared },
    )
}

#[derive(Debug)]
struct FakeTransport {
    shared: Arc<Mutex<FakeTransportState>>,
}

impl MpvJsonIpcTransport for FakeTransport {
    fn send_line_until(&mut self, line: &str, _deadline: std::time::Instant) -> io::Result<()> {
        let mut state = self
            .shared
            .lock()
            .expect("fake transport mutex should not be poisoned");
        state.writes.push(line.to_owned());
        let Ok(request) = serde_json::from_str::<Value>(line.trim_end()) else {
            return Ok(());
        };
        let Some(request_id) = request.get("request_id").and_then(Value::as_u64) else {
            return Ok(());
        };
        let Some(command) = request.get("command").and_then(Value::as_array) else {
            return Ok(());
        };
        if command.first().and_then(Value::as_str) == Some(MPV_COMMAND_LOADFILE) {
            state.current_playlist_entry_id = Some(
                state
                    .current_playlist_entry_id
                    .unwrap_or(0)
                    .saturating_add(1),
            );
            state.current_playlist_path = command
                .get(1)
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        } else if command.first().and_then(Value::as_str) == Some(MPV_COMMAND_GET_PROPERTY)
            && command.get(1).and_then(Value::as_str) == Some(MPV_PROPERTY_PLAYLIST)
        {
            // Existing scripted fixtures predate the causal playlist query. Keep their later
            // response IDs aligned, then synthesize mpv's authoritative current entry without
            // consuming an unrelated event or response from the script.
            for queued in &mut state.reads {
                let Ok(mut value) = serde_json::from_str::<Value>(queued.trim_end()) else {
                    continue;
                };
                let Some(scripted_id) = value.get("request_id").and_then(Value::as_u64) else {
                    continue;
                };
                if scripted_id >= request_id {
                    value["request_id"] = json!(scripted_id.saturating_add(1));
                    *queued = value.to_string() + "\n";
                }
            }
            let response = match state.playlist_query_overrides.pop_front() {
                Some(FakePlaylistQueryOverride::Unavailable) => json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": null,
                }),
                Some(FakePlaylistQueryOverride::Error) => json!({
                    "request_id": request_id,
                    "error": "property unavailable",
                }),
                None => {
                    if let Some(queued_start_id) = state.reads.iter().find_map(|queued| {
                        let event = serde_json::from_str::<Value>(queued.trim_end()).ok()?;
                        (event.get("event").and_then(Value::as_str) == Some(MPV_EVENT_START_FILE))
                            .then(|| event.get("playlist_entry_id").and_then(Value::as_u64))
                            .flatten()
                    }) {
                        state.current_playlist_entry_id = Some(queued_start_id);
                    }
                    let data = state.current_playlist_entry_id.map(|id| {
                        json!([{
                            "id": id,
                            "filename": state.current_playlist_path,
                            "current": true,
                            "playing": true,
                        }])
                    });
                    json!({
                        "request_id": request_id,
                        "error": "success",
                        "data": data,
                    })
                }
            };
            state.reads.push_front(response.to_string() + "\n");
        } else if command.first().and_then(Value::as_str) == Some(MPV_COMMAND_GET_PROPERTY)
            && command.get(1).and_then(Value::as_str) == Some(MPV_PROPERTY_PATH)
            && state.synthesize_path_queries
        {
            for queued in &mut state.reads {
                let Ok(mut value) = serde_json::from_str::<Value>(queued.trim_end()) else {
                    continue;
                };
                let Some(scripted_id) = value.get("request_id").and_then(Value::as_u64) else {
                    continue;
                };
                if scripted_id >= request_id {
                    value["request_id"] = json!(scripted_id.saturating_add(1));
                    *queued = value.to_string() + "\n";
                }
            }
            let current_playlist_path = state.current_playlist_path.clone();
            state.reads.push_front(
                json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": current_playlist_path,
                })
                .to_string()
                    + "\n",
            );
        } else if command.first().and_then(Value::as_str) == Some(MPV_COMMAND_GET_PROPERTY)
            && state.synthesize_path_queries
            && matches!(
                command.get(1).and_then(Value::as_str),
                Some(MPV_PROPERTY_DURATION) | Some(MPV_PROPERTY_FILE_SIZE)
            )
        {
            for queued in &mut state.reads {
                let Ok(mut value) = serde_json::from_str::<Value>(queued.trim_end()) else {
                    continue;
                };
                let Some(scripted_id) = value.get("request_id").and_then(Value::as_u64) else {
                    continue;
                };
                if scripted_id >= request_id {
                    value["request_id"] = json!(scripted_id.saturating_add(1));
                    *queued = value.to_string() + "\n";
                }
            }
            let data = if command.get(1).and_then(Value::as_str) == Some(MPV_PROPERTY_DURATION) {
                json!(1200.0)
            } else {
                json!(4096)
            };
            state.reads.push_front(
                json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": data,
                })
                .to_string()
                    + "\n",
            );
        }
        Ok(())
    }

    fn read_line_until(
        &mut self,
        line: &mut String,
        _deadline: std::time::Instant,
    ) -> io::Result<usize> {
        let mut guard = self
            .shared
            .lock()
            .expect("fake transport mutex should not be poisoned");
        let Some(next) = guard.reads.pop_front() else {
            line.clear();
            return Ok(0);
        };
        let next = fake_syncplayintf_event_for_marker(next.trim(), &guard.writes).unwrap_or(next);
        if let Ok(event) = serde_json::from_str::<Value>(next.trim_end())
            && event.get("event").and_then(Value::as_str) == Some(MPV_EVENT_START_FILE)
            && let Some(playlist_entry_id) = event.get("playlist_entry_id").and_then(Value::as_u64)
        {
            guard.current_playlist_entry_id = Some(
                guard
                    .current_playlist_entry_id
                    .map_or(playlist_entry_id, |current| current.max(playlist_entry_id)),
            );
        }
        line.clear();
        line.push_str(&next);
        Ok(line.len())
    }
}

fn fake_syncplayintf_event_for_marker(marker: &str, writes: &[String]) -> Option<String> {
    let message_name = match marker {
        FAKE_SYNCPLAYINTF_PONG_EVENT | FAKE_SYNCPLAYINTF_RELOADED_PONG_EVENT => {
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG
        }
        FAKE_SYNCPLAYINTF_LEASE_EXPIRED_EVENT => LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED,
        FAKE_SYNCPLAYINTF_ACK_EVENT
        | FAKE_SYNCPLAYINTF_STALE_ACK_EVENT
        | FAKE_SYNCPLAYINTF_FUTURE_ACK_EVENT
        | FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT
        | FAKE_SYNCPLAYINTF_SETTINGS_REJECTED_ACK_EVENT
        | FAKE_SYNCPLAYINTF_MALFORMED_ACK_EVENT => {
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED
        }
        _ => return None,
    };
    if marker == FAKE_SYNCPLAYINTF_MALFORMED_ACK_EVENT {
        return Some(
            json!({
                "event": "client-message",
                "args": [message_name, "{malformed-json"],
            })
            .to_string()
                + "\n",
        );
    }

    let request = writes
        .last()
        .and_then(|line| serde_json::from_str::<Value>(line.trim()).ok())?;
    let request_payload = request
        .pointer("/command/3")
        .and_then(Value::as_str)
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())?;
    let payload = if matches!(
        marker,
        FAKE_SYNCPLAYINTF_PONG_EVENT | FAKE_SYNCPLAYINTF_RELOADED_PONG_EVENT
    ) {
        json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "nonce": request_payload.get("nonce")?,
            "bridgeInstanceId": if marker == FAKE_SYNCPLAYINTF_RELOADED_PONG_EVENT {
                "reloaded-test-bridge"
            } else {
                "test-bridge"
            },
            "scriptName": LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
        })
    } else if marker == FAKE_SYNCPLAYINTF_LEASE_EXPIRED_EVENT {
        json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "bridgeInstanceId": request_payload.get("bridgeInstanceId")?,
            "ownerId": request_payload.get("ownerId")?,
            "attachmentId": request_payload.get("attachmentId")?,
        })
    } else {
        let generation = request_payload.get("generation")?.as_u64()?;
        let generation = match marker {
            FAKE_SYNCPLAYINTF_STALE_ACK_EVENT => generation.saturating_sub(1),
            FAKE_SYNCPLAYINTF_FUTURE_ACK_EVENT => generation.saturating_add(1),
            _ => generation,
        };
        let status = match marker {
            FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT => "busy",
            FAKE_SYNCPLAYINTF_SETTINGS_REJECTED_ACK_EVENT => "rejected",
            _ => "applied",
        };
        json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "bridgeInstanceId": request_payload.get("bridgeInstanceId")?,
            "ownerId": request_payload.get("ownerId")?,
            "attachmentId": request_payload.get("attachmentId")?,
            "generation": generation,
            "status": status,
            "error": match status {
                "busy" => Some("another Sorotte owner holds the live lease"),
                "rejected" => Some("the requested bridge settings were rejected"),
                _ => None,
            },
        })
    };
    Some(
        json!({
            "event": "client-message",
            "args": [message_name, payload.to_string()],
        })
        .to_string()
            + "\n",
    )
}

#[derive(Debug)]
struct FakeTransportState {
    reads: VecDeque<String>,
    writes: Vec<String>,
    current_playlist_entry_id: Option<u64>,
    current_playlist_path: Option<String>,
    playlist_query_overrides: VecDeque<FakePlaylistQueryOverride>,
    synthesize_path_queries: bool,
}

#[derive(Debug, Clone, Copy)]
enum FakePlaylistQueryOverride {
    Unavailable,
    Error,
}

#[derive(Debug)]
struct FakeTransportStateHandle {
    shared: Arc<Mutex<FakeTransportState>>,
}

impl FakeTransportStateHandle {
    fn writes(&self) -> Vec<String> {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .writes
            .clone()
    }

    fn queue_reads(&self, lines: &[&str]) {
        let mut state = self
            .shared
            .lock()
            .expect("fake transport mutex should not be poisoned");
        state.reads.extend(lines.iter().map(|line| {
            let mut line = (*line).to_owned();
            if !line.ends_with('\n') {
                line.push('\n');
            }
            line
        }));
    }

    fn queue_playlist_query_unavailable(&self) {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .playlist_query_overrides
            .push_back(FakePlaylistQueryOverride::Unavailable);
    }

    fn queue_playlist_query_error(&self) {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .playlist_query_overrides
            .push_back(FakePlaylistQueryOverride::Error);
    }

    fn synthesize_path_queries(&self) {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .synthesize_path_queries = true;
    }
}
