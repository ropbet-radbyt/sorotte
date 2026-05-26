use super::*;

#[test]
fn list_request_returns_room_snapshot_for_session() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"List":null}"#)
        .expect("list request should succeed");
    assert_eq!(outbound_lines.len(), 1);

    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    match response {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room = rooms.get("room1").expect("room1 should be present");
                let alice = room.get("alice").expect("alice should be listed");
                assert_eq!(alice.is_ready, None);
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected List message, got {}", other.kind()),
    }
}

#[test]
fn hello_truncates_room_name_to_legacy_limit() {
    let mut runtime = ServerRuntime::default();
    let long_room = "r".repeat(DEFAULT_MAX_ROOM_NAME_LENGTH + 10);
    let hello = format!(
        r#"{{"Hello":{{"username":"alice","room":{{"name":"{long_room}"}},"version":"1.2.255"}}}}"#
    );

    runtime
        .handle_line("client-1", &hello)
        .expect("hello should establish session");

    let expected_room = "r".repeat(DEFAULT_MAX_ROOM_NAME_LENGTH);
    assert_eq!(
        runtime
            .session("client-1")
            .expect("session should exist")
            .room,
        expected_room
    );
}

#[test]
fn set_file_broadcasts_user_file_update_and_list_includes_file() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789,"mediaMatch":{"schema":"sorotte.mediaMatch.v3","profiles":[{"profile":"combined-v3","algorithmVersion":3,"durationMs":95500,"audio":{"algorithm":"sorotte-audio-constellation-v3","timeBaseMs":1,"anchors":"U0FVMwEAAAA="},"video":{"algorithm":"sorotte-video-scene-v3","timeBaseMs":1,"anchors":"U1ZJMwEAAAA="}}]}}}}"#,
        )
        .expect("set file should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    for recipient in ["client-1", "client-2"] {
        let file = directed_messages
            .iter()
            .find_map(|(client_id, message)| {
                if client_id != recipient {
                    return None;
                }
                let ProtocolMessage::Set(payload) = message else {
                    return None;
                };
                payload
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get("alice"))
                    .and_then(|user| user.file.as_ref())
            })
            .expect("recipient should receive alice file update");
        assert_eq!(file.get("name").and_then(Value::as_str), Some("movie.mkv"));
        assert_eq!(file.get("duration").and_then(Value::as_f64), Some(95.5));
        assert_eq!(file.get("size").and_then(Value::as_i64), Some(123456789));
        assert_eq!(
            file.get("mediaMatch"),
            Some(&json!({
                "schema": "sorotte.mediaMatch.v3",
                "profiles": [{
                    "profile": "combined-v3",
                    "algorithmVersion": 3,
                    "durationMs": 95500,
                    "audio": {
                        "algorithm": "sorotte-audio-constellation-v3",
                        "timeBaseMs": 1,
                        "anchors": "U0FVMwEAAAA="
                    },
                    "video": {
                        "algorithm": "sorotte-video-scene-v3",
                        "timeBaseMs": 1,
                        "anchors": "U1ZJMwEAAAA="
                    }
                }]
            }))
        );
    }

    let outbound_lines = runtime
        .handle_line("client-2", r#"{"List":null}"#)
        .expect("list request should succeed");
    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    let ProtocolMessage::List(payload) = response else {
        panic!("expected List message");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("expected room snapshot list");
    };
    let alice_file = rooms["room1"]["alice"]
        .file
        .as_ref()
        .expect("alice list entry should include file");
    assert_eq!(
        alice_file.get("name").and_then(Value::as_str),
        Some("movie.mkv")
    );
    assert_eq!(
        alice_file.get("duration").and_then(Value::as_f64),
        Some(95.5)
    );
    assert_eq!(
        alice_file.get("mediaMatch"),
        Some(&json!({
            "schema": "sorotte.mediaMatch.v3",
            "profiles": [{
                "profile": "combined-v3",
                "algorithmVersion": 3,
                "durationMs": 95500,
                "audio": {
                    "algorithm": "sorotte-audio-constellation-v3",
                    "timeBaseMs": 1,
                    "anchors": "U0FVMwEAAAA="
                },
                "video": {
                    "algorithm": "sorotte-video-scene-v3",
                    "timeBaseMs": 1,
                    "anchors": "U1ZJMwEAAAA="
                }
            }]
        }))
    );
}

#[test]
fn set_features_updates_list_snapshot_features() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"uiMode":"CLI","chat":true}}}"#,
        )
        .expect("alice hello should establish session");

    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Set":{"features":{"uiMode":"GUI","chat":false}}}"#,
        )
        .expect("feature update should be accepted");
    assert!(
        outbound_lines.is_empty(),
        "Python server stores feature updates without immediate fanout"
    );

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"List":null}"#)
        .expect("list request should succeed");
    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    let ProtocolMessage::List(payload) = response else {
        panic!("expected List message");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("expected room snapshot list");
    };
    assert_eq!(
        rooms["room1"]["alice"].features.as_ref(),
        Some(&json!({"uiMode":"GUI","chat":false}))
    );
}

#[test]
fn set_room_truncates_room_name_to_legacy_limit() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let long_room = "r".repeat(DEFAULT_MAX_ROOM_NAME_LENGTH + 10);
    let set_room = format!(r#"{{"Set":{{"room":{{"name":"{long_room}"}}}}}}"#);
    runtime
        .handle_line("client-1", &set_room)
        .expect("set room should succeed");

    assert_eq!(
        runtime
            .session("client-1")
            .expect("session should exist")
            .room
            .chars()
            .count(),
        DEFAULT_MAX_ROOM_NAME_LENGTH
    );
}

#[test]
fn set_empty_file_clears_list_file_without_broadcast() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-1",
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5}}}"#,
        )
        .expect("initial set file should succeed");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"file":{}}}"#)
        .expect("empty set file should succeed");
    assert!(
        directed_lines.is_empty(),
        "Python server stores empty file state but does not broadcast falsey file updates"
    );

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"List":null}"#)
        .expect("list request should succeed");
    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    let ProtocolMessage::List(payload) = response else {
        panic!("expected List message");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("expected room snapshot list");
    };
    assert_eq!(rooms["room1"]["alice"].file.as_ref(), Some(&json!({})));
}

#[test]
fn set_file_truncates_filename_to_legacy_limit() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let long_name = "x".repeat(DEFAULT_MAX_FILENAME_LENGTH + 10);
    let set_file = format!(
        r#"{{"Set":{{"file":{{"name":"{long_name}","mediaMatch":{{"schema":"sorotte.mediaMatch.v3","profiles":[{{"profile":"audio-constellation-v3"}}]}}}}}}}}"#
    );
    let directed_lines = runtime
        .handle_line_fanout("client-1", &set_file)
        .expect("set file should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    let file = directed_messages
        .iter()
        .find_map(|(_, message)| {
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            payload
                .set
                .user
                .as_ref()
                .and_then(|users| users.get("alice"))
                .and_then(|user| user.file.as_ref())
        })
        .expect("file update should include file payload");

    let file_name = file
        .get("name")
        .and_then(Value::as_str)
        .expect("file update should include a name");
    assert_eq!(file_name.chars().count(), DEFAULT_MAX_FILENAME_LENGTH);
    assert_eq!(
        file.get("mediaMatch"),
        Some(&json!({
            "schema": "sorotte.mediaMatch.v3",
            "profiles": [{"profile": "audio-constellation-v3"}]
        }))
    );
}

#[test]
fn set_room_moves_session_between_rooms() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("set room should succeed");
    assert_eq!(outbound_lines.len(), 5);
    assert!(!runtime.room_is_present("room1"));
    assert!(runtime.room_is_present("room2"));
    let outbound_messages: Vec<_> = outbound_lines
        .iter()
        .map(|line| decode_message_line(line).expect("outbound line should decode"))
        .collect();
    assert!(
        outbound_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) => payload
                .set
                .user
                .as_ref()
                .and_then(|users| users.get("alice"))
                .and_then(|user| user.room.as_ref())
                .is_some_and(|room| room.name == "room2"),
            _ => false,
        }),
        "sender should receive user room update"
    );
    assert!(
        outbound_messages.iter().any(|message| {
            matches!(
                message,
                ProtocolMessage::State(payload)
                if payload.state.playstate.as_ref().is_some_and(|playstate| {
                    playstate.do_seek == Some(true) && playstate.paused == Some(true)
                })
            )
        }),
        "sender should receive seek room sync state update"
    );
    assert_eq!(
        runtime
            .session("client-1")
            .expect("session should exist")
            .room
            .as_str(),
        "room2"
    );
}

#[test]
fn hello_and_room_switch_send_null_playlist_index_snapshot() {
    let mut runtime = ServerRuntime::default();
    let hello_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");
    let hello_messages = decode_directed_lines(&hello_lines);
    assert!(
        has_null_playlist_index_snapshot(&hello_messages, "client-1"),
        "Python sends a null playlistIndex snapshot when the room has no selected index"
    );
    acknowledge_directed_state_counters(&mut runtime, &hello_messages);

    let room_switch_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("set room should succeed");
    let room_switch_messages = decode_directed_lines(&room_switch_lines);
    assert!(
        has_null_playlist_index_snapshot(&room_switch_messages, "client-1"),
        "Python sends a null room-switch playlistIndex snapshot when the destination has no selected index"
    );
}

#[test]
fn batched_set_subcommands_are_processed_in_wire_order() {
    fn first_file_room_for_bob(messages: &[(String, ProtocolMessage)]) -> Option<String> {
        messages.iter().find_map(|(client_id, message)| {
            if client_id != "client-2" {
                return None;
            }
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            let alice = payload.set.user.as_ref()?.get("alice")?;
            alice.file.as_ref()?;
            Some(alice.room.as_ref()?.name.clone())
        })
    }

    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = format!(
            r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.2.255"}}}}"#
        );
        runtime
            .handle_line(client_id, &hello)
            .expect("hello should establish session");
    }
    let file_then_room = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"file":{"name":"movie.mkv"},"room":{"name":"room2"}}}"#,
        )
        .expect("batched Set should succeed");
    let file_then_room_messages = decode_directed_lines(&file_then_room);
    assert_eq!(
        first_file_room_for_bob(&file_then_room_messages).as_deref(),
        Some("room1"),
        "file before room should publish the file in the source room"
    );

    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = format!(
            r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.2.255"}}}}"#
        );
        runtime
            .handle_line(client_id, &hello)
            .expect("hello should establish session");
    }
    let room_then_file = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"room":{"name":"room2"},"file":{"name":"movie.mkv"}}}"#,
        )
        .expect("batched Set should succeed");
    let room_then_file_messages = decode_directed_lines(&room_then_file);
    assert_eq!(
        first_file_room_for_bob(&room_then_file_messages).as_deref(),
        Some("room2"),
        "room before file should publish the file in the destination room"
    );
}

#[test]
fn batched_top_level_commands_are_processed_in_wire_order() {
    let mut runtime = ServerRuntime::default();
    let hello_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");
    acknowledge_outbound_state_counters(&mut runtime, "client-1", &hello_lines);

    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Set":{"room":{"name":"room2"}},"List":null}"#,
        )
        .expect("batched command line should succeed");
    let outbound_messages: Vec<_> = outbound_lines
        .iter()
        .map(|line| decode_message_line(line).expect("outbound line should decode"))
        .collect();
    let list_response = outbound_messages
        .iter()
        .find_map(|message| match message {
            ProtocolMessage::List(payload) => Some(&payload.list),
            _ => None,
        })
        .expect("batched List command should emit a list response");
    let ListPayload::Rooms(rooms) = list_response else {
        panic!("batched List response should contain rooms");
    };

    assert!(
        rooms.contains_key("room2"),
        "List should run after the preceding Set.room command"
    );
    assert!(
        !rooms.contains_key("room1"),
        "old room should be cleaned before the batched List snapshot"
    );
}

#[test]
fn set_room_seek_sync_state_counter_increments_without_ack() {
    let mut runtime = ServerRuntime::default();
    let hello_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");
    acknowledge_outbound_state_counters(&mut runtime, "client-1", &hello_lines);

    let first_switch = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("first room switch should succeed");
    let first_messages = decode_directed_lines(&first_switch);
    assert_eq!(
        room_seek_sync_server_counters(&first_messages, "client-1"),
        vec![1],
        "first room-switch seek sync should carry server counter 1"
    );

    let second_switch = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room3"}}}"#)
        .expect("second room switch should succeed");
    let second_messages = decode_directed_lines(&second_switch);
    assert_eq!(
        room_seek_sync_server_counters(&second_messages, "client-1"),
        vec![2],
        "second room-switch seek sync should increment server counter without ack"
    );
}

#[test]
fn list_requires_existing_session() {
    let mut runtime = ServerRuntime::default();
    let err = runtime
        .handle_line("unknown-client", r#"{"List":null}"#)
        .expect_err("list without hello should fail");
    assert!(matches!(err, ServerRuntimeError::MissingSession(_)));
}

#[test]
fn hello_fanout_notifies_existing_room_members() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("first hello should establish room");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("second hello should fan out user events");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert_eq!(directed_messages.len(), 6);

    assert!(
        directed_messages.iter().any(|(recipient, message)| {
            recipient == "client-2" && matches!(message, ProtocolMessage::Hello(_))
        }),
        "expected hello response to new client"
    );
    assert!(
        has_user_event(&directed_messages, "client-1", "bob", "joined"),
        "existing room member should receive joined event for bob"
    );
    assert!(
        has_ready_update_state(&directed_messages, "client-1", "bob", None),
        "existing room member should receive bob's unknown readiness"
    );
    assert!(
        has_ready_update_state(&directed_messages, "client-2", "bob", None),
        "joining room member should receive its unknown readiness"
    );
    assert!(
        has_playlist_snapshot(&directed_messages, "client-2", &[]),
        "new client should receive playlist snapshot before hello"
    );
    assert!(
        !has_user_event(&directed_messages, "client-2", "alice", "joined"),
        "new client should not receive synthetic joined snapshot for existing users"
    );
    assert!(
        !has_room_sync_state_update(&directed_messages, "client-1", true),
        "existing room member should not receive a join-time room sync state update"
    );
    assert!(
        !has_room_sync_state_update(&directed_messages, "client-2", true),
        "new room member should not receive a join-time room sync state update"
    );
}

#[test]
fn first_hello_receives_unknown_readiness_without_join_time_state() {
    let mut runtime = ServerRuntime::default();

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("first hello should fan out initial messages");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_ready_update_state(&directed_messages, "client-1", "alice", None),
        "joining user should receive unknown readiness before publishing Set.ready"
    );
    assert!(
        !has_room_sync_state_update(&directed_messages, "client-1", true),
        "first room member should not receive a join-time room sync state update"
    );
}

#[test]
fn hello_username_conflict_is_resolved_with_underscored_variant() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("first hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("second hello should resolve username conflict");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_user_event(&directed_messages, "client-1", "alice_", "joined"),
        "existing user should observe conflict-resolved username"
    );
    let response_message = directed_messages
        .iter()
        .find(|(client_id, message)| {
            client_id == "client-2" && matches!(message, ProtocolMessage::Hello(_))
        })
        .expect("conflict-resolved hello response should be sent to joining client")
        .1
        .clone();
    let response_hello =
        extract_hello_from_message(response_message).expect("hello response should decode");
    assert_eq!(response_hello.username, "alice_");
    assert_eq!(
        runtime
            .session("client-2")
            .expect("session should be registered")
            .username,
        "alice_"
    );
}

#[test]
fn hello_username_conflict_applies_legacy_trailing_underscore_rules() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("first hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("second hello should apply trailing-underscore conflict handling");
    assert_eq!(
        runtime
            .session("client-2")
            .expect("second session should exist")
            .username,
        "alice",
        "collision on name ending with underscore should first strip underscores"
    );

    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("third hello should append underscores until free");
    assert_eq!(
        runtime
            .session("client-3")
            .expect("third session should exist")
            .username,
        "alice__",
        "after stripping to a conflicting base username, underscores should be appended"
    );
}

#[test]
fn hello_response_features_reflect_chat_readiness_and_length_limits() {
    let mut runtime = ServerRuntime::default();
    runtime.set_chat_enabled(false);
    runtime.set_readiness_enabled(false);
    runtime.set_max_chat_message_length(42);
    runtime.set_max_username_length(12);

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);

    let hello_message = directed_messages
        .into_iter()
        .find(|(recipient, message)| {
            recipient == "client-1" && matches!(message, ProtocolMessage::Hello(_))
        })
        .expect("hello response should be present")
        .1;
    let hello = extract_hello_from_message(hello_message).expect("hello payload should decode");
    let features = hello
        .features
        .expect("server hello should include features");
    assert_eq!(features.get("chat").and_then(Value::as_bool), Some(false));
    assert_eq!(
        features.get("readiness").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        features.get("maxChatMessageLength").and_then(Value::as_u64),
        Some(42)
    );
    assert_eq!(
        features.get("maxRoomNameLength").and_then(Value::as_u64),
        Some(DEFAULT_MAX_ROOM_NAME_LENGTH as u64)
    );
    assert_eq!(
        features.get("maxFilenameLength").and_then(Value::as_u64),
        Some(DEFAULT_MAX_FILENAME_LENGTH as u64)
    );
    assert_eq!(
        features.get("maxUsernameLength").and_then(Value::as_u64),
        Some(12)
    );
    assert_eq!(
        features.get("uiMode").and_then(Value::as_str),
        Some(LEGACY_UI_MODE_UNKNOWN)
    );
}

#[test]
fn server_feature_list_includes_shared_playlists() {
    let features = crate::server_feature_list(false, false, true, true, 150, 16);

    assert_eq!(
        features.get("sharedPlaylists").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn server_feature_list_set_others_readiness_tracks_readiness_enabled() {
    let enabled_features = crate::server_feature_list(false, false, true, true, 150, 16);
    let disabled_features = crate::server_feature_list(false, false, true, false, 150, 16);

    assert_eq!(
        enabled_features
            .get("setOthersReadiness")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        disabled_features
            .get("setOthersReadiness")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn hello_response_features_reflect_isolate_rooms() {
    let mut runtime = ServerRuntime::default();
    runtime.set_isolate_rooms(true);

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    let hello_message = directed_messages
        .into_iter()
        .find(|(recipient, message)| {
            recipient == "client-1" && matches!(message, ProtocolMessage::Hello(_))
        })
        .expect("hello response should be present")
        .1;
    let hello = extract_hello_from_message(hello_message).expect("hello payload should decode");
    let features = hello
        .features
        .expect("server hello should include features");
    assert_eq!(
        features.get("isolateRooms").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn hello_requires_server_password_token_when_configured() {
    let mut runtime = ServerRuntime::default();
    runtime.set_server_password_token(Some("secret".to_owned()));

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should return protocol error response");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        directed_messages.iter().any(|(recipient, message)| {
            recipient == "client-1"
                && matches!(
                    message,
                    ProtocolMessage::Error(payload)
                        if payload.error.message == LEGACY_SERVER_PASSWORD_REQUIRED_ERROR
                )
        }),
        "hello without password should receive legacy password-required error"
    );
    assert!(
        runtime.session("client-1").is_none(),
        "session should not be created after password failure"
    );
}

#[test]
fn hello_password_error_dispatch_schedules_close_after_error() {
    let mut runtime = ServerRuntime::default();
    runtime.set_server_password_token(Some("secret".to_owned()));

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("password error should produce dispatch");

    assert_eq!(
        dispatch_error_message(&dispatch).as_deref(),
        Some(LEGACY_SERVER_PASSWORD_REQUIRED_ERROR)
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "password error should close after Error like Python dropWithError"
    );
}

#[test]
fn hello_server_password_token_accepts_exact_match_and_username_is_truncated() {
    let mut runtime = ServerRuntime::default();
    runtime.set_server_password_token(Some("secret".to_owned()));
    runtime.set_max_username_length(4);

    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice-long","password":"secret","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello with matching password should succeed");
    assert_eq!(
        runtime
            .session("client-1")
            .expect("session should exist")
            .username,
        "alic"
    );
}

#[test]
fn hello_server_password_token_accepts_legacy_python_md5_hash() {
    let mut runtime = ServerRuntime::default();
    runtime.set_server_password_token(Some("secret".to_owned()));

    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","password":"5ebe2294ecd0e0f08eab7690d2a6ee69","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello with Python-style MD5 password token should succeed");
    assert!(
        runtime.session("client-1").is_some(),
        "session should be created after MD5-compatible password match"
    );
}

#[test]
fn hello_server_password_token_rejects_non_matching_token() {
    let mut runtime = ServerRuntime::default();
    runtime.set_server_password_token(Some("secret".to_owned()));

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","password":"deadbeef","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should return protocol error response");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        directed_messages.iter().any(|(recipient, message)| {
            recipient == "client-1"
                && matches!(
                    message,
                    ProtocolMessage::Error(payload)
                        if payload.error.message == LEGACY_SERVER_WRONG_PASSWORD_ERROR
                )
        }),
        "hello with wrong password token should receive legacy wrong-password error"
    );
    assert!(
        runtime.session("client-1").is_none(),
        "session should not be created after wrong password"
    );
}

#[test]
fn chat_and_ready_updates_obey_runtime_disable_flags_and_chat_limit() {
    let mut runtime = ServerRuntime::default();
    runtime.set_chat_enabled(false);
    runtime.set_readiness_enabled(false);
    runtime.set_max_chat_message_length(4);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("bob hello should succeed");

    let chat_disabled = runtime
        .handle_line_fanout("client-1", r#"{"Chat":"hello world"}"#)
        .expect("chat while disabled should be ignored");
    assert!(
        chat_disabled.is_empty(),
        "chat should be ignored when disabled"
    );

    let ready_disabled = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("ready while disabled should emit null readiness");
    let ready_disabled_messages = decode_directed_lines(&ready_disabled);
    assert_eq!(
        ready_disabled_messages.len(),
        2,
        "disabled readiness should still fan out the null readiness state"
    );
    assert!(
        has_ready_update_state(&ready_disabled_messages, "client-1", "alice", None),
        "sender should receive null readiness while readiness is disabled"
    );
    assert!(
        has_ready_update_state(&ready_disabled_messages, "client-2", "alice", None),
        "peer should receive null readiness while readiness is disabled"
    );
    for line in &ready_disabled {
        let value: Value =
            serde_json::from_str(&line.line).expect("ready update should be valid json");
        let ready = value
            .get("Set")
            .and_then(|set| set.get("ready"))
            .expect("ready update should be present");
        assert!(
            ready.get("isReady").is_some_and(Value::is_null),
            "disabled readiness should serialize isReady as null"
        );
    }

    runtime.set_chat_enabled(true);
    let chat_enabled = runtime
        .handle_line_fanout("client-1", r#"{"Chat":"hello world"}"#)
        .expect("chat after enabling should fan out");
    let directed_messages = decode_directed_lines(&chat_enabled);
    assert!(
        directed_messages.iter().any(|(recipient, message)| {
            recipient == "client-2"
                && matches!(
                    message,
                    ProtocolMessage::Chat(payload)
                        if matches!(
                            &payload.chat,
                            ChatPayload::Message(chat) if chat.message == "hell"
                        ) || matches!(&payload.chat, ChatPayload::Text(text) if text == "hell")
                )
        }),
        "chat message should be truncated to runtime max length"
    );

    let room_switch_disabled = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch while readiness is disabled should succeed");
    assert!(
        room_switch_disabled.iter().any(|line| {
            let value: Value =
                serde_json::from_str(&line.line).expect("room switch output should be valid json");
            value
                .get("Set")
                .and_then(|set| set.get("ready"))
                .is_some_and(|ready| {
                    ready.get("username").and_then(Value::as_str) == Some("alice")
                        && ready.get("isReady").is_some_and(Value::is_null)
                })
        }),
        "room switch should republish disabled readiness as null"
    );
}

#[test]
fn chat_fanout_skips_clients_below_legacy_chat_min_version() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should succeed");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Chat":"hello room"}"#)
        .expect("chat should fan out to supported clients");
    let chat_recipients: Vec<_> = decode_directed_lines(&directed_lines)
        .into_iter()
        .filter_map(|(client_id, message)| {
            matches!(message, ProtocolMessage::Chat(_)).then_some(client_id)
        })
        .collect();

    assert_eq!(
        chat_recipients,
        vec!["client-1".to_owned()],
        "legacy clients below the chat minimum version should not receive Chat frames"
    );
}

#[test]
fn isolate_rooms_join_events_do_not_leak_to_other_rooms() {
    let mut runtime = ServerRuntime::default();
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("charlie hello should succeed");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-3",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    assert!(
        has_user_event(&directed_messages, "client-1", "bob", "joined"),
        "same-room peer should receive join event"
    );
    assert!(
        !has_user_event(&directed_messages, "client-2", "bob", "joined"),
        "other-room peer should not receive join event when isolateRooms is enabled"
    );
}

#[test]
fn isolate_rooms_list_request_is_scoped_to_requester_room() {
    let mut runtime = ServerRuntime::default();
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("charlie hello should succeed");

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"List":null}"#)
        .expect("list request should succeed");
    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    let ProtocolMessage::List(payload) = response else {
        panic!("expected list response");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("expected room snapshot list");
    };
    assert!(rooms.contains_key("room1"));
    assert!(
        !rooms.contains_key("room2"),
        "other rooms should be hidden in isolateRooms mode"
    );
}

#[test]
fn isolate_rooms_room_switch_sends_left_to_old_room_without_destination_leak() {
    let mut runtime = ServerRuntime::default();
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should succeed");
    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("charlie hello should succeed");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_user_event(&directed_messages, "client-2", "alice", "left"),
        "old-room peer should receive left event"
    );
    assert!(
        has_user_event(&directed_messages, "client-1", "alice", "left"),
        "moving client should receive the old-room left event before destination updates"
    );
    assert!(
        !has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
        "old-room peer should not receive destination room update in isolateRooms mode"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
        "new-room peer should receive room update for moved user"
    );
    let mover_left_index = directed_messages
        .iter()
        .position(|(client_id, message)| {
            client_id == "client-1"
                && matches!(message, ProtocolMessage::Set(_))
                && has_user_event(
                    &[(client_id.clone(), message.clone())],
                    "client-1",
                    "alice",
                    "left",
                )
        })
        .expect("moving client should receive left event");
    let mover_room_update_index = directed_messages
        .iter()
        .position(|(client_id, message)| {
            client_id == "client-1"
                && matches!(message, ProtocolMessage::Set(_))
                && has_user_room_update(
                    &[(client_id.clone(), message.clone())],
                    "client-1",
                    "alice",
                    "room2",
                )
        })
        .expect("moving client should receive destination room update");
    assert!(
        mover_left_index < mover_room_update_index,
        "isolated room switch should report the old-room leave before destination-room updates"
    );
}

#[test]
fn isolate_rooms_room_switch_republishes_file_to_destination_room() {
    let mut runtime = ServerRuntime::default();
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789}}}"#,
        )
        .expect("alice file update should be stored");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should succeed");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_user_file_update(&directed_messages, "client-2", "alice", "movie.mkv"),
        "destination room peer should receive the mover's current file metadata"
    );
    let file_index = directed_messages
        .iter()
        .position(|(client_id, message)| {
            client_id == "client-2"
                && has_user_file_update(
                    &[(client_id.clone(), message.clone())],
                    "client-2",
                    "alice",
                    "movie.mkv",
                )
        })
        .expect("destination room peer should receive file metadata");
    let room_update_index = directed_messages
        .iter()
        .position(|(client_id, message)| {
            if client_id != "client-2" {
                return false;
            }
            let ProtocolMessage::Set(payload) = message else {
                return false;
            };
            payload
                .set
                .user
                .as_ref()
                .and_then(|users| users.get("alice"))
                .is_some_and(|user| {
                    user.room
                        .as_ref()
                        .is_some_and(|room_ref| room_ref.name == "room2")
                        && user.file.is_none()
                        && user.event.is_none()
                })
        })
        .expect("destination room peer should receive standalone room update");
    assert!(
        file_index < room_update_index,
        "isolated room switch should republish file metadata before destination room updates"
    );
}

#[test]
fn ready_updates_are_broadcast_to_room_members() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("ready update should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_ready_update(&directed_messages, "client-1", "alice", true),
        "sender should receive echoed ready update"
    );
    assert!(
        has_ready_update(&directed_messages, "client-2", "alice", true),
        "peer should receive ready update"
    );
}

#[test]
fn room_switch_preserves_and_republishes_readiness() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room2"},"version":"1.7.5"}}"#,
        )
        .expect("bob hello should establish session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("alice ready update should succeed");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert_eq!(runtime.user_ready("alice", "room2"), Some(true));
    assert!(
        has_ready_update(&directed_messages, "client-1", "alice", true)
            && has_ready_update(&directed_messages, "client-2", "alice", true),
        "room switch should preserve and immediately republish readiness in the destination room"
    );

    let outbound_lines = runtime
        .handle_line("client-2", r#"{"List":null}"#)
        .expect("list request should succeed");
    let response = decode_message_line(&outbound_lines[0]).expect("list response should decode");
    let ProtocolMessage::List(payload) = response else {
        panic!("expected list response");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("expected room snapshot list");
    };
    assert_eq!(rooms["room2"]["alice"].is_ready, Some(true));
}

#[test]
fn ready_update_defaults_manually_initiated_false_when_omitted() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"ready":{"isReady":true}}}"#)
        .expect("ready update should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);
    let manually_initiated = directed_messages
        .iter()
        .find_map(|(_, message)| {
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            payload
                .set
                .ready
                .as_ref()
                .and_then(|ready| ready.manually_initiated)
        })
        .expect("ready update should include manuallyInitiated");

    assert!(
        !manually_initiated,
        "Python server defaults missing manuallyInitiated to false"
    );
}

#[test]
fn controller_can_set_other_user_readiness_in_current_room() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"username":"bob","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("controller ready update should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    for recipient in ["client-1", "client-2"] {
        assert!(
            directed_messages.iter().any(|(client_id, message)| {
                if client_id != recipient {
                    return false;
                }
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.ready.as_ref().is_some_and(|ready| {
                            ready.username.as_deref() == Some("bob")
                                && ready.is_ready == Some(true)
                                && ready.set_by.as_deref() == Some("alice")
                        })
                )
            }),
            "room peer should receive bob readiness update setBy alice"
        );
    }
    assert_eq!(runtime.user_ready("bob", "room1"), Some(true));
}

#[test]
fn controller_ready_update_sends_legacy_chat_only_to_clients_without_set_others_readiness() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"setOthersReadiness":true}}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.1"}}"#,
        )
        .expect("bob hello should establish session");
    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"carol","room":{"name":"room1"},"version":"1.7.5","features":{"setOthersReadiness":true}}}"#,
        )
        .expect("carol hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"username":"bob","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("controller ready update should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);
    let chat_recipients: Vec<_> = directed_messages
        .iter()
        .filter_map(|(client_id, message)| match message {
            ProtocolMessage::Chat(payload) => Some((client_id.as_str(), &payload.chat)),
            _ => None,
        })
        .collect();

    assert_eq!(chat_recipients.len(), 1);
    assert_eq!(chat_recipients[0].0, "client-2");
    assert_eq!(
        chat_recipients[0].1,
        &ChatPayload::message("alice", "I have set bob as ready.")
    );
}

#[test]
fn non_controller_cannot_set_other_user_readiness_in_controlled_room() {
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    let controlled_room = controlled_room_name_for_test("room1", "ABC-123-456");
    let alice_hello = format!(
        r#"{{"Hello":{{"username":"alice","room":{{"name":"{controlled_room}"}},"version":"1.2.255"}}}}"#
    );
    let bob_hello = format!(
        r#"{{"Hello":{{"username":"bob","room":{{"name":"{controlled_room}"}},"version":"1.2.255"}}}}"#
    );
    runtime
        .handle_line("client-1", &alice_hello)
        .expect("alice hello should establish session");
    runtime
        .handle_line("client-2", &bob_hello)
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"ready":{"username":"bob","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("non-controller ready update should be ignored");

    assert!(
        directed_lines.is_empty(),
        "non-controller should not be allowed to change another user's readiness"
    );
    assert_eq!(runtime.user_ready("bob", &controlled_room), None);
}
