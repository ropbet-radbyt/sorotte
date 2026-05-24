use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::UNIX_EPOCH;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MEDIA_MATCH_ALGORITHM_VERSION: u32 = 2;
pub const MEDIA_MATCH_FILE_PAYLOAD_KEY: &str = "mediaMatch";
pub const MEDIA_MATCH_WIRE_SCHEMA_V2: &str = "sorotte.mediaMatch.v2";
pub const MEDIA_MATCH_WIRE_MAX_BYTES: usize = 32 * 1024;
pub const MEDIA_MATCH_ANCHOR_VERSION: u32 = 2;

const FRAME_HASH_BITS: u32 = 64;
pub const DEFAULT_FRAME_HAMMING_THRESHOLD: u32 = 10;
const DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS: i64 = 1_000;
const DEFAULT_ANCHOR_OFFSET_BIN_MS: i64 = 1_000;
const VIDEO_LSH_BANDS: u32 = 4;
const VIDEO_LSH_BITS_PER_BAND: u32 = 16;
const FAST_VIDEO_SAMPLE_FRAMES: usize = 12;
const FAST_AUDIO_ANCHOR_LIMIT: usize = 96;
const FAST_VIDEO_ANCHOR_LIMIT: usize = 48;
const FULL_AUDIO_ANCHOR_LIMIT: usize = 512;
const FULL_VIDEO_ANCHOR_LIMIT: usize = 192;
const AUDIO_ANCHOR_WINDOW_TOKENS: usize = 4;
const MAX_SUMMARY_ANCHORS: usize = 1024;
const VIDEO_FRAME_WIDTH: usize = 32;
const VIDEO_FRAME_HEIGHT: usize = 32;
const VIDEO_FRAME_BYTES: usize = VIDEO_FRAME_WIDTH * VIDEO_FRAME_HEIGHT;
const FAST_AUDIO_SAMPLE_SECONDS: u32 = 120;
const MEDIA_TOOL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const FFPROBE_TIMEOUT: Duration = Duration::from_secs(15);
const FPCALC_TIMEOUT: Duration = Duration::from_secs(90);
const FFMPEG_FAST_FRAME_TIMEOUT: Duration = Duration::from_secs(45);
const FFMPEG_FULL_VIDEO_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaMatchTier {
    Exact,
    Strong,
    Probable,
    Weak,
    Reject,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaMatchAutoplayPolicy {
    #[default]
    DiagnosticsOnly,
    AllowStrongSameMedia,
}

fn default_media_match_background_warmup_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchSettings {
    pub fingerprinting_enabled: bool,
    #[serde(default = "default_media_match_background_warmup_enabled")]
    pub background_warmup_enabled: bool,
    #[serde(default = "default_media_match_background_warmup_enabled")]
    pub wire_sharing_enabled: bool,
    pub runtime_tolerance_enabled: bool,
    pub runtime_tolerance_seconds: f64,
    pub autoplay_policy: MediaMatchAutoplayPolicy,
    pub audio_strong_similarity: f64,
    pub audio_probable_similarity: f64,
    pub video_strong_coverage: f64,
    pub video_probable_coverage: f64,
    pub video_weak_coverage: f64,
    pub max_alignment_drift_ratio: f64,
}

impl Default for MediaMatchSettings {
    fn default() -> Self {
        Self {
            fingerprinting_enabled: false,
            background_warmup_enabled: true,
            wire_sharing_enabled: true,
            runtime_tolerance_enabled: true,
            runtime_tolerance_seconds: 3.0,
            autoplay_policy: MediaMatchAutoplayPolicy::DiagnosticsOnly,
            audio_strong_similarity: 0.90,
            audio_probable_similarity: 0.68,
            video_strong_coverage: 0.66,
            video_probable_coverage: 0.55,
            video_weak_coverage: 0.18,
            max_alignment_drift_ratio: 0.015,
        }
    }
}

impl MediaMatchSettings {
    pub fn autoplay_allows_strong_same_media(&self) -> bool {
        self.autoplay_policy == MediaMatchAutoplayPolicy::AllowStrongSameMedia
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchDecision {
    pub tier: MediaMatchTier,
    pub evidence: MediaMatchEvidence,
    pub explanation: String,
}

impl MediaMatchDecision {
    pub fn same_media_for_autoplay(&self, settings: &MediaMatchSettings) -> bool {
        settings.autoplay_allows_strong_same_media() && self.tier == MediaMatchTier::Strong
    }

    pub fn unknown(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            tier: MediaMatchTier::Unknown,
            evidence: MediaMatchEvidence {
                metadata: MetadataMatchEvidence::default(),
                audio: None,
                video: None,
                alignment: None,
                notes: vec![reason.clone()],
            },
            explanation: reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaMatchEvidence {
    pub metadata: MetadataMatchEvidence,
    pub audio: Option<AudioMatchEvidence>,
    pub video: Option<VideoMatchEvidence>,
    pub alignment: Option<MediaTimelineAlignment>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MetadataMatchEvidence {
    pub same_normalized_path: bool,
    pub same_size: Option<bool>,
    pub duration_delta_seconds: Option<f64>,
    pub duration_within_tolerance: Option<bool>,
    pub extension_match: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioMatchEvidence {
    pub similarity: f64,
    pub shared_token_ratio: f64,
    pub duration_delta_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoMatchEvidence {
    pub aligned_pairs: usize,
    pub query_coverage: f64,
    pub candidate_coverage: f64,
    pub best_offset_seconds: f64,
    pub drift_ratio: f64,
    pub mean_hamming_distance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTimelineAlignment {
    pub offset_seconds: f64,
    pub scale_ppm: i32,
    pub drift_ratio: f64,
    pub aligned_pairs: usize,
    pub aligned_audio_anchors: usize,
    pub aligned_video_anchors: usize,
    pub aligned_span_seconds: f64,
    pub second_best_offset_margin: f64,
    pub first_query_second: f64,
    pub last_query_second: f64,
    pub first_candidate_second: f64,
    pub last_candidate_second: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFileIdentity {
    pub normalized_path: String,
    pub modified_unix_millis: u64,
    pub size_bytes: u64,
}

impl MediaFileIdentity {
    pub fn new(path: impl AsRef<Path>, modified_unix_millis: u64, size_bytes: u64) -> Self {
        Self {
            normalized_path: normalize_media_path(path),
            modified_unix_millis,
            size_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaFingerprintProfile {
    FastAnchorV2,
    FullAnchorV2,
}

impl MediaFingerprintProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FastAnchorV2 => "fast-anchor-v2",
            Self::FullAnchorV2 => "full-anchor-v2",
        }
    }

    pub fn is_fast(self) -> bool {
        matches!(self, Self::FastAnchorV2)
    }
}

fn default_media_fingerprint_profile() -> MediaFingerprintProfile {
    MediaFingerprintProfile::FullAnchorV2
}

fn default_audio_sample_seconds() -> u32 {
    0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaExtractionSettings {
    #[serde(default = "default_media_fingerprint_profile")]
    pub profile: MediaFingerprintProfile,
    pub frame_sample_interval_seconds: u32,
    pub max_frames: usize,
    #[serde(default = "default_audio_sample_seconds")]
    pub audio_sample_seconds: u32,
    pub audio_algorithm: String,
    pub video_algorithm: String,
}

impl Default for MediaExtractionSettings {
    fn default() -> Self {
        Self::full_anchor_v2()
    }
}

impl MediaExtractionSettings {
    pub fn fast_anchor_v2() -> Self {
        Self {
            profile: MediaFingerprintProfile::FastAnchorV2,
            frame_sample_interval_seconds: 0,
            max_frames: FAST_VIDEO_SAMPLE_FRAMES,
            audio_sample_seconds: FAST_AUDIO_SAMPLE_SECONDS,
            audio_algorithm: format!("chromaprint-anchor-v2-{FAST_AUDIO_SAMPLE_SECONDS}s"),
            video_algorithm: "sorotte-luma-anchor-v2-fast".to_owned(),
        }
    }

    pub fn full_anchor_v2() -> Self {
        Self {
            profile: MediaFingerprintProfile::FullAnchorV2,
            frame_sample_interval_seconds: 10,
            max_frames: 720,
            audio_sample_seconds: 0,
            audio_algorithm: "chromaprint-anchor-v2-full".to_owned(),
            video_algorithm: "sorotte-luma-anchor-v2-full".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFingerprintRecord {
    pub identity: MediaFileIdentity,
    pub algorithm_version: u32,
    pub extraction_settings: MediaExtractionSettings,
    pub duration_seconds: Option<f64>,
    pub container_fingerprint: String,
    pub audio: Option<AudioFingerprint>,
    pub video: Option<VideoFingerprint>,
    #[serde(default)]
    pub audio_anchors: Vec<AudioAnchor>,
    #[serde(default)]
    pub video_anchors: Vec<VideoAnchor>,
    #[serde(default)]
    pub audio_error: Option<String>,
    #[serde(default)]
    pub video_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatchToolPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub fpcalc: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaToolInvocationCounts {
    pub ffmpeg: u32,
    pub ffprobe: u32,
    pub fpcalc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaExtractionTimings {
    pub ffprobe_millis: u128,
    pub audio_millis: u128,
    pub video_millis: u128,
    pub total_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaFingerprintExtractionReport {
    pub invocations: MediaToolInvocationCounts,
    pub timings: MediaExtractionTimings,
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

pub fn expected_media_tool_invocation_counts(
    _settings: &MediaExtractionSettings,
) -> MediaToolInvocationCounts {
    MediaToolInvocationCounts {
        ffmpeg: 1,
        ffprobe: 1,
        fpcalc: 1,
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

#[derive(Debug, Clone, PartialEq)]
pub struct MediaMatchCandidateDecision {
    pub candidate_path: String,
    pub decision: MediaMatchDecision,
}

impl MediaFingerprintRecord {
    pub fn valid_for(
        &self,
        normalized_path: &str,
        modified_unix_millis: u64,
        size_bytes: u64,
        algorithm_version: u32,
        extraction_settings: &MediaExtractionSettings,
    ) -> bool {
        self.identity.normalized_path == normalized_path
            && self.identity.modified_unix_millis == modified_unix_millis
            && self.identity.size_bytes == size_bytes
            && self.algorithm_version == algorithm_version
            && &self.extraction_settings == extraction_settings
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFingerprint {
    pub duration_seconds: Option<f64>,
    pub fingerprint_tokens: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFingerprint {
    pub duration_seconds: Option<u32>,
    pub frames: Vec<FrameFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameFingerprint {
    pub timestamp_millis: u64,
    pub hash: u64,
}

impl FrameFingerprint {
    pub fn new(timestamp_seconds: f64, hash: u64) -> Self {
        let timestamp_millis = if timestamp_seconds.is_finite() && timestamp_seconds > 0.0 {
            (timestamp_seconds * 1000.0).round() as u64
        } else {
            0
        };
        Self {
            timestamp_millis,
            hash,
        }
    }

    pub fn timestamp_seconds(self) -> f64 {
        self.timestamp_millis as f64 / 1000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAnchor {
    pub bucket: u32,
    pub t_ms: u32,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoAnchor {
    pub bucket: u32,
    pub t_ms: u32,
    pub hash64: u64,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAnchorProfile {
    pub version: u32,
    pub profile: String,
    pub duration_ms: Option<u32>,
    pub audio_anchors: Vec<AudioAnchor>,
    pub video_anchors: Vec<VideoAnchor>,
}

impl MediaAnchorProfile {
    pub fn is_empty(&self) -> bool {
        self.audio_anchors.is_empty() && self.video_anchors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFingerprintSummary {
    pub profile: String,
    pub settings_hash: [u8; 32],
    pub duration_ms: Option<u32>,
    pub audio_summary: Option<Vec<u8>>,
    pub video_summary: Option<Vec<u8>>,
    pub audio_anchor_count: usize,
    pub video_anchor_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSummaryDecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyAnchors(usize),
}

impl fmt::Display for MediaSummaryDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(formatter, "invalid media anchor summary magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported media anchor summary version {version}"
                )
            }
            Self::InvalidLength => write!(formatter, "invalid media anchor summary length"),
            Self::TooManyAnchors(count) => {
                write!(
                    formatter,
                    "media anchor summary has too many anchors ({count})"
                )
            }
        }
    }
}

impl std::error::Error for MediaSummaryDecodeError {}

const AUDIO_SUMMARY_MAGIC: &[u8; 4] = b"SAU2";
const VIDEO_SUMMARY_MAGIC: &[u8; 4] = b"SVI2";
const SUMMARY_FORMAT_VERSION: u16 = 1;

pub fn media_fingerprint_summary_from_record(
    record: &MediaFingerprintRecord,
) -> MediaFingerprintSummary {
    let audio_anchors = audio_anchors_from_record(record);
    let video_anchors = video_anchors_from_record(record);
    MediaFingerprintSummary {
        profile: record.extraction_settings.profile.label().to_owned(),
        settings_hash: media_extraction_settings_hash(&record.extraction_settings),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_summary: (!audio_anchors.is_empty())
            .then(|| encode_audio_anchor_summary(&audio_anchors)),
        video_summary: (!video_anchors.is_empty())
            .then(|| encode_video_anchor_summary(&video_anchors)),
        audio_anchor_count: audio_anchors.len(),
        video_anchor_count: video_anchors.len(),
    }
}

pub fn media_anchor_profile_from_record(record: &MediaFingerprintRecord) -> MediaAnchorProfile {
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: record.extraction_settings.profile.label().to_owned(),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_anchors: audio_anchors_from_record(record),
        video_anchors: video_anchors_from_record(record),
    }
}

pub fn media_anchor_profile_from_summaries(
    profile: impl Into<String>,
    duration_ms: Option<u32>,
    audio_summary: Option<&[u8]>,
    video_summary: Option<&[u8]>,
) -> Result<MediaAnchorProfile, MediaSummaryDecodeError> {
    Ok(MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: profile.into(),
        duration_ms,
        audio_anchors: audio_summary
            .map(decode_audio_anchor_summary)
            .transpose()?
            .unwrap_or_default(),
        video_anchors: video_summary
            .map(decode_video_anchor_summary)
            .transpose()?
            .unwrap_or_default(),
    })
}

pub fn audio_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<AudioAnchor> {
    if !record.audio_anchors.is_empty() {
        return record.audio_anchors.clone();
    }
    let limit = match record.extraction_settings.profile {
        MediaFingerprintProfile::FastAnchorV2 => FAST_AUDIO_ANCHOR_LIMIT,
        MediaFingerprintProfile::FullAnchorV2 => FULL_AUDIO_ANCHOR_LIMIT,
    };
    record
        .audio
        .as_ref()
        .map(|audio| audio_anchors_from_fingerprint(audio, limit))
        .unwrap_or_default()
}

pub fn video_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<VideoAnchor> {
    if !record.video_anchors.is_empty() {
        return record.video_anchors.clone();
    }
    let limit = match record.extraction_settings.profile {
        MediaFingerprintProfile::FastAnchorV2 => FAST_VIDEO_ANCHOR_LIMIT,
        MediaFingerprintProfile::FullAnchorV2 => FULL_VIDEO_ANCHOR_LIMIT,
    };
    record
        .video
        .as_ref()
        .map(|video| video_anchors_from_fingerprint(video, limit))
        .unwrap_or_default()
}

pub fn audio_anchors_from_fingerprint(
    audio: &AudioFingerprint,
    max_anchors: usize,
) -> Vec<AudioAnchor> {
    let tokens = &audio.fingerprint_tokens;
    if tokens.len() < AUDIO_ANCHOR_WINDOW_TOKENS || max_anchors == 0 {
        return Vec::new();
    }
    let duration_ms = audio
        .duration_seconds
        .and_then(duration_seconds_to_millis)
        .unwrap_or_else(|| tokens.len().saturating_mul(1_000).min(u32::MAX as usize) as u32);
    let token_span = tokens
        .len()
        .saturating_sub(AUDIO_ANCHOR_WINDOW_TOKENS)
        .max(1);
    let mut anchors = tokens
        .windows(AUDIO_ANCHOR_WINDOW_TOKENS)
        .enumerate()
        .map(|(index, window)| {
            let hash = stable_hash_u64(window.iter().flat_map(|token| token.to_le_bytes()));
            let t_ms = ((u64::from(duration_ms) * index as u64) / token_span as u64)
                .min(u64::from(u32::MAX)) as u32;
            AudioAnchor {
                bucket: anchor_bucket(hash),
                t_ms,
                weight: 1,
            }
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_audio_anchors(&mut anchors, max_anchors)
}

pub fn video_anchors_from_fingerprint(
    video: &VideoFingerprint,
    max_anchors: usize,
) -> Vec<VideoAnchor> {
    if max_anchors == 0 {
        return Vec::new();
    }
    let mut anchors = video
        .frames
        .iter()
        .flat_map(|frame| {
            let t_ms = frame.timestamp_millis.min(u64::from(u32::MAX)) as u32;
            video_lsh_buckets(frame.hash)
                .into_iter()
                .map(move |bucket| VideoAnchor {
                    bucket,
                    t_ms,
                    hash64: frame.hash,
                    weight: 1,
                })
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_video_anchors(&mut anchors, max_anchors)
}

pub fn encode_audio_anchor_summary(anchors: &[AudioAnchor]) -> Vec<u8> {
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.weight));
    let count = sorted.len().min(MAX_SUMMARY_ANCHORS);
    let mut bytes = Vec::with_capacity(8 + count * 10);
    bytes.extend_from_slice(AUDIO_SUMMARY_MAGIC);
    bytes.extend_from_slice(&SUMMARY_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(count as u16).to_le_bytes());
    let mut previous_t_ms = 0u32;
    for anchor in sorted.into_iter().take(count) {
        let delta_t_ms = anchor.t_ms.saturating_sub(previous_t_ms);
        previous_t_ms = anchor.t_ms;
        bytes.extend_from_slice(&delta_t_ms.to_le_bytes());
        bytes.extend_from_slice(&anchor.bucket.to_le_bytes());
        bytes.extend_from_slice(&anchor.weight.to_le_bytes());
    }
    bytes
}

pub fn decode_audio_anchor_summary(
    bytes: &[u8],
) -> Result<Vec<AudioAnchor>, MediaSummaryDecodeError> {
    if bytes.len() < 8 {
        return Err(MediaSummaryDecodeError::InvalidLength);
    }
    if &bytes[0..4] != AUDIO_SUMMARY_MAGIC {
        return Err(MediaSummaryDecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != SUMMARY_FORMAT_VERSION {
        return Err(MediaSummaryDecodeError::UnsupportedVersion(version));
    }
    let count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if count > MAX_SUMMARY_ANCHORS {
        return Err(MediaSummaryDecodeError::TooManyAnchors(count));
    }
    let expected = 8 + count * 10;
    if bytes.len() != expected {
        return Err(MediaSummaryDecodeError::InvalidLength);
    }
    let mut anchors = Vec::with_capacity(count);
    let mut cursor = 8;
    let mut t_ms = 0u32;
    for _ in 0..count {
        let delta_t_ms = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let bucket = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        t_ms = t_ms.saturating_add(delta_t_ms);
        anchors.push(AudioAnchor {
            bucket,
            t_ms,
            weight,
        });
    }
    Ok(anchors)
}

pub fn encode_video_anchor_summary(anchors: &[VideoAnchor]) -> Vec<u8> {
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.hash64, anchor.weight));
    let count = sorted.len().min(MAX_SUMMARY_ANCHORS);
    let mut bytes = Vec::with_capacity(8 + count * 18);
    bytes.extend_from_slice(VIDEO_SUMMARY_MAGIC);
    bytes.extend_from_slice(&SUMMARY_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(count as u16).to_le_bytes());
    let mut previous_t_ms = 0u32;
    for anchor in sorted.into_iter().take(count) {
        let delta_t_ms = anchor.t_ms.saturating_sub(previous_t_ms);
        previous_t_ms = anchor.t_ms;
        bytes.extend_from_slice(&delta_t_ms.to_le_bytes());
        bytes.extend_from_slice(&anchor.bucket.to_le_bytes());
        bytes.extend_from_slice(&anchor.hash64.to_le_bytes());
        bytes.extend_from_slice(&anchor.weight.to_le_bytes());
    }
    bytes
}

pub fn decode_video_anchor_summary(
    bytes: &[u8],
) -> Result<Vec<VideoAnchor>, MediaSummaryDecodeError> {
    if bytes.len() < 8 {
        return Err(MediaSummaryDecodeError::InvalidLength);
    }
    if &bytes[0..4] != VIDEO_SUMMARY_MAGIC {
        return Err(MediaSummaryDecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != SUMMARY_FORMAT_VERSION {
        return Err(MediaSummaryDecodeError::UnsupportedVersion(version));
    }
    let count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if count > MAX_SUMMARY_ANCHORS {
        return Err(MediaSummaryDecodeError::TooManyAnchors(count));
    }
    let expected = 8 + count * 18;
    if bytes.len() != expected {
        return Err(MediaSummaryDecodeError::InvalidLength);
    }
    let mut anchors = Vec::with_capacity(count);
    let mut cursor = 8;
    let mut t_ms = 0u32;
    for _ in 0..count {
        let delta_t_ms = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let bucket = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let hash64 = read_u64_le(bytes, cursor)?;
        cursor += 8;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        t_ms = t_ms.saturating_add(delta_t_ms);
        anchors.push(VideoAnchor {
            bucket,
            t_ms,
            hash64,
            weight,
        });
    }
    Ok(anchors)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireSignatureV2 {
    pub schema: String,
    pub profiles: Vec<MediaMatchWireAnchorProfile>,
}

impl Default for MediaMatchWireSignatureV2 {
    fn default() -> Self {
        Self {
            schema: MEDIA_MATCH_WIRE_SCHEMA_V2.to_owned(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireAnchorProfile {
    pub profile: String,
    pub algorithm_version: u32,
    pub duration_ms: Option<u32>,
    pub audio: Option<MediaMatchWireAnchorBlock>,
    pub video: Option<MediaMatchWireAnchorBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireAnchorBlock {
    pub algorithm: String,
    pub time_base_ms: u32,
    pub anchors: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaMatchCache {
    pub records: BTreeMap<String, MediaFingerprintRecord>,
}

impl MediaMatchCache {
    pub fn insert(&mut self, record: MediaFingerprintRecord) {
        self.records
            .insert(record.identity.normalized_path.clone(), record);
    }

    pub fn get_valid(
        &self,
        path: impl AsRef<Path>,
        modified_unix_millis: u64,
        size_bytes: u64,
        algorithm_version: u32,
        extraction_settings: &MediaExtractionSettings,
    ) -> Option<&MediaFingerprintRecord> {
        let normalized_path = normalize_media_path(path);
        let record = self.records.get(&normalized_path)?;
        record
            .valid_for(
                &normalized_path,
                modified_unix_millis,
                size_bytes,
                algorithm_version,
                extraction_settings,
            )
            .then_some(record)
    }

    pub fn remove_stale(
        &mut self,
        path: impl AsRef<Path>,
        modified_unix_millis: u64,
        size_bytes: u64,
        algorithm_version: u32,
        extraction_settings: &MediaExtractionSettings,
    ) -> bool {
        let normalized_path = normalize_media_path(path);
        let stale = self.records.get(&normalized_path).is_some_and(|record| {
            !record.valid_for(
                &normalized_path,
                modified_unix_millis,
                size_bytes,
                algorithm_version,
                extraction_settings,
            )
        });
        if stale {
            self.records.remove(&normalized_path);
        }
        stale
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }
}

pub fn media_match_wire_signature_from_records(
    records: &[MediaFingerprintRecord],
) -> MediaMatchWireSignatureV2 {
    let mut signature = MediaMatchWireSignatureV2::default();
    for record in records {
        if let Some(profile) = media_match_wire_anchor_profile_from_record(record) {
            signature.profiles.push(profile);
        }
    }
    signature
}

pub fn media_match_wire_value_from_records(records: &[MediaFingerprintRecord]) -> Option<Value> {
    let signature = media_match_wire_signature_from_records(records);
    if signature.profiles.is_empty() {
        return None;
    }
    let value = serde_json::to_value(&signature).ok()?;
    let bytes = serde_json::to_vec(&value).ok()?;
    (bytes.len() <= MEDIA_MATCH_WIRE_MAX_BYTES).then_some(value)
}

pub fn media_match_wire_signature_from_value(
    value: &Value,
) -> Result<MediaMatchWireSignatureV2, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("media match wire signature could not serialize: {error}"))?;
    if bytes.len() > MEDIA_MATCH_WIRE_MAX_BYTES {
        return Err("media match wire signature exceeds the payload limit".to_owned());
    }
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| "media match wire signature has no schema".to_owned())?;
    if schema != MEDIA_MATCH_WIRE_SCHEMA_V2 {
        return Err("media match wire signature schema is unsupported".to_owned());
    }
    let signature: MediaMatchWireSignatureV2 = serde_json::from_value(value.clone())
        .map_err(|error| format!("media match v2 wire signature is invalid: {error}"))?;
    if signature.profiles.is_empty() {
        return Err("media match wire signature has no profiles".to_owned());
    }
    for profile in &signature.profiles {
        media_anchor_profile_from_wire_profile(profile)?;
    }
    Ok(signature)
}

pub fn decide_media_match_against_wire_signature(
    query: &MediaFingerprintRecord,
    signature: &MediaMatchWireSignatureV2,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let query_profile = media_anchor_profile_from_record(query);
    let mut ranked = signature
        .profiles
        .iter()
        .filter_map(|profile| media_anchor_profile_from_wire_profile(profile).ok())
        .map(|candidate| decide_media_match_anchors(&query_profile, &candidate, settings))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        media_match_tier_rank(right.tier).cmp(&media_match_tier_rank(left.tier))
    });
    ranked
        .into_iter()
        .next()
        .unwrap_or_else(|| MediaMatchDecision::unknown("no comparable media match wire profiles"))
}

fn media_match_wire_anchor_profile_from_record(
    record: &MediaFingerprintRecord,
) -> Option<MediaMatchWireAnchorProfile> {
    let anchor_profile = media_anchor_profile_from_record(record);
    media_match_wire_anchor_profile_from_anchor_profile(
        &anchor_profile,
        &record.extraction_settings.audio_algorithm,
        &record.extraction_settings.video_algorithm,
    )
}

pub fn media_match_wire_anchor_profile_from_anchor_profile(
    profile: &MediaAnchorProfile,
    audio_algorithm: &str,
    video_algorithm: &str,
) -> Option<MediaMatchWireAnchorProfile> {
    if profile.is_empty() {
        return None;
    }
    let audio_summary = (!profile.audio_anchors.is_empty())
        .then(|| encode_audio_anchor_summary(&profile.audio_anchors));
    let video_summary = (!profile.video_anchors.is_empty())
        .then(|| encode_video_anchor_summary(&profile.video_anchors));
    Some(MediaMatchWireAnchorProfile {
        profile: profile.profile.clone(),
        algorithm_version: profile.version,
        duration_ms: profile.duration_ms,
        audio: audio_summary.map(|summary| MediaMatchWireAnchorBlock {
            algorithm: audio_algorithm.to_owned(),
            time_base_ms: 1,
            anchors: base64::engine::general_purpose::STANDARD.encode(summary),
        }),
        video: video_summary.map(|summary| MediaMatchWireAnchorBlock {
            algorithm: video_algorithm.to_owned(),
            time_base_ms: 1,
            anchors: base64::engine::general_purpose::STANDARD.encode(summary),
        }),
    })
}

pub fn media_anchor_profile_from_wire_profile(
    profile: &MediaMatchWireAnchorProfile,
) -> Result<MediaAnchorProfile, String> {
    if profile.algorithm_version != MEDIA_MATCH_ANCHOR_VERSION {
        return Err(format!(
            "media match v2 profile '{}' uses unsupported algorithm version {}",
            profile.profile, profile.algorithm_version
        ));
    }
    let expected_settings = media_extraction_settings_for_profile_label(&profile.profile)
        .ok_or_else(|| format!("media match v2 profile '{}' is unknown", profile.profile))?;
    if let Some(block) = profile.audio.as_ref() {
        validate_wire_anchor_block(
            "audio",
            block,
            &expected_settings.audio_algorithm,
            profile.profile.as_str(),
        )?;
    }
    if let Some(block) = profile.video.as_ref() {
        validate_wire_anchor_block(
            "video",
            block,
            &expected_settings.video_algorithm,
            profile.profile.as_str(),
        )?;
    }
    let audio_summary = profile
        .audio
        .as_ref()
        .map(|block| {
            base64::engine::general_purpose::STANDARD
                .decode(block.anchors.as_bytes())
                .map_err(|error| format!("media match v2 audio anchors are not base64: {error}"))
        })
        .transpose()?;
    let video_summary = profile
        .video
        .as_ref()
        .map(|block| {
            base64::engine::general_purpose::STANDARD
                .decode(block.anchors.as_bytes())
                .map_err(|error| format!("media match v2 video anchors are not base64: {error}"))
        })
        .transpose()?;
    media_anchor_profile_from_summaries(
        profile.profile.clone(),
        profile.duration_ms,
        audio_summary.as_deref(),
        video_summary.as_deref(),
    )
    .map_err(|error| format!("media match v2 anchors could not decode: {error}"))
}

fn media_extraction_settings_for_profile_label(label: &str) -> Option<MediaExtractionSettings> {
    match label {
        "fast-anchor-v2" => Some(MediaExtractionSettings::fast_anchor_v2()),
        "full-anchor-v2" => Some(MediaExtractionSettings::full_anchor_v2()),
        _ => None,
    }
}

fn validate_wire_anchor_block(
    modality: &str,
    block: &MediaMatchWireAnchorBlock,
    expected_algorithm: &str,
    profile_label: &str,
) -> Result<(), String> {
    if block.algorithm != expected_algorithm {
        return Err(format!(
            "media match v2 {modality} algorithm '{}' is unsupported for profile '{profile_label}'",
            block.algorithm
        ));
    }
    if block.time_base_ms != 1 {
        return Err(format!(
            "media match v2 {modality} time base {}ms is unsupported",
            block.time_base_ms
        ));
    }
    Ok(())
}

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
    let started_at = Instant::now();
    let audio_result = extract_audio_fingerprint_with_length(
        &tools.fpcalc,
        path,
        extraction_settings,
        cancel_flag,
    );
    report.invocations.fpcalc = 1;
    report.timings.audio_millis = started_at.elapsed().as_millis();
    let audio = match audio_result {
        Ok(audio) => Some(audio),
        Err(error) => {
            report.audio_error = Some(error.to_string());
            None
        }
    };
    let started_at = Instant::now();
    let video_result = extract_video_fingerprint_with_cancellation(
        &tools.ffmpeg,
        path,
        duration_seconds,
        extraction_settings,
        cancel_flag,
    );
    report.invocations.ffmpeg = 1;
    report.timings.video_millis = started_at.elapsed().as_millis();
    let video = match video_result {
        Ok(video) => Some(video),
        Err(error) => {
            report.video_error = Some(error.to_string());
            None
        }
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
        audio,
        video,
        audio_anchors: Vec::new(),
        video_anchors: Vec::new(),
        audio_error,
        video_error,
    };
    record.audio_anchors = audio_anchors_from_record(&record);
    record.video_anchors = video_anchors_from_record(&record);
    let summary = media_fingerprint_summary_from_record(&record);
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

pub fn extract_audio_fingerprint(
    fpcalc: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
) -> Result<AudioFingerprint, MediaFingerprintError> {
    extract_audio_fingerprint_with_length(
        fpcalc,
        media_path,
        &MediaExtractionSettings::full_anchor_v2(),
        None,
    )
}

fn extract_audio_fingerprint_with_length(
    fpcalc: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<AudioFingerprint, MediaFingerprintError> {
    let mut args = vec!["-raw".into()];
    match extraction_settings.audio_sample_seconds {
        0 => {
            args.push("-length".into());
            args.push("0".into());
        }
        sample_seconds => {
            args.push("-length".into());
            args.push(sample_seconds.to_string().into());
        }
    }
    args.push(media_path.as_ref().as_os_str().to_os_string());
    let output = run_tool_output("fpcalc", fpcalc.as_ref(), args, cancel_flag, FPCALC_TIMEOUT)?;
    ensure_tool_success("fpcalc", &output)?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_fpcalc_output(&text).ok_or_else(|| MediaFingerprintError::InvalidToolOutput {
        tool: "fpcalc",
        reason: "missing raw Chromaprint fingerprint tokens".to_owned(),
    })
}

pub fn extract_video_fingerprint(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    extraction_settings: &MediaExtractionSettings,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    extract_video_fingerprint_with_cancellation(
        ffmpeg,
        media_path,
        duration_seconds,
        extraction_settings,
        None,
    )
}

fn extract_video_fingerprint_with_cancellation(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    match extraction_settings.profile {
        MediaFingerprintProfile::FastAnchorV2 => extract_fast_video_fingerprint(
            ffmpeg,
            media_path,
            duration_seconds,
            extraction_settings,
            cancel_flag,
        ),
        MediaFingerprintProfile::FullAnchorV2 => extract_full_video_fingerprint(
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
            "error".into(),
            "-nostdin".into(),
            "-i".into(),
            media_path.as_ref().as_os_str().to_os_string(),
            "-vf".into(),
            format!(
                "fps=1/{interval},scale={VIDEO_FRAME_WIDTH}:{VIDEO_FRAME_HEIGHT}:flags=bicubic,format=gray"
            )
            .into(),
            "-frames:v".into(),
            extraction_settings.max_frames.max(1).to_string().into(),
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

    let mut frames = Vec::new();
    for (index, chunk) in output.stdout.chunks_exact(VIDEO_FRAME_BYTES).enumerate() {
        let hash =
            pdq_style_luma_hash(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, chunk).ok_or_else(|| {
                MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason:
                        "raw grayscale frame size did not match the requested extraction geometry"
                            .to_owned(),
                }
            })?;
        frames.push(FrameFingerprint::new(
            index as f64 * f64::from(interval),
            hash,
        ));
    }

    if frames.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "no raw grayscale frames were extracted".to_owned(),
        });
    }

    Ok(VideoFingerprint {
        duration_seconds: duration_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round().min(f64::from(u32::MAX)) as u32),
        frames,
    })
}

fn extract_fast_video_fingerprint(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<VideoFingerprint, MediaFingerprintError> {
    let timestamps = fast_video_sample_timestamps(duration_seconds, extraction_settings.max_frames);
    if timestamps.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "no fast video sample timestamps were selected".to_owned(),
        });
    }
    if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(MediaFingerprintError::Cancelled { tool: "ffmpeg" });
    }
    let start = timestamps.first().copied().unwrap_or(0.0).max(0.0);
    let end = timestamps
        .last()
        .copied()
        .map(|value| value + 1.0)
        .unwrap_or(start + 1.0)
        .max(start + 1.0);
    let step = timestamps
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).max(1.0))
        .min_by(f64::total_cmp)
        .unwrap_or(60.0)
        .max(1.0);
    let filter = format!(
        "trim=start={start:.3}:end={end:.3},select='isnan(prev_selected_t)+gte(t-prev_selected_t\\,{step:.3})',showinfo,scale={VIDEO_FRAME_WIDTH}:{VIDEO_FRAME_HEIGHT}:flags=bicubic,format=gray"
    );
    let output = run_tool_output(
        "ffmpeg",
        ffmpeg.as_ref(),
        [
            "-v".into(),
            "info".into(),
            "-nostdin".into(),
            "-ss".into(),
            format!("{start:.3}").into(),
            "-copyts".into(),
            "-i".into(),
            media_path.as_ref().as_os_str().to_os_string(),
            "-vf".into(),
            filter.into(),
            "-frames:v".into(),
            timestamps.len().to_string().into(),
            "-vsync".into(),
            "vfr".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "gray".into(),
            "-".into(),
        ],
        cancel_flag,
        FFMPEG_FAST_FRAME_TIMEOUT,
    )?;
    ensure_tool_success("ffmpeg", &output)?;
    if output.stdout.len() % VIDEO_FRAME_BYTES != 0 {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "raw grayscale frame output had a partial trailing frame".to_owned(),
        });
    }
    let frame_count = output.stdout.len() / VIDEO_FRAME_BYTES;
    let stderr = String::from_utf8_lossy(&output.stderr);
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
    for (index, chunk) in output.stdout.chunks_exact(VIDEO_FRAME_BYTES).enumerate() {
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

    Ok(VideoFingerprint {
        duration_seconds: duration_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round().min(f64::from(u32::MAX)) as u32),
        frames,
    })
}

pub fn fast_video_sample_timestamps(
    duration_seconds: Option<f64>,
    requested_frames: usize,
) -> Vec<f64> {
    let count = requested_frames.clamp(1, FAST_VIDEO_SAMPLE_FRAMES);
    let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 0.0) else {
        return (0..count).map(|index| (index as f64) * 60.0).collect();
    };
    if count == 1 {
        return vec![(duration / 2.0).max(0.0)];
    }
    let edge_margin = if duration >= 300.0 {
        duration.mul_add(0.10, 0.0).min(120.0)
    } else if duration >= 120.0 {
        duration * 0.08
    } else {
        0.0
    };
    let start = edge_margin.min(duration / 3.0);
    let end = (duration - edge_margin).max(start);
    let step = (end - start) / ((count - 1) as f64);
    (0..count)
        .map(|index| start + (step * index as f64))
        .collect()
}

pub fn rank_media_match_candidates<'a>(
    query: &MediaFingerprintRecord,
    candidates: impl IntoIterator<Item = &'a MediaFingerprintRecord>,
    settings: &MediaMatchSettings,
) -> Vec<MediaMatchCandidateDecision> {
    let mut decisions = candidates
        .into_iter()
        .map(|candidate| MediaMatchCandidateDecision {
            candidate_path: candidate.identity.normalized_path.clone(),
            decision: decide_media_match(query, candidate, settings),
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| {
        media_match_tier_rank(right.decision.tier)
            .cmp(&media_match_tier_rank(left.decision.tier))
            .then_with(|| {
                right
                    .decision
                    .evidence
                    .video
                    .as_ref()
                    .map(|video| video.query_coverage.min(video.candidate_coverage))
                    .unwrap_or(0.0)
                    .total_cmp(
                        &left
                            .decision
                            .evidence
                            .video
                            .as_ref()
                            .map(|video| video.query_coverage.min(video.candidate_coverage))
                            .unwrap_or(0.0),
                    )
            })
            .then_with(|| left.candidate_path.cmp(&right.candidate_path))
    });
    decisions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnchorMatchPair {
    query_t_ms: u32,
    candidate_t_ms: u32,
    modality: AnchorModality,
    weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnchorModality {
    Audio,
    Video,
}

#[derive(Debug, Clone)]
struct AnchorScaleOffsetFit {
    offset_ms: i64,
    scale_ppm: i32,
    drift_ratio: f64,
    aligned: Vec<AnchorMatchPair>,
}

#[derive(Debug, Clone)]
struct AnchorFitCandidate {
    score: u32,
    inlier_count: usize,
    span_ms: u32,
    max_residual_ms: f64,
    scale: f64,
    offset: f64,
    aligned: Vec<AnchorMatchPair>,
}

pub fn decide_media_match_anchors(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let mut evidence = MediaMatchEvidence {
        metadata: MetadataMatchEvidence {
            duration_delta_seconds: query.duration_ms.zip(candidate.duration_ms).map(
                |(left, right)| (i64::from(left) - i64::from(right)).unsigned_abs() as f64 / 1000.0,
            ),
            duration_within_tolerance: query.duration_ms.zip(candidate.duration_ms).map(
                |(left, right)| {
                    !settings.runtime_tolerance_enabled
                        || (i64::from(left) - i64::from(right)).abs() as f64 / 1000.0
                            <= settings.runtime_tolerance_seconds
                },
            ),
            ..MetadataMatchEvidence::default()
        },
        audio: None,
        video: None,
        alignment: None,
        notes: Vec::new(),
    };

    if !settings.fingerprinting_enabled {
        evidence
            .notes
            .push("fingerprinting disabled; metadata is diagnostic only".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "fingerprinting disabled; no same-media decision",
        );
    }

    if query.is_empty() || candidate.is_empty() {
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "no comparable media match anchors",
        );
    }

    let pairs = collect_anchor_match_pairs(query, candidate);
    if pairs.is_empty() {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "anchor lookup found no shared timeline evidence",
        );
    }

    let Some((best_offset_ms, best_weight, second_weight)) = dominant_anchor_offset(&pairs) else {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "anchor offsets did not form a dominant hypothesis",
        );
    };
    let Some(fit) = fit_anchor_scale_offset(&pairs, best_offset_ms) else {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "no anchors fit the dominant offset",
        );
    };
    let aligned = fit.aligned;

    let audio_pairs = aligned
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Audio)
        .count();
    let video_pairs = aligned
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Video)
        .count();
    let span_ms = aligned_anchor_span_ms(&aligned);
    let drift_ratio = fit.drift_ratio;
    let second_best_offset_margin = if best_weight > 0 {
        1.0 - (f64::from(second_weight) / f64::from(best_weight))
    } else {
        0.0
    };
    let query_audio_coverage = anchor_coverage(audio_pairs, query.audio_anchors.len());
    let candidate_audio_coverage = anchor_coverage(audio_pairs, candidate.audio_anchors.len());
    let query_video_coverage = anchor_coverage(video_pairs, query.video_anchors.len());
    let candidate_video_coverage = anchor_coverage(video_pairs, candidate.video_anchors.len());
    if !query.audio_anchors.is_empty() && !candidate.audio_anchors.is_empty() {
        evidence.audio = Some(AudioMatchEvidence {
            similarity: query_audio_coverage.min(candidate_audio_coverage),
            shared_token_ratio: query_audio_coverage.min(candidate_audio_coverage),
            duration_delta_seconds: evidence.metadata.duration_delta_seconds,
        });
    }
    if !query.video_anchors.is_empty() && !candidate.video_anchors.is_empty() {
        evidence.video = Some(VideoMatchEvidence {
            aligned_pairs: video_pairs,
            query_coverage: query_video_coverage,
            candidate_coverage: candidate_video_coverage,
            best_offset_seconds: fit.offset_ms as f64 / 1000.0,
            drift_ratio,
            mean_hamming_distance: 0.0,
        });
    }
    let (first_query_ms, last_query_ms, first_candidate_ms, last_candidate_ms) =
        aligned_anchor_bounds(&aligned);
    evidence.alignment = Some(MediaTimelineAlignment {
        offset_seconds: fit.offset_ms as f64 / 1000.0,
        scale_ppm: fit.scale_ppm,
        drift_ratio,
        aligned_pairs: aligned.len(),
        aligned_audio_anchors: audio_pairs,
        aligned_video_anchors: video_pairs,
        aligned_span_seconds: span_ms as f64 / 1000.0,
        second_best_offset_margin,
        first_query_second: first_query_ms as f64 / 1000.0,
        last_query_second: last_query_ms as f64 / 1000.0,
        first_candidate_second: first_candidate_ms as f64 / 1000.0,
        last_candidate_second: last_candidate_ms as f64 / 1000.0,
    });

    let duration_ok = evidence.metadata.duration_within_tolerance.unwrap_or(true);
    let drift_ok = drift_ratio <= settings.max_alignment_drift_ratio;
    let margin_ok = second_best_offset_margin >= 0.35 || second_weight == 0;
    let span_seconds = span_ms as f64 / 1000.0;
    let continuity_ok = aligned_anchor_largest_gap_ratio(&aligned) <= 0.65;
    let shorter_duration_seconds = query
        .duration_ms
        .zip(candidate.duration_ms)
        .map(|(left, right)| left.min(right) as f64 / 1000.0);
    let meaningful_span = shorter_duration_seconds
        .map(|duration| {
            let target = if duration >= 2400.0 {
                (duration * 0.30).min(600.0)
            } else {
                (duration * 0.25).min(300.0)
            };
            span_seconds >= target.clamp(30.0, 300.0)
        })
        .unwrap_or(span_seconds >= 30.0);
    let both_modalities = audio_pairs >= 3 && video_pairs >= 3;
    let very_strong_single_modality =
        (audio_pairs >= 16 || video_pairs >= 10) && meaningful_span && margin_ok && continuity_ok;
    let weak_evidence = audio_pairs >= 2 || video_pairs >= 2 || aligned.len() >= 3;

    if both_modalities && meaningful_span && drift_ok && margin_ok && duration_ok && continuity_ok {
        return decision(
            MediaMatchTier::Strong,
            evidence,
            "anchor timelines strongly align across audio and video",
        );
    }
    if very_strong_single_modality && drift_ok && (duration_ok || span_seconds >= 300.0) {
        return decision(
            MediaMatchTier::Strong,
            evidence,
            "anchor timeline strongly aligns in one modality over a broad span",
        );
    }
    if (both_modalities && drift_ok && margin_ok)
        || (weak_evidence && meaningful_span && drift_ok && continuity_ok)
    {
        return decision(
            MediaMatchTier::Probable,
            evidence,
            "anchor timelines align but evidence is not strong enough for autoplay",
        );
    }
    if weak_evidence {
        return decision(
            MediaMatchTier::Weak,
            evidence,
            "partial anchor timeline evidence",
        );
    }
    decision(
        MediaMatchTier::Reject,
        evidence,
        "anchor timeline evidence is insufficient",
    )
}

pub fn decide_media_match(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let mut evidence = MediaMatchEvidence {
        metadata: metadata_evidence(query, candidate, settings),
        audio: None,
        video: None,
        alignment: None,
        notes: Vec::new(),
    };

    if query.algorithm_version != candidate.algorithm_version {
        evidence.notes.push("algorithm version mismatch".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "algorithm version mismatch",
        );
    }

    if query.identity.normalized_path == candidate.identity.normalized_path
        && query.identity.modified_unix_millis == candidate.identity.modified_unix_millis
        && query.identity.size_bytes == candidate.identity.size_bytes
    {
        return decision(
            MediaMatchTier::Exact,
            evidence,
            "same path, modified time, and size",
        );
    }

    if !settings.fingerprinting_enabled {
        evidence
            .notes
            .push("fingerprinting disabled; metadata is diagnostic only".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "fingerprinting disabled; no same-media decision",
        );
    }

    if query.container_fingerprint == candidate.container_fingerprint
        && query.identity.size_bytes == candidate.identity.size_bytes
        && query.identity.size_bytes > 0
    {
        return decision(
            MediaMatchTier::Exact,
            evidence,
            "same container fingerprint and size",
        );
    }

    let query_profile = media_anchor_profile_from_record(query);
    let candidate_profile = media_anchor_profile_from_record(candidate);
    decide_media_match_anchors(&query_profile, &candidate_profile, settings)
}

pub fn compare_audio_fingerprints(
    query: &AudioFingerprint,
    candidate: &AudioFingerprint,
) -> AudioMatchEvidence {
    let query_set = query
        .fingerprint_tokens
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let candidate_set = candidate
        .fingerprint_tokens
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let intersection = query_set.intersection(&candidate_set).count() as f64;
    let union = query_set.union(&candidate_set).count() as f64;
    let shared_token_ratio = if union > 0.0 {
        intersection / union
    } else {
        0.0
    };

    let sequence_similarity =
        longest_common_subsequence_ratio(&query.fingerprint_tokens, &candidate.fingerprint_tokens);
    let similarity = (shared_token_ratio * 0.35) + (sequence_similarity * 0.65);
    AudioMatchEvidence {
        similarity,
        shared_token_ratio,
        duration_delta_seconds: query
            .duration_seconds
            .zip(candidate.duration_seconds)
            .map(|(left, right)| (left - right).abs()),
    }
}

pub fn align_video_fingerprints(
    query: &VideoFingerprint,
    candidate: &VideoFingerprint,
) -> Option<VideoMatchEvidence> {
    if query.frames.is_empty() || candidate.frames.is_empty() {
        return None;
    }

    let mut all_pairs = Vec::new();
    for (query_index, query_frame) in query.frames.iter().enumerate() {
        for (candidate_index, candidate_frame) in candidate.frames.iter().enumerate() {
            let distance = frame_hash_distance(query_frame.hash, candidate_frame.hash);
            if distance <= DEFAULT_FRAME_HAMMING_THRESHOLD {
                all_pairs.push((query_index, candidate_index, distance));
            }
        }
    }

    all_pairs.sort_by_key(|(query_index, candidate_index, distance)| {
        (*query_index, *distance, *candidate_index)
    });

    let mut used_query = HashSet::new();
    let mut used_candidate = HashSet::new();
    let mut aligned = Vec::new();
    for (query_index, candidate_index, distance) in all_pairs {
        if used_query.contains(&query_index) || used_candidate.contains(&candidate_index) {
            continue;
        }
        let query_time = query.frames[query_index].timestamp_seconds();
        let candidate_time = candidate.frames[candidate_index].timestamp_seconds();
        used_query.insert(query_index);
        used_candidate.insert(candidate_index);
        aligned.push((query_time, candidate_time, distance));
    }

    if aligned.is_empty() {
        return None;
    }

    aligned.sort_by(|left, right| left.0.total_cmp(&right.0));
    let first = aligned[0];
    let last = aligned[aligned.len() - 1];
    let mut offsets = aligned
        .iter()
        .map(|(query_time, candidate_time, _)| candidate_time - query_time)
        .collect::<Vec<_>>();
    offsets.sort_by(f64::total_cmp);
    let best_offset_seconds = offsets[offsets.len() / 2];
    let query_span = (last.0 - first.0).abs().max(1.0);
    let offset_drift = ((last.1 - last.0) - (first.1 - first.0)).abs();
    let drift_ratio = offset_drift / query_span;
    let distance_sum: u32 = aligned.iter().map(|(_, _, distance)| *distance).sum();

    Some(VideoMatchEvidence {
        aligned_pairs: aligned.len(),
        query_coverage: aligned.len() as f64 / query.frames.len() as f64,
        candidate_coverage: aligned.len() as f64 / candidate.frames.len() as f64,
        best_offset_seconds,
        drift_ratio,
        mean_hamming_distance: distance_sum as f64 / aligned.len() as f64,
    })
}

pub fn parse_fpcalc_output(output: &str) -> Option<AudioFingerprint> {
    let mut duration_seconds = None;
    let mut tokens = Vec::new();

    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "DURATION" => {
                duration_seconds = value.trim().parse::<f64>().ok();
            }
            "FINGERPRINT" => {
                tokens = value
                    .split(',')
                    .filter_map(|token| token.trim().parse::<u32>().ok())
                    .collect();
            }
            _ => {}
        }
    }

    (!tokens.is_empty()).then_some(AudioFingerprint {
        duration_seconds,
        fingerprint_tokens: tokens,
    })
}

fn parse_ffmpeg_showinfo_pts_times(output: &str) -> Vec<f64> {
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

fn run_tool_output<I>(
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
    let started_at = Instant::now();

    loop {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaFingerprintError::Cancelled { tool });
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaFingerprintError::TimedOut {
                tool,
                timeout_seconds: timeout.as_secs().max(1),
            });
        }
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child.wait_with_output().map_err(|error| {
                    MediaFingerprintError::ToolFailed {
                        tool,
                        status: None,
                        stderr: error.to_string(),
                    }
                });
            }
            Ok(None) => thread::sleep(MEDIA_TOOL_POLL_INTERVAL),
            Err(error) => {
                return Err(MediaFingerprintError::ToolFailed {
                    tool,
                    status: None,
                    stderr: error.to_string(),
                });
            }
        }
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

fn collect_anchor_match_pairs(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
) -> Vec<AnchorMatchPair> {
    let mut candidate_audio = HashMap::<u32, Vec<&AudioAnchor>>::new();
    for anchor in &candidate.audio_anchors {
        candidate_audio
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
    }
    let mut candidate_video = HashMap::<u32, Vec<&VideoAnchor>>::new();
    for anchor in &candidate.video_anchors {
        candidate_video
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
    }
    let mut pairs = Vec::new();
    for query_anchor in &query.audio_anchors {
        if let Some(candidate_anchors) = candidate_audio.get(&query_anchor.bucket) {
            for candidate_anchor in candidate_anchors {
                pairs.push(AnchorMatchPair {
                    query_t_ms: query_anchor.t_ms,
                    candidate_t_ms: candidate_anchor.t_ms,
                    modality: AnchorModality::Audio,
                    weight: query_anchor.weight.min(candidate_anchor.weight),
                });
            }
        }
    }
    let mut seen_video_pairs = HashSet::<(u32, u32, u64, u64)>::new();
    for query_anchor in &query.video_anchors {
        if let Some(candidate_anchors) = candidate_video.get(&query_anchor.bucket) {
            for candidate_anchor in candidate_anchors {
                if !video_anchor_hashes_match(query_anchor.hash64, candidate_anchor.hash64) {
                    continue;
                }
                if !seen_video_pairs.insert((
                    query_anchor.t_ms,
                    candidate_anchor.t_ms,
                    query_anchor.hash64,
                    candidate_anchor.hash64,
                )) {
                    continue;
                }
                pairs.push(AnchorMatchPair {
                    query_t_ms: query_anchor.t_ms,
                    candidate_t_ms: candidate_anchor.t_ms,
                    modality: AnchorModality::Video,
                    weight: query_anchor.weight.min(candidate_anchor.weight),
                });
            }
        }
    }
    pairs
}

fn dominant_anchor_offset(pairs: &[AnchorMatchPair]) -> Option<(i64, u32, u32)> {
    let mut bins = HashMap::<i64, u32>::new();
    for pair in pairs {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        let bin = rounded_offset_bin(offset);
        *bins.entry(bin).or_default() += u32::from(pair.weight.max(1));
    }
    let mut ranked = bins.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (best_bin, best_weight) = ranked.first().copied()?;
    let second_weight = ranked.get(1).map(|(_, weight)| *weight).unwrap_or(0);
    Some((
        best_bin * DEFAULT_ANCHOR_OFFSET_BIN_MS,
        best_weight,
        second_weight,
    ))
}

fn rounded_offset_bin(offset_ms: i64) -> i64 {
    if offset_ms >= 0 {
        (offset_ms + (DEFAULT_ANCHOR_OFFSET_BIN_MS / 2)) / DEFAULT_ANCHOR_OFFSET_BIN_MS
    } else {
        (offset_ms - (DEFAULT_ANCHOR_OFFSET_BIN_MS / 2)) / DEFAULT_ANCHOR_OFFSET_BIN_MS
    }
}

fn fit_anchor_scale_offset(
    pairs: &[AnchorMatchPair],
    voted_offset_ms: i64,
) -> Option<AnchorScaleOffsetFit> {
    let seeded = pairs
        .iter()
        .copied()
        .filter(|pair| {
            let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
            (offset - voted_offset_ms).abs() <= DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
        })
        .collect::<Vec<_>>();
    if seeded.is_empty() {
        return None;
    }

    let mut candidates = vec![(1.0, voted_offset_ms as f64)];
    for (left_index, left) in seeded.iter().enumerate() {
        for right in seeded.iter().skip(left_index + 1) {
            let query_delta = f64::from(right.query_t_ms) - f64::from(left.query_t_ms);
            if query_delta.abs() < 10_000.0 {
                continue;
            }
            let candidate_delta = f64::from(right.candidate_t_ms) - f64::from(left.candidate_t_ms);
            let scale = candidate_delta / query_delta;
            if !(0.95..=1.05).contains(&scale) {
                continue;
            }
            let offset = f64::from(left.candidate_t_ms) - (scale * f64::from(left.query_t_ms));
            candidates.push((scale, offset));
        }
    }

    let mut best: Option<AnchorFitCandidate> = None;
    for (scale, offset) in candidates {
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.is_empty() {
            continue;
        }
        let (scale, offset) = least_squares_anchor_fit(&inliers).unwrap_or((scale, offset));
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.is_empty() {
            continue;
        }
        let score = inliers
            .iter()
            .map(|pair| u32::from(pair.weight.max(1)))
            .sum::<u32>();
        let span = aligned_anchor_span_ms(&inliers);
        let max_residual = max_anchor_fit_residual_ms(&inliers, scale, offset);
        let candidate = AnchorFitCandidate {
            score,
            inlier_count: inliers.len(),
            span_ms: span,
            max_residual_ms: max_residual,
            scale,
            offset,
            aligned: inliers,
        };
        let replace = best.as_ref().is_none_or(|current| {
            candidate
                .score
                .cmp(&current.score)
                .then_with(|| candidate.inlier_count.cmp(&current.inlier_count))
                .then_with(|| candidate.span_ms.cmp(&current.span_ms))
                .then_with(|| {
                    current
                        .max_residual_ms
                        .total_cmp(&candidate.max_residual_ms)
                })
                .is_gt()
        });
        if replace {
            best = Some(candidate);
        }
    }

    let best = best?;
    let drift_ratio = if best.span_ms > 0 {
        best.max_residual_ms / f64::from(best.span_ms)
    } else {
        0.0
    };
    let scale_ppm = (best.scale * 1_000_000.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    Some(AnchorScaleOffsetFit {
        offset_ms: best.offset.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64,
        scale_ppm,
        drift_ratio,
        aligned: best.aligned,
    })
}

fn anchor_fit_inliers(pairs: &[AnchorMatchPair], scale: f64, offset: f64) -> Vec<AnchorMatchPair> {
    pairs
        .iter()
        .copied()
        .filter(|pair| {
            let predicted = (scale * f64::from(pair.query_t_ms)) + offset;
            (f64::from(pair.candidate_t_ms) - predicted).abs()
                <= DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS as f64
        })
        .collect()
}

fn least_squares_anchor_fit(pairs: &[AnchorMatchPair]) -> Option<(f64, f64)> {
    if pairs.len() < 2 {
        return None;
    }
    let mut sum_weight = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    for pair in pairs {
        let weight = f64::from(pair.weight.max(1));
        let x = f64::from(pair.query_t_ms);
        let y = f64::from(pair.candidate_t_ms);
        sum_weight += weight;
        sum_x += weight * x;
        sum_y += weight * y;
        sum_xx += weight * x * x;
        sum_xy += weight * x * y;
    }
    let denominator = (sum_weight * sum_xx) - (sum_x * sum_x);
    if denominator.abs() < f64::EPSILON {
        return None;
    }
    let scale = ((sum_weight * sum_xy) - (sum_x * sum_y)) / denominator;
    if !(0.95..=1.05).contains(&scale) {
        return None;
    }
    let offset = (sum_y - (scale * sum_x)) / sum_weight;
    Some((scale, offset))
}

fn max_anchor_fit_residual_ms(pairs: &[AnchorMatchPair], scale: f64, offset: f64) -> f64 {
    pairs
        .iter()
        .map(|pair| {
            let predicted = (scale * f64::from(pair.query_t_ms)) + offset;
            (f64::from(pair.candidate_t_ms) - predicted).abs()
        })
        .fold(0.0, f64::max)
}

fn anchor_coverage(aligned: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        aligned as f64 / total as f64
    }
}

fn aligned_anchor_bounds(pairs: &[AnchorMatchPair]) -> (u32, u32, u32, u32) {
    let first_query = pairs.iter().map(|pair| pair.query_t_ms).min().unwrap_or(0);
    let last_query = pairs.iter().map(|pair| pair.query_t_ms).max().unwrap_or(0);
    let first_candidate = pairs
        .iter()
        .map(|pair| pair.candidate_t_ms)
        .min()
        .unwrap_or(0);
    let last_candidate = pairs
        .iter()
        .map(|pair| pair.candidate_t_ms)
        .max()
        .unwrap_or(0);
    (first_query, last_query, first_candidate, last_candidate)
}

fn aligned_anchor_span_ms(pairs: &[AnchorMatchPair]) -> u32 {
    let (first_query, last_query, _, _) = aligned_anchor_bounds(pairs);
    last_query.saturating_sub(first_query)
}

fn aligned_anchor_largest_gap_ratio(pairs: &[AnchorMatchPair]) -> f64 {
    if pairs.len() < 3 {
        return 1.0;
    }
    let mut times = pairs.iter().map(|pair| pair.query_t_ms).collect::<Vec<_>>();
    times.sort_unstable();
    times.dedup();
    if times.len() < 3 {
        return 1.0;
    }
    let span = times[times.len() - 1].saturating_sub(times[0]).max(1);
    let largest_gap = times
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .max()
        .unwrap_or(span);
    largest_gap as f64 / span as f64
}

fn bounded_time_distributed_audio_anchors(
    anchors: &mut [AudioAnchor],
    max_anchors: usize,
) -> Vec<AudioAnchor> {
    anchors.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket));
    if anchors.len() <= max_anchors {
        return anchors.to_vec();
    }
    let stride = anchors.len() as f64 / max_anchors as f64;
    (0..max_anchors)
        .map(|index| anchors[(index as f64 * stride).floor() as usize])
        .collect()
}

fn bounded_time_distributed_video_anchors(
    anchors: &mut [VideoAnchor],
    max_anchors: usize,
) -> Vec<VideoAnchor> {
    anchors.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.hash64));
    if anchors.len() <= max_anchors {
        return anchors.to_vec();
    }
    let stride = anchors.len() as f64 / max_anchors as f64;
    (0..max_anchors)
        .map(|index| anchors[(index as f64 * stride).floor() as usize])
        .collect()
}

fn stable_hash_u64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hasher = Sha256::new();
    for byte in bytes {
        hasher.update([byte]);
    }
    let digest = hasher.finalize();
    u64::from_le_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

fn anchor_bucket(hash: u64) -> u32 {
    (hash >> 32) as u32
}

fn video_lsh_buckets(hash: u64) -> [u32; VIDEO_LSH_BANDS as usize] {
    let mask = (1u64 << VIDEO_LSH_BITS_PER_BAND) - 1;
    let mut buckets = [0u32; VIDEO_LSH_BANDS as usize];
    for band in 0..VIDEO_LSH_BANDS {
        let shift = band * VIDEO_LSH_BITS_PER_BAND;
        let band_bits = ((hash >> shift) & mask) as u32;
        buckets[band as usize] = (band << VIDEO_LSH_BITS_PER_BAND) | band_bits;
    }
    buckets
}

pub fn video_anchor_hashes_match(left: u64, right: u64) -> bool {
    frame_hash_distance(left, right) <= DEFAULT_FRAME_HAMMING_THRESHOLD
}

pub fn media_extraction_settings_hash(settings: &MediaExtractionSettings) -> [u8; 32] {
    let bytes = serde_json::to_vec(settings).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn duration_seconds_to_millis(duration_seconds: f64) -> Option<u32> {
    duration_seconds
        .is_finite()
        .then_some(duration_seconds)
        .filter(|value| *value >= 0.0)
        .map(|value| (value * 1000.0).round().min(f64::from(u32::MAX)) as u32)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, MediaSummaryDecodeError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(MediaSummaryDecodeError::InvalidLength)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, MediaSummaryDecodeError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(MediaSummaryDecodeError::InvalidLength)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, MediaSummaryDecodeError> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(MediaSummaryDecodeError::InvalidLength)?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

pub fn pdq_style_luma_hash(width: usize, height: usize, luma: &[u8]) -> Option<u64> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }

    let mut cells = [0u32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let start_x = x * width / 8;
            let end_x = ((x + 1) * width / 8).max(start_x + 1).min(width);
            let start_y = y * height / 8;
            let end_y = ((y + 1) * height / 8).max(start_y + 1).min(height);
            let mut sum = 0u32;
            let mut count = 0u32;
            for source_y in start_y..end_y {
                let row = source_y * width;
                for source_x in start_x..end_x {
                    sum += u32::from(luma[row + source_x]);
                    count += 1;
                }
            }
            cells[y * 8 + x] = sum.checked_div(count).unwrap_or(0);
        }
    }

    let mean = cells.iter().sum::<u32>() / FRAME_HASH_BITS;
    let mut hash = 0u64;
    for (index, cell) in cells.iter().enumerate() {
        if *cell >= mean {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

pub fn frame_hash_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

pub fn normalize_media_path(path: impl AsRef<Path>) -> String {
    let mut parts = Vec::new();
    for component in path.as_ref().components() {
        match component {
            Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Component::RootDir => parts.push(String::new()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().to_ascii_lowercase()),
        }
    }
    let mut normalized = parts.join("/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

pub fn container_fingerprint_from_metadata(
    normalized_path: &str,
    modified_unix_millis: u64,
    size_bytes: u64,
    duration_seconds: Option<f64>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_path.as_bytes());
    hasher.update(modified_unix_millis.to_le_bytes());
    hasher.update(size_bytes.to_le_bytes());
    if let Some(duration_seconds) = duration_seconds {
        hasher.update(duration_seconds.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn metadata_evidence(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MetadataMatchEvidence {
    let duration_delta_seconds = query
        .duration_seconds
        .zip(candidate.duration_seconds)
        .map(|(left, right)| (left - right).abs());
    MetadataMatchEvidence {
        same_normalized_path: query.identity.normalized_path == candidate.identity.normalized_path,
        same_size: Some(query.identity.size_bytes == candidate.identity.size_bytes),
        duration_delta_seconds,
        duration_within_tolerance: duration_delta_seconds.map(|delta| {
            !settings.runtime_tolerance_enabled || delta <= settings.runtime_tolerance_seconds
        }),
        extension_match: extension(&query.identity.normalized_path)
            .zip(extension(&candidate.identity.normalized_path))
            .map(|(left, right)| left == right),
    }
}

fn extension(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(_, extension)| extension)
}

fn decision(
    tier: MediaMatchTier,
    evidence: MediaMatchEvidence,
    explanation: impl Into<String>,
) -> MediaMatchDecision {
    MediaMatchDecision {
        tier,
        evidence,
        explanation: explanation.into(),
    }
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

fn media_match_tier_rank(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Exact => 5,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Unknown => 1,
        MediaMatchTier::Reject => 0,
    }
}

fn longest_common_subsequence_ratio(left: &[u32], right: &[u32]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut previous = vec![0usize; right.len() + 1];
    let mut current = vec![0usize; right.len() + 1];
    for left_item in left {
        for (right_index, right_item) in right.iter().enumerate() {
            current[right_index + 1] = if left_item == right_item {
                previous[right_index] + 1
            } else {
                current[right_index].max(previous[right_index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous[right.len()] as f64 / left.len().min(right.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        path: &str,
        size: u64,
        duration: Option<f64>,
        audio: Option<AudioFingerprint>,
        video: Option<VideoFingerprint>,
    ) -> MediaFingerprintRecord {
        let normalized_path = normalize_media_path(path);
        MediaFingerprintRecord {
            identity: MediaFileIdentity {
                normalized_path: normalized_path.clone(),
                modified_unix_millis: 1000,
                size_bytes: size,
            },
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: MediaExtractionSettings::full_anchor_v2(),
            duration_seconds: duration,
            container_fingerprint: container_fingerprint_from_metadata(
                &normalized_path,
                1000,
                size,
                duration,
            ),
            audio,
            video,
            audio_anchors: Vec::new(),
            video_anchors: Vec::new(),
            audio_error: None,
            video_error: None,
        }
    }

    fn record_with_extraction_settings(
        path: &str,
        size: u64,
        duration: Option<f64>,
        audio: Option<AudioFingerprint>,
        video: Option<VideoFingerprint>,
        extraction_settings: MediaExtractionSettings,
    ) -> MediaFingerprintRecord {
        let mut record = record(path, size, duration, audio, video);
        record.extraction_settings = extraction_settings;
        record
    }

    fn record_from_anchor_profile(
        path: &str,
        size: u64,
        profile: MediaAnchorProfile,
    ) -> MediaFingerprintRecord {
        let mut record = record(
            path,
            size,
            profile.duration_ms.map(|duration| duration as f64 / 1000.0),
            None,
            None,
        );
        record.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        record.audio_anchors = profile.audio_anchors;
        record.video_anchors = profile.video_anchors;
        record
    }

    fn audio(tokens: &[u32]) -> AudioFingerprint {
        AudioFingerprint {
            duration_seconds: Some(tokens.len() as f64),
            fingerprint_tokens: tokens.to_vec(),
        }
    }

    fn video_from_hashes(start_second: u64, step_seconds: u64, hashes: &[u64]) -> VideoFingerprint {
        VideoFingerprint {
            duration_seconds: Some(start_second as u32 + step_seconds as u32 * hashes.len() as u32),
            frames: hashes
                .iter()
                .enumerate()
                .map(|(index, hash)| {
                    FrameFingerprint::new(
                        (start_second + step_seconds * index as u64) as f64,
                        *hash,
                    )
                })
                .collect(),
        }
    }

    fn shifted_video(offset_seconds: u64, hashes: &[u64]) -> VideoFingerprint {
        video_from_hashes(offset_seconds, 10, hashes)
    }

    fn synthetic_hash(value: u64) -> u64 {
        let mut x = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }

    fn synthetic_hashes(values: &[u64]) -> Vec<u64> {
        values.iter().copied().map(synthetic_hash).collect()
    }

    fn anchor_profile(
        duration_ms: u32,
        audio: &[(u32, u32)],
        video: &[(u32, u32, u64)],
    ) -> MediaAnchorProfile {
        MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "fast-anchor-v2".to_owned(),
            duration_ms: Some(duration_ms),
            audio_anchors: audio
                .iter()
                .map(|(bucket, t_ms)| AudioAnchor {
                    bucket: *bucket,
                    t_ms: *t_ms,
                    weight: 1,
                })
                .collect(),
            video_anchors: video
                .iter()
                .map(|(bucket, t_ms, hash64)| VideoAnchor {
                    bucket: *bucket,
                    t_ms: *t_ms,
                    hash64: *hash64,
                    weight: 1,
                })
                .collect(),
        }
    }

    fn regular_anchor_profile(
        duration_ms: u32,
        offset_ms: i32,
        drift_ppm: i32,
    ) -> MediaAnchorProfile {
        let query_times = (0..12).map(|index| 60_000 + index * 60_000);
        let audio = query_times
            .clone()
            .map(|t_ms| {
                let candidate_t = shifted_anchor_time(t_ms, offset_ms, drift_ppm);
                (t_ms / 60_000 + 1, candidate_t)
            })
            .collect::<Vec<_>>();
        let video = query_times
            .map(|t_ms| {
                let candidate_t = shifted_anchor_time(t_ms, offset_ms, drift_ppm);
                let hash = synthetic_hash(u64::from(t_ms));
                (t_ms / 60_000 + 100, candidate_t, hash)
            })
            .collect::<Vec<_>>();
        anchor_profile(duration_ms, &audio, &video)
    }

    fn shifted_anchor_time(t_ms: u32, offset_ms: i32, drift_ppm: i32) -> u32 {
        let scaled = i64::from(t_ms) + ((i64::from(t_ms) * i64::from(drift_ppm)) / 1_000_000);
        (scaled + i64::from(offset_ms))
            .max(0)
            .min(i64::from(u32::MAX)) as u32
    }

    fn enabled_settings() -> MediaMatchSettings {
        MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        }
    }

    #[test]
    fn compact_audio_summary_round_trips_with_delta_times() {
        let anchors = vec![
            AudioAnchor {
                bucket: 7,
                t_ms: 2_000,
                weight: 2,
            },
            AudioAnchor {
                bucket: 5,
                t_ms: 500,
                weight: 1,
            },
        ];

        let encoded = encode_audio_anchor_summary(&anchors);
        let decoded = decode_audio_anchor_summary(&encoded).expect("audio summary should decode");

        assert_eq!(
            decoded,
            vec![
                AudioAnchor {
                    bucket: 5,
                    t_ms: 500,
                    weight: 1,
                },
                AudioAnchor {
                    bucket: 7,
                    t_ms: 2_000,
                    weight: 2,
                },
            ]
        );
        assert!(encoded.len() < 64);
    }

    #[test]
    fn compact_video_summary_round_trips_hashes() {
        let anchors = vec![
            VideoAnchor {
                bucket: 9,
                t_ms: 1_000,
                hash64: 0x0123_4567_89ab_cdef,
                weight: 1,
            },
            VideoAnchor {
                bucket: 10,
                t_ms: 3_000,
                hash64: 0xfedc_ba98_7654_3210,
                weight: 3,
            },
        ];

        let encoded = encode_video_anchor_summary(&anchors);
        let decoded = decode_video_anchor_summary(&encoded).expect("video summary should decode");

        assert_eq!(decoded, anchors);
        assert!(encoded.len() < 80);
    }

    #[test]
    fn anchor_matching_estimates_simple_offset_within_one_second() {
        let query = regular_anchor_profile(900_000, 0, 0);
        let candidate = regular_anchor_profile(901_000, 1_000, 0);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Strong);
        let alignment = decision.evidence.alignment.expect("alignment evidence");
        assert!((alignment.offset_seconds - 1.0).abs() <= 1.0);
        assert!(alignment.aligned_audio_anchors >= 10);
        assert!(alignment.aligned_video_anchors >= 10);
    }

    #[test]
    fn anchor_matching_accepts_small_drift_but_reports_it() {
        let query = regular_anchor_profile(900_000, 0, 0);
        let candidate = regular_anchor_profile(901_000, 1_000, 2_000);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ));
        let alignment = decision.evidence.alignment.expect("alignment evidence");
        assert!(alignment.drift_ratio <= 0.015);
        assert!(alignment.scale_ppm > 1_000_000);
    }

    #[test]
    fn video_lsh_matches_hashes_with_high_bit_differences() {
        let query_hash = 0x0123_4567_89ab_cdef;
        let candidate_hash = query_hash ^ (1 << 60);
        assert!(video_anchor_hashes_match(query_hash, candidate_hash));

        let query_video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![FrameFingerprint {
                timestamp_millis: 30_000,
                hash: query_hash,
            }],
        };
        let candidate_video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![FrameFingerprint {
                timestamp_millis: 31_000,
                hash: candidate_hash,
            }],
        };
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "fast-anchor-v2".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&query_video, 4),
        };
        let candidate = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "fast-anchor-v2".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
        };

        let pairs = collect_anchor_match_pairs(&query, &candidate);

        assert!(
            !pairs.is_empty(),
            "multi-bucket LSH should find a Hamming-near video hash even when high bits differ"
        );
    }

    #[test]
    fn anchor_matching_handles_trimmed_start_body_overlap() {
        let query = regular_anchor_profile(1_200_000, 0, 0);
        let candidate_audio = query.audio_anchors[3..].to_vec();
        let candidate_video = query.video_anchors[3..].to_vec();
        let candidate = MediaAnchorProfile {
            audio_anchors: candidate_audio,
            video_anchors: candidate_video,
            duration_ms: Some(1_020_000),
            ..query.clone()
        };

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ));
        assert!(
            decision
                .evidence
                .alignment
                .as_ref()
                .is_some_and(|alignment| alignment.aligned_span_seconds >= 300.0)
        );
    }

    #[test]
    fn anchor_matching_rejects_wrong_episode_with_shared_edges() {
        let intro_times = [0, 30_000, 60_000];
        let outro_times = [1_100_000, 1_130_000, 1_160_000];
        let query_audio = intro_times
            .into_iter()
            .chain(outro_times)
            .enumerate()
            .map(|(index, t_ms)| (index as u32 + 1, t_ms))
            .collect::<Vec<_>>();
        let query_video = query_audio
            .iter()
            .map(|(bucket, t_ms)| (*bucket + 100, *t_ms, synthetic_hash(u64::from(*bucket))))
            .collect::<Vec<_>>();
        let query = anchor_profile(1_200_000, &query_audio, &query_video);
        let candidate = query.clone();

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(
            !matches!(decision.tier, MediaMatchTier::Strong),
            "shared intro/outro anchors must not be strong: {decision:?}"
        );
    }

    #[test]
    fn fast_anchor_profile_process_budget_is_three_external_tools() {
        let counts =
            expected_media_tool_invocation_counts(&MediaExtractionSettings::fast_anchor_v2());
        assert_eq!(counts.ffmpeg + counts.ffprobe + counts.fpcalc, 3);
        assert_eq!(counts.ffmpeg, 1);
    }

    #[test]
    fn wire_signature_round_trips_fast_profile() {
        let hashes = synthetic_hashes(&[1, 2, 3, 4]);
        let record = record_with_extraction_settings(
            "[Judas] Show - S01E07.mkv",
            100,
            Some(1412.37),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(30, &hashes)),
            MediaExtractionSettings::fast_anchor_v2(),
        );

        let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
            .expect("wire value should serialize");
        let signature =
            media_match_wire_signature_from_value(&value).expect("wire signature should parse");
        let profile = media_anchor_profile_from_wire_profile(&signature.profiles[0])
            .expect("v2 profile should decode");

        assert_eq!(signature.schema, MEDIA_MATCH_WIRE_SCHEMA_V2);
        assert_eq!(signature.profiles[0].profile, "fast-anchor-v2");
        assert!(!profile.audio_anchors.is_empty());
        assert!(!profile.video_anchors.is_empty());
    }

    #[test]
    fn wire_signature_compares_local_record_to_remote_profile() {
        let hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let audio_tokens = (0..360).collect::<Vec<u32>>();
        let query = record_with_extraction_settings(
            "[Judas] Show - S01E07.mkv",
            100,
            Some(1412.0),
            Some(audio(&audio_tokens)),
            Some(video_from_hashes(0, 60, &hashes)),
            MediaExtractionSettings::fast_anchor_v2(),
        );
        let remote = record_with_extraction_settings(
            "[Erai-raws] Show - 07.mkv",
            200,
            Some(1413.0),
            Some(audio(&audio_tokens)),
            Some(video_from_hashes(0, 60, &hashes)),
            MediaExtractionSettings::fast_anchor_v2(),
        );
        let value =
            media_match_wire_value_from_records(&[remote]).expect("wire value should serialize");
        let signature =
            media_match_wire_signature_from_value(&value).expect("wire signature should parse");

        let decision =
            decide_media_match_against_wire_signature(&query, &signature, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Strong);
    }

    #[test]
    fn malformed_wire_signatures_are_ignored_for_autoplay() {
        let unsupported = serde_json::json!({
            "schema": "sorotte.mediaMatch.v999",
            "profiles": []
        });
        assert!(media_match_wire_signature_from_value(&unsupported).is_err());

        let legacy_v1 = serde_json::json!({
            "schema": format!("sorotte.mediaMatch.v{}", 1),
            "profiles": [{"profile": format!("fast-v{}", 1)}]
        });
        assert!(media_match_wire_signature_from_value(&legacy_v1).is_err());

        let empty_v2 = serde_json::json!({
            "schema": MEDIA_MATCH_WIRE_SCHEMA_V2,
            "profiles": []
        });
        assert!(media_match_wire_signature_from_value(&empty_v2).is_err());
    }

    #[test]
    fn wire_signature_rejects_unsupported_v2_profile_fields() {
        let record = record_with_extraction_settings(
            "episode.mkv",
            100,
            Some(120.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(0, &synthetic_hashes(&[1, 2, 3, 4]))),
            MediaExtractionSettings::fast_anchor_v2(),
        );
        let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
            .expect("wire value should serialize");

        let mut unsupported_version = value.clone();
        unsupported_version["profiles"][0]["algorithmVersion"] =
            serde_json::json!(MEDIA_MATCH_ANCHOR_VERSION + 1);
        assert!(media_match_wire_signature_from_value(&unsupported_version).is_err());

        let mut unknown_profile = value.clone();
        unknown_profile["profiles"][0]["profile"] = serde_json::json!("fast-anchor-v999");
        assert!(media_match_wire_signature_from_value(&unknown_profile).is_err());

        let mut wrong_time_base = value.clone();
        wrong_time_base["profiles"][0]["audio"]["timeBaseMs"] = serde_json::json!(1000);
        assert!(media_match_wire_signature_from_value(&wrong_time_base).is_err());

        let mut wrong_algorithm = value;
        wrong_algorithm["profiles"][0]["video"]["algorithm"] =
            serde_json::json!("unsupported-video-anchor-algorithm");
        assert!(media_match_wire_signature_from_value(&wrong_algorithm).is_err());
    }

    #[test]
    fn ffmpeg_showinfo_parser_preserves_irregular_frame_pts() {
        let stderr = "\
[Parsed_showinfo_1 @ 000001] n:   0 pts: 48000 pts_time:2.000 pos:0
[Parsed_showinfo_1 @ 000001] n:   1 pts: 103200 pts_time:4.300 pos:0
[Parsed_showinfo_1 @ 000001] n:   2 pts: 247200 pts_time:10.300 pos:0
";

        assert_eq!(
            parse_ffmpeg_showinfo_pts_times(stderr),
            vec![2.0, 4.3, 10.3]
        );
    }

    #[test]
    fn exact_decision_uses_path_mtime_and_size() {
        let query = record("C:/Media/Movie.mkv", 100, Some(100.0), None, None);
        let candidate = query.clone();

        let decision = decide_media_match(&query, &candidate, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Exact);
    }

    #[test]
    fn strong_decision_requires_strong_fingerprint_evidence() {
        let query_profile = regular_anchor_profile(900_000, 0, 0);
        let candidate_profile = regular_anchor_profile(901_000, 20_000, 0);
        let query = record_from_anchor_profile("show.s01e01.web.mkv", 100, query_profile);
        let candidate = record_from_anchor_profile("Show - 01 BluRay.mkv", 120, candidate_profile);

        let decision = decide_media_match(&query, &candidate, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Strong);
        assert!(
            decision
                .evidence
                .alignment
                .as_ref()
                .is_some_and(|alignment| alignment.offset_seconds > 19.0)
        );
    }

    #[test]
    fn fast_strong_requires_audio_video_and_runtime_evidence() {
        let query_profile = regular_anchor_profile(900_000, 0, 0);
        let mut candidate_profile = regular_anchor_profile(901_000, 20_000, 0);
        candidate_profile.video_anchors.truncate(9);
        let query = record_from_anchor_profile("[Judas] Show - 07.mkv", 100, query_profile);
        let candidate =
            record_from_anchor_profile("[Erai-raws] Show - 07.mkv", 120, candidate_profile.clone());
        let mut no_audio_profile = candidate_profile.clone();
        no_audio_profile.audio_anchors.clear();
        let no_audio =
            record_from_anchor_profile("[Erai-raws] Show - 07 no-audio.mkv", 121, no_audio_profile);
        let mut no_video_profile = candidate_profile.clone();
        no_video_profile.video_anchors.clear();
        let no_video =
            record_from_anchor_profile("[Erai-raws] Show - 07 no-video.mkv", 122, no_video_profile);
        let mut wrong_runtime_profile = candidate_profile;
        wrong_runtime_profile.duration_ms = Some(910_000);
        let wrong_runtime = record_from_anchor_profile(
            "[Erai-raws] Show - 07 long.mkv",
            123,
            wrong_runtime_profile,
        );
        let settings = enabled_settings();

        assert_eq!(
            decide_media_match(&query, &candidate, &settings).tier,
            MediaMatchTier::Strong
        );
        assert_ne!(
            decide_media_match(&query, &no_audio, &settings).tier,
            MediaMatchTier::Strong
        );
        assert_ne!(
            decide_media_match(&query, &no_video, &settings).tier,
            MediaMatchTier::Strong
        );
        assert_ne!(
            decide_media_match(&query, &wrong_runtime, &settings).tier,
            MediaMatchTier::Strong
        );
    }

    #[test]
    fn probable_decision_is_not_autoplay_eligible() {
        let mut query_profile = regular_anchor_profile(900_000, 0, 0);
        query_profile.video_anchors.clear();
        query_profile.audio_anchors.truncate(8);
        let mut candidate_profile = regular_anchor_profile(900_000, 0, 0);
        candidate_profile.video_anchors.clear();
        candidate_profile.audio_anchors.truncate(8);
        let query = record_from_anchor_profile("episode-a.mkv", 100, query_profile);
        let candidate = record_from_anchor_profile("episode-b.mkv", 110, candidate_profile);
        let settings = MediaMatchSettings {
            autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ..enabled_settings()
        };

        let decision = decide_media_match(&query, &candidate, &settings);

        assert_eq!(decision.tier, MediaMatchTier::Probable);
        assert!(!decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn weak_or_reject_for_wrong_episode_with_shared_intro_and_outro() {
        let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let candidate_hashes = synthetic_hashes(&[1, 2, 3, 90, 91, 92, 93, 94, 95, 10, 11, 12]);
        let query = record(
            "show-e01.mkv",
            100,
            Some(1200.0),
            None,
            Some(shifted_video(0, &query_hashes)),
        );
        let candidate = record(
            "show-e02.mkv",
            100,
            Some(1200.0),
            None,
            Some(shifted_video(0, &candidate_hashes)),
        );

        let decision = decide_media_match(&query, &candidate, &enabled_settings());

        assert!(
            matches!(decision.tier, MediaMatchTier::Weak | MediaMatchTier::Reject),
            "shared intro/outro must not be strong/probable: {decision:?}"
        );
    }

    #[test]
    fn synthetic_alignment_handles_trimmed_intro() {
        let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let candidate_hashes = synthetic_hashes(&[3, 4, 5, 6, 7, 8]);
        let query = video_from_hashes(0, 10, &query_hashes);
        let candidate = video_from_hashes(0, 10, &candidate_hashes);

        let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

        assert_eq!(evidence.aligned_pairs, 6);
        assert!(evidence.query_coverage >= 0.75);
        assert_eq!(evidence.candidate_coverage, 1.0);
        assert!(evidence.best_offset_seconds < -19.0);
    }

    #[test]
    fn synthetic_alignment_handles_trimmed_credits() {
        let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let candidate_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6]);
        let query = video_from_hashes(0, 10, &query_hashes);
        let candidate = video_from_hashes(0, 10, &candidate_hashes);

        let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

        assert_eq!(evidence.aligned_pairs, 6);
        assert!(evidence.query_coverage >= 0.75);
        assert_eq!(evidence.candidate_coverage, 1.0);
    }

    #[test]
    fn synthetic_alignment_rejects_mild_drift_as_strong() {
        let hashes = synthetic_hashes(&[10, 20, 30, 40, 50, 60, 70, 80]);
        let query = VideoFingerprint {
            duration_seconds: Some(80),
            frames: hashes
                .iter()
                .enumerate()
                .map(|(index, hash)| FrameFingerprint::new(index as f64 * 10.0, *hash))
                .collect(),
        };
        let candidate = VideoFingerprint {
            duration_seconds: Some(86),
            frames: hashes
                .iter()
                .enumerate()
                .map(|(index, hash)| FrameFingerprint::new(index as f64 * 10.8, *hash))
                .collect(),
        };

        let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

        assert!(evidence.drift_ratio > 0.015);
    }

    #[test]
    fn candidate_ranking_prefers_stronger_media_match_tiers() {
        let query_profile = regular_anchor_profile(900_000, 0, 0);
        let strong_profile = regular_anchor_profile(901_000, 10_000, 0);
        let mut weak_profile = regular_anchor_profile(900_000, 0, 0);
        weak_profile.audio_anchors.truncate(2);
        weak_profile.video_anchors.clear();
        let query = record_from_anchor_profile("episode.web.mkv", 100, query_profile);
        let weak = record_from_anchor_profile("maybe-episode.mkv", 110, weak_profile);
        let strong = record_from_anchor_profile("episode.bluray.mkv", 120, strong_profile);

        let ranked = rank_media_match_candidates(&query, [&weak, &strong], &enabled_settings());

        assert_eq!(ranked[0].decision.tier, MediaMatchTier::Strong);
        assert_eq!(
            ranked[0].candidate_path,
            normalize_media_path("episode.bluray.mkv")
        );
    }

    #[test]
    fn cache_invalidates_on_identity_and_algorithm_inputs() {
        let settings = MediaExtractionSettings::full_anchor_v2();
        let fast_settings = MediaExtractionSettings::fast_anchor_v2();
        let mut cache = MediaMatchCache::default();
        let record = record("movie.mkv", 100, Some(10.0), None, None);
        cache.insert(record);

        assert!(
            cache
                .get_valid(
                    "movie.mkv",
                    1000,
                    100,
                    MEDIA_MATCH_ALGORITHM_VERSION,
                    &settings
                )
                .is_some()
        );
        assert!(
            cache
                .get_valid(
                    "movie.mkv",
                    1001,
                    100,
                    MEDIA_MATCH_ALGORITHM_VERSION,
                    &settings
                )
                .is_none()
        );
        assert!(
            cache
                .get_valid(
                    "movie.mkv",
                    1000,
                    101,
                    MEDIA_MATCH_ALGORITHM_VERSION,
                    &settings
                )
                .is_none()
        );
        assert!(
            cache
                .get_valid(
                    "movie.mkv",
                    1000,
                    100,
                    MEDIA_MATCH_ALGORITHM_VERSION + 1,
                    &settings
                )
                .is_none()
        );
        assert!(
            cache
                .get_valid(
                    "movie.mkv",
                    1000,
                    100,
                    MEDIA_MATCH_ALGORITHM_VERSION,
                    &fast_settings
                )
                .is_none()
        );
    }

    #[test]
    fn fpcalc_output_parser_accepts_duration_and_tokens() {
        let parsed = parse_fpcalc_output("DURATION=123.45\nFINGERPRINT=1,2,3,5,8\n")
            .expect("fpcalc output should parse");

        assert_eq!(parsed.duration_seconds, Some(123.45));
        assert_eq!(parsed.fingerprint_tokens, vec![1, 2, 3, 5, 8]);
    }

    #[test]
    fn media_tool_runner_times_out_long_running_processes() {
        #[cfg(windows)]
        let (executable, args) = (
            Path::new("powershell.exe"),
            vec![
                std::ffi::OsString::from("-NoProfile"),
                std::ffi::OsString::from("-Command"),
                std::ffi::OsString::from("Start-Sleep -Seconds 2"),
            ],
        );
        #[cfg(not(windows))]
        let (executable, args) = (
            Path::new("/bin/sh"),
            vec![
                std::ffi::OsString::from("-c"),
                std::ffi::OsString::from("sleep 2"),
            ],
        );

        let error = run_tool_output(
            "test-tool",
            executable,
            args,
            None,
            Duration::from_millis(75),
        )
        .expect_err("long-running media helper should time out");

        assert_eq!(
            error,
            MediaFingerprintError::TimedOut {
                tool: "test-tool",
                timeout_seconds: 1,
            }
        );
    }

    #[test]
    fn fast_video_sample_timestamps_are_sparse_and_skip_stable_edges() {
        let timestamps = fast_video_sample_timestamps(Some(1800.0), 12);

        assert_eq!(timestamps.len(), 12);
        assert!(timestamps[0] >= 120.0);
        assert!(timestamps[11] <= 1680.0);
        assert!(
            timestamps
                .windows(2)
                .all(|pair| pair[0] < pair[1] && pair[1] - pair[0] > 60.0)
        );
    }

    #[test]
    fn pdq_style_luma_hash_is_stable_for_same_pixels() {
        let luma = (0u8..64).collect::<Vec<_>>();

        let left = pdq_style_luma_hash(8, 8, &luma).expect("hash");
        let right = pdq_style_luma_hash(8, 8, &luma).expect("hash");

        assert_eq!(left, right);
    }
}
