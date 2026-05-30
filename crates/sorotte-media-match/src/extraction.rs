use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
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

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
        MediaFingerprintProfile, MediaSampledAudioSourceStrategy, media_extraction_settings_hash,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPacketMapV3 {
    pub source_identity: MediaFileIdentity,
    pub container_format: String,
    pub audio_stream_index: u32,
    pub audio_codec: String,
    pub time_base_num: i32,
    pub time_base_den: i32,
    pub packets: Vec<AudioPacketPositionV3>,
    pub complete: bool,
}

impl AudioPacketMapV3 {
    pub fn valid_for(
        &self,
        normalized_path: &str,
        modified_unix_millis: u64,
        size_bytes: u64,
        audio_stream_index: u32,
        container_format: &str,
        audio_codec: &str,
    ) -> bool {
        self.source_identity
            .valid_for(normalized_path, modified_unix_millis, size_bytes)
            && self.audio_stream_index == audio_stream_index
            && self.container_format == container_format
            && self.audio_codec == audio_codec
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPacketPositionV3 {
    pub pts_ms: i64,
    pub duration_ms: i64,
    pub file_pos: u64,
    pub size_bytes: u32,
    pub key: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MediaFingerprintExtractionOptions {
    pub sampled_audio_source: MediaSampledAudioSourceStrategy,
    pub sampled_pcm_cache_root: Option<PathBuf>,
    pub adaptive_sampled_fast: bool,
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

#[derive(Debug, Clone, PartialEq, Default)]
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
    pub selected_sampled_audio_source_strategy: Option<String>,
    pub source_strategy_decision_reason: Option<String>,
    pub source_strategy_fallback_count: u32,
    pub audio_packet_map_cache_hit: Option<bool>,
    pub audio_packet_map_build_millis: Option<u128>,
    pub audio_packet_map_packet_count: Option<usize>,
    pub audio_packet_map_bytes: Option<u64>,
    pub audio_packet_map_complete: Option<bool>,
    pub audio_packet_map_fallback_reason: Option<String>,
    pub audio_packet_window_count: Option<usize>,
    pub audio_packet_ranges: Option<usize>,
    pub audio_packet_range_bytes: Option<u64>,
    pub audio_packet_coalesced_range_bytes: Option<u64>,
    pub audio_packet_range_read_millis: Option<u128>,
    pub audio_packet_range_read_ops: Option<u64>,
    pub audio_packet_read_amplification_vs_pcm: Option<f64>,
    pub audio_packet_estimated_savings_vs_current: Option<i64>,
    pub sampled_pcm_cache_hit: Option<bool>,
    pub sampled_pcm_cache_bytes: Option<u64>,
    pub sampled_pcm_cache_read_millis: Option<u128>,
    pub sampled_pcm_cache_write_millis: Option<u128>,
    pub sampled_pcm_cache_saved_millis: Option<i64>,
    pub audio_sidecar_mode: Option<String>,
    pub audio_sidecar_fallback_reason: Option<String>,
    pub sampled_ffmpeg_window_strategy: Option<String>,
    pub sampled_windows_planned: Option<usize>,
    pub sampled_stop_reason: Option<String>,
    pub provisional_landmark_count: Option<usize>,
    pub provisional_body_region_count: Option<usize>,
    pub adaptive_saved_seconds: Option<u32>,
    pub adaptive_saved_estimated_read_bytes: Option<u64>,
    pub mkv_parser_used: Option<bool>,
    pub mkv_cues_present: Option<bool>,
    pub mkv_audio_track_found: Option<bool>,
    pub mkv_clusters_scanned: Option<usize>,
    pub mkv_cluster_bytes_read: Option<u64>,
    pub mkv_audio_block_bytes_read: Option<u64>,
    pub mkv_coalesced_range_bytes: Option<u64>,
    pub mkv_estimated_savings_vs_current: Option<i64>,
    pub mkv_fallback_reason: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Default)]
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
    fingerprint_media_file_with_report_and_options(
        path,
        tools,
        extraction_settings,
        cancel_flag,
        &MediaFingerprintExtractionOptions::default(),
    )
}

pub fn fingerprint_media_file_with_report_and_options(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
    options: &MediaFingerprintExtractionOptions,
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
    let source_identity = MediaFileIdentity {
        normalized_path: normalized_path.clone(),
        modified_unix_millis,
        size_bytes,
    };
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
            extract_audio_constellation_v3_sampled_index_with_metrics_and_options(
                &tools.ffmpeg,
                path,
                duration_seconds,
                extraction_settings.audio_index_mode,
                SampledAudioExtractionContext {
                    source_identity: &source_identity,
                    settings_hash: media_extraction_settings_hash(extraction_settings),
                    options,
                    ffprobe: Some(tools.ffprobe.as_path()),
                },
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

#[derive(Clone, Copy)]
struct SampledAudioExtractionContext<'a> {
    source_identity: &'a MediaFileIdentity,
    settings_hash: [u8; 32],
    options: &'a MediaFingerprintExtractionOptions,
    ffprobe: Option<&'a Path>,
}

fn extract_audio_constellation_v3_sampled_index_with_metrics_and_options(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    index_mode: MediaAudioIndexMode,
    context: SampledAudioExtractionContext<'_>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut config = sampled_audio_index_config(index_mode);
    if context.options.adaptive_sampled_fast && index_mode == MediaAudioIndexMode::SampledFast {
        config.min_windows = config.min_windows.clamp(1, 2);
        config.min_body_regions = config.min_body_regions.min(config.min_windows);
    }
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
    combined_metrics.selected_sampled_audio_source_strategy =
        Some(context.options.sampled_audio_source.label().to_owned());
    combined_metrics.sampled_ffmpeg_window_strategy =
        Some(sampled_ffmpeg_window_strategy_label(context.options.sampled_audio_source).to_owned());
    combined_metrics.sampled_windows_planned = Some(windows.len());
    combined_metrics.source_strategy_decision_reason = Some(
        match context.options.sampled_audio_source {
            MediaSampledAudioSourceStrategy::Current => "explicit-current",
            MediaSampledAudioSourceStrategy::SingleProcessFilter => {
                "explicit-single-process-filter"
            }
            MediaSampledAudioSourceStrategy::FastSeekPerWindow => "explicit-fast-seek-per-window",
            MediaSampledAudioSourceStrategy::OutputSeekPerWindow => {
                "explicit-output-seek-per-window"
            }
            MediaSampledAudioSourceStrategy::FfprobeProbe => "explicit-ffprobe-probe",
            MediaSampledAudioSourceStrategy::PacketMap => "explicit-packet-map-feasibility",
            MediaSampledAudioSourceStrategy::MkvAudioRanges => "explicit-mkv-audio-ranges",
            MediaSampledAudioSourceStrategy::SampledPcmCache => "explicit-sampled-pcm-cache",
            MediaSampledAudioSourceStrategy::Auto => "auto-safe-current-with-cache-probe",
        }
        .to_owned(),
    );

    maybe_attach_packet_map_feasibility(
        media_path.as_ref(),
        &mut combined_metrics,
        &windows,
        context,
    );
    maybe_attach_mkv_audio_range_feasibility(media_path.as_ref(), &mut combined_metrics, &windows);

    if let Some((landmarks, metrics)) = try_sampled_pcm_cache_read(
        media_path.as_ref(),
        duration_seconds,
        index_mode,
        config,
        &windows,
        context,
        &combined_metrics,
    )? {
        return Ok((landmarks, metrics));
    }

    if sampled_pcm_cache_enabled(context) {
        return extract_sampled_index_with_pcm_cache_fill(
            ffmpeg,
            media_path,
            duration_seconds,
            index_mode,
            config,
            context,
            combined_metrics,
            cancel_flag,
        );
    }

    if context.options.sampled_audio_source == MediaSampledAudioSourceStrategy::SingleProcessFilter
    {
        return extract_sampled_index_with_single_process_filter(
            ffmpeg,
            media_path,
            duration_seconds,
            config,
            windows,
            combined_metrics,
            cancel_flag,
        );
    }

    let mut process_wall_millis = 0u128;
    let mut body_regions = BTreeSet::new();
    let mut unique_hashes = BTreeSet::new();
    let mut stop_reason = "max-windows";
    for (window_index, (start_seconds, window_seconds)) in windows.iter().copied().enumerate() {
        let seek_mode = match context.options.sampled_audio_source {
            MediaSampledAudioSourceStrategy::OutputSeekPerWindow => {
                FfmpegSampledWindowSeekMode::Output
            }
            _ => FfmpegSampledWindowSeekMode::Input,
        };
        let (window_pcm, streaming_output, window_wall) =
            decode_sampled_window_pcm_bytes_with_seek_mode(
                ffmpeg.as_ref(),
                media_path.as_ref(),
                start_seconds,
                window_seconds,
                config.sample_rate,
                seek_mode,
                cancel_flag,
            )?;
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
        let (mut landmarks, metrics) =
            analyze_sampled_window_pcm_bytes(&window_pcm, window_seconds, config.sample_rate)?;
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
        combined_metrics.provisional_landmark_count = Some(all_landmarks.len());
        combined_metrics.provisional_body_region_count = Some(body_regions.len());
        if windows_decoded >= config.min_windows
            && all_landmarks.len() >= config.target_landmarks
            && body_regions.len() >= config.min_body_regions
            && unique_hashes.len() >= config.target_landmarks.saturating_mul(3) / 4
        {
            stop_reason = if context.options.adaptive_sampled_fast
                && index_mode == MediaAudioIndexMode::SampledFast
                && windows_decoded < windows.len()
            {
                "adaptive-quality-threshold"
            } else {
                "quality-threshold"
            };
            break;
        }
    }
    combined_metrics.sampled_stop_reason = Some(stop_reason.to_owned());
    combined_metrics.adaptive_saved_seconds = Some(
        windows
            .len()
            .saturating_sub(combined_metrics.sampled_audio_windows_decoded) as u32
            * config.window_seconds,
    );
    combined_metrics.adaptive_saved_estimated_read_bytes =
        estimated_saved_read_bytes(&combined_metrics);

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
    update_mkv_estimated_savings(&mut combined_metrics);
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((bounded, combined_metrics))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SampledPcmCacheManifest {
    source_identity: MediaFileIdentity,
    settings_hash: String,
    audio_index_mode: MediaAudioIndexMode,
    sample_rate: u32,
    windows: Vec<SampledPcmCacheWindow>,
    pcm_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SampledPcmCacheWindow {
    start_ms: u32,
    seconds: u32,
    byte_offset: u64,
    byte_len: u64,
}

fn sampled_pcm_cache_enabled(context: SampledAudioExtractionContext<'_>) -> bool {
    context.options.sampled_pcm_cache_root.is_some()
        && matches!(
            context.options.sampled_audio_source,
            MediaSampledAudioSourceStrategy::SampledPcmCache
                | MediaSampledAudioSourceStrategy::Auto
        )
}

fn sampled_ffmpeg_window_strategy_label(strategy: MediaSampledAudioSourceStrategy) -> &'static str {
    match strategy {
        MediaSampledAudioSourceStrategy::Current
        | MediaSampledAudioSourceStrategy::FfprobeProbe
        | MediaSampledAudioSourceStrategy::PacketMap
        | MediaSampledAudioSourceStrategy::MkvAudioRanges
        | MediaSampledAudioSourceStrategy::SampledPcmCache
        | MediaSampledAudioSourceStrategy::Auto => "current-three-invocations",
        MediaSampledAudioSourceStrategy::SingleProcessFilter => "single-process-filter",
        MediaSampledAudioSourceStrategy::FastSeekPerWindow => "fast-seek-per-window",
        MediaSampledAudioSourceStrategy::OutputSeekPerWindow => "output-seek-per-window",
    }
}

fn estimated_saved_read_bytes(metrics: &MediaAudioStreamMetrics) -> Option<u64> {
    let decoded = u64::from(metrics.sampled_audio_seconds_decoded);
    let saved = u64::from(metrics.adaptive_saved_seconds?);
    if decoded == 0 || saved == 0 {
        return Some(0);
    }
    metrics
        .ffmpeg_input_read_bytes
        .map(|read_bytes| read_bytes.saturating_mul(saved) / decoded)
}

fn update_mkv_estimated_savings(metrics: &mut MediaAudioStreamMetrics) {
    if metrics.mkv_estimated_savings_vs_current.is_some() {
        return;
    }
    if let (Some(read_bytes), Some(range_bytes)) = (
        metrics.ffmpeg_input_read_bytes,
        metrics.mkv_coalesced_range_bytes,
    ) {
        metrics.mkv_estimated_savings_vs_current =
            i64::try_from(read_bytes as i128 - range_bytes as i128).ok();
    }
}

fn sampled_audio_cache_key(
    source_identity: &MediaFileIdentity,
    settings_hash: [u8; 32],
    index_mode: MediaAudioIndexMode,
    config: SampledAudioIndexConfig,
    windows: &[(f64, u32)],
    adaptive_sampled_fast: bool,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_identity.normalized_path.as_bytes());
    hasher.update(source_identity.modified_unix_millis.to_le_bytes());
    hasher.update(source_identity.size_bytes.to_le_bytes());
    hasher.update(settings_hash);
    hasher.update(index_mode.label().as_bytes());
    hasher.update(config.sample_rate.to_le_bytes());
    hasher.update(config.window_seconds.to_le_bytes());
    hasher.update([u8::from(adaptive_sampled_fast)]);
    hasher.update((windows.len() as u64).to_le_bytes());
    for (start, seconds) in windows {
        hasher.update(start.to_le_bytes());
        hasher.update(seconds.to_le_bytes());
    }
    lower_hex(&hasher.finalize())
}

fn sampled_pcm_cache_paths(
    context: SampledAudioExtractionContext<'_>,
    index_mode: MediaAudioIndexMode,
    config: SampledAudioIndexConfig,
    windows: &[(f64, u32)],
) -> Option<(PathBuf, PathBuf, String)> {
    let root = context.options.sampled_pcm_cache_root.as_ref()?;
    let key = sampled_audio_cache_key(
        context.source_identity,
        context.settings_hash,
        index_mode,
        config,
        windows,
        context.options.adaptive_sampled_fast,
    );
    let dir = root.join("sampled-pcm-v3");
    Some((
        dir.join(format!("{key}.json")),
        dir.join(format!("{key}.s16le")),
        key,
    ))
}

fn try_sampled_pcm_cache_read(
    media_path: &Path,
    duration_seconds: Option<f64>,
    index_mode: MediaAudioIndexMode,
    config: SampledAudioIndexConfig,
    windows: &[(f64, u32)],
    context: SampledAudioExtractionContext<'_>,
    base_metrics: &MediaAudioStreamMetrics,
) -> Result<Option<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics)>, MediaFingerprintError> {
    if !sampled_pcm_cache_enabled(context) {
        return Ok(None);
    }
    let Some((manifest_path, pcm_path, _key)) =
        sampled_pcm_cache_paths(context, index_mode, config, windows)
    else {
        return Ok(None);
    };
    if !manifest_path.is_file() || !pcm_path.is_file() {
        return Ok(None);
    }
    let read_started_at = Instant::now();
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|error| MediaFingerprintError::FileMetadata {
            path: manifest_path.display().to_string(),
            error: error.to_string(),
        })?;
    let manifest: SampledPcmCacheManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            MediaFingerprintError::InvalidToolOutput {
                tool: "sampled-pcm-cache",
                reason: format!("failed parsing sampled PCM cache manifest: {error}"),
            }
        })?;
    let settings_hash = lower_hex(&context.settings_hash);
    if !manifest.source_identity.valid_for(
        &context.source_identity.normalized_path,
        context.source_identity.modified_unix_millis,
        context.source_identity.size_bytes,
    ) || manifest.settings_hash != settings_hash
        || manifest.audio_index_mode != index_mode
        || manifest.sample_rate != config.sample_rate
    {
        return Ok(None);
    }
    let pcm = fs::read(&pcm_path).map_err(|error| MediaFingerprintError::FileMetadata {
        path: pcm_path.display().to_string(),
        error: error.to_string(),
    })?;
    let read_millis = read_started_at.elapsed().as_millis();
    let (landmarks, mut metrics) = analyze_sampled_pcm_cache_windows(
        &pcm,
        &manifest.windows,
        config,
        duration_seconds,
        base_metrics.clone(),
    )?;
    metrics.selected_sampled_audio_source_strategy = Some("sampled-pcm-cache".to_owned());
    metrics.source_strategy_decision_reason = Some(
        if context.options.sampled_audio_source == MediaSampledAudioSourceStrategy::Auto {
            "auto-cache-hit"
        } else {
            "explicit-cache-hit"
        }
        .to_owned(),
    );
    metrics.sampled_pcm_cache_hit = Some(true);
    metrics.sampled_pcm_cache_bytes = Some(pcm.len() as u64);
    metrics.sampled_pcm_cache_read_millis = Some(read_millis);
    metrics.sampled_pcm_cache_saved_millis = Some(0);
    metrics.audio_sidecar_mode = Some("sampled-pcm-cache".to_owned());
    metrics.sampled_windows_planned = Some(windows.len());
    metrics.sampled_stop_reason = Some("sampled-pcm-cache-hit".to_owned());
    metrics.provisional_landmark_count = Some(landmarks.len());
    let _ = media_path;
    Ok(Some((landmarks, metrics)))
}

#[allow(clippy::too_many_arguments)]
fn extract_sampled_index_with_pcm_cache_fill(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    index_mode: MediaAudioIndexMode,
    config: SampledAudioIndexConfig,
    context: SampledAudioExtractionContext<'_>,
    mut combined_metrics: MediaAudioStreamMetrics,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let windows = sampled_audio_windows_v3(duration_seconds, config);
    let mut all_landmarks = Vec::new();
    let mut body_regions = BTreeSet::new();
    let mut unique_hashes = BTreeSet::new();
    let mut process_wall_millis = 0u128;
    let mut pcm_bytes = Vec::<u8>::new();
    let mut manifest_windows = Vec::<SampledPcmCacheWindow>::new();
    combined_metrics.sampled_windows_planned = Some(windows.len());
    let mut stop_reason = "max-windows";
    for (window_index, (start_seconds, window_seconds)) in windows.iter().copied().enumerate() {
        let (window_pcm, streaming_output, window_wall) = decode_sampled_window_pcm_bytes(
            ffmpeg.as_ref(),
            media_path.as_ref(),
            start_seconds,
            window_seconds,
            config.sample_rate,
            cancel_flag,
        )?;
        let byte_offset = pcm_bytes.len() as u64;
        let byte_len = window_pcm.len() as u64;
        pcm_bytes.extend_from_slice(&window_pcm);
        manifest_windows.push(SampledPcmCacheWindow {
            start_ms: seconds_to_u32_millis(start_seconds),
            seconds: window_seconds,
            byte_offset,
            byte_len,
        });
        process_wall_millis = process_wall_millis.saturating_add(window_wall);
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
        combined_metrics.sampled_window_decode_millis = combined_metrics
            .sampled_window_decode_millis
            .saturating_add(window_wall);
        combined_metrics.ffmpeg_exit_millis = combined_metrics
            .ffmpeg_exit_millis
            .saturating_add(streaming_output.exit_millis);
        let (mut landmarks, metrics) =
            analyze_sampled_window_pcm_bytes(&window_pcm, window_seconds, config.sample_rate)?;
        let start_ms = seconds_to_u32_millis(start_seconds);
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
        combined_metrics.provisional_landmark_count = Some(all_landmarks.len());
        combined_metrics.provisional_body_region_count = Some(body_regions.len());
        if windows_decoded >= config.min_windows
            && all_landmarks.len() >= config.target_landmarks
            && body_regions.len() >= config.min_body_regions
            && unique_hashes.len() >= config.target_landmarks.saturating_mul(3) / 4
        {
            stop_reason = if context.options.adaptive_sampled_fast
                && index_mode == MediaAudioIndexMode::SampledFast
                && windows_decoded < windows.len()
            {
                "adaptive-quality-threshold"
            } else {
                "quality-threshold"
            };
            break;
        }
    }
    combined_metrics.sampled_stop_reason = Some(stop_reason.to_owned());
    combined_metrics.adaptive_saved_seconds = Some(
        windows
            .len()
            .saturating_sub(combined_metrics.sampled_audio_windows_decoded) as u32
            * config.window_seconds,
    );
    combined_metrics.adaptive_saved_estimated_read_bytes =
        estimated_saved_read_bytes(&combined_metrics);

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
    combined_metrics.sampled_pcm_cache_hit = Some(false);
    combined_metrics.sampled_pcm_cache_bytes = Some(pcm_bytes.len() as u64);
    combined_metrics.audio_sidecar_mode = Some("sampled-pcm-cache".to_owned());
    write_sampled_pcm_cache(
        context,
        index_mode,
        config,
        &windows,
        &manifest_windows,
        &pcm_bytes,
        &mut combined_metrics,
    )?;
    update_mkv_estimated_savings(&mut combined_metrics);
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((bounded, combined_metrics))
}

fn decode_sampled_window_pcm_bytes(
    ffmpeg: &Path,
    media_path: &Path,
    start_seconds: f64,
    window_seconds: u32,
    sample_rate: u32,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<u8>, MediaToolStreamingOutput, u128), MediaFingerprintError> {
    decode_sampled_window_pcm_bytes_with_seek_mode(
        ffmpeg,
        media_path,
        start_seconds,
        window_seconds,
        sample_rate,
        FfmpegSampledWindowSeekMode::Input,
        cancel_flag,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FfmpegSampledWindowSeekMode {
    Input,
    Output,
}

fn decode_sampled_window_pcm_bytes_with_seek_mode(
    ffmpeg: &Path,
    media_path: &Path,
    start_seconds: f64,
    window_seconds: u32,
    sample_rate: u32,
    seek_mode: FfmpegSampledWindowSeekMode,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<u8>, MediaToolStreamingOutput, u128), MediaFingerprintError> {
    let started_at = Instant::now();
    let pcm = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pcm_writer = Arc::clone(&pcm);
    let mut args = vec![
        "-v".into(),
        "error".into(),
        "-nostdin".into(),
        "-threads".into(),
        "1".into(),
    ];
    if seek_mode == FfmpegSampledWindowSeekMode::Input {
        args.extend([
            "-ss".into(),
            format!("{start_seconds:.3}").into(),
            "-t".into(),
            window_seconds.to_string().into(),
        ]);
    }
    args.extend(["-i".into(), media_path.as_os_str().to_os_string()]);
    if seek_mode == FfmpegSampledWindowSeekMode::Output {
        args.extend([
            "-ss".into(),
            format!("{start_seconds:.3}").into(),
            "-t".into(),
            window_seconds.to_string().into(),
        ]);
    }
    args.extend([
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
    ]);
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
                    reason: "sampled PCM cache buffer was poisoned".to_owned(),
                })?
                .extend_from_slice(chunk);
            Ok(())
        },
    )?;
    let pcm = Arc::try_unwrap(pcm)
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled PCM cache buffer was still shared".to_owned(),
        })?
        .into_inner()
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "sampled PCM cache buffer was poisoned".to_owned(),
        })?;
    Ok((pcm, streaming_output, started_at.elapsed().as_millis()))
}

fn extract_sampled_index_with_single_process_filter(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    config: SampledAudioIndexConfig,
    windows: Vec<(f64, u32)>,
    mut combined_metrics: MediaAudioStreamMetrics,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let started_at = Instant::now();
    let pcm = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pcm_writer = Arc::clone(&pcm);
    let filter = sampled_windows_filter_complex(&windows);
    let streaming_output = run_tool_streaming_stdout(
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
            "-filter_complex".into(),
            filter.into(),
            "-map".into(),
            "[out]".into(),
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
            pcm_writer
                .lock()
                .map_err(|_| MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason: "single-process sampled PCM buffer was poisoned".to_owned(),
                })?
                .extend_from_slice(chunk);
            Ok(())
        },
    )?;
    let wall = started_at.elapsed().as_millis();
    let pcm = Arc::try_unwrap(pcm)
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "single-process sampled PCM buffer was still shared".to_owned(),
        })?
        .into_inner()
        .map_err(|_| MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "single-process sampled PCM buffer was poisoned".to_owned(),
        })?;
    combined_metrics.ffmpeg_invocation_count += 1;
    combined_metrics.ffmpeg_output_pcm_bytes = streaming_output.stdout_bytes;
    combined_metrics.ffmpeg_input_read_bytes = streaming_output.process_io.read_bytes;
    combined_metrics.ffmpeg_input_read_ops = streaming_output.process_io.read_ops;
    combined_metrics.sampled_window_decode_millis = combined_metrics
        .sampled_window_decode_millis
        .saturating_add(wall);
    combined_metrics.ffmpeg_exit_millis = combined_metrics
        .ffmpeg_exit_millis
        .saturating_add(streaming_output.exit_millis);

    let mut all_landmarks = Vec::new();
    let mut body_regions = BTreeSet::new();
    let bytes_per_second = config.sample_rate as usize * 2;
    let mut byte_offset = 0usize;
    for (start_seconds, window_seconds) in &windows {
        let byte_len = *window_seconds as usize * bytes_per_second;
        let byte_end = byte_offset.saturating_add(byte_len).min(pcm.len());
        if byte_offset >= byte_end {
            break;
        }
        let (mut landmarks, metrics) = analyze_sampled_window_pcm_bytes(
            &pcm[byte_offset..byte_end],
            *window_seconds,
            config.sample_rate,
        )?;
        let start_ms = seconds_to_u32_millis(*start_seconds);
        for landmark in &mut landmarks {
            landmark.t_ms = landmark.t_ms.saturating_add(start_ms);
        }
        for landmark in &landmarks {
            body_regions.insert(landmark.t_ms / 60_000);
        }
        all_landmarks.extend(landmarks);
        merge_audio_stream_metrics(&mut combined_metrics, &metrics);
        combined_metrics.sampled_audio_seconds_decoded = combined_metrics
            .sampled_audio_seconds_decoded
            .saturating_add(*window_seconds);
        combined_metrics.sampled_audio_windows_decoded += 1;
        byte_offset = byte_offset.saturating_add(byte_len);
    }

    let selection_started_at = Instant::now();
    let raw_before_bounding = all_landmarks.len();
    let bounded = bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut all_landmarks,
        config.max_landmarks,
        duration_seconds,
    );
    combined_metrics.final_selection_millis = combined_metrics
        .final_selection_millis
        .saturating_add(selection_started_at.elapsed().as_millis());
    combined_metrics.final_landmarks = bounded.len();
    combined_metrics.raw_landmarks_before_bounding = raw_before_bounding;
    combined_metrics.ffmpeg_process_wall_millis = wall;
    combined_metrics.pcm_decode_drain_millis = wall;
    combined_metrics.ffmpeg_decode_stream_millis = wall;
    update_mkv_estimated_savings(&mut combined_metrics);
    combined_metrics.provisional_landmark_count = Some(raw_before_bounding);
    combined_metrics.provisional_body_region_count = Some(body_regions.len());
    combined_metrics.sampled_stop_reason = Some("single-process-filter-all-windows".to_owned());
    combined_metrics.adaptive_saved_seconds = Some(0);
    combined_metrics.adaptive_saved_estimated_read_bytes = Some(0);
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "single-process sampled decoded audio did not produce landmarks".to_owned(),
        });
    }
    Ok((bounded, combined_metrics))
}

fn sampled_windows_filter_complex(windows: &[(f64, u32)]) -> String {
    let mut parts = Vec::new();
    let mut labels = Vec::new();
    for (index, (start, seconds)) in windows.iter().enumerate() {
        let label = format!("a{index}");
        labels.push(format!("[{label}]"));
        parts.push(format!(
            "[0:a:0]atrim=start={start:.3}:duration={seconds},asetpts=PTS-STARTPTS[{label}]"
        ));
    }
    parts.push(format!(
        "{}concat=n={}:v=0:a=1[out]",
        labels.join(""),
        labels.len()
    ));
    parts.join(";")
}

fn analyze_sampled_window_pcm_bytes(
    pcm: &[u8],
    window_seconds: u32,
    sample_rate: u32,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut stream = AudioConstellationV3PcmStream::new(sample_rate);
    stream.push_bytes(pcm)?;
    stream.finish(Some(f64::from(window_seconds)))
}

fn analyze_sampled_pcm_cache_windows(
    pcm: &[u8],
    windows: &[SampledPcmCacheWindow],
    config: SampledAudioIndexConfig,
    duration_seconds: Option<f64>,
    mut combined_metrics: MediaAudioStreamMetrics,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut all_landmarks = Vec::new();
    for window in windows {
        let start = usize::try_from(window.byte_offset).unwrap_or(usize::MAX);
        let len = usize::try_from(window.byte_len).unwrap_or(usize::MAX);
        let end = start.saturating_add(len);
        if start > pcm.len() || end > pcm.len() {
            return Err(MediaFingerprintError::InvalidToolOutput {
                tool: "sampled-pcm-cache",
                reason: "sampled PCM cache window exceeds PCM blob".to_owned(),
            });
        }
        let (mut landmarks, metrics) =
            analyze_sampled_window_pcm_bytes(&pcm[start..end], window.seconds, config.sample_rate)?;
        for landmark in &mut landmarks {
            landmark.t_ms = landmark.t_ms.saturating_add(window.start_ms);
        }
        all_landmarks.extend(landmarks);
        merge_audio_stream_metrics(&mut combined_metrics, &metrics);
        combined_metrics.sampled_audio_seconds_decoded = combined_metrics
            .sampled_audio_seconds_decoded
            .saturating_add(window.seconds);
        combined_metrics.sampled_audio_windows_decoded += 1;
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
    if bounded.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "sampled-pcm-cache",
            reason: "cached sampled PCM did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((bounded, combined_metrics))
}

fn write_sampled_pcm_cache(
    context: SampledAudioExtractionContext<'_>,
    index_mode: MediaAudioIndexMode,
    config: SampledAudioIndexConfig,
    windows: &[(f64, u32)],
    manifest_windows: &[SampledPcmCacheWindow],
    pcm: &[u8],
    metrics: &mut MediaAudioStreamMetrics,
) -> Result<(), MediaFingerprintError> {
    let Some((manifest_path, pcm_path, _key)) =
        sampled_pcm_cache_paths(context, index_mode, config, windows)
    else {
        return Ok(());
    };
    let started_at = Instant::now();
    let parent = manifest_path
        .parent()
        .ok_or_else(|| MediaFingerprintError::FileMetadata {
            path: manifest_path.display().to_string(),
            error: "sampled PCM cache manifest has no parent".to_owned(),
        })?;
    fs::create_dir_all(parent).map_err(|error| MediaFingerprintError::FileMetadata {
        path: parent.display().to_string(),
        error: error.to_string(),
    })?;
    let manifest = SampledPcmCacheManifest {
        source_identity: context.source_identity.clone(),
        settings_hash: lower_hex(&context.settings_hash),
        audio_index_mode: index_mode,
        sample_rate: config.sample_rate,
        windows: manifest_windows.to_vec(),
        pcm_file: pcm_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "sampled.s16le".to_owned()),
    };
    fs::write(&pcm_path, pcm).map_err(|error| MediaFingerprintError::FileMetadata {
        path: pcm_path.display().to_string(),
        error: error.to_string(),
    })?;
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        MediaFingerprintError::InvalidToolOutput {
            tool: "sampled-pcm-cache",
            reason: format!("failed serializing sampled PCM cache manifest: {error}"),
        }
    })?;
    fs::write(&manifest_path, manifest_json).map_err(|error| {
        MediaFingerprintError::FileMetadata {
            path: manifest_path.display().to_string(),
            error: error.to_string(),
        }
    })?;
    metrics.sampled_pcm_cache_write_millis = Some(started_at.elapsed().as_millis());
    Ok(())
}

fn seconds_to_u32_millis(seconds: f64) -> u32 {
    (seconds * 1000.0).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn maybe_attach_packet_map_feasibility(
    media_path: &Path,
    metrics: &mut MediaAudioStreamMetrics,
    windows: &[(f64, u32)],
    context: SampledAudioExtractionContext<'_>,
) {
    if !matches!(
        context.options.sampled_audio_source,
        MediaSampledAudioSourceStrategy::FfprobeProbe | MediaSampledAudioSourceStrategy::PacketMap
    ) {
        return;
    }
    let Some(ffprobe) = context.ffprobe else {
        metrics.audio_packet_map_fallback_reason = Some("ffprobe-not-configured".to_owned());
        metrics.source_strategy_fallback_count =
            metrics.source_strategy_fallback_count.saturating_add(1);
        return;
    };
    let started_at = Instant::now();
    match load_or_build_audio_packet_map_v3(ffprobe, media_path, windows, context) {
        Ok((map, cache_hit, map_bytes, probe_read_bytes)) => {
            metrics.audio_packet_map_cache_hit = Some(cache_hit);
            metrics.audio_packet_map_build_millis = Some(started_at.elapsed().as_millis());
            metrics.audio_packet_map_packet_count = Some(map.packets.len());
            metrics.audio_packet_map_bytes = Some(map_bytes);
            metrics.audio_packet_map_complete = Some(map.complete);
            metrics.audio_packet_probe_read_bytes = probe_read_bytes;
            metrics.container_format = Some(map.container_format.clone());
            metrics.audio_stream_index = Some(map.audio_stream_index as usize);
            metrics.audio_codec = Some(map.audio_codec.clone());
            metrics.ffmpeg_selected_stream = Some(format!("0:{}", map.audio_stream_index));
            let ranges = packet_ranges_for_windows(&map, windows, 128 * 1024);
            let range_bytes = ranges
                .iter()
                .map(|range| range.1.saturating_sub(range.0))
                .sum::<u64>();
            metrics.audio_packet_window_count = Some(windows.len());
            metrics.audio_packet_ranges = Some(ranges.len());
            metrics.audio_packet_range_bytes = Some(
                map.packets
                    .iter()
                    .filter(|packet| {
                        packet_overlaps_sampled_windows(
                            packet.pts_ms as f64 / 1000.0,
                            packet.duration_ms as f64 / 1000.0,
                            windows,
                        )
                    })
                    .map(|packet| u64::from(packet.size_bytes))
                    .sum(),
            );
            metrics.audio_packet_coalesced_range_bytes = Some(range_bytes);
            if let Some(pcm_bytes) =
                (metrics.ffmpeg_output_pcm_bytes > 0).then_some(metrics.ffmpeg_output_pcm_bytes)
            {
                metrics.audio_packet_read_amplification_vs_pcm =
                    Some(range_bytes as f64 / pcm_bytes as f64);
            }
            metrics.audio_packet_estimated_savings_vs_current = metrics
                .ffmpeg_input_read_bytes
                .map(|read_bytes| read_bytes as i128 - range_bytes as i128)
                .and_then(|value| i64::try_from(value).ok());
            if context.options.sampled_audio_source == MediaSampledAudioSourceStrategy::PacketMap {
                let read_started_at = Instant::now();
                match read_packet_ranges(media_path, &ranges) {
                    Ok((bytes, ops)) => {
                        metrics.audio_packet_range_read_millis =
                            Some(read_started_at.elapsed().as_millis());
                        metrics.audio_packet_range_read_ops = Some(ops);
                        metrics.audio_packet_coalesced_range_bytes = Some(bytes.len() as u64);
                        metrics.audio_sidecar_fallback_reason =
                            Some("packet-map-range-read-feasibility-only".to_owned());
                        metrics.source_strategy_fallback_count =
                            metrics.source_strategy_fallback_count.saturating_add(1);
                    }
                    Err(error) => {
                        metrics.audio_packet_map_fallback_reason =
                            Some(format!("packet-range-read-error: {error}"));
                        metrics.source_strategy_fallback_count =
                            metrics.source_strategy_fallback_count.saturating_add(1);
                    }
                }
            }
        }
        Err(error) => {
            metrics.audio_packet_map_fallback_reason = Some(error);
            metrics.source_strategy_fallback_count =
                metrics.source_strategy_fallback_count.saturating_add(1);
        }
    }
}

fn maybe_attach_mkv_audio_range_feasibility(
    media_path: &Path,
    metrics: &mut MediaAudioStreamMetrics,
    windows: &[(f64, u32)],
) {
    if metrics.selected_sampled_audio_source_strategy.as_deref() != Some("mkv-audio-ranges") {
        return;
    }
    match mkv_audio_range_feasibility(media_path, windows, 128 * 1024) {
        Ok(feasibility) => {
            metrics.mkv_parser_used = Some(true);
            metrics.mkv_cues_present = Some(feasibility.cues_present);
            metrics.mkv_audio_track_found = Some(feasibility.audio_track_found);
            metrics.mkv_clusters_scanned = Some(feasibility.clusters_scanned);
            metrics.mkv_cluster_bytes_read = Some(feasibility.cluster_bytes_read);
            metrics.mkv_audio_block_bytes_read = Some(feasibility.audio_block_bytes);
            metrics.mkv_coalesced_range_bytes = Some(feasibility.coalesced_range_bytes);
            metrics.audio_packet_window_count = Some(windows.len());
            metrics.audio_packet_ranges = Some(feasibility.coalesced_range_count);
            metrics.audio_packet_range_bytes = Some(feasibility.audio_block_bytes);
            metrics.audio_packet_coalesced_range_bytes = Some(feasibility.coalesced_range_bytes);
            metrics.audio_sidecar_fallback_reason =
                Some("mkv-audio-ranges-feasibility-only".to_owned());
            metrics.source_strategy_fallback_count =
                metrics.source_strategy_fallback_count.saturating_add(1);
        }
        Err(reason) => {
            metrics.mkv_parser_used = Some(false);
            metrics.mkv_fallback_reason = Some(reason.clone());
            metrics.audio_packet_map_fallback_reason = Some(reason);
            metrics.source_strategy_fallback_count =
                metrics.source_strategy_fallback_count.saturating_add(1);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MkvAudioRangeFeasibility {
    cues_present: bool,
    audio_track_found: bool,
    clusters_scanned: usize,
    cluster_bytes_read: u64,
    audio_block_bytes: u64,
    coalesced_range_bytes: u64,
    coalesced_range_count: usize,
}

fn load_or_build_audio_packet_map_v3(
    ffprobe: &Path,
    media_path: &Path,
    windows: &[(f64, u32)],
    context: SampledAudioExtractionContext<'_>,
) -> Result<(AudioPacketMapV3, bool, u64, Option<u64>), String> {
    let Some((map_path, _key)) = audio_packet_map_cache_path(context, windows) else {
        return build_audio_packet_map_v3_from_ffprobe(ffprobe, media_path, context)
            .map(|(map, bytes, read_bytes)| (map, false, bytes, read_bytes));
    };
    if map_path.is_file() {
        let bytes = fs::read(&map_path).map_err(|error| error.to_string())?;
        let map: AudioPacketMapV3 = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "failed parsing audio packet map cache '{}': {error}",
                map_path.display()
            )
        })?;
        if map.source_identity.valid_for(
            &context.source_identity.normalized_path,
            context.source_identity.modified_unix_millis,
            context.source_identity.size_bytes,
        ) {
            return Ok((map, true, bytes.len() as u64, None));
        }
    }
    let (map, _bytes, read_bytes) =
        build_audio_packet_map_v3_from_ffprobe(ffprobe, media_path, context)?;
    let parent = map_path
        .parent()
        .ok_or_else(|| "packet map cache path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&map).map_err(|error| error.to_string())?;
    fs::write(&map_path, &bytes).map_err(|error| error.to_string())?;
    Ok((map, false, bytes.len() as u64, read_bytes))
}

fn audio_packet_map_cache_path(
    context: SampledAudioExtractionContext<'_>,
    windows: &[(f64, u32)],
) -> Option<(PathBuf, String)> {
    let root = context.options.sampled_pcm_cache_root.as_ref()?;
    let mut hasher = Sha256::new();
    hasher.update(b"audio-packet-map-v3");
    hasher.update(context.source_identity.normalized_path.as_bytes());
    hasher.update(context.source_identity.modified_unix_millis.to_le_bytes());
    hasher.update(context.source_identity.size_bytes.to_le_bytes());
    hasher.update(context.settings_hash);
    for (start, seconds) in windows {
        hasher.update(start.to_le_bytes());
        hasher.update(seconds.to_le_bytes());
    }
    let key = lower_hex(&hasher.finalize());
    Some((
        root.join("audio-packet-map-v3").join(format!("{key}.json")),
        key,
    ))
}

fn build_audio_packet_map_v3_from_ffprobe(
    ffprobe: &Path,
    media_path: &Path,
    context: SampledAudioExtractionContext<'_>,
) -> Result<(AudioPacketMapV3, u64, Option<u64>), String> {
    let output = run_tool_output_with_metrics(
        "ffprobe",
        ffprobe,
        [
            "-v".into(),
            "error".into(),
            "-select_streams".into(),
            "a:0".into(),
            "-show_entries".into(),
            "format=format_name:stream=index,codec_name,time_base:packet=pts_time,dts_time,duration_time,pos,size,flags".into(),
            "-of".into(),
            "json".into(),
            media_path.as_os_str().to_os_string(),
        ],
        None,
        FFPROBE_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;
    ensure_tool_success("ffprobe", &output.output).map_err(|error| error.to_string())?;
    let map =
        audio_packet_map_from_ffprobe_json(&output.output.stdout, context.source_identity.clone())?;
    Ok((
        map,
        output.output.stdout.len() as u64,
        output.io_metrics.read_bytes,
    ))
}

fn audio_packet_map_from_ffprobe_json(
    stdout: &[u8],
    source_identity: MediaFileIdentity,
) -> Result<AudioPacketMapV3, String> {
    let value: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|error| format!("invalid ffprobe JSON: {error}"))?;
    let container_format = value
        .get("format")
        .and_then(|format| format.get("format_name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let stream = value
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .and_then(|streams| streams.first())
        .ok_or_else(|| "ffprobe packet map did not include an audio stream".to_owned())?;
    let audio_stream_index = json_u64(stream.get("index")).unwrap_or(0) as u32;
    let audio_codec = stream
        .get("codec_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let (time_base_num, time_base_den) = stream
        .get("time_base")
        .and_then(serde_json::Value::as_str)
        .and_then(|text| text.split_once('/'))
        .and_then(|(num, den)| Some((num.parse::<i32>().ok()?, den.parse::<i32>().ok()?)))
        .unwrap_or((1, 1000));
    let mut packets = Vec::new();
    if let Some(values) = value.get("packets").and_then(serde_json::Value::as_array) {
        for packet in values {
            let Some(file_pos) = json_u64(packet.get("pos")) else {
                continue;
            };
            let Some(size_bytes) =
                json_u64(packet.get("size")).and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let pts_ms =
                json_seconds_millis(packet.get("pts_time").or_else(|| packet.get("dts_time")))
                    .unwrap_or(0);
            let duration_ms = json_seconds_millis(packet.get("duration_time")).unwrap_or(0);
            let key = packet
                .get("flags")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|flags| flags.contains('K'));
            packets.push(AudioPacketPositionV3 {
                pts_ms,
                duration_ms,
                file_pos,
                size_bytes,
                key,
            });
        }
    }
    packets.sort_by_key(|packet| (packet.pts_ms, packet.file_pos));
    Ok(AudioPacketMapV3 {
        source_identity,
        container_format,
        audio_stream_index,
        audio_codec,
        time_base_num,
        time_base_den,
        complete: !packets.is_empty(),
        packets,
    })
}

fn packet_ranges_for_windows(
    map: &AudioPacketMapV3,
    windows: &[(f64, u32)],
    coalesce_gap_bytes: u64,
) -> Vec<(u64, u64)> {
    let ranges = map
        .packets
        .iter()
        .filter(|packet| {
            packet_overlaps_sampled_windows(
                packet.pts_ms as f64 / 1000.0,
                packet.duration_ms as f64 / 1000.0,
                windows,
            )
        })
        .map(|packet| {
            (
                packet.file_pos,
                packet.file_pos.saturating_add(u64::from(packet.size_bytes)),
            )
        })
        .collect::<Vec<_>>();
    coalesced_ranges_with_gap(ranges, coalesce_gap_bytes)
}

fn coalesced_ranges_with_gap(mut ranges: Vec<(u64, u64)>, gap_bytes: u64) -> Vec<(u64, u64)> {
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges.sort_unstable();
    let mut output = Vec::new();
    let (mut current_start, mut current_end) = ranges[0];
    for (start, end) in ranges.into_iter().skip(1) {
        if start <= current_end.saturating_add(gap_bytes) {
            current_end = current_end.max(end);
        } else {
            output.push((current_start, current_end));
            current_start = start;
            current_end = end;
        }
    }
    output.push((current_start, current_end));
    output
}

fn read_packet_ranges(path: &Path, ranges: &[(u64, u64)]) -> Result<(Vec<u8>, u64), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    let mut ops = 0u64;
    for (start, end) in ranges {
        let len = end.saturating_sub(*start);
        let len_usize = usize::try_from(len).map_err(|_| "packet range too large".to_owned())?;
        file.seek(SeekFrom::Start(*start))
            .map_err(|error| error.to_string())?;
        let mut buffer = vec![0u8; len_usize];
        file.read_exact(&mut buffer)
            .map_err(|error| error.to_string())?;
        output.extend_from_slice(&buffer);
        ops = ops.saturating_add(1);
    }
    Ok((output, ops))
}

const MKV_ID_SEGMENT: u64 = 0x1853_8067;
const MKV_ID_SEEK_HEAD: u64 = 0x114D_9B74;
const MKV_ID_SEEK: u64 = 0x4DBB;
const MKV_ID_SEEK_ID: u64 = 0x53AB;
const MKV_ID_SEEK_POSITION: u64 = 0x53AC;
const MKV_ID_INFO: u64 = 0x1549_A966;
const MKV_ID_TIMESTAMP_SCALE: u64 = 0x002A_D7B1;
const MKV_ID_TRACKS: u64 = 0x1654_AE6B;
const MKV_ID_TRACK_ENTRY: u64 = 0xAE;
const MKV_ID_TRACK_NUMBER: u64 = 0xD7;
const MKV_ID_TRACK_TYPE: u64 = 0x83;
const MKV_ID_CUES: u64 = 0x1C53_BB6B;
const MKV_ID_CUE_POINT: u64 = 0xBB;
const MKV_ID_CUE_TIME: u64 = 0xB3;
const MKV_ID_CUE_TRACK_POSITIONS: u64 = 0xB7;
const MKV_ID_CUE_TRACK: u64 = 0xF7;
const MKV_ID_CUE_CLUSTER_POSITION: u64 = 0xF1;
const MKV_ID_CLUSTER: u64 = 0x1F43_B675;
const MKV_ID_CLUSTER_TIMECODE: u64 = 0xE7;
const MKV_ID_SIMPLE_BLOCK: u64 = 0xA3;
const MKV_ID_BLOCK_GROUP: u64 = 0xA0;
const MKV_ID_BLOCK: u64 = 0xA1;

#[derive(Debug, Clone, Copy)]
struct EbmlElement<'a> {
    id: u64,
    data_offset: usize,
    data: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
struct EbmlElementHeader {
    id: u64,
    header_len: usize,
    data_offset: usize,
    size: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct MkvSegmentLocation {
    data_start: u64,
    size: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct MkvCuePoint {
    time_ms: i64,
    cluster_position: u64,
}

fn mkv_audio_range_feasibility(
    media_path: &Path,
    windows: &[(f64, u32)],
    coalesce_gap_bytes: u64,
) -> Result<MkvAudioRangeFeasibility, String> {
    if media_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| !extension.eq_ignore_ascii_case("mkv"))
    {
        return Err("not-mkv-extension".to_owned());
    }
    let mut file = fs::File::open(media_path).map_err(|error| error.to_string())?;
    let file_len = file.metadata().map_err(|error| error.to_string())?.len();
    let prefix_len = file_len.min(4 * 1024 * 1024);
    let prefix = read_file_range(&mut file, 0, prefix_len)?;
    let segment = find_mkv_segment(&prefix).ok_or_else(|| "mkv-segment-not-found".to_owned())?;
    let seek_positions = parse_mkv_seek_positions(&prefix, segment.data_start as usize);
    let timestamp_scale = read_seek_element(&mut file, segment, &seek_positions, MKV_ID_INFO)
        .and_then(|bytes| parse_mkv_timestamp_scale(&bytes))
        .unwrap_or(1_000_000);
    let audio_track = read_seek_element(&mut file, segment, &seek_positions, MKV_ID_TRACKS)
        .and_then(|bytes| parse_mkv_audio_track_number(&bytes));
    let Some(audio_track) = audio_track else {
        return Ok(MkvAudioRangeFeasibility {
            cues_present: seek_positions.iter().any(|(id, _)| *id == MKV_ID_CUES),
            audio_track_found: false,
            ..MkvAudioRangeFeasibility::default()
        });
    };
    let Some(cues_bytes) = read_seek_element(&mut file, segment, &seek_positions, MKV_ID_CUES)
    else {
        return Ok(MkvAudioRangeFeasibility {
            cues_present: false,
            audio_track_found: true,
            ..MkvAudioRangeFeasibility::default()
        });
    };
    let mut cues = parse_mkv_cues(&cues_bytes, timestamp_scale, Some(audio_track));
    if cues.is_empty() {
        cues = parse_mkv_cues(&cues_bytes, timestamp_scale, None);
    }
    if cues.is_empty() {
        return Ok(MkvAudioRangeFeasibility {
            cues_present: true,
            audio_track_found: true,
            ..MkvAudioRangeFeasibility::default()
        });
    }
    cues.sort_by_key(|cue| (cue.time_ms, cue.cluster_position));
    cues.dedup_by_key(|cue| (cue.time_ms, cue.cluster_position));
    let selected_cluster_ranges =
        selected_mkv_cluster_ranges(segment, segment.size, file_len, &cues, windows);
    let mut audio_block_ranges = Vec::new();
    let mut cluster_bytes_read = 0u64;
    let mut clusters_scanned = 0usize;
    for (start, end) in selected_cluster_ranges {
        let len = end.saturating_sub(start).min(32 * 1024 * 1024);
        if len == 0 {
            continue;
        }
        let bytes = read_file_range(&mut file, start, len)?;
        cluster_bytes_read = cluster_bytes_read.saturating_add(bytes.len() as u64);
        clusters_scanned += 1;
        audio_block_ranges.extend(parse_mkv_cluster_audio_block_ranges(
            &bytes,
            start,
            timestamp_scale,
            audio_track,
            windows,
        ));
    }
    let audio_block_bytes = audio_block_ranges
        .iter()
        .map(|(start, end)| end.saturating_sub(*start))
        .sum::<u64>();
    let coalesced = coalesced_ranges_with_gap(audio_block_ranges, coalesce_gap_bytes);
    let coalesced_range_bytes = coalesced
        .iter()
        .map(|(start, end)| end.saturating_sub(*start))
        .sum::<u64>();
    Ok(MkvAudioRangeFeasibility {
        cues_present: true,
        audio_track_found: true,
        clusters_scanned,
        cluster_bytes_read,
        audio_block_bytes,
        coalesced_range_bytes,
        coalesced_range_count: coalesced.len(),
    })
}

fn read_file_range(file: &mut fs::File, start: u64, len: u64) -> Result<Vec<u8>, String> {
    let len = usize::try_from(len).map_err(|_| "range-too-large".to_owned())?;
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn find_mkv_segment(prefix: &[u8]) -> Option<MkvSegmentLocation> {
    let mut offset = 0usize;
    while let Some(header) = ebml_read_header(prefix, offset) {
        if header.id == MKV_ID_SEGMENT {
            let data_start = header.data_offset as u64;
            let size = (header.size != usize::MAX).then_some(header.size as u64);
            return Some(MkvSegmentLocation { data_start, size });
        }
        offset = header.data_offset.saturating_add(header.size);
        if offset <= header.data_offset {
            break;
        }
    }
    None
}

fn parse_mkv_seek_positions(prefix: &[u8], segment_data_start: usize) -> Vec<(u64, u64)> {
    let mut output = Vec::new();
    let mut offset = segment_data_start;
    while let Some(header) = ebml_read_header(prefix, offset) {
        let element_end = header.data_offset.saturating_add(header.size);
        if element_end > prefix.len() {
            break;
        }
        if header.id == MKV_ID_SEEK_HEAD {
            for seek in ebml_child_elements(&prefix[header.data_offset..element_end]) {
                if seek.id != MKV_ID_SEEK {
                    continue;
                }
                let mut target_id = None;
                let mut position = None;
                for child in ebml_child_elements(seek.data) {
                    match child.id {
                        MKV_ID_SEEK_ID => target_id = Some(ebml_uint_raw_id(child.data)),
                        MKV_ID_SEEK_POSITION => position = ebml_uint(child.data),
                        _ => {}
                    }
                }
                if let (Some(target_id), Some(position)) = (target_id, position) {
                    output.push((target_id, position));
                }
            }
        }
        if header.id == MKV_ID_TRACKS || header.id == MKV_ID_CUES {
            output.push((header.id, offset.saturating_sub(segment_data_start) as u64));
        }
        offset = element_end;
        if offset > segment_data_start.saturating_add(4 * 1024 * 1024) {
            break;
        }
    }
    output.sort_unstable();
    output.dedup();
    output
}

fn read_seek_element(
    file: &mut fs::File,
    segment: MkvSegmentLocation,
    seek_positions: &[(u64, u64)],
    target_id: u64,
) -> Option<Vec<u8>> {
    for (_, relative) in seek_positions.iter().filter(|(id, _)| *id == target_id) {
        let absolute = segment.data_start.saturating_add(*relative);
        let Ok(header_bytes) = read_file_range(file, absolute, 16) else {
            continue;
        };
        let Some(header) = ebml_read_header(&header_bytes, 0) else {
            continue;
        };
        if header.id != target_id || header.size == usize::MAX {
            continue;
        }
        let total = header.header_len.saturating_add(header.size);
        let total = u64::try_from(total).ok()?.min(32 * 1024 * 1024);
        let Ok(bytes) = read_file_range(file, absolute, total) else {
            continue;
        };
        let header = ebml_read_header(&bytes, 0)?;
        return Some(
            bytes[header.data_offset..header.data_offset.saturating_add(header.size)].to_vec(),
        );
    }
    None
}

fn parse_mkv_timestamp_scale(info: &[u8]) -> Option<u64> {
    for child in ebml_child_elements(info) {
        if child.id == MKV_ID_TIMESTAMP_SCALE {
            return ebml_uint(child.data);
        }
    }
    None
}

fn parse_mkv_audio_track_number(tracks: &[u8]) -> Option<u64> {
    for entry in ebml_child_elements(tracks) {
        if entry.id != MKV_ID_TRACK_ENTRY {
            continue;
        }
        let mut track_number = None;
        let mut track_type = None;
        for child in ebml_child_elements(entry.data) {
            match child.id {
                MKV_ID_TRACK_NUMBER => track_number = ebml_uint(child.data),
                MKV_ID_TRACK_TYPE => track_type = ebml_uint(child.data),
                _ => {}
            }
        }
        if track_type == Some(2) {
            return track_number;
        }
    }
    None
}

fn parse_mkv_cues(
    cues: &[u8],
    timestamp_scale: u64,
    required_track: Option<u64>,
) -> Vec<MkvCuePoint> {
    let mut output = Vec::new();
    for point in ebml_child_elements(cues) {
        if point.id != MKV_ID_CUE_POINT {
            continue;
        }
        let mut time = None;
        let mut positions = Vec::new();
        for child in ebml_child_elements(point.data) {
            match child.id {
                MKV_ID_CUE_TIME => time = ebml_uint(child.data),
                MKV_ID_CUE_TRACK_POSITIONS => {
                    let mut track = None;
                    let mut cluster_position = None;
                    for pos_child in ebml_child_elements(child.data) {
                        match pos_child.id {
                            MKV_ID_CUE_TRACK => track = ebml_uint(pos_child.data),
                            MKV_ID_CUE_CLUSTER_POSITION => {
                                cluster_position = ebml_uint(pos_child.data)
                            }
                            _ => {}
                        }
                    }
                    if required_track.is_none_or(|required| track == Some(required))
                        && let Some(cluster_position) = cluster_position
                    {
                        positions.push(cluster_position);
                    }
                }
                _ => {}
            }
        }
        if let Some(time) = time {
            let time_ms = ebml_timestamp_to_millis(time, timestamp_scale);
            for cluster_position in positions {
                output.push(MkvCuePoint {
                    time_ms,
                    cluster_position,
                });
            }
        }
    }
    output
}

fn selected_mkv_cluster_ranges(
    segment: MkvSegmentLocation,
    segment_size: Option<u64>,
    file_len: u64,
    cues: &[MkvCuePoint],
    windows: &[(f64, u32)],
) -> Vec<(u64, u64)> {
    let segment_end = segment_size
        .map(|size| segment.data_start.saturating_add(size).min(file_len))
        .unwrap_or(file_len);
    let mut ranges = Vec::new();
    for (index, cue) in cues.iter().enumerate() {
        let next_position = cues
            .iter()
            .skip(index + 1)
            .map(|next| next.cluster_position)
            .find(|position| *position > cue.cluster_position)
            .unwrap_or_else(|| segment_end.saturating_sub(segment.data_start));
        let cue_time = cue.time_ms as f64 / 1000.0;
        let next_time = cues
            .iter()
            .skip(index + 1)
            .find(|next| next.cluster_position > cue.cluster_position)
            .map(|next| next.time_ms as f64 / 1000.0)
            .unwrap_or(cue_time + 30.0);
        if windows.iter().any(|(start, seconds)| {
            let end = *start + f64::from(*seconds);
            cue_time <= end && next_time >= *start
        }) {
            let start = segment.data_start.saturating_add(cue.cluster_position);
            let end = segment
                .data_start
                .saturating_add(next_position)
                .min(segment_end);
            if end > start {
                ranges.push((start, end));
            }
        }
    }
    coalesced_ranges_with_gap(ranges, 0)
}

fn parse_mkv_cluster_audio_block_ranges(
    bytes: &[u8],
    absolute_start: u64,
    timestamp_scale: u64,
    audio_track: u64,
    windows: &[(f64, u32)],
) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let Some(cluster_header) = ebml_read_header(bytes, 0) else {
        return ranges;
    };
    if cluster_header.id != MKV_ID_CLUSTER {
        return ranges;
    }
    let mut cluster_time_ms = 0i64;
    let cluster_data = &bytes[cluster_header.data_offset
        ..cluster_header
            .data_offset
            .saturating_add(cluster_header.size)
            .min(bytes.len())];
    for child in ebml_child_elements(cluster_data) {
        if child.id == MKV_ID_CLUSTER_TIMECODE {
            cluster_time_ms = ebml_uint(child.data)
                .map(|value| ebml_timestamp_to_millis(value, timestamp_scale))
                .unwrap_or(0);
        }
    }
    for child in ebml_child_elements(cluster_data) {
        match child.id {
            MKV_ID_SIMPLE_BLOCK => {
                if let Some((track, relative_ms)) = parse_mkv_block_track_and_time(child.data)
                    && track == audio_track
                {
                    let pts_seconds = (cluster_time_ms + relative_ms) as f64 / 1000.0;
                    if packet_overlaps_sampled_windows(pts_seconds, 0.1, windows) {
                        ranges.push((
                            absolute_start
                                .saturating_add(cluster_header.data_offset as u64)
                                .saturating_add(child.data_offset as u64),
                            absolute_start
                                .saturating_add(cluster_header.data_offset as u64)
                                .saturating_add(child.data_offset as u64)
                                .saturating_add(child.data.len() as u64),
                        ));
                    }
                }
            }
            MKV_ID_BLOCK_GROUP => {
                for block_child in ebml_child_elements(child.data) {
                    if block_child.id != MKV_ID_BLOCK {
                        continue;
                    }
                    if let Some((track, relative_ms)) =
                        parse_mkv_block_track_and_time(block_child.data)
                        && track == audio_track
                    {
                        let pts_seconds = (cluster_time_ms + relative_ms) as f64 / 1000.0;
                        if packet_overlaps_sampled_windows(pts_seconds, 0.1, windows) {
                            ranges.push((
                                absolute_start
                                    .saturating_add(cluster_header.data_offset as u64)
                                    .saturating_add(child.data_offset as u64)
                                    .saturating_add(block_child.data_offset as u64),
                                absolute_start
                                    .saturating_add(cluster_header.data_offset as u64)
                                    .saturating_add(child.data_offset as u64)
                                    .saturating_add(block_child.data_offset as u64)
                                    .saturating_add(block_child.data.len() as u64),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ranges
}

fn parse_mkv_block_track_and_time(data: &[u8]) -> Option<(u64, i64)> {
    let (track, track_len) = ebml_read_vint_value(data, 0)?;
    if data.len() < track_len + 3 {
        return None;
    }
    let relative = i16::from_be_bytes([data[track_len], data[track_len + 1]]) as i64;
    Some((track, relative))
}

fn ebml_child_elements(data: &[u8]) -> Vec<EbmlElement<'_>> {
    let mut elements = Vec::new();
    let mut offset = 0usize;
    while let Some(header) = ebml_read_header(data, offset) {
        let end = header.data_offset.saturating_add(header.size);
        if header.size == usize::MAX || end > data.len() || end <= offset {
            break;
        }
        elements.push(EbmlElement {
            id: header.id,
            data_offset: header.data_offset,
            data: &data[header.data_offset..end],
        });
        offset = end;
    }
    elements
}

fn ebml_read_header(data: &[u8], offset: usize) -> Option<EbmlElementHeader> {
    let (id, id_len) = ebml_read_id(data, offset)?;
    let (size, size_len) = ebml_read_size(data, offset + id_len)?;
    let data_offset = offset.checked_add(id_len)?.checked_add(size_len)?;
    Some(EbmlElementHeader {
        id,
        header_len: id_len + size_len,
        data_offset,
        size,
    })
}

fn ebml_read_id(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    let first = *data.get(offset)?;
    let len = ebml_vint_len(first)?;
    if len > 4 || offset.checked_add(len)? > data.len() {
        return None;
    }
    let mut value = 0u64;
    for byte in &data[offset..offset + len] {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, len))
}

fn ebml_read_size(data: &[u8], offset: usize) -> Option<(usize, usize)> {
    let (value, len) = ebml_read_vint_value(data, offset)?;
    let unknown = value == ((1u64 << (7 * len)) - 1);
    let size = if unknown {
        usize::MAX
    } else {
        usize::try_from(value).ok()?
    };
    Some((size, len))
}

fn ebml_read_vint_value(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    let first = *data.get(offset)?;
    let len = ebml_vint_len(first)?;
    if offset.checked_add(len)? > data.len() {
        return None;
    }
    let marker = 1u8 << (8 - len);
    let mut value = u64::from(first & !marker);
    for byte in &data[offset + 1..offset + len] {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, len))
}

fn ebml_vint_len(first: u8) -> Option<usize> {
    if first == 0 {
        return None;
    }
    Some(first.leading_zeros() as usize + 1)
}

fn ebml_uint(data: &[u8]) -> Option<u64> {
    if data.len() > 8 {
        return None;
    }
    let mut value = 0u64;
    for byte in data {
        value = (value << 8) | u64::from(*byte);
    }
    Some(value)
}

fn ebml_uint_raw_id(data: &[u8]) -> u64 {
    let mut value = 0u64;
    for byte in data {
        value = (value << 8) | u64::from(*byte);
    }
    value
}

fn ebml_timestamp_to_millis(value: u64, timestamp_scale: u64) -> i64 {
    let millis = u128::from(value)
        .saturating_mul(u128::from(timestamp_scale))
        .saturating_div(1_000_000);
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
    if target.selected_sampled_audio_source_strategy.is_none() {
        target.selected_sampled_audio_source_strategy =
            source.selected_sampled_audio_source_strategy.clone();
    }
    if target.source_strategy_decision_reason.is_none() {
        target.source_strategy_decision_reason = source.source_strategy_decision_reason.clone();
    }
    target.source_strategy_fallback_count = target
        .source_strategy_fallback_count
        .saturating_add(source.source_strategy_fallback_count);
    if target.audio_packet_map_cache_hit.is_none() {
        target.audio_packet_map_cache_hit = source.audio_packet_map_cache_hit;
    }
    if target.audio_packet_map_build_millis.is_none() {
        target.audio_packet_map_build_millis = source.audio_packet_map_build_millis;
    }
    if target.audio_packet_map_packet_count.is_none() {
        target.audio_packet_map_packet_count = source.audio_packet_map_packet_count;
    }
    if target.audio_packet_map_bytes.is_none() {
        target.audio_packet_map_bytes = source.audio_packet_map_bytes;
    }
    if target.audio_packet_map_complete.is_none() {
        target.audio_packet_map_complete = source.audio_packet_map_complete;
    }
    if target.audio_packet_map_fallback_reason.is_none() {
        target.audio_packet_map_fallback_reason = source.audio_packet_map_fallback_reason.clone();
    }
    if target.audio_packet_window_count.is_none() {
        target.audio_packet_window_count = source.audio_packet_window_count;
    }
    if target.audio_packet_ranges.is_none() {
        target.audio_packet_ranges = source.audio_packet_ranges;
    }
    if target.audio_packet_range_bytes.is_none() {
        target.audio_packet_range_bytes = source.audio_packet_range_bytes;
    }
    if target.audio_packet_coalesced_range_bytes.is_none() {
        target.audio_packet_coalesced_range_bytes = source.audio_packet_coalesced_range_bytes;
    }
    if target.audio_packet_range_read_millis.is_none() {
        target.audio_packet_range_read_millis = source.audio_packet_range_read_millis;
    }
    if target.audio_packet_range_read_ops.is_none() {
        target.audio_packet_range_read_ops = source.audio_packet_range_read_ops;
    }
    if target.audio_packet_read_amplification_vs_pcm.is_none() {
        target.audio_packet_read_amplification_vs_pcm =
            source.audio_packet_read_amplification_vs_pcm;
    }
    if target.audio_packet_estimated_savings_vs_current.is_none() {
        target.audio_packet_estimated_savings_vs_current =
            source.audio_packet_estimated_savings_vs_current;
    }
    if target.sampled_pcm_cache_hit.is_none() {
        target.sampled_pcm_cache_hit = source.sampled_pcm_cache_hit;
    }
    if target.sampled_pcm_cache_bytes.is_none() {
        target.sampled_pcm_cache_bytes = source.sampled_pcm_cache_bytes;
    }
    if target.sampled_pcm_cache_read_millis.is_none() {
        target.sampled_pcm_cache_read_millis = source.sampled_pcm_cache_read_millis;
    }
    if target.sampled_pcm_cache_write_millis.is_none() {
        target.sampled_pcm_cache_write_millis = source.sampled_pcm_cache_write_millis;
    }
    if target.sampled_pcm_cache_saved_millis.is_none() {
        target.sampled_pcm_cache_saved_millis = source.sampled_pcm_cache_saved_millis;
    }
    if target.audio_sidecar_mode.is_none() {
        target.audio_sidecar_mode = source.audio_sidecar_mode.clone();
    }
    if target.audio_sidecar_fallback_reason.is_none() {
        target.audio_sidecar_fallback_reason = source.audio_sidecar_fallback_reason.clone();
    }
    if target.sampled_ffmpeg_window_strategy.is_none() {
        target.sampled_ffmpeg_window_strategy = source.sampled_ffmpeg_window_strategy.clone();
    }
    if target.sampled_windows_planned.is_none() {
        target.sampled_windows_planned = source.sampled_windows_planned;
    }
    if target.sampled_stop_reason.is_none() {
        target.sampled_stop_reason = source.sampled_stop_reason.clone();
    }
    if target.provisional_landmark_count.is_none() {
        target.provisional_landmark_count = source.provisional_landmark_count;
    }
    if target.provisional_body_region_count.is_none() {
        target.provisional_body_region_count = source.provisional_body_region_count;
    }
    if target.adaptive_saved_seconds.is_none() {
        target.adaptive_saved_seconds = source.adaptive_saved_seconds;
    }
    if target.adaptive_saved_estimated_read_bytes.is_none() {
        target.adaptive_saved_estimated_read_bytes = source.adaptive_saved_estimated_read_bytes;
    }
    if target.mkv_parser_used.is_none() {
        target.mkv_parser_used = source.mkv_parser_used;
    }
    if target.mkv_cues_present.is_none() {
        target.mkv_cues_present = source.mkv_cues_present;
    }
    if target.mkv_audio_track_found.is_none() {
        target.mkv_audio_track_found = source.mkv_audio_track_found;
    }
    if target.mkv_clusters_scanned.is_none() {
        target.mkv_clusters_scanned = source.mkv_clusters_scanned;
    }
    if target.mkv_cluster_bytes_read.is_none() {
        target.mkv_cluster_bytes_read = source.mkv_cluster_bytes_read;
    }
    if target.mkv_audio_block_bytes_read.is_none() {
        target.mkv_audio_block_bytes_read = source.mkv_audio_block_bytes_read;
    }
    if target.mkv_coalesced_range_bytes.is_none() {
        target.mkv_coalesced_range_bytes = source.mkv_coalesced_range_bytes;
    }
    if target.mkv_estimated_savings_vs_current.is_none() {
        target.mkv_estimated_savings_vs_current = source.mkv_estimated_savings_vs_current;
    }
    if target.mkv_fallback_reason.is_none() {
        target.mkv_fallback_reason = source.mkv_fallback_reason.clone();
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

    #[test]
    fn audio_packet_map_parses_ranges_and_uses_identity_sensitive_cache_key() {
        let json = br#"{
          "format": { "format_name": "matroska,webm" },
          "streams": [
            { "index": 1, "codec_name": "flac", "time_base": "1/1000" }
          ],
          "packets": [
            { "pts_time": "0.000", "duration_time": "1.000", "pos": "100", "size": "10" },
            { "pts_time": "10.000", "duration_time": "1.000", "pos": "200", "size": "20" },
            { "pts_time": "10.500", "duration_time": "1.000", "pos": "220", "size": "30" },
            { "pts_time": "60.000", "duration_time": "1.000", "pos": "500", "size": "40" }
          ]
        }"#;
        let identity = MediaFileIdentity {
            normalized_path: "c:\\media\\episode.mkv".to_owned(),
            modified_unix_millis: 123,
            size_bytes: 456,
        };

        let map = audio_packet_map_from_ffprobe_json(json, identity.clone())
            .expect("packet map should parse");

        assert!(map.complete);
        assert!(map.valid_for(
            &identity.normalized_path,
            identity.modified_unix_millis,
            identity.size_bytes,
            1,
            "matroska,webm",
            "flac",
        ));
        assert!(!map.valid_for(
            &identity.normalized_path,
            identity.modified_unix_millis + 1,
            identity.size_bytes,
            1,
            "matroska,webm",
            "flac",
        ));
        assert_eq!(map.packets.len(), 4);
        assert_eq!(map.packets[1].pts_ms, 10_000);

        let ranges = packet_ranges_for_windows(&map, &[(10.0, 2)], 128);

        assert_eq!(ranges, vec![(200, 250)]);

        let options = MediaFingerprintExtractionOptions {
            sampled_audio_source: MediaSampledAudioSourceStrategy::PacketMap,
            sampled_pcm_cache_root: Some(PathBuf::from("packet-cache")),
            adaptive_sampled_fast: false,
        };
        let context = SampledAudioExtractionContext {
            source_identity: &identity,
            settings_hash: [7; 32],
            options: &options,
            ffprobe: None,
        };
        let (_, key) = audio_packet_map_cache_path(context, &[(10.0, 2)])
            .expect("packet map cache path should be available");
        let changed_identity = MediaFileIdentity {
            modified_unix_millis: identity.modified_unix_millis + 1,
            ..identity
        };
        let changed_context = SampledAudioExtractionContext {
            source_identity: &changed_identity,
            settings_hash: [7; 32],
            options: &options,
            ffprobe: None,
        };
        let (_, changed_key) = audio_packet_map_cache_path(changed_context, &[(10.0, 2)])
            .expect("changed packet map cache path should be available");

        assert_ne!(key, changed_key);
    }

    #[test]
    fn sampled_pcm_cache_key_includes_adaptive_mode() {
        let identity = MediaFileIdentity {
            normalized_path: "c:\\media\\episode.mkv".to_owned(),
            modified_unix_millis: 123,
            size_bytes: 456,
        };
        let config = sampled_audio_index_config(MediaAudioIndexMode::SampledFast);
        let windows = sampled_audio_windows_v3(Some(1500.0), config);

        let fixed = sampled_audio_cache_key(
            &identity,
            [9; 32],
            MediaAudioIndexMode::SampledFast,
            config,
            &windows,
            false,
        );
        let adaptive = sampled_audio_cache_key(
            &identity,
            [9; 32],
            MediaAudioIndexMode::SampledFast,
            config,
            &windows,
            true,
        );

        assert_ne!(fixed, adaptive);
    }

    #[test]
    fn sampled_windows_filter_complex_concatenates_expected_windows() {
        let filter = sampled_windows_filter_complex(&[(180.0, 20), (750.5, 20), (1320.0, 20)]);

        assert!(filter.contains("atrim=start=180.000:duration=20"));
        assert!(filter.contains("atrim=start=750.500:duration=20"));
        assert!(filter.contains("[a0][a1][a2]concat=n=3:v=0:a=1[out]"));
    }

    #[test]
    fn mkv_audio_range_feasibility_parses_cues_and_audio_blocks() {
        let root =
            std::env::temp_dir().join(format!("sorotte-mkv-feasibility-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        let path = root.join("fixture.mkv");

        let tracks = ebml_elem(
            &[0x16, 0x54, 0xae, 0x6b],
            &ebml_elem(
                &[0xae],
                &[ebml_elem(&[0xd7], &[0x01]), ebml_elem(&[0x83], &[0x02])].concat(),
            ),
        );
        let simple_block = {
            let mut data = vec![0x81, 0x00, 0x00, 0x00];
            data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
            ebml_elem(&[0xa3], &data)
        };
        let cluster = ebml_elem(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[ebml_elem(&[0xe7], &[0x27, 0x10]), simple_block].concat(),
        );
        let cues_with_placeholder = ebml_elem(
            &[0x1c, 0x53, 0xbb, 0x6b],
            &ebml_elem(
                &[0xbb],
                &[
                    ebml_elem(&[0xb3], &[0x27, 0x10]),
                    ebml_elem(
                        &[0xb7],
                        &[ebml_elem(&[0xf7], &[0x01]), ebml_elem(&[0xf1], &[0x00])].concat(),
                    ),
                ]
                .concat(),
            ),
        );
        let cluster_pos = tracks.len() + cues_with_placeholder.len();
        let cues = ebml_elem(
            &[0x1c, 0x53, 0xbb, 0x6b],
            &ebml_elem(
                &[0xbb],
                &[
                    ebml_elem(&[0xb3], &[0x27, 0x10]),
                    ebml_elem(
                        &[0xb7],
                        &[
                            ebml_elem(&[0xf7], &[0x01]),
                            ebml_elem(&[0xf1], &[cluster_pos as u8]),
                        ]
                        .concat(),
                    ),
                ]
                .concat(),
            ),
        );
        let segment = ebml_elem(&[0x18, 0x53, 0x80, 0x67], &[tracks, cues, cluster].concat());
        fs::write(&path, segment).expect("mkv fixture should be written");

        let feasibility =
            mkv_audio_range_feasibility(&path, &[(10.0, 1)], 128).expect("mkv should parse");

        assert!(feasibility.cues_present);
        assert!(feasibility.audio_track_found);
        assert_eq!(feasibility.clusters_scanned, 1);
        assert!(feasibility.audio_block_bytes > 0);
        assert!(feasibility.coalesced_range_bytes >= feasibility.audio_block_bytes);
    }

    fn ebml_elem(id: &[u8], data: &[u8]) -> Vec<u8> {
        assert!(data.len() < 127);
        let mut output = Vec::new();
        output.extend_from_slice(id);
        output.push(0x80 | data.len() as u8);
        output.extend_from_slice(data);
        output
    }
}
