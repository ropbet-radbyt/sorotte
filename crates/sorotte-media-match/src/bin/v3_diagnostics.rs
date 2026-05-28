use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sorotte_media_match::{
    MediaDenseAudioProfile, MediaMatchToolPaths, MediaMatchV3DiagnosticIndexMode,
    MediaMatchV3DiagnosticManifest, MediaMatchV3DiagnosticReport, MediaMatchV3DiagnosticRunOptions,
    MediaMatchV3ResolvedManifest, MediaMatchV3ResolvedManifestCase,
    media_match_v3_diagnostic_manifest_from_json, resolve_media_match_v3_diagnostic_manifest,
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
    let CliArgs {
        manifest_path,
        output_path,
        cache_root,
        keep_cache,
        refresh_cache,
        index_mode,
        dense_audio_profile,
        bench_dense_audio_profiles,
        max_full_promotions_per_query,
        promote_expected_candidates,
        mode,
        selected_cases,
    } = parse_args(args)?;
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "failed reading manifest '{}': {error}",
            manifest_path.display()
        )
    })?;
    let manifest = media_match_v3_diagnostic_manifest_from_json(&manifest_text)?;
    let manifest_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let manifest = filter_manifest_cases(manifest, &selected_cases)?;
    if mode == CliMode::ListCases {
        let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir)?;
        write_case_listing(&resolved, stdout);
        return Ok(true);
    }
    if mode == CliMode::ValidateOnly {
        let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir)?;
        validate_resolved_manifest_files_exist(&resolved)?;
        stdout.push_str("media-match V3 diagnostic manifest is valid\n");
        return Ok(true);
    }
    let supplied_cache_root = cache_root.is_some();
    let retain_cache = supplied_cache_root || keep_cache;
    let cache_root = cache_root.unwrap_or_else(temp_cache_root);
    if bench_dense_audio_profiles {
        let mut report = match run_dense_audio_profile_benchmark(DenseAudioProfileBenchmarkRun {
            manifest: &manifest,
            manifest_dir: &manifest_dir,
            cache_root: &cache_root,
            cache_retained: retain_cache,
            refresh_cache,
            index_mode,
            max_full_promotions_per_query,
            promote_expected_candidates,
        }) {
            Ok(report) => report,
            Err(error) => {
                if !retain_cache {
                    cleanup_temporary_cache(&cache_root);
                }
                return Err(error);
            }
        };
        let passed = report.profiles.iter().all(|profile| profile.passed);
        let retain_cache_for_report = should_retain_cache_for_report(retain_cache, passed);
        if retain_cache_for_report {
            report.cache_retained = true;
            eprintln!(
                "media-match V3 diagnostic cache retained at {}",
                cache_root.display()
            );
        } else {
            report.cache_retained = !cleanup_temporary_cache(&cache_root);
        }
        let report_json = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("failed serializing dense benchmark JSON: {error}"))?;
        if let Some(output_path) = output_path {
            fs::write(&output_path, report_json).map_err(|error| {
                format!("failed writing report '{}': {error}", output_path.display())
            })?;
        } else {
            stdout.push_str(&report_json);
            stdout.push('\n');
        }
        return Ok(passed);
    }
    let mut report = match run_media_match_v3_diagnostic_manifest(
        &manifest,
        MediaMatchV3DiagnosticRunOptions {
            manifest_dir,
            cache_root: cache_root.clone(),
            cache_retained: retain_cache,
            refresh_cache,
            index_mode,
            dense_audio_profile,
            max_full_promotions_per_query,
            promote_expected_candidates,
            tools: tool_paths(),
            generated_at_unix_millis: None,
        },
    ) {
        Ok(report) => report,
        Err(error) => {
            if !retain_cache {
                cleanup_temporary_cache(&cache_root);
            }
            return Err(error);
        }
    };
    let passed = report.summary.failed == 0;
    let retain_cache_for_report = should_retain_cache_for_report(retain_cache, passed);
    if retain_cache_for_report {
        report.cache_retained = true;
        eprintln!(
            "media-match V3 diagnostic cache retained at {}",
            cache_root.display()
        );
    } else {
        report.cache_retained = !cleanup_temporary_cache(&cache_root);
    }
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("failed serializing report JSON: {error}"))?;
    if let Some(output_path) = output_path {
        fs::write(&output_path, report_json).map_err(|error| {
            format!("failed writing report '{}': {error}", output_path.display())
        })?;
    } else {
        stdout.push_str(&report_json);
        stdout.push('\n');
    }
    Ok(passed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliMode {
    Run,
    ListCases,
    ValidateOnly,
}

struct CliArgs {
    manifest_path: PathBuf,
    output_path: Option<PathBuf>,
    cache_root: Option<PathBuf>,
    keep_cache: bool,
    refresh_cache: bool,
    index_mode: MediaMatchV3DiagnosticIndexMode,
    dense_audio_profile: MediaDenseAudioProfile,
    bench_dense_audio_profiles: bool,
    max_full_promotions_per_query: usize,
    promote_expected_candidates: bool,
    mode: CliMode,
    selected_cases: Vec<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut manifest_path = None;
    let mut output_path = None;
    let mut cache_root = None;
    let mut keep_cache = false;
    let mut refresh_cache = false;
    let mut index_mode = MediaMatchV3DiagnosticIndexMode::Full;
    let mut dense_audio_profile = MediaDenseAudioProfile::DenseCurrent;
    let mut bench_dense_audio_profiles = false;
    let mut max_full_promotions_per_query = 1usize;
    let mut promote_expected_candidates = false;
    let mut mode = CliMode::Run;
    let mut selected_cases = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                output_path = Some(PathBuf::from(value));
            }
            "--cache-root" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                cache_root = Some(PathBuf::from(value));
            }
            "--keep-cache" => {
                keep_cache = true;
            }
            "--refresh-cache" => {
                refresh_cache = true;
            }
            "--index-mode" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                index_mode = parse_index_mode(&value)?;
            }
            "--dense-audio-profile" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                dense_audio_profile = parse_dense_audio_profile(&value)?;
            }
            "--bench-dense-audio-profiles" => {
                bench_dense_audio_profiles = true;
            }
            "--max-full-promotions" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                max_full_promotions_per_query = value.parse::<usize>().map_err(|_| usage())?.max(1);
            }
            "--promote-expected-candidates" => {
                promote_expected_candidates = true;
            }
            "--list-cases" => {
                if mode != CliMode::Run {
                    return Err(usage());
                }
                mode = CliMode::ListCases;
            }
            "--validate-only" => {
                if mode != CliMode::Run {
                    return Err(usage());
                }
                mode = CliMode::ValidateOnly;
            }
            "--case" => {
                let Some(value) = args.next() else {
                    return Err(usage());
                };
                selected_cases.push(value);
            }
            _ if manifest_path.is_none() => {
                manifest_path = Some(PathBuf::from(arg));
            }
            _ => return Err(usage()),
        }
    }
    let Some(manifest_path) = manifest_path else {
        return Err(usage());
    };
    Ok(CliArgs {
        manifest_path,
        output_path,
        cache_root,
        keep_cache,
        refresh_cache,
        index_mode,
        dense_audio_profile,
        bench_dense_audio_profiles,
        max_full_promotions_per_query,
        promote_expected_candidates,
        mode,
        selected_cases,
    })
}

fn usage() -> String {
    "usage: v3_diagnostics <manifest.json> [--output report.json] [--cache-root dir] [--keep-cache] [--refresh-cache] [--index-mode full|sparse-full|sampled-fast|sampled-normal|sampled|sampled-then-full|production] [--dense-audio-profile dense-current|dense-realfft|dense-8k|dense-hop2048|dense-8k-hop2048|dense-8k-window1024-hop1024|dense-max-peaks-4|dense-pair-retain-16|dense-gated|dense-fast-combined-candidate] [--bench-dense-audio-profiles] [--max-full-promotions n] [--promote-expected-candidates] [--list-cases|--validate-only] [--case name]"
        .to_owned()
}

fn parse_index_mode(value: &str) -> Result<MediaMatchV3DiagnosticIndexMode, String> {
    match value {
        "full" => Ok(MediaMatchV3DiagnosticIndexMode::Full),
        "sparse-full" => Ok(MediaMatchV3DiagnosticIndexMode::SparseFull),
        "sampled-fast" => Ok(MediaMatchV3DiagnosticIndexMode::SampledFast),
        "sampled" | "sampled-normal" => Ok(MediaMatchV3DiagnosticIndexMode::SampledNormal),
        "sampled-then-full" => Ok(MediaMatchV3DiagnosticIndexMode::SampledThenFull),
        "production" => Ok(MediaMatchV3DiagnosticIndexMode::Production),
        _ => Err(usage()),
    }
}

fn parse_dense_audio_profile(value: &str) -> Result<MediaDenseAudioProfile, String> {
    match value {
        "dense-current" => Ok(MediaDenseAudioProfile::DenseCurrent),
        "dense-realfft" => Ok(MediaDenseAudioProfile::DenseRealfft),
        "dense-8k" => Ok(MediaDenseAudioProfile::Dense8k),
        "dense-hop2048" => Ok(MediaDenseAudioProfile::DenseHop2048),
        "dense-8k-hop2048" => Ok(MediaDenseAudioProfile::Dense8kHop2048),
        "dense-8k-window1024-hop1024" => Ok(MediaDenseAudioProfile::Dense8kWindow1024Hop1024),
        "dense-max-peaks-4" => Ok(MediaDenseAudioProfile::DenseMaxPeaks4),
        "dense-pair-retain-16" => Ok(MediaDenseAudioProfile::DensePairRetain16),
        "dense-gated" => Ok(MediaDenseAudioProfile::DenseGated),
        "dense-fast-combined-candidate" => Ok(MediaDenseAudioProfile::DenseFastCombinedCandidate),
        _ => Err(usage()),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DenseAudioProfileBenchmarkReport {
    index_mode: String,
    cache_root: String,
    cache_retained: bool,
    generated_at_unix_millis: u64,
    profiles: Vec<DenseAudioProfileBenchmarkProfileReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DenseAudioProfileBenchmarkProfileReport {
    profile: String,
    passed: bool,
    summary: DenseAudioProfileBenchmarkProfileSummary,
    report: MediaMatchV3DiagnosticReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DenseAudioProfileBenchmarkProfileSummary {
    extraction_millis: u128,
    run_wall_millis: u128,
    decision_total_millis: u128,
    failed: usize,
    passed: usize,
    total_raw_hit_rows_processed: i64,
}

struct DenseAudioProfileBenchmarkRun<'a> {
    manifest: &'a MediaMatchV3DiagnosticManifest,
    manifest_dir: &'a Path,
    cache_root: &'a Path,
    cache_retained: bool,
    refresh_cache: bool,
    index_mode: MediaMatchV3DiagnosticIndexMode,
    max_full_promotions_per_query: usize,
    promote_expected_candidates: bool,
}

fn run_dense_audio_profile_benchmark(
    options: DenseAudioProfileBenchmarkRun<'_>,
) -> Result<DenseAudioProfileBenchmarkReport, String> {
    let DenseAudioProfileBenchmarkRun {
        manifest,
        manifest_dir,
        cache_root,
        cache_retained,
        refresh_cache,
        index_mode,
        max_full_promotions_per_query,
        promote_expected_candidates,
    } = options;
    if index_mode != MediaMatchV3DiagnosticIndexMode::Full {
        return Err("--bench-dense-audio-profiles requires --index-mode full".to_owned());
    }
    let generated_at_unix_millis = current_unix_millis();
    let profiles = [
        MediaDenseAudioProfile::DenseCurrent,
        MediaDenseAudioProfile::DenseRealfft,
        MediaDenseAudioProfile::Dense8k,
        MediaDenseAudioProfile::DenseHop2048,
        MediaDenseAudioProfile::Dense8kHop2048,
        MediaDenseAudioProfile::Dense8kWindow1024Hop1024,
        MediaDenseAudioProfile::DenseMaxPeaks4,
        MediaDenseAudioProfile::DensePairRetain16,
        MediaDenseAudioProfile::DenseGated,
        MediaDenseAudioProfile::DenseFastCombinedCandidate,
    ];
    let mut reports = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let report = run_media_match_v3_diagnostic_manifest(
            manifest,
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: manifest_dir.to_path_buf(),
                cache_root: cache_root.to_path_buf(),
                cache_retained,
                refresh_cache,
                index_mode,
                dense_audio_profile: profile,
                max_full_promotions_per_query,
                promote_expected_candidates,
                tools: tool_paths(),
                generated_at_unix_millis: Some(generated_at_unix_millis),
            },
        )?;
        reports.push(DenseAudioProfileBenchmarkProfileReport {
            profile: profile.label().to_owned(),
            passed: report.summary.failed == 0,
            summary: DenseAudioProfileBenchmarkProfileSummary {
                extraction_millis: report.summary.total_extraction_millis,
                run_wall_millis: report.summary.run_wall_millis,
                decision_total_millis: report.summary.decision_total_millis,
                failed: report.summary.failed,
                passed: report.summary.passed,
                total_raw_hit_rows_processed: report.summary.total_raw_hit_rows_processed,
            },
            report,
        });
    }
    Ok(DenseAudioProfileBenchmarkReport {
        index_mode: index_mode.label().to_owned(),
        cache_root: cache_root.to_string_lossy().to_string(),
        cache_retained,
        generated_at_unix_millis,
        profiles: reports,
    })
}

fn filter_manifest_cases(
    manifest: MediaMatchV3DiagnosticManifest,
    selected_cases: &[String],
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    if selected_cases.is_empty() {
        return Ok(manifest);
    }
    let requested = selected_cases
        .iter()
        .map(|case_name| case_name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut found = std::collections::BTreeSet::new();
    let cases = manifest
        .cases
        .into_iter()
        .filter(|case| {
            if requested.contains(case.name.as_str()) {
                found.insert(case.name.clone());
                true
            } else {
                false
            }
        })
        .collect::<Vec<_>>();
    let missing = requested
        .iter()
        .filter(|case_name| !found.contains::<str>(*case_name))
        .map(|case_name| (*case_name).to_owned())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "unknown diagnostic case(s): {}",
            missing.join(", ")
        ));
    }
    Ok(MediaMatchV3DiagnosticManifest { cases, ..manifest })
}

fn write_case_listing(resolved: &MediaMatchV3ResolvedManifest, output: &mut String) {
    for case in &resolved.cases {
        let _ = writeln!(output, "case: {}", case.name);
        let _ = writeln!(output, "  query: {}", case.query.display());
        for candidate in &case.candidates {
            match candidate.expectation.id.as_deref() {
                Some(id) => {
                    let _ = writeln!(
                        output,
                        "  candidate: id={} path={}",
                        id,
                        candidate.path.display()
                    );
                }
                None => {
                    let _ = writeln!(output, "  candidate: path={}", candidate.path.display());
                }
            }
        }
    }
}

fn validate_resolved_manifest_files_exist(
    resolved: &MediaMatchV3ResolvedManifest,
) -> Result<(), String> {
    for case in &resolved.cases {
        validate_manifest_file_exists(case, "query", &case.query)?;
        for candidate in &case.candidates {
            validate_manifest_file_exists(case, "candidate", &candidate.path)?;
        }
    }
    Ok(())
}

fn validate_manifest_file_exists(
    case: &MediaMatchV3ResolvedManifestCase,
    role: &str,
    path: &Path,
) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!(
            "case '{}' {} file does not exist: {}",
            case.name,
            role,
            path.display()
        ))
    }
}

fn tool_paths() -> MediaMatchToolPaths {
    MediaMatchToolPaths {
        ffmpeg: env_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg"),
        ffprobe: env_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe"),
    }
}

fn env_tool_path(env_key: &str, default: &str) -> PathBuf {
    env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn temp_cache_root() -> PathBuf {
    let mut path = env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.push(format!(
        "sorotte-v3-diagnostics-{}-{nanos}",
        std::process::id()
    ));
    path
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn cleanup_temporary_cache(cache_root: &Path) -> bool {
    match fs::remove_dir_all(cache_root) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            eprintln!(
                "warning: failed to remove temporary media-match V3 diagnostic cache '{}': {error}",
                cache_root.display()
            );
            false
        }
    }
}

fn should_retain_cache_for_report(retain_cache_requested: bool, passed: bool) -> bool {
    retain_cache_requested || !passed
}

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    use super::*;
    use sorotte_media_match::{
        compare_media_match_v3_reports, validate_media_match_v3_diagnostic_report,
    };

    #[test]
    fn parse_args_accepts_output_and_cache_root() {
        let args = parse_args([
            "manifest.json".to_owned(),
            "--output".to_owned(),
            "report.json".to_owned(),
            "--cache-root".to_owned(),
            "cache".to_owned(),
            "--keep-cache".to_owned(),
            "--refresh-cache".to_owned(),
            "--index-mode".to_owned(),
            "sampled".to_owned(),
            "--dense-audio-profile".to_owned(),
            "dense-8k-hop2048".to_owned(),
            "--max-full-promotions".to_owned(),
            "2".to_owned(),
            "--promote-expected-candidates".to_owned(),
            "--case".to_owned(),
            "copied-synthetic".to_owned(),
        ])
        .expect("args should parse");

        assert_eq!(args.manifest_path, PathBuf::from("manifest.json"));
        assert_eq!(args.output_path, Some(PathBuf::from("report.json")));
        assert_eq!(args.cache_root, Some(PathBuf::from("cache")));
        assert!(args.keep_cache);
        assert!(args.refresh_cache);
        assert_eq!(
            args.index_mode,
            MediaMatchV3DiagnosticIndexMode::SampledNormal
        );
        assert_eq!(
            args.dense_audio_profile,
            MediaDenseAudioProfile::Dense8kHop2048
        );
        assert!(!args.bench_dense_audio_profiles);
        assert_eq!(args.max_full_promotions_per_query, 2);
        assert!(args.promote_expected_candidates);
        assert_eq!(args.selected_cases, vec!["copied-synthetic"]);
        assert_eq!(args.mode, CliMode::Run);
    }

    #[test]
    fn parse_args_accepts_list_and_validate_modes() {
        let list = parse_args(["manifest.json".to_owned(), "--list-cases".to_owned()])
            .expect("list args should parse");
        assert_eq!(list.mode, CliMode::ListCases);

        let validate = parse_args(["manifest.json".to_owned(), "--validate-only".to_owned()])
            .expect("validate args should parse");
        assert_eq!(validate.mode, CliMode::ValidateOnly);
    }

    #[test]
    fn parse_args_accepts_production_index_mode() {
        let args = parse_args([
            "manifest.json".to_owned(),
            "--index-mode".to_owned(),
            "production".to_owned(),
        ])
        .expect("production mode should parse");

        assert_eq!(args.index_mode, MediaMatchV3DiagnosticIndexMode::Production);
        assert_eq!(args.max_full_promotions_per_query, 1);
    }

    #[test]
    fn parse_args_accepts_dense_audio_profile_benchmark() {
        let args = parse_args([
            "manifest.json".to_owned(),
            "--bench-dense-audio-profiles".to_owned(),
        ])
        .expect("dense benchmark mode should parse");

        assert_eq!(
            args.dense_audio_profile,
            MediaDenseAudioProfile::DenseCurrent
        );
        assert!(args.bench_dense_audio_profiles);
    }

    #[test]
    fn list_cases_does_not_require_media_files() {
        let root = temp_dir("v3-diagnostics-list");
        let manifest_path = write_manifest(
            &root,
            serde_json::json!({
                "profile": "audio-constellation-v3",
                "cases": [
                    {
                        "name": "alpha",
                        "query": "missing-query-a.mkv",
                        "candidates": [{
                            "id": "alpha-candidate",
                            "path": "missing-candidate-a.mkv"
                        }]
                    },
                    {
                        "name": "beta",
                        "query": "missing-query-b.mkv",
                        "candidates": [{
                            "path": "missing-candidate-b.mkv"
                        }]
                    }
                ]
            }),
        );
        let mut output = String::new();

        let passed = run_cli_with_output(
            [
                manifest_path.to_string_lossy().to_string(),
                "--list-cases".to_owned(),
                "--case".to_owned(),
                "alpha".to_owned(),
            ],
            &mut output,
        )
        .expect("case listing should not fingerprint or check media existence");

        assert!(passed);
        assert!(output.contains("case: alpha"));
        assert!(output.contains("id=alpha-candidate"));
        assert!(!output.contains("case: beta"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_only_reports_missing_files() {
        let root = temp_dir("v3-diagnostics-validate");
        let manifest_path = write_manifest(
            &root,
            serde_json::json!({
                "profile": "audio-constellation-v3",
                "cases": [{
                    "name": "missing",
                    "query": "missing-query.mkv",
                    "candidates": [{
                        "path": "missing-candidate.mkv"
                    }]
                }]
            }),
        );
        let mut output = String::new();

        let error = run_cli_with_output(
            [
                manifest_path.to_string_lossy().to_string(),
                "--validate-only".to_owned(),
            ],
            &mut output,
        )
        .expect_err("validate-only should reject missing files");

        assert!(error.contains("does not exist"));
        assert!(output.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_case_filter_errors_clearly() {
        let root = temp_dir("v3-diagnostics-case-filter");
        let manifest_path = write_manifest(
            &root,
            serde_json::json!({
                "profile": "audio-constellation-v3",
                "cases": [{
                    "name": "known",
                    "query": "query.mkv",
                    "candidates": [{
                        "path": "candidate.mkv"
                    }]
                }]
            }),
        );
        let mut output = String::new();

        let error = run_cli_with_output(
            [
                manifest_path.to_string_lossy().to_string(),
                "--list-cases".to_owned(),
                "--case".to_owned(),
                "unknown".to_owned(),
            ],
            &mut output,
        )
        .expect_err("unknown case filter should fail");

        assert!(error.contains("unknown diagnostic case"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_temporary_cache_removes_cache_root() {
        let root = temp_dir("v3-diagnostics-cleanup");
        fs::create_dir_all(root.join("cache").join("media-match"))
            .expect("cache dir should be created");
        fs::write(
            root.join("cache").join("media-match").join("sentinel"),
            b"x",
        )
        .expect("sentinel should be written");

        assert!(cleanup_temporary_cache(&root));
        assert!(!root.exists());
    }

    #[test]
    fn failed_expectation_reports_retain_temporary_cache() {
        assert!(!should_retain_cache_for_report(false, true));
        assert!(should_retain_cache_for_report(false, false));
        assert!(should_retain_cache_for_report(true, true));
        assert!(should_retain_cache_for_report(true, false));
    }

    #[test]
    #[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
    fn v3_manifest_harness_runs_small_synthetic_case() {
        let Some(ffmpeg) = test_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg") else {
            eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
            return;
        };
        if test_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe").is_none() {
            eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
            return;
        }
        let root = temp_dir("v3-diagnostics");
        let media_dir = root.join("media");
        fs::create_dir_all(&media_dir).expect("media dir should be created");
        let query = media_dir.join("query.wav");
        let candidate = media_dir.join("candidate.wav");
        generate_synthetic_audio(&ffmpeg, &query);
        fs::copy(&query, &candidate).expect("candidate copy should succeed");
        let manifest = serde_json::json!({
            "profile": "audio-constellation-v3",
            "baseDir": "media",
            "cases": [{
                "name": "copied-synthetic",
                "query": "query.wav",
                "candidates": [{
                    "path": "candidate.wav",
                    "mustBeRetrieved": true
                }]
            }]
        });
        let cache_root = root.join("cache-root");
        let report = run_media_match_v3_diagnostic_manifest(
            &media_match_v3_diagnostic_manifest_from_json(&manifest.to_string())
                .expect("manifest should parse"),
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: true,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                tools: tool_paths(),
                generated_at_unix_millis: Some(123),
            },
        )
        .expect("diagnostic harness should run");
        let report_json = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(report.summary.failed, 0, "{report_json}");
        assert_eq!(
            report.summary.unique_fresh_fingerprint_count, 2,
            "{report_json}"
        );
        assert_eq!(
            report.summary.fresh_fingerprint_report_count, 2,
            "{report_json}"
        );
        assert_eq!(
            report.summary.unique_sqlite_cache_fingerprint_count, 0,
            "{report_json}"
        );
        assert_eq!(
            report.summary.sqlite_cache_fingerprint_report_count, 0,
            "{report_json}"
        );
        assert_eq!(report.cases[0].query.source, "fresh");
        assert_eq!(report.cases[0].candidates[0].source, "fresh");
        assert_eq!(report_json["generatedAtUnixMillis"], 123);
        assert!(
            report_json["cases"][0]["retrieval"]["queryBucketsTotal"]
                .as_i64()
                .unwrap_or_default()
                >= 0
        );
        assert!(
            report_json["cases"][0]["retrieval"]["retrievedCandidates"]
                .as_array()
                .is_some()
        );
        assert!(
            report_json["cases"][0]["query"]["diagnostics"]["audioBlobBytes"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        validate_media_match_v3_diagnostic_report(&report).expect("cold report should validate");
        assert_report_self_compares(&report);

        let warm_report = run_media_match_v3_diagnostic_manifest(
            &media_match_v3_diagnostic_manifest_from_json(&manifest.to_string())
                .expect("manifest should parse"),
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: false,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                tools: tool_paths(),
                generated_at_unix_millis: Some(124),
            },
        )
        .expect("warm diagnostic harness run should use sqlite cache");

        assert_eq!(warm_report.summary.failed, 0);
        assert_eq!(warm_report.summary.unique_fresh_fingerprint_count, 0);
        assert_eq!(warm_report.summary.unique_sqlite_cache_fingerprint_count, 2);
        assert_eq!(warm_report.summary.fresh_fingerprint_report_count, 0);
        assert_eq!(warm_report.summary.sqlite_cache_fingerprint_report_count, 2);
        assert_eq!(warm_report.summary.total_extraction_millis, 0);
        assert_eq!(warm_report.cases[0].query.source, "sqlite-cache");
        assert_eq!(warm_report.cases[0].candidates[0].source, "sqlite-cache");
        assert!(
            warm_report.summary.total_extraction_millis <= report.summary.total_extraction_millis
        );
        assert_eq!(
            warm_report.cases[0].candidates[0].decision.tier,
            report.cases[0].candidates[0].decision.tier
        );
        assert_eq!(
            warm_report.cases[0].candidates[0].decision.class,
            report.cases[0].candidates[0].decision.class
        );
        validate_media_match_v3_diagnostic_report(&warm_report)
            .expect("warm report should validate");
        assert_report_self_compares(&warm_report);

        let refresh_report = run_media_match_v3_diagnostic_manifest(
            &media_match_v3_diagnostic_manifest_from_json(&manifest.to_string())
                .expect("manifest should parse"),
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: cache_root.clone(),
                cache_retained: true,
                refresh_cache: true,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                tools: tool_paths(),
                generated_at_unix_millis: Some(125),
            },
        )
        .expect("refresh run should bypass sqlite cache");

        assert_eq!(refresh_report.summary.failed, 0);
        assert_eq!(refresh_report.summary.unique_fresh_fingerprint_count, 2);
        assert_eq!(
            refresh_report.summary.unique_sqlite_cache_fingerprint_count,
            0
        );
        assert_eq!(refresh_report.summary.fresh_fingerprint_report_count, 2);
        assert_eq!(
            refresh_report.summary.sqlite_cache_fingerprint_report_count,
            0
        );
        assert_eq!(refresh_report.cases[0].query.source, "fresh");
        assert_eq!(refresh_report.cases[0].candidates[0].source, "fresh");
        assert_eq!(
            refresh_report.cases[0].candidates[0].decision.tier,
            report.cases[0].candidates[0].decision.tier
        );
        assert_eq!(
            refresh_report.cases[0].candidates[0].decision.class,
            report.cases[0].candidates[0].decision.class
        );
        validate_media_match_v3_diagnostic_report(&refresh_report)
            .expect("refresh report should validate");
        assert_report_self_compares(&refresh_report);

        let duplicate_manifest = serde_json::json!({
            "profile": "audio-constellation-v3",
            "baseDir": "media",
            "cases": [{
                "name": "duplicate-synthetic",
                "query": "query.wav",
                "candidates": [{
                    "path": "query.wav"
                }]
            }]
        });
        let duplicate_refresh_report = run_media_match_v3_diagnostic_manifest(
            &media_match_v3_diagnostic_manifest_from_json(&duplicate_manifest.to_string())
                .expect("duplicate manifest should parse"),
            MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root,
                cache_retained: true,
                refresh_cache: true,
                index_mode: MediaMatchV3DiagnosticIndexMode::Full,
                dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
                max_full_promotions_per_query: 1,
                promote_expected_candidates: false,
                tools: tool_paths(),
                generated_at_unix_millis: Some(126),
            },
        )
        .expect("refresh run should still use memory-cache for duplicate paths");

        assert_eq!(duplicate_refresh_report.summary.failed, 0);
        assert_eq!(
            duplicate_refresh_report
                .summary
                .unique_fresh_fingerprint_count,
            1
        );
        assert_eq!(
            duplicate_refresh_report
                .summary
                .fresh_fingerprint_report_count,
            1
        );
        assert_eq!(
            duplicate_refresh_report
                .summary
                .memory_cache_fingerprint_report_count,
            1
        );
        assert_eq!(duplicate_refresh_report.cases[0].query.source, "fresh");
        assert_eq!(
            duplicate_refresh_report.cases[0].candidates[0].source,
            "memory-cache"
        );
        validate_media_match_v3_diagnostic_report(&duplicate_refresh_report)
            .expect("duplicate refresh report should validate");
        assert_report_self_compares(&duplicate_refresh_report);
        let _ = fs::remove_dir_all(root);
    }

    fn assert_report_self_compares(report: &sorotte_media_match::MediaMatchV3DiagnosticReport) {
        let comparison =
            compare_media_match_v3_reports(report, report).expect("report should self-compare");
        assert!(!comparison.current_has_regressions());
        assert!(!comparison.current_has_unresolved_failures());
        assert_eq!(comparison.summary.missing_pairs, 0);
        assert_eq!(comparison.summary.new_pairs, 0);
        assert_eq!(comparison.summary.new_failures, 0);
    }

    fn test_tool_path(env_key: &str, default_name: &str) -> Option<PathBuf> {
        let path = env::var_os(env_key)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default_name));
        let status = Command::new(&path)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        status.success().then_some(path)
    }

    fn generate_synthetic_audio(ffmpeg: &Path, path: &Path) {
        let status = Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.7*sin(2*PI*(220+30*t)*t)+0.4*sin(2*PI*(660+17*t)*t):s=44100:d=24",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(path)
            .status()
            .expect("ffmpeg should run");
        assert!(status.success(), "ffmpeg synthetic audio generation failed");
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

    fn write_manifest(root: &Path, value: serde_json::Value) -> PathBuf {
        let path = root.join("manifest.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&value).expect("manifest should serialize"),
        )
        .expect("manifest should be written");
        path
    }
}
