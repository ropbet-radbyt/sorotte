use std::{collections::VecDeque, io, time::Instant};

use serde_json::{Value, json};

use crate::ipc::{MpvJsonIpcClient, MpvJsonIpcTransport};

pub(crate) fn unacknowledging_syncplayintf_client() -> MpvJsonIpcClient {
    MpvJsonIpcClient::new(Box::new(SuccessfulNoAckTransport::default()))
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
