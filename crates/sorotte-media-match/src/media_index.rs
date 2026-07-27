use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use rusqlite::{Connection, OptionalExtension, backup::Backup, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MEDIA_MATCH_ANCHOR_VERSION, MediaExtractionSettings, MediaFingerprintRecord, MediaMatchCache,
    MediaMatchV3RetrievalStats, MediaMatchV3SaveStats, MediaMatchV3SqliteSizeReport,
    media_extraction_settings_hash,
    v3_index::{
        anchor_stats_v3_dirty, delete_media_match_v3_file_and_fingerprints,
        delete_media_match_v3_fingerprints_and_anchors, load_media_match_v3_cache_for_settings,
        load_media_match_v3_record_for_path, media_match_v3_anchor_candidate_paths_with_stats,
        media_match_v3_index_path, media_match_v3_sqlite_size_report,
        open_existing_media_match_v3_index, open_media_match_v3_index, refresh_all_anchor_stats_v3,
        refresh_anchor_stats_v3, save_media_match_v3_record, save_media_match_v3_record_with_stats,
    },
};

const MEDIA_INDEX_MANIFEST_FILE: &str = "current.json";
const MEDIA_INDEX_ALTERNATE_MANIFEST_FILE: &str = "current-b.json";
const MEDIA_INDEX_ACTIVATION_LOCK_FILE: &str = ".media-index-activation.lock";
const MEDIA_INDEX_GENERATIONS_DIR: &str = "generations";
const MEDIA_INDEX_MANIFEST_VERSION: u32 = 3;
const MEDIA_INDEX_BUILD_PREFIX: &str = ".media-match-build-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaIndexCommitOutcome {
    Activated { cleanup_warning: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaIndexCommitError {
    NotActivated(String),
    StaleBase(String),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaIndexCommitFailurePoint {
    BeforeGenerationCreation,
    DuringGenerationCopy,
    DuringReplacementValidation,
    DuringGenerationParentSync,
    DuringManifestReplacement,
    AfterManifestReplacementBeforeDirectorySync,
    DuringStagingCleanup,
}

impl std::fmt::Display for MediaIndexCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotActivated(message) => formatter.write_str(message),
            Self::StaleBase(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MediaIndexCommitError {}

impl From<String> for MediaIndexCommitError {
    fn from(message: String) -> Self {
        Self::NotActivated(message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MediaIndexManifest {
    version: u32,
    epoch: u64,
    current: String,
    previous: Option<String>,
    checksum: String,
}

#[derive(Debug)]
enum ResolvedMediaIndexRoot {
    ExistingGeneration(PathBuf),
    LegacyOrNew(PathBuf),
}

#[derive(Debug)]
struct ResolvedMediaIndex {
    root: ResolvedMediaIndexRoot,
    epoch: u64,
    current_generation: Option<String>,
}

#[derive(Debug)]
enum ManifestRead {
    Missing,
    Valid(MediaIndexManifest),
    Legacy(MediaIndexManifest),
    CorruptKnownFormat(String),
    UnsupportedVersion(u32),
}

struct MediaIndexActivationLock {
    file: File,
}

impl Drop for MediaIndexActivationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

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
        let active_root = resolve_media_index_root(&self.root)
            .map(|resolved| match resolved.root {
                ResolvedMediaIndexRoot::ExistingGeneration(root)
                | ResolvedMediaIndexRoot::LegacyOrNew(root) => root,
            })
            .unwrap_or_else(|_| self.root.clone());
        media_match_v3_index_path(&active_root)
    }

    pub fn open(&self) -> Result<MediaIndexSession, String> {
        cleanup_abandoned_media_index_builds(&self.root);
        match resolve_media_index_root(&self.root)?.root {
            ResolvedMediaIndexRoot::ExistingGeneration(active_root) => {
                open_existing_media_match_v3_index(&active_root).map(|connection| {
                    MediaIndexSession {
                        root: active_root,
                        connection,
                    }
                })
            }
            ResolvedMediaIndexRoot::LegacyOrNew(active_root) => {
                open_media_match_v3_index(&active_root).map(|connection| MediaIndexSession {
                    root: active_root,
                    connection,
                })
            }
        }
    }
}

/// Isolates an index rebuild from the live WAL database until a validated same-directory swap.
#[derive(Debug)]
pub struct MediaIndexBuildTransaction {
    live_root: PathBuf,
    staging_root: PathBuf,
    had_live_index: bool,
    base_epoch: u64,
    base_generation: Option<String>,
    finished: bool,
    manifest_replaced: bool,
    manifest_durable: bool,
    created_generation_root: Option<PathBuf>,
    #[cfg(test)]
    test_failure_point: Option<MediaIndexCommitFailurePoint>,
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
        cleanup_abandoned_media_index_builds(&live_root);
        fs::create_dir_all(&staging_root).map_err(|error| {
            format!(
                "failed creating media-match staging directory '{}': {error}",
                staging_root.display()
            )
        })?;

        let resolved = resolve_media_index_root(&live_root)?;
        let base_epoch = resolved.epoch;
        let base_generation = resolved.current_generation;
        let active_root = match resolved.root {
            ResolvedMediaIndexRoot::ExistingGeneration(root)
            | ResolvedMediaIndexRoot::LegacyOrNew(root) => root,
        };
        let live_path = media_match_v3_index_path(&active_root);
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
            base_epoch,
            base_generation,
            finished: false,
            manifest_replaced: false,
            manifest_durable: false,
            created_generation_root: None,
            #[cfg(test)]
            test_failure_point: None,
        })
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub fn had_live_index(&self) -> bool {
        self.had_live_index
    }

    #[cfg(test)]
    pub(crate) fn set_test_failure_point(&mut self, point: MediaIndexCommitFailurePoint) {
        self.test_failure_point = Some(point);
    }

    #[cfg(test)]
    fn inject_test_failure(&self, point: MediaIndexCommitFailurePoint) -> Result<(), String> {
        if self.test_failure_point == Some(point) {
            Err(format!("injected media-index commit failure at {point:?}"))
        } else {
            Ok(())
        }
    }

    pub fn commit(mut self) -> Result<MediaIndexCommitOutcome, MediaIndexCommitError> {
        self.commit_inner()
    }

    fn commit_inner(&mut self) -> Result<MediaIndexCommitOutcome, MediaIndexCommitError> {
        let staging_path = media_match_v3_index_path(&self.staging_root);
        validate_media_index_database(&staging_path)?;
        let _activation_lock = acquire_media_index_activation_lock(&self.live_root)?;
        let resolved = resolve_media_index_root_locked(&self.live_root)?;
        if resolved.epoch != self.base_epoch || resolved.current_generation != self.base_generation
        {
            return Err(MediaIndexCommitError::StaleBase(format!(
                "media-match index changed while this rebuild was staging (base epoch {}, current epoch {}); retry the rebuild against the latest index",
                self.base_epoch, resolved.epoch
            )));
        }

        let unique = media_index_transaction_unique();
        let generation = format!("generation-{unique}");
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::BeforeGenerationCreation)?;
        let generations_root = create_or_validate_media_index_generations_root(&self.live_root)?;
        let generation_root = generations_root.join(&generation);
        fs::create_dir(&generation_root).map_err(|error| {
            format!(
                "failed creating media-match generation directory '{}': {error}",
                generation_root.display()
            )
        })?;
        self.created_generation_root = Some(generation_root.clone());
        let replacement_path = media_match_v3_index_path(&generation_root);
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::DuringGenerationCopy)?;
        online_backup_database(&staging_path, &replacement_path)?;
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::DuringReplacementValidation)?;
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
        sync_directory(&generation_root)?;
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::DuringGenerationParentSync)?;
        // The database and its own directory must become durable before either manifest can name
        // the generation. Synchronize both directory entries in order: first generation-X inside
        // generations/, then generations/ inside the live index root.
        sync_directory(&generations_root)?;
        sync_directory(&self.live_root)?;
        let previous = self.base_generation.clone();
        let next_epoch = self.base_epoch.checked_add(1).ok_or_else(|| {
            MediaIndexCommitError::NotActivated(
                "media-match activation epoch is exhausted; the index cannot be safely replaced"
                    .to_owned(),
            )
        })?;
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::DuringManifestReplacement)?;
        #[cfg(test)]
        let manifest_write = write_media_index_manifest_with_post_replace_check(
            &self.live_root,
            next_epoch,
            &generation,
            previous.as_deref(),
            || {
                self.inject_test_failure(
                    MediaIndexCommitFailurePoint::AfterManifestReplacementBeforeDirectorySync,
                )
            },
        )?;
        #[cfg(not(test))]
        let manifest_write = write_media_index_manifest(
            &self.live_root,
            next_epoch,
            &generation,
            previous.as_deref(),
        )?;
        self.manifest_replaced = true;
        self.manifest_durable = manifest_write.durable;
        self.created_generation_root = None;
        self.finished = true;
        let mut cleanup_warnings = Vec::new();
        if let Some(warning) = manifest_write.warning {
            cleanup_warnings.push(warning);
        }
        #[cfg(test)]
        let staging_cleanup = self
            .inject_test_failure(MediaIndexCommitFailurePoint::DuringStagingCleanup)
            .and_then(|()| remove_directory_if_exists(&self.staging_root));
        #[cfg(not(test))]
        let staging_cleanup = remove_directory_if_exists(&self.staging_root);
        if let Err(error) = staging_cleanup {
            cleanup_warnings.push(error);
        }
        if self.manifest_durable
            && let Err(error) =
                read_best_media_index_manifest(&self.live_root).and_then(|manifest| {
                    let manifest = manifest.ok_or_else(|| {
                        "media-match activation manifest disappeared before collection".to_owned()
                    })?;
                    collect_old_media_index_generations(
                        &self.live_root,
                        &manifest.current,
                        manifest.previous.as_deref(),
                    )
                })
        {
            cleanup_warnings.push(error);
        }
        Ok(MediaIndexCommitOutcome::Activated {
            cleanup_warning: (!cleanup_warnings.is_empty()).then(|| cleanup_warnings.join("; ")),
        })
    }

    pub fn abort(mut self) -> Result<(), String> {
        remove_directory_if_exists(&self.staging_root)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for MediaIndexBuildTransaction {
    fn drop(&mut self) {
        if !self.manifest_replaced
            && let Some(generation_root) = self.created_generation_root.take()
        {
            let _ = remove_directory_if_exists(&generation_root);
        }
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

fn acquire_media_index_activation_lock(root: &Path) -> Result<MediaIndexActivationLock, String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "failed creating media-match index root '{}' for activation locking: {error}",
            root.display()
        )
    })?;
    let path = root.join(MEDIA_INDEX_ACTIVATION_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "failed opening media-match activation lock '{}': {error}",
                path.display()
            )
        })?;
    file.lock().map_err(|error| {
        format!(
            "failed acquiring media-match activation lock '{}': {error}",
            path.display()
        )
    })?;
    Ok(MediaIndexActivationLock { file })
}

fn resolve_media_index_root(root: &Path) -> Result<ResolvedMediaIndex, String> {
    let _activation_lock = acquire_media_index_activation_lock(root)?;
    resolve_media_index_root_locked(root)
}

fn resolve_media_index_root_locked(root: &Path) -> Result<ResolvedMediaIndex, String> {
    let manifest = read_best_media_index_manifest(root)?;
    if let Some(mut manifest) = manifest {
        if manifest.version != MEDIA_INDEX_MANIFEST_VERSION {
            manifest.version = MEDIA_INDEX_MANIFEST_VERSION;
            manifest.epoch = manifest.epoch.max(1);
            manifest.checksum = manifest_checksum(
                manifest.version,
                manifest.epoch,
                &manifest.current,
                manifest.previous.as_deref(),
            );
            write_media_index_manifest(
                root,
                manifest.epoch,
                &manifest.current,
                manifest.previous.as_deref(),
            )?;
        } else if media_index_manifest_slots_need_repair(root, &manifest) {
            write_media_index_manifest(
                root,
                manifest.epoch,
                &manifest.current,
                manifest.previous.as_deref(),
            )?;
        }
        let generations_root = existing_media_index_generations_root(root)?
            .unwrap_or_else(|| root.join(MEDIA_INDEX_GENERATIONS_DIR));
        let current_path = generations_root.join(&manifest.current);
        if validate_media_index_database(&media_match_v3_index_path(&current_path)).is_ok() {
            let _ = collect_old_media_index_generations(
                root,
                &manifest.current,
                manifest.previous.as_deref(),
            );
            return Ok(ResolvedMediaIndex {
                root: ResolvedMediaIndexRoot::ExistingGeneration(current_path),
                epoch: manifest.epoch,
                current_generation: Some(manifest.current),
            });
        }
        if let Some(previous) = manifest.previous.as_deref() {
            let previous_path = generations_root.join(previous);
            if validate_media_index_database(&media_match_v3_index_path(&previous_path)).is_ok() {
                let recovery_epoch = manifest.epoch.checked_add(1).ok_or_else(|| {
                    "media-match activation epoch is exhausted during rollback recovery".to_owned()
                })?;
                write_media_index_manifest(root, recovery_epoch, previous, None)?;
                let _ = collect_old_media_index_generations(root, previous, None);
                return Ok(ResolvedMediaIndex {
                    root: ResolvedMediaIndexRoot::ExistingGeneration(previous_path),
                    epoch: recovery_epoch,
                    current_generation: Some(previous.to_owned()),
                });
            }
        }
        let legacy_path = media_match_v3_index_path(root);
        if legacy_path.exists() && validate_media_index_database(&legacy_path).is_ok() {
            return Ok(ResolvedMediaIndex {
                root: ResolvedMediaIndexRoot::LegacyOrNew(root.to_path_buf()),
                epoch: manifest.epoch,
                current_generation: None,
            });
        }
        return Err(format!(
            "media-match manifests under '{}' reference no valid activated index",
            root.display()
        ));
    }

    let legacy_path = media_match_v3_index_path(root);
    if legacy_path.exists() {
        validate_media_index_database(&legacy_path).map_err(|error| {
            format!(
                "media-match generation recovery failed and the legacy index is invalid: {error}"
            )
        })?;
        return Ok(ResolvedMediaIndex {
            root: ResolvedMediaIndexRoot::LegacyOrNew(root.to_path_buf()),
            epoch: 0,
            current_generation: None,
        });
    }
    if !media_index_generation_names(root).is_empty() {
        return Err(format!(
            "media-match generation data exists under '{}', but neither checksummed activation manifest is recoverable",
            root.display()
        ));
    }
    Ok(ResolvedMediaIndex {
        root: ResolvedMediaIndexRoot::LegacyOrNew(root.to_path_buf()),
        epoch: 0,
        current_generation: None,
    })
}

fn valid_generation_name(generation: &str) -> bool {
    !generation.is_empty()
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn metadata_is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn validate_real_media_index_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed inspecting directory '{}': {error}", path.display()))?;
    if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
        return Err(format!(
            "refusing link, reparse point, or non-directory media-index path '{}'",
            path.display()
        ));
    }
    Ok(())
}

fn existing_media_index_generations_root(root: &Path) -> Result<Option<PathBuf>, String> {
    let generations_root = root.join(MEDIA_INDEX_GENERATIONS_DIR);
    match fs::symlink_metadata(&generations_root) {
        Ok(_) => {
            validate_real_media_index_directory(&generations_root)?;
            Ok(Some(generations_root))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed inspecting media-match generations directory '{}': {error}",
            generations_root.display()
        )),
    }
}

fn create_or_validate_media_index_generations_root(root: &Path) -> Result<PathBuf, String> {
    if let Some(generations_root) = existing_media_index_generations_root(root)? {
        return Ok(generations_root);
    }
    let generations_root = root.join(MEDIA_INDEX_GENERATIONS_DIR);
    fs::create_dir(&generations_root).map_err(|error| {
        format!(
            "failed creating media-match generations directory '{}': {error}",
            generations_root.display()
        )
    })?;
    validate_real_media_index_directory(&generations_root)?;
    Ok(generations_root)
}

fn manifest_checksum(version: u32, epoch: u64, current: &str, previous: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(version.to_le_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.update((current.len() as u64).to_le_bytes());
    hasher.update(current.as_bytes());
    let previous = previous.unwrap_or_default();
    hasher.update((previous.len() as u64).to_le_bytes());
    hasher.update(previous.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_media_index_manifest_slot(path: &Path) -> ManifestRead {
    if !path.is_file() {
        return ManifestRead::Missing;
    }
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) => {
            return ManifestRead::CorruptKnownFormat(format!(
                "failed reading media-match manifest '{}': {error}",
                path.display()
            ));
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&contents) {
        Ok(value) => value,
        Err(error) => {
            return ManifestRead::CorruptKnownFormat(format!(
                "failed parsing media-match manifest '{}': {error}",
                path.display()
            ));
        }
    };
    let Some(version) = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
    else {
        return ManifestRead::CorruptKnownFormat(format!(
            "media-match manifest '{}' has no supported version",
            path.display()
        ));
    };
    let manifest = if version == MEDIA_INDEX_MANIFEST_VERSION {
        match serde_json::from_value::<MediaIndexManifest>(value) {
            Ok(manifest) => manifest,
            Err(error) => {
                return ManifestRead::CorruptKnownFormat(format!(
                    "failed decoding media-match manifest '{}': {error}",
                    path.display()
                ));
            }
        }
    } else if version == 1 {
        let Some(current) = value.get("generation").and_then(serde_json::Value::as_str) else {
            return ManifestRead::CorruptKnownFormat(format!(
                "legacy media-match manifest '{}' has no generation",
                path.display()
            ));
        };
        MediaIndexManifest {
            version,
            epoch: 0,
            current: current.to_owned(),
            previous: None,
            checksum: String::new(),
        }
    } else if version == 2 {
        let Some(current) = value.get("current").and_then(serde_json::Value::as_str) else {
            return ManifestRead::CorruptKnownFormat(format!(
                "legacy media-match manifest '{}' has no current generation",
                path.display()
            ));
        };
        MediaIndexManifest {
            version,
            epoch: 0,
            current: current.to_owned(),
            previous: value
                .get("previous")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            checksum: String::new(),
        }
    } else {
        return ManifestRead::UnsupportedVersion(version);
    };
    if !valid_generation_name(&manifest.current)
        || manifest
            .previous
            .as_deref()
            .is_some_and(|previous| !valid_generation_name(previous))
    {
        return ManifestRead::CorruptKnownFormat(format!(
            "media-match manifest '{}' contains an invalid generation name",
            path.display()
        ));
    }
    if version == MEDIA_INDEX_MANIFEST_VERSION {
        let expected = manifest_checksum(
            manifest.version,
            manifest.epoch,
            &manifest.current,
            manifest.previous.as_deref(),
        );
        if manifest.checksum != expected {
            return ManifestRead::CorruptKnownFormat(format!(
                "media-match manifest '{}' failed checksum validation",
                path.display()
            ));
        }
        ManifestRead::Valid(manifest)
    } else {
        ManifestRead::Legacy(manifest)
    }
}

fn read_best_media_index_manifest(root: &Path) -> Result<Option<MediaIndexManifest>, String> {
    let reads = [
        read_media_index_manifest_slot(&root.join(MEDIA_INDEX_MANIFEST_FILE)),
        read_media_index_manifest_slot(&root.join(MEDIA_INDEX_ALTERNATE_MANIFEST_FILE)),
    ];
    if let Some(version) = reads.iter().find_map(|read| match read {
        ManifestRead::UnsupportedVersion(version) => Some(*version),
        _ => None,
    }) {
        return Err(format!(
            "media-match activation manifest uses unsupported version {version}; refusing to rewrite or collect generations"
        ));
    }
    let mut valid = reads
        .iter()
        .filter_map(|read| match read {
            ManifestRead::Valid(manifest) => Some(manifest.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    valid.sort_by_key(|manifest| manifest.epoch);
    if let Some(manifest) = valid.pop() {
        return Ok(Some(manifest));
    }
    if let Some(manifest) = reads.iter().find_map(|read| match read {
        ManifestRead::Legacy(manifest) => Some(manifest.clone()),
        _ => None,
    }) {
        return Ok(Some(manifest));
    }
    let errors = reads
        .iter()
        .filter_map(|read| match read {
            ManifestRead::CorruptKnownFormat(error) => Some(error.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(format!(
            "no recoverable media-match activation manifest remains: {}",
            errors.join("; ")
        ));
    }
    Ok(None)
}

fn media_index_manifest_slots_need_repair(root: &Path, selected: &MediaIndexManifest) -> bool {
    [
        root.join(MEDIA_INDEX_MANIFEST_FILE),
        root.join(MEDIA_INDEX_ALTERNATE_MANIFEST_FILE),
    ]
    .iter()
    .any(|path| match read_media_index_manifest_slot(path) {
        ManifestRead::Valid(manifest) => manifest != *selected,
        _ => true,
    })
}

fn media_index_generation_names(root: &Path) -> Vec<String> {
    let Ok(Some(generations_root)) = existing_media_index_generations_root(root) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&generations_root) else {
        return Vec::new();
    };
    let mut entries = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
                return None;
            }
            let name = entry.file_name().to_str()?.to_owned();
            if !valid_generation_name(&name) {
                return None;
            }
            Some(name)
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn cleanup_abandoned_media_index_builds(root: &Path) {
    let Some(cache_root) = root.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(cache_root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name
            .to_str()
            .filter(|name| name.starts_with(MEDIA_INDEX_BUILD_PREFIX))
        else {
            continue;
        };
        if media_index_build_owner_is_running(name) {
            continue;
        }
        let _ = remove_directory_if_exists(&entry.path());
    }
}

fn media_index_build_owner_pid(name: &str) -> Option<u32> {
    name.strip_prefix(MEDIA_INDEX_BUILD_PREFIX)?
        .split_once('-')?
        .0
        .parse()
        .ok()
}

#[cfg(windows)]
fn media_index_build_owner_is_running(name: &str) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    let Some(pid) = media_index_build_owner_pid(name) else {
        return false;
    };
    // SAFETY: OpenProcess requests synchronization access only. A non-null handle is waited on
    // without dereferencing and closed exactly once below.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: handle is valid until CloseHandle and a zero-duration wait does not mutate it.
    let running = unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT };
    // SAFETY: handle was returned by OpenProcess and has not previously been closed.
    unsafe {
        CloseHandle(handle);
    }
    running
}

#[cfg(not(windows))]
fn media_index_build_owner_is_running(name: &str) -> bool {
    let Some(pid) = media_index_build_owner_pid(name) else {
        return false;
    };
    if pid == std::process::id() {
        return true;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        // Without a portable process-liveness primitive, preserve another process's build.
        true
    }
}

fn collect_old_media_index_generations(
    root: &Path,
    current: &str,
    previous: Option<&str>,
) -> Result<(), String> {
    let Some(generations_root) = existing_media_index_generations_root(root)? else {
        return Ok(());
    };
    let entries = fs::read_dir(&generations_root).map_err(|error| {
        format!(
            "failed reading media-match generations directory '{}': {error}",
            generations_root.display()
        )
    })?;
    let mut warnings = Vec::new();
    let legacy_path = media_match_v3_index_path(root);
    if previous.is_some()
        && legacy_path.exists()
        && let Err(error) = remove_sqlite_file_set(&legacy_path)
    {
        warnings.push(error);
    }
    for entry in entries.filter_map(Result::ok) {
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if name == current || previous == Some(name.as_str()) {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("failed inspecting '{}': {error}", path.display()));
                continue;
            }
        };
        if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
            warnings.push(format!(
                "refusing to remove link, reparse point, or non-directory generation '{}'",
                path.display()
            ));
            continue;
        }
        if let Err(error) = remove_directory_if_exists(&path) {
            warnings.push(error);
        }
    }
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "media-match generation cleanup deferred: {}",
            warnings.join("; ")
        ))
    }
}

#[derive(Debug)]
struct MediaIndexManifestWriteOutcome {
    durable: bool,
    warning: Option<String>,
}

fn write_media_index_manifest(
    root: &Path,
    epoch: u64,
    generation: &str,
    previous: Option<&str>,
) -> Result<MediaIndexManifestWriteOutcome, String> {
    write_media_index_manifest_with_post_replace_check(root, epoch, generation, previous, || Ok(()))
}

fn write_media_index_manifest_with_post_replace_check(
    root: &Path,
    epoch: u64,
    generation: &str,
    previous: Option<&str>,
    post_replace_check: impl FnOnce() -> Result<(), String>,
) -> Result<MediaIndexManifestWriteOutcome, String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "failed creating media-match manifest directory '{}': {error}",
            root.display()
        )
    })?;
    let checksum = manifest_checksum(MEDIA_INDEX_MANIFEST_VERSION, epoch, generation, previous);
    let manifest = serde_json::to_vec(&MediaIndexManifest {
        version: MEDIA_INDEX_MANIFEST_VERSION,
        epoch,
        current: generation.to_owned(),
        previous: previous.map(ToOwned::to_owned),
        checksum,
    })
    .expect("media-index manifest serialization cannot fail");
    for manifest_file in [
        MEDIA_INDEX_MANIFEST_FILE,
        MEDIA_INDEX_ALTERNATE_MANIFEST_FILE,
    ] {
        let path = root.join(manifest_file);
        if path.is_dir() {
            return Err(format!(
                "media-match activation manifest '{}' is a directory",
                path.display()
            ));
        }
    }
    // The alternate slot is the activation boundary. Updating the conventional primary slot
    // afterwards keeps older readers and diagnostics useful without making recovery depend on it.
    write_media_index_manifest_slot(root, MEDIA_INDEX_ALTERNATE_MANIFEST_FILE, &manifest)?;
    let primary_path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    let primary_warning =
        write_media_index_manifest_slot(root, MEDIA_INDEX_MANIFEST_FILE, &manifest)
            .err()
            .map(|error| {
                format!(
                    "media-match generation activated through '{}', but primary manifest '{}' could not be refreshed: {error}",
                    MEDIA_INDEX_ALTERNATE_MANIFEST_FILE,
                    primary_path.display()
                )
            });
    match post_replace_check().and_then(|()| sync_directory(root)) {
        Ok(()) => Ok(MediaIndexManifestWriteOutcome {
            durable: true,
            warning: primary_warning,
        }),
        Err(error) => Ok(MediaIndexManifestWriteOutcome {
            durable: false,
            warning: Some(format!(
                "media-match generation manifest slots under '{}' were replaced but directory durability could not be confirmed: {error}",
                root.display()
            )),
        }),
    }
}

fn write_media_index_manifest_slot(
    root: &Path,
    manifest_file: &str,
    manifest: &[u8],
) -> Result<(), String> {
    let manifest_path = root.join(manifest_file);
    let temporary_path = root.join(format!(
        "{manifest_file}.tmp-{}",
        media_index_transaction_unique()
    ));
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
    file.write_all(manifest)
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
fn sync_directory(path: &Path) -> Result<(), String> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed synchronizing media-match directory '{}': {error}",
                path.display()
            )
        })
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), String> {
    // MoveFileExW with MOVEFILE_WRITE_THROUGH provides the durable replacement boundary.
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
    let root = path.parent().ok_or_else(|| {
        format!(
            "media-match index candidate '{}' has no parent directory",
            path.display()
        )
    })?;
    open_existing_media_match_v3_index(root)
        .map(drop)
        .map_err(|error| {
            format!(
                "media-match index candidate '{}' failed activated-open validation: {error}",
                path.display()
            )
        })
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

#[cfg(test)]
mod generation_link_safety_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn generation_cleanup_does_not_follow_generations_root_junction() {
        let unique = media_index_transaction_unique();
        let fixture =
            std::env::temp_dir().join(format!("sorotte-media-index-generation-junction-{unique}"));
        let live_root = fixture.join("live");
        let outside_root = fixture.join("outside");
        let outside_generation = outside_root.join("generation-stale");
        fs::create_dir_all(&live_root).expect("live index root should be created");
        fs::create_dir_all(&outside_generation).expect("outside generation should be created");
        let canary = outside_generation.join("must-not-be-deleted.txt");
        fs::write(&canary, b"outside").expect("outside canary should be written");

        let generations_link = live_root.join(MEDIA_INDEX_GENERATIONS_DIR);
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&generations_link)
            .arg(&outside_root)
            .status()
            .expect("junction creation command should launch");
        assert!(status.success(), "junction creation should succeed");

        let cleanup = collect_old_media_index_generations(&live_root, "generation-current", None);
        let creation = create_or_validate_media_index_generations_root(&live_root);
        let canary_survived = canary.is_file();

        fs::remove_dir(&generations_link)
            .expect("generations junction should be removed without following it");
        fs::remove_dir_all(&fixture).expect("fixture should be removed");

        assert!(
            cleanup.is_err(),
            "generation cleanup must reject a junction root"
        );
        assert!(
            creation.is_err(),
            "generation creation must reject a junction root"
        );
        assert!(canary_survived, "the junction target must remain untouched");
    }

    #[cfg(windows)]
    #[test]
    fn generation_cleanup_does_not_follow_generation_entry_junction() {
        let unique = media_index_transaction_unique();
        let fixture = std::env::temp_dir().join(format!(
            "sorotte-media-index-generation-entry-junction-{unique}"
        ));
        let live_root = fixture.join("live");
        let generations_root = live_root.join(MEDIA_INDEX_GENERATIONS_DIR);
        let outside_generation = fixture.join("outside-generation");
        fs::create_dir_all(&generations_root).expect("generations root should be created");
        fs::create_dir_all(&outside_generation).expect("outside generation should be created");
        let canary = outside_generation.join("must-not-be-deleted.txt");
        fs::write(&canary, b"outside").expect("outside canary should be written");

        let generation_link = generations_root.join("generation-stale");
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&generation_link)
            .arg(&outside_generation)
            .status()
            .expect("junction creation command should launch");
        assert!(status.success(), "junction creation should succeed");

        let cleanup = collect_old_media_index_generations(
            &live_root,
            "generation-current",
            Some("generation-previous"),
        );
        let canary_survived = canary.is_file();

        fs::remove_dir(&generation_link)
            .expect("generation junction should be removed without following it");
        fs::remove_dir_all(&fixture).expect("fixture should be removed");

        assert!(
            cleanup.is_err(),
            "generation cleanup must reject a junction entry"
        );
        assert!(canary_survived, "the junction target must remain untouched");
    }

    #[cfg(unix)]
    #[test]
    fn generation_cleanup_does_not_follow_generations_root_symlink() {
        use std::os::unix::fs::symlink;

        let unique = media_index_transaction_unique();
        let fixture =
            std::env::temp_dir().join(format!("sorotte-media-index-generation-link-{unique}"));
        let live_root = fixture.join("live");
        let outside_root = fixture.join("outside");
        let outside_generation = outside_root.join("generation-stale");
        fs::create_dir_all(&live_root).expect("live index root should be created");
        fs::create_dir_all(&outside_generation).expect("outside generation should be created");
        let canary = outside_generation.join("must-not-be-deleted.txt");
        fs::write(&canary, b"outside").expect("outside canary should be written");

        let generations_link = live_root.join(MEDIA_INDEX_GENERATIONS_DIR);
        symlink(&outside_root, &generations_link).expect("generations symlink should be created");

        let cleanup = collect_old_media_index_generations(
            &live_root,
            "generation-current",
            Some("generation-previous"),
        );
        let creation = create_or_validate_media_index_generations_root(&live_root);
        let canary_survived = canary.is_file();

        fs::remove_file(&generations_link).expect("generations symlink should be removed");
        fs::remove_dir_all(&fixture).expect("fixture should be removed");

        assert!(
            cleanup.is_err(),
            "generation cleanup must reject a symlink root"
        );
        assert!(
            creation.is_err(),
            "generation creation must reject a symlink root"
        );
        assert!(canary_survived, "the symlink target must remain untouched");
    }
}
