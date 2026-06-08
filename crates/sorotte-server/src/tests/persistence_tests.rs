use super::*;
use sorotte_protocol::ListUserEntry;
use std::collections::BTreeMap;

fn decode_single_list_rooms(
    lines: Vec<String>,
) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
    assert_eq!(lines.len(), 1);
    let message = decode_message_line(&lines[0]).expect("list line should decode");
    match message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => rooms,
            other => panic!("expected room snapshot, got {other:?}"),
        },
        other => panic!("expected list message, got {}", other.kind()),
    }
}

#[test]
fn persistent_room_retains_playlist_index_and_position_after_empty_transition() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
        )
        .expect("initial hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("playlist index change should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":42.0,"paused":true,"doSeek":true}}}"#,
        )
        .expect("state update should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
        .expect("room switch should succeed");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
        )
        .expect("rejoin to persistent room should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    acknowledge_directed_state_counters(&mut runtime, &directed_messages);
    assert!(
        has_playlist_snapshot_with_user(
            &directed_messages,
            "client-2",
            &["episode1.mkv", "episode2.mkv"],
            "alice",
        ),
        "joining user should receive persisted playlist snapshot"
    );
    assert!(
        has_playlist_index_snapshot(&directed_messages, "client-2", 1),
        "joining user should receive persisted playlist index snapshot"
    );

    let periodic_lines = runtime
        .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
        .expect("periodic tick should succeed");
    let periodic_messages = decode_directed_lines(&periodic_lines);
    let client_state = periodic_messages
        .iter()
        .find(|(recipient, message)| {
            recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
        })
        .expect("rejoined client should receive periodic room state")
        .1
        .clone();
    let ProtocolMessage::State(state_payload) = client_state else {
        panic!("periodic room state should decode as state message");
    };
    let playstate = state_payload
        .state
        .playstate
        .expect("periodic room state should include playstate");
    assert_eq!(playstate.position, Some(42.0));
    assert_eq!(playstate.paused, Some(true));
}

#[test]
fn temporary_room_does_not_retain_playlist_or_position_when_empty() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"session-temp"},"version":"9.9.9"}}"#,
        )
        .expect("initial hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("playlist index change should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":37.0,"paused":true,"doSeek":true}}}"#,
        )
        .expect("state update should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
        .expect("room switch should succeed");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"session-temp"},"version":"9.9.9"}}"#,
        )
        .expect("rejoin to temporary room should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    acknowledge_directed_state_counters(&mut runtime, &directed_messages);
    assert!(
        has_playlist_snapshot(&directed_messages, "client-2", &[]),
        "temporary room should reset playlist state when emptied"
    );
    assert!(
        !has_playlist_index_snapshot(&directed_messages, "client-2", 1),
        "temporary room should not retain playlist index"
    );

    let periodic_lines = runtime
        .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
        .expect("periodic tick should succeed");
    let periodic_messages = decode_directed_lines(&periodic_lines);
    let client_state = periodic_messages
        .iter()
        .find(|(recipient, message)| {
            recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
        })
        .expect("rejoined client should receive periodic room state")
        .1
        .clone();
    let ProtocolMessage::State(state_payload) = client_state else {
        panic!("periodic room state should decode as state message");
    };
    let playstate = state_payload
        .state
        .playstate
        .expect("periodic room state should include playstate");
    assert_eq!(playstate.position, Some(0.0));
    assert_eq!(playstate.paused, Some(true));
}

#[test]
fn persistent_room_sqlite_reload_restores_playlist_index_and_position() {
    let db_path = temporary_sqlite_path("persistent-rooms-reload");
    let _ = fs::remove_file(&db_path);
    {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("runtime should initialize sqlite persistence");
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
            )
            .expect("initial hello should establish session");
        acknowledge_server_state_counter(&mut runtime, "client-1", 1);
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
            )
            .expect("playlist change should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
            .expect("playlist index change should succeed");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":24.0,"paused":true,"doSeek":true}}}"#,
            )
            .expect("state update should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
            .expect("room switch should persist empty-room state");
    }

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("runtime should load sqlite persistence snapshot");
    runtime.set_time_now_override_seconds(Some(0.0));
    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
        )
        .expect("hello should restore persisted room snapshot");
    let directed_messages = decode_directed_lines(&directed_lines);
    acknowledge_directed_state_counters(&mut runtime, &directed_messages);
    assert!(
        has_playlist_snapshot(
            &directed_messages,
            "client-2",
            &["episode1.mkv", "episode2.mkv"]
        ),
        "sqlite-backed reload should restore playlist snapshot"
    );
    assert!(
        has_playlist_index_snapshot(&directed_messages, "client-2", 1),
        "sqlite-backed reload should restore playlist index"
    );

    let periodic_lines = runtime
        .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
        .expect("periodic tick should succeed");
    let periodic_messages = decode_directed_lines(&periodic_lines);
    let client_state = periodic_messages
        .iter()
        .find(|(recipient, message)| {
            recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
        })
        .expect("reloaded client should receive periodic room state")
        .1
        .clone();
    let ProtocolMessage::State(state_payload) = client_state else {
        panic!("periodic room state should decode as state message");
    };
    let playstate = state_payload
        .state
        .playstate
        .expect("periodic room state should include playstate");
    assert_eq!(playstate.position, Some(24.0));
    assert_eq!(playstate.paused, Some(true));

    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn room_persistence_sets_busy_timeout_or_wal() {
    let db_path = temporary_sqlite_path("persistent-rooms-pragma");
    let _ = fs::remove_file(&db_path);
    let store = crate::RoomPersistenceStore::open(&db_path)
        .expect("room persistence should initialize sqlite pragmas");
    let connection = store
        .connection("test inspect pragmas")
        .expect("room persistence connection should open");

    let busy_timeout_ms: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("busy timeout pragma should query");
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode pragma should query");

    assert!(
        busy_timeout_ms >= 5_000,
        "room persistence connections should set a nonzero busy timeout"
    );
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    drop(connection);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn permanent_rooms_file_ignores_blank_lines() {
    assert_eq!(
        crate::parse_permanent_rooms_file(" room-a \n\n\t\nroom-b\n"),
        BTreeSet::from(["room-a".to_owned(), "room-b".to_owned()])
    );
}

#[test]
fn permanent_rooms_file_ignores_comment_lines() {
    assert_eq!(
        crate::parse_permanent_rooms_file("# comment\nroom-a\n  # another comment\nroom-b\n"),
        BTreeSet::from(["room-a".to_owned(), "room-b".to_owned()])
    );
}

#[test]
fn permanent_room_file_retains_empty_playlist_state_when_room_empties() {
    let db_path = temporary_sqlite_path("permanent-room-retention");
    let permanent_rooms_file = temporary_text_path("permanent-room-retention");
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_file(&permanent_rooms_file);
    fs::write(&permanent_rooms_file, "permanent-room\n")
        .expect("permanent rooms file should be writable");

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("runtime should initialize sqlite persistence");
    runtime
        .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
        .expect("runtime should load permanent rooms file");
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"permanent-room"},"version":"9.9.9"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"]}}}"#,
        )
        .expect("playlist change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("playlist index change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("playlist clear should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
        .expect("room switch should succeed");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"permanent-room"},"version":"9.9.9"}}"#,
        )
        .expect("bob hello should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    assert!(
        has_playlist_snapshot(&directed_messages, "client-2", &[]),
        "permanent room should preserve empty playlist snapshot"
    );
    assert!(
        has_playlist_index_snapshot(&directed_messages, "client-2", 0),
        "permanent room should preserve playlist index even when playlist is empty"
    );

    fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn gui_list_shows_dummy_entry_for_empty_permanent_room() {
    let db_path = temporary_sqlite_path("gui-dummy-room-list");
    let permanent_rooms_file = temporary_text_path("gui-dummy-room-list");
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_file(&permanent_rooms_file);
    fs::write(&permanent_rooms_file, "permanent-room\n")
        .expect("permanent rooms file should be writable");

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("runtime should initialize sqlite persistence");
    runtime
        .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
        .expect("runtime should load permanent rooms file");
    runtime
        .handle_line(
            "gui-client",
            r#"{"Hello":{"username":"gui-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("gui user hello should establish session");
    runtime
        .handle_line(
            "cli-client",
            r#"{"Hello":{"username":"cli-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
        )
        .expect("cli user hello should establish session");

    let gui_list_lines = runtime
        .handle_line("gui-client", r#"{"List":null}"#)
        .expect("gui list request should succeed");
    assert_eq!(gui_list_lines.len(), 1);
    let gui_list_message =
        decode_message_line(&gui_list_lines[0]).expect("gui list output should decode");
    let gui_rooms = match gui_list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => rooms,
            other => panic!("expected gui room snapshot, got {other:?}"),
        },
        other => panic!(
            "expected list message for gui request, got {}",
            other.kind()
        ),
    };
    let permanent_room = gui_rooms
        .get("permanent-room")
        .expect("gui list should include empty permanent room");
    assert_eq!(permanent_room.len(), 1);
    let (dummy_username, dummy_entry) = permanent_room
        .iter()
        .next()
        .expect("dummy entry should be present");
    assert_eq!(dummy_username, " ");
    assert_eq!(dummy_entry.position, Some(0.0));
    assert_eq!(dummy_entry.file.as_ref(), Some(&json!({})));
    assert_eq!(dummy_entry.controller, Some(false));
    assert_eq!(dummy_entry.is_ready, Some(true));
    assert_eq!(dummy_entry.features.as_ref(), Some(&json!([])));

    let cli_list_lines = runtime
        .handle_line("cli-client", r#"{"List":null}"#)
        .expect("cli list request should succeed");
    assert_eq!(cli_list_lines.len(), 1);
    let cli_list_message =
        decode_message_line(&cli_list_lines[0]).expect("cli list output should decode");
    let cli_rooms = match cli_list_message {
        ProtocolMessage::List(payload) => match payload.list {
            ListPayload::Rooms(rooms) => rooms,
            other => panic!("expected cli room snapshot, got {other:?}"),
        },
        other => panic!(
            "expected list message for cli request, got {}",
            other.kind()
        ),
    };
    assert!(
        !cli_rooms.contains_key("permanent-room"),
        "cli list should not include dummy empty permanent room"
    );

    fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn isolated_gui_list_shows_dummy_entry_for_empty_permanent_room() {
    let db_path = temporary_sqlite_path("isolated-gui-dummy-room-list");
    let permanent_rooms_file = temporary_text_path("isolated-gui-dummy-room-list");
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_file(&permanent_rooms_file);
    fs::write(&permanent_rooms_file, "permanent-room\n")
        .expect("permanent rooms file should be writable");

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_isolate_rooms(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("runtime should initialize sqlite persistence");
    runtime
        .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
        .expect("runtime should load permanent rooms file");
    runtime
        .handle_line(
            "gui-client",
            r#"{"Hello":{"username":"gui-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("gui user hello should establish session");
    runtime
        .handle_line(
            "other-client",
            r#"{"Hello":{"username":"other-user","room":{"name":"occupied-room"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("other room user hello should establish session");
    runtime
        .handle_line(
            "cli-client",
            r#"{"Hello":{"username":"cli-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
        )
        .expect("cli user hello should establish session");

    let gui_rooms = decode_single_list_rooms(
        runtime
            .handle_line("gui-client", r#"{"List":null}"#)
            .expect("gui list request should succeed"),
    );
    assert!(
        gui_rooms.contains_key("lobby"),
        "isolated gui list should include the user's current room"
    );
    assert!(
        !gui_rooms.contains_key("occupied-room"),
        "isolated gui list should not include occupied rooms outside the user's room"
    );
    let permanent_room = gui_rooms
        .get("permanent-room")
        .expect("isolated gui list should include empty permanent room dummy entry");
    assert_eq!(permanent_room.len(), 1);
    assert_eq!(
        permanent_room
            .get(" ")
            .and_then(|entry| entry.file.as_ref()),
        Some(&json!({}))
    );

    let cli_rooms = decode_single_list_rooms(
        runtime
            .handle_line("cli-client", r#"{"List":null}"#)
            .expect("cli list request should succeed"),
    );
    assert!(
        !cli_rooms.contains_key("permanent-room"),
        "isolated cli list should not include dummy empty permanent room"
    );

    fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persistent_list_updates_include_legacy_default_ui_mode_clients() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
        )
        .expect("client-1 hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("client-2 hello should establish session");
    let directed_messages = decode_directed_lines(&directed_lines);
    let list_recipients: BTreeSet<_> = directed_messages
        .iter()
        .filter_map(|(recipient, message)| {
            if matches!(message, ProtocolMessage::List(_)) {
                Some(recipient.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(
        list_recipients.contains("client-1"),
        "persistent list updates should include clients that advertise uiMode"
    );
    assert!(
        list_recipients.contains("client-2"),
        "legacy clients that omit features should receive Python-synthesized uiMode defaults"
    );
}

#[test]
fn isolated_persistent_list_updates_are_room_scoped() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "room1-client",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("room1 client hello should establish session");
    runtime
        .handle_line(
            "room2-client",
            r#"{"Hello":{"username":"bob","room":{"name":"room2"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("room2 client hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "joining-client",
            r#"{"Hello":{"username":"carol","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("joining client hello should establish session");
    let directed_messages = decode_directed_lines(&directed_lines);
    let list_recipients: BTreeSet<_> = directed_messages
        .iter()
        .filter_map(|(recipient, message)| {
            if matches!(message, ProtocolMessage::List(_)) {
                Some(recipient.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(
        list_recipients.contains("room1-client"),
        "isolated persistent list update should include existing clients in the join room"
    );
    assert!(
        list_recipients.contains("joining-client"),
        "isolated persistent list update should include the joining client"
    );
    assert!(
        !list_recipients.contains("room2-client"),
        "isolated persistent list update should not reach unrelated rooms"
    );
}

#[test]
fn isolated_persistent_disconnect_does_not_emit_list_update() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_isolate_rooms(true);
    runtime
        .handle_line(
            "leaving-client",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("leaving client hello should establish session");
    runtime
        .handle_line(
            "remaining-client",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("remaining client hello should establish session");

    let directed_lines = runtime
        .handle_transport_disconnect_fanout("leaving-client")
        .expect("transport disconnect should generate fanout");
    let directed_messages = decode_directed_lines(&directed_lines);
    assert!(
        has_user_event(&directed_messages, "remaining-client", "alice", "left"),
        "remaining room peer should receive the left event"
    );
    assert!(
        directed_messages
            .iter()
            .all(|(_, message)| !matches!(message, ProtocolMessage::List(_))),
        "isolated persistent disconnect should not emit GUI-only List refreshes"
    );
}

#[test]
fn persistent_timeout_disconnect_emits_ui_mode_scoped_list_update() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
        )
        .expect("client-1 hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
        )
        .expect("client-2 hello should establish session");
    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"charlie","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("client-3 hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-3", 1);

    runtime
        .advance_time_and_collect_fanout(crate::PROTOCOL_TIMEOUT_SECONDS - 2.0)
        .expect("time advance should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
        )
        .expect("client-1 heartbeat state should succeed");
    runtime
        .handle_line_fanout(
            "client-3",
            r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
        )
        .expect("client-3 heartbeat state should succeed");

    let timeout_lines = runtime
        .advance_time_and_collect_fanout(3.0)
        .expect("timeout-producing time advance should succeed");
    let timeout_messages = decode_directed_lines(&timeout_lines);
    let list_recipients: BTreeSet<_> = timeout_messages
        .iter()
        .filter_map(|(recipient, message)| {
            if matches!(message, ProtocolMessage::List(_)) {
                Some(recipient.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(
        list_recipients.contains("client-1"),
        "timeout list update should target connected clients that advertise uiMode"
    );
    assert!(
        list_recipients.contains("client-3"),
        "timeout list update should include legacy clients with Python-synthesized uiMode defaults"
    );
}
