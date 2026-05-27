use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use crate::{
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION,
    anchors::{
        audio_anchors_from_record, media_fingerprint_wire_summary_from_record,
        video_anchors_from_record,
    },
    audio_v3::{
        AudioConstellationV3PcmStream, AudioLandmarkV3,
        bounded_time_distributed_audio_landmarks_v3_for_duration,
    },
    identity::{container_fingerprint_from_metadata, normalize_media_path},
    settings::{MediaAudioIndexMode, MediaExtractionSettings, MediaFingerprintProfile},
    tuning::{
        FFMPEG_AUDIO_V3_TIMEOUT, FFMPEG_FULL_VIDEO_TIMEOUT, FFPROBE_TIMEOUT,
        MEDIA_TOOL_POLL_INTERVAL, V3_AUDIO_SAMPLE_RATE, V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
        V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS, V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS,
        V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE, V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS,
        V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS, V3_AUDIO_SAMPLED_MIN_BODY_REGIONS,
        V3_AUDIO_SAMPLED_NORMAL_INDEX_LANDMARK_LIMIT, V3_AUDIO_SAMPLED_NORMAL_MAX_WINDOWS,
        V3_AUDIO_SAMPLED_NORMAL_MIN_WINDOWS, V3_AUDIO_SAMPLED_NORMAL_SAMPLE_RATE,
        V3_AUDIO_SAMPLED_NORMAL_TARGET_LANDMARKS, V3_AUDIO_SAMPLED_NORMAL_WINDOW_SECONDS,
        V3_AUDIO_SPARSE_FULL_SAMPLE_RATE, V3_AUDIO_SPARSE_FULL_VERIFY_LANDMARK_LIMIT,
        V3_AUDIO_VERIFY_LANDMARK_LIMIT, VIDEO_FRAME_BYTES, VIDEO_FRAME_HEIGHT, VIDEO_FRAME_WIDTH,
    },
    types::{MediaFileIdentity, MediaFingerprintRecord},
    video_v3::{
        FrameFingerprint, VideoFingerprint, pdq_style_luma_hash,
        video_landmarks_v3_from_luma_frames,
    },
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
pub struct MediaExtractionTimings {
    pub ffprobe_millis: u128,
    pub audio_millis: u128,
    pub video_millis: u128,
    pub total_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaAudioStreamMetrics {
    pub streamed_bytes: usize,
    pub streamed_samples: usize,
    pub peak_frames: usize,
    pub raw_landmarks_emitted: usize,
    pub raw_landmarks_before_bounding: usize,
    pub final_landmarks: usize,
    pub max_buffer_samples: usize,
    pub max_raw_landmarks_seen: usize,
    pub max_raw_landmarks_after_compaction: usize,
    pub raw_landmark_compactions: usize,
    pub analyzer_millis: u128,
    pub peak_selection_millis: u128,
    pub pairing_millis: u128,
    pub compaction_millis: u128,
    pub reservoir_millis: u128,
    pub final_selection_millis: u128,
    pub ffmpeg_process_wall_millis: u128,
    pub pcm_decode_drain_millis: u128,
    pub ffmpeg_decode_stream_millis: u128,
    pub sampled_audio_seconds_decoded: u32,
    pub sampled_audio_windows_decoded: usize,
    pub full_audio_seconds_decoded: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaFingerprintExtractionReport {
    pub invocations: MediaToolInvocationCounts,
    pub timings: MediaExtractionTimings,
    pub audio_stream: MediaAudioStreamMetrics,
    pub audio_error: Option<String>,
    pub video_error: Option<String>,
    pub serialized_debug_record_bytes: usize,
    pub audio_summary_bytes: usize,
    pub video_summary_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentedMediaFingerprint {
    pub record: MediaFingerprintRecord,
    pub report: MediaFingerprintExtractionReport,
}

#[cfg(test)]
pub(crate) fn expected_media_tool_invocation_counts(
    settings: &MediaExtractionSettings,
) -> MediaToolInvocationCounts {
    MediaToolInvocationCounts {
        ffmpeg: if settings.profile.uses_video_by_default() {
            2
        } else {
            1
        },
        ffprobe: 1,
    }
}

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
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, None)
        .map(|fingerprint| fingerprint.record)
}

pub fn fingerprint_media_file_cancellable(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: &AtomicBool,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, Some(cancel_flag))
        .map(|fingerprint| fingerprint.record)
}

pub fn fingerprint_media_file_cancellable_with_report(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: &AtomicBool,
) -> Result<InstrumentedMediaFingerprint, MediaFingerprintError> {
    fingerprint_media_file_with_report(path, tools, extraction_settings, Some(cancel_flag))
}

pub fn fingerprint_media_file_with_report(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<InstrumentedMediaFingerprint, MediaFingerprintError> {
    let total_started_at = Instant::now();
    let path = path.as_ref();
    let metadata =
        std::fs::metadata(path).map_err(|error| MediaFingerprintError::FileMetadata {
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
    let mut audio_anchors = Vec::new();
    let started_at = Instant::now();
    let audio_result = match extraction_settings.audio_index_mode {
        MediaAudioIndexMode::FullVerify => extract_audio_constellation_v3_with_metrics(
            &tools.ffmpeg,
            path,
            duration_seconds,
            cancel_flag,
        ),
        MediaAudioIndexMode::SparseFull => extract_audio_constellation_v3_sparse_full_with_metrics(
            &tools.ffmpeg,
            path,
            duration_seconds,
            cancel_flag,
        ),
        MediaAudioIndexMode::SampledFast | MediaAudioIndexMode::SampledNormal => {
            extract_audio_constellation_v3_sampled_index_with_metrics(
                &tools.ffmpeg,
                path,
                duration_seconds,
                extraction_settings.audio_index_mode,
                cancel_flag,
            )
        }
    };
    report.invocations.ffmpeg += 1;
    report.timings.audio_millis = started_at.elapsed().as_millis();
    match audio_result {
        Ok((anchors, metrics)) => {
            report.audio_stream = metrics;
            audio_anchors = anchors
                .into_iter()
                .map(|landmark| AudioAnchor {
                    bucket: landmark.hash,
                    t_ms: landmark.t_ms,
                    weight: u16::from(landmark.weight.max(1)),
                })
                .collect();
        }
        Err(MediaFingerprintError::Cancelled { tool }) => {
            return Err(MediaFingerprintError::Cancelled { tool });
        }
        Err(error) => {
            report.audio_error = Some(error.to_string());
        }
    }
    let (video, video_anchors) = if extraction_settings.profile.uses_video_by_default() {
        let started_at = Instant::now();
        let video_result = extract_video_fingerprint_with_cancellation(
            &tools.ffmpeg,
            path,
            duration_seconds,
            extraction_settings,
            cancel_flag,
        );
        report.invocations.ffmpeg += 1;
        report.timings.video_millis = started_at.elapsed().as_millis();
        match video_result {
            Ok(video) => (Some(video), Vec::new()),
            Err(MediaFingerprintError::Cancelled { tool }) => {
                return Err(MediaFingerprintError::Cancelled { tool });
            }
            Err(error) => {
                report.video_error = Some(error.to_string());
                (None, Vec::new())
            }
        }
    } else {
        (None, Vec::new())
    };

    let audio_error = report.audio_error.clone();
    let video_error = report.video_error.clone();
    let mut record = MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path,
            modified_unix_millis,
            size_bytes,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: extraction_settings.clone(),
        duration_seconds,
        container_fingerprint,
        video,
        audio_anchors,
        video_anchors,
        audio_error,
        video_error,
    };
    if record.audio_anchors.is_empty() {
        record.audio_anchors = audio_anchors_from_record(&record);
    }
    if record.video_anchors.is_empty() {
        record.video_anchors = video_anchors_from_record(&record);
    }
    let summary = media_fingerprint_wire_summary_from_record(&record);
    report.serialized_debug_record_bytes = serde_json::to_vec(&record)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    report.audio_summary_bytes = summary.audio_summary.as_ref().map(Vec::len).unwrap_or(0);
    report.video_summary_bytes = summary.video_summary.as_ref().map(Vec::len).unwrap_or(0);
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

pub(crate) fn extract_audio_constellation_v3_with_metrics(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    extract_audio_constellation_v3_with_sample_rate_and_limit(
        ffmpeg,
        media_path,
        duration_seconds,
        V3_AUDIO_SAMPLE_RATE,
        V3_AUDIO_VERIFY_LANDMARK_LIMIT,
        cancel_flag,
    )
}

pub(crate) fn extract_audio_constellation_v3_sparse_full_with_metrics(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    extract_audio_constellation_v3_with_sample_rate_and_limit(
        ffmpeg,
        media_path,
        duration_seconds,
        V3_AUDIO_SPARSE_FULL_SAMPLE_RATE,
        V3_AUDIO_SPARSE_FULL_VERIFY_LANDMARK_LIMIT,
        cancel_flag,
    )
}

fn extract_audio_constellation_v3_with_sample_rate_and_limit(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    sample_rate: u32,
    landmark_limit: usize,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let stream = Arc::new(Mutex::new(
        AudioConstellationV3PcmStream::with_landmark_limit(sample_rate, landmark_limit),
    ));
    let stream_reader = Arc::clone(&stream);
    let decode_started_at = Instant::now();
    run_tool_streaming_stdout(
        "ffmpeg",
        ffmpeg.as_ref(),
        [
            "-v".into(),
            "error".into(),
            "-nostdin".into(),
            "-i".into(),
            media_path.as_ref().as_os_str().to_os_string(),
            "-vn".into(),
            "-ac".into(),
            "1".into(),
            "-ar".into(),
            sample_rate.to_string().into(),
            "-f".into(),
            "s16le".into(),
            "-".into(),
        ],
        cancel_flag,
        FFMPEG_AUDIO_V3_TIMEOUT,
        move |chunk| {
            stream_reader
                .lock()
                .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason: "audio stream state was poisoned".to_owned(),
                })?
                .push_bytes(chunk)
        },
    )?;
    let decode_stream_millis = decode_started_at.elapsed().as_millis();
    let stream = Arc::try_unwrap(stream).map_err(|_| MediaFingerprintError::InvalidToolOutput {
        tool: "ffmpeg",
        reason: "audio stream state was still shared after ffmpeg exit".to_owned(),
    })?;
    let stream = stream
        .into_inner()
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "audio stream state was poisoned".to_owned(),
        })?;
    let (landmarks, mut metrics) = stream.finish(duration_seconds)?;
    metrics.ffmpeg_process_wall_millis = decode_stream_millis;
    metrics.pcm_decode_drain_millis = decode_stream_millis;
    metrics.ffmpeg_decode_stream_millis = decode_stream_millis;
    metrics.full_audio_seconds_decoded = duration_seconds
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .map(|duration| duration.ceil() as u32)
        .unwrap_or(0);
    if landmarks.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((landmarks, metrics))
}

pub(crate) fn extract_audio_constellation_v3_sampled_index_with_metrics(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    index_mode: MediaAudioIndexMode,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let config = sampled_audio_index_config(index_mode);
    let windows = sampled_audio_windows_v3(duration_seconds, config);
    if windows.is_empty() {
        return extract_audio_constellation_v3_with_metrics(
            ffmpeg,
            media_path,
            duration_seconds,
            cancel_flag,
        );
    }

    let mut all_landmarks = Vec::new();
    let mut combined_metrics = MediaAudioStreamMetrics::default();
    let mut process_wall_millis = 0u128;
    let mut body_regions = BTreeSet::new();
    let mut unique_hashes = BTreeSet::new();
    for (window_index, (start_seconds, window_seconds)) in windows.into_iter().enumerate() {
        let started_at = Instant::now();
        let stream = Arc::new(Mutex::new(AudioConstellationV3PcmStream::new(
            config.sample_rate,
        )));
        let stream_reader = Arc::clone(&stream);
        run_tool_streaming_stdout(
            "ffmpeg",
            ffmpeg.as_ref(),
            [
                "-v".into(),
                "error".into(),
                "-nostdin".into(),
                "-ss".into(),
                format!("{start_seconds:.3}").into(),
                "-t".into(),
                window_seconds.to_string().into(),
                "-i".into(),
                media_path.as_ref().as_os_str().to_os_string(),
                "-vn".into(),
                "-ac".into(),
                "1".into(),
                "-ar".into(),
                config.sample_rate.to_string().into(),
                "-f".into(),
                "s16le".into(),
                "-".into(),
            ],
            cancel_flag,
            FFMPEG_AUDIO_V3_TIMEOUT,
            move |chunk| {
                stream_reader
                    .lock()
                    .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                        tool: "ffmpeg",
                        reason: "audio stream state was poisoned".to_owned(),
                    })?
                    .push_bytes(chunk)
            },
        )?;
        let window_wall = started_at.elapsed().as_millis();
        process_wall_millis += window_wall;
        let stream =
            Arc::try_unwrap(stream).map_err(|_| MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "audio stream state was still shared after ffmpeg exit".to_owned(),
            })?;
        let stream = stream
            .into_inner()
            .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "audio stream state was poisoned".to_owned(),
            })?;
        let (mut landmarks, metrics) = stream.finish(Some(f64::from(window_seconds)))?;
        let start_ms = (start_seconds * 1000.0)
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32;
        for landmark in &mut landmarks {
            landmark.t_ms = landmark.t_ms.saturating_add(start_ms);
        }
        for landmark in &landmarks {
            body_regions.insert(landmark.t_ms / 60_000);
            unique_hashes.insert(landmark.hash);
        }
        all_landmarks.extend(landmarks);
        merge_audio_stream_metrics(&mut combined_metrics, &metrics);
        combined_metrics.sampled_audio_seconds_decoded = combined_metrics
            .sampled_audio_seconds_decoded
            .saturating_add(window_seconds);
        combined_metrics.sampled_audio_windows_decoded += 1;
        let windows_decoded = window_index + 1;
        if windows_decoded >= config.min_windows
            && all_landmarks.len() >= config.target_landmarks
            && body_regions.len() >= config.min_body_regions
            && unique_hashes.len() >= config.target_landmarks.saturating_mul(3) / 4
        {
            break;
        }
    }

    let selection_started_at = Instant::now();
    let mut bounded = all_landmarks;
    let raw_before_bounding = bounded.len();
    bounded = bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut bounded,
        config.max_landmarks,
        duration_seconds,
    );
    combined_metrics.final_selection_millis = combined_metrics
        .final_selection_millis
        .saturating_add(selection_started_at.elapsed().as_millis());
    combined_metrics.final_landmarks = bounded.len();
    combined_metrics.raw_landmarks_before_bounding = raw_before_bounding;
    combined_metrics.ffmpeg_process_wall_millis = process_wall_millis;
    combined_metrics.pcm_decode_drain_millis = process_wall_millis;
    combined_metrics.ffmpeg_decode_stream_millis = process_wall_millis;
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((bounded, combined_metrics))
}

#[derive(Debug, Clone, Copy)]
struct SampledAudioIndexConfig {
    sample_rate: u32,
    window_seconds: u32,
    min_windows: usize,
    max_windows: usize,
    target_landmarks: usize,
    max_landmarks: usize,
    min_body_regions: usize,
}

fn sampled_audio_index_config(index_mode: MediaAudioIndexMode) -> SampledAudioIndexConfig {
    match index_mode {
        MediaAudioIndexMode::FullVerify | MediaAudioIndexMode::SparseFull => {
            SampledAudioIndexConfig {
                sample_rate: V3_AUDIO_SAMPLE_RATE,
                window_seconds: V3_AUDIO_SAMPLED_NORMAL_WINDOW_SECONDS,
                min_windows: V3_AUDIO_SAMPLED_NORMAL_MIN_WINDOWS,
                max_windows: V3_AUDIO_SAMPLED_NORMAL_MAX_WINDOWS,
                target_landmarks: V3_AUDIO_SAMPLED_NORMAL_TARGET_LANDMARKS,
                max_landmarks: V3_AUDIO_SAMPLED_NORMAL_INDEX_LANDMARK_LIMIT,
                min_body_regions: V3_AUDIO_SAMPLED_MIN_BODY_REGIONS,
            }
        }
        MediaAudioIndexMode::SampledFast => SampledAudioIndexConfig {
            sample_rate: V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
            window_seconds: V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
            min_windows: V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS,
            max_windows: V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS,
            target_landmarks: V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS,
            max_landmarks: V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
            min_body_regions: V3_AUDIO_SAMPLED_MIN_BODY_REGIONS
                .min(V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS),
        },
        MediaAudioIndexMode::SampledNormal => SampledAudioIndexConfig {
            sample_rate: V3_AUDIO_SAMPLED_NORMAL_SAMPLE_RATE,
            window_seconds: V3_AUDIO_SAMPLED_NORMAL_WINDOW_SECONDS,
            min_windows: V3_AUDIO_SAMPLED_NORMAL_MIN_WINDOWS,
            max_windows: V3_AUDIO_SAMPLED_NORMAL_MAX_WINDOWS,
            target_landmarks: V3_AUDIO_SAMPLED_NORMAL_TARGET_LANDMARKS,
            max_landmarks: V3_AUDIO_SAMPLED_NORMAL_INDEX_LANDMARK_LIMIT,
            min_body_regions: V3_AUDIO_SAMPLED_MIN_BODY_REGIONS,
        },
    }
}

fn sampled_audio_windows_v3(
    duration_seconds: Option<f64>,
    config: SampledAudioIndexConfig,
) -> Vec<(f64, u32)> {
    let duration = duration_seconds.unwrap_or(0.0);
    if !duration.is_finite() || duration <= f64::from(config.window_seconds) {
        return Vec::new();
    }
    let window = config.window_seconds;
    let count = config.max_windows;
    let edge_skip: f64 = if duration >= 600.0 { 180.0 } else { 30.0 };
    let body_start = edge_skip.min((duration - f64::from(window)).max(0.0));
    let body_end = (duration - edge_skip - f64::from(window)).max(body_start);
    let mut starts = Vec::new();
    if count <= 1 || body_end <= body_start {
        starts.push(body_start);
    } else {
        for index in 0..count {
            let fraction = index as f64 / (count - 1) as f64;
            starts.push(body_start + (body_end - body_start) * fraction);
        }
    }
    starts
        .into_iter()
        .map(|start| (start.max(0.0), window))
        .collect()
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
    target.final_selection_millis = target
        .final_selection_millis
        .saturating_add(source.final_selection_millis);
    target.sampled_audio_windows_decoded = target
        .sampled_audio_windows_decoded
        .saturating_add(source.sampled_audio_windows_decoded);
}

pub(crate) fn extract_video_fingerprint_with_cancellation(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    match extraction_settings.profile {
        MediaFingerprintProfile::AudioConstellationV3 => {
            Err(MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "audio-constellation-v3 does not request video extraction".to_owned(),
            })
        }
        MediaFingerprintProfile::CombinedV3 => extract_full_video_fingerprint(
            ffmpeg,
            media_path,
            duration_seconds,
            extraction_settings,
            cancel_flag,
        ),
    }
}

fn extract_full_video_fingerprint(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    let interval = extraction_settings.frame_sample_interval_seconds.max(1);
    let output = run_tool_output(
        "ffmpeg",
        ffmpeg.as_ref(),
        [
            "-v".into(),
            "info".into(),
            "-hide_banner".into(),
            "-nostats".into(),
            "-nostdin".into(),
            "-i".into(),
            media_path.as_ref().as_os_str().to_os_string(),
            "-vf".into(),
            format!(
                "fps=1/{interval},showinfo,scale={VIDEO_FRAME_WIDTH}:{VIDEO_FRAME_HEIGHT}:flags=bicubic,format=gray"
            )
            .into(),
            "-frames:v".into(),
            extraction_settings.max_frames.max(1).to_string().into(),
            "-fps_mode".into(),
            "vfr".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "gray".into(),
            "-".into(),
        ],
        cancel_flag,
        FFMPEG_FULL_VIDEO_TIMEOUT,
    )?;
    ensure_tool_success("ffmpeg", &output)?;
    video_fingerprint_from_ffmpeg_rawvideo(&output.stdout, &output.stderr, duration_seconds)
}

pub(crate) fn video_fingerprint_from_ffmpeg_rawvideo(
    stdout: &[u8],
    stderr: &[u8],
    duration_seconds: Option<f64>,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    let frames = video_frames_from_ffmpeg_rawvideo(stdout, stderr)?;
    let pts_times = frames
        .iter()
        .map(|frame| frame.timestamp_millis.min(u64::from(u32::MAX)) as u32)
        .collect::<Vec<_>>();
    let mut luma_frames = Vec::with_capacity(pts_times.len());
    for (t_ms, chunk) in pts_times
        .iter()
        .copied()
        .zip(stdout.chunks_exact(VIDEO_FRAME_BYTES))
    {
        luma_frames.push((t_ms, chunk.to_vec()));
    }
    let v3_landmarks =
        video_landmarks_v3_from_luma_frames(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, &luma_frames);

    Ok(VideoFingerprint {
        duration_seconds: duration_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round().min(f64::from(u32::MAX)) as u32),
        frames,
        v3_landmarks,
    })
}

pub(crate) fn video_frames_from_ffmpeg_rawvideo(
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Vec<FrameFingerprint>, MediaFingerprintError> {
    if !stdout.len().is_multiple_of(VIDEO_FRAME_BYTES) {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "raw grayscale frame output had a partial trailing frame".to_owned(),
        });
    }
    let frame_count = stdout.len() / VIDEO_FRAME_BYTES;
    let stderr = String::from_utf8_lossy(stderr);
    let pts_times = parse_ffmpeg_showinfo_pts_times(&stderr);
    if pts_times.len() < frame_count {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: format!(
                "ffmpeg emitted {frame_count} raw frames but only {} frame timestamps",
                pts_times.len()
            ),
        });
    }
    let mut frames = Vec::new();
    for (index, chunk) in stdout.chunks_exact(VIDEO_FRAME_BYTES).enumerate() {
        let hash =
            pdq_style_luma_hash(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, chunk).ok_or_else(|| {
                MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason:
                        "raw grayscale frame size did not match the requested extraction geometry"
                            .to_owned(),
                }
            })?;
        frames.push(FrameFingerprint::new(pts_times[index], hash));
    }

    if frames.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "no raw grayscale frames were extracted".to_owned(),
        });
    }
    Ok(frames)
}

pub(crate) fn parse_ffmpeg_showinfo_pts_times(output: &str) -> Vec<f64> {
    output
        .lines()
        .filter_map(|line| {
            let (_, after_marker) = line.split_once("pts_time:")?;
            let value = after_marker
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim();
            value
                .parse::<f64>()
                .ok()
                .filter(|timestamp| timestamp.is_finite() && *timestamp >= 0.0)
        })
        .collect()
}

pub(crate) fn run_tool_output<I>(
    tool: &'static str,
    executable: &Path,
    args: I,
    cancel_flag: Option<&AtomicBool>,
    timeout: Duration,
) -> Result<Output, MediaFingerprintError>
where
    I: IntoIterator<Item = std::ffi::OsString>,
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
) -> Result<(), MediaFingerprintError>
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
                return Ok(());
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

fn ensure_tool_success(
    tool: &'static str,
    output: &std::process::Output,
) -> Result<(), MediaFingerprintError> {
    if output.status.success() {
        return Ok(());
    }
    Err(MediaFingerprintError::ToolFailed {
        tool,
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}
