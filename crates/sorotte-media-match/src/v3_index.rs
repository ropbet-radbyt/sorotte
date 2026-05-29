use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION,
    anchors::{
        MediaFingerprintBlobV3, audio_index_landmarks_v3_from_record,
        decode_media_fingerprint_blob_v3, encode_media_fingerprint_blob_v3,
        media_fingerprint_blob_v3_from_record, media_fingerprint_record_apply_blob_v3,
        video_index_landmarks_v3_from_record,
    },
    identity::container_fingerprint_from_metadata,
    settings::{MediaExtractionSettings, media_extraction_settings_hash},
    tuning::{
        V3_COMMON_BUCKET_FILE_DIVISOR, V3_COMMON_BUCKET_MIN_SKIP_DF, V3_RETRIEVAL_GAP_MS,
        V3_RETRIEVAL_OFFSET_BIN_MS, V3_RETRIEVAL_PREFILTER_LIMIT, V3_RETRIEVAL_REGION_MS,
    },
    types::{MediaFileIdentity, MediaFingerprintRecord, MediaMatchCache},
    video_v3::validate_video_landmarks_v3,
};

const MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION: i64 = 3;
const MEDIA_MATCH_V3_INDEX_FILE: &str = "index-v3.sqlite3";
const MEDIA_MATCH_V3_MODALITY_AUDIO: i64 = 1;
const MEDIA_MATCH_V3_MODALITY_VIDEO: i64 = 2;
const MEDIA_MATCH_V3_ANCHOR_STATS_DIRTY_PREFIX: &str = "anchor_stats_v3_dirty:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatchV3IndexPaths {
    pub root: PathBuf,
    pub index_path: PathBuf,
}

impl MediaMatchV3IndexPaths {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            index_path: media_match_v3_index_path(root),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3RetrievalStats {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
    pub stats_dirty_check_millis: u128,
    pub stats_refresh_millis: u128,
    pub query_anchor_load_millis: u128,
    pub common_bucket_filter_millis: u128,
    pub sql_hit_fetch_millis: u128,
    pub rust_aggregation_millis: u128,
    pub candidate_sort_millis: u128,
    pub path_lookup_millis: u128,
    pub explain_query_plan_millis: u128,
    pub stats_refresh_ran: bool,
    pub stats_buckets_refreshed: i64,
    pub stats_anchor_rows_scanned: i64,
    pub anchor_stats_dirty_before_run: bool,
    pub anchor_stats_refreshed: bool,
    pub anchor_stats_refresh_millis: u128,
    pub anchor_stats_dirty_after_run: bool,
    pub query_anchor_count: i64,
    pub query_buckets_after_common_skip: i64,
    pub sql_rows_returned: i64,
    pub candidates_aggregated: i64,
    pub candidates_returned: i64,
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
    pub video_hits: i64,
    pub score_ratio_to_next: Option<f64>,
    pub query_duration_ms: Option<i64>,
    pub candidate_duration_ms: Option<i64>,
    pub duration_compatibility: String,
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
    file_id: i64,
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
    video_hits: i64,
    approximate_span_ms: i64,
    robust_score: i128,
    duration_compatibility: V3DurationCompatibility,
    short_clip_penalty_applied: bool,
    offset_bins: BTreeMap<i64, V3CandidateOffsetScore>,
}

#[derive(Debug, Clone, Default)]
struct V3CandidateOffsetScore {
    weighted_score: i64,
    query_regions: BTreeSet<i64>,
    candidate_regions: BTreeSet<i64>,
    query_times: Vec<i64>,
    candidate_times: Vec<i64>,
    audio_hits: i64,
    video_hits: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum V3DurationCompatibility {
    #[default]
    Unknown,
    Compatible,
    QueryFullCandidateShort,
    CandidateFullQueryShort,
}

impl V3DurationCompatibility {
    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Compatible => "compatible",
            Self::QueryFullCandidateShort => "query-full-candidate-short",
            Self::CandidateFullQueryShort => "candidate-full-query-short",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MediaMatchV3AnchorStatsRefreshStats {
    dirty_check_millis: u128,
    refresh_millis: u128,
    refresh_ran: bool,
    buckets_refreshed: i64,
    anchor_rows_scanned: i64,
    dirty_before: bool,
    dirty_after: bool,
}

pub fn media_match_v3_index_path(root: &Path) -> PathBuf {
    root.join("cache")
        .join("media-match")
        .join(MEDIA_MATCH_V3_INDEX_FILE)
}

pub fn open_media_match_v3_index(root: &Path) -> Result<Connection, String> {
    let paths = MediaMatchV3IndexPaths::new(root);
    if let Some(parent) = paths.index_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed creating media-match cache directory '{}': {error}",
                parent.display()
            )
        })?;
    }
    let connection = Connection::open(&paths.index_path).map_err(|error| {
        format!(
            "failed opening media-match SQLite index '{}': {error}",
            paths.index_path.display()
        )
    })?;
    initialize_media_match_v3_index(&connection)?;
    Ok(connection)
}

pub fn initialize_media_match_v3_index(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = NORMAL;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -65536;
            DROP TABLE IF EXISTS fingerprints_v1;
            DROP TABLE IF EXISTS audio_anchors;
            DROP TABLE IF EXISTS video_anchors;
            DROP TABLE IF EXISTS fingerprints;
            DROP TABLE IF EXISTS media_files;
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS media_files_v3 (
                file_id INTEGER PRIMARY KEY,
                normalized_path TEXT NOT NULL UNIQUE,
                modified_unix_millis INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                duration_ms INTEGER,
                container_format TEXT,
                video_codec TEXT,
                audio_codec TEXT,
                width INTEGER,
                height INTEGER,
                updated_unix_millis INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS fingerprints_v3 (
                file_id INTEGER NOT NULL,
                algorithm_version INTEGER NOT NULL,
                settings_hash BLOB NOT NULL,
                status TEXT NOT NULL,
                duration_ms INTEGER,
                audio_blob BLOB,
                video_blob BLOB,
                audio_verify_count INTEGER NOT NULL DEFAULT 0,
                video_verify_count INTEGER NOT NULL DEFAULT 0,
                audio_index_count INTEGER NOT NULL DEFAULT 0,
                video_index_count INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                updated_unix_millis INTEGER NOT NULL,
                PRIMARY KEY (file_id, algorithm_version, settings_hash),
                FOREIGN KEY (file_id) REFERENCES media_files_v3(file_id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS anchor_index_v3 (
                algorithm_version INTEGER NOT NULL,
                settings_hash BLOB NOT NULL,
                modality INTEGER NOT NULL,
                bucket INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                t_ms INTEGER NOT NULL,
                weight INTEGER NOT NULL,
                PRIMARY KEY (
                    algorithm_version, settings_hash, modality, bucket, file_id, t_ms
                )
            );
            CREATE INDEX IF NOT EXISTS idx_anchor_index_v3_lookup
                ON anchor_index_v3(algorithm_version, settings_hash, modality, bucket);
            CREATE INDEX IF NOT EXISTS idx_anchor_index_v3_lookup_covering
                ON anchor_index_v3(
                    algorithm_version,
                    settings_hash,
                    modality,
                    bucket,
                    file_id,
                    t_ms,
                    weight
                );
            CREATE INDEX IF NOT EXISTS idx_anchor_index_v3_file_settings
                ON anchor_index_v3(
                    algorithm_version,
                    settings_hash,
                    file_id,
                    modality,
                    bucket,
                    t_ms,
                    weight
                );
            CREATE TABLE IF NOT EXISTS anchor_stats_v3 (
                algorithm_version INTEGER NOT NULL,
                settings_hash BLOB NOT NULL,
                modality INTEGER NOT NULL,
                bucket INTEGER NOT NULL,
                document_frequency INTEGER NOT NULL,
                updated_unix_millis INTEGER NOT NULL,
                PRIMARY KEY (algorithm_version, settings_hash, modality, bucket)
            );
            ",
        )
        .map_err(|error| format!("failed initializing media-match V3 SQLite index: {error}"))?;
    connection
        .pragma_update(None, "user_version", MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION)
        .map_err(|error| format!("failed setting media-match V3 schema version: {error}"))?;
    Ok(())
}

pub fn save_media_match_v3_record(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    error: Option<&str>,
) -> Result<(), String> {
    save_media_match_v3_record_with_stats(connection, record, error).map(|_| ())
}

pub fn save_media_match_v3_record_with_stats(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    error: Option<&str>,
) -> Result<MediaMatchV3SaveStats, String> {
    let save_started_at = Instant::now();
    let mut stats = MediaMatchV3SaveStats::default();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match v3 save transaction: {error}"))?;
    let now = current_unix_millis() as i64;
    let duration_ms = duration_ms_from_seconds(record.duration_seconds);
    if let Some((file_id, old_mtime, old_size)) = transaction
        .query_row(
            "SELECT file_id, modified_unix_millis, size_bytes
             FROM media_files_v3
             WHERE normalized_path = ?1",
            [record.identity.normalized_path.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("failed reading media-match v3 media file row: {error}"))?
        && (old_mtime != record.identity.modified_unix_millis as i64
            || old_size != record.identity.size_bytes as i64)
    {
        delete_media_match_v3_fingerprints_and_anchors(&transaction, file_id)?;
    }
    transaction
        .execute(
            "INSERT INTO media_files_v3 (
                normalized_path,
                modified_unix_millis,
                size_bytes,
                duration_ms,
                container_format,
                video_codec,
                audio_codec,
                width,
                height,
                updated_unix_millis
            ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, ?6)
            ON CONFLICT(normalized_path) DO UPDATE SET
                modified_unix_millis = excluded.modified_unix_millis,
                size_bytes = excluded.size_bytes,
                duration_ms = excluded.duration_ms,
                container_format = excluded.container_format,
                updated_unix_millis = excluded.updated_unix_millis",
            params![
                record.identity.normalized_path.as_str(),
                record.identity.modified_unix_millis as i64,
                record.identity.size_bytes as i64,
                duration_ms,
                record.container_fingerprint.as_str(),
                now,
            ],
        )
        .map_err(|error| format!("failed writing media-match v3 media file row: {error}"))?;
    let file_id = transaction
        .query_row(
            "SELECT file_id FROM media_files_v3 WHERE normalized_path = ?1",
            [record.identity.normalized_path.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed reading media-match v3 file id: {error}"))?;
    let combined_error = error.map(str::to_owned).or_else(|| {
        let mut errors = Vec::new();
        if let Some(audio_error) = &record.audio_error {
            errors.push(format!("audio: {audio_error}"));
        }
        if let Some(video_error) = &record.video_error {
            errors.push(format!("video: {video_error}"));
        }
        (!errors.is_empty()).then(|| errors.join("; "))
    });
    if let Some(video) = &record.video
        && !video.v3_landmarks.is_empty()
    {
        validate_video_landmarks_v3(&video.v3_landmarks)
            .map_err(|error| format!("invalid media-match V3 video fingerprint: {error}"))?;
    }
    let blob_started_at = Instant::now();
    let blob = media_fingerprint_blob_v3_from_record(record);
    validate_video_landmarks_v3(&blob.video_landmarks)
        .map_err(|error| format!("invalid media-match V3 video fingerprint: {error}"))?;
    let audio_blob = (!blob.audio_landmarks.is_empty()).then(|| {
        encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
            duration_ms: blob.duration_ms,
            audio_landmarks: blob.audio_landmarks.clone(),
            video_landmarks: Vec::new(),
        })
    });
    let video_blob = (!blob.video_landmarks.is_empty()).then(|| {
        encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
            duration_ms: blob.duration_ms,
            audio_landmarks: Vec::new(),
            video_landmarks: blob.video_landmarks.clone(),
        })
    });
    let audio_index = audio_index_landmarks_v3_from_record(record);
    let video_index = video_index_landmarks_v3_from_record(record);
    stats.blob_encode_millis = blob_started_at.elapsed().as_millis();
    validate_video_landmarks_v3(&video_index)
        .map_err(|error| format!("invalid media-match V3 video index: {error}"))?;
    let settings_hash = media_extraction_settings_hash(&record.extraction_settings).to_vec();
    let index_started_at = Instant::now();
    transaction
        .execute(
            "DELETE FROM anchor_index_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2 AND file_id = ?3",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                settings_hash,
                file_id
            ],
        )
        .map_err(|error| format!("failed clearing media-match v3 anchor index: {error}"))?;
    for landmark in &audio_index {
        insert_anchor_index_v3(
            &transaction,
            file_id,
            &settings_hash,
            MEDIA_MATCH_V3_MODALITY_AUDIO,
            landmark.hash,
            landmark.t_ms,
            i64::from(landmark.weight.max(1)),
        )?;
    }
    for landmark in &video_index {
        insert_anchor_index_v3(
            &transaction,
            file_id,
            &settings_hash,
            MEDIA_MATCH_V3_MODALITY_VIDEO,
            landmark.bucket,
            landmark.t_ms,
            i64::from(landmark.weight.max(1)),
        )?;
    }
    mark_anchor_stats_v3_dirty(&transaction, &settings_hash)?;
    stats.index_insert_millis = index_started_at.elapsed().as_millis();
    let status = if error.is_some() {
        "error"
    } else if combined_error.is_some() {
        "partial"
    } else if blob.audio_landmarks.is_empty() && blob.video_landmarks.is_empty() {
        "empty"
    } else {
        "complete"
    };
    transaction
        .execute(
            "INSERT OR REPLACE INTO fingerprints_v3 (
                file_id,
                algorithm_version,
                settings_hash,
                status,
                duration_ms,
                audio_blob,
                video_blob,
                audio_verify_count,
                video_verify_count,
                audio_index_count,
                video_index_count,
                error,
                updated_unix_millis
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                file_id,
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                settings_hash,
                status,
                duration_ms,
                audio_blob,
                video_blob,
                blob.audio_landmarks.len() as i64,
                blob.video_landmarks.len() as i64,
                audio_index.len() as i64,
                video_index.len() as i64,
                combined_error,
                now,
            ],
        )
        .map_err(|error| format!("failed checkpointing media-match v3 fingerprint row: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed committing media-match v3 save transaction: {error}"))?;
    stats.sqlite_save_millis = save_started_at.elapsed().as_millis();
    Ok(stats)
}

pub fn load_media_match_v3_cache_for_settings(
    connection: &Connection,
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaMatchCache, String> {
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let mut statement = connection
        .prepare(
            "SELECT
                media_files_v3.normalized_path,
                media_files_v3.modified_unix_millis,
                media_files_v3.size_bytes,
                media_files_v3.duration_ms,
                media_files_v3.container_format,
                fingerprints_v3.duration_ms,
                fingerprints_v3.audio_blob,
                fingerprints_v3.video_blob,
                fingerprints_v3.error
             FROM fingerprints_v3
             JOIN media_files_v3 ON media_files_v3.file_id = fingerprints_v3.file_id
             WHERE fingerprints_v3.algorithm_version = ?1
                AND fingerprints_v3.settings_hash = ?2",
        )
        .map_err(|error| format!("failed preparing media-match v3 cache query: {error}"))?;
    let rows = statement
        .query_map(
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .map_err(|error| format!("failed reading media-match v3 cache rows: {error}"))?;
    let mut cache = MediaMatchCache::default();
    for row in rows.flatten() {
        if let Some(record) = media_match_v3_record_from_cached_blobs(
            row.0,
            row.1,
            row.2,
            row.3,
            row.4,
            row.5,
            row.6,
            row.7,
            row.8,
            extraction_settings,
        ) {
            cache.insert(record);
        }
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
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let row = connection
        .query_row(
            "SELECT
                media_files_v3.normalized_path,
                media_files_v3.modified_unix_millis,
                media_files_v3.size_bytes,
                media_files_v3.duration_ms,
                media_files_v3.container_format,
                fingerprints_v3.duration_ms,
                fingerprints_v3.audio_blob,
                fingerprints_v3.video_blob,
                fingerprints_v3.error
             FROM fingerprints_v3
             JOIN media_files_v3 ON media_files_v3.file_id = fingerprints_v3.file_id
             WHERE fingerprints_v3.algorithm_version = ?1
                AND fingerprints_v3.settings_hash = ?2
                AND media_files_v3.normalized_path = ?3",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                settings_hash,
                normalized_path,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("failed reading media-match v3 direct record: {error}"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let Some(record) = media_match_v3_record_from_cached_blobs(
        row.0,
        row.1,
        row.2,
        row.3,
        row.4,
        row.5,
        row.6,
        row.7,
        row.8,
        extraction_settings,
    ) else {
        return Ok(None);
    };
    Ok(record
        .valid_for(
            normalized_path,
            modified_unix_millis,
            size_bytes,
            MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings,
        )
        .then_some(record))
}

pub fn media_match_v3_anchor_candidate_paths_with_stats(
    connection: &Connection,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<(Vec<String>, MediaMatchV3RetrievalStats), String> {
    let (candidates, stats) = media_match_v3_anchor_candidate_details_with_stats(
        connection,
        normalized_current_path,
        extraction_settings,
    )?;
    Ok((
        candidates
            .into_iter()
            .map(|candidate| candidate.normalized_path)
            .collect(),
        stats,
    ))
}

pub fn media_match_v3_anchor_candidate_details_with_stats(
    connection: &Connection,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<
    (
        Vec<MediaMatchV3RetrievedCandidate>,
        MediaMatchV3RetrievalStats,
    ),
    String,
> {
    let started_at = Instant::now();
    let mut stats = MediaMatchV3RetrievalStats::default();
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let refresh_stats =
        refresh_dirty_anchor_stats_v3_if_needed_with_stats(connection, &settings_hash)?;
    stats.stats_dirty_check_millis = refresh_stats.dirty_check_millis;
    stats.stats_refresh_millis = refresh_stats.refresh_millis;
    stats.stats_refresh_ran = refresh_stats.refresh_ran;
    stats.stats_buckets_refreshed = refresh_stats.buckets_refreshed;
    stats.stats_anchor_rows_scanned = refresh_stats.anchor_rows_scanned;
    stats.anchor_stats_dirty_before_run = refresh_stats.dirty_before;
    stats.anchor_stats_refreshed = refresh_stats.refresh_ran;
    stats.anchor_stats_refresh_millis = refresh_stats.refresh_millis;
    stats.anchor_stats_dirty_after_run = refresh_stats.dirty_after;
    let query_anchor_started_at = Instant::now();
    let Some(current_file_id) = connection
        .query_row(
            "SELECT file_id FROM media_files_v3 WHERE normalized_path = ?1",
            [normalized_current_path],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("failed reading current media-match v3 file id: {error}"))?
    else {
        stats.retrieval_elapsed_ms = started_at.elapsed().as_millis();
        return Ok((Vec::new(), stats));
    };
    let query_duration_ms = connection
        .query_row(
            "SELECT duration_ms FROM media_files_v3 WHERE file_id = ?1",
            [current_file_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();
    stats.query_anchor_load_millis = query_anchor_started_at.elapsed().as_millis();
    let indexed_file_count = connection
        .query_row(
            "SELECT COUNT(DISTINCT file_id)
             FROM anchor_index_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), &settings_hash],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(1);
    let common_bucket_threshold =
        V3_COMMON_BUCKET_MIN_SKIP_DF.max(indexed_file_count / V3_COMMON_BUCKET_FILE_DIVISOR);
    let common_filter_started_at = Instant::now();
    let (query_buckets_total, query_buckets_skipped_common) = connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN COALESCE(stats.document_frequency, 1) > ?4 THEN 1 ELSE 0 END), 0)
             FROM anchor_index_v3 query
             LEFT JOIN anchor_stats_v3 stats
               ON stats.algorithm_version = query.algorithm_version
              AND stats.settings_hash = query.settings_hash
              AND stats.modality = query.modality
              AND stats.bucket = query.bucket
             WHERE query.algorithm_version = ?1
               AND query.settings_hash = ?2
               AND query.file_id = ?3",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                &settings_hash,
                current_file_id,
                common_bucket_threshold,
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap_or((0, 0));
    stats.query_buckets_total = query_buckets_total;
    stats.query_buckets_skipped_common = query_buckets_skipped_common;
    stats.query_anchor_count = query_buckets_total;
    stats.query_buckets_after_common_skip =
        query_buckets_total.saturating_sub(query_buckets_skipped_common);
    stats.common_bucket_filter_millis = common_filter_started_at.elapsed().as_millis();
    let query_anchor_started_at = Instant::now();
    connection
        .execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS media_match_v3_query_anchors (
                modality INTEGER NOT NULL,
                bucket INTEGER NOT NULL,
                query_t_ms INTEGER NOT NULL,
                query_weight INTEGER NOT NULL,
                document_frequency INTEGER NOT NULL
            );
            DELETE FROM media_match_v3_query_anchors;",
        )
        .map_err(|error| format!("failed preparing media-match v3 query anchors: {error}"))?;
    let query_buckets_after_common_skip = connection
        .execute(
            "INSERT INTO media_match_v3_query_anchors (
                modality,
                bucket,
                query_t_ms,
                query_weight,
                document_frequency
            )
            SELECT query.modality,
                   query.bucket,
                   query.t_ms,
                   query.weight,
                   COALESCE(stats.document_frequency, 1)
            FROM anchor_index_v3 query INDEXED BY idx_anchor_index_v3_file_settings
            LEFT JOIN anchor_stats_v3 stats
              ON stats.algorithm_version = query.algorithm_version
             AND stats.settings_hash = query.settings_hash
             AND stats.modality = query.modality
             AND stats.bucket = query.bucket
            WHERE query.algorithm_version = ?1
              AND query.settings_hash = ?2
              AND query.file_id = ?3
              AND COALESCE(stats.document_frequency, 1) <= ?4",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                &settings_hash,
                current_file_id,
                common_bucket_threshold,
            ],
        )
        .map_err(|error| format!("failed loading media-match v3 query anchors: {error}"))?;
    stats.query_buckets_after_common_skip = query_buckets_after_common_skip as i64;
    stats.query_anchor_load_millis = stats
        .query_anchor_load_millis
        .saturating_add(query_anchor_started_at.elapsed().as_millis());
    let mut statement = connection
        .prepare(
            "SELECT candidate.file_id,
                    query.query_t_ms,
                    candidate.t_ms,
                    query.modality,
                    MIN(query.query_weight, candidate.weight) AS hit_weight,
                    query.document_frequency,
                    candidate_file.duration_ms
             FROM media_match_v3_query_anchors query
             CROSS JOIN anchor_index_v3 candidate INDEXED BY idx_anchor_index_v3_lookup_covering
               ON candidate.algorithm_version = ?1
              AND candidate.settings_hash = ?2
              AND candidate.modality = query.modality
              AND candidate.bucket = query.bucket
              AND candidate.file_id != ?3
             JOIN media_files_v3 candidate_file
               ON candidate_file.file_id = candidate.file_id
             ",
        )
        .map_err(|error| {
            format!("failed preparing media-match v3 anchor candidate query: {error}")
        })?;
    let sql_hit_started_at = Instant::now();
    let rows = statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                &settings_hash,
                current_file_id,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(|error| format!("failed querying media-match v3 anchor candidates: {error}"))?;
    let hit_rows = rows.flatten().collect::<Vec<_>>();
    stats.sql_hit_fetch_millis = sql_hit_started_at.elapsed().as_millis();
    stats.sql_rows_returned = hit_rows.len() as i64;
    let aggregation_started_at = Instant::now();
    let mut scores = BTreeMap::<i64, V3CandidateRetrievalScore>::new();
    for row in hit_rows {
        stats.raw_hit_rows_processed += 1;
        let (
            file_id,
            query_t_ms,
            candidate_t_ms,
            modality,
            hit_weight,
            document_frequency,
            candidate_duration_ms,
        ) = row;
        let weighted_score =
            hit_weight.max(1) * media_match_v3_document_frequency_weight(document_frequency);
        let offset_bin = media_match_v3_rounded_offset_bin(candidate_t_ms - query_t_ms);
        let score = scores
            .entry(file_id)
            .or_insert_with(|| V3CandidateRetrievalScore {
                file_id,
                candidate_duration_ms,
                ..V3CandidateRetrievalScore::default()
            });
        score.candidate_duration_ms = score.candidate_duration_ms.or(candidate_duration_ms);
        score.total_score += weighted_score;
        match modality {
            MEDIA_MATCH_V3_MODALITY_AUDIO => score.audio_hits += 1,
            MEDIA_MATCH_V3_MODALITY_VIDEO => score.video_hits += 1,
            _ => {}
        }
        let offset_score = score.offset_bins.entry(offset_bin).or_default();
        offset_score.weighted_score += weighted_score;
        offset_score
            .query_regions
            .insert(query_t_ms / V3_RETRIEVAL_REGION_MS);
        offset_score
            .candidate_regions
            .insert(candidate_t_ms / V3_RETRIEVAL_REGION_MS);
        offset_score.query_times.push(query_t_ms);
        offset_score.candidate_times.push(candidate_t_ms);
        match modality {
            MEDIA_MATCH_V3_MODALITY_AUDIO => offset_score.audio_hits += 1,
            MEDIA_MATCH_V3_MODALITY_VIDEO => offset_score.video_hits += 1,
            _ => {}
        }
    }
    stats.rust_aggregation_millis = aggregation_started_at.elapsed().as_millis();
    stats.candidates_aggregated = scores.len() as i64;
    let sort_started_at = Instant::now();
    let mut ranked = scores
        .into_values()
        .map(|score| finalize_v3_candidate_retrieval_score(score, query_duration_ms))
        .collect::<Vec<_>>();
    stats.candidates_scored = ranked.len() as i64;
    ranked.sort_by(|left, right| {
        right
            .robust_score
            .cmp(&left.robust_score)
            .then_with(|| right.best_offset_score.cmp(&left.best_offset_score))
            .then_with(|| {
                (right.best_offset_score * left.total_score.max(1))
                    .cmp(&(left.best_offset_score * right.total_score.max(1)))
            })
            .then_with(|| right.approximate_span_ms.cmp(&left.approximate_span_ms))
            .then_with(|| {
                right
                    .distinct_query_regions
                    .cmp(&left.distinct_query_regions)
            })
            .then_with(|| {
                right
                    .distinct_candidate_regions
                    .cmp(&left.distinct_candidate_regions)
            })
            .then_with(|| {
                right
                    .best_offset_modality_count()
                    .cmp(&left.best_offset_modality_count())
            })
            .then_with(|| right.total_score.cmp(&left.total_score))
            .then_with(|| left.file_id.cmp(&right.file_id))
    });
    stats.candidate_sort_millis = sort_started_at.elapsed().as_millis();
    let ranked_limit = ranked.len().min(V3_RETRIEVAL_PREFILTER_LIMIT);
    let path_lookup_started_at = Instant::now();
    let mut candidates = Vec::new();
    for index in 0..ranked_limit {
        let score = &ranked[index];
        if let Ok((path, candidate_duration_ms)) = connection.query_row(
            "SELECT normalized_path, duration_ms FROM media_files_v3 WHERE file_id = ?1",
            [score.file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
        ) {
            let (body_region_count, edge_region_count) =
                score.best_offset_body_edge_region_counts(query_duration_ms, candidate_duration_ms);
            candidates.push(MediaMatchV3RetrievedCandidate {
                normalized_path: path,
                rank: index + 1,
                total_score: score.total_score,
                best_offset_bin_ms: score
                    .best_offset_bin
                    .saturating_mul(V3_RETRIEVAL_OFFSET_BIN_MS),
                best_offset_score: score.best_offset_score,
                second_offset_score: score.second_offset_score,
                distinct_query_regions: score.distinct_query_regions,
                distinct_candidate_regions: score.distinct_candidate_regions,
                body_region_count,
                edge_region_count,
                approximate_span_ms: score.approximate_span_ms,
                audio_hits: score.audio_hits,
                video_hits: score.video_hits,
                score_ratio_to_next: ranked.get(index + 1).and_then(|next| {
                    (next.best_offset_score > 0)
                        .then(|| score.best_offset_score as f64 / next.best_offset_score as f64)
                }),
                query_duration_ms,
                candidate_duration_ms,
                duration_compatibility: score.duration_compatibility.label().to_owned(),
                short_clip_penalty_applied: score.short_clip_penalty_applied,
                robust_score: score.robust_score as f64,
            });
        }
    }
    stats.path_lookup_millis = path_lookup_started_at.elapsed().as_millis();
    stats.candidates_returned = candidates.len() as i64;
    stats.retrieval_elapsed_ms = started_at.elapsed().as_millis();
    Ok((candidates, stats))
}

pub fn refresh_dirty_anchor_stats_v3_if_needed(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<(), String> {
    refresh_dirty_anchor_stats_v3_if_needed_with_stats(connection, settings_hash).map(|_| ())
}

fn refresh_dirty_anchor_stats_v3_if_needed_with_stats(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<MediaMatchV3AnchorStatsRefreshStats, String> {
    let dirty_started_at = Instant::now();
    let dirty_before = anchor_stats_v3_dirty(connection, settings_hash)?;
    let dirty_check_millis = dirty_started_at.elapsed().as_millis();
    if !dirty_before {
        return Ok(MediaMatchV3AnchorStatsRefreshStats {
            dirty_check_millis,
            dirty_before,
            dirty_after: false,
            ..MediaMatchV3AnchorStatsRefreshStats::default()
        });
    }

    let refresh_started_at = Instant::now();
    let anchor_rows_scanned = count_anchor_rows_for_settings(connection, settings_hash)?;
    let buckets_refreshed = refresh_anchor_stats_v3_with_count(
        connection,
        settings_hash,
        current_unix_millis() as i64,
    )?;
    let refresh_millis = refresh_started_at.elapsed().as_millis();
    let dirty_after = anchor_stats_v3_dirty(connection, settings_hash)?;
    Ok(MediaMatchV3AnchorStatsRefreshStats {
        dirty_check_millis,
        refresh_millis,
        refresh_ran: true,
        buckets_refreshed,
        anchor_rows_scanned,
        dirty_before,
        dirty_after,
    })
}

pub fn refresh_anchor_stats_v3(
    connection: &Connection,
    settings_hash: &[u8],
    now: i64,
) -> Result<(), String> {
    refresh_anchor_stats_v3_with_count(connection, settings_hash, now).map(|_| ())
}

fn refresh_anchor_stats_v3_with_count(
    connection: &Connection,
    settings_hash: &[u8],
    now: i64,
) -> Result<i64, String> {
    connection
        .execute(
            "DELETE FROM anchor_stats_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash],
        )
        .map_err(|error| format!("failed clearing media-match v3 anchor stats: {error}"))?;
    let buckets_refreshed = connection
        .execute(
            "INSERT INTO anchor_stats_v3 (
                algorithm_version,
                settings_hash,
                modality,
                bucket,
                document_frequency,
                updated_unix_millis
            )
            SELECT algorithm_version,
                   settings_hash,
                   modality,
                   bucket,
                   COUNT(DISTINCT file_id),
                   ?3
            FROM anchor_index_v3
            WHERE algorithm_version = ?1 AND settings_hash = ?2
            GROUP BY algorithm_version, settings_hash, modality, bucket",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash, now],
        )
        .map_err(|error| format!("failed refreshing media-match v3 anchor stats: {error}"))?;
    clear_anchor_stats_v3_dirty(connection, settings_hash)?;
    let _ = connection.execute_batch("ANALYZE anchor_index_v3; ANALYZE anchor_stats_v3;");
    Ok(buckets_refreshed as i64)
}

pub fn refresh_all_anchor_stats_v3(connection: &Connection, now: i64) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM anchor_stats_v3
             WHERE algorithm_version = ?1",
            [i64::from(MEDIA_MATCH_ANCHOR_VERSION)],
        )
        .map_err(|error| format!("failed clearing media-match v3 anchor stats: {error}"))?;
    connection
        .execute(
            "INSERT INTO anchor_stats_v3 (
                algorithm_version,
                settings_hash,
                modality,
                bucket,
                document_frequency,
                updated_unix_millis
            )
            SELECT algorithm_version,
                   settings_hash,
                   modality,
                   bucket,
                   COUNT(DISTINCT file_id),
                   ?2
            FROM anchor_index_v3
            WHERE algorithm_version = ?1
            GROUP BY algorithm_version, settings_hash, modality, bucket",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), now],
        )
        .map(|_| ())
        .map_err(|error| format!("failed refreshing all media-match v3 anchor stats: {error}"))?;
    clear_all_anchor_stats_v3_dirty(connection)?;
    let _ = connection.execute_batch("ANALYZE anchor_index_v3; ANALYZE anchor_stats_v3;");
    Ok(())
}

fn count_anchor_rows_for_settings(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COUNT(*)
             FROM anchor_index_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed counting media-match v3 anchor rows: {error}"))
}

pub fn mark_anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<(), String> {
    let key = anchor_stats_v3_dirty_key(settings_hash);
    connection
        .execute(
            "INSERT INTO metadata (key, value)
             VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key],
        )
        .map(|_| ())
        .map_err(|error| format!("failed marking media-match v3 anchor stats dirty: {error}"))
}

pub fn clear_anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<(), String> {
    let key = anchor_stats_v3_dirty_key(settings_hash);
    connection
        .execute("DELETE FROM metadata WHERE key = ?1", [key])
        .map(|_| ())
        .map_err(|error| {
            format!("failed clearing media-match v3 anchor stats dirty marker: {error}")
        })
}

pub fn clear_all_anchor_stats_v3_dirty(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM metadata WHERE key LIKE ?1",
            [format!("{MEDIA_MATCH_V3_ANCHOR_STATS_DIRTY_PREFIX}%")],
        )
        .map(|_| ())
        .map_err(|error| {
            format!("failed clearing media-match v3 anchor stats dirty markers: {error}")
        })
}

pub fn anchor_stats_v3_dirty(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<bool, String> {
    let key = anchor_stats_v3_dirty_key(settings_hash);
    let value = connection
        .query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| {
            format!("failed reading media-match v3 anchor stats dirty marker: {error}")
        })?;
    Ok(value.is_some_and(|value| value != "0"))
}

pub fn mark_anchor_stats_v3_dirty_for_file(
    connection: &Connection,
    file_id: i64,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT settings_hash FROM fingerprints_v3 WHERE file_id = ?1
             UNION
             SELECT DISTINCT settings_hash FROM anchor_index_v3 WHERE file_id = ?1",
        )
        .map_err(|error| {
            format!("failed preparing media-match v3 dirty-settings query: {error}")
        })?;
    let hashes = statement
        .query_map([file_id], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|error| format!("failed querying media-match v3 dirty settings: {error}"))?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    for settings_hash in hashes {
        mark_anchor_stats_v3_dirty(connection, &settings_hash)?;
    }
    Ok(())
}

pub fn delete_media_match_v3_fingerprints_and_anchors(
    connection: &Connection,
    file_id: i64,
) -> Result<(), String> {
    mark_anchor_stats_v3_dirty_for_file(connection, file_id)?;
    connection
        .execute("DELETE FROM anchor_index_v3 WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match v3 anchors: {error}"))?;
    connection
        .execute("DELETE FROM fingerprints_v3 WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match v3 fingerprints: {error}"))?;
    Ok(())
}

pub fn delete_media_match_v3_file_and_fingerprints(
    connection: &Connection,
    file_id: i64,
) -> Result<(), String> {
    delete_media_match_v3_fingerprints_and_anchors(connection, file_id)?;
    connection
        .execute("DELETE FROM media_files_v3 WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match v3 file row: {error}"))?;
    Ok(())
}

fn insert_anchor_index_v3(
    connection: &Connection,
    file_id: i64,
    settings_hash: &[u8],
    modality: i64,
    bucket: u32,
    t_ms: u32,
    weight: i64,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR REPLACE INTO anchor_index_v3 (
                algorithm_version, settings_hash, modality, bucket, file_id, t_ms, weight
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                settings_hash,
                modality,
                i64::from(bucket),
                file_id,
                i64::from(t_ms),
                weight,
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("failed writing media-match v3 anchor index row: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn media_match_v3_record_from_cached_blobs(
    normalized_path: String,
    modified_unix_millis: i64,
    size_bytes: i64,
    media_duration_ms: Option<i64>,
    container_format: Option<String>,
    fingerprint_duration_ms: Option<i64>,
    audio_blob: Option<Vec<u8>>,
    video_blob: Option<Vec<u8>>,
    error: Option<String>,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaFingerprintRecord> {
    let duration_ms = fingerprint_duration_ms.or(media_duration_ms);
    let duration_seconds = duration_ms.map(|value| value as f64 / 1000.0);
    let mut record = MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: normalized_path.clone(),
            modified_unix_millis: modified_unix_millis.max(0) as u64,
            size_bytes: size_bytes.max(0) as u64,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: extraction_settings.clone(),
        duration_seconds,
        container_fingerprint: container_format.unwrap_or_else(|| {
            container_fingerprint_from_metadata(
                &normalized_path,
                modified_unix_millis.max(0) as u64,
                size_bytes.max(0) as u64,
                duration_seconds,
            )
        }),
        video: None,
        audio_anchors: Vec::new(),
        video_anchors: Vec::new(),
        audio_error: error.clone(),
        video_error: error,
    };
    if let Some(blob_bytes) = audio_blob {
        let blob = decode_media_fingerprint_blob_v3(&blob_bytes).ok()?;
        media_fingerprint_record_apply_blob_v3(&mut record, blob);
    }
    if let Some(blob_bytes) = video_blob {
        let blob = decode_media_fingerprint_blob_v3(&blob_bytes).ok()?;
        validate_video_landmarks_v3(&blob.video_landmarks).ok()?;
        let duration = record.duration_seconds;
        let audio_anchors = std::mem::take(&mut record.audio_anchors);
        media_fingerprint_record_apply_blob_v3(&mut record, blob);
        record.audio_anchors = audio_anchors;
        record.duration_seconds = record.duration_seconds.or(duration);
    }
    Some(record)
}

fn finalize_v3_candidate_retrieval_score(
    mut score: V3CandidateRetrievalScore,
    query_duration_ms: Option<i64>,
) -> V3CandidateRetrievalScore {
    let mut offset_bins = score
        .offset_bins
        .iter()
        .map(|(offset_bin, offset_score)| {
            let span = media_match_v3_longest_contiguous_span_ms(&offset_score.query_times).max(
                media_match_v3_longest_contiguous_span_ms(&offset_score.candidate_times),
            );
            (*offset_bin, offset_score.weighted_score, span)
        })
        .collect::<Vec<_>>();
    offset_bins.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    if let Some((best_offset_bin, best_score, best_span)) = offset_bins.first().copied() {
        score.best_offset_bin = best_offset_bin;
        score.best_offset_score = best_score;
        score.approximate_span_ms = best_span;
        score.second_offset_score = offset_bins.get(1).map(|(_, value, _)| *value).unwrap_or(0);
        if let Some(best_offset_score) = score.offset_bins.get(&best_offset_bin) {
            score.distinct_query_regions = best_offset_score.query_regions.len() as i64;
            score.distinct_candidate_regions = best_offset_score.candidate_regions.len() as i64;
        }
    }
    let (body_region_count, edge_region_count) =
        score.best_offset_body_edge_region_counts(query_duration_ms, score.candidate_duration_ms);
    score.body_region_count = body_region_count;
    score.edge_region_count = edge_region_count;
    score.duration_compatibility =
        media_match_v3_duration_compatibility(query_duration_ms, score.candidate_duration_ms);
    score.short_clip_penalty_applied =
        score.duration_compatibility == V3DurationCompatibility::QueryFullCandidateShort;
    score.robust_score = media_match_v3_robust_retrieval_score(&score);
    score
}

impl V3CandidateRetrievalScore {
    fn best_offset_modality_count(&self) -> i64 {
        self.offset_bins
            .get(&self.best_offset_bin)
            .map(|score| {
                (if score.audio_hits > 0 { 1 } else { 0 })
                    + (if score.video_hits > 0 { 1 } else { 0 })
            })
            .unwrap_or(0)
    }

    fn best_offset_body_edge_region_counts(
        &self,
        query_duration_ms: Option<i64>,
        candidate_duration_ms: Option<i64>,
    ) -> (i64, i64) {
        let Some(offset_score) = self.offset_bins.get(&self.best_offset_bin) else {
            return (0, 0);
        };
        let mut body_regions = BTreeSet::new();
        let mut edge_regions = BTreeSet::new();
        for (query_t_ms, candidate_t_ms) in offset_score
            .query_times
            .iter()
            .copied()
            .zip(offset_score.candidate_times.iter().copied())
        {
            let query_region = query_t_ms / V3_RETRIEVAL_REGION_MS;
            if media_match_v3_time_is_edge(query_t_ms, query_duration_ms)
                || media_match_v3_time_is_edge(candidate_t_ms, candidate_duration_ms)
            {
                edge_regions.insert(query_region);
            } else {
                body_regions.insert(query_region);
            }
        }
        (body_regions.len() as i64, edge_regions.len() as i64)
    }
}

fn media_match_v3_duration_compatibility(
    query_duration_ms: Option<i64>,
    candidate_duration_ms: Option<i64>,
) -> V3DurationCompatibility {
    const SHORT_CLIP_MS: i64 = 5 * 60 * 1000;
    const FULL_LENGTH_MS: i64 = 10 * 60 * 1000;
    match (query_duration_ms, candidate_duration_ms) {
        (Some(query), Some(candidate)) if query >= FULL_LENGTH_MS && candidate < SHORT_CLIP_MS => {
            V3DurationCompatibility::QueryFullCandidateShort
        }
        (Some(query), Some(candidate)) if query < SHORT_CLIP_MS && candidate >= FULL_LENGTH_MS => {
            V3DurationCompatibility::CandidateFullQueryShort
        }
        (Some(_), Some(_)) => V3DurationCompatibility::Compatible,
        _ => V3DurationCompatibility::Unknown,
    }
}

fn media_match_v3_robust_retrieval_score(score: &V3CandidateRetrievalScore) -> i128 {
    let mut robust = i128::from(score.best_offset_score.max(0)) * span_factor(score) / 1_000;
    robust = robust * region_factor(score) / 1_000;
    robust = robust * offset_dominance_factor(score) / 1_000;
    robust = robust * duration_factor(score) / 1_000;
    robust.max(0)
}

fn span_factor(score: &V3CandidateRetrievalScore) -> i128 {
    match score.approximate_span_ms {
        span if span < 1_000 => 200,
        span if span < 2_000 => 500,
        span if span < 5_000 => 1_000 + i128::from(span / 20),
        span => 2_000 + i128::from(span.min(60_000) / 20),
    }
}

fn region_factor(score: &V3CandidateRetrievalScore) -> i128 {
    let query_regions = score.distinct_query_regions.saturating_sub(1).clamp(0, 8);
    let candidate_regions = score
        .distinct_candidate_regions
        .saturating_sub(1)
        .clamp(0, 8);
    let body_regions = score.body_region_count.clamp(0, 8);
    let edge_only_penalty = if score.body_region_count == 0 && score.edge_region_count > 0 {
        650
    } else {
        1_000
    };
    (1_000
        + 100 * i128::from(query_regions)
        + 100 * i128::from(candidate_regions)
        + 150 * i128::from(body_regions))
        * edge_only_penalty
        / 1_000
}

fn offset_dominance_factor(score: &V3CandidateRetrievalScore) -> i128 {
    if score.second_offset_score <= 0 {
        return 1_300;
    }
    let ratio_milli = (score.best_offset_score.max(0) * 1_000 / score.second_offset_score.max(1))
        .clamp(1_000, 4_000);
    900 + i128::from(ratio_milli / 5)
}

fn duration_factor(score: &V3CandidateRetrievalScore) -> i128 {
    match score.duration_compatibility {
        V3DurationCompatibility::QueryFullCandidateShort => {
            if score.approximate_span_ms >= 30_000
                && score.body_region_count >= 2
                && score.best_offset_score >= 10_000
            {
                1_000
            } else {
                250
            }
        }
        V3DurationCompatibility::CandidateFullQueryShort => 850,
        V3DurationCompatibility::Compatible | V3DurationCompatibility::Unknown => 1_000,
    }
}

fn media_match_v3_document_frequency_weight(document_frequency: i64) -> i64 {
    match document_frequency {
        frequency if frequency <= 1 => 4,
        2..=4 => 3,
        5..=16 => 2,
        _ => 1,
    }
}

fn media_match_v3_rounded_offset_bin(offset_ms: i64) -> i64 {
    if offset_ms >= 0 {
        (offset_ms + (V3_RETRIEVAL_OFFSET_BIN_MS / 2)) / V3_RETRIEVAL_OFFSET_BIN_MS
    } else {
        (offset_ms - (V3_RETRIEVAL_OFFSET_BIN_MS / 2)) / V3_RETRIEVAL_OFFSET_BIN_MS
    }
}

fn media_match_v3_time_is_edge(time_ms: i64, duration_ms: Option<i64>) -> bool {
    const EDGE_REGION_MS: i64 = 180_000;
    time_ms < EDGE_REGION_MS
        || duration_ms
            .map(|duration_ms| duration_ms.saturating_sub(time_ms) < EDGE_REGION_MS)
            .unwrap_or(false)
}

fn media_match_v3_longest_contiguous_span_ms(times: &[i64]) -> i64 {
    if times.len() < 2 {
        return 0;
    }
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut segment_start = sorted[0];
    let mut previous = sorted[0];
    let mut best = 0;
    for time in sorted.into_iter().skip(1) {
        if time - previous > V3_RETRIEVAL_GAP_MS {
            best = best.max(previous - segment_start);
            segment_start = time;
        }
        previous = time;
    }
    best.max(previous - segment_start)
}

fn duration_ms_from_seconds(duration_seconds: Option<f64>) -> Option<i64> {
    duration_seconds
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| (value * 1000.0).round().min(f64::from(u32::MAX)) as i64)
}

fn anchor_stats_v3_dirty_key(settings_hash: &[u8]) -> String {
    format!(
        "{MEDIA_MATCH_V3_ANCHOR_STATS_DIRTY_PREFIX}{}",
        bytes_to_lower_hex(settings_hash)
    )
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn current_unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robust_retrieval_score_demotes_short_clip_for_full_query() {
        let full_candidate = V3CandidateRetrievalScore {
            best_offset_score: 2_426,
            second_offset_score: 48,
            approximate_span_ms: 12_160,
            distinct_query_regions: 2,
            distinct_candidate_regions: 1,
            body_region_count: 2,
            edge_region_count: 0,
            duration_compatibility: V3DurationCompatibility::Compatible,
            ..V3CandidateRetrievalScore::default()
        };
        let op_clip = V3CandidateRetrievalScore {
            best_offset_score: 3_982,
            second_offset_score: 56,
            approximate_span_ms: 2_101,
            distinct_query_regions: 1,
            distinct_candidate_regions: 1,
            body_region_count: 0,
            edge_region_count: 1,
            duration_compatibility: V3DurationCompatibility::QueryFullCandidateShort,
            short_clip_penalty_applied: true,
            ..V3CandidateRetrievalScore::default()
        };

        assert!(
            media_match_v3_robust_retrieval_score(&full_candidate)
                > media_match_v3_robust_retrieval_score(&op_clip)
        );
    }

    #[test]
    fn robust_retrieval_score_rewards_coherent_span_over_one_region_collision() {
        let true_candidate = V3CandidateRetrievalScore {
            best_offset_score: 818,
            second_offset_score: 228,
            approximate_span_ms: 7_680,
            distinct_query_regions: 1,
            distinct_candidate_regions: 1,
            body_region_count: 1,
            edge_region_count: 0,
            duration_compatibility: V3DurationCompatibility::Compatible,
            ..V3CandidateRetrievalScore::default()
        };
        let one_region_collision = V3CandidateRetrievalScore {
            best_offset_score: 1_040,
            second_offset_score: 0,
            approximate_span_ms: 512,
            distinct_query_regions: 1,
            distinct_candidate_regions: 1,
            body_region_count: 1,
            edge_region_count: 0,
            duration_compatibility: V3DurationCompatibility::Compatible,
            ..V3CandidateRetrievalScore::default()
        };

        assert!(
            media_match_v3_robust_retrieval_score(&true_candidate)
                > media_match_v3_robust_retrieval_score(&one_region_collision)
        );
    }

    #[test]
    fn dirty_anchor_stats_refresh_runs_once_and_clears_marker() {
        let connection = Connection::open_in_memory().expect("in-memory SQLite should open");
        initialize_media_match_v3_index(&connection).expect("schema should initialize");
        let settings_hash = [7_u8; 32];
        for file_id in 1..=2 {
            connection
                .execute(
                    "INSERT INTO media_files_v3 (
                        file_id,
                        normalized_path,
                        modified_unix_millis,
                        size_bytes,
                        duration_ms,
                        container_format,
                        updated_unix_millis
                    ) VALUES (?1, ?2, 1, 1, 1000, 'test', 1)",
                    params![file_id, format!("file-{file_id}.mkv")],
                )
                .expect("media row should insert");
            insert_anchor_index_v3(&connection, file_id, &settings_hash, 1, 42, 1000, 1)
                .expect("anchor row should insert");
        }
        mark_anchor_stats_v3_dirty(&connection, &settings_hash).expect("dirty marker should set");

        let first = refresh_dirty_anchor_stats_v3_if_needed_with_stats(&connection, &settings_hash)
            .expect("first refresh should run");
        let second =
            refresh_dirty_anchor_stats_v3_if_needed_with_stats(&connection, &settings_hash)
                .expect("second refresh should not run");

        assert!(first.dirty_before);
        assert!(first.refresh_ran);
        assert_eq!(first.anchor_rows_scanned, 2);
        assert_eq!(first.buckets_refreshed, 1);
        assert!(!first.dirty_after);
        assert!(!second.dirty_before);
        assert!(!second.refresh_ran);
        assert!(!anchor_stats_v3_dirty(&connection, &settings_hash).expect("dirty should read"));
    }
}
