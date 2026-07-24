use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use rusqlite::{Connection, OptionalExtension, backup::Backup, params};

use crate::{
    MEDIA_MATCH_ANCHOR_VERSION, MediaExtractionSettings, MediaFingerprintRecord, MediaMatchCache,
    MediaMatchV3RetrievalStats, MediaMatchV3SaveStats, MediaMatchV3SqliteSizeReport,
    media_extraction_settings_hash,
    v3_index::{
        anchor_stats_v3_dirty, delete_media_match_v3_file_and_fingerprints,
        delete_media_match_v3_fingerprints_and_anchors, load_media_match_v3_cache_for_settings,
        load_media_match_v3_record_for_path, media_match_v3_anchor_candidate_paths_with_stats,
        media_match_v3_index_path, media_match_v3_sqlite_size_report, open_media_match_v3_index,
        refresh_all_anchor_stats_v3, refresh_anchor_stats_v3, save_media_match_v3_record,
        save_media_match_v3_record_with_stats,
    },
};

const MEDIA_INDEX_MANIFEST_FILE: &str = "current.json";
const MEDIA_INDEX_GENERATIONS_DIR: &str = "generations";
const MEDIA_INDEX_MANIFEST_VERSION: u32 = 1;

/// Filesystem metadata used to update one row of the media inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIndexInventoryEntry {
    pub normalized_path: String,
    pub modified_unix_millis: u64,
    pub size_bytes: u64,
}

impl MediaIndexInventoryEntry {
    pub fn new(
        normalized_path: impl Into<String>,
        modified_unix_millis: u64,
        size_bytes: u64,
    ) -> Self {
        Self {
            normalized_path: normalized_path.into(),
            modified_unix_millis,
            size_bytes,
        }
    }
}

/// Compact, storage-independent status for one media index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaIndexSummary {
    pub inventory_count: usize,
    pub fixed_settings_fingerprint_count: usize,
    pub current_settings_fingerprint_count: usize,
    pub database_bytes: u64,
    pub v3_audio_blob_bytes: u64,
    pub v3_fingerprint_row_count: usize,
    pub v3_audio_verify_count: u64,
    pub v3_audio_index_count: u64,
}

/// Owns the location and lifecycle of one media-match index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIndexService {
    root: PathBuf,
}

impl MediaIndexService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index_path(&self) -> PathBuf {
        media_match_v3_index_path(&active_media_index_root(&self.root))
    }

    pub fn open(&self) -> Result<MediaIndexSession, String> {
        let active_root = active_media_index_root(&self.root);
        open_media_match_v3_index(&active_root).map(|connection| MediaIndexSession {
            root: active_root,
            connection,
        })
    }
}

/// Isolates an index rebuild from the live WAL database until a validated same-directory swap.
#[derive(Debug)]
pub struct MediaIndexBuildTransaction {
    live_root: PathBuf,
    staging_root: PathBuf,
    had_live_index: bool,
    finished: bool,
}

impl MediaIndexBuildTransaction {
    pub fn begin(
        live_root: impl Into<PathBuf>,
        staging_root: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let live_root = live_root.into();
        let staging_root = staging_root.into();
        if staging_root.exists() {
            return Err(format!(
                "media-match staging directory '{}' already exists",
                staging_root.display()
            ));
        }
        fs::create_dir_all(&staging_root).map_err(|error| {
            format!(
                "failed creating media-match staging directory '{}': {error}",
                staging_root.display()
            )
        })?;

        let live_path = MediaIndexService::new(&live_root).index_path();
        let had_live_index = live_path.exists();
        if had_live_index {
            let staging_path = media_match_v3_index_path(&staging_root);
            online_backup_database(&live_path, &staging_path)?;
            validate_media_index_database(&staging_path)?;
        }

        Ok(Self {
            live_root,
            staging_root,
            had_live_index,
            finished: false,
        })
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub fn had_live_index(&self) -> bool {
        self.had_live_index
    }

    pub fn commit(mut self) -> Result<(), String> {
        let staging_path = media_match_v3_index_path(&self.staging_root);
        validate_media_index_database(&staging_path)?;

        let unique = media_index_transaction_unique();
        let generation = format!("generation-{unique}");
        let generation_root = self
            .live_root
            .join(MEDIA_INDEX_GENERATIONS_DIR)
            .join(&generation);
        fs::create_dir_all(&generation_root).map_err(|error| {
            format!(
                "failed creating media-match generation directory '{}': {error}",
                generation_root.display()
            )
        })?;
        let replacement_path = media_match_v3_index_path(&generation_root);
        online_backup_database(&staging_path, &replacement_path)?;
        validate_media_index_database(&replacement_path)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&replacement_path)
            .and_then(|file| file.sync_all())
            .map_err(|error| {
                format!(
                    "failed flushing validated media-match replacement '{}': {error}",
                    replacement_path.display()
                )
            })?;
        write_media_index_manifest(&self.live_root, &generation)?;
        remove_directory_if_exists(&self.staging_root)?;
        self.finished = true;
        Ok(())
    }

    pub fn abort(mut self) -> Result<(), String> {
        remove_directory_if_exists(&self.staging_root)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for MediaIndexBuildTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = remove_directory_if_exists(&self.staging_root);
        }
    }
}

/// An initialized media-index connection with semantic operations.
pub struct MediaIndexSession {
    root: PathBuf,
    connection: Connection,
}

impl MediaIndexSession {
    pub fn load_cache(
        &self,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<MediaMatchCache, String> {
        load_media_match_v3_cache_for_settings(&self.connection, extraction_settings)
    }

    pub fn load_record(
        &self,
        normalized_path: &str,
        extraction_settings: &MediaExtractionSettings,
        modified_unix_millis: u64,
        size_bytes: u64,
    ) -> Result<Option<MediaFingerprintRecord>, String> {
        load_media_match_v3_record_for_path(
            &self.connection,
            normalized_path,
            extraction_settings,
            modified_unix_millis,
            size_bytes,
        )
    }

    pub fn save_record(
        &self,
        record: &MediaFingerprintRecord,
        error: Option<&str>,
    ) -> Result<(), String> {
        save_media_match_v3_record(&self.connection, record, error)
    }

    pub fn save_record_with_stats(
        &self,
        record: &MediaFingerprintRecord,
        now_unix_millis: i64,
    ) -> Result<MediaMatchV3SaveStats, String> {
        save_media_match_v3_record_with_stats(&self.connection, record, now_unix_millis)
    }

    pub fn anchor_candidate_paths(
        &self,
        normalized_current_path: &str,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<(Vec<String>, MediaMatchV3RetrievalStats), String> {
        media_match_v3_anchor_candidate_paths_with_stats(
            &self.connection,
            normalized_current_path,
            extraction_settings,
        )
    }

    /// Replaces the scanned portion of the inventory in one transaction.
    ///
    /// Fingerprints and anchors are invalidated when an existing file's identity changes. Rows
    /// absent from `seen_normalized_paths` are pruned only when they live under one of the scanned
    /// roots. Any error or cancellation leaves the complete pre-refresh index intact.
    pub fn refresh_inventory(
        &self,
        entries: &[MediaIndexInventoryEntry],
        seen_normalized_paths: &[String],
        scanned_normalized_roots: &[String],
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<(), String> {
        let transaction = self.connection.unchecked_transaction().map_err(|error| {
            format!("failed starting media-match inventory transaction: {error}")
        })?;
        let updated_unix_millis = current_unix_millis() as i64;

        for entry in entries {
            check_inventory_cancelled(&mut is_cancelled)?;
            let old_identity = transaction
                .query_row(
                    "SELECT modified_unix_millis, size_bytes
                     FROM media_files_v3
                     WHERE normalized_path = ?1",
                    [entry.normalized_path.as_str()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(|error| format!("failed reading media-match inventory row: {error}"))?;
            if old_identity.is_some_and(|(old_mtime, old_size)| {
                old_mtime != entry.modified_unix_millis as i64
                    || old_size != entry.size_bytes as i64
            }) {
                delete_media_match_v3_fingerprints_and_anchors(
                    &transaction,
                    &entry.normalized_path,
                )?;
            }
            transaction
                .execute(
                    "INSERT INTO media_files_v3 (
                        normalized_path,
                        modified_unix_millis,
                        size_bytes,
                        duration_ms,
                        container_fingerprint,
                        updated_unix_millis
                    ) VALUES (?1, ?2, ?3, NULL, '', ?4)
                    ON CONFLICT(normalized_path) DO UPDATE SET
                        modified_unix_millis = excluded.modified_unix_millis,
                        size_bytes = excluded.size_bytes,
                        updated_unix_millis = excluded.updated_unix_millis",
                    params![
                        entry.normalized_path,
                        entry.modified_unix_millis as i64,
                        entry.size_bytes as i64,
                        updated_unix_millis,
                    ],
                )
                .map_err(|error| format!("failed writing media-match inventory row: {error}"))?;
        }

        check_inventory_cancelled(&mut is_cancelled)?;
        let seen_paths = seen_normalized_paths.iter().collect::<BTreeSet<_>>();
        let mut statement = transaction
            .prepare("SELECT normalized_path FROM media_files_v3")
            .map_err(|error| {
                format!("failed preparing media-match stale inventory query: {error}")
            })?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| {
                format!("failed querying media-match stale inventory rows: {error}")
            })?;
        let mut stale_paths = Vec::new();
        for row in rows {
            check_inventory_cancelled(&mut is_cancelled)?;
            let normalized_path =
                row.map_err(|error| format!("failed reading media-match inventory row: {error}"))?;
            let under_scanned_root = scanned_normalized_roots
                .iter()
                .any(|root| media_path_is_under_root(&normalized_path, root));
            if under_scanned_root && !seen_paths.contains(&normalized_path) {
                stale_paths.push(normalized_path);
            }
        }
        drop(statement);

        for normalized_path in stale_paths {
            check_inventory_cancelled(&mut is_cancelled)?;
            delete_media_match_v3_file_and_fingerprints(&transaction, &normalized_path)?;
        }
        check_inventory_cancelled(&mut is_cancelled)?;
        transaction.commit().map_err(|error| {
            format!("failed committing media-match inventory transaction: {error}")
        })
    }

    pub fn inventory_paths(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT normalized_path FROM media_files_v3 ORDER BY normalized_path")
            .map_err(|error| format!("failed preparing media-match inventory query: {error}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("failed querying media-match inventory: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed reading media-match inventory: {error}"))
    }

    pub fn summary(
        &self,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<MediaIndexSummary, String> {
        let inventory_count = inventory_count(&self.connection)?;
        let fixed_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let fixed_settings_fingerprint_count =
            fingerprint_count(&self.connection, &fixed_settings)?;
        let current_settings_fingerprint_count =
            fingerprint_count(&self.connection, extraction_settings)?;
        let database_bytes = fs::metadata(media_match_v3_index_path(&self.root))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let (blob_bytes, row_count, verify_count, index_count) = self
            .connection
            .query_row(
                "SELECT
                    COALESCE(SUM(COALESCE(LENGTH(audio_blob), 0)), 0),
                    COUNT(*),
                    COALESCE(SUM(audio_verify_count), 0),
                    COALESCE(SUM(audio_index_count), 0)
                 FROM fingerprints_v3
                 JOIN settings_v3 ON settings_v3.settings_id = fingerprints_v3.settings_id
                 WHERE settings_v3.algorithm_version = ?1",
                [i64::from(MEDIA_MATCH_ANCHOR_VERSION)],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("failed reading media-match storage summary: {error}"))?;
        Ok(MediaIndexSummary {
            inventory_count,
            fixed_settings_fingerprint_count,
            current_settings_fingerprint_count,
            database_bytes,
            v3_audio_blob_bytes: blob_bytes.max(0) as u64,
            v3_fingerprint_row_count: row_count.max(0) as usize,
            v3_audio_verify_count: verify_count.max(0) as u64,
            v3_audio_index_count: index_count.max(0) as u64,
        })
    }

    pub fn refresh_anchor_stats(
        &self,
        settings_hash: &[u8; 32],
        now_unix_millis: i64,
    ) -> Result<(), String> {
        refresh_anchor_stats_v3(&self.connection, settings_hash, now_unix_millis)
    }

    pub fn refresh_all_anchor_stats(&self, now_unix_millis: i64) -> Result<(), String> {
        refresh_all_anchor_stats_v3(&self.connection, now_unix_millis)
    }

    pub fn anchor_stats_dirty(&self, settings_hash: &[u8; 32]) -> Result<bool, String> {
        anchor_stats_v3_dirty(&self.connection, settings_hash)
    }

    pub fn record_updated_unix_millis(
        &self,
        normalized_path: &str,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<Option<i64>, String> {
        self.connection
            .query_row(
                "SELECT fingerprints_v3.updated_unix_millis
                 FROM fingerprints_v3
                 JOIN media_files_v3 ON media_files_v3.file_id = fingerprints_v3.file_id
                 JOIN settings_v3 ON settings_v3.settings_id = fingerprints_v3.settings_id
                 WHERE media_files_v3.normalized_path = ?1
                   AND settings_v3.algorithm_version = ?2
                   AND settings_v3.settings_hash = ?3",
                params![
                    normalized_path,
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    media_extraction_settings_hash(extraction_settings).to_vec(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("failed reading media-match record timestamp: {error}"))
    }

    pub fn audio_bucket_document_frequency(
        &self,
        settings_hash: &[u8; 32],
        bucket: u32,
    ) -> Result<Option<i64>, String> {
        self.connection
            .query_row(
                "SELECT document_frequency
                 FROM audio_anchor_buckets_v3
                 JOIN settings_v3 ON settings_v3.settings_id = audio_anchor_buckets_v3.settings_id
                 WHERE settings_v3.algorithm_version = ?1
                   AND settings_v3.settings_hash = ?2
                   AND audio_anchor_buckets_v3.bucket = ?3",
                params![
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    settings_hash.as_slice(),
                    i64::from(bucket),
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("failed reading media-match anchor frequency: {error}"))
    }

    pub fn positive_anchor_bucket_count(&self) -> Result<usize, String> {
        self.connection
            .query_row(
                "SELECT COUNT(*)
                 FROM audio_anchor_buckets_v3
                 WHERE document_frequency > 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(|error| format!("failed reading media-match anchor stats count: {error}"))
    }

    pub fn sqlite_size_report(&self) -> Result<MediaMatchV3SqliteSizeReport, String> {
        media_match_v3_sqlite_size_report(&self.root, &self.connection)
    }

    pub fn delete_fingerprints(&self, normalized_path: &str) -> Result<(), String> {
        delete_media_match_v3_fingerprints_and_anchors(&self.connection, normalized_path)
    }

    pub fn delete_file(&self, normalized_path: &str) -> Result<(), String> {
        delete_media_match_v3_file_and_fingerprints(&self.connection, normalized_path)
    }
}

fn fingerprint_count(
    connection: &Connection,
    extraction_settings: &MediaExtractionSettings,
) -> Result<usize, String> {
    connection
        .query_row(
            "SELECT COUNT(*)
             FROM fingerprints_v3
             JOIN settings_v3 ON settings_v3.settings_id = fingerprints_v3.settings_id
             WHERE settings_v3.algorithm_version = ?1
               AND settings_v3.settings_hash = ?2",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                media_extraction_settings_hash(extraction_settings).to_vec(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("failed reading media-match fingerprint count: {error}"))
}

fn inventory_count(connection: &Connection) -> Result<usize, String> {
    connection
        .query_row("SELECT COUNT(*) FROM media_files_v3", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("failed reading media-match inventory count: {error}"))
}

fn active_media_index_root(root: &Path) -> PathBuf {
    let manifest_path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    let Some(generation) = fs::read(&manifest_path)
        .ok()
        .and_then(|contents| serde_json::from_slice::<serde_json::Value>(&contents).ok())
        .and_then(|manifest| {
            (manifest.get("version").and_then(serde_json::Value::as_u64)
                == Some(u64::from(MEDIA_INDEX_MANIFEST_VERSION)))
            .then(|| {
                manifest
                    .get("generation")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .flatten()
        })
        .filter(|generation| {
            !generation.is_empty()
                && generation
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    else {
        return root.to_path_buf();
    };
    root.join(MEDIA_INDEX_GENERATIONS_DIR).join(generation)
}

fn write_media_index_manifest(root: &Path, generation: &str) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "failed creating media-match manifest directory '{}': {error}",
            root.display()
        )
    })?;
    let manifest_path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    let temporary_path = root.join(format!(
        "{MEDIA_INDEX_MANIFEST_FILE}.tmp-{}",
        media_index_transaction_unique()
    ));
    let manifest = serde_json::to_vec(&serde_json::json!({
        "version": MEDIA_INDEX_MANIFEST_VERSION,
        "generation": generation,
    }))
    .expect("media-index manifest serialization cannot fail");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| {
            format!(
                "failed creating media-match manifest staging file '{}': {error}",
                temporary_path.display()
            )
        })?;
    file.write_all(&manifest)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            format!(
                "failed flushing media-match manifest staging file '{}': {error}",
                temporary_path.display()
            )
        })?;
    drop(file);
    if let Err(error) = atomic_replace_path(&temporary_path, &manifest_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "failed activating media-match generation manifest '{}': {error}",
            manifest_path.display()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are stable, NUL-terminated UTF-16 paths for the duration of the call.
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn online_backup_database(source_path: &Path, destination_path: &Path) -> Result<(), String> {
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed creating media-match backup directory '{}': {error}",
                parent.display()
            )
        })?;
    }
    remove_sqlite_file_set(destination_path)?;
    let source = Connection::open(source_path).map_err(|error| {
        format!(
            "failed opening media-match source database '{}': {error}",
            source_path.display()
        )
    })?;
    let mut destination = Connection::open(destination_path).map_err(|error| {
        format!(
            "failed opening media-match backup destination '{}': {error}",
            destination_path.display()
        )
    })?;
    {
        let backup = Backup::new(&source, &mut destination).map_err(|error| {
            format!(
                "failed starting online media-match backup '{}' to '{}': {error}",
                source_path.display(),
                destination_path.display()
            )
        })?;
        backup
            .run_to_completion(64, Duration::from_millis(5), None)
            .map_err(|error| {
                format!(
                    "failed completing online media-match backup '{}' to '{}': {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
    }
    destination
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .map_err(|error| {
            format!(
                "failed checkpointing media-match backup '{}': {error}",
                destination_path.display()
            )
        })?;
    Ok(())
}

fn validate_media_index_database(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "media-match index candidate '{}' is missing",
            path.display()
        ));
    }
    let connection = Connection::open(path).map_err(|error| {
        format!(
            "failed opening media-match index candidate '{}': {error}",
            path.display()
        )
    })?;
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| {
            format!(
                "failed validating media-match index candidate '{}': {error}",
                path.display()
            )
        })?;
    if quick_check != "ok" {
        return Err(format!(
            "media-match index candidate '{}' failed quick_check: {quick_check}",
            path.display()
        ));
    }
    for table in [
        "media_files_v3",
        "settings_v3",
        "fingerprints_v3",
        "audio_anchor_buckets_v3",
    ] {
        let exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| {
                format!(
                    "failed checking media-match schema in '{}': {error}",
                    path.display()
                )
            })?
            .is_some();
        if !exists {
            return Err(format!(
                "media-match index candidate '{}' is missing required table '{table}'",
                path.display()
            ));
        }
    }
    Ok(())
}

fn media_index_transaction_unique() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn path_with_appended_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn sqlite_file_set(path: &Path) -> [PathBuf; 4] {
    [
        path.to_path_buf(),
        path_with_appended_suffix(path, "-wal"),
        path_with_appended_suffix(path, "-shm"),
        path_with_appended_suffix(path, "-journal"),
    ]
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("failed removing '{}': {error}", path.display()))?;
    }
    Ok(())
}

fn remove_sqlite_file_set(path: &Path) -> Result<(), String> {
    for member in sqlite_file_set(path) {
        remove_file_if_exists(&member)?;
    }
    Ok(())
}

fn remove_directory_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("failed removing '{}': {error}", path.display()))?;
    }
    Ok(())
}

fn check_inventory_cancelled(is_cancelled: &mut impl FnMut() -> bool) -> Result<(), String> {
    if is_cancelled() {
        Err("Media Matching inventory scan was canceled.".to_owned())
    } else {
        Ok(())
    }
}

fn media_path_is_under_root(normalized_path: &str, normalized_root: &str) -> bool {
    normalized_path == normalized_root
        || normalized_path
            .strip_prefix(normalized_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
