use super::*;
use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid JSON payload: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unexpected message kind: expected '{expected}', found '{found}'")]
    UnexpectedMessageKind {
        expected: &'static str,
        found: &'static str,
    },
    #[error("server error: {message}")]
    ServerError { message: String },
    #[error("unexpected TLS negotiation frame: startTLS='{start_tls}'")]
    UnexpectedTlsMessage { start_tls: String },
}

pub fn decode_line(line: &str) -> Result<Value, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::from)
}

pub fn encode_line(value: &Value) -> Result<String, ProtocolError> {
    serde_json::to_string(value).map_err(ProtocolError::from)
}

fn normalize_legacy_message_variants(value: &mut Value) {
    if let Some(is_ready) = value.pointer_mut("/Set/ready/isReady")
        && is_ready.is_null()
    {
        *is_ready = Value::Bool(false);
    }

    if value
        .pointer("/Set/playlistIndex/index")
        .is_some_and(Value::is_null)
        && let Some(set_payload) = value.get_mut("Set").and_then(Value::as_object_mut)
    {
        set_payload.remove("playlistIndex");
    }
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

pub fn decode_message_lines(line: &str) -> Result<Vec<ProtocolMessage>, ProtocolError> {
    let mut value = decode_line(line)?;
    normalize_legacy_message_variants(&mut value);
    let Some(object) = value.as_object() else {
        let message = serde_json::from_value(value).map_err(ProtocolError::from)?;
        return Ok(vec![message]);
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
        let message = serde_json::from_value(value).map_err(ProtocolError::from)?;
        return Ok(vec![message]);
    }

    let mut messages = Vec::with_capacity(command_keys.len());
    for command_key in command_keys {
        let Some(payload) = object.get(&command_key).cloned() else {
            continue;
        };
        let mut command_object = serde_json::Map::new();
        command_object.insert(command_key, payload);
        messages.push(
            serde_json::from_value(Value::Object(command_object)).map_err(ProtocolError::from)?,
        );
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
