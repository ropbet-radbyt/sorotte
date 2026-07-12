use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::json;

use super::{
    ChatMessagePayload, ChatPayload, ControllerAuthPayload, ErrorPayload, FilePayload,
    HelloPayload, IgnoringOnTheFlyPayload, ListPayload, ListUserEntry, MediaLoadIntent,
    MediaReadyPayload, NewControlledRoomPayload, PingPayload, PlaybackBarrierPolicy,
    PlaybackBarrierSetExtension, PlaybackBarrierStateExtension, PlaybackBarrierTimeoutAction,
    PlaylistChangePayload, PlaylistIndexPayload, PlaystatePayload, PrepareMediaPayload,
    ProtocolError, ProtocolMessage, ReadyPayload, RoomBufferingPhase, RoomBufferingPolicy,
    RoomBufferingPolicyPayload, RoomBufferingStatusPayload, RoomRef, SOROTTE_PLAYBACK_BARRIER_V1,
    SOROTTE_PLEX_PLAYLIST_URIS_KEY, SetPayload, StartedAckPayload, StatePayload, TlsPayload,
    TransportBufferingReportPayload, UserSetPayload, canonical_playlist_files_from_change,
    decode_line, decode_message_line, decode_message_line_items, decode_message_lines, encode_line,
    encode_message_line, extract_hello, extract_hello_from_message,
    playlist_change_with_plex_sidecar,
};

fn fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("protocol");
    path
}

fn fixture_path(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn read_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("fixture file should be readable")
}

#[test]
fn decode_hello_fixture() {
    let fixture = read_fixture("hello_minimal.json");
    let value = decode_line(&fixture).expect("fixture JSON should decode");
    let hello = extract_hello(&value).expect("hello payload should parse");

    assert_eq!(hello.username, "alice");
    assert_eq!(hello.room.name, "room1");
    assert_eq!(hello.version, "1.2.255");
    assert_eq!(hello.realversion.as_deref(), Some("1.7.5"));
    assert_eq!(hello.effective_version(), "1.7.5");
}

#[test]
fn decode_message_hello_fixture() {
    let fixture = read_fixture("hello_minimal.json");
    let message = decode_message_line(&fixture).expect("fixture should decode as protocol message");
    let hello = extract_hello_from_message(message).expect("hello message should be extracted");
    assert_eq!(hello.username, "alice");
}

#[test]
fn decode_all_fixtures_as_protocol_messages() {
    let fixture_paths = fs::read_dir(fixture_dir()).expect("fixture directory should exist");
    for entry in fixture_paths {
        let entry = entry.expect("fixture entry should be readable");
        if !entry
            .file_type()
            .expect("file type should be readable")
            .is_file()
        {
            continue;
        }
        let fixture = fs::read_to_string(entry.path()).expect("fixture file should be readable");
        let message =
            decode_message_line(&fixture).expect("each fixture should decode as protocol message");
        assert!(!message.kind().is_empty());
    }
}

#[test]
fn roundtrip_message_fixture() {
    let fixture = read_fixture("state_ping.json");
    let message = decode_message_line(&fixture).expect("state fixture should decode");
    let encoded = encode_message_line(&message).expect("message should encode");
    let decoded = decode_message_line(&encoded).expect("encoded message should decode");
    assert_eq!(message, decoded);
}

#[test]
fn roundtrip_raw_json_value_fixture() {
    let fixture = read_fixture("state_ping.json");
    let value = decode_line(&fixture).expect("fixture JSON should decode");
    let encoded = encode_line(&value).expect("value should encode");
    let decoded = decode_line(&encoded).expect("encoded JSON should decode");
    assert_eq!(value, decoded);
}

#[test]
fn playback_barrier_fixtures_decode_from_nested_extension_maps() {
    let prepare_message = decode_message_line(&read_fixture("set_playback_barrier_prepare.json"))
        .expect("prepare fixture should decode");
    let ProtocolMessage::Set(prepare_message) = prepare_message else {
        panic!("prepare fixture should be a Set message");
    };
    let prepare = prepare_message
        .set
        .playback_barrier_v1()
        .expect("prepare extension should be well formed")
        .and_then(|extension| extension.prepare)
        .expect("prepare extension should contain PrepareMedia");
    assert_eq!(prepare.media_generation, 7);
    assert_eq!(prepare.request_nonce, 55);
    assert_eq!(prepare.load_intent, MediaLoadIntent::NewPlayback);
    assert_eq!(prepare.logical_media_id, "youtube:example");
    assert_eq!(prepare.policy, PlaybackBarrierPolicy::Quorum);
    assert_eq!(prepare.quorum, Some(2));
    assert_eq!(prepare.quorum_percent, Some(75));
    assert_eq!(
        prepare.timeout_action,
        Some(PlaybackBarrierTimeoutAction::Continue)
    );
    assert_eq!(prepare.deadline, Some(115.0));

    let observations =
        decode_message_line(&read_fixture("state_playback_barrier_observations.json"))
            .expect("observation fixture should decode");
    let ProtocolMessage::State(observations) = observations else {
        panic!("observation fixture should be a State message");
    };
    let observations = observations
        .state
        .playback_barrier_v1()
        .expect("observation extension should be well formed")
        .expect("observation extension should be present");
    assert!(observations.ready.is_some_and(|ready| ready.is_ready()));
    assert_eq!(
        observations
            .started
            .as_ref()
            .map(|started| started.state_revision),
        Some(11)
    );
}

#[test]
fn playback_barrier_builders_remain_additive_and_roundtrip() {
    let prepare = ProtocolMessage::set(
        SetPayload::new().with_playback_barrier_v1(
            PlaybackBarrierSetExtension::new().with_prepare(
                PrepareMediaPayload::request(
                    91,
                    "logical-media",
                    42.0,
                    PlaybackBarrierPolicy::Quorum,
                    MediaLoadIntent::Replay,
                )
                .with_quorum_percent(75)
                .with_timeout_ms(20_000)
                .with_timeout_action(PlaybackBarrierTimeoutAction::AskController),
            ),
        ),
    );
    let ready = ProtocolMessage::state(
        StatePayload::new().with_playback_barrier_v1(
            PlaybackBarrierStateExtension::new()
                .with_ready(MediaReadyPayload::new(9, true, true).with_seekable(true))
                .with_started(StartedAckPayload::new(9, 3, 42.1)),
        ),
    );

    for message in [prepare, ready] {
        let encoded = encode_message_line(&message).expect("barrier message should encode");
        let value = decode_line(&encoded).expect("barrier message should remain valid JSON");
        assert!(
            value
                .as_object()
                .is_some_and(|commands| commands.len() == 1),
            "barrier messages must not add an unconditional top-level command"
        );
        let nested = value.pointer(&format!(
            "/{}/{}",
            message.kind(),
            SOROTTE_PLAYBACK_BARRIER_V1
        ));
        assert!(
            nested.is_some(),
            "barrier object should be nested in Set/State"
        );
        assert_eq!(
            decode_message_line(&encoded).expect("barrier message should decode"),
            message
        );
    }
}

#[test]
fn room_buffering_policy_and_transport_reports_roundtrip_inside_capability_extension() {
    let policy = RoomBufferingPolicyPayload::new(9, RoomBufferingPolicy::Quorum)
        .with_request_nonce(12)
        .with_load_intent(MediaLoadIntent::Replay)
        .with_state_revision(3)
        .with_quorum_percent(75)
        .with_debounce_ms(800)
        .with_resume_hysteresis_ms(1_500)
        .with_max_pause_ms(30_000);
    let status = RoomBufferingStatusPayload {
        config: policy.clone(),
        phase: RoomBufferingPhase::Paused,
        eligible_clients: 3,
        required_buffering_clients: 3,
        buffering_clients: ["alice".to_owned(), "bob".to_owned()].into_iter().collect(),
        pause_deadline: Some(140.0),
    };
    let set_message = ProtocolMessage::set(
        SetPayload::new().with_playback_barrier_v1(
            PlaybackBarrierSetExtension::new()
                .with_buffering_policy(policy.clone())
                .with_buffering_status(status),
        ),
    );
    let transport = TransportBufferingReportPayload::new(9, true)
        .with_state_revision(3)
        .with_buffered_seconds(0.25)
        .with_observed_at(110.5);
    let state_message = ProtocolMessage::state(StatePayload::new().with_playback_barrier_v1(
        PlaybackBarrierStateExtension::new().with_transport(transport.clone()),
    ));

    let encoded_set = encode_message_line(&set_message).expect("policy should encode");
    let set_json = decode_line(&encoded_set).expect("policy JSON should decode");
    assert_eq!(
        set_json
            .pointer("/Set/sorottePlaybackBarrierV1/bufferingPolicy/policy")
            .and_then(serde_json::Value::as_str),
        Some("quorum")
    );
    assert_eq!(
        set_json
            .pointer("/Set/sorottePlaybackBarrierV1/bufferingPolicy/requestNonce")
            .and_then(serde_json::Value::as_u64),
        Some(12)
    );
    assert_eq!(
        set_json
            .pointer("/Set/sorottePlaybackBarrierV1/bufferingPolicy/loadIntent")
            .and_then(serde_json::Value::as_str),
        Some("replay")
    );
    assert_eq!(
        decode_message_line(&encoded_set).expect("policy should roundtrip"),
        set_message
    );

    let encoded_state = encode_message_line(&state_message).expect("transport should encode");
    assert_eq!(
        decode_message_line(&encoded_state).expect("transport should roundtrip"),
        state_message
    );
    let ProtocolMessage::State(decoded) =
        decode_message_line(&encoded_state).expect("transport should decode")
    else {
        panic!("transport report should remain a State message");
    };
    assert_eq!(
        decoded
            .state
            .playback_barrier_v1()
            .expect("transport extension should be valid")
            .and_then(|extension| extension.transport),
        Some(transport)
    );
}

#[test]
fn malformed_playback_barrier_extension_does_not_break_envelope_decoding() {
    let message = decode_message_line(
        r#"{"State":{"ping":{"clientRtt":0.1},"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":"invalid"}}}}"#,
    )
    .expect("permissive State envelope should preserve a malformed extension");
    let ProtocolMessage::State(message) = message else {
        panic!("expected State message");
    };
    assert!(message.state.ping.is_some());
    assert!(message.state.playback_barrier_v1().is_err());
}

#[test]
fn playback_barrier_debug_redacts_logical_media_identity() {
    const MARKER: &str = "private-logical-media-token";
    let prepare = PrepareMediaPayload::new(1, MARKER, 0.0, PlaybackBarrierPolicy::Controller);
    let debug = format!("{:?}", prepare);
    assert!(!debug.contains(MARKER));
    assert!(debug.contains("<redacted>"));

    let message_debug = format!(
            "{:?}",
            ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_prepare(prepare),
            ),)
        );
    assert!(!message_debug.contains(MARKER));
}

#[test]
fn list_request_fixture_decodes_as_request_variant() {
    let fixture = read_fixture("list_request.json");
    let message = decode_message_line(&fixture).expect("list request should decode");
    match message {
        ProtocolMessage::List(payload) => {
            assert!(matches!(payload.list, ListPayload::Request(_)));
        }
        other => panic!("expected List message, found {}", other.kind()),
    }
}

#[test]
fn chat_fixture_supports_text_and_object_variants() {
    let text_message =
        decode_message_line(&read_fixture("chat_text.json")).expect("text chat should decode");
    match text_message {
        ProtocolMessage::Chat(chat) => assert!(matches!(chat.chat, ChatPayload::Text(_))),
        other => panic!("expected Chat message, found {}", other.kind()),
    }

    let object_message =
        decode_message_line(&read_fixture("chat_message.json")).expect("object chat should decode");
    match object_message {
        ProtocolMessage::Chat(chat) => assert!(matches!(chat.chat, ChatPayload::Message(_))),
        other => panic!("expected Chat message, found {}", other.kind()),
    }
}

#[test]
fn ready_message_with_null_is_ready_decodes_as_unknown() {
    let message = decode_message_line(
        r#"{"Set":{"ready":{"username":"alice","isReady":null,"manuallyInitiated":false}}}"#,
    )
    .expect("legacy nullable ready payload should decode");
    let ProtocolMessage::Set(set_message) = message else {
        panic!("expected Set message");
    };
    let ready = set_message
        .set
        .ready
        .expect("set message should include a ready payload");
    assert_eq!(ready.is_ready, None);
    assert_eq!(ready.username.as_deref(), Some("alice"));
    assert_eq!(ready.manually_initiated, Some(false));
}

#[test]
fn playlist_index_message_with_null_index_decodes_as_null_snapshot() {
    let message = decode_message_line(r#"{"Set":{"playlistIndex":{"user":null,"index":null}}}"#)
        .expect("legacy nullable playlistIndex payload should decode");
    let ProtocolMessage::Set(set_message) = message else {
        panic!("expected Set message");
    };
    let playlist_index = set_message
        .set
        .playlist_index
        .expect("nullable playlistIndex payload should be retained");
    assert_eq!(playlist_index.index_value(), None);

    let encoded = encode_message_line(&ProtocolMessage::set(
        SetPayload::new().with_playlist_index(playlist_index),
    ))
    .expect("nullable playlistIndex payload should encode");
    let encoded_value = decode_line(&encoded).expect("encoded playlistIndex should decode");
    assert_eq!(
        encoded_value,
        json!({"Set":{"playlistIndex":{"index":null,"user":null}}})
    );
}

#[test]
fn playlist_change_message_with_null_user_roundtrips() {
    let message = decode_message_line(r#"{"Set":{"playlistChange":{"files":[],"user":null}}}"#)
        .expect("legacy nullable playlistChange payload should decode");
    let ProtocolMessage::Set(set_message) = message else {
        panic!("expected Set message");
    };
    let playlist_change = set_message
        .set
        .playlist_change
        .expect("nullable playlistChange payload should be retained");

    let encoded = encode_message_line(&ProtocolMessage::set(
        SetPayload::new().with_playlist_change(playlist_change),
    ))
    .expect("nullable playlistChange payload should encode");
    let encoded_value = decode_line(&encoded).expect("encoded playlistChange should decode");
    assert_eq!(
        encoded_value,
        json!({"Set":{"playlistChange":{"files":[],"user":null}}})
    );
}

#[test]
fn plex_playlist_sidecar_keeps_syncplay_files_baseline() {
    let plex_uri = "plex://server-machine-id/metadata/14452?title=Episode%2011&file=%5BErai-raws%5D%20Re%20Zero%20-%2011%20%5B1080p%5D.mkv&duration=1470058&type=episode";
    let payload =
        playlist_change_with_plex_sidecar([plex_uri, "plain-episode.mkv"], true).with_user("alice");

    assert_eq!(
        payload.files,
        vec![
            "[Erai-raws] Re Zero - 11 [1080p].mkv".to_owned(),
            "plain-episode.mkv".to_owned()
        ]
    );
    assert_eq!(
        payload.extra.get(SOROTTE_PLEX_PLAYLIST_URIS_KEY),
        Some(&json!([plex_uri, null]))
    );
    assert_eq!(
        canonical_playlist_files_from_change(&payload),
        vec![plex_uri.to_owned(), "plain-episode.mkv".to_owned()]
    );

    let encoded = encode_message_line(&ProtocolMessage::set(
        SetPayload::new().with_playlist_change(payload),
    ))
    .expect("playlist sidecar message should encode");
    let encoded_value = decode_line(&encoded).expect("encoded playlist sidecar should decode");
    assert_eq!(
        encoded_value,
        json!({
            "Set": {
                "playlistChange": {
                    "files": ["[Erai-raws] Re Zero - 11 [1080p].mkv", "plain-episode.mkv"],
                    "user": "alice",
                    "sorottePlexPlaylistUris": [plex_uri, null]
                }
            }
        })
    );
}

#[test]
fn plex_playlist_sidecar_can_be_omitted_for_legacy_recipients() {
    let plex_uri =
        "plex://server-machine-id/metadata/99?title=Movie&file=Folder%5CMovie%20Name.mkv";
    let payload = playlist_change_with_plex_sidecar([plex_uri], false);

    assert_eq!(payload.files, vec!["Movie Name.mkv".to_owned()]);
    assert!(!payload.extra.contains_key(SOROTTE_PLEX_PLAYLIST_URIS_KEY));
    assert_eq!(
        canonical_playlist_files_from_change(&payload),
        vec!["Movie Name.mkv".to_owned()]
    );
}

#[test]
fn decode_message_lines_preserves_top_level_command_order() {
    let messages = decode_message_lines(r#"{"Set":{"room":{"name":"room2"}},"List":null}"#)
        .expect("multi-command protocol line should decode");

    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], ProtocolMessage::Set(_)));
    assert!(matches!(messages[1], ProtocolMessage::List(_)));
}

#[test]
fn decode_message_line_items_preserves_errors_after_valid_commands() {
    let items = decode_message_line_items(r#"{"Set":{"room":{"name":"room2"}},"Bogus":{"x":1}}"#)
        .expect("mixed multi-command protocol line should parse as JSON");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].command.as_deref(), Some("Set"));
    assert!(items[0].message.is_ok());
    assert_eq!(items[1].command.as_deref(), Some("Bogus"));
    assert_eq!(items[1].payload, json!({"x": 1}));
    assert!(items[1].message.is_err());
}

#[test]
fn set_payload_preserves_nested_command_order() {
    let message =
        decode_message_line(r#"{"Set":{"file":{"name":"movie.mkv"},"room":{"name":"room2"}}}"#)
            .expect("set payload should decode");
    let ProtocolMessage::Set(set_message) = message else {
        panic!("expected Set message");
    };
    assert_eq!(
        set_message.set.command_order,
        vec!["file".to_owned(), "room".to_owned()]
    );
}

#[test]
fn set_fixtures_decode_user_event_variants() {
    let joined_message = decode_message_line(&read_fixture("set_user_joined.json"))
        .expect("set joined fixture should decode");
    match joined_message {
        ProtocolMessage::Set(payload) => {
            let users = payload.set.user.expect("user payload should be present");
            let alice = users.get("alice").expect("alice user entry should exist");
            assert_eq!(
                alice.room.as_ref().map(|room| room.name.as_str()),
                Some("room1")
            );
            assert_eq!(alice.event.as_ref(), Some(&json!({"joined": true})));
            assert_eq!(alice.features.as_ref(), Some(&json!({"uiMode": "GUI"})));
            assert_eq!(alice.controller, Some(false));
            assert_eq!(alice.is_ready, Some(true));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let left_message = decode_message_line(&read_fixture("set_user_left.json"))
        .expect("set left fixture should decode");
    match left_message {
        ProtocolMessage::Set(payload) => {
            let users = payload.set.user.expect("user payload should be present");
            let alice = users.get("alice").expect("alice user entry should exist");
            assert_eq!(alice.event.as_ref(), Some(&json!({"left": true})));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }
}

#[test]
fn set_fixtures_decode_controller_playlist_and_file_variants() {
    let controller_auth_message =
        decode_message_line(&read_fixture("set_controller_auth_success.json"))
            .expect("controller auth fixture should decode");
    match controller_auth_message {
        ProtocolMessage::Set(payload) => {
            let controller_auth = payload
                .set
                .controller_auth
                .expect("controllerAuth payload should be present");
            assert_eq!(controller_auth.room.as_deref(), Some("room1"));
            assert_eq!(
                controller_auth
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("secret")
            );
            assert_eq!(controller_auth.user.as_deref(), Some("alice"));
            assert_eq!(controller_auth.success, Some(true));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let controlled_room_message =
        decode_message_line(&read_fixture("set_new_controlled_room.json"))
            .expect("new controlled room fixture should decode");
    match controlled_room_message {
        ProtocolMessage::Set(payload) => {
            let room = payload
                .set
                .new_controlled_room
                .expect("newControlledRoom payload should be present");
            assert_eq!(room.room_name.as_deref(), Some("managed-room"));
            assert_eq!(
                room.password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("roompass")
            );
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let playlist_change_message = decode_message_line(&read_fixture("set_playlist_change.json"))
        .expect("playlist change fixture should decode");
    match playlist_change_message {
        ProtocolMessage::Set(payload) => {
            let playlist_change = payload
                .set
                .playlist_change
                .expect("playlistChange payload should be present");
            assert_eq!(
                playlist_change.files,
                vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
            );
            assert_eq!(playlist_change.user.as_deref(), Some("alice"));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let playlist_index_message = decode_message_line(&read_fixture("set_playlist_index.json"))
        .expect("playlist index fixture should decode");
    match playlist_index_message {
        ProtocolMessage::Set(payload) => {
            let playlist_index = payload
                .set
                .playlist_index
                .expect("playlistIndex payload should be present");
            assert_eq!(playlist_index.index_value(), Some(1));
            assert_eq!(playlist_index.index, 1);
            assert_eq!(playlist_index.user.as_deref(), Some("alice"));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let file_message = decode_message_line(&read_fixture("set_file_full.json"))
        .expect("set file fixture should decode");
    match file_message {
        ProtocolMessage::Set(payload) => {
            let file = payload.set.file.expect("file payload should be present");
            assert_eq!(file.name.as_deref(), Some("movie.mkv"));
            assert_eq!(file.duration, Some(95.5));
            assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
            assert_eq!(file.path.as_deref(), Some("/media/movie.mkv"));
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }

    let features_message = decode_message_line(&read_fixture("set_features_update.json"))
        .expect("set features fixture should decode");
    match features_message {
        ProtocolMessage::Set(payload) => {
            assert_eq!(
                payload.set.features.as_ref(),
                Some(&json!({"username":"alice","features":{"chat":true,"readiness":true}}))
            );
        }
        other => panic!("expected Set message, found {}", other.kind()),
    }
}

#[test]
fn credential_payload_debug_is_redacted() {
    const DIRECT_MARKER: &str = "controller-secret-value";
    const EXTRA_MARKER: &str = "nested-controller-token-canary-293fa8";
    let mut controller_auth = ControllerAuthPayload::new().with_password(DIRECT_MARKER);
    controller_auth.extra.insert(
        "vendorExtension".to_owned(),
        json!({ "nested": [{ "accessToken": EXTRA_MARKER }] }),
    );
    let mut new_controlled_room =
        NewControlledRoomPayload::new().with_password("new-room-secret-value");
    new_controlled_room.extra.insert(
        "vendorExtension".to_owned(),
        json!({ "nested": { "roomPassword": EXTRA_MARKER } }),
    );

    let controller_debug = format!("{controller_auth:?}");
    assert!(controller_debug.contains("<redacted>"));
    assert!(!controller_debug.contains(DIRECT_MARKER));
    assert!(!controller_debug.contains(EXTRA_MARKER));

    let new_room_debug = format!("{new_controlled_room:?}");
    assert!(new_room_debug.contains("<redacted>"));
    assert!(!new_room_debug.contains("new-room-secret-value"));
    assert!(!new_room_debug.contains(EXTRA_MARKER));

    let set = SetPayload::new()
        .with_controller_auth(controller_auth)
        .with_new_controlled_room(new_controlled_room);
    for debug in [
        format!("{set:?}"),
        format!("{:?}", ProtocolMessage::set(set)),
    ] {
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(DIRECT_MARKER));
        assert!(!debug.contains("new-room-secret-value"));
        assert!(!debug.contains(EXTRA_MARKER));
    }
}

#[test]
fn hello_debug_redacts_sensitive_flattened_fields() {
    const MARKER: &str = "protocol-hello-secret-canary-4f5c1d";
    let mut hello = HelloPayload::new("alice", "room", "1.2.255");
    hello.extra.insert(
        "password".to_owned(),
        serde_json::Value::String(MARKER.to_owned()),
    );
    hello.extra.insert(
        "vendorAccessToken".to_owned(),
        serde_json::Value::String(MARKER.to_owned()),
    );
    hello.extra.insert(
        "nested".to_owned(),
        serde_json::json!({ "credentials": { "authTokenValue": MARKER } }),
    );
    hello.features = Some(serde_json::json!({ "futureSecret": MARKER }));

    let direct_debug = format!("{hello:?}");
    let message_debug = format!("{:?}", ProtocolMessage::hello(hello));
    for debug in [direct_debug, message_debug] {
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(MARKER));
    }
}

#[test]
fn decoded_line_item_debug_redacts_nested_raw_payload_credentials() {
    const MARKER: &str = "decoded-line-token-canary-a69be1";
    let items = decode_message_line_items(&format!(
        r#"{{"Bogus":{{"vendor":{{"nested":{{"sessionToken":"{MARKER}"}}}}}}}}"#
    ))
    .expect("raw protocol line should decode into an item");

    let debug = format!(
        "{:?}",
        items.first().expect("decoded item should be present")
    );
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(MARKER));
}

#[test]
fn tokenized_media_url_debug_is_redacted_at_every_protocol_layer() {
    const MARKER: &str = "protocol-media-url-token-canary-71d8";
    let target = format!("https://plex.invalid/video?X-Plex-Token={MARKER}");
    let file = FilePayload::new()
        .with_name(target.clone())
        .with_path(target.clone());
    let playlist = PlaylistChangePayload::new([target.clone()]);
    let raw_line = serde_json::json!({
        "Set": { "file": { "path": target } }
    })
    .to_string();
    let decoded = decode_message_line_items(&raw_line).expect("raw Set line should decode");

    for debug in [
        format!("{file:?}"),
        format!("{playlist:?}"),
        format!(
            "{:?}",
            ProtocolMessage::set(SetPayload::new().with_file(file))
        ),
        format!("{:?}", decoded.first().expect("decoded item should exist")),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(MARKER));
    }
}

#[test]
fn reflected_protocol_error_debug_and_display_hide_credentials() {
    const MARKER: &str = "protocol-reflected-error-password-canary-6a2f";
    let reflected = format!(r#"Not JSON: {{"password" : "{MARKER}"}}"#);
    let payload = ErrorPayload::new(reflected.clone());
    let error = ProtocolError::ServerError { message: reflected };

    for rendered in [
        format!("{payload:?}"),
        format!("{:?}", ProtocolMessage::error(payload)),
        format!("{error:?}"),
        error.to_string(),
    ] {
        assert!(!rendered.contains(MARKER));
    }
    assert!(error.to_string().contains(sorotte_secret::REDACTED_SECRET));
}

#[test]
fn permissive_protocol_dto_debug_recursively_redacts_unknown_credentials() {
    const MARKER: &str = "permissive-protocol-secret-canary-e3915b";
    fn nested_secret() -> serde_json::Value {
        json!({ "vendor": [{ "futureCredential": MARKER }] })
    }
    fn assert_redacted(debug: &str) {
        assert!(debug.contains("<redacted>"), "missing redaction in {debug}");
        assert!(!debug.contains(MARKER), "credential leaked in {debug}");
    }

    let mut set = SetPayload::new().with_features(nested_secret());
    set.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{set:?}"));
    assert_redacted(&format!("{:?}", ProtocolMessage::set(set)));

    let mut file = FilePayload::new()
        .with_size(nested_secret())
        .with_path(format!("https://media.invalid/video?access_token={MARKER}"));
    file.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{file:?}"));
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::set(SetPayload::new().with_file(file))
    ));

    let mut user = UserSetPayload::new()
        .with_file(nested_secret())
        .with_event(nested_secret())
        .with_features(nested_secret());
    user.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{user:?}"));
    let mut users = BTreeMap::new();
    users.insert("alice".to_owned(), user);
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::set(SetPayload::new().with_user(users))
    ));

    let mut ready = ReadyPayload::new(true);
    ready.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{ready:?}"));
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::set(SetPayload::new().with_ready(ready))
    ));

    let mut playlist_change = PlaylistChangePayload::new(["movie.mkv"]);
    playlist_change
        .extra
        .insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{playlist_change:?}"));
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change))
    ));

    let mut playlist_index = PlaylistIndexPayload::new(0);
    playlist_index
        .extra
        .insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{playlist_index:?}"));
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::set(SetPayload::new().with_playlist_index(playlist_index))
    ));

    let mut chat = ChatMessagePayload::new("alice", "hello");
    chat.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{chat:?}"));
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::chat(ChatPayload::Message(chat))
    ));

    let mut error = ErrorPayload::new("failure");
    error.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{error:?}"));
    assert_redacted(&format!("{:?}", ProtocolMessage::error(error)));

    let mut tls = TlsPayload::new("true");
    tls.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{tls:?}"));
    assert_redacted(&format!("{:?}", ProtocolMessage::tls(tls)));

    let mut playstate = PlaystatePayload::new();
    playstate
        .extra
        .insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{playstate:?}"));
    let mut ping = PingPayload::new();
    ping.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{ping:?}"));
    let mut ignoring = IgnoringOnTheFlyPayload::new();
    ignoring
        .extra
        .insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{ignoring:?}"));
    let mut state = StatePayload::new()
        .with_playstate(playstate)
        .with_ping(ping)
        .with_ignoring_on_the_fly(ignoring);
    state.extra.insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{state:?}"));
    assert_redacted(&format!("{:?}", ProtocolMessage::state(state)));

    let mut list_user = ListUserEntry::new()
        .with_file(nested_secret())
        .with_features(nested_secret());
    list_user
        .extra
        .insert("extension".to_owned(), nested_secret());
    assert_redacted(&format!("{list_user:?}"));
    let mut room_users = BTreeMap::new();
    room_users.insert("alice".to_owned(), list_user);
    let mut rooms = BTreeMap::new();
    rooms.insert("room".to_owned(), room_users);
    assert_redacted(&format!(
        "{:?}",
        ProtocolMessage::list(ListPayload::rooms(rooms))
    ));
}

#[test]
fn state_fixtures_decode_playstate_ping_and_ignore_variants() {
    let playstate_message = decode_message_line(&read_fixture("state_playstate_setby.json"))
        .expect("state playstate fixture should decode");
    match playstate_message {
        ProtocolMessage::State(payload) => {
            let playstate = payload
                .state
                .playstate
                .expect("playstate payload should be present");
            assert_eq!(playstate.position, Some(42.0));
            assert_eq!(playstate.paused, Some(true));
            assert_eq!(playstate.do_seek, Some(true));
            assert_eq!(playstate.set_by.as_deref(), Some("alice"));
        }
        other => panic!("expected State message, found {}", other.kind()),
    }

    let ping_message = decode_message_line(&read_fixture("state_ping_full.json"))
        .expect("state ping full fixture should decode");
    match ping_message {
        ProtocolMessage::State(payload) => {
            let ping = payload.state.ping.expect("ping payload should be present");
            assert_eq!(ping.latency_calculation, Some(173.4));
            assert_eq!(ping.client_latency_calculation, Some(174.1));
            assert_eq!(ping.client_rtt, Some(0.12));
            assert_eq!(ping.server_rtt, Some(0.09));
        }
        other => panic!("expected State message, found {}", other.kind()),
    }

    let ignore_server_message = decode_message_line(&read_fixture("state_ignoring_server.json"))
        .expect("state ignoring server fixture should decode");
    match ignore_server_message {
        ProtocolMessage::State(payload) => {
            let ignore = payload
                .state
                .ignoring_on_the_fly
                .expect("ignoringOnTheFly payload should be present");
            assert_eq!(ignore.server, Some(2));
            assert_eq!(ignore.client, None);
        }
        other => panic!("expected State message, found {}", other.kind()),
    }

    let ignore_client_message = decode_message_line(&read_fixture("state_ignoring_client.json"))
        .expect("state ignoring client fixture should decode");
    match ignore_client_message {
        ProtocolMessage::State(payload) => {
            let ignore = payload
                .state
                .ignoring_on_the_fly
                .expect("ignoringOnTheFly payload should be present");
            assert_eq!(ignore.server, None);
            assert_eq!(ignore.client, Some(1));
        }
        other => panic!("expected State message, found {}", other.kind()),
    }
}

#[test]
fn hello_constructor_matches_expected_wire_shape() {
    let message = ProtocolMessage::hello(
        HelloPayload::new("alice", "room1", "1.2.255")
            .with_realversion("1.7.5")
            .with_features(json!({"featureList": true})),
    );

    let encoded = encode_message_line(&message).expect("constructor-built message should encode");
    let value = decode_line(&encoded).expect("encoded message should be valid JSON");
    assert_eq!(
        value,
        json!({
            "Hello": {
                "username": "alice",
                "room": { "name": "room1" },
                "version": "1.2.255",
                "realversion": "1.7.5",
                "features": { "featureList": true }
            }
        })
    );
}

#[test]
fn convenience_constructors_match_common_wire_shapes() {
    let list_value = decode_line(
        &encode_message_line(&ProtocolMessage::list_request())
            .expect("list request message should encode"),
    )
    .expect("list request JSON should decode");
    assert_eq!(list_value, json!({"List": null}));

    let chat_value = decode_line(
        &encode_message_line(&ProtocolMessage::chat_message("alice", "hello everyone"))
            .expect("chat message should encode"),
    )
    .expect("chat JSON should decode");
    assert_eq!(
        chat_value,
        json!({"Chat": {"username": "alice", "message": "hello everyone"}})
    );
}

#[test]
fn set_and_state_builder_messages_roundtrip() {
    let set_message = ProtocolMessage::set(
        SetPayload::new()
            .with_room(RoomRef::new("room1"))
            .with_ready(
                ReadyPayload::new(true)
                    .with_manually_initiated(true)
                    .with_username("alice"),
            ),
    );
    let set_encoded = encode_message_line(&set_message).expect("set message should encode");
    let set_decoded = decode_message_line(&set_encoded).expect("set message should decode");
    assert_eq!(set_message, set_decoded);

    let state_message = ProtocolMessage::state(
        StatePayload::new()
            .with_ping(
                PingPayload::new()
                    .with_latency_calculation(1.0)
                    .with_client_latency_calculation(2.0)
                    .with_client_rtt(0.01),
            )
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(12.5)
                    .with_paused(false)
                    .with_do_seek(false),
            ),
    );
    let state_encoded = encode_message_line(&state_message).expect("state message should encode");
    let state_decoded = decode_message_line(&state_encoded).expect("state message should decode");
    assert_eq!(state_message, state_decoded);
}
