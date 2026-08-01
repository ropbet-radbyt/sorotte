//! Generated and adversarial assurance for the public protocol codec.
//!
//! The examples in `tests.rs` remain the readable wire specifications. These
//! tests explore bounded arbitrary inputs and compare the production codec
//! against structural and metamorphic oracles which do not call its private
//! scanners.

use std::collections::{BTreeMap, BTreeSet};

use proptest::{prelude::*, test_runner::Config as ProptestConfig};
use serde_json::{Value, json};

use super::*;

const DEFAULT_PROPTEST_CASES: u32 = 256;
const MAX_PROPTEST_CASES: u32 = 100_000;
const TOP_LEVEL_COMMANDS: [&str; 7] = ["Hello", "Set", "List", "State", "Chat", "Error", "TLS"];
const GENERATED_COMMANDS: [&str; 8] = [
    "Hello", "Set", "List", "State", "Chat", "Error", "TLS", "Future",
];

fn resolve_proptest_cases(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_PROPTEST_CASES);
    };
    let cases = raw
        .parse::<u32>()
        .map_err(|_| format!("PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"))?;
    if cases == 0 {
        return Err(format!(
            "PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"
        ));
    }
    Ok(cases.min(MAX_PROPTEST_CASES))
}

fn configured_proptest() -> ProptestConfig {
    let raw_cases = std::env::var("PROPTEST_CASES").ok();
    ProptestConfig {
        cases: resolve_proptest_cases(raw_cases.as_deref())
            .unwrap_or_else(|reason| panic!("{reason}")),
        max_shrink_iters: 20_000,
        ..ProptestConfig::default()
    }
}

fn bounded_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..=24)
        .prop_map(|characters| characters.into_iter().collect())
}

fn bounded_json_key() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..=12)
        .prop_map(|characters| characters.into_iter().collect())
}

fn bounded_json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        (-1_000_000i64..=1_000_000).prop_map(|number| json!(number)),
        bounded_text().prop_map(Value::String),
    ];

    leaf.prop_recursive(4, 96, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..=6).prop_map(Value::Array),
            proptest::collection::btree_map(bounded_json_key(), inner, 0..=6)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

#[derive(Clone, Debug)]
struct GeneratedMessageSeed {
    shape: u8,
    first: String,
    second: String,
    third: String,
    flag: bool,
    number: i32,
    files: Vec<String>,
    extension: Value,
}

fn message_from_seed(seed: GeneratedMessageSeed) -> ProtocolMessage {
    let GeneratedMessageSeed {
        shape,
        first,
        second,
        third,
        flag,
        number,
        files,
        extension,
    } = seed;
    match shape % 9 {
        0 => {
            let mut hello = HelloPayload::new(first, second, third);
            hello.realversion = flag.then(|| "generated-real-version".to_owned());
            hello.features = Some(extension.clone());
            hello.extra.insert("x-generated".to_owned(), extension);
            ProtocolMessage::hello(hello)
        }
        1 => {
            let mut users = BTreeMap::new();
            users.insert(
                first.clone(),
                UserSetPayload::new()
                    .with_room(RoomRef::new(second.clone()))
                    .with_file(extension.clone())
                    .with_controller(flag)
                    .with_is_ready(!flag),
            );
            let playlist_change = if flag {
                PlaylistChangePayload::new(files.clone()).with_user(first.clone())
            } else {
                PlaylistChangePayload::new(files.clone()).with_null_user()
            };
            let playlist_index = if flag {
                PlaylistIndexPayload::new(i64::from(number)).with_user(first.clone())
            } else {
                PlaylistIndexPayload::null().with_null_user()
            };
            ProtocolMessage::set(
                SetPayload::new()
                    .with_room(RoomRef::new(second))
                    .with_file(
                        FilePayload::new()
                            .with_name(third)
                            .with_duration(f64::from(number) / 10.0)
                            .with_size(extension.clone())
                            .with_path(first),
                    )
                    .with_user(users)
                    .with_ready(
                        ReadyPayload::new(flag)
                            .with_manually_initiated(!flag)
                            .with_set_by("generated"),
                    )
                    .with_playlist_change(playlist_change)
                    .with_playlist_index(playlist_index)
                    .with_features(extension),
            )
        }
        2 => ProtocolMessage::list_request(),
        3 => {
            let mut entry = ListUserEntry::new()
                .with_position(f64::from(number) / 10.0)
                .with_file(extension.clone())
                .with_controller(flag)
                .with_is_ready(!flag)
                .with_features(extension.clone());
            entry.extra.insert("x-generated".to_owned(), extension);
            let mut users = BTreeMap::new();
            users.insert(first, entry);
            let mut rooms = BTreeMap::new();
            rooms.insert(second, users);
            ProtocolMessage::list(ListPayload::rooms(rooms))
        }
        4 => {
            let mut state = StatePayload::new()
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(f64::from(number) / 10.0)
                        .with_paused(flag)
                        .with_do_seek(!flag)
                        .with_set_by(first),
                )
                .with_ping(
                    PingPayload::new()
                        .with_latency_calculation(f64::from(number))
                        .with_client_latency_calculation(f64::from(number) / 2.0)
                        .with_client_rtt(f64::from(number) / 100.0)
                        .with_server_rtt(f64::from(number) / 200.0),
                )
                .with_ignoring_on_the_fly(
                    IgnoringOnTheFlyPayload::new()
                        .with_server(number.unsigned_abs())
                        .with_client(number.unsigned_abs().saturating_add(1)),
                );
            state.extra.insert("x-generated".to_owned(), extension);
            ProtocolMessage::state(state)
        }
        5 => ProtocolMessage::chat_text(first),
        6 => {
            let mut chat = ChatMessagePayload::new(first, second);
            chat.extra.insert("x-generated".to_owned(), extension);
            ProtocolMessage::chat(ChatPayload::Message(chat))
        }
        7 => {
            let mut error = ErrorPayload::new(first);
            error.extra.insert("x-generated".to_owned(), extension);
            ProtocolMessage::error(error)
        }
        8 => {
            let mut tls = TlsPayload::new(first);
            tls.extra.insert("x-generated".to_owned(), extension);
            ProtocolMessage::tls(tls)
        }
        _ => unreachable!("shape is reduced modulo nine"),
    }
}

fn supported_message_strategy() -> impl Strategy<Value = ProtocolMessage> {
    (
        any::<u8>(),
        bounded_text(),
        bounded_text(),
        bounded_text(),
        any::<bool>(),
        -1_000_000i32..=1_000_000,
        proptest::collection::vec(bounded_text(), 0..=6),
        bounded_json_value().prop_map(|value| json!({"generated": value})),
    )
        .prop_map(
            |(shape, first, second, third, flag, number, files, extension)| {
                message_from_seed(GeneratedMessageSeed {
                    shape,
                    first,
                    second,
                    third,
                    flag,
                    number,
                    files,
                    extension,
                })
            },
        )
}

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

fn message_outcome(message: &Result<ProtocolMessage, ProtocolError>) -> Option<&'static str> {
    message.as_ref().ok().map(ProtocolMessage::kind)
}

fn unexpected_message_kind(error: ProtocolError) -> Option<(&'static str, &'static str)> {
    match error {
        ProtocolError::UnexpectedMessageKind { expected, found } => Some((expected, found)),
        _ => None,
    }
}

fn expected_item_layout(value: &Value) -> Vec<(Option<String>, Value)> {
    let Some(object) = value.as_object() else {
        return vec![(None, value.clone())];
    };
    if object.len() <= 1 {
        let command = object.keys().next().cloned();
        let payload = command
            .as_ref()
            .and_then(|command| object.get(command))
            .cloned()
            .unwrap_or_else(|| value.clone());
        return vec![(command, payload)];
    }
    object
        .iter()
        .map(|(command, payload)| (Some(command.clone()), payload.clone()))
        .collect()
}

fn ascii_unicode_escaped_key(key: &str) -> String {
    let escaped = key
        .bytes()
        .map(|byte| format!("\\u{byte:04x}"))
        .collect::<String>();
    format!("\"{escaped}\"")
}

#[derive(Clone, Debug)]
struct RawCommandEntry {
    command_index: u8,
    ordinal: i16,
    escape_key: bool,
}

fn raw_command_entry_strategy() -> impl Strategy<Value = RawCommandEntry> {
    (
        0u8..GENERATED_COMMANDS.len() as u8,
        any::<i16>(),
        any::<bool>(),
    )
        .prop_map(|(command_index, ordinal, escape_key)| RawCommandEntry {
            command_index,
            ordinal,
            escape_key,
        })
}

fn raw_composite_line(entries: &[RawCommandEntry]) -> String {
    let members = entries
        .iter()
        .map(|entry| {
            let key = GENERATED_COMMANDS[usize::from(entry.command_index)];
            let encoded_key = if entry.escape_key {
                ascii_unicode_escaped_key(key)
            } else {
                serde_json::to_string(key).expect("static command key should encode")
            };
            let payload = json!({
                "ordinal": entry.ordinal,
                "decoy": "},\"Future\":{\"nested\":true}",
                "nested": {"TLS": {"startTLS": "false"}},
            });
            format!(
                "\n  {encoded_key} \t: {}",
                serde_json::to_string(&payload).expect("generated payload should encode")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{members}\n}}")
}

#[test]
fn protocol_proptest_case_budget_rejects_zero_and_caps_excessive_values() {
    assert_eq!(resolve_proptest_cases(None), Ok(DEFAULT_PROPTEST_CASES));
    assert_eq!(resolve_proptest_cases(Some("2048")), Ok(2_048));
    assert_eq!(
        resolve_proptest_cases(Some(&u32::MAX.to_string())),
        Ok(MAX_PROPTEST_CASES)
    );
    for invalid in ["", "0", "-1", "not-a-number"] {
        assert!(
            resolve_proptest_cases(Some(invalid)).is_err(),
            "{invalid:?} must not silently weaken the property budget"
        );
    }
}

#[test]
fn generated_message_vocabulary_covers_every_supported_envelope() {
    let commands = (0..9)
        .map(|shape| {
            wire_command(&message_from_seed(GeneratedMessageSeed {
                shape,
                first: "first".to_owned(),
                second: "second".to_owned(),
                third: "third".to_owned(),
                flag: shape % 2 == 0,
                number: i32::from(shape),
                files: vec!["one.mkv".to_owned(), "two.mkv".to_owned()],
                extension: json!({"future": [shape]}),
            }))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(commands, TOP_LEVEL_COMMANDS.into_iter().collect());
}

#[test]
fn malformed_json_and_malformed_envelopes_fail_at_the_expected_boundary() {
    let malformed_json = [
        "",
        " ",
        "{",
        "[",
        "\"",
        "{\"Set\":",
        "{\"Set\":{}} trailing",
        "{\"Set\":{},}",
        "[1,]",
        r#"{"Chat":"\x"}"#,
        r#"{"Chat":"\uD800"}"#,
        r#"{"State":/* comment */{}}"#,
    ];
    for line in malformed_json {
        assert!(
            decode_line(line).is_err(),
            "{line:?} should not be valid JSON"
        );
        assert!(decode_message_line_items(line).is_err());
        assert!(decode_message_lines(line).is_err());
        assert!(decode_message_line(line).is_err());
    }

    let malformed_envelopes = [
        "null",
        "[]",
        "{}",
        r#"{"Hello":null}"#,
        r#"{"Hello":{"username":"alice"}}"#,
        r#"{"Set":[]}"#,
        r#"{"List":true}"#,
        r#"{"State":"invalid"}"#,
        r#"{"Chat":42}"#,
        r#"{"Error":{"message":false}}"#,
        r#"{"TLS":{"startTLS":true}}"#,
        r#"{"Future":{"payload":true}}"#,
    ];
    for line in malformed_envelopes {
        assert!(decode_line(line).is_ok(), "{line:?} should be valid JSON");
        let items = decode_message_line_items(line).expect("outer JSON should decode");
        assert_eq!(items.len(), 1);
        assert!(
            items[0].message.is_err(),
            "{line:?} should fail typed envelope decoding"
        );
        assert!(decode_message_lines(line).is_err());
        assert!(decode_message_line(line).is_err());
    }
}

#[test]
fn codec_remains_total_at_line_size_and_recursion_boundaries() {
    const CHAT_OVERHEAD: usize = r#"{"Chat":""}"#.len();
    let at_limit = format!(
        r#"{{"Chat":"{}"}}"#,
        "x".repeat(DEFAULT_MAX_PROTOCOL_LINE_BYTES - CHAT_OVERHEAD)
    );
    let above_limit = format!(
        r#"{{"Chat":"{}"}}"#,
        "x".repeat(DEFAULT_MAX_PROTOCOL_LINE_BYTES)
    );
    assert_eq!(at_limit.len(), DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    assert!(above_limit.len() > DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    for line in [&at_limit, &above_limit] {
        assert!(decode_line(line).is_ok());
        assert!(decode_message_line_items(line).is_ok());
        assert!(decode_message_lines(line).is_ok());
        assert!(decode_message_line(line).is_ok());
    }

    let truncated_large_string = &above_limit[..above_limit.len() - 2];
    assert!(decode_line(truncated_large_string).is_err());
    assert!(decode_message_line_items(truncated_large_string).is_err());
    assert!(decode_message_lines(truncated_large_string).is_err());
    assert!(decode_message_line(truncated_large_string).is_err());

    let deeply_nested = format!("{}null{}", "[".repeat(256), "]".repeat(256));
    assert!(
        decode_line(&deeply_nested).is_err(),
        "serde_json's recursion guard should reject adversarial nesting safely"
    );
    assert!(decode_message_line_items(&deeply_nested).is_err());
    assert!(decode_message_lines(&deeply_nested).is_err());
    assert!(decode_message_line(&deeply_nested).is_err());
}

#[test]
fn duplicate_commands_have_first_position_and_last_value_semantics() {
    let line = r#"{
        "TLS":{"startTLS":"false"},
        "Chat":"first",
        "\u0054LS":{"startTLS":"true"},
        "Future":{"generation":1},
        "Chat":"last"
    }"#;
    let items = decode_message_line_items(line).expect("duplicate-member JSON should decode");
    assert_eq!(
        items
            .iter()
            .map(|item| item.command.as_deref())
            .collect::<Vec<_>>(),
        [Some("TLS"), Some("Chat"), Some("Future")]
    );
    assert_eq!(items[0].payload, json!({"startTLS": "true"}));
    assert_eq!(items[1].payload, json!("last"));
    assert_eq!(items[2].payload, json!({"generation": 1}));
    assert_eq!(message_outcome(&items[0].message), Some("TLS"));
    assert_eq!(message_outcome(&items[1].message), Some("Chat"));
    assert!(items[2].message.is_err());

    assert!(
        decode_message_lines(line).is_err(),
        "aggregate decoding must retain the unknown-command failure"
    );
    assert!(
        decode_message_line(line).is_err(),
        "singular decoding is also aggregate-strict before selecting the first command"
    );
}

#[test]
fn duplicate_set_members_have_first_position_and_last_value_semantics() {
    let line = r#"{
        "Set":{
            "ready":{"isReady":true},
            "file":{"name":"first.mkv"},
            "\u0072eady":{"isReady":false},
            "file":{"name":"last.mkv"}
        }
    }"#;
    let message = decode_message_line(line).expect("duplicate Set members should decode");
    let ProtocolMessage::Set(message) = message else {
        panic!("expected Set message");
    };
    assert_eq!(message.set.command_order, ["ready", "file"]);
    assert_eq!(
        message.set.ready.as_ref().and_then(|ready| ready.is_ready),
        Some(false)
    );
    assert_eq!(
        message
            .set
            .file
            .as_ref()
            .and_then(|file| file.name.as_deref()),
        Some("last.mkv")
    );
}

#[test]
fn collapsed_duplicate_set_members_appear_once_in_command_order() {
    let line = r#"{
        "Set":{
            "ready":{"isReady":true},
            "file":{"name":"first.mkv"},
            "ready":{"isReady":false},
            "file":{"name":"last.mkv"}
        }
    }"#;
    let message = decode_message_line(line).expect("duplicate Set members should decode");
    let ProtocolMessage::Set(message) = message else {
        panic!("expected Set message");
    };
    let unique_commands = message.set.command_order.iter().collect::<BTreeSet<_>>();
    assert_eq!(
        message.set.command_order.len(),
        unique_commands.len(),
        "collapsed duplicate Set members must appear once in command order"
    );
}

#[test]
fn mixed_supported_and_unknown_commands_preserve_each_item_result() {
    let line = r#"{
        "Chat":"hello",
        "Future":{"password":"untrusted"},
        "TLS":{"startTLS":"true"}
    }"#;
    let items = decode_message_line_items(line).expect("outer composite JSON should decode");
    assert_eq!(items.len(), 3);
    assert_eq!(message_outcome(&items[0].message), Some("Chat"));
    assert!(items[1].message.is_err());
    assert_eq!(message_outcome(&items[2].message), Some("TLS"));
    assert!(decode_message_lines(line).is_err());
    assert!(
        decode_message_line(line).is_err(),
        "singular decoding must retain failures from later composite commands"
    );
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn arbitrary_byte_strings_are_total_across_public_decode_entrypoints(
        bytes in proptest::collection::vec(any::<u8>(), 0..=2_048),
    ) {
        let line = String::from_utf8_lossy(&bytes);
        let raw = decode_line(&line);
        let items = decode_message_line_items(&line);
        let messages = decode_message_lines(&line);
        let first = decode_message_line(&line);

        prop_assert_eq!(raw.is_ok(), items.is_ok());
        match items {
            Err(_) => {
                prop_assert!(messages.is_err());
                prop_assert!(first.is_err());
            }
            Ok(items) => {
                prop_assert!(!items.is_empty());
                let all_messages_valid = items.iter().all(|item| item.message.is_ok());
                prop_assert_eq!(messages.is_ok(), all_messages_valid);
                prop_assert_eq!(first.is_ok(), all_messages_valid);
                if all_messages_valid {
                    prop_assert_eq!(
                        message_outcome(&first),
                        message_outcome(&items[0].message)
                    );
                }
                if let Ok(messages) = messages {
                    let expected_kinds = items
                        .iter()
                        .map(|item| {
                            item.message
                                .as_ref()
                                .expect("all item results were checked")
                                .kind()
                        })
                        .collect::<Vec<_>>();
                    prop_assert_eq!(
                        messages.iter().map(ProtocolMessage::kind).collect::<Vec<_>>(),
                        expected_kinds
                    );
                }
            }
        }
    }

    #[test]
    fn arbitrary_json_matches_structural_and_whitespace_oracles(
        value in bounded_json_value(),
    ) {
        let compact = serde_json::to_string(&value).expect("generated JSON should encode");
        let pretty = serde_json::to_string_pretty(&value).expect("generated JSON should encode");

        let decoded_compact = decode_line(&compact);
        let decoded_pretty = decode_line(&pretty);
        prop_assert_eq!(decoded_compact.as_ref().ok(), Some(&value));
        prop_assert_eq!(decoded_pretty.as_ref().ok(), Some(&value));
        let _ = extract_hello(&value);
        let encoded = encode_line(&value).expect("JSON Value should always encode");
        let decoded_encoded = decode_line(&encoded);
        prop_assert_eq!(decoded_encoded.as_ref().ok(), Some(&value));

        let expected = expected_item_layout(&value);
        let compact_items =
            decode_message_line_items(&compact).expect("valid compact JSON should decode");
        let pretty_items =
            decode_message_line_items(&pretty).expect("valid pretty JSON should decode");
        prop_assert_eq!(compact_items.len(), expected.len());
        prop_assert_eq!(pretty_items.len(), expected.len());

        for (index, (expected_command, expected_payload)) in expected.iter().enumerate() {
            prop_assert_eq!(&compact_items[index].command, expected_command);
            prop_assert_eq!(&compact_items[index].payload, expected_payload);
            prop_assert_eq!(&pretty_items[index].command, expected_command);
            prop_assert_eq!(&pretty_items[index].payload, expected_payload);
            prop_assert_eq!(
                message_outcome(&compact_items[index].message),
                message_outcome(&pretty_items[index].message)
            );
        }

        let all_messages_valid = compact_items.iter().all(|item| item.message.is_ok());
        prop_assert_eq!(decode_message_lines(&compact).is_ok(), all_messages_valid);
        let first = decode_message_line(&compact);
        prop_assert_eq!(first.is_ok(), all_messages_valid);
        if all_messages_valid {
            prop_assert_eq!(
                message_outcome(&first),
                message_outcome(&compact_items[0].message)
            );
        }
    }

    #[test]
    fn supported_messages_roundtrip_with_wire_and_extraction_invariants(
        message in supported_message_strategy(),
    ) {
        let expected_command = wire_command(&message);
        let expected_value =
            serde_json::to_value(&message).expect("generated protocol message should serialize");
        let encoded =
            encode_message_line(&message).expect("generated protocol message should encode");
        let decoded_value = decode_line(&encoded);
        prop_assert_eq!(decoded_value.as_ref().ok(), Some(&expected_value));

        let object = expected_value
            .as_object()
            .expect("supported envelopes must serialize as objects");
        prop_assert_eq!(object.len(), 1);
        prop_assert!(object.contains_key(expected_command));

        let decoded =
            decode_message_line(&encoded).expect("encoded supported message should decode");
        prop_assert_eq!(decoded.kind(), message.kind());
        prop_assert_eq!(&decoded, &message);
        let decoded_many =
            decode_message_lines(&encoded).expect("single supported message should decode");
        prop_assert_eq!(decoded_many.as_slice(), std::slice::from_ref(&message));

        let items =
            decode_message_line_items(&encoded).expect("single supported message should decode");
        prop_assert_eq!(items.len(), 1);
        prop_assert_eq!(items[0].command.as_deref(), Some(expected_command));
        prop_assert_eq!(
            &items[0].payload,
            object
                .get(expected_command)
                .expect("wire command must have a payload")
        );
        prop_assert_eq!(message_outcome(&items[0].message), Some(message.kind()));

        match &message {
            ProtocolMessage::Hello(expected) => {
                let extracted =
                    extract_hello(&expected_value).expect("Hello extraction should succeed");
                prop_assert_eq!(&extracted, &expected.hello);
                let extracted_from_message = extract_hello_from_message(decoded)
                    .expect("decoded Hello extraction should succeed");
                prop_assert_eq!(&extracted_from_message, &expected.hello);
            }
            _ => {
                let error = extract_hello(&expected_value)
                    .expect_err("non-Hello extraction must remain type-safe");
                prop_assert_eq!(
                    unexpected_message_kind(error),
                    Some(("Hello", message.kind()))
                );
                let error = extract_hello_from_message(decoded)
                    .expect_err("non-Hello extraction must remain type-safe");
                prop_assert_eq!(
                    unexpected_message_kind(error),
                    Some(("Hello", message.kind()))
                );
            }
        }
    }

    #[test]
    fn generated_duplicate_composites_match_first_position_last_value_oracle(
        entries in proptest::collection::vec(raw_command_entry_strategy(), 1..=24),
    ) {
        let line = raw_composite_line(&entries);
        let parsed = decode_line(&line).expect("generated composite should be valid JSON");
        let parsed_object = parsed
            .as_object()
            .expect("generated composite should be an object");

        let mut seen = BTreeSet::new();
        let expected_order = entries
            .iter()
            .filter_map(|entry| {
                let command = GENERATED_COMMANDS[usize::from(entry.command_index)];
                seen.insert(command).then_some(command)
            })
            .collect::<Vec<_>>();
        let items =
            decode_message_line_items(&line).expect("generated composite should decode to items");
        prop_assert_eq!(items.len(), expected_order.len());
        for (item, expected_command) in items.iter().zip(expected_order) {
            prop_assert_eq!(item.command.as_deref(), Some(expected_command));
            prop_assert_eq!(
                &item.payload,
                parsed_object
                    .get(expected_command)
                    .expect("last duplicate value must remain in parsed object")
            );
        }
        let all_messages_valid = items.iter().all(|item| item.message.is_ok());
        prop_assert_eq!(decode_message_lines(&line).is_ok(), all_messages_valid);
        let first = decode_message_line(&line);
        prop_assert_eq!(first.is_ok(), all_messages_valid);
        if all_messages_valid {
            prop_assert_eq!(
                message_outcome(&first),
                message_outcome(&items[0].message)
            );
        }
    }
}
