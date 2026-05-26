use std::{env, fs, process::ExitCode};

use sorotte_media_match::{
    MediaMatchV3DiagnosticReport, MediaMatchV3ReportComparison, compare_media_match_v3_reports,
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
    let args = parse_args(env::args().skip(1))?;
    let baseline = read_report(&args.baseline_path)?;
    let current = read_report(&args.current_path)?;
    let mut comparison = compare_media_match_v3_reports(&baseline, &current);
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
    baseline_path: String,
    current_path: String,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut mode = ComparisonMode::Regression;
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
    "usage: v3_report_compare [--strict|--net-failures-only] <baseline-report.json> <current-report.json>".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_media_match::{MediaMatchV3ReportComparisonSummary, MediaMatchV3ReportPairKey};

    #[test]
    fn parse_default_mode() {
        let args = parse_args(["baseline.json".to_owned(), "current.json".to_owned()])
            .expect("args should parse");

        assert_eq!(args.mode, ComparisonMode::Regression);
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
                duplicate_pairs_in_baseline: 0,
                duplicate_pairs_in_current: 0,
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
            duplicate_pairs_in_baseline: Vec::new(),
            duplicate_pairs_in_current: Vec::new(),
            class_changes: Vec::new(),
            tier_changes: Vec::new(),
            retrieval_rank_changes: Vec::new(),
            autoplay_eligibility_changes: Vec::new(),
            offset_error_changes: Vec::new(),
            metric_deltas: Vec::new(),
        }
    }
}
