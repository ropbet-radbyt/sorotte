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
use serde::{Deserialize, Serialize};

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
const MEDIA_INDEX_GENERATIONS_DIR: &str = "generations";
const MEDIA_INDEX_MANIFEST_VERSION: u32 = 2;
const MEDIA_INDEX_BUILD_PREFIX: &str = ".media-match-build-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaIndexCommitOutcome {
    Activated { cleanup_warning: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaIndexCommitError {
    NotActivated(String),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaIndexCommitFailurePoint {
    BeforeGenerationCreation,
    DuringGenerationCopy,
    DuringReplacementValidation,
    DuringManifestReplacement,
    AfterManifestReplacementBeforeDirectorySync,
    DuringStagingCleanup,
}

impl std::fmt::Display for MediaIndexCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotActivated(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MediaIndexCommitError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MediaIndexManifest {
    version: u32,
    current: String,
    previous: Option<String>,
}

#[derive(Debug)]
enum ResolvedMediaIndexRoot {
    ExistingGeneration(PathBuf),
    LegacyOrNew(PathBuf),
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
            .map(|resolved| match resolved {
                ResolvedMediaIndexRoot::ExistingGeneration(root)
                | ResolvedMediaIndexRoot::LegacyOrNew(root) => root,
            })
            .unwrap_or_else(|_| self.root.clone());
        media_match_v3_index_path(&active_root)
    }

    pub fn open(&self) -> Result<MediaIndexSession, String> {
        cleanup_abandoned_media_index_builds(&self.root);
        match resolve_media_index_root(&self.root)? {
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
    previous_generation: Option<String>,
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

        let (active_root, previous_generation) = match resolve_media_index_root(&live_root)? {
            ResolvedMediaIndexRoot::ExistingGeneration(root) => {
                let generation = root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| valid_generation_name(name))
                    .ok_or_else(|| {
                        format!(
                            "resolved media-match generation '{}' has no valid generation name",
                            root.display()
                        )
                    })?
                    .to_owned();
                (root, Some(generation))
            }
            ResolvedMediaIndexRoot::LegacyOrNew(root) => (root, None),
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
            previous_generation,
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
            .map_err(MediaIndexCommitError::NotActivated)
    }

    fn commit_inner(&mut self) -> Result<MediaIndexCommitOutcome, String> {
        let staging_path = media_match_v3_index_path(&self.staging_root);
        validate_media_index_database(&staging_path)?;

        let unique = media_index_transaction_unique();
        let generation = format!("generation-{unique}");
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::BeforeGenerationCreation)?;
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
        let previous = self.previous_generation.clone();
        #[cfg(test)]
        self.inject_test_failure(MediaIndexCommitFailurePoint::DuringManifestReplacement)?;
        #[cfg(test)]
        let manifest_write = write_media_index_manifest_with_post_replace_check(
            &self.live_root,
            &generation,
            previous.as_deref(),
            || {
                self.inject_test_failure(
                    MediaIndexCommitFailurePoint::AfterManifestReplacementBeforeDirectorySync,
                )
            },
        )?;
        #[cfg(not(test))]
        let manifest_write =
            write_media_index_manifest(&self.live_root, &generation, previous.as_deref())?;
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
            && let Err(error) = collect_old_media_index_generations(
                &self.live_root,
                &generation,
                previous.as_deref(),
            )
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

fn resolve_media_index_root(root: &Path) -> Result<ResolvedMediaIndexRoot, String> {
    let manifest_path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    let manifest_present = manifest_path.exists();
    let manifest = read_media_index_manifest(root).ok().flatten();
    let generation_names = media_index_generation_names(root);
    let mut candidates = Vec::new();
    if let Some(manifest) = manifest.as_ref() {
        candidates.push(manifest.current.clone());
        if let Some(previous) = manifest.previous.as_ref() {
            candidates.push(previous.clone());
        }
    }
    for generation in &generation_names {
        if !candidates.contains(generation) {
            candidates.push(generation.clone());
        }
    }
    let valid_candidates = candidates
        .into_iter()
        .filter(|generation| {
            validate_media_index_database(&media_match_v3_index_path(
                &root.join(MEDIA_INDEX_GENERATIONS_DIR).join(generation),
            ))
            .is_ok()
        })
        .collect::<Vec<_>>();
    if let Some(current) = valid_candidates.first() {
        let previous = valid_candidates.get(1).map(String::as_str);
        let manifest_matches = manifest.as_ref().is_some_and(|manifest| {
            manifest.version == MEDIA_INDEX_MANIFEST_VERSION
                && manifest.current == *current
                && manifest.previous.as_deref() == previous
        });
        if !manifest_matches {
            write_media_index_manifest(root, current, previous)?;
        }
        let _ = collect_old_media_index_generations(root, current, previous);
        return Ok(ResolvedMediaIndexRoot::ExistingGeneration(
            root.join(MEDIA_INDEX_GENERATIONS_DIR).join(current),
        ));
    }

    let legacy_path = media_match_v3_index_path(root);
    if legacy_path.exists() {
        validate_media_index_database(&legacy_path).map_err(|error| {
            format!(
                "media-match generation recovery failed and the legacy index is invalid: {error}"
            )
        })?;
        return Ok(ResolvedMediaIndexRoot::LegacyOrNew(root.to_path_buf()));
    }
    if manifest_present || !generation_names.is_empty() {
        return Err(format!(
            "media-match manifest or generation data exists under '{}', but no valid activated index can be recovered",
            root.display()
        ));
    }
    Ok(ResolvedMediaIndexRoot::LegacyOrNew(root.to_path_buf()))
}

fn valid_generation_name(generation: &str) -> bool {
    !generation.is_empty()
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn read_media_index_manifest(root: &Path) -> Result<Option<MediaIndexManifest>, String> {
    let path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read(&path).map_err(|error| {
        format!(
            "failed reading media-match manifest '{}': {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_slice::<serde_json::Value>(&contents).map_err(|error| {
        format!(
            "failed parsing media-match manifest '{}': {error}",
            path.display()
        )
    })?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            format!(
                "media-match manifest '{}' has no supported version",
                path.display()
            )
        })?;
    let manifest = if version == MEDIA_INDEX_MANIFEST_VERSION {
        serde_json::from_value::<MediaIndexManifest>(value).map_err(|error| {
            format!(
                "failed decoding media-match manifest '{}': {error}",
                path.display()
            )
        })?
    } else if version == 1 {
        let current = value
            .get("generation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                format!(
                    "legacy media-match manifest '{}' has no generation",
                    path.display()
                )
            })?
            .to_owned();
        MediaIndexManifest {
            version,
            current,
            previous: None,
        }
    } else {
        return Err(format!(
            "media-match manifest '{}' uses unsupported version {version}",
            path.display()
        ));
    };
    if !valid_generation_name(&manifest.current)
        || manifest
            .previous
            .as_deref()
            .is_some_and(|previous| !valid_generation_name(previous))
    {
        return Err(format!(
            "media-match manifest '{}' contains an invalid generation name",
            path.display()
        ));
    }
    Ok(Some(manifest))
}

fn media_index_generation_names(root: &Path) -> Vec<String> {
    let generations_root = root.join(MEDIA_INDEX_GENERATIONS_DIR);
    let Ok(entries) = fs::read_dir(&generations_root) else {
        return Vec::new();
    };
    let mut entries = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name().to_str()?.to_owned();
            if !valid_generation_name(&name) {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((modified, name))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.cmp(left));
    entries.into_iter().map(|(_, name)| name).collect()
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
    let generations_root = root.join(MEDIA_INDEX_GENERATIONS_DIR);
    let Ok(entries) = fs::read_dir(&generations_root) else {
        return Ok(());
    };
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
        if let Err(error) = remove_directory_if_exists(&entry.path()) {
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
    generation: &str,
    previous: Option<&str>,
) -> Result<MediaIndexManifestWriteOutcome, String> {
    write_media_index_manifest_with_post_replace_check(root, generation, previous, || Ok(()))
}

fn write_media_index_manifest_with_post_replace_check(
    root: &Path,
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
    let manifest_path = root.join(MEDIA_INDEX_MANIFEST_FILE);
    let temporary_path = root.join(format!(
        "{MEDIA_INDEX_MANIFEST_FILE}.tmp-{}",
        media_index_transaction_unique()
    ));
    let manifest = serde_json::to_vec(&MediaIndexManifest {
        version: MEDIA_INDEX_MANIFEST_VERSION,
        current: generation.to_owned(),
        previous: previous.map(ToOwned::to_owned),
    })
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
    match post_replace_check().and_then(|()| sync_directory(root)) {
        Ok(()) => Ok(MediaIndexManifestWriteOutcome {
            durable: true,
            warning: None,
        }),
        Err(error) => Ok(MediaIndexManifestWriteOutcome {
            durable: false,
            warning: Some(format!(
                "media-match generation manifest '{}' was replaced but directory durability could not be confirmed: {error}",
                manifest_path.display()
            )),
        }),
    }
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
