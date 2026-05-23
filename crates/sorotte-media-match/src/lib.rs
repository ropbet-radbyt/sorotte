use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use std::time::UNIX_EPOCH;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MEDIA_MATCH_CACHE_VERSION: u32 = 1;
pub const MEDIA_MATCH_ALGORITHM_VERSION: u32 = 1;
pub const MEDIA_MATCH_FILE_PAYLOAD_KEY: &str = "mediaMatch";
pub const MEDIA_MATCH_WIRE_SCHEMA_V1: &str = "sorotte.mediaMatch.v1";
pub const MEDIA_MATCH_WIRE_MAX_BYTES: usize = 32 * 1024;

const FRAME_HASH_BITS: u32 = 64;
const DEFAULT_FRAME_HAMMING_THRESHOLD: u32 = 10;
const DEFAULT_ALIGNMENT_TOLERANCE_SECONDS: f64 = 1.25;
const FAST_VIDEO_SAMPLE_FRAMES: usize = 12;
const VIDEO_FRAME_WIDTH: usize = 32;
const VIDEO_FRAME_HEIGHT: usize = 32;
const VIDEO_FRAME_BYTES: usize = VIDEO_FRAME_WIDTH * VIDEO_FRAME_HEIGHT;
const FAST_AUDIO_SAMPLE_SECONDS: u32 = 120;
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
    pub drift_ratio: f64,
    pub aligned_pairs: usize,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaFingerprintProfile {
    FastV1,
    FullV1,
}

impl MediaFingerprintProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FastV1 => "fast-v1",
            Self::FullV1 => "full-v1",
        }
    }
}

fn default_media_fingerprint_profile() -> MediaFingerprintProfile {
    MediaFingerprintProfile::FullV1
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
        Self::full_v1()
    }
}

impl MediaExtractionSettings {
    pub fn fast_v1() -> Self {
        Self {
            profile: MediaFingerprintProfile::FastV1,
            frame_sample_interval_seconds: 0,
            max_frames: FAST_VIDEO_SAMPLE_FRAMES,
            audio_sample_seconds: FAST_AUDIO_SAMPLE_SECONDS,
            audio_algorithm: format!("chromaprint-fpcalc-{FAST_AUDIO_SAMPLE_SECONDS}s"),
            video_algorithm: "sorotte-pdq-style-fast-v1".to_owned(),
        }
    }

    pub fn full_v1() -> Self {
        Self {
            profile: MediaFingerprintProfile::FullV1,
            frame_sample_interval_seconds: 10,
            max_frames: 720,
            audio_sample_seconds: 0,
            audio_algorithm: "chromaprint-fpcalc".to_owned(),
            video_algorithm: "sorotte-pdq-style-full-v1".to_owned(),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatchToolPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub fpcalc: PathBuf,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireSignatureV1 {
    pub schema: String,
    pub profiles: Vec<MediaMatchWireProfile>,
}

impl Default for MediaMatchWireSignatureV1 {
    fn default() -> Self {
        Self {
            schema: MEDIA_MATCH_WIRE_SCHEMA_V1.to_owned(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireProfile {
    pub profile: String,
    pub algorithm_version: u32,
    pub duration_seconds: Option<f64>,
    pub audio: Option<MediaMatchWireAudioFingerprint>,
    pub video: Option<MediaMatchWireVideoFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchWireAudioFingerprint {
    pub algorithm: String,
    pub tokens: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchWireVideoFingerprint {
    pub algorithm: String,
    pub frames: Vec<MediaMatchWireVideoFrame>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchWireVideoFrame {
    pub second: f64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaMatchCacheV1 {
    pub version: u32,
    pub records: BTreeMap<String, MediaFingerprintRecord>,
}

impl Default for MediaMatchCacheV1 {
    fn default() -> Self {
        Self {
            version: MEDIA_MATCH_CACHE_VERSION,
            records: BTreeMap::new(),
        }
    }
}

impl MediaMatchCacheV1 {
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
) -> MediaMatchWireSignatureV1 {
    let mut signature = MediaMatchWireSignatureV1::default();
    for record in records {
        if let Some(profile) = media_match_wire_profile_from_record(record) {
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
) -> Result<MediaMatchWireSignatureV1, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("media match wire signature could not serialize: {error}"))?;
    if bytes.len() > MEDIA_MATCH_WIRE_MAX_BYTES {
        return Err("media match wire signature exceeds the payload limit".to_owned());
    }
    let signature: MediaMatchWireSignatureV1 = serde_json::from_value(value.clone())
        .map_err(|error| format!("media match wire signature is invalid: {error}"))?;
    if signature.schema != MEDIA_MATCH_WIRE_SCHEMA_V1 {
        return Err("media match wire signature schema is unsupported".to_owned());
    }
    if signature.profiles.is_empty() {
        return Err("media match wire signature has no profiles".to_owned());
    }
    Ok(signature)
}

pub fn media_match_wire_records_from_signature(
    signature: &MediaMatchWireSignatureV1,
) -> Vec<MediaFingerprintRecord> {
    if signature.schema != MEDIA_MATCH_WIRE_SCHEMA_V1 {
        return Vec::new();
    }
    signature
        .profiles
        .iter()
        .filter_map(media_match_wire_record_from_profile)
        .collect()
}

pub fn decide_media_match_against_wire_signature(
    query: &MediaFingerprintRecord,
    signature: &MediaMatchWireSignatureV1,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let records = media_match_wire_records_from_signature(signature);
    let Some(best) = rank_media_match_candidates(query, records.iter(), settings)
        .into_iter()
        .next()
    else {
        return MediaMatchDecision::unknown("no comparable media match wire profiles");
    };
    best.decision
}

fn media_match_wire_profile_from_record(
    record: &MediaFingerprintRecord,
) -> Option<MediaMatchWireProfile> {
    if record.audio.is_none() && record.video.is_none() {
        return None;
    }
    Some(MediaMatchWireProfile {
        profile: record.extraction_settings.profile.label().to_owned(),
        algorithm_version: record.algorithm_version,
        duration_seconds: record.duration_seconds,
        audio: record
            .audio
            .as_ref()
            .map(|audio| MediaMatchWireAudioFingerprint {
                algorithm: record.extraction_settings.audio_algorithm.clone(),
                tokens: audio.fingerprint_tokens.clone(),
            }),
        video: record
            .video
            .as_ref()
            .map(|video| MediaMatchWireVideoFingerprint {
                algorithm: record.extraction_settings.video_algorithm.clone(),
                frames: video
                    .frames
                    .iter()
                    .map(|frame| MediaMatchWireVideoFrame {
                        second: frame.timestamp_seconds(),
                        hash: format!("{:016x}", frame.hash),
                    })
                    .collect(),
            }),
    })
}

fn media_match_wire_record_from_profile(
    profile: &MediaMatchWireProfile,
) -> Option<MediaFingerprintRecord> {
    let fingerprint_profile = match profile.profile.as_str() {
        "fast-v1" => MediaFingerprintProfile::FastV1,
        "full-v1" => MediaFingerprintProfile::FullV1,
        _ => return None,
    };
    let extraction_settings = match fingerprint_profile {
        MediaFingerprintProfile::FastV1 => MediaExtractionSettings::fast_v1(),
        MediaFingerprintProfile::FullV1 => MediaExtractionSettings::full_v1(),
    };
    if profile.algorithm_version != MEDIA_MATCH_ALGORITHM_VERSION {
        return None;
    }
    let audio = profile.audio.as_ref().and_then(|audio| {
        (audio.algorithm == extraction_settings.audio_algorithm && !audio.tokens.is_empty()).then(
            || AudioFingerprint {
                duration_seconds: profile.duration_seconds,
                fingerprint_tokens: audio.tokens.clone(),
            },
        )
    });
    let video = profile.video.as_ref().and_then(|video| {
        if video.algorithm != extraction_settings.video_algorithm || video.frames.is_empty() {
            return None;
        }
        let mut frames = Vec::with_capacity(video.frames.len());
        for frame in &video.frames {
            let hash = u64::from_str_radix(frame.hash.trim(), 16).ok()?;
            frames.push(FrameFingerprint::new(frame.second, hash));
        }
        Some(VideoFingerprint {
            duration_seconds: profile.duration_seconds.and_then(|duration| {
                (duration.is_finite() && duration >= 0.0).then_some(duration.round() as u32)
            }),
            frames,
        })
    });
    if audio.is_none() && video.is_none() {
        return None;
    }
    let normalized_path = format!("wire://media-match/{}", profile.profile);
    Some(MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: normalized_path.clone(),
            modified_unix_millis: 0,
            size_bytes: 0,
        },
        algorithm_version: profile.algorithm_version,
        extraction_settings,
        duration_seconds: profile.duration_seconds,
        container_fingerprint: container_fingerprint_from_metadata(
            &normalized_path,
            0,
            0,
            profile.duration_seconds,
        ),
        audio,
        video,
    })
}

pub fn fingerprint_media_file(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_cancellation(path, tools, extraction_settings, None)
}

pub fn fingerprint_media_file_cancellable(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: &AtomicBool,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
    fingerprint_media_file_with_cancellation(path, tools, extraction_settings, Some(cancel_flag))
}

fn fingerprint_media_file_with_cancellation(
    path: impl AsRef<Path>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<MediaFingerprintRecord, MediaFingerprintError> {
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
    let duration_seconds = probe_media_duration_seconds(&tools.ffprobe, path)?;
    let container_fingerprint = container_fingerprint_from_metadata(
        &normalized_path,
        modified_unix_millis,
        size_bytes,
        duration_seconds,
    );
    let audio = extract_audio_fingerprint_with_length(
        &tools.fpcalc,
        path,
        extraction_settings,
        cancel_flag,
    )
    .ok();
    let video = extract_video_fingerprint_with_cancellation(
        &tools.ffmpeg,
        path,
        duration_seconds,
        extraction_settings,
        cancel_flag,
    )
    .ok();

    Ok(MediaFingerprintRecord {
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
    })
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
        &MediaExtractionSettings::full_v1(),
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
    if extraction_settings.audio_sample_seconds > 0 {
        args.push("-length".into());
        args.push(extraction_settings.audio_sample_seconds.to_string().into());
    }
    args.push(media_path.as_ref().as_os_str().to_os_string());
    let output = run_tool_output("fpcalc", fpcalc.as_ref(), args, cancel_flag)?;
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
        MediaFingerprintProfile::FastV1 => extract_fast_video_fingerprint(
            ffmpeg,
            media_path,
            duration_seconds,
            extraction_settings,
            cancel_flag,
        ),
        MediaFingerprintProfile::FullV1 => extract_full_video_fingerprint(
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
    let mut frames = Vec::new();
    for timestamp in timestamps {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(MediaFingerprintError::Cancelled { tool: "ffmpeg" });
        }
        let output = run_tool_output(
            "ffmpeg",
            ffmpeg.as_ref(),
            [
                "-v".into(),
                "error".into(),
                "-ss".into(),
                format!("{timestamp:.3}").into(),
                "-i".into(),
                media_path.as_ref().as_os_str().to_os_string(),
                "-frames:v".into(),
                "1".into(),
                "-vf".into(),
                format!("scale={VIDEO_FRAME_WIDTH}:{VIDEO_FRAME_HEIGHT}:flags=bicubic,format=gray")
                    .into(),
                "-f".into(),
                "rawvideo".into(),
                "-pix_fmt".into(),
                "gray".into(),
                "-".into(),
            ],
            cancel_flag,
        )?;
        ensure_tool_success("ffmpeg", &output)?;
        let Some(chunk) = output.stdout.chunks_exact(VIDEO_FRAME_BYTES).next() else {
            continue;
        };
        let hash =
            pdq_style_luma_hash(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, chunk).ok_or_else(|| {
                MediaFingerprintError::InvalidToolOutput {
                    tool: "ffmpeg",
                    reason:
                        "raw grayscale frame size did not match the requested extraction geometry"
                            .to_owned(),
                }
            })?;
        frames.push(FrameFingerprint::new(timestamp, hash));
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

    if let (Some(query_audio), Some(candidate_audio)) = (&query.audio, &candidate.audio) {
        evidence.audio = Some(compare_audio_fingerprints(query_audio, candidate_audio));
    }

    if let (Some(query_video), Some(candidate_video)) = (&query.video, &candidate.video)
        && let Some(video_evidence) = align_video_fingerprints(query_video, candidate_video)
    {
        evidence.alignment = Some(MediaTimelineAlignment {
            offset_seconds: video_evidence.best_offset_seconds,
            drift_ratio: video_evidence.drift_ratio,
            aligned_pairs: video_evidence.aligned_pairs,
            first_query_second: first_aligned_time(query_video, candidate_video, &video_evidence)
                .map(|times| times.0)
                .unwrap_or(0.0),
            last_query_second: last_aligned_time(query_video, candidate_video, &video_evidence)
                .map(|times| times.0)
                .unwrap_or(0.0),
            first_candidate_second: first_aligned_time(
                query_video,
                candidate_video,
                &video_evidence,
            )
            .map(|times| times.1)
            .unwrap_or(0.0),
            last_candidate_second: last_aligned_time(query_video, candidate_video, &video_evidence)
                .map(|times| times.1)
                .unwrap_or(0.0),
        });
        evidence.video = Some(video_evidence);
    }

    let audio_similarity = evidence.audio.as_ref().map(|audio| audio.similarity);
    let video_coverage = evidence
        .video
        .as_ref()
        .map(|video| video.query_coverage.min(video.candidate_coverage));
    let video_drift_ok = evidence
        .video
        .as_ref()
        .is_some_and(|video| video.drift_ratio <= settings.max_alignment_drift_ratio);
    let duration_ok = evidence.metadata.duration_within_tolerance.unwrap_or(true);

    let audio_strong =
        audio_similarity.is_some_and(|value| value >= settings.audio_strong_similarity);
    let audio_probable =
        audio_similarity.is_some_and(|value| value >= settings.audio_probable_similarity);
    let video_strong = video_coverage.is_some_and(|value| value >= settings.video_strong_coverage)
        && video_drift_ok;
    let video_probable =
        video_coverage.is_some_and(|value| value >= settings.video_probable_coverage);
    let video_weak = video_coverage.is_some_and(|value| value >= settings.video_weak_coverage);
    let fast_profile = matches!(
        query.extraction_settings.profile,
        MediaFingerprintProfile::FastV1
    ) || matches!(
        candidate.extraction_settings.profile,
        MediaFingerprintProfile::FastV1
    );
    let strict_runtime_ok = settings.runtime_tolerance_enabled
        && evidence.metadata.duration_within_tolerance == Some(true);

    if fast_profile {
        if audio_strong && video_strong && strict_runtime_ok {
            return decision(
                MediaMatchTier::Strong,
                evidence,
                "fast fingerprint strong match: runtime tolerance, audio, and sparse video agree",
            );
        }
        if audio_probable && (strict_runtime_ok || video_probable) || video_strong {
            return decision(
                MediaMatchTier::Probable,
                evidence,
                "fast fingerprint evidence is consistent but not strong enough for autoplay",
            );
        }
        if audio_probable || video_probable || video_weak {
            return decision(
                MediaMatchTier::Weak,
                evidence,
                "partial fast fingerprint evidence; keep diagnostic only",
            );
        }
        if evidence.audio.is_none() && evidence.video.is_none() {
            return decision(
                MediaMatchTier::Unknown,
                evidence,
                "no comparable fingerprints",
            );
        }
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "fast fingerprints do not support same-media match",
        );
    }

    if audio_strong && video_strong {
        return decision(
            MediaMatchTier::Strong,
            evidence,
            "strong audio fingerprint plus aligned video frame hashes",
        );
    }

    if video_strong && duration_ok {
        return decision(
            MediaMatchTier::Strong,
            evidence,
            "strong aligned video frame hashes within runtime tolerance",
        );
    }

    if audio_strong && duration_ok {
        return decision(
            MediaMatchTier::Strong,
            evidence,
            "strong audio fingerprint within runtime tolerance",
        );
    }

    if audio_probable && (duration_ok || video_probable) || video_strong {
        return decision(
            MediaMatchTier::Probable,
            evidence,
            "fingerprint evidence is consistent but not strong enough for autoplay",
        );
    }

    if audio_probable || video_probable || video_weak {
        return decision(
            MediaMatchTier::Weak,
            evidence,
            "partial fingerprint evidence; keep diagnostic only",
        );
    }

    if evidence.audio.is_none() && evidence.video.is_none() {
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "no comparable fingerprints",
        );
    }

    decision(
        MediaMatchTier::Reject,
        evidence,
        "fingerprints do not support same-media match",
    )
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

fn run_tool_output<I>(
    tool: &'static str,
    executable: &Path,
    args: I,
    cancel_flag: Option<&AtomicBool>,
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

    loop {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaFingerprintError::Cancelled { tool });
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
            Ok(None) => thread::sleep(Duration::from_millis(25)),
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

fn hidden_media_match_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
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

fn first_aligned_time(
    query: &VideoFingerprint,
    candidate: &VideoFingerprint,
    video: &VideoMatchEvidence,
) -> Option<(f64, f64)> {
    aligned_times(query, candidate, video).first().copied()
}

fn last_aligned_time(
    query: &VideoFingerprint,
    candidate: &VideoFingerprint,
    video: &VideoMatchEvidence,
) -> Option<(f64, f64)> {
    aligned_times(query, candidate, video).last().copied()
}

fn aligned_times(
    query: &VideoFingerprint,
    candidate: &VideoFingerprint,
    video: &VideoMatchEvidence,
) -> Vec<(f64, f64)> {
    let mut times = Vec::new();
    let mut used_candidate = HashSet::new();
    for query_frame in &query.frames {
        let query_time = query_frame.timestamp_seconds();
        let expected_candidate_time = query_time + video.best_offset_seconds;
        let best = candidate
            .frames
            .iter()
            .enumerate()
            .filter(|(candidate_index, _)| !used_candidate.contains(candidate_index))
            .filter_map(|(candidate_index, candidate_frame)| {
                let distance = frame_hash_distance(query_frame.hash, candidate_frame.hash);
                (distance <= DEFAULT_FRAME_HAMMING_THRESHOLD).then(|| {
                    let candidate_time = candidate_frame.timestamp_seconds();
                    let offset_error = (candidate_time - expected_candidate_time).abs();
                    (candidate_index, candidate_time, offset_error)
                })
            })
            .filter(|(_, _, offset_error)| *offset_error <= DEFAULT_ALIGNMENT_TOLERANCE_SECONDS)
            .min_by(|left, right| left.2.total_cmp(&right.2));
        if let Some((candidate_index, candidate_time, _)) = best {
            used_candidate.insert(candidate_index);
            times.push((query_time, candidate_time));
        }
    }
    times.sort_by(|left, right| left.0.total_cmp(&right.0));
    times
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
            extraction_settings: MediaExtractionSettings::default(),
            duration_seconds: duration,
            container_fingerprint: container_fingerprint_from_metadata(
                &normalized_path,
                1000,
                size,
                duration,
            ),
            audio,
            video,
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

    fn enabled_settings() -> MediaMatchSettings {
        MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        }
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
            MediaExtractionSettings::fast_v1(),
        );

        let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
            .expect("wire value should serialize");
        let signature =
            media_match_wire_signature_from_value(&value).expect("wire signature should parse");
        let records = media_match_wire_records_from_signature(&signature);

        assert_eq!(signature.schema, MEDIA_MATCH_WIRE_SCHEMA_V1);
        assert_eq!(signature.profiles[0].profile, "fast-v1");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].extraction_settings,
            MediaExtractionSettings::fast_v1()
        );
        assert_eq!(
            records[0]
                .audio
                .as_ref()
                .expect("audio should round-trip")
                .fingerprint_tokens,
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn wire_signature_compares_local_record_to_remote_profile() {
        let hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6]);
        let query = record_with_extraction_settings(
            "[Judas] Show - S01E07.mkv",
            100,
            Some(1412.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(0, &hashes)),
            MediaExtractionSettings::fast_v1(),
        );
        let remote = record_with_extraction_settings(
            "[Erai-raws] Show - 07.mkv",
            200,
            Some(1413.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(20, &hashes)),
            MediaExtractionSettings::fast_v1(),
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

        let no_evidence = MediaMatchWireSignatureV1 {
            profiles: vec![MediaMatchWireProfile {
                profile: "fast-v1".to_owned(),
                algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
                duration_seconds: Some(120.0),
                audio: None,
                video: None,
            }],
            ..MediaMatchWireSignatureV1::default()
        };
        assert!(media_match_wire_records_from_signature(&no_evidence).is_empty());
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
        let hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6]);
        let query = record(
            "show.s01e01.web.mkv",
            100,
            Some(3600.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7])),
            Some(shifted_video(0, &hashes)),
        );
        let candidate = record(
            "Show - 01 BluRay.mkv",
            120,
            Some(3601.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7])),
            Some(shifted_video(20, &hashes)),
        );

        let decision = decide_media_match(&query, &candidate, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Strong);
        assert!(decision.evidence.video.unwrap().best_offset_seconds > 19.0);
    }

    #[test]
    fn fast_strong_requires_audio_video_and_runtime_evidence() {
        let hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let fast_settings = MediaExtractionSettings::fast_v1();
        let query = record_with_extraction_settings(
            "[Judas] Show - 07.mkv",
            100,
            Some(1200.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(0, &hashes)),
            fast_settings.clone(),
        );
        let candidate = record_with_extraction_settings(
            "[Erai-raws] Show - 07.mkv",
            120,
            Some(1201.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(20, &hashes)),
            fast_settings.clone(),
        );
        let no_audio = record_with_extraction_settings(
            "[Erai-raws] Show - 07 no-audio.mkv",
            121,
            Some(1201.0),
            None,
            Some(shifted_video(20, &hashes)),
            fast_settings.clone(),
        );
        let no_video = record_with_extraction_settings(
            "[Erai-raws] Show - 07 no-video.mkv",
            122,
            Some(1201.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            None,
            fast_settings.clone(),
        );
        let wrong_runtime = record_with_extraction_settings(
            "[Erai-raws] Show - 07 long.mkv",
            123,
            Some(1210.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Some(shifted_video(20, &hashes)),
            fast_settings,
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
        let query = record(
            "episode-a.mkv",
            100,
            Some(1000.0),
            Some(audio(&[1, 2, 3, 4, 5, 6, 7, 8])),
            None,
        );
        let candidate = record(
            "episode-b.mkv",
            110,
            Some(1000.0),
            Some(audio(&[0, 2, 3, 4, 5, 6, 7, 9])),
            None,
        );
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
        let hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6]);
        let query = record(
            "episode.web.mkv",
            100,
            Some(600.0),
            Some(audio(&[1, 2, 3, 4, 5, 6])),
            Some(shifted_video(0, &hashes)),
        );
        let weak = record(
            "maybe-episode.mkv",
            110,
            Some(600.0),
            Some(audio(&[1, 2, 9, 10, 11, 12])),
            None,
        );
        let strong = record(
            "episode.bluray.mkv",
            120,
            Some(601.0),
            Some(audio(&[1, 2, 3, 4, 5, 6])),
            Some(shifted_video(10, &hashes)),
        );

        let ranked = rank_media_match_candidates(&query, [&weak, &strong], &enabled_settings());

        assert_eq!(ranked[0].decision.tier, MediaMatchTier::Strong);
        assert_eq!(
            ranked[0].candidate_path,
            normalize_media_path("episode.bluray.mkv")
        );
    }

    #[test]
    fn cache_invalidates_on_identity_and_algorithm_inputs() {
        let settings = MediaExtractionSettings::default();
        let fast_settings = MediaExtractionSettings::fast_v1();
        let mut cache = MediaMatchCacheV1::default();
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
