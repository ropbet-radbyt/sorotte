use super::*;

#[test]
fn scripted_server_runtime_controlled_room_permissions_scenario_validates_auth_and_playlist_corrections()
 {
    let events =
        replay_server_runtime_scenario_fixture("server_runtime_controlled_room_permissions.jsonl")
            .expect("controlled-room scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 11);

    let create_room_auth_event = events
        .get(2)
        .expect("step 3 controllerAuth event should be present");
    assert_eq!(create_room_auth_event.outbound_lines.len(), 1);
    assert_eq!(
        create_room_auth_event.outbound_lines[0].client_id,
        "client-1"
    );
    let create_room_message = decode_message_line(&create_room_auth_event.outbound_lines[0].line)
        .expect("step 3 response should decode");
    match create_room_message {
        ProtocolMessage::Set(payload) => {
            let new_controlled_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("step 3 should include newControlledRoom payload");
            assert_eq!(
                new_controlled_room.room_name.as_deref(),
                Some("+room1:CB39A19549E8")
            );
            assert_eq!(
                new_controlled_room
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("AB-123-456")
            );
        }
        other => panic!("expected set response at step 3, got {}", other.kind()),
    }

    let bob_playlist_attempt_event = events
        .get(5)
        .expect("step 6 bob playlist attempt event should be present");
    assert_eq!(bob_playlist_attempt_event.outbound_lines.len(), 2);
    assert!(
        bob_playlist_attempt_event
            .outbound_lines
            .iter()
            .all(|line| line.client_id == "client-2"),
        "non-controller correction should be directed only to sender"
    );
    let bob_correction_messages: Vec<_> = bob_playlist_attempt_event
        .outbound_lines
        .iter()
        .map(|line| decode_message_line(&line.line).expect("correction line should decode"))
        .collect();
    assert!(
        bob_correction_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist| {
                        playlist.files.is_empty()
                            && playlist.user.as_deref() == Some("+room1:CB39A19549E8")
                    }),
            _ => false,
        }),
        "step 6 should include playlistChange correction for controlled room state"
    );
    assert!(
        bob_correction_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index_value().is_none()
                            && playlist_index.user.as_deref() == Some("+room1:CB39A19549E8")
                    }),
            _ => false,
        }),
        "step 6 should include playlistIndex correction for controlled room state"
    );
    let controller_auth_success_event = events
        .get(6)
        .expect("step 7 controllerAuth success event should be present");
    assert_eq!(controller_auth_success_event.outbound_lines.len(), 2);
    assert!(
        controller_auth_success_event
            .outbound_lines
            .iter()
            .any(|line| line.client_id == "client-1")
            && controller_auth_success_event
                .outbound_lines
                .iter()
                .any(|line| line.client_id == "client-2"),
        "controller auth success should be broadcast to all clients"
    );
    for line in &controller_auth_success_event.outbound_lines {
        let message =
            decode_message_line(&line.line).expect("step 7 controller auth response should decode");
        match message {
            ProtocolMessage::Set(payload) => {
                let auth = payload
                    .set
                    .controller_auth
                    .as_ref()
                    .expect("step 7 response should include controllerAuth");
                assert_eq!(auth.user.as_deref(), Some("alice"));
                assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                assert_eq!(auth.success, Some(true));
            }
            other => panic!("expected set response at step 7, got {}", other.kind()),
        }
    }

    let list_event = events
        .get(10)
        .expect("step 11 list event should be present");
    assert_eq!(list_event.outbound_lines.len(), 1);
    assert_eq!(list_event.outbound_lines[0].client_id, "client-2");
    let list_message = decode_message_line(&list_event.outbound_lines[0].line)
        .expect("step 11 list response should decode");
    match list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room = rooms
                    .get("+room1:CB39A19549E8")
                    .expect("controlled room should be present in list");
                assert!(
                    room.get("alice")
                        .and_then(|entry| entry.controller)
                        .expect("alice should be listed in controlled room")
                );
                assert!(
                    !room
                        .get("bob")
                        .and_then(|entry| entry.controller)
                        .expect("bob should be listed in controlled room")
                );
            }
            other => panic!("expected list room snapshot at step 11, got {other:?}"),
        },
        other => panic!("expected list response at step 11, got {}", other.kind()),
    }
}
