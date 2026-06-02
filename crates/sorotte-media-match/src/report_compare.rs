use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    MediaMatchV3DiagnosticCandidateReport, MediaMatchV3DiagnosticReport,
    MediaMatchV3DiagnosticSummaryReport,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportPairKey {
    pub case_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub candidate_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportCompatibilityOptions {
    pub allow_different_profile: bool,
    pub allow_different_settings: bool,
    pub allow_different_tuning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportCompatibility {
    pub algorithm_version_matches: bool,
    pub fingerprint_cache_version_matches: bool,
    pub profile_matches: bool,
    pub settings_hash_matches: bool,
    pub tuning_matches: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportStatusChange {
    pub key: MediaMatchV3ReportPairKey,
    pub baseline_passed: bool,
    pub current_passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportValueChange {
    pub key: MediaMatchV3ReportPairKey,
    pub field: String,
    pub baseline: String,
    pub current: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportMetricDelta {
    pub field: String,
    pub baseline: i128,
    pub current: i128,
    pub delta: i128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportComparison {
    pub comparison_mode: String,
    pub compatibility: MediaMatchV3ReportCompatibility,
    pub compatibility_options: MediaMatchV3ReportCompatibilityOptions,
    pub summary: MediaMatchV3ReportComparisonSummary,
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
    pub fn current_has_regressions(&self) -> bool {
        self.summary.regression
    }

    pub fn current_has_unresolved_failures(&self) -> bool {
        self.summary.unresolved_failure
    }

    pub fn current_has_more_failures(&self) -> bool {
        self.summary.current_failed > self.summary.baseline_failed
    }
}

pub fn validate_media_match_v3_diagnostic_report(
    report: &MediaMatchV3DiagnosticReport,
) -> Result<(), String> {
    if report.profile.trim().is_empty() {
        return Err("report profile is empty".to_owned());
    }
    if report.index_mode != "sampled-fast" {
        return Err(format!(
            "report indexMode '{}' is not supported; normal V3 reports must be sampled-fast",
            report.index_mode
        ));
    }
    if !report.sampled_policy_production_compatible {
        return Err("report sampled policy is not production compatible".to_owned());
    }
    if report.summary.case_count != report.cases.len() {
        return Err(format!(
            "summary.caseCount={} but report has {} cases",
            report.summary.case_count,
            report.cases.len()
        ));
    }
    let mut pair_count = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut keys = BTreeSet::new();
    for case in &report.cases {
        for candidate in &case.candidates {
            pair_count += 1;
            if candidate.expectation_passed {
                passed += 1;
            } else {
                failed += 1;
            }
            if let Some(id) = candidate.id.as_deref()
                && id.trim().is_empty()
            {
                return Err(format!("case '{}' has blank candidate id", case.case_name));
            }
            let key = report_pair_key(&case.case_name, candidate);
            if !keys.insert(key.clone()) {
                return Err(format!(
                    "duplicate report comparison key in case '{}': {:?}",
                    case.case_name, key
                ));
            }
        }
        for hard_negative in &case.hard_negatives {
            if !hard_negative.passed {
                failed += 1;
            }
            if let Some(id) = hard_negative.id.as_deref()
                && id.trim().is_empty()
            {
                return Err(format!(
                    "case '{}' has blank hard negative id",
                    case.case_name
                ));
            }
        }
    }
    if report.summary.pair_count != pair_count {
        return Err(format!(
            "summary.pairCount={} but report has {pair_count} candidates",
            report.summary.pair_count
        ));
    }
    if report.summary.passed != passed {
        return Err(format!(
            "summary.passed={} but report has {passed} passed candidates",
            report.summary.passed
        ));
    }
    if report.summary.failed != failed {
        return Err(format!(
            "summary.failed={} but report has {failed} failed expectations",
            report.summary.failed
        ));
    }
    Ok(())
}

pub fn validate_media_match_v3_report_pair_compatible(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
    options: &MediaMatchV3ReportCompatibilityOptions,
) -> Result<(), String> {
    let compatibility = report_compatibility(baseline, current);
    if !compatibility.algorithm_version_matches {
        return Err("reports have different algorithmVersion values".to_owned());
    }
    if !compatibility.fingerprint_cache_version_matches {
        return Err("reports have different fingerprintCacheVersion values".to_owned());
    }
    if !compatibility.profile_matches && !options.allow_different_profile {
        return Err("reports have different profile values".to_owned());
    }
    if !compatibility.settings_hash_matches && !options.allow_different_settings {
        return Err("reports have different settingsHash values".to_owned());
    }
    if !compatibility.tuning_matches && !options.allow_different_tuning {
        return Err("reports have different tuning values".to_owned());
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
        .map_err(|error| format!("invalid baseline report: {error}"))?;
    validate_media_match_v3_diagnostic_report(current)
        .map_err(|error| format!("invalid current report: {error}"))?;
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
    let baseline_pairs = report_pairs_by_key(baseline);
    let current_pairs = report_pairs_by_key(current);
    let mut new_failures = Vec::new();
    let mut resolved_failures = Vec::new();
    let mut missing_pairs_in_current = Vec::new();
    let mut new_pairs_in_current = Vec::new();
    let mut new_failed_pairs_in_current = Vec::new();
    let mut retrieval_misses = Vec::new();
    let mut new_retrieval_misses = Vec::new();
    let mut class_changes = Vec::new();
    let mut tier_changes = Vec::new();
    let mut retrieval_rank_changes = Vec::new();
    let mut autoplay_eligibility_changes = Vec::new();

    for (key, baseline_candidate) in &baseline_pairs {
        let Some(current_candidate) = current_pairs.get(key) else {
            missing_pairs_in_current.push(key.clone());
            continue;
        };
        if baseline_candidate.expectation_passed && !current_candidate.expectation_passed {
            new_failures.push(status_change(key, baseline_candidate, current_candidate));
        }
        if !baseline_candidate.expectation_passed && current_candidate.expectation_passed {
            resolved_failures.push(status_change(key, baseline_candidate, current_candidate));
        }
        let baseline_miss = retrieval_miss(baseline_candidate);
        let current_miss = retrieval_miss(current_candidate);
        if current_miss {
            retrieval_misses.push(key.clone());
        }
        if current_miss && !baseline_miss {
            new_retrieval_misses.push(key.clone());
        }
        value_change(
            &mut class_changes,
            key,
            "class",
            candidate_class(baseline_candidate),
            candidate_class(current_candidate),
        );
        value_change(
            &mut tier_changes,
            key,
            "tier",
            candidate_tier(baseline_candidate),
            candidate_tier(current_candidate),
        );
        value_change(
            &mut retrieval_rank_changes,
            key,
            "retrievalRank",
            optional_usize(baseline_candidate.retrieval_rank),
            optional_usize(current_candidate.retrieval_rank),
        );
        value_change(
            &mut autoplay_eligibility_changes,
            key,
            "autoplayEligible",
            candidate_autoplay(baseline_candidate),
            candidate_autoplay(current_candidate),
        );
    }
    for (key, current_candidate) in &current_pairs {
        if !baseline_pairs.contains_key(key) {
            new_pairs_in_current.push(key.clone());
            if !current_candidate.expectation_passed {
                new_failed_pairs_in_current.push(key.clone());
            }
        }
    }
    let baseline_failed = baseline.summary.failed;
    let current_failed = current.summary.failed;
    let regression = !new_failures.is_empty()
        || !missing_pairs_in_current.is_empty()
        || !new_failed_pairs_in_current.is_empty()
        || !new_retrieval_misses.is_empty();
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
        compatibility: report_compatibility(baseline, current),
        compatibility_options,
        summary,
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
        offset_error_changes: Vec::new(),
        metric_deltas: metric_deltas(&baseline.summary, &current.summary),
    }
}

fn report_pairs_by_key(
    report: &MediaMatchV3DiagnosticReport,
) -> BTreeMap<MediaMatchV3ReportPairKey, &MediaMatchV3DiagnosticCandidateReport> {
    let mut pairs = BTreeMap::new();
    for case in &report.cases {
        for candidate in &case.candidates {
            pairs.insert(report_pair_key(&case.case_name, candidate), candidate);
        }
    }
    pairs
}

fn report_pair_key(
    case_name: &str,
    candidate: &MediaMatchV3DiagnosticCandidateReport,
) -> MediaMatchV3ReportPairKey {
    MediaMatchV3ReportPairKey {
        case_name: case_name.to_owned(),
        candidate_id: candidate.id.clone(),
        candidate_path: if candidate.id.is_some() {
            String::new()
        } else {
            candidate.path.clone()
        },
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
        baseline_passed: baseline.expectation_passed,
        current_passed: current.expectation_passed,
        baseline_failure_reason: baseline.failure_reason.clone(),
        current_failure_reason: current.failure_reason.clone(),
    }
}

fn value_change(
    changes: &mut Vec<MediaMatchV3ReportValueChange>,
    key: &MediaMatchV3ReportPairKey,
    field: &str,
    baseline: String,
    current: String,
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

fn retrieval_miss(candidate: &MediaMatchV3DiagnosticCandidateReport) -> bool {
    candidate.expectation.must_be_retrieved && !candidate.retrieved
}

fn candidate_class(candidate: &MediaMatchV3DiagnosticCandidateReport) -> String {
    candidate
        .decision
        .as_ref()
        .and_then(|decision| decision.class)
        .map(|class| format!("{class:?}"))
        .unwrap_or_else(|| "none".to_owned())
}

fn candidate_tier(candidate: &MediaMatchV3DiagnosticCandidateReport) -> String {
    candidate
        .decision
        .as_ref()
        .map(|decision| format!("{:?}", decision.tier))
        .unwrap_or_else(|| "none".to_owned())
}

fn candidate_autoplay(candidate: &MediaMatchV3DiagnosticCandidateReport) -> String {
    candidate
        .decision
        .as_ref()
        .map(|decision| decision.autoplay_eligible.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn metric_deltas(
    baseline: &MediaMatchV3DiagnosticSummaryReport,
    current: &MediaMatchV3DiagnosticSummaryReport,
) -> Vec<MediaMatchV3ReportMetricDelta> {
    [
        metric_delta(
            "totalExtractionMillis",
            baseline.total_extraction_millis as i128,
            current.total_extraction_millis as i128,
        ),
        metric_delta(
            "totalRetrievalMillis",
            baseline.total_retrieval_millis as i128,
            current.total_retrieval_millis as i128,
        ),
        metric_delta(
            "freshFingerprintReportCount",
            baseline.fresh_fingerprint_report_count as i128,
            current.fresh_fingerprint_report_count as i128,
        ),
        metric_delta(
            "sqliteCacheFingerprintReportCount",
            baseline.sqlite_cache_fingerprint_report_count as i128,
            current.sqlite_cache_fingerprint_report_count as i128,
        ),
        metric_delta(
            "dbTotalBytes",
            baseline.db_total_bytes.unwrap_or_default() as i128,
            current.db_total_bytes.unwrap_or_default() as i128,
        ),
        metric_delta(
            "dbAnchorIndexBytes",
            baseline.db_anchor_index_bytes.unwrap_or_default() as i128,
            current.db_anchor_index_bytes.unwrap_or_default() as i128,
        ),
    ]
    .into_iter()
    .filter(|delta| delta.delta != 0)
    .collect()
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
        MediaMatchV3DiagnosticExpectation, MediaMatchV3DiagnosticFingerprintReport,
        MediaMatchV3DiagnosticRetrievalReport, MediaMatchV3DiagnosticSummary,
        MediaMatchV3RetrievalStats, current_v3_tuning,
    };

    #[test]
    fn same_retrieval_miss_is_unresolved_not_regression() {
        let baseline = report(false, true);
        let current = report(false, true);
        let comparison = compare_media_match_v3_reports(&baseline, &current).unwrap();

        assert!(comparison.current_has_unresolved_failures());
        assert!(!comparison.current_has_regressions());
    }

    #[test]
    fn new_retrieval_miss_is_regression() {
        let baseline = report(true, true);
        let current = report(false, true);
        let comparison = compare_media_match_v3_reports(&baseline, &current).unwrap();

        assert!(comparison.current_has_regressions());
    }

    fn report(retrieved: bool, passed: bool) -> MediaMatchV3DiagnosticReport {
        let candidate = MediaMatchV3DiagnosticCandidateReport {
            id: Some("candidate".to_owned()),
            path: "candidate.mkv".to_owned(),
            fingerprint: fingerprint("candidate.mkv"),
            retrieved,
            retrieval_rank: retrieved.then_some(1),
            strict_rank1_passed: retrieved,
            within_top_k: retrieved,
            top_k_retrieval_passed: retrieved,
            decision: Some(MediaMatchV3DiagnosticDecisionReport {
                tier: crate::MediaMatchTier::Probable,
                class: Some(crate::MatchClassV3::SameCutProbable),
                autoplay_eligible: false,
                explanation: String::new(),
                notes: Vec::new(),
            }),
            expectation: MediaMatchV3DiagnosticExpectation {
                must_be_retrieved: true,
                skip_decision_expectation: true,
                ..MediaMatchV3DiagnosticExpectation::default()
            },
            expectation_passed: passed,
            failure_reason: (!passed).then(|| "failed".to_owned()),
        };
        MediaMatchV3DiagnosticReport {
            schema_version: 3,
            algorithm_version: crate::MEDIA_MATCH_ALGORITHM_VERSION,
            fingerprint_cache_version: crate::MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
            profile: "audio-constellation-v3".to_owned(),
            index_mode: "sampled-fast".to_owned(),
            sampled_policy_production_compatible: true,
            settings_hash: "hash".to_owned(),
            tuning: current_v3_tuning(),
            cache_root: ".".to_owned(),
            cache_retained: true,
            generated_at_unix_millis: 0,
            retrieval_benchmark_only: false,
            cases: vec![MediaMatchV3DiagnosticCaseReport {
                case_name: "case".to_owned(),
                query: fingerprint("query.mkv"),
                retrieval: MediaMatchV3DiagnosticRetrievalReport {
                    elapsed_millis: 0,
                    raw_hit_rows_processed: 0,
                    candidates_scored: 0,
                    candidates_returned: 0,
                    stats: MediaMatchV3RetrievalStats::default(),
                    candidates: Vec::new(),
                },
                candidates: vec![candidate],
                hard_negatives: Vec::new(),
                passed,
            }],
            summary: MediaMatchV3DiagnosticSummaryReport {
                case_count: 1,
                pair_count: 1,
                passed: usize::from(passed),
                failed: usize::from(!passed),
                ..MediaMatchV3DiagnosticSummaryReport::default()
            },
            sqlite_size: None,
        }
    }

    fn fingerprint(path: &str) -> MediaMatchV3DiagnosticFingerprintReport {
        MediaMatchV3DiagnosticFingerprintReport {
            path: path.to_owned(),
            source: "fresh".to_owned(),
            diagnostics: MediaMatchV3DiagnosticSummary::default(),
            sqlite_save_millis: 0,
            blob_encode_millis: 0,
            index_insert_millis: 0,
        }
    }
}
