use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION, MediaMatchToolPaths,
    V3Tuning, current_v3_tuning, decide_media_match, fingerprint_media_file_with_report,
    identity::normalize_media_path,
    settings::{MediaExtractionSettings, media_extraction_settings_hash},
    summarize_instrumented_record_v3_diagnostics, summarize_record_v3_diagnostics,
    types::{
        MatchClassV3, MediaFingerprintRecord, MediaMatchAutoplayPolicy, MediaMatchSettings,
        MediaMatchTier,
    },
    v3_index::{
        MediaMatchV3RetrievalStats, MediaMatchV3RetrievalStrategy, MediaMatchV3RetrievedCandidate,
        MediaMatchV3SqliteSizeReport, load_media_match_v3_record_for_path,
        media_match_v3_anchor_candidate_details_with_strategy, media_match_v3_sqlite_size_report,
        open_media_match_v3_index, refresh_dirty_anchor_stats_v3_if_needed,
        save_media_match_v3_record_with_stats,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MediaMatchV3DiagnosticIndexMode {
    #[default]
    SampledFast,
}

impl MediaMatchV3DiagnosticIndexMode {
    pub fn label(self) -> &'static str {
        "sampled-fast"
    }

    pub fn settings(self) -> MediaExtractionSettings {
        match self {
            Self::SampledFast => MediaExtractionSettings::sampled_fast_audio_index_v3(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifest {
    #[serde(default = "default_manifest_profile")]
    pub profile: String,
    #[serde(default)]
    pub base_dir: Option<String>,
    #[serde(default)]
    pub cases: Vec<MediaMatchV3DiagnosticManifestCase>,
}

fn default_manifest_profile() -> String {
    "audio-constellation-v3".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifestCase {
    #[serde(rename = "caseName", alias = "name")]
    pub case_name: String,
    #[serde(rename = "queryPath", alias = "query")]
    pub query_path: String,
    #[serde(default)]
    pub candidates: Vec<MediaMatchV3DiagnosticManifestCandidate>,
    #[serde(default)]
    pub hard_negatives: Vec<MediaMatchV3DiagnosticHardNegative>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifestCandidate {
    #[serde(default)]
    pub id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub expected_class: Option<String>,
    #[serde(default)]
    pub minimum_tier: Option<String>,
    #[serde(default)]
    pub autoplay_eligible: Option<bool>,
    #[serde(default)]
    pub must_be_retrieved: bool,
    #[serde(default)]
    pub expected_retrieved: Option<bool>,
    #[serde(default)]
    pub max_retrieval_rank: Option<usize>,
    #[serde(default)]
    pub skip_decision_expectation: bool,
    #[serde(default)]
    pub max_promotion_rank: Option<usize>,
    #[serde(default)]
    pub expect_within_promotion_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticHardNegative {
    #[serde(default)]
    pub id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub must_not_be_top_rank: bool,
    #[serde(default)]
    pub must_not_beat_candidate_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticExpectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_class: Option<MatchClassV3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_tier: Option<MediaMatchTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay_eligible: Option<bool>,
    #[serde(default)]
    pub must_be_retrieved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_retrieved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retrieval_rank: Option<usize>,
    #[serde(default)]
    pub skip_decision_expectation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_promotion_rank: Option<usize>,
    #[serde(default)]
    pub expect_within_promotion_budget: bool,
}

impl Default for MediaMatchV3DiagnosticExpectation {
    fn default() -> Self {
        Self {
            expected_class: None,
            minimum_tier: None,
            autoplay_eligible: None,
            must_be_retrieved: true,
            expected_retrieved: None,
            max_retrieval_rank: Some(1),
            skip_decision_expectation: false,
            max_promotion_rank: Some(3),
            expect_within_promotion_budget: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3DiagnosticRunOptions {
    pub manifest_dir: PathBuf,
    pub cache_root: PathBuf,
    pub cache_retained: bool,
    pub refresh_cache: bool,
    pub index_mode: MediaMatchV3DiagnosticIndexMode,
    pub retrieval_benchmark_only: bool,
    pub retrieval_strategy: MediaMatchV3RetrievalStrategy,
    pub tools: MediaMatchToolPaths,
    pub generated_at_unix_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ResolvedManifest {
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
    pub cases: Vec<MediaMatchV3ResolvedManifestCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ResolvedManifestCase {
    pub case_name: String,
    pub query_path: String,
    pub candidates: Vec<MediaMatchV3ResolvedManifestCandidate>,
    #[serde(default)]
    pub hard_negatives: Vec<MediaMatchV3ResolvedManifestHardNegative>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ResolvedManifestCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    pub expectation: MediaMatchV3DiagnosticExpectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ResolvedManifestHardNegative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub must_not_be_top_rank: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_not_beat_candidate_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticReport {
    pub schema_version: u32,
    pub algorithm_version: u32,
    pub fingerprint_cache_version: u32,
    pub profile: String,
    pub index_mode: String,
    pub sampled_policy_production_compatible: bool,
    pub settings_hash: String,
    pub tuning: V3Tuning,
    pub cache_root: String,
    pub cache_retained: bool,
    pub generated_at_unix_millis: u64,
    pub retrieval_benchmark_only: bool,
    pub cases: Vec<MediaMatchV3DiagnosticCaseReport>,
    pub summary: MediaMatchV3DiagnosticSummaryReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite_size: Option<MediaMatchV3SqliteSizeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCaseReport {
    pub case_name: String,
    pub query: MediaMatchV3DiagnosticFingerprintReport,
    pub retrieval: MediaMatchV3DiagnosticRetrievalReport,
    pub candidates: Vec<MediaMatchV3DiagnosticCandidateReport>,
    #[serde(default)]
    pub hard_negatives: Vec<MediaMatchV3DiagnosticHardNegativeReport>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticFingerprintReport {
    pub path: String,
    pub source: String,
    pub diagnostics: crate::MediaMatchV3DiagnosticSummary,
    pub sqlite_save_millis: u128,
    pub blob_encode_millis: u128,
    pub index_insert_millis: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalReport {
    pub elapsed_millis: u128,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub candidates_returned: i64,
    pub retrieval_strategy: String,
    pub stats: MediaMatchV3RetrievalStats,
    pub candidates: Vec<MediaMatchV3DiagnosticRetrievalCandidateReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalCandidateReport {
    pub path: String,
    pub rank: usize,
    pub total_score: i64,
    pub robust_score: f64,
    pub best_offset_bin_ms: i64,
    pub best_offset_score: i64,
    pub second_offset_score: i64,
    pub audio_hits: i64,
    pub approximate_span_ms: i64,
    pub distinct_query_regions: i64,
    pub distinct_candidate_regions: i64,
    pub body_region_count: i64,
    pub edge_region_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_ratio_to_next: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_duration_ms: Option<i64>,
    pub duration_compatibility: String,
    pub short_clip_penalty_applied: bool,
}

impl From<&MediaMatchV3RetrievedCandidate> for MediaMatchV3DiagnosticRetrievalCandidateReport {
    fn from(candidate: &MediaMatchV3RetrievedCandidate) -> Self {
        Self {
            path: candidate.normalized_path.clone(),
            rank: candidate.rank,
            total_score: candidate.total_score,
            robust_score: candidate.robust_score,
            best_offset_bin_ms: candidate.best_offset_bin_ms,
            best_offset_score: candidate.best_offset_score,
            second_offset_score: candidate.second_offset_score,
            audio_hits: candidate.audio_hits,
            approximate_span_ms: candidate.approximate_span_ms,
            distinct_query_regions: candidate.distinct_query_regions,
            distinct_candidate_regions: candidate.distinct_candidate_regions,
            body_region_count: candidate.body_region_count,
            edge_region_count: candidate.edge_region_count,
            score_ratio_to_next: candidate.score_ratio_to_next,
            query_duration_ms: candidate.query_duration_ms,
            candidate_duration_ms: candidate.candidate_duration_ms,
            duration_compatibility: candidate.duration_compatibility.clone(),
            short_clip_penalty_applied: candidate.short_clip_penalty_applied,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCandidateReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    pub fingerprint: MediaMatchV3DiagnosticFingerprintReport,
    pub retrieved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_rank: Option<usize>,
    pub strict_rank1_passed: bool,
    pub within_promotion_budget: bool,
    pub production_retrieval_passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<MediaMatchV3DiagnosticDecisionReport>,
    pub expectation: MediaMatchV3DiagnosticExpectation,
    pub expectation_passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticDecisionReport {
    pub tier: MediaMatchTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<MatchClassV3>,
    pub autoplay_eligible: bool,
    pub explanation: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticHardNegativeReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    pub retrieved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_rank: Option<usize>,
    pub must_not_be_top_rank: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_not_beat_candidate_id: Option<String>,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummaryReport {
    pub case_count: usize,
    pub pair_count: usize,
    pub hard_negative_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub strict_rank1_passed: usize,
    pub within_promotion_budget: usize,
    pub production_retrieval_passed: usize,
    pub retrieval_misses: usize,
    pub unique_fresh_fingerprint_count: usize,
    pub unique_memory_cache_fingerprint_count: usize,
    pub unique_sqlite_cache_fingerprint_count: usize,
    pub fresh_fingerprint_report_count: usize,
    pub memory_cache_fingerprint_report_count: usize,
    pub sqlite_cache_fingerprint_report_count: usize,
    pub total_extraction_millis: u128,
    pub total_sqlite_save_millis: u128,
    pub total_sqlite_index_insert_millis: u128,
    pub total_retrieval_millis: u128,
    pub total_raw_hit_rows_processed: i64,
    pub total_audio_blob_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_anchor_index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_fingerprint_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_bytes_per_fingerprint: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_bytes_per_anchor: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SourceIndexReport {
    pub unique_fresh_fingerprint_count: usize,
    pub unique_memory_cache_fingerprint_count: usize,
    pub unique_sqlite_cache_fingerprint_count: usize,
    pub fresh_fingerprint_report_count: usize,
    pub memory_cache_fingerprint_report_count: usize,
    pub sqlite_cache_fingerprint_report_count: usize,
}

#[derive(Debug, Clone)]
struct CachedFingerprint {
    record: MediaFingerprintRecord,
    report: MediaMatchV3DiagnosticFingerprintReport,
}

pub fn media_match_v3_diagnostic_manifest_from_json(
    text: &str,
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    let manifest = serde_json::from_str::<MediaMatchV3DiagnosticManifest>(text)
        .map_err(|error| format!("failed parsing V3 diagnostic manifest: {error}"))?;
    validate_media_match_v3_diagnostic_manifest(&manifest)?;
    Ok(manifest)
}

pub fn media_match_v3_diagnostic_manifest_report_json(
    report: &MediaMatchV3DiagnosticReport,
) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed serializing V3 diagnostic report: {error}"))
}

pub fn validate_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
) -> Result<(), String> {
    if manifest.profile.trim().is_empty() {
        return Err("manifest profile must be non-empty".to_owned());
    }
    if manifest.profile != "audio-constellation-v3" {
        return Err(format!(
            "unsupported V3 diagnostic profile '{}'; only audio-constellation-v3 is supported",
            manifest.profile
        ));
    }
    if manifest.cases.is_empty() {
        return Err("manifest must contain at least one case".to_owned());
    }
    for case in &manifest.cases {
        if case.case_name.trim().is_empty() {
            return Err("case name must be non-empty".to_owned());
        }
        if case.query_path.trim().is_empty() {
            return Err(format!(
                "case '{}' query path must be non-empty",
                case.case_name
            ));
        }
        let mut ids = BTreeSet::new();
        let mut no_id_paths = BTreeSet::new();
        for candidate in &case.candidates {
            if candidate.path.trim().is_empty() {
                return Err(format!(
                    "case '{}' candidate path must be non-empty",
                    case.case_name
                ));
            }
            if let Some(id) = candidate.id.as_deref() {
                let id = id.trim();
                if id.is_empty() {
                    return Err(format!(
                        "case '{}' candidate id must be non-empty",
                        case.case_name
                    ));
                }
                if !ids.insert(id.to_owned()) {
                    return Err(format!(
                        "case '{}' has duplicate candidate id '{}'",
                        case.case_name, id
                    ));
                }
            } else if !no_id_paths.insert(candidate.path.clone()) {
                return Err(format!(
                    "case '{}' has duplicate no-id candidate path '{}'",
                    case.case_name, candidate.path
                ));
            }
        }
        for negative in &case.hard_negatives {
            if negative.path.trim().is_empty() {
                return Err(format!(
                    "case '{}' hard negative path must be non-empty",
                    case.case_name
                ));
            }
            if let Some(id) = negative.id.as_deref()
                && id.trim().is_empty()
            {
                return Err(format!(
                    "case '{}' hard negative id must be non-empty",
                    case.case_name
                ));
            }
        }
    }
    Ok(())
}

pub fn resolve_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    manifest_dir: &Path,
) -> Result<MediaMatchV3ResolvedManifest, String> {
    validate_media_match_v3_diagnostic_manifest(manifest)?;
    let base_dir = manifest
        .base_dir
        .as_deref()
        .map(|base| resolve_manifest_path(manifest_dir, base))
        .unwrap_or_else(|| manifest_dir.to_path_buf());
    let cases = manifest
        .cases
        .iter()
        .map(|case| {
            let candidates = case
                .candidates
                .iter()
                .map(|candidate| {
                    Ok(MediaMatchV3ResolvedManifestCandidate {
                        id: candidate.id.as_ref().map(|id| id.trim().to_owned()),
                        path: resolve_manifest_path(&base_dir, &candidate.path)
                            .to_string_lossy()
                            .to_string(),
                        expectation: expectation_from_candidate(candidate)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let hard_negatives = case
                .hard_negatives
                .iter()
                .map(|negative| MediaMatchV3ResolvedManifestHardNegative {
                    id: negative.id.as_ref().map(|id| id.trim().to_owned()),
                    path: resolve_manifest_path(&base_dir, &negative.path)
                        .to_string_lossy()
                        .to_string(),
                    must_not_be_top_rank: negative.must_not_be_top_rank,
                    must_not_beat_candidate_id: negative.must_not_beat_candidate_id.clone(),
                })
                .collect();
            Ok(MediaMatchV3ResolvedManifestCase {
                case_name: case.case_name.trim().to_owned(),
                query_path: resolve_manifest_path(&base_dir, &case.query_path)
                    .to_string_lossy()
                    .to_string(),
                candidates,
                hard_negatives,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(MediaMatchV3ResolvedManifest {
        profile: manifest.profile.clone(),
        base_dir: Some(base_dir.to_string_lossy().to_string()),
        cases,
    })
}

pub fn run_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    options: &MediaMatchV3DiagnosticRunOptions,
) -> Result<MediaMatchV3DiagnosticReport, String> {
    let resolved = resolve_media_match_v3_diagnostic_manifest(manifest, &options.manifest_dir)?;
    let settings = options.index_mode.settings();
    if !settings.sampled_audio_policy.is_production_compatible() {
        return Err("diagnostic settings are not production-compatible sampled-fast".to_owned());
    }
    let connection = open_media_match_v3_index(&options.cache_root)?;
    let mut memory = HashMap::<String, CachedFingerprint>::new();
    let mut source_counts = SourceCounts::default();
    let mut case_reports = Vec::new();
    let mut summary = MediaMatchV3DiagnosticSummaryReport {
        case_count: resolved.cases.len(),
        ..MediaMatchV3DiagnosticSummaryReport::default()
    };
    let now = options
        .generated_at_unix_millis
        .unwrap_or_else(current_unix_millis) as i64;

    for case in &resolved.cases {
        let query = fingerprint_cached(
            &connection,
            &case.query_path,
            &settings,
            options,
            &mut memory,
            &mut source_counts,
        )?;
        let mut candidate_entries = Vec::new();
        for candidate in &case.candidates {
            let fingerprint = fingerprint_cached(
                &connection,
                &candidate.path,
                &settings,
                options,
                &mut memory,
                &mut source_counts,
            )?;
            candidate_entries.push((candidate, fingerprint));
        }
        let mut hard_negative_entries = Vec::new();
        for negative in &case.hard_negatives {
            let fingerprint = fingerprint_cached(
                &connection,
                &negative.path,
                &settings,
                options,
                &mut memory,
                &mut source_counts,
            )?;
            hard_negative_entries.push((negative, fingerprint));
        }

        let settings_hash = media_extraction_settings_hash(&settings);
        refresh_dirty_anchor_stats_v3_if_needed(&connection, &settings_hash, now)?;
        let retrieval_started_at = Instant::now();
        let (retrieved, retrieval_stats) = media_match_v3_anchor_candidate_details_with_strategy(
            &connection,
            &query.record,
            now,
            options.retrieval_strategy,
        )?;
        let retrieval_elapsed_ms = retrieval_started_at.elapsed().as_millis();
        summary.total_retrieval_millis += retrieval_elapsed_ms;
        summary.total_raw_hit_rows_processed += retrieval_stats.raw_hit_rows_processed;
        let retrieval_report = MediaMatchV3DiagnosticRetrievalReport {
            elapsed_millis: retrieval_elapsed_ms,
            raw_hit_rows_processed: retrieval_stats.raw_hit_rows_processed,
            candidates_scored: retrieval_stats.candidates_scored,
            candidates_returned: retrieval_stats.candidates_returned,
            retrieval_strategy: retrieval_stats.retrieval_strategy.clone(),
            stats: retrieval_stats,
            candidates: retrieved.iter().map(Into::into).collect(),
        };
        let rank_by_path = retrieved
            .iter()
            .map(|candidate| (candidate.normalized_path.clone(), candidate.rank))
            .collect::<BTreeMap<_, _>>();

        let mut candidate_reports = Vec::new();
        let autoplay_settings = autoplay_diagnostic_settings();
        for (candidate, fingerprint) in candidate_entries {
            summary.pair_count += 1;
            let rank = rank_by_path
                .get(&fingerprint.record.identity.normalized_path)
                .copied();
            let decision = (!options.retrieval_benchmark_only).then(|| {
                let decision =
                    decide_media_match(&query.record, &fingerprint.record, &autoplay_settings);
                let autoplay_eligible = decision.same_media_for_autoplay(&autoplay_settings);
                MediaMatchV3DiagnosticDecisionReport {
                    tier: decision.tier,
                    class: decision.evidence.v3_class,
                    autoplay_eligible,
                    explanation: decision.explanation,
                    notes: decision.evidence.notes,
                }
            });
            let evaluation = evaluate_expectation(&candidate.expectation, rank, decision.as_ref());
            if evaluation.passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            if rank.is_none_or(|rank| rank != 1) {
                summary.retrieval_misses += usize::from(candidate.expectation.must_be_retrieved);
            }
            if rank == Some(1) {
                summary.strict_rank1_passed += 1;
            }
            if evaluation.within_promotion_budget {
                summary.within_promotion_budget += 1;
            }
            if evaluation.production_retrieval_passed {
                summary.production_retrieval_passed += 1;
            }
            summary.total_sqlite_save_millis += fingerprint.report.sqlite_save_millis;
            summary.total_sqlite_index_insert_millis += fingerprint.report.index_insert_millis;
            summary.total_audio_blob_bytes += fingerprint.report.diagnostics.audio_blob_bytes;
            if fingerprint.report.source == "fresh" {
                summary.total_extraction_millis += fingerprint
                    .report
                    .diagnostics
                    .extraction_total_millis
                    .unwrap_or_default();
            }
            candidate_reports.push(MediaMatchV3DiagnosticCandidateReport {
                id: candidate.id.clone(),
                path: candidate.path.clone(),
                fingerprint: fingerprint.report.clone(),
                retrieved: rank.is_some(),
                retrieval_rank: rank,
                strict_rank1_passed: rank == Some(1),
                within_promotion_budget: evaluation.within_promotion_budget,
                production_retrieval_passed: evaluation.production_retrieval_passed,
                decision,
                expectation: candidate.expectation.clone(),
                expectation_passed: evaluation.passed,
                failure_reason: evaluation.failure_reason,
            });
        }

        let hard_negative_reports = hard_negative_entries
            .into_iter()
            .map(|(negative, fingerprint)| {
                let rank = rank_by_path
                    .get(&fingerprint.record.identity.normalized_path)
                    .copied();
                let candidate_to_beat_rank = negative
                    .must_not_beat_candidate_id
                    .as_deref()
                    .and_then(|id| {
                        candidate_reports
                            .iter()
                            .find(|candidate| candidate.id.as_deref() == Some(id))
                            .and_then(|candidate| candidate.retrieval_rank)
                    });
                let mut passed = true;
                let mut failure_reason = None;
                if negative.must_not_be_top_rank && rank == Some(1) {
                    passed = false;
                    failure_reason = Some("hard negative was top-ranked".to_owned());
                }
                if let (Some(rank), Some(expected_rank)) = (rank, candidate_to_beat_rank)
                    && rank < expected_rank
                {
                    passed = false;
                    failure_reason = Some("hard negative outranked expected candidate".to_owned());
                }
                MediaMatchV3DiagnosticHardNegativeReport {
                    id: negative.id.clone(),
                    path: negative.path.clone(),
                    retrieved: rank.is_some(),
                    retrieval_rank: rank,
                    must_not_be_top_rank: negative.must_not_be_top_rank,
                    must_not_beat_candidate_id: negative.must_not_beat_candidate_id.clone(),
                    passed,
                    failure_reason,
                }
            })
            .collect::<Vec<_>>();
        summary.hard_negative_count += hard_negative_reports.len();
        let hard_negatives_passed = hard_negative_reports.iter().all(|negative| negative.passed);
        if !hard_negatives_passed {
            summary.failed += hard_negative_reports
                .iter()
                .filter(|negative| !negative.passed)
                .count();
        }
        let passed = candidate_reports
            .iter()
            .all(|candidate| candidate.expectation_passed)
            && hard_negatives_passed;
        case_reports.push(MediaMatchV3DiagnosticCaseReport {
            case_name: case.case_name.clone(),
            query: query.report.clone(),
            retrieval: retrieval_report,
            candidates: candidate_reports,
            hard_negatives: hard_negative_reports,
            passed,
        });
    }

    summary.unique_fresh_fingerprint_count = source_counts.unique_fresh;
    summary.unique_memory_cache_fingerprint_count = source_counts.unique_memory;
    summary.unique_sqlite_cache_fingerprint_count = source_counts.unique_sqlite;
    summary.fresh_fingerprint_report_count = source_counts.report_fresh;
    summary.memory_cache_fingerprint_report_count = source_counts.report_memory;
    summary.sqlite_cache_fingerprint_report_count = source_counts.report_sqlite;

    let sqlite_size = media_match_v3_sqlite_size_report(&options.cache_root, &connection).ok();
    if let Some(size) = &sqlite_size {
        summary.db_total_bytes = Some(size.total_bytes);
        summary.db_anchor_index_bytes = Some(size.anchor_index_bytes);
        summary.db_fingerprint_bytes = Some(size.fingerprint_blob_bytes);
        summary.db_index_bytes = Some(size.db_index_bytes);
        summary.db_bytes_per_fingerprint = Some(size.db_bytes_per_fingerprint);
        summary.db_bytes_per_anchor = Some(size.db_bytes_per_anchor);
    }

    Ok(MediaMatchV3DiagnosticReport {
        schema_version: 3,
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        fingerprint_cache_version: MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
        profile: settings.profile.label().to_owned(),
        index_mode: options.index_mode.label().to_owned(),
        sampled_policy_production_compatible: settings
            .sampled_audio_policy
            .is_production_compatible(),
        settings_hash: hex_hash(&media_extraction_settings_hash(&settings)),
        tuning: current_v3_tuning(),
        cache_root: options.cache_root.to_string_lossy().to_string(),
        cache_retained: options.cache_retained,
        generated_at_unix_millis: options
            .generated_at_unix_millis
            .unwrap_or_else(current_unix_millis),
        retrieval_benchmark_only: options.retrieval_benchmark_only,
        cases: case_reports,
        summary,
        sqlite_size,
    })
}

fn fingerprint_cached(
    connection: &rusqlite::Connection,
    path: &str,
    settings: &MediaExtractionSettings,
    options: &MediaMatchV3DiagnosticRunOptions,
    memory: &mut HashMap<String, CachedFingerprint>,
    source_counts: &mut SourceCounts,
) -> Result<CachedFingerprint, String> {
    let normalized = normalize_media_path(path);
    if let Some(existing) = memory.get(&normalized) {
        source_counts.report_memory += 1;
        let mut cached = existing.clone();
        cached.report.source = "memory-cache".to_owned();
        cached.report.diagnostics = summarize_record_v3_diagnostics(&cached.record);
        return Ok(cached);
    }

    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed reading media file metadata '{}': {error}", path))?;
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let size_bytes = metadata.len();
    if !options.refresh_cache
        && let Some(record) = load_media_match_v3_record_for_path(
            connection,
            &normalized,
            settings,
            modified_unix_millis,
            size_bytes,
        )?
    {
        source_counts.unique_sqlite += 1;
        source_counts.report_sqlite += 1;
        let report = MediaMatchV3DiagnosticFingerprintReport {
            path: normalized.clone(),
            source: "sqlite-cache".to_owned(),
            diagnostics: summarize_record_v3_diagnostics(&record),
            sqlite_save_millis: 0,
            blob_encode_millis: 0,
            index_insert_millis: 0,
        };
        let cached = CachedFingerprint { record, report };
        memory.insert(normalized, cached.clone());
        return Ok(cached);
    }

    let fingerprint = fingerprint_media_file_with_report(path, &options.tools, settings, None)
        .map_err(|error| error.to_string())?;
    let now = options
        .generated_at_unix_millis
        .unwrap_or_else(current_unix_millis) as i64;
    let save_stats = save_media_match_v3_record_with_stats(connection, &fingerprint.record, now)?;
    source_counts.unique_fresh += 1;
    source_counts.report_fresh += 1;
    let report = MediaMatchV3DiagnosticFingerprintReport {
        path: fingerprint.record.identity.normalized_path.clone(),
        source: "fresh".to_owned(),
        diagnostics: summarize_instrumented_record_v3_diagnostics(&fingerprint),
        sqlite_save_millis: save_stats.sqlite_save_millis,
        blob_encode_millis: save_stats.blob_encode_millis,
        index_insert_millis: save_stats.index_insert_millis,
    };
    let cached = CachedFingerprint {
        record: fingerprint.record,
        report,
    };
    memory.insert(normalized, cached.clone());
    Ok(cached)
}

#[derive(Debug, Clone, Copy, Default)]
struct SourceCounts {
    unique_fresh: usize,
    unique_memory: usize,
    unique_sqlite: usize,
    report_fresh: usize,
    report_memory: usize,
    report_sqlite: usize,
}

#[derive(Debug, Clone)]
struct ExpectationEvaluation {
    passed: bool,
    within_promotion_budget: bool,
    production_retrieval_passed: bool,
    failure_reason: Option<String>,
}

fn evaluate_expectation(
    expectation: &MediaMatchV3DiagnosticExpectation,
    rank: Option<usize>,
    decision: Option<&MediaMatchV3DiagnosticDecisionReport>,
) -> ExpectationEvaluation {
    let mut failure_reason = None;
    let expected_retrieved = expectation
        .expected_retrieved
        .unwrap_or(expectation.must_be_retrieved);
    if expected_retrieved && rank.is_none() {
        failure_reason = Some("candidate was not retrieved".to_owned());
    }
    if !expected_retrieved && rank.is_some() {
        failure_reason = Some("candidate was retrieved unexpectedly".to_owned());
    }
    if let (Some(max_rank), Some(rank)) = (expectation.max_retrieval_rank, rank)
        && rank > max_rank
    {
        failure_reason = Some(format!(
            "candidate rank {rank} exceeded max rank {max_rank}"
        ));
    }
    let promotion_rank = expectation.max_promotion_rank.unwrap_or(3);
    let within_promotion_budget = rank.is_some_and(|rank| rank <= promotion_rank);
    let production_retrieval_passed = if expectation.expect_within_promotion_budget {
        within_promotion_budget
    } else {
        rank.is_some()
    };
    if expectation.expect_within_promotion_budget && !within_promotion_budget {
        failure_reason = Some(format!(
            "candidate was not within promotion budget {promotion_rank}"
        ));
    }
    if !expectation.skip_decision_expectation {
        if let Some(expected_class) = expectation.expected_class {
            match decision.and_then(|decision| decision.class) {
                Some(actual) if actual == expected_class => {}
                Some(actual) => {
                    failure_reason =
                        Some(format!("expected class {expected_class:?}, got {actual:?}"));
                }
                None => failure_reason = Some("decision was not evaluated".to_owned()),
            }
        }
        if let Some(minimum_tier) = expectation.minimum_tier {
            match decision.map(|decision| decision.tier) {
                Some(actual) if tier_rank(actual) >= tier_rank(minimum_tier) => {}
                Some(actual) => {
                    failure_reason = Some(format!(
                        "expected tier at least {minimum_tier:?}, got {actual:?}"
                    ));
                }
                None => failure_reason = Some("decision was not evaluated".to_owned()),
            }
        }
        if let Some(expected_autoplay) = expectation.autoplay_eligible {
            match decision.map(|decision| decision.autoplay_eligible) {
                Some(actual) if actual == expected_autoplay => {}
                Some(actual) => {
                    failure_reason = Some(format!(
                        "expected autoplayEligible={expected_autoplay}, got {actual}"
                    ));
                }
                None => failure_reason = Some("decision was not evaluated".to_owned()),
            }
        }
    }
    ExpectationEvaluation {
        passed: failure_reason.is_none(),
        within_promotion_budget,
        production_retrieval_passed,
        failure_reason,
    }
}

fn expectation_from_candidate(
    candidate: &MediaMatchV3DiagnosticManifestCandidate,
) -> Result<MediaMatchV3DiagnosticExpectation, String> {
    Ok(MediaMatchV3DiagnosticExpectation {
        expected_class: candidate
            .expected_class
            .as_deref()
            .map(parse_match_class)
            .transpose()?,
        minimum_tier: candidate
            .minimum_tier
            .as_deref()
            .map(parse_media_match_tier)
            .transpose()?,
        autoplay_eligible: candidate.autoplay_eligible,
        must_be_retrieved: candidate.must_be_retrieved
            || candidate.expected_retrieved.unwrap_or(true),
        expected_retrieved: candidate.expected_retrieved,
        max_retrieval_rank: candidate.max_retrieval_rank.or(Some(1)),
        skip_decision_expectation: candidate.skip_decision_expectation,
        max_promotion_rank: candidate.max_promotion_rank.or(Some(3)),
        expect_within_promotion_budget: candidate.expect_within_promotion_budget,
    })
}

fn parse_match_class(value: &str) -> Result<MatchClassV3, String> {
    match normalized_token(value).as_str() {
        "samecutstrong" => Ok(MatchClassV3::SameCutStrong),
        "samecutprobable" => Ok(MatchClassV3::SameCutProbable),
        "samemediadifferentcut" => Ok(MatchClassV3::SameMediaDifferentCut),
        "partialoverlap" => Ok(MatchClassV3::PartialOverlap),
        "sharedintrooutroonly" => Ok(MatchClassV3::SharedIntroOutroOnly),
        "reject" => Ok(MatchClassV3::Reject),
        "unknown" => Ok(MatchClassV3::Unknown),
        _ => Err(format!("unsupported expectedClass '{value}'")),
    }
}

fn parse_media_match_tier(value: &str) -> Result<MediaMatchTier, String> {
    match normalized_token(value).as_str() {
        "exact" => Ok(MediaMatchTier::Exact),
        "strong" => Ok(MediaMatchTier::Strong),
        "probable" => Ok(MediaMatchTier::Probable),
        "weak" => Ok(MediaMatchTier::Weak),
        "reject" => Ok(MediaMatchTier::Reject),
        "unknown" => Ok(MediaMatchTier::Unknown),
        _ => Err(format!("unsupported minimumTier '{value}'")),
    }
}

fn normalized_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn tier_rank(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Exact => 5,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Unknown => 1,
        MediaMatchTier::Reject => 0,
    }
}

fn autoplay_diagnostic_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    }
}

fn resolve_manifest_path(base_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn hex_hash(hash: &[u8; 32]) -> String {
    hash.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_validation_rejects_blank_candidate_id() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                case_name: "case".to_owned(),
                query_path: "query.mkv".to_owned(),
                candidates: vec![MediaMatchV3DiagnosticManifestCandidate {
                    id: Some(" ".to_owned()),
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    autoplay_eligible: None,
                    must_be_retrieved: true,
                    expected_retrieved: None,
                    max_retrieval_rank: None,
                    skip_decision_expectation: true,
                    max_promotion_rank: None,
                    expect_within_promotion_budget: false,
                }],
                hard_negatives: Vec::new(),
            }],
        };

        assert!(validate_media_match_v3_diagnostic_manifest(&manifest).is_err());
    }

    #[test]
    fn production_sampled_fast_index_mode_is_fixed() {
        let settings = MediaMatchV3DiagnosticIndexMode::SampledFast.settings();

        assert_eq!(settings.profile.label(), "audio-constellation-v3");
        assert_eq!(settings.audio_index_mode.label(), "sampled-fast");
        assert!(settings.sampled_audio_policy.is_production_compatible());
    }
}
