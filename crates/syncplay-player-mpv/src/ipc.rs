use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    path::Path,
    sync::mpsc,
    time::Duration,
};

use serde_json::{Value, json};

use crate::constants::{
    MPV_COMMAND_GET_PROPERTY, MPV_COMMAND_OBSERVE_PROPERTY, MPV_RESPONSE_SUCCESS,
};

pub(crate) trait MpvJsonIpcTransport: Send + Sync {
    fn send_line(&mut self, line: &str) -> io::Result<()>;
    fn read_line(&mut self, line: &mut String) -> io::Result<usize>;
}

pub(crate) const MPV_IPC_MAX_LINE_BYTES: usize = 1024 * 1024;
pub(crate) const MPV_IPC_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct MpvJsonIpcClient {
    command_tx: mpsc::Sender<MpvIpcWorkerRequest>,
    command_timeout: Duration,
    pending_events: VecDeque<Value>,
}

impl MpvJsonIpcClient {
    pub(crate) fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self::new_with_command_timeout(transport, MPV_IPC_COMMAND_TIMEOUT)
    }

    pub(crate) fn new_with_command_timeout(
        transport: Box<dyn MpvJsonIpcTransport>,
        command_timeout: Duration,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<MpvIpcWorkerRequest>();
        let _worker_thread = std::thread::spawn(move || {
            let mut worker = MpvIpcWorker::new(transport);
            while let Ok(request) = command_rx.recv() {
                let outcome = worker.send_command(request.command);
                let _ = request.response_tx.send(outcome);
            }
        });
        Self {
            command_tx,
            command_timeout,
            pending_events: VecDeque::new(),
        }
    }

    pub(crate) fn connect(path: &Path) -> Result<Self, String> {
        let transport = MpvPipeTransport::connect(path)
            .map_err(|err| format!("failed to connect mpv IPC at {}: {err}", path.display()))?;
        Ok(Self::new(Box::new(transport)))
    }

    pub(crate) fn send_command_expect_success(&mut self, command: Value) -> Result<(), String> {
        self.send_command(command).map(|_| ())
    }

    pub(crate) fn observe_property(
        &mut self,
        observer_id: u64,
        property_name: &str,
    ) -> Result<(), String> {
        self.send_command_expect_success(json!([
            MPV_COMMAND_OBSERVE_PROPERTY,
            observer_id,
            property_name
        ]))
    }

    pub(crate) fn get_property(&mut self, property_name: &str) -> Result<Option<Value>, String> {
        let response = self.send_command(json!([MPV_COMMAND_GET_PROPERTY, property_name]))?;
        Ok(response
            .get("data")
            .cloned()
            .filter(|value| !value.is_null()))
    }

    pub(crate) fn get_property_string(
        &mut self,
        property_name: &str,
    ) -> Result<Option<String>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(Value::as_str)
            .map(ToOwned::to_owned))
    }

    pub(crate) fn get_property_f64(&mut self, property_name: &str) -> Result<Option<f64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value.as_ref().and_then(Value::as_f64))
    }

    pub(crate) fn get_property_u64(&mut self, property_name: &str) -> Result<Option<u64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok())))
    }

    fn send_command(&mut self, command: Value) -> Result<Value, String> {
        let (response_tx, response_rx) = mpsc::channel();
        self.command_tx
            .send(MpvIpcWorkerRequest {
                command,
                response_tx,
            })
            .map_err(|err| format!("failed to queue mpv IPC command: {err}"))?;
        let outcome = match response_rx.recv_timeout(self.command_timeout) {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!(
                    "mpv IPC command timed out after {:.1} seconds",
                    self.command_timeout.as_secs_f64()
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("mpv IPC command worker disconnected".to_owned());
            }
        };
        self.pending_events.extend(outcome.pending_events);
        outcome.result
    }

    pub(crate) fn take_pending_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
    }
}

struct MpvIpcWorkerRequest {
    command: Value,
    response_tx: mpsc::Sender<MpvIpcCommandOutcome>,
}

struct MpvIpcCommandOutcome {
    result: Result<Value, String>,
    pending_events: Vec<Value>,
}

struct MpvIpcWorker {
    transport: Box<dyn MpvJsonIpcTransport>,
    next_request_id: u64,
}

impl MpvIpcWorker {
    fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self {
            transport,
            next_request_id: 1,
        }
    }

    fn send_command(&mut self, command: Value) -> MpvIpcCommandOutcome {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let mut pending_events = Vec::new();

        let request = json!({
            "command": command,
            "request_id": request_id,
        });
        let mut line = match serde_json::to_string(&request) {
            Ok(line) => line,
            Err(err) => {
                return MpvIpcCommandOutcome {
                    result: Err(format!("failed to serialize mpv IPC request: {err}")),
                    pending_events,
                };
            }
        };
        line.push('\n');
        if let Err(err) = self.transport.send_line(&line) {
            return MpvIpcCommandOutcome {
                result: Err(format!("failed to write mpv IPC request: {err}")),
                pending_events,
            };
        }

        let mut response_line = String::new();
        loop {
            let bytes_read = match self.transport.read_line(&mut response_line) {
                Ok(bytes_read) => bytes_read,
                Err(err) => {
                    return MpvIpcCommandOutcome {
                        result: Err(format!("failed to read mpv IPC response: {err}")),
                        pending_events,
                    };
                }
            };
            if bytes_read == 0 {
                return MpvIpcCommandOutcome {
                    result: Err(format!(
                        "unexpected EOF while waiting for mpv IPC response (request_id={request_id})"
                    )),
                    pending_events,
                };
            }
            if let Err(err) = validate_mpv_ipc_line_len(response_line.as_bytes()) {
                return MpvIpcCommandOutcome {
                    result: Err(err),
                    pending_events,
                };
            }

            let trimmed = response_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }

            let parsed: Value = match serde_json::from_str(trimmed) {
                Ok(parsed) => parsed,
                Err(err) => {
                    return MpvIpcCommandOutcome {
                        result: Err(format!("invalid mpv IPC JSON line '{trimmed}': {err}")),
                        pending_events,
                    };
                }
            };
            if parsed.get("event").and_then(Value::as_str).is_some() {
                pending_events.push(parsed);
                continue;
            }
            let Some(parsed_request_id) = parsed.get("request_id").and_then(Value::as_u64) else {
                // Ignore non-event lines without request_id while waiting for the response.
                continue;
            };
            if parsed_request_id != request_id {
                continue;
            }

            let error = parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("<missing error>");
            if error != MPV_RESPONSE_SUCCESS {
                return MpvIpcCommandOutcome {
                    result: Err(format!(
                        "mpv command failed for request_id={request_id}: {error}"
                    )),
                    pending_events,
                };
            }

            return MpvIpcCommandOutcome {
                result: Ok(parsed),
                pending_events,
            };
        }
    }
}

struct MpvPipeTransport {
    stream: MpvPipeStream,
    read_buffer: Vec<u8>,
}

impl MpvPipeTransport {
    pub(crate) fn connect(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect(path)?;
            Ok(Self {
                stream: MpvPipeStream::Unix(stream),
                read_buffer: Vec::new(),
            })
        }

        #[cfg(windows)]
        {
            let stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            Ok(Self {
                stream: MpvPipeStream::Windows(stream),
                read_buffer: Vec::new(),
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mpv IPC transport not implemented for this platform",
            ))
        }
    }
}

impl MpvJsonIpcTransport for MpvPipeTransport {
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => write_line_to_stream(stream, line),
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => write_line_to_stream(stream, line),
        }
    }

    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => {
                read_line_from_stream(stream, &mut self.read_buffer, line)
            }
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => {
                read_line_from_stream(stream, &mut self.read_buffer, line)
            }
        }
    }
}

enum MpvPipeStream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Windows(std::fs::File),
}

fn write_line_to_stream(stream: &mut impl Write, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn decode_line_bytes(bytes: Vec<u8>, line: &mut String) -> io::Result<usize> {
    let decoded = String::from_utf8(bytes).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("mpv IPC response was not valid UTF-8: {err}"),
        )
    })?;
    line.clear();
    line.push_str(&decoded);
    Ok(line.len())
}

fn mpv_ipc_line_len(bytes: &[u8]) -> usize {
    let without_lf = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf).len()
}

fn mpv_ipc_line_too_long_error() -> String {
    format!("mpv IPC line too long: exceeded {MPV_IPC_MAX_LINE_BYTES} bytes")
}

fn validate_mpv_ipc_line_len(bytes: &[u8]) -> Result<(), String> {
    if mpv_ipc_line_len(bytes) > MPV_IPC_MAX_LINE_BYTES {
        Err(mpv_ipc_line_too_long_error())
    } else {
        Ok(())
    }
}

fn mpv_ipc_line_too_long_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, mpv_ipc_line_too_long_error())
}

pub(crate) fn read_line_from_stream(
    stream: &mut impl Read,
    read_buffer: &mut Vec<u8>,
    line: &mut String,
) -> io::Result<usize> {
    loop {
        if let Some(newline_index) = read_buffer.iter().position(|byte| *byte == b'\n') {
            if mpv_ipc_line_len(&read_buffer[..=newline_index]) > MPV_IPC_MAX_LINE_BYTES {
                return Err(mpv_ipc_line_too_long_io_error());
            }
            let remainder = read_buffer.split_off(newline_index + 1);
            let bytes = std::mem::replace(read_buffer, remainder);
            return decode_line_bytes(bytes, line);
        }

        let mut chunk = [0_u8; 8 * 1024];
        match stream.read(&mut chunk) {
            Ok(0) => {
                if read_buffer.is_empty() {
                    line.clear();
                    return Ok(0);
                }
                let bytes = std::mem::take(read_buffer);
                return decode_line_bytes(bytes, line);
            }
            Ok(bytes_read) => {
                read_buffer.extend_from_slice(&chunk[..bytes_read]);
                if !read_buffer.contains(&b'\n') && read_buffer.len() > MPV_IPC_MAX_LINE_BYTES {
                    return Err(mpv_ipc_line_too_long_io_error());
                }
            }
            Err(err) => return Err(err),
        }
    }
}
