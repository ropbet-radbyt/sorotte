use std::{env, fs, process::ExitCode};

use sorotte_media_match::{
    MediaMatchV3DiagnosticReport, MediaMatchV3ReportComparison,
    MediaMatchV3ReportCompatibilityOptions, compare_media_match_v3_reports_with_options,
};

fn main() -> ExitCode {
    match run() {
        Ok(comparison) => {
            let should_fail = comparison_fails_selected_mode(&comparison);
            match serde_json::to_string_pretty(&comparison) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("failed serializing comparison: {error}");
                    return ExitCode::from(2);
                }
            }
            if should_fail {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<MediaMatchV3ReportComparison, String> {
    run_with_args(env::args().skip(1))
}

fn run_with_args(
    args: impl IntoIterator<Item = String>,
) -> Result<MediaMatchV3ReportComparison, String> {
    let args = parse_args(args)?;
    let baseline = read_report(&args.baseline_path)?;
    let current = read_report(&args.current_path)?;
    let mut comparison = compare_media_match_v3_reports_with_options(
        &baseline,
        &current,
        &args.compatibility_options,
    )?;
    comparison.comparison_mode = args.mode.label().to_owned();
    Ok(comparison)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonMode {
    Regression,
    Strict,
    NetFailuresOnly,
}

impl ComparisonMode {
    fn label(self) -> &'static str {
        match self {
            Self::Regression => "regression",
            Self::Strict => "strict",
            Self::NetFailuresOnly => "net-failures-only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliArgs {
    mode: ComparisonMode,
    compatibility_options: MediaMatchV3ReportCompatibilityOptions,
    baseline_path: String,
    current_path: String,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut mode = ComparisonMode::Regression;
    let mut compatibility_options = MediaMatchV3ReportCompatibilityOptions::default();
    let mut baseline_path = None;
    let mut current_path = None;
    for arg in args {
        match arg.as_str() {
            "--strict" => {
                if mode != ComparisonMode::Regression {
                    return Err(usage());
                }
                mode = ComparisonMode::Strict;
            }
            "--net-failures-only" => {
                if mode != ComparisonMode::Regression {
                    return Err(usage());
                }
                mode = ComparisonMode::NetFailuresOnly;
            }
            "--allow-different-profile" => {
                compatibility_options.allow_different_profile = true;
            }
            "--allow-different-settings" => {
                compatibility_options.allow_different_settings = true;
            }
            "--allow-different-tuning" => {
                compatibility_options.allow_different_tuning = true;
            }
            _ if baseline_path.is_none() => baseline_path = Some(arg),
            _ if current_path.is_none() => current_path = Some(arg),
            _ => return Err(usage()),
        }
    }
    let Some(baseline_path) = baseline_path else {
        return Err(usage());
    };
    let Some(current_path) = current_path else {
        return Err(usage());
    };
    Ok(CliArgs {
        mode,
        compatibility_options,
        baseline_path,
        current_path,
    })
}

fn comparison_fails_selected_mode(comparison: &MediaMatchV3ReportComparison) -> bool {
    match comparison.comparison_mode.as_str() {
        "strict" => comparison_fails_mode(ComparisonMode::Strict, comparison),
        "net-failures-only" => comparison_fails_mode(ComparisonMode::NetFailuresOnly, comparison),
        _ => comparison_fails_mode(ComparisonMode::Regression, comparison),
    }
}

fn comparison_fails_mode(mode: ComparisonMode, comparison: &MediaMatchV3ReportComparison) -> bool {
    match mode {
        ComparisonMode::Regression => comparison.current_has_regressions(),
        ComparisonMode::Strict => comparison.current_has_unresolved_failures(),
        ComparisonMode::NetFailuresOnly => comparison.current_has_more_failures(),
    }
}

fn read_report(path: &str) -> Result<MediaMatchV3DiagnosticReport, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("failed reading '{path}': {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed parsing V3 diagnostic report '{path}': {error}"))
}

fn usage() -> String {
    "usage: v3_report_compare [--strict|--net-failures-only] [--allow-different-profile] [--allow-different-settings] [--allow-different-tuning] <baseline-report.json> <current-report.json>".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_media_match::MediaMatchV3ReportCompatibility;
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use sorotte_media_match::{
        MediaMatchV3DiagnosticCandidateReport, MediaMatchV3DiagnosticCaseReport,
        MediaMatchV3DiagnosticDecisionReport, MediaMatchV3DiagnosticExpectation,
        MediaMatchV3DiagnosticFingerprintReport, MediaMatchV3DiagnosticRetrievalReport,
        MediaMatchV3DiagnosticSummary, MediaMatchV3DiagnosticSummaryReport,
        MediaMatchV3ReportComparisonSummary, MediaMatchV3ReportPairKey, current_v3_tuning,
    };

    #[test]
    fn parse_default_mode() {
        let args = parse_args(["baseline.json".to_owned(), "current.json".to_owned()])
            .expect("args should parse");

        assert_eq!(args.mode, ComparisonMode::Regression);
        assert!(!args.compatibility_options.allow_different_profile);
        assert!(!args.compatibility_options.allow_different_settings);
        assert!(!args.compatibility_options.allow_different_tuning);
        assert_eq!(args.baseline_path, "baseline.json");
        assert_eq!(args.current_path, "current.json");
    }

    #[test]
    fn parse_strict_mode() {
        let args = parse_args([
            "--strict".to_owned(),
            "baseline.json".to_owned(),
            "current.json".to_owned(),
        ])
        .expect("args should parse");

        assert_eq!(args.mode, ComparisonMode::Strict);
    }

    #[test]
    fn parse_net_failures_only_mode() {
        let args = parse_args([
            "--net-failures-only".to_owned(),
            "baseline.json".to_owned(),
            "current.json".to_owned(),
        ])
        .expect("args should parse");

        assert_eq!(args.mode, ComparisonMode::NetFailuresOnly);
    }

    #[test]
    fn parse_allow_compatibility_flags() {
        let args = parse_args([
            "--allow-different-profile".to_owned(),
            "--allow-different-settings".to_owned(),
            "--allow-different-tuning".to_owned(),
            "baseline.json".to_owned(),
            "current.json".to_owned(),
        ])
        .expect("args should parse");

        assert!(args.compatibility_options.allow_different_profile);
        assert!(args.compatibility_options.allow_different_settings);
        assert!(args.compatibility_options.allow_different_tuning);
    }

    #[test]
    fn parse_rejects_conflicting_modes() {
        assert!(
            parse_args([
                "--strict".to_owned(),
                "--net-failures-only".to_owned(),
                "baseline.json".to_owned(),
                "current.json".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn run_rejects_invalid_baseline_report() {
        let root = temp_dir("v3-report-compare-baseline-invalid");
        let baseline = root.join("baseline.json");
        let current = root.join("current.json");
        let mut invalid = report(true, Some("candidate"));
        invalid.summary.failed = 1;
        write_report(&baseline, &invalid);
        write_report(&current, &report(true, Some("candidate")));

        let error = run_with_args([
            baseline.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ])
        .expect_err("invalid baseline should be rejected");

        assert!(error.contains("baseline report is invalid"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_rejects_invalid_current_report() {
        let root = temp_dir("v3-report-compare-current-invalid");
        let baseline = root.join("baseline.json");
        let current = root.join("current.json");
        let mut invalid = report(true, Some("candidate"));
        invalid.cases[0].candidates[0].candidate_id = Some(" ".to_owned());
        write_report(&baseline, &report(true, Some("candidate")));
        write_report(&current, &invalid);

        let error = run_with_args([
            baseline.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ])
        .expect_err("invalid current should be rejected");

        assert!(error.contains("current report is invalid"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_rejects_incompatible_profile_by_default() {
        let root = temp_dir("v3-report-compare-profile-invalid");
        let baseline = root.join("baseline.json");
        let current = root.join("current.json");
        let mut different_profile = report(true, Some("candidate"));
        different_profile.profile = "combined-v3".to_owned();
        write_report(&baseline, &report(true, Some("candidate")));
        write_report(&current, &different_profile);

        let error = run_with_args([
            baseline.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ])
        .expect_err("different profile should be rejected");

        assert!(error.contains("profile differs"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_allows_selected_profile_mismatch() {
        let root = temp_dir("v3-report-compare-profile-allowed");
        let baseline = root.join("baseline.json");
        let current = root.join("current.json");
        let mut different_profile = report(true, Some("candidate"));
        different_profile.profile = "combined-v3".to_owned();
        write_report(&baseline, &report(true, Some("candidate")));
        write_report(&current, &different_profile);

        let comparison = run_with_args([
            "--allow-different-profile".to_owned(),
            baseline.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ])
        .expect("allowed profile mismatch should compare");

        assert!(!comparison.compatibility.profile_matches);
        assert!(comparison.compatibility_options.allow_different_profile);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_rejects_invalid_report_before_compatibility_check() {
        let root = temp_dir("v3-report-compare-invalid-before-compat");
        let baseline = root.join("baseline.json");
        let current = root.join("current.json");
        let mut invalid = report(true, Some("candidate"));
        invalid.summary.failed = 1;
        let mut different_profile = report(true, Some("candidate"));
        different_profile.profile = "combined-v3".to_owned();
        write_report(&baseline, &invalid);
        write_report(&current, &different_profile);

        let error = run_with_args([
            baseline.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ])
        .expect_err("invalid report should be rejected before compatibility");

        assert!(error.contains("baseline report is invalid"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selected_modes_use_distinct_failure_predicates() {
        let old_unresolved = comparison(false, true, false);

        assert!(!comparison_fails_mode(
            ComparisonMode::Regression,
            &old_unresolved
        ));
        assert!(comparison_fails_mode(
            ComparisonMode::Strict,
            &old_unresolved
        ));
        assert!(!comparison_fails_mode(
            ComparisonMode::NetFailuresOnly,
            &old_unresolved
        ));

        let new_regression = comparison(true, true, false);
        assert!(comparison_fails_mode(
            ComparisonMode::Regression,
            &new_regression
        ));

        let net_failure = comparison(false, true, true);
        assert!(comparison_fails_mode(
            ComparisonMode::NetFailuresOnly,
            &net_failure
        ));
    }

    fn comparison(
        regression: bool,
        unresolved_failure: bool,
        current_has_more_failures: bool,
    ) -> MediaMatchV3ReportComparison {
        let (baseline_failed, current_failed) = if current_has_more_failures {
            (0, 1)
        } else if unresolved_failure {
            (1, 1)
        } else {
            (0, 0)
        };
        let key = MediaMatchV3ReportPairKey {
            case_name: "case".to_owned(),
            candidate_id: Some("candidate".to_owned()),
            candidate_path: None,
        };
        MediaMatchV3ReportComparison {
            comparison_mode: "regression".to_owned(),
            compatibility: MediaMatchV3ReportCompatibility {
                algorithm_version_matches: true,
                profile_matches: true,
                settings_hash_matches: true,
                tuning_matches: true,
            },
            compatibility_options: MediaMatchV3ReportCompatibilityOptions::default(),
            summary: MediaMatchV3ReportComparisonSummary {
                regression,
                unresolved_failure,
                baseline_failed,
                current_failed,
                new_failures: usize::from(regression),
                resolved_failures: 0,
                missing_pairs: 0,
                new_pairs: 0,
                new_failed_pairs: 0,
                retrieval_misses: usize::from(unresolved_failure),
                new_retrieval_misses: usize::from(regression),
            },
            baseline_failed,
            current_failed,
            new_failures: Vec::new(),
            resolved_failures: Vec::new(),
            missing_pairs_in_current: Vec::new(),
            new_pairs_in_current: Vec::new(),
            new_failed_pairs_in_current: Vec::new(),
            retrieval_misses: unresolved_failure
                .then(|| key.clone())
                .into_iter()
                .collect(),
            new_retrieval_misses: regression.then_some(key).into_iter().collect(),
            class_changes: Vec::new(),
            tier_changes: Vec::new(),
            retrieval_rank_changes: Vec::new(),
            autoplay_eligibility_changes: Vec::new(),
            offset_error_changes: Vec::new(),
            metric_deltas: Vec::new(),
        }
    }

    fn write_report(path: &PathBuf, report: &MediaMatchV3DiagnosticReport) {
        fs::write(
            path,
            serde_json::to_string(report).expect("report should serialize"),
        )
        .expect("report should be written");
    }

    fn report(passed: bool, candidate_id: Option<&str>) -> MediaMatchV3DiagnosticReport {
        let failed = usize::from(!passed);
        MediaMatchV3DiagnosticReport {
            algorithm_version: 3,
            profile: "audio-constellation-v3".to_owned(),
            settings_hash: "00".to_owned(),
            tuning: current_v3_tuning(),
            cache_root: "cache".to_owned(),
            cache_retained: true,
            generated_at_unix_millis: 1,
            cases: vec![MediaMatchV3DiagnosticCaseReport {
                name: "case".to_owned(),
                query: MediaMatchV3DiagnosticFingerprintReport {
                    path: "query.mkv".to_owned(),
                    diagnostics: diagnostic_summary("query.mkv"),
                    source: "fresh".to_owned(),
                },
                retrieval: MediaMatchV3DiagnosticRetrievalReport {
                    raw_hit_rows_processed: 10,
                    retrieval_elapsed_ms: 2,
                    ..MediaMatchV3DiagnosticRetrievalReport::default()
                },
                candidates: vec![MediaMatchV3DiagnosticCandidateReport {
                    candidate_id: candidate_id.map(str::to_owned),
                    path: "candidate.mkv".to_owned(),
                    diagnostics: diagnostic_summary("candidate.mkv"),
                    source: "fresh".to_owned(),
                    retrieved: true,
                    retrieval_rank: Some(1),
                    decision: MediaMatchV3DiagnosticDecisionReport {
                        tier: "Strong".to_owned(),
                        class: Some("SameCutStrong".to_owned()),
                        explanation: "same cut".to_owned(),
                        autoplay_eligible: true,
                        offset_seconds: Some(0.0),
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
                        path: "candidate.mkv".to_owned(),
                        expected_class: Some("SameCutStrong".to_owned()),
                        minimum_tier: Some("Strong".to_owned()),
                        expected_offset_ms: Some(0),
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
                passed: usize::from(passed),
                failed,
                fresh_fingerprint_count: 2,
                memory_cache_fingerprint_count: 0,
                sqlite_cache_fingerprint_count: 0,
                total_extraction_millis: 20,
                total_audio_blob_bytes: 200,
                total_video_blob_bytes: 0,
                total_raw_hit_rows_processed: 10,
                total_retrieval_millis: 2,
            },
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
