use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    reject_nth_active_network_option_client(1).0
}

pub(crate) fn reject_nth_active_network_option_client(
    rejected_write: usize,
) -> (MpvJsonIpcClient, Arc<Mutex<Vec<Value>>>) {
    assert!(
        rejected_write > 0,
        "active-network option write indices are one-based"
    );
    let commands = Arc::new(Mutex::new(Vec::new()));
    let transport = RejectNthActiveNetworkOptionTransport {
        responses: VecDeque::new(),
        rejected_write,
        active_network_option_writes: 0,
        commands: Arc::clone(&commands),
    };
    (MpvJsonIpcClient::new(Box::new(transport)), commands)
}

pub(crate) fn delayed_active_network_media_client() -> (MpvJsonIpcClient, Arc<Mutex<Vec<Value>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let transport = DelayedActiveNetworkMediaTransport {
        responses: VecDeque::new(),
        path_queries: 0,
        commands: Arc::clone(&commands),
    };
    (MpvJsonIpcClient::new(Box::new(transport)), commands)
}

pub(crate) fn external_network_media_transition_client(
    reject_option_write: bool,
) -> (MpvJsonIpcClient, Arc<Mutex<Vec<Value>>>, Arc<AtomicBool>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let transition_trigger = Arc::new(AtomicBool::new(false));
    let transport = ExternalNetworkMediaTransitionTransport {
        responses: VecDeque::new(),
        commands: Arc::clone(&commands),
        transition_trigger: Arc::clone(&transition_trigger),
        transition_count: 0,
        reject_option_write,
        option_write_rejected: false,
    };
    (
        MpvJsonIpcClient::new(Box::new(transport)),
        commands,
        transition_trigger,
    )
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

#[derive(Debug)]
struct RejectNthActiveNetworkOptionTransport {
    responses: VecDeque<String>,
    rejected_write: usize,
    active_network_option_writes: usize,
    commands: Arc<Mutex<Vec<Value>>>,
}

impl MpvJsonIpcTransport for RejectNthActiveNetworkOptionTransport {
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
        let reject_active_network_option = command_name == Some("set_property")
            && property_name.is_some_and(|property| property.starts_with("file-local-options/"))
            && {
                self.active_network_option_writes += 1;
                self.commands
                    .lock()
                    .expect("active-network command log should not be poisoned")
                    .push(Value::Array(command.clone()));
                self.active_network_option_writes == self.rejected_write
            };
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
            (Some("set_property"), Some(_)) if reject_active_network_option => {
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

#[derive(Debug)]
struct DelayedActiveNetworkMediaTransport {
    responses: VecDeque<String>,
    path_queries: usize,
    commands: Arc<Mutex<Vec<Value>>>,
}

#[derive(Debug)]
struct ExternalNetworkMediaTransitionTransport {
    responses: VecDeque<String>,
    commands: Arc<Mutex<Vec<Value>>>,
    transition_trigger: Arc<AtomicBool>,
    transition_count: usize,
    reject_option_write: bool,
    option_write_rejected: bool,
}

impl MpvJsonIpcTransport for ExternalNetworkMediaTransitionTransport {
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

        if self.transition_trigger.swap(false, Ordering::SeqCst) {
            if self.transition_count > 0 {
                self.responses.push_back(
                    json!({"event": "start-file", "playlist_entry_id": 43}).to_string() + "\n",
                );
                self.responses.push_back(
                    json!({
                        "event": "property-change",
                        "name": "path",
                        "data": "C:/media/recovery-local.mkv",
                    })
                    .to_string()
                        + "\n",
                );
                self.responses
                    .push_back(json!({"event": "file-loaded"}).to_string() + "\n");
            }
            self.transition_count += 1;
            self.responses.push_back(
                json!({
                    "event": "start-file",
                    "playlist_entry_id": 43 + self.transition_count,
                })
                .to_string()
                    + "\n",
            );
            self.responses.push_back(
                json!({
                    "event": "property-change",
                    "name": "path",
                    "data": "https://media.example.test/main-stream.m3u8",
                })
                .to_string()
                    + "\n",
            );
            self.responses
                .push_back(json!({"event": "file-loaded"}).to_string() + "\n");
        }

        let is_file_local_write = command_name == Some("set_property")
            && property_name.is_some_and(|property| property.starts_with("file-local-options/"));
        if is_file_local_write {
            self.commands
                .lock()
                .expect("external-transition command log should not be poisoned")
                .push(Value::Array(command.clone()));
        }
        let reject_this_option_write =
            is_file_local_write && self.reject_option_write && !self.option_write_rejected;
        if reject_this_option_write {
            self.option_write_rejected = true;
        }
        let response = match (command_name, property_name) {
            (Some("get_property"), Some("path")) => json!({
                "request_id": request_id,
                "error": "success",
                "data": if self.transition_count > 0 {
                    "https://media.example.test/main-stream.m3u8"
                } else {
                    "C:/media/local-intro.mkv"
                },
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
            (Some("set_property"), Some(_)) if reject_this_option_write => {
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

impl MpvJsonIpcTransport for DelayedActiveNetworkMediaTransport {
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
            (Some("get_property"), Some("path")) => {
                self.path_queries += 1;
                if self.path_queries == 1 {
                    json!({"request_id": request_id, "error": "success", "data": null})
                } else {
                    json!({
                        "request_id": request_id,
                        "error": "success",
                        "data": "https://media.example.test/delayed.m3u8",
                    })
                }
            }
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
                if property.starts_with("file-local-options/") =>
            {
                self.commands
                    .lock()
                    .expect("active-network command log should not be poisoned")
                    .push(Value::Array(command.clone()));
                json!({"request_id": request_id, "error": "success"})
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
