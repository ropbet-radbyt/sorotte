use std::{env, fs, process::ExitCode};

use sorotte_media_match::{MediaMatchV3DiagnosticReport, compare_media_match_v3_reports};

fn main() -> ExitCode {
    match run() {
        Ok(comparison) => {
            let current_has_more_failures = comparison.current_has_more_failures();
            match serde_json::to_string_pretty(&comparison) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("failed serializing comparison: {error}");
                    return ExitCode::from(2);
                }
            }
            if current_has_more_failures {
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

fn run() -> Result<sorotte_media_match::MediaMatchV3ReportComparison, String> {
    let mut args = env::args().skip(1);
    let baseline_path = args.next().ok_or_else(usage)?;
    let current_path = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }
    let baseline = read_report(&baseline_path)?;
    let current = read_report(&current_path)?;
    Ok(compare_media_match_v3_reports(&baseline, &current))
}

fn read_report(path: &str) -> Result<MediaMatchV3DiagnosticReport, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("failed reading '{path}': {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed parsing V3 diagnostic report '{path}': {error}"))
}

fn usage() -> String {
    "usage: v3_report_compare <baseline-report.json> <current-report.json>".to_owned()
}
