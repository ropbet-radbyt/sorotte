use super::*;
use std::collections::BTreeSet;

use crate::{first_non_empty_stdout_line, python_probe::run_python_probe_raw};
use sorotte_protocol::{HelloPayload, decode_message_line_items};

const GENERATED_JSON_FRAMING_SEED: u64 = 0x5a17_d1ff_e2c4_907b;
const GENERATED_JSON_FRAMING_BUDGET: usize = 256;
const GENERATED_JSON_FRAMING_VALID_CASES: usize = 224;
const GENERATED_JSON_FRAMING_MALFORMED_JSON_CASES: usize = 16;
const GENERATED_JSON_FRAMING_MALFORMED_UTF8_CASES: usize = 16;
const JSON_FRAMING_COMMANDS: [&str; 7] = ["Hello", "Set", "List", "State", "Chat", "Error", "TLS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedJsonFramingCaseKind {
    Valid,
    MalformedJson,
    MalformedUtf8,
}

#[derive(Debug)]
struct GeneratedJsonFramingCase {
    id: String,
    wire: Vec<u8>,
    kind: GeneratedJsonFramingCaseKind,
    command_count: usize,
    has_duplicate_command: bool,
}

#[derive(Debug)]
struct DeterministicJsonFramingGenerator {
    state: u64,
}

impl DeterministicJsonFramingGenerator {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn bounded(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        (self.next_u64() % upper_bound as u64) as usize
    }
}

fn generated_protocol_message(command_index: usize, token: u64) -> ProtocolMessage {
    match JSON_FRAMING_COMMANDS[command_index] {
        "Hello" => ProtocolMessage::hello(
            HelloPayload::new(
                format!("generated-user-{token}-雪"),
                format!("generated-room-{}", token % 13),
                "1.7.5",
            )
            .with_features(json!({
                "uiMode": if token.is_multiple_of(2) { "CLI" } else { "GUI" },
                "generatedToken": token,
                "escaped": "\"quoted\\value\n🦀",
            })),
        ),
        "Set" => ProtocolMessage::set(
            SetPayload::new().with_room(RoomRef::new(format!("generated-room-{}", token % 17))),
        ),
        "List" => ProtocolMessage::list_request(),
        "State" => ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position((token % 50_000) as f64 / 10.0)
                    .with_paused(token.is_multiple_of(2))
                    .with_do_seek(token.is_multiple_of(3)),
            ),
        ),
        "Chat" => ProtocolMessage::chat_text(format!("generated-chat-{token}-\"quoted\"-\\-雪")),
        "Error" => ProtocolMessage::error_message(format!("generated-error-{token}")),
        "TLS" => ProtocolMessage::start_tls(if token.is_multiple_of(2) {
            "send"
        } else {
            "false"
        }),
        command => panic!("unsupported generated JSON framing command {command}"),
    }
}

fn escaped_json_command_key(command: &str, escape_mask: usize) -> String {
    let mut encoded = String::from("\"");
    for (index, byte) in command.bytes().enumerate() {
        if escape_mask & (1 << index) == 0 {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("\\u{byte:04x}"));
        }
    }
    encoded.push('"');
    encoded
}

fn generated_valid_json_framing_case(
    generator: &mut DeterministicJsonFramingGenerator,
    index: usize,
) -> GeneratedJsonFramingCase {
    let entry_count = if index < JSON_FRAMING_COMMANDS.len() {
        1
    } else {
        2 + generator.bounded(9)
    };
    let first_command = if index < JSON_FRAMING_COMMANDS.len() {
        index
    } else {
        generator.bounded(JSON_FRAMING_COMMANDS.len())
    };
    let force_duplicate = index >= JSON_FRAMING_COMMANDS.len() && index.is_multiple_of(2);
    let outer_whitespace = ["", " ", "\t", "\r"];
    let separator_whitespace = ["", " ", "\t"];
    let mut command_indices = Vec::with_capacity(entry_count);
    let mut entries = Vec::with_capacity(entry_count);
    for entry_index in 0..entry_count {
        let command_index =
            if entry_index == 0 || (force_duplicate && entry_index + 1 == entry_count) {
                first_command
            } else {
                generator.bounded(JSON_FRAMING_COMMANDS.len())
            };
        command_indices.push(command_index);
        let command = JSON_FRAMING_COMMANDS[command_index];
        let message = generated_protocol_message(command_index, generator.next_u64());
        let value =
            serde_json::to_value(message).expect("generated protocol message should serialize");
        let payload = value
            .get(command)
            .unwrap_or_else(|| panic!("generated {command} envelope should contain its payload"));
        let key = escaped_json_command_key(command, generator.bounded(1 << command.len()));
        let before_colon = separator_whitespace[generator.bounded(separator_whitespace.len())];
        let after_colon = separator_whitespace[generator.bounded(separator_whitespace.len())];
        entries.push(format!(
            "{key}{before_colon}:{after_colon}{}",
            serde_json::to_string(payload).expect("generated payload should serialize")
        ));
    }

    let prefix = outer_whitespace[generator.bounded(outer_whitespace.len())];
    let suffix = outer_whitespace[generator.bounded(outer_whitespace.len())];
    let comma = [",", ", ", ",\t"][generator.bounded(3)];
    let wire = format!("{prefix}{{{}}}{suffix}", entries.join(comma));
    let unique_commands = command_indices.iter().copied().collect::<BTreeSet<_>>();

    GeneratedJsonFramingCase {
        id: format!("valid-{index:03}"),
        wire: wire.into_bytes(),
        kind: GeneratedJsonFramingCaseKind::Valid,
        command_count: entry_count,
        has_duplicate_command: unique_commands.len() != command_indices.len(),
    }
}

fn generated_json_framing_cases() -> Vec<GeneratedJsonFramingCase> {
    let mut generator = DeterministicJsonFramingGenerator::new(GENERATED_JSON_FRAMING_SEED);
    let mut cases = Vec::with_capacity(GENERATED_JSON_FRAMING_BUDGET);
    for index in 0..GENERATED_JSON_FRAMING_VALID_CASES {
        cases.push(generated_valid_json_framing_case(&mut generator, index));
    }
    for index in 0..GENERATED_JSON_FRAMING_MALFORMED_JSON_CASES {
        let mut wire = format!(r#"{{"Chat":"generated-truncated-{index}-雪"}}"#).into_bytes();
        assert_eq!(wire.pop(), Some(b'}'));
        cases.push(GeneratedJsonFramingCase {
            id: format!("malformed-json-{index:03}"),
            wire,
            kind: GeneratedJsonFramingCaseKind::MalformedJson,
            command_count: 0,
            has_duplicate_command: false,
        });
    }

    const INVALID_UTF8: [&[u8]; 7] = [
        &[0x80],
        &[0xc0, 0xaf],
        &[0xed, 0xa0, 0x80],
        &[0xf4, 0x90, 0x80, 0x80],
        &[0xc2],
        &[0xe2, 0x82],
        &[0xf0, 0x9f, 0x92],
    ];
    for index in 0..GENERATED_JSON_FRAMING_MALFORMED_UTF8_CASES {
        let mut wire = br#"{"Chat":"generated-before-"#.to_vec();
        wire.extend_from_slice(INVALID_UTF8[index % INVALID_UTF8.len()]);
        wire.extend_from_slice(br#"-after"}"#);
        cases.push(GeneratedJsonFramingCase {
            id: format!("malformed-utf8-{index:03}"),
            wire,
            kind: GeneratedJsonFramingCaseKind::MalformedUtf8,
            command_count: 0,
            has_duplicate_command: false,
        });
    }
    assert_eq!(cases.len(), GENERATED_JSON_FRAMING_BUDGET);
    cases
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn rust_json_framing_outcome(wire: &[u8]) -> (bool, Vec<Value>) {
    let Ok(line) = std::str::from_utf8(wire) else {
        return (false, Vec::new());
    };
    let Ok(items) = decode_message_line_items(line) else {
        return (false, Vec::new());
    };
    if items.iter().any(|item| item.message.is_err()) {
        return (false, Vec::new());
    }

    let events = items
        .into_iter()
        .map(|item| {
            json!({
                "command": item.command.expect("accepted generated envelope has a command"),
                "payload": item.payload,
            })
        })
        .collect();
    (true, events)
}

fn assert_exact_json_object_keys(
    object: &serde_json::Map<String, Value>,
    expected: &[&str],
    label: &str,
) {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{label} keys must remain closed");
}

#[test]
fn generated_json_framing_matches_pinned_python_oracle() {
    let cases = generated_json_framing_cases();
    let seed = format!("{GENERATED_JSON_FRAMING_SEED:#018x}");
    let case_documents = cases
        .iter()
        .map(|case| {
            json!({
                "id": case.id,
                "lineHex": lowercase_hex(&case.wire),
            })
        })
        .collect::<Vec<_>>();
    let request = serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "seed": seed,
        "budget": GENERATED_JSON_FRAMING_BUDGET,
        "cases": case_documents,
    }))
    .expect("generated JSON framing batch should serialize");

    let stdout = match run_python_probe_raw(&["--json-framing-oracle-batch"], &request) {
        Ok(stdout) => stdout,
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "generated JSON framing differential skipped due to missing local prerequisites"
            );
            return;
        }
        Err(err) => panic!("generated JSON framing differential should succeed, got: {err}"),
    };
    let response_line =
        first_non_empty_stdout_line(&stdout).expect("Python framing oracle should respond");
    let response: Value =
        serde_json::from_str(response_line).expect("Python framing oracle should return JSON");
    let response = response
        .as_object()
        .expect("Python framing oracle response should be an object");
    assert_exact_json_object_keys(
        response,
        &["schemaVersion", "seed", "budget", "processed", "results"],
        "Python framing response",
    );
    assert_eq!(response.get("schemaVersion"), Some(&json!(1)));
    assert_eq!(response.get("seed"), Some(&json!(seed)));
    assert_eq!(
        response.get("budget"),
        Some(&json!(GENERATED_JSON_FRAMING_BUDGET))
    );
    assert_eq!(
        response.get("processed"),
        Some(&json!(GENERATED_JSON_FRAMING_BUDGET))
    );

    let results = response
        .get("results")
        .and_then(Value::as_array)
        .expect("Python framing response should contain results");
    assert_eq!(results.len(), cases.len());
    let mut remaining = cases
        .iter()
        .map(|case| (case.id.clone(), case))
        .collect::<BTreeMap<_, _>>();
    let mut observed_commands = BTreeSet::new();
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    for result in results {
        let result = result
            .as_object()
            .expect("Python framing case result should be an object");
        assert_exact_json_object_keys(
            result,
            &["id", "accepted", "events", "errorCount", "raised"],
            "Python framing case result",
        );
        let id = result
            .get("id")
            .and_then(Value::as_str)
            .expect("Python framing result should contain an id");
        let case = remaining
            .remove(id)
            .unwrap_or_else(|| panic!("Python framing result id {id:?} must be unique and known"));
        let python_accepted = result
            .get("accepted")
            .and_then(Value::as_bool)
            .expect("Python framing result should classify acceptance");
        let python_events = result
            .get("events")
            .and_then(Value::as_array)
            .expect("Python framing result should contain events");
        let error_count = result
            .get("errorCount")
            .and_then(Value::as_u64)
            .expect("Python framing result should count errors");
        let raised = result
            .get("raised")
            .and_then(Value::as_bool)
            .expect("Python framing result should classify exceptions");
        assert_eq!(
            python_accepted,
            !raised && error_count == 0,
            "{id} Python classification must be internally consistent"
        );

        let (rust_accepted, rust_events) = rust_json_framing_outcome(&case.wire);
        assert_eq!(
            rust_accepted,
            case.kind == GeneratedJsonFramingCaseKind::Valid,
            "{id} generated category must match the independent Rust classification"
        );
        assert_eq!(
            python_accepted, rust_accepted,
            "{id} acceptance differs between pinned Python and Rust"
        );
        if python_accepted {
            accepted += 1;
            observed_commands.extend(python_events.iter().map(|event| {
                event
                    .get("command")
                    .and_then(Value::as_str)
                    .expect("accepted Python event should identify its command")
            }));
            assert_eq!(
                python_events, &rust_events,
                "{id} command order or last duplicate payload differs"
            );
        } else {
            rejected += 1;
            assert!(
                python_events.is_empty(),
                "{id} rejected framing case must not dispatch partial commands"
            );
        }
    }
    assert!(
        remaining.is_empty(),
        "every generated framing case must be reported exactly once: {remaining:?}"
    );
    assert_eq!(
        observed_commands,
        JSON_FRAMING_COMMANDS.into_iter().collect::<BTreeSet<_>>(),
        "the fixed generator must exercise every supported command"
    );

    assert_eq!(accepted, GENERATED_JSON_FRAMING_VALID_CASES);
    assert_eq!(
        rejected,
        GENERATED_JSON_FRAMING_MALFORMED_JSON_CASES + GENERATED_JSON_FRAMING_MALFORMED_UTF8_CASES
    );
    assert_eq!(
        cases
            .iter()
            .filter(|case| case.kind == GeneratedJsonFramingCaseKind::Valid)
            .count(),
        GENERATED_JSON_FRAMING_VALID_CASES
    );
    assert_eq!(
        cases
            .iter()
            .filter(|case| case.kind == GeneratedJsonFramingCaseKind::MalformedJson)
            .count(),
        GENERATED_JSON_FRAMING_MALFORMED_JSON_CASES
    );
    assert_eq!(
        cases
            .iter()
            .filter(|case| case.kind == GeneratedJsonFramingCaseKind::MalformedUtf8)
            .count(),
        GENERATED_JSON_FRAMING_MALFORMED_UTF8_CASES
    );
    assert!(
        cases
            .iter()
            .filter(|case| case.kind == GeneratedJsonFramingCaseKind::Valid)
            .all(|case| case.command_count > 0),
        "every valid case must exercise at least one command"
    );
    assert!(
        cases
            .iter()
            .filter(|case| case.has_duplicate_command)
            .count()
            >= 100,
        "fixed generator must retain broad duplicate-command coverage"
    );
}

#[test]
fn live_python_peer_probe_advertises_the_playlist_behavior_it_exercises() {
    let path = python_live_peer_probe_script_path();
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read live Python peer probe {path:?}: {error}"));

    assert!(
        source.contains(r#""sharedPlaylists": True"#),
        "the reference peer must advertise shared-playlist protocol support"
    );
    assert!(
        source.contains(r#""sharedPlaylistEnabled": True"#),
        "the reference peer must enable the shared-playlist client path"
    );
}

#[test]
fn python_interop_roundtrip_returns_server_hello() {
    let transcript = match run_python_handshake_roundtrip() {
        Ok(transcript) => transcript,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python interop handshake test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => panic!("python interop handshake should succeed, got: {err}"),
    };

    assert_eq!(transcript.response_hello.username, "interop-client");
    assert_eq!(transcript.response_hello.room.name, "interop-room");
    assert_eq!(transcript.response_hello.version, "sorotte-dev");
    assert_eq!(
        transcript.response_hello.realversion.as_deref(),
        Some("1.7.5")
    );
}

#[test]
fn python_interop_sequence_supports_list_set_and_state() {
    let requests = vec![
        ProtocolMessage::hello(default_rust_client_hello_for_interop()),
        ProtocolMessage::list_request(),
        ProtocolMessage::set(SetPayload::new().with_room(RoomRef::new("interop-room-2"))),
        ProtocolMessage::list_request(),
        ProtocolMessage::set(
            SetPayload::new().with_ready(
                ReadyPayload::new(true)
                    .with_manually_initiated(true)
                    .with_username("interop-client"),
            ),
        ),
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(42.0)
                    .with_paused(false)
                    .with_do_seek(false),
            ),
        ),
    ];

    let transcript = match run_python_protocol_roundtrip(&requests) {
        Ok(transcript) => transcript,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python interop sequence test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => panic!("python interop sequence should succeed, got: {err}"),
    };

    assert_eq!(transcript.steps.len(), requests.len());

    let hello = extract_hello_from_message(
        transcript.steps[0]
            .response_messages
            .first()
            .expect("hello step should return one message")
            .clone(),
    )
    .expect("first response should be hello");
    assert_eq!(hello.room.name, "interop-room");

    match transcript.steps[1]
        .response_messages
        .first()
        .expect("list response should be present")
    {
        ProtocolMessage::List(payload) => match &payload.list {
            ListPayload::Rooms(rooms) => {
                assert!(rooms.contains_key("interop-room"));
                let room = rooms.get("interop-room").expect("room should exist");
                assert!(room.contains_key("interop-client"));
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected list response, got {}", other.kind()),
    }

    match transcript.steps[2]
        .response_messages
        .first()
        .expect("set room response should be present")
    {
        ProtocolMessage::Set(payload) => {
            let room = payload
                .set
                .room
                .as_ref()
                .expect("set room payload should exist");
            assert_eq!(room.name, "interop-room-2");
        }
        other => panic!("expected set response, got {}", other.kind()),
    }

    match transcript.steps[3]
        .response_messages
        .first()
        .expect("second list response should be present")
    {
        ProtocolMessage::List(payload) => match &payload.list {
            ListPayload::Rooms(rooms) => {
                assert!(rooms.contains_key("interop-room-2"));
                let room = rooms.get("interop-room-2").expect("room should exist");
                assert!(room.contains_key("interop-client"));
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected list response, got {}", other.kind()),
    }

    match transcript.steps[4]
        .response_messages
        .first()
        .expect("set ready response should be present")
    {
        ProtocolMessage::Set(payload) => {
            let ready = payload
                .set
                .ready
                .as_ref()
                .expect("ready payload should be present");
            assert_eq!(ready.username.as_deref(), Some("interop-client"));
            assert_eq!(ready.is_ready, Some(true));
        }
        other => panic!("expected set response, got {}", other.kind()),
    }

    assert!(
        transcript.steps[5].response_messages.is_empty(),
        "state message should be accepted without an immediate response"
    );
}
