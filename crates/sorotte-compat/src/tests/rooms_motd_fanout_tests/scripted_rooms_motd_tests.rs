use super::*;

#[test]
fn scripted_server_runtime_username_conflict_scenario_uses_bounded_numbered_suffixes() {
    let events = replay_server_runtime_scenario_fixture("server_runtime_username_conflict.jsonl")
        .expect("username conflict scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 4);

    let second_hello_event = events
        .get(1)
        .expect("step 2 second hello event should be present");
    let second_hello_response = second_hello_event
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "client-2")
        .find_map(|line| {
            decode_message_line(&line.line)
                .ok()
                .and_then(|message| extract_hello_from_message(message).ok())
        })
        .expect("step 2 should include hello response for client-2");
    assert_eq!(second_hello_response.username, "alice_2");

    let third_hello_event = events
        .get(2)
        .expect("step 3 third hello event should be present");
    let third_hello_response = third_hello_event
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "client-3")
        .find_map(|line| {
            decode_message_line(&line.line)
                .ok()
                .and_then(|message| extract_hello_from_message(message).ok())
        })
        .expect("step 3 should include hello response for client-3");
    assert_eq!(third_hello_response.username, "alice_");

    let list_event = events.get(3).expect("step 4 list event should be present");
    let list_response = decode_message_line(
        &list_event
            .outbound_lines
            .first()
            .expect("step 4 should include one list response")
            .line,
    )
    .expect("step 4 list output should decode");
    match list_response {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room = rooms.get("room1").expect("room1 should be listed");
                assert!(room.contains_key("alice"));
                assert!(room.contains_key("alice_2"));
                assert!(room.contains_key("alice_"));
            }
            other => panic!("expected list room snapshot at step 4, got {other:?}"),
        },
        other => panic!("expected list response at step 4, got {}", other.kind()),
    }
}

#[test]
fn scripted_server_runtime_motd_template_scenario_applies_custom_template() {
    let steps = load_server_runtime_scenario_fixture(MOTD_TEMPLATE_SCENARIO)
        .expect("motd-template scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_motd_template(
        &steps,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    )
    .expect("motd-template scenario should replay through server runtime");
    assert_eq!(events.len(), 1);

    let hello_response = events[0]
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "client-1")
        .find_map(|line| {
            decode_message_line(&line.line)
                .ok()
                .and_then(|message| extract_hello_from_message(message).ok())
        })
        .expect("scenario should include hello response for client-1");
    let motd = hello_response
        .extra
        .get("motd")
        .and_then(Value::as_str)
        .expect("hello response should include motd");
    assert!(
        motd.starts_with("Compat MOTD latest="),
        "motd template output should include latest-version prefix"
    );
    assert!(
        !motd.contains("{latest_version}"),
        "motd template placeholder should be rendered"
    );
}

#[test]
fn scripted_server_runtime_motd_template_outdated_client_scenario_prepends_upgrade_warning() {
    let steps = load_server_runtime_scenario_fixture(MOTD_TEMPLATE_OUTDATED_SCENARIO)
        .expect("motd-template outdated-client scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_motd_template(
        &steps,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    )
    .expect("motd-template outdated-client scenario should replay through server runtime");
    assert_eq!(events.len(), 1);

    let hello_response = events[0]
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "client-1")
        .find_map(|line| {
            decode_message_line(&line.line)
                .ok()
                .and_then(|message| extract_hello_from_message(message).ok())
        })
        .expect("scenario should include hello response for client-1");
    let motd = hello_response
        .extra
        .get("motd")
        .and_then(Value::as_str)
        .expect("hello response should include motd");
    assert_eq!(motd, MOTD_TEMPLATE_OUTDATED_EXPECTED);
}

#[test]
fn scripted_server_runtime_persistent_rooms_notice_scenario_emits_notice_and_feature() {
    let steps = load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_NOTICE_SCENARIO)
        .expect("persistent-rooms notice scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
        .expect("persistent-rooms notice scenario should replay through server runtime");
    assert_eq!(events.len(), 1);

    let hello_response = events[0]
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "client-1")
        .find_map(|line| {
            decode_message_line(&line.line)
                .ok()
                .and_then(|message| extract_hello_from_message(message).ok())
        })
        .expect("scenario should include hello response for client-1");
    let persistent_rooms = hello_response
        .features
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|features| features.get("persistentRooms"))
        .and_then(Value::as_bool);
    assert_eq!(persistent_rooms, Some(true));
    let motd = hello_response
        .extra
        .get("motd")
        .and_then(Value::as_str)
        .expect("hello response should include motd");
    assert_eq!(motd, PERSISTENT_ROOMS_NOTICE);
}

#[test]
fn scripted_server_runtime_persistent_rooms_lifecycle_scenario_replays_saved_room_state() {
    let steps = load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_LIFECYCLE_SCENARIO)
        .expect("persistent-rooms lifecycle scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
        .expect("persistent-rooms lifecycle scenario should replay through server runtime");
    assert_eq!(events.len(), 7);

    let rejoin_event = events
        .get(5)
        .expect("step 6 should exist for rejoin snapshot assertions");
    let mut saw_playlist_snapshot = false;
    let mut saw_playlist_index_snapshot = false;
    for outbound in &rejoin_event.outbound_lines {
        if outbound.client_id != "client-2" {
            continue;
        }
        let message = decode_message_line(&outbound.line)
            .expect("rejoin event output should decode as protocol message");
        if let ProtocolMessage::Set(payload) = message {
            if payload
                .set
                .playlist_change
                .as_ref()
                .is_some_and(|playlist_change| {
                    playlist_change.files == vec!["episode1.mkv", "episode2.mkv"]
                        && playlist_change.user.as_deref() == Some("alice")
                })
            {
                saw_playlist_snapshot = true;
            }
            if payload
                .set
                .playlist_index
                .as_ref()
                .is_some_and(|playlist_index| {
                    playlist_index.index == 1 && playlist_index.user.as_deref() == Some("alice")
                })
            {
                saw_playlist_index_snapshot = true;
            }
        }
    }
    assert!(
        saw_playlist_snapshot,
        "rejoin should include persisted playlist snapshot"
    );
    assert!(
        saw_playlist_index_snapshot,
        "rejoin should include persisted playlist index snapshot"
    );

    let periodic_event = events
        .get(6)
        .expect("step 7 should exist for periodic-state assertion");
    let periodic_state = periodic_event
        .outbound_lines
        .iter()
        .find_map(|outbound| {
            if outbound.client_id != "client-2" {
                return None;
            }
            decode_message_line(&outbound.line)
                .ok()
                .and_then(|message| {
                    if let ProtocolMessage::State(payload) = message {
                        payload.state.playstate
                    } else {
                        None
                    }
                })
        })
        .expect("step 7 should include periodic state for rejoined client");
    assert_eq!(periodic_state.position, Some(42.0));
    assert_eq!(periodic_state.paused, Some(true));
}

#[test]
fn scripted_server_runtime_permanent_rooms_file_scenario_retains_room_and_gui_dummy_entry() {
    let steps = load_server_runtime_scenario_fixture(PERMANENT_ROOMS_FILE_SCENARIO)
        .expect("permanent-rooms-file scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_full_overrides(
        &steps,
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    )
    .expect("permanent-rooms-file scenario should replay through server runtime");
    assert_eq!(events.len(), 9);

    let list_event = events
        .get(7)
        .expect("step 8 should exist for GUI list assertions");
    let list_message = list_event
        .outbound_lines
        .iter()
        .find(|line| line.client_id == "client-2")
        .and_then(|line| decode_message_line(&line.line).ok())
        .expect("step 7 should include list response for GUI client");
    match list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let dummy_room = rooms
                    .get("permanent-room")
                    .expect("GUI list should include empty permanent room");
                let (dummy_username, dummy_entry) = dummy_room
                    .iter()
                    .next()
                    .expect("dummy entry should be present");
                assert_eq!(dummy_username, " ");
                assert_eq!(dummy_entry.features.as_ref(), Some(&json!([])));
                assert_eq!(dummy_entry.is_ready, Some(true));
            }
            other => panic!("expected list room snapshot at step 8, got {other:?}"),
        },
        other => panic!("expected list response at step 8, got {}", other.kind()),
    }

    let rejoin_event = events
        .get(8)
        .expect("step 9 should exist for permanent-room snapshot assertions");
    let mut saw_playlist_snapshot = false;
    let mut saw_playlist_index_snapshot = false;
    for outbound in &rejoin_event.outbound_lines {
        if outbound.client_id != "client-3" {
            continue;
        }
        let message = decode_message_line(&outbound.line)
            .expect("step 8 output should decode as protocol message");
        if let ProtocolMessage::Set(payload) = message {
            if payload
                .set
                .playlist_change
                .as_ref()
                .is_some_and(|playlist_change| playlist_change.files.is_empty())
            {
                saw_playlist_snapshot = true;
            }
            if payload
                .set
                .playlist_index
                .as_ref()
                .is_some_and(|playlist_index| {
                    playlist_index.index == 0 && playlist_index.user.as_deref() == Some("alice")
                })
            {
                saw_playlist_index_snapshot = true;
            }
        }
    }
    assert!(
        saw_playlist_snapshot,
        "rejoin should include empty playlist snapshot for permanent room"
    );
    assert!(
        saw_playlist_index_snapshot,
        "rejoin should include retained playlist index for permanent room"
    );
}

#[test]
fn scripted_server_runtime_persistent_rooms_timeout_list_updates_scenario_is_ui_mode_scoped() {
    let steps =
        load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO)
            .expect("persistent timeout-list-updates scenario fixture should load");
    let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
        .expect("persistent timeout-list-updates scenario should replay through server runtime");
    assert_eq!(events.len(), 7);

    let timeout_event = events
        .get(5)
        .expect("step 6 should exist for timeout list-update assertions");
    let mut saw_timeout_left_for_bob = false;
    let mut list_to_client_1 = false;
    let mut list_to_client_3 = false;
    for outbound in &timeout_event.outbound_lines {
        let message = decode_message_line(&outbound.line)
            .expect("step 6 output should decode as protocol message");
        match message {
            ProtocolMessage::Set(payload)
                if payload
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get("bob"))
                    .and_then(|user| user.event.as_ref())
                    .and_then(|event| event.get("left"))
                    .and_then(Value::as_bool)
                    == Some(true) =>
            {
                saw_timeout_left_for_bob = true;
            }
            ProtocolMessage::List(_) => {
                if outbound.client_id == "client-1" {
                    list_to_client_1 = true;
                }
                if outbound.client_id == "client-3" {
                    list_to_client_3 = true;
                }
            }
            _ => {}
        }
    }
    assert!(
        saw_timeout_left_for_bob,
        "step 6 should include timeout left event for bob"
    );
    assert!(
        list_to_client_1,
        "step 6 should include persistent list update for client that advertises uiMode"
    );
    assert!(
        list_to_client_3,
        "step 6 should include persistent list update for legacy clients with synthesized uiMode defaults"
    );

    let final_list_event = events
        .get(6)
        .expect("step 7 should exist for post-timeout list assertions");
    let list_message = final_list_event
        .outbound_lines
        .iter()
        .find(|line| line.client_id == "client-1")
        .and_then(|line| decode_message_line(&line.line).ok())
        .expect("step 7 should include list response for client-1");
    match list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => {
                let room = rooms.get("room1").expect("room1 should be listed");
                assert!(room.contains_key("alice"));
                assert!(room.contains_key("charlie"));
                assert!(!room.contains_key("bob"));
            }
            other => panic!("expected list room snapshot at step 7, got {other:?}"),
        },
        other => panic!("expected list response at step 7, got {}", other.kind()),
    }
}
