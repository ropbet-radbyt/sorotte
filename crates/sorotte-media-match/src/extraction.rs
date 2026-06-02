use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use crate::{
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION,
    anchors::media_fingerprint_wire_summary_from_record,
    audio_v3::{
        AudioConstellationV3Config, AudioConstellationV3PcmStream, AudioLandmarkV3,
        bounded_time_distributed_audio_landmarks_v3_for_duration,
    },
    identity::{container_fingerprint_from_metadata, normalize_media_path},
    tuning::{
        FFMPEG_AUDIO_V3_TIMEOUT, FFPROBE_TIMEOUT, MEDIA_TOOL_POLL_INTERVAL,
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT, V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
        V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT, V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
    },
    types::{MediaFileIdentity, MediaFingerprintRecord},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatchToolPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaToolInvocationCounts {
    pub ffmpeg: u32,
    pub ffprobe: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaSourcePathInfo {
    pub root: String,
    pub kind: String,
    pub volume_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaToolProcessIoMetrics {
    pub read_bytes: Option<u64>,
    pub read_ops: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaToolStreamingOutput {
    pub stdout_bytes: u64,
    pub process_io: MediaToolProcessIoMetrics,
    pub exit_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaExtractionTimings {
    pub ffprobe_millis: u128,
    pub audio_millis: u128,
    pub total_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaAudioStreamMetrics {
    pub source_path_root: Option<String>,
    pub source_path_kind: Option<String>,
    pub source_volume_id: Option<String>,
    pub ffmpeg_command_kind: Option<String>,
    pub ffmpeg_selected_stream: Option<String>,
    pub ffmpeg_disabled_non_audio_streams: bool,
    pub ffmpeg_disabled_subtitles: bool,
    pub ffmpeg_disabled_data: bool,
    pub streamed_bytes: usize,
    pub streamed_samples: usize,
    pub peak_frames: usize,
    pub raw_landmarks_emitted: usize,
    pub raw_landmarks_before_bounding: usize,
    pub raw_landmarks_kept_before_final: usize,
    pub final_landmarks: usize,
    pub max_buffer_samples: usize,
    pub max_raw_landmarks_seen: usize,
    pub max_raw_landmarks_after_compaction: usize,
    pub raw_landmark_compactions: usize,
    pub ffmpeg_process_wall_millis: u128,
    pub ffmpeg_input_read_bytes: Option<u64>,
    pub ffmpeg_input_read_ops: Option<u64>,
    pub ffmpeg_output_pcm_bytes: u64,
    pub ffmpeg_invocation_count: usize,
    pub sampled_window_decode_millis: u128,
    pub ffmpeg_exit_millis: u128,
    pub pcm_decode_drain_millis: u128,
    pub ffmpeg_decode_stream_millis: u128,
    pub analyzer_millis: u128,
    pub peak_selection_millis: u128,
    pub pairing_millis: u128,
    pub compaction_millis: u128,
    pub reservoir_millis: u128,
    pub final_selection_millis: u128,
    pub candidate_pairs_considered: usize,
    pub candidate_pairs_skipped_by_anchor_gate: usize,
    pub candidate_pairs_skipped_by_target_gate: usize,
    pub candidate_pairs_skipped_by_saturation: usize,
    pub candidate_pairs_emitted: usize,
    pub anchor_peaks_considered: usize,
    pub anchor_peaks_selected: usize,
    pub anchor_peaks_skipped_by_gate: usize,
    pub target_peaks_considered: usize,
    pub target_peaks_selected: usize,
    pub landmarks_accepted_into_reservoir: usize,
    pub landmarks_rejected_by_reservoir: usize,
    pub sampled_audio_seconds_decoded: u32,
    pub sampled_audio_windows_decoded: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaFingerprintExtractionReport {
    pub invocations: MediaToolInvocationCounts,
    pub timings: MediaExtractionTimings,
    pub audio_stream: MediaAudioStreamMetrics,
    pub audio_error: Option<String>,
    pub serialized_debug_record_bytes: usize,
    pub audio_summary_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentedMediaFingerprint {
    pub record: MediaFingerprintRecord,
    pub report: MediaFingerprintExtractionReport,
}

#[derive(Debug, Clone, Default)]
pub struct MediaFingerprintExtractionOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaFingerprintError {
    FileMetadata {
        path: String,
        error: String,
    },
    ToolFailed {
        tool: &'static str,
        status: Option<i32>,
        stderr: String,
    },
    TimedOut {
        tool: &'static str,
        timeout_seconds: u64,
    },
    Cancelled {
        tool: &'static str,
    },
    InvalidToolOutput {
        tool: &'static str,
        reason: String,
    },
}

impl fmt::Display for MediaFingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileMetadata { path, error } => {
                write!(
                    formatter,
                    "failed reading media metadata for '{path}': {error}"
                )
            }
            Self::ToolFailed {
                tool,
                status,
                stderr,
            } => {
                let status = status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "terminated".to_owned());
                write!(formatter, "{tool} failed with status {status}: {stderr}")
            }
            Self::TimedOut {
                tool,
                timeout_seconds,
            } => {
                write!(
                    formatter,
                    "{tool} timed out after {timeout_seconds} seconds during media fingerprinting"
                )
            }
            Self::Cancelled { tool } => {
                write!(formatter, "{tool} was canceled during media fingerprinting")
            }
            Self::InvalidToolOutput { tool, reason } => {
                write!(formatter, "{tool} output could not be parsed: {reason}")
            }
        }
    }
}

impl std::error::Error for MediaFingerprintError {}

pub fn fingerprint_media_file(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &crate::MediaExtractionSettings,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, None)
        .map(|fingerprint| fingerprint.record)
}

pub fn fingerprint_media_file_cancellable(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &crate::MediaExtractionSettings,
    cancel_flag: &AtomicBool,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, Some(cancel_flag))
        .map(|fingerprint| fingerprint.record)
}

pub fn fingerprint_media_file_cancellable_with_report(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &crate::MediaExtractionSettings,
    cancel_flag: &AtomicBool,
) -> Result<InstrumentedMediaFingerprint, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, Some(cancel_flag))
}

pub fn fingerprint_media_file_with_report(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &crate::MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<InstrumentedMediaFingerprint, MediaFingerprintError> {
    fingerprint_media_file_with_report_and_options(
        path,
        tools,
        extraction_settings,
        cancel_flag,
        &MediaFingerprintExtractionOptions,
    )
}

pub fn fingerprint_media_file_with_report_and_options(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &crate::MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
    _options: &MediaFingerprintExtractionOptions,
) -> Result<InstrumentedMediaFingerprint, MediaFingerprintError> {
    let total_started_at = Instant::now();
    let path = path.as_ref();
    let metadata = fs::metadata(path).map_err(|error| MediaFingerprintError::FileMetadata {
        path: path.display().to_string(),
        error: error.to_string(),
    })?;
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    let size_bytes = metadata.len();
    let normalized_path = normalize_media_path(path);
    let mut report = MediaFingerprintExtractionReport::default();

    let started_at = Instant::now();
    let duration_seconds = probe_media_duration_seconds(&tools.ffprobe, path)?;
    report.invocations.ffprobe = 1;
    report.timings.ffprobe_millis = started_at.elapsed().as_millis();

    let container_fingerprint = container_fingerprint_from_metadata(
        &normalized_path,
        modified_unix_millis,
        size_bytes,
        duration_seconds,
    );

    let started_at = Instant::now();
    let audio_result =
        extract_fixed_sampled_fast_audio(&tools.ffmpeg, path, duration_seconds, cancel_flag);
    report.invocations.ffmpeg = V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT as u32;
    report.timings.audio_millis = started_at.elapsed().as_millis();
    let (audio_anchors, metrics) = match audio_result {
        Ok((landmarks, metrics)) => (
            landmarks
                .into_iter()
                .map(|landmark| AudioAnchor {
                    bucket: landmark.hash,
                    t_ms: landmark.t_ms,
                    weight: u16::from(landmark.weight.max(1)),
                })
                .collect(),
            metrics,
        ),
        Err(MediaFingerprintError::Cancelled { tool }) => {
            return Err(MediaFingerprintError::Cancelled { tool });
        }
        Err(error) => {
            report.audio_error = Some(error.to_string());
            (Vec::new(), MediaAudioStreamMetrics::default())
        }
    };
    report.audio_stream = metrics;

    let audio_error = report.audio_error.clone();
    let record = MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path,
            modified_unix_millis,
            size_bytes,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: extraction_settings.clone(),
        duration_seconds,
        container_fingerprint,
        audio_anchors,
        audio_error,
    };

    let summary = media_fingerprint_wire_summary_from_record(&record);
    report.serialized_debug_record_bytes = serde_json::to_vec(&record)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    report.audio_summary_bytes = summary.audio_summary.as_ref().map(Vec::len).unwrap_or(0);
    report.timings.total_millis = total_started_at.elapsed().as_millis();
    Ok(InstrumentedMediaFingerprint { record, report })
}

pub fn probe_media_duration_seconds(
    ffprobe: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
) -> Result<Option<f64>, MediaFingerprintError> {
    let output = run_tool_output(
        "ffprobe",
        ffprobe.as_ref(),
        [
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration".into(),
            "-of".into(),
            "default=noprint_wrappers=1:nokey=1".into(),
            media_path.as_ref().as_os_str().to_os_string(),
        ],
        None,
        FFPROBE_TIMEOUT,
    )?;
    ensure_tool_success("ffprobe", &output)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let value = text
        .lines()
        .find_map(|line| line.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0);
    Ok(value)
}

fn extract_fixed_sampled_fast_audio(
    ffmpeg: &Path,
    media_path: &Path,
    duration_seconds: Option<f64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let windows = sampled_audio_windows_v3(duration_seconds);
    if windows.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "media is too short for fixed sampled-fast windows".to_owned(),
        });
    }

    let mut all_landmarks = Vec::new();
    let mut metrics = MediaAudioStreamMetrics::default();
    let source_info = media_source_path_info(media_path);
    metrics.source_path_root = Some(source_info.root);
    metrics.source_path_kind = Some(source_info.kind);
    metrics.source_volume_id = source_info.volume_id;
    metrics.ffmpeg_command_kind = Some("audio-only-pcm".to_owned());
    metrics.ffmpeg_selected_stream = Some("0:a:0".to_owned());
    metrics.ffmpeg_disabled_non_audio_streams = true;
    metrics.ffmpeg_disabled_subtitles = true;
    metrics.ffmpeg_disabled_data = true;

    let mut process_wall_millis = 0u128;
    for (start_seconds, window_seconds) in windows {
        let (window_pcm, streaming_output, window_wall) = decode_sampled_window_pcm_bytes(
            ffmpeg,
            media_path,
            start_seconds,
            window_seconds,
            V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
            cancel_flag,
        )?;
        process_wall_millis = process_wall_millis.saturating_add(window_wall);
        metrics.ffmpeg_invocation_count += 1;
        metrics.ffmpeg_output_pcm_bytes = metrics
            .ffmpeg_output_pcm_bytes
            .saturating_add(streaming_output.stdout_bytes);
        metrics.ffmpeg_input_read_bytes = sum_optional_u64(
            metrics.ffmpeg_input_read_bytes,
            streaming_output.process_io.read_bytes,
        );
        metrics.ffmpeg_input_read_ops = sum_optional_u64(
            metrics.ffmpeg_input_read_ops,
            streaming_output.process_io.read_ops,
        );
        metrics.sampled_window_decode_millis = metrics
            .sampled_window_decode_millis
            .saturating_add(window_wall);
        metrics.ffmpeg_exit_millis = metrics
            .ffmpeg_exit_millis
            .saturating_add(streaming_output.exit_millis);

        let (mut landmarks, window_metrics) =
            analyze_sampled_window_pcm_bytes(&window_pcm, window_seconds)?;
        let start_ms = (start_seconds * 1000.0)
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32;
        for landmark in &mut landmarks {
            landmark.t_ms = landmark.t_ms.saturating_add(start_ms);
        }
        all_landmarks.extend(landmarks);
        merge_audio_stream_metrics(&mut metrics, &window_metrics);
        metrics.sampled_audio_seconds_decoded = metrics
            .sampled_audio_seconds_decoded
            .saturating_add(window_seconds);
        metrics.sampled_audio_windows_decoded += 1;
    }

    let selection_started_at = Instant::now();
    let raw_before_bounding = all_landmarks.len();
    let bounded = bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut all_landmarks,
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
        duration_seconds,
    );
    metrics.final_selection_millis = selection_started_at.elapsed().as_millis();
    metrics.final_landmarks = bounded.len();
    metrics.raw_landmarks_before_bounding = raw_before_bounding;
    metrics.raw_landmarks_kept_before_final = raw_before_bounding;
    metrics.ffmpeg_process_wall_millis = process_wall_millis;
    metrics.pcm_decode_drain_millis = process_wall_millis;
    metrics.ffmpeg_decode_stream_millis = process_wall_millis;
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((bounded, metrics))
}

fn sampled_audio_windows_v3(duration_seconds: Option<f64>) -> Vec<(f64, u32)> {
    let duration = duration_seconds.unwrap_or(0.0);
    let window = V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS;
    if !duration.is_finite() || duration <= f64::from(window) {
        return vec![(0.0, window)];
    }
    let count = V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT;
    let edge_skip: f64 = if duration >= 600.0 { 180.0 } else { 30.0 };
    let body_start = edge_skip.min((duration - f64::from(window)).max(0.0));
    let body_end = (duration - edge_skip - f64::from(window)).max(body_start);
    if count <= 1 || body_end <= body_start {
        return vec![(body_start, window)];
    }
    (0..count)
        .map(|index| {
            let fraction = index as f64 / (count - 1) as f64;
            (body_start + (body_end - body_start) * fraction, window)
        })
        .map(|(start, seconds)| (start.max(0.0), seconds))
        .collect()
}

fn decode_sampled_window_pcm_bytes(
    ffmpeg: &Path,
    media_path: &Path,
    start_seconds: f64,
    window_seconds: u32,
    sample_rate: u32,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<u8>, MediaToolStreamingOutput, u128), MediaFingerprintError> {
    let started_at = Instant::now();
    let pcm = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pcm_writer = Arc::clone(&pcm);
    let args = vec![
        "-v".into(),
        "error".into(),
        "-nostdin".into(),
        "-threads".into(),
        "1".into(),
        "-ss".into(),
        format!("{start_seconds:.3}").into(),
        "-t".into(),
        window_seconds.to_string().into(),
        "-i".into(),
        media_path.as_os_str().to_os_string(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-sn".into(),
        "-dn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        sample_rate.to_string().into(),
        "-f".into(),
        "s16le".into(),
        "-".into(),
    ];
    let streaming_output = run_tool_streaming_stdout(
        "ffmpeg",
        ffmpeg,
        args,
        cancel_flag,
        FFMPEG_AUDIO_V3_TIMEOUT,
        move |chunk| {
            pcm_writer
                .lock()
                .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason: "sampled PCM buffer was poisoned".to_owned(),
                })?
                .extend_from_slice(chunk);
            Ok(())
        },
    )?;
    let pcm = Arc::try_unwrap(pcm)
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled PCM buffer was still shared".to_owned(),
        })?
        .into_inner()
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled PCM buffer was poisoned".to_owned(),
        })?;
    Ok((pcm, streaming_output, started_at.elapsed().as_millis()))
}

fn analyze_sampled_window_pcm_bytes(
    pcm: &[u8],
    window_seconds: u32,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut stream = AudioConstellationV3PcmStream::with_config(
        AudioConstellationV3Config::with_sample_rate(V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE),
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
    );
    stream.push_bytes(pcm)?;
    stream.finish(Some(f64::from(window_seconds)))
}

fn merge_audio_stream_metrics(
    target: &mut MediaAudioStreamMetrics,
    source: &MediaAudioStreamMetrics,
) {
    target.streamed_bytes = target.streamed_bytes.saturating_add(source.streamed_bytes);
    target.streamed_samples = target
        .streamed_samples
        .saturating_add(source.streamed_samples);
    target.peak_frames = target.peak_frames.saturating_add(source.peak_frames);
    target.raw_landmarks_emitted = target
        .raw_landmarks_emitted
        .saturating_add(source.raw_landmarks_emitted);
    target.raw_landmarks_before_bounding = target
        .raw_landmarks_before_bounding
        .saturating_add(source.raw_landmarks_before_bounding);
    target.max_buffer_samples = target.max_buffer_samples.max(source.max_buffer_samples);
    target.max_raw_landmarks_seen = target
        .max_raw_landmarks_seen
        .max(source.max_raw_landmarks_seen);
    target.max_raw_landmarks_after_compaction = target
        .max_raw_landmarks_after_compaction
        .max(source.max_raw_landmarks_after_compaction);
    target.raw_landmark_compactions = target
        .raw_landmark_compactions
        .saturating_add(source.raw_landmark_compactions);
    target.analyzer_millis = target
        .analyzer_millis
        .saturating_add(source.analyzer_millis);
    target.peak_selection_millis = target
        .peak_selection_millis
        .saturating_add(source.peak_selection_millis);
    target.pairing_millis = target.pairing_millis.saturating_add(source.pairing_millis);
    target.compaction_millis = target
        .compaction_millis
        .saturating_add(source.compaction_millis);
    target.reservoir_millis = target
        .reservoir_millis
        .saturating_add(source.reservoir_millis);
    target.candidate_pairs_considered = target
        .candidate_pairs_considered
        .saturating_add(source.candidate_pairs_considered);
    target.candidate_pairs_skipped_by_anchor_gate = target
        .candidate_pairs_skipped_by_anchor_gate
        .saturating_add(source.candidate_pairs_skipped_by_anchor_gate);
    target.candidate_pairs_skipped_by_target_gate = target
        .candidate_pairs_skipped_by_target_gate
        .saturating_add(source.candidate_pairs_skipped_by_target_gate);
    target.candidate_pairs_skipped_by_saturation = target
        .candidate_pairs_skipped_by_saturation
        .saturating_add(source.candidate_pairs_skipped_by_saturation);
    target.candidate_pairs_emitted = target
        .candidate_pairs_emitted
        .saturating_add(source.candidate_pairs_emitted);
    target.anchor_peaks_considered = target
        .anchor_peaks_considered
        .saturating_add(source.anchor_peaks_considered);
    target.anchor_peaks_selected = target
        .anchor_peaks_selected
        .saturating_add(source.anchor_peaks_selected);
    target.anchor_peaks_skipped_by_gate = target
        .anchor_peaks_skipped_by_gate
        .saturating_add(source.anchor_peaks_skipped_by_gate);
    target.target_peaks_considered = target
        .target_peaks_considered
        .saturating_add(source.target_peaks_considered);
    target.target_peaks_selected = target
        .target_peaks_selected
        .saturating_add(source.target_peaks_selected);
    target.landmarks_accepted_into_reservoir = target
        .landmarks_accepted_into_reservoir
        .saturating_add(source.landmarks_accepted_into_reservoir);
    target.landmarks_rejected_by_reservoir = target
        .landmarks_rejected_by_reservoir
        .saturating_add(source.landmarks_rejected_by_reservoir);
}

fn sum_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub fn media_source_path_info(path: impl AsRef<Path>) -> MediaSourcePathInfo {
    let root = media_source_root(path.as_ref());
    let kind = media_source_path_kind(&root);
    MediaSourcePathInfo {
        volume_id: (!root.is_empty()).then(|| root.clone()),
        root,
        kind,
    }
}

fn media_source_root(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = text.strip_prefix("\\\\") {
        let mut parts = stripped.split('\\').filter(|part| !part.is_empty());
        if let (Some(server), Some(share)) = (parts.next(), parts.next()) {
            return format!("\\\\{server}\\{share}\\").to_lowercase();
        }
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return format!("{}:\\", text[..1].to_ascii_uppercase());
    }
    path.components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(windows)]
fn media_source_path_kind(root: &str) -> String {
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    let mut normalized = root.replace('/', "\\");
    if normalized.len() == 2 && normalized.ends_with(':') {
        normalized.push('\\');
    }
    if normalized.is_empty() {
        return "unknown".to_owned();
    }
    let wide = std::ffi::OsStr::new(&normalized)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide` is a null-terminated UTF-16 buffer that lives for this call.
    let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
    match drive_type {
        DRIVE_FIXED => "local",
        DRIVE_REMOTE => "network",
        DRIVE_REMOVABLE => "removable",
        _ => "unknown",
    }
    .to_owned()
}

#[cfg(not(windows))]
fn media_source_path_kind(root: &str) -> String {
    if root.is_empty() {
        "unknown".to_owned()
    } else {
        "local".to_owned()
    }
}

pub(crate) fn run_tool_output<I>(
    tool: &'static str,
    executable: &Path,
    args: I,
    cancel_flag: Option<&AtomicBool>,
    timeout: Duration,
) -> Result<Output, MediaFingerprintError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut command = hidden_media_match_command(executable);
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: error.to_string(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: "failed capturing stdout".to_owned(),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: "failed capturing stderr".to_owned(),
        })?;
    let stdout_reader = thread::spawn(move || read_pipe_to_end(stdout));
    let stderr_reader = thread::spawn(move || read_pipe_to_end(stderr));
    let started_at = Instant::now();

    loop {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_pipe_reader(stdout_reader, tool, "stdout");
            let _ = join_pipe_reader(stderr_reader, tool, "stderr");
            return Err(MediaFingerprintError::Cancelled { tool });
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_pipe_reader(stdout_reader, tool, "stdout");
            let _ = join_pipe_reader(stderr_reader, tool, "stderr");
            return Err(MediaFingerprintError::TimedOut {
                tool,
                timeout_seconds: timeout.as_secs().max(1),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = join_pipe_reader(stdout_reader, tool, "stdout")?;
                let stderr = join_pipe_reader(stderr_reader, tool, "stderr")?;
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => thread::sleep(MEDIA_TOOL_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe_reader(stdout_reader, tool, "stdout");
                let _ = join_pipe_reader(stderr_reader, tool, "stderr");
                return Err(MediaFingerprintError::ToolFailed {
                    tool,
                    status: None,
                    stderr: error.to_string(),
                });
            }
        }
    }
}

pub(crate) fn run_tool_streaming_stdout<I, F>(
    tool: &'static str,
    executable: &Path,
    args: I,
    cancel_flag: Option<&AtomicBool>,
    timeout: Duration,
    mut on_stdout_chunk: F,
) -> Result<MediaToolStreamingOutput, MediaFingerprintError>
where
    I: IntoIterator<Item = OsString>,
    F: FnMut(&[u8]) -> Result<(), MediaFingerprintError> + Send + 'static,
{
    let mut command = hidden_media_match_command(executable);
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: error.to_string(),
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: "failed capturing stdout".to_owned(),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: "failed capturing stderr".to_owned(),
        })?;
    let (stdout_error_sender, stdout_error_receiver) = mpsc::channel::<MediaFingerprintError>();
    let stdout_bytes = Arc::new(AtomicU64::new(0));
    let stdout_byte_counter = Arc::clone(&stdout_bytes);
    let stdout_reader = thread::spawn(move || {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = match stdout.read(&mut buffer) {
                Ok(count) => count,
                Err(error) => {
                    let error = MediaFingerprintError::ToolFailed {
                        tool,
                        status: None,
                        stderr: format!("failed reading stdout: {error}"),
                    };
                    let _ = stdout_error_sender.send(error.clone());
                    return Err(error);
                }
            };
            if count == 0 {
                return Ok(());
            }
            stdout_byte_counter.fetch_add(count as u64, Ordering::Relaxed);
            if let Err(error) = on_stdout_chunk(&buffer[..count]) {
                let _ = stdout_error_sender.send(error.clone());
                return Err(error);
            }
        }
    });
    let stderr_reader = thread::spawn(move || read_pipe_to_end(stderr));
    let started_at = Instant::now();

    loop {
        match stdout_error_receiver.try_recv() {
            Ok(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_streaming_stdout_reader(stdout_reader, tool);
                let _ = join_pipe_reader(stderr_reader, tool, "stderr");
                return Err(error);
            }
            Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => {}
        }
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_streaming_stdout_reader(stdout_reader, tool);
            let _ = join_pipe_reader(stderr_reader, tool, "stderr");
            return Err(MediaFingerprintError::Cancelled { tool });
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_streaming_stdout_reader(stdout_reader, tool);
            let _ = join_pipe_reader(stderr_reader, tool, "stderr");
            return Err(MediaFingerprintError::TimedOut {
                tool,
                timeout_seconds: timeout.as_secs().max(1),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let exit_millis = started_at.elapsed().as_millis();
                let process_io = process_io_counters(&child);
                let stdout_result = join_streaming_stdout_reader(stdout_reader, tool);
                let stderr = join_pipe_reader(stderr_reader, tool, "stderr")?;
                if !status.success() {
                    return Err(MediaFingerprintError::ToolFailed {
                        tool,
                        status: status.code(),
                        stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
                    });
                }
                stdout_result?;
                return Ok(MediaToolStreamingOutput {
                    stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                    process_io,
                    exit_millis,
                });
            }
            Ok(None) => thread::sleep(MEDIA_TOOL_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_streaming_stdout_reader(stdout_reader, tool);
                let _ = join_pipe_reader(stderr_reader, tool, "stderr");
                return Err(MediaFingerprintError::ToolFailed {
                    tool,
                    status: None,
                    stderr: error.to_string(),
                });
            }
        }
    }
}

fn join_streaming_stdout_reader(
    reader: thread::JoinHandle<Result<(), MediaFingerprintError>>,
    tool: &'static str,
) -> Result<(), MediaFingerprintError> {
    match reader.join() {
        Ok(result) => result,
        Err(_) => Err(MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: "stdout reader thread panicked".to_owned(),
        }),
    }
}

fn read_pipe_to_end(mut pipe: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .map(|_| bytes)
        .map_err(|error| error.to_string())
}

fn join_pipe_reader(
    reader: thread::JoinHandle<Result<Vec<u8>, String>>,
    tool: &'static str,
    pipe: &'static str,
) -> Result<Vec<u8>, MediaFingerprintError> {
    match reader.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: format!("failed reading {pipe}: {error}"),
        }),
        Err(_) => Err(MediaFingerprintError::ToolFailed {
            tool,
            status: None,
            stderr: format!("{pipe} reader thread panicked"),
        }),
    }
}

fn ensure_tool_success(tool: &'static str, output: &Output) -> Result<(), MediaFingerprintError> {
    if output.status.success() {
        return Ok(());
    }
    Err(MediaFingerprintError::ToolFailed {
        tool,
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

#[cfg(windows)]
fn hidden_media_match_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn hidden_media_match_command(executable: &Path) -> Command {
    Command::new(executable)
}

#[cfg(windows)]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct WindowsIoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetDriveTypeW(root_path_name: *const u16) -> u32;
    fn GetProcessIoCounters(
        process: *mut std::ffi::c_void,
        counters: *mut WindowsIoCounters,
    ) -> i32;
}

#[cfg(windows)]
fn process_io_counters(child: &std::process::Child) -> MediaToolProcessIoMetrics {
    let mut counters = WindowsIoCounters::default();
    // SAFETY: `child.as_raw_handle()` is a live process handle while `child` is alive, and
    // `counters` points to writable memory with the Windows `IO_COUNTERS` layout.
    let ok = unsafe {
        GetProcessIoCounters(
            child.as_raw_handle().cast::<std::ffi::c_void>(),
            &mut counters,
        )
    };
    if ok == 0 {
        return MediaToolProcessIoMetrics::default();
    }
    MediaToolProcessIoMetrics {
        read_bytes: Some(counters.read_transfer_count),
        read_ops: Some(counters.read_operation_count),
    }
}

#[cfg(not(windows))]
fn process_io_counters(_child: &std::process::Child) -> MediaToolProcessIoMetrics {
    MediaToolProcessIoMetrics::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_sampled_windows_use_three_body_windows() {
        let windows = sampled_audio_windows_v3(Some(1500.0));

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].1, 20);
        assert!(windows[0].0 >= 180.0);
        assert!(windows[2].0 > windows[1].0);
    }

    #[test]
    fn source_path_info_classifies_drive_roots() {
        let info = media_source_path_info("E:\\Anime\\file.mkv");

        assert_eq!(info.root, "E:\\");
        assert!(matches!(
            info.kind.as_str(),
            "local" | "network" | "removable" | "unknown"
        ));
    }
}
