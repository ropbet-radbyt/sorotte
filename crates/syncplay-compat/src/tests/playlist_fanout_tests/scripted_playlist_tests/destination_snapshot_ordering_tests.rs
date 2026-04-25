use super::*;

#[test]
fn scripted_server_runtime_playlist_room_switch_snapshot_then_destination_update_ordering_preserves_snapshot_before_followup_updates()
 {
    let events = replay_server_runtime_scenario_fixture(
            PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_UPDATE_ORDERING_SCENARIO,
        )
        .expect(
            "playlist room-switch snapshot/update-ordering scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 8);

    let step6 = events
        .get(5)
        .expect("step 6 room switch event should be present");
    assert_eq!(step6.client_id, "client-3");
    let mut saw_snapshot_playlist_change = false;
    let mut saw_snapshot_playlist_index = false;
    let mut saw_non_mover_playlist_message = false;
    let mut saw_followup_values_too_early = false;
    for outbound in &step6.outbound_lines {
        let Ok(ProtocolMessage::Set(payload)) = decode_message_line(&outbound.line) else {
            continue;
        };
        if let Some(playlist_change) = payload.set.playlist_change.as_ref() {
            if outbound.client_id == "client-3"
                && playlist_change.files == vec!["room2-initial-ep1.mkv".to_owned()]
            {
                saw_snapshot_playlist_change = true;
            } else if outbound.client_id != "client-3" {
                saw_non_mover_playlist_message = true;
            }
            if playlist_change.files
                == vec![
                    "room2-updated-after-switch.mkv".to_owned(),
                    "room2-updated-after-switch-2.mkv".to_owned(),
                ]
            {
                saw_followup_values_too_early = true;
            }
        }
        if let Some(playlist_index) = payload.set.playlist_index.as_ref() {
            if outbound.client_id == "client-3" && playlist_index.index == 0 {
                saw_snapshot_playlist_index = true;
            } else if outbound.client_id != "client-3" {
                saw_non_mover_playlist_message = true;
            }
            if playlist_index.index == 1 {
                saw_followup_values_too_early = true;
            }
        }
    }
    assert!(
        saw_snapshot_playlist_change,
        "room switch should deliver destination-room playlistChange snapshot to mover"
    );
    assert!(
        saw_snapshot_playlist_index,
        "room switch should deliver destination-room playlistIndex snapshot to mover"
    );
    assert!(
        !saw_non_mover_playlist_message,
        "room switch playlist snapshots should be directed only to the mover"
    );
    assert!(
        !saw_followup_values_too_early,
        "room switch event should not contain immediate follow-up destination playlist update values"
    );

    let step7 = events
        .get(6)
        .expect("step 7 destination follow-up playlistChange event should be present");
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
                    playlist_change.user.as_deref() == Some("bob")
                        && playlist_change.files
                            == vec![
                                "room2-updated-after-switch.mkv".to_owned(),
                                "room2-updated-after-switch-2.mkv".to_owned(),
                            ]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step7_playlist_change_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "immediate destination-room playlistChange after room switch should fan out to destination members only, after snapshot event"
    );

    let step8 = events
        .get(7)
        .expect("step 8 destination follow-up playlistIndex event should be present");
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
                    playlist_index.user.as_deref() == Some("bob") && playlist_index.index == 1
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step8_playlist_index_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "immediate destination-room playlistIndex after room switch should fan out to destination members only, after snapshot event"
    );
}

#[test]
fn scripted_server_runtime_playlist_room_switch_snapshot_then_destination_then_old_update_ordering_preserves_snapshot_and_followup_scopes()
 {
    let events = replay_server_runtime_scenario_fixture(
            PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_THEN_OLD_UPDATE_ORDERING_SCENARIO,
        )
        .expect(
            "playlist room-switch snapshot/destination-then-old-update-ordering scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 10);

    let step6 = events
        .get(5)
        .expect("step 6 room switch event should be present");
    assert_eq!(step6.client_id, "client-3");
    let mut saw_snapshot_playlist_change = false;
    let mut saw_snapshot_playlist_index = false;
    let mut saw_non_mover_playlist_message = false;
    let mut saw_followup_values_too_early = false;
    for outbound in &step6.outbound_lines {
        let Ok(ProtocolMessage::Set(payload)) = decode_message_line(&outbound.line) else {
            continue;
        };
        if let Some(playlist_change) = payload.set.playlist_change.as_ref() {
            if outbound.client_id == "client-3"
                && playlist_change.files == vec!["room2-initial-ep1.mkv".to_owned()]
            {
                saw_snapshot_playlist_change = true;
            } else if outbound.client_id != "client-3" {
                saw_non_mover_playlist_message = true;
            }
            if playlist_change.files
                == vec![
                    "room1-after-carol-switch.mkv".to_owned(),
                    "room1-after-carol-switch-2.mkv".to_owned(),
                ]
                || playlist_change.files
                    == vec![
                        "room2-after-carol-switch.mkv".to_owned(),
                        "room2-after-carol-switch-2.mkv".to_owned(),
                    ]
            {
                saw_followup_values_too_early = true;
            }
        }
        if let Some(playlist_index) = payload.set.playlist_index.as_ref() {
            if outbound.client_id == "client-3" && playlist_index.index == 0 {
                saw_snapshot_playlist_index = true;
            } else if outbound.client_id != "client-3" {
                saw_non_mover_playlist_message = true;
            }
            if playlist_index.index == 1 {
                saw_followup_values_too_early = true;
            }
        }
    }
    assert!(
        saw_snapshot_playlist_change,
        "room switch should deliver destination-room playlistChange snapshot to mover"
    );
    assert!(
        saw_snapshot_playlist_index,
        "room switch should deliver destination-room playlistIndex snapshot to mover"
    );
    assert!(
        !saw_non_mover_playlist_message,
        "room switch playlist snapshots should be directed only to the mover"
    );
    assert!(
        !saw_followup_values_too_early,
        "room switch event should not contain immediate old-room or destination-room follow-up playlist values"
    );

    let step7 = events
        .get(6)
        .expect("step 7 destination-room follow-up playlistChange event should be present");
    assert_eq!(step7.client_id, "client-2");
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
                    playlist_change.user.as_deref() == Some("bob")
                        && playlist_change.files
                            == vec![
                                "room2-after-carol-switch.mkv".to_owned(),
                                "room2-after-carol-switch-2.mkv".to_owned(),
                            ]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step7_playlist_change_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room follow-up playlistChange should fan out to destination members before old-room follow-up events"
    );

    let step8 = events
        .get(7)
        .expect("step 8 destination-room follow-up playlistIndex event should be present");
    assert_eq!(step8.client_id, "client-2");
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
                    playlist_index.user.as_deref() == Some("bob") && playlist_index.index == 1
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step8_playlist_index_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room follow-up playlistIndex should fan out to destination members before old-room follow-up events"
    );

    let step9 = events
        .get(8)
        .expect("step 9 old-room follow-up playlistChange event should be present");
    assert_eq!(step9.client_id, "client-1");
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
                    playlist_change.user.as_deref() == Some("alice")
                        && playlist_change.files
                            == vec![
                                "room1-after-carol-switch.mkv".to_owned(),
                                "room1-after-carol-switch-2.mkv".to_owned(),
                            ]
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step9_playlist_change_recipients,
        vec!["client-1".to_owned()],
        "old-room follow-up playlistChange should stay scoped to remaining old-room members after destination-room follow-up events"
    );

    let step10 = events
        .get(9)
        .expect("step 10 old-room follow-up playlistIndex event should be present");
    assert_eq!(step10.client_id, "client-1");
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
                    playlist_index.user.as_deref() == Some("alice") && playlist_index.index == 1
                })
                .map(|_| outbound.client_id.clone())
        })
        .collect();
    assert_eq!(
        step10_playlist_index_recipients,
        vec!["client-1".to_owned()],
        "old-room follow-up playlistIndex should stay scoped to remaining old-room members after destination-room follow-up events"
    );
}
