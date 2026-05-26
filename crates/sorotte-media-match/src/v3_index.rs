use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION, MediaExtractionSettings,
    MediaFileIdentity, MediaFingerprintBlobV3, MediaFingerprintRecord, MediaMatchCache,
    audio_index_landmarks_v3_from_record, container_fingerprint_from_metadata,
    decode_media_fingerprint_blob_v3, encode_media_fingerprint_blob_v3,
    media_extraction_settings_hash, media_fingerprint_blob_v3_from_record,
    media_fingerprint_record_apply_blob_v3, validate_video_landmarks_v3,
    video_index_landmarks_v3_from_record,
};

const MEDIA_MATCH_V3_SQLITE_SCHEMA_VERSION: i64 = 3;
const MEDIA_MATCH_V3_INDEX_FILE: &str = "index-v3.sqlite3";
const MEDIA_MATCH_V3_MODALITY_AUDIO: i64 = 1;
const MEDIA_MATCH_V3_MODALITY_VIDEO: i64 = 2;
const MEDIA_MATCH_V3_PREFILTER_LIMIT: usize = 24;
const MEDIA_MATCH_V3_OFFSET_BIN_MS: i64 = 1_000;
const MEDIA_MATCH_V3_RETRIEVAL_REGION_MS: i64 = 60_000;
const MEDIA_MATCH_V3_RETRIEVAL_GAP_MS: i64 = 120_000;
const MEDIA_MATCH_V3_COMMON_BUCKET_MIN_SKIP_DF: i64 = 256;
const MEDIA_MATCH_V3_COMMON_BUCKET_FILE_DIVISOR: i64 = 20;
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

#[derive(Debug)]
pub struct MediaMatchV3Index;

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3RetrievalStats {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
}

#[derive(Debug, Clone, Default)]
struct V3CandidateRetrievalScore {
    file_id: i64,
    total_score: i64,
    best_offset_bin: i64,
    best_offset_score: i64,
    second_offset_score: i64,
    distinct_query_regions: i64,
    distinct_candidate_regions: i64,
    audio_hits: i64,
    video_hits: i64,
    approximate_span_ms: i64,
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
    validate_video_landmarks_v3(&video_index)
        .map_err(|error| format!("invalid media-match V3 video index: {error}"))?;
    let settings_hash = media_extraction_settings_hash(&record.extraction_settings).to_vec();
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
        .map_err(|error| format!("failed committing media-match v3 save transaction: {error}"))
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
    let started_at = Instant::now();
    let mut stats = MediaMatchV3RetrievalStats::default();
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    refresh_dirty_anchor_stats_v3_if_needed(connection, &settings_hash)?;
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
    let indexed_file_count = connection
        .query_row(
            "SELECT COUNT(DISTINCT file_id)
             FROM anchor_index_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(1);
    let common_bucket_threshold = MEDIA_MATCH_V3_COMMON_BUCKET_MIN_SKIP_DF
        .max(indexed_file_count / MEDIA_MATCH_V3_COMMON_BUCKET_FILE_DIVISOR);
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
    let mut statement = connection
        .prepare(
            "SELECT candidate.file_id,
                    query.t_ms,
                    candidate.t_ms,
                    query.modality,
                    MIN(query.weight, candidate.weight) AS hit_weight,
                    COALESCE(stats.document_frequency, 1) AS document_frequency
             FROM anchor_index_v3 query
             JOIN anchor_index_v3 candidate
               ON candidate.algorithm_version = query.algorithm_version
              AND candidate.settings_hash = query.settings_hash
              AND candidate.modality = query.modality
              AND candidate.bucket = query.bucket
              AND candidate.file_id != query.file_id
             LEFT JOIN anchor_stats_v3 stats
               ON stats.algorithm_version = query.algorithm_version
              AND stats.settings_hash = query.settings_hash
              AND stats.modality = query.modality
              AND stats.bucket = query.bucket
             WHERE query.algorithm_version = ?1
               AND query.settings_hash = ?2
               AND query.file_id = ?3
               AND COALESCE(stats.document_frequency, 1) <= ?4",
        )
        .map_err(|error| {
            format!("failed preparing media-match v3 anchor candidate query: {error}")
        })?;
    let rows = statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                settings_hash,
                current_file_id,
                common_bucket_threshold,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(|error| format!("failed querying media-match v3 anchor candidates: {error}"))?;
    let mut scores = BTreeMap::<i64, V3CandidateRetrievalScore>::new();
    for row in rows.flatten() {
        stats.raw_hit_rows_processed += 1;
        let (file_id, query_t_ms, candidate_t_ms, modality, hit_weight, document_frequency) = row;
        let weighted_score =
            hit_weight.max(1) * media_match_v3_document_frequency_weight(document_frequency);
        let offset_bin = media_match_v3_rounded_offset_bin(candidate_t_ms - query_t_ms);
        let score = scores
            .entry(file_id)
            .or_insert_with(|| V3CandidateRetrievalScore {
                file_id,
                ..V3CandidateRetrievalScore::default()
            });
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
            .insert(query_t_ms / MEDIA_MATCH_V3_RETRIEVAL_REGION_MS);
        offset_score
            .candidate_regions
            .insert(candidate_t_ms / MEDIA_MATCH_V3_RETRIEVAL_REGION_MS);
        offset_score.query_times.push(query_t_ms);
        offset_score.candidate_times.push(candidate_t_ms);
        match modality {
            MEDIA_MATCH_V3_MODALITY_AUDIO => offset_score.audio_hits += 1,
            MEDIA_MATCH_V3_MODALITY_VIDEO => offset_score.video_hits += 1,
            _ => {}
        }
    }
    let mut ranked = scores
        .into_values()
        .map(finalize_v3_candidate_retrieval_score)
        .collect::<Vec<_>>();
    stats.candidates_scored = ranked.len() as i64;
    ranked.sort_by(|left, right| {
        right
            .best_offset_score
            .cmp(&left.best_offset_score)
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
    let mut paths = Vec::new();
    for score in ranked.into_iter().take(MEDIA_MATCH_V3_PREFILTER_LIMIT) {
        if let Ok(path) = connection.query_row(
            "SELECT normalized_path FROM media_files_v3 WHERE file_id = ?1",
            [score.file_id],
            |row| row.get::<_, String>(0),
        ) {
            paths.push(path);
        }
    }
    stats.retrieval_elapsed_ms = started_at.elapsed().as_millis();
    Ok((paths, stats))
}

pub fn refresh_dirty_anchor_stats_v3_if_needed(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<(), String> {
    if anchor_stats_v3_dirty(connection, settings_hash)? {
        refresh_anchor_stats_v3(connection, settings_hash, current_unix_millis() as i64)?;
    }
    Ok(())
}

pub fn refresh_anchor_stats_v3(
    connection: &Connection,
    settings_hash: &[u8],
    now: i64,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM anchor_stats_v3
             WHERE algorithm_version = ?1 AND settings_hash = ?2",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash],
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
                   ?3
            FROM anchor_index_v3
            WHERE algorithm_version = ?1 AND settings_hash = ?2
            GROUP BY algorithm_version, settings_hash, modality, bucket",
            params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), settings_hash, now],
        )
        .map(|_| ())
        .map_err(|error| format!("failed refreshing media-match v3 anchor stats: {error}"))?;
    clear_anchor_stats_v3_dirty(connection, settings_hash)
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
    clear_all_anchor_stats_v3_dirty(connection)
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
        audio: None,
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
        (offset_ms + (MEDIA_MATCH_V3_OFFSET_BIN_MS / 2)) / MEDIA_MATCH_V3_OFFSET_BIN_MS
    } else {
        (offset_ms - (MEDIA_MATCH_V3_OFFSET_BIN_MS / 2)) / MEDIA_MATCH_V3_OFFSET_BIN_MS
    }
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
        if time - previous > MEDIA_MATCH_V3_RETRIEVAL_GAP_MS {
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
