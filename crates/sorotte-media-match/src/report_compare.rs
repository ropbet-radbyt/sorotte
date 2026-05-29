use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{
    MediaMatchV3DiagnosticCandidateReport, MediaMatchV3DiagnosticFingerprintReport,
    MediaMatchV3DiagnosticReport, MediaMatchV3DiagnosticSummary,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportPairKey {
    pub case_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_path: Option<String>,
}

impl MediaMatchV3ReportPairKey {
    fn label(&self) -> String {
        match self.candidate_id.as_deref() {
            Some(candidate_id) => {
                format!("case '{}' candidate id '{}'", self.case_name, candidate_id)
            }
            None => format!(
                "case '{}' candidate path '{}'",
                self.case_name,
                self.candidate_path.as_deref().unwrap_or("<missing>")
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportCompatibilityOptions {
    pub allow_different_profile: bool,
    pub allow_different_settings: bool,
    pub allow_different_tuning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportCompatibility {
    pub algorithm_version_matches: bool,
    pub fingerprint_cache_version_matches: bool,
    pub profile_matches: bool,
    pub settings_hash_matches: bool,
    pub tuning_matches: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportStatusChange {
    pub key: MediaMatchV3ReportPairKey,
    pub baseline_passed: Option<bool>,
    pub current_passed: Option<bool>,
    pub baseline_failure_reason: Option<String>,
    pub current_failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportValueChange {
    pub key: MediaMatchV3ReportPairKey,
    pub field: String,
    pub baseline: Option<String>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportMetricDelta {
    pub field: String,
    pub baseline: i128,
    pub current: i128,
    pub delta: i128,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportComparisonSummary {
    pub regression: bool,
    pub unresolved_failure: bool,
    pub baseline_failed: usize,
    pub current_failed: usize,
    pub new_failures: usize,
    pub resolved_failures: usize,
    pub missing_pairs: usize,
    pub new_pairs: usize,
    pub new_failed_pairs: usize,
    pub retrieval_misses: usize,
    pub new_retrieval_misses: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportComparison {
    pub comparison_mode: String,
    pub compatibility: MediaMatchV3ReportCompatibility,
    pub compatibility_options: MediaMatchV3ReportCompatibilityOptions,
    pub summary: MediaMatchV3ReportComparisonSummary,
    pub baseline_failed: usize,
    pub current_failed: usize,
    pub new_failures: Vec<MediaMatchV3ReportStatusChange>,
    pub resolved_failures: Vec<MediaMatchV3ReportStatusChange>,
    pub missing_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
    pub new_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
    pub new_failed_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
    pub retrieval_misses: Vec<MediaMatchV3ReportPairKey>,
    pub new_retrieval_misses: Vec<MediaMatchV3ReportPairKey>,
    pub class_changes: Vec<MediaMatchV3ReportValueChange>,
    pub tier_changes: Vec<MediaMatchV3ReportValueChange>,
    pub retrieval_rank_changes: Vec<MediaMatchV3ReportValueChange>,
    pub autoplay_eligibility_changes: Vec<MediaMatchV3ReportValueChange>,
    pub offset_error_changes: Vec<MediaMatchV3ReportValueChange>,
    pub metric_deltas: Vec<MediaMatchV3ReportMetricDelta>,
}

impl MediaMatchV3ReportComparison {
    pub fn current_has_more_failures(&self) -> bool {
        self.current_failed > self.baseline_failed
    }

    pub fn current_has_regressions(&self) -> bool {
        self.summary.regression
    }

    pub fn current_has_unresolved_failures(&self) -> bool {
        self.summary.unresolved_failure
    }
}

pub fn validate_media_match_v3_diagnostic_report(
    report: &MediaMatchV3DiagnosticReport,
) -> Result<(), String> {
    let case_count = report.cases.len();
    if report.summary.case_count != case_count {
        return Err(format!(
            "summary.caseCount={} does not match cases.len()={case_count}",
            report.summary.case_count
        ));
    }

    let mut pair_count = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut hard_negative_count = 0usize;
    let mut hard_negative_passed = 0usize;
    let mut hard_negative_failed = 0usize;
    let mut total_raw_hit_rows_processed = 0i64;
    let mut total_retrieval_millis = 0u128;
    let mut retrieval_unaccounted_millis_total = 0u128;
    let mut sql_hit_fetch_millis_total = 0u128;
    let mut rust_aggregation_millis_total = 0u128;
    let mut candidate_metadata_load_millis_total = 0u128;
    let mut robust_rerank_millis_total = 0u128;
    let mut total_full_promotion_millis = 0u128;
    let mut promoted_candidates = 0usize;
    let mut report_source_counts = FingerprintSourceCounts::default();

    for case in &report.cases {
        pair_count += case.candidates.len();
        total_raw_hit_rows_processed += case.retrieval.raw_hit_rows_processed;
        total_retrieval_millis += case.retrieval.retrieval_elapsed_ms;
        retrieval_unaccounted_millis_total += case.retrieval.retrieval_unaccounted_millis;
        sql_hit_fetch_millis_total += case.retrieval.sql_hit_fetch_millis;
        rust_aggregation_millis_total += case.retrieval.rust_aggregation_millis;
        candidate_metadata_load_millis_total += case.retrieval.candidate_metadata_load_millis;
        robust_rerank_millis_total += case.retrieval.robust_rerank_millis;
        validate_fingerprint_source(&case.name, "query", &case.query.source)?;
        report_source_counts.increment(&case.query.source);
        for candidate in &case.candidates {
            validate_fingerprint_source(&case.name, &candidate.path, &candidate.source)?;
            report_source_counts.increment(&candidate.source);
            if let Some(candidate_id) = candidate.candidate_id.as_deref()
                && candidate_id.trim().is_empty()
            {
                return Err(format!(
                    "case '{}' candidate '{}' has a blank candidateId",
                    case.name, candidate.path
                ));
            }
            if let Some(candidate_id) = candidate.candidate_id.as_deref()
                && candidate_id.trim() != candidate_id
            {
                return Err(format!(
                    "case '{}' candidate '{}' has a candidateId with leading or trailing whitespace",
                    case.name, candidate.path
                ));
            }
            if candidate.passed {
                passed += 1;
            } else {
                failed += 1;
            }
            total_full_promotion_millis += candidate.full_promotion_millis;
            if candidate.promotion_reason.is_some() {
                promoted_candidates += 1;
            }
        }
        for hard_negative in &case.hard_negatives {
            validate_fingerprint_source(&case.name, &hard_negative.path, &hard_negative.source)?;
            report_source_counts.increment(&hard_negative.source);
            if let Some(candidate_id) = hard_negative.candidate_id.as_deref()
                && candidate_id.trim().is_empty()
            {
                return Err(format!(
                    "case '{}' hard negative '{}' has a blank candidateId",
                    case.name, hard_negative.path
                ));
            }
            hard_negative_count += 1;
            if hard_negative.passed {
                hard_negative_passed += 1;
            } else {
                hard_negative_failed += 1;
            }
        }
    }

    if report.summary.pair_count != pair_count {
        return Err(format!(
            "summary.pairCount={} does not match candidate count={pair_count}",
            report.summary.pair_count
        ));
    }
    if report.summary.passed != passed {
        return Err(format!(
            "summary.passed={} does not match passed candidates={passed}",
            report.summary.passed
        ));
    }
    if report.summary.failed != failed {
        return Err(format!(
            "summary.failed={} does not match failed candidates={failed}",
            report.summary.failed
        ));
    }
    if report.summary.passed + report.summary.failed != report.summary.pair_count {
        return Err(format!(
            "summary.passed + summary.failed = {} does not match summary.pairCount={}",
            report.summary.passed + report.summary.failed,
            report.summary.pair_count
        ));
    }
    if report.summary.hard_negative_count != hard_negative_count {
        return Err(format!(
            "summary.hardNegativeCount={} does not match hard-negative count={hard_negative_count}",
            report.summary.hard_negative_count
        ));
    }
    if report.summary.hard_negative_passed != hard_negative_passed {
        return Err(format!(
            "summary.hardNegativePassed={} does not match passed hard-negatives={hard_negative_passed}",
            report.summary.hard_negative_passed
        ));
    }
    if report.summary.hard_negative_failed != hard_negative_failed {
        return Err(format!(
            "summary.hardNegativeFailed={} does not match failed hard-negatives={hard_negative_failed}",
            report.summary.hard_negative_failed
        ));
    }
    if report.summary.total_raw_hit_rows_processed != total_raw_hit_rows_processed {
        return Err(format!(
            "summary.totalRawHitRowsProcessed={} does not match retrieval total={total_raw_hit_rows_processed}",
            report.summary.total_raw_hit_rows_processed
        ));
    }
    if report.summary.total_retrieval_millis != total_retrieval_millis {
        return Err(format!(
            "summary.totalRetrievalMillis={} does not match retrieval total={total_retrieval_millis}",
            report.summary.total_retrieval_millis
        ));
    }
    if report.summary.retrieval_total_millis != total_retrieval_millis {
        return Err(format!(
            "summary.retrievalTotalMillis={} does not match retrieval total={total_retrieval_millis}",
            report.summary.retrieval_total_millis
        ));
    }
    if report.summary.retrieval_unaccounted_millis_total != retrieval_unaccounted_millis_total {
        return Err(format!(
            "summary.retrievalUnaccountedMillisTotal={} does not match retrieval total={retrieval_unaccounted_millis_total}",
            report.summary.retrieval_unaccounted_millis_total
        ));
    }
    if report.summary.sql_hit_fetch_millis_total != sql_hit_fetch_millis_total {
        return Err(format!(
            "summary.sqlHitFetchMillisTotal={} does not match retrieval total={sql_hit_fetch_millis_total}",
            report.summary.sql_hit_fetch_millis_total
        ));
    }
    if report.summary.rust_aggregation_millis_total != rust_aggregation_millis_total {
        return Err(format!(
            "summary.rustAggregationMillisTotal={} does not match retrieval total={rust_aggregation_millis_total}",
            report.summary.rust_aggregation_millis_total
        ));
    }
    if report.summary.candidate_metadata_load_millis_total != candidate_metadata_load_millis_total {
        return Err(format!(
            "summary.candidateMetadataLoadMillisTotal={} does not match retrieval total={candidate_metadata_load_millis_total}",
            report.summary.candidate_metadata_load_millis_total
        ));
    }
    if report.summary.robust_rerank_millis_total != robust_rerank_millis_total {
        return Err(format!(
            "summary.robustRerankMillisTotal={} does not match retrieval total={robust_rerank_millis_total}",
            report.summary.robust_rerank_millis_total
        ));
    }
    validate_retrieval_percentile_summary(report)?;
    if report.summary.full_promotion_millis != total_full_promotion_millis {
        return Err(format!(
            "summary.fullPromotionMillis={} does not match candidate total={total_full_promotion_millis}",
            report.summary.full_promotion_millis
        ));
    }
    if report.summary.candidates_promoted_to_full_verify != promoted_candidates {
        return Err(format!(
            "summary.candidatesPromotedToFullVerify={} does not match promoted candidates={promoted_candidates}",
            report.summary.candidates_promoted_to_full_verify
        ));
    }
    let expected_production_total = report
        .summary
        .production_sampled_index_millis
        .saturating_add(report.summary.production_full_promotion_millis);
    if report.summary.production_total_millis != expected_production_total {
        return Err(format!(
            "summary.productionTotalMillis={} does not match production sampled+promotion total={expected_production_total}",
            report.summary.production_total_millis
        ));
    }
    let aggregate_totals = report_aggregate_fingerprint_totals(report);
    if report.summary.total_extraction_millis != aggregate_totals.extraction_millis {
        return Err(format!(
            "summary.totalExtractionMillis={} does not match unique fingerprint total={}",
            report.summary.total_extraction_millis, aggregate_totals.extraction_millis
        ));
    }
    if report.summary.total_audio_blob_bytes != aggregate_totals.audio_blob_bytes {
        return Err(format!(
            "summary.totalAudioBlobBytes={} does not match unique fingerprint total={}",
            report.summary.total_audio_blob_bytes, aggregate_totals.audio_blob_bytes
        ));
    }
    if report.summary.total_video_blob_bytes != aggregate_totals.video_blob_bytes {
        return Err(format!(
            "summary.totalVideoBlobBytes={} does not match unique fingerprint total={}",
            report.summary.total_video_blob_bytes, aggregate_totals.video_blob_bytes
        ));
    }
    if report.summary.unique_fresh_fingerprint_count != aggregate_totals.fresh_count {
        return Err(format!(
            "summary.uniqueFreshFingerprintCount={} does not match unique fingerprint total={}",
            report.summary.unique_fresh_fingerprint_count, aggregate_totals.fresh_count
        ));
    }
    if report.summary.unique_memory_cache_fingerprint_count != aggregate_totals.memory_cache_count {
        return Err(format!(
            "summary.uniqueMemoryCacheFingerprintCount={} does not match unique fingerprint total={}",
            report.summary.unique_memory_cache_fingerprint_count,
            aggregate_totals.memory_cache_count
        ));
    }
    if report.summary.unique_sqlite_cache_fingerprint_count != aggregate_totals.sqlite_cache_count {
        return Err(format!(
            "summary.uniqueSqliteCacheFingerprintCount={} does not match unique fingerprint total={}",
            report.summary.unique_sqlite_cache_fingerprint_count,
            aggregate_totals.sqlite_cache_count
        ));
    }
    if report.summary.fresh_fingerprint_report_count != report_source_counts.fresh {
        return Err(format!(
            "summary.freshFingerprintReportCount={} does not match report row total={}",
            report.summary.fresh_fingerprint_report_count, report_source_counts.fresh
        ));
    }
    if report.summary.memory_cache_fingerprint_report_count != report_source_counts.memory_cache {
        return Err(format!(
            "summary.memoryCacheFingerprintReportCount={} does not match report row total={}",
            report.summary.memory_cache_fingerprint_report_count, report_source_counts.memory_cache
        ));
    }
    if report.summary.sqlite_cache_fingerprint_report_count != report_source_counts.sqlite_cache {
        return Err(format!(
            "summary.sqliteCacheFingerprintReportCount={} does not match report row total={}",
            report.summary.sqlite_cache_fingerprint_report_count, report_source_counts.sqlite_cache
        ));
    }
    if report.summary.fresh_fingerprint_report_count
        + report.summary.memory_cache_fingerprint_report_count
        + report.summary.sqlite_cache_fingerprint_report_count
        != report.summary.case_count
            + report.summary.pair_count
            + report.summary.hard_negative_count
    {
        return Err(format!(
            "fingerprint report source counts sum to {}, expected query+candidate+hard-negative row count={}",
            report.summary.fresh_fingerprint_report_count
                + report.summary.memory_cache_fingerprint_report_count
                + report.summary.sqlite_cache_fingerprint_report_count,
            report.summary.case_count
                + report.summary.pair_count
                + report.summary.hard_negative_count
        ));
    }

    let pairs = report_pairs_by_key(report);
    if let Some(key) = pairs.duplicate_keys.first() {
        return Err(format!(
            "duplicate comparison key in report: {}",
            key.label()
        ));
    }

    Ok(())
}

fn validate_retrieval_percentile_summary(
    report: &MediaMatchV3DiagnosticReport,
) -> Result<(), String> {
    if report.cases.is_empty() {
        return Ok(());
    }
    let mut retrieval_millis = report
        .cases
        .iter()
        .map(|case| case.retrieval.retrieval_elapsed_ms)
        .collect::<Vec<_>>();
    retrieval_millis.sort_unstable();
    let expected_p50 = percentile_sorted_u128(&retrieval_millis, 50);
    let expected_p95 = percentile_sorted_u128(&retrieval_millis, 95);
    let expected_p99 = percentile_sorted_u128(&retrieval_millis, 99);
    let expected_max = retrieval_millis.last().copied().unwrap_or_default();
    if report.summary.per_query_retrieval_millis_p50 != expected_p50 {
        return Err(format!(
            "summary.perQueryRetrievalMillisP50={} does not match retrieval p50={expected_p50}",
            report.summary.per_query_retrieval_millis_p50
        ));
    }
    if report.summary.per_query_retrieval_millis_p95 != expected_p95 {
        return Err(format!(
            "summary.perQueryRetrievalMillisP95={} does not match retrieval p95={expected_p95}",
            report.summary.per_query_retrieval_millis_p95
        ));
    }
    if report.summary.per_query_retrieval_millis_p99 != expected_p99 {
        return Err(format!(
            "summary.perQueryRetrievalMillisP99={} does not match retrieval p99={expected_p99}",
            report.summary.per_query_retrieval_millis_p99
        ));
    }
    if report.summary.per_query_retrieval_millis_max != expected_max {
        return Err(format!(
            "summary.perQueryRetrievalMillisMax={} does not match retrieval max={expected_max}",
            report.summary.per_query_retrieval_millis_max
        ));
    }

    let mut unaccounted_millis = report
        .cases
        .iter()
        .map(|case| case.retrieval.retrieval_unaccounted_millis)
        .collect::<Vec<_>>();
    unaccounted_millis.sort_unstable();
    let expected_unaccounted_p95 = percentile_sorted_u128(&unaccounted_millis, 95);
    if report.summary.retrieval_unaccounted_millis_p95 != expected_unaccounted_p95 {
        return Err(format!(
            "summary.retrievalUnaccountedMillisP95={} does not match retrieval p95={expected_unaccounted_p95}",
            report.summary.retrieval_unaccounted_millis_p95
        ));
    }

    Ok(())
}

fn percentile_sorted_u128(values: &[u128], percentile: u32) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let percentile = percentile.min(100) as usize;
    let index = (values.len() - 1) * percentile / 100;
    values[index]
}

fn validate_fingerprint_source(case_name: &str, path: &str, source: &str) -> Result<(), String> {
    match source {
        "fresh" | "memory-cache" | "sqlite-cache" => Ok(()),
        _ => Err(format!(
            "case '{case_name}' fingerprint '{path}' has unknown source '{source}'"
        )),
    }
}

pub fn validate_media_match_v3_report_pair_compatible(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
    options: &MediaMatchV3ReportCompatibilityOptions,
) -> Result<(), String> {
    let compatibility = report_compatibility(baseline, current);
    if !compatibility.algorithm_version_matches {
        return Err(format!(
            "algorithmVersion differs: baseline={}, current={}",
            baseline.algorithm_version, current.algorithm_version
        ));
    }
    if !compatibility.fingerprint_cache_version_matches {
        return Err(format!(
            "fingerprintCacheVersion differs: baseline={}, current={}",
            baseline.fingerprint_cache_version, current.fingerprint_cache_version
        ));
    }
    if !options.allow_different_profile && !compatibility.profile_matches {
        return Err(format!(
            "profile differs: baseline='{}', current='{}'",
            baseline.profile, current.profile
        ));
    }
    if !options.allow_different_settings && !compatibility.settings_hash_matches {
        return Err(format!(
            "settingsHash differs: baseline='{}', current='{}'",
            baseline.settings_hash, current.settings_hash
        ));
    }
    if !options.allow_different_tuning && !compatibility.tuning_matches {
        return Err("tuning differs between reports".to_owned());
    }
    Ok(())
}

pub fn compare_media_match_v3_reports(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
) -> Result<MediaMatchV3ReportComparison, String> {
    compare_media_match_v3_reports_with_options(
        baseline,
        current,
        &MediaMatchV3ReportCompatibilityOptions::default(),
    )
}

pub fn compare_media_match_v3_reports_with_options(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
    options: &MediaMatchV3ReportCompatibilityOptions,
) -> Result<MediaMatchV3ReportComparison, String> {
    validate_media_match_v3_diagnostic_report(baseline)
        .map_err(|error| format!("baseline report is invalid: {error}"))?;
    validate_media_match_v3_diagnostic_report(current)
        .map_err(|error| format!("current report is invalid: {error}"))?;
    validate_media_match_v3_report_pair_compatible(baseline, current, options)?;
    Ok(compare_media_match_v3_reports_unchecked(
        baseline, current, *options,
    ))
}

fn compare_media_match_v3_reports_unchecked(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
    compatibility_options: MediaMatchV3ReportCompatibilityOptions,
) -> MediaMatchV3ReportComparison {
    let compatibility = report_compatibility(baseline, current);
    let baseline_pairs = report_pairs_by_key(baseline);
    let current_pairs = report_pairs_by_key(current);
    let baseline_keys = baseline_pairs
        .pairs
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let current_keys = current_pairs.pairs.keys().cloned().collect::<BTreeSet<_>>();
    let mut new_failures = Vec::new();
    let mut resolved_failures = Vec::new();
    let mut class_changes = Vec::new();
    let mut tier_changes = Vec::new();
    let mut retrieval_rank_changes = Vec::new();
    let mut autoplay_eligibility_changes = Vec::new();
    let mut offset_error_changes = Vec::new();

    for key in baseline_keys.intersection(&current_keys) {
        let baseline_pair = baseline_pairs
            .pairs
            .get(key)
            .expect("intersection key should exist in baseline");
        let current_pair = current_pairs
            .pairs
            .get(key)
            .expect("intersection key should exist in current");
        if baseline_pair.passed && !current_pair.passed {
            new_failures.push(status_change(key, baseline_pair, current_pair));
        } else if !baseline_pair.passed && current_pair.passed {
            resolved_failures.push(status_change(key, baseline_pair, current_pair));
        }
        push_value_change(
            &mut class_changes,
            key,
            "class",
            baseline_pair.decision.class.clone(),
            current_pair.decision.class.clone(),
        );
        push_value_change(
            &mut tier_changes,
            key,
            "tier",
            Some(baseline_pair.decision.tier.clone()),
            Some(current_pair.decision.tier.clone()),
        );
        push_value_change(
            &mut retrieval_rank_changes,
            key,
            "retrievalRank",
            baseline_pair.retrieval_rank.map(|rank| rank.to_string()),
            current_pair.retrieval_rank.map(|rank| rank.to_string()),
        );
        push_value_change(
            &mut autoplay_eligibility_changes,
            key,
            "autoplayEligible",
            Some(baseline_pair.decision.autoplay_eligible.to_string()),
            Some(current_pair.decision.autoplay_eligible.to_string()),
        );
        push_value_change(
            &mut offset_error_changes,
            key,
            "offsetErrorMs",
            offset_error_ms(baseline_pair).map(|error| error.to_string()),
            offset_error_ms(current_pair).map(|error| error.to_string()),
        );
    }

    let missing_pairs_in_current = baseline_keys
        .difference(&current_keys)
        .cloned()
        .collect::<Vec<_>>();
    let new_pairs_in_current = current_keys
        .difference(&baseline_keys)
        .cloned()
        .collect::<Vec<_>>();
    let new_failed_pairs_in_current = new_pairs_in_current
        .iter()
        .filter(|key| {
            current_pairs
                .pairs
                .get(*key)
                .is_some_and(|pair| !pair.passed)
        })
        .cloned()
        .collect::<Vec<_>>();
    let retrieval_misses = current_pairs
        .pairs
        .iter()
        .filter(|(_, pair)| {
            pair.expectation
                .as_ref()
                .is_some_and(|e| e.must_be_retrieved)
        })
        .filter(|(_, pair)| !pair.retrieved)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let new_retrieval_misses = retrieval_misses
        .iter()
        .filter(|key| {
            baseline_pairs
                .pairs
                .get(*key)
                .is_none_or(|baseline_pair| !is_retrieval_miss(baseline_pair))
        })
        .cloned()
        .collect::<Vec<_>>();
    let baseline_failed = baseline
        .summary
        .failed
        .saturating_add(baseline.summary.hard_negative_failed);
    let current_failed = current
        .summary
        .failed
        .saturating_add(current.summary.hard_negative_failed);
    let hard_negative_regression =
        current.summary.hard_negative_failed > baseline.summary.hard_negative_failed;
    let regression = !new_failures.is_empty()
        || !missing_pairs_in_current.is_empty()
        || !new_failed_pairs_in_current.is_empty()
        || !new_retrieval_misses.is_empty()
        || hard_negative_regression;
    let unresolved_failure =
        current_failed > 0 || !retrieval_misses.is_empty() || !missing_pairs_in_current.is_empty();
    let summary = MediaMatchV3ReportComparisonSummary {
        regression,
        unresolved_failure,
        baseline_failed,
        current_failed,
        new_failures: new_failures.len(),
        resolved_failures: resolved_failures.len(),
        missing_pairs: missing_pairs_in_current.len(),
        new_pairs: new_pairs_in_current.len(),
        new_failed_pairs: new_failed_pairs_in_current.len(),
        retrieval_misses: retrieval_misses.len(),
        new_retrieval_misses: new_retrieval_misses.len(),
    };

    MediaMatchV3ReportComparison {
        comparison_mode: "regression".to_owned(),
        compatibility,
        compatibility_options,
        summary,
        baseline_failed,
        current_failed,
        new_failures,
        resolved_failures,
        missing_pairs_in_current,
        new_pairs_in_current,
        new_failed_pairs_in_current,
        retrieval_misses,
        new_retrieval_misses,
        class_changes,
        tier_changes,
        retrieval_rank_changes,
        autoplay_eligibility_changes,
        offset_error_changes,
        metric_deltas: report_metric_deltas(baseline, current),
    }
}

struct ReportPairs<'a> {
    pairs: BTreeMap<MediaMatchV3ReportPairKey, &'a MediaMatchV3DiagnosticCandidateReport>,
    duplicate_keys: Vec<MediaMatchV3ReportPairKey>,
}

fn report_pairs_by_key(report: &MediaMatchV3DiagnosticReport) -> ReportPairs<'_> {
    let mut pairs = BTreeMap::new();
    let mut duplicate_keys = BTreeSet::new();
    for case in &report.cases {
        for candidate in &case.candidates {
            let key = report_pair_key(&case.name, candidate);
            match pairs.entry(key.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(candidate);
                }
                Entry::Occupied(_) => {
                    duplicate_keys.insert(key);
                }
            }
        }
    }
    ReportPairs {
        pairs,
        duplicate_keys: duplicate_keys.into_iter().collect(),
    }
}

fn report_pair_key(
    case_name: &str,
    candidate: &MediaMatchV3DiagnosticCandidateReport,
) -> MediaMatchV3ReportPairKey {
    if let Some(candidate_id) = candidate.candidate_id.as_ref() {
        MediaMatchV3ReportPairKey {
            case_name: case_name.to_owned(),
            candidate_id: Some(candidate_id.trim().to_owned()),
            candidate_path: None,
        }
    } else {
        MediaMatchV3ReportPairKey {
            case_name: case_name.to_owned(),
            candidate_id: None,
            candidate_path: Some(candidate.path.clone()),
        }
    }
}

fn report_compatibility(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
) -> MediaMatchV3ReportCompatibility {
    MediaMatchV3ReportCompatibility {
        algorithm_version_matches: baseline.algorithm_version == current.algorithm_version,
        fingerprint_cache_version_matches: baseline.fingerprint_cache_version
            == current.fingerprint_cache_version,
        profile_matches: baseline.profile == current.profile,
        settings_hash_matches: baseline.settings_hash == current.settings_hash,
        tuning_matches: baseline.tuning == current.tuning,
    }
}

fn status_change(
    key: &MediaMatchV3ReportPairKey,
    baseline: &MediaMatchV3DiagnosticCandidateReport,
    current: &MediaMatchV3DiagnosticCandidateReport,
) -> MediaMatchV3ReportStatusChange {
    MediaMatchV3ReportStatusChange {
        key: key.clone(),
        baseline_passed: Some(baseline.passed),
        current_passed: Some(current.passed),
        baseline_failure_reason: baseline.failure_reason.clone(),
        current_failure_reason: current.failure_reason.clone(),
    }
}

fn push_value_change(
    changes: &mut Vec<MediaMatchV3ReportValueChange>,
    key: &MediaMatchV3ReportPairKey,
    field: &str,
    baseline: Option<String>,
    current: Option<String>,
) {
    if baseline != current {
        changes.push(MediaMatchV3ReportValueChange {
            key: key.clone(),
            field: field.to_owned(),
            baseline,
            current,
        });
    }
}

fn offset_error_ms(candidate: &MediaMatchV3DiagnosticCandidateReport) -> Option<i64> {
    let actual = candidate.decision.offset_seconds?;
    let expected = candidate
        .expectation
        .as_ref()
        .and_then(|expectation| expectation.expected_offset_ms)
        .unwrap_or_default();
    Some(((actual * 1000.0).round() as i64 - expected).abs())
}

fn is_retrieval_miss(candidate: &MediaMatchV3DiagnosticCandidateReport) -> bool {
    candidate
        .expectation
        .as_ref()
        .is_some_and(|expectation| expectation.must_be_retrieved)
        && !candidate.retrieved
}

fn report_metric_deltas(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
) -> Vec<MediaMatchV3ReportMetricDelta> {
    let baseline_fingerprints = fingerprint_totals(baseline);
    let current_fingerprints = fingerprint_totals(current);
    vec![
        metric_delta(
            "totalExtractionMillis",
            baseline.summary.total_extraction_millis as i128,
            current.summary.total_extraction_millis as i128,
        ),
        metric_delta(
            "totalAudioBlobBytes",
            baseline.summary.total_audio_blob_bytes as i128,
            current.summary.total_audio_blob_bytes as i128,
        ),
        metric_delta(
            "totalVideoBlobBytes",
            baseline.summary.total_video_blob_bytes as i128,
            current.summary.total_video_blob_bytes as i128,
        ),
        metric_delta(
            "totalIndexRows",
            baseline_fingerprints.index_rows,
            current_fingerprints.index_rows,
        ),
        metric_delta(
            "totalRawHitRowsProcessed",
            i128::from(baseline.summary.total_raw_hit_rows_processed),
            i128::from(current.summary.total_raw_hit_rows_processed),
        ),
        metric_delta(
            "totalRetrievalMillis",
            baseline.summary.total_retrieval_millis as i128,
            current.summary.total_retrieval_millis as i128,
        ),
        metric_delta(
            "perQueryRetrievalMillisP50",
            baseline.summary.per_query_retrieval_millis_p50 as i128,
            current.summary.per_query_retrieval_millis_p50 as i128,
        ),
        metric_delta(
            "perQueryRetrievalMillisP95",
            baseline.summary.per_query_retrieval_millis_p95 as i128,
            current.summary.per_query_retrieval_millis_p95 as i128,
        ),
        metric_delta(
            "perQueryRetrievalMillisP99",
            baseline.summary.per_query_retrieval_millis_p99 as i128,
            current.summary.per_query_retrieval_millis_p99 as i128,
        ),
        metric_delta(
            "perQueryRetrievalMillisMax",
            baseline.summary.per_query_retrieval_millis_max as i128,
            current.summary.per_query_retrieval_millis_max as i128,
        ),
        metric_delta(
            "retrievalUnaccountedMillisTotal",
            baseline.summary.retrieval_unaccounted_millis_total as i128,
            current.summary.retrieval_unaccounted_millis_total as i128,
        ),
        metric_delta(
            "retrievalUnaccountedMillisP95",
            baseline.summary.retrieval_unaccounted_millis_p95 as i128,
            current.summary.retrieval_unaccounted_millis_p95 as i128,
        ),
        metric_delta(
            "sqlHitFetchMillisTotal",
            baseline.summary.sql_hit_fetch_millis_total as i128,
            current.summary.sql_hit_fetch_millis_total as i128,
        ),
        metric_delta(
            "rustAggregationMillisTotal",
            baseline.summary.rust_aggregation_millis_total as i128,
            current.summary.rust_aggregation_millis_total as i128,
        ),
        metric_delta(
            "candidateMetadataLoadMillisTotal",
            baseline.summary.candidate_metadata_load_millis_total as i128,
            current.summary.candidate_metadata_load_millis_total as i128,
        ),
        metric_delta(
            "robustRerankMillisTotal",
            baseline.summary.robust_rerank_millis_total as i128,
            current.summary.robust_rerank_millis_total as i128,
        ),
        metric_delta(
            "runWallMillis",
            baseline.summary.run_wall_millis as i128,
            current.summary.run_wall_millis as i128,
        ),
        metric_delta(
            "fingerprintTotalMillis",
            baseline.summary.fingerprint_total_millis as i128,
            current.summary.fingerprint_total_millis as i128,
        ),
        metric_delta(
            "sqliteLoadMillis",
            baseline.summary.sqlite_load_millis as i128,
            current.summary.sqlite_load_millis as i128,
        ),
        metric_delta(
            "sqliteSaveMillis",
            baseline.summary.sqlite_save_millis as i128,
            current.summary.sqlite_save_millis as i128,
        ),
        metric_delta(
            "sqliteIndexInsertMillis",
            baseline.summary.sqlite_index_insert_millis as i128,
            current.summary.sqlite_index_insert_millis as i128,
        ),
        metric_delta(
            "decisionTotalMillis",
            baseline.summary.decision_total_millis as i128,
            current.summary.decision_total_millis as i128,
        ),
        metric_delta(
            "uniqueFreshFingerprintCount",
            baseline.summary.unique_fresh_fingerprint_count as i128,
            current.summary.unique_fresh_fingerprint_count as i128,
        ),
        metric_delta(
            "uniqueMemoryCacheFingerprintCount",
            baseline.summary.unique_memory_cache_fingerprint_count as i128,
            current.summary.unique_memory_cache_fingerprint_count as i128,
        ),
        metric_delta(
            "uniqueSqliteCacheFingerprintCount",
            baseline.summary.unique_sqlite_cache_fingerprint_count as i128,
            current.summary.unique_sqlite_cache_fingerprint_count as i128,
        ),
        metric_delta(
            "freshFingerprintReportCount",
            baseline.summary.fresh_fingerprint_report_count as i128,
            current.summary.fresh_fingerprint_report_count as i128,
        ),
        metric_delta(
            "memoryCacheFingerprintReportCount",
            baseline.summary.memory_cache_fingerprint_report_count as i128,
            current.summary.memory_cache_fingerprint_report_count as i128,
        ),
        metric_delta(
            "sqliteCacheFingerprintReportCount",
            baseline.summary.sqlite_cache_fingerprint_report_count as i128,
            current.summary.sqlite_cache_fingerprint_report_count as i128,
        ),
        metric_delta(
            "hardNegativeCount",
            baseline.summary.hard_negative_count as i128,
            current.summary.hard_negative_count as i128,
        ),
        metric_delta(
            "hardNegativePassed",
            baseline.summary.hard_negative_passed as i128,
            current.summary.hard_negative_passed as i128,
        ),
        metric_delta(
            "hardNegativeFailed",
            baseline.summary.hard_negative_failed as i128,
            current.summary.hard_negative_failed as i128,
        ),
        metric_delta(
            "sampledFingerprintCount",
            baseline.summary.sampled_fingerprint_count as i128,
            current.summary.sampled_fingerprint_count as i128,
        ),
        metric_delta(
            "fullFingerprintCount",
            baseline.summary.full_fingerprint_count as i128,
            current.summary.full_fingerprint_count as i128,
        ),
        metric_delta(
            "candidatesPromotedToFullVerify",
            baseline.summary.candidates_promoted_to_full_verify as i128,
            current.summary.candidates_promoted_to_full_verify as i128,
        ),
        metric_delta(
            "fullPromotionMillis",
            baseline.summary.full_promotion_millis as i128,
            current.summary.full_promotion_millis as i128,
        ),
        metric_delta(
            "fullPromotionCacheHits",
            baseline.summary.full_promotion_cache_hits as i128,
            current.summary.full_promotion_cache_hits as i128,
        ),
        metric_delta(
            "productionSampledIndexMillis",
            baseline.summary.production_sampled_index_millis as i128,
            current.summary.production_sampled_index_millis as i128,
        ),
        metric_delta(
            "productionFullPromotionMillis",
            baseline.summary.production_full_promotion_millis as i128,
            current.summary.production_full_promotion_millis as i128,
        ),
        metric_delta(
            "productionTotalMillis",
            baseline.summary.production_total_millis as i128,
            current.summary.production_total_millis as i128,
        ),
        metric_delta(
            "sampledIndexedFileCount",
            baseline.summary.sampled_indexed_file_count as i128,
            current.summary.sampled_indexed_file_count as i128,
        ),
        metric_delta(
            "fullPromotedFileCount",
            baseline.summary.full_promoted_file_count as i128,
            current.summary.full_promoted_file_count as i128,
        ),
        metric_delta(
            "sampledFastWorkerCount",
            baseline.summary.sampled_fast_worker_count as i128,
            current.summary.sampled_fast_worker_count as i128,
        ),
        metric_delta(
            "fullVerifyWorkerCount",
            baseline.summary.full_verify_worker_count as i128,
            current.summary.full_verify_worker_count as i128,
        ),
        metric_delta(
            "filesPerMinute",
            baseline.summary.files_per_minute as i128,
            current.summary.files_per_minute as i128,
        ),
    ]
}

#[derive(Debug, Clone, Copy, Default)]
struct FingerprintTotals {
    index_rows: i128,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReportAggregateFingerprintTotals {
    extraction_millis: u128,
    audio_blob_bytes: usize,
    video_blob_bytes: usize,
    fresh_count: usize,
    memory_cache_count: usize,
    sqlite_cache_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct FingerprintSourceCounts {
    fresh: usize,
    memory_cache: usize,
    sqlite_cache: usize,
}

impl FingerprintSourceCounts {
    fn increment(&mut self, source: &str) {
        match source {
            "fresh" => self.fresh += 1,
            "memory-cache" => self.memory_cache += 1,
            "sqlite-cache" => self.sqlite_cache += 1,
            _ => {}
        }
    }
}

fn report_aggregate_fingerprint_totals(
    report: &MediaMatchV3DiagnosticReport,
) -> ReportAggregateFingerprintTotals {
    let mut seen = BTreeSet::<(String, String)>::new();
    let mut totals = ReportAggregateFingerprintTotals::default();
    for case in &report.cases {
        add_aggregate_fingerprint_totals(
            &case.query.path,
            &case.query.diagnostics,
            &case.query.source,
            &mut seen,
            &mut totals,
        );
        for candidate in &case.candidates {
            add_aggregate_fingerprint_totals(
                &candidate.path,
                &candidate.diagnostics,
                &candidate.source,
                &mut seen,
                &mut totals,
            );
        }
        for hard_negative in &case.hard_negatives {
            add_aggregate_fingerprint_totals(
                &hard_negative.path,
                &hard_negative.diagnostics,
                &hard_negative.source,
                &mut seen,
                &mut totals,
            );
        }
    }
    totals
}

fn add_aggregate_fingerprint_totals(
    path: &str,
    diagnostics: &MediaMatchV3DiagnosticSummary,
    source: &str,
    seen: &mut BTreeSet<(String, String)>,
    totals: &mut ReportAggregateFingerprintTotals,
) {
    if seen.insert((path.to_owned(), diagnostics.profile.clone())) {
        totals.extraction_millis += diagnostics.extraction_total_millis.unwrap_or_default();
        totals.audio_blob_bytes += diagnostics.audio_blob_bytes;
        totals.video_blob_bytes += diagnostics.video_blob_bytes;
        match source {
            "fresh" => totals.fresh_count += 1,
            "memory-cache" => totals.memory_cache_count += 1,
            "sqlite-cache" => totals.sqlite_cache_count += 1,
            _ => {}
        }
    }
}

fn fingerprint_totals(report: &MediaMatchV3DiagnosticReport) -> FingerprintTotals {
    let mut seen = BTreeSet::<String>::new();
    let mut totals = FingerprintTotals::default();
    for case in &report.cases {
        add_fingerprint_totals(&case.query, &mut seen, &mut totals);
        for candidate in &case.candidates {
            add_summary_totals(
                &candidate.path,
                &candidate.diagnostics,
                &mut seen,
                &mut totals,
            );
        }
        for hard_negative in &case.hard_negatives {
            add_summary_totals(
                &hard_negative.path,
                &hard_negative.diagnostics,
                &mut seen,
                &mut totals,
            );
        }
    }
    totals
}

fn add_fingerprint_totals(
    fingerprint: &MediaMatchV3DiagnosticFingerprintReport,
    seen: &mut BTreeSet<String>,
    totals: &mut FingerprintTotals,
) {
    add_summary_totals(&fingerprint.path, &fingerprint.diagnostics, seen, totals);
}

fn add_summary_totals(
    path: &str,
    diagnostics: &MediaMatchV3DiagnosticSummary,
    seen: &mut BTreeSet<String>,
    totals: &mut FingerprintTotals,
) {
    if seen.insert(path.to_owned()) {
        totals.index_rows +=
            (diagnostics.audio_index_count + diagnostics.video_index_count) as i128;
    }
}

fn metric_delta(field: &str, baseline: i128, current: i128) -> MediaMatchV3ReportMetricDelta {
    MediaMatchV3ReportMetricDelta {
        field: field.to_owned(),
        baseline,
        current,
        delta: current - baseline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MediaMatchV3DiagnosticCaseReport, MediaMatchV3DiagnosticDecisionReport,
        MediaMatchV3DiagnosticExpectation, MediaMatchV3DiagnosticRetrievalReport,
        MediaMatchV3DiagnosticSummaryReport, current_v3_tuning,
    };

    #[test]
    fn comparison_reports_new_failure_and_nonzero_condition() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let current =
            report_with_candidate("case", "candidate.mkv", false, "Reject", "Reject", None);

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(comparison.current_has_more_failures());
        assert!(comparison.current_has_regressions());
        assert!(comparison.summary.regression);
        assert_eq!(comparison.summary.new_failures, 1);
        assert_eq!(comparison.summary.new_failed_pairs, 0);
        assert_eq!(comparison.summary.new_retrieval_misses, 1);
        assert_eq!(comparison.new_failures.len(), 1);
        assert_eq!(comparison.tier_changes.len(), 1);
        assert_eq!(comparison.class_changes.len(), 1);
        assert_eq!(comparison.retrieval_rank_changes.len(), 1);

        let value = serde_json::to_value(&comparison).expect("comparison should serialize");
        assert_eq!(value["summary"]["regression"], true);
        assert_eq!(value["summary"]["unresolvedFailure"], true);
        assert_eq!(value["summary"]["newFailures"], 1);
        assert_eq!(value["summary"]["resolvedFailures"], 0);
        assert_eq!(value["comparisonMode"], "regression");
        assert_eq!(
            value["compatibility"]["fingerprintCacheVersionMatches"],
            true
        );
        assert_eq!(
            value["compatibilityOptions"]["allowDifferentProfile"],
            false
        );
        assert_eq!(
            value["compatibilityOptions"]["allowDifferentSettings"],
            false
        );
        assert_eq!(value["compatibilityOptions"]["allowDifferentTuning"], false);
    }

    #[test]
    fn comparison_reports_production_metric_deltas() {
        let mut baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        baseline.summary.production_sampled_index_millis = 100;
        baseline.summary.production_full_promotion_millis = 40;
        baseline.summary.production_total_millis = 140;
        baseline.summary.sampled_indexed_file_count = 2;
        baseline.summary.full_promoted_file_count = 1;
        baseline.summary.sampled_fast_worker_count = 1;
        baseline.summary.full_verify_worker_count = 1;
        baseline.summary.files_per_minute = 120;

        let mut current = baseline.clone();
        current.summary.production_sampled_index_millis = 80;
        current.summary.production_full_promotion_millis = 50;
        current.summary.production_total_millis = 130;
        current.summary.sampled_indexed_file_count = 3;
        current.summary.full_promoted_file_count = 2;
        current.summary.sampled_fast_worker_count = 2;
        current.summary.full_verify_worker_count = 1;
        current.summary.files_per_minute = 180;

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");
        let value = serde_json::to_value(&comparison).expect("comparison should serialize");
        let metrics = value["metricDeltas"]
            .as_array()
            .expect("metric deltas should be an array");

        assert!(metrics.iter().any(|metric| {
            metric["field"] == "productionTotalMillis" && metric["delta"] == -10
        }));
        assert!(metrics.iter().any(|metric| {
            metric["field"] == "sampledIndexedFileCount" && metric["delta"] == 1
        }));
        assert!(
            metrics.iter().any(|metric| {
                metric["field"] == "fullPromotedFileCount" && metric["delta"] == 1
            })
        );
        assert!(
            metrics.iter().any(|metric| {
                metric["field"] == "sampledFastWorkerCount" && metric["delta"] == 1
            })
        );
        assert!(
            metrics
                .iter()
                .any(|metric| metric["field"] == "filesPerMinute" && metric["delta"] == 60)
        );
    }

    #[test]
    fn comparison_reports_resolved_failure() {
        let baseline =
            report_with_candidate("case", "candidate.mkv", false, "Reject", "Reject", None);
        let current = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_more_failures());
        assert!(!comparison.current_has_regressions());
        assert!(!comparison.current_has_unresolved_failures());
        assert_eq!(comparison.resolved_failures.len(), 1);
    }

    #[test]
    fn comparison_treats_new_failure_offset_by_resolution_as_regression() {
        let mut baseline = report_with_candidate(
            "case",
            "new-failure.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let baseline_resolved =
            report_with_candidate("case", "resolved.mkv", false, "Reject", "Reject", None);
        baseline.cases[0]
            .candidates
            .push(baseline_resolved.cases[0].candidates[0].clone());
        baseline.summary.pair_count = 2;
        baseline.summary.passed = 1;
        baseline.summary.failed = 1;
        baseline.summary.unique_fresh_fingerprint_count = 3;
        baseline.summary.fresh_fingerprint_report_count = 3;
        baseline.summary.total_extraction_millis = 30;
        baseline.summary.total_audio_blob_bytes = 300;
        baseline.summary.total_video_blob_bytes = 0;

        let mut current =
            report_with_candidate("case", "new-failure.mkv", false, "Reject", "Reject", None);
        let current_resolved = report_with_candidate(
            "case",
            "resolved.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(2),
        );
        current.cases[0]
            .candidates
            .push(current_resolved.cases[0].candidates[0].clone());
        current.summary.pair_count = 2;
        current.summary.passed = 1;
        current.summary.failed = 1;
        current.summary.unique_fresh_fingerprint_count = 3;
        current.summary.fresh_fingerprint_report_count = 3;
        current.summary.total_extraction_millis = 30;
        current.summary.total_audio_blob_bytes = 300;
        current.summary.total_video_blob_bytes = 0;

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_more_failures());
        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.new_failures, 1);
        assert_eq!(comparison.summary.resolved_failures, 1);
    }

    #[test]
    fn old_retrieval_miss_is_unresolved_but_not_regression() {
        let baseline =
            report_with_candidate("case", "candidate.mkv", false, "Reject", "Reject", None);
        let current =
            report_with_candidate("case", "candidate.mkv", false, "Reject", "Reject", None);

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_regressions());
        assert!(comparison.current_has_unresolved_failures());
        assert!(comparison.summary.unresolved_failure);
        assert_eq!(comparison.summary.retrieval_misses, 1);
        assert_eq!(comparison.summary.new_retrieval_misses, 0);
    }

    #[test]
    fn new_retrieval_miss_is_regression() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let current =
            report_with_candidate("case", "candidate.mkv", false, "Reject", "Reject", None);

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.new_retrieval_misses, 1);
        assert_eq!(comparison.new_retrieval_misses.len(), 1);
    }

    #[test]
    fn current_only_failed_pair_is_regression() {
        let baseline = report_with_candidate(
            "case",
            "baseline.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        let current_only =
            report_with_candidate("case", "current-only.mkv", false, "Reject", "Reject", None);
        current.cases[0]
            .candidates
            .push(current_only.cases[0].candidates[0].clone());
        current.summary.pair_count = 2;
        current.summary.passed = 1;
        current.summary.failed = 1;
        current.summary.unique_fresh_fingerprint_count = 3;
        current.summary.fresh_fingerprint_report_count = 3;
        current.summary.total_extraction_millis = 30;
        current.summary.total_audio_blob_bytes = 300;
        current.summary.total_video_blob_bytes = 0;

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.new_pairs, 1);
        assert_eq!(comparison.summary.new_failed_pairs, 1);
        assert_eq!(comparison.new_failed_pairs_in_current.len(), 1);
    }

    #[test]
    fn current_only_passing_pair_is_reported_but_not_regression() {
        let baseline = report_with_candidate(
            "case",
            "baseline.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        let current_only = report_with_candidate(
            "case",
            "current-only.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(2),
        );
        current.cases[0]
            .candidates
            .push(current_only.cases[0].candidates[0].clone());
        current.summary.pair_count = 2;
        current.summary.passed = 2;
        current.summary.failed = 0;
        current.summary.unique_fresh_fingerprint_count = 3;
        current.summary.fresh_fingerprint_report_count = 3;
        current.summary.total_extraction_millis = 30;
        current.summary.total_audio_blob_bytes = 300;
        current.summary.total_video_blob_bytes = 0;

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_regressions());
        assert_eq!(comparison.summary.new_pairs, 1);
        assert_eq!(comparison.summary.new_failed_pairs, 0);
    }

    #[test]
    fn comparison_reports_unknown_pairs_deterministically() {
        let baseline = report_with_candidate(
            "a",
            "baseline.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let current =
            report_with_candidate("b", "current.mkv", true, "Strong", "SameCutStrong", Some(1));

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert_eq!(comparison.missing_pairs_in_current[0].case_name, "a");
        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.new_pairs_in_current[0].case_name, "b");
    }

    #[test]
    fn missing_pair_is_regression_even_when_net_failures_decrease() {
        let mut baseline =
            report_with_candidate("case", "missing.mkv", false, "Reject", "Reject", None);
        let second =
            report_with_candidate("case", "still-present.mkv", false, "Reject", "Reject", None);
        baseline.cases[0]
            .candidates
            .push(second.cases[0].candidates[0].clone());
        baseline.summary.pair_count = 2;
        baseline.summary.passed = 0;
        baseline.summary.failed = 2;
        baseline.summary.unique_fresh_fingerprint_count = 3;
        baseline.summary.fresh_fingerprint_report_count = 3;
        baseline.summary.total_extraction_millis = 30;
        baseline.summary.total_audio_blob_bytes = 300;
        baseline.summary.total_video_blob_bytes = 0;

        let current = report_with_candidate(
            "case",
            "still-present.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_more_failures());
        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.missing_pairs, 1);
    }

    #[test]
    fn report_validation_rejects_duplicate_report_keys() {
        let mut current = report_with_candidate_id(
            "case",
            "first.mkv",
            Some("duplicate"),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let duplicate = report_with_candidate_id(
            "case",
            "second.mkv",
            Some("duplicate"),
            true,
            "Strong",
            "SameCutStrong",
            Some(2),
        );
        current.cases[0]
            .candidates
            .push(duplicate.cases[0].candidates[0].clone());
        current.summary.pair_count = 2;
        current.summary.passed = 2;
        current.summary.unique_fresh_fingerprint_count = 3;
        current.summary.fresh_fingerprint_report_count = 3;
        current.summary.total_extraction_millis = 30;
        current.summary.total_audio_blob_bytes = 300;
        current.summary.total_video_blob_bytes = 0;

        let error = validate_media_match_v3_diagnostic_report(&current)
            .expect_err("duplicate comparison keys should be invalid");

        assert!(error.contains("duplicate comparison key"));
    }

    #[test]
    fn comparison_uses_candidate_id_before_path() {
        let baseline = report_with_candidate_id(
            "case",
            "old-root/candidate.mkv",
            Some("same-pair"),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let current = report_with_candidate_id(
            "case",
            "new-root/candidate.mkv",
            Some("same-pair"),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(comparison.missing_pairs_in_current.is_empty());
        assert!(comparison.new_pairs_in_current.is_empty());
        assert_eq!(
            comparison.class_changes.first().map(|change| &change.key),
            None
        );
    }

    #[test]
    fn comparison_falls_back_to_path_without_candidate_id() {
        let baseline = report_with_candidate(
            "case",
            "old-root/candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let current = report_with_candidate(
            "case",
            "new-root/candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert_eq!(
            comparison.missing_pairs_in_current[0]
                .candidate_path
                .as_deref(),
            Some("old-root/candidate.mkv")
        );
        assert_eq!(
            comparison.new_pairs_in_current[0].candidate_path.as_deref(),
            Some("new-root/candidate.mkv")
        );
    }

    #[test]
    fn comparison_reports_retrieval_time_delta() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        current.summary.total_retrieval_millis = 9;
        current.summary.retrieval_total_millis = 9;
        current.summary.per_query_retrieval_millis_p50 = 9;
        current.summary.per_query_retrieval_millis_p95 = 9;
        current.summary.per_query_retrieval_millis_p99 = 9;
        current.summary.per_query_retrieval_millis_max = 9;
        current.cases[0].retrieval.retrieval_elapsed_ms = 9;

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");
        let delta = comparison
            .metric_deltas
            .iter()
            .find(|delta| delta.field == "totalRetrievalMillis")
            .expect("retrieval time delta should be reported");

        assert_eq!(delta.baseline, 2);
        assert_eq!(delta.current, 9);
        assert_eq!(delta.delta, 7);
        for field in [
            "uniqueFreshFingerprintCount",
            "uniqueMemoryCacheFingerprintCount",
            "uniqueSqliteCacheFingerprintCount",
            "freshFingerprintReportCount",
            "memoryCacheFingerprintReportCount",
            "sqliteCacheFingerprintReportCount",
        ] {
            assert!(
                comparison
                    .metric_deltas
                    .iter()
                    .any(|delta| delta.field == field),
                "{field} delta should be reported"
            );
        }
    }

    #[test]
    fn cold_vs_warm_source_count_deltas_do_not_fail_regression_mode() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        mark_report_sources(&mut current, "sqlite-cache");

        let comparison =
            compare_media_match_v3_reports(&baseline, &current).expect("reports should compare");

        assert!(!comparison.current_has_regressions());
        assert_metric_delta(&comparison, "uniqueFreshFingerprintCount", 2, 0, -2);
        assert_metric_delta(&comparison, "uniqueSqliteCacheFingerprintCount", 0, 2, 2);
        assert_metric_delta(&comparison, "freshFingerprintReportCount", 2, 0, -2);
        assert_metric_delta(&comparison, "sqliteCacheFingerprintReportCount", 0, 2, 2);
    }

    #[test]
    fn report_validation_accepts_generated_style_report() {
        let report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        validate_media_match_v3_diagnostic_report(&report).expect("report should validate");
    }

    #[test]
    fn report_validation_rejects_mismatched_failed_summary() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.failed = 1;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("mismatched failure count should be invalid");

        assert!(error.contains("summary.failed"));
    }

    #[test]
    fn report_validation_rejects_mismatched_pair_count() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.pair_count = 2;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("mismatched pair count should be invalid");

        assert!(error.contains("summary.pairCount"));
    }

    #[test]
    fn report_validation_rejects_blank_candidate_id() {
        let report = report_with_candidate_id(
            "case",
            "candidate.mkv",
            Some(""),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("blank candidate id should be invalid");

        assert!(error.contains("blank candidateId"));
    }

    #[test]
    fn report_validation_rejects_whitespace_candidate_id() {
        let report = report_with_candidate_id(
            "case",
            "candidate.mkv",
            Some(" candidate "),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("candidate id whitespace should be invalid");

        assert!(error.contains("leading or trailing whitespace"));
    }

    #[test]
    fn report_validation_rejects_duplicate_candidate_id_keys() {
        let mut report = report_with_candidate_id(
            "case",
            "first.mkv",
            Some("duplicate"),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let duplicate = report_with_candidate_id(
            "case",
            "second.mkv",
            Some("duplicate"),
            true,
            "Strong",
            "SameCutStrong",
            Some(2),
        );
        report.cases[0]
            .candidates
            .push(duplicate.cases[0].candidates[0].clone());
        report.summary.pair_count = 2;
        report.summary.passed = 2;
        report.summary.unique_fresh_fingerprint_count = 3;
        report.summary.fresh_fingerprint_report_count = 3;
        report.summary.total_extraction_millis = 30;
        report.summary.total_audio_blob_bytes = 300;
        report.summary.total_video_blob_bytes = 0;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("duplicate comparison keys should be invalid");

        assert!(error.contains("duplicate comparison key"));
    }

    #[test]
    fn report_validation_rejects_retrieval_summary_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.total_retrieval_millis = 99;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("retrieval total mismatch should be invalid");

        assert!(error.contains("summary.totalRetrievalMillis"));
    }

    #[test]
    fn report_validation_rejects_production_total_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.production_sampled_index_millis = 100;
        report.summary.production_full_promotion_millis = 50;
        report.summary.production_total_millis = 151;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("production total mismatch should be invalid");

        assert!(error.contains("summary.productionTotalMillis"));
    }

    #[test]
    fn report_validation_rejects_extraction_summary_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.total_extraction_millis = 999;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("extraction total mismatch should be invalid");

        assert!(error.contains("summary.totalExtractionMillis"));
    }

    #[test]
    fn report_validation_rejects_source_count_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.unique_fresh_fingerprint_count = 99;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("source count mismatch should be invalid");

        assert!(error.contains("summary.uniqueFreshFingerprintCount"));
    }

    #[test]
    fn report_validation_rejects_report_source_count_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.fresh_fingerprint_report_count = 99;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("report source count mismatch should be invalid");

        assert!(error.contains("summary.freshFingerprintReportCount"));
    }

    #[test]
    fn report_validation_rejects_audio_blob_summary_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.total_audio_blob_bytes = 999;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("audio blob total mismatch should be invalid");

        assert!(error.contains("summary.totalAudioBlobBytes"));
    }

    #[test]
    fn report_validation_rejects_video_blob_summary_mismatch() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.summary.total_video_blob_bytes = 999;

        let error = validate_media_match_v3_diagnostic_report(&report)
            .expect_err("video blob total mismatch should be invalid");

        assert!(error.contains("summary.totalVideoBlobBytes"));
    }

    #[test]
    fn report_validation_counts_duplicate_fingerprint_path_once() {
        let mut report = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        report.cases[0].candidates[0].path = "query.mkv".to_owned();
        report.summary.unique_fresh_fingerprint_count = 1;
        report.summary.total_extraction_millis = 10;
        report.summary.total_audio_blob_bytes = 100;
        report.summary.total_video_blob_bytes = 0;

        validate_media_match_v3_diagnostic_report(&report)
            .expect("duplicate fingerprint path should be counted once");
    }

    #[test]
    fn direct_compare_rejects_invalid_report() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.summary.failed = 1;

        let error = compare_media_match_v3_reports(&baseline, &current)
            .expect_err("invalid current report should be rejected");

        assert!(error.contains("current report is invalid"));
    }

    #[test]
    fn direct_compare_rejects_incompatible_profile() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.profile = "combined-v3".to_owned();

        let error = compare_media_match_v3_reports(&baseline, &current)
            .expect_err("different profile should be rejected");

        assert!(error.contains("profile differs"));
    }

    #[test]
    fn direct_compare_rejects_incompatible_settings_hash() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.settings_hash = "01".to_owned();

        let error = compare_media_match_v3_reports(&baseline, &current)
            .expect_err("different settings hash should be rejected");

        assert!(error.contains("settingsHash differs"));
    }

    #[test]
    fn direct_compare_rejects_incompatible_fingerprint_cache_version() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.fingerprint_cache_version += 1;

        let error = compare_media_match_v3_reports(&baseline, &current)
            .expect_err("different fingerprint cache version should be rejected");

        assert!(error.contains("fingerprintCacheVersion differs"));
    }

    #[test]
    fn direct_compare_rejects_incompatible_tuning() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.tuning.retrieval_prefilter_limit += 1;

        let error = compare_media_match_v3_reports(&baseline, &current)
            .expect_err("different tuning should be rejected");

        assert!(error.contains("tuning differs"));
    }

    #[test]
    fn compare_options_allow_selected_mismatch() {
        let baseline = report_with_candidate(
            "case",
            "candidate.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let mut current = baseline.clone();
        current.settings_hash = "01".to_owned();

        let comparison = compare_media_match_v3_reports_with_options(
            &baseline,
            &current,
            &MediaMatchV3ReportCompatibilityOptions {
                allow_different_settings: true,
                ..MediaMatchV3ReportCompatibilityOptions::default()
            },
        )
        .expect("allowed settings mismatch should compare");

        assert!(!comparison.compatibility.settings_hash_matches);
        assert!(comparison.compatibility_options.allow_different_settings);
        let value = serde_json::to_value(&comparison).expect("comparison should serialize");
        assert_eq!(
            value["compatibilityOptions"]["allowDifferentSettings"],
            true
        );
    }

    fn report_with_candidate(
        case_name: &str,
        candidate_path: &str,
        passed: bool,
        tier: &str,
        class: &str,
        retrieval_rank: Option<usize>,
    ) -> MediaMatchV3DiagnosticReport {
        report_with_candidate_id(
            case_name,
            candidate_path,
            None,
            passed,
            tier,
            class,
            retrieval_rank,
        )
    }

    fn report_with_candidate_id(
        case_name: &str,
        candidate_path: &str,
        candidate_id: Option<&str>,
        passed: bool,
        tier: &str,
        class: &str,
        retrieval_rank: Option<usize>,
    ) -> MediaMatchV3DiagnosticReport {
        let failed = if passed { 0 } else { 1 };
        MediaMatchV3DiagnosticReport {
            algorithm_version: 3,
            fingerprint_cache_version: crate::MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
            profile: "audio-constellation-v3".to_owned(),
            index_mode: "full".to_owned(),
            dense_audio_profile: "dense-current".to_owned(),
            settings_hash: "00".to_owned(),
            tuning: current_v3_tuning(),
            cache_root: "cache".to_owned(),
            cache_retained: true,
            generated_at_unix_millis: 1,
            cases: vec![MediaMatchV3DiagnosticCaseReport {
                name: case_name.to_owned(),
                query: fingerprint("query.mkv"),
                retrieval: MediaMatchV3DiagnosticRetrievalReport {
                    raw_hit_rows_processed: 10,
                    retrieval_elapsed_ms: 2,
                    ..MediaMatchV3DiagnosticRetrievalReport::default()
                },
                candidates: vec![MediaMatchV3DiagnosticCandidateReport {
                    candidate_id: candidate_id.map(str::to_owned),
                    path: candidate_path.to_owned(),
                    diagnostics: diagnostic_summary(candidate_path),
                    source: "fresh".to_owned(),
                    sqlite_save_millis: 0,
                    blob_encode_millis: 0,
                    index_insert_millis: 0,
                    retrieved: retrieval_rank.is_some(),
                    retrieval_rank,
                    sampled_retrieval_rank: retrieval_rank,
                    final_verified_rank: None,
                    within_promotion_budget: retrieval_rank.is_some_and(|rank| rank <= 3),
                    promotion_budget_exhausted: retrieval_rank.is_some_and(|rank| rank > 3),
                    promoted_candidate_ranks: Vec::new(),
                    first_strong_candidate_rank: None,
                    promotion_reason: None,
                    full_promotion_millis: 0,
                    decision: MediaMatchV3DiagnosticDecisionReport {
                        tier: tier.to_owned(),
                        class: Some(class.to_owned()),
                        explanation: class.to_owned(),
                        autoplay_eligible: passed && tier == "Strong" && class == "SameCutStrong",
                        offset_seconds: Some(5.0),
                        scale_ppm: Some(1_000_000),
                        segment_count: 1,
                        total_aligned_span_ms: 60_000,
                        largest_gap_ms: 0,
                        edge_only: false,
                        audio_video_conflict: false,
                        piecewise_pair_count: Some(8),
                        piecewise_hypothesis_count: Some(4),
                        piecewise_fit_millis: Some(1),
                        decision_pair_collection_millis: Some(1),
                        fast_audio_verifier_millis: Some(1),
                        global_fit_millis: Some(1),
                        offset_histogram_millis: Some(1),
                        fast_global_fit_millis: Some(1),
                        broad_global_fit_millis: Some(0),
                        global_fit_candidate_count: Some(1),
                        global_fit_inlier_count: Some(8),
                        global_fit_fallback_used: Some(false),
                        timeline_map_millis: Some(1),
                        evidence_formatting_millis: Some(1),
                        total_decision_millis: Some(5),
                    },
                    expectation: Some(MediaMatchV3DiagnosticExpectation {
                        id: candidate_id.map(str::to_owned),
                        path: candidate_path.to_owned(),
                        expected_class: Some("SameCutStrong".to_owned()),
                        minimum_tier: Some("Strong".to_owned()),
                        expected_offset_ms: Some(5_000),
                        max_offset_error_ms: Some(1_000),
                        autoplay_eligible: Some(true),
                        must_be_retrieved: true,
                        expected_retrieved: None,
                        max_retrieval_rank: None,
                        max_promotion_rank: None,
                        expect_within_promotion_budget: false,
                        skip_decision_expectation: false,
                    }),
                    passed,
                    failure_reason: (!passed).then(|| "failed".to_owned()),
                }],
                hard_negatives: Vec::new(),
            }],
            summary: MediaMatchV3DiagnosticSummaryReport {
                case_count: 1,
                pair_count: 1,
                passed: if passed { 1 } else { 0 },
                failed,
                unique_fresh_fingerprint_count: 2,
                unique_memory_cache_fingerprint_count: 0,
                unique_sqlite_cache_fingerprint_count: 0,
                fresh_fingerprint_report_count: 2,
                memory_cache_fingerprint_report_count: 0,
                sqlite_cache_fingerprint_report_count: 0,
                total_extraction_millis: 20,
                total_audio_blob_bytes: 200,
                total_video_blob_bytes: 0,
                total_raw_hit_rows_processed: 10,
                total_retrieval_millis: 2,
                per_query_retrieval_millis_p50: 2,
                per_query_retrieval_millis_p95: 2,
                per_query_retrieval_millis_p99: 2,
                per_query_retrieval_millis_max: 2,
                run_wall_millis: 20,
                fingerprint_total_millis: 20,
                retrieval_total_millis: 2,
                full_fingerprint_count: 2,
                ..MediaMatchV3DiagnosticSummaryReport::default()
            },
        }
    }

    fn fingerprint(path: &str) -> MediaMatchV3DiagnosticFingerprintReport {
        MediaMatchV3DiagnosticFingerprintReport {
            path: path.to_owned(),
            diagnostics: diagnostic_summary(path),
            source: "fresh".to_owned(),
            sqlite_save_millis: 0,
            blob_encode_millis: 0,
            index_insert_millis: 0,
        }
    }

    fn diagnostic_summary(path: &str) -> MediaMatchV3DiagnosticSummary {
        MediaMatchV3DiagnosticSummary {
            file_path: Some(path.to_owned()),
            profile: "audio-constellation-v3".to_owned(),
            index_quality: "full-verify".to_owned(),
            duration_ms: Some(60_000),
            extraction_total_millis: Some(10),
            extraction_audio_millis: Some(8),
            extraction_video_millis: Some(0),
            audio_verify_count: 10,
            video_verify_count: 0,
            audio_index_count: 5,
            video_index_count: 0,
            audio_blob_bytes: 100,
            video_blob_bytes: 0,
            retrieval_candidates_count: None,
            piecewise_pair_count: None,
            piecewise_hypothesis_count: None,
            piecewise_segment_count: None,
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
            decision_tier: None,
            decision_class: None,
            streamed_bytes: None,
            streamed_samples: None,
            peak_frames: None,
            raw_landmarks_emitted: None,
            raw_landmarks_before_bounding: None,
            raw_landmarks_kept_before_final: None,
            final_landmarks: None,
            max_buffer_samples: None,
            max_raw_landmarks_seen: None,
            max_raw_landmarks_after_compaction: None,
            raw_landmark_compactions: None,
            ffmpeg_process_wall_millis: None,
            pcm_decode_drain_millis: None,
            analyzer_millis: None,
            peak_selection_millis: None,
            pairing_millis: None,
            compaction_millis: None,
            reservoir_millis: None,
            final_selection_millis: None,
            pcm_drain_thread_millis: None,
            analyzer_thread_millis: None,
            channel_backpressure_millis: None,
            max_queued_pcm_bytes: None,
            candidate_pairs_considered: None,
            candidate_pairs_skipped_by_anchor_gate: None,
            candidate_pairs_skipped_by_target_gate: None,
            candidate_pairs_skipped_by_saturation: None,
            candidate_pairs_emitted: None,
            anchor_peaks_considered: None,
            anchor_peaks_selected: None,
            anchor_peaks_skipped_by_gate: None,
            target_peaks_considered: None,
            target_peaks_selected: None,
            landmarks_accepted_into_reservoir: None,
            landmarks_rejected_by_reservoir: None,
            reservoir_acceptance_ratio: None,
            sampled_audio_seconds_decoded: None,
            sampled_audio_windows_decoded: None,
            full_audio_seconds_decoded: None,
            effective_decoded_seconds_per_second: None,
            notes: Vec::new(),
        }
    }

    fn mark_report_sources(report: &mut MediaMatchV3DiagnosticReport, source: &str) {
        report.cases[0].query.source = source.to_owned();
        report.cases[0].query.diagnostics.extraction_total_millis = None;
        report.cases[0].query.diagnostics.extraction_audio_millis = None;
        report.cases[0].query.diagnostics.extraction_video_millis = None;
        for candidate in &mut report.cases[0].candidates {
            candidate.source = source.to_owned();
            candidate.diagnostics.extraction_total_millis = None;
            candidate.diagnostics.extraction_audio_millis = None;
            candidate.diagnostics.extraction_video_millis = None;
        }
        report.summary.unique_fresh_fingerprint_count = usize::from(source == "fresh") * 2;
        report.summary.unique_memory_cache_fingerprint_count =
            usize::from(source == "memory-cache") * 2;
        report.summary.unique_sqlite_cache_fingerprint_count =
            usize::from(source == "sqlite-cache") * 2;
        report.summary.fresh_fingerprint_report_count = usize::from(source == "fresh") * 2;
        report.summary.memory_cache_fingerprint_report_count =
            usize::from(source == "memory-cache") * 2;
        report.summary.sqlite_cache_fingerprint_report_count =
            usize::from(source == "sqlite-cache") * 2;
        report.summary.total_extraction_millis = if source == "fresh" { 20 } else { 0 };
    }

    fn assert_metric_delta(
        comparison: &MediaMatchV3ReportComparison,
        field: &str,
        baseline: i128,
        current: i128,
        delta: i128,
    ) {
        let actual = comparison
            .metric_deltas
            .iter()
            .find(|metric| metric.field == field)
            .unwrap_or_else(|| panic!("{field} metric delta should be reported"));
        assert_eq!(actual.baseline, baseline);
        assert_eq!(actual.current, current);
        assert_eq!(actual.delta, delta);
    }
}
