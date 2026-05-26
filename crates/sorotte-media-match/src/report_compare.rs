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
    pub duplicate_pairs_in_baseline: usize,
    pub duplicate_pairs_in_current: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3ReportComparison {
    pub comparison_mode: String,
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
    pub duplicate_pairs_in_baseline: Vec<MediaMatchV3ReportPairKey>,
    pub duplicate_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
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
        !self.new_failures.is_empty()
            || !self.missing_pairs_in_current.is_empty()
            || !self.new_failed_pairs_in_current.is_empty()
            || !self.new_retrieval_misses.is_empty()
            || !self.duplicate_pairs_in_baseline.is_empty()
            || !self.duplicate_pairs_in_current.is_empty()
    }

    pub fn current_has_unresolved_failures(&self) -> bool {
        self.current_failed > 0
            || !self.retrieval_misses.is_empty()
            || !self.missing_pairs_in_current.is_empty()
            || !self.duplicate_pairs_in_baseline.is_empty()
            || !self.duplicate_pairs_in_current.is_empty()
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
    let mut total_raw_hit_rows_processed = 0i64;
    let mut total_retrieval_millis = 0u128;

    for case in &report.cases {
        pair_count += case.candidates.len();
        total_raw_hit_rows_processed += case.retrieval.raw_hit_rows_processed;
        total_retrieval_millis += case.retrieval.retrieval_elapsed_ms;
        for candidate in &case.candidates {
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

    let pairs = report_pairs_by_key(report);
    if let Some(key) = pairs.duplicate_keys.first() {
        return Err(format!(
            "duplicate comparison key in report: {}",
            key.label()
        ));
    }

    Ok(())
}

pub fn compare_media_match_v3_reports(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
) -> MediaMatchV3ReportComparison {
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
    let regression = !new_failures.is_empty()
        || !missing_pairs_in_current.is_empty()
        || !new_failed_pairs_in_current.is_empty()
        || !new_retrieval_misses.is_empty()
        || !baseline_pairs.duplicate_keys.is_empty()
        || !current_pairs.duplicate_keys.is_empty();
    let unresolved_failure = current.summary.failed > 0
        || !retrieval_misses.is_empty()
        || !missing_pairs_in_current.is_empty()
        || !baseline_pairs.duplicate_keys.is_empty()
        || !current_pairs.duplicate_keys.is_empty();
    let summary = MediaMatchV3ReportComparisonSummary {
        regression,
        unresolved_failure,
        baseline_failed: baseline.summary.failed,
        current_failed: current.summary.failed,
        new_failures: new_failures.len(),
        resolved_failures: resolved_failures.len(),
        missing_pairs: missing_pairs_in_current.len(),
        new_pairs: new_pairs_in_current.len(),
        new_failed_pairs: new_failed_pairs_in_current.len(),
        retrieval_misses: retrieval_misses.len(),
        new_retrieval_misses: new_retrieval_misses.len(),
        duplicate_pairs_in_baseline: baseline_pairs.duplicate_keys.len(),
        duplicate_pairs_in_current: current_pairs.duplicate_keys.len(),
    };

    MediaMatchV3ReportComparison {
        comparison_mode: "regression".to_owned(),
        summary,
        baseline_failed: baseline.summary.failed,
        current_failed: current.summary.failed,
        new_failures,
        resolved_failures,
        missing_pairs_in_current,
        new_pairs_in_current,
        new_failed_pairs_in_current,
        retrieval_misses,
        new_retrieval_misses,
        duplicate_pairs_in_baseline: baseline_pairs.duplicate_keys,
        duplicate_pairs_in_current: current_pairs.duplicate_keys,
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
    ]
}

#[derive(Debug, Clone, Copy, Default)]
struct FingerprintTotals {
    index_rows: i128,
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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let current = report_with_candidate(
            "case",
            "still-present.mkv",
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );

        let comparison = compare_media_match_v3_reports(&baseline, &current);

        assert!(!comparison.current_has_more_failures());
        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.missing_pairs, 1);
    }

    #[test]
    fn duplicate_report_keys_are_reported_as_regressions() {
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

        let baseline = report_with_candidate_id(
            "case",
            "first.mkv",
            Some("duplicate"),
            true,
            "Strong",
            "SameCutStrong",
            Some(1),
        );
        let comparison = compare_media_match_v3_reports(&baseline, &current);

        assert!(comparison.current_has_regressions());
        assert_eq!(comparison.summary.duplicate_pairs_in_current, 1);
        assert_eq!(comparison.duplicate_pairs_in_current.len(), 1);
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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);

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

        let comparison = compare_media_match_v3_reports(&baseline, &current);
        let delta = comparison
            .metric_deltas
            .iter()
            .find(|delta| delta.field == "totalRetrievalMillis")
            .expect("retrieval time delta should be reported");

        assert_eq!(delta.baseline, 2);
        assert_eq!(delta.current, 9);
        assert_eq!(delta.delta, 7);
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
            profile: "audio-constellation-v3".to_owned(),
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
                    retrieved: retrieval_rank.is_some(),
                    retrieval_rank,
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
                    }),
                    passed,
                    failure_reason: (!passed).then(|| "failed".to_owned()),
                }],
            }],
            summary: MediaMatchV3DiagnosticSummaryReport {
                case_count: 1,
                pair_count: 1,
                passed: if passed { 1 } else { 0 },
                failed,
                total_extraction_millis: 20,
                total_audio_blob_bytes: 200,
                total_video_blob_bytes: 100,
                total_raw_hit_rows_processed: 10,
                total_retrieval_millis: 2,
            },
        }
    }

    fn fingerprint(path: &str) -> MediaMatchV3DiagnosticFingerprintReport {
        MediaMatchV3DiagnosticFingerprintReport {
            path: path.to_owned(),
            diagnostics: diagnostic_summary(path),
            source: "fresh".to_owned(),
        }
    }

    fn diagnostic_summary(path: &str) -> MediaMatchV3DiagnosticSummary {
        MediaMatchV3DiagnosticSummary {
            file_path: Some(path.to_owned()),
            profile: "audio-constellation-v3".to_owned(),
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
            decision_tier: None,
            decision_class: None,
            streamed_bytes: None,
            streamed_samples: None,
            peak_frames: None,
            raw_landmarks_before_bounding: None,
            final_landmarks: None,
            max_buffer_samples: None,
            max_raw_landmarks_seen: None,
            max_raw_landmarks_after_compaction: None,
            raw_landmark_compactions: None,
            notes: Vec::new(),
        }
    }
}
