use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, ErrorCode};
use tokio::sync::broadcast;

use crate::{
    PersistedRoomState, RoomPersistenceError, RoomPersistenceService, RoomPersistenceStore,
    ServerPersistenceEffect,
};

#[cfg(windows)]
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::OpenOptionsExt,
};

const ROOM_NAME: &str = "platform-fault-room";

struct SqliteFixture {
    db_path: PathBuf,
}

impl SqliteFixture {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        Self {
            db_path: std::env::temp_dir().join(format!(
                "sorotte-{label}-{}-{unique}.sqlite3",
                std::process::id()
            )),
        }
    }

    fn displaced_path(&self) -> PathBuf {
        self.db_path.with_extension("sqlite3.displaced")
    }
}

impl Drop for SqliteFixture {
    fn drop(&mut self) {
        remove_path_if_present(&self.db_path);
        remove_sqlite_sidecars(&self.db_path);
        let displaced = self.displaced_path();
        remove_path_if_present(&displaced);
        remove_sqlite_sidecars(&displaced);
    }
}

fn remove_path_if_present(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn remove_sqlite_sidecars(db_path: &Path) {
    let path = db_path.to_string_lossy();
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = fs::remove_file(format!("{path}{suffix}"));
    }
}

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

fn raw_room_row(db_path: &Path) -> RawPersistedRoomRow {
    Connection::open(db_path)
        .expect("database should open for raw durable-state inspection")
        .query_row(
            "SELECT playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                    persistenceVersion, ownerBucket, createdAt \
             FROM persistent_rooms WHERE name = ?1",
            [ROOM_NAME],
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
        .expect("the complete persisted room row should remain queryable")
}

fn assert_integrity(db_path: &Path) {
    let connection =
        Connection::open(db_path).expect("database should reopen for its integrity check");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("SQLite should report an integrity result");
    assert_eq!(integrity, "ok");
}

fn baseline_state() -> PersistedRoomState {
    PersistedRoomState {
        files: vec![
            "baseline-first.mkv".to_owned(),
            "baseline-second.mkv".to_owned(),
        ],
        index: Some(1),
        position: 41.25,
        last_activity_at_seconds: 42.5,
        version: 41,
        owner_bucket: Some("platform-fault-owner-v1".to_owned()),
        created_at_seconds: 40.75,
    }
}

fn replacement_state() -> PersistedRoomState {
    PersistedRoomState {
        files: vec![
            "recovered-first.mkv".to_owned(),
            "recovered-second.mkv".to_owned(),
            "recovered-third.mkv".to_owned(),
        ],
        index: Some(2),
        position: 43.25,
        last_activity_at_seconds: 44.5,
        version: 42,
        owner_bucket: Some("platform-fault-owner-v2".to_owned()),
        created_at_seconds: 40.75,
    }
}

fn room_effect(state: &PersistedRoomState) -> ServerPersistenceEffect {
    ServerPersistenceEffect::SaveRoom {
        room_name: ROOM_NAME.to_owned(),
        files: state.files.clone(),
        playlist_index: state.index,
        position: state.position,
        last_activity_at_seconds: state.last_activity_at_seconds,
        owner_bucket: state.owner_bucket.clone(),
        created_at_seconds: state.created_at_seconds,
        version: state.version,
    }
}

fn seed_checkpointed_baseline(
    fixture: &SqliteFixture,
) -> (RoomPersistenceStore, RawPersistedRoomRow, Vec<u8>) {
    let store =
        RoomPersistenceStore::open(&fixture.db_path).expect("room persistence should initialize");
    let baseline = baseline_state();
    let connection = store
        .connection("seed platform fault baseline")
        .expect("baseline connection should open");
    store
        .save_room(&connection, ROOM_NAME, &baseline)
        .expect("complete baseline room should persist");
    let checkpoint_busy: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .expect("baseline WAL should checkpoint");
    assert_eq!(
        checkpoint_busy, 0,
        "the durable baseline checkpoint must not be busy"
    );
    drop(connection);

    let raw = raw_room_row(&fixture.db_path);
    assert_raw_row_matches_state(&raw, &baseline);
    assert_integrity(&fixture.db_path);
    let bytes = fs::read(&fixture.db_path).expect("checkpointed database bytes should be readable");
    (store, raw, bytes)
}

fn assert_raw_row_matches_state(row: &RawPersistedRoomRow, state: &PersistedRoomState) {
    assert_eq!(row.playlist, state.files.join("\n"));
    assert_eq!(
        row.playlist_json,
        serde_json::to_string(&state.files).expect("string playlist should serialize")
    );
    assert_eq!(row.playlist_index, state.index);
    assert_eq!(row.position, state.position);
    assert_eq!(row.last_activity_at_seconds, state.last_activity_at_seconds);
    assert_eq!(row.persistence_version, state.version as i64);
    assert_eq!(row.owner_bucket, state.owner_bucket);
    assert_eq!(row.created_at_seconds, state.created_at_seconds);
}

fn assert_worker_open_is_denied(store: RoomPersistenceStore, expected_path: &Path) {
    let (events, _) = broadcast::channel(8);
    let error = RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
        .expect_err("the host filesystem condition must deny the worker's production open");
    let RoomPersistenceError::Sqlite {
        action,
        path,
        source,
    } = error;
    assert_eq!(action, "connect persistence worker");
    assert_eq!(path, expected_path);
    let rusqlite::Error::SqliteFailure(sqlite_error, message) = source else {
        panic!("the worker open should retain SQLite's classified VFS failure");
    };
    assert_eq!(
        sqlite_error.code,
        ErrorCode::CannotOpen,
        "the host denial must reach SQLite as SQLITE_CANTOPEN"
    );
    assert!(
        message
            .as_deref()
            .is_some_and(|message| message.contains("unable to open database file")),
        "the SQLite VFS should retain concrete open-failure context"
    );
}

fn assert_worker_recovery(store: RoomPersistenceStore, db_path: &Path) {
    let replacement = replacement_state();
    let (events, _) = broadcast::channel(8);
    let service = RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
        .expect("the room worker should start after the host denial is removed");
    service.enqueue(room_effect(&replacement));
    assert!(
        service.flush(),
        "the recovered worker should durably acknowledge its replacement"
    );
    drop(service);

    let reopened =
        RoomPersistenceStore::open(db_path).expect("recovered persistence should reopen normally");
    assert_eq!(
        reopened
            .load_rooms()
            .expect("recovered room state should load")
            .get(ROOM_NAME),
        Some(&replacement)
    );
    let raw = raw_room_row(db_path);
    assert_raw_row_matches_state(&raw, &replacement);
    assert_integrity(db_path);
}

#[cfg(windows)]
fn bytes_from_exclusive_handle(file: &mut File) -> Vec<u8> {
    file.seek(SeekFrom::Start(0))
        .expect("exclusive database handle should seek");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .expect("exclusive database handle should read");
    bytes
}

#[cfg(windows)]
#[test]
fn room_persistence_windows_share_denial_preserves_and_recovers_durable_state() {
    const ERROR_SHARING_VIOLATION: i32 = 32;

    let fixture = SqliteFixture::new("room-platform-windows-share-denial");
    let (store, durable_before, checkpointed_bytes) = seed_checkpointed_baseline(&fixture);
    let rename_target = fixture.displaced_path();
    let mut exclusive = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&fixture.db_path)
        .expect("the closed checkpointed database should accept an exclusive host handle");
    assert_eq!(
        bytes_from_exclusive_handle(&mut exclusive),
        checkpointed_bytes
    );

    let rename_error = fs::rename(&fixture.db_path, &rename_target)
        .expect_err("a no-share Windows handle must block database rename");
    assert_eq!(
        rename_error.raw_os_error(),
        Some(ERROR_SHARING_VIOLATION),
        "the rename must be rejected by the Windows kernel sharing contract"
    );
    let delete_error = fs::remove_file(&fixture.db_path)
        .expect_err("a no-share Windows handle must block database deletion");
    assert_eq!(
        delete_error.raw_os_error(),
        Some(ERROR_SHARING_VIOLATION),
        "the delete must be rejected by the Windows kernel sharing contract"
    );

    assert_worker_open_is_denied(store.clone(), &fixture.db_path);
    assert_eq!(
        bytes_from_exclusive_handle(&mut exclusive),
        checkpointed_bytes,
        "the failed production open must not change the checkpointed database bytes"
    );
    drop(exclusive);

    assert_eq!(raw_room_row(&fixture.db_path), durable_before);
    assert_integrity(&fixture.db_path);
    assert_worker_recovery(store, &fixture.db_path);
}

#[cfg(unix)]
struct UnixNamespaceDenial {
    db_path: PathBuf,
    displaced_path: PathBuf,
    restored: bool,
}

#[cfg(unix)]
impl UnixNamespaceDenial {
    fn install(fixture: &SqliteFixture) -> Self {
        let displaced_path = fixture.displaced_path();
        fs::rename(&fixture.db_path, &displaced_path)
            .expect("the Unix rename syscall should displace the checkpointed database");
        fs::create_dir(&fixture.db_path)
            .expect("a directory should occupy the production database path");
        Self {
            db_path: fixture.db_path.clone(),
            displaced_path,
            restored: false,
        }
    }

    fn restore(&mut self) {
        fs::remove_dir(&self.db_path)
            .expect("the temporary database-path directory should be removable");
        fs::rename(&self.displaced_path, &self.db_path)
            .expect("the checkpointed database should return to its production path");
        self.restored = true;
    }
}

#[cfg(unix)]
impl Drop for UnixNamespaceDenial {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        remove_path_if_present(&self.db_path);
        if self.displaced_path.exists() {
            let _ = fs::rename(&self.displaced_path, &self.db_path);
        }
    }
}

#[cfg(unix)]
#[test]
fn room_persistence_unix_namespace_denial_preserves_and_recovers_durable_state() {
    let fixture = SqliteFixture::new("room-platform-unix-namespace-denial");
    let (store, durable_before, checkpointed_bytes) = seed_checkpointed_baseline(&fixture);
    let mut denial = UnixNamespaceDenial::install(&fixture);

    assert_worker_open_is_denied(store.clone(), &fixture.db_path);
    assert_eq!(
        fs::read(&denial.displaced_path)
            .expect("the displaced checkpointed database should remain readable"),
        checkpointed_bytes,
        "the failed production open must not change the checkpointed database bytes"
    );
    assert_eq!(raw_room_row(&denial.displaced_path), durable_before);
    assert_integrity(&denial.displaced_path);

    denial.restore();
    assert_eq!(raw_room_row(&fixture.db_path), durable_before);
    assert_integrity(&fixture.db_path);
    assert_worker_recovery(store, &fixture.db_path);
}
