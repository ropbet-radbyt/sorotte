use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sorotte_media_match::{
    InstrumentedMediaFingerprint, MEDIA_MATCH_ANCHOR_VERSION, MatchClassV3,
    MediaExtractionSettings, MediaMatchAutoplayPolicy, MediaMatchDecision, MediaMatchSettings,
    MediaMatchTier, MediaMatchToolPaths, V3Tuning, current_v3_tuning, decide_media_match,
    fingerprint_media_file_with_report, summarize_decision_v3_diagnostics,
    summarize_instrumented_record_v3_diagnostics,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default = "default_profile")]
    profile: String,
    cases: Vec<ManifestCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestCase {
    name: String,
    query: String,
    candidates: Vec<ManifestCandidate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestCandidate {
    path: String,
    expected_class: Option<String>,
    minimum_tier: Option<String>,
    expected_offset_ms: Option<i64>,
    max_offset_error_ms: Option<i64>,
    autoplay_eligible: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticReport {
    algorithm_version: u32,
    profile: String,
    generated_at_unix_millis: u64,
    tuning: V3Tuning,
    cases: Vec<CaseReport>,
    summary: ReportSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    name: String,
    query: FingerprintReport,
    candidates: Vec<CandidateReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintReport {
    path: String,
    diagnostics: sorotte_media_match::MediaMatchV3DiagnosticSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateReport {
    path: String,
    diagnostics: sorotte_media_match::MediaMatchV3DiagnosticSummary,
    decision: DecisionReport,
    expectation: Option<ManifestCandidate>,
    passed: bool,
    failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionReport {
    tier: String,
    class: Option<String>,
    explanation: String,
    offset_seconds: Option<f64>,
    scale_ppm: Option<i32>,
    segment_count: usize,
    total_aligned_span_ms: u32,
    largest_gap_ms: u32,
    edge_only: bool,
    audio_video_conflict: bool,
    piecewise_pair_count: Option<usize>,
    piecewise_hypothesis_count: Option<usize>,
    piecewise_fit_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReportSummary {
    case_count: usize,
    pair_count: usize,
    passed: usize,
    failed: usize,
    total_extraction_millis: u128,
    total_audio_blob_bytes: usize,
    total_video_blob_bytes: usize,
}

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
    let CliArgs {
        manifest_path,
        output_path,
    } = parse_args(env::args().skip(1))?;
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "failed reading manifest '{}': {error}",
            manifest_path.display()
        )
    })?;
    let manifest = parse_manifest(&manifest_text)?;
    let report = run_manifest(&manifest)?;
    let passed = report.summary.failed == 0;
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("failed serializing report JSON: {error}"))?;
    if let Some(output_path) = output_path {
        fs::write(&output_path, report_json).map_err(|error| {
            format!("failed writing report '{}': {error}", output_path.display())
        })?;
    } else {
        println!("{report_json}");
    }
    Ok(passed)
}

struct CliArgs {
    manifest_path: PathBuf,
    output_path: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut manifest_path = None;
    let mut output_path = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--output" {
            let Some(value) = args.next() else {
                return Err("usage: v3_diagnostics <manifest.json> [--output report.json]".into());
            };
            output_path = Some(PathBuf::from(value));
        } else if manifest_path.is_none() {
            manifest_path = Some(PathBuf::from(arg));
        } else {
            return Err("usage: v3_diagnostics <manifest.json> [--output report.json]".into());
        }
    }
    let Some(manifest_path) = manifest_path else {
        return Err("usage: v3_diagnostics <manifest.json> [--output report.json]".into());
    };
    Ok(CliArgs {
        manifest_path,
        output_path,
    })
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    serde_json::from_str(text).map_err(|error| format!("failed parsing manifest JSON: {error}"))
}

fn run_manifest(manifest: &Manifest) -> Result<DiagnosticReport, String> {
    let settings = settings_for_profile(&manifest.profile)?;
    let tools = tool_paths();
    let autoplay_settings = diagnostic_decision_settings();
    let mut records = BTreeMap::<String, InstrumentedMediaFingerprint>::new();
    let mut cases = Vec::new();
    let mut summary = ReportSummary {
        case_count: manifest.cases.len(),
        ..ReportSummary::default()
    };
    for case in &manifest.cases {
        let query = fingerprint_cached(&mut records, &case.query, &tools, &settings)?.clone();
        let query_report = FingerprintReport {
            path: case.query.clone(),
            diagnostics: summarize_instrumented_record_v3_diagnostics(&query),
        };
        let mut candidate_reports = Vec::new();
        for candidate in &case.candidates {
            let candidate_fingerprint =
                fingerprint_cached(&mut records, &candidate.path, &tools, &settings)?.clone();
            let decision = decide_media_match(
                &query.record,
                &candidate_fingerprint.record,
                &autoplay_settings,
            );
            let decision_report = DecisionReport::from_decision(&decision);
            let failures = evaluate_expectation(&decision, candidate, &autoplay_settings);
            let passed = failures.is_empty();
            summary.pair_count += 1;
            if passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            candidate_reports.push(CandidateReport {
                path: candidate.path.clone(),
                diagnostics: summarize_instrumented_record_v3_diagnostics(&candidate_fingerprint),
                decision: decision_report,
                expectation: Some(candidate.clone()),
                passed,
                failure_reason: (!failures.is_empty()).then(|| failures.join("; ")),
            });
        }
        cases.push(CaseReport {
            name: case.name.clone(),
            query: query_report,
            candidates: candidate_reports,
        });
    }
    for fingerprint in records.values() {
        let diagnostics = summarize_instrumented_record_v3_diagnostics(fingerprint);
        summary.total_extraction_millis += fingerprint.report.timings.total_millis;
        summary.total_audio_blob_bytes += diagnostics.audio_blob_bytes;
        summary.total_video_blob_bytes += diagnostics.video_blob_bytes;
    }
    Ok(DiagnosticReport {
        algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: settings.profile.label().to_owned(),
        generated_at_unix_millis: unix_millis_now(),
        tuning: current_v3_tuning(),
        cases,
        summary,
    })
}

fn diagnostic_decision_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    }
}

fn fingerprint_cached<'a>(
    records: &'a mut BTreeMap<String, InstrumentedMediaFingerprint>,
    path: &str,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
) -> Result<&'a InstrumentedMediaFingerprint, String> {
    if !records.contains_key(path) {
        let fingerprint = fingerprint_media_file_with_report(path, tools, settings, None)
            .map_err(|error| format!("failed fingerprinting '{path}': {error}"))?;
        records.insert(path.to_owned(), fingerprint);
    }
    records
        .get(path)
        .ok_or_else(|| format!("fingerprint cache missed '{path}'"))
}

fn evaluate_expectation(
    decision: &MediaMatchDecision,
    expected: &ManifestCandidate,
    autoplay_settings: &MediaMatchSettings,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected_class) = expected.expected_class.as_deref() {
        match parse_match_class(expected_class) {
            Some(class) if Some(class) == decision.evidence.v3_class => {}
            Some(_) => failures.push(format!(
                "expected class {expected_class}, got {}",
                decision
                    .evidence
                    .v3_class
                    .map(|class| format!("{class:?}"))
                    .unwrap_or_else(|| "None".to_owned())
            )),
            None => failures.push(format!("unknown expected class {expected_class}")),
        }
    }
    if let Some(minimum_tier) = expected.minimum_tier.as_deref() {
        match parse_tier(minimum_tier) {
            Some(tier) if tier_score(decision.tier) >= tier_score(tier) => {}
            Some(_) => failures.push(format!(
                "expected tier at least {minimum_tier}, got {:?}",
                decision.tier
            )),
            None => failures.push(format!("unknown expected tier {minimum_tier}")),
        }
    }
    if let Some(max_offset_error_ms) = expected.max_offset_error_ms {
        match decision.evidence.alignment.as_ref() {
            Some(alignment) => {
                let actual_offset_ms = (alignment.offset_seconds * 1000.0).round() as i64;
                let expected_offset_ms = expected.expected_offset_ms.unwrap_or(0);
                let offset_error_ms = (actual_offset_ms - expected_offset_ms).abs();
                if offset_error_ms > max_offset_error_ms {
                    if expected.expected_offset_ms.is_some() {
                        failures.push(format!(
                            "expected offset {expected_offset_ms}ms +/- {max_offset_error_ms}ms, got {actual_offset_ms}ms (error {offset_error_ms}ms)"
                        ));
                    } else {
                        failures.push(format!(
                            "expected absolute offset <= {max_offset_error_ms}ms, got {actual_offset_ms}ms"
                        ));
                    }
                }
            }
            None => failures.push("expected offset evidence, got none".to_owned()),
        }
    }
    if let Some(expected_autoplay) = expected.autoplay_eligible {
        let actual = decision.same_media_for_autoplay(autoplay_settings);
        if actual != expected_autoplay {
            failures.push(format!(
                "expected autoplayEligible={expected_autoplay}, got {actual}"
            ));
        }
    }
    failures
}

impl DecisionReport {
    fn from_decision(decision: &MediaMatchDecision) -> Self {
        let map = decision.evidence.timeline_map_v3.as_ref();
        let summary = summarize_decision_v3_diagnostics(decision);
        Self {
            tier: format!("{:?}", decision.tier),
            class: decision.evidence.v3_class.map(|class| format!("{class:?}")),
            explanation: decision.explanation.clone(),
            offset_seconds: decision
                .evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.offset_seconds),
            scale_ppm: decision
                .evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.scale_ppm),
            segment_count: map.map(|map| map.segments.len()).unwrap_or_default(),
            total_aligned_span_ms: map.map(|map| map.total_aligned_span_ms).unwrap_or_default(),
            largest_gap_ms: map.map(|map| map.largest_gap_ms).unwrap_or_default(),
            edge_only: map.map(|map| map.edge_only).unwrap_or(false),
            audio_video_conflict: map.map(|map| map.audio_video_conflict).unwrap_or(false),
            piecewise_pair_count: summary.piecewise_pair_count,
            piecewise_hypothesis_count: summary.piecewise_hypothesis_count,
            piecewise_fit_millis: summary.piecewise_fit_millis,
        }
    }
}

fn settings_for_profile(profile: &str) -> Result<MediaExtractionSettings, String> {
    match normalized_label(profile).as_str() {
        "audioconstellationv3" => Ok(MediaExtractionSettings::audio_constellation_v3()),
        "combinedv3" => Ok(MediaExtractionSettings::combined_v3()),
        _ => Err(format!(
            "unsupported profile '{profile}', expected audio-constellation-v3 or combined-v3"
        )),
    }
}

fn parse_match_class(label: &str) -> Option<MatchClassV3> {
    match normalized_label(label).as_str() {
        "samecutstrong" => Some(MatchClassV3::SameCutStrong),
        "samecutprobable" => Some(MatchClassV3::SameCutProbable),
        "samemediadifferentcut" => Some(MatchClassV3::SameMediaDifferentCut),
        "samevideodifferentaudio" => Some(MatchClassV3::SameVideoDifferentAudio),
        "sameaudiodifferentvideo" => Some(MatchClassV3::SameAudioDifferentVideo),
        "partialoverlap" => Some(MatchClassV3::PartialOverlap),
        "sharedintrooutroonly" => Some(MatchClassV3::SharedIntroOutroOnly),
        "reject" => Some(MatchClassV3::Reject),
        "unknown" => Some(MatchClassV3::Unknown),
        _ => None,
    }
}

fn parse_tier(label: &str) -> Option<MediaMatchTier> {
    match normalized_label(label).as_str() {
        "exact" => Some(MediaMatchTier::Exact),
        "strong" => Some(MediaMatchTier::Strong),
        "probable" => Some(MediaMatchTier::Probable),
        "weak" => Some(MediaMatchTier::Weak),
        "reject" => Some(MediaMatchTier::Reject),
        "unknown" => Some(MediaMatchTier::Unknown),
        _ => None,
    }
}

fn tier_score(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Unknown => 0,
        MediaMatchTier::Reject => 1,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Exact => 5,
    }
}

fn normalized_label(label: &str) -> String {
    label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn tool_paths() -> MediaMatchToolPaths {
    MediaMatchToolPaths {
        ffmpeg: env_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg"),
        ffprobe: env_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe"),
        fpcalc: PathBuf::from("fpcalc-not-used-for-v3"),
    }
}

fn env_tool_path(env_key: &str, default: &str) -> PathBuf {
    env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn default_profile() -> String {
    "audio-constellation-v3".to_owned()
}

fn unix_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        process::{Command, Stdio},
        time::Duration,
    };

    use super::*;
    use sorotte_media_match::{
        MEDIA_MATCH_ALGORITHM_VERSION, MediaFileIdentity, MediaFingerprintRecord,
        MediaMatchEvidence, MediaTimelineAlignment, MetadataMatchEvidence,
    };

    #[test]
    fn manifest_parsing_accepts_expected_shape() {
        let manifest = parse_manifest(
            r#"{
              "profile": "combined-v3",
              "cases": [{
                "name": "same-episode",
                "query": "query.mkv",
                "candidates": [{
                  "path": "candidate.mkv",
                  "expectedClass": "SameCutStrong",
                  "minimumTier": "Strong",
                  "expectedOffsetMs": 5000,
                  "maxOffsetErrorMs": 1000,
                  "autoplayEligible": true
                }]
              }]
            }"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.profile, "combined-v3");
        assert_eq!(manifest.cases[0].candidates[0].path, "candidate.mkv");
        assert_eq!(
            manifest.cases[0].candidates[0].expected_offset_ms,
            Some(5000)
        );
        assert_eq!(
            parse_match_class(
                manifest.cases[0].candidates[0]
                    .expected_class
                    .as_deref()
                    .unwrap()
            ),
            Some(MatchClassV3::SameCutStrong)
        );
    }

    #[test]
    fn expectation_evaluation_reports_failures() {
        let decision = MediaMatchDecision {
            tier: MediaMatchTier::Probable,
            evidence: MediaMatchEvidence {
                metadata: MetadataMatchEvidence::default(),
                v3_class: Some(MatchClassV3::PartialOverlap),
                ..MediaMatchEvidence::default()
            },
            explanation: "partial".to_owned(),
        };
        let expected = ManifestCandidate {
            path: "candidate.mkv".to_owned(),
            expected_class: Some("SameCutStrong".to_owned()),
            minimum_tier: Some("Strong".to_owned()),
            expected_offset_ms: None,
            max_offset_error_ms: None,
            autoplay_eligible: Some(true),
        };
        let settings = MediaMatchSettings {
            autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ..MediaMatchSettings::default()
        };

        let failures = evaluate_expectation(&decision, &expected, &settings);

        assert_eq!(failures.len(), 3, "{failures:?}");
    }

    #[test]
    fn expectation_offset_with_expected_value_uses_delta() {
        let decision = decision_with_offset_ms(5200);
        let settings = diagnostic_decision_settings();
        let expected = ManifestCandidate {
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: Some(5000),
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
        };

        let failures = evaluate_expectation(&decision, &expected, &settings);

        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn expectation_offset_with_expected_value_reports_delta_failure() {
        let decision = decision_with_offset_ms(8000);
        let settings = diagnostic_decision_settings();
        let expected = ManifestCandidate {
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: Some(5000),
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
        };

        let failures = evaluate_expectation(&decision, &expected, &settings);

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("expected offset 5000ms +/- 1000ms"));
        assert!(failures[0].contains("got 8000ms"));
    }

    #[test]
    fn expectation_offset_without_expected_value_keeps_absolute_behavior() {
        let decision = decision_with_offset_ms(800);
        let settings = diagnostic_decision_settings();
        let expected = ManifestCandidate {
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
        };

        let failures = evaluate_expectation(&decision, &expected, &settings);

        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn diagnostic_decision_settings_enable_fingerprinting() {
        let query = exactish_record("query.mkv");
        let mut candidate = exactish_record("candidate.mkv");
        candidate.identity.normalized_path = "candidate.mkv".to_owned();
        let decision = decide_media_match(&query, &candidate, &diagnostic_decision_settings());

        assert_ne!(decision.tier, MediaMatchTier::Unknown, "{decision:?}");
        assert!(!decision.explanation.contains("fingerprinting disabled"));
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
        let query = root.join("query.mkv");
        let candidate = root.join("candidate.mkv");
        generate_synthetic_media(&ffmpeg, &query);
        fs::copy(&query, &candidate).expect("candidate copy should succeed");
        let manifest = Manifest {
            profile: "combined-v3".to_owned(),
            cases: vec![ManifestCase {
                name: "copied-synthetic".to_owned(),
                query: query.to_string_lossy().to_string(),
                candidates: vec![ManifestCandidate {
                    path: candidate.to_string_lossy().to_string(),
                    expected_class: None,
                    minimum_tier: Some("Probable".to_owned()),
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                }],
            }],
        };

        let report = run_manifest(&manifest).expect("diagnostic harness should run");
        let report_json = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(report.summary.failed, 0, "{report_json}");
        assert!(report_json["cases"][0]["candidates"][0]["decision"]["tier"].is_string());
        assert!(report_json["cases"][0]["candidates"][0]["decision"]["class"].is_string());
        assert!(
            report_json["cases"][0]["query"]["diagnostics"]["audioBlobBytes"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        assert!(
            report_json["cases"][0]["candidates"][0]["decision"]["segmentCount"]
                .as_u64()
                .is_some()
        );
        let _ = fs::remove_dir_all(root);
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

    fn generate_synthetic_media(ffmpeg: &Path, path: &Path) {
        let status = Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x96:rate=1:duration=90",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=sin(2*PI*440*t)+0.5*sin(2*PI*880*t):s=44100:d=90",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(path)
            .status()
            .expect("ffmpeg should run");
        assert!(status.success(), "ffmpeg synthetic media generation failed");
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

    fn decision_with_offset_ms(offset_ms: i64) -> MediaMatchDecision {
        MediaMatchDecision {
            tier: MediaMatchTier::Strong,
            evidence: MediaMatchEvidence {
                metadata: MetadataMatchEvidence::default(),
                alignment: Some(MediaTimelineAlignment {
                    offset_seconds: offset_ms as f64 / 1000.0,
                    scale_ppm: 1_000_000,
                    drift_ratio: 0.0,
                    aligned_pairs: 12,
                    aligned_audio_anchors: 12,
                    aligned_video_anchors: 0,
                    aligned_span_seconds: 300.0,
                    second_best_offset_margin: 1.0,
                    first_query_second: 0.0,
                    last_query_second: 300.0,
                    first_candidate_second: offset_ms as f64 / 1000.0,
                    last_candidate_second: 300.0 + offset_ms as f64 / 1000.0,
                }),
                v3_class: Some(MatchClassV3::SameCutStrong),
                ..MediaMatchEvidence::default()
            },
            explanation: "strong".to_owned(),
        }
    }

    fn exactish_record(path: &str) -> MediaFingerprintRecord {
        MediaFingerprintRecord {
            identity: MediaFileIdentity {
                normalized_path: path.to_owned(),
                modified_unix_millis: 1,
                size_bytes: 123,
            },
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: MediaExtractionSettings::audio_constellation_v3(),
            duration_seconds: Some(60.0),
            container_fingerprint: "same-container".to_owned(),
            audio: None,
            video: None,
            audio_anchors: Vec::new(),
            video_anchors: Vec::new(),
            audio_error: None,
            video_error: None,
        }
    }
}
