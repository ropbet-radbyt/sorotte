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
                assert_eq!(alice.is_ready, Some(false));
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected List message, got {}", other.kind()),
    }
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
    assert_eq!(outbound_lines.len(), 4);
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
                    playstate.do_seek == Some(false) && playstate.paused == Some(true)
                })
            )
        }),
        "sender should receive baseline room sync state update"
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
fn set_room_seek_sync_state_counter_increments_without_ack() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");

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

    assert_eq!(directed_messages.len(), 5);

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
        has_playlist_snapshot(&directed_messages, "client-2", &[]),
        "new client should receive playlist snapshot before hello"
    );
    assert!(
        !has_user_event(&directed_messages, "client-2", "alice", "joined"),
        "new client should not receive synthetic joined snapshot for existing users"
    );
    assert!(
        has_room_sync_state_update(&directed_messages, "client-1", false),
        "existing room member should receive baseline room sync state update on peer join"
    );
    assert!(
        has_room_sync_state_update(&directed_messages, "client-2", false),
        "new room member should receive baseline room sync state update on join"
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
        features.get("maxUsernameLength").and_then(Value::as_u64),
        Some(12)
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
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should succeed");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
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
        .expect("ready while disabled should be ignored");
    assert!(
        ready_disabled.is_empty(),
        "ready update should be ignored when readiness is disabled"
    );

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
        !has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
        "old-room peer should not receive destination room update in isolateRooms mode"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
        "new-room peer should receive room update for moved user"
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
