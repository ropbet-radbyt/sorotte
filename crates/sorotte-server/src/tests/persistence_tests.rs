use super::*;
use crate::RoomPersistenceStore;
use rusqlite::params;
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

    drop(runtime);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persistent_playlist_json_roundtrips_newlines_carriage_returns_empty_items_and_unicode() {
    let db_path = temporary_sqlite_path("persistent-playlist-json-roundtrip");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room store should initialize");
    let connection = store
        .connection("test playlist JSON roundtrip")
        .expect("room store connection should open");
    let files = vec![
        "line\nbreak.mkv".to_owned(),
        "carriage\rreturn.mkv".to_owned(),
        String::new(),
        "   ".to_owned(),
        "雪だるま-☃.mkv".to_owned(),
    ];

    store
        .save_room(
            &connection,
            "room",
            &PersistedRoomState {
                files: files.clone(),
                index: Some(4),
                position: 12.0,
                last_activity_at_seconds: 123.5,
                version: 1,
                owner_bucket: None,
                created_at_seconds: 123.5,
            },
        )
        .expect("playlist should save as JSON");
    drop(connection);
    let rooms = store.load_rooms().expect("playlist should reload");

    assert_eq!(rooms["room"].files, files);
    assert_eq!(rooms["room"].index, Some(4));
    assert_eq!(rooms["room"].last_activity_at_seconds, 123.5);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persistent_room_store_rejects_stale_saves_and_deletes_by_durable_version() {
    let db_path = temporary_sqlite_path("persistent-room-durable-version");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room persistence should initialize");
    let connection = store
        .connection("version test")
        .expect("room persistence connection should open");

    store
        .save_room(
            &connection,
            "room",
            &PersistedRoomState {
                files: vec!["new.mkv".to_owned()],
                index: Some(0),
                position: 50.0,
                last_activity_at_seconds: 50.0,
                version: 5,
                owner_bucket: Some("quota:v1:test".to_owned()),
                created_at_seconds: 10.0,
            },
        )
        .expect("new room version should persist");
    store
        .save_room(
            &connection,
            "room",
            &PersistedRoomState {
                files: vec!["stale.mkv".to_owned()],
                index: Some(0),
                position: 40.0,
                last_activity_at_seconds: 40.0,
                version: 4,
                owner_bucket: None,
                created_at_seconds: 5.0,
            },
        )
        .expect("stale upsert should be ignored without failing");
    store
        .delete_room(&connection, "room", 4)
        .expect("stale delete should be ignored without failing");

    let persisted = store.load_rooms().expect("room should remain loadable");
    let room = persisted.get("room").expect("newer room should remain");
    assert_eq!(room.files, vec!["new.mkv".to_owned()]);
    assert_eq!(room.position, 50.0);
    assert_eq!(room.version, 5);
    assert_eq!(room.owner_bucket.as_deref(), Some("quota:v1:test"));

    store
        .delete_room(&connection, "room", 6)
        .expect("newer delete should remove the row");
    assert!(
        store
            .load_rooms()
            .expect("empty room store should load")
            .is_empty()
    );

    drop(connection);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn legacy_multiline_playlist_rows_are_migrated_to_json_on_load() {
    let db_path = temporary_sqlite_path("persistent-playlist-json-migration");
    let _ = fs::remove_file(&db_path);
    let connection = Connection::open(&db_path).expect("legacy sqlite file should open");
    connection
        .execute(
            "CREATE TABLE persistent_rooms (\
             name STRING PRIMARY KEY, playlist STRING, playlistIndex INTEGER, \
             position REAL, lastSavedUpdate INTEGER)",
            [],
        )
        .expect("legacy schema should initialize");
    connection
        .execute(
            "INSERT INTO persistent_rooms \
             (name, playlist, playlistIndex, position, lastSavedUpdate) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["legacy-room", "one.mkv\ntwo.mkv", 1_i64, 3.0_f64, 0_i64],
        )
        .expect("legacy row should insert");
    drop(connection);

    let store = RoomPersistenceStore::open(&db_path).expect("legacy schema should migrate");
    let rooms = store.load_rooms().expect("legacy row should load");
    assert_eq!(
        rooms["legacy-room"].files,
        vec!["one.mkv".to_owned(), "two.mkv".to_owned()]
    );

    let connection = store
        .connection("inspect migrated playlist JSON")
        .expect("migrated store should reopen");
    let playlist_json: String = connection
        .query_row(
            "SELECT playlistJson FROM persistent_rooms WHERE name = ?1",
            params!["legacy-room"],
            |row| row.get(0),
        )
        .expect("migrated row should contain JSON");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&playlist_json)
            .expect("migrated playlist JSON should decode"),
        vec!["one.mkv".to_owned(), "two.mkv".to_owned()]
    );

    drop(connection);
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(500.0));
    runtime.set_persistent_room_inactivity_expiry_seconds(10.0);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("legacy row should load into the runtime");
    assert_eq!(
        runtime.persistent_room_last_activity_at.get("legacy-room"),
        Some(&500.0),
        "legacy zero timestamps should receive a safe startup-time fallback"
    );
    runtime
        .flush_persistence()
        .expect("legacy timestamp migration should reach SQLite");
    assert_eq!(
        store
            .load_rooms()
            .expect("migrated legacy row should reload")["legacy-room"]
            .last_activity_at_seconds,
        500.0,
        "the one-time fallback must be durable across future restarts"
    );
    drop(runtime);

    let mut restarted = ServerRuntime::with_persistent_rooms_enabled(true);
    restarted.set_time_now_override_seconds(Some(505.0));
    restarted.set_persistent_room_inactivity_expiry_seconds(10.0);
    restarted
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("migrated legacy row should survive another restart");
    assert_eq!(
        restarted
            .persistent_room_last_activity_at
            .get("legacy-room"),
        Some(&500.0),
        "a later restart must not renew the migrated grace period"
    );
    restarted
        .collect_dispatch_at(509.99)
        .expect("legacy room maintenance should succeed before expiry");
    assert!(
        restarted.room_playlists.contains_key("legacy-room"),
        "a legacy timestamp must not cause immediate expiry after upgrade"
    );
    restarted
        .collect_dispatch_at(510.0)
        .expect("migrated legacy room should expire at its durable deadline");
    assert!(!restarted.room_playlists.contains_key("legacy-room"));
    restarted
        .flush_persistence()
        .expect("legacy room expiry should reach SQLite");
    drop(restarted);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persisted_playlist_indices_are_normalized_during_load_and_repaired_on_disk() {
    let db_path = temporary_sqlite_path("persistent-playlist-index-normalization");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room store should initialize");
    let connection = store
        .connection("seed invalid playlist indices")
        .expect("room store connection should open");
    connection
        .execute(
            "INSERT INTO persistent_rooms \
             (name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "short",
                "only.mkv",
                "[\"only.mkv\"]",
                99_i64,
                0.0_f64,
                0_i64
            ],
        )
        .expect("out-of-range legacy state should be seedable");
    connection
        .execute(
            "INSERT INTO persistent_rooms \
             (name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["empty", "", "[]", -1_i64, 0.0_f64, 0_i64],
        )
        .expect("empty invalid legacy state should be seedable");
    drop(connection);

    let rooms = store
        .load_rooms()
        .expect("invalid indices should normalize");
    assert_eq!(rooms["short"].index, Some(0));
    assert_eq!(rooms["empty"].index, None);

    let connection = store
        .connection("inspect normalized playlist indices")
        .expect("room store connection should reopen");
    let short_index: Option<i64> = connection
        .query_row(
            "SELECT playlistIndex FROM persistent_rooms WHERE name = 'short'",
            [],
            |row| row.get(0),
        )
        .expect("short room should remain persisted");
    let empty_index: Option<i64> = connection
        .query_row(
            "SELECT playlistIndex FROM persistent_rooms WHERE name = 'empty'",
            [],
            |row| row.get(0),
        )
        .expect("empty room should remain persisted");
    assert_eq!(short_index, Some(0));
    assert_eq!(empty_index, None);

    drop(connection);
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
fn permanent_rooms_file_works_without_a_rooms_database() {
    let permanent_rooms_file = temporary_text_path("permanent-room-without-database");
    let _ = fs::remove_file(&permanent_rooms_file);
    fs::write(&permanent_rooms_file, "permanent-room\n")
        .expect("permanent rooms file should be writable");

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
        .expect("runtime should load the documented standalone permanent-rooms file");

    assert!(
        runtime.room_playlists.contains_key("permanent-room"),
        "a permanent-rooms file must create its configured empty room even without SQLite"
    );
    assert!(
        runtime.room_is_permanent("permanent-room"),
        "permanence must come from configuration rather than database presence"
    );
    runtime
        .cleanup_room_if_empty("permanent-room")
        .expect("standalone permanent room cleanup should succeed");
    assert!(
        runtime.room_playlists.contains_key("permanent-room"),
        "an empty standalone permanent room must remain available"
    );

    fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
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
        directed_messages.iter().any(|(client_id, message)| {
            client_id == "client-2"
                && matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|index| {
                            index.index_value().is_none()
                        })
                )
        }),
        "empty permanent-room playlists must restore a null index"
    );

    drop(runtime);
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

    drop(runtime);
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

    drop(runtime);
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
fn persistent_room_quota_bounds_client_created_durable_state_and_recovers_after_clear() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_max_persistent_rooms(1);
    runtime
        .handle_line(
            "client-a",
            r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#,
        )
        .expect("first room hello should succeed");
    runtime
        .handle_line(
            "client-b",
            r#"{"Hello":{"username":"bob","room":{"name":"room-b"},"version":"1.7.5"}}"#,
        )
        .expect("second room hello should succeed");
    runtime
        .handle_line_fanout(
            "client-a",
            r#"{"Set":{"playlistChange":{"files":["a.mkv"]}}}"#,
        )
        .expect("first durable room should fit quota");

    let rejected = runtime
        .handle_line_fanout(
            "client-b",
            r#"{"Set":{"playlistChange":{"files":["b.mkv"]}}}"#,
        )
        .expect("quota rejection should return the canonical snapshot");
    assert!(runtime.room_playlist_state("room-b").files.is_empty());
    assert!(
        decode_directed_lines(&rejected)
            .iter()
            .any(|(client_id, message)| {
                client_id == "client-b"
                    && matches!(
                        message,
                        ProtocolMessage::Set(payload)
                            if payload.set.playlist_change.as_ref().is_some_and(|playlist| {
                                playlist.files.is_empty()
                            })
                    )
            })
    );

    runtime
        .handle_line_fanout("client-a", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("clearing the first durable room should release quota");
    runtime
        .handle_line_fanout(
            "client-b",
            r#"{"Set":{"playlistChange":{"files":["b.mkv"]}}}"#,
        )
        .expect("second room should be accepted after quota is released");
    assert_eq!(
        runtime.room_playlist_state("room-b").files,
        vec!["b.mkv".to_owned()]
    );
}

#[test]
fn persistent_room_creation_is_limited_and_rate_limited_by_peer_ip() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime.set_max_persistent_rooms(10);
    runtime.set_max_persistent_rooms_per_identity(1);
    runtime.set_persistent_room_creation_cooldown_seconds(0.0);
    for (client_id, username, room_name, peer_ip) in [
        ("client-a", "alice", "room-a", "192.0.2.1"),
        ("client-b", "bob", "room-b", "192.0.2.1"),
        ("client-c", "carol", "room-c", "192.0.2.2"),
    ] {
        runtime
            .handle_line_fanout_with_transport_actions_for_peer(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                ),
                Some(peer_ip),
            )
            .expect("peer hello should establish a session");
    }
    runtime
        .handle_line_fanout(
            "client-a",
            r#"{"Set":{"playlistChange":{"files":["a.mkv"]}}}"#,
        )
        .expect("first room for an IP should be accepted");
    runtime
        .handle_line_fanout(
            "client-b",
            r#"{"Set":{"playlistChange":{"files":["b.mkv"]}}}"#,
        )
        .expect("same-IP quota rejection should return a correction");
    runtime
        .handle_line_fanout(
            "client-c",
            r#"{"Set":{"playlistChange":{"files":["c.mkv"]}}}"#,
        )
        .expect("a different IP should retain its own quota");
    assert!(runtime.room_playlist_state("room-b").files.is_empty());
    assert_eq!(
        runtime.room_playlist_state("room-c").files,
        vec!["c.mkv".to_owned()]
    );

    let mut rate_limited = ServerRuntime::with_persistent_rooms_enabled(true);
    rate_limited.set_time_now_override_seconds(Some(10.0));
    rate_limited.set_max_persistent_rooms_per_identity(10);
    rate_limited.set_persistent_room_creation_cooldown_seconds(5.0);
    for (client_id, room_name) in [("first", "first-room"), ("second", "second-room")] {
        rate_limited
            .handle_line_fanout_with_transport_actions_for_peer(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{client_id}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                ),
                Some("198.51.100.8"),
            )
            .expect("same-IP test peer should connect");
    }
    rate_limited
        .handle_line_fanout(
            "first",
            r#"{"Set":{"playlistChange":{"files":["first.mkv"]}}}"#,
        )
        .expect("first creation should be accepted");
    rate_limited
        .handle_line_fanout(
            "second",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
        )
        .expect("cooldown rejection should return a correction");
    assert!(
        rate_limited
            .room_playlist_state("second-room")
            .files
            .is_empty()
    );
    rate_limited.set_time_now_override_seconds(Some(15.0));
    rate_limited
        .handle_line_fanout(
            "second",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
        )
        .expect("creation should become eligible after cooldown");
    assert_eq!(
        rate_limited.room_playlist_state("second-room").files,
        vec!["second.mkv".to_owned()]
    );
}

#[test]
fn persistent_room_owner_quota_survives_restart_without_storing_raw_peer_ip() {
    let db_path = temporary_sqlite_path("persistent-room-owner-quota");
    let _ = fs::remove_file(&db_path);
    let peer_ip = "192.0.2.91";

    let mut first = ServerRuntime::with_persistent_rooms_enabled(true);
    first
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("room persistence should initialize");
    first.set_max_persistent_rooms(10);
    first.set_max_persistent_rooms_per_identity(1);
    first.set_persistent_room_creation_cooldown_seconds(0.0);
    first
        .handle_line_fanout_with_transport_actions_for_peer(
            "first",
            r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#,
            Some(peer_ip),
        )
        .expect("first peer should connect");
    first
        .handle_line_fanout("first", r#"{"Set":{"playlistChange":{"files":["a.mkv"]}}}"#)
        .expect("first durable room should be accepted");
    first
        .flush_persistence()
        .expect("first room should become durable");
    drop(first);

    let connection = rusqlite::Connection::open(&db_path).expect("database should open");
    let owner_bucket: String = connection
        .query_row(
            "SELECT ownerBucket FROM persistent_rooms WHERE name = 'room-a'",
            [],
            |row| row.get(0),
        )
        .expect("owner bucket should be durable");
    assert!(owner_bucket.starts_with("quota:v1:"));
    assert!(!owner_bucket.contains(peer_ip));
    drop(connection);

    let mut restarted = ServerRuntime::with_persistent_rooms_enabled(true);
    restarted
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("persisted rooms should reload");
    restarted.set_max_persistent_rooms(10);
    restarted.set_max_persistent_rooms_per_identity(1);
    restarted.set_persistent_room_creation_cooldown_seconds(0.0);
    for (client_id, username, room_name, address) in [
        ("same", "bob", "room-b", peer_ip),
        ("different", "carol", "room-c", "192.0.2.92"),
    ] {
        restarted
            .handle_line_fanout_with_transport_actions_for_peer(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                ),
                Some(address),
            )
            .expect("restart quota peer should connect");
    }
    restarted
        .handle_line_fanout("same", r#"{"Set":{"playlistChange":{"files":["b.mkv"]}}}"#)
        .expect("same-identity quota rejection should return a correction");
    restarted
        .handle_line_fanout(
            "different",
            r#"{"Set":{"playlistChange":{"files":["c.mkv"]}}}"#,
        )
        .expect("different identity should retain its own allocation");
    assert!(restarted.room_playlist_state("room-b").files.is_empty());
    assert_eq!(
        restarted.room_playlist_state("room-c").files,
        vec!["c.mkv".to_owned()]
    );

    drop(restarted);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persistent_room_creation_cooldown_survives_restart() {
    let db_path = temporary_sqlite_path("persistent-room-creation-cooldown-restart");
    let _ = fs::remove_file(&db_path);
    let peer_ip = "192.0.2.93";

    let mut first = ServerRuntime::with_persistent_rooms_enabled(true);
    first.set_time_now_override_seconds(Some(100.0));
    first
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("room persistence should initialize");
    first.set_max_persistent_rooms(10);
    first.set_max_persistent_rooms_per_identity(10);
    first.set_persistent_room_creation_cooldown_seconds(60.0);
    first
        .handle_line_fanout_with_transport_actions_for_peer(
            "first",
            r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#,
            Some(peer_ip),
        )
        .expect("first peer should connect");
    first
        .handle_line_fanout("first", r#"{"Set":{"playlistChange":{"files":["a.mkv"]}}}"#)
        .expect("first durable room should be accepted");
    first
        .flush_persistence()
        .expect("first room should become durable");
    drop(first);

    let mut restarted = ServerRuntime::with_persistent_rooms_enabled(true);
    restarted.set_time_now_override_seconds(Some(101.0));
    restarted
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("persisted rooms should reload");
    restarted.set_max_persistent_rooms(10);
    restarted.set_max_persistent_rooms_per_identity(10);
    restarted.set_persistent_room_creation_cooldown_seconds(60.0);
    restarted
        .handle_line_fanout_with_transport_actions_for_peer(
            "second",
            r#"{"Hello":{"username":"bob","room":{"name":"room-b"},"version":"1.7.5"}}"#,
            Some(peer_ip),
        )
        .expect("same peer should reconnect");
    restarted
        .handle_line_fanout(
            "second",
            r#"{"Set":{"playlistChange":{"files":["b.mkv"]}}}"#,
        )
        .expect("cooldown rejection should return a correction");

    assert!(
        restarted.room_playlist_state("room-b").files.is_empty(),
        "a restart must not reset a persisted creator's room-creation cooldown"
    );

    drop(restarted);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn empty_room_churn_does_not_allocate_persistence_versions_or_database_rows() {
    let db_path = temporary_sqlite_path("persistent-empty-room-churn");
    let _ = fs::remove_file(&db_path);
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("room persistence should initialize");

    for index in 0..1_000 {
        let client_id = format!("client-{index}");
        runtime
            .handle_line(
                &client_id,
                &format!(
                    r#"{{"Hello":{{"username":"user-{index}","room":{{"name":"room-{index}"}},"version":"1.7.5"}}}}"#
                ),
            )
            .expect("empty room client should connect");
        runtime
            .handle_transport_disconnect_fanout(&client_id)
            .expect("empty room client should disconnect");
    }
    runtime
        .flush_persistence()
        .expect("empty-room churn should have no unresolved persistence");

    assert!(runtime.persisted_room_names.is_empty());
    assert_eq!(runtime.next_room_persistence_version, 0);
    let connection = rusqlite::Connection::open(&db_path).expect("database should open");
    let row_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM persistent_rooms", [], |row| {
            row.get(0)
        })
        .expect("persisted room count should be queryable");
    assert_eq!(row_count, 0);

    drop(runtime);
    drop(connection);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn inactive_persistent_rooms_expire_on_disk_without_affecting_active_rooms() {
    let db_path = temporary_sqlite_path("persistent-room-inactivity-expiry");
    let _ = fs::remove_file(&db_path);
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("room persistence should initialize");
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime.set_persistent_room_creation_cooldown_seconds(0.0);
    runtime.set_persistent_room_inactivity_expiry_seconds(10.0);
    for (client_id, username, room_name) in [
        ("inactive-client", "alice", "inactive-room"),
        ("active-client", "bob", "active-room"),
    ] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                ),
            )
            .expect("persistent-room peer should connect");
        runtime
            .handle_line_fanout(
                client_id,
                &format!(r#"{{"Set":{{"playlistChange":{{"files":["{room_name}.mkv"]}}}}}}"#),
            )
            .expect("persistent playlist should be accepted");
    }
    runtime
        .handle_transport_disconnect_fanout("inactive-client")
        .expect("inactive room owner should disconnect cleanly");

    runtime
        .collect_dispatch_at(9.99)
        .expect("room should remain before expiry");
    assert!(runtime.room_playlists.contains_key("inactive-room"));
    runtime
        .collect_dispatch_at(10.0)
        .expect("maintenance should expire inactive durable state");
    assert!(!runtime.room_playlists.contains_key("inactive-room"));
    assert!(runtime.room_playlists.contains_key("active-room"));
    runtime
        .flush_persistence()
        .expect("expiry deletion should reach SQLite");

    let persisted = RoomPersistenceStore::open(&db_path)
        .expect("room store should reopen")
        .load_rooms()
        .expect("persisted rooms should load");
    assert!(!persisted.contains_key("inactive-room"));
    assert!(persisted.contains_key("active-room"));

    drop(runtime);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn persisted_activity_survives_restart_and_expires_on_first_maintenance() {
    let db_path = temporary_sqlite_path("persistent-room-restart-expiry");
    let _ = fs::remove_file(&db_path);
    {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime.set_persistent_room_creation_cooldown_seconds(0.0);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("room persistence should initialize");
        runtime.set_permanent_rooms(["permanent-room"]);

        for (client_id, username, room_name) in [
            ("expired-client", "alice", "expired-room"),
            ("active-client", "bob", "active-room"),
            ("permanent-client", "carol", "permanent-room"),
        ] {
            runtime
                .handle_line(
                    client_id,
                    &format!(
                        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                    ),
                )
                .expect("persistent-room peer should connect");
            runtime
                .handle_line_fanout(
                    client_id,
                    &format!(r#"{{"Set":{{"playlistChange":{{"files":["{room_name}.mkv"]}}}}}}"#),
                )
                .expect("persistent playlist should be accepted");
            runtime
                .handle_transport_disconnect_fanout(client_id)
                .expect("persistent-room peer should disconnect cleanly");
        }
        runtime
            .flush_persistence()
            .expect("activity timestamps should reach SQLite");
    }

    let stored_before_restart = RoomPersistenceStore::open(&db_path)
        .expect("room store should reopen")
        .load_rooms()
        .expect("persisted rooms should load");
    for room_name in ["expired-room", "active-room", "permanent-room"] {
        assert_eq!(
            stored_before_restart[room_name].last_activity_at_seconds, 100.0,
            "the runtime activity timestamp should be durable"
        );
    }

    let mut restarted = ServerRuntime::with_persistent_rooms_enabled(true);
    restarted.set_time_now_override_seconds(Some(200.0));
    restarted.set_persistent_room_inactivity_expiry_seconds(50.0);
    restarted.set_permanent_rooms(["permanent-room"]);
    restarted
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("restart should load persisted activity timestamps");
    assert_eq!(
        restarted
            .persistent_room_last_activity_at
            .get("expired-room"),
        Some(&100.0),
        "restart must retain the original activity time"
    );
    restarted
        .handle_line(
            "active-rejoin",
            r#"{"Hello":{"username":"dave","room":{"name":"active-room"},"version":"1.7.5"}}"#,
        )
        .expect("one old room should be active during startup maintenance");

    restarted
        .collect_dispatch_at(200.0)
        .expect("first post-restart maintenance should succeed");
    assert!(!restarted.room_playlists.contains_key("expired-room"));
    assert!(restarted.room_playlists.contains_key("active-room"));
    assert!(restarted.room_playlists.contains_key("permanent-room"));
    restarted
        .flush_persistence()
        .expect("restart expiry deletion should reach SQLite");

    let stored_after_restart = RoomPersistenceStore::open(&db_path)
        .expect("room store should reopen after expiry")
        .load_rooms()
        .expect("remaining rooms should load");
    assert!(!stored_after_restart.contains_key("expired-room"));
    assert!(stored_after_restart.contains_key("active-room"));
    assert!(stored_after_restart.contains_key("permanent-room"));

    drop(restarted);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn occupied_room_activity_heartbeat_survives_unclean_restart_without_media_mutation() {
    let db_path = temporary_sqlite_path("persistent-room-active-crash-heartbeat");
    let _ = fs::remove_file(&db_path);
    {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime.set_persistent_room_creation_cooldown_seconds(0.0);
        runtime.set_persistent_room_inactivity_expiry_seconds(60.0);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("room persistence should initialize");
        runtime
            .handle_line(
                "active-client",
                r#"{"Hello":{"username":"alice","room":{"name":"active-room"},"version":"1.7.5"}}"#,
            )
            .expect("active peer should connect");
        runtime
            .handle_line_fanout(
                "active-client",
                r#"{"Set":{"playlistChange":{"files":["episode.mkv"]}}}"#,
            )
            .expect("persistent playlist should be accepted");

        runtime
            .collect_dispatch_at(140.0)
            .expect("periodic maintenance should persist occupied-room activity");
        runtime
            .flush_persistence()
            .expect("activity heartbeat should reach SQLite before simulated crash");
        // Drop without a disconnect/room switch. This models the persistence state left by an
        // unclean process restart rather than the clean empty-room path.
    }

    let stored_after_crash = RoomPersistenceStore::open(&db_path)
        .expect("room store should reopen")
        .load_rooms()
        .expect("persisted room should load");
    assert_eq!(
        stored_after_crash["active-room"].last_activity_at_seconds, 140.0,
        "periodic occupied-room activity must be durable without a media mutation"
    );

    let mut restarted = ServerRuntime::with_persistent_rooms_enabled(true);
    restarted.set_time_now_override_seconds(Some(165.0));
    restarted.set_persistent_room_inactivity_expiry_seconds(60.0);
    restarted
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("restart should load the heartbeat timestamp");
    restarted
        .collect_dispatch_at(165.0)
        .expect("first maintenance after restart should succeed");
    assert!(
        restarted.room_playlists.contains_key("active-room"),
        "the active-at-crash room must not be deleted from its older media-mutation timestamp"
    );

    restarted
        .collect_dispatch_at(200.0)
        .expect("room should expire once the durable heartbeat itself ages out");
    assert!(!restarted.room_playlists.contains_key("active-room"));
    restarted
        .flush_persistence()
        .expect("expiry deletion should reach SQLite");

    drop(restarted);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
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
