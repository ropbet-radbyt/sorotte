#![no_main]

use std::collections::BTreeSet;
use std::fmt;

use libfuzzer_sys::fuzz_target;
use serde::de::{Deserializer as _, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use sorotte_protocol::{
    DEFAULT_MAX_PROTOCOL_LINE_BYTES, ProtocolMessage, decode_line, decode_message_line,
    decode_message_line_items, decode_message_lines, encode_line, encode_message_line,
};

struct SourceOrderVisitor;

impl<'de> Visitor<'de> for SourceOrderVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a top-level JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            keys.push(key);
            map.next_value::<IgnoredAny>()?;
        }
        Ok(keys)
    }
}

fn unique_source_key_order(line: &str) -> serde_json::Result<Vec<String>> {
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let keys = deserializer.deserialize_map(SourceOrderVisitor)?;
    deserializer.end()?;

    let mut seen = BTreeSet::new();
    Ok(keys
        .into_iter()
        .filter(|key| seen.insert(key.clone()))
        .collect())
}

fn matches_tc_protocol_004(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            if left == right {
                return true;
            }
            let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) else {
                return false;
            };
            left.is_finite()
                && right.is_finite()
                && left.is_sign_negative() == right.is_sign_negative()
                && left.to_bits().abs_diff(right.to_bits()) == 1
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| matches_tc_protocol_004(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| matches_tc_protocol_004(left, right))
                })
        }
        _ => left == right,
    }
}

fn assert_typed_roundtrip(message: &ProtocolMessage) {
    let encoded = encode_message_line(message).expect("typed protocol messages must serialize");
    let decoded =
        decode_message_line(&encoded).expect("serialized typed protocol messages must decode");
    if &decoded != message {
        let before =
            serde_json::to_value(message).expect("decoded typed messages must serialize to JSON");
        let after =
            serde_json::to_value(&decoded).expect("roundtripped messages must serialize to JSON");
        assert!(
            matches_tc_protocol_004(&before, &after),
            "typed protocol encode/decode drifted outside registered TC-PROTOCOL-004"
        );
    }
}

fn exercise_public_protocol_boundary(line: &str) {
    let raw = decode_line(line);
    let items = decode_message_line_items(line);
    let messages = decode_message_lines(line);
    let first = decode_message_line(line);

    assert_eq!(
        raw.is_ok(),
        items.is_ok(),
        "raw and diagnostic decoders must agree on JSON syntax"
    );

    let Ok(value) = raw else {
        assert!(messages.is_err(), "invalid JSON cannot aggregate-decode");
        assert!(first.is_err(), "invalid JSON cannot singular-decode");
        return;
    };

    let items = items.expect("syntactically valid JSON must produce diagnostics");
    assert!(
        !items.is_empty(),
        "syntactically valid JSON must produce at least one diagnostic item"
    );

    let encoded_value = encode_line(&value).expect("decoded JSON values must serialize");
    let decoded_value =
        decode_line(&encoded_value).expect("serialized JSON values must decode");
    assert!(
        matches_tc_protocol_004(&value, &decoded_value),
        "raw JSON encode/decode drifted outside registered TC-PROTOCOL-004"
    );

    if let Some(object) = value.as_object() {
        let expected_order =
            unique_source_key_order(line).expect("independent source-order oracle must parse");
        let actual_order = items
            .iter()
            .filter_map(|item| item.command.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            actual_order, expected_order,
            "diagnostic items must preserve unique top-level source order"
        );

        for item in &items {
            if let Some(command) = &item.command {
                assert_eq!(
                    object.get(command),
                    Some(&item.payload),
                    "duplicate commands must expose serde_json's surviving value"
                );
            }
        }
    } else {
        assert_eq!(items.len(), 1, "non-object JSON has one diagnostic item");
        assert_eq!(items[0].command, None);
        assert_eq!(items[0].payload, value);
    }

    let all_items_typed = items.iter().all(|item| item.message.is_ok());
    assert_eq!(
        messages.is_ok(),
        all_items_typed,
        "aggregate decoding must be strict across every diagnostic item"
    );
    assert_eq!(
        first.is_ok(),
        all_items_typed,
        "singular decoding must retain aggregate-strict failure behavior"
    );

    if let Ok(messages) = messages {
        assert_eq!(
            messages.len(),
            items.len(),
            "aggregate decoding must retain every diagnostic item"
        );
        assert_eq!(
            first
                .as_ref()
                .expect("all diagnostic items decoded")
                .kind(),
            messages
                .first()
                .expect("valid aggregate cannot be empty")
                .kind()
        );
        assert_eq!(
            messages
                .iter()
                .map(ProtocolMessage::kind)
                .collect::<Vec<_>>(),
            items
                .iter()
                .map(|item| {
                    item.message
                        .as_ref()
                        .expect("all diagnostic items decoded")
                        .kind()
                })
                .collect::<Vec<_>>()
        );
    }

    for item in items {
        if let Ok(message) = item.message {
            assert_typed_roundtrip(&message);
        }
    }
}

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > DEFAULT_MAX_PROTOCOL_LINE_BYTES {
        return;
    }
    if let Ok(line) = std::str::from_utf8(bytes) {
        exercise_public_protocol_boundary(line);
    }
});
