use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    InstrumentedMediaFingerprint, MEDIA_MATCH_ANCHOR_VERSION, MatchClassV3,
    MediaExtractionSettings, MediaFingerprintBlobV3, MediaMatchAutoplayPolicy, MediaMatchDecision,
    MediaMatchSettings, MediaMatchTier, MediaMatchToolPaths, MediaMatchV3DiagnosticSummary,
    V3Tuning, audio_index_landmarks_v3_from_record, current_v3_tuning, decide_media_match,
    encode_media_fingerprint_blob_v3, fingerprint_media_file_with_report,
    media_extraction_settings_hash, media_fingerprint_blob_v3_from_record, normalize_media_path,
    summarize_decision_v3_diagnostics, summarize_instrumented_record_v3_diagnostics,
    validate_video_landmarks_v3, video_index_landmarks_v3_from_record,
};

const DIAGNOSTIC_SQLITE_SCHEMA_VERSION: i64 = 3;
const DIAGNOSTIC_INDEX_FILE: &str = "index-v3.sqlite3";
const DIAGNOSTIC_MODALITY_AUDIO: i64 = 1;
const DIAGNOSTIC_MODALITY_VIDEO: i64 = 2;
const DIAGNOSTIC_PREFILTER_LIMIT: usize = 24;
const DIAGNOSTIC_OFFSET_BIN_MS: i64 = 1_000;
const DIAGNOSTIC_RETRIEVAL_REGION_MS: i64 = 60_000;
const DIAGNOSTIC_RETRIEVAL_GAP_MS: i64 = 120_000;
const DIAGNOSTIC_COMMON_BUCKET_MIN_SKIP_DF: i64 = 256;
const DIAGNOSTIC_COMMON_BUCKET_FILE_DIVISOR: i64 = 20;
const DIAGNOSTIC_ANCHOR_STATS_DIRTY_PREFIX: &str = "anchor_stats_v3_dirty:";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifest {
    #[serde(default = "default_diagnostic_profile")]
    pub profile: String,
    #[serde(default)]
    pub base_dir: Option<String>,
    pub cases: Vec<MediaMatchV3DiagnosticManifestCase>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifestCase {
    pub name: String,
    pub query: String,
    pub candidates: Vec<MediaMatchV3DiagnosticExpectation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticExpectation {
    pub path: String,
    pub expected_class: Option<String>,
    pub minimum_tier: Option<String>,
    pub expected_offset_ms: Option<i64>,
    pub max_offset_error_ms: Option<i64>,
    pub autoplay_eligible: Option<bool>,
    #[serde(default)]
    pub must_be_retrieved: bool,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3DiagnosticRunOptions {
    pub manifest_dir: PathBuf,
    pub cache_root: PathBuf,
    pub tools: MediaMatchToolPaths,
    pub generated_at_unix_millis: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifest {
    pub profile: String,
    pub cases: Vec<MediaMatchV3ResolvedManifestCase>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestCase {
    pub name: String,
    pub query: PathBuf,
    pub candidates: Vec<MediaMatchV3ResolvedManifestCandidate>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestCandidate {
    pub path: PathBuf,
    pub expectation: MediaMatchV3DiagnosticExpectation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticReport {
    pub algorithm_version: u32,
    pub profile: String,
    pub settings_hash: String,
    pub tuning: V3Tuning,
    pub generated_at_unix_millis: u64,
    pub cases: Vec<MediaMatchV3DiagnosticCaseReport>,
    pub summary: MediaMatchV3DiagnosticSummaryReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCaseReport {
    pub name: String,
    pub query: MediaMatchV3DiagnosticFingerprintReport,
    pub retrieval: MediaMatchV3DiagnosticRetrievalReport,
    pub candidates: Vec<MediaMatchV3DiagnosticCandidateReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticFingerprintReport {
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCandidateReport {
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
    pub retrieved: bool,
    pub retrieval_rank: Option<usize>,
    pub decision: MediaMatchV3DiagnosticDecisionReport,
    pub expectation: Option<MediaMatchV3DiagnosticExpectation>,
    pub passed: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticDecisionReport {
    pub tier: String,
    pub class: Option<String>,
    pub explanation: String,
    pub offset_seconds: Option<f64>,
    pub scale_ppm: Option<i32>,
    pub segment_count: usize,
    pub total_aligned_span_ms: u32,
    pub largest_gap_ms: u32,
    pub edge_only: bool,
    pub audio_video_conflict: bool,
    pub piecewise_pair_count: Option<usize>,
    pub piecewise_hypothesis_count: Option<usize>,
    pub piecewise_fit_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalReport {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
    pub retrieved_candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummaryReport {
    pub case_count: usize,
    pub pair_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_extraction_millis: u128,
    pub total_audio_blob_bytes: usize,
    pub total_video_blob_bytes: usize,
    pub total_raw_hit_rows_processed: i64,
    pub total_retrieval_millis: u128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaMatchV3RetrievalStats {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
}

#[derive(Debug, Clone)]
struct CachedFingerprint {
    fingerprint: InstrumentedMediaFingerprint,
    source: &'static str,
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

pub fn media_match_v3_diagnostic_manifest_from_json(
    manifest_json: &str,
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    serde_json::from_str(manifest_json)
        .map_err(|error| format!("failed parsing media-match V3 diagnostic manifest: {error}"))
}

pub fn media_match_v3_diagnostic_manifest_report_json(
    manifest_json: &str,
    options: MediaMatchV3DiagnosticRunOptions,
) -> Result<String, String> {
    let manifest = media_match_v3_diagnostic_manifest_from_json(manifest_json)?;
    let report = run_media_match_v3_diagnostic_manifest(&manifest, options)?;
    serde_json::to_string_pretty(&report)
        .map_err(|error| format!("failed serializing media-match V3 diagnostic report: {error}"))
}

pub fn run_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    options: MediaMatchV3DiagnosticRunOptions,
) -> Result<MediaMatchV3DiagnosticReport, String> {
    let settings = diagnostic_settings_for_profile(&manifest.profile)?;
    let settings_hash = media_extraction_settings_hash(&settings);
    let resolved = resolve_media_match_v3_diagnostic_manifest(manifest, &options.manifest_dir)?;
    let connection = open_diagnostic_sqlite_index(&options.cache_root)?;
    let autoplay_settings = diagnostic_decision_settings();
    let mut cache = BTreeMap::<(String, [u8; 32]), InstrumentedMediaFingerprint>::new();
    let mut cases = Vec::new();
    let mut summary = MediaMatchV3DiagnosticSummaryReport {
        case_count: resolved.cases.len(),
        ..MediaMatchV3DiagnosticSummaryReport::default()
    };

    for case in &resolved.cases {
        let query = fingerprint_cached(&mut cache, &case.query, &options.tools, &settings)?;
        save_diagnostic_v3_record(&connection, &query.fingerprint, None)?;

        let mut candidate_records = Vec::new();
        for candidate in &case.candidates {
            let fingerprint =
                fingerprint_cached(&mut cache, &candidate.path, &options.tools, &settings)?;
            save_diagnostic_v3_record(&connection, &fingerprint.fingerprint, None)?;
            candidate_records.push((candidate, fingerprint));
        }

        let (retrieved_candidates, retrieval_stats) =
            media_match_v3_anchor_candidate_paths_with_stats(
                &options.cache_root,
                &query.fingerprint.record.identity.normalized_path,
                &settings,
            )?;
        let retrieval_report = MediaMatchV3DiagnosticRetrievalReport::from_stats(
            retrieval_stats,
            retrieved_candidates,
        );
        summary.total_raw_hit_rows_processed += retrieval_report.raw_hit_rows_processed;
        summary.total_retrieval_millis += retrieval_report.retrieval_elapsed_ms;

        let query_report = MediaMatchV3DiagnosticFingerprintReport {
            path: query.fingerprint.record.identity.normalized_path.clone(),
            diagnostics: summarize_instrumented_record_v3_diagnostics(&query.fingerprint),
            source: query.source.to_owned(),
        };
        let mut reports = Vec::new();
        for (candidate, fingerprint) in candidate_records {
            let decision = decide_media_match(
                &query.fingerprint.record,
                &fingerprint.fingerprint.record,
                &autoplay_settings,
            );
            let normalized_candidate = &fingerprint.fingerprint.record.identity.normalized_path;
            let retrieval_rank = retrieval_report
                .retrieved_candidates
                .iter()
                .position(|path| path == normalized_candidate)
                .map(|index| index + 1);
            let retrieved = retrieval_rank.is_some();
            let failures = evaluate_diagnostic_expectation(
                &decision,
                &candidate.expectation,
                &autoplay_settings,
                retrieved,
            );
            let passed = failures.is_empty();
            summary.pair_count += 1;
            if passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            reports.push(MediaMatchV3DiagnosticCandidateReport {
                path: normalized_candidate.clone(),
                diagnostics: summarize_instrumented_record_v3_diagnostics(&fingerprint.fingerprint),
                source: fingerprint.source.to_owned(),
                retrieved,
                retrieval_rank,
                decision: MediaMatchV3DiagnosticDecisionReport::from_decision(&decision),
                expectation: Some(candidate.expectation.clone()),
                passed,
                failure_reason: (!failures.is_empty()).then(|| failures.join("; ")),
            });
        }
        cases.push(MediaMatchV3DiagnosticCaseReport {
            name: case.name.clone(),
            query: query_report,
            retrieval: retrieval_report,
            candidates: reports,
        });
    }

    for fingerprint in cache.values() {
        let diagnostics = summarize_instrumented_record_v3_diagnostics(fingerprint);
        summary.total_extraction_millis += fingerprint.report.timings.total_millis;
        summary.total_audio_blob_bytes += diagnostics.audio_blob_bytes;
        summary.total_video_blob_bytes += diagnostics.video_blob_bytes;
    }

    Ok(MediaMatchV3DiagnosticReport {
        algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: settings.profile.label().to_owned(),
        settings_hash: bytes_to_lower_hex(&settings_hash),
        tuning: current_v3_tuning(),
        generated_at_unix_millis: options
            .generated_at_unix_millis
            .unwrap_or_else(current_unix_millis),
        cases,
        summary,
    })
}

pub fn resolve_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    manifest_dir: &Path,
) -> Result<MediaMatchV3ResolvedManifest, String> {
    let base = manifest
        .base_dir
        .as_deref()
        .map(|base_dir| resolve_manifest_path(manifest_dir, manifest_dir, base_dir))
        .unwrap_or_else(|| manifest_dir.to_path_buf());
    let cases = manifest
        .cases
        .iter()
        .map(|case| {
            Ok(MediaMatchV3ResolvedManifestCase {
                name: case.name.clone(),
                query: resolve_manifest_path(manifest_dir, &base, &case.query),
                candidates: case
                    .candidates
                    .iter()
                    .map(|candidate| MediaMatchV3ResolvedManifestCandidate {
                        path: resolve_manifest_path(manifest_dir, &base, &candidate.path),
                        expectation: candidate.clone(),
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(MediaMatchV3ResolvedManifest {
        profile: manifest.profile.clone(),
        cases,
    })
}

pub fn media_match_v3_anchor_candidate_paths_with_stats(
    root: &Path,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<(Vec<String>, MediaMatchV3RetrievalStats), String> {
    let started_at = Instant::now();
    let mut stats = MediaMatchV3RetrievalStats::default();
    if !diagnostic_index_path(root).exists() {
        return Ok((Vec::new(), stats));
    }
    let connection = open_diagnostic_sqlite_index(root)?;
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    refresh_dirty_anchor_stats_v3_if_needed(&connection, &settings_hash)?;
    let Some(current_file_id) = connection
        .query_row(
            "SELECT file_id FROM media_files_v3 WHERE normalized_path = ?1",
            [normalized_current_path],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("failed reading media-match v3 file id: {error}"))?
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
    let common_bucket_threshold = DIAGNOSTIC_COMMON_BUCKET_MIN_SKIP_DF
        .max(indexed_file_count / DIAGNOSTIC_COMMON_BUCKET_FILE_DIVISOR);
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
            DIAGNOSTIC_MODALITY_AUDIO => score.audio_hits += 1,
            DIAGNOSTIC_MODALITY_VIDEO => score.video_hits += 1,
            _ => {}
        }
        let offset_score = score.offset_bins.entry(offset_bin).or_default();
        offset_score.weighted_score += weighted_score;
        offset_score
            .query_regions
            .insert(query_t_ms / DIAGNOSTIC_RETRIEVAL_REGION_MS);
        offset_score
            .candidate_regions
            .insert(candidate_t_ms / DIAGNOSTIC_RETRIEVAL_REGION_MS);
        offset_score.query_times.push(query_t_ms);
        offset_score.candidate_times.push(candidate_t_ms);
        match modality {
            DIAGNOSTIC_MODALITY_AUDIO => offset_score.audio_hits += 1,
            DIAGNOSTIC_MODALITY_VIDEO => offset_score.video_hits += 1,
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
    for score in ranked.into_iter().take(DIAGNOSTIC_PREFILTER_LIMIT) {
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

impl MediaMatchV3DiagnosticRetrievalReport {
    fn from_stats(stats: MediaMatchV3RetrievalStats, retrieved_candidates: Vec<String>) -> Self {
        Self {
            query_buckets_total: stats.query_buckets_total,
            query_buckets_skipped_common: stats.query_buckets_skipped_common,
            raw_hit_rows_processed: stats.raw_hit_rows_processed,
            candidates_scored: stats.candidates_scored,
            retrieval_elapsed_ms: stats.retrieval_elapsed_ms,
            retrieved_candidates,
        }
    }
}

impl MediaMatchV3DiagnosticDecisionReport {
    fn from_decision(decision: &MediaMatchDecision) -> Self {
        let map = decision.evidence.timeline_map_v3.as_ref();
        let summary = summarize_decision_v3_diagnostics(decision);
        Self {
            tier: format!("{:?}", decision.tier),
            class: decision.evidence.v3_class.map(|class| format!("{class:?}")),
            explanation: decision.explanation.clone(),
            offset_seconds: decision
                .evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.offset_seconds),
            scale_ppm: decision
                .evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.scale_ppm),
            segment_count: map.map(|map| map.segments.len()).unwrap_or_default(),
            total_aligned_span_ms: map.map(|map| map.total_aligned_span_ms).unwrap_or_default(),
            largest_gap_ms: map.map(|map| map.largest_gap_ms).unwrap_or_default(),
            edge_only: map.map(|map| map.edge_only).unwrap_or(false),
            audio_video_conflict: map.map(|map| map.audio_video_conflict).unwrap_or(false),
            piecewise_pair_count: summary.piecewise_pair_count,
            piecewise_hypothesis_count: summary.piecewise_hypothesis_count,
            piecewise_fit_millis: summary.piecewise_fit_millis,
        }
    }
}

fn fingerprint_cached(
    cache: &mut BTreeMap<(String, [u8; 32]), InstrumentedMediaFingerprint>,
    path: &Path,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
) -> Result<CachedFingerprint, String> {
    let normalized_path = normalize_media_path(path);
    let cache_key = (normalized_path, media_extraction_settings_hash(settings));
    if let Some(fingerprint) = cache.get(&cache_key) {
        return Ok(CachedFingerprint {
            fingerprint: fingerprint.clone(),
            source: "cache",
        });
    }
    let fingerprint = fingerprint_media_file_with_report(path, tools, settings, None)
        .map_err(|error| format!("failed fingerprinting '{}': {error}", path.display()))?;
    cache.insert(cache_key, fingerprint.clone());
    Ok(CachedFingerprint {
        fingerprint,
        source: "fresh",
    })
}

fn evaluate_diagnostic_expectation(
    decision: &MediaMatchDecision,
    expected: &MediaMatchV3DiagnosticExpectation,
    autoplay_settings: &MediaMatchSettings,
    retrieved: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected_class) = expected.expected_class.as_deref() {
        match parse_match_class(expected_class) {
            Some(class) if Some(class) == decision.evidence.v3_class => {}
            Some(_) => failures.push(format!(
                "expected class {expected_class}, got {}",
                decision
                    .evidence
                    .v3_class
                    .map(|class| format!("{class:?}"))
                    .unwrap_or_else(|| "None".to_owned())
            )),
            None => failures.push(format!("unknown expected class {expected_class}")),
        }
    }
    if let Some(minimum_tier) = expected.minimum_tier.as_deref() {
        match parse_tier(minimum_tier) {
            Some(tier) if tier_score(decision.tier) >= tier_score(tier) => {}
            Some(_) => failures.push(format!(
                "expected tier at least {minimum_tier}, got {:?}",
                decision.tier
            )),
            None => failures.push(format!("unknown expected tier {minimum_tier}")),
        }
    }
    if let Some(max_offset_error_ms) = expected.max_offset_error_ms {
        match decision.evidence.alignment.as_ref() {
            Some(alignment) => {
                let actual_offset_ms = (alignment.offset_seconds * 1000.0).round() as i64;
                let expected_offset_ms = expected.expected_offset_ms.unwrap_or(0);
                let offset_error_ms = (actual_offset_ms - expected_offset_ms).abs();
                if offset_error_ms > max_offset_error_ms {
                    if expected.expected_offset_ms.is_some() {
                        failures.push(format!(
                            "expected offset {expected_offset_ms}ms +/- {max_offset_error_ms}ms, got {actual_offset_ms}ms (error {offset_error_ms}ms)"
                        ));
                    } else {
                        failures.push(format!(
                            "expected absolute offset <= {max_offset_error_ms}ms, got {actual_offset_ms}ms"
                        ));
                    }
                }
            }
            None => failures.push("expected offset evidence, got none".to_owned()),
        }
    }
    if let Some(expected_autoplay) = expected.autoplay_eligible {
        let actual = decision.same_media_for_autoplay(autoplay_settings);
        if actual != expected_autoplay {
            failures.push(format!(
                "expected autoplayEligible={expected_autoplay}, got {actual}"
            ));
        }
    }
    if expected.must_be_retrieved && !retrieved {
        failures.push("expected candidate to be retrieved, but it was absent".to_owned());
    }
    failures
}

fn save_diagnostic_v3_record(
    connection: &Connection,
    fingerprint: &InstrumentedMediaFingerprint,
    error: Option<&str>,
) -> Result<(), String> {
    let record = &fingerprint.record;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match v3 save transaction: {error}"))?;
    let now = current_unix_millis() as i64;
    let duration_ms = record
        .duration_seconds
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| (value * 1000.0).round().min(f64::from(u32::MAX)) as i64);
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
        transaction
            .execute("DELETE FROM anchor_index_v3 WHERE file_id = ?1", [file_id])
            .map_err(|error| format!("failed deleting stale media-match v3 anchors: {error}"))?;
        transaction
            .execute("DELETE FROM fingerprints_v3 WHERE file_id = ?1", [file_id])
            .map_err(|error| {
                format!("failed deleting stale media-match v3 fingerprints: {error}")
            })?;
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
            DIAGNOSTIC_MODALITY_AUDIO,
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
            DIAGNOSTIC_MODALITY_VIDEO,
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

fn open_diagnostic_sqlite_index(root: &Path) -> Result<Connection, String> {
    let index_path = diagnostic_index_path(root);
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed creating media-match diagnostic cache directory '{}': {error}",
                parent.display()
            )
        })?;
    }
    let connection = Connection::open(&index_path).map_err(|error| {
        format!(
            "failed opening media-match diagnostic SQLite index '{}': {error}",
            index_path.display()
        )
    })?;
    initialize_diagnostic_sqlite_index(&connection)?;
    Ok(connection)
}

fn initialize_diagnostic_sqlite_index(connection: &Connection) -> Result<(), String> {
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
        .map_err(|error| {
            format!("failed initializing media-match diagnostic SQLite index: {error}")
        })?;
    connection
        .pragma_update(None, "user_version", DIAGNOSTIC_SQLITE_SCHEMA_VERSION)
        .map_err(|error| {
            format!("failed setting media-match diagnostic schema version: {error}")
        })?;
    Ok(())
}

fn refresh_dirty_anchor_stats_v3_if_needed(
    connection: &Connection,
    settings_hash: &[u8],
) -> Result<(), String> {
    let key = anchor_stats_v3_dirty_key(settings_hash);
    let dirty = connection
        .query_row(
            "SELECT 1 FROM metadata WHERE key = ?1",
            [key],
            |_row| Ok(()),
        )
        .optional()
        .map_err(|error| format!("failed reading media-match v3 dirty stats marker: {error}"))?
        .is_some();
    if dirty {
        refresh_anchor_stats_v3(connection, settings_hash, current_unix_millis() as i64)?;
    }
    Ok(())
}

fn refresh_anchor_stats_v3(
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

fn mark_anchor_stats_v3_dirty(connection: &Connection, settings_hash: &[u8]) -> Result<(), String> {
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

fn clear_anchor_stats_v3_dirty(
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

fn anchor_stats_v3_dirty_key(settings_hash: &[u8]) -> String {
    format!(
        "{DIAGNOSTIC_ANCHOR_STATS_DIRTY_PREFIX}{}",
        bytes_to_lower_hex(settings_hash)
    )
}

fn diagnostic_index_path(root: &Path) -> PathBuf {
    root.join("cache")
        .join("media-match")
        .join(DIAGNOSTIC_INDEX_FILE)
}

fn resolve_manifest_path(_manifest_dir: &Path, base_dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn diagnostic_settings_for_profile(profile: &str) -> Result<MediaExtractionSettings, String> {
    match normalized_label(profile).as_str() {
        "audioconstellationv3" => Ok(MediaExtractionSettings::audio_constellation_v3()),
        "combinedv3" => Ok(MediaExtractionSettings::combined_v3()),
        _ => Err(format!(
            "unsupported profile '{profile}', expected audio-constellation-v3 or combined-v3"
        )),
    }
}

fn diagnostic_decision_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    }
}

fn parse_match_class(label: &str) -> Option<MatchClassV3> {
    match normalized_label(label).as_str() {
        "samecutstrong" => Some(MatchClassV3::SameCutStrong),
        "samecutprobable" => Some(MatchClassV3::SameCutProbable),
        "samemediadifferentcut" => Some(MatchClassV3::SameMediaDifferentCut),
        "samevideodifferentaudio" => Some(MatchClassV3::SameVideoDifferentAudio),
        "sameaudiodifferentvideo" => Some(MatchClassV3::SameAudioDifferentVideo),
        "partialoverlap" => Some(MatchClassV3::PartialOverlap),
        "sharedintrooutroonly" => Some(MatchClassV3::SharedIntroOutroOnly),
        "reject" => Some(MatchClassV3::Reject),
        "unknown" => Some(MatchClassV3::Unknown),
        _ => None,
    }
}

fn parse_tier(label: &str) -> Option<MediaMatchTier> {
    match normalized_label(label).as_str() {
        "exact" => Some(MediaMatchTier::Exact),
        "strong" => Some(MediaMatchTier::Strong),
        "probable" => Some(MediaMatchTier::Probable),
        "weak" => Some(MediaMatchTier::Weak),
        "reject" => Some(MediaMatchTier::Reject),
        "unknown" => Some(MediaMatchTier::Unknown),
        _ => None,
    }
}

fn tier_score(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Unknown => 0,
        MediaMatchTier::Reject => 1,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Exact => 5,
    }
}

fn normalized_label(label: &str) -> String {
    label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
        (offset_ms + (DIAGNOSTIC_OFFSET_BIN_MS / 2)) / DIAGNOSTIC_OFFSET_BIN_MS
    } else {
        (offset_ms - (DIAGNOSTIC_OFFSET_BIN_MS / 2)) / DIAGNOSTIC_OFFSET_BIN_MS
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
        if time - previous > DIAGNOSTIC_RETRIEVAL_GAP_MS {
            best = best.max(previous - segment_start);
            segment_start = time;
        }
        previous = time;
    }
    best.max(previous - segment_start)
}

fn current_unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
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

fn default_diagnostic_profile() -> String {
    "audio-constellation-v3".to_owned()
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;
    use crate::{MediaMatchEvidence, MediaTimelineAlignment, MetadataMatchEvidence};

    #[test]
    fn manifest_parsing_accepts_canonical_shape() {
        let manifest = media_match_v3_diagnostic_manifest_from_json(
            r#"{
              "profile": "combined-v3",
              "baseDir": "media",
              "cases": [{
                "name": "same-episode",
                "query": "query.mkv",
                "candidates": [{
                  "path": "candidate.mkv",
                  "expectedClass": "SameCutStrong",
                  "minimumTier": "Strong",
                  "expectedOffsetMs": 5000,
                  "maxOffsetErrorMs": 1000,
                  "autoplayEligible": true,
                  "mustBeRetrieved": true
                }]
              }]
            }"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.profile, "combined-v3");
        assert_eq!(manifest.base_dir.as_deref(), Some("media"));
        assert_eq!(manifest.cases[0].candidates[0].path, "candidate.mkv");
        assert_eq!(
            manifest.cases[0].candidates[0].expected_offset_ms,
            Some(5000)
        );
        assert!(manifest.cases[0].candidates[0].must_be_retrieved);
        serde_json::to_string(&manifest).expect("canonical manifest should serialize");
    }

    #[test]
    fn manifest_paths_resolve_relative_to_manifest_dir() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "relative".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![MediaMatchV3DiagnosticExpectation {
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                }],
            }],
        };
        let manifest_dir = PathBuf::from("C:/manifest-root");

        let resolved =
            resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir).expect("resolve");

        assert_eq!(resolved.cases[0].query, manifest_dir.join("query.mkv"));
        assert_eq!(
            resolved.cases[0].candidates[0].path,
            manifest_dir.join("candidate.mkv")
        );
    }

    #[test]
    fn manifest_base_dir_resolves_relative_to_manifest_dir() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: Some("media".to_owned()),
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "base".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![MediaMatchV3DiagnosticExpectation {
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                }],
            }],
        };
        let manifest_dir = PathBuf::from("C:/manifest-root");

        let resolved =
            resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir).expect("resolve");

        assert_eq!(
            resolved.cases[0].query,
            manifest_dir.join("media/query.mkv")
        );
        assert_eq!(
            resolved.cases[0].candidates[0].path,
            manifest_dir.join("media/candidate.mkv")
        );
    }

    #[test]
    fn manifest_absolute_paths_are_unchanged() {
        let absolute = env::current_dir()
            .expect("current dir")
            .join("absolute-query.mkv");
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: Some("media".to_owned()),
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "absolute".to_owned(),
                query: absolute.to_string_lossy().to_string(),
                candidates: Vec::new(),
            }],
        };

        let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, Path::new("unused"))
            .expect("resolve");

        assert_eq!(resolved.cases[0].query, absolute);
    }

    #[test]
    fn expectation_evaluation_covers_offsets_autoplay_and_retrieval() {
        let settings = diagnostic_decision_settings();
        let expected = MediaMatchV3DiagnosticExpectation {
            path: "candidate.mkv".to_owned(),
            expected_class: Some("SameCutStrong".to_owned()),
            minimum_tier: Some("Strong".to_owned()),
            expected_offset_ms: Some(5000),
            max_offset_error_ms: Some(1000),
            autoplay_eligible: Some(true),
            must_be_retrieved: true,
        };

        let pass = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(5200),
            &expected,
            &settings,
            true,
        );
        let fail = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(8000),
            &expected,
            &settings,
            false,
        );

        assert!(pass.is_empty(), "{pass:?}");
        assert!(
            fail.iter()
                .any(|failure| failure.contains("expected offset 5000ms")),
            "{fail:?}"
        );
        assert!(
            fail.iter()
                .any(|failure| failure.contains("expected candidate to be retrieved")),
            "{fail:?}"
        );
    }

    #[test]
    fn expectation_offset_without_expected_value_keeps_absolute_behavior() {
        let settings = diagnostic_decision_settings();
        let expected = MediaMatchV3DiagnosticExpectation {
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
            must_be_retrieved: false,
        };

        let failures = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(800),
            &expected,
            &settings,
            false,
        );

        assert!(failures.is_empty(), "{failures:?}");
    }

    fn decision_with_offset_ms(offset_ms: i64) -> MediaMatchDecision {
        MediaMatchDecision {
            tier: MediaMatchTier::Strong,
            evidence: MediaMatchEvidence {
                metadata: MetadataMatchEvidence::default(),
                alignment: Some(MediaTimelineAlignment {
                    offset_seconds: offset_ms as f64 / 1000.0,
                    scale_ppm: 1_000_000,
                    drift_ratio: 0.0,
                    aligned_pairs: 12,
                    aligned_audio_anchors: 12,
                    aligned_video_anchors: 0,
                    aligned_span_seconds: 300.0,
                    second_best_offset_margin: 1.0,
                    first_query_second: 0.0,
                    last_query_second: 300.0,
                    first_candidate_second: offset_ms as f64 / 1000.0,
                    last_candidate_second: 300.0 + offset_ms as f64 / 1000.0,
                }),
                v3_class: Some(MatchClassV3::SameCutStrong),
                ..MediaMatchEvidence::default()
            },
            explanation: "strong".to_owned(),
        }
    }
}
