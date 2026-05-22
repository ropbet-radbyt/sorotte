use super::*;

#[test]
fn scripted_server_runtime_cross_room_playlist_scoping_scenario_validates_room_scoped_playlist_snapshots_and_broadcasts()
 {
    let events = replay_server_runtime_scenario_fixture(CROSS_ROOM_PLAYLIST_SCOPING_SCENARIO)
        .expect(
            "cross-room playlist scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 9);

    let room2_join_event = events
        .get(6)
        .expect("step 7 hello event should be present for room2 snapshot assertions");
    assert_eq!(room2_join_event.client_id, "client-3");
    let mut saw_room2_playlist_change_snapshot = false;
    let mut saw_room2_playlist_index_snapshot = false;
    for outbound in &room2_join_event.outbound_lines {
        if outbound.client_id != "client-3" {
            continue;
        }
        let message =
            decode_message_line(&outbound.line).expect("step 7 outbound line should decode");
        if let ProtocolMessage::Set(payload) = message {
            if payload
                .set
                .playlist_change
                .as_ref()
                .is_some_and(|playlist_change| {
                    playlist_change.files == vec!["room2-ep1.mkv".to_owned()]
                })
            {
                saw_room2_playlist_change_snapshot = true;
            }
            if payload
                .set
                .playlist_index
                .as_ref()
                .is_some_and(|playlist_index| playlist_index.index == 0)
            {
                saw_room2_playlist_index_snapshot = true;
            }
        }
    }
    assert!(
        saw_room2_playlist_change_snapshot,
        "room2 join should include room2 playlist snapshot"
    );
    assert!(
        saw_room2_playlist_index_snapshot,
        "room2 join should include room2 playlist index snapshot"
    );

    let room_switch_event = events
        .get(7)
        .expect("step 8 room-change event should be present");
    assert_eq!(room_switch_event.client_id, "client-3");
    let mut saw_room1_playlist_change_snapshot = false;
    let mut saw_room1_playlist_index_snapshot = false;
    let mut saw_non_mover_playlist_message = false;
    for outbound in &room_switch_event.outbound_lines {
        let message =
            decode_message_line(&outbound.line).expect("step 8 outbound line should decode");
        if let ProtocolMessage::Set(payload) = message {
            if let Some(playlist_change) = payload.set.playlist_change.as_ref() {
                if outbound.client_id == "client-3"
                    && playlist_change.files
                        == vec!["room1-ep1.mkv".to_owned(), "room1-ep2.mkv".to_owned()]
                {
                    saw_room1_playlist_change_snapshot = true;
                } else {
                    saw_non_mover_playlist_message = true;
                }
            }
            if let Some(playlist_index) = payload.set.playlist_index.as_ref() {
                if outbound.client_id == "client-3" && playlist_index.index == 1 {
                    saw_room1_playlist_index_snapshot = true;
                } else {
                    saw_non_mover_playlist_message = true;
                }
            }
        }
    }
    assert!(
        saw_room1_playlist_change_snapshot,
        "room switch to room1 should include room1 playlist snapshot for mover"
    );
    assert!(
        saw_room1_playlist_index_snapshot,
        "room switch to room1 should include room1 playlist index snapshot for mover"
    );
    assert!(
        !saw_non_mover_playlist_message,
        "room switch playlist snapshots should be directed only to the mover"
    );

    let room2_followup_playlist_event = events
        .get(8)
        .expect("step 9 room2 playlist change event should be present");
    assert_eq!(room2_followup_playlist_event.client_id, "client-2");
    let playlist_change_recipients: Vec<_> = room2_followup_playlist_event
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let message = decode_message_line(&outbound.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            payload
                .set
                .playlist_change
                .as_ref()
                .filter(|playlist_change| playlist_change.files == vec!["room2-ep2.mkv".to_owned()])
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        playlist_change_recipients,
        vec!["client-2".to_owned()],
        "post-switch room2 playlist change should stay scoped to remaining room2 members"
    );
}
