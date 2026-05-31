use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use sorotte_media_match::{
    MediaMatchToolPaths, MediaMatchV3DiagnosticIndexMode, MediaMatchV3DiagnosticManifest,
    MediaMatchV3DiagnosticRunOptions, MediaMatchV3ResolvedManifest, MediaMatchV3RetrievalStrategy,
    media_match_v3_diagnostic_manifest_from_json, media_match_v3_diagnostic_manifest_report_json,
    media_match_v3_index_path, media_match_v3_sqlite_size_report, open_media_match_v3_index,
    refresh_all_anchor_stats_v3, resolve_media_match_v3_diagnostic_manifest,
    run_media_match_v3_diagnostic_manifest,
};

fn main() -> ExitCode {
    match run_cli() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run_cli() -> Result<bool, String> {
    let mut stdout = String::new();
    let result = run_cli_with_output(env::args().skip(1), &mut stdout);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    result
}

fn run_cli_with_output(
    args: impl IntoIterator<Item = String>,
    stdout: &mut String,
) -> Result<bool, String> {
    let args = parse_args(args)?;
    if args.mode == CliMode::CacheSizeReport {
        let cache_root = args
            .cache_root
            .ok_or_else(|| "--cache-size-report requires --cache-root".to_owned())?;
        let index_path = media_match_v3_index_path(&cache_root);
        let connection = Connection::open(&index_path).map_err(|error| {
            format!(
                "failed opening media-match SQLite index '{}': {error}",
                index_path.display()
            )
        })?;
        let report = media_match_v3_sqlite_size_report(&cache_root, &connection)?;
        write_json(&report, args.output_path.as_deref(), stdout)?;
        return Ok(true);
    }

    let manifest_path = args.manifest_path.ok_or_else(usage)?;
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "failed reading manifest '{}': {error}",
            manifest_path.display()
        )
    })?;
    let manifest = media_match_v3_diagnostic_manifest_from_json(&manifest_text)?;
    let manifest = filter_manifest_cases(manifest, &args.selected_cases)?;
    let manifest_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match args.mode {
        CliMode::ListCases => {
            let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir)?;
            write_case_listing(&resolved, stdout);
            return Ok(true);
        }
        CliMode::ValidateOnly => {
            let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir)?;
            validate_resolved_manifest_files_exist(&resolved)?;
            stdout.push_str("media-match V3 diagnostic manifest is valid\n");
            return Ok(true);
        }
        CliMode::PrepareIndexStats => {
            let cache_root = args
                .cache_root
                .ok_or_else(|| "--prepare-index-stats requires --cache-root".to_owned())?;
            let connection = open_media_match_v3_index(&cache_root)?;
            refresh_all_anchor_stats_v3(&connection, current_unix_millis() as i64)?;
            stdout.push_str("media-match V3 anchor stats prepared\n");
            return Ok(true);
        }
        CliMode::Run | CliMode::CacheSizeReport => {}
    }

    let supplied_cache_root = args.cache_root.is_some();
    let retain_cache = supplied_cache_root || args.keep_cache;
    let cache_root = args.cache_root.unwrap_or_else(temp_cache_root);
    let options = MediaMatchV3DiagnosticRunOptions {
        manifest_dir,
        cache_root: cache_root.clone(),
        cache_retained: retain_cache,
        refresh_cache: args.refresh_cache,
        index_mode: MediaMatchV3DiagnosticIndexMode::SampledFast,
        retrieval_benchmark_only: args.retrieval_benchmark_only,
        retrieval_strategy: args.retrieval_strategy,
        tools: default_tool_paths(),
        generated_at_unix_millis: Some(current_unix_millis()),
    };
    let mut report = run_media_match_v3_diagnostic_manifest(&manifest, &options)?;
    let passed = report.summary.failed == 0;
    if !retain_cache {
        report.cache_retained = !cleanup_temporary_cache(&cache_root);
    }
    let json = media_match_v3_diagnostic_manifest_report_json(&report)?;
    write_string(&json, args.output_path.as_deref(), stdout)?;
    Ok(passed)
}

#[derive(Debug, Clone)]
struct CliArgs {
    manifest_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    cache_root: Option<PathBuf>,
    keep_cache: bool,
    refresh_cache: bool,
    retrieval_benchmark_only: bool,
    retrieval_strategy: MediaMatchV3RetrievalStrategy,
    selected_cases: Vec<String>,
    mode: CliMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliMode {
    Run,
    ListCases,
    ValidateOnly,
    CacheSizeReport,
    PrepareIndexStats,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut parsed = CliArgs {
        manifest_path: None,
        output_path: None,
        cache_root: None,
        keep_cache: false,
        refresh_cache: false,
        retrieval_benchmark_only: false,
        retrieval_strategy: MediaMatchV3RetrievalStrategy::Auto,
        selected_cases: Vec::new(),
        mode: CliMode::Run,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(usage()),
            "--output" => parsed.output_path = Some(expect_path(&mut args, "--output")?),
            "--cache-root" => parsed.cache_root = Some(expect_path(&mut args, "--cache-root")?),
            "--keep-cache" => parsed.keep_cache = true,
            "--refresh-cache" => parsed.refresh_cache = true,
            "--list-cases" => parsed.mode = CliMode::ListCases,
            "--validate-only" => parsed.mode = CliMode::ValidateOnly,
            "--cache-size-report" => parsed.mode = CliMode::CacheSizeReport,
            "--prepare-index-stats" => parsed.mode = CliMode::PrepareIndexStats,
            "--retrieval-benchmark-only" => parsed.retrieval_benchmark_only = true,
            "--case" => parsed
                .selected_cases
                .push(expect_value(&mut args, "--case")?),
            "--retrieval-strategy" => {
                parsed.retrieval_strategy =
                    parse_retrieval_strategy(&expect_value(&mut args, "--retrieval-strategy")?)?;
            }
            "--index-mode" => {
                let value = expect_value(&mut args, "--index-mode")?;
                if value != "sampled-fast" {
                    return Err(format!(
                        "unsupported index mode '{value}'; expected sampled-fast"
                    ));
                }
            }
            value if value.starts_with("--") => return Err(format!("unknown option {value}")),
            value => {
                if parsed.manifest_path.is_some() {
                    return Err(format!("unexpected extra argument '{value}'"));
                }
                parsed.manifest_path = Some(PathBuf::from(value));
            }
        }
    }
    Ok(parsed)
}

fn expect_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn expect_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(expect_value(args, flag)?))
}

fn parse_retrieval_strategy(value: &str) -> Result<MediaMatchV3RetrievalStrategy, String> {
    match value {
        "auto" => Ok(MediaMatchV3RetrievalStrategy::Auto),
        "bucket-fetch" => Ok(MediaMatchV3RetrievalStrategy::BucketFetch),
        _ => Err(format!("unsupported retrieval strategy '{value}'")),
    }
}

fn filter_manifest_cases(
    mut manifest: MediaMatchV3DiagnosticManifest,
    selected_cases: &[String],
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    if selected_cases.is_empty() {
        return Ok(manifest);
    }
    let selected = selected_cases
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let before = manifest.cases.len();
    manifest
        .cases
        .retain(|case| selected.contains(&case.case_name));
    if manifest.cases.len() != selected.len() {
        let found = manifest
            .cases
            .iter()
            .map(|case| case.case_name.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let missing = selected
            .difference(&found)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("unknown diagnostic case(s): {missing}"));
    }
    if before > 0 && manifest.cases.is_empty() {
        return Err("no diagnostic cases selected".to_owned());
    }
    Ok(manifest)
}

fn write_case_listing(resolved: &MediaMatchV3ResolvedManifest, stdout: &mut String) {
    for case in &resolved.cases {
        stdout.push_str(&case.case_name);
        stdout.push('\n');
        stdout.push_str("  query: ");
        stdout.push_str(&case.query_path);
        stdout.push('\n');
        for candidate in &case.candidates {
            stdout.push_str("  candidate");
            if let Some(id) = &candidate.id {
                stdout.push(' ');
                stdout.push_str(id);
            }
            stdout.push_str(": ");
            stdout.push_str(&candidate.path);
            stdout.push('\n');
        }
        for negative in &case.hard_negatives {
            stdout.push_str("  hard-negative");
            if let Some(id) = &negative.id {
                stdout.push(' ');
                stdout.push_str(id);
            }
            stdout.push_str(": ");
            stdout.push_str(&negative.path);
            stdout.push('\n');
        }
    }
}

fn validate_resolved_manifest_files_exist(
    resolved: &MediaMatchV3ResolvedManifest,
) -> Result<(), String> {
    for case in &resolved.cases {
        ensure_file_exists(&case.query_path)?;
        for candidate in &case.candidates {
            ensure_file_exists(&candidate.path)?;
        }
        for negative in &case.hard_negatives {
            ensure_file_exists(&negative.path)?;
        }
    }
    Ok(())
}

fn ensure_file_exists(path: &str) -> Result<(), String> {
    if Path::new(path).is_file() {
        Ok(())
    } else {
        Err(format!("referenced media file does not exist: {path}"))
    }
}

fn write_json<T: serde::Serialize>(
    value: &T,
    output_path: Option<&Path>,
    stdout: &mut String,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed serializing JSON: {error}"))?;
    write_string(&json, output_path, stdout)
}

fn write_string(text: &str, output_path: Option<&Path>, stdout: &mut String) -> Result<(), String> {
    if let Some(output_path) = output_path {
        fs::write(output_path, text)
            .map_err(|error| format!("failed writing report '{}': {error}", output_path.display()))
    } else {
        stdout.push_str(text);
        stdout.push('\n');
        Ok(())
    }
}

fn default_tool_paths() -> MediaMatchToolPaths {
    MediaMatchToolPaths {
        ffmpeg: env::var_os("SOROTTE_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffmpeg")),
        ffprobe: env::var_os("SOROTTE_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffprobe")),
    }
}

fn temp_cache_root() -> PathBuf {
    env::temp_dir().join(format!("sorotte-media-match-v3-{}", current_unix_millis()))
}

fn cleanup_temporary_cache(path: &Path) -> bool {
    fs::remove_dir_all(path).is_ok()
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn usage() -> String {
    "usage: v3_diagnostics [--output report.json] [--cache-root dir] [--refresh-cache] [--list-cases|--validate-only|--cache-size-report|--prepare-index-stats] [--case name...] [--retrieval-benchmark-only] [--retrieval-strategy auto|bucket-fetch] <manifest.json>\n\nOnly the fixed sampled-fast V3 production policy is supported.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_mode() {
        let args = parse_args(["manifest.json".to_owned()]).unwrap();
        assert_eq!(args.manifest_path, Some(PathBuf::from("manifest.json")));
        assert_eq!(args.mode, CliMode::Run);
    }

    #[test]
    fn rejects_unsupported_index_mode() {
        let error = parse_args([
            "--index-mode".to_owned(),
            "full".to_owned(),
            "manifest.json".to_owned(),
        ])
        .unwrap_err();
        assert!(error.contains("unsupported index mode"));
    }

    #[test]
    fn parse_retrieval_benchmark() {
        let args = parse_args([
            "--retrieval-benchmark-only".to_owned(),
            "--retrieval-strategy".to_owned(),
            "bucket-fetch".to_owned(),
            "manifest.json".to_owned(),
        ])
        .unwrap();
        assert!(args.retrieval_benchmark_only);
        assert_eq!(
            args.retrieval_strategy,
            MediaMatchV3RetrievalStrategy::BucketFetch
        );
    }
}
