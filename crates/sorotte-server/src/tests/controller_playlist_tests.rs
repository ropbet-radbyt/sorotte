use super::*;

fn controller_auth_payloads(
    directed_lines: &[DirectedOutboundLine],
) -> Vec<(String, String, bool)> {
    directed_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            let auth = payload.set.controller_auth?;
            Some((
                line.client_id.clone(),
                auth.room?,
                auth.success.unwrap_or(false),
            ))
        })
        .collect()
}

fn playlist_change_payloads(
    directed_lines: &[DirectedOutboundLine],
) -> Vec<(String, PlaylistChangePayload)> {
    directed_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            Some((line.client_id.clone(), payload.set.playlist_change?))
        })
        .collect()
}

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
        has_room_sync_state_update(&directed_messages, "client-1", true),
        "moved user should receive seek room sync state update"
    );
}

#[test]
fn controller_auth_grants_requested_room_when_current_room_differs() {
    let controlled_room_name = controlled_room_name_for_test("target", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"lobby"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            &format!(
                r#"{{"Hello":{{"username":"bob","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
            ),
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed for requested room");

    assert!(runtime.user_is_room_controller("alice", &controlled_room_name));
    assert!(
        !runtime.user_is_room_controller("alice", "lobby"),
        "auth should not be granted for the sender's current room"
    );
    assert_eq!(
        controller_auth_payloads(&directed_lines),
        vec![("client-2".to_owned(), controlled_room_name, true)]
    );
}

#[test]
fn controller_auth_omitted_room_uses_current_room() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
                ),
            )
            .expect("hello should establish session");
    }

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("alice auth should use current room");

    assert!(runtime.user_is_room_controller("alice", &controlled_room_name));
    assert_eq!(
        controller_auth_payloads(&directed_lines),
        vec![
            ("client-1".to_owned(), controlled_room_name.clone(), true),
            ("client-2".to_owned(), controlled_room_name, true),
        ]
    );
}

#[test]
fn controller_auth_status_reports_requested_room() {
    let controlled_room_name = controlled_room_name_for_test("target", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"lobby"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            &format!(
                r#"{{"Hello":{{"username":"bob","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
            ),
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed for requested room");

    let auth_payloads = controller_auth_payloads(&directed_lines);
    assert!(
        !auth_payloads.is_empty(),
        "controllerAuth status should fan out to requested room peers"
    );
    assert!(
        auth_payloads
            .iter()
            .all(|(_, room, success)| room == &controlled_room_name && *success),
        "controllerAuth status should report the requested room"
    );
}

#[test]
fn controller_auth_on_uncontrolled_room_returns_new_controlled_room_to_sender() {
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
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
            assert_eq!(
                new_room
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("AB-123-456")
            );
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
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
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
    assert_eq!(bob_change_attempt.len(), 2);
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
    assert!(
        bob_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index_value().is_none()
                            && playlist_index.user.as_deref() == Some(controlled_room_name.as_str())
                    }),
            _ => false,
        }),
        "non-controller should receive playlistIndex correction for room state"
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

#[test]
fn plex_playlist_sidecar_is_sent_only_to_opted_in_sorotte_clients() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlexPlaylistUris":true}}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"python","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("python hello should establish session");

    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"playlistChange":{{"files":["Episode 11 [1080p].mkv"],"sorottePlexPlaylistUris":["{plex_uri}"]}}}}}}"#
            ),
        )
        .expect("playlist sidecar update should fan out");

    let payloads = playlist_change_payloads(&directed_lines);
    let alice_payload = payloads
        .iter()
        .find(|(client_id, _)| client_id == "client-1")
        .map(|(_, payload)| payload)
        .expect("alice should receive playlist update");
    let python_payload = payloads
        .iter()
        .find(|(client_id, _)| client_id == "client-2")
        .map(|(_, payload)| payload)
        .expect("python client should receive playlist update");

    assert_eq!(
        alice_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        alice_payload.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
    assert_eq!(
        python_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert!(!python_payload.extra.contains_key("sorottePlexPlaylistUris"));

    let late_join_lines = runtime
        .handle_line_fanout(
            "client-3",
            r#"{"Hello":{"username":"carol","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlexPlaylistUris":true}}}"#,
        )
        .expect("late Sorotte hello should receive room snapshot");
    let late_payload = playlist_change_payloads(&late_join_lines)
        .into_iter()
        .find(|(client_id, _)| client_id == "client-3")
        .map(|(_, payload)| payload)
        .expect("late Sorotte client should receive playlist snapshot");
    assert_eq!(
        late_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        late_payload.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
}

#[test]
fn invalid_playlist_change_is_rejected_with_current_room_playlist_correction() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let files: Vec<String> = (0..=DEFAULT_PLAYLIST_MAX_ITEMS)
        .map(|index| format!("episode-{index}.mkv"))
        .collect();
    let messages = runtime
        .handle_protocol_message_fanout(
            "client-1",
            ProtocolMessage::set(
                SetPayload::new().with_playlist_change(PlaylistChangePayload::new(files)),
            ),
        )
        .expect("invalid playlist should be rejected with correction");

    assert_eq!(
        runtime.room_playlist_state("room1").files,
        Vec::<String>::new(),
        "invalid playlist should not replace room playlist state"
    );
    assert!(
        messages.iter().any(|message| {
            message.client_id == "client-1"
                && matches!(
                    &message.message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_change.as_ref().is_some_and(|playlist| {
                            playlist.files.is_empty()
                                && playlist.user.as_deref() == Some("room1")
                        })
                )
        }),
        "sender should receive current playlist correction"
    );
}

#[test]
fn non_controller_playlist_index_update_receives_current_index_correction() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = format!(
            r#"{{"Hello":{{"username":"{username}","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
        );
        runtime
            .handle_line(client_id, &hello)
            .expect("hello should establish session");
    }
    runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("controller playlist change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("controller playlist index should succeed");

    let bob_index_attempt = runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("non-controller playlist index attempt should respond");
    let bob_messages = decode_directed_lines(&bob_index_attempt);

    assert!(
        bob_messages.iter().any(|(client_id, message)| {
            client_id == "client-2"
                && matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|playlist_index| {
                            playlist_index.index == 1
                                && playlist_index.user.as_deref()
                                    == Some(controlled_room_name.as_str())
                        })
                )
        }),
        "non-controller should receive current playlistIndex correction"
    );
}
