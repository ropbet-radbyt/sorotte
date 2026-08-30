use super::*;

#[cfg(test)]
pub(crate) mod test_crash {
    pub(crate) const HELPER_ENV: &str = "SOROTTE_PERSISTENCE_CRASH_HELPER";
    pub(crate) const POINT_ENV: &str = "SOROTTE_PERSISTENCE_CRASH_POINT";
    pub(crate) const ACTION_ENV: &str = "SOROTTE_PERSISTENCE_CRASH_ACTION";
    pub(crate) const DB_PATH_ENV: &str = "SOROTTE_PERSISTENCE_CRASH_DB_PATH";
    pub(crate) const EXIT_CODE: i32 = 86;

    pub(crate) const SCHEMA_AFTER_PLAYLIST_JSON: &str = "schema-after-playlist-json";
    pub(crate) const SCHEMA_AFTER_PERSISTENCE_VERSION: &str = "schema-after-persistence-version";
    pub(crate) const SCHEMA_AFTER_OWNER_BUCKET: &str = "schema-after-owner-bucket";
    pub(crate) const SCHEMA_AFTER_CREATED_AT: &str = "schema-after-created-at";
    pub(crate) const SCHEMA_AFTER_METADATA: &str = "schema-after-metadata";
    pub(crate) const ROOM_MIGRATION_AFTER_ROW: &str = "room-migration-after-row";
    pub(crate) const ROOM_MIGRATION_AFTER_COMMIT: &str = "room-migration-after-commit";
    pub(crate) const ROOM_EFFECT_AFTER_WRITE: &str = "room-effect-after-write";
    pub(crate) const ROOM_EFFECT_AFTER_COMMIT: &str = "room-effect-after-commit";
    pub(crate) const STATS_AFTER_FIRST_ROW: &str = "stats-after-first-row";
    pub(crate) const STATS_AFTER_COMMIT: &str = "stats-after-commit";
    pub(crate) const QUOTA_SECRET_AFTER_GENERATE: &str = "quota-secret-after-generate";
    pub(crate) const QUOTA_SECRET_AFTER_INSERT: &str = "quota-secret-after-insert";

    pub(crate) fn exit_if_armed(point: &str) {
        if std::env::var_os(HELPER_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
            && std::env::var_os(POINT_ENV).as_deref() == Some(std::ffi::OsStr::new(point))
        {
            std::process::exit(EXIT_CODE);
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct PersistedRoomState {
    pub(crate) files: Vec<String>,
    pub(crate) index: Option<i64>,
    pub(crate) position: f64,
    pub(crate) last_activity_at_seconds: f64,
    pub(crate) version: u64,
    pub(crate) owner_bucket: Option<String>,
    pub(crate) created_at_seconds: f64,
}

impl std::fmt::Debug for PersistedRoomState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistedRoomState")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .field("position", &self.position)
            .field("last_activity_at_seconds", &self.last_activity_at_seconds)
            .field("version", &self.version)
            .field("has_owner_bucket", &self.owner_bucket.is_some())
            .field("created_at_seconds", &self.created_at_seconds)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StatsPersistenceError {
    #[error("stats persistence '{action}' failed for '{path}': {source}")]
    Sqlite {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatsPersistenceStore {
    db_path: PathBuf,
}

impl StatsPersistenceStore {
    pub(crate) fn open(db_path: impl AsRef<Path>) -> Result<Self, StatsPersistenceError> {
        let store = Self {
            db_path: db_path.as_ref().to_path_buf(),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    pub(crate) fn add_version_logs(
        &self,
        connection: &mut Connection,
        snapshot_time: i64,
        versions: &[String],
    ) -> Result<(), StatsPersistenceError> {
        let transaction = connection
            .transaction()
            .map_err(|source| self.sqlite_error("begin clients snapshot transaction", source))?;
        {
            let mut statement = transaction
                .prepare("INSERT INTO clients_snapshots (snapshot_time, version) VALUES (?1, ?2)")
                .map_err(|source| self.sqlite_error("prepare clients snapshot insert", source))?;
            for version in versions {
                statement
                    .execute(params![snapshot_time, version])
                    .map_err(|source| self.sqlite_error("insert clients snapshot row", source))?;
                #[cfg(test)]
                test_crash::exit_if_armed(test_crash::STATS_AFTER_FIRST_ROW);
            }
        }
        transaction
            .commit()
            .map_err(|source| self.sqlite_error("commit clients snapshot transaction", source))?;
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), StatsPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS clients_snapshots (\
                 snapshot_time INTEGER, \
                 version STRING\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize schema", source))?;
        Ok(())
    }

    pub(crate) fn connection(
        &self,
        action: &'static str,
    ) -> Result<Connection, StatsPersistenceError> {
        Connection::open(&self.db_path).map_err(|source| self.sqlite_error(action, source))
    }

    fn sqlite_error(&self, action: &'static str, source: rusqlite::Error) -> StatsPersistenceError {
        StatsPersistenceError::Sqlite {
            action,
            path: self.db_path.clone(),
            source,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoomPersistenceError {
    #[error("room persistence '{action}' failed for '{path}': {source}")]
    Sqlite {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomPersistenceStore {
    db_path: PathBuf,
}

impl RoomPersistenceStore {
    pub(crate) fn open(db_path: impl AsRef<Path>) -> Result<Self, RoomPersistenceError> {
        let store = Self {
            db_path: db_path.as_ref().to_path_buf(),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    pub(crate) fn load_rooms(
        &self,
    ) -> Result<BTreeMap<String, PersistedRoomState>, RoomPersistenceError> {
        let mut connection = self.connection("connect")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|source| {
                self.sqlite_error("begin persisted room migration transaction", source)
            })?;
        let mut statement = transaction
            .prepare(
                "SELECT name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate \
                        , persistenceVersion, ownerBucket, createdAt \
                 FROM persistent_rooms",
            )
            .map_err(|source| self.sqlite_error("prepare load query", source))?;
        let rows = statement
            .query_map([], |row| {
                let room_name: String = row.get(0)?;
                let playlist_multiline: Option<String> = row.get(1)?;
                let playlist_json: Option<String> = row.get(2)?;
                let playlist_index: Option<i64> = row.get(3)?;
                let position: Option<f64> = row.get(4)?;
                let last_activity_at_seconds: Option<f64> = row.get(5)?;
                let version: Option<i64> = row.get(6)?;
                let owner_bucket: Option<String> = row.get(7)?;
                let created_at_seconds: Option<f64> = row.get(8)?;
                let decoded_json = playlist_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok());
                let needs_json_migration = decoded_json.is_none();
                let files = decoded_json.unwrap_or_else(|| {
                    multiline_as_playlist(&playlist_multiline.unwrap_or_default())
                });
                let mut playlist = RoomPlaylistState {
                    files,
                    index: playlist_index,
                    epoch: 0,
                };
                let needs_index_migration = playlist.normalize_index();
                Ok((
                    room_name,
                    PersistedRoomState {
                        files: playlist.files,
                        index: playlist.index,
                        position: position.unwrap_or(0.0),
                        last_activity_at_seconds: last_activity_at_seconds.unwrap_or(0.0),
                        version: version.unwrap_or(0).max(0) as u64,
                        owner_bucket,
                        created_at_seconds: created_at_seconds.unwrap_or(0.0),
                    },
                    needs_json_migration,
                    needs_index_migration,
                ))
            })
            .map_err(|source| self.sqlite_error("query persisted rooms", source))?;

        let mut decoded_rows = Vec::new();
        for row in rows {
            let (room_name, room_state, needs_json_migration, needs_index_migration) =
                row.map_err(|source| self.sqlite_error("decode persisted room row", source))?;
            decoded_rows.push((
                room_name,
                room_state,
                needs_json_migration,
                needs_index_migration,
            ));
        }
        drop(statement);

        let mut rooms = BTreeMap::new();
        for (room_name, room_state, needs_json_migration, needs_index_migration) in decoded_rows {
            if needs_json_migration {
                let playlist_json = serde_json::to_string(&room_state.files)
                    .expect("serializing a string playlist cannot fail");
                transaction
                    .execute(
                        "UPDATE persistent_rooms SET playlistJson = ?1 WHERE name = ?2",
                        params![playlist_json, room_name],
                    )
                    .map_err(|source| {
                        self.sqlite_error("migrate persisted playlist JSON", source)
                    })?;
            }
            if needs_index_migration {
                transaction
                    .execute(
                        "UPDATE persistent_rooms SET playlistIndex = ?1 WHERE name = ?2",
                        params![room_state.index, room_name],
                    )
                    .map_err(|source| {
                        self.sqlite_error("normalize persisted playlist index", source)
                    })?;
            }
            rooms.insert(room_name, room_state);
            #[cfg(test)]
            test_crash::exit_if_armed(test_crash::ROOM_MIGRATION_AFTER_ROW);
        }
        transaction
            .commit()
            .map_err(|source| self.sqlite_error("commit persisted room migrations", source))?;
        #[cfg(test)]
        test_crash::exit_if_armed(test_crash::ROOM_MIGRATION_AFTER_COMMIT);
        Ok(rooms)
    }

    pub(crate) fn save_room(
        &self,
        connection: &Connection,
        room_name: &str,
        state: &PersistedRoomState,
    ) -> Result<(), RoomPersistenceError> {
        let mut playlist = RoomPlaylistState {
            files: state.files.clone(),
            index: state.index,
            epoch: 0,
        };
        playlist.normalize_index();
        let persistence_version = i64::try_from(state.version)
            .expect("runtime room persistence version must fit SQLite i64");
        connection
            .execute(
                "INSERT INTO persistent_rooms \
                 (name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                  persistenceVersion, ownerBucket, createdAt) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(name) DO UPDATE SET \
                    playlist = excluded.playlist, \
                    playlistJson = excluded.playlistJson, \
                    playlistIndex = excluded.playlistIndex, \
                    position = excluded.position, \
                    lastSavedUpdate = excluded.lastSavedUpdate, \
                    persistenceVersion = excluded.persistenceVersion, \
                    ownerBucket = excluded.ownerBucket, \
                    createdAt = excluded.createdAt \
                 WHERE excluded.persistenceVersion > persistent_rooms.persistenceVersion",
                params![
                    room_name,
                    playlist_as_multiline(&playlist.files),
                    serde_json::to_string(&playlist.files)
                        .expect("serializing a string playlist cannot fail"),
                    playlist.index,
                    state.position,
                    state.last_activity_at_seconds,
                    persistence_version,
                    state.owner_bucket.as_deref(),
                    state.created_at_seconds
                ],
            )
            .map_err(|source| self.sqlite_error("save persisted room", source))?;
        Ok(())
    }

    pub(crate) fn delete_room(
        &self,
        connection: &Connection,
        room_name: &str,
        version: u64,
    ) -> Result<(), RoomPersistenceError> {
        let persistence_version =
            i64::try_from(version).expect("runtime room persistence version must fit SQLite i64");
        connection
            .execute(
                "DELETE FROM persistent_rooms WHERE name = ?1 AND persistenceVersion < ?2",
                params![room_name, persistence_version],
            )
            .map_err(|source| self.sqlite_error("delete persisted room", source))?;
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), RoomPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| self.sqlite_error("enable WAL journal mode", source))?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS persistent_rooms (\
                 name STRING PRIMARY KEY, \
                 playlist STRING, \
                 playlistJson STRING, \
                 playlistIndex INTEGER, \
                 position REAL, \
                 lastSavedUpdate REAL, \
                 persistenceVersion INTEGER NOT NULL DEFAULT 0, \
                 ownerBucket STRING, \
                 createdAt REAL NOT NULL DEFAULT 0\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize schema", source))?;
        let columns = {
            let mut statement = connection
                .prepare("PRAGMA table_info(persistent_rooms)")
                .map_err(|source| self.sqlite_error("inspect schema", source))?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|source| self.sqlite_error("query schema", source))?;
            let mut found = BTreeSet::new();
            for column in columns {
                found.insert(column.map_err(|source| self.sqlite_error("decode schema", source))?);
            }
            found
        };
        if !columns.contains("playlistJson") {
            connection
                .execute(
                    "ALTER TABLE persistent_rooms ADD COLUMN playlistJson STRING",
                    [],
                )
                .map_err(|source| self.sqlite_error("migrate schema", source))?;
            #[cfg(test)]
            test_crash::exit_if_armed(test_crash::SCHEMA_AFTER_PLAYLIST_JSON);
        }
        for (column, definition) in [
            ("persistenceVersion", "INTEGER NOT NULL DEFAULT 0"),
            ("ownerBucket", "STRING"),
            ("createdAt", "REAL NOT NULL DEFAULT 0"),
        ] {
            if !columns.contains(column) {
                connection
                    .execute(
                        &format!("ALTER TABLE persistent_rooms ADD COLUMN {column} {definition}"),
                        [],
                    )
                    .map_err(|source| self.sqlite_error("migrate schema", source))?;
                #[cfg(test)]
                test_crash::exit_if_armed(match column {
                    "persistenceVersion" => test_crash::SCHEMA_AFTER_PERSISTENCE_VERSION,
                    "ownerBucket" => test_crash::SCHEMA_AFTER_OWNER_BUCKET,
                    "createdAt" => test_crash::SCHEMA_AFTER_CREATED_AT,
                    _ => unreachable!("only known schema migrations are enumerated"),
                });
            }
        }
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS persistence_metadata (\
                 key STRING PRIMARY KEY, \
                 value BLOB NOT NULL\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize metadata schema", source))?;
        #[cfg(test)]
        test_crash::exit_if_armed(test_crash::SCHEMA_AFTER_METADATA);
        Ok(())
    }

    pub(crate) fn load_or_create_quota_secret(&self) -> Result<[u8; 32], RoomPersistenceError> {
        self.load_or_create_quota_secret_inner(|| {})
    }

    #[cfg(test)]
    pub(crate) fn load_or_create_quota_secret_with_before_create<F>(
        &self,
        before_create: F,
    ) -> Result<[u8; 32], RoomPersistenceError>
    where
        F: FnOnce(),
    {
        self.load_or_create_quota_secret_inner(before_create)
    }

    fn load_or_create_quota_secret_inner<F>(
        &self,
        before_create: F,
    ) -> Result<[u8; 32], RoomPersistenceError>
    where
        F: FnOnce(),
    {
        let connection = self.connection("connect quota metadata")?;
        let existing = connection
            .query_row(
                "SELECT value FROM persistence_metadata WHERE key = 'quota-secret-v1'",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|source| self.sqlite_error("load quota secret", source))?;
        if let Some(existing) = existing {
            return self.decode_quota_secret(existing);
        }

        before_create();
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).expect("operating system random source should be available");
        #[cfg(test)]
        test_crash::exit_if_armed(test_crash::QUOTA_SECRET_AFTER_GENERATE);
        connection
            .execute(
                "INSERT INTO persistence_metadata (key, value) \
                 VALUES ('quota-secret-v1', ?1) \
                 ON CONFLICT(key) DO NOTHING",
                params![secret.as_slice()],
            )
            .map_err(|source| self.sqlite_error("create quota secret", source))?;
        #[cfg(test)]
        test_crash::exit_if_armed(test_crash::QUOTA_SECRET_AFTER_INSERT);
        let stored = connection
            .query_row(
                "SELECT value FROM persistence_metadata WHERE key = 'quota-secret-v1'",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|source| self.sqlite_error("reload quota secret", source))?;
        self.decode_quota_secret(stored)
    }

    fn decode_quota_secret(&self, value: Vec<u8>) -> Result<[u8; 32], RoomPersistenceError> {
        value
            .try_into()
            .map_err(|_| self.sqlite_error("decode quota secret", rusqlite::Error::InvalidQuery))
    }

    pub(crate) fn connection(
        &self,
        action: &'static str,
    ) -> Result<Connection, RoomPersistenceError> {
        let connection =
            Connection::open(&self.db_path).map_err(|source| self.sqlite_error(action, source))?;
        // Room persistence is intentionally synchronous and should stay lightweight.
        // A busy timeout prevents transient writer contention from failing immediately.
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|source| self.sqlite_error("set busy timeout", source))?;
        Ok(connection)
    }

    fn sqlite_error(&self, action: &'static str, source: rusqlite::Error) -> RoomPersistenceError {
        RoomPersistenceError::Sqlite {
            action,
            path: self.db_path.clone(),
            source,
        }
    }
}
