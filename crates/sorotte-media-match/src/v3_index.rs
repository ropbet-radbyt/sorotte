use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_V3_PROFILE_LABEL,
    anchors::{
        MediaFingerprintBlobV3, audio_index_landmarks_v3_from_record,
        decode_media_fingerprint_blob_v3, encode_media_fingerprint_blob_v3,
        media_fingerprint_blob_v3_from_record, media_fingerprint_record_apply_blob_v3,
    },
    identity::container_fingerprint_from_metadata,
    settings::{
        MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION, MediaExtractionSettings,
        media_extraction_settings_hash,
    },
    tuning::{
        V3_COMMON_BUCKET_FILE_DIVISOR, V3_COMMON_BUCKET_MIN_SKIP_DF, V3_RETRIEVAL_GAP_MS,
        V3_RETRIEVAL_OFFSET_BIN_MS, V3_RETRIEVAL_PREFILTER_LIMIT, V3_RETRIEVAL_REGION_MS,
        current_v3_tuning,
    },
    types::{
        MediaDurationCompatibility, MediaFileIdentity, MediaFingerprintRecord, MediaMatchCache,
        media_duration_compatibility_ms,
    },
};

// Version 7 invalidates indexes whose path identities were unconditionally case-folded. Those
// keys cannot safely address files on case-sensitive filesystems and must be rebuilt.
const MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION: i64 = 7;
const MEDIA_MATCH_V3_INDEX_FILE: &str = "index-v3.sqlite3";
const MEDIA_MATCH_V3_ANCHOR_STATS_DIRTY_PREFIX: &str = "anchor_stats_v3_dirty:";

#[derive(Debug, Clone, Serialize, serde::Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3RetrievalStats {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
    pub retrieval_measured_stage_millis: u128,
    pub retrieval_unaccounted_millis: u128,
    pub stats_dirty_check_millis: u128,
    pub stats_refresh_millis: u128,
    pub query_anchor_load_millis: u128,
    pub common_bucket_filter_millis: u128,
    pub sql_hit_fetch_millis: u128,
    pub rust_aggregation_millis: u128,
    pub candidate_metadata_load_millis: u128,
    pub robust_rerank_millis: u128,
    pub candidate_sort_millis: u128,
    pub candidates_returned: i64,
}

impl MediaMatchV3RetrievalStats {
    fn finish_timing(&mut self, started_at: Instant) {
        self.retrieval_elapsed_ms = started_at.elapsed().as_millis();
        self.retrieval_measured_stage_millis = self
            .stats_dirty_check_millis
            .saturating_add(self.stats_refresh_millis)
            .saturating_add(self.query_anchor_load_millis)
            .saturating_add(self.common_bucket_filter_millis)
            .saturating_add(self.sql_hit_fetch_millis)
            .saturating_add(self.rust_aggregation_millis)
            .saturating_add(self.candidate_metadata_load_millis)
            .saturating_add(self.robust_rerank_millis)
            .saturating_add(self.candidate_sort_millis);
        self.retrieval_unaccounted_millis = self
            .retrieval_elapsed_ms
            .saturating_sub(self.retrieval_measured_stage_millis);
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SqliteSizeReport {
    pub database_path: String,
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_count: u64,
    pub total_bytes: u64,
    pub live_bytes: u64,
    pub free_bytes: u64,
    pub dbstat_available: bool,
    pub db_object_bytes_available: bool,
    pub object_bytes: Vec<MediaMatchV3SqliteObjectBytes>,
    pub row_counts: Vec<MediaMatchV3SqliteRowCount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_index_bytes_per_anchor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_bytes: Option<u64>,
    pub fingerprint_blob_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_file_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_anchor_index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_fingerprint_bytes: Option<u64>,
    pub db_bytes_per_fingerprint: f64,
    pub db_bytes_per_anchor: f64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SqliteObjectBytes {
    pub name: String,
    pub object_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SqliteRowCount {
    pub table: String,
    pub row_count: u64,
    pub avg_bytes_per_row: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3RetrievedCandidate {
    pub normalized_path: String,
    pub rank: usize,
    pub total_score: i64,
    pub best_offset_bin_ms: i64,
    pub best_offset_score: i64,
    pub second_offset_score: i64,
    pub distinct_query_regions: i64,
    pub distinct_candidate_regions: i64,
    pub body_region_count: i64,
    pub edge_region_count: i64,
    pub approximate_span_ms: i64,
    pub audio_hits: i64,
    pub score_ratio_to_next: Option<f64>,
    pub query_duration_ms: Option<i64>,
    pub candidate_duration_ms: Option<i64>,
    pub duration_compatibility: MediaDurationCompatibility,
    pub short_clip_penalty_applied: bool,
    pub robust_score: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SaveStats {
    pub sqlite_save_millis: u128,
    pub blob_encode_millis: u128,
    pub index_insert_millis: u128,
}

#[derive(Debug, Clone, Default)]
struct V3CandidateRetrievalScore {
    normalized_path: String,
    query_duration_ms: Option<i64>,
    candidate_duration_ms: Option<i64>,
    total_score: i64,
    best_offset_bin: i64,
    best_offset_score: i64,
    second_offset_score: i64,
    distinct_query_regions: i64,
    distinct_candidate_regions: i64,
    body_region_count: i64,
    edge_region_count: i64,
    audio_hits: i64,
    approximate_span_ms: i64,
    robust_score: i128,
    duration_compatibility: MediaDurationCompatibility,
    short_clip_penalty_applied: bool,
    offset_bins: BTreeMap<i64, V3CandidateOffsetScore>,
}

#[derive(Debug, Clone, Default)]
struct V3CandidateOffsetScore {
    weighted_score: i64,
    query_regions: BTreeSet<i64>,
    candidate_regions: BTreeSet<i64>,
    body_regions: BTreeSet<i64>,
    edge_regions: BTreeSet<i64>,
    query_times: BTreeSet<i64>,
    candidate_times: BTreeSet<i64>,
    audio_hits: i64,
}

pub fn media_match_v3_index_path(root: &Path) -> PathBuf {
    root.join(MEDIA_MATCH_V3_INDEX_FILE)
}

pub fn open_media_match_v3_index(root: &Path) -> Result<Connection, String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "failed creating media-match V3 index directory '{}': {error}",
            root.display()
        )
    })?;
    let path = media_match_v3_index_path(root);
    let connection = Connection::open(&path).map_err(|error| {
        format!(
            "failed opening media-match V3 index '{}': {error}",
            path.display()
        )
    })?;
    initialize_media_match_v3_index(&connection)?;
    Ok(connection)
}

pub fn initialize_media_match_v3_index(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA temp_store = MEMORY;
            ",
        )
        .map_err(|error| format!("failed configuring media-match V3 SQLite pragmas: {error}"))?;
    let version = sqlite_schema_version(connection)?;
    if version != MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION {
        reset_media_match_v3_schema(connection)?;
    }
    Ok(())
}

fn sqlite_schema_version(connection: &Connection) -> Result<i64, String> {
    if !sqlite_table_exists(connection, "metadata")? {
        return Ok(0);
    }
    connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("failed reading media-match V3 schema version: {error}"))?
        .and_then(|value| value.parse::<i64>().ok())
        .map_or(Ok(0), Ok)
}

fn reset_media_match_v3_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            DROP TABLE IF EXISTS audio_anchor_occurrences_v3;
            DROP TABLE IF EXISTS audio_anchor_buckets_v3;
            DROP TABLE IF EXISTS fingerprints_v3;
            DROP TABLE IF EXISTS settings_v3;
            DROP TABLE IF EXISTS media_files_v3;
            DROP TABLE IF EXISTS metadata;

            CREATE TABLE metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE media_files_v3 (
                file_id INTEGER PRIMARY KEY,
                normalized_path TEXT NOT NULL UNIQUE,
                modified_unix_millis INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                duration_ms INTEGER,
                container_fingerprint TEXT NOT NULL,
                updated_unix_millis INTEGER NOT NULL
            );

            CREATE TABLE settings_v3 (
                settings_id INTEGER PRIMARY KEY,
                settings_hash BLOB NOT NULL UNIQUE,
                algorithm_version INTEGER NOT NULL,
                fingerprint_cache_version INTEGER NOT NULL,
                profile TEXT NOT NULL,
                tuning_json TEXT,
                created_unix_millis INTEGER NOT NULL
            );

            CREATE TABLE fingerprints_v3 (
                file_id INTEGER NOT NULL,
                settings_id INTEGER NOT NULL,
                duration_ms INTEGER,
                audio_blob BLOB,
                audio_verify_count INTEGER NOT NULL DEFAULT 0,
                audio_index_count INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                updated_unix_millis INTEGER NOT NULL,
                PRIMARY KEY (file_id, settings_id),
                FOREIGN KEY (file_id) REFERENCES media_files_v3(file_id) ON DELETE CASCADE,
                FOREIGN KEY (settings_id) REFERENCES settings_v3(settings_id) ON DELETE CASCADE
            ) WITHOUT ROWID;

            CREATE TABLE audio_anchor_buckets_v3 (
                bucket_id INTEGER PRIMARY KEY,
                settings_id INTEGER NOT NULL,
                bucket INTEGER NOT NULL,
                document_frequency INTEGER NOT NULL DEFAULT 0,
                updated_unix_millis INTEGER NOT NULL,
                UNIQUE(settings_id, bucket),
                FOREIGN KEY (settings_id) REFERENCES settings_v3(settings_id) ON DELETE CASCADE
            );

            CREATE TABLE audio_anchor_occurrences_v3 (
                bucket_id INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                t_ms INTEGER NOT NULL,
                weight INTEGER NOT NULL,
                PRIMARY KEY (bucket_id, file_id, t_ms),
                FOREIGN KEY (bucket_id) REFERENCES audio_anchor_buckets_v3(bucket_id) ON DELETE CASCADE,
                FOREIGN KEY (file_id) REFERENCES media_files_v3(file_id) ON DELETE CASCADE
            ) WITHOUT ROWID;

            CREATE INDEX idx_audio_anchor_occurrences_v3_file
                ON audio_anchor_occurrences_v3(file_id, bucket_id, t_ms, weight);

            INSERT INTO metadata (key, value)
            VALUES ('schema_version', '7');
            ",
        )
        .map_err(|error| format!("failed resetting media-match V3 schema: {error}"))?;
    Ok(())
}

pub fn save_media_match_v3_record(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    _error: Option<&str>,
) -> Result<(), String> {
    save_media_match_v3_record_with_stats(connection, record, current_unix_millis() as i64)
        .map(|_| ())
}

pub fn save_media_match_v3_record_with_stats(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    now: i64,
) -> Result<MediaMatchV3SaveStats, String> {
    save_media_match_v3_record_with_stats_and_hook(connection, record, now, |_| Ok(()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V3SaveProgress {
    FingerprintWritten,
    AnchorsDeleted,
    AnchorInserted(usize),
}

fn save_media_match_v3_record_with_stats_and_hook<F>(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    now: i64,
    mut after_progress: F,
) -> Result<MediaMatchV3SaveStats, String>
where
    F: FnMut(V3SaveProgress) -> Result<(), String>,
{
    let started_at = Instant::now();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match V3 save transaction: {error}"))?;
    let settings_hash = media_extraction_settings_hash(&record.extraction_settings);
    let settings_id = ensure_media_match_v3_settings_id(&transaction, settings_hash, now)?;
    let duration_ms = duration_ms_from_seconds(record.duration_seconds);
    let file_id = upsert_media_file_v3(&transaction, record, duration_ms, now)?;
    let blob_started_at = Instant::now();
    let blob = media_fingerprint_blob_v3_from_record(record);
    let audio_blob = (!blob.audio_landmarks.is_empty()).then(|| {
        encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
            duration_ms: blob.duration_ms,
            audio_landmarks: blob.audio_landmarks.clone(),
        })
    });
    let blob_encode_millis = blob_started_at.elapsed().as_millis();
    let audio_index = audio_index_landmarks_v3_from_record(record);
    let error = record.audio_error.clone();
    transaction
        .execute(
            "INSERT OR REPLACE INTO fingerprints_v3 (
                file_id, settings_id, duration_ms, audio_blob,
                audio_verify_count, audio_index_count, error, updated_unix_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file_id,
                settings_id,
                duration_ms,
                audio_blob,
                blob.audio_landmarks.len() as i64,
                audio_index.len() as i64,
                error,
                now,
            ],
        )
        .map_err(|error| format!("failed saving media-match V3 fingerprint: {error}"))?;
    after_progress(V3SaveProgress::FingerprintWritten)?;

    let index_started_at = Instant::now();
    transaction
        .execute(
            "DELETE FROM audio_anchor_occurrences_v3
             WHERE file_id = ?1
               AND bucket_id IN (
                   SELECT bucket_id FROM audio_anchor_buckets_v3 WHERE settings_id = ?2
               )",
            params![file_id, settings_id],
        )
        .map_err(|error| format!("failed deleting stale V3 anchors: {error}"))?;
    after_progress(V3SaveProgress::AnchorsDeleted)?;
    let mut bucket_ids = BTreeMap::<u32, i64>::new();
    for (index, landmark) in audio_index.iter().enumerate() {
        let bucket_id = audio_anchor_bucket_id_v3(
            &transaction,
            settings_id,
            landmark.hash,
            now,
            &mut bucket_ids,
        )?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO audio_anchor_occurrences_v3 (
                    bucket_id, file_id, t_ms, weight
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    bucket_id,
                    file_id,
                    i64::from(landmark.t_ms),
                    i64::from(landmark.weight.max(1)),
                ],
            )
            .map_err(|error| format!("failed inserting V3 audio anchor: {error}"))?;
        after_progress(V3SaveProgress::AnchorInserted(index + 1))?;
    }
    mark_anchor_stats_v3_dirty(&transaction, &settings_hash)?;
    let stats = MediaMatchV3SaveStats {
        sqlite_save_millis: started_at.elapsed().as_millis(),
        blob_encode_millis,
        index_insert_millis: index_started_at.elapsed().as_millis(),
    };
    transaction
        .commit()
        .map_err(|error| format!("failed committing media-match V3 save transaction: {error}"))?;
    Ok(stats)
}

fn upsert_media_file_v3(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    duration_ms: Option<i64>,
    now: i64,
) -> Result<i64, String> {
    connection
        .execute(
            "INSERT INTO media_files_v3 (
                normalized_path, modified_unix_millis, size_bytes,
                duration_ms, container_fingerprint, updated_unix_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(normalized_path) DO UPDATE SET
                modified_unix_millis = excluded.modified_unix_millis,
                size_bytes = excluded.size_bytes,
                duration_ms = excluded.duration_ms,
                container_fingerprint = excluded.container_fingerprint,
                updated_unix_millis = excluded.updated_unix_millis",
            params![
                record.identity.normalized_path,
                record.identity.modified_unix_millis as i64,
                record.identity.size_bytes as i64,
                duration_ms,
                record.container_fingerprint,
                now,
            ],
        )
        .map_err(|error| format!("failed saving media-match V3 media file: {error}"))?;
    media_file_id_for_path(connection, &record.identity.normalized_path)
}

fn ensure_media_match_v3_settings_id(
    connection: &Connection,
    settings_hash: [u8; 32],
    now: i64,
) -> Result<i64, String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO settings_v3 (
                settings_hash, algorithm_version, fingerprint_cache_version,
                profile, tuning_json, created_unix_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                settings_hash.as_slice(),
                MEDIA_MATCH_ALGORITHM_VERSION as i64,
                MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION as i64,
                MEDIA_MATCH_V3_PROFILE_LABEL,
                serde_json::to_string(&current_v3_tuning()).unwrap_or_default(),
                now,
            ],
        )
        .map_err(|error| format!("failed saving media-match V3 settings: {error}"))?;
    media_match_v3_settings_id_for_hash(connection, &settings_hash)
}

fn media_match_v3_settings_id_for_hash(
    connection: &Connection,
    settings_hash: &[u8; 32],
) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT settings_id FROM settings_v3 WHERE settings_hash = ?1",
            params![settings_hash.as_slice()],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed loading media-match V3 settings id: {error}"))
}

fn media_file_id_for_path(connection: &Connection, normalized_path: &str) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT file_id FROM media_files_v3 WHERE normalized_path = ?1",
            [normalized_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed loading media-match V3 file id: {error}"))
}

fn audio_anchor_bucket_id_v3(
    connection: &Connection,
    settings_id: i64,
    bucket: u32,
    now: i64,
    bucket_ids: &mut BTreeMap<u32, i64>,
) -> Result<i64, String> {
    if let Some(bucket_id) = bucket_ids.get(&bucket).copied() {
        return Ok(bucket_id);
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO audio_anchor_buckets_v3 (
                settings_id, bucket, updated_unix_millis
             ) VALUES (?1, ?2, ?3)",
            params![settings_id, i64::from(bucket), now],
        )
        .map_err(|error| format!("failed saving V3 audio anchor bucket: {error}"))?;
    let bucket_id = connection
        .query_row(
            "SELECT bucket_id FROM audio_anchor_buckets_v3
             WHERE settings_id = ?1 AND bucket = ?2",
            params![settings_id, i64::from(bucket)],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed loading V3 audio anchor bucket: {error}"))?;
    bucket_ids.insert(bucket, bucket_id);
    Ok(bucket_id)
}

pub fn load_media_match_v3_cache_for_settings(
    connection: &Connection,
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaMatchCache, String> {
    let settings_hash = media_extraction_settings_hash(extraction_settings);
    let settings_id = match media_match_v3_settings_id_for_hash(connection, &settings_hash) {
        Ok(settings_id) => settings_id,
        Err(_) => return Ok(MediaMatchCache::default()),
    };
    let mut statement = connection
        .prepare(
            "SELECT media_files_v3.normalized_path, media_files_v3.modified_unix_millis,
                    media_files_v3.size_bytes, media_files_v3.container_fingerprint,
                    fingerprints_v3.duration_ms, fingerprints_v3.audio_blob, fingerprints_v3.error
             FROM fingerprints_v3
             JOIN media_files_v3 ON media_files_v3.file_id = fingerprints_v3.file_id
             WHERE fingerprints_v3.settings_id = ?1",
        )
        .map_err(|error| format!("failed preparing V3 cache load: {error}"))?;
    let records = statement
        .query_map([settings_id], |row| {
            media_match_v3_record_from_cached_blobs(row, extraction_settings.clone())
        })
        .map_err(|error| format!("failed querying V3 cache: {error}"))?;
    let mut cache = MediaMatchCache::default();
    for record in records {
        cache.insert(record.map_err(|error| format!("failed reading cached V3 record: {error}"))?);
    }
    Ok(cache)
}

pub fn load_media_match_v3_record_for_path(
    connection: &Connection,
    normalized_path: &str,
    extraction_settings: &MediaExtractionSettings,
    modified_unix_millis: u64,
    size_bytes: u64,
) -> Result<Option<MediaFingerprintRecord>, String> {
    let settings_hash = media_extraction_settings_hash(extraction_settings);
    let settings_id = match media_match_v3_settings_id_for_hash(connection, &settings_hash) {
        Ok(settings_id) => settings_id,
        Err(_) => return Ok(None),
    };
    let mut statement = connection
        .prepare(
            "SELECT media_files_v3.normalized_path, media_files_v3.modified_unix_millis,
                    media_files_v3.size_bytes, media_files_v3.container_fingerprint,
                    fingerprints_v3.duration_ms, fingerprints_v3.audio_blob, fingerprints_v3.error
             FROM fingerprints_v3
              JOIN media_files_v3 ON media_files_v3.file_id = fingerprints_v3.file_id
              WHERE fingerprints_v3.settings_id = ?1
                AND media_files_v3.normalized_path = ?2
                AND media_files_v3.size_bytes = ?4
              ORDER BY CASE WHEN media_files_v3.modified_unix_millis = ?3 THEN 0 ELSE 1 END
              LIMIT 1",
        )
        .map_err(|error| format!("failed preparing V3 record load: {error}"))?;
    statement
        .query_row(
            params![
                settings_id,
                normalized_path,
                modified_unix_millis as i64,
                size_bytes as i64,
            ],
            |row| media_match_v3_record_from_cached_blobs(row, extraction_settings.clone()),
        )
        .optional()
        .map_err(|error| format!("failed loading cached V3 record: {error}"))
}

pub fn media_match_v3_anchor_candidate_paths_with_stats(
    connection: &Connection,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<(Vec<String>, MediaMatchV3RetrievalStats), String> {
    let current = media_match_v3_record_stub_for_path(
        connection,
        normalized_current_path,
        extraction_settings,
    )?;
    let (candidates, stats) = media_match_v3_anchor_candidate_details_with_stats(
        connection,
        &current,
        current_unix_millis() as i64,
    )?;
    Ok((
        candidates
            .into_iter()
            .map(|candidate| candidate.normalized_path)
            .collect(),
        stats,
    ))
}

fn media_match_v3_record_stub_for_path(
    connection: &Connection,
    normalized_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaFingerprintRecord, String> {
    connection
        .query_row(
            "SELECT modified_unix_millis, size_bytes, duration_ms, container_fingerprint
             FROM media_files_v3
             WHERE normalized_path = ?1",
            [normalized_path],
            |row| {
                let modified_unix_millis: i64 = row.get(0)?;
                let size_bytes: i64 = row.get(1)?;
                let duration_ms: Option<i64> = row.get(2)?;
                let container_fingerprint: String = row.get(3)?;
                Ok(MediaFingerprintRecord {
                    identity: MediaFileIdentity {
                        normalized_path: normalized_path.to_owned(),
                        modified_unix_millis: modified_unix_millis.max(0) as u64,
                        size_bytes: size_bytes.max(0) as u64,
                    },
                    algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
                    extraction_settings: extraction_settings.clone(),
                    duration_seconds: duration_ms.map(|duration_ms| duration_ms as f64 / 1000.0),
                    container_fingerprint,
                    audio_anchors: Vec::new(),
                    audio_error: None,
                })
            },
        )
        .map_err(|error| format!("failed loading V3 query record for retrieval: {error}"))
}

pub fn media_match_v3_anchor_candidate_details_with_stats(
    connection: &Connection,
    current: &MediaFingerprintRecord,
    now: i64,
) -> Result<
    (
        Vec<MediaMatchV3RetrievedCandidate>,
        MediaMatchV3RetrievalStats,
    ),
    String,
> {
    let started_at = Instant::now();
    let settings_hash = media_extraction_settings_hash(&current.extraction_settings);
    let settings_id = media_match_v3_settings_id_for_hash(connection, &settings_hash)?;
    let mut stats = MediaMatchV3RetrievalStats::default();
    let dirty_started_at = Instant::now();
    let dirty = anchor_stats_v3_dirty(connection, &settings_hash)?;
    stats.stats_dirty_check_millis = dirty_started_at.elapsed().as_millis();
    if dirty {
        let refresh_started_at = Instant::now();
        refresh_anchor_stats_v3(connection, &settings_hash, now)?;
        stats.stats_refresh_millis = refresh_started_at.elapsed().as_millis();
    }

    let current_file_id = media_file_id_for_path(connection, &current.identity.normalized_path)?;
    let query_started_at = Instant::now();
    let query_anchors = load_query_audio_anchors(connection, settings_id, current_file_id)?;
    stats.query_anchor_load_millis = query_started_at.elapsed().as_millis();
    stats.query_buckets_total = query_anchors.len() as i64;

    let common_started_at = Instant::now();
    let file_count = media_file_count_for_settings(connection, settings_id)?.max(1);
    let common_threshold =
        (file_count / V3_COMMON_BUCKET_FILE_DIVISOR).max(V3_COMMON_BUCKET_MIN_SKIP_DF);
    let query_anchors = query_anchors
        .into_iter()
        .filter(|anchor| {
            if anchor.document_frequency >= common_threshold {
                stats.query_buckets_skipped_common += 1;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    stats.common_bucket_filter_millis = common_started_at.elapsed().as_millis();

    let fetch_started_at = Instant::now();
    let hit_rows = fetch_audio_hit_rows(connection, settings_id, current_file_id, &query_anchors)?;
    stats.sql_hit_fetch_millis = fetch_started_at.elapsed().as_millis();
    stats.raw_hit_rows_processed = hit_rows.len() as i64;

    let aggregate_started_at = Instant::now();
    let mut by_file = BTreeMap::<i64, V3CandidateRetrievalScore>::new();
    let query_duration_ms = duration_ms_from_seconds(current.duration_seconds);
    for row in hit_rows {
        let score = by_file
            .entry(row.file_id)
            .or_insert_with(|| V3CandidateRetrievalScore {
                normalized_path: row.normalized_path.clone(),
                query_duration_ms,
                candidate_duration_ms: row.duration_ms,
                duration_compatibility: media_duration_compatibility_ms(
                    query_duration_ms,
                    row.duration_ms,
                ),
                ..V3CandidateRetrievalScore::default()
            });
        let weight = row.query_weight.min(row.weight).max(1);
        score.total_score += weight;
        score.audio_hits += 1;
        let offset_bin = media_match_v3_rounded_offset_bin(row.t_ms - row.query_t_ms);
        let offset = score.offset_bins.entry(offset_bin).or_default();
        offset.weighted_score += weight;
        offset.audio_hits += 1;
        offset
            .query_regions
            .insert(row.query_t_ms / V3_RETRIEVAL_REGION_MS);
        offset
            .candidate_regions
            .insert(row.t_ms / V3_RETRIEVAL_REGION_MS);
        if media_match_v3_time_is_edge(row.query_t_ms, query_duration_ms) {
            offset
                .edge_regions
                .insert(row.query_t_ms / V3_RETRIEVAL_REGION_MS);
        } else {
            offset
                .body_regions
                .insert(row.query_t_ms / V3_RETRIEVAL_REGION_MS);
        }
        offset.query_times.insert(row.query_t_ms);
        offset.candidate_times.insert(row.t_ms);
    }
    stats.rust_aggregation_millis = aggregate_started_at.elapsed().as_millis();

    let metadata_started_at = Instant::now();
    let mut candidates = by_file
        .into_values()
        .map(finalize_v3_candidate_retrieval_score)
        .collect::<Vec<_>>();
    stats.candidate_metadata_load_millis = metadata_started_at.elapsed().as_millis();
    stats.candidates_scored = candidates.len() as i64;

    let rerank_started_at = Instant::now();
    for candidate in &mut candidates {
        candidate.robust_score = candidate.robust_score.max(0.0);
    }
    stats.robust_rerank_millis = rerank_started_at.elapsed().as_millis();

    let sort_started_at = Instant::now();
    candidates.sort_by(|left, right| {
        right
            .robust_score
            .total_cmp(&left.robust_score)
            .then_with(|| right.best_offset_score.cmp(&left.best_offset_score))
            .then_with(|| right.total_score.cmp(&left.total_score))
            .then_with(|| left.normalized_path.cmp(&right.normalized_path))
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }
    let next_scores = candidates
        .iter()
        .skip(1)
        .map(|candidate| candidate.robust_score.max(1.0))
        .chain(std::iter::once(1.0))
        .collect::<Vec<_>>();
    for (candidate, next_score) in candidates.iter_mut().zip(next_scores) {
        candidate.score_ratio_to_next = Some(candidate.robust_score.max(1.0) / next_score);
    }
    candidates.truncate(V3_RETRIEVAL_PREFILTER_LIMIT);
    stats.candidate_sort_millis = sort_started_at.elapsed().as_millis();
    stats.candidates_returned = candidates.len() as i64;
    stats.finish_timing(started_at);
    Ok((candidates, stats))
}

#[derive(Debug, Clone)]
struct QueryAudioAnchor {
    bucket_id: i64,
    query_t_ms: i64,
    query_weight: i64,
    document_frequency: i64,
}

#[derive(Debug, Clone)]
struct AudioHitRow {
    file_id: i64,
    normalized_path: String,
    duration_ms: Option<i64>,
    query_t_ms: i64,
    query_weight: i64,
    t_ms: i64,
    weight: i64,
}

fn load_query_audio_anchors(
    connection: &Connection,
    settings_id: i64,
    current_file_id: i64,
) -> Result<Vec<QueryAudioAnchor>, String> {
    let mut statement = connection
        .prepare(
            "SELECT occurrence.bucket_id, occurrence.t_ms, occurrence.weight,
                    bucket.document_frequency
             FROM audio_anchor_occurrences_v3 occurrence
             JOIN audio_anchor_buckets_v3 bucket ON bucket.bucket_id = occurrence.bucket_id
             WHERE occurrence.file_id = ?1
               AND bucket.settings_id = ?2",
        )
        .map_err(|error| format!("failed preparing V3 query anchor load: {error}"))?;
    let rows = statement
        .query_map(params![current_file_id, settings_id], |row| {
            Ok(QueryAudioAnchor {
                bucket_id: row.get(0)?,
                query_t_ms: row.get(1)?,
                query_weight: row.get(2)?,
                document_frequency: row.get(3)?,
            })
        })
        .map_err(|error| format!("failed loading V3 query anchors: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed reading V3 query anchors: {error}"))
}

fn fetch_audio_hit_rows(
    connection: &Connection,
    settings_id: i64,
    current_file_id: i64,
    query_anchors: &[QueryAudioAnchor],
) -> Result<Vec<AudioHitRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT occurrence.file_id, media.normalized_path, media.duration_ms,
                    occurrence.t_ms, occurrence.weight
             FROM audio_anchor_occurrences_v3 occurrence
             JOIN media_files_v3 media ON media.file_id = occurrence.file_id
             JOIN audio_anchor_buckets_v3 bucket ON bucket.bucket_id = occurrence.bucket_id
             WHERE bucket.settings_id = ?1
               AND occurrence.bucket_id = ?2
               AND occurrence.file_id != ?3",
        )
        .map_err(|error| format!("failed preparing V3 hit lookup: {error}"))?;
    let mut rows = Vec::new();
    for query in query_anchors {
        let hit_rows = statement
            .query_map(
                params![settings_id, query.bucket_id, current_file_id],
                |row| {
                    Ok(AudioHitRow {
                        file_id: row.get(0)?,
                        normalized_path: row.get(1)?,
                        duration_ms: row.get(2)?,
                        query_t_ms: query.query_t_ms,
                        query_weight: query.query_weight,
                        t_ms: row.get(3)?,
                        weight: row.get(4)?,
                    })
                },
            )
            .map_err(|error| format!("failed querying V3 hit rows: {error}"))?;
        for row in hit_rows {
            rows.push(row.map_err(|error| format!("failed reading V3 hit row: {error}"))?);
        }
    }
    Ok(rows)
}

fn finalize_v3_candidate_retrieval_score(
    mut score: V3CandidateRetrievalScore,
) -> MediaMatchV3RetrievedCandidate {
    let mut offsets = score.offset_bins.iter().collect::<Vec<_>>();
    offsets.sort_by(|(left_bin, left), (right_bin, right)| {
        right
            .weighted_score
            .cmp(&left.weighted_score)
            .then_with(|| right.audio_hits.cmp(&left.audio_hits))
            .then_with(|| left_bin.cmp(right_bin))
    });
    if let Some((best_bin, best)) = offsets.first() {
        score.best_offset_bin = **best_bin;
        score.best_offset_score = best.weighted_score;
        score.distinct_query_regions = best.query_regions.len() as i64;
        score.distinct_candidate_regions = best.candidate_regions.len() as i64;
        score.body_region_count = best.body_regions.len() as i64;
        score.edge_region_count = best.edge_regions.len() as i64;
        score.approximate_span_ms = media_match_v3_longest_contiguous_span_ms(&best.query_times)
            .max(media_match_v3_longest_contiguous_span_ms(
                &best.candidate_times,
            ));
    }
    score.second_offset_score = offsets
        .get(1)
        .map(|(_, offset)| offset.weighted_score)
        .unwrap_or(0);
    score.short_clip_penalty_applied = score.duration_compatibility
        == MediaDurationCompatibility::ContainedOrPartial
        && shorter_duration_ms(score.query_duration_ms, score.candidate_duration_ms)
            .is_some_and(|shorter| shorter <= 5 * 60 * 1000);
    score.robust_score = media_match_v3_robust_retrieval_score(&score);
    MediaMatchV3RetrievedCandidate {
        normalized_path: score.normalized_path,
        rank: 0,
        total_score: score.total_score,
        best_offset_bin_ms: score.best_offset_bin,
        best_offset_score: score.best_offset_score,
        second_offset_score: score.second_offset_score,
        distinct_query_regions: score.distinct_query_regions,
        distinct_candidate_regions: score.distinct_candidate_regions,
        body_region_count: score.body_region_count,
        edge_region_count: score.edge_region_count,
        approximate_span_ms: score.approximate_span_ms,
        audio_hits: score.audio_hits,
        score_ratio_to_next: None,
        query_duration_ms: score.query_duration_ms,
        candidate_duration_ms: score.candidate_duration_ms,
        duration_compatibility: score.duration_compatibility,
        short_clip_penalty_applied: score.short_clip_penalty_applied,
        robust_score: score.robust_score as f64,
    }
}

fn media_match_v3_robust_retrieval_score(score: &V3CandidateRetrievalScore) -> i128 {
    let mut value = i128::from(score.best_offset_score.max(score.total_score / 3).max(1));
    value *= span_factor(score);
    value *= region_factor(score);
    value *= offset_dominance_factor(score);
    value *= duration_factor(score);
    value / 10_000
}

fn span_factor(score: &V3CandidateRetrievalScore) -> i128 {
    let seconds = (score.approximate_span_ms / 1000).max(1) as f64;
    (100.0 + seconds.log2().max(0.0) * 35.0).round() as i128
}

fn region_factor(score: &V3CandidateRetrievalScore) -> i128 {
    let query = score.distinct_query_regions.max(1) as i128;
    let candidate = score.distinct_candidate_regions.max(1) as i128;
    75 + query.min(candidate).min(8) * 12 + score.body_region_count.clamp(0, 8) as i128 * 10
}

fn offset_dominance_factor(score: &V3CandidateRetrievalScore) -> i128 {
    if score.second_offset_score <= 0 {
        return 160;
    }
    let ratio = score.best_offset_score as f64 / score.second_offset_score.max(1) as f64;
    (90.0 + ratio.min(4.0) * 25.0).round() as i128
}

fn duration_factor(score: &V3CandidateRetrievalScore) -> i128 {
    match score.duration_compatibility {
        MediaDurationCompatibility::SameCutCompatible | MediaDurationCompatibility::Unknown => 100,
        MediaDurationCompatibility::NearCompatible => 95,
        MediaDurationCompatibility::ContainedOrPartial => 75,
        MediaDurationCompatibility::IncompatibleSameCut => 45,
    }
}

fn shorter_duration_ms(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    let left = left?;
    let right = right?;
    if left <= 0 || right <= 0 {
        return None;
    }
    Some(left.min(right))
}

fn media_match_v3_rounded_offset_bin(offset_ms: i64) -> i64 {
    let bin = V3_RETRIEVAL_OFFSET_BIN_MS.max(1);
    ((offset_ms + bin / 2).div_euclid(bin)) * bin
}

fn media_match_v3_time_is_edge(time_ms: i64, duration_ms: Option<i64>) -> bool {
    let edge_ms = 120_000;
    time_ms < edge_ms || duration_ms.is_some_and(|duration| time_ms > duration - edge_ms)
}

fn media_match_v3_longest_contiguous_span_ms(times: &BTreeSet<i64>) -> i64 {
    let Some(mut start) = times.first().copied() else {
        return 0;
    };
    let mut previous = start;
    let mut best = 0;
    for time in times.iter().copied().skip(1) {
        if time - previous > V3_RETRIEVAL_GAP_MS {
            best = best.max(previous - start);
            start = time;
        }
        previous = time;
    }
    best.max(previous - start)
}

fn media_file_count_for_settings(connection: &Connection, settings_id: i64) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COUNT(DISTINCT file_id) FROM fingerprints_v3 WHERE settings_id = ?1",
            [settings_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed counting media-match V3 files: {error}"))
}

pub fn refresh_dirty_anchor_stats_v3_if_needed(
    connection: &Connection,
    settings_hash: &[u8; 32],
    now: i64,
) -> Result<(), String> {
    if anchor_stats_v3_dirty(connection, settings_hash)? {
        refresh_anchor_stats_v3(connection, settings_hash, now)?;
    }
    Ok(())
}

pub fn refresh_anchor_stats_v3(
    connection: &Connection,
    settings_hash: &[u8; 32],
    _now: i64,
) -> Result<(), String> {
    let settings_id = media_match_v3_settings_id_for_hash(connection, settings_hash)?;
    connection
        .execute(
            "UPDATE audio_anchor_buckets_v3
             SET document_frequency = (
                SELECT COUNT(DISTINCT file_id)
                FROM audio_anchor_occurrences_v3 occurrence
                WHERE occurrence.bucket_id = audio_anchor_buckets_v3.bucket_id
             )
             WHERE settings_id = ?1",
            [settings_id],
        )
        .map_err(|error| format!("failed refreshing V3 anchor stats: {error}"))?;
    clear_anchor_stats_v3_dirty(connection, settings_hash)
}

pub fn refresh_all_anchor_stats_v3(connection: &Connection, now: i64) -> Result<(), String> {
    let mut statement = connection
        .prepare("SELECT settings_hash FROM settings_v3")
        .map_err(|error| format!("failed preparing V3 stats refresh: {error}"))?;
    let hashes = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|error| format!("failed querying V3 settings hashes: {error}"))?;
    for hash in hashes {
        let hash = hash.map_err(|error| format!("failed reading V3 settings hash: {error}"))?;
        if hash.len() == 32 {
            let mut settings_hash = [0u8; 32];
            settings_hash.copy_from_slice(&hash);
            refresh_anchor_stats_v3(connection, &settings_hash, now)?;
        }
    }
    Ok(())
}

pub fn mark_anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8; 32],
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, '1')",
            [anchor_stats_v3_dirty_key(settings_hash)],
        )
        .map_err(|error| format!("failed marking V3 anchor stats dirty: {error}"))?;
    Ok(())
}

pub fn clear_anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8; 32],
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM metadata WHERE key = ?1",
            [anchor_stats_v3_dirty_key(settings_hash)],
        )
        .map_err(|error| format!("failed clearing V3 anchor stats dirty flag: {error}"))?;
    Ok(())
}

pub fn anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8; 32],
) -> Result<bool, String> {
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            [anchor_stats_v3_dirty_key(settings_hash)],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("failed reading V3 anchor stats dirty flag: {error}"))?;
    Ok(value.is_some())
}

pub fn delete_media_match_v3_fingerprints_and_anchors(
    connection: &Connection,
    normalized_path: &str,
) -> Result<(), String> {
    let Ok(file_id) = media_file_id_for_path(connection, normalized_path) else {
        return Ok(());
    };
    let mut statement = connection
        .prepare(
            "SELECT settings_v3.settings_hash
             FROM fingerprints_v3
             JOIN settings_v3 ON settings_v3.settings_id = fingerprints_v3.settings_id
             WHERE fingerprints_v3.file_id = ?1",
        )
        .map_err(|error| format!("failed preparing V3 settings dirty query: {error}"))?;
    let settings_hashes = statement
        .query_map([file_id], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|error| format!("failed querying V3 settings dirty rows: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed reading V3 settings dirty row: {error}"))?;
    connection
        .execute(
            "DELETE FROM audio_anchor_occurrences_v3 WHERE file_id = ?1",
            [file_id],
        )
        .map_err(|error| format!("failed deleting V3 anchors: {error}"))?;
    connection
        .execute("DELETE FROM fingerprints_v3 WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting V3 fingerprints: {error}"))?;
    for settings_hash in settings_hashes {
        if let Ok(settings_hash) = <[u8; 32]>::try_from(settings_hash.as_slice()) {
            mark_anchor_stats_v3_dirty(connection, &settings_hash)?;
        }
    }
    Ok(())
}

pub fn delete_media_match_v3_file_and_fingerprints(
    connection: &Connection,
    normalized_path: &str,
) -> Result<(), String> {
    delete_media_match_v3_fingerprints_and_anchors(connection, normalized_path)?;
    connection
        .execute(
            "DELETE FROM media_files_v3 WHERE normalized_path = ?1",
            [normalized_path],
        )
        .map_err(|error| format!("failed deleting V3 media file: {error}"))?;
    Ok(())
}

fn media_match_v3_record_from_cached_blobs(
    row: &rusqlite::Row<'_>,
    extraction_settings: MediaExtractionSettings,
) -> rusqlite::Result<MediaFingerprintRecord> {
    let normalized_path: String = row.get(0)?;
    let modified_unix_millis: i64 = row.get(1)?;
    let size_bytes: i64 = row.get(2)?;
    let container_fingerprint: String = row.get(3)?;
    let duration_ms: Option<i64> = row.get(4)?;
    let audio_blob: Option<Vec<u8>> = row.get(5)?;
    let error: Option<String> = row.get(6)?;
    let mut record = MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: normalized_path.clone(),
            modified_unix_millis: modified_unix_millis.max(0) as u64,
            size_bytes: size_bytes.max(0) as u64,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings,
        duration_seconds: duration_ms.map(|duration_ms| duration_ms as f64 / 1000.0),
        container_fingerprint,
        audio_anchors: Vec::new(),
        audio_error: error,
    };
    if let Some(blob_bytes) = audio_blob
        && let Ok(blob) = decode_media_fingerprint_blob_v3(&blob_bytes)
    {
        media_fingerprint_record_apply_blob_v3(&mut record, blob);
    }
    if record.container_fingerprint.is_empty() {
        record.container_fingerprint = container_fingerprint_from_metadata(
            &normalized_path,
            record.identity.modified_unix_millis,
            record.identity.size_bytes,
            record.duration_seconds,
        );
    }
    Ok(record)
}

pub fn media_match_v3_sqlite_size_report(
    root: &Path,
    connection: &Connection,
) -> Result<MediaMatchV3SqliteSizeReport, String> {
    let index_path = media_match_v3_index_path(root);
    let page_size = sqlite_pragma_u64(connection, "page_size")?;
    let page_count = sqlite_pragma_u64(connection, "page_count")?;
    let freelist_count = sqlite_pragma_u64(connection, "freelist_count")?;
    let total_bytes = page_size.saturating_mul(page_count);
    let free_bytes = page_size.saturating_mul(freelist_count);
    let live_bytes = total_bytes.saturating_sub(free_bytes);
    let object_bytes = sqlite_object_bytes(connection).unwrap_or_default();
    let db_object_bytes_available = !object_bytes.is_empty();
    let row_counts =
        sqlite_row_counts(connection, &object_bytes, db_object_bytes_available).unwrap_or_default();
    let anchor_rows = row_count(connection, "audio_anchor_occurrences_v3");
    let fingerprint_rows = row_count(connection, "fingerprints_v3");
    let anchor_index_bytes = db_object_bytes_available.then(|| {
        object_bytes
            .iter()
            .filter(|object| object.name.contains("audio_anchor"))
            .map(|object| object.bytes)
            .sum()
    });
    let fingerprint_blob_bytes: u64 = connection
        .query_row(
            "SELECT COALESCE(SUM(COALESCE(length(audio_blob), 0)), 0) FROM fingerprints_v3",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as u64;
    let fingerprint_bytes = db_object_bytes_available.then(|| {
        object_bytes
            .iter()
            .filter(|object| object.name.contains("fingerprints_v3"))
            .map(|object| object.bytes)
            .sum()
    });
    let media_file_bytes = db_object_bytes_available.then(|| {
        object_bytes
            .iter()
            .filter(|object| object.name.contains("media_files_v3"))
            .map(|object| object.bytes)
            .sum()
    });
    let metadata_bytes = db_object_bytes_available.then(|| {
        object_bytes
            .iter()
            .filter(|object| object.name == "metadata" || object.name == "settings_v3")
            .map(|object| object.bytes)
            .sum()
    });
    let db_index_bytes = db_object_bytes_available.then(|| {
        object_bytes
            .iter()
            .filter(|object| object.object_type == "index")
            .map(|object| object.bytes)
            .sum()
    });
    let estimated_anchor_index_bytes =
        (!db_object_bytes_available).then(|| total_bytes.saturating_sub(fingerprint_blob_bytes));
    let estimated_fingerprint_bytes =
        (!db_object_bytes_available).then_some(fingerprint_blob_bytes);
    Ok(MediaMatchV3SqliteSizeReport {
        database_path: index_path.display().to_string(),
        page_size,
        page_count,
        freelist_count,
        total_bytes,
        live_bytes,
        free_bytes,
        dbstat_available: db_object_bytes_available,
        db_object_bytes_available,
        object_bytes,
        row_counts,
        anchor_index_bytes,
        anchor_index_bytes_per_anchor: anchor_index_bytes
            .map(|bytes| ratio_u64(bytes, anchor_rows)),
        fingerprint_bytes,
        fingerprint_blob_bytes,
        media_file_bytes,
        metadata_bytes,
        db_index_bytes,
        estimated_anchor_index_bytes,
        estimated_fingerprint_bytes,
        db_bytes_per_fingerprint: ratio_u64(total_bytes, fingerprint_rows),
        db_bytes_per_anchor: ratio_u64(total_bytes, anchor_rows),
    })
}

fn sqlite_pragma_u64(connection: &Connection, pragma_name: &str) -> Result<u64, String> {
    connection
        .query_row(&format!("PRAGMA {pragma_name}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|value| value.max(0) as u64)
        .map_err(|error| format!("failed reading SQLite pragma {pragma_name}: {error}"))
}

fn sqlite_object_bytes(
    connection: &Connection,
) -> Result<Vec<MediaMatchV3SqliteObjectBytes>, String> {
    let mut statement = connection
        .prepare("SELECT name, aggregate, SUM(pgsize) FROM dbstat GROUP BY name, aggregate")
        .map_err(|error| format!("SQLite dbstat unavailable: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(MediaMatchV3SqliteObjectBytes {
                name: row.get(0)?,
                object_type: row.get(1)?,
                bytes: row.get::<_, i64>(2)?.max(0) as u64,
            })
        })
        .map_err(|error| format!("failed querying SQLite dbstat: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed reading SQLite dbstat: {error}"))
}

fn sqlite_row_counts(
    connection: &Connection,
    object_bytes: &[MediaMatchV3SqliteObjectBytes],
    db_object_bytes_available: bool,
) -> Result<Vec<MediaMatchV3SqliteRowCount>, String> {
    let mut rows = Vec::new();
    for table in [
        "settings_v3",
        "media_files_v3",
        "fingerprints_v3",
        "audio_anchor_buckets_v3",
        "audio_anchor_occurrences_v3",
    ] {
        let row_count = row_count(connection, table);
        let bytes = object_bytes
            .iter()
            .filter(|object| object.name == table)
            .map(|object| object.bytes)
            .sum::<u64>();
        rows.push(MediaMatchV3SqliteRowCount {
            table: table.to_owned(),
            row_count,
            avg_bytes_per_row: (db_object_bytes_available && row_count > 0)
                .then(|| bytes as f64 / row_count as f64),
        });
    }
    Ok(rows)
}

fn row_count(connection: &Connection, table: &str) -> u64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0)
        .max(0) as u64
}

fn sqlite_table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    let exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed checking SQLite table '{table}': {error}"))?;
    Ok(exists > 0)
}

#[cfg(test)]
fn table_has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("failed reading SQLite table info for '{table}': {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed querying SQLite table info for '{table}': {error}"))?;
    let columns = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed reading SQLite table info for '{table}': {error}"))?;
    Ok(columns.iter().any(|name| name == column))
}

fn ratio_u64(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn duration_ms_from_seconds(duration_seconds: Option<f64>) -> Option<i64> {
    duration_seconds.map(|duration| (duration * 1000.0).round().max(0.0) as i64)
}

fn anchor_stats_v3_dirty_key(settings_hash: &[u8]) -> String {
    format!(
        "{MEDIA_MATCH_V3_ANCHOR_STATS_DIRTY_PREFIX}{}",
        bytes_to_lower_hex(settings_hash)
    )
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record(path: &str, buckets: &[u32]) -> MediaFingerprintRecord {
        test_record_with_duration(path, buckets, Some(120.0))
    }

    fn test_record_with_duration(
        path: &str,
        buckets: &[u32],
        duration_seconds: Option<f64>,
    ) -> MediaFingerprintRecord {
        MediaFingerprintRecord {
            identity: MediaFileIdentity::new(path, 10, 20),
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds,
            container_fingerprint: "container".to_owned(),
            audio_anchors: buckets
                .iter()
                .enumerate()
                .map(|(index, bucket)| crate::AudioAnchor {
                    bucket: *bucket,
                    t_ms: index as u32 * 1000,
                    weight: 10,
                })
                .collect(),
            audio_error: None,
        }
    }

    #[test]
    fn schema_is_audio_only() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();

        assert!(sqlite_table_exists(&connection, "audio_anchor_occurrences_v3").unwrap());
        assert!(sqlite_table_exists(&connection, "audio_anchor_buckets_v3").unwrap());
        assert!(!sqlite_table_exists(&connection, "video_anchor_occurrences_v3").unwrap());
        assert!(!sqlite_table_exists(&connection, "video_anchor_buckets_v3").unwrap());
        assert!(!sqlite_table_exists(&connection, "anchor_index_v3").unwrap());
        assert!(!sqlite_table_exists(&connection, "anchor_stats_v3").unwrap());
        assert!(!table_has_column(&connection, "fingerprints_v3", "video_blob").unwrap());
        assert!(!table_has_column(&connection, "fingerprints_v3", "video_index_count").unwrap());
        assert!(table_has_column(&connection, "fingerprints_v3", "audio_blob").unwrap());
        assert!(table_has_column(&connection, "fingerprints_v3", "audio_index_count").unwrap());
    }

    #[test]
    fn previous_schema_version_resets_cached_path_identities() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let record = test_record("MixedCase/Show.S01E01.mkv", &[1, 2, 3, 4]);
        save_media_match_v3_record(&connection, &record, None).unwrap();
        connection
            .execute(
                "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
                [MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION - 1],
            )
            .unwrap();

        initialize_media_match_v3_index(&connection).unwrap();

        assert_eq!(
            sqlite_schema_version(&connection).unwrap(),
            MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION
        );
        assert_eq!(row_count(&connection, "media_files_v3"), 0);
        assert_eq!(row_count(&connection, "fingerprints_v3"), 0);
    }

    #[test]
    fn save_and_load_audio_record() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let record = test_record("a.mkv", &[1, 2, 3, 4]);

        save_media_match_v3_record(&connection, &record, None).unwrap();
        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &record.identity.normalized_path,
            &record.extraction_settings,
            record.identity.modified_unix_millis,
            record.identity.size_bytes,
        )
        .unwrap()
        .unwrap();

        assert_eq!(loaded.audio_anchors.len(), 4);
        assert_eq!(loaded.identity, record.identity);
    }

    #[test]
    fn v3_record_save_rolls_back_every_reported_partial_write_cut_point() {
        let failpoints = [
            V3SaveProgress::FingerprintWritten,
            V3SaveProgress::AnchorsDeleted,
            V3SaveProgress::AnchorInserted(1),
            V3SaveProgress::AnchorInserted(2),
            V3SaveProgress::AnchorInserted(3),
            V3SaveProgress::AnchorInserted(4),
        ];

        for failpoint in failpoints {
            let connection = Connection::open_in_memory().unwrap();
            initialize_media_match_v3_index(&connection).unwrap();
            let original = test_record("transactional.mkv", &[1, 2, 3, 4]);
            save_media_match_v3_record(&connection, &original, None).unwrap();
            refresh_all_anchor_stats_v3(&connection, 100).unwrap();
            let settings_hash = media_extraction_settings_hash(&original.extraction_settings);
            assert!(!anchor_stats_v3_dirty(&connection, &settings_hash).unwrap());

            let mut replacement = test_record("transactional.mkv", &[11, 12, 13, 14]);
            replacement.identity.modified_unix_millis = 99;
            replacement.identity.size_bytes = 1234;
            replacement.container_fingerprint = "replacement-container".to_owned();
            let error = save_media_match_v3_record_with_stats_and_hook(
                &connection,
                &replacement,
                200,
                |progress| {
                    if progress == failpoint {
                        Err(format!("injected V3 save failure at {progress:?}"))
                    } else {
                        Ok(())
                    }
                },
            )
            .expect_err("the selected V3 save failpoint should abort the transaction");
            assert!(error.contains("injected V3 save failure"));

            let loaded = load_media_match_v3_record_for_path(
                &connection,
                &original.identity.normalized_path,
                &original.extraction_settings,
                original.identity.modified_unix_millis,
                original.identity.size_bytes,
            )
            .unwrap()
            .expect("the prior fingerprint must survive a failed replacement");
            assert_eq!(
                loaded.identity, original.identity,
                "failpoint {failpoint:?}"
            );
            assert_eq!(
                loaded.container_fingerprint, original.container_fingerprint,
                "failpoint {failpoint:?}"
            );
            assert_eq!(
                loaded.audio_anchors, original.audio_anchors,
                "failpoint {failpoint:?}"
            );
            assert!(
                !anchor_stats_v3_dirty(&connection, &settings_hash).unwrap(),
                "dirty-stat mutation must roll back at {failpoint:?}"
            );
        }
    }

    #[test]
    fn load_audio_record_tolerates_modified_time_drift_when_path_and_size_match() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let record = test_record("mtime-drift.mkv", &[1, 2, 3, 4]);

        save_media_match_v3_record(&connection, &record, None).unwrap();
        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &record.identity.normalized_path,
            &record.extraction_settings,
            record.identity.modified_unix_millis + 39_600_000,
            record.identity.size_bytes,
        )
        .unwrap()
        .unwrap();

        assert_eq!(loaded.audio_anchors.len(), 4);
        assert_eq!(loaded.identity, record.identity);
    }

    #[test]
    fn load_audio_record_rejects_size_mismatch_when_path_matches() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let record = test_record("size-drift.mkv", &[1, 2, 3, 4]);

        save_media_match_v3_record(&connection, &record, None).unwrap();
        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &record.identity.normalized_path,
            &record.extraction_settings,
            record.identity.modified_unix_millis + 39_600_000,
            record.identity.size_bytes + 1,
        )
        .unwrap();

        assert!(loaded.is_none());
    }

    #[test]
    fn retrieval_uses_audio_anchors() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let query = test_record("query.mkv", &[1, 2, 3, 4, 5, 6, 7, 8]);
        let candidate = test_record("candidate.mkv", &[1, 2, 3, 4, 5, 6, 7, 8]);

        save_media_match_v3_record(&connection, &query, None).unwrap();
        save_media_match_v3_record(&connection, &candidate, None).unwrap();
        refresh_all_anchor_stats_v3(&connection, current_unix_millis() as i64).unwrap();
        let (candidates, _stats) =
            media_match_v3_anchor_candidate_details_with_stats(&connection, &query, 0).unwrap();

        assert_eq!(
            candidates.first().unwrap().normalized_path,
            candidate.identity.normalized_path
        );
        assert!(!candidates.is_empty());
    }

    #[test]
    fn retrieved_candidate_diagnostics_expose_duration_compatibility() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_media_match_v3_index(&connection).unwrap();
        let query =
            test_record_with_duration("query.mkv", &[1, 2, 3, 4, 5, 6, 7, 8], Some(24.0 * 60.0));
        let candidate = test_record_with_duration(
            "candidate.mkv",
            &[1, 2, 3, 4, 5, 6, 7, 8],
            Some(12.0 * 60.0),
        );

        save_media_match_v3_record(&connection, &query, None).unwrap();
        save_media_match_v3_record(&connection, &candidate, None).unwrap();
        refresh_all_anchor_stats_v3(&connection, current_unix_millis() as i64).unwrap();
        let (candidates, _stats) =
            media_match_v3_anchor_candidate_details_with_stats(&connection, &query, 0).unwrap();
        let candidate = candidates.first().expect("candidate retrieved");

        assert_eq!(
            candidate.duration_compatibility,
            MediaDurationCompatibility::IncompatibleSameCut
        );
        let report = crate::MediaMatchV3DiagnosticRetrievalCandidateReport::from(candidate);
        assert_eq!(
            report.duration_compatibility,
            MediaDurationCompatibility::IncompatibleSameCut
        );
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["durationCompatibility"], "incompatible-same-cut");
    }

    #[test]
    fn sqlite_size_report_omits_unavailable_object_bytes() {
        let report = MediaMatchV3SqliteSizeReport {
            database_path: "index-v3.sqlite3".to_owned(),
            page_size: 4096,
            page_count: 10,
            freelist_count: 0,
            total_bytes: 40960,
            live_bytes: 40960,
            free_bytes: 0,
            dbstat_available: false,
            db_object_bytes_available: false,
            object_bytes: Vec::new(),
            row_counts: Vec::new(),
            anchor_index_bytes: None,
            anchor_index_bytes_per_anchor: None,
            fingerprint_bytes: None,
            fingerprint_blob_bytes: 2400,
            media_file_bytes: None,
            metadata_bytes: None,
            db_index_bytes: None,
            estimated_anchor_index_bytes: Some(38560),
            estimated_fingerprint_bytes: Some(2400),
            db_bytes_per_fingerprint: 10240.0,
            db_bytes_per_anchor: 26.6,
        };

        let value = serde_json::to_value(&report).unwrap();

        assert_eq!(value["dbObjectBytesAvailable"], false);
        assert!(value.get("anchorIndexBytes").is_none());
        assert!(value.get("dbIndexBytes").is_none());
        assert_eq!(value["estimatedAnchorIndexBytes"], 38560);
        assert_eq!(value["fingerprintBlobBytes"], 2400);
    }
}
