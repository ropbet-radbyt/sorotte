use std::fs;
use std::path::PathBuf;

use serde_json::json;

use super::{
    ChatPayload, ControllerAuthPayload, HelloPayload, ListPayload, NewControlledRoomPayload,
    PingPayload, PlaystatePayload, ProtocolMessage, ReadyPayload, RoomRef,
    SOROTTE_PLEX_PLAYLIST_URIS_KEY, SetPayload, StatePayload, canonical_playlist_files_from_change,
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
    let controller_auth = ControllerAuthPayload::new().with_password("controller-secret-value");
    let new_controlled_room =
        NewControlledRoomPayload::new().with_password("new-room-secret-value");

    let controller_debug = format!("{controller_auth:?}");
    assert!(controller_debug.contains("<redacted>"));
    assert!(!controller_debug.contains("controller-secret-value"));

    let new_room_debug = format!("{new_controlled_room:?}");
    assert!(new_room_debug.contains("<redacted>"));
    assert!(!new_room_debug.contains("new-room-secret-value"));
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
