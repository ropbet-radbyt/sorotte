use super::*;
use crate::redacted_debug::RedactedJsonValue;
use std::collections::BTreeSet;

pub const DEFAULT_MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

/// Maximum UTF-8 JSON payload, excluding LF or CRLF, accepted by current Rust
/// transports. Larger server output requires explicit recipient support.
pub const SOROTTE_MAX_PROTOCOL_LINE_BYTES: usize = 512 * 1024;
pub const SOROTTE_LARGE_PROTOCOL_FRAMES_V1: &str = "sorotteLargeProtocolFramesV1";
/// The pinned Python peer inherits Twisted LineReceiver's 16 KiB payload cap.
pub const LEGACY_MAX_PROTOCOL_LINE_BYTES: usize = 16 * 1024;

/// Measures encoded JSON without allocating a serialized copy, stopping as
/// soon as it exceeds the byte budget. Delimiters are not counted.
pub fn message_fits_line_limit(
    message: &ProtocolMessage,
    limit: usize,
) -> Result<bool, ProtocolError> {
    serialized_value_fits_line_limit(message, limit)
}

fn serialized_value_fits_line_limit(
    message: &impl serde::Serialize,
    limit: usize,
) -> Result<bool, ProtocolError> {
    struct Budget {
        remaining: usize,
        exceeded: bool,
    }
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.remaining {
                self.exceeded = true;
                return Err(std::io::Error::other("protocol frame exceeds byte budget"));
            }
            self.remaining -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut budget = Budget {
        remaining: limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut budget, message) {
        Ok(()) => Ok(true),
        Err(_) if budget.exceeded => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub struct DecodedMessageLineItem {
    pub command: Option<String>,
    pub payload: Value,
    pub message: Result<ProtocolMessage, ProtocolError>,
}

struct RedactedCommandName<'a>(&'a Option<String>);

impl std::fmt::Debug for RedactedCommandName<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.as_deref() {
            None => formatter.write_str("None"),
            Some(command)
                if matches!(
                    command,
                    "Hello" | "Set" | "List" | "State" | "Chat" | "Error" | "TLS"
                ) =>
            {
                formatter.debug_tuple("Some").field(&command).finish()
            }
            Some(_) => formatter
                .debug_tuple("Some")
                .field(&"<unknown-protocol-command>")
                .finish(),
        }
    }
}

impl std::fmt::Debug for DecodedMessageLineItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedMessageLineItem")
            .field("command", &RedactedCommandName(&self.command))
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

fn next_non_whitespace_index(bytes: &[u8], start: usize) -> usize {
    bytes
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, byte)| (!byte.is_ascii_whitespace()).then_some(index))
        .unwrap_or(bytes.len())
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
                if let (1, true) = (depth, expect_key) {
                    let after_string = next_non_whitespace_index(bytes, index + 1);
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
                if let (b'{', 1) = (*byte, depth) {
                    expect_key = true;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b',' => {
                if let 1 = depth {
                    expect_key = true;
                }
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
    let mut matched_span = None;

    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
                if let (1, true) = (depth, expect_key) {
                    let after_string = next_non_whitespace_index(bytes, index + 1);
                    if bytes.get(after_string) == Some(&b':') {
                        let raw_key = &json_line[string_start..index];
                        let quoted_key = format!("\"{raw_key}\"");
                        let key_matches = serde_json::from_str::<String>(&quoted_key)
                            .is_ok_and(|key| key == wanted_key);
                        if key_matches {
                            let value_start = next_non_whitespace_index(bytes, after_string + 1);
                            matched_span = matching_object_end(bytes, value_start)
                                .map(|value_end| (value_start, value_end));
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
                if let (b'{', 1) = (*byte, depth) {
                    expect_key = true;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b',' => {
                if let 1 = depth {
                    expect_key = true;
                }
            }
            _ => {}
        }
    }

    matched_span
}

fn set_command_order(json_line: &str) -> Vec<String> {
    let Some((start, end)) = top_level_object_value_span(json_line, "Set") else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    top_level_key_order(&json_line[start..end])
        .into_iter()
        .filter(|command| seen.insert(command.clone()))
        .collect()
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

#[cfg(test)]
mod tests {
    #[test]
    fn advertised_protocol_limits_accept_the_exact_payload_and_reject_one_more_byte() {
        let empty: super::ProtocolMessage =
            serde_json::from_value(serde_json::json!({"Chat": ""})).unwrap();
        let overhead = super::encode_message_line(&empty).unwrap().len();
        for (limit, expected_bytes) in [
            (super::SOROTTE_MAX_PROTOCOL_LINE_BYTES, 524_288),
            (super::LEGACY_MAX_PROTOCOL_LINE_BYTES, 16_384),
        ] {
            for extra in [0, 1] {
                let message: super::ProtocolMessage = serde_json::from_value(
                    serde_json::json!({"Chat": "x".repeat(expected_bytes - overhead + extra)}),
                )
                .unwrap();
                assert_eq!(
                    super::encode_message_line(&message).unwrap().len(),
                    expected_bytes + extra
                );
                assert_eq!(
                    super::message_fits_line_limit(&message, limit).unwrap(),
                    extra == 0
                );
            }
        }
    }

    #[test]
    fn frame_budget_preserves_serialization_errors_that_are_not_capacity_failures() {
        struct Rejected;
        impl serde::Serialize for Rejected {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("rejected serialization fixture"))
            }
        }
        let error = super::serialized_value_fits_line_limit(&Rejected, 1024).unwrap_err();
        assert!(matches!(error, super::ProtocolError::InvalidJson(error) if error.is_data()));
    }

    #[test]
    fn encoded_frame_budget_tracks_utf8_and_json_escaping_at_exact_boundary() {
        for atom in ["a", "界", "🙂", "\n", "\"", "\\", "\u{0001}"] {
            for repetitions in [1, 7, 100, 10_000] {
                let message = super::decode_message_line(
                    &serde_json::json!({"Chat":atom.repeat(repetitions)}).to_string(),
                )
                .unwrap();
                let encoded = super::encode_message_line(&message).unwrap();
                assert!(super::message_fits_line_limit(&message, encoded.len()).unwrap());
                assert!(!super::message_fits_line_limit(&message, encoded.len() - 1).unwrap());
                for delimiter in ["\n", "\r\n"] {
                    let decoded =
                        super::decode_message_line(&format!("{encoded}{delimiter}")).unwrap();
                    assert!(super::message_fits_line_limit(&decoded, encoded.len()).unwrap());
                }
            }
        }
    }

    use std::error::Error as _;

    use super::*;

    #[test]
    fn default_protocol_line_limit_is_exactly_sixty_four_kibibytes() {
        assert_eq!(DEFAULT_MAX_PROTOCOL_LINE_BYTES, 65_536);
    }

    #[test]
    fn protocol_error_debug_and_source_preserve_safe_variant_information() {
        let invalid_json = decode_line("{").expect_err("truncated object should be invalid");
        assert!(format!("{invalid_json:?}").starts_with("InvalidJson("));
        assert!(invalid_json.source().is_some());

        let unexpected = ProtocolError::UnexpectedMessageKind {
            expected: "Hello",
            found: "Set",
        };
        assert_eq!(
            format!("{unexpected:?}"),
            "UnexpectedMessageKind { expected: \"Hello\", found: \"Set\" }"
        );
        assert!(unexpected.source().is_none());

        let server = ProtocolError::ServerError {
            message: "secret".to_owned(),
        };
        assert_eq!(format!("{server:?}"), "ServerError { message_bytes: 6 }");
        assert!(server.source().is_none());

        let tls = ProtocolError::UnexpectedTlsMessage {
            start_tls: "false".to_owned(),
        };
        assert_eq!(
            format!("{tls:?}"),
            "UnexpectedTlsMessage { start_tls: \"false\" }"
        );
        assert!(tls.source().is_none());
    }

    #[test]
    fn whitespace_scanner_returns_the_next_bounded_index() {
        let bytes = b" \t\r\nx  ";
        assert_eq!(next_non_whitespace_index(bytes, 0), 4);
        assert_eq!(next_non_whitespace_index(bytes, 2), 4);
        assert_eq!(next_non_whitespace_index(bytes, 4), 4);
        assert_eq!(next_non_whitespace_index(bytes, 5), bytes.len());
        assert_eq!(
            next_non_whitespace_index(bytes, bytes.len() + 5),
            bytes.len()
        );
    }

    #[test]
    fn top_level_key_scanner_ignores_nested_and_string_lookalikes() {
        let line = r#"{
            "first" : 1,
            "nested": {"inside": 2, "other": {"deep": 3}},
            "array": [{"hidden": 4}, {"alsoHidden": 5}],
            "last": "text, } \"fake\": {"
        }"#;

        assert_eq!(
            top_level_key_order(line),
            ["first", "nested", "array", "last"]
        );
        assert!(top_level_key_order(r#"[{"notTopLevel": true}]"#).is_empty());
        assert!(top_level_key_order("null").is_empty());
    }

    #[test]
    fn top_level_key_scanner_decodes_escaped_keys_and_whitespace() {
        assert_eq!(
            top_level_key_order(
                "{ \"\\u0066irst\" \t:\n 1,\r\n \"se\\u0063ond\" : {\"nested\": 2} }"
            ),
            ["first", "second"]
        );
    }

    #[test]
    fn matching_object_end_is_exclusive_and_string_aware() {
        let object = br#"{"text":"} { \" quoted","nested":{"value":1}}"#;
        let mut bytes = b"xx".to_vec();
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"tail");

        assert_eq!(matching_object_end(&bytes, 2), Some(2 + object.len()));
        assert_eq!(matching_object_end(b"xx{}tail", 2), Some(4));
        assert_eq!(matching_object_end(b"xx[]tail", 2), None);
        assert_eq!(matching_object_end(b"xx{\"open\":{", 2), None);
        assert_eq!(matching_object_end(b"{}", 3), None);
    }

    #[test]
    fn top_level_object_span_selects_only_the_requested_top_level_object() {
        let wanted = r#"{"room":{"name":"a,}"},"file":{"name":"movie.mkv"}}"#;
        let line = format!(
            r#"{{"before":{{"Set":{{"shadow":true}}}},"\u0053et" :  {wanted},"after":{{}}}}"#
        );
        let expected_start = line.find(wanted).expect("fixture contains wanted object");
        let expected_end = expected_start + wanted.len();
        let actual_span =
            top_level_object_value_span(&line, "Set").expect("Set object should have a span");

        assert_eq!(actual_span, (expected_start, expected_end));
        assert_eq!(&line[actual_span.0..actual_span.1], wanted);
    }

    #[test]
    fn top_level_object_span_rejects_nested_non_object_and_missing_values() {
        assert_eq!(
            top_level_object_value_span(r#"{"outer":{"Set":{"shadow":true}}}"#, "Set"),
            None
        );
        assert_eq!(
            top_level_object_value_span(r#"{"before":{},"Set":null}"#, "Set"),
            None
        );
        assert_eq!(
            top_level_object_value_span(r#"{"before":{},"after":{}}"#, "Set"),
            None
        );
    }

    #[test]
    fn set_command_scanner_preserves_only_direct_set_member_order() {
        let line = r#"{
            "before": {"Set": {"shadow": true}},
            "Set" : {
                "room": {"name": "room, }"},
                "file": {"name": "\"playlistIndex\": {}"},
                "playlistIndex": {"index": 2}
            },
            "after": {}
        }"#;

        assert_eq!(set_command_order(line), ["room", "file", "playlistIndex"]);
        assert!(set_command_order(r#"{"before":{"Set":{"shadow":true}}}"#).is_empty());
    }
}
