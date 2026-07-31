use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use sorotte_media_match::{
    MEDIA_MATCH_ALGORITHM_VERSION, MediaMatchToolPaths, MediaMatchV3DiagnosticIndexMode,
    MediaMatchV3DiagnosticRunOptions, media_match_v3_diagnostic_manifest_from_json,
    media_match_v3_diagnostic_manifest_report_json, run_media_match_v3_diagnostic_manifest,
};

struct GeneratedMediaRoot {
    path: PathBuf,
}

impl GeneratedMediaRoot {
    fn create() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sorotte-media-match-generated-v3-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("generated-media test root should be created");
        Self { path }
    }
}

impl Drop for GeneratedMediaRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!(
                "failed to remove generated-media test root {}: {error}",
                self.path.display()
            );
        }
    }
}

fn required_tool(environment_key: &str, default_name: &str) -> PathBuf {
    let path = std::env::var_os(environment_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_name));
    let status = Command::new(&path)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "required generated-media tool {} ({}) could not start: {error}",
                environment_key,
                path.display()
            )
        });
    assert!(
        status.success(),
        "required generated-media tool {} ({}) returned {status}",
        environment_key,
        path.display()
    );
    path
}

fn generate_synthetic_media(ffmpeg: &Path, path: &Path) {
    let output = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=1:duration=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=30",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-shortest",
            "-c:v",
            "ffv1",
            "-level",
            "3",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .expect("ffmpeg should start for generated-media fixture creation");
    assert!(
        output.status.success(),
        "ffmpeg fixture generation failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "required Linux CI integration test; requires ffmpeg and ffprobe binaries"]
fn v3_manifest_harness_runs_small_synthetic_case() {
    let ffmpeg = required_tool("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg");
    let ffprobe = required_tool("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe");
    let root = GeneratedMediaRoot::create();
    let media_dir = root.path.join("media");
    std::fs::create_dir(&media_dir).expect("media directory should be created");
    let query = media_dir.join("query.mkv");
    let candidate = media_dir.join("candidate.mkv");
    generate_synthetic_media(&ffmpeg, &query);
    std::fs::copy(&query, &candidate).expect("candidate media should be copied");

    let manifest = serde_json::json!({
        "profile": "audio-constellation-v3",
        "baseDir": "media",
        "cases": [{
            "name": "copied-synthetic-media",
            "query": "query.mkv",
            "candidates": [{
                "path": "candidate.mkv",
                "minimumTier": "Probable",
                "mustBeRetrieved": true
            }]
        }]
    });
    let manifest = media_match_v3_diagnostic_manifest_from_json(&manifest.to_string())
        .expect("manifest should parse");
    let report = run_media_match_v3_diagnostic_manifest(
        &manifest,
        &MediaMatchV3DiagnosticRunOptions {
            manifest_dir: root.path.clone(),
            cache_root: root.path.join("diagnostic-cache"),
            cache_retained: true,
            refresh_cache: false,
            index_mode: MediaMatchV3DiagnosticIndexMode::SampledFast,
            retrieval_benchmark_only: false,
            tools: MediaMatchToolPaths { ffmpeg, ffprobe },
            generated_at_unix_millis: Some(123),
        },
    )
    .expect("diagnostic manifest should run");
    let case = report.cases.first().expect("one diagnostic case");
    let candidate_fingerprint = &case
        .candidates
        .first()
        .expect("one diagnostic candidate")
        .fingerprint;
    for (label, fingerprint) in [("query", &case.query), ("candidate", candidate_fingerprint)] {
        let diagnostics = &fingerprint.diagnostics;
        assert_eq!(
            diagnostics.duration_ms,
            Some(30_000),
            "{label} ffprobe duration should match the generated fixture"
        );
        assert!(
            diagnostics.audio_verify_count > 0,
            "{label} should contain verification audio landmarks"
        );
        assert!(
            diagnostics.audio_index_count > 0,
            "{label} should contain index audio landmarks"
        );
        assert!(
            diagnostics
                .ffmpeg_output_pcm_bytes
                .is_some_and(|bytes| bytes > 0),
            "{label} should report decoded ffmpeg PCM output"
        );
    }
    assert!(
        case.retrieval.stats.query_buckets_total > 0,
        "query landmarks should populate at least one retrieval bucket"
    );
    let report_json =
        media_match_v3_diagnostic_manifest_report_json(&report).expect("report should serialize");
    let report: serde_json::Value =
        serde_json::from_str(&report_json).expect("report JSON should parse");
    let candidate = &report["cases"][0]["candidates"][0];

    assert_eq!(report["algorithmVersion"], MEDIA_MATCH_ALGORITHM_VERSION);
    assert_eq!(report["generatedAtUnixMillis"], 123);
    assert_eq!(candidate["expectationPassed"], true);
    assert_eq!(candidate["retrieved"], true);
    assert!(candidate["decision"].get("class").is_some());
}
