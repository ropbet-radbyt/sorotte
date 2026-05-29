use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    anchors::{
        audio_anchors_from_record, media_fingerprint_wire_summary_from_record,
        video_anchors_from_record,
    },
    audio_v3::{
        AudioConstellationV3Config, AudioConstellationV3PcmStream, AudioLandmarkV3,
        bounded_time_distributed_audio_landmarks_v3_for_duration,
    },
    identity::{container_fingerprint_from_metadata, normalize_media_path},
    settings::{
        MediaAudioIndexMode, MediaDenseAudioProfile, MediaExtractionSettings,
        MediaFingerprintProfile,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaExtractionTimings {
    pub ffprobe_millis: u128,
    pub audio_millis: u128,
    pub video_millis: u128,
    pub total_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaAudioStreamMetrics {
    pub source_path_root: Option<String>,
    pub source_path_kind: Option<String>,
    pub source_volume_id: Option<String>,
    pub ffmpeg_command_kind: Option<String>,
    pub ffmpeg_selected_stream: Option<String>,
    pub ffmpeg_disabled_video: bool,
    pub ffmpeg_disabled_subtitles: bool,
    pub ffmpeg_disabled_data: bool,
    pub container_format: Option<String>,
    pub audio_stream_index: Option<usize>,
    pub audio_codec: Option<String>,
    pub audio_bitrate_bps: Option<u64>,
    pub audio_duration_millis: Option<u64>,
    pub audio_start_time_millis: Option<i64>,
    pub audio_packet_positions_available: Option<bool>,
    pub audio_packet_position_completeness_per_mille: Option<u16>,
    pub audio_packet_positions_monotonic: Option<bool>,
    pub average_audio_packet_size_bytes: Option<u64>,
    pub audio_packet_count_in_sampled_windows: Option<usize>,
    pub audio_packet_probe_millis: Option<u128>,
    pub audio_packet_probe_read_bytes: Option<u64>,
    pub audio_packet_window_compressed_bytes: Option<u64>,
    pub audio_packet_window_coalesced_range_bytes: Option<u64>,
    pub audio_packet_read_savings_estimate_bytes: Option<i64>,
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
    pub pcm_drain_thread_millis: u128,
    pub analyzer_thread_millis: u128,
    pub channel_backpressure_millis: u128,
    pub max_queued_pcm_bytes: usize,
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
    pub ffmpeg_process_wall_millis: u128,
    pub ffmpeg_input_read_bytes: Option<u64>,
    pub ffmpeg_input_read_ops: Option<u64>,
    pub ffmpeg_output_pcm_bytes: u64,
    pub ffmpeg_invocation_count: usize,
    pub sampled_window_seek_millis: u128,
    pub sampled_window_decode_millis: u128,
    pub ffmpeg_open_probe_millis: u128,
    pub ffmpeg_exit_millis: u128,
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
            extraction_settings.dense_audio_profile,
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
    dense_profile: MediaDenseAudioProfile,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let config = AudioConstellationV3Config::dense(dense_profile);
    extract_audio_constellation_v3_with_config_and_limit(
        ffmpeg,
        media_path,
        duration_seconds,
        config,
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
    extract_audio_constellation_v3_with_config_and_limit(
        ffmpeg,
        media_path,
        duration_seconds,
        AudioConstellationV3Config::with_sample_rate(V3_AUDIO_SPARSE_FULL_SAMPLE_RATE),
        V3_AUDIO_SPARSE_FULL_VERIFY_LANDMARK_LIMIT,
        cancel_flag,
    )
}

fn extract_audio_constellation_v3_with_config_and_limit(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    config: AudioConstellationV3Config,
    landmark_limit: usize,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    const PCM_CHUNK_QUEUE_LIMIT: usize = 8;

    let (pcm_sender, pcm_receiver) = mpsc::sync_channel::<Vec<u8>>(PCM_CHUNK_QUEUE_LIMIT);
    let queued_pcm_bytes = Arc::new(AtomicUsize::new(0));
    let max_queued_pcm_bytes = Arc::new(AtomicUsize::new(0));
    let channel_backpressure_nanos = Arc::new(AtomicU64::new(0));
    let analyzer_queued_pcm_bytes = Arc::clone(&queued_pcm_bytes);
    let analyzer_thread = thread::spawn(move || {
        let analyzer_started_at = Instant::now();
        let mut stream = AudioConstellationV3PcmStream::with_config(config, landmark_limit);
        for chunk in pcm_receiver {
            analyzer_queued_pcm_bytes.fetch_sub(chunk.len(), Ordering::AcqRel);
            stream.push_bytes(&chunk)?;
        }
        let (landmarks, mut metrics) = stream.finish(duration_seconds)?;
        metrics.analyzer_thread_millis = analyzer_started_at.elapsed().as_millis();
        Ok::<_, MediaFingerprintError>((landmarks, metrics))
    });
    let sender_queued_pcm_bytes = Arc::clone(&queued_pcm_bytes);
    let sender_max_queued_pcm_bytes = Arc::clone(&max_queued_pcm_bytes);
    let sender_backpressure_nanos = Arc::clone(&channel_backpressure_nanos);
    let decode_started_at = Instant::now();
    let streaming_result = run_tool_streaming_stdout(
        "ffmpeg",
        ffmpeg.as_ref(),
        [
            "-v".into(),
            "error".into(),
            "-nostdin".into(),
            "-threads".into(),
            "1".into(),
            "-i".into(),
            media_path.as_ref().as_os_str().to_os_string(),
            "-map".into(),
            "0:a:0".into(),
            "-vn".into(),
            "-sn".into(),
            "-dn".into(),
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
            let queued =
                sender_queued_pcm_bytes.fetch_add(chunk.len(), Ordering::AcqRel) + chunk.len();
            update_atomic_max_usize(&sender_max_queued_pcm_bytes, queued);
            let send_started_at = Instant::now();
            let result = pcm_sender.send(chunk.to_vec()).map_err(|_| {
                sender_queued_pcm_bytes.fetch_sub(chunk.len(), Ordering::AcqRel);
                MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason: "audio analyzer stopped while ffmpeg was streaming PCM".to_owned(),
                }
            });
            sender_backpressure_nanos.fetch_add(
                send_started_at
                    .elapsed()
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64,
                Ordering::Relaxed,
            );
            result
        },
    );
    let decode_stream_millis = decode_started_at.elapsed().as_millis();
    let analyzer_result =
        analyzer_thread
            .join()
            .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "audio analyzer thread panicked".to_owned(),
            })?;
    let streaming_output = match streaming_result {
        Ok(output) => output,
        Err(error) => {
            let _ = analyzer_result;
            return Err(error);
        }
    };
    let (landmarks, mut metrics) = analyzer_result?;
    let source_info = media_source_path_info(media_path.as_ref());
    metrics.source_path_root = Some(source_info.root);
    metrics.source_path_kind = Some(source_info.kind);
    metrics.source_volume_id = source_info.volume_id;
    mark_audio_only_ffmpeg_command(&mut metrics);
    metrics.pcm_drain_thread_millis = decode_stream_millis;
    metrics.channel_backpressure_millis =
        u128::from(channel_backpressure_nanos.load(Ordering::Relaxed)) / 1_000_000;
    metrics.max_queued_pcm_bytes = max_queued_pcm_bytes.load(Ordering::Relaxed);
    metrics.ffmpeg_process_wall_millis = decode_stream_millis;
    metrics.ffmpeg_input_read_bytes = streaming_output.process_io.read_bytes;
    metrics.ffmpeg_input_read_ops = streaming_output.process_io.read_ops;
    metrics.ffmpeg_output_pcm_bytes = streaming_output.stdout_bytes;
    metrics.ffmpeg_invocation_count = 1;
    metrics.ffmpeg_exit_millis = streaming_output.exit_millis;
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

fn sum_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn mark_audio_only_ffmpeg_command(metrics: &mut MediaAudioStreamMetrics) {
    metrics.ffmpeg_command_kind = Some("audio-only-pcm".to_owned());
    metrics.ffmpeg_selected_stream = Some("0:a:0".to_owned());
    metrics.ffmpeg_disabled_video = true;
    metrics.ffmpeg_disabled_subtitles = true;
    metrics.ffmpeg_disabled_data = true;
}

fn update_atomic_max_usize(target: &AtomicUsize, value: usize) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
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
            MediaDenseAudioProfile::DenseCurrent,
            cancel_flag,
        );
    }

    let mut all_landmarks = Vec::new();
    let mut combined_metrics = MediaAudioStreamMetrics::default();
    let source_info = media_source_path_info(media_path.as_ref());
    combined_metrics.source_path_root = Some(source_info.root);
    combined_metrics.source_path_kind = Some(source_info.kind);
    combined_metrics.source_volume_id = source_info.volume_id;
    mark_audio_only_ffmpeg_command(&mut combined_metrics);
    let mut process_wall_millis = 0u128;
    let mut body_regions = BTreeSet::new();
    let mut unique_hashes = BTreeSet::new();
    for (window_index, (start_seconds, window_seconds)) in windows.into_iter().enumerate() {
        let started_at = Instant::now();
        let stream = Arc::new(Mutex::new(AudioConstellationV3PcmStream::new(
            config.sample_rate,
        )));
        let stream_reader = Arc::clone(&stream);
        let streaming_output = run_tool_streaming_stdout(
            "ffmpeg",
            ffmpeg.as_ref(),
            [
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
                media_path.as_ref().as_os_str().to_os_string(),
                "-map".into(),
                "0:a:0".into(),
                "-vn".into(),
                "-sn".into(),
                "-dn".into(),
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
        combined_metrics.ffmpeg_invocation_count += 1;
        combined_metrics.ffmpeg_output_pcm_bytes = combined_metrics
            .ffmpeg_output_pcm_bytes
            .saturating_add(streaming_output.stdout_bytes);
        combined_metrics.ffmpeg_input_read_bytes = sum_optional_u64(
            combined_metrics.ffmpeg_input_read_bytes,
            streaming_output.process_io.read_bytes,
        );
        combined_metrics.ffmpeg_input_read_ops = sum_optional_u64(
            combined_metrics.ffmpeg_input_read_ops,
            streaming_output.process_io.read_ops,
        );
        combined_metrics.sampled_window_seek_millis = combined_metrics
            .sampled_window_seek_millis
            .saturating_add(0);
        combined_metrics.sampled_window_decode_millis = combined_metrics
            .sampled_window_decode_millis
            .saturating_add(window_wall);
        combined_metrics.ffmpeg_exit_millis = combined_metrics
            .ffmpeg_exit_millis
            .saturating_add(streaming_output.exit_millis);
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

pub(crate) fn probe_audio_packet_positions_for_sampled_windows(
    ffprobe: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    index_mode: MediaAudioIndexMode,
    ffmpeg_input_read_bytes: Option<u64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<MediaAudioStreamMetrics, MediaFingerprintError> {
    let config = sampled_audio_index_config(index_mode);
    let windows = sampled_audio_windows_v3(duration_seconds, config);
    let started_at = Instant::now();
    let output = run_tool_output_with_metrics(
        "ffprobe",
        ffprobe.as_ref(),
        [
            "-v".into(),
            "error".into(),
            "-select_streams".into(),
            "a:0".into(),
            "-show_entries".into(),
            "format=format_name:stream=index,codec_name,bit_rate,duration,start_time:packet=pts_time,dts_time,duration_time,pos,size".into(),
            "-of".into(),
            "json".into(),
            media_path.as_ref().as_os_str().to_os_string(),
        ],
        cancel_flag,
        FFPROBE_TIMEOUT,
    )?;
    ensure_tool_success("ffprobe", &output.output)?;
    let probe_millis = started_at.elapsed().as_millis();
    let mut metrics = audio_packet_probe_metrics_from_ffprobe_json(
        &output.output.stdout,
        &windows,
        probe_millis,
        ffmpeg_input_read_bytes,
    )?;
    metrics.audio_packet_probe_read_bytes = output.io_metrics.read_bytes;
    Ok(metrics)
}

fn audio_packet_probe_metrics_from_ffprobe_json(
    stdout: &[u8],
    windows: &[(f64, u32)],
    probe_millis: u128,
    ffmpeg_input_read_bytes: Option<u64>,
) -> Result<MediaAudioStreamMetrics, MediaFingerprintError> {
    let value: serde_json::Value = serde_json::from_slice(stdout).map_err(|error| {
        MediaFingerprintError::InvalidToolOutput {
            tool: "ffprobe",
            reason: format!("failed parsing audio packet JSON: {error}"),
        }
    })?;
    let mut metrics = MediaAudioStreamMetrics {
        audio_packet_probe_millis: Some(probe_millis),
        ..MediaAudioStreamMetrics::default()
    };
    metrics.container_format = value
        .get("format")
        .and_then(|format| format.get("format_name"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if let Some(stream) = value
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .and_then(|streams| streams.first())
    {
        metrics.audio_stream_index = json_u64(stream.get("index")).map(|value| value as usize);
        metrics.audio_codec = stream
            .get("codec_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        metrics.audio_bitrate_bps = json_u64(stream.get("bit_rate"));
        metrics.audio_duration_millis =
            json_seconds_millis(stream.get("duration")).and_then(|value| u64::try_from(value).ok());
        metrics.audio_start_time_millis = json_seconds_millis(stream.get("start_time"));
        if let Some(index) = metrics.audio_stream_index {
            metrics.ffmpeg_selected_stream = Some(format!("0:{index}"));
        }
    }

    let mut total_packets = 0usize;
    let mut packets_with_position = 0usize;
    let mut monotonic = true;
    let mut previous_position = None::<u64>;
    let mut total_packet_bytes = 0u64;
    let mut sampled_packets = 0usize;
    let mut sampled_packet_bytes = 0u64;
    let mut ranges = Vec::<(u64, u64)>::new();

    if let Some(packets) = value.get("packets").and_then(serde_json::Value::as_array) {
        for packet in packets {
            total_packets += 1;
            let position = json_u64(packet.get("pos"));
            let size = json_u64(packet.get("size"));
            if let Some(size) = size {
                total_packet_bytes = total_packet_bytes.saturating_add(size);
            }
            if let Some(position) = position {
                packets_with_position += 1;
                if let Some(previous_position) = previous_position
                    && position < previous_position
                {
                    monotonic = false;
                }
                previous_position = Some(position);
            }
            let packet_time = json_seconds(packet.get("pts_time"))
                .or_else(|| json_seconds(packet.get("dts_time")));
            let packet_duration = json_seconds(packet.get("duration_time")).unwrap_or(0.0);
            if let (Some(packet_time), Some(position), Some(size)) = (packet_time, position, size)
                && packet_overlaps_sampled_windows(packet_time, packet_duration, windows)
            {
                sampled_packets += 1;
                sampled_packet_bytes = sampled_packet_bytes.saturating_add(size);
                ranges.push((position, position.saturating_add(size)));
            }
        }
    }

    let coalesced_bytes = coalesced_range_bytes(ranges);
    metrics.audio_packet_positions_available = Some(packets_with_position > 0);
    metrics.audio_packet_position_completeness_per_mille = Some(
        packets_with_position
            .saturating_mul(1000)
            .checked_div(total_packets)
            .unwrap_or(0)
            .min(1000) as u16,
    );
    metrics.audio_packet_positions_monotonic = Some(monotonic);
    metrics.average_audio_packet_size_bytes = if total_packets == 0 {
        None
    } else {
        Some(total_packet_bytes.saturating_div(total_packets as u64))
    };
    metrics.audio_packet_count_in_sampled_windows = Some(sampled_packets);
    metrics.audio_packet_window_compressed_bytes = Some(sampled_packet_bytes);
    metrics.audio_packet_window_coalesced_range_bytes = Some(coalesced_bytes);
    metrics.audio_packet_read_savings_estimate_bytes = ffmpeg_input_read_bytes
        .map(|read_bytes| read_bytes as i128 - coalesced_bytes as i128)
        .and_then(|value| i64::try_from(value).ok());
    Ok(metrics)
}

fn packet_overlaps_sampled_windows(
    packet_time: f64,
    packet_duration: f64,
    windows: &[(f64, u32)],
) -> bool {
    let packet_end = packet_time + packet_duration.max(0.0);
    windows.iter().any(|(start, seconds)| {
        let end = *start + f64::from(*seconds);
        packet_time < end && packet_end >= *start
    })
}

fn coalesced_range_bytes(mut ranges: Vec<(u64, u64)>) -> u64 {
    if ranges.is_empty() {
        return 0;
    }
    ranges.sort_unstable();
    let mut total = 0u64;
    let (mut current_start, mut current_end) = ranges[0];
    for (start, end) in ranges.into_iter().skip(1) {
        if start <= current_end {
            current_end = current_end.max(end);
        } else {
            total = total.saturating_add(current_end.saturating_sub(current_start));
            current_start = start;
            current_end = end;
        }
    }
    total.saturating_add(current_end.saturating_sub(current_start))
}

fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    match value? {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn json_seconds(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn json_seconds_millis(value: Option<&serde_json::Value>) -> Option<i64> {
    json_seconds(value).map(|seconds| (seconds * 1000.0).round() as i64)
}

fn merge_audio_stream_metrics(
    target: &mut MediaAudioStreamMetrics,
    source: &MediaAudioStreamMetrics,
) {
    if target.source_path_root.is_none() {
        target.source_path_root = source.source_path_root.clone();
    }
    if target.source_path_kind.is_none() {
        target.source_path_kind = source.source_path_kind.clone();
    }
    if target.source_volume_id.is_none() {
        target.source_volume_id = source.source_volume_id.clone();
    }
    if target.ffmpeg_command_kind.is_none() {
        target.ffmpeg_command_kind = source.ffmpeg_command_kind.clone();
    }
    if target.ffmpeg_selected_stream.is_none() {
        target.ffmpeg_selected_stream = source.ffmpeg_selected_stream.clone();
    }
    target.ffmpeg_disabled_video |= source.ffmpeg_disabled_video;
    target.ffmpeg_disabled_subtitles |= source.ffmpeg_disabled_subtitles;
    target.ffmpeg_disabled_data |= source.ffmpeg_disabled_data;
    if target.container_format.is_none() {
        target.container_format = source.container_format.clone();
    }
    if target.audio_stream_index.is_none() {
        target.audio_stream_index = source.audio_stream_index;
    }
    if target.audio_codec.is_none() {
        target.audio_codec = source.audio_codec.clone();
    }
    if target.audio_bitrate_bps.is_none() {
        target.audio_bitrate_bps = source.audio_bitrate_bps;
    }
    if target.audio_duration_millis.is_none() {
        target.audio_duration_millis = source.audio_duration_millis;
    }
    if target.audio_start_time_millis.is_none() {
        target.audio_start_time_millis = source.audio_start_time_millis;
    }
    if target.audio_packet_positions_available.is_none() {
        target.audio_packet_positions_available = source.audio_packet_positions_available;
    }
    if target
        .audio_packet_position_completeness_per_mille
        .is_none()
    {
        target.audio_packet_position_completeness_per_mille =
            source.audio_packet_position_completeness_per_mille;
    }
    if target.audio_packet_positions_monotonic.is_none() {
        target.audio_packet_positions_monotonic = source.audio_packet_positions_monotonic;
    }
    if target.average_audio_packet_size_bytes.is_none() {
        target.average_audio_packet_size_bytes = source.average_audio_packet_size_bytes;
    }
    if target.audio_packet_count_in_sampled_windows.is_none() {
        target.audio_packet_count_in_sampled_windows = source.audio_packet_count_in_sampled_windows;
    }
    if target.audio_packet_probe_millis.is_none() {
        target.audio_packet_probe_millis = source.audio_packet_probe_millis;
    }
    if target.audio_packet_probe_read_bytes.is_none() {
        target.audio_packet_probe_read_bytes = source.audio_packet_probe_read_bytes;
    }
    if target.audio_packet_window_compressed_bytes.is_none() {
        target.audio_packet_window_compressed_bytes = source.audio_packet_window_compressed_bytes;
    }
    if target.audio_packet_window_coalesced_range_bytes.is_none() {
        target.audio_packet_window_coalesced_range_bytes =
            source.audio_packet_window_coalesced_range_bytes;
    }
    if target.audio_packet_read_savings_estimate_bytes.is_none() {
        target.audio_packet_read_savings_estimate_bytes =
            source.audio_packet_read_savings_estimate_bytes;
    }
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
    target.pcm_drain_thread_millis = target
        .pcm_drain_thread_millis
        .saturating_add(source.pcm_drain_thread_millis);
    target.analyzer_thread_millis = target
        .analyzer_thread_millis
        .saturating_add(source.analyzer_thread_millis);
    target.channel_backpressure_millis = target
        .channel_backpressure_millis
        .saturating_add(source.channel_backpressure_millis);
    target.max_queued_pcm_bytes = target.max_queued_pcm_bytes.max(source.max_queued_pcm_bytes);
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
    target.ffmpeg_input_read_bytes = sum_optional_u64(
        target.ffmpeg_input_read_bytes,
        source.ffmpeg_input_read_bytes,
    );
    target.ffmpeg_input_read_ops =
        sum_optional_u64(target.ffmpeg_input_read_ops, source.ffmpeg_input_read_ops);
    target.ffmpeg_output_pcm_bytes = target
        .ffmpeg_output_pcm_bytes
        .saturating_add(source.ffmpeg_output_pcm_bytes);
    target.ffmpeg_invocation_count = target
        .ffmpeg_invocation_count
        .saturating_add(source.ffmpeg_invocation_count);
    target.sampled_window_seek_millis = target
        .sampled_window_seek_millis
        .saturating_add(source.sampled_window_seek_millis);
    target.sampled_window_decode_millis = target
        .sampled_window_decode_millis
        .saturating_add(source.sampled_window_decode_millis);
    target.ffmpeg_open_probe_millis = target
        .ffmpeg_open_probe_millis
        .saturating_add(source.ffmpeg_open_probe_millis);
    target.ffmpeg_exit_millis = target
        .ffmpeg_exit_millis
        .saturating_add(source.ffmpeg_exit_millis);
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
            "-threads".into(),
            "1".into(),
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
    run_tool_output_with_metrics(tool, executable, args, cancel_flag, timeout)
        .map(|output| output.output)
}

#[derive(Debug)]
struct MediaToolCapturedOutput {
    output: Output,
    io_metrics: MediaToolProcessIoMetrics,
}

fn run_tool_output_with_metrics<I>(
    tool: &'static str,
    executable: &Path,
    args: I,
    cancel_flag: Option<&AtomicBool>,
    timeout: Duration,
) -> Result<MediaToolCapturedOutput, MediaFingerprintError>
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
                let io_metrics = process_io_counters(&child);
                let stdout = join_pipe_reader(stdout_reader, tool, "stdout")?;
                let stderr = join_pipe_reader(stderr_reader, tool, "stderr")?;
                return Ok(MediaToolCapturedOutput {
                    output: Output {
                        status,
                        stdout,
                        stderr,
                    },
                    io_metrics,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_packet_probe_metrics_reports_sampled_window_ranges() {
        let json = br#"{
          "format": { "format_name": "matroska,webm" },
          "streams": [
            {
              "index": 1,
              "codec_name": "flac",
              "bit_rate": "512000",
              "duration": "120.000",
              "start_time": "0.000"
            }
          ],
          "packets": [
            { "pts_time": "0.000", "duration_time": "1.000", "pos": "100", "size": "10" },
            { "pts_time": "10.000", "duration_time": "1.000", "pos": "200", "size": "20" },
            { "pts_time": "10.500", "duration_time": "1.000", "pos": "220", "size": "30" },
            { "pts_time": "60.000", "duration_time": "1.000", "pos": "500", "size": "40" }
          ]
        }"#;

        let metrics =
            audio_packet_probe_metrics_from_ffprobe_json(json, &[(10.0, 2)], 7, Some(1_000))
                .expect("packet probe JSON should parse");

        assert_eq!(metrics.container_format.as_deref(), Some("matroska,webm"));
        assert_eq!(metrics.audio_stream_index, Some(1));
        assert_eq!(metrics.ffmpeg_selected_stream.as_deref(), Some("0:1"));
        assert_eq!(metrics.audio_codec.as_deref(), Some("flac"));
        assert_eq!(metrics.audio_bitrate_bps, Some(512_000));
        assert_eq!(metrics.audio_duration_millis, Some(120_000));
        assert_eq!(metrics.audio_start_time_millis, Some(0));
        assert_eq!(metrics.audio_packet_positions_available, Some(true));
        assert_eq!(
            metrics.audio_packet_position_completeness_per_mille,
            Some(1000)
        );
        assert_eq!(metrics.audio_packet_positions_monotonic, Some(true));
        assert_eq!(metrics.average_audio_packet_size_bytes, Some(25));
        assert_eq!(metrics.audio_packet_count_in_sampled_windows, Some(2));
        assert_eq!(metrics.audio_packet_probe_millis, Some(7));
        assert_eq!(metrics.audio_packet_window_compressed_bytes, Some(50));
        assert_eq!(metrics.audio_packet_window_coalesced_range_bytes, Some(50));
        assert_eq!(metrics.audio_packet_read_savings_estimate_bytes, Some(950));
    }
}
