use super::*;

#[test]
fn room_change_fanout_emits_global_room_update_and_playlist_snapshot() {
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
    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"carol","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("carol hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room change should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_user_room_update(&directed_messages, "client-1", "alice", "room2"),
        "sender should receive global user room update"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
        "old-room peer should receive global user room update"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
        "new-room peer should receive global user room update"
    );
    assert!(
        has_playlist_snapshot(&directed_messages, "client-1", &[]),
        "moved user should receive playlist snapshot after room switch"
    );
    assert!(
        !has_playlist_snapshot(&directed_messages, "client-3", &[]),
        "destination room peers should not receive direct playlist snapshot for mover"
    );
    assert!(
        has_room_sync_state_update(&directed_messages, "client-1", false),
        "moved user should receive baseline room sync state update"
    );
    assert!(
        has_room_sync_state_update(&directed_messages, "client-1", true),
        "moved user should receive seek room sync state update"
    );
}

#[test]
fn controller_auth_on_uncontrolled_room_returns_new_controlled_room_to_sender() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
        )
        .expect("controller auth on uncontrolled room should respond");
    assert_eq!(directed_lines.len(), 1);
    assert_eq!(directed_lines[0].client_id, "client-1");

    let message = decode_message_line(&directed_lines[0].line)
        .expect("new controlled room line should decode");
    let expected_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    match message {
        ProtocolMessage::Set(payload) => {
            let new_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("newControlledRoom payload should be present");
            assert_eq!(new_room.password.as_deref(), Some("AB-123-456"));
            assert_eq!(
                new_room.room_name.as_deref(),
                Some(expected_room_name.as_str())
            );
        }
        other => panic!("expected set response, got {}", other.kind()),
    }
}

#[test]
fn controller_auth_respects_runtime_configured_room_password_salt() {
    let mut runtime = ServerRuntime::with_room_password_salt("custom-salt");
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
        )
        .expect("controller auth on uncontrolled room should respond");
    assert_eq!(directed_lines.len(), 1);
    assert_eq!(directed_lines[0].client_id, "client-1");

    let message = decode_message_line(&directed_lines[0].line)
        .expect("new controlled room line should decode");
    let expected_room_name =
        controlled_room_name_for_salt_test("room1", "AB-123-456", "custom-salt");
    let default_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    match message {
        ProtocolMessage::Set(payload) => {
            let new_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("newControlledRoom payload should be present");
            assert_eq!(
                new_room.room_name.as_deref(),
                Some(expected_room_name.as_str())
            );
            assert_ne!(
                new_room.room_name.as_deref(),
                Some(default_room_name.as_str())
            );
        }
        other => panic!("expected set response, got {}", other.kind()),
    }
}

#[test]
fn controlled_room_playlist_updates_require_controller_auth() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
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
    runtime
        .handle_line_fanout(
            "client-1",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("alice room switch should succeed");
    runtime
        .handle_line_fanout(
            "client-2",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("bob room switch should succeed");

    let bob_change_attempt = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("bob playlist change attempt should respond");
    assert_eq!(bob_change_attempt.len(), 1);
    assert!(
        bob_change_attempt
            .iter()
            .all(|line| line.client_id == "client-2"),
        "non-controller correction should be directed only to sender"
    );
    let bob_messages: Vec<_> = bob_change_attempt
        .iter()
        .map(|line| decode_message_line(&line.line).expect("line should decode"))
        .collect();
    assert!(
        bob_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist| {
                        playlist.files.is_empty()
                            && playlist.user.as_deref() == Some(controlled_room_name.as_str())
                    },),
            _ => false,
        }),
        "non-controller should receive playlistChange correction for room state"
    );
    let alice_auth = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed");
    assert!(
        alice_auth.iter().any(|line| {
            decode_message_line(&line.line)
                .ok()
                .is_some_and(|message| match message {
                    ProtocolMessage::Set(payload) => payload
                        .set
                        .controller_auth
                        .as_ref()
                        .is_some_and(|auth| auth.success == Some(true)),
                    _ => false,
                })
        }),
        "controller auth success should be broadcast"
    );

    let alice_change = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("alice playlist change should succeed as controller");
    assert!(
        alice_change.iter().any(|line| line.client_id == "client-1")
            && alice_change.iter().any(|line| line.client_id == "client-2"),
        "controller playlist change should fan out to room peers"
    );
}
