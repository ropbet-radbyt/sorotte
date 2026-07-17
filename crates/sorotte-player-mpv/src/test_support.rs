use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use serde_json::{Value, json};

use crate::constants::{LEGACY_SYNCPLAYINTF_PING_MESSAGE, LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE};
use crate::ipc::{MpvJsonIpcClient, MpvJsonIpcTransport};

pub(crate) fn unacknowledging_syncplayintf_client() -> MpvJsonIpcClient {
    MpvJsonIpcClient::new(Box::new(SuccessfulNoAckTransport::default()))
}

pub(crate) fn undiscoverable_syncplayintf_client() -> MpvJsonIpcClient {
    MpvJsonIpcClient::new(Box::new(SuccessfulNoAckTransport::default()))
}

pub(crate) fn rejecting_syncplayintf_discovery_client() -> MpvJsonIpcClient {
    MpvJsonIpcClient::new(Box::new(DiscoveryRejectingTransport::default()))
}

pub(crate) fn release_recording_syncplayintf_client() -> (MpvJsonIpcClient, Arc<AtomicUsize>) {
    let release_count = Arc::new(AtomicUsize::new(0));
    let transport = ReleaseRecordingTransport {
        release_count: Arc::clone(&release_count),
        commands: None,
    };
    (MpvJsonIpcClient::new(Box::new(transport)), release_count)
}

pub(crate) fn cleanup_recording_syncplayintf_client() -> (MpvJsonIpcClient, Arc<Mutex<Vec<Value>>>)
{
    let commands = Arc::new(Mutex::new(Vec::new()));
    let transport = ReleaseRecordingTransport {
        release_count: Arc::new(AtomicUsize::new(0)),
        commands: Some(Arc::clone(&commands)),
    };
    (MpvJsonIpcClient::new(Box::new(transport)), commands)
}

pub(crate) fn reject_first_active_network_option_client() -> MpvJsonIpcClient {
    MpvJsonIpcClient::new(Box::new(RejectFirstActiveNetworkOptionTransport::default()))
}

#[derive(Debug, Default)]
struct SuccessfulNoAckTransport {
    responses: VecDeque<String>,
}

impl MpvJsonIpcTransport for SuccessfulNoAckTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let request_id = request.get("request_id").cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "test mpv request omitted request_id",
            )
        })?;
        let is_property_read = request
            .pointer("/command/0")
            .and_then(Value::as_str)
            .is_some_and(|command| command == "get_property");
        let response = if is_property_read {
            json!({"request_id": request_id, "error": "success", "data": false})
        } else {
            json!({"request_id": request_id, "error": "success"})
        };
        self.responses.push_back(response.to_string() + "\n");
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test mpv transport had no queued response",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

#[derive(Debug, Default)]
struct RejectFirstActiveNetworkOptionTransport {
    responses: VecDeque<String>,
    active_network_option_rejected: bool,
}

impl MpvJsonIpcTransport for RejectFirstActiveNetworkOptionTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let request_id = request.get("request_id").cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "test mpv request omitted request_id",
            )
        })?;
        let command = request
            .get("command")
            .and_then(Value::as_array)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing command array"))?;
        let command_name = command.first().and_then(Value::as_str);
        let property_name = command.get(1).and_then(Value::as_str);
        let response = match (command_name, property_name) {
            (Some("get_property"), Some("path")) => json!({
                "request_id": request_id,
                "error": "success",
                "data": "https://media.example.test/active.m3u8",
            }),
            (Some("get_property"), Some("osd-align-y")) => json!({
                "request_id": request_id,
                "error": "success",
                "data": "top",
            }),
            (Some("get_property"), Some("osd-margin-y")) => json!({
                "request_id": request_id,
                "error": "success",
                "data": 16,
            }),
            (Some("get_property"), _) => {
                json!({"request_id": request_id, "error": "success", "data": false})
            }
            (Some("set_property"), Some(property))
                if property.starts_with("file-local-options/")
                    && !self.active_network_option_rejected =>
            {
                self.active_network_option_rejected = true;
                json!({"request_id": request_id, "error": "invalid parameter"})
            }
            _ => json!({"request_id": request_id, "error": "success"}),
        };
        self.responses.push_back(response.to_string() + "\n");
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test mpv transport had no queued response",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

#[derive(Debug, Default)]
struct DiscoveryRejectingTransport {
    responses: VecDeque<String>,
}

impl MpvJsonIpcTransport for DiscoveryRejectingTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let request_id = request.get("request_id").cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "test mpv request omitted request_id",
            )
        })?;
        let rejects_discovery = request.pointer("/command/2").and_then(Value::as_str)
            == Some(LEGACY_SYNCPLAYINTF_PING_MESSAGE);
        let response = if rejects_discovery {
            json!({"request_id": request_id, "error": "invalid parameter"})
        } else if request.pointer("/command/0").and_then(Value::as_str) == Some("get_property") {
            json!({"request_id": request_id, "error": "success", "data": false})
        } else {
            json!({"request_id": request_id, "error": "success"})
        };
        self.responses.push_back(response.to_string() + "\n");
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test mpv transport had no queued response",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

#[derive(Debug)]
struct ReleaseRecordingTransport {
    release_count: Arc<AtomicUsize>,
    commands: Option<Arc<Mutex<Vec<Value>>>>,
}

impl MpvJsonIpcTransport for ReleaseRecordingTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(commands) = &self.commands {
            commands
                .lock()
                .expect("cleanup command log should not be poisoned")
                .push(request["command"].clone());
        }
        if request.pointer("/command/2").and_then(Value::as_str)
            == Some(LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE)
        {
            self.release_count.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    fn read_line_until(&mut self, _line: &mut String, _deadline: Instant) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "release-recording transport does not serve command responses",
        ))
    }
}
