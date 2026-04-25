use super::*;

#[test]
fn scripted_server_runtime_playlist_room_switch_peer_transition_scoping_applies_room_change_before_peer_playlist_fanout()
 {
    let events = replay_server_runtime_scenario_fixture(
            PLAYLIST_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO,
        )
        .expect(
            "playlist room-switch peer-transition scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 10);

    let step5 = events
        .get(4)
        .expect("step 5 old-room peer playlistChange event should be present");
    let step5_playlist_change_recipients: Vec<_> = step5
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("alice")
                        && playlist_change.files == vec!["room1-after-bob-switch.mkv".to_owned()]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step5_playlist_change_recipients,
        vec!["client-1".to_owned()],
        "old-room peer playlistChange immediately after bob switches should not leak to bob"
    );

    let step6 = events
        .get(5)
        .expect("step 6 old-room peer playlistIndex event should be present");
    let step6_playlist_index_recipients: Vec<_> = step6
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("alice") && playlist_index.index == 0
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step6_playlist_index_recipients,
        vec!["client-1".to_owned()],
        "old-room peer playlistIndex immediately after bob switches should stay scoped to room1"
    );

    let step7 = events
        .get(6)
        .expect("step 7 destination-room peer playlistChange event should be present");
    let step7_playlist_change_recipients: Vec<_> = step7
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("carol")
                        && playlist_change.files
                            == vec![
                                "room2-peer-after-bob-switch.mkv".to_owned(),
                                "room2-peer-2.mkv".to_owned(),
                            ]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step7_playlist_change_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room peer playlistChange immediately after bob switches should include bob and carol in stable recipient order"
    );

    let step8 = events
        .get(7)
        .expect("step 8 destination-room peer playlistIndex event should be present");
    let step8_playlist_index_recipients: Vec<_> = step8
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("carol") && playlist_index.index == 1
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step8_playlist_index_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room peer playlistIndex immediately after bob switches should include bob and carol"
    );

    let step9 = events
        .get(8)
        .expect("step 9 moved-sender playlistChange event should be present");
    let step9_playlist_change_recipients: Vec<_> = step9
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("bob")
                        && playlist_change.files == vec!["room2-bob-after-switch.mkv".to_owned()]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step9_playlist_change_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "moved sender playlistChange should use destination-room membership immediately after peer playlist updates"
    );

    let step10 = events
        .get(9)
        .expect("step 10 moved-sender playlistIndex event should be present");
    let step10_playlist_index_recipients: Vec<_> = step10
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("bob") && playlist_index.index == 0
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step10_playlist_index_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "moved sender playlistIndex should remain scoped to destination-room members"
    );
}

#[test]
fn scripted_server_runtime_playlist_double_room_switch_scoping_uses_latest_room_membership_for_fanout()
 {
    let events = replay_server_runtime_scenario_fixture(
        PLAYLIST_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO,
    )
    .expect(
        "playlist double-room-switch scoping scenario fixture should replay through server runtime",
    );
    assert_eq!(events.len(), 11);

    let step6 = events
        .get(5)
        .expect("step 6 room2 playlistChange after bounce should be present");
    let step6_playlist_change_recipients: Vec<_> = step6
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("carol")
                        && playlist_change.files == vec!["room2-no-bob-after-bounce.mkv".to_owned()]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step6_playlist_change_recipients,
        vec!["client-3".to_owned()],
        "after bob bounces back to room1, room2 playlistChange should not leak to bob"
    );

    let step7 = events
        .get(6)
        .expect("step 7 room2 playlistIndex after bounce should be present");
    let step7_playlist_index_recipients: Vec<_> = step7
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("carol") && playlist_index.index == 0
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step7_playlist_index_recipients,
        vec!["client-3".to_owned()],
        "after bob bounces back to room1, room2 playlistIndex should not leak to bob"
    );

    let step8 = events
        .get(7)
        .expect("step 8 room1 peer playlistChange after bounce should be present");
    let step8_playlist_change_recipients: Vec<_> = step8
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("alice")
                        && playlist_change.files
                            == vec![
                                "room1-still-has-bob.mkv".to_owned(),
                                "room1-bob-back.mkv".to_owned(),
                            ]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step8_playlist_change_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "post-bounce room1 peer playlistChange should include bob again using final room membership"
    );

    let step9 = events
        .get(8)
        .expect("step 9 room1 peer playlistIndex after bounce should be present");
    let step9_playlist_index_recipients: Vec<_> = step9
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("alice") && playlist_index.index == 1
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step9_playlist_index_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "post-bounce room1 peer playlistIndex should include bob again using final room membership"
    );

    let step10 = events
        .get(9)
        .expect("step 10 bob playlistChange after bounce should be present");
    let step10_playlist_change_recipients: Vec<_> = step10
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| {
                    playlist_change.user.as_deref() == Some("bob")
                        && playlist_change.files == vec!["bob-back-room1.mkv".to_owned()]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step10_playlist_change_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "sender playlistChange after double room switch should use final room membership (room1)"
    );

    let step11 = events
        .get(10)
        .expect("step 11 bob playlistIndex after bounce should be present");
    let step11_playlist_index_recipients: Vec<_> = step11
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Set(payload) = decode_message_line(&outbound.line).ok()? else {
                return None;
            };
            payload
                .set
                .playlist_index
                .as_ref()
                .filter(|playlist_index| {
                    playlist_index.user.as_deref() == Some("bob") && playlist_index.index == 0
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step11_playlist_index_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "sender playlistIndex after double room switch should use final room membership (room1)"
    );
}
