use super::*;
use crate::redacted_debug::RedactedJsonValue;
use std::collections::BTreeSet;

pub const DEFAULT_MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub struct DecodedMessageLineItem {
    pub command: Option<String>,
    pub payload: Value,
    pub message: Result<ProtocolMessage, ProtocolError>,
}

impl std::fmt::Debug for DecodedMessageLineItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedMessageLineItem")
            .field("command", &self.command)
            .field("payload", &RedactedJsonValue(&self.payload))
            .field("message", &self.message)
            .finish()
    }
}

pub enum ProtocolError {
    InvalidJson(serde_json::Error),
    UnexpectedMessageKind {
        expected: &'static str,
        found: &'static str,
    },
    ServerError {
        message: String,
    },
    UnexpectedTlsMessage {
        start_tls: String,
    },
}

impl std::fmt::Debug for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => formatter.debug_tuple("InvalidJson").field(error).finish(),
            Self::UnexpectedMessageKind { expected, found } => formatter
                .debug_struct("UnexpectedMessageKind")
                .field("expected", expected)
                .field("found", found)
                .finish(),
            Self::ServerError { message } => formatter
                .debug_struct("ServerError")
                .field("message_bytes", &message.len())
                .finish(),
            Self::UnexpectedTlsMessage { start_tls } => formatter
                .debug_struct("UnexpectedTlsMessage")
                .field("start_tls", start_tls)
                .finish(),
        }
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid JSON payload: {error}"),
            Self::UnexpectedMessageKind { expected, found } => write!(
                formatter,
                "unexpected message kind: expected '{expected}', found '{found}'"
            ),
            // Server error text is untrusted and may contain a reflected raw
            // protocol line. Keep the original value available through the
            // typed variant, but never render it through the ordinary error
            // formatting boundary.
            Self::ServerError { .. } => write!(
                formatter,
                "server error: {}",
                sorotte_secret::REDACTED_SECRET
            ),
            Self::UnexpectedTlsMessage { start_tls } => write!(
                formatter,
                "unexpected TLS negotiation frame: startTLS='{start_tls}'"
            ),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::UnexpectedMessageKind { .. }
            | Self::ServerError { .. }
            | Self::UnexpectedTlsMessage { .. } => None,
        }
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidJson(value)
    }
}

pub fn decode_line(line: &str) -> Result<Value, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::from)
}

pub fn encode_line(value: &Value) -> Result<String, ProtocolError> {
    serde_json::to_string(value).map_err(ProtocolError::from)
}

fn top_level_key_order(json_line: &str) -> Vec<String> {
    let bytes = json_line.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut expect_key = false;
    let mut string_start = 0usize;

    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
                if depth == 1 && expect_key {
                    let mut after_string = index + 1;
                    while bytes.get(after_string).is_some_and(u8::is_ascii_whitespace) {
                        after_string += 1;
                    }
                    if bytes.get(after_string) == Some(&b':') {
                        let raw_key = &json_line[string_start..index];
                        let quoted_key = format!("\"{raw_key}\"");
                        if let Ok(key) = serde_json::from_str::<String>(&quoted_key) {
                            keys.push(key);
                        }
                        expect_key = false;
                    }
                }
            }
            continue;
        }

        match *byte {
            b'"' => {
                in_string = true;
                escaped = false;
                string_start = index + 1;
            }
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if *byte == b'{' && depth == 1 {
                    expect_key = true;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b',' if depth == 1 => {
                expect_key = true;
            }
            _ => {}
        }
    }

    keys
}

fn matching_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'{') {
        return None;
    }

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match *byte {
            b'"' => {
                in_string = true;
                escaped = false;
            }
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn top_level_object_value_span(json_line: &str, wanted_key: &str) -> Option<(usize, usize)> {
    let bytes = json_line.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut expect_key = false;
    let mut string_start = 0usize;

    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
                if depth == 1 && expect_key {
                    let mut after_string = index + 1;
                    while bytes.get(after_string).is_some_and(u8::is_ascii_whitespace) {
                        after_string += 1;
                    }
                    if bytes.get(after_string) == Some(&b':') {
                        let raw_key = &json_line[string_start..index];
                        let quoted_key = format!("\"{raw_key}\"");
                        if let Ok(key) = serde_json::from_str::<String>(&quoted_key)
                            && key == wanted_key
                        {
                            let mut value_start = after_string + 1;
                            while bytes.get(value_start).is_some_and(u8::is_ascii_whitespace) {
                                value_start += 1;
                            }
                            let value_end = matching_object_end(bytes, value_start)?;
                            return Some((value_start, value_end));
                        }
                        expect_key = false;
                    }
                }
            }
            continue;
        }

        match *byte {
            b'"' => {
                in_string = true;
                escaped = false;
                string_start = index + 1;
            }
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if *byte == b'{' && depth == 1 {
                    expect_key = true;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b',' if depth == 1 => {
                expect_key = true;
            }
            _ => {}
        }
    }

    None
}

fn set_command_order(json_line: &str) -> Vec<String> {
    let Some((start, end)) = top_level_object_value_span(json_line, "Set") else {
        return Vec::new();
    };
    top_level_key_order(&json_line[start..end])
}

fn decode_protocol_message_with_command_order(
    value: Value,
    original_line: &str,
) -> Result<ProtocolMessage, ProtocolError> {
    let mut message: ProtocolMessage =
        serde_json::from_value(value).map_err(ProtocolError::from)?;
    if let ProtocolMessage::Set(set_message) = &mut message {
        set_message.set.command_order = set_command_order(original_line);
    }
    Ok(message)
}

pub fn decode_message_lines(line: &str) -> Result<Vec<ProtocolMessage>, ProtocolError> {
    decode_message_line_items(line)?
        .into_iter()
        .map(|item| item.message)
        .collect()
}

pub fn decode_message_line_items(line: &str) -> Result<Vec<DecodedMessageLineItem>, ProtocolError> {
    let value = decode_line(line)?;
    let Some(object) = value.as_object() else {
        let message = decode_protocol_message_with_command_order(value.clone(), line);
        return Ok(vec![DecodedMessageLineItem {
            command: None,
            payload: value,
            message,
        }]);
    };

    let mut command_keys = Vec::new();
    let mut seen = BTreeSet::new();
    for key in top_level_key_order(line) {
        if object.contains_key(&key) && seen.insert(key.clone()) {
            command_keys.push(key);
        }
    }
    for key in object.keys() {
        if seen.insert(key.clone()) {
            command_keys.push(key.clone());
        }
    }

    if command_keys.len() <= 1 {
        let command = command_keys.first().cloned();
        let payload = command
            .as_ref()
            .and_then(|command| object.get(command))
            .cloned()
            .unwrap_or_else(|| value.clone());
        let message = decode_protocol_message_with_command_order(value, line);
        return Ok(vec![DecodedMessageLineItem {
            command,
            payload,
            message,
        }]);
    }

    let mut messages = Vec::with_capacity(command_keys.len());
    for command_key in command_keys {
        let Some(payload) = object.get(&command_key).cloned() else {
            continue;
        };
        let mut command_object = serde_json::Map::new();
        command_object.insert(command_key.clone(), payload.clone());
        let message =
            decode_protocol_message_with_command_order(Value::Object(command_object), line);
        messages.push(DecodedMessageLineItem {
            command: Some(command_key),
            payload,
            message,
        });
    }
    Ok(messages)
}

pub fn decode_message_line(line: &str) -> Result<ProtocolMessage, ProtocolError> {
    let mut messages = decode_message_lines(line)?;
    Ok(messages.remove(0))
}

pub fn encode_message_line(message: &ProtocolMessage) -> Result<String, ProtocolError> {
    serde_json::to_string(message).map_err(ProtocolError::from)
}

pub fn extract_hello(value: &Value) -> Result<HelloPayload, ProtocolError> {
    let message: ProtocolMessage = serde_json::from_value(value.clone())?;
    match message {
        ProtocolMessage::Hello(hello) => Ok(hello.hello),
        other => Err(ProtocolError::UnexpectedMessageKind {
            expected: "Hello",
            found: other.kind(),
        }),
    }
}

pub fn extract_hello_from_message(message: ProtocolMessage) -> Result<HelloPayload, ProtocolError> {
    match message {
        ProtocolMessage::Hello(hello) => Ok(hello.hello),
        other => Err(ProtocolError::UnexpectedMessageKind {
            expected: "Hello",
            found: other.kind(),
        }),
    }
}
