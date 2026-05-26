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
    pub candidate_path: String,
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
pub struct MediaMatchV3ReportComparison {
    pub baseline_failed: usize,
    pub current_failed: usize,
    pub new_failures: Vec<MediaMatchV3ReportStatusChange>,
    pub resolved_failures: Vec<MediaMatchV3ReportStatusChange>,
    pub missing_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
    pub new_pairs_in_current: Vec<MediaMatchV3ReportPairKey>,
    pub retrieval_misses: Vec<MediaMatchV3ReportPairKey>,
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
}

pub fn compare_media_match_v3_reports(
    baseline: &MediaMatchV3DiagnosticReport,
    current: &MediaMatchV3DiagnosticReport,
) -> MediaMatchV3ReportComparison {
    let baseline_pairs = report_pairs_by_key(baseline);
    let current_pairs = report_pairs_by_key(current);
    let baseline_keys = baseline_pairs.keys().cloned().collect::<BTreeSet<_>>();
    let current_keys = current_pairs.keys().cloned().collect::<BTreeSet<_>>();
    let mut new_failures = Vec::new();
    let mut resolved_failures = Vec::new();
    let mut class_changes = Vec::new();
    let mut tier_changes = Vec::new();
    let mut retrieval_rank_changes = Vec::new();
    let mut autoplay_eligibility_changes = Vec::new();
    let mut offset_error_changes = Vec::new();

    for key in baseline_keys.intersection(&current_keys) {
        let baseline_pair = baseline_pairs
            .get(key)
            .expect("intersection key should exist in baseline");
        let current_pair = current_pairs
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

    MediaMatchV3ReportComparison {
        baseline_failed: baseline.summary.failed,
        current_failed: current.summary.failed,
        new_failures,
        resolved_failures,
        missing_pairs_in_current: baseline_keys
            .difference(&current_keys)
            .cloned()
            .collect::<Vec<_>>(),
        new_pairs_in_current: current_keys
            .difference(&baseline_keys)
            .cloned()
            .collect::<Vec<_>>(),
        retrieval_misses: current_pairs
            .iter()
            .filter(|(_, pair)| {
                pair.expectation
                    .as_ref()
                    .is_some_and(|e| e.must_be_retrieved)
            })
            .filter(|(_, pair)| !pair.retrieved)
            .map(|(key, _)| key.clone())
            .collect(),
        class_changes,
        tier_changes,
        retrieval_rank_changes,
        autoplay_eligibility_changes,
        offset_error_changes,
        metric_deltas: report_metric_deltas(baseline, current),
    }
}

fn report_pairs_by_key(
    report: &MediaMatchV3DiagnosticReport,
) -> BTreeMap<MediaMatchV3ReportPairKey, &MediaMatchV3DiagnosticCandidateReport> {
    let mut pairs = BTreeMap::new();
    for case in &report.cases {
        for candidate in &case.candidates {
            pairs.insert(
                MediaMatchV3ReportPairKey {
                    case_name: case.name.clone(),
                    candidate_path: candidate.path.clone(),
                },
                candidate,
            );
        }
    }
    pairs
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
        assert_eq!(comparison.new_failures.len(), 1);
        assert_eq!(comparison.tier_changes.len(), 1);
        assert_eq!(comparison.class_changes.len(), 1);
        assert_eq!(comparison.retrieval_rank_changes.len(), 1);
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
        assert_eq!(comparison.resolved_failures.len(), 1);
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
        assert_eq!(comparison.new_pairs_in_current[0].case_name, "b");
    }

    fn report_with_candidate(
        case_name: &str,
        candidate_path: &str,
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
                    ..MediaMatchV3DiagnosticRetrievalReport::default()
                },
                candidates: vec![MediaMatchV3DiagnosticCandidateReport {
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
