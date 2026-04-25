use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    path::Path,
};

use serde_json::{Value, json};

use crate::constants::{
    MPV_COMMAND_GET_PROPERTY, MPV_COMMAND_OBSERVE_PROPERTY, MPV_RESPONSE_SUCCESS,
};

pub(crate) trait MpvJsonIpcTransport: Send + Sync {
    fn send_line(&mut self, line: &str) -> io::Result<()>;
    fn read_line(&mut self, line: &mut String) -> io::Result<usize>;
}

pub(crate) struct MpvJsonIpcClient {
    transport: Box<dyn MpvJsonIpcTransport>,
    next_request_id: u64,
    pending_events: VecDeque<Value>,
}

impl MpvJsonIpcClient {
    pub(crate) fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self {
            transport,
            next_request_id: 1,
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
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        let request = json!({
            "command": command,
            "request_id": request_id,
        });
        let mut line = serde_json::to_string(&request)
            .map_err(|err| format!("failed to serialize mpv IPC request: {err}"))?;
        line.push('\n');
        self.transport
            .send_line(&line)
            .map_err(|err| format!("failed to write mpv IPC request: {err}"))?;

        let mut response_line = String::new();
        loop {
            let bytes_read = self
                .transport
                .read_line(&mut response_line)
                .map_err(|err| format!("failed to read mpv IPC response: {err}"))?;
            if bytes_read == 0 {
                return Err(format!(
                    "unexpected EOF while waiting for mpv IPC response (request_id={request_id})"
                ));
            }

            let trimmed = response_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }

            let parsed: Value = serde_json::from_str(trimmed)
                .map_err(|err| format!("invalid mpv IPC JSON line '{trimmed}': {err}"))?;
            if parsed.get("event").and_then(Value::as_str).is_some() {
                self.pending_events.push_back(parsed);
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
                return Err(format!(
                    "mpv command failed for request_id={request_id}: {error}"
                ));
            }

            return Ok(parsed);
        }
    }

    pub(crate) fn take_pending_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
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
            return Ok(Self {
                stream: MpvPipeStream::Unix(stream),
                read_buffer: Vec::new(),
            });
        }

        #[cfg(windows)]
        {
            let stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            return Ok(Self {
                stream: MpvPipeStream::Windows(stream),
                read_buffer: Vec::new(),
            });
        }

        #[allow(unreachable_code)]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mpv IPC transport not implemented for this platform",
        ))
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

pub(crate) fn read_line_from_stream(
    stream: &mut impl Read,
    read_buffer: &mut Vec<u8>,
    line: &mut String,
) -> io::Result<usize> {
    loop {
        if let Some(newline_index) = read_buffer.iter().position(|byte| *byte == b'\n') {
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
            Ok(bytes_read) => read_buffer.extend_from_slice(&chunk[..bytes_read]),
            Err(err) => return Err(err),
        }
    }
}
