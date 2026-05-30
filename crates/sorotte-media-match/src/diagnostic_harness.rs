use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::extraction::{
    fingerprint_media_file_with_report_and_options, media_source_path_info,
    probe_audio_packet_positions_for_sampled_windows,
};
use crate::{
    InstrumentedMediaFingerprint, MEDIA_MATCH_ANCHOR_VERSION, MatchClassV3, MediaAudioIndexMode,
    MediaAudioStreamMetrics, MediaDenseAudioProfile, MediaExtractionSettings,
    MediaFingerprintExtractionOptions, MediaMatchAutoplayPolicy, MediaMatchDecision,
    MediaMatchSettings, MediaMatchTier, MediaMatchToolPaths, MediaMatchV3DiagnosticSummary,
    MediaMatchV3RetrievalStats, MediaMatchV3RetrievalStrategy, MediaMatchV3RetrievedCandidate,
    MediaMatchV3SaveStats, MediaMatchV3SqliteSizeReport, MediaSampledAudioPolicy,
    MediaSampledAudioSourceStrategy, V3Tuning, current_v3_tuning, decide_media_match,
    fingerprint_media_file_with_report, load_media_match_v3_record_for_path,
    media_extraction_settings_hash, media_match_v3_anchor_candidate_details_with_strategy,
    media_match_v3_incompatible_record_count_for_path, media_match_v3_sqlite_size_report,
    normalize_media_path, open_media_match_v3_index, save_media_match_v3_record_with_stats,
    summarize_decision_v3_diagnostics, summarize_instrumented_record_v3_diagnostics,
    summarize_record_v3_diagnostics,
};

#[cfg(test)]
use crate::save_media_match_v3_record;

const FINGERPRINT_SOURCE_FRESH: &str = "fresh";
const FINGERPRINT_SOURCE_MEMORY_CACHE: &str = "memory-cache";
const FINGERPRINT_SOURCE_SQLITE_CACHE: &str = "sqlite-cache";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifest {
    #[serde(default = "default_diagnostic_profile")]
    pub profile: String,
    #[serde(default)]
    pub base_dir: Option<String>,
    pub cases: Vec<MediaMatchV3DiagnosticManifestCase>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifestCase {
    pub name: String,
    pub query: String,
    pub candidates: Vec<MediaMatchV3DiagnosticExpectation>,
    #[serde(default)]
    pub hard_negatives: Vec<MediaMatchV3DiagnosticHardNegative>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticExpectation {
    #[serde(default)]
    pub id: Option<String>,
    pub path: String,
    pub expected_class: Option<String>,
    pub minimum_tier: Option<String>,
    pub expected_offset_ms: Option<i64>,
    pub max_offset_error_ms: Option<i64>,
    pub autoplay_eligible: Option<bool>,
    #[serde(default)]
    pub must_be_retrieved: bool,
    #[serde(default)]
    pub expected_retrieved: Option<bool>,
    #[serde(default)]
    pub max_retrieval_rank: Option<usize>,
    #[serde(default)]
    pub max_promotion_rank: Option<usize>,
    #[serde(default)]
    pub expect_within_promotion_budget: bool,
    #[serde(default)]
    pub skip_decision_expectation: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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

#[derive(Debug, Clone)]
pub struct MediaMatchV3DiagnosticRunOptions {
    pub manifest_dir: PathBuf,
    pub cache_root: PathBuf,
    pub cache_retained: bool,
    pub refresh_cache: bool,
    pub index_mode: MediaMatchV3DiagnosticIndexMode,
    pub dense_audio_profile: MediaDenseAudioProfile,
    pub max_full_promotions_per_query: usize,
    pub promote_expected_candidates: bool,
    pub retrieval_benchmark_only: bool,
    pub retrieval_strategy: MediaMatchV3RetrievalStrategy,
    pub sampled_fast_global_workers: Option<usize>,
    pub sampled_fast_per_local_source_workers: Option<usize>,
    pub sampled_fast_per_network_source_workers: Option<usize>,
    pub sampled_fast_per_removable_source_workers: Option<usize>,
    pub probe_audio_packets: bool,
    pub sampled_audio_source: MediaSampledAudioSourceStrategy,
    pub experimental_sampled_audio_source: bool,
    pub sampled_pcm_cache_root: Option<PathBuf>,
    pub tools: MediaMatchToolPaths,
    pub generated_at_unix_millis: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaMatchV3DiagnosticIndexMode {
    #[default]
    Full,
    SparseFull,
    SampledFast,
    SampledNormal,
    SampledThenFull,
    Production,
}

impl MediaMatchV3DiagnosticIndexMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::SparseFull => "sparse-full",
            Self::SampledFast => "sampled-fast",
            Self::SampledNormal => "sampled-normal",
            Self::SampledThenFull => "sampled-then-full",
            Self::Production => "production",
        }
    }
}

fn sampled_audio_source_is_production_compatible(source: MediaSampledAudioSourceStrategy) -> bool {
    matches!(source, MediaSampledAudioSourceStrategy::Current)
}

fn sampled_audio_source_equivalence_report(
    source: MediaSampledAudioSourceStrategy,
) -> MediaSampledAudioSourceEquivalenceReport {
    if sampled_audio_source_is_production_compatible(source) {
        MediaSampledAudioSourceEquivalenceReport {
            pcm_byte_exact_match: Some(true),
            pcm_rms_delta: Some(0.0),
            pcm_max_abs_delta: Some(0),
            landmark_overlap_ratio: Some(1.0),
            exact_landmark_overlap_count: None,
            retrieval_rank_delta: Some(0),
            retrieval_score_delta: Some(0),
            expected_candidate_still_retrieved: None,
            hard_negatives_still_pass: None,
        }
    } else {
        MediaSampledAudioSourceEquivalenceReport::default()
    }
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
    pub hard_negatives: Vec<MediaMatchV3ResolvedManifestHardNegative>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestCandidate {
    pub path: PathBuf,
    pub expectation: MediaMatchV3DiagnosticExpectation,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestHardNegative {
    pub path: PathBuf,
    pub expectation: MediaMatchV3DiagnosticHardNegative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticReport {
    pub algorithm_version: u32,
    pub fingerprint_cache_version: u32,
    pub profile: String,
    pub index_mode: String,
    pub dense_audio_profile: String,
    pub settings_hash: String,
    pub tuning: V3Tuning,
    pub cache_root: String,
    pub cache_retained: bool,
    pub generated_at_unix_millis: u64,
    pub cases: Vec<MediaMatchV3DiagnosticCaseReport>,
    pub summary: MediaMatchV3DiagnosticSummaryReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite_size: Option<MediaMatchV3SqliteSizeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCaseReport {
    pub name: String,
    pub query: MediaMatchV3DiagnosticFingerprintReport,
    pub retrieval: MediaMatchV3DiagnosticRetrievalReport,
    pub candidates: Vec<MediaMatchV3DiagnosticCandidateReport>,
    #[serde(default)]
    pub hard_negatives: Vec<MediaMatchV3DiagnosticHardNegativeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticFingerprintReport {
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
    pub sqlite_save_millis: u128,
    pub blob_encode_millis: u128,
    pub index_insert_millis: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCandidateReport {
    pub candidate_id: Option<String>,
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
    pub sqlite_save_millis: u128,
    pub blob_encode_millis: u128,
    pub index_insert_millis: u128,
    pub retrieved: bool,
    pub retrieval_rank: Option<usize>,
    #[serde(default)]
    pub sampled_retrieval_rank: Option<usize>,
    #[serde(default)]
    pub final_verified_rank: Option<usize>,
    #[serde(default)]
    pub within_promotion_budget: bool,
    #[serde(default)]
    pub strict_rank1_passed: bool,
    #[serde(default)]
    pub production_retrieval_passed: bool,
    #[serde(default)]
    pub promotion_budget_exhausted: bool,
    #[serde(default)]
    pub promoted_candidate_ranks: Vec<usize>,
    #[serde(default)]
    pub first_strong_candidate_rank: Option<usize>,
    pub promotion_reason: Option<String>,
    pub full_promotion_millis: u128,
    pub decision: MediaMatchV3DiagnosticDecisionReport,
    pub expectation: Option<MediaMatchV3DiagnosticExpectation>,
    pub passed: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticHardNegativeReport {
    pub candidate_id: Option<String>,
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
    pub sqlite_save_millis: u128,
    pub blob_encode_millis: u128,
    pub index_insert_millis: u128,
    pub retrieved: bool,
    pub retrieval_rank: Option<usize>,
    pub must_not_be_top_rank: bool,
    pub must_not_beat_candidate_id: Option<String>,
    pub passed: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticDecisionReport {
    pub tier: String,
    pub class: Option<String>,
    pub explanation: String,
    pub autoplay_eligible: bool,
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
    pub decision_pair_collection_millis: Option<u64>,
    pub fast_audio_verifier_millis: Option<u64>,
    pub global_fit_millis: Option<u64>,
    pub offset_histogram_millis: Option<u64>,
    pub fast_global_fit_millis: Option<u64>,
    pub broad_global_fit_millis: Option<u64>,
    pub global_fit_candidate_count: Option<usize>,
    pub global_fit_inlier_count: Option<usize>,
    pub global_fit_fallback_used: Option<bool>,
    pub timeline_map_millis: Option<u64>,
    pub evidence_formatting_millis: Option<u64>,
    pub total_decision_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalReport {
    #[serde(default)]
    pub retrieval_strategy: String,
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
    #[serde(default)]
    pub retrieval_measured_stage_millis: u128,
    #[serde(default)]
    pub retrieval_unaccounted_millis: u128,
    #[serde(default)]
    pub stats_dirty_check_millis: u128,
    #[serde(default)]
    pub stats_refresh_millis: u128,
    #[serde(default)]
    pub query_anchor_load_millis: u128,
    #[serde(default)]
    pub common_bucket_filter_millis: u128,
    #[serde(default)]
    pub sql_hit_fetch_millis: u128,
    #[serde(default)]
    pub temp_table_create_millis: u128,
    #[serde(default)]
    pub temp_table_insert_millis: u128,
    #[serde(default)]
    pub temp_table_index_millis: u128,
    #[serde(default)]
    pub temp_table_drop_millis: u128,
    #[serde(default)]
    pub sql_prepare_millis: u128,
    #[serde(default)]
    pub sql_execute_millis: u128,
    #[serde(default)]
    pub rust_aggregation_millis: u128,
    #[serde(default)]
    pub candidate_metadata_load_millis: u128,
    #[serde(default)]
    pub robust_rerank_millis: u128,
    #[serde(default)]
    pub candidate_sort_millis: u128,
    #[serde(default)]
    pub retrieved_candidate_detail_build_millis: u128,
    #[serde(default)]
    pub retrieved_path_load_millis: u128,
    #[serde(default)]
    pub report_candidate_attach_millis: u128,
    #[serde(default)]
    pub path_lookup_millis: u128,
    #[serde(default)]
    pub explain_query_plan_millis: u128,
    #[serde(default)]
    pub stats_refresh_ran: bool,
    #[serde(default)]
    pub stats_buckets_refreshed: i64,
    #[serde(default)]
    pub stats_anchor_rows_scanned: i64,
    #[serde(default)]
    pub anchor_stats_dirty_before_run: bool,
    #[serde(default)]
    pub anchor_stats_refreshed: bool,
    #[serde(default)]
    pub anchor_stats_refresh_millis: u128,
    #[serde(default)]
    pub anchor_stats_dirty_after_run: bool,
    #[serde(default)]
    pub query_anchor_count: i64,
    #[serde(default)]
    pub query_buckets_after_common_skip: i64,
    #[serde(default)]
    pub sql_rows_returned: i64,
    #[serde(default)]
    pub candidates_aggregated: i64,
    #[serde(default)]
    pub candidates_returned: i64,
    pub retrieved_candidates: Vec<String>,
    #[serde(default)]
    pub retrieved_candidate_details: Vec<MediaMatchV3DiagnosticRetrievalCandidateReport>,
    pub correct_candidate_rank: Option<usize>,
    pub hard_negative_best_rank: Option<usize>,
    pub hard_negative_count_above_correct: usize,
    pub top1_is_expected: bool,
    pub top_k_expected_present: bool,
    pub retrieval_margin: Option<MediaMatchV3DiagnosticRetrievalMarginReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalCandidateReport {
    pub candidate_id: Option<String>,
    pub path: String,
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
    #[serde(default)]
    pub query_duration_ms: Option<i64>,
    #[serde(default)]
    pub candidate_duration_ms: Option<i64>,
    #[serde(default)]
    pub duration_compatibility: String,
    #[serde(default)]
    pub short_clip_penalty_applied: bool,
    #[serde(default)]
    pub robust_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalMarginReport {
    pub top1_score: Option<i64>,
    pub top2_score: Option<i64>,
    pub expected_score: Option<i64>,
    pub best_negative_score: Option<i64>,
    pub expected_best_offset_score: Option<i64>,
    pub best_negative_offset_score: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaSampledAudioSourceEquivalenceReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm_byte_exact_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm_rms_delta: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm_max_abs_delta: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmark_overlap_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_landmark_overlap_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_rank_delta: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_score_delta: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_candidate_still_retrieved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_negatives_still_pass: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummaryReport {
    pub case_count: usize,
    pub pair_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub hard_negative_count: usize,
    pub hard_negative_passed: usize,
    pub hard_negative_failed: usize,
    pub unique_fresh_fingerprint_count: usize,
    pub unique_memory_cache_fingerprint_count: usize,
    pub unique_sqlite_cache_fingerprint_count: usize,
    pub fresh_fingerprint_report_count: usize,
    pub memory_cache_fingerprint_report_count: usize,
    pub sqlite_cache_fingerprint_report_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_audio_policy: Option<MediaSampledAudioPolicy>,
    #[serde(default)]
    pub sampled_policy_production_compatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_audio_source_strategy: Option<String>,
    #[serde(default)]
    pub experimental_sampled_audio_source: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_policy_cache_recommendation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_audio_source_equivalence: Option<MediaSampledAudioSourceEquivalenceReport>,
    #[serde(default)]
    pub sqlite_cache_compatible_hit_count: usize,
    #[serde(default)]
    pub sqlite_cache_incompatible_miss_count: usize,
    #[serde(default)]
    pub sampled_policy_mismatch_count: usize,
    pub total_extraction_millis: u128,
    pub total_audio_blob_bytes: usize,
    pub total_video_blob_bytes: usize,
    pub total_raw_hit_rows_processed: i64,
    pub total_retrieval_millis: u128,
    pub run_wall_millis: u128,
    pub manifest_parse_millis: u128,
    pub cache_open_millis: u128,
    pub fingerprint_total_millis: u128,
    pub sqlite_load_millis: u128,
    pub sqlite_save_millis: u128,
    pub sqlite_index_insert_millis: u128,
    pub retrieval_total_millis: u128,
    #[serde(default)]
    pub per_query_retrieval_millis_p50: u128,
    #[serde(default)]
    pub per_query_retrieval_millis_p95: u128,
    #[serde(default)]
    pub per_query_retrieval_millis_p99: u128,
    #[serde(default)]
    pub per_query_retrieval_millis_max: u128,
    #[serde(default)]
    pub retrieval_unaccounted_millis_total: u128,
    #[serde(default)]
    pub retrieval_unaccounted_millis_p95: u128,
    #[serde(default)]
    pub sql_hit_fetch_millis_total: u128,
    #[serde(default)]
    pub rust_aggregation_millis_total: u128,
    #[serde(default)]
    pub candidate_metadata_load_millis_total: u128,
    #[serde(default)]
    pub robust_rerank_millis_total: u128,
    #[serde(default)]
    pub db_total_bytes: u64,
    #[serde(default)]
    pub db_anchor_index_bytes: u64,
    #[serde(default)]
    pub db_fingerprint_bytes: u64,
    #[serde(default)]
    pub db_stats_bytes: u64,
    #[serde(default)]
    pub db_index_bytes: u64,
    #[serde(default)]
    pub db_bytes_per_fingerprint: f64,
    #[serde(default)]
    pub db_bytes_per_anchor: f64,
    pub decision_total_millis: u128,
    pub report_serialize_millis: u128,
    pub sampled_fingerprint_count: usize,
    pub full_fingerprint_count: usize,
    pub candidates_promoted_to_full_verify: usize,
    pub full_promotion_millis: u128,
    pub full_promotion_cache_hits: usize,
    pub production_sampled_index_millis: u128,
    #[serde(default)]
    pub production_retrieval_millis: u128,
    pub production_full_promotion_millis: u128,
    pub production_total_millis: u128,
    pub sampled_indexed_file_count: usize,
    #[serde(default)]
    pub sampled_cache_hit_count: usize,
    pub full_promoted_file_count: usize,
    pub max_full_promotions_per_query: usize,
    #[serde(default)]
    pub sampled_fast_worker_count: usize,
    #[serde(default)]
    pub full_verify_worker_count: usize,
    #[serde(default)]
    pub files_per_minute: u64,
    #[serde(default)]
    pub files_skipped_from_cache: usize,
    #[serde(default)]
    pub files_failed: usize,
    #[serde(default)]
    pub extraction_worker_wall_millis: u128,
    #[serde(default)]
    pub extraction_queue_wait_millis: u128,
    #[serde(default)]
    pub sqlite_writer_millis: u128,
    #[serde(default)]
    pub sqlite_write_queue_depth_max: usize,
    #[serde(default)]
    pub producer_worker_count: usize,
    #[serde(default)]
    pub writer_thread_enabled: bool,
    #[serde(default)]
    pub result_queue_depth_max: usize,
    #[serde(default)]
    pub result_queue_wait_millis: u128,
    #[serde(default)]
    pub writer_idle_millis: u128,
    #[serde(default)]
    pub writer_busy_millis: u128,
    #[serde(default)]
    pub writer_batch_count: usize,
    #[serde(default)]
    pub writer_rows_inserted: usize,
    #[serde(default)]
    pub writer_records_inserted: usize,
    #[serde(default)]
    pub writer_records_per_batch: f64,
    #[serde(default)]
    pub writer_commit_millis: u128,
    #[serde(default)]
    pub extraction_worker_idle_millis: u128,
    #[serde(default)]
    pub extraction_worker_active_millis: u128,
    #[serde(default)]
    pub end_to_end_index_wall_millis: u128,
    #[serde(default)]
    pub fresh_fingerprint_millis_p50: u128,
    #[serde(default)]
    pub fresh_fingerprint_millis_p75: u128,
    #[serde(default)]
    pub fresh_fingerprint_millis_p95: u128,
    #[serde(default)]
    pub fresh_fingerprint_millis_p99: u128,
    #[serde(default)]
    pub fresh_fingerprint_millis_max: u128,
    #[serde(default)]
    pub fresh_fingerprint_ffmpeg_wall_millis_p50: u128,
    #[serde(default)]
    pub fresh_fingerprint_ffmpeg_wall_millis_p95: u128,
    #[serde(default)]
    pub fresh_fingerprint_analyzer_millis_p50: u128,
    #[serde(default)]
    pub fresh_fingerprint_analyzer_millis_p95: u128,
    #[serde(default)]
    pub fresh_sampled_audio_seconds_decoded_p50: u128,
    #[serde(default)]
    pub fresh_sampled_audio_seconds_decoded_p95: u128,
    #[serde(default)]
    pub fresh_sampled_audio_windows_decoded_p50: u128,
    #[serde(default)]
    pub fresh_sampled_audio_windows_decoded_p95: u128,
    #[serde(default)]
    pub fresh_fingerprint_queue_wait_millis_p50: u128,
    #[serde(default)]
    pub fresh_fingerprint_queue_wait_millis_p95: u128,
    #[serde(default)]
    pub slowest_fresh_fingerprints: Vec<MediaMatchV3SlowFingerprintReport>,
    #[serde(default)]
    pub source_index_reports: Vec<MediaMatchV3SourceIndexReport>,
    #[serde(default)]
    pub cancelled_file_count: usize,
    #[serde(default)]
    pub resumed_file_count: usize,
    #[serde(default)]
    pub production_passed: bool,
}

#[derive(Debug, Clone)]
struct CachedFingerprint {
    fingerprint: InstrumentedMediaFingerprint,
    source: &'static str,
    sqlite_load_millis: u128,
    save_stats: MediaMatchV3SaveStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SlowFingerprintReport {
    pub path: String,
    pub source_path_root: Option<String>,
    pub source_path_kind: Option<String>,
    pub source_volume_id: Option<String>,
    pub duration_ms: Option<u32>,
    pub size_bytes: u64,
    pub extraction_total_millis: u128,
    pub queue_wait_millis: u128,
    pub ffmpeg_process_wall_millis: Option<u128>,
    pub ffmpeg_input_read_bytes: Option<u64>,
    pub ffmpeg_input_read_ops: Option<u64>,
    pub ffmpeg_output_pcm_bytes: Option<u64>,
    pub read_amplification_ratio: Option<f64>,
    pub ffmpeg_invocation_count: Option<usize>,
    pub sampled_window_seek_millis: Option<u128>,
    pub sampled_window_decode_millis: Option<u128>,
    pub analyzer_millis: Option<u128>,
    pub sampled_audio_seconds_decoded: Option<u32>,
    pub sampled_audio_windows_decoded: Option<usize>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3SourceIndexReport {
    pub source_path_root: String,
    pub source_path_kind: String,
    pub worker_limit: usize,
    pub files_indexed: usize,
    pub files_failed: usize,
    pub files_per_minute: u64,
    pub ffmpeg_wall_millis_p50: u128,
    pub ffmpeg_wall_millis_p95: u128,
    pub extraction_millis_p50: u128,
    pub extraction_millis_p95: u128,
    pub source_read_bytes: u64,
    pub source_read_ops: u64,
    pub output_pcm_bytes: u64,
    pub read_amplification_ratio: Option<f64>,
    pub queue_depth_max: usize,
    pub concurrency_changes: Vec<String>,
}

#[derive(Debug, Clone)]
struct DiagnosticFingerprintOccurrence {
    occurrence_key: (usize, usize),
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct DiagnosticFreshFingerprintJob {
    path: PathBuf,
    normalized_path: String,
    settings_hash: [u8; 32],
    queued_at: Instant,
    source_key: String,
    source_kind: String,
    source_worker_limit: usize,
}

#[derive(Debug)]
struct DiagnosticFreshFingerprintOutput {
    normalized_path: String,
    path: PathBuf,
    settings_hash: [u8; 32],
    source_key: String,
    source_kind: String,
    source_worker_limit: usize,
    queue_wait_millis: u128,
    worker_wall_millis: u128,
    result: Result<InstrumentedMediaFingerprint, String>,
}

#[derive(Debug, Clone)]
struct DiagnosticFreshFingerprintTiming {
    path: String,
    source_path_root: Option<String>,
    source_path_kind: Option<String>,
    source_volume_id: Option<String>,
    source_worker_limit: usize,
    duration_ms: Option<u32>,
    size_bytes: u64,
    extraction_total_millis: u128,
    queue_wait_millis: u128,
    ffmpeg_process_wall_millis: Option<u128>,
    ffmpeg_input_read_bytes: Option<u64>,
    ffmpeg_input_read_ops: Option<u64>,
    ffmpeg_output_pcm_bytes: Option<u64>,
    read_amplification_ratio: Option<f64>,
    ffmpeg_invocation_count: Option<usize>,
    sampled_window_seek_millis: Option<u128>,
    sampled_window_decode_millis: Option<u128>,
    analyzer_millis: Option<u128>,
    sampled_audio_seconds_decoded: Option<u32>,
    sampled_audio_windows_decoded: Option<usize>,
    status: String,
}

struct DiagnosticFreshFingerprintTimingContext<'a> {
    path: &'a Path,
    source_key: &'a str,
    source_kind: &'a str,
    source_worker_limit: usize,
    queue_wait_millis: u128,
    worker_wall_millis: u128,
    status: &'a str,
}

#[derive(Debug, Clone, Default)]
struct DiagnosticFingerprintBatchStats {
    worker_count: usize,
    extraction_queue_wait_millis: u128,
    extraction_worker_wall_millis: u128,
    sqlite_writer_millis: u128,
    sqlite_write_queue_depth_max: usize,
    producer_worker_count: usize,
    writer_thread_enabled: bool,
    result_queue_depth_max: usize,
    result_queue_wait_millis: u128,
    writer_idle_millis: u128,
    writer_busy_millis: u128,
    writer_batch_count: usize,
    writer_rows_inserted: usize,
    writer_records_inserted: usize,
    writer_commit_millis: u128,
    extraction_worker_idle_millis: u128,
    extraction_worker_active_millis: u128,
    end_to_end_index_wall_millis: u128,
    fresh_timings: Vec<DiagnosticFreshFingerprintTiming>,
    files_indexed: usize,
    files_skipped_from_cache: usize,
    files_failed: usize,
    resumed_file_count: usize,
    sqlite_cache_compatible_hit_count: usize,
    sqlite_cache_incompatible_miss_count: usize,
    sampled_policy_mismatch_count: usize,
}

struct DiagnosticFingerprintRecords {
    cache: BTreeMap<(String, [u8; 32]), CachedFingerprint>,
    occurrence_cache: BTreeMap<(usize, usize), CachedFingerprint>,
    stats: DiagnosticFingerprintBatchStats,
}

pub fn media_match_v3_diagnostic_manifest_from_json(
    manifest_json: &str,
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    let manifest = serde_json::from_str(manifest_json)
        .map_err(|error| format!("failed parsing media-match V3 diagnostic manifest: {error}"))?;
    validate_media_match_v3_diagnostic_manifest(&manifest)?;
    Ok(manifest)
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
    let run_started_at = Instant::now();
    let index_mode = options.index_mode;
    let dense_audio_profile = options.dense_audio_profile;
    let mut settings = diagnostic_settings_for_profile(&manifest.profile)?
        .with_dense_audio_profile(dense_audio_profile);
    if !matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Full) {
        let index_settings = match index_mode {
            MediaMatchV3DiagnosticIndexMode::SparseFull => {
                MediaExtractionSettings::sparse_full_audio_v3()
            }
            MediaMatchV3DiagnosticIndexMode::SampledFast => {
                MediaExtractionSettings::sampled_fast_audio_index_v3()
            }
            MediaMatchV3DiagnosticIndexMode::SampledNormal
            | MediaMatchV3DiagnosticIndexMode::SampledThenFull => {
                MediaExtractionSettings::sampled_audio_index_v3()
            }
            MediaMatchV3DiagnosticIndexMode::Production => {
                MediaExtractionSettings::sampled_fast_audio_index_v3()
            }
            MediaMatchV3DiagnosticIndexMode::Full => unreachable!("handled above"),
        };
        settings.audio_index_mode = index_settings.audio_index_mode;
        settings.audio_algorithm = index_settings.audio_algorithm;
    }
    if settings.audio_index_mode.is_sampled() {
        if !sampled_audio_source_is_production_compatible(options.sampled_audio_source)
            && !options.experimental_sampled_audio_source
        {
            return Err(format!(
                "sampled audio source '{}' is experimental; set experimental_sampled_audio_source for diagnostics and use a separate cache root",
                options.sampled_audio_source.label()
            ));
        }
        if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production)
            && !sampled_audio_source_is_production_compatible(options.sampled_audio_source)
        {
            return Err(
                "production diagnostics require the fixed current sampled-fast source".to_owned(),
            );
        }
        settings = settings.with_sampled_audio_policy(
            MediaSampledAudioPolicy::for_sampled_fast_source_strategy(options.sampled_audio_source),
        );
    }
    let verify_settings = if matches!(
        index_mode,
        MediaMatchV3DiagnosticIndexMode::SampledThenFull
            | MediaMatchV3DiagnosticIndexMode::Production
    ) {
        diagnostic_settings_for_profile(&manifest.profile)?
            .with_dense_audio_profile(dense_audio_profile)
    } else {
        settings.clone()
    };
    let settings_hash = media_extraction_settings_hash(&settings);
    let resolved = resolve_media_match_v3_diagnostic_manifest(manifest, &options.manifest_dir)?;
    let cache_open_started_at = Instant::now();
    let connection = open_media_match_v3_index(&options.cache_root)?;
    let cache_open_millis = cache_open_started_at.elapsed().as_millis();
    let autoplay_settings = diagnostic_decision_settings();
    let mut cases = Vec::new();
    let mut summary = MediaMatchV3DiagnosticSummaryReport {
        case_count: resolved.cases.len(),
        cache_open_millis,
        max_full_promotions_per_query: options.max_full_promotions_per_query.max(1),
        sampled_audio_policy: settings
            .audio_index_mode
            .is_sampled()
            .then(|| settings.sampled_audio_policy.clone()),
        sampled_policy_production_compatible: !settings.audio_index_mode.is_sampled()
            || (settings.sampled_audio_policy.is_production_compatible()
                && sampled_audio_source_is_production_compatible(options.sampled_audio_source)),
        sampled_audio_source_strategy: settings
            .audio_index_mode
            .is_sampled()
            .then(|| options.sampled_audio_source.label().to_owned()),
        experimental_sampled_audio_source: settings.audio_index_mode.is_sampled()
            && options.experimental_sampled_audio_source,
        sampled_policy_cache_recommendation: (settings.audio_index_mode.is_sampled()
            && !sampled_audio_source_is_production_compatible(options.sampled_audio_source))
        .then(|| "experimental sampled audio sources should use a dedicated cache root and must not be mixed with the production sampled-fast cache".to_owned()),
        sampled_audio_source_equivalence: settings
            .audio_index_mode
            .is_sampled()
            .then(|| sampled_audio_source_equivalence_report(options.sampled_audio_source)),
        ..MediaMatchV3DiagnosticSummaryReport::default()
    };

    let fingerprint_started_at = Instant::now();
    let fingerprint_records = fingerprint_diagnostic_manifest_records(
        &resolved,
        &connection,
        &options.tools,
        &settings,
        options.refresh_cache,
        &options,
    )?;
    let mut cache = fingerprint_records.cache;
    let occurrence_cache = fingerprint_records.occurrence_cache;
    let fingerprint_batch_stats = fingerprint_records.stats;
    summary.fingerprint_total_millis = fingerprint_started_at.elapsed().as_millis();
    summary.sampled_fast_worker_count = fingerprint_batch_stats.worker_count;
    summary.sqlite_cache_compatible_hit_count =
        fingerprint_batch_stats.sqlite_cache_compatible_hit_count;
    summary.sqlite_cache_incompatible_miss_count =
        fingerprint_batch_stats.sqlite_cache_incompatible_miss_count;
    summary.sampled_policy_mismatch_count = fingerprint_batch_stats.sampled_policy_mismatch_count;
    summary.extraction_queue_wait_millis = fingerprint_batch_stats.extraction_queue_wait_millis;
    summary.extraction_worker_wall_millis = fingerprint_batch_stats.extraction_worker_wall_millis;
    summary.sqlite_writer_millis = fingerprint_batch_stats.sqlite_writer_millis;
    summary.sqlite_write_queue_depth_max = fingerprint_batch_stats.sqlite_write_queue_depth_max;
    summary.producer_worker_count = fingerprint_batch_stats.producer_worker_count;
    summary.writer_thread_enabled = fingerprint_batch_stats.writer_thread_enabled;
    summary.result_queue_depth_max = fingerprint_batch_stats.result_queue_depth_max;
    summary.result_queue_wait_millis = fingerprint_batch_stats.result_queue_wait_millis;
    summary.writer_idle_millis = fingerprint_batch_stats.writer_idle_millis;
    summary.writer_busy_millis = fingerprint_batch_stats.writer_busy_millis;
    summary.writer_batch_count = fingerprint_batch_stats.writer_batch_count;
    summary.writer_rows_inserted = fingerprint_batch_stats.writer_rows_inserted;
    summary.writer_records_inserted = fingerprint_batch_stats.writer_records_inserted;
    summary.writer_records_per_batch = if fingerprint_batch_stats.writer_batch_count == 0 {
        0.0
    } else {
        fingerprint_batch_stats.writer_records_inserted as f64
            / fingerprint_batch_stats.writer_batch_count as f64
    };
    summary.writer_commit_millis = fingerprint_batch_stats.writer_commit_millis;
    summary.extraction_worker_idle_millis = fingerprint_batch_stats.extraction_worker_idle_millis;
    summary.extraction_worker_active_millis =
        fingerprint_batch_stats.extraction_worker_active_millis;
    summary.end_to_end_index_wall_millis = fingerprint_batch_stats.end_to_end_index_wall_millis;
    apply_fresh_fingerprint_timing_summary(&mut summary, &fingerprint_batch_stats.fresh_timings);
    summary.files_skipped_from_cache = fingerprint_batch_stats.files_skipped_from_cache;
    summary.files_failed = fingerprint_batch_stats.files_failed;
    summary.resumed_file_count = fingerprint_batch_stats.resumed_file_count;
    summary.sampled_cache_hit_count = fingerprint_batch_stats.files_skipped_from_cache;
    if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production) {
        summary.production_sampled_index_millis = summary.fingerprint_total_millis;
    }

    for (case_index, case) in resolved.cases.iter().enumerate() {
        let query = occurrence_cache
            .get(&(case_index, 0))
            .ok_or_else(|| format!("missing diagnostic query fingerprint for '{}'", case.name))?
            .clone();
        let (retrieved_candidates, retrieval_stats) =
            media_match_v3_anchor_candidate_details_with_strategy(
                &connection,
                &query.fingerprint.record.identity.normalized_path,
                &settings,
                options.retrieval_strategy,
            )?;
        let known_candidate_ids = known_candidate_ids_for_case(case);
        let expected_candidate_paths = expected_candidate_paths_for_case(case);
        let hard_negative_paths = hard_negative_paths_for_case(case);
        let retrieval_report = MediaMatchV3DiagnosticRetrievalReport::from_stats(
            retrieval_stats,
            retrieved_candidates,
            &known_candidate_ids,
            &expected_candidate_paths,
            &hard_negative_paths,
        );
        summary.total_raw_hit_rows_processed += retrieval_report.raw_hit_rows_processed;
        summary.total_retrieval_millis += retrieval_report.retrieval_elapsed_ms;
        summary.retrieval_total_millis += retrieval_report.retrieval_elapsed_ms;
        summary.retrieval_unaccounted_millis_total += retrieval_report.retrieval_unaccounted_millis;
        summary.sql_hit_fetch_millis_total += retrieval_report.sql_hit_fetch_millis;
        summary.rust_aggregation_millis_total += retrieval_report.rust_aggregation_millis;
        summary.candidate_metadata_load_millis_total +=
            retrieval_report.candidate_metadata_load_millis;
        summary.robust_rerank_millis_total += retrieval_report.robust_rerank_millis;

        let query_report = MediaMatchV3DiagnosticFingerprintReport {
            path: query.fingerprint.record.identity.normalized_path.clone(),
            diagnostics: diagnostics_for_cached_fingerprint(&query),
            source: query.source.to_owned(),
            sqlite_save_millis: query.save_stats.sqlite_save_millis,
            blob_encode_millis: query.save_stats.blob_encode_millis,
            index_insert_millis: query.save_stats.index_insert_millis,
        };
        increment_report_source_count(&mut summary, query.source);
        let mut reports = Vec::new();
        let mut positive_rank_by_id = BTreeMap::new();
        for (candidate_index, candidate) in case.candidates.iter().enumerate() {
            let index_fingerprint = occurrence_cache
                .get(&(case_index, candidate_index + 1))
                .ok_or_else(|| {
                    format!(
                        "missing diagnostic candidate fingerprint for '{}'",
                        candidate.path.display()
                    )
                })?
                .clone();
            let sampled_normalized_candidate = index_fingerprint
                .fingerprint
                .record
                .identity
                .normalized_path
                .clone();
            let retrieval_rank = retrieval_report
                .retrieved_candidates
                .iter()
                .position(|path| path == &sampled_normalized_candidate)
                .map(|index| index + 1);
            if let Some(id) = candidate.expectation.id.as_deref() {
                positive_rank_by_id.insert(id.to_owned(), retrieval_rank);
            }
            let retrieved = retrieval_rank.is_some();
            let max_promotion_rank = candidate
                .expectation
                .max_promotion_rank
                .unwrap_or_else(|| options.max_full_promotions_per_query.max(1));
            let within_promotion_budget = retrieval_rank
                .map(|rank| rank <= max_promotion_rank)
                .unwrap_or(false);
            let strict_rank1_passed = retrieval_rank == Some(1);
            let production_retrieval_passed = within_promotion_budget;
            let sampled_retrieval_rank = matches!(
                index_mode,
                MediaMatchV3DiagnosticIndexMode::SampledFast
                    | MediaMatchV3DiagnosticIndexMode::SampledNormal
                    | MediaMatchV3DiagnosticIndexMode::SampledThenFull
                    | MediaMatchV3DiagnosticIndexMode::Production
            )
            .then_some(retrieval_rank)
            .flatten();
            let mut query_for_decision = query.clone();
            let mut fingerprint = index_fingerprint;
            let mut promotion_reason = None;
            let mut full_promotion_millis = 0;
            let production_promotion = !options.retrieval_benchmark_only
                && matches!(
                    index_mode,
                    MediaMatchV3DiagnosticIndexMode::SampledThenFull
                        | MediaMatchV3DiagnosticIndexMode::Production
                );
            if production_promotion
                && let Some(reason) = sampled_then_full_promotion_reason(
                    &candidate.expectation,
                    retrieval_rank,
                    &options,
                )
            {
                let promotion_started_at = Instant::now();
                query_for_decision = fingerprint_cached(
                    &mut cache,
                    &connection,
                    &case.query,
                    &options.tools,
                    &verify_settings,
                    options.refresh_cache,
                )?;
                if query_for_decision.source != FINGERPRINT_SOURCE_FRESH {
                    summary.full_promotion_cache_hits += 1;
                }
                save_fresh_fingerprint_if_needed(&mut cache, &mut query_for_decision, &connection)?;
                fingerprint = fingerprint_cached(
                    &mut cache,
                    &connection,
                    &candidate.path,
                    &options.tools,
                    &verify_settings,
                    options.refresh_cache,
                )?;
                if fingerprint.source != FINGERPRINT_SOURCE_FRESH {
                    summary.full_promotion_cache_hits += 1;
                }
                save_fresh_fingerprint_if_needed(&mut cache, &mut fingerprint, &connection)?;
                full_promotion_millis = promotion_started_at.elapsed().as_millis();
                summary.fingerprint_total_millis += full_promotion_millis;
                summary.full_promotion_millis += full_promotion_millis;
                if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production) {
                    summary.production_full_promotion_millis += full_promotion_millis;
                }
                summary.candidates_promoted_to_full_verify += 1;
                summary.full_promoted_file_count += 1;
                promotion_reason = Some(reason);
            }
            let (decision, decision_report) = if options.retrieval_benchmark_only {
                (None, retrieval_benchmark_decision_report())
            } else {
                let decision_started_at = Instant::now();
                let decision = cap_sampled_record_decision_if_needed(
                    decide_media_match(
                        &query_for_decision.fingerprint.record,
                        &fingerprint.fingerprint.record,
                        &autoplay_settings,
                    ),
                    &query_for_decision.fingerprint.record,
                    &fingerprint.fingerprint.record,
                );
                summary.decision_total_millis += decision_started_at.elapsed().as_millis();
                let report = MediaMatchV3DiagnosticDecisionReport::from_decision(
                    &decision,
                    &autoplay_settings,
                );
                (Some(decision), report)
            };
            let normalized_candidate = &fingerprint.fingerprint.record.identity.normalized_path;
            let failures = if let Some(decision) = decision.as_ref() {
                evaluate_diagnostic_expectation(
                    decision,
                    &candidate.expectation,
                    &autoplay_settings,
                    retrieved,
                    retrieval_rank,
                    within_promotion_budget,
                )
            } else {
                evaluate_retrieval_benchmark_expectation(
                    &candidate.expectation,
                    retrieved,
                    retrieval_rank,
                    within_promotion_budget,
                )
            };
            let passed = failures.is_empty();
            summary.pair_count += 1;
            if passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            increment_report_source_count(&mut summary, fingerprint.source);
            let final_verified_rank = promotion_reason.as_ref().and(retrieval_rank);
            let first_strong_candidate_rank = decision.as_ref().and_then(|decision| {
                (decision.tier == MediaMatchTier::Strong)
                    .then_some(final_verified_rank)
                    .flatten()
            });
            let promoted_candidate_ranks = final_verified_rank.into_iter().collect::<Vec<_>>();
            reports.push(MediaMatchV3DiagnosticCandidateReport {
                candidate_id: candidate.expectation.id.clone(),
                path: normalized_candidate.clone(),
                diagnostics: diagnostics_for_cached_fingerprint(&fingerprint),
                source: fingerprint.source.to_owned(),
                sqlite_save_millis: fingerprint.save_stats.sqlite_save_millis,
                blob_encode_millis: fingerprint.save_stats.blob_encode_millis,
                index_insert_millis: fingerprint.save_stats.index_insert_millis,
                retrieved,
                retrieval_rank,
                sampled_retrieval_rank,
                final_verified_rank,
                within_promotion_budget,
                strict_rank1_passed,
                production_retrieval_passed,
                promotion_budget_exhausted: !within_promotion_budget && retrieval_rank.is_some(),
                promoted_candidate_ranks,
                first_strong_candidate_rank,
                promotion_reason,
                full_promotion_millis,
                decision: decision_report,
                expectation: Some(candidate.expectation.clone()),
                passed,
                failure_reason: (!failures.is_empty()).then(|| failures.join("; ")),
            });
        }
        let mut hard_negative_reports = Vec::new();
        for (hard_negative_index, hard_negative) in case.hard_negatives.iter().enumerate() {
            let fingerprint = occurrence_cache
                .get(&(case_index, case.candidates.len() + hard_negative_index + 1))
                .ok_or_else(|| {
                    format!(
                        "missing diagnostic hard-negative fingerprint for '{}'",
                        hard_negative.path.display()
                    )
                })?
                .clone();
            let normalized_path = fingerprint
                .fingerprint
                .record
                .identity
                .normalized_path
                .clone();
            let retrieval_rank = retrieval_report
                .retrieved_candidates
                .iter()
                .position(|path| path == &normalized_path)
                .map(|index| index + 1);
            let failures = evaluate_hard_negative_expectation(
                &hard_negative.expectation,
                retrieval_rank,
                &positive_rank_by_id,
            );
            let passed = failures.is_empty();
            summary.hard_negative_count += 1;
            if passed {
                summary.hard_negative_passed += 1;
            } else {
                summary.hard_negative_failed += 1;
            }
            increment_report_source_count(&mut summary, fingerprint.source);
            hard_negative_reports.push(MediaMatchV3DiagnosticHardNegativeReport {
                candidate_id: hard_negative.expectation.id.clone(),
                path: normalized_path,
                diagnostics: diagnostics_for_cached_fingerprint(&fingerprint),
                source: fingerprint.source.to_owned(),
                sqlite_save_millis: fingerprint.save_stats.sqlite_save_millis,
                blob_encode_millis: fingerprint.save_stats.blob_encode_millis,
                index_insert_millis: fingerprint.save_stats.index_insert_millis,
                retrieved: retrieval_rank.is_some(),
                retrieval_rank,
                must_not_be_top_rank: hard_negative.expectation.must_not_be_top_rank,
                must_not_beat_candidate_id: hard_negative
                    .expectation
                    .must_not_beat_candidate_id
                    .clone(),
                passed,
                failure_reason: (!failures.is_empty()).then(|| failures.join("; ")),
            });
        }
        cases.push(MediaMatchV3DiagnosticCaseReport {
            name: case.name.clone(),
            query: query_report,
            retrieval: retrieval_report,
            candidates: reports,
            hard_negatives: hard_negative_reports,
        });
    }

    for fingerprint in cache.values() {
        summary.sqlite_load_millis += fingerprint.sqlite_load_millis;
        summary.sqlite_save_millis += fingerprint.save_stats.sqlite_save_millis;
        summary.sqlite_index_insert_millis += fingerprint.save_stats.index_insert_millis;
        match fingerprint
            .fingerprint
            .record
            .extraction_settings
            .audio_index_mode
        {
            MediaAudioIndexMode::SampledFast | MediaAudioIndexMode::SampledNormal => {
                summary.sampled_fingerprint_count += 1;
                summary.sampled_indexed_file_count += 1;
            }
            MediaAudioIndexMode::FullVerify | MediaAudioIndexMode::SparseFull => {
                summary.full_fingerprint_count += 1
            }
        }
    }
    if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production) {
        summary.full_promoted_file_count = summary.full_fingerprint_count;
    }
    apply_report_row_fingerprint_totals(&mut summary, &cases);
    apply_retrieval_percentile_summary(&mut summary, &cases);

    summary.run_wall_millis = run_started_at.elapsed().as_millis();
    if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production) {
        summary.production_retrieval_millis = summary.retrieval_total_millis;
        summary.production_total_millis = summary
            .production_sampled_index_millis
            .saturating_add(summary.production_retrieval_millis)
            .saturating_add(summary.production_full_promotion_millis);
        summary.production_passed = summary.failed == 0;
    }
    apply_production_throughput_summary(&mut summary, index_mode);
    let sqlite_size = media_match_v3_sqlite_size_report(&options.cache_root, &connection).ok();
    if let Some(sqlite_size) = sqlite_size.as_ref() {
        summary.db_total_bytes = sqlite_size.total_bytes;
        summary.db_anchor_index_bytes = sqlite_size.anchor_index_bytes;
        summary.db_fingerprint_bytes = sqlite_size
            .object_bytes
            .iter()
            .filter(|object| object.name.contains("fingerprints_v3"))
            .map(|object| object.bytes)
            .sum();
        summary.db_stats_bytes = sqlite_size
            .object_bytes
            .iter()
            .filter(|object| object.name.contains("anchor_stats_v3"))
            .map(|object| object.bytes)
            .sum();
        summary.db_index_bytes = sqlite_size.db_index_bytes;
        summary.db_bytes_per_fingerprint = sqlite_size.db_bytes_per_fingerprint;
        summary.db_bytes_per_anchor = sqlite_size.db_bytes_per_anchor;
    }

    Ok(MediaMatchV3DiagnosticReport {
        algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
        fingerprint_cache_version: crate::MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
        profile: settings.profile.label().to_owned(),
        index_mode: index_mode.label().to_owned(),
        dense_audio_profile: dense_audio_profile.label().to_owned(),
        settings_hash: bytes_to_lower_hex(&settings_hash),
        tuning: current_v3_tuning(),
        cache_root: options.cache_root.to_string_lossy().to_string(),
        cache_retained: options.cache_retained,
        generated_at_unix_millis: options
            .generated_at_unix_millis
            .unwrap_or_else(current_unix_millis),
        cases,
        summary,
        sqlite_size,
    })
}

fn apply_production_throughput_summary(
    summary: &mut MediaMatchV3DiagnosticSummaryReport,
    index_mode: MediaMatchV3DiagnosticIndexMode,
) {
    let uses_sampled_index = matches!(
        index_mode,
        MediaMatchV3DiagnosticIndexMode::SampledFast
            | MediaMatchV3DiagnosticIndexMode::SampledNormal
            | MediaMatchV3DiagnosticIndexMode::SampledThenFull
            | MediaMatchV3DiagnosticIndexMode::Production
    );
    if summary.sampled_fast_worker_count == 0 {
        summary.sampled_fast_worker_count =
            usize::from(summary.sampled_indexed_file_count > 0 && uses_sampled_index);
    }
    if summary.full_verify_worker_count == 0 {
        summary.full_verify_worker_count = usize::from(summary.full_fingerprint_count > 0);
    }
    let (files, millis) = if matches!(index_mode, MediaMatchV3DiagnosticIndexMode::Production) {
        (
            summary.sampled_indexed_file_count,
            summary.production_sampled_index_millis,
        )
    } else {
        (
            summary
                .sampled_indexed_file_count
                .saturating_add(summary.full_fingerprint_count),
            summary.fingerprint_total_millis,
        )
    };
    summary.files_per_minute = files_per_minute(files, millis);
}

fn files_per_minute(files: usize, millis: u128) -> u64 {
    if files == 0 || millis == 0 {
        return 0;
    }
    let rounded = (files as u128)
        .saturating_mul(60_000)
        .saturating_add(millis / 2)
        / millis;
    rounded.min(u64::MAX as u128) as u64
}

fn apply_retrieval_percentile_summary(
    summary: &mut MediaMatchV3DiagnosticSummaryReport,
    cases: &[MediaMatchV3DiagnosticCaseReport],
) {
    let mut retrieval_millis = cases
        .iter()
        .map(|case| case.retrieval.retrieval_elapsed_ms)
        .collect::<Vec<_>>();
    if retrieval_millis.is_empty() {
        return;
    }
    retrieval_millis.sort_unstable();
    summary.per_query_retrieval_millis_p50 = percentile_u128(&retrieval_millis, 50);
    summary.per_query_retrieval_millis_p95 = percentile_u128(&retrieval_millis, 95);
    summary.per_query_retrieval_millis_p99 = percentile_u128(&retrieval_millis, 99);
    summary.per_query_retrieval_millis_max = retrieval_millis.last().copied().unwrap_or_default();

    let mut unaccounted_millis = cases
        .iter()
        .map(|case| case.retrieval.retrieval_unaccounted_millis)
        .collect::<Vec<_>>();
    unaccounted_millis.sort_unstable();
    summary.retrieval_unaccounted_millis_p95 = percentile_u128(&unaccounted_millis, 95);
}

fn apply_fresh_fingerprint_timing_summary(
    summary: &mut MediaMatchV3DiagnosticSummaryReport,
    timings: &[DiagnosticFreshFingerprintTiming],
) {
    if timings.is_empty() {
        return;
    }
    let mut extraction = timings
        .iter()
        .map(|timing| timing.extraction_total_millis)
        .collect::<Vec<_>>();
    extraction.sort_unstable();
    summary.fresh_fingerprint_millis_p50 = percentile_u128(&extraction, 50);
    summary.fresh_fingerprint_millis_p75 = percentile_u128(&extraction, 75);
    summary.fresh_fingerprint_millis_p95 = percentile_u128(&extraction, 95);
    summary.fresh_fingerprint_millis_p99 = percentile_u128(&extraction, 99);
    summary.fresh_fingerprint_millis_max = extraction.last().copied().unwrap_or_default();

    let mut ffmpeg = timings
        .iter()
        .filter_map(|timing| timing.ffmpeg_process_wall_millis)
        .collect::<Vec<_>>();
    ffmpeg.sort_unstable();
    summary.fresh_fingerprint_ffmpeg_wall_millis_p50 = percentile_u128(&ffmpeg, 50);
    summary.fresh_fingerprint_ffmpeg_wall_millis_p95 = percentile_u128(&ffmpeg, 95);

    let mut analyzer = timings
        .iter()
        .filter_map(|timing| timing.analyzer_millis)
        .collect::<Vec<_>>();
    analyzer.sort_unstable();
    summary.fresh_fingerprint_analyzer_millis_p50 = percentile_u128(&analyzer, 50);
    summary.fresh_fingerprint_analyzer_millis_p95 = percentile_u128(&analyzer, 95);

    let mut sampled_seconds = timings
        .iter()
        .filter_map(|timing| timing.sampled_audio_seconds_decoded)
        .map(u128::from)
        .collect::<Vec<_>>();
    sampled_seconds.sort_unstable();
    summary.fresh_sampled_audio_seconds_decoded_p50 = percentile_u128(&sampled_seconds, 50);
    summary.fresh_sampled_audio_seconds_decoded_p95 = percentile_u128(&sampled_seconds, 95);

    let mut sampled_windows = timings
        .iter()
        .filter_map(|timing| timing.sampled_audio_windows_decoded)
        .map(|value| value as u128)
        .collect::<Vec<_>>();
    sampled_windows.sort_unstable();
    summary.fresh_sampled_audio_windows_decoded_p50 = percentile_u128(&sampled_windows, 50);
    summary.fresh_sampled_audio_windows_decoded_p95 = percentile_u128(&sampled_windows, 95);

    let mut queue_wait = timings
        .iter()
        .map(|timing| timing.queue_wait_millis)
        .collect::<Vec<_>>();
    queue_wait.sort_unstable();
    summary.fresh_fingerprint_queue_wait_millis_p50 = percentile_u128(&queue_wait, 50);
    summary.fresh_fingerprint_queue_wait_millis_p95 = percentile_u128(&queue_wait, 95);

    let mut slowest = timings.to_vec();
    slowest.sort_by(|left, right| {
        right
            .extraction_total_millis
            .cmp(&left.extraction_total_millis)
            .then_with(|| left.path.cmp(&right.path))
    });
    summary.slowest_fresh_fingerprints = slowest
        .into_iter()
        .take(20)
        .map(|timing| MediaMatchV3SlowFingerprintReport {
            path: timing.path,
            source_path_root: timing.source_path_root,
            source_path_kind: timing.source_path_kind,
            source_volume_id: timing.source_volume_id,
            duration_ms: timing.duration_ms,
            size_bytes: timing.size_bytes,
            extraction_total_millis: timing.extraction_total_millis,
            queue_wait_millis: timing.queue_wait_millis,
            ffmpeg_process_wall_millis: timing.ffmpeg_process_wall_millis,
            ffmpeg_input_read_bytes: timing.ffmpeg_input_read_bytes,
            ffmpeg_input_read_ops: timing.ffmpeg_input_read_ops,
            ffmpeg_output_pcm_bytes: timing.ffmpeg_output_pcm_bytes,
            read_amplification_ratio: timing.read_amplification_ratio,
            ffmpeg_invocation_count: timing.ffmpeg_invocation_count,
            sampled_window_seek_millis: timing.sampled_window_seek_millis,
            sampled_window_decode_millis: timing.sampled_window_decode_millis,
            analyzer_millis: timing.analyzer_millis,
            sampled_audio_seconds_decoded: timing.sampled_audio_seconds_decoded,
            sampled_audio_windows_decoded: timing.sampled_audio_windows_decoded,
            status: timing.status,
        })
        .collect();
    summary.source_index_reports = source_index_reports_from_timings(timings);
}

#[derive(Debug, Default)]
struct SourceIndexTimingAggregate {
    source_path_kind: String,
    worker_limit: usize,
    files_indexed: usize,
    files_failed: usize,
    extraction_millis: Vec<u128>,
    ffmpeg_wall_millis: Vec<u128>,
    extraction_millis_sum: u128,
    source_read_bytes: u64,
    source_read_ops: u64,
    output_pcm_bytes: u64,
}

fn source_index_reports_from_timings(
    timings: &[DiagnosticFreshFingerprintTiming],
) -> Vec<MediaMatchV3SourceIndexReport> {
    let mut aggregates = BTreeMap::<String, SourceIndexTimingAggregate>::new();
    for timing in timings {
        let source_root = timing
            .source_path_root
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let source_kind = timing
            .source_path_kind
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let aggregate =
            aggregates
                .entry(source_root)
                .or_insert_with(|| SourceIndexTimingAggregate {
                    source_path_kind: source_kind.clone(),
                    worker_limit: timing.source_worker_limit.max(1),
                    ..SourceIndexTimingAggregate::default()
                });
        aggregate.source_path_kind = source_kind;
        aggregate.worker_limit = aggregate
            .worker_limit
            .max(timing.source_worker_limit.max(1));
        if timing.status == "complete" {
            aggregate.files_indexed += 1;
        } else {
            aggregate.files_failed += 1;
        }
        aggregate
            .extraction_millis
            .push(timing.extraction_total_millis);
        aggregate.extraction_millis_sum = aggregate
            .extraction_millis_sum
            .saturating_add(timing.extraction_total_millis);
        if let Some(ffmpeg_millis) = timing.ffmpeg_process_wall_millis {
            aggregate.ffmpeg_wall_millis.push(ffmpeg_millis);
        }
        aggregate.source_read_bytes = aggregate
            .source_read_bytes
            .saturating_add(timing.ffmpeg_input_read_bytes.unwrap_or_default());
        aggregate.source_read_ops = aggregate
            .source_read_ops
            .saturating_add(timing.ffmpeg_input_read_ops.unwrap_or_default());
        aggregate.output_pcm_bytes = aggregate
            .output_pcm_bytes
            .saturating_add(timing.ffmpeg_output_pcm_bytes.unwrap_or_default());
    }

    aggregates
        .into_iter()
        .map(|(source_path_root, mut aggregate)| {
            aggregate.extraction_millis.sort_unstable();
            aggregate.ffmpeg_wall_millis.sort_unstable();
            let effective_millis = aggregate
                .extraction_millis_sum
                .checked_div(aggregate.worker_limit.max(1) as u128)
                .unwrap_or_default()
                .max(1);
            let read_amplification_ratio =
                if aggregate.output_pcm_bytes > 0 && aggregate.source_read_bytes > 0 {
                    Some(aggregate.source_read_bytes as f64 / aggregate.output_pcm_bytes as f64)
                } else {
                    None
                };
            MediaMatchV3SourceIndexReport {
                source_path_root,
                source_path_kind: aggregate.source_path_kind,
                worker_limit: aggregate.worker_limit,
                files_indexed: aggregate.files_indexed,
                files_failed: aggregate.files_failed,
                files_per_minute: files_per_minute(aggregate.files_indexed, effective_millis),
                ffmpeg_wall_millis_p50: percentile_u128(&aggregate.ffmpeg_wall_millis, 50),
                ffmpeg_wall_millis_p95: percentile_u128(&aggregate.ffmpeg_wall_millis, 95),
                extraction_millis_p50: percentile_u128(&aggregate.extraction_millis, 50),
                extraction_millis_p95: percentile_u128(&aggregate.extraction_millis, 95),
                source_read_bytes: aggregate.source_read_bytes,
                source_read_ops: aggregate.source_read_ops,
                output_pcm_bytes: aggregate.output_pcm_bytes,
                read_amplification_ratio,
                queue_depth_max: 0,
                concurrency_changes: Vec::new(),
            }
        })
        .collect()
}

fn percentile_u128(sorted_values: &[u128], percentile: usize) -> u128 {
    if sorted_values.is_empty() {
        return 0;
    }
    let clamped = percentile.min(100);
    let index = (sorted_values.len() - 1) * clamped / 100;
    sorted_values[index]
}

fn apply_report_row_fingerprint_totals(
    summary: &mut MediaMatchV3DiagnosticSummaryReport,
    cases: &[MediaMatchV3DiagnosticCaseReport],
) {
    let mut seen = BTreeSet::<(String, String)>::new();
    summary.total_extraction_millis = 0;
    summary.total_audio_blob_bytes = 0;
    summary.total_video_blob_bytes = 0;
    summary.unique_fresh_fingerprint_count = 0;
    summary.unique_memory_cache_fingerprint_count = 0;
    summary.unique_sqlite_cache_fingerprint_count = 0;
    for case in cases {
        add_report_row_fingerprint_totals(
            &case.query.path,
            &case.query.diagnostics,
            &case.query.source,
            &mut seen,
            summary,
        );
        for candidate in &case.candidates {
            add_report_row_fingerprint_totals(
                &candidate.path,
                &candidate.diagnostics,
                &candidate.source,
                &mut seen,
                summary,
            );
        }
        for hard_negative in &case.hard_negatives {
            add_report_row_fingerprint_totals(
                &hard_negative.path,
                &hard_negative.diagnostics,
                &hard_negative.source,
                &mut seen,
                summary,
            );
        }
    }
}

fn add_report_row_fingerprint_totals(
    path: &str,
    diagnostics: &MediaMatchV3DiagnosticSummary,
    source: &str,
    seen: &mut BTreeSet<(String, String)>,
    summary: &mut MediaMatchV3DiagnosticSummaryReport,
) {
    if !seen.insert((path.to_owned(), diagnostics.profile.clone())) {
        return;
    }
    summary.total_extraction_millis += diagnostics.extraction_total_millis.unwrap_or_default();
    summary.total_audio_blob_bytes += diagnostics.audio_blob_bytes;
    summary.total_video_blob_bytes += diagnostics.video_blob_bytes;
    match source {
        FINGERPRINT_SOURCE_FRESH => summary.unique_fresh_fingerprint_count += 1,
        FINGERPRINT_SOURCE_MEMORY_CACHE => summary.unique_memory_cache_fingerprint_count += 1,
        FINGERPRINT_SOURCE_SQLITE_CACHE => summary.unique_sqlite_cache_fingerprint_count += 1,
        _ => {}
    }
}

pub fn resolve_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    manifest_dir: &Path,
) -> Result<MediaMatchV3ResolvedManifest, String> {
    validate_media_match_v3_diagnostic_manifest(manifest)?;
    let base = manifest
        .base_dir
        .as_deref()
        .map(|base_dir| resolve_manifest_path(manifest_dir, manifest_dir, base_dir))
        .unwrap_or_else(|| manifest_dir.to_path_buf());
    let cases = manifest
        .cases
        .iter()
        .map(|case| {
            let mut candidate_ids = BTreeSet::new();
            let mut no_id_candidate_paths = BTreeSet::new();
            let candidates = case
                .candidates
                .iter()
                .map(|candidate| {
                    let path = resolve_manifest_path(manifest_dir, &base, &candidate.path);
                    let mut expectation = candidate.clone();
                    if let Some(id) = expectation.id.as_deref() {
                        let trimmed = id.trim();
                        if trimmed.is_empty() {
                            return Err(format!(
                                "case '{}' candidate '{}' has a blank id",
                                case.name, candidate.path
                            ));
                        }
                        if !candidate_ids.insert(trimmed.to_owned()) {
                            return Err(format!(
                                "case '{}' has duplicate candidate id '{}'",
                                case.name, trimmed
                            ));
                        }
                        expectation.id = Some(trimmed.to_owned());
                    } else {
                        let key = path.to_string_lossy().to_string();
                        if !no_id_candidate_paths.insert(key.clone()) {
                            return Err(format!(
                                "case '{}' has duplicate candidate path '{}' without an id",
                                case.name, key
                            ));
                        }
                    }
                    Ok(MediaMatchV3ResolvedManifestCandidate { path, expectation })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let hard_negatives = case
                .hard_negatives
                .iter()
                .map(|hard_negative| {
                    let path = resolve_manifest_path(manifest_dir, &base, &hard_negative.path);
                    let mut expectation = hard_negative.clone();
                    if let Some(id) = expectation.id.as_deref() {
                        let trimmed = id.trim();
                        if trimmed.is_empty() {
                            return Err(format!(
                                "case '{}' hard negative '{}' has a blank id",
                                case.name, hard_negative.path
                            ));
                        }
                        if !candidate_ids.insert(trimmed.to_owned()) {
                            return Err(format!(
                                "case '{}' has duplicate candidate/hard-negative id '{}'",
                                case.name, trimmed
                            ));
                        }
                        expectation.id = Some(trimmed.to_owned());
                    } else {
                        let key = path.to_string_lossy().to_string();
                        if !no_id_candidate_paths.insert(key.clone()) {
                            return Err(format!(
                                "case '{}' has duplicate candidate/hard-negative path '{}' without an id",
                                case.name, key
                            ));
                        }
                    }
                    Ok(MediaMatchV3ResolvedManifestHardNegative { path, expectation })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(MediaMatchV3ResolvedManifestCase {
                name: case.name.clone(),
                query: resolve_manifest_path(manifest_dir, &base, &case.query),
                candidates,
                hard_negatives,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(MediaMatchV3ResolvedManifest {
        profile: manifest.profile.clone(),
        cases,
    })
}

pub fn validate_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
) -> Result<(), String> {
    for case in &manifest.cases {
        if case.name.trim().is_empty() {
            return Err("media-match V3 diagnostic manifest has a blank case name".to_owned());
        }
        if case.query.trim().is_empty() {
            return Err(format!("case '{}' has a blank query path", case.name));
        }
        let mut candidate_ids = BTreeSet::new();
        let mut positive_candidate_ids = BTreeSet::new();
        let mut no_id_candidate_paths = BTreeSet::new();
        for candidate in &case.candidates {
            if candidate.path.trim().is_empty() {
                return Err(format!("case '{}' has a blank candidate path", case.name));
            }
            if let Some(id) = candidate.id.as_deref() {
                let trimmed = id.trim();
                if trimmed.is_empty() {
                    return Err(format!(
                        "case '{}' candidate '{}' has a blank id",
                        case.name, candidate.path
                    ));
                }
                if !candidate_ids.insert(trimmed.to_owned()) {
                    return Err(format!(
                        "case '{}' has duplicate candidate id '{}'",
                        case.name, trimmed
                    ));
                }
                positive_candidate_ids.insert(trimmed.to_owned());
            } else if !no_id_candidate_paths.insert(candidate.path.clone()) {
                return Err(format!(
                    "case '{}' has duplicate candidate path '{}' without an id",
                    case.name, candidate.path
                ));
            }
            if candidate.max_retrieval_rank == Some(0) {
                return Err(format!(
                    "case '{}' candidate '{}' has maxRetrievalRank=0; ranks are 1-based",
                    case.name, candidate.path
                ));
            }
        }
        for hard_negative in &case.hard_negatives {
            if hard_negative.path.trim().is_empty() {
                return Err(format!(
                    "case '{}' has a blank hard-negative path",
                    case.name
                ));
            }
            if let Some(id) = hard_negative.id.as_deref() {
                let trimmed = id.trim();
                if trimmed.is_empty() {
                    return Err(format!(
                        "case '{}' hard negative '{}' has a blank id",
                        case.name, hard_negative.path
                    ));
                }
                if !candidate_ids.insert(trimmed.to_owned()) {
                    return Err(format!(
                        "case '{}' has duplicate candidate/hard-negative id '{}'",
                        case.name, trimmed
                    ));
                }
            } else if !no_id_candidate_paths.insert(hard_negative.path.clone()) {
                return Err(format!(
                    "case '{}' has duplicate candidate/hard-negative path '{}' without an id",
                    case.name, hard_negative.path
                ));
            }
            if let Some(target_id) = hard_negative.must_not_beat_candidate_id.as_deref()
                && target_id.trim().is_empty()
            {
                return Err(format!(
                    "case '{}' hard negative '{}' has a blank mustNotBeatCandidateId",
                    case.name, hard_negative.path
                ));
            }
            if let Some(target_id) = hard_negative.must_not_beat_candidate_id.as_deref()
                && !positive_candidate_ids.contains(target_id.trim())
            {
                return Err(format!(
                    "case '{}' hard negative '{}' references unknown mustNotBeatCandidateId '{}'",
                    case.name, hard_negative.path, target_id
                ));
            }
        }
    }
    Ok(())
}

fn known_candidate_ids_for_case(
    case: &MediaMatchV3ResolvedManifestCase,
) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for candidate in &case.candidates {
        if let Some(id) = candidate.expectation.id.as_deref() {
            ids.insert(normalize_media_path(&candidate.path), id.to_owned());
        }
    }
    for hard_negative in &case.hard_negatives {
        if let Some(id) = hard_negative.expectation.id.as_deref() {
            ids.insert(normalize_media_path(&hard_negative.path), id.to_owned());
        }
    }
    ids
}

fn expected_candidate_paths_for_case(case: &MediaMatchV3ResolvedManifestCase) -> BTreeSet<String> {
    case.candidates
        .iter()
        .map(|candidate| normalize_media_path(&candidate.path))
        .collect()
}

fn hard_negative_paths_for_case(case: &MediaMatchV3ResolvedManifestCase) -> BTreeSet<String> {
    case.hard_negatives
        .iter()
        .map(|hard_negative| normalize_media_path(&hard_negative.path))
        .collect()
}

fn retrieval_margin_report(
    retrieved_candidates: &[MediaMatchV3RetrievedCandidate],
    expected_candidate_paths: &BTreeSet<String>,
    hard_negative_paths: &BTreeSet<String>,
) -> Option<MediaMatchV3DiagnosticRetrievalMarginReport> {
    if retrieved_candidates.is_empty() {
        return None;
    }
    let expected = retrieved_candidates
        .iter()
        .filter(|candidate| expected_candidate_paths.contains(&candidate.normalized_path))
        .max_by_key(|candidate| candidate.total_score);
    let hard_negative = retrieved_candidates
        .iter()
        .filter(|candidate| hard_negative_paths.contains(&candidate.normalized_path))
        .max_by_key(|candidate| candidate.total_score);
    Some(MediaMatchV3DiagnosticRetrievalMarginReport {
        top1_score: retrieved_candidates
            .first()
            .map(|candidate| candidate.total_score),
        top2_score: retrieved_candidates
            .get(1)
            .map(|candidate| candidate.total_score),
        expected_score: expected.map(|candidate| candidate.total_score),
        best_negative_score: hard_negative.map(|candidate| candidate.total_score),
        expected_best_offset_score: expected.map(|candidate| candidate.best_offset_score),
        best_negative_offset_score: hard_negative.map(|candidate| candidate.best_offset_score),
    })
}

impl MediaMatchV3DiagnosticRetrievalReport {
    fn from_stats(
        stats: MediaMatchV3RetrievalStats,
        retrieved_candidates: Vec<MediaMatchV3RetrievedCandidate>,
        known_candidate_ids: &BTreeMap<String, String>,
        expected_candidate_paths: &BTreeSet<String>,
        hard_negative_paths: &BTreeSet<String>,
    ) -> Self {
        let details = retrieved_candidates
            .iter()
            .map(|candidate| MediaMatchV3DiagnosticRetrievalCandidateReport {
                candidate_id: known_candidate_ids.get(&candidate.normalized_path).cloned(),
                path: candidate.normalized_path.clone(),
                rank: candidate.rank,
                total_score: candidate.total_score,
                best_offset_bin_ms: candidate.best_offset_bin_ms,
                best_offset_score: candidate.best_offset_score,
                second_offset_score: candidate.second_offset_score,
                distinct_query_regions: candidate.distinct_query_regions,
                distinct_candidate_regions: candidate.distinct_candidate_regions,
                body_region_count: candidate.body_region_count,
                edge_region_count: candidate.edge_region_count,
                approximate_span_ms: candidate.approximate_span_ms,
                audio_hits: candidate.audio_hits,
                video_hits: candidate.video_hits,
                score_ratio_to_next: candidate.score_ratio_to_next,
                query_duration_ms: candidate.query_duration_ms,
                candidate_duration_ms: candidate.candidate_duration_ms,
                duration_compatibility: candidate.duration_compatibility.clone(),
                short_clip_penalty_applied: candidate.short_clip_penalty_applied,
                robust_score: candidate.robust_score,
            })
            .collect::<Vec<_>>();
        let retrieved_paths = retrieved_candidates
            .iter()
            .map(|candidate| candidate.normalized_path.clone())
            .collect::<Vec<_>>();
        let correct_candidate_rank = retrieved_candidates
            .iter()
            .filter(|candidate| expected_candidate_paths.contains(&candidate.normalized_path))
            .map(|candidate| candidate.rank)
            .min();
        let hard_negative_best_rank = retrieved_candidates
            .iter()
            .filter(|candidate| hard_negative_paths.contains(&candidate.normalized_path))
            .map(|candidate| candidate.rank)
            .min();
        let hard_negative_count_above_correct = correct_candidate_rank
            .map(|correct_rank| {
                retrieved_candidates
                    .iter()
                    .filter(|candidate| {
                        hard_negative_paths.contains(&candidate.normalized_path)
                            && candidate.rank < correct_rank
                    })
                    .count()
            })
            .unwrap_or_default();
        let top1_is_expected = retrieved_candidates
            .first()
            .is_some_and(|candidate| expected_candidate_paths.contains(&candidate.normalized_path));
        let top_k_expected_present = !expected_candidate_paths.is_empty()
            && expected_candidate_paths
                .iter()
                .all(|path| retrieved_paths.iter().any(|retrieved| retrieved == path));
        let retrieval_margin = retrieval_margin_report(
            &retrieved_candidates,
            expected_candidate_paths,
            hard_negative_paths,
        );
        Self {
            retrieval_strategy: stats.retrieval_strategy,
            query_buckets_total: stats.query_buckets_total,
            query_buckets_skipped_common: stats.query_buckets_skipped_common,
            raw_hit_rows_processed: stats.raw_hit_rows_processed,
            candidates_scored: stats.candidates_scored,
            retrieval_elapsed_ms: stats.retrieval_elapsed_ms,
            retrieval_measured_stage_millis: stats.retrieval_measured_stage_millis,
            retrieval_unaccounted_millis: stats.retrieval_unaccounted_millis,
            stats_dirty_check_millis: stats.stats_dirty_check_millis,
            stats_refresh_millis: stats.stats_refresh_millis,
            query_anchor_load_millis: stats.query_anchor_load_millis,
            common_bucket_filter_millis: stats.common_bucket_filter_millis,
            sql_hit_fetch_millis: stats.sql_hit_fetch_millis,
            temp_table_create_millis: stats.temp_table_create_millis,
            temp_table_insert_millis: stats.temp_table_insert_millis,
            temp_table_index_millis: stats.temp_table_index_millis,
            temp_table_drop_millis: stats.temp_table_drop_millis,
            sql_prepare_millis: stats.sql_prepare_millis,
            sql_execute_millis: stats.sql_execute_millis,
            rust_aggregation_millis: stats.rust_aggregation_millis,
            candidate_metadata_load_millis: stats.candidate_metadata_load_millis,
            robust_rerank_millis: stats.robust_rerank_millis,
            candidate_sort_millis: stats.candidate_sort_millis,
            retrieved_candidate_detail_build_millis: stats.retrieved_candidate_detail_build_millis,
            retrieved_path_load_millis: stats.retrieved_path_load_millis,
            report_candidate_attach_millis: stats.report_candidate_attach_millis,
            path_lookup_millis: stats.path_lookup_millis,
            explain_query_plan_millis: stats.explain_query_plan_millis,
            stats_refresh_ran: stats.stats_refresh_ran,
            stats_buckets_refreshed: stats.stats_buckets_refreshed,
            stats_anchor_rows_scanned: stats.stats_anchor_rows_scanned,
            anchor_stats_dirty_before_run: stats.anchor_stats_dirty_before_run,
            anchor_stats_refreshed: stats.anchor_stats_refreshed,
            anchor_stats_refresh_millis: stats.anchor_stats_refresh_millis,
            anchor_stats_dirty_after_run: stats.anchor_stats_dirty_after_run,
            query_anchor_count: stats.query_anchor_count,
            query_buckets_after_common_skip: stats.query_buckets_after_common_skip,
            sql_rows_returned: stats.sql_rows_returned,
            candidates_aggregated: stats.candidates_aggregated,
            candidates_returned: stats.candidates_returned,
            retrieved_candidates: retrieved_paths,
            retrieved_candidate_details: details,
            correct_candidate_rank,
            hard_negative_best_rank,
            hard_negative_count_above_correct,
            top1_is_expected,
            top_k_expected_present,
            retrieval_margin,
        }
    }
}

impl MediaMatchV3DiagnosticDecisionReport {
    fn from_decision(decision: &MediaMatchDecision, settings: &MediaMatchSettings) -> Self {
        let map = decision.evidence.timeline_map_v3.as_ref();
        let summary = summarize_decision_v3_diagnostics(decision);
        Self {
            tier: format!("{:?}", decision.tier),
            class: decision.evidence.v3_class.map(|class| format!("{class:?}")),
            explanation: decision.explanation.clone(),
            autoplay_eligible: decision.same_media_for_autoplay(settings),
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
            decision_pair_collection_millis: summary.decision_pair_collection_millis,
            fast_audio_verifier_millis: summary.fast_audio_verifier_millis,
            global_fit_millis: summary.global_fit_millis,
            offset_histogram_millis: summary.offset_histogram_millis,
            fast_global_fit_millis: summary.fast_global_fit_millis,
            broad_global_fit_millis: summary.broad_global_fit_millis,
            global_fit_candidate_count: summary.global_fit_candidate_count,
            global_fit_inlier_count: summary.global_fit_inlier_count,
            global_fit_fallback_used: summary.global_fit_fallback_used,
            timeline_map_millis: summary.timeline_map_millis,
            evidence_formatting_millis: summary.evidence_formatting_millis,
            total_decision_millis: summary.total_decision_millis,
        }
    }
}

fn retrieval_benchmark_decision_report() -> MediaMatchV3DiagnosticDecisionReport {
    MediaMatchV3DiagnosticDecisionReport {
        tier: "Skipped".to_owned(),
        class: None,
        explanation: "retrieval benchmark only; direct media-match decision skipped".to_owned(),
        autoplay_eligible: false,
        offset_seconds: None,
        scale_ppm: None,
        segment_count: 0,
        total_aligned_span_ms: 0,
        largest_gap_ms: 0,
        edge_only: false,
        audio_video_conflict: false,
        piecewise_pair_count: None,
        piecewise_hypothesis_count: None,
        piecewise_fit_millis: None,
        decision_pair_collection_millis: None,
        fast_audio_verifier_millis: None,
        global_fit_millis: None,
        offset_histogram_millis: None,
        fast_global_fit_millis: None,
        broad_global_fit_millis: None,
        global_fit_candidate_count: None,
        global_fit_inlier_count: None,
        global_fit_fallback_used: None,
        timeline_map_millis: None,
        evidence_formatting_millis: None,
        total_decision_millis: None,
    }
}

fn fingerprint_diagnostic_manifest_records(
    resolved: &MediaMatchV3ResolvedManifest,
    connection: &Connection,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
    refresh_cache: bool,
    options: &MediaMatchV3DiagnosticRunOptions,
) -> Result<DiagnosticFingerprintRecords, String> {
    let occurrences = diagnostic_fingerprint_occurrences(resolved);
    let settings_hash = media_extraction_settings_hash(settings);
    let mut cache = BTreeMap::<(String, [u8; 32]), CachedFingerprint>::new();
    let mut occurrence_cache = BTreeMap::<(usize, usize), CachedFingerprint>::new();
    let mut fresh_jobs = Vec::new();
    let mut pending_occurrences = BTreeMap::<(String, [u8; 32]), Vec<(usize, usize)>>::new();
    let mut stats = DiagnosticFingerprintBatchStats::default();

    for occurrence in &occurrences {
        let normalized_path = normalize_media_path(&occurrence.path);
        let cache_key = (normalized_path.clone(), settings_hash);
        if let Some(fingerprint) = cache.get(&cache_key) {
            occurrence_cache.insert(
                occurrence.occurrence_key,
                CachedFingerprint {
                    fingerprint: fingerprint.fingerprint.clone(),
                    source: FINGERPRINT_SOURCE_MEMORY_CACHE,
                    sqlite_load_millis: 0,
                    save_stats: MediaMatchV3SaveStats::default(),
                },
            );
            continue;
        }

        let (modified_unix_millis, size_bytes) = media_file_identity_parts(&occurrence.path)?;
        let sqlite_load_started_at = Instant::now();
        if !refresh_cache
            && let Some(record) = load_media_match_v3_record_for_path(
                connection,
                &normalized_path,
                settings,
                modified_unix_millis,
                size_bytes,
            )?
        {
            let sqlite_load_millis = sqlite_load_started_at.elapsed().as_millis();
            let fingerprint = InstrumentedMediaFingerprint {
                record,
                report: Default::default(),
            };
            let cached = CachedFingerprint {
                fingerprint,
                source: FINGERPRINT_SOURCE_SQLITE_CACHE,
                sqlite_load_millis,
                save_stats: MediaMatchV3SaveStats::default(),
            };
            cache.insert(cache_key, cached.clone());
            occurrence_cache.insert(occurrence.occurrence_key, cached);
            stats.files_skipped_from_cache += 1;
            stats.sqlite_cache_compatible_hit_count += 1;
            continue;
        }

        if !pending_occurrences.contains_key(&cache_key) {
            if !refresh_cache {
                let incompatible_count = media_match_v3_incompatible_record_count_for_path(
                    connection,
                    &normalized_path,
                    settings,
                    modified_unix_millis,
                    size_bytes,
                )?;
                if incompatible_count > 0 {
                    stats.sqlite_cache_incompatible_miss_count += 1;
                    if settings.audio_index_mode.is_sampled() {
                        stats.sampled_policy_mismatch_count += 1;
                    }
                }
            }
            let source_info = media_source_path_info(&occurrence.path);
            let source_worker_limit = source_worker_limit(&source_info.kind, settings, options);
            fresh_jobs.push(DiagnosticFreshFingerprintJob {
                path: occurrence.path.clone(),
                normalized_path: normalized_path.clone(),
                settings_hash,
                queued_at: Instant::now(),
                source_key: source_info.root,
                source_kind: source_info.kind,
                source_worker_limit,
            });
        }
        pending_occurrences
            .entry(cache_key)
            .or_default()
            .push(occurrence.occurrence_key);
    }

    if !fresh_jobs.is_empty() {
        let fresh =
            fingerprint_fresh_diagnostic_jobs(connection, tools, settings, fresh_jobs, options)?;
        stats.worker_count = fresh.worker_count;
        stats.extraction_queue_wait_millis = fresh.extraction_queue_wait_millis;
        stats.extraction_worker_wall_millis = fresh.extraction_worker_wall_millis;
        stats.sqlite_writer_millis = fresh.sqlite_writer_millis;
        stats.sqlite_write_queue_depth_max = fresh.sqlite_write_queue_depth_max;
        stats.producer_worker_count = fresh.producer_worker_count;
        stats.writer_thread_enabled = fresh.writer_thread_enabled;
        stats.result_queue_depth_max = fresh.result_queue_depth_max;
        stats.result_queue_wait_millis = fresh.result_queue_wait_millis;
        stats.writer_idle_millis = fresh.writer_idle_millis;
        stats.writer_busy_millis = fresh.writer_busy_millis;
        stats.writer_batch_count = fresh.writer_batch_count;
        stats.writer_rows_inserted = fresh.writer_rows_inserted;
        stats.writer_records_inserted = fresh.writer_records_inserted;
        stats.writer_commit_millis = fresh.writer_commit_millis;
        stats.extraction_worker_idle_millis = fresh.extraction_worker_idle_millis;
        stats.extraction_worker_active_millis = fresh.extraction_worker_active_millis;
        stats.end_to_end_index_wall_millis = fresh.end_to_end_index_wall_millis;
        stats.fresh_timings = fresh.fresh_timings;
        stats.files_indexed = fresh.files_indexed;
        stats.files_failed = fresh.files_failed;
        for (key, fingerprint) in fresh.cache {
            cache.insert(key, fingerprint);
        }
    }

    let mut fresh_first_occurrence_assigned = BTreeSet::new();
    for occurrence in occurrences {
        if occurrence_cache.contains_key(&occurrence.occurrence_key) {
            continue;
        }
        let normalized_path = normalize_media_path(&occurrence.path);
        let cache_key = (normalized_path, settings_hash);
        let fingerprint = cache
            .get(&cache_key)
            .ok_or_else(|| {
                format!(
                    "missing diagnostic fingerprint for '{}'",
                    occurrence.path.display()
                )
            })?
            .clone();
        let source = if pending_occurrences.contains_key(&cache_key)
            && fresh_first_occurrence_assigned.insert(cache_key)
        {
            fingerprint.source
        } else {
            FINGERPRINT_SOURCE_MEMORY_CACHE
        };
        let sqlite_load_millis = if source == fingerprint.source {
            fingerprint.sqlite_load_millis
        } else {
            0
        };
        let save_stats = if source == fingerprint.source {
            fingerprint.save_stats
        } else {
            MediaMatchV3SaveStats::default()
        };
        occurrence_cache.insert(
            occurrence.occurrence_key,
            CachedFingerprint {
                fingerprint: fingerprint.fingerprint,
                source,
                sqlite_load_millis,
                save_stats,
            },
        );
    }

    Ok(DiagnosticFingerprintRecords {
        cache,
        occurrence_cache,
        stats,
    })
}

fn diagnostic_fingerprint_occurrences(
    resolved: &MediaMatchV3ResolvedManifest,
) -> Vec<DiagnosticFingerprintOccurrence> {
    let mut occurrences = Vec::new();
    for (case_index, case) in resolved.cases.iter().enumerate() {
        occurrences.push(DiagnosticFingerprintOccurrence {
            occurrence_key: (case_index, 0),
            path: case.query.clone(),
        });
        for (candidate_index, candidate) in case.candidates.iter().enumerate() {
            occurrences.push(DiagnosticFingerprintOccurrence {
                occurrence_key: (case_index, candidate_index + 1),
                path: candidate.path.clone(),
            });
        }
        for (hard_negative_index, hard_negative) in case.hard_negatives.iter().enumerate() {
            occurrences.push(DiagnosticFingerprintOccurrence {
                occurrence_key: (case_index, case.candidates.len() + hard_negative_index + 1),
                path: hard_negative.path.clone(),
            });
        }
    }
    occurrences
}

struct DiagnosticFreshFingerprintBatch {
    cache: BTreeMap<(String, [u8; 32]), CachedFingerprint>,
    worker_count: usize,
    extraction_queue_wait_millis: u128,
    extraction_worker_wall_millis: u128,
    sqlite_writer_millis: u128,
    sqlite_write_queue_depth_max: usize,
    producer_worker_count: usize,
    writer_thread_enabled: bool,
    result_queue_depth_max: usize,
    result_queue_wait_millis: u128,
    writer_idle_millis: u128,
    writer_busy_millis: u128,
    writer_batch_count: usize,
    writer_rows_inserted: usize,
    writer_records_inserted: usize,
    writer_commit_millis: u128,
    extraction_worker_idle_millis: u128,
    extraction_worker_active_millis: u128,
    end_to_end_index_wall_millis: u128,
    fresh_timings: Vec<DiagnosticFreshFingerprintTiming>,
    files_indexed: usize,
    files_failed: usize,
}

struct DiagnosticJobQueue {
    jobs: VecDeque<DiagnosticFreshFingerprintJob>,
    active_by_source: BTreeMap<String, usize>,
}

enum DiagnosticJobQueuePop {
    Job(DiagnosticFreshFingerprintJob),
    Blocked,
    Empty,
}

impl DiagnosticJobQueue {
    fn new(jobs: Vec<DiagnosticFreshFingerprintJob>) -> Self {
        Self {
            jobs: VecDeque::from(jobs),
            active_by_source: BTreeMap::new(),
        }
    }

    fn pop_available(&mut self) -> DiagnosticJobQueuePop {
        if self.jobs.is_empty() {
            return DiagnosticJobQueuePop::Empty;
        }
        for index in 0..self.jobs.len() {
            let Some(job) = self.jobs.get(index) else {
                continue;
            };
            let active = self
                .active_by_source
                .get(&job.source_key)
                .copied()
                .unwrap_or_default();
            if active < job.source_worker_limit.max(1) {
                let job = self.jobs.remove(index).expect("job index should exist");
                *self
                    .active_by_source
                    .entry(job.source_key.clone())
                    .or_default() += 1;
                return DiagnosticJobQueuePop::Job(job);
            }
        }
        DiagnosticJobQueuePop::Blocked
    }

    fn finish_source(&mut self, source_key: &str) {
        if let Some(active) = self.active_by_source.get_mut(source_key) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                self.active_by_source.remove(source_key);
            }
        }
    }
}

fn fingerprint_fresh_diagnostic_jobs(
    connection: &Connection,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
    jobs: Vec<DiagnosticFreshFingerprintJob>,
    options: &MediaMatchV3DiagnosticRunOptions,
) -> Result<DiagnosticFreshFingerprintBatch, String> {
    let end_to_end_started_at = Instant::now();
    if jobs.is_empty() {
        return Ok(DiagnosticFreshFingerprintBatch {
            cache: BTreeMap::new(),
            worker_count: 0,
            extraction_queue_wait_millis: 0,
            extraction_worker_wall_millis: 0,
            sqlite_writer_millis: 0,
            sqlite_write_queue_depth_max: 0,
            producer_worker_count: 0,
            writer_thread_enabled: false,
            result_queue_depth_max: 0,
            result_queue_wait_millis: 0,
            writer_idle_millis: 0,
            writer_busy_millis: 0,
            writer_batch_count: 0,
            writer_rows_inserted: 0,
            writer_records_inserted: 0,
            writer_commit_millis: 0,
            extraction_worker_idle_millis: 0,
            extraction_worker_active_millis: 0,
            end_to_end_index_wall_millis: 0,
            fresh_timings: Vec::new(),
            files_indexed: 0,
            files_failed: 0,
        });
    }
    let worker_count = options
        .sampled_fast_global_workers
        .unwrap_or_else(|| diagnostic_extraction_worker_count(settings))
        .clamp(1, jobs.len());
    let mut cache = BTreeMap::new();
    let mut extraction_queue_wait_millis = 0u128;
    let mut extraction_worker_wall_millis = 0u128;
    let mut sqlite_writer_millis = 0u128;
    let mut writer_idle_millis = 0u128;
    let mut writer_busy_millis = 0u128;
    let mut writer_commit_millis = 0u128;
    let mut writer_batch_count = 0usize;
    let mut writer_rows_inserted = 0usize;
    let mut writer_records_inserted = 0usize;
    let mut fresh_timings = Vec::new();
    let mut files_indexed = 0;
    let mut files_failed = 0;
    let mut errors = Vec::new();
    let result_queue_wait_millis = Arc::new(AtomicU64::new(0));
    let result_queue_depth = Arc::new(AtomicUsize::new(0));
    let result_queue_depth_max = Arc::new(AtomicUsize::new(0));
    let extraction_worker_idle_millis = Arc::new(AtomicU64::new(0));

    thread::scope(|scope| {
        let queue_capacity = worker_count.saturating_mul(2).max(1);
        let jobs = Arc::new(Mutex::new(DiagnosticJobQueue::new(jobs)));
        let (tx, rx) = mpsc::sync_channel(queue_capacity);
        for _ in 0..worker_count {
            let jobs = Arc::clone(&jobs);
            let tx = tx.clone();
            let tools = tools.clone();
            let settings = settings.clone();
            let result_queue_wait_millis = Arc::clone(&result_queue_wait_millis);
            let result_queue_depth = Arc::clone(&result_queue_depth);
            let result_queue_depth_max = Arc::clone(&result_queue_depth_max);
            let extraction_worker_idle_millis = Arc::clone(&extraction_worker_idle_millis);
            scope.spawn(move || {
                loop {
                    let dequeue_started_at = Instant::now();
                    let queue_pop = jobs
                        .lock()
                        .map(|mut jobs| jobs.pop_available())
                        .unwrap_or(DiagnosticJobQueuePop::Empty);
                    extraction_worker_idle_millis.fetch_add(
                        saturating_u128_to_u64(dequeue_started_at.elapsed().as_millis()),
                        Ordering::Relaxed,
                    );
                    let job = match queue_pop {
                        DiagnosticJobQueuePop::Job(job) => job,
                        DiagnosticJobQueuePop::Blocked => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                        DiagnosticJobQueuePop::Empty => break,
                    };
                    let queue_wait_millis = job.queued_at.elapsed().as_millis();
                    let worker_started_at = Instant::now();
                    let source_key = job.source_key.clone();
                    let extraction_options = MediaFingerprintExtractionOptions {
                        sampled_audio_source: options.sampled_audio_source,
                        sampled_pcm_cache_root: options.sampled_pcm_cache_root.clone(),
                    };
                    let result = fingerprint_media_file_with_report_and_options(
                        &job.path,
                        &tools,
                        &settings,
                        None,
                        &extraction_options,
                    )
                    .map_err(|error| {
                        format!("failed fingerprinting '{}': {error}", job.path.display())
                    });
                    if let Ok(mut jobs) = jobs.lock() {
                        jobs.finish_source(&source_key);
                    }
                    let output = DiagnosticFreshFingerprintOutput {
                        normalized_path: job.normalized_path,
                        path: job.path,
                        settings_hash: job.settings_hash,
                        source_key: job.source_key,
                        source_kind: job.source_kind,
                        source_worker_limit: job.source_worker_limit,
                        queue_wait_millis,
                        worker_wall_millis: worker_started_at.elapsed().as_millis(),
                        result,
                    };
                    let queued = result_queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
                    update_atomic_max(&result_queue_depth_max, queued);
                    let send_started_at = Instant::now();
                    if tx.send(output).is_err() {
                        result_queue_depth.fetch_sub(1, Ordering::Relaxed);
                        break;
                    }
                    result_queue_wait_millis.fetch_add(
                        saturating_u128_to_u64(send_started_at.elapsed().as_millis()),
                        Ordering::Relaxed,
                    );
                }
            });
        }
        drop(tx);

        loop {
            let receive_started_at = Instant::now();
            let output = match rx.recv() {
                Ok(output) => output,
                Err(_) => break,
            };
            writer_idle_millis =
                writer_idle_millis.saturating_add(receive_started_at.elapsed().as_millis());
            result_queue_depth.fetch_sub(1, Ordering::Relaxed);
            extraction_queue_wait_millis =
                extraction_queue_wait_millis.saturating_add(output.queue_wait_millis);
            extraction_worker_wall_millis =
                extraction_worker_wall_millis.saturating_add(output.worker_wall_millis);
            match output.result {
                Ok(mut fingerprint) => {
                    if options.probe_audio_packets && settings.audio_index_mode.is_sampled() {
                        match probe_audio_packet_positions_for_sampled_windows(
                            &tools.ffprobe,
                            &output.path,
                            fingerprint.record.duration_seconds,
                            settings.audio_index_mode,
                            fingerprint.report.audio_stream.ffmpeg_input_read_bytes,
                            None,
                        ) {
                            Ok(packet_metrics) => merge_packet_probe_metrics(
                                &mut fingerprint.report.audio_stream,
                                packet_metrics,
                            ),
                            Err(error) => {
                                fingerprint
                                    .report
                                    .audio_stream
                                    .audio_packet_positions_available = Some(false);
                                fingerprint
                                    .report
                                    .audio_stream
                                    .ffmpeg_command_kind
                                    .get_or_insert_with(|| {
                                        format!("audio-only-pcm packet-probe-error={error}")
                                    });
                            }
                        };
                    }
                    let diagnostics = summarize_instrumented_record_v3_diagnostics(&fingerprint);
                    let rows_inserted = fingerprint
                        .record
                        .audio_anchors
                        .len()
                        .saturating_add(fingerprint.record.video_anchors.len());
                    let save_started_at = Instant::now();
                    match save_media_match_v3_record_with_stats(
                        connection,
                        &fingerprint.record,
                        None,
                    ) {
                        Ok(save_stats) => {
                            let elapsed = save_started_at.elapsed().as_millis();
                            sqlite_writer_millis = sqlite_writer_millis.saturating_add(elapsed);
                            writer_busy_millis = writer_busy_millis.saturating_add(elapsed);
                            writer_commit_millis =
                                writer_commit_millis.saturating_add(save_stats.sqlite_save_millis);
                            writer_batch_count += 1;
                            writer_records_inserted += 1;
                            writer_rows_inserted =
                                writer_rows_inserted.saturating_add(rows_inserted);
                            fresh_timings.push(fresh_timing_for_output(
                                DiagnosticFreshFingerprintTimingContext {
                                    path: &output.path,
                                    source_key: &output.source_key,
                                    source_kind: &output.source_kind,
                                    source_worker_limit: output.source_worker_limit,
                                    queue_wait_millis: output.queue_wait_millis,
                                    worker_wall_millis: output.worker_wall_millis,
                                    status: "complete",
                                },
                                &fingerprint,
                                &diagnostics,
                            ));
                            cache.insert(
                                (output.normalized_path, output.settings_hash),
                                CachedFingerprint {
                                    fingerprint,
                                    source: FINGERPRINT_SOURCE_FRESH,
                                    sqlite_load_millis: 0,
                                    save_stats,
                                },
                            );
                            files_indexed += 1;
                        }
                        Err(error) => {
                            files_failed += 1;
                            fresh_timings.push(failed_fresh_timing_for_output(
                                &output.path,
                                &output.source_key,
                                &output.source_kind,
                                output.source_worker_limit,
                                output.queue_wait_millis,
                                output.worker_wall_millis,
                                format!("save-error: {error}"),
                            ));
                            errors.push(error);
                        }
                    }
                }
                Err(error) => {
                    files_failed += 1;
                    fresh_timings.push(failed_fresh_timing_for_output(
                        &output.path,
                        &output.source_key,
                        &output.source_kind,
                        output.source_worker_limit,
                        output.queue_wait_millis,
                        output.worker_wall_millis,
                        "extract-error".to_owned(),
                    ));
                    errors.push(error);
                }
            }
        }
    });

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let result_queue_depth_max = result_queue_depth_max.load(Ordering::Relaxed);
    Ok(DiagnosticFreshFingerprintBatch {
        cache,
        worker_count,
        extraction_queue_wait_millis,
        extraction_worker_wall_millis,
        sqlite_writer_millis,
        sqlite_write_queue_depth_max: result_queue_depth_max,
        producer_worker_count: worker_count,
        writer_thread_enabled: worker_count > 1,
        result_queue_depth_max,
        result_queue_wait_millis: u128::from(result_queue_wait_millis.load(Ordering::Relaxed)),
        writer_idle_millis,
        writer_busy_millis,
        writer_batch_count,
        writer_rows_inserted,
        writer_records_inserted,
        writer_commit_millis,
        extraction_worker_idle_millis: u128::from(
            extraction_worker_idle_millis.load(Ordering::Relaxed),
        ),
        extraction_worker_active_millis: extraction_worker_wall_millis,
        end_to_end_index_wall_millis: end_to_end_started_at.elapsed().as_millis(),
        fresh_timings,
        files_indexed,
        files_failed,
    })
}

fn merge_packet_probe_metrics(
    target: &mut MediaAudioStreamMetrics,
    source: MediaAudioStreamMetrics,
) {
    if target.container_format.is_none() {
        target.container_format = source.container_format;
    }
    if target.audio_stream_index.is_none() {
        target.audio_stream_index = source.audio_stream_index;
    }
    if target.audio_codec.is_none() {
        target.audio_codec = source.audio_codec;
    }
    if target.audio_bitrate_bps.is_none() {
        target.audio_bitrate_bps = source.audio_bitrate_bps;
    }
    if target.audio_duration_millis.is_none() {
        target.audio_duration_millis = source.audio_duration_millis;
    }
    if target.audio_start_time_millis.is_none() {
        target.audio_start_time_millis = source.audio_start_time_millis;
    }
    if target.audio_packet_positions_available.is_none() {
        target.audio_packet_positions_available = source.audio_packet_positions_available;
    }
    if target
        .audio_packet_position_completeness_per_mille
        .is_none()
    {
        target.audio_packet_position_completeness_per_mille =
            source.audio_packet_position_completeness_per_mille;
    }
    if target.audio_packet_positions_monotonic.is_none() {
        target.audio_packet_positions_monotonic = source.audio_packet_positions_monotonic;
    }
    if target.average_audio_packet_size_bytes.is_none() {
        target.average_audio_packet_size_bytes = source.average_audio_packet_size_bytes;
    }
    if target.audio_packet_count_in_sampled_windows.is_none() {
        target.audio_packet_count_in_sampled_windows = source.audio_packet_count_in_sampled_windows;
    }
    if target.audio_packet_probe_millis.is_none() {
        target.audio_packet_probe_millis = source.audio_packet_probe_millis;
    }
    if target.audio_packet_probe_read_bytes.is_none() {
        target.audio_packet_probe_read_bytes = source.audio_packet_probe_read_bytes;
    }
    if target.audio_packet_window_compressed_bytes.is_none() {
        target.audio_packet_window_compressed_bytes = source.audio_packet_window_compressed_bytes;
    }
    if target.audio_packet_window_coalesced_range_bytes.is_none() {
        target.audio_packet_window_coalesced_range_bytes =
            source.audio_packet_window_coalesced_range_bytes;
    }
    if target.audio_packet_read_savings_estimate_bytes.is_none() {
        target.audio_packet_read_savings_estimate_bytes =
            source.audio_packet_read_savings_estimate_bytes;
    }
}

fn fresh_timing_for_output(
    context: DiagnosticFreshFingerprintTimingContext<'_>,
    fingerprint: &InstrumentedMediaFingerprint,
    diagnostics: &MediaMatchV3DiagnosticSummary,
) -> DiagnosticFreshFingerprintTiming {
    DiagnosticFreshFingerprintTiming {
        path: fingerprint.record.identity.normalized_path.clone(),
        source_path_root: diagnostics
            .source_path_root
            .clone()
            .or_else(|| Some(context.source_key.to_owned())),
        source_path_kind: diagnostics
            .source_path_kind
            .clone()
            .or_else(|| Some(context.source_kind.to_owned())),
        source_volume_id: diagnostics.source_volume_id.clone(),
        source_worker_limit: context.source_worker_limit,
        duration_ms: diagnostics.duration_ms,
        size_bytes: fingerprint.record.identity.size_bytes,
        extraction_total_millis: diagnostics
            .extraction_total_millis
            .unwrap_or(context.worker_wall_millis),
        queue_wait_millis: context.queue_wait_millis,
        ffmpeg_process_wall_millis: diagnostics.ffmpeg_process_wall_millis,
        ffmpeg_input_read_bytes: diagnostics.ffmpeg_input_read_bytes,
        ffmpeg_input_read_ops: diagnostics.ffmpeg_input_read_ops,
        ffmpeg_output_pcm_bytes: diagnostics.ffmpeg_output_pcm_bytes,
        read_amplification_ratio: diagnostics.read_amplification_ratio,
        ffmpeg_invocation_count: diagnostics.ffmpeg_invocation_count,
        sampled_window_seek_millis: diagnostics.sampled_window_seek_millis,
        sampled_window_decode_millis: diagnostics.sampled_window_decode_millis,
        analyzer_millis: diagnostics.analyzer_millis,
        sampled_audio_seconds_decoded: diagnostics.sampled_audio_seconds_decoded,
        sampled_audio_windows_decoded: diagnostics.sampled_audio_windows_decoded,
        status: context.status.to_owned(),
    }
    .with_fallback_path(context.path)
}

fn failed_fresh_timing_for_output(
    path: &Path,
    source_key: &str,
    source_kind: &str,
    source_worker_limit: usize,
    queue_wait_millis: u128,
    worker_wall_millis: u128,
    status: String,
) -> DiagnosticFreshFingerprintTiming {
    DiagnosticFreshFingerprintTiming {
        path: normalize_media_path(path),
        source_path_root: Some(source_key.to_owned()),
        source_path_kind: Some(source_kind.to_owned()),
        source_volume_id: Some(source_key.to_owned()),
        source_worker_limit,
        duration_ms: None,
        size_bytes: fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        extraction_total_millis: worker_wall_millis,
        queue_wait_millis,
        ffmpeg_process_wall_millis: None,
        ffmpeg_input_read_bytes: None,
        ffmpeg_input_read_ops: None,
        ffmpeg_output_pcm_bytes: None,
        read_amplification_ratio: None,
        ffmpeg_invocation_count: None,
        sampled_window_seek_millis: None,
        sampled_window_decode_millis: None,
        analyzer_millis: None,
        sampled_audio_seconds_decoded: None,
        sampled_audio_windows_decoded: None,
        status,
    }
}

impl DiagnosticFreshFingerprintTiming {
    fn with_fallback_path(mut self, path: &Path) -> Self {
        if self.path.is_empty() {
            self.path = normalize_media_path(path);
        }
        self
    }
}

fn update_atomic_max(target: &AtomicUsize, value: usize) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn saturating_u128_to_u64(value: u128) -> u64 {
    value.min(u128::from(u64::MAX)) as u64
}

fn diagnostic_extraction_worker_count(settings: &MediaExtractionSettings) -> usize {
    let cores = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    if settings.audio_index_mode.is_sampled() {
        cores.clamp(1, 8)
    } else if settings.audio_index_mode.is_dense_full_verify() {
        (cores / 2).clamp(1, 4)
    } else {
        cores.clamp(1, 4)
    }
}

fn source_worker_limit(
    source_kind: &str,
    settings: &MediaExtractionSettings,
    options: &MediaMatchV3DiagnosticRunOptions,
) -> usize {
    if !settings.audio_index_mode.is_sampled() {
        return diagnostic_extraction_worker_count(settings).max(1);
    }
    match source_kind {
        "local" => options
            .sampled_fast_per_local_source_workers
            .or(options.sampled_fast_global_workers)
            .unwrap_or_else(|| diagnostic_extraction_worker_count(settings)),
        "network" => options.sampled_fast_per_network_source_workers.unwrap_or(2),
        "removable" => options
            .sampled_fast_per_removable_source_workers
            .unwrap_or(1),
        _ => options
            .sampled_fast_per_network_source_workers
            .or(options.sampled_fast_per_removable_source_workers)
            .unwrap_or(2),
    }
    .max(1)
}

fn fingerprint_cached(
    cache: &mut BTreeMap<(String, [u8; 32]), CachedFingerprint>,
    connection: &Connection,
    path: &Path,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
    refresh_cache: bool,
) -> Result<CachedFingerprint, String> {
    let normalized_path = normalize_media_path(path);
    let settings_hash = media_extraction_settings_hash(settings);
    let cache_key = (normalized_path.clone(), settings_hash);
    if let Some(fingerprint) = cache.get(&cache_key) {
        return Ok(CachedFingerprint {
            fingerprint: fingerprint.fingerprint.clone(),
            source: FINGERPRINT_SOURCE_MEMORY_CACHE,
            sqlite_load_millis: 0,
            save_stats: MediaMatchV3SaveStats::default(),
        });
    }
    let (modified_unix_millis, size_bytes) = media_file_identity_parts(path)?;
    let sqlite_load_started_at = Instant::now();
    if !refresh_cache
        && let Some(record) = load_media_match_v3_record_for_path(
            connection,
            &normalized_path,
            settings,
            modified_unix_millis,
            size_bytes,
        )?
    {
        let sqlite_load_millis = sqlite_load_started_at.elapsed().as_millis();
        let fingerprint = InstrumentedMediaFingerprint {
            record,
            report: Default::default(),
        };
        cache.insert(
            cache_key,
            CachedFingerprint {
                fingerprint: fingerprint.clone(),
                source: FINGERPRINT_SOURCE_SQLITE_CACHE,
                sqlite_load_millis,
                save_stats: MediaMatchV3SaveStats::default(),
            },
        );
        return Ok(CachedFingerprint {
            fingerprint,
            source: FINGERPRINT_SOURCE_SQLITE_CACHE,
            sqlite_load_millis,
            save_stats: MediaMatchV3SaveStats::default(),
        });
    }
    let fingerprint = fingerprint_media_file_with_report(path, tools, settings, None)
        .map_err(|error| format!("failed fingerprinting '{}': {error}", path.display()))?;
    cache.insert(
        cache_key,
        CachedFingerprint {
            fingerprint: fingerprint.clone(),
            source: FINGERPRINT_SOURCE_FRESH,
            sqlite_load_millis: 0,
            save_stats: MediaMatchV3SaveStats::default(),
        },
    );
    Ok(CachedFingerprint {
        fingerprint,
        source: FINGERPRINT_SOURCE_FRESH,
        sqlite_load_millis: 0,
        save_stats: MediaMatchV3SaveStats::default(),
    })
}

fn save_fresh_fingerprint_if_needed(
    cache: &mut BTreeMap<(String, [u8; 32]), CachedFingerprint>,
    fingerprint: &mut CachedFingerprint,
    connection: &Connection,
) -> Result<(), String> {
    if fingerprint.source != FINGERPRINT_SOURCE_FRESH
        || fingerprint.save_stats.sqlite_save_millis != 0
    {
        return Ok(());
    }
    let save_stats =
        save_media_match_v3_record_with_stats(connection, &fingerprint.fingerprint.record, None)?;
    fingerprint.save_stats = save_stats;
    let cache_key = (
        fingerprint
            .fingerprint
            .record
            .identity
            .normalized_path
            .clone(),
        media_extraction_settings_hash(&fingerprint.fingerprint.record.extraction_settings),
    );
    if let Some(cached) = cache.get_mut(&cache_key) {
        cached.save_stats = save_stats;
    }
    Ok(())
}

fn cap_sampled_index_decision_if_needed(
    mut decision: MediaMatchDecision,
    index_mode: MediaMatchV3DiagnosticIndexMode,
) -> MediaMatchDecision {
    if !matches!(
        index_mode,
        MediaMatchV3DiagnosticIndexMode::SampledFast
            | MediaMatchV3DiagnosticIndexMode::SampledNormal
    ) {
        return decision;
    }
    if decision.tier == MediaMatchTier::Strong {
        decision.tier = MediaMatchTier::Probable;
        if decision.evidence.v3_class == Some(MatchClassV3::SameCutStrong) {
            decision.evidence.v3_class = Some(MatchClassV3::SameCutProbable);
        }
        if let Some(map) = &mut decision.evidence.timeline_map_v3
            && map.global_class == MatchClassV3::SameCutStrong
        {
            map.global_class = MatchClassV3::SameCutProbable;
            map.current_position_class = MatchClassV3::SameCutProbable;
        }
        decision
            .evidence
            .notes
            .push("sampled index mode caps direct decision below Strong; full verification is required for SameCutStrong autoplay".to_owned());
        decision.explanation = format!(
            "{}; sampled index requires full verification for Strong",
            decision.explanation
        );
    }
    decision
}

fn cap_sampled_record_decision_if_needed(
    decision: MediaMatchDecision,
    query: &crate::MediaFingerprintRecord,
    candidate: &crate::MediaFingerprintRecord,
) -> MediaMatchDecision {
    if query.extraction_settings.audio_index_mode.is_sampled()
        || candidate.extraction_settings.audio_index_mode.is_sampled()
    {
        cap_sampled_index_decision_if_needed(decision, MediaMatchV3DiagnosticIndexMode::SampledFast)
    } else {
        decision
    }
}

fn sampled_then_full_promotion_reason(
    expected: &MediaMatchV3DiagnosticExpectation,
    retrieval_rank: Option<usize>,
    options: &MediaMatchV3DiagnosticRunOptions,
) -> Option<String> {
    if options.promote_expected_candidates {
        return Some("expected-candidate".to_owned());
    }
    let max_promotions = options.max_full_promotions_per_query.max(1);
    match retrieval_rank {
        Some(rank) if rank <= max_promotions => Some(format!("retrieval-rank-{rank}")),
        _ if expected.autoplay_eligible == Some(true) && retrieval_rank == Some(1) => {
            Some("autoplay-expected-top-candidate".to_owned())
        }
        _ => None,
    }
}

fn diagnostics_for_cached_fingerprint(
    fingerprint: &CachedFingerprint,
) -> MediaMatchV3DiagnosticSummary {
    if fingerprint.source == FINGERPRINT_SOURCE_FRESH {
        summarize_instrumented_record_v3_diagnostics(&fingerprint.fingerprint)
    } else {
        summarize_record_v3_diagnostics(&fingerprint.fingerprint.record)
    }
}

fn increment_report_source_count(summary: &mut MediaMatchV3DiagnosticSummaryReport, source: &str) {
    match source {
        FINGERPRINT_SOURCE_FRESH => summary.fresh_fingerprint_report_count += 1,
        FINGERPRINT_SOURCE_MEMORY_CACHE => summary.memory_cache_fingerprint_report_count += 1,
        FINGERPRINT_SOURCE_SQLITE_CACHE => summary.sqlite_cache_fingerprint_report_count += 1,
        _ => {}
    }
}

fn media_file_identity_parts(path: &Path) -> Result<(u64, u64), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed reading metadata for '{}': {error}", path.display()))?;
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    Ok((modified_unix_millis, metadata.len()))
}

fn evaluate_retrieval_benchmark_expectation(
    expected: &MediaMatchV3DiagnosticExpectation,
    retrieved: bool,
    retrieval_rank: Option<usize>,
    within_promotion_budget: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected_retrieved) = expected.expected_retrieved
        && retrieved != expected_retrieved
    {
        failures.push(format!(
            "expected retrieved={expected_retrieved}, got {retrieved}"
        ));
    }
    if let Some(max_retrieval_rank) = expected.max_retrieval_rank {
        match retrieval_rank {
            Some(rank) if rank <= max_retrieval_rank => {}
            Some(rank) => failures.push(format!(
                "expected retrieval rank <= {max_retrieval_rank}, got {rank}"
            )),
            None => failures.push(format!(
                "expected retrieval rank <= {max_retrieval_rank}, but candidate was absent"
            )),
        }
    }
    if expected.must_be_retrieved && !retrieved {
        failures.push("expected candidate to be retrieved, but it was absent".to_owned());
    }
    if let Some(max_promotion_rank) = expected.max_promotion_rank {
        match retrieval_rank {
            Some(rank) if rank <= max_promotion_rank => {}
            Some(rank) => failures.push(format!(
                "expected promotion rank <= {max_promotion_rank}, got {rank}"
            )),
            None => failures.push(format!(
                "expected promotion rank <= {max_promotion_rank}, but candidate was absent"
            )),
        }
    }
    if expected.expect_within_promotion_budget && !within_promotion_budget {
        failures.push("expected candidate within promotion budget".to_owned());
    }
    failures
}

fn evaluate_diagnostic_expectation(
    decision: &MediaMatchDecision,
    expected: &MediaMatchV3DiagnosticExpectation,
    autoplay_settings: &MediaMatchSettings,
    retrieved: bool,
    retrieval_rank: Option<usize>,
    within_promotion_budget: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected_retrieved) = expected.expected_retrieved
        && retrieved != expected_retrieved
    {
        failures.push(format!(
            "expected retrieved={expected_retrieved}, got {retrieved}"
        ));
    }
    if let Some(max_retrieval_rank) = expected.max_retrieval_rank {
        match retrieval_rank {
            Some(rank) if rank <= max_retrieval_rank => {}
            Some(rank) => failures.push(format!(
                "expected retrieval rank <= {max_retrieval_rank}, got {rank}"
            )),
            None => failures.push(format!(
                "expected retrieval rank <= {max_retrieval_rank}, but candidate was absent"
            )),
        }
    }
    if expected.must_be_retrieved && !retrieved {
        failures.push("expected candidate to be retrieved, but it was absent".to_owned());
    }
    if let Some(max_promotion_rank) = expected.max_promotion_rank {
        match retrieval_rank {
            Some(rank) if rank <= max_promotion_rank => {}
            Some(rank) => failures.push(format!(
                "expected promotion rank <= {max_promotion_rank}, got {rank}"
            )),
            None => failures.push(format!(
                "expected promotion rank <= {max_promotion_rank}, but candidate was absent"
            )),
        }
    }
    if expected.expect_within_promotion_budget && !within_promotion_budget {
        failures.push("expected candidate within promotion budget".to_owned());
    }
    if expected.skip_decision_expectation {
        return failures;
    }
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
    failures
}

fn evaluate_hard_negative_expectation(
    expected: &MediaMatchV3DiagnosticHardNegative,
    retrieval_rank: Option<usize>,
    positive_rank_by_id: &BTreeMap<String, Option<usize>>,
) -> Vec<String> {
    let mut failures = Vec::new();
    if expected.must_not_be_top_rank && retrieval_rank == Some(1) {
        failures.push("hard negative was retrieved at rank 1".to_owned());
    }
    if let Some(target_id) = expected.must_not_beat_candidate_id.as_deref() {
        match (
            retrieval_rank,
            positive_rank_by_id.get(target_id).copied().flatten(),
        ) {
            (Some(hard_negative_rank), Some(target_rank)) if hard_negative_rank < target_rank => {
                failures.push(format!(
                    "hard negative rank {hard_negative_rank} beat candidate id '{target_id}' rank {target_rank}"
                ));
            }
            (Some(hard_negative_rank), None) => {
                failures.push(format!(
                    "hard negative rank {hard_negative_rank} was retrieved but target candidate id '{target_id}' was absent"
                ));
            }
            _ => {}
        }
    }
    failures
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
    use std::time::Duration;

    use super::*;
    use crate::{
        AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION, MediaFileIdentity, MediaFingerprintRecord,
        MediaMatchEvidence, MediaTimelineAlignment, MetadataMatchEvidence,
        identity::container_fingerprint_from_metadata,
    };

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
                  "id": "same-episode-candidate",
                  "path": "candidate.mkv",
                  "expectedClass": "SameCutStrong",
                  "minimumTier": "Strong",
                  "expectedOffsetMs": 5000,
                  "maxOffsetErrorMs": 1000,
                  "autoplayEligible": true,
                  "mustBeRetrieved": true,
                  "expectedRetrieved": true,
                  "maxRetrievalRank": 1,
                  "skipDecisionExpectation": false
                }]
              }]
            }"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.profile, "combined-v3");
        assert_eq!(manifest.base_dir.as_deref(), Some("media"));
        assert_eq!(
            manifest.cases[0].candidates[0].id.as_deref(),
            Some("same-episode-candidate")
        );
        assert_eq!(manifest.cases[0].candidates[0].path, "candidate.mkv");
        assert_eq!(
            manifest.cases[0].candidates[0].expected_offset_ms,
            Some(5000)
        );
        assert!(manifest.cases[0].candidates[0].must_be_retrieved);
        assert_eq!(
            manifest.cases[0].candidates[0].expected_retrieved,
            Some(true)
        );
        assert_eq!(manifest.cases[0].candidates[0].max_retrieval_rank, Some(1));
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
                    id: None,
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                    expected_retrieved: None,
                    max_retrieval_rank: None,
                    max_promotion_rank: None,
                    expect_within_promotion_budget: false,
                    skip_decision_expectation: false,
                }],
                hard_negatives: Vec::new(),
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
    fn manifest_rejects_duplicate_candidate_ids_in_case() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "duplicate".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![
                    test_expectation_with_id("same-id", "candidate-a.mkv"),
                    test_expectation_with_id("same-id", "candidate-b.mkv"),
                ],
                hard_negatives: Vec::new(),
            }],
        };

        let error = resolve_media_match_v3_diagnostic_manifest(
            &manifest,
            &PathBuf::from("C:/manifest-root"),
        )
        .expect_err("duplicate candidate ids should be rejected");

        assert!(error.contains("duplicate candidate id"));
    }

    #[test]
    fn manifest_rejects_blank_candidate_id() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "blank".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![test_expectation_with_id("  ", "candidate.mkv")],
                hard_negatives: Vec::new(),
            }],
        };

        let error = resolve_media_match_v3_diagnostic_manifest(
            &manifest,
            &PathBuf::from("C:/manifest-root"),
        )
        .expect_err("blank candidate id should be rejected");

        assert!(error.contains("blank id"));
    }

    #[test]
    fn manifest_rejects_duplicate_candidate_paths_without_ids() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "paths".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![
                    test_expectation_without_id("candidate.mkv"),
                    test_expectation_without_id("candidate.mkv"),
                ],
                hard_negatives: Vec::new(),
            }],
        };

        let error = resolve_media_match_v3_diagnostic_manifest(
            &manifest,
            &PathBuf::from("C:/manifest-root"),
        )
        .expect_err("duplicate no-id paths should be rejected");

        assert!(error.contains("duplicate candidate path"));
    }

    #[test]
    fn manifest_rejects_duplicate_candidate_and_hard_negative_ids() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "hard-negative".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![test_expectation_with_id("same-id", "candidate.mkv")],
                hard_negatives: vec![MediaMatchV3DiagnosticHardNegative {
                    id: Some("same-id".to_owned()),
                    path: "wrong-episode.mkv".to_owned(),
                    must_not_be_top_rank: true,
                    must_not_beat_candidate_id: Some("same-id".to_owned()),
                }],
            }],
        };

        let error = validate_media_match_v3_diagnostic_manifest(&manifest)
            .expect_err("duplicate candidate/hard-negative ids should be rejected");

        assert!(error.contains("duplicate candidate/hard-negative id"));
    }

    #[test]
    fn manifest_validation_rejects_blank_hard_negative_id() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "hard-negative".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: Vec::new(),
                hard_negatives: vec![MediaMatchV3DiagnosticHardNegative {
                    id: Some(" ".to_owned()),
                    path: "wrong-episode.mkv".to_owned(),
                    must_not_be_top_rank: false,
                    must_not_beat_candidate_id: None,
                }],
            }],
        };

        let error = validate_media_match_v3_diagnostic_manifest(&manifest)
            .expect_err("blank hard-negative id should be rejected");

        assert!(error.contains("blank id"));
    }

    #[test]
    fn retrieval_report_includes_scores_and_hard_negative_margin() {
        let stats = MediaMatchV3RetrievalStats {
            query_buckets_total: 5,
            query_buckets_skipped_common: 1,
            raw_hit_rows_processed: 20,
            candidates_scored: 2,
            retrieval_elapsed_ms: 7,
            sql_rows_returned: 20,
            candidates_returned: 2,
            ..MediaMatchV3RetrievalStats::default()
        };
        let candidates = vec![
            MediaMatchV3RetrievedCandidate {
                normalized_path: "correct.mkv".to_owned(),
                rank: 1,
                total_score: 100,
                best_offset_bin_ms: 0,
                best_offset_score: 70,
                second_offset_score: 10,
                distinct_query_regions: 4,
                distinct_candidate_regions: 4,
                body_region_count: 3,
                edge_region_count: 1,
                approximate_span_ms: 180_000,
                audio_hits: 12,
                video_hits: 0,
                score_ratio_to_next: Some(2.0),
                query_duration_ms: Some(1_500_000),
                candidate_duration_ms: Some(1_500_000),
                duration_compatibility: "compatible".to_owned(),
                short_clip_penalty_applied: false,
                robust_score: 120.0,
            },
            MediaMatchV3RetrievedCandidate {
                normalized_path: "wrong.mkv".to_owned(),
                rank: 2,
                total_score: 50,
                best_offset_bin_ms: 90_000,
                best_offset_score: 30,
                second_offset_score: 4,
                distinct_query_regions: 2,
                distinct_candidate_regions: 2,
                body_region_count: 1,
                edge_region_count: 1,
                approximate_span_ms: 60_000,
                audio_hits: 5,
                video_hits: 0,
                score_ratio_to_next: None,
                query_duration_ms: Some(1_500_000),
                candidate_duration_ms: Some(90_000),
                duration_compatibility: "query-full-candidate-short".to_owned(),
                short_clip_penalty_applied: true,
                robust_score: 8.0,
            },
        ];
        let known_ids = BTreeMap::from([
            ("correct.mkv".to_owned(), "correct-id".to_owned()),
            ("wrong.mkv".to_owned(), "wrong-id".to_owned()),
        ]);
        let expected_paths = BTreeSet::from(["correct.mkv".to_owned()]);
        let hard_negative_paths = BTreeSet::from(["wrong.mkv".to_owned()]);

        let report = MediaMatchV3DiagnosticRetrievalReport::from_stats(
            stats,
            candidates,
            &known_ids,
            &expected_paths,
            &hard_negative_paths,
        );

        assert_eq!(report.retrieved_candidate_details.len(), 2);
        assert_eq!(
            report.retrieved_candidate_details[0]
                .candidate_id
                .as_deref(),
            Some("correct-id")
        );
        assert_eq!(report.correct_candidate_rank, Some(1));
        assert_eq!(report.hard_negative_best_rank, Some(2));
        assert_eq!(report.hard_negative_count_above_correct, 0);
        assert!(report.top1_is_expected);
        assert!(report.top_k_expected_present);
        let margin = report.retrieval_margin.expect("margin should be present");
        assert_eq!(margin.top1_score, Some(100));
        assert_eq!(margin.top2_score, Some(50));
        assert_eq!(margin.expected_score, Some(100));
        assert_eq!(margin.best_negative_score, Some(50));
        assert_eq!(margin.expected_best_offset_score, Some(70));
        assert_eq!(margin.best_negative_offset_score, Some(30));
    }

    #[test]
    fn hard_negative_expectation_fails_when_it_beats_target_candidate() {
        let expected = MediaMatchV3DiagnosticHardNegative {
            id: Some("wrong".to_owned()),
            path: "wrong.mkv".to_owned(),
            must_not_be_top_rank: true,
            must_not_beat_candidate_id: Some("correct".to_owned()),
        };
        let positive_rank_by_id = BTreeMap::from([("correct".to_owned(), Some(2usize))]);

        let failures = evaluate_hard_negative_expectation(&expected, Some(1), &positive_rank_by_id);

        assert_eq!(failures.len(), 2);
        assert!(failures[0].contains("rank 1"));
        assert!(failures[1].contains("beat candidate id"));
    }

    #[test]
    fn manifest_validation_rejects_blank_case_name() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: " ".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: Vec::new(),
                hard_negatives: Vec::new(),
            }],
        };

        let error = validate_media_match_v3_diagnostic_manifest(&manifest)
            .expect_err("blank case name should be rejected");

        assert!(error.contains("blank case name"));
    }

    #[test]
    fn manifest_validation_rejects_blank_query_path() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "case".to_owned(),
                query: " ".to_owned(),
                candidates: Vec::new(),
                hard_negatives: Vec::new(),
            }],
        };

        let error = validate_media_match_v3_diagnostic_manifest(&manifest)
            .expect_err("blank query path should be rejected");

        assert!(error.contains("blank query path"));
    }

    #[test]
    fn manifest_validation_rejects_blank_candidate_path() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "case".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![test_expectation_without_id(" ")],
                hard_negatives: Vec::new(),
            }],
        };

        let error = validate_media_match_v3_diagnostic_manifest(&manifest)
            .expect_err("blank candidate path should be rejected");

        assert!(error.contains("blank candidate path"));
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
                    id: None,
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                    expected_retrieved: None,
                    max_retrieval_rank: None,
                    max_promotion_rank: None,
                    expect_within_promotion_budget: false,
                    skip_decision_expectation: false,
                }],
                hard_negatives: Vec::new(),
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
                hard_negatives: Vec::new(),
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
            id: None,
            path: "candidate.mkv".to_owned(),
            expected_class: Some("SameCutStrong".to_owned()),
            minimum_tier: Some("Strong".to_owned()),
            expected_offset_ms: Some(5000),
            max_offset_error_ms: Some(1000),
            autoplay_eligible: Some(true),
            must_be_retrieved: true,
            expected_retrieved: None,
            max_retrieval_rank: None,
            max_promotion_rank: None,
            expect_within_promotion_budget: false,
            skip_decision_expectation: false,
        };

        let pass = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(5200),
            &expected,
            &settings,
            true,
            Some(1),
            true,
        );
        let fail = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(8000),
            &expected,
            &settings,
            false,
            None,
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
            id: None,
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
            must_be_retrieved: false,
            expected_retrieved: None,
            max_retrieval_rank: None,
            max_promotion_rank: None,
            expect_within_promotion_budget: false,
            skip_decision_expectation: false,
        };

        let failures = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(800),
            &expected,
            &settings,
            false,
            None,
            false,
        );

        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn retrieval_only_expectation_skips_sampled_decision_requirements() {
        let settings = diagnostic_decision_settings();
        let expected = MediaMatchV3DiagnosticExpectation {
            path: "candidate.mkv".to_owned(),
            expected_class: Some("SameCutStrong".to_owned()),
            minimum_tier: Some("Strong".to_owned()),
            autoplay_eligible: Some(true),
            expected_retrieved: Some(true),
            max_retrieval_rank: Some(1),
            skip_decision_expectation: true,
            ..MediaMatchV3DiagnosticExpectation::default()
        };
        let mut sampled_decision = decision_with_offset_ms(0);
        sampled_decision.tier = MediaMatchTier::Probable;
        sampled_decision.evidence.v3_class = Some(MatchClassV3::SameCutProbable);

        let pass = evaluate_diagnostic_expectation(
            &sampled_decision,
            &expected,
            &settings,
            true,
            Some(1),
            true,
        );
        let fail = evaluate_diagnostic_expectation(
            &sampled_decision,
            &expected,
            &settings,
            true,
            Some(2),
            true,
        );

        assert!(pass.is_empty(), "{pass:?}");
        assert!(
            fail.iter()
                .any(|failure| failure.contains("expected retrieval rank <= 1")),
            "{fail:?}"
        );
        assert!(!sampled_decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn sampled_index_decision_is_capped_below_autoplay_strong() {
        let settings = diagnostic_decision_settings();
        let decision = cap_sampled_index_decision_if_needed(
            decision_with_offset_ms(0),
            MediaMatchV3DiagnosticIndexMode::SampledNormal,
        );

        assert_eq!(decision.tier, MediaMatchTier::Probable);
        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutProbable)
        );
        assert!(!decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn diagnostic_report_includes_cache_root_and_retention() {
        let cache_root = PathBuf::from("C:/diagnostic-cache");
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: Vec::new(),
        };

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: PathBuf::from("C:/manifest"),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: MediaMatchToolPaths {
                    ffmpeg: PathBuf::from("ffmpeg"),
                    ffprobe: PathBuf::from("ffprobe"),
                },
                generated_at_unix_millis: Some(123),
            },
        )
        .expect("empty diagnostic manifest should run");
        let value = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(value["cacheRoot"], cache_root.to_string_lossy().as_ref());
        assert_eq!(value["cacheRetained"], true);
        assert_eq!(
            value["fingerprintCacheVersion"],
            crate::MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION
        );
    }

    #[test]
    fn production_mode_reports_promotion_summary_fields() {
        let root = temp_dir("v3-diagnostics-production-summary");
        let cache_root = root.join("cache");
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: Vec::new(),
        };

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root,
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Production,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: MediaMatchToolPaths {
                    ffmpeg: PathBuf::from("ffmpeg"),
                    ffprobe: PathBuf::from("ffprobe"),
                },
                generated_at_unix_millis: Some(123),
            },
        )
        .expect("empty production diagnostic manifest should run");
        let value = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(report.index_mode, "production");
        assert_eq!(report.summary.max_full_promotions_per_query, 1);
        assert_eq!(report.summary.production_sampled_index_millis, 0);
        assert_eq!(report.summary.production_retrieval_millis, 0);
        assert_eq!(report.summary.production_full_promotion_millis, 0);
        assert_eq!(report.summary.production_total_millis, 0);
        assert_eq!(report.summary.sampled_cache_hit_count, 0);
        assert_eq!(report.summary.sampled_fast_worker_count, 0);
        assert_eq!(report.summary.full_verify_worker_count, 0);
        assert_eq!(report.summary.files_per_minute, 0);
        assert_eq!(report.summary.files_skipped_from_cache, 0);
        assert_eq!(report.summary.files_failed, 0);
        assert_eq!(report.summary.sqlite_writer_millis, 0);
        assert!(report.summary.production_passed);
        assert_eq!(value["summary"]["maxFullPromotionsPerQuery"], 1);
        assert_eq!(value["summary"]["productionSampledIndexMillis"], 0);
        assert_eq!(value["summary"]["productionRetrievalMillis"], 0);
        assert_eq!(value["summary"]["productionFullPromotionMillis"], 0);
        assert_eq!(value["summary"]["productionTotalMillis"], 0);
        assert_eq!(value["summary"]["sampledCacheHitCount"], 0);
        assert_eq!(value["summary"]["sampledFastWorkerCount"], 0);
        assert_eq!(value["summary"]["fullVerifyWorkerCount"], 0);
        assert_eq!(value["summary"]["filesPerMinute"], 0);
        assert_eq!(value["summary"]["filesSkippedFromCache"], 0);
        assert_eq!(value["summary"]["filesFailed"], 0);
        assert_eq!(value["summary"]["sqliteWriterMillis"], 0);
        assert_eq!(value["summary"]["productionPassed"], true);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_mode_promotes_rank_two_candidate_within_budget() {
        let root = temp_dir("v3-diagnostics-production-rank-two");
        let cache_root = root.join("cache");
        let query = root.join("query.mkv");
        let expected = root.join("zzz-expected.mkv");
        let sampled_rank_one = root.join("aaa-sampled-rank-one.mkv");
        for path in [&query, &expected, &sampled_rank_one] {
            fs::write(path, b"media").expect("fixture media should be written");
        }
        let sampled_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let full_settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        for record in [
            fixture_record(&query, &sampled_settings, 0),
            fixture_record(&sampled_rank_one, &sampled_settings, 0),
            fixture_record(&expected, &sampled_settings, 0),
            fixture_record(&query, &full_settings, 0),
            fixture_record(&expected, &full_settings, 0),
        ] {
            save_media_match_v3_record(&connection, &record, None)
                .expect("cached diagnostic record should save");
        }
        drop(connection);
        let mut expected_candidate =
            test_expectation_with_id("expected", &expected.to_string_lossy());
        expected_candidate.expected_retrieved = Some(true);
        expected_candidate.max_promotion_rank = Some(3);
        expected_candidate.expect_within_promotion_budget = true;
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "rank-two-production".to_owned(),
                query: query.to_string_lossy().to_string(),
                candidates: vec![expected_candidate],
                hard_negatives: vec![MediaMatchV3DiagnosticHardNegative {
                    id: Some("sampled-rank-one".to_owned()),
                    path: sampled_rank_one.to_string_lossy().to_string(),
                    must_not_be_top_rank: false,
                    must_not_beat_candidate_id: None,
                }],
            }],
        };

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Production,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 3,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(123),
            },
        )
        .expect("cached production diagnostic should not invoke ffmpeg");

        let candidate = &report.cases[0].candidates[0];
        assert!(candidate.passed);
        assert_eq!(candidate.retrieval_rank, Some(2));
        assert_eq!(candidate.sampled_retrieval_rank, Some(2));
        assert!(candidate.within_promotion_budget);
        assert!(!candidate.strict_rank1_passed);
        assert!(candidate.production_retrieval_passed);
        assert_eq!(candidate.final_verified_rank, Some(2));
        assert_eq!(candidate.promoted_candidate_ranks, vec![2]);
        assert_eq!(
            candidate.promotion_reason.as_deref(),
            Some("retrieval-rank-2")
        );
        assert!(report.summary.production_passed);
        assert_eq!(report.summary.max_full_promotions_per_query, 3);
        assert_eq!(
            report.summary.production_retrieval_millis,
            report.summary.retrieval_total_millis
        );
        assert!(report.summary.full_promotion_cache_hits >= 2);
        assert_eq!(
            report.summary.production_total_millis,
            report
                .summary
                .production_sampled_index_millis
                .saturating_add(report.summary.production_retrieval_millis)
                .saturating_add(report.summary.production_full_promotion_millis)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostic_harness_uses_sqlite_cache_before_fresh_extraction() {
        let root = temp_dir("v3-diagnostics-sqlite-cache");
        let query = root.join("query.mkv");
        let candidate = root.join("candidate.mkv");
        fs::write(&query, b"query").expect("query should be written");
        fs::write(&candidate, b"candidate").expect("candidate should be written");
        let cache_root = root.join("cache");
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&query, &settings, 0), None)
            .expect("query record should save");
        save_media_match_v3_record(&connection, &fixture_record(&candidate, &settings, 0), None)
            .expect("candidate record should save");
        drop(connection);
        let manifest = manifest_for_paths("sqlite-cache", &query, &[&candidate]);

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect("sqlite cache should avoid fresh extraction");
        let value = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(
            report.cases[0].query.source,
            FINGERPRINT_SOURCE_SQLITE_CACHE
        );
        assert_eq!(
            report.cases[0].candidates[0].source,
            FINGERPRINT_SOURCE_SQLITE_CACHE
        );
        assert_eq!(report.summary.unique_fresh_fingerprint_count, 0);
        assert_eq!(report.summary.unique_memory_cache_fingerprint_count, 0);
        assert_eq!(report.summary.unique_sqlite_cache_fingerprint_count, 2);
        assert_eq!(report.summary.fresh_fingerprint_report_count, 0);
        assert_eq!(report.summary.memory_cache_fingerprint_report_count, 0);
        assert_eq!(report.summary.sqlite_cache_fingerprint_report_count, 2);
        assert_eq!(report.summary.total_extraction_millis, 0);
        assert_eq!(value["summary"]["uniqueFreshFingerprintCount"], 0);
        assert_eq!(value["summary"]["uniqueMemoryCacheFingerprintCount"], 0);
        assert_eq!(value["summary"]["uniqueSqliteCacheFingerprintCount"], 2);
        assert_eq!(value["summary"]["freshFingerprintReportCount"], 0);
        assert_eq!(value["summary"]["memoryCacheFingerprintReportCount"], 0);
        assert_eq!(value["summary"]["sqliteCacheFingerprintReportCount"], 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_path_reports_memory_cache_for_duplicate_use() {
        let root = temp_dir("v3-diagnostics-memory-cache");
        let media = root.join("same.mkv");
        fs::write(&media, b"same").expect("media should be written");
        let cache_root = root.join("cache");
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&media, &settings, 0), None)
            .expect("record should save");
        drop(connection);
        let manifest = manifest_for_paths("memory-cache", &media, &[&media]);

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect("duplicate path should use in-memory cache after sqlite load");

        assert_eq!(
            report.cases[0].query.source,
            FINGERPRINT_SOURCE_SQLITE_CACHE
        );
        assert_eq!(
            report.cases[0].candidates[0].source,
            FINGERPRINT_SOURCE_MEMORY_CACHE
        );
        assert_eq!(report.summary.unique_sqlite_cache_fingerprint_count, 1);
        assert_eq!(report.summary.unique_memory_cache_fingerprint_count, 0);
        assert_eq!(report.summary.sqlite_cache_fingerprint_report_count, 1);
        assert_eq!(report.summary.memory_cache_fingerprint_report_count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn modified_file_invalidates_sqlite_cache() {
        let root = temp_dir("v3-diagnostics-stale-cache");
        let media = root.join("stale.mkv");
        fs::write(&media, b"before").expect("media should be written");
        let cache_root = root.join("cache");
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&media, &settings, 0), None)
            .expect("record should save");
        drop(connection);
        fs::write(&media, b"after with different size").expect("media should change");
        let manifest = manifest_for_paths("stale-cache", &media, &[]);

        let error = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root,
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect_err("stale cache should not be reused");

        assert!(error.contains("failed fingerprinting"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn different_profile_does_not_reuse_sqlite_cache_record() {
        let root = temp_dir("v3-diagnostics-settings-cache");
        let media = root.join("profile.mkv");
        fs::write(&media, b"profile").expect("media should be written");
        let cache_root = root.join("cache");
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&media, &settings, 0), None)
            .expect("record should save");
        drop(connection);
        let mut manifest = manifest_for_paths("profile-cache", &media, &[]);
        manifest.profile = "combined-v3".to_owned();

        let error = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root,
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: false,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect_err("different settings hash should not reuse cached record");

        assert!(error.contains("failed fingerprinting"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fingerprint_config_hash_mismatch_does_not_reuse_sqlite_cache_record() {
        let root = temp_dir("v3-diagnostics-hash-cache");
        let media = root.join("hash.mkv");
        fs::write(&media, b"hash").expect("media should be written");
        let cache_root = root.join("cache");
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&media, &settings, 0), None)
            .expect("record should save");
        connection
            .execute(
                "UPDATE settings_v3 SET settings_hash = ?1",
                [vec![0xff; 32]],
            )
            .expect("stored settings hash should be changed");
        let (modified_unix_millis, size_bytes) =
            media_file_identity_parts(&media).expect("identity should load");

        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &normalize_media_path(&media),
            &settings,
            modified_unix_millis,
            size_bytes,
        )
        .expect("cache lookup should not fail");

        assert!(
            loaded.is_none(),
            "changed fingerprint config hash should miss the SQLite record"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn experimental_sampled_policy_mismatch_does_not_reuse_production_cache_record() {
        let root = temp_dir("v3-diagnostics-sampled-policy-cache");
        let media = root.join("sampled.mkv");
        fs::write(&media, b"sampled").expect("media should be written");
        let cache_root = root.join("cache");
        let fixed_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let experimental_policy = MediaSampledAudioPolicy::for_sampled_fast_source_strategy(
            MediaSampledAudioSourceStrategy::OutputSeekPerWindow,
        );
        let experimental_settings = MediaExtractionSettings::sampled_fast_audio_index_v3()
            .with_sampled_audio_policy(experimental_policy);
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(
            &connection,
            &fixture_record(&media, &fixed_settings, 0),
            None,
        )
        .expect("fixed sampled record should save");
        let (modified_unix_millis, size_bytes) =
            media_file_identity_parts(&media).expect("identity should load");

        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &normalize_media_path(&media),
            &experimental_settings,
            modified_unix_millis,
            size_bytes,
        )
        .expect("cache lookup should not fail");
        let incompatible_count = media_match_v3_incompatible_record_count_for_path(
            &connection,
            &normalize_media_path(&media),
            &experimental_settings,
            modified_unix_millis,
            size_bytes,
        )
        .expect("incompatible cache check should not fail");

        assert!(
            loaded.is_none(),
            "experimental sampled-fast policy must not reuse production sampled-fast records"
        );
        assert_eq!(incompatible_count, 1);

        save_media_match_v3_record(
            &connection,
            &fixture_record(&media, &experimental_settings, 1_000),
            None,
        )
        .expect("experimental sampled record should save");
        let loaded = load_media_match_v3_record_for_path(
            &connection,
            &normalize_media_path(&media),
            &experimental_settings,
            modified_unix_millis,
            size_bytes,
        )
        .expect("cache lookup should not fail");

        assert!(
            loaded.is_some(),
            "same experimental sampled-fast policy should reuse SQLite cache"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sampled_policy_is_reported_for_sampled_fast_runs() {
        let root = temp_dir("v3-diagnostics-sampled-policy-report");
        let media = root.join("sampled-report.mkv");
        fs::write(&media, b"sampled report").expect("media should be written");
        let cache_root = root.join("cache");
        let policy = MediaSampledAudioPolicy::for_sampled_fast_source_strategy(
            MediaSampledAudioSourceStrategy::Current,
        );
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3()
            .with_sampled_audio_policy(policy.clone());
        let connection = open_media_match_v3_index(&cache_root).expect("index should open");
        save_media_match_v3_record(&connection, &fixture_record(&media, &settings, 0), None)
            .expect("sampled record should save");
        drop(connection);
        let manifest = manifest_for_paths("sampled-policy", &media, &[]);

        let report = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root,
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::SampledFast,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: true,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::Current,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect("matching sampled policy should load from cache");

        assert_eq!(report.summary.sampled_audio_policy, Some(policy));
        assert!(report.summary.sampled_policy_production_compatible);
        assert_eq!(
            report.summary.sampled_audio_source_strategy.as_deref(),
            Some("current")
        );
        assert!(!report.summary.experimental_sampled_audio_source);
        assert_eq!(
            report
                .summary
                .sampled_audio_source_equivalence
                .as_ref()
                .and_then(|equivalence| equivalence.pcm_byte_exact_match),
            Some(true)
        );
        assert_eq!(report.summary.sqlite_cache_compatible_hit_count, 1);
        assert_eq!(report.summary.sqlite_cache_incompatible_miss_count, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_current_sampled_source_requires_experimental_run_option() {
        let root = temp_dir("v3-diagnostics-experimental-source-guard");
        let media = root.join("source-guard.mkv");
        fs::write(&media, b"sampled source guard").expect("media should be written");
        let manifest = manifest_for_paths("source-guard", &media, &[]);

        let error = run_media_match_v3_diagnostic_manifest(
            &manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: root.join("cache"),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::SampledFast,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                retrieval_benchmark_only: true,
                retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
                sampled_fast_global_workers: None,
                sampled_fast_per_local_source_workers: None,
                sampled_fast_per_network_source_workers: None,
                sampled_fast_per_removable_source_workers: None,
                probe_audio_packets: false,
                sampled_audio_source: MediaSampledAudioSourceStrategy::MkvAudioRanges,
                experimental_sampled_audio_source: false,
                sampled_pcm_cache_root: None,
                tools: unavailable_tools(),
                generated_at_unix_millis: Some(1),
            },
        )
        .expect_err("normal sampled-fast diagnostics should reject experimental source strategies");

        assert!(error.contains("experimental"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    fn test_expectation_with_id(id: &str, path: &str) -> MediaMatchV3DiagnosticExpectation {
        MediaMatchV3DiagnosticExpectation {
            id: Some(id.to_owned()),
            path: path.to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: None,
            autoplay_eligible: None,
            must_be_retrieved: false,
            expected_retrieved: None,
            max_retrieval_rank: None,
            max_promotion_rank: None,
            expect_within_promotion_budget: false,
            skip_decision_expectation: false,
        }
    }

    fn test_expectation_without_id(path: &str) -> MediaMatchV3DiagnosticExpectation {
        MediaMatchV3DiagnosticExpectation {
            id: None,
            path: path.to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: None,
            autoplay_eligible: None,
            must_be_retrieved: false,
            expected_retrieved: None,
            max_retrieval_rank: None,
            max_promotion_rank: None,
            expect_within_promotion_budget: false,
            skip_decision_expectation: false,
        }
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

    fn manifest_for_paths(
        name: &str,
        query: &Path,
        candidates: &[&Path],
    ) -> MediaMatchV3DiagnosticManifest {
        MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: name.to_owned(),
                query: query.to_string_lossy().to_string(),
                candidates: candidates
                    .iter()
                    .map(|path| test_expectation_without_id(&path.to_string_lossy()))
                    .collect(),
                hard_negatives: Vec::new(),
            }],
        }
    }

    fn fixture_record(
        path: &Path,
        settings: &MediaExtractionSettings,
        bucket_offset: u32,
    ) -> MediaFingerprintRecord {
        let metadata = fs::metadata(path).expect("fixture metadata should be readable");
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let size_bytes = metadata.len();
        let identity = MediaFileIdentity::new(path, modified_unix_millis, size_bytes);
        let duration_seconds = Some(180.0);
        let container_fingerprint = container_fingerprint_from_metadata(
            &identity.normalized_path,
            modified_unix_millis,
            size_bytes,
            duration_seconds,
        );
        let audio_anchors = (0..24)
            .map(|index| AudioAnchor {
                bucket: bucket_offset + 10_000 + index,
                t_ms: index * 7_500,
                weight: 10,
            })
            .collect();
        MediaFingerprintRecord {
            identity,
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: settings.clone(),
            duration_seconds,
            container_fingerprint,
            video: None,
            audio_anchors,
            video_anchors: Vec::new(),
            audio_error: None,
            video_error: None,
        }
    }

    fn unavailable_tools() -> MediaMatchToolPaths {
        MediaMatchToolPaths {
            ffmpeg: PathBuf::from("missing-sorotte-v3-diagnostics-ffmpeg"),
            ffprobe: PathBuf::from("missing-sorotte-v3-diagnostics-ffprobe"),
        }
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "sorotte-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }
}
