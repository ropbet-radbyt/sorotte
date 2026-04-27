use super::*;

#[test]
fn scripted_server_runtime_cross_room_ready_list_scenario_validates_list_snapshots() {
    let events =
        replay_server_runtime_scenario_fixture("server_runtime_cross_room_ready_list.jsonl")
            .expect("cross-room list scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 8);

    let pre_move_list_event = events
        .get(5)
        .expect("step 6 list request event should be present");
    assert_eq!(pre_move_list_event.client_id, "client-1");
    assert_eq!(pre_move_list_event.outbound_lines.len(), 1);
    assert_eq!(pre_move_list_event.outbound_lines[0].client_id, "client-1");
    let pre_move_list = decode_message_line(&pre_move_list_event.outbound_lines[0].line)
        .expect("step 6 list response should decode");
    match pre_move_list {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room1 = rooms.get("room1").expect("room1 should be present");
                let room2 = rooms.get("room2").expect("room2 should be present");
                assert!(
                    room1
                        .get("alice")
                        .and_then(|entry| entry.is_ready)
                        .expect("alice should be in room1 with ready state")
                );
                assert_eq!(
                    room1.get("alice").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
                assert!(
                    room1
                        .get("carol")
                        .and_then(|entry| entry.is_ready)
                        .expect("carol should be in room1 with ready state")
                );
                assert_eq!(
                    room1.get("carol").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
                assert_eq!(
                    room2.get("bob").and_then(|entry| entry.is_ready),
                    None,
                    "bob should be in room2 with unknown ready state"
                );
                assert_eq!(
                    room2.get("bob").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
            }
            other => panic!("expected list room snapshot at step 6, got {other:?}"),
        },
        other => panic!("expected list response at step 6, got {}", other.kind()),
    }

    let post_move_list_event = events
        .get(7)
        .expect("step 8 list request event should be present");
    assert_eq!(post_move_list_event.client_id, "client-3");
    assert_eq!(post_move_list_event.outbound_lines.len(), 1);
    assert_eq!(post_move_list_event.outbound_lines[0].client_id, "client-3");
    let post_move_list = decode_message_line(&post_move_list_event.outbound_lines[0].line)
        .expect("step 8 list response should decode");
    match post_move_list {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                assert!(
                    !rooms.contains_key("room2"),
                    "room2 should be absent after bob moved to room1"
                );
                let room1 = rooms.get("room1").expect("room1 should be present");
                assert!(
                    room1
                        .get("alice")
                        .and_then(|entry| entry.is_ready)
                        .expect("alice should be in room1 with ready state")
                );
                assert_eq!(
                    room1.get("alice").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
                assert_eq!(
                    room1.get("bob").and_then(|entry| entry.is_ready),
                    None,
                    "bob should be in room1 with unknown ready state"
                );
                assert_eq!(
                    room1.get("bob").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
                assert!(
                    room1
                        .get("carol")
                        .and_then(|entry| entry.is_ready)
                        .expect("carol should be in room1 with ready state")
                );
                assert_eq!(
                    room1.get("carol").and_then(|entry| entry.file.as_ref()),
                    Some(&json!({})),
                    "legacy list snapshots keep empty file objects for no-file users"
                );
            }
            other => panic!("expected list room snapshot at step 8, got {other:?}"),
        },
        other => panic!("expected list response at step 8, got {}", other.kind()),
    }
}
