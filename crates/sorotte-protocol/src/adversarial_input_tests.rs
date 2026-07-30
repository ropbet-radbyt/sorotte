//! Exhaustive raw-wire boundary checks which complement the generated DTO
//! properties in `property_tests`.
//!
//! The public codec accepts `&str`, so malformed UTF-8 is rejected by the
//! caller's byte-to-text boundary. These tests make that boundary explicit,
//! then drive every representable adversarial input through the production
//! decode entrypoints.

use serde_json::json;

use super::*;

fn wire_command(message: &ProtocolMessage) -> &'static str {
    match message {
        ProtocolMessage::Hello(_) => "Hello",
        ProtocolMessage::Set(_) => "Set",
        ProtocolMessage::List(_) => "List",
        ProtocolMessage::State(_) => "State",
        ProtocolMessage::Chat(_) => "Chat",
        ProtocolMessage::Error(_) => "Error",
        ProtocolMessage::Tls(_) => "TLS",
    }
}

fn supported_envelope_samples() -> Vec<ProtocolMessage> {
    vec![
        ProtocolMessage::hello(
            HelloPayload::new("雪🔐", "room", "1.2.255")
                .with_features(json!({"quoted": "\"\\\n", "emoji": "🦀"})),
        ),
        ProtocolMessage::set(SetPayload::new()),
        ProtocolMessage::list_request(),
        ProtocolMessage::state(StatePayload::new()),
        ProtocolMessage::chat_text("hello"),
        ProtocolMessage::error(ErrorPayload::new("failure")),
        ProtocolMessage::tls(TlsPayload::new("true")),
    ]
}

fn json_key_with_escape_mask(key: &str, escape_mask: usize) -> String {
    debug_assert!(key.is_ascii());
    let mut encoded = String::from("\"");
    for (index, byte) in key.bytes().enumerate() {
        if escape_mask & (1 << index) == 0 {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("\\u{byte:04x}"));
        }
    }
    encoded.push('"');
    encoded
}

fn invalid_json_signature(error: ProtocolError) -> (String, String) {
    assert!(
        matches!(&error, ProtocolError::InvalidJson(_)),
        "raw syntax failures must retain the InvalidJson variant, found {error:?}"
    );
    (format!("{error:?}"), error.to_string())
}

fn assert_invalid_json_at_every_entrypoint(line: &str) -> (String, String) {
    let raw = invalid_json_signature(decode_line(line).expect_err("raw JSON must be rejected"));
    let items = invalid_json_signature(
        decode_message_line_items(line).expect_err("item decoding must reject invalid JSON"),
    );
    let messages = invalid_json_signature(
        decode_message_lines(line).expect_err("aggregate decoding must reject invalid JSON"),
    );
    let first = invalid_json_signature(
        decode_message_line(line).expect_err("single decoding must reject invalid JSON"),
    );

    assert_eq!(items, raw);
    assert_eq!(messages, raw);
    assert_eq!(first, raw);
    raw
}

fn lossy_wire(invalid_sequence: &[u8]) -> Vec<u8> {
    let mut wire = br#"{"Chat":"before-"#.to_vec();
    wire.extend_from_slice(invalid_sequence);
    wire.extend_from_slice(br#"-after"}"#);
    wire
}

#[test]
fn every_mixed_command_key_escape_spelling_has_one_canonical_meaning() {
    let mut spellings = 0usize;

    for expected in supported_envelope_samples() {
        let command = wire_command(&expected);
        let expected_value =
            serde_json::to_value(&expected).expect("sample message should serialize");
        let payload = expected_value
            .as_object()
            .and_then(|object| object.get(command))
            .expect("sample envelope must contain its command");
        let payload = serde_json::to_string(payload).expect("sample payload should serialize");
        let canonical =
            encode_message_line(&expected).expect("sample message should canonically encode");

        for escape_mask in 0..(1usize << command.len()) {
            spellings += 1;
            let encoded_key = json_key_with_escape_mask(command, escape_mask);
            let line = format!("{{{encoded_key}:{payload}}}");

            assert_eq!(
                decode_line(&line).expect("escaped-key JSON should decode"),
                expected_value,
                "{command} mask {escape_mask:#b}"
            );
            let items = decode_message_line_items(&line)
                .expect("escaped-key envelope should decode into an item");
            assert_eq!(items.len(), 1, "{command} mask {escape_mask:#b}");
            assert_eq!(
                items[0].command.as_deref(),
                Some(command),
                "{command} mask {escape_mask:#b}"
            );
            assert_eq!(
                items[0]
                    .message
                    .as_ref()
                    .expect("escaped-key message should type-check"),
                &expected,
                "{command} mask {escape_mask:#b}"
            );

            let decoded =
                decode_message_line(&line).expect("escaped-key message should type-check");
            assert_eq!(decoded, expected, "{command} mask {escape_mask:#b}");
            assert_eq!(
                encode_message_line(&decoded).expect("decoded message should re-encode"),
                canonical,
                "{command} mask {escape_mask:#b}"
            );
        }
    }

    assert_eq!(
        spellings, 144,
        "the exhaustive mask count guards accidental generator weakening"
    );
}

#[test]
fn every_character_boundary_truncation_is_rejected_consistently_and_without_reflection() {
    const MARKER: &str = "truncation-access-token-canary-7f92";
    let line = encode_message_line(&ProtocolMessage::hello(
        HelloPayload::new("雪🔐", "room", "1.2.255").with_features(json!({
            "diagnostic": format!("access_token={MARKER}"),
            "escaped": "\"quoted\\value\n",
        })),
    ))
    .expect("adversarial Hello sample should encode");
    assert!(decode_message_line(&line).is_ok());

    let mut character_boundaries = 0usize;
    let mut non_utf8_boundaries = 0usize;
    for offset in 0..line.len() {
        if line.is_char_boundary(offset) {
            character_boundaries += 1;
            let signature = assert_invalid_json_at_every_entrypoint(&line[..offset]);
            assert!(
                !signature.0.contains(MARKER) && !signature.1.contains(MARKER),
                "syntax diagnostics must not reflect credential-bearing input at offset {offset}"
            );
        } else {
            non_utf8_boundaries += 1;
            assert!(
                std::str::from_utf8(&line.as_bytes()[..offset]).is_err(),
                "a cut inside a scalar must fail before the &str codec boundary"
            );
        }
    }

    assert!(character_boundaries > 100);
    assert!(
        non_utf8_boundaries >= 4,
        "the fixture must retain multibyte UTF-8 cut points"
    );
}

#[test]
fn malformed_utf8_classes_fail_before_the_str_codec_and_lossy_text_remains_total() {
    let malformed_sequences: &[&[u8]] = &[
        &[0x80],
        &[0xc0, 0xaf],
        &[0xed, 0xa0, 0x80],
        &[0xf4, 0x90, 0x80, 0x80],
        &[0xc2],
        &[0xe2, 0x82],
        &[0xf0, 0x9f, 0x92],
    ];

    for invalid_sequence in malformed_sequences {
        let wire = lossy_wire(invalid_sequence);
        assert!(
            std::str::from_utf8(&wire).is_err(),
            "{invalid_sequence:02x?} must not cross the public &str boundary"
        );

        let normalized = String::from_utf8_lossy(&wire);
        let value = decode_line(&normalized).expect("lossy-normalized wire should be valid JSON");
        let items = decode_message_line_items(&normalized)
            .expect("lossy-normalized wire should decode into an item");
        let messages = decode_message_lines(&normalized)
            .expect("lossy-normalized Chat should decode as an aggregate");
        let first = decode_message_line(&normalized)
            .expect("lossy-normalized Chat should decode as one message");

        assert!(normalized.contains('\u{fffd}'));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].command.as_deref(), Some("Chat"));
        assert_eq!(messages.as_slice(), std::slice::from_ref(&first));
        assert_eq!(
            decode_line(&encode_line(&value).expect("normalized JSON should re-encode"))
                .expect("normalized JSON should roundtrip"),
            value
        );
    }
}

#[test]
fn malformed_string_escape_matrix_is_rejected_at_the_syntax_boundary() {
    let malformed_escapes = [
        r"\x20",
        r"\u",
        r"\u0",
        r"\u00",
        r"\u000",
        r"\u00xz",
        r"\uD800",
        r"\uDC00",
        r"\uD800\u0041",
        r"\uD800\uD800",
    ];

    for malformed_escape in malformed_escapes {
        let line = format!(r#"{{"Chat":"{malformed_escape}"}}"#);
        assert_invalid_json_at_every_entrypoint(&line);
    }

    for control in 0u8..=0x1f {
        let mut line = br#"{"Chat":"before"#.to_vec();
        line.push(control);
        line.extend_from_slice(br#"after"}"#);
        let line = std::str::from_utf8(&line).expect("ASCII controls are valid UTF-8");
        assert_invalid_json_at_every_entrypoint(line);
    }
}

#[test]
fn nested_extension_depth_has_one_monotonic_public_decode_boundary() {
    const PREFIX: &str =
        r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1","features":"#;
    const SUFFIX: &str = "}}";
    const MAX_DEPTH: usize = 256;

    let mut accepted = Vec::with_capacity(MAX_DEPTH + 1);
    for depth in 0..=MAX_DEPTH {
        let line = format!(
            "{PREFIX}{}null{}{SUFFIX}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let raw = decode_line(&line);
        let items = decode_message_line_items(&line);
        let messages = decode_message_lines(&line);
        let first = decode_message_line(&line);
        let raw_ok = raw.is_ok();

        assert_eq!(
            items.is_ok(),
            raw_ok,
            "item decode boundary diverged at depth {depth}"
        );
        assert_eq!(
            messages.is_ok(),
            raw_ok,
            "aggregate decode boundary diverged at depth {depth}"
        );
        assert_eq!(
            first.is_ok(),
            raw_ok,
            "single decode boundary diverged at depth {depth}"
        );
        if !raw_ok {
            assert!(matches!(raw, Err(ProtocolError::InvalidJson(_))));
            assert!(matches!(items, Err(ProtocolError::InvalidJson(_))));
            assert!(matches!(messages, Err(ProtocolError::InvalidJson(_))));
            assert!(matches!(first, Err(ProtocolError::InvalidJson(_))));
        }
        accepted.push(raw_ok);
    }

    let first_rejected = accepted
        .iter()
        .position(|is_accepted| !is_accepted)
        .expect("the parser recursion guard must reject within the bounded sweep");
    assert!(first_rejected > 0, "shallow Hello input must remain valid");
    assert!(
        accepted[..first_rejected]
            .iter()
            .all(|is_accepted| *is_accepted)
    );
    assert!(
        accepted[first_rejected..]
            .iter()
            .all(|is_accepted| !*is_accepted),
        "deeper inputs must not become valid again after the recursion boundary"
    );
}

#[test]
#[should_panic(
    expected = "surviving duplicate Set payload must determine nested command execution order"
)]
fn known_defect_duplicate_top_level_set_uses_discarded_payload_order() {
    let line = r#"{
        "Set":{
            "ready":{"isReady":true},
            "file":{"name":"discarded.mkv"}
        },
        "\u0053et":{
            "playlistIndex":{"index":9},
            "room":{"name":"surviving-room"}
        }
    }"#;

    let ProtocolMessage::Set(message) =
        decode_message_line(line).expect("duplicate Set envelope should decode")
    else {
        panic!("expected Set message");
    };
    assert!(message.set.ready.is_none());
    assert!(message.set.file.is_none());
    assert!(message.set.playlist_index.is_some());
    assert_eq!(
        message.set.room.as_ref().map(|room| room.name.as_str()),
        Some("surviving-room")
    );
    assert_eq!(
        message.set.command_order,
        ["playlistIndex", "room"],
        "surviving duplicate Set payload must determine nested command execution order"
    );
}

#[test]
#[should_panic(expected = "credential-bearing unknown command must not appear in diagnostics")]
fn known_defect_decoded_item_debug_exposes_credential_bearing_unknown_command() {
    const MARKER: &str = "unknown-command-token-canary-2a71";
    let line = format!(r#"{{"Future?access_token={MARKER}":null}}"#);
    let items =
        decode_message_line_items(&line).expect("unknown command should remain an item error");
    let debug = format!("{:?}", items.first().expect("one item should be retained"));

    assert!(
        !debug.contains(MARKER),
        "credential-bearing unknown command must not appear in diagnostics"
    );
}

#[test]
fn typed_decode_error_redacts_credential_bearing_invalid_payload() {
    const MARKER: &str = "invalid-payload-password-canary-4c18";
    let line = format!(r#"{{"Hello":"password={MARKER}"}}"#);
    let items = decode_message_line_items(&line)
        .expect("invalid typed payload should remain an item error");
    let item_debug = format!("{:?}", items.first().expect("one item should be retained"));
    let aggregate_error =
        decode_message_line(&line).expect_err("invalid Hello payload must not type-check");
    let rendered = format!("{item_debug}\n{aggregate_error:?}\n{aggregate_error}");

    assert!(
        !rendered.contains(MARKER),
        "credential-bearing invalid payload must not appear in diagnostics"
    );
}
