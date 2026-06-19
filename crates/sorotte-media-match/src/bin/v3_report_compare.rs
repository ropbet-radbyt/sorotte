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
            "--allow-different-profile" => compatibility_options.allow_different_profile = true,
            "--allow-different-settings" => compatibility_options.allow_different_settings = true,
            "--allow-different-tuning" => compatibility_options.allow_different_tuning = true,
            _ if baseline_path.is_none() => baseline_path = Some(arg),
            _ if current_path.is_none() => current_path = Some(arg),
            _ => return Err(usage()),
        }
    }
    Ok(CliArgs {
        mode,
        compatibility_options,
        baseline_path: baseline_path.ok_or_else(usage)?,
        current_path: current_path.ok_or_else(usage)?,
    })
}

fn comparison_fails_selected_mode(comparison: &MediaMatchV3ReportComparison) -> bool {
    match comparison.comparison_mode.as_str() {
        "strict" => comparison.current_has_unresolved_failures(),
        "net-failures-only" => comparison.current_has_more_failures(),
        _ => comparison.current_has_regressions(),
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
}
