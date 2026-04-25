use super::*;

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

pub fn decode_message_line(line: &str) -> Result<ProtocolMessage, ProtocolError> {
    let mut value = decode_line(line)?;
    normalize_legacy_message_variants(&mut value);
    serde_json::from_value(value).map_err(ProtocolError::from)
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
