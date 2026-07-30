use super::*;
use crate::{RoomPersistenceError, RoomPersistenceStore};
use rusqlite::{ErrorCode, params};
use sorotte_protocol::ListUserEntry;
use std::collections::BTreeMap;
use std::sync::Barrier;

#[derive(Debug, PartialEq)]
struct RawPersistedRoomRow {
    playlist: String,
    playlist_json: String,
    playlist_index: Option<i64>,
    position: f64,
    last_activity_at_seconds: f64,
    persistence_version: i64,
    owner_bucket: Option<String>,
    created_at_seconds: f64,
}

fn sqlite_full_baseline_room_state() -> PersistedRoomState {
    PersistedRoomState {
        files: vec![
            "baseline-episode-01.mkv".to_owned(),
            "baseline-episode-02.mkv".to_owned(),
            "baseline-episode-03.mkv".to_owned(),
        ],
        index: Some(1),
        position: 125.25,
        last_activity_at_seconds: 1_001.5,
        version: 41,
        owner_bucket: Some("quota:v1:sqlite-full-baseline".to_owned()),
        created_at_seconds: 901.25,
    }
}

fn sqlite_full_replacement_room_state(version: u64) -> PersistedRoomState {
    PersistedRoomState {
        files: (0..512)
            .map(|index| {
                format!(
                    "replacement-{index:04}-{}.mkv",
                    "deterministic-payload-".repeat(256)
                )
            })
            .collect(),
        index: Some(511),
        position: 987.75,
        last_activity_at_seconds: 2_002.5,
        version,
        owner_bucket: Some("quota:v1:sqlite-full-replacement".to_owned()),
        created_at_seconds: 902.5,
    }
}

fn raw_persisted_room_row(
    connection: &rusqlite::Connection,
    room_name: &str,
) -> RawPersistedRoomRow {
    connection
        .query_row(
            "SELECT playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                    persistenceVersion, ownerBucket, createdAt \
             FROM persistent_rooms WHERE name = ?1",
            [room_name],
            |row| {
                Ok(RawPersistedRoomRow {
                    playlist: row.get(0)?,
                    playlist_json: row.get(1)?,
                    playlist_index: row.get(2)?,
                    position: row.get(3)?,
                    last_activity_at_seconds: row.get(4)?,
                    persistence_version: row.get(5)?,
                    owner_bucket: row.get(6)?,
                    created_at_seconds: row.get(7)?,
                })
            },
        )
        .expect("the durable room row should remain queryable")
}

fn assert_sqlite_integrity_ok(connection: &rusqlite::Connection) {
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("SQLite integrity check should complete");
    assert_eq!(integrity, "ok");
}

fn assert_room_persistence_disk_full(error: &RoomPersistenceError, expected_path: &Path) {
    let RoomPersistenceError::Sqlite {
        action,
        path,
        source,
    } = error;
    assert_eq!(*action, "save persisted room");
    assert_eq!(path, expected_path);
    let rusqlite::Error::SqliteFailure(sqlite_error, _) = source else {
        panic!("expected a classified SQLite failure, got {source:?}");
    };
    assert_eq!(
        sqlite_error.code,
        ErrorCode::DiskFull,
        "the constrained production write must surface SQLITE_FULL"
    );
}

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
fn corrupt_quota_secret_lengths_fail_closed_without_overwriting_metadata() {
    let db_path = temporary_sqlite_path("corrupt-quota-secret-length");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room store should initialize");

    for length in [0_usize, 1, 31, 33, 1_024] {
        let corrupt_secret = vec![0xa5; length];
        let connection = store
            .connection("seed corrupt quota secret")
            .expect("room store connection should open");
        connection
            .execute(
                "INSERT OR REPLACE INTO persistence_metadata (key, value) \
                 VALUES ('quota-secret-v1', ?1)",
                params![corrupt_secret],
            )
            .expect("corrupt metadata fixture should be seedable");
        drop(connection);

        for attempt in 1..=2 {
            let error = store.load_or_create_quota_secret().unwrap_err();
            match error {
                RoomPersistenceError::Sqlite { action, .. } => assert_eq!(
                    action, "decode quota secret",
                    "attempt {attempt} for length {length} reported the wrong boundary"
                ),
            }
        }

        let connection = store
            .connection("inspect corrupt quota secret")
            .expect("room store connection should reopen");
        let persisted: Vec<u8> = connection
            .query_row(
                "SELECT value FROM persistence_metadata WHERE key = 'quota-secret-v1'",
                [],
                |row| row.get(0),
            )
            .expect("corrupt metadata row should remain observable");
        assert_eq!(
            persisted, corrupt_secret,
            "failed decoding must not silently replace durable identity"
        );
    }

    drop(store);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn concurrent_quota_secret_creation_converges_on_one_durable_value() {
    let db_path = temporary_sqlite_path("concurrent-quota-secret-creation");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room store should initialize");
    let creation_barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let store = store.clone();
            let creation_barrier = Arc::clone(&creation_barrier);
            std::thread::spawn(move || {
                store.load_or_create_quota_secret_with_before_create(|| {
                    creation_barrier.wait();
                })
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("quota-secret creation worker should not panic")
        })
        .collect::<Vec<_>>();

    let successful_secrets = results
        .into_iter()
        .map(|result| result.expect("both concurrent creators should load the durable secret"))
        .collect::<Vec<_>>();
    assert_eq!(
        successful_secrets.len(),
        2,
        "both concurrent callers should receive the durable secret"
    );
    assert_eq!(
        successful_secrets[0], successful_secrets[1],
        "concurrent creators must converge on the same durable value"
    );

    let connection = store
        .connection("inspect concurrently created quota secret")
        .expect("room store connection should reopen");
    let persisted: Vec<u8> = connection
        .query_row(
            "SELECT value FROM persistence_metadata WHERE key = 'quota-secret-v1'",
            [],
            |row| row.get(0),
        )
        .expect("the converged secret should be durable");
    assert_eq!(persisted, successful_secrets[0]);
    drop(connection);
    drop(store);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn playlist_json_migration_rolls_back_all_rows_after_later_failure() {
    let db_path = temporary_sqlite_path("playlist-json-migration-atomicity");
    let _ = fs::remove_file(&db_path);
    let store = RoomPersistenceStore::open(&db_path).expect("room store should initialize");
    let connection = store
        .connection("seed migration atomicity fault")
        .expect("room store connection should open");
    for (name, playlist) in [("first", "one.mkv"), ("second", "two.mkv")] {
        connection
            .execute(
                "INSERT INTO persistent_rooms \
                 (name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate) \
                 VALUES (?1, ?2, NULL, 0, 0, 0)",
                params![name, playlist],
            )
            .expect("legacy row should be seedable");
    }
    connection
        .execute_batch(
            "CREATE TABLE migration_fault (attempts INTEGER NOT NULL);
             INSERT INTO migration_fault (attempts) VALUES (0);
             CREATE TRIGGER fail_second_playlist_json_migration
             BEFORE UPDATE OF playlistJson ON persistent_rooms
             BEGIN
               UPDATE migration_fault SET attempts = attempts + 1;
               SELECT CASE
                 WHEN (SELECT attempts FROM migration_fault) = 2
                 THEN RAISE(ABORT, 'injected second migration failure')
               END;
             END;",
        )
        .expect("deterministic migration failpoint should install");
    drop(connection);

    let error = store
        .load_rooms()
        .expect_err("the second playlist migration write should fail");
    assert!(
        error
            .to_string()
            .contains("injected second migration failure"),
        "unexpected migration error: {error}"
    );

    let connection = store
        .connection("inspect interrupted migration")
        .expect("room store connection should reopen");
    let migrated_before_restart: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM persistent_rooms WHERE playlistJson IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("partial migration count should be readable");
    assert_eq!(
        migrated_before_restart, 0,
        "a failed playlist migration must roll back every row"
    );
    connection
        .execute_batch("DROP TRIGGER fail_second_playlist_json_migration;")
        .expect("failpoint should be removable before restart");
    drop(connection);

    let restarted_store =
        RoomPersistenceStore::open(&db_path).expect("interrupted store should reopen");
    let rooms = restarted_store
        .load_rooms()
        .expect("a retry without the failpoint should finish migration");
    assert_eq!(rooms["first"].files, vec!["one.mkv".to_owned()]);
    assert_eq!(rooms["second"].files, vec!["two.mkv".to_owned()]);
    let connection = restarted_store
        .connection("inspect recovered migration")
        .expect("recovered store connection should open");
    let migrated_after_restart: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM persistent_rooms WHERE playlistJson IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("recovered migration count should be readable");
    assert_eq!(migrated_after_restart, 2);

    drop(connection);
    drop(restarted_store);
    drop(store);
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
fn room_persistence_sqlite_full_preserves_old_row_and_recovers_after_limit_lift() {
    let db_path = temporary_sqlite_path("room-persistence-sqlite-full-recovery");
    let _ = fs::remove_file(&db_path);
    let store =
        RoomPersistenceStore::open(&db_path).expect("room persistence schema should initialize");
    let connection = store
        .connection("test SQLITE_FULL durability")
        .expect("room persistence connection should open");
    let baseline = sqlite_full_baseline_room_state();
    store
        .save_room(&connection, "durable-room", &baseline)
        .expect("the baseline room should persist");
    let checkpoint_busy: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .expect("the baseline WAL should checkpoint");
    assert_eq!(
        checkpoint_busy, 0,
        "the baseline checkpoint must not be busy"
    );

    let durable_before_failure = raw_persisted_room_row(&connection, "durable-room");
    let page_count: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("the current SQLite page count should be queryable");
    connection
        .pragma_update(None, "max_page_count", page_count)
        .expect("the database page limit should be constrained to its current size");
    let constrained_page_count: i64 = connection
        .query_row("PRAGMA max_page_count", [], |row| row.get(0))
        .expect("the constrained SQLite page count should be queryable");
    assert_eq!(
        constrained_page_count, page_count,
        "the fixture must leave no page allocation headroom"
    );

    let replacement = sqlite_full_replacement_room_state(42);
    let error = store
        .save_room(&connection, "durable-room", &replacement)
        .expect_err("a materially larger replacement must exhaust the page limit");
    assert_room_persistence_disk_full(&error, &db_path);

    let durable_after_failure = raw_persisted_room_row(&connection, "durable-room");
    assert_eq!(
        durable_after_failure, durable_before_failure,
        "SQLITE_FULL must not leak any replacement playlist, index, scalar, or version column"
    );
    assert_sqlite_integrity_ok(&connection);

    let reopened =
        RoomPersistenceStore::open(&db_path).expect("the database should reopen after SQLITE_FULL");
    let recovered_old_rooms = reopened
        .load_rooms()
        .expect("the old durable snapshot should recover normally");
    assert_eq!(
        recovered_old_rooms.get("durable-room"),
        Some(&baseline),
        "normal recovery must expose the complete pre-failure room"
    );
    drop(reopened);

    connection
        .pragma_update(None, "max_page_count", page_count + 4_096)
        .expect("the artificial page limit should lift");
    store
        .save_room(&connection, "durable-room", &replacement)
        .expect("the newer room should persist after capacity returns");
    assert_sqlite_integrity_ok(&connection);
    drop(connection);

    let final_reopen = RoomPersistenceStore::open(&db_path)
        .expect("the recovered database should reopen normally");
    let recovered_new_rooms = final_reopen
        .load_rooms()
        .expect("the newer durable snapshot should reload");
    assert_eq!(
        recovered_new_rooms.get("durable-room"),
        Some(&replacement),
        "capacity restoration must permit complete forward progress"
    );
    let final_connection = final_reopen
        .connection("final SQLITE_FULL integrity check")
        .expect("the final inspection connection should open");
    assert_sqlite_integrity_ok(&final_connection);

    drop(final_connection);
    drop(final_reopen);
    drop(store);
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
    assert_eq!(
        runtime.room_playlist_state("permanent-room").index,
        Some(0),
        "legacy permanent-room placeholders begin with playlist index zero"
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
                            index.index_value() == Some(0)
                        })
                )
        }),
        "empty permanent-room playlists must preserve their explicit legacy index"
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

    let client_2_messages: Vec<_> = directed_messages
        .iter()
        .filter(|(recipient, _)| recipient == "client-2")
        .map(|(_, message)| message)
        .collect();
    let list_position = client_2_messages
        .iter()
        .position(|message| matches!(message, ProtocolMessage::List(_)))
        .expect("joining client should receive the persistent room list");
    let playlist_position = client_2_messages
        .iter()
        .position(|message| {
            matches!(
                message,
                ProtocolMessage::Set(payload) if payload.set.playlist_change.is_some()
            )
        })
        .expect("joining client should receive its playlist snapshot");
    let hello_position = client_2_messages
        .iter()
        .position(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("joining client should receive Hello");
    assert!(
        list_position < playlist_position && playlist_position < hello_position,
        "legacy persistent-room order is list, playlist snapshot, then Hello for the joiner"
    );
}

#[test]
fn persistent_room_switch_list_precedes_playlist_snapshot() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("client hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);
    let client_messages: Vec<_> = directed_messages
        .iter()
        .filter(|(recipient, _)| recipient == "client-1")
        .map(|(_, message)| message)
        .collect();
    let list_position = client_messages
        .iter()
        .position(|message| matches!(message, ProtocolMessage::List(_)))
        .expect("switching client should receive a persistent room list");
    let playlist_position = client_messages
        .iter()
        .position(|message| {
            matches!(
                message,
                ProtocolMessage::Set(payload) if payload.set.playlist_change.is_some()
            )
        })
        .expect("switching client should receive its destination playlist snapshot");
    let playlist_index_position = client_messages
        .iter()
        .position(|message| {
            matches!(
                message,
                ProtocolMessage::Set(payload) if payload.set.playlist_index.is_some()
            )
        })
        .expect("switching client should receive its destination playlist index");
    assert!(
        list_position < playlist_position && playlist_position < playlist_index_position,
        "legacy persistent-room switch order is list before playlist and index snapshots"
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

#[test]
fn switching_room_database_replaces_the_previous_snapshot_without_copying_it() {
    let first_db = temporary_sqlite_path("persistent-room-db-switch-first");
    let second_db = temporary_sqlite_path("persistent-room-db-switch-second");
    let _ = fs::remove_file(&first_db);
    let _ = fs::remove_file(&second_db);

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .set_persistent_rooms_db_path(Some(first_db.clone()))
        .expect("first room database should initialize");
    runtime
        .handle_line(
            "owner",
            r#"{"Hello":{"username":"alice","room":{"name":"private-room"},"version":"1.7.5","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("owner should join the first database room");
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["private-episode.mkv"]}}}"#,
        )
        .expect("private playlist should persist in the first database");
    runtime
        .handle_line_fanout("owner", r#"{"Set":{"room":{"name":"lobby"}}}"#)
        .expect("owner should leave the private room");
    runtime
        .flush_persistence()
        .expect("first database state should be durable");

    runtime
        .set_persistent_rooms_db_path(Some(second_db.clone()))
        .expect("replacement empty room database should initialize");

    let list_rooms = decode_single_list_rooms(
        runtime
            .handle_line("owner", r#"{"List":null}"#)
            .expect("GUI room list should succeed after database replacement"),
    );
    assert!(
        !list_rooms.contains_key("private-room"),
        "replacing the database must not expose rooms retained from the previous snapshot"
    );

    let joined = decode_directed_lines(
        &runtime
            .handle_line_fanout(
                "reader",
                r#"{"Hello":{"username":"bob","room":{"name":"private-room"},"version":"1.7.5"}}"#,
            )
            .expect("reader should join a fresh room in the replacement database"),
    );
    assert!(
        has_playlist_snapshot(&joined, "reader", &[]),
        "a room absent from the replacement database must not inherit the previous playlist"
    );

    runtime
        .collect_dispatch_at(131.0)
        .expect("occupied-room maintenance should succeed");
    runtime
        .flush_persistence()
        .expect("replacement database maintenance should flush");
    let replacement_rooms = RoomPersistenceStore::open(&second_db)
        .expect("replacement room store should open")
        .load_rooms()
        .expect("replacement room store should load");
    assert!(
        !replacement_rooms.contains_key("private-room"),
        "old private state must not be copied into the replacement database by a heartbeat"
    );

    drop(runtime);
    fs::remove_file(&first_db).expect("first temporary sqlite db should be removable");
    fs::remove_file(&second_db).expect("second temporary sqlite db should be removable");
}

#[test]
fn switching_room_database_rejects_an_occupied_persisted_room() {
    let first_db = temporary_sqlite_path("persistent-room-db-switch-busy-first");
    let second_db = temporary_sqlite_path("persistent-room-db-switch-busy-second");
    let _ = fs::remove_file(&first_db);
    let _ = fs::remove_file(&second_db);

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(first_db.clone()))
        .expect("first room database should initialize");
    runtime
        .handle_line(
            "owner",
            r#"{"Hello":{"username":"alice","room":{"name":"occupied-room"},"version":"1.7.5"}}"#,
        )
        .expect("owner should join");
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["first.mkv"]}}}"#,
        )
        .expect("occupied room should persist");
    runtime
        .flush_persistence()
        .expect("first database should flush");

    let error = runtime
        .set_persistent_rooms_db_path(Some(second_db.clone()))
        .expect_err("an occupied persisted room must fence database replacement");
    assert!(matches!(
        error,
        ServerRuntimeError::PersistentRoomDatabaseReconfigurationBusy(ref room)
            if room == "occupied-room"
    ));
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
        )
        .expect("the original runtime should remain usable");
    runtime
        .flush_persistence()
        .expect("the original persistence service should remain attached");
    let persisted = RoomPersistenceStore::open(&first_db)
        .expect("original database should reopen")
        .load_rooms()
        .expect("original database should load");
    assert_eq!(persisted["occupied-room"].files, vec!["second.mkv"]);

    drop(runtime);
    fs::remove_file(&first_db).expect("first temporary sqlite db should be removable");
    fs::remove_file(&second_db).expect("second temporary sqlite db should be removable");
}

#[test]
fn failed_room_database_reconfiguration_keeps_the_existing_persistence_service() {
    let db_path = temporary_sqlite_path("persistent-room-db-reconfigure-failure");
    let invalid_db_path = temporary_text_path("persistent-room-db-reconfigure-directory");
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_file(&invalid_db_path);
    let _ = fs::remove_dir_all(&invalid_db_path);
    fs::create_dir_all(&invalid_db_path).expect("invalid database directory fixture should exist");

    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("initial room database should initialize");
    runtime
        .handle_line(
            "owner",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .expect("owner should join");
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["first.mkv"]}}}"#,
        )
        .expect("initial playlist should persist");
    runtime
        .flush_persistence()
        .expect("initial database write should flush");

    runtime
        .set_persistent_rooms_db_path(Some(invalid_db_path.clone()))
        .expect_err("a directory cannot be opened as a SQLite database file");
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
        )
        .expect("runtime should continue after rejected reconfiguration");
    runtime
        .flush_persistence()
        .expect("the original persistence service should remain flushable");

    let persisted = RoomPersistenceStore::open(&db_path)
        .expect("original database should reopen")
        .load_rooms()
        .expect("original database should load");
    assert_eq!(
        persisted["room"].files,
        vec!["second.mkv".to_owned()],
        "a rejected reconfiguration must not detach the original durable store"
    );

    drop(runtime);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    fs::remove_dir_all(&invalid_db_path).expect("invalid database fixture should be removable");
}

#[test]
fn ipv4_mapped_ipv6_peer_cannot_bypass_persistent_room_identity_quota() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime.set_max_persistent_rooms(10);
    runtime.set_max_persistent_rooms_per_identity(1);
    runtime.set_persistent_room_creation_cooldown_seconds(0.0);

    for (client_id, username, room_name, peer_ip) in [
        ("ipv4", "alice", "room-v4", "192.0.2.44"),
        ("mapped", "bob", "room-v4-mapped", "::ffff:192.0.2.44"),
    ] {
        runtime
            .handle_line_fanout_with_transport_actions_for_peer(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room_name}"}},"version":"1.7.5"}}}}"#
                ),
                Some(peer_ip),
            )
            .expect("peer should connect");
    }

    runtime
        .handle_line_fanout(
            "ipv4",
            r#"{"Set":{"playlistChange":{"files":["first.mkv"]}}}"#,
        )
        .expect("first room should fit the identity quota");
    runtime
        .handle_line_fanout(
            "mapped",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
        )
        .expect("quota rejection should return a correction");

    assert!(
        runtime
            .room_playlist_state("room-v4-mapped")
            .files
            .is_empty(),
        "IPv4 and its mapped IPv6 representation must share one quota identity"
    );
}

#[test]
fn removing_permanent_room_configuration_removes_only_an_empty_placeholder() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_permanent_rooms(["retired-permanent-room"]);
    runtime
        .handle_line(
            "gui-client",
            r#"{"Hello":{"username":"alice","room":{"name":"lobby"},"version":"1.7.5","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("GUI client should connect");

    let configured_rooms = decode_single_list_rooms(
        runtime
            .handle_line("gui-client", r#"{"List":null}"#)
            .expect("configured room list should succeed"),
    );
    assert!(configured_rooms.contains_key("retired-permanent-room"));

    runtime.set_permanent_rooms(Vec::<String>::new());
    let reconfigured_rooms = decode_single_list_rooms(
        runtime
            .handle_line("gui-client", r#"{"List":null}"#)
            .expect("reconfigured room list should succeed"),
    );
    assert!(
        !reconfigured_rooms.contains_key("retired-permanent-room"),
        "removing a permanent room must remove its empty placeholder"
    );
}

#[test]
fn removing_permanent_room_configuration_retains_nonempty_persistent_state() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    runtime.set_permanent_rooms(["former-permanent-room"]);
    runtime
        .handle_line(
            "owner",
            r#"{"Hello":{"username":"alice","room":{"name":"former-permanent-room"},"version":"1.7.5","features":{"uiMode":"GUI"}}}"#,
        )
        .expect("owner should join");
    runtime
        .handle_line_fanout(
            "owner",
            r#"{"Set":{"playlistChange":{"files":["keep.mkv"]}}}"#,
        )
        .expect("playlist should update");
    runtime
        .handle_line_fanout("owner", r#"{"Set":{"room":{"name":"lobby"}}}"#)
        .expect("owner should leave the former permanent room");

    runtime.set_permanent_rooms(Vec::<String>::new());
    let rooms = decode_single_list_rooms(
        runtime
            .handle_line("owner", r#"{"List":null}"#)
            .expect("room list should succeed"),
    );
    assert!(
        rooms.contains_key("former-permanent-room"),
        "removing permanent status must retain a nonempty room as ordinary persistent state"
    );
    assert_eq!(
        runtime.room_playlist_state("former-permanent-room").files,
        vec!["keep.mkv".to_owned()]
    );
}
