use super::{
    ConnectedMpvPlayer, LegacySyncplayOsdKind, LegacySyncplayUiSettings,
    MpvActiveNetworkMediaOptionsApplyOutcome, MpvAdapter, MpvNetworkMediaOptionsTransitionOutcome,
    MpvNetworkMediaPolicyOutcome, MpvNetworkMediaPolicyState, MpvNetworkOptionsHookHealth,
    MpvNetworkOptionsHookHealthTransition, SimulatedPlayer, SorotteBridgeFailureKind,
    SorotteBridgeHealth,
};
use crate::constants::{
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED,
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED, LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG,
    LEGACY_SYNCPLAYINTF_PROTOCOL, LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE,
    LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
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
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .writes
            .push(line.to_owned());
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
}
