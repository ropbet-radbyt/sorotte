use super::*;

#[derive(Clone, PartialEq)]
pub(crate) struct PersistedRoomState {
    pub(crate) files: Vec<String>,
    pub(crate) index: Option<i64>,
    pub(crate) position: f64,
}

impl std::fmt::Debug for PersistedRoomState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistedRoomState")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .field("position", &self.position)
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
        let connection = self.connection("connect")?;
        let mut statement = connection
            .prepare(
                "SELECT name, playlist, playlistIndex, position \
                 FROM persistent_rooms",
            )
            .map_err(|source| self.sqlite_error("prepare load query", source))?;
        let rows = statement
            .query_map([], |row| {
                let room_name: String = row.get(0)?;
                let playlist_multiline: Option<String> = row.get(1)?;
                let playlist_index: Option<i64> = row.get(2)?;
                let position: Option<f64> = row.get(3)?;
                Ok((
                    room_name,
                    PersistedRoomState {
                        files: multiline_as_playlist(&playlist_multiline.unwrap_or_default()),
                        index: playlist_index,
                        position: position.unwrap_or(0.0),
                    },
                ))
            })
            .map_err(|source| self.sqlite_error("query persisted rooms", source))?;

        let mut rooms = BTreeMap::new();
        for row in rows {
            let (room_name, room_state) =
                row.map_err(|source| self.sqlite_error("decode persisted room row", source))?;
            rooms.insert(room_name, room_state);
        }
        Ok(rooms)
    }

    pub(crate) fn save_room(
        &self,
        connection: &Connection,
        room_name: &str,
        files: &[String],
        playlist_index: Option<i64>,
        position: f64,
    ) -> Result<(), RoomPersistenceError> {
        connection
            .execute(
                "INSERT OR REPLACE INTO persistent_rooms \
                 (name, playlist, playlistIndex, position, lastSavedUpdate) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    room_name,
                    playlist_as_multiline(files),
                    playlist_index,
                    position,
                    0_i64
                ],
            )
            .map_err(|source| self.sqlite_error("save persisted room", source))?;
        Ok(())
    }

    pub(crate) fn delete_room(
        &self,
        connection: &Connection,
        room_name: &str,
    ) -> Result<(), RoomPersistenceError> {
        connection
            .execute(
                "DELETE FROM persistent_rooms WHERE name = ?1",
                params![room_name],
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
                 playlistIndex INTEGER, \
                 position REAL, \
                 lastSavedUpdate INTEGER\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize schema", source))?;
        Ok(())
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
