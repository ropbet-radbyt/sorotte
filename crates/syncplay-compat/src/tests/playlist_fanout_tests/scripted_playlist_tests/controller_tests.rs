use super::*;

#[test]
fn scripted_server_runtime_playlist_controller_scenario_replays_and_fanout_decodes() {
    let events = replay_server_runtime_scenario_fixture("server_runtime_playlist_controller.jsonl")
        .expect("playlist/controller scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 7);

    let mut saw_playlist_change_broadcast = false;
    let mut saw_playlist_index_broadcast = false;
    let mut saw_controller_auth_broadcast = false;
    let mut saw_new_controlled_room_ignored = false;
    let mut saw_list_snapshot_with_both_users = false;

    for (step_index, event) in events.iter().enumerate() {
        if step_index == 5 {
            assert!(
                event.outbound_lines.is_empty(),
                "newControlledRoom client input should currently be ignored by runtime"
            );
            saw_new_controlled_room_ignored = true;
        }

        for outbound in &event.outbound_lines {
            let message = decode_message_line(&outbound.line)
                .expect("fanout output line should decode as protocol message");
            match message {
                ProtocolMessage::Set(payload) => {
                    if let Some(playlist_change) = payload.set.playlist_change.as_ref()
                        && playlist_change.user.as_deref() == Some("alice")
                        && playlist_change.files == vec!["episode1.mkv", "episode2.mkv"]
                    {
                        saw_playlist_change_broadcast = true;
                    }

                    if let Some(playlist_index) = payload.set.playlist_index.as_ref()
                        && playlist_index.user.as_deref() == Some("bob")
                        && playlist_index.index == 1
                    {
                        saw_playlist_index_broadcast = true;
                    }

                    if let Some(controller_auth) = payload.set.controller_auth.as_ref()
                        && controller_auth.user.as_deref() == Some("alice")
                        && controller_auth.room.as_deref() == Some("room1")
                        && controller_auth.success == Some(false)
                    {
                        saw_controller_auth_broadcast = true;
                    }
                }
                ProtocolMessage::List(payload) => {
                    if let ListPayload::Rooms(rooms) = payload.list {
                        let room = rooms.get("room1");
                        if room.is_some_and(|users| {
                            users.contains_key("alice") && users.contains_key("bob")
                        }) {
                            saw_list_snapshot_with_both_users = true;
                        }
                    }
                }
                ProtocolMessage::Hello(_)
                | ProtocolMessage::State(_)
                | ProtocolMessage::Chat(_)
                | ProtocolMessage::Error(_)
                | ProtocolMessage::Tls(_) => {}
            }
        }
    }

    assert!(
        saw_playlist_change_broadcast,
        "scenario should include playlistChange broadcast"
    );
    assert!(
        saw_playlist_index_broadcast,
        "scenario should include playlistIndex broadcast"
    );
    assert!(
        saw_controller_auth_broadcast,
        "scenario should include controllerAuth broadcast"
    );
    assert!(
        saw_new_controlled_room_ignored,
        "scenario should include ignored newControlledRoom client input"
    );
    assert!(
        saw_list_snapshot_with_both_users,
        "scenario should include list snapshot with both users in room1"
    );
}
