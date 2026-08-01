use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use rusqlite::Connection;
#[cfg(target_os = "linux")]
use std::sync::{Arc, atomic::AtomicUsize};
#[cfg(target_os = "linux")]
use tokio::sync::broadcast;

#[cfg(target_os = "linux")]
use crate::{
    PersistedRoomState, RoomPersistenceService, RoomPersistenceStore, ServerPersistenceEffect,
};

const ENABLE_ENV: &str = "SOROTTE_PERSISTENCE_POWERLOSS_ENABLE";
const ENABLE_TOKEN: &str = "owned-disposable-images-only-v1";
#[cfg(target_os = "linux")]
const ROOT_ENV: &str = "SOROTTE_PERSISTENCE_POWERLOSS_ROOT";
#[cfg(target_os = "linux")]
const NONCE_ENV: &str = "SOROTTE_PERSISTENCE_POWERLOSS_NONCE";
#[cfg(target_os = "linux")]
const DB_PATH_ENV: &str = "SOROTTE_PERSISTENCE_POWERLOSS_DB_PATH";
#[cfg(target_os = "linux")]
const PHASE_ENV: &str = "SOROTTE_PERSISTENCE_POWERLOSS_PHASE";
const ROOT_PREFIX: &str = "sorotte-powerloss-";
const MARKER_NAME: &str = ".sorotte-powerloss-owned-v1";
#[cfg(target_os = "linux")]
const ROOM_NAME: &str = "disposable-powerloss-room";

fn marker_contents(nonce: &str) -> String {
    format!("sorotte-powerloss-owned-v1\nnonce={nonce}\n")
}

fn validate_nonce(nonce: &str) -> Result<(), String> {
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("the harness nonce must contain exactly 32 ASCII hex digits".to_owned());
    }
    Ok(())
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{label} metadata failed: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} must be a real directory, not a symlink"));
    }
    Ok(())
}

fn validate_driver_db_path(
    root_argument: &Path,
    nonce: &str,
    db_argument: &Path,
) -> Result<PathBuf, String> {
    validate_nonce(nonce)?;
    require_real_directory(root_argument, "harness root")?;
    let root = fs::canonicalize(root_argument)
        .map_err(|error| format!("harness root canonicalization failed: {error}"))?;
    if !root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(ROOT_PREFIX))
    {
        return Err(format!("harness root name must start with {ROOT_PREFIX:?}"));
    }

    let marker = root.join(MARKER_NAME);
    let marker_metadata = fs::symlink_metadata(&marker)
        .map_err(|error| format!("ownership marker metadata failed: {error}"))?;
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Err("ownership marker must be a real regular file".to_owned());
    }
    let actual_marker = fs::read_to_string(&marker)
        .map_err(|error| format!("ownership marker read failed: {error}"))?;
    if actual_marker != marker_contents(nonce) {
        return Err("ownership marker does not match the supplied nonce".to_owned());
    }

    let mount = root.join("mount");
    let database_directory = mount.join("sorotte");
    require_real_directory(&mount, "harness mount directory")?;
    require_real_directory(&database_directory, "harness database directory")?;
    let canonical_database_directory = fs::canonicalize(&database_directory)
        .map_err(|error| format!("database directory canonicalization failed: {error}"))?;
    if canonical_database_directory != database_directory {
        return Err("harness database directory must use its canonical path".to_owned());
    }

    let expected_db_path = canonical_database_directory.join("rooms.sqlite3");
    let argument_parent = db_argument
        .parent()
        .ok_or_else(|| "database path must have an owned parent".to_owned())?;
    let canonical_argument_parent = fs::canonicalize(argument_parent)
        .map_err(|error| format!("database path parent canonicalization failed: {error}"))?;
    if canonical_argument_parent != canonical_database_directory
        || db_argument.file_name() != Some(std::ffi::OsStr::new("rooms.sqlite3"))
    {
        return Err(format!(
            "database path must be exactly '{}'",
            expected_db_path.display()
        ));
    }
    match fs::symlink_metadata(&expected_db_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("existing database path must be a real regular file".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("database path metadata failed: {error}")),
    }
    Ok(expected_db_path)
}

#[cfg(target_os = "linux")]
fn baseline_state() -> PersistedRoomState {
    PersistedRoomState {
        files: vec![
            "durable-baseline-first.mkv".to_owned(),
            "durable-baseline-second.mkv".to_owned(),
        ],
        index: Some(1),
        position: 101.25,
        last_activity_at_seconds: 102.5,
        version: 100,
        owner_bucket: Some("durable-owner-v1".to_owned()),
        created_at_seconds: 99.75,
    }
}

#[cfg(target_os = "linux")]
fn replacement_state() -> PersistedRoomState {
    PersistedRoomState {
        files: vec![
            "durable-replacement-first.mkv".to_owned(),
            "durable-replacement-second.mkv".to_owned(),
            "durable-replacement-third.mkv".to_owned(),
        ],
        index: Some(2),
        position: 201.25,
        last_activity_at_seconds: 202.5,
        version: 200,
        owner_bucket: Some("durable-owner-v2".to_owned()),
        created_at_seconds: 99.75,
    }
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
fn write_and_acknowledge(db_path: &Path, state: &PersistedRoomState) {
    let store =
        RoomPersistenceStore::open(db_path).expect("disposable persistence store should open");
    let (events, _) = broadcast::channel(8);
    let service = RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
        .expect("disposable persistence worker should start");
    service.enqueue(room_effect(state));
    assert!(
        service.flush(),
        "the production persistence worker must acknowledge the requested state"
    );
    drop(service);
}

#[cfg(target_os = "linux")]
fn assert_raw_state(connection: &Connection, expected: &PersistedRoomState) {
    let raw = connection
        .query_row(
            "SELECT playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                    persistenceVersion, ownerBucket, createdAt \
             FROM persistent_rooms WHERE name = ?1",
            [ROOM_NAME],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, f64>(7)?,
                ))
            },
        )
        .expect("the complete durable room row should be queryable");
    assert_eq!(raw.0, expected.files.join("\n"));
    assert_eq!(
        raw.1,
        serde_json::to_string(&expected.files).expect("string playlist should serialize")
    );
    assert_eq!(raw.2, expected.index);
    assert_eq!(raw.3, expected.position);
    assert_eq!(raw.4, expected.last_activity_at_seconds);
    assert_eq!(raw.5, expected.version as i64);
    assert_eq!(raw.6, expected.owner_bucket);
    assert_eq!(raw.7, expected.created_at_seconds);
}

#[cfg(target_os = "linux")]
fn observed_state(db_path: &Path) -> &'static str {
    let store =
        RoomPersistenceStore::open(db_path).expect("replayed persistence store should reopen");
    let rooms = store
        .load_rooms()
        .expect("replayed persistence state should load");
    assert_eq!(
        rooms.len(),
        1,
        "replayed persistence must contain exactly the test room"
    );
    let state = rooms
        .get(ROOM_NAME)
        .expect("replayed persistence must retain the test room");
    let observed = if state == &baseline_state() {
        "baseline"
    } else if state == &replacement_state() {
        "replacement"
    } else {
        panic!("replayed room must be one complete acknowledged generation: {state:?}");
    };

    let connection =
        Connection::open(db_path).expect("replayed database should open for raw verification");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("SQLite should return an integrity result");
    assert_eq!(integrity, "ok", "replayed SQLite integrity must be intact");
    assert_raw_state(&connection, state);
    observed
}

#[cfg(target_os = "linux")]
fn run_disposable_block_driver() {
    let root = PathBuf::from(
        std::env::var_os(ROOT_ENV).expect("the harness must supply its test-owned root"),
    );
    let nonce = std::env::var(NONCE_ENV).expect("the harness must supply its ownership nonce");
    let db_argument = PathBuf::from(
        std::env::var_os(DB_PATH_ENV).expect("the harness must supply its exact database path"),
    );
    let db_path = validate_driver_db_path(&root, &nonce, &db_argument)
        .unwrap_or_else(|error| panic!("unsafe disposable-image driver path: {error}"));
    let phase = std::env::var(PHASE_ENV).expect("the harness must supply an exact driver phase");

    let result = match phase.as_str() {
        "seed-baseline" => {
            write_and_acknowledge(&db_path, &baseline_state());
            assert_eq!(observed_state(&db_path), "baseline");
            "baseline"
        }
        "write-replacement" => {
            assert_eq!(
                observed_state(&db_path),
                "baseline",
                "replacement phase must begin from the flushed baseline"
            );
            write_and_acknowledge(&db_path, &replacement_state());
            assert_eq!(observed_state(&db_path), "replacement");
            "replacement"
        }
        "verify-baseline" => {
            assert_eq!(
                observed_state(&db_path),
                "baseline",
                "baseline replay must recover the complete flushed baseline"
            );
            "baseline"
        }
        "verify-old-or-new" => observed_state(&db_path),
        "verify-replacement" => {
            assert_eq!(
                observed_state(&db_path),
                "replacement",
                "syncfs replay must recover the complete replacement"
            );
            "replacement"
        }
        other => panic!("unsupported disposable-image driver phase {other:?}"),
    };
    println!("SOROTTE_POWERLOSS_RESULT={result}");
}

#[cfg(not(target_os = "linux"))]
fn run_disposable_block_driver() {
    panic!("the disposable block replay driver is Linux-only");
}

#[test]
fn room_persistence_disposable_block_driver() {
    if std::env::var_os(ENABLE_ENV).as_deref() != Some(std::ffi::OsStr::new(ENABLE_TOKEN)) {
        return;
    }
    run_disposable_block_driver();
}

#[cfg(target_os = "linux")]
#[test]
fn disposable_block_driver_phase_model_round_trips_on_plain_temp_store() {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos()
    );
    let fixture = std::env::temp_dir().join(format!("sorotte-powerloss-driver-{unique}"));
    fs::create_dir(&fixture).expect("plain driver fixture should be creatable");
    let db_path = fixture.join("rooms.sqlite3");

    write_and_acknowledge(&db_path, &baseline_state());
    assert_eq!(observed_state(&db_path), "baseline");
    write_and_acknowledge(&db_path, &replacement_state());
    assert_eq!(observed_state(&db_path), "replacement");

    for candidate in [
        db_path.clone(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
        PathBuf::from(format!("{}-journal", db_path.display())),
    ] {
        match fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!(
                "exact plain driver artifact '{}' should be removable: {error}",
                candidate.display()
            ),
        }
    }
    fs::remove_dir(&fixture).expect("exact plain driver fixture should be removable");
}

#[test]
fn disposable_block_driver_path_contract_is_fail_closed() {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos()
    );
    let root = std::env::temp_dir().join(format!("{ROOT_PREFIX}{unique}"));
    let mount = root.join("mount");
    let database_directory = mount.join("sorotte");
    fs::create_dir_all(&database_directory).expect("owned contract fixture should be creatable");
    let nonce = "0123456789abcdef0123456789abcdef";
    fs::write(root.join(MARKER_NAME), marker_contents(nonce))
        .expect("owned contract marker should be writable");
    let db_path = database_directory.join("rooms.sqlite3");

    let validated = validate_driver_db_path(&root, nonce, &db_path)
        .expect("the exact owned contract should validate");
    assert_eq!(
        validated,
        fs::canonicalize(&database_directory)
            .expect("contract database directory should canonicalize")
            .join("rooms.sqlite3")
    );
    assert!(
        validate_driver_db_path(&root, "short", &db_path)
            .expect_err("a malformed nonce must fail closed")
            .contains("32 ASCII hex")
    );
    fs::write(
        root.join(MARKER_NAME),
        marker_contents("ffffffffffffffffffffffffffffffff"),
    )
    .expect("contract marker should be replaceable");
    assert!(
        validate_driver_db_path(&root, nonce, &db_path)
            .expect_err("a mismatched ownership marker must fail closed")
            .contains("does not match")
    );
    fs::write(root.join(MARKER_NAME), marker_contents(nonce))
        .expect("contract marker should be restorable");
    let outside = root.join("outside.sqlite3");
    assert!(
        validate_driver_db_path(&root, nonce, &outside)
            .expect_err("an arbitrary path must fail closed")
            .contains("must be exactly")
    );

    fs::remove_dir_all(&root).expect("the exact test-owned contract fixture should be removable");
}
