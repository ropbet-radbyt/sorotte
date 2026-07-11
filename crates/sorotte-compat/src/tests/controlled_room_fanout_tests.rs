use super::*;

#[test]
fn scripted_server_runtime_controlled_room_invalid_password_scenario_validates_failures() {
    let events = replay_server_runtime_scenario_fixture(
        "server_runtime_controlled_room_invalid_password.jsonl",
    )
    .expect("controlled-room invalid-password scenario fixture should replay");
    assert_eq!(events.len(), 9);

    let invalid_plain_room_auth = events
        .get(2)
        .expect("step 3 invalid plain-room auth event should exist");
    assert_eq!(invalid_plain_room_auth.outbound_lines.len(), 2);
    for line in &invalid_plain_room_auth.outbound_lines {
        let message = decode_message_line(&line.line)
            .expect("step 3 invalid plain-room auth output should decode");
        match message {
            ProtocolMessage::Set(payload) => {
                let auth = payload
                    .set
                    .controller_auth
                    .as_ref()
                    .expect("step 3 should include controllerAuth");
                assert_eq!(auth.user.as_deref(), Some("alice"));
                assert_eq!(auth.room.as_deref(), Some("room1"));
                assert_eq!(auth.success, Some(false));
            }
            other => panic!("expected set response at step 3, got {}", other.kind()),
        }
    }

    let create_controlled_room = events
        .get(3)
        .expect("step 4 controlled-room creation response should exist");
    assert_eq!(create_controlled_room.outbound_lines.len(), 1);
    assert_eq!(
        create_controlled_room.outbound_lines[0].client_id,
        "client-1"
    );
    let create_controlled_room_message =
        decode_message_line(&create_controlled_room.outbound_lines[0].line)
            .expect("step 4 controlled-room creation output should decode");
    match create_controlled_room_message {
        ProtocolMessage::Set(payload) => {
            let new_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("step 4 should include newControlledRoom");
            assert_eq!(new_room.room_name.as_deref(), Some("+room1:CB39A19549E8"));
            assert_eq!(
                new_room
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("AB-123-456")
            );
        }
        other => panic!("expected set response at step 4, got {}", other.kind()),
    }

    let invalid_controlled_room_auth = events
        .get(6)
        .expect("step 7 invalid controlled-room auth should exist");
    assert_eq!(invalid_controlled_room_auth.outbound_lines.len(), 2);
    for line in &invalid_controlled_room_auth.outbound_lines {
        let message = decode_message_line(&line.line)
            .expect("step 7 invalid controlled-room auth output should decode");
        match message {
            ProtocolMessage::Set(payload) => {
                let auth = payload
                    .set
                    .controller_auth
                    .as_ref()
                    .expect("step 7 should include controllerAuth");
                assert_eq!(auth.user.as_deref(), Some("bob"));
                assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                assert_eq!(auth.success, Some(false));
            }
            other => panic!("expected set response at step 7, got {}", other.kind()),
        }
    }

    let wrong_but_valid_format_password = events
        .get(7)
        .expect("step 8 wrong valid-format password auth should exist");
    assert_eq!(wrong_but_valid_format_password.outbound_lines.len(), 2);
    for line in &wrong_but_valid_format_password.outbound_lines {
        let message = decode_message_line(&line.line)
            .expect("step 8 wrong valid-format auth output should decode");
        match message {
            ProtocolMessage::Set(payload) => {
                let auth = payload
                    .set
                    .controller_auth
                    .as_ref()
                    .expect("step 8 should include controllerAuth");
                assert_eq!(auth.user.as_deref(), Some("bob"));
                assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                assert_eq!(auth.success, Some(false));
            }
            other => panic!("expected set response at step 8, got {}", other.kind()),
        }
    }

    let list_event = events.get(8).expect("step 9 list response should exist");
    assert_eq!(list_event.outbound_lines.len(), 1);
    assert_eq!(list_event.outbound_lines[0].client_id, "client-2");
    let list_message = decode_message_line(&list_event.outbound_lines[0].line)
        .expect("step 9 list response should decode");
    match list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room = rooms
                    .get("+room1:CB39A19549E8")
                    .expect("controlled room should be listed");
                assert!(
                    !room
                        .get("alice")
                        .and_then(|entry| entry.controller)
                        .expect("alice should be listed")
                );
                assert!(
                    !room
                        .get("bob")
                        .and_then(|entry| entry.controller)
                        .expect("bob should be listed")
                );
            }
            other => panic!("expected list room snapshot at step 9, got {other:?}"),
        },
        other => panic!("expected list response at step 9, got {}", other.kind()),
    }
}

#[test]
fn scripted_server_runtime_controlled_room_state_forced_correction_scenario_validates_forced_pair()
{
    let events = replay_server_runtime_scenario_fixture(
        "server_runtime_controlled_room_state_forced_correction.jsonl",
    )
    .expect("controlled-room forced-correction scenario should replay");
    assert_eq!(events.len(), 8);

    let forced_correction_event = events
        .get(7)
        .expect("step 8 non-controller state correction should exist");
    assert_eq!(forced_correction_event.client_id, "client-2");
    assert_eq!(forced_correction_event.outbound_lines.len(), 2);
    assert!(
        forced_correction_event
            .outbound_lines
            .iter()
            .all(|line| line.client_id == "client-2"),
        "forced correction should be directed only to non-controller sender"
    );

    let first_message = decode_message_line(&forced_correction_event.outbound_lines[0].line)
        .expect("first forced correction message should decode");
    match first_message {
        ProtocolMessage::State(payload) => {
            let playstate = payload
                .state
                .playstate
                .as_ref()
                .expect("first correction should include playstate");
            assert_eq!(playstate.position, Some(0.0));
            assert_eq!(playstate.paused, Some(false));
            assert_eq!(playstate.do_seek, Some(false));
            assert_eq!(playstate.set_by.as_deref(), Some("bob"));
            assert_eq!(
                payload
                    .state
                    .ignoring_on_the_fly
                    .as_ref()
                    .and_then(|ignore| ignore.server),
                Some(1),
                "first correction should include server ignore counter 1"
            );
        }
        other => panic!(
            "expected state response at step 8 output 0, got {}",
            other.kind()
        ),
    }

    let second_message = decode_message_line(&forced_correction_event.outbound_lines[1].line)
        .expect("second forced correction message should decode");
    match second_message {
        ProtocolMessage::State(payload) => {
            let playstate = payload
                .state
                .playstate
                .as_ref()
                .expect("second correction should include playstate");
            assert_eq!(playstate.position, Some(0.0));
            assert_eq!(playstate.paused, Some(true));
            assert_eq!(playstate.do_seek, Some(true));
            assert_eq!(playstate.set_by, None);
            assert_eq!(
                payload
                    .state
                    .ignoring_on_the_fly
                    .as_ref()
                    .and_then(|ignore| ignore.server),
                Some(2),
                "second correction should include server ignore counter 2"
            );
        }
        other => panic!(
            "expected state response at step 8 output 1, got {}",
            other.kind()
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_permissions_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_permissions.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for controlled-room permissions scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_invalid_password_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_invalid_password.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for controlled-room invalid-password scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_state_forced_correction_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_state_forced_correction.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for controlled-room forced-correction scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_permissions_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_permissions.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for controlled-room permissions scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_invalid_password_scenario()
 {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_invalid_password.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for controlled-room invalid-password scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_state_forced_correction_scenario()
 {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_controlled_room_state_forced_correction.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for controlled-room forced-correction scenario should succeed, got: {err}"
        ),
    }
}
