use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{
    InstrumentedMediaFingerprint, MEDIA_MATCH_ANCHOR_VERSION, MatchClassV3,
    MediaExtractionSettings, MediaMatchAutoplayPolicy, MediaMatchDecision, MediaMatchSettings,
    MediaMatchTier, MediaMatchToolPaths, MediaMatchV3DiagnosticSummary, MediaMatchV3RetrievalStats,
    V3Tuning, current_v3_tuning, decide_media_match, fingerprint_media_file_with_report,
    media_extraction_settings_hash, media_match_v3_anchor_candidate_paths_with_stats,
    normalize_media_path, open_media_match_v3_index, save_media_match_v3_record,
    summarize_decision_v3_diagnostics, summarize_instrumented_record_v3_diagnostics,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifest {
    #[serde(default = "default_diagnostic_profile")]
    pub profile: String,
    #[serde(default)]
    pub base_dir: Option<String>,
    pub cases: Vec<MediaMatchV3DiagnosticManifestCase>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticManifestCase {
    pub name: String,
    pub query: String,
    pub candidates: Vec<MediaMatchV3DiagnosticExpectation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticExpectation {
    pub path: String,
    pub expected_class: Option<String>,
    pub minimum_tier: Option<String>,
    pub expected_offset_ms: Option<i64>,
    pub max_offset_error_ms: Option<i64>,
    pub autoplay_eligible: Option<bool>,
    #[serde(default)]
    pub must_be_retrieved: bool,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3DiagnosticRunOptions {
    pub manifest_dir: PathBuf,
    pub cache_root: PathBuf,
    pub tools: MediaMatchToolPaths,
    pub generated_at_unix_millis: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifest {
    pub profile: String,
    pub cases: Vec<MediaMatchV3ResolvedManifestCase>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestCase {
    pub name: String,
    pub query: PathBuf,
    pub candidates: Vec<MediaMatchV3ResolvedManifestCandidate>,
}

#[derive(Debug, Clone)]
pub struct MediaMatchV3ResolvedManifestCandidate {
    pub path: PathBuf,
    pub expectation: MediaMatchV3DiagnosticExpectation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticReport {
    pub algorithm_version: u32,
    pub profile: String,
    pub settings_hash: String,
    pub tuning: V3Tuning,
    pub generated_at_unix_millis: u64,
    pub cases: Vec<MediaMatchV3DiagnosticCaseReport>,
    pub summary: MediaMatchV3DiagnosticSummaryReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCaseReport {
    pub name: String,
    pub query: MediaMatchV3DiagnosticFingerprintReport,
    pub retrieval: MediaMatchV3DiagnosticRetrievalReport,
    pub candidates: Vec<MediaMatchV3DiagnosticCandidateReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticFingerprintReport {
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticCandidateReport {
    pub path: String,
    pub diagnostics: MediaMatchV3DiagnosticSummary,
    pub source: String,
    pub retrieved: bool,
    pub retrieval_rank: Option<usize>,
    pub decision: MediaMatchV3DiagnosticDecisionReport,
    pub expectation: Option<MediaMatchV3DiagnosticExpectation>,
    pub passed: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticDecisionReport {
    pub tier: String,
    pub class: Option<String>,
    pub explanation: String,
    pub offset_seconds: Option<f64>,
    pub scale_ppm: Option<i32>,
    pub segment_count: usize,
    pub total_aligned_span_ms: u32,
    pub largest_gap_ms: u32,
    pub edge_only: bool,
    pub audio_video_conflict: bool,
    pub piecewise_pair_count: Option<usize>,
    pub piecewise_hypothesis_count: Option<usize>,
    pub piecewise_fit_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticRetrievalReport {
    pub query_buckets_total: i64,
    pub query_buckets_skipped_common: i64,
    pub raw_hit_rows_processed: i64,
    pub candidates_scored: i64,
    pub retrieval_elapsed_ms: u128,
    pub retrieved_candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummaryReport {
    pub case_count: usize,
    pub pair_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_extraction_millis: u128,
    pub total_audio_blob_bytes: usize,
    pub total_video_blob_bytes: usize,
    pub total_raw_hit_rows_processed: i64,
    pub total_retrieval_millis: u128,
}

#[derive(Debug, Clone)]
struct CachedFingerprint {
    fingerprint: InstrumentedMediaFingerprint,
    source: &'static str,
}

pub fn media_match_v3_diagnostic_manifest_from_json(
    manifest_json: &str,
) -> Result<MediaMatchV3DiagnosticManifest, String> {
    serde_json::from_str(manifest_json)
        .map_err(|error| format!("failed parsing media-match V3 diagnostic manifest: {error}"))
}

pub fn media_match_v3_diagnostic_manifest_report_json(
    manifest_json: &str,
    options: MediaMatchV3DiagnosticRunOptions,
) -> Result<String, String> {
    let manifest = media_match_v3_diagnostic_manifest_from_json(manifest_json)?;
    let report = run_media_match_v3_diagnostic_manifest(&manifest, options)?;
    serde_json::to_string_pretty(&report)
        .map_err(|error| format!("failed serializing media-match V3 diagnostic report: {error}"))
}

pub fn run_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    options: MediaMatchV3DiagnosticRunOptions,
) -> Result<MediaMatchV3DiagnosticReport, String> {
    let settings = diagnostic_settings_for_profile(&manifest.profile)?;
    let settings_hash = media_extraction_settings_hash(&settings);
    let resolved = resolve_media_match_v3_diagnostic_manifest(manifest, &options.manifest_dir)?;
    let connection = open_media_match_v3_index(&options.cache_root)?;
    let autoplay_settings = diagnostic_decision_settings();
    let mut cache = BTreeMap::<(String, [u8; 32]), InstrumentedMediaFingerprint>::new();
    let mut cases = Vec::new();
    let mut summary = MediaMatchV3DiagnosticSummaryReport {
        case_count: resolved.cases.len(),
        ..MediaMatchV3DiagnosticSummaryReport::default()
    };

    for case in &resolved.cases {
        let query = fingerprint_cached(&mut cache, &case.query, &options.tools, &settings)?;
        save_media_match_v3_record(&connection, &query.fingerprint.record, None)?;

        let mut candidate_records = Vec::new();
        for candidate in &case.candidates {
            let fingerprint =
                fingerprint_cached(&mut cache, &candidate.path, &options.tools, &settings)?;
            save_media_match_v3_record(&connection, &fingerprint.fingerprint.record, None)?;
            candidate_records.push((candidate, fingerprint));
        }

        let (retrieved_candidates, retrieval_stats) =
            media_match_v3_anchor_candidate_paths_with_stats(
                &connection,
                &query.fingerprint.record.identity.normalized_path,
                &settings,
            )?;
        let retrieval_report = MediaMatchV3DiagnosticRetrievalReport::from_stats(
            retrieval_stats,
            retrieved_candidates,
        );
        summary.total_raw_hit_rows_processed += retrieval_report.raw_hit_rows_processed;
        summary.total_retrieval_millis += retrieval_report.retrieval_elapsed_ms;

        let query_report = MediaMatchV3DiagnosticFingerprintReport {
            path: query.fingerprint.record.identity.normalized_path.clone(),
            diagnostics: summarize_instrumented_record_v3_diagnostics(&query.fingerprint),
            source: query.source.to_owned(),
        };
        let mut reports = Vec::new();
        for (candidate, fingerprint) in candidate_records {
            let decision = decide_media_match(
                &query.fingerprint.record,
                &fingerprint.fingerprint.record,
                &autoplay_settings,
            );
            let normalized_candidate = &fingerprint.fingerprint.record.identity.normalized_path;
            let retrieval_rank = retrieval_report
                .retrieved_candidates
                .iter()
                .position(|path| path == normalized_candidate)
                .map(|index| index + 1);
            let retrieved = retrieval_rank.is_some();
            let failures = evaluate_diagnostic_expectation(
                &decision,
                &candidate.expectation,
                &autoplay_settings,
                retrieved,
            );
            let passed = failures.is_empty();
            summary.pair_count += 1;
            if passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            reports.push(MediaMatchV3DiagnosticCandidateReport {
                path: normalized_candidate.clone(),
                diagnostics: summarize_instrumented_record_v3_diagnostics(&fingerprint.fingerprint),
                source: fingerprint.source.to_owned(),
                retrieved,
                retrieval_rank,
                decision: MediaMatchV3DiagnosticDecisionReport::from_decision(&decision),
                expectation: Some(candidate.expectation.clone()),
                passed,
                failure_reason: (!failures.is_empty()).then(|| failures.join("; ")),
            });
        }
        cases.push(MediaMatchV3DiagnosticCaseReport {
            name: case.name.clone(),
            query: query_report,
            retrieval: retrieval_report,
            candidates: reports,
        });
    }

    for fingerprint in cache.values() {
        let diagnostics = summarize_instrumented_record_v3_diagnostics(fingerprint);
        summary.total_extraction_millis += fingerprint.report.timings.total_millis;
        summary.total_audio_blob_bytes += diagnostics.audio_blob_bytes;
        summary.total_video_blob_bytes += diagnostics.video_blob_bytes;
    }

    Ok(MediaMatchV3DiagnosticReport {
        algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: settings.profile.label().to_owned(),
        settings_hash: bytes_to_lower_hex(&settings_hash),
        tuning: current_v3_tuning(),
        generated_at_unix_millis: options
            .generated_at_unix_millis
            .unwrap_or_else(current_unix_millis),
        cases,
        summary,
    })
}

pub fn resolve_media_match_v3_diagnostic_manifest(
    manifest: &MediaMatchV3DiagnosticManifest,
    manifest_dir: &Path,
) -> Result<MediaMatchV3ResolvedManifest, String> {
    let base = manifest
        .base_dir
        .as_deref()
        .map(|base_dir| resolve_manifest_path(manifest_dir, manifest_dir, base_dir))
        .unwrap_or_else(|| manifest_dir.to_path_buf());
    let cases = manifest
        .cases
        .iter()
        .map(|case| {
            Ok(MediaMatchV3ResolvedManifestCase {
                name: case.name.clone(),
                query: resolve_manifest_path(manifest_dir, &base, &case.query),
                candidates: case
                    .candidates
                    .iter()
                    .map(|candidate| MediaMatchV3ResolvedManifestCandidate {
                        path: resolve_manifest_path(manifest_dir, &base, &candidate.path),
                        expectation: candidate.clone(),
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(MediaMatchV3ResolvedManifest {
        profile: manifest.profile.clone(),
        cases,
    })
}

impl MediaMatchV3DiagnosticRetrievalReport {
    fn from_stats(stats: MediaMatchV3RetrievalStats, retrieved_candidates: Vec<String>) -> Self {
        Self {
            query_buckets_total: stats.query_buckets_total,
            query_buckets_skipped_common: stats.query_buckets_skipped_common,
            raw_hit_rows_processed: stats.raw_hit_rows_processed,
            candidates_scored: stats.candidates_scored,
            retrieval_elapsed_ms: stats.retrieval_elapsed_ms,
            retrieved_candidates,
        }
    }
}

impl MediaMatchV3DiagnosticDecisionReport {
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

fn fingerprint_cached(
    cache: &mut BTreeMap<(String, [u8; 32]), InstrumentedMediaFingerprint>,
    path: &Path,
    tools: &MediaMatchToolPaths,
    settings: &MediaExtractionSettings,
) -> Result<CachedFingerprint, String> {
    let normalized_path = normalize_media_path(path);
    let cache_key = (normalized_path, media_extraction_settings_hash(settings));
    if let Some(fingerprint) = cache.get(&cache_key) {
        return Ok(CachedFingerprint {
            fingerprint: fingerprint.clone(),
            source: "cache",
        });
    }
    let fingerprint = fingerprint_media_file_with_report(path, tools, settings, None)
        .map_err(|error| format!("failed fingerprinting '{}': {error}", path.display()))?;
    cache.insert(cache_key, fingerprint.clone());
    Ok(CachedFingerprint {
        fingerprint,
        source: "fresh",
    })
}

fn evaluate_diagnostic_expectation(
    decision: &MediaMatchDecision,
    expected: &MediaMatchV3DiagnosticExpectation,
    autoplay_settings: &MediaMatchSettings,
    retrieved: bool,
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
    if expected.must_be_retrieved && !retrieved {
        failures.push("expected candidate to be retrieved, but it was absent".to_owned());
    }
    failures
}

fn resolve_manifest_path(_manifest_dir: &Path, base_dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn diagnostic_settings_for_profile(profile: &str) -> Result<MediaExtractionSettings, String> {
    match normalized_label(profile).as_str() {
        "audioconstellationv3" => Ok(MediaExtractionSettings::audio_constellation_v3()),
        "combinedv3" => Ok(MediaExtractionSettings::combined_v3()),
        _ => Err(format!(
            "unsupported profile '{profile}', expected audio-constellation-v3 or combined-v3"
        )),
    }
}

fn diagnostic_decision_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
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

fn current_unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn default_diagnostic_profile() -> String {
    "audio-constellation-v3".to_owned()
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;
    use crate::{MediaMatchEvidence, MediaTimelineAlignment, MetadataMatchEvidence};

    #[test]
    fn manifest_parsing_accepts_canonical_shape() {
        let manifest = media_match_v3_diagnostic_manifest_from_json(
            r#"{
              "profile": "combined-v3",
              "baseDir": "media",
              "cases": [{
                "name": "same-episode",
                "query": "query.mkv",
                "candidates": [{
                  "path": "candidate.mkv",
                  "expectedClass": "SameCutStrong",
                  "minimumTier": "Strong",
                  "expectedOffsetMs": 5000,
                  "maxOffsetErrorMs": 1000,
                  "autoplayEligible": true,
                  "mustBeRetrieved": true
                }]
              }]
            }"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.profile, "combined-v3");
        assert_eq!(manifest.base_dir.as_deref(), Some("media"));
        assert_eq!(manifest.cases[0].candidates[0].path, "candidate.mkv");
        assert_eq!(
            manifest.cases[0].candidates[0].expected_offset_ms,
            Some(5000)
        );
        assert!(manifest.cases[0].candidates[0].must_be_retrieved);
        serde_json::to_string(&manifest).expect("canonical manifest should serialize");
    }

    #[test]
    fn manifest_paths_resolve_relative_to_manifest_dir() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: None,
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "relative".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![MediaMatchV3DiagnosticExpectation {
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                }],
            }],
        };
        let manifest_dir = PathBuf::from("C:/manifest-root");

        let resolved =
            resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir).expect("resolve");

        assert_eq!(resolved.cases[0].query, manifest_dir.join("query.mkv"));
        assert_eq!(
            resolved.cases[0].candidates[0].path,
            manifest_dir.join("candidate.mkv")
        );
    }

    #[test]
    fn manifest_base_dir_resolves_relative_to_manifest_dir() {
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: Some("media".to_owned()),
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "base".to_owned(),
                query: "query.mkv".to_owned(),
                candidates: vec![MediaMatchV3DiagnosticExpectation {
                    path: "candidate.mkv".to_owned(),
                    expected_class: None,
                    minimum_tier: None,
                    expected_offset_ms: None,
                    max_offset_error_ms: None,
                    autoplay_eligible: None,
                    must_be_retrieved: false,
                }],
            }],
        };
        let manifest_dir = PathBuf::from("C:/manifest-root");

        let resolved =
            resolve_media_match_v3_diagnostic_manifest(&manifest, &manifest_dir).expect("resolve");

        assert_eq!(
            resolved.cases[0].query,
            manifest_dir.join("media/query.mkv")
        );
        assert_eq!(
            resolved.cases[0].candidates[0].path,
            manifest_dir.join("media/candidate.mkv")
        );
    }

    #[test]
    fn manifest_absolute_paths_are_unchanged() {
        let absolute = env::current_dir()
            .expect("current dir")
            .join("absolute-query.mkv");
        let manifest = MediaMatchV3DiagnosticManifest {
            profile: "audio-constellation-v3".to_owned(),
            base_dir: Some("media".to_owned()),
            cases: vec![MediaMatchV3DiagnosticManifestCase {
                name: "absolute".to_owned(),
                query: absolute.to_string_lossy().to_string(),
                candidates: Vec::new(),
            }],
        };

        let resolved = resolve_media_match_v3_diagnostic_manifest(&manifest, Path::new("unused"))
            .expect("resolve");

        assert_eq!(resolved.cases[0].query, absolute);
    }

    #[test]
    fn expectation_evaluation_covers_offsets_autoplay_and_retrieval() {
        let settings = diagnostic_decision_settings();
        let expected = MediaMatchV3DiagnosticExpectation {
            path: "candidate.mkv".to_owned(),
            expected_class: Some("SameCutStrong".to_owned()),
            minimum_tier: Some("Strong".to_owned()),
            expected_offset_ms: Some(5000),
            max_offset_error_ms: Some(1000),
            autoplay_eligible: Some(true),
            must_be_retrieved: true,
        };

        let pass = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(5200),
            &expected,
            &settings,
            true,
        );
        let fail = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(8000),
            &expected,
            &settings,
            false,
        );

        assert!(pass.is_empty(), "{pass:?}");
        assert!(
            fail.iter()
                .any(|failure| failure.contains("expected offset 5000ms")),
            "{fail:?}"
        );
        assert!(
            fail.iter()
                .any(|failure| failure.contains("expected candidate to be retrieved")),
            "{fail:?}"
        );
    }

    #[test]
    fn expectation_offset_without_expected_value_keeps_absolute_behavior() {
        let settings = diagnostic_decision_settings();
        let expected = MediaMatchV3DiagnosticExpectation {
            path: "candidate.mkv".to_owned(),
            expected_class: None,
            minimum_tier: None,
            expected_offset_ms: None,
            max_offset_error_ms: Some(1000),
            autoplay_eligible: None,
            must_be_retrieved: false,
        };

        let failures = evaluate_diagnostic_expectation(
            &decision_with_offset_ms(800),
            &expected,
            &settings,
            false,
        );

        assert!(failures.is_empty(), "{failures:?}");
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
}
