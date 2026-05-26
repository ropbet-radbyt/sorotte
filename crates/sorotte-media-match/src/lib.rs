use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
#[cfg(test)]
use std::path::PathBuf;
use std::path::{Component, Path};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::UNIX_EPOCH;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod audio_v3;
mod diagnostic_harness;
mod diagnostics;
mod extraction;
mod settings;
mod timeline_v3;
mod types;
mod v3_index;
mod video_v3;
mod wire;

pub use audio_v3::AudioLandmarkV3;
pub use diagnostic_harness::{
    MediaMatchV3DiagnosticCandidateReport, MediaMatchV3DiagnosticDecisionReport,
    MediaMatchV3DiagnosticExpectation, MediaMatchV3DiagnosticFingerprintReport,
    MediaMatchV3DiagnosticManifest, MediaMatchV3DiagnosticManifestCase,
    MediaMatchV3DiagnosticReport, MediaMatchV3DiagnosticRetrievalReport,
    MediaMatchV3DiagnosticRunOptions, MediaMatchV3DiagnosticSummaryReport,
    MediaMatchV3ResolvedManifest, MediaMatchV3ResolvedManifestCandidate,
    MediaMatchV3ResolvedManifestCase, media_match_v3_diagnostic_manifest_from_json,
    media_match_v3_diagnostic_manifest_report_json, resolve_media_match_v3_diagnostic_manifest,
    run_media_match_v3_diagnostic_manifest,
};
pub use diagnostics::{
    MediaMatchV3DiagnosticSummary, summarize_decision_v3_diagnostics,
    summarize_instrumented_record_v3_diagnostics, summarize_record_v3_diagnostics,
};
pub use extraction::{
    InstrumentedMediaFingerprint, MediaAudioStreamMetrics, MediaExtractionTimings,
    MediaFingerprintError, MediaFingerprintExtractionReport, MediaMatchToolPaths,
    MediaToolInvocationCounts, expected_media_tool_invocation_counts,
};
pub use settings::{MediaExtractionSettings, MediaFingerprintProfile};
pub use timeline_v3::{
    classify_timeline_at_query_ms, map_candidate_position_to_query_ms,
    map_query_position_to_candidate_ms, timeline_map_contains_query_position,
};
pub use types::{
    AlignedSegmentV3, AudioMatchEvidence, MatchClassV3, MediaFileIdentity,
    MediaMatchAutoplayPolicy, MediaMatchCache, MediaMatchDecision, MediaMatchEvidence,
    MediaMatchSettings, MediaMatchTier, MediaTimelineAlignment, MediaTimelineMapV3,
    MetadataMatchEvidence, TimelinePositionMapResult, VideoMatchEvidence,
};
pub use v3_index::{
    MediaMatchV3Index, MediaMatchV3IndexPaths, MediaMatchV3RetrievalStats, anchor_stats_v3_dirty,
    clear_all_anchor_stats_v3_dirty, clear_anchor_stats_v3_dirty,
    delete_media_match_v3_file_and_fingerprints, delete_media_match_v3_fingerprints_and_anchors,
    initialize_media_match_v3_index, load_media_match_v3_cache_for_settings,
    load_media_match_v3_record_for_path, mark_anchor_stats_v3_dirty,
    mark_anchor_stats_v3_dirty_for_file, media_match_v3_anchor_candidate_paths_with_stats,
    media_match_v3_index_path, open_media_match_v3_index, refresh_all_anchor_stats_v3,
    refresh_anchor_stats_v3, refresh_dirty_anchor_stats_v3_if_needed, save_media_match_v3_record,
};
pub use video_v3::VideoLandmarkV3;
pub use wire::{
    MediaMatchWireAnchorBlock, MediaMatchWireProfile, MediaMatchWireSignature,
    decide_media_match_against_wire_signature, media_anchor_profile_from_wire_profile,
    media_match_wire_anchor_profile_from_anchor_profile, media_match_wire_signature_from_records,
    media_match_wire_signature_from_value, media_match_wire_value_from_records,
};

// TODO(media-match): continue extracting the remaining large anchor, audio,
// video, and matching algorithm bodies into their existing focused modules.
pub const MEDIA_MATCH_ALGORITHM_VERSION: u32 = 3;
pub const MEDIA_MATCH_FILE_PAYLOAD_KEY: &str = "mediaMatch";
pub const MEDIA_MATCH_WIRE_SCHEMA_V3: &str = "sorotte.mediaMatch.v3";
pub const MEDIA_MATCH_WIRE_MAX_BYTES: usize = 32 * 1024;
pub const MEDIA_MATCH_ANCHOR_VERSION: u32 = 3;

const FRAME_HASH_BITS: u32 = 64;
pub const DEFAULT_FRAME_HAMMING_THRESHOLD: u32 = 10;
const DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS: i64 = 1_000;
const DEFAULT_ANCHOR_OFFSET_BIN_MS: i64 = 1_000;

// V3 piecewise timeline fitting thresholds. These are deliberately conservative:
// segments need enough local evidence to map time, while gaps remain explicit.
const V3_SEGMENT_MIN_PAIR_DELTA_MS: u32 = 30_000;
const V3_SEGMENT_SPLIT_GAP_MS: u32 = 75_000;
const V3_SEGMENT_AUDIO_MIN_PAIRS: usize = 6;
const V3_SEGMENT_AUDIO_MIN_SPAN_MS: u32 = 60_000;
const V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS: usize = 3;
const V3_SEGMENT_AUDIO_VIDEO_MIN_SPAN_MS: u32 = 45_000;
const V3_SEGMENT_VIDEO_MIN_PAIRS: usize = 5;
const V3_SEGMENT_VIDEO_MIN_SPAN_MS: u32 = 60_000;
const V3_SEGMENT_MERGE_GAP_MS: u32 = 45_000;
const V3_SEGMENT_MERGE_SCALE_PPM: i32 = 2_500;
const V3_EDGE_REGION_MIN_MS: u32 = 120_000;
const V3_EDGE_REGION_MAX_MS: u32 = 300_000;
const V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS: usize = 512;
const MAX_BROAD_SCALE_FIT_PAIRS: usize = 128;

// V3 video retrieval and descriptor thresholds.
const VIDEO_LSH_BANDS: u32 = 4;
const VIDEO_LSH_BITS_PER_BAND: u32 = 16;
const V3_VIDEO_BUCKET_KIND_SHIFT: u32 = 28;
const V3_VIDEO_BUCKET_VALUE_MASK: u32 = 0x0fff_ffff;
pub const V3_VIDEO_KIND_LEGACY_LUMA: u8 = 0;
pub const V3_VIDEO_KIND_GLOBAL_DCT: u8 = 1;
pub const V3_VIDEO_KIND_CENTER_DCT: u8 = 2;
pub const V3_VIDEO_KIND_EDGE: u8 = 3;
pub const V3_VIDEO_KIND_TEMPORAL_SHINGLE: u8 = 4;

// V3 native audio constellation extraction thresholds.
const V3_AUDIO_SAMPLE_RATE: u32 = 11_025;
const V3_AUDIO_WINDOW_SAMPLES: usize = 2048;
const V3_AUDIO_HOP_SAMPLES: usize = 512;
const V3_AUDIO_MIN_FREQ_HZ: f32 = 250.0;
const V3_AUDIO_MAX_FREQ_HZ: f32 = 5_000.0;
const V3_AUDIO_MAX_PEAKS_PER_FRAME: usize = 6;
const V3_AUDIO_PEAK_NEIGHBORHOOD: usize = 2;
const V3_AUDIO_PAIR_MIN_DELTA_FRAMES: usize = 8;
const V3_AUDIO_PAIR_MAX_DELTA_FRAMES: usize = 108;
const V3_AUDIO_PAIR_FANOUT: usize = 8;
const V3_AUDIO_VERIFY_LANDMARK_LIMIT: usize = 768;
const V3_AUDIO_INDEX_LANDMARK_LIMIT: usize = 192;
// Streaming audio keeps only a winnowed raw landmark buffer; this bounds noisy/long files
// while preserving enough oversampling for the final time-distributed selector.
pub const V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 8;
const V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 4;
const V3_VIDEO_VERIFY_LANDMARK_LIMIT: usize = 192;
const V3_VIDEO_INDEX_LANDMARK_LIMIT: usize = 64;
const V3_VIDEO_PHASH_SIZE: usize = 32;
const V3_VIDEO_PHASH_LOW_FREQ: usize = 8;
const V3_VIDEO_MIN_VARIANCE: f64 = 6.0;
const V3_VIDEO_TEMPORAL_MIN_DELTA_MS: u32 = 5_000;
const V3_VIDEO_TEMPORAL_MAX_DELTA_MS: u32 = 60_000;
const V3_VIDEO_TEMPORAL_DELTA_BUCKET_MS: u32 = 5_000;
const V3_VIDEO_TEMPORAL_FANOUT: usize = 2;
const MAX_SUMMARY_ANCHORS: usize = 1024;
const MAX_V3_LANDMARKS: usize = 4096;
const VIDEO_FRAME_WIDTH: usize = 32;
const VIDEO_FRAME_HEIGHT: usize = 32;
const VIDEO_FRAME_BYTES: usize = VIDEO_FRAME_WIDTH * VIDEO_FRAME_HEIGHT;
const MEDIA_TOOL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const FFPROBE_TIMEOUT: Duration = Duration::from_secs(15);
const FFMPEG_AUDIO_V3_TIMEOUT: Duration = Duration::from_secs(180);
const FFMPEG_FULL_VIDEO_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V3Tuning {
    pub segment_min_pair_delta_ms: u32,
    pub segment_split_gap_ms: u32,
    pub segment_audio_min_pairs: usize,
    pub segment_video_min_pairs: usize,
    pub piecewise_max_hypothesis_pairs: usize,
    pub audio_raw_landmark_buffer_limit: usize,
    pub audio_raw_landmark_retain_limit: usize,
    pub video_hamming_global: u32,
    pub video_hamming_center: u32,
    pub video_hamming_edge: u32,
    pub video_hamming_temporal: u32,
}

pub fn current_v3_tuning() -> V3Tuning {
    V3Tuning {
        segment_min_pair_delta_ms: V3_SEGMENT_MIN_PAIR_DELTA_MS,
        segment_split_gap_ms: V3_SEGMENT_SPLIT_GAP_MS,
        segment_audio_min_pairs: V3_SEGMENT_AUDIO_MIN_PAIRS,
        segment_video_min_pairs: V3_SEGMENT_VIDEO_MIN_PAIRS,
        piecewise_max_hypothesis_pairs: V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS,
        audio_raw_landmark_buffer_limit: V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        audio_raw_landmark_retain_limit: V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
        video_hamming_global: 10,
        video_hamming_center: 10,
        video_hamming_edge: 12,
        video_hamming_temporal: 0,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFingerprintRecord {
    pub identity: MediaFileIdentity,
    pub algorithm_version: u32,
    pub extraction_settings: MediaExtractionSettings,
    pub duration_seconds: Option<f64>,
    pub container_fingerprint: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFingerprint {
    pub duration_seconds: Option<u32>,
    pub frames: Vec<FrameFingerprint>,
    #[serde(default)]
    pub v3_landmarks: Vec<VideoLandmarkV3>,
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
    #[serde(default)]
    pub kind: u8,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LumaRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
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
    UnsupportedVideoKind(u8),
    MismatchedVideoBucketKind { kind: u8, bucket_kind: u8 },
    InvalidTemporalVideoBucket { expected: u32, actual: u32 },
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
            Self::UnsupportedVideoKind(kind) => {
                write!(formatter, "unsupported media v3 video landmark kind {kind}")
            }
            Self::MismatchedVideoBucketKind { kind, bucket_kind } => {
                write!(
                    formatter,
                    "media v3 video landmark kind {kind} does not match bucket kind {bucket_kind}"
                )
            }
            Self::InvalidTemporalVideoBucket { expected, actual } => {
                write!(
                    formatter,
                    "media v3 temporal video bucket {actual} does not match expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for MediaSummaryDecodeError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaFingerprintBlobV3 {
    pub duration_ms: Option<u64>,
    pub audio_landmarks: Vec<AudioLandmarkV3>,
    pub video_landmarks: Vec<VideoLandmarkV3>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaFingerprintBlobV3DecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyLandmarks(usize),
    InvalidSection(u8),
    NonMonotonicTime,
    UnsupportedVideoKind(u8),
    MismatchedVideoBucketKind { kind: u8, bucket_kind: u8 },
    InvalidTemporalVideoBucket { expected: u32, actual: u32 },
}

impl fmt::Display for MediaFingerprintBlobV3DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(formatter, "invalid media fingerprint v3 blob magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported media fingerprint v3 blob version {version}"
                )
            }
            Self::InvalidLength => write!(formatter, "invalid media fingerprint v3 blob length"),
            Self::TooManyLandmarks(count) => {
                write!(
                    formatter,
                    "media fingerprint v3 blob has too many landmarks ({count})"
                )
            }
            Self::InvalidSection(section) => {
                write!(
                    formatter,
                    "invalid media fingerprint v3 blob section {section}"
                )
            }
            Self::NonMonotonicTime => {
                write!(
                    formatter,
                    "media fingerprint v3 blob timestamps are not monotonic"
                )
            }
            Self::UnsupportedVideoKind(kind) => {
                write!(
                    formatter,
                    "unsupported media fingerprint v3 video landmark kind {kind}"
                )
            }
            Self::MismatchedVideoBucketKind { kind, bucket_kind } => {
                write!(
                    formatter,
                    "media fingerprint v3 video landmark kind {kind} does not match bucket kind {bucket_kind}"
                )
            }
            Self::InvalidTemporalVideoBucket { expected, actual } => {
                write!(
                    formatter,
                    "media fingerprint v3 temporal video bucket {actual} does not match expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for MediaFingerprintBlobV3DecodeError {}

const AUDIO_SUMMARY_MAGIC: &[u8; 4] = b"SAU2";
const VIDEO_SUMMARY_MAGIC: &[u8; 4] = b"SVI3";
const SUMMARY_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_MAGIC: &[u8; 4] = b"SMM3";
const FINGERPRINT_BLOB_V3_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_SECTION_AUDIO: u8 = 1;
const FINGERPRINT_BLOB_V3_SECTION_VIDEO: u8 = 2;

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

pub fn media_fingerprint_blob_v3_from_record(
    record: &MediaFingerprintRecord,
) -> MediaFingerprintBlobV3 {
    MediaFingerprintBlobV3 {
        duration_ms: record
            .duration_seconds
            .and_then(duration_seconds_to_millis)
            .map(u64::from),
        audio_landmarks: audio_landmarks_v3_from_record(record),
        video_landmarks: video_landmarks_v3_from_record(record),
    }
}

pub fn media_fingerprint_record_apply_blob_v3(
    record: &mut MediaFingerprintRecord,
    blob: MediaFingerprintBlobV3,
) {
    record.duration_seconds = blob
        .duration_ms
        .map(|duration_ms| duration_ms as f64 / 1000.0);
    record.audio_anchors = blob
        .audio_landmarks
        .into_iter()
        .map(|landmark| AudioAnchor {
            bucket: landmark.hash,
            t_ms: landmark.t_ms,
            weight: u16::from(landmark.weight.max(1)),
        })
        .collect();
    record.video_anchors = blob
        .video_landmarks
        .into_iter()
        .map(|landmark| VideoAnchor {
            bucket: landmark.bucket,
            t_ms: landmark.t_ms,
            hash64: landmark.hash64,
            kind: landmark.kind,
            weight: u16::from(landmark.weight.max(1)),
        })
        .collect();
}

pub fn encode_media_fingerprint_blob_v3(blob: &MediaFingerprintBlobV3) -> Vec<u8> {
    let mut audio = blob.audio_landmarks.clone();
    audio.sort_by_key(|landmark| (landmark.t_ms, landmark.hash, landmark.weight));
    audio.truncate(MAX_V3_LANDMARKS);
    let mut video = blob.video_landmarks.clone();
    video.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.bucket,
            landmark.hash64,
            landmark.kind,
            landmark.weight,
        )
    });
    video.truncate(MAX_V3_LANDMARKS);

    let section_count = u8::from(!audio.is_empty()) + u8::from(!video.is_empty());
    let mut bytes = Vec::with_capacity(16 + audio.len() * 7 + video.len() * 18);
    bytes.extend_from_slice(FINGERPRINT_BLOB_V3_MAGIC);
    bytes.extend_from_slice(&FINGERPRINT_BLOB_V3_FORMAT_VERSION.to_le_bytes());
    encode_varint(blob.duration_ms.unwrap_or(u64::MAX), &mut bytes);
    bytes.push(section_count);
    if !audio.is_empty() {
        bytes.push(FINGERPRINT_BLOB_V3_SECTION_AUDIO);
        encode_varint(audio.len() as u64, &mut bytes);
        let mut previous_t_ms = 0u32;
        for landmark in audio {
            encode_varint(
                u64::from(landmark.t_ms.saturating_sub(previous_t_ms)),
                &mut bytes,
            );
            previous_t_ms = landmark.t_ms;
            bytes.extend_from_slice(&landmark.hash.to_le_bytes());
            bytes.push(landmark.weight);
        }
    }
    if !video.is_empty() {
        bytes.push(FINGERPRINT_BLOB_V3_SECTION_VIDEO);
        encode_varint(video.len() as u64, &mut bytes);
        let mut previous_t_ms = 0u32;
        for landmark in video {
            encode_varint(
                u64::from(landmark.t_ms.saturating_sub(previous_t_ms)),
                &mut bytes,
            );
            previous_t_ms = landmark.t_ms;
            bytes.extend_from_slice(&landmark.bucket.to_le_bytes());
            bytes.extend_from_slice(&landmark.hash64.to_le_bytes());
            bytes.push(landmark.kind);
            bytes.push(landmark.weight);
        }
    }
    bytes
}

pub fn decode_media_fingerprint_blob_v3(
    bytes: &[u8],
) -> Result<MediaFingerprintBlobV3, MediaFingerprintBlobV3DecodeError> {
    if bytes.len() < 7 {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    }
    if &bytes[0..4] != FINGERPRINT_BLOB_V3_MAGIC {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != FINGERPRINT_BLOB_V3_FORMAT_VERSION {
        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVersion(
            version,
        ));
    }
    let mut cursor = 6;
    let encoded_duration = decode_varint(bytes, &mut cursor)?;
    let duration_ms = (encoded_duration != u64::MAX).then_some(encoded_duration);
    let Some(section_count) = bytes.get(cursor).copied() else {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    };
    cursor += 1;
    let mut blob = MediaFingerprintBlobV3 {
        duration_ms,
        audio_landmarks: Vec::new(),
        video_landmarks: Vec::new(),
    };
    for _ in 0..section_count {
        let Some(section) = bytes.get(cursor).copied() else {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        };
        cursor += 1;
        let count = decode_varint(bytes, &mut cursor)? as usize;
        if count > MAX_V3_LANDMARKS {
            return Err(MediaFingerprintBlobV3DecodeError::TooManyLandmarks(count));
        }
        match section {
            FINGERPRINT_BLOB_V3_SECTION_AUDIO => {
                let mut t_ms = 0u32;
                let mut landmarks = Vec::with_capacity(count);
                for _ in 0..count {
                    let delta = decode_varint(bytes, &mut cursor)?;
                    let delta = u32::try_from(delta)
                        .map_err(|_| MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    t_ms = t_ms
                        .checked_add(delta)
                        .ok_or(MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    if cursor + 5 > bytes.len() {
                        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
                    }
                    let hash = u32::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                    ]);
                    cursor += 4;
                    let weight = bytes[cursor];
                    cursor += 1;
                    landmarks.push(AudioLandmarkV3 { hash, t_ms, weight });
                }
                blob.audio_landmarks = landmarks;
            }
            FINGERPRINT_BLOB_V3_SECTION_VIDEO => {
                let mut t_ms = 0u32;
                let mut landmarks = Vec::with_capacity(count);
                for _ in 0..count {
                    let delta = decode_varint(bytes, &mut cursor)?;
                    let delta = u32::try_from(delta)
                        .map_err(|_| MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    t_ms = t_ms
                        .checked_add(delta)
                        .ok_or(MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    if cursor + 14 > bytes.len() {
                        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
                    }
                    let bucket = u32::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                    ]);
                    cursor += 4;
                    let hash64 = u64::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                        bytes[cursor + 4],
                        bytes[cursor + 5],
                        bytes[cursor + 6],
                        bytes[cursor + 7],
                    ]);
                    cursor += 8;
                    let kind = bytes[cursor];
                    cursor += 1;
                    let weight = bytes[cursor];
                    cursor += 1;
                    if !v3_video_kind_is_supported(kind) {
                        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(
                            kind,
                        ));
                    }
                    let Some(bucket_kind) = v3_video_kind_from_bucket(bucket) else {
                        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(
                            (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8,
                        ));
                    };
                    if bucket_kind != kind {
                        return Err(
                            MediaFingerprintBlobV3DecodeError::MismatchedVideoBucketKind {
                                kind,
                                bucket_kind,
                            },
                        );
                    }
                    if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
                        let expected = v3_video_bucket_for_kind(kind, anchor_bucket(hash64));
                        if bucket != expected {
                            return Err(
                                MediaFingerprintBlobV3DecodeError::InvalidTemporalVideoBucket {
                                    expected,
                                    actual: bucket,
                                },
                            );
                        }
                    }
                    landmarks.push(VideoLandmarkV3 {
                        bucket,
                        hash64,
                        t_ms,
                        kind,
                        weight,
                    });
                }
                blob.video_landmarks = landmarks;
            }
            section => return Err(MediaFingerprintBlobV3DecodeError::InvalidSection(section)),
        }
    }
    if cursor != bytes.len() {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    }
    Ok(blob)
}

fn encode_varint(mut value: u64, bytes: &mut Vec<u8>) {
    while value >= 0x80 {
        bytes.push((value as u8) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn decode_varint(
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<u64, MediaFingerprintBlobV3DecodeError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let Some(byte) = bytes.get(*cursor).copied() else {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        };
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        }
    }
}

pub fn audio_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<AudioAnchor> {
    if !record.audio_anchors.is_empty() {
        return record.audio_anchors.clone();
    }
    Vec::new()
}

pub fn video_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<VideoAnchor> {
    if !record.video_anchors.is_empty() {
        return record.video_anchors.clone();
    }
    if matches!(
        record.extraction_settings.profile,
        MediaFingerprintProfile::CombinedV3
    ) && let Some(video) = &record.video
        && !video.v3_landmarks.is_empty()
    {
        let mut anchors = video
            .v3_landmarks
            .iter()
            .map(|landmark| VideoAnchor {
                bucket: landmark.bucket,
                t_ms: landmark.t_ms,
                hash64: landmark.hash64,
                kind: landmark.kind,
                weight: u16::from(landmark.weight.max(1)),
            })
            .collect::<Vec<_>>();
        return bounded_time_distributed_video_anchors(
            &mut anchors,
            V3_VIDEO_VERIFY_LANDMARK_LIMIT,
        );
    }
    let limit = if matches!(
        record.extraction_settings.profile,
        MediaFingerprintProfile::CombinedV3
    ) {
        V3_VIDEO_VERIFY_LANDMARK_LIMIT
    } else {
        0
    };
    record
        .video
        .as_ref()
        .map(|video| video_anchors_from_fingerprint(video, limit))
        .unwrap_or_default()
}

pub fn audio_landmarks_v3_from_record(record: &MediaFingerprintRecord) -> Vec<AudioLandmarkV3> {
    let mut landmarks = audio_anchors_from_record(record)
        .into_iter()
        .map(|anchor| AudioLandmarkV3 {
            hash: anchor.bucket,
            t_ms: anchor.t_ms,
            weight: anchor.weight.min(u16::from(u8::MAX)).max(1) as u8,
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_audio_landmarks_v3(&mut landmarks, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

pub fn video_landmarks_v3_from_record(record: &MediaFingerprintRecord) -> Vec<VideoLandmarkV3> {
    if let Some(video) = &record.video
        && !video.v3_landmarks.is_empty()
    {
        let mut landmarks = video.v3_landmarks.clone();
        return bounded_time_distributed_video_landmarks_v3(
            &mut landmarks,
            V3_VIDEO_VERIFY_LANDMARK_LIMIT,
        );
    }
    let mut landmarks = video_anchors_from_record(record)
        .into_iter()
        .map(|anchor| VideoLandmarkV3 {
            bucket: anchor.bucket,
            hash64: anchor.hash64,
            t_ms: anchor.t_ms,
            kind: anchor.kind,
            weight: anchor.weight.min(u16::from(u8::MAX)).max(1) as u8,
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_VERIFY_LANDMARK_LIMIT)
}

pub fn audio_index_landmarks_v3_from_record(
    record: &MediaFingerprintRecord,
) -> Vec<AudioLandmarkV3> {
    let mut landmarks = audio_landmarks_v3_from_record(record);
    bounded_time_distributed_audio_landmarks_v3(&mut landmarks, V3_AUDIO_INDEX_LANDMARK_LIMIT)
}

pub fn video_index_landmarks_v3_from_record(
    record: &MediaFingerprintRecord,
) -> Vec<VideoLandmarkV3> {
    let mut landmarks = video_landmarks_v3_from_record(record);
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_INDEX_LANDMARK_LIMIT)
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
                    kind: V3_VIDEO_KIND_LEGACY_LUMA,
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
    sorted.sort_by_key(|anchor| {
        (
            anchor.t_ms,
            anchor.bucket,
            anchor.hash64,
            anchor.kind,
            anchor.weight,
        )
    });
    let count = sorted.len().min(MAX_SUMMARY_ANCHORS);
    let mut bytes = Vec::with_capacity(8 + count * 19);
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
        bytes.push(anchor.kind);
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
    let expected = 8 + count * 19;
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
        let kind = bytes[cursor];
        cursor += 1;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        if !v3_video_kind_is_supported(kind) {
            return Err(MediaSummaryDecodeError::UnsupportedVideoKind(kind));
        }
        let Some(bucket_kind) = v3_video_kind_from_bucket(bucket) else {
            return Err(MediaSummaryDecodeError::UnsupportedVideoKind(
                (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8,
            ));
        };
        if bucket_kind != kind {
            return Err(MediaSummaryDecodeError::MismatchedVideoBucketKind { kind, bucket_kind });
        }
        if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
            let expected = v3_video_bucket_for_kind(kind, anchor_bucket(hash64));
            if bucket != expected {
                return Err(MediaSummaryDecodeError::InvalidTemporalVideoBucket {
                    expected,
                    actual: bucket,
                });
            }
        }
        t_ms = t_ms.saturating_add(delta_t_ms);
        anchors.push(VideoAnchor {
            bucket,
            t_ms,
            hash64,
            kind,
            weight,
        });
    }
    Ok(anchors)
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
    let mut audio_anchors = Vec::new();
    let started_at = Instant::now();
    let audio_result = extract_audio_constellation_v3_with_metrics(
        &tools.ffmpeg,
        path,
        duration_seconds,
        cancel_flag,
    );
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

pub fn extract_audio_constellation_v3(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<Vec<AudioLandmarkV3>, MediaFingerprintError> {
    extract_audio_constellation_v3_with_metrics(ffmpeg, media_path, duration_seconds, cancel_flag)
        .map(|(landmarks, _)| landmarks)
}

fn extract_audio_constellation_v3_with_metrics(
    ffmpeg: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    duration_seconds: Option<f64>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let stream = Arc::new(Mutex::new(AudioConstellationV3PcmStream::new(
        V3_AUDIO_SAMPLE_RATE,
    )));
    let stream_reader = Arc::clone(&stream);
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
            V3_AUDIO_SAMPLE_RATE.to_string().into(),
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
    let (landmarks, metrics) = stream.finish(duration_seconds)?;
    if landmarks.is_empty() {
        return Err(MediaFingerprintError::InvalidToolOutput {
            tool: "ffmpeg",
            reason: "decoded audio did not produce constellation landmarks".to_owned(),
        });
    }
    Ok((landmarks, metrics))
}

struct AudioConstellationV3PcmStream {
    pending_byte: Option<u8>,
    builder: AudioConstellationV3Builder,
    streamed_bytes: usize,
    streamed_samples: usize,
}

impl AudioConstellationV3PcmStream {
    fn new(sample_rate: u32) -> Self {
        Self {
            pending_byte: None,
            builder: AudioConstellationV3Builder::new(sample_rate),
            streamed_bytes: 0,
            streamed_samples: 0,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), MediaFingerprintError> {
        self.streamed_bytes += bytes.len();
        let mut samples =
            Vec::with_capacity((bytes.len() + usize::from(self.pending_byte.is_some())) / 2);
        let mut cursor = 0usize;
        if let Some(left) = self.pending_byte.take() {
            if let Some(right) = bytes.first().copied() {
                samples.push(i16::from_le_bytes([left, right]));
                cursor = 1;
            } else {
                self.pending_byte = Some(left);
                return Ok(());
            }
        }
        let chunks = bytes[cursor..].chunks_exact(2);
        let remainder = chunks.remainder();
        for chunk in chunks {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        if let Some(byte) = remainder.first().copied() {
            self.pending_byte = Some(byte);
        }
        self.streamed_samples += samples.len();
        self.builder.push_pcm_i16(&samples);
        Ok(())
    }

    fn finish(
        self,
        duration_seconds: Option<f64>,
    ) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
        if self.pending_byte.is_some() {
            return Err(MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "decoded PCM had a partial trailing sample".to_owned(),
            });
        }
        let streamed_bytes = self.streamed_bytes;
        let streamed_samples = self.streamed_samples;
        let (landmarks, mut metrics) = self.builder.finish_with_metrics(duration_seconds);
        metrics.streamed_bytes = streamed_bytes;
        metrics.streamed_samples = streamed_samples;
        Ok((landmarks, metrics))
    }
}

struct AudioConstellationV3Builder {
    sample_rate: u32,
    analyzer: Option<AudioSpectralAnalyzerV3>,
    rolling_samples: Vec<i16>,
    recent_frames: VecDeque<AudioPeakFrameV3>,
    raw_landmarks: Vec<AudioLandmarkV3>,
    next_frame_index: usize,
    peak_frames: usize,
    max_buffer_samples: usize,
    max_raw_landmarks_buffered: usize,
    max_raw_landmarks_seen: usize,
    max_raw_landmarks_after_compaction: usize,
    raw_landmark_compactions: usize,
}

impl AudioConstellationV3Builder {
    fn new(sample_rate: u32) -> Self {
        let analyzer = (sample_rate != 0).then(|| AudioSpectralAnalyzerV3::new(sample_rate));
        Self {
            sample_rate,
            analyzer,
            rolling_samples: Vec::with_capacity(V3_AUDIO_WINDOW_SAMPLES),
            recent_frames: VecDeque::new(),
            raw_landmarks: Vec::new(),
            next_frame_index: 0,
            peak_frames: 0,
            max_buffer_samples: 0,
            max_raw_landmarks_buffered: 0,
            max_raw_landmarks_seen: 0,
            max_raw_landmarks_after_compaction: 0,
            raw_landmark_compactions: 0,
        }
    }

    fn push_pcm_i16(&mut self, samples: &[i16]) {
        if self.analyzer.is_none() || samples.is_empty() {
            return;
        }
        let mut cursor = 0usize;
        while cursor < samples.len() {
            let needed = V3_AUDIO_WINDOW_SAMPLES.saturating_sub(self.rolling_samples.len());
            let take = needed.min(samples.len() - cursor).max(1);
            self.rolling_samples
                .extend_from_slice(&samples[cursor..cursor + take]);
            cursor += take;
            self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            while self.rolling_samples.len() >= V3_AUDIO_WINDOW_SAMPLES {
                let peaks = self
                    .analyzer
                    .as_mut()
                    .expect("analyzer exists")
                    .peaks_for_frame(&self.rolling_samples[..V3_AUDIO_WINDOW_SAMPLES]);
                self.process_peak_frame(self.next_frame_index, peaks);
                self.next_frame_index += 1;
                self.rolling_samples.drain(..V3_AUDIO_HOP_SAMPLES);
                self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            }
        }
    }

    fn process_peak_frame(&mut self, frame_index: usize, peaks: Vec<AudioSpectralPeakV3>) {
        let mut needs_compaction = false;
        for anchor_frame in &mut self.recent_frames {
            let delta_frames = frame_index.saturating_sub(anchor_frame.frame_index);
            if !(V3_AUDIO_PAIR_MIN_DELTA_FRAMES..=V3_AUDIO_PAIR_MAX_DELTA_FRAMES)
                .contains(&delta_frames)
            {
                continue;
            }
            for (peak_index, anchor_peak) in anchor_frame.peaks.iter().enumerate() {
                let mut emitted = anchor_frame.emitted_per_peak[peak_index];
                if emitted >= V3_AUDIO_PAIR_FANOUT {
                    continue;
                }
                for target_peak in &peaks {
                    let t_ms = audio_frame_timestamp_ms(anchor_frame.frame_index, self.sample_rate);
                    let hash =
                        audio_landmark_hash_v3(anchor_peak.bin, target_peak.bin, delta_frames);
                    let strength = ((anchor_peak.magnitude + target_peak.magnitude) * 4.0)
                        .round()
                        .clamp(1.0, f32::from(u8::MAX)) as u8;
                    self.raw_landmarks.push(AudioLandmarkV3 {
                        hash,
                        t_ms,
                        weight: strength,
                    });
                    self.max_raw_landmarks_seen =
                        self.max_raw_landmarks_seen.max(self.raw_landmarks.len());
                    needs_compaction |=
                        self.raw_landmarks.len() > V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT;
                    emitted += 1;
                    if emitted >= V3_AUDIO_PAIR_FANOUT {
                        break;
                    }
                }
                anchor_frame.emitted_per_peak[peak_index] = emitted;
            }
        }
        if needs_compaction {
            self.compact_raw_landmarks_if_needed();
        } else {
            self.max_raw_landmarks_buffered = self
                .max_raw_landmarks_buffered
                .max(self.raw_landmarks.len());
        }
        while self.recent_frames.front().is_some_and(|frame| {
            frame_index.saturating_sub(frame.frame_index) >= V3_AUDIO_PAIR_MAX_DELTA_FRAMES
        }) {
            self.recent_frames.pop_front();
        }
        self.peak_frames += 1;
        let emitted_per_peak = vec![0; peaks.len()];
        self.recent_frames.push_back(AudioPeakFrameV3 {
            frame_index,
            peaks,
            emitted_per_peak,
        });
    }

    fn compact_raw_landmarks_if_needed(&mut self) {
        if self.raw_landmarks.len() > V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT {
            compact_audio_landmark_buffer_v3(
                &mut self.raw_landmarks,
                V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
            );
            self.raw_landmark_compactions += 1;
            self.max_raw_landmarks_after_compaction = self
                .max_raw_landmarks_after_compaction
                .max(self.raw_landmarks.len());
        }
        self.max_raw_landmarks_buffered = self
            .max_raw_landmarks_buffered
            .max(self.raw_landmarks.len());
    }

    fn finish_with_metrics(
        self,
        duration_seconds: Option<f64>,
    ) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
        let raw_count = self.raw_landmarks.len();
        let landmarks = finish_bounded_audio_landmarks_v3(self.raw_landmarks, duration_seconds);
        let metrics = MediaAudioStreamMetrics {
            peak_frames: self.peak_frames,
            raw_landmarks_before_bounding: raw_count,
            final_landmarks: landmarks.len(),
            max_buffer_samples: self.max_buffer_samples,
            max_raw_landmarks_buffered: self.max_raw_landmarks_buffered.max(raw_count),
            max_raw_landmarks_seen: self.max_raw_landmarks_seen.max(raw_count),
            max_raw_landmarks_after_compaction: self.max_raw_landmarks_after_compaction,
            raw_landmark_compactions: self.raw_landmark_compactions,
            ..MediaAudioStreamMetrics::default()
        };
        (landmarks, metrics)
    }
}

#[derive(Debug)]
struct AudioPeakFrameV3 {
    frame_index: usize,
    peaks: Vec<AudioSpectralPeakV3>,
    emitted_per_peak: Vec<usize>,
}

struct AudioSpectralAnalyzerV3 {
    min_bin: usize,
    max_bin: usize,
    hann: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex<f32>>,
}

impl AudioSpectralAnalyzerV3 {
    fn new(sample_rate: u32) -> Self {
        let (min_bin, max_bin) = v3_audio_bin_range(sample_rate);
        let hann = v3_audio_hann_window();
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(V3_AUDIO_WINDOW_SAMPLES);
        Self {
            min_bin,
            max_bin,
            hann,
            fft,
            buffer: vec![Complex::new(0.0f32, 0.0f32); V3_AUDIO_WINDOW_SAMPLES],
        }
    }

    fn peaks_for_frame(&mut self, samples: &[i16]) -> Vec<AudioSpectralPeakV3> {
        for (index, slot) in self.buffer.iter_mut().enumerate() {
            let sample = samples[index] as f32 / f32::from(i16::MAX);
            *slot = Complex::new(sample * self.hann[index], 0.0);
        }
        self.fft.process(&mut self.buffer);
        audio_spectral_peaks_from_fft_bins(&self.buffer, self.min_bin, self.max_bin)
    }
}

#[cfg(test)]
fn audio_constellation_landmarks_v3_from_pcm_streaming(
    samples: &[i16],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
    let mut builder = AudioConstellationV3Builder::new(sample_rate);
    builder.push_pcm_i16(samples);
    let (landmarks, mut metrics) = builder.finish_with_metrics(duration_seconds);
    metrics.streamed_samples = samples.len();
    metrics.streamed_bytes = samples.len().saturating_mul(2);
    (landmarks, metrics)
}

#[cfg(test)]
fn audio_constellation_landmarks_v3_from_pcm_chunks(
    chunks: &[&[i16]],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
    let mut builder = AudioConstellationV3Builder::new(sample_rate);
    let mut samples = 0usize;
    for chunk in chunks {
        samples += chunk.len();
        builder.push_pcm_i16(chunk);
    }
    let (landmarks, mut metrics) = builder.finish_with_metrics(duration_seconds);
    metrics.streamed_samples = samples;
    metrics.streamed_bytes = samples.saturating_mul(2);
    (landmarks, metrics)
}

fn finish_bounded_audio_landmarks_v3(
    mut raw: Vec<AudioLandmarkV3>,
    duration_seconds: Option<f64>,
) -> Vec<AudioLandmarkV3> {
    dedupe_audio_landmarks_v3(&mut raw);
    if let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 120.0) {
        downweight_edge_audio_landmarks_v3(&mut raw, duration);
    }
    bounded_time_distributed_audio_landmarks_v3(&mut raw, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

fn compact_audio_landmark_buffer_v3(landmarks: &mut Vec<AudioLandmarkV3>, retain_limit: usize) {
    dedupe_audio_landmarks_v3(landmarks);
    if landmarks.len() <= retain_limit {
        return;
    }
    let mut by_weight = landmarks.clone();
    by_weight.sort_by_key(|landmark| {
        (
            std::cmp::Reverse(landmark.weight),
            landmark.t_ms,
            landmark.hash,
        )
    });
    let high_weight_limit = retain_limit / 2;
    let mut selected = by_weight
        .into_iter()
        .take(high_weight_limit)
        .collect::<Vec<_>>();
    let mut selected_keys = selected
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    let distributed_limit = retain_limit.saturating_sub(selected.len());
    let mut distributed = bounded_time_distributed_audio_landmarks_v3(landmarks, distributed_limit);
    for landmark in distributed.drain(..) {
        if selected_keys.insert((landmark.hash, landmark.t_ms)) {
            selected.push(landmark);
        }
    }
    if selected.len() < retain_limit {
        let mut remaining = landmarks.clone();
        remaining.sort_by_key(|landmark| {
            (
                landmark.t_ms,
                std::cmp::Reverse(landmark.weight),
                landmark.hash,
            )
        });
        for landmark in remaining {
            if selected.len() >= retain_limit {
                break;
            }
            if selected_keys.insert((landmark.hash, landmark.t_ms)) {
                selected.push(landmark);
            }
        }
    }
    selected.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    *landmarks = selected;
}

#[cfg(test)]
fn finish_pcm_stream_for_test(
    chunks: &[&[u8]],
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut stream = AudioConstellationV3PcmStream::new(V3_AUDIO_SAMPLE_RATE);
    for chunk in chunks {
        stream.push_bytes(chunk)?;
    }
    stream.finish(None)
}

#[cfg(test)]
fn audio_streaming_reference_overlap(left: &[AudioLandmarkV3], right: &[AudioLandmarkV3]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left = left
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    let right = right
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    left.intersection(&right).count() as f64 / left.len().max(right.len()) as f64
}

fn v3_audio_bin_range(sample_rate: u32) -> (usize, usize) {
    if sample_rate == 0 {
        return (1, V3_AUDIO_WINDOW_SAMPLES / 2);
    }
    let min_bin =
        ((V3_AUDIO_MIN_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).ceil();
    let max_bin =
        ((V3_AUDIO_MAX_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).floor();
    let min_bin = (min_bin as usize).clamp(1, (V3_AUDIO_WINDOW_SAMPLES / 2).saturating_sub(1));
    let max_bin = (max_bin as usize).clamp(min_bin + 1, V3_AUDIO_WINDOW_SAMPLES / 2);
    (min_bin, max_bin)
}

fn v3_audio_hann_window() -> Vec<f32> {
    (0..V3_AUDIO_WINDOW_SAMPLES)
        .map(|index| {
            let phase =
                (std::f32::consts::TAU * index as f32) / (V3_AUDIO_WINDOW_SAMPLES - 1) as f32;
            0.5 - (0.5 * phase.cos())
        })
        .collect()
}

fn audio_spectral_peaks_from_fft_bins(
    buffer: &[Complex<f32>],
    min_bin: usize,
    max_bin: usize,
) -> Vec<AudioSpectralPeakV3> {
    let magnitudes = (min_bin..max_bin)
        .map(|bin| {
            let value = buffer[bin].norm_sqr().max(f32::MIN_POSITIVE).log10();
            (bin, value)
        })
        .collect::<Vec<_>>();
    let mean = if magnitudes.is_empty() {
        0.0
    } else {
        magnitudes
            .iter()
            .map(|(_, magnitude)| *magnitude)
            .sum::<f32>()
            / magnitudes.len() as f32
    };
    let mut peaks = Vec::new();
    for (local_index, (bin, magnitude)) in magnitudes.iter().enumerate() {
        if *magnitude < mean + 0.35 {
            continue;
        }
        let left = local_index.saturating_sub(V3_AUDIO_PEAK_NEIGHBORHOOD);
        let right = (local_index + V3_AUDIO_PEAK_NEIGHBORHOOD + 1).min(magnitudes.len());
        if magnitudes[left..right]
            .iter()
            .all(|(_, neighbor)| *magnitude >= *neighbor)
        {
            peaks.push(AudioSpectralPeakV3 {
                bin: *bin,
                magnitude: *magnitude - mean,
            });
        }
    }
    peaks.sort_by(|left, right| {
        right
            .magnitude
            .total_cmp(&left.magnitude)
            .then_with(|| left.bin.cmp(&right.bin))
    });
    peaks.truncate(V3_AUDIO_MAX_PEAKS_PER_FRAME);
    peaks.sort_by_key(|peak| peak.bin);
    peaks
}

#[cfg(test)]
pub fn audio_constellation_stream_rejects_odd_trailing_byte_for_test(
    bytes: &[u8],
) -> Result<(), MediaFingerprintError> {
    finish_pcm_stream_for_test(&[bytes]).map(|_| ())
}

#[cfg(test)]
pub fn audio_constellation_streaming_cancel_flag_for_test(
    executable: &Path,
    args: Vec<OsString>,
    cancel_flag: &AtomicBool,
) -> Result<(), MediaFingerprintError> {
    run_tool_streaming_stdout(
        "test-tool",
        executable,
        args,
        Some(cancel_flag),
        Duration::from_secs(5),
        |_chunk| Ok(()),
    )
}

#[cfg(test)]
fn audio_constellation_streaming_decode_pcm_bytes_for_test(
    bytes: &[u8],
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    finish_pcm_stream_for_test(&[bytes])
}

#[cfg(test)]
fn audio_constellation_streaming_decode_split_bytes_for_test(
    bytes: &[u8],
) -> Result<MediaAudioStreamMetrics, MediaFingerprintError> {
    let chunks = bytes.chunks(3).collect::<Vec<_>>();
    finish_pcm_stream_for_test(&chunks).map(|(_, metrics)| metrics)
}

pub fn audio_constellation_landmarks_v3_from_pcm(
    samples: &[i16],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> Vec<AudioLandmarkV3> {
    if samples.len() < V3_AUDIO_WINDOW_SAMPLES || sample_rate == 0 {
        return Vec::new();
    }
    let min_bin =
        ((V3_AUDIO_MIN_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).ceil();
    let max_bin =
        ((V3_AUDIO_MAX_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).floor();
    let min_bin = (min_bin as usize).clamp(1, (V3_AUDIO_WINDOW_SAMPLES / 2).saturating_sub(1));
    let max_bin = (max_bin as usize).clamp(min_bin + 1, V3_AUDIO_WINDOW_SAMPLES / 2);
    let frames = audio_spectral_peak_frames_v3(samples, min_bin, max_bin);
    if frames.is_empty() {
        return Vec::new();
    }
    let mut raw = Vec::new();
    for (frame_index, peaks) in frames.iter().enumerate() {
        for anchor_peak in peaks {
            let start = frame_index + V3_AUDIO_PAIR_MIN_DELTA_FRAMES;
            let end = (frame_index + V3_AUDIO_PAIR_MAX_DELTA_FRAMES + 1).min(frames.len());
            if start >= end {
                continue;
            }
            let mut emitted = 0usize;
            'targets: for (target_frame, target_peaks) in
                frames.iter().enumerate().take(end).skip(start)
            {
                let delta_frames = target_frame.saturating_sub(frame_index);
                for target_peak in target_peaks {
                    let t_ms = audio_frame_timestamp_ms(frame_index, sample_rate);
                    let hash =
                        audio_landmark_hash_v3(anchor_peak.bin, target_peak.bin, delta_frames);
                    let strength = ((anchor_peak.magnitude + target_peak.magnitude) * 4.0)
                        .round()
                        .clamp(1.0, f32::from(u8::MAX)) as u8;
                    raw.push(AudioLandmarkV3 {
                        hash,
                        t_ms,
                        weight: strength,
                    });
                    emitted += 1;
                    if emitted >= V3_AUDIO_PAIR_FANOUT {
                        break 'targets;
                    }
                }
            }
        }
    }
    dedupe_audio_landmarks_v3(&mut raw);
    if let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 120.0) {
        downweight_edge_audio_landmarks_v3(&mut raw, duration);
    }
    bounded_time_distributed_audio_landmarks_v3(&mut raw, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

#[derive(Debug, Clone, Copy)]
struct AudioSpectralPeakV3 {
    bin: usize,
    magnitude: f32,
}

fn audio_spectral_peak_frames_v3(
    samples: &[i16],
    min_bin: usize,
    max_bin: usize,
) -> Vec<Vec<AudioSpectralPeakV3>> {
    let frame_count = (samples.len() - V3_AUDIO_WINDOW_SAMPLES) / V3_AUDIO_HOP_SAMPLES + 1;
    let hann = (0..V3_AUDIO_WINDOW_SAMPLES)
        .map(|index| {
            let phase =
                (std::f32::consts::TAU * index as f32) / (V3_AUDIO_WINDOW_SAMPLES - 1) as f32;
            0.5 - (0.5 * phase.cos())
        })
        .collect::<Vec<_>>();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(V3_AUDIO_WINDOW_SAMPLES);
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); V3_AUDIO_WINDOW_SAMPLES];
    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let start = frame_index * V3_AUDIO_HOP_SAMPLES;
        for (index, slot) in buffer.iter_mut().enumerate() {
            let sample = samples[start + index] as f32 / f32::from(i16::MAX);
            *slot = Complex::new(sample * hann[index], 0.0);
        }
        fft.process(&mut buffer);
        let magnitudes = (min_bin..max_bin)
            .map(|bin| {
                let value = buffer[bin].norm_sqr().max(f32::MIN_POSITIVE).log10();
                (bin, value)
            })
            .collect::<Vec<_>>();
        let mean = if magnitudes.is_empty() {
            0.0
        } else {
            magnitudes
                .iter()
                .map(|(_, magnitude)| *magnitude)
                .sum::<f32>()
                / magnitudes.len() as f32
        };
        let mut peaks = Vec::new();
        for (local_index, (bin, magnitude)) in magnitudes.iter().enumerate() {
            if *magnitude < mean + 0.35 {
                continue;
            }
            let left = local_index.saturating_sub(V3_AUDIO_PEAK_NEIGHBORHOOD);
            let right = (local_index + V3_AUDIO_PEAK_NEIGHBORHOOD + 1).min(magnitudes.len());
            if magnitudes[left..right]
                .iter()
                .all(|(_, neighbor)| *magnitude >= *neighbor)
            {
                peaks.push(AudioSpectralPeakV3 {
                    bin: *bin,
                    magnitude: *magnitude - mean,
                });
            }
        }
        peaks.sort_by(|left, right| {
            right
                .magnitude
                .total_cmp(&left.magnitude)
                .then_with(|| left.bin.cmp(&right.bin))
        });
        peaks.truncate(V3_AUDIO_MAX_PEAKS_PER_FRAME);
        peaks.sort_by_key(|peak| peak.bin);
        frames.push(peaks);
    }
    frames
}

fn audio_frame_timestamp_ms(frame_index: usize, sample_rate: u32) -> u32 {
    let samples = frame_index.saturating_mul(V3_AUDIO_HOP_SAMPLES) as u64;
    ((samples * 1000) / u64::from(sample_rate)).min(u64::from(u32::MAX)) as u32
}

fn audio_landmark_hash_v3(anchor_bin: usize, target_bin: usize, delta_frames: usize) -> u32 {
    let anchor_bin = (anchor_bin as u32 / 2).min(0x3ff);
    let target_bin = (target_bin as u32 / 2).min(0x3ff);
    let delta = (delta_frames as u32).min(0x3ff);
    let packed = anchor_bin | (target_bin << 10) | (delta << 20);
    stable_hash_u64(packed.to_le_bytes()) as u32
}

fn dedupe_audio_landmarks_v3(landmarks: &mut Vec<AudioLandmarkV3>) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    landmarks.dedup_by(|left, right| left.t_ms == right.t_ms && left.hash == right.hash);
}

fn downweight_edge_audio_landmarks_v3(landmarks: &mut [AudioLandmarkV3], duration_seconds: f64) {
    let edge_ms = (duration_seconds * 1000.0 * 0.08).clamp(30_000.0, 120_000.0) as u32;
    let duration_ms = (duration_seconds * 1000.0).min(f64::from(u32::MAX)) as u32;
    for landmark in landmarks {
        if landmark.t_ms < edge_ms || landmark.t_ms > duration_ms.saturating_sub(edge_ms) {
            landmark.weight = landmark.weight.saturating_sub(landmark.weight / 2).max(1);
        }
    }
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

fn video_fingerprint_from_ffmpeg_rawvideo(
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

fn video_frames_from_ffmpeg_rawvideo(
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
                media_match_candidate_aligned_pairs(&right.decision)
                    .cmp(&media_match_candidate_aligned_pairs(&left.decision))
            })
            .then_with(|| {
                media_match_candidate_aligned_span(&right.decision)
                    .total_cmp(&media_match_candidate_aligned_span(&left.decision))
            })
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

fn media_match_candidate_aligned_pairs(decision: &MediaMatchDecision) -> usize {
    decision
        .evidence
        .alignment
        .as_ref()
        .map(|alignment| alignment.aligned_pairs)
        .or_else(|| {
            decision
                .evidence
                .video
                .as_ref()
                .map(|video| video.aligned_pairs)
        })
        .unwrap_or(0)
}

fn media_match_candidate_aligned_span(decision: &MediaMatchDecision) -> f64 {
    decision
        .evidence
        .alignment
        .as_ref()
        .map(|alignment| alignment.aligned_span_seconds)
        .unwrap_or(0.0)
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

impl AnchorMatchPair {
    fn modality_order(self) -> u8 {
        match self.modality {
            AnchorModality::Audio => 0,
            AnchorModality::Video => 1,
        }
    }
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

#[derive(Debug, Clone)]
struct V3SegmentCandidate {
    query_start_ms: u32,
    query_end_ms: u32,
    candidate_start_ms: u32,
    candidate_end_ms: u32,
    scale_ppm: i32,
    audio_pairs: usize,
    video_pairs: usize,
    weighted_score: u32,
    residual_ms: f64,
    confidence: f32,
}

#[derive(Debug, Clone)]
struct V3TimelineAnalysis {
    segments: Vec<V3SegmentCandidate>,
    total_aligned_span_ms: u32,
    largest_gap_ms: u32,
    edge_only: bool,
    audio_video_conflict: bool,
    best_segment_score: u32,
    second_best_segment_score: u32,
    audio_pairs: usize,
    video_pairs: usize,
    piecewise_pair_count: usize,
    piecewise_hypothesis_count: usize,
    piecewise_segment_candidate_count: usize,
    piecewise_segment_chain_count: usize,
    piecewise_fit_millis: u64,
}

#[derive(Debug, Clone, Copy)]
struct V3ClassificationContext {
    duration_ok: bool,
    meaningful_span: bool,
    drift_ok: bool,
    margin_ok: bool,
    continuity_ok: bool,
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
        v3_class: None,
        timeline_map_v3: None,
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
    let query_video_coverage = anchor_coverage(
        video_pairs,
        unique_video_frame_anchor_count(&query.video_anchors),
    );
    let candidate_video_coverage = anchor_coverage(
        video_pairs,
        unique_video_frame_anchor_count(&candidate.video_anchors),
    );
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
    let timeline_analysis = build_v3_timeline_analysis(query, candidate, &pairs);
    let v3_class = classify_v3_timeline(
        query,
        candidate,
        &timeline_analysis,
        V3ClassificationContext {
            duration_ok,
            meaningful_span,
            drift_ok,
            margin_ok,
            continuity_ok,
        },
    );
    let video_inconclusive = !query.video_anchors.is_empty()
        && !candidate.video_anchors.is_empty()
        && timeline_analysis.video_pairs < V3_SEGMENT_VIDEO_MIN_PAIRS
        && !timeline_analysis.audio_video_conflict;
    evidence.notes.push(format!(
        "v3 segments={} span={:.1}s largest_gap={:.1}s edge_only={} audio_video_conflict={} video_inconclusive={} best_segment_score={} second_segment_score={} pair_count={} hypotheses={} segment_candidates={} chained_segments={} piecewise_fit_ms={}",
        timeline_analysis.segments.len(),
        f64::from(timeline_analysis.total_aligned_span_ms) / 1000.0,
        f64::from(timeline_analysis.largest_gap_ms) / 1000.0,
        timeline_analysis.edge_only,
        timeline_analysis.audio_video_conflict,
        video_inconclusive,
        timeline_analysis.best_segment_score,
        timeline_analysis.second_best_segment_score,
        timeline_analysis.piecewise_pair_count,
        timeline_analysis.piecewise_hypothesis_count,
        timeline_analysis.piecewise_segment_candidate_count,
        timeline_analysis.piecewise_segment_chain_count,
        timeline_analysis.piecewise_fit_millis
    ));

    let tier = match v3_class {
        MatchClassV3::SameCutStrong
            if (both_modalities
                && meaningful_span
                && drift_ok
                && margin_ok
                && duration_ok
                && continuity_ok)
                || (very_strong_single_modality
                    && drift_ok
                    && (duration_ok || span_seconds >= 300.0)) =>
        {
            MediaMatchTier::Strong
        }
        MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable => MediaMatchTier::Probable,
        MatchClassV3::SameMediaDifferentCut
        | MatchClassV3::SameAudioDifferentVideo
        | MatchClassV3::SameVideoDifferentAudio => MediaMatchTier::Probable,
        MatchClassV3::PartialOverlap => MediaMatchTier::Weak,
        MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject => MediaMatchTier::Reject,
        MatchClassV3::Unknown => {
            if weak_evidence {
                MediaMatchTier::Weak
            } else {
                MediaMatchTier::Unknown
            }
        }
    };
    let timeline_map = media_timeline_map_v3_from_analysis(v3_class, &timeline_analysis);
    evidence.timeline_map_v3 = Some(timeline_map);

    let explanation = match v3_class {
        MatchClassV3::SameCutStrong => "anchor timelines strongly align across the same cut",
        MatchClassV3::SameCutProbable => {
            "anchor timelines align but evidence is below same-cut strong confidence"
        }
        MatchClassV3::SameMediaDifferentCut => {
            "anchor timelines align in multiple body segments with edit differences"
        }
        MatchClassV3::SameAudioDifferentVideo => {
            "audio timeline aligns but video evidence conflicts or is absent"
        }
        MatchClassV3::SameVideoDifferentAudio => {
            "video timeline aligns but audio evidence conflicts or is absent"
        }
        MatchClassV3::PartialOverlap => "partial anchor timeline overlap",
        MatchClassV3::SharedIntroOutroOnly => {
            "anchor timeline evidence is concentrated at shared edges"
        }
        MatchClassV3::Reject => "anchor timeline evidence is insufficient",
        MatchClassV3::Unknown => "insufficient comparable anchor evidence",
    };
    decision(tier, evidence, explanation)
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
        v3_class: None,
        timeline_map_v3: None,
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

/// Legacy diagnostic helper for direct frame-hash sequence alignment.
///
/// Media Matching decisions use compact time-local anchors via
/// [`decide_media_match_anchors`] instead of this non-queryable comparison.
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

fn run_tool_streaming_stdout<I, F>(
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
    let mut candidate_video_all = Vec::new();
    for anchor in &candidate.video_anchors {
        candidate_video
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
        candidate_video_all.push(anchor);
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
    let mut seen_video_pairs = HashSet::<(u32, u32, u8, u64, u64)>::new();
    for query_anchor in &query.video_anchors {
        if let Some(candidate_anchors) = candidate_video.get(&query_anchor.bucket) {
            for candidate_anchor in candidate_anchors {
                push_video_anchor_match_pair(
                    &mut pairs,
                    &mut seen_video_pairs,
                    query_anchor,
                    candidate_anchor,
                );
            }
        }
        for candidate_anchor in &candidate_video_all {
            push_video_anchor_match_pair(
                &mut pairs,
                &mut seen_video_pairs,
                query_anchor,
                candidate_anchor,
            );
        }
    }
    pairs
}

fn push_video_anchor_match_pair(
    pairs: &mut Vec<AnchorMatchPair>,
    seen_video_pairs: &mut HashSet<(u32, u32, u8, u64, u64)>,
    query_anchor: &VideoAnchor,
    candidate_anchor: &VideoAnchor,
) -> bool {
    if !v3_video_kind_is_supported(query_anchor.kind)
        || !v3_video_kind_is_supported(candidate_anchor.kind)
        || !v3_video_bucket_kind_matches(query_anchor.kind, query_anchor.bucket)
        || !v3_video_bucket_kind_matches(candidate_anchor.kind, candidate_anchor.bucket)
    {
        return false;
    }
    if query_anchor.kind != candidate_anchor.kind {
        return false;
    }
    if !v3_video_anchor_hashes_match(
        query_anchor.kind,
        query_anchor.hash64,
        candidate_anchor.hash64,
    ) {
        return false;
    }
    if !seen_video_pairs.insert((
        query_anchor.t_ms,
        candidate_anchor.t_ms,
        query_anchor.kind,
        query_anchor.hash64,
        candidate_anchor.hash64,
    )) {
        return true;
    }
    pairs.push(AnchorMatchPair {
        query_t_ms: query_anchor.t_ms,
        candidate_t_ms: candidate_anchor.t_ms,
        modality: AnchorModality::Video,
        weight: query_anchor.weight.min(candidate_anchor.weight),
    });
    true
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
    add_scale_offset_candidates_from_pairs(&seeded, &mut candidates);
    if seeded.len() < pairs.len() {
        let broad_pairs = broad_scale_fit_sample(pairs);
        add_scale_offset_candidates_from_pairs(&broad_pairs, &mut candidates);
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

fn add_scale_offset_candidates_from_pairs(
    pairs: &[AnchorMatchPair],
    candidates: &mut Vec<(f64, f64)>,
) {
    for (left_index, left) in pairs.iter().enumerate() {
        for right in pairs.iter().skip(left_index + 1) {
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
}

fn broad_scale_fit_sample(pairs: &[AnchorMatchPair]) -> Vec<AnchorMatchPair> {
    if pairs.len() <= MAX_BROAD_SCALE_FIT_PAIRS {
        return pairs.to_vec();
    }
    let stride = pairs.len() as f64 / MAX_BROAD_SCALE_FIT_PAIRS as f64;
    (0..MAX_BROAD_SCALE_FIT_PAIRS)
        .map(|index| pairs[(index as f64 * stride).floor() as usize])
        .collect()
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

fn build_v3_timeline_analysis(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
) -> V3TimelineAnalysis {
    let started_at = Instant::now();
    let mut hypotheses = Vec::<(f64, f64)>::new();
    let mut offset_bins = pairs
        .iter()
        .map(|pair| rounded_offset_bin(i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms)))
        .collect::<Vec<_>>();
    offset_bins.sort_unstable();
    offset_bins.dedup();
    for offset_bin in offset_bins {
        hypotheses.push((1.0, (offset_bin * DEFAULT_ANCHOR_OFFSET_BIN_MS) as f64));
    }
    let hypothesis_pairs = select_v3_piecewise_hypothesis_pairs(pairs);
    add_v3_piecewise_hypotheses_from_pairs(&hypothesis_pairs, &mut hypotheses);
    let piecewise_hypothesis_count = hypotheses.len();

    let mut segment_candidates = Vec::<V3SegmentCandidate>::new();
    for (scale, offset) in hypotheses {
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.len() < 2 {
            continue;
        }
        let (scale, offset) = least_squares_anchor_fit(&inliers).unwrap_or((scale, offset));
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        segment_candidates.extend(v3_segments_from_inliers(&inliers, scale, offset));
    }
    let piecewise_segment_candidate_count = segment_candidates.len();
    let mut segments = chain_v3_segments(dedup_v3_segments(segment_candidates));
    let piecewise_segment_chain_count = segments.len();
    merge_adjacent_v3_segments(&mut segments);
    let total_aligned_span_ms = segments
        .iter()
        .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
        .sum::<u32>();
    let largest_gap_ms = largest_v3_segment_gap_ms(&segments);
    let mut scores = segments
        .iter()
        .map(|segment| segment.weighted_score)
        .collect::<Vec<_>>();
    scores.sort_unstable_by(|left, right| right.cmp(left));
    let best_segment_score = scores.first().copied().unwrap_or(0);
    let second_best_segment_score = scores.get(1).copied().unwrap_or(0);
    let audio_pairs = segments.iter().map(|segment| segment.audio_pairs).sum();
    let video_pairs = segments.iter().map(|segment| segment.video_pairs).sum();
    let edge_only = v3_segments_are_edge_only(&segments, query.duration_ms, candidate.duration_ms);
    let audio_video_conflict = v3_audio_video_conflict(query, candidate, pairs, &segments);
    V3TimelineAnalysis {
        segments,
        total_aligned_span_ms,
        largest_gap_ms,
        edge_only,
        audio_video_conflict,
        best_segment_score,
        second_best_segment_score,
        audio_pairs,
        video_pairs,
        piecewise_pair_count: pairs.len(),
        piecewise_hypothesis_count,
        piecewise_segment_candidate_count,
        piecewise_segment_chain_count,
        piecewise_fit_millis: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

fn select_v3_piecewise_hypothesis_pairs(pairs: &[AnchorMatchPair]) -> Vec<AnchorMatchPair> {
    if pairs.len() <= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS {
        return pairs.to_vec();
    }
    let mut bin_scores = HashMap::<i64, u32>::new();
    for pair in pairs {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        *bin_scores.entry(rounded_offset_bin(offset)).or_default() += u32::from(pair.weight.max(1));
    }
    let mut bins = bin_scores.into_iter().collect::<Vec<_>>();
    bins.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut selected = Vec::with_capacity(V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS);
    let mut seen = HashSet::<(u32, u32, u8)>::new();
    for modality in [AnchorModality::Audio, AnchorModality::Video] {
        let modality_quota = V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS / 8;
        for (bin, _) in &bins {
            let mut candidates = pairs
                .iter()
                .copied()
                .filter(|pair| {
                    pair.modality == modality
                        && rounded_offset_bin(
                            i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms),
                        ) == *bin
                })
                .collect::<Vec<_>>();
            candidates.sort_by(v3_hypothesis_pair_order);
            for pair in candidates {
                push_v3_hypothesis_pair(&mut selected, &mut seen, pair);
                if selected
                    .iter()
                    .filter(|selected| selected.modality == modality)
                    .count()
                    >= modality_quota
                    || selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS
                {
                    break;
                }
            }
            if selected
                .iter()
                .filter(|selected| selected.modality == modality)
                .count()
                >= modality_quota
                || selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS
            {
                break;
            }
        }
    }
    for (bin, _) in bins {
        let mut candidates = pairs
            .iter()
            .copied()
            .filter(|pair| {
                rounded_offset_bin(i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms))
                    == bin
            })
            .collect::<Vec<_>>();
        candidates.sort_by(v3_hypothesis_pair_order);
        for pair in candidates {
            push_v3_hypothesis_pair(&mut selected, &mut seen, pair);
            if selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS {
                return selected;
            }
        }
    }
    selected
}

fn push_v3_hypothesis_pair(
    selected: &mut Vec<AnchorMatchPair>,
    seen: &mut HashSet<(u32, u32, u8)>,
    pair: AnchorMatchPair,
) {
    if seen.insert((pair.query_t_ms, pair.candidate_t_ms, pair.modality_order())) {
        selected.push(pair);
    }
}

fn v3_hypothesis_pair_order(left: &AnchorMatchPair, right: &AnchorMatchPair) -> std::cmp::Ordering {
    right
        .weight
        .cmp(&left.weight)
        .then_with(|| left.query_t_ms.cmp(&right.query_t_ms))
        .then_with(|| left.candidate_t_ms.cmp(&right.candidate_t_ms))
        .then_with(|| left.modality_order().cmp(&right.modality_order()))
}

fn add_v3_piecewise_hypotheses_from_pairs(
    pairs: &[AnchorMatchPair],
    hypotheses: &mut Vec<(f64, f64)>,
) {
    for (left_index, left) in pairs.iter().enumerate() {
        for right in pairs.iter().skip(left_index + 1) {
            let query_delta = right.query_t_ms.abs_diff(left.query_t_ms);
            if query_delta < V3_SEGMENT_MIN_PAIR_DELTA_MS {
                continue;
            }
            let query_delta = f64::from(right.query_t_ms) - f64::from(left.query_t_ms);
            let candidate_delta = f64::from(right.candidate_t_ms) - f64::from(left.candidate_t_ms);
            if query_delta.abs() < f64::EPSILON {
                continue;
            }
            let scale = candidate_delta / query_delta;
            if !(0.95..=1.05).contains(&scale) {
                continue;
            }
            let offset = f64::from(left.candidate_t_ms) - (scale * f64::from(left.query_t_ms));
            hypotheses.push((scale, offset));
        }
    }
    hypotheses.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    hypotheses.dedup_by(|left, right| {
        (left.0 - right.0).abs() < 0.000_1 && (left.1 - right.1).abs() < 250.0
    });
}

fn v3_segments_from_inliers(
    inliers: &[AnchorMatchPair],
    scale: f64,
    offset: f64,
) -> Vec<V3SegmentCandidate> {
    let mut sorted = inliers.to_vec();
    sorted.sort_by_key(|pair| (pair.query_t_ms, pair.candidate_t_ms, pair.modality_order()));
    let mut segments = Vec::new();
    let mut current = Vec::<AnchorMatchPair>::new();
    for pair in sorted {
        if let Some(previous) = current.last().copied() {
            let query_gap = pair.query_t_ms.saturating_sub(previous.query_t_ms);
            let candidate_gap = pair.candidate_t_ms.saturating_sub(previous.candidate_t_ms);
            let current_span = aligned_anchor_span_ms(&current);
            let gap_threshold =
                V3_SEGMENT_SPLIT_GAP_MS.max((f64::from(current_span) * 0.15).round() as u32);
            let gap_delta = query_gap.abs_diff(candidate_gap);
            let large_common_gap = query_gap.max(candidate_gap) > 300_000;
            if gap_delta > gap_threshold || large_common_gap {
                if let Some(segment) = v3_segment_candidate_from_pairs(&current, scale, offset) {
                    segments.push(segment);
                }
                current.clear();
            }
        }
        current.push(pair);
    }
    if let Some(segment) = v3_segment_candidate_from_pairs(&current, scale, offset) {
        segments.push(segment);
    }
    segments
}

fn v3_segment_candidate_from_pairs(
    pairs: &[AnchorMatchPair],
    scale: f64,
    offset: f64,
) -> Option<V3SegmentCandidate> {
    if pairs.is_empty() {
        return None;
    }
    let audio_pairs = pairs
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Audio)
        .count();
    let video_pairs = pairs
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Video)
        .count();
    let span_ms = aligned_anchor_span_ms(pairs);
    let enough = if audio_pairs >= V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS
        && video_pairs >= V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS
    {
        span_ms >= V3_SEGMENT_AUDIO_VIDEO_MIN_SPAN_MS
    } else if audio_pairs >= V3_SEGMENT_AUDIO_MIN_PAIRS {
        span_ms >= V3_SEGMENT_AUDIO_MIN_SPAN_MS
    } else if video_pairs >= V3_SEGMENT_VIDEO_MIN_PAIRS {
        span_ms >= V3_SEGMENT_VIDEO_MIN_SPAN_MS
    } else {
        false
    };
    if !enough {
        return None;
    }
    let (query_start_ms, query_end_ms, candidate_start_ms, candidate_end_ms) =
        aligned_anchor_bounds(pairs);
    let weighted_score = pairs
        .iter()
        .map(|pair| u32::from(pair.weight.max(1)))
        .sum::<u32>();
    let residual_ms = max_anchor_fit_residual_ms(pairs, scale, offset);
    let scale_ppm = (scale * 1_000_000.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let confidence = (weighted_score as f32 / 64.0)
        .min(1.0)
        .max((span_ms as f32 / 600_000.0).min(1.0));
    Some(V3SegmentCandidate {
        query_start_ms,
        query_end_ms,
        candidate_start_ms,
        candidate_end_ms,
        scale_ppm,
        audio_pairs,
        video_pairs,
        weighted_score,
        residual_ms,
        confidence,
    })
}

fn dedup_v3_segments(mut segments: Vec<V3SegmentCandidate>) -> Vec<V3SegmentCandidate> {
    segments.sort_by(|left, right| {
        left.query_start_ms
            .cmp(&right.query_start_ms)
            .then_with(|| left.query_end_ms.cmp(&right.query_end_ms))
            .then_with(|| left.candidate_start_ms.cmp(&right.candidate_start_ms))
            .then_with(|| left.candidate_end_ms.cmp(&right.candidate_end_ms))
            .then_with(|| right.weighted_score.cmp(&left.weighted_score))
    });
    let mut deduped = Vec::<V3SegmentCandidate>::new();
    for segment in segments {
        let duplicate = deduped.iter().any(|current| {
            current.query_start_ms.abs_diff(segment.query_start_ms) <= 1_000
                && current.query_end_ms.abs_diff(segment.query_end_ms) <= 1_000
                && current
                    .candidate_start_ms
                    .abs_diff(segment.candidate_start_ms)
                    <= 1_000
                && current.candidate_end_ms.abs_diff(segment.candidate_end_ms) <= 1_000
        });
        if !duplicate {
            deduped.push(segment);
        }
    }
    deduped
}

fn chain_v3_segments(mut segments: Vec<V3SegmentCandidate>) -> Vec<V3SegmentCandidate> {
    if segments.len() <= 1 {
        return segments;
    }
    segments.sort_by(|left, right| {
        left.query_start_ms
            .cmp(&right.query_start_ms)
            .then_with(|| left.candidate_start_ms.cmp(&right.candidate_start_ms))
            .then_with(|| right.weighted_score.cmp(&left.weighted_score))
    });
    let mut best_scores = vec![0i64; segments.len()];
    let mut previous = vec![None; segments.len()];
    for index in 0..segments.len() {
        best_scores[index] = v3_segment_chain_score(&segments[index]);
        for prev_index in 0..index {
            if !v3_segments_are_chain_compatible(&segments[prev_index], &segments[index]) {
                continue;
            }
            let candidate_score =
                best_scores[prev_index] + v3_segment_chain_score(&segments[index]);
            if candidate_score > best_scores[index]
                || (candidate_score == best_scores[index]
                    && previous[index].is_none_or(|current| prev_index < current))
            {
                best_scores[index] = candidate_score;
                previous[index] = Some(prev_index);
            }
        }
    }
    let Some((mut index, _)) = best_scores
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(&left.0)))
    else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    loop {
        chain.push(segments[index].clone());
        let Some(prev_index) = previous[index] else {
            break;
        };
        index = prev_index;
    }
    chain.reverse();
    chain
}

fn v3_segment_chain_score(segment: &V3SegmentCandidate) -> i64 {
    i64::from(segment.weighted_score) * 1_000
        + i64::from(segment.query_end_ms.saturating_sub(segment.query_start_ms) / 1_000)
}

fn v3_segments_are_chain_compatible(left: &V3SegmentCandidate, right: &V3SegmentCandidate) -> bool {
    left.query_end_ms <= right.query_start_ms && left.candidate_end_ms <= right.candidate_start_ms
}

fn merge_adjacent_v3_segments(segments: &mut Vec<V3SegmentCandidate>) {
    if segments.len() < 2 {
        return;
    }
    let mut merged = Vec::<V3SegmentCandidate>::new();
    for segment in segments.drain(..) {
        if let Some(previous) = merged.last_mut()
            && v3_segments_can_merge(previous, &segment)
        {
            previous.query_end_ms = previous.query_end_ms.max(segment.query_end_ms);
            previous.candidate_end_ms = previous.candidate_end_ms.max(segment.candidate_end_ms);
            previous.audio_pairs += segment.audio_pairs;
            previous.video_pairs += segment.video_pairs;
            previous.weighted_score += segment.weighted_score;
            previous.residual_ms = previous.residual_ms.max(segment.residual_ms);
            previous.confidence = previous.confidence.max(segment.confidence);
            continue;
        }
        merged.push(segment);
    }
    *segments = merged;
}

fn v3_segments_can_merge(left: &V3SegmentCandidate, right: &V3SegmentCandidate) -> bool {
    left.query_end_ms <= right.query_start_ms
        && left.candidate_end_ms <= right.candidate_start_ms
        && right.query_start_ms.saturating_sub(left.query_end_ms) <= V3_SEGMENT_MERGE_GAP_MS
        && right
            .candidate_start_ms
            .saturating_sub(left.candidate_end_ms)
            <= V3_SEGMENT_MERGE_GAP_MS
        && (left.scale_ppm - right.scale_ppm).abs() <= V3_SEGMENT_MERGE_SCALE_PPM
}

fn largest_v3_segment_gap_ms(segments: &[V3SegmentCandidate]) -> u32 {
    segments
        .windows(2)
        .map(|pair| {
            pair[1]
                .query_start_ms
                .saturating_sub(pair[0].query_end_ms)
                .max(
                    pair[1]
                        .candidate_start_ms
                        .saturating_sub(pair[0].candidate_end_ms),
                )
        })
        .max()
        .unwrap_or(0)
}

fn v3_segments_are_edge_only(
    segments: &[V3SegmentCandidate],
    query_duration_ms: Option<u32>,
    candidate_duration_ms: Option<u32>,
) -> bool {
    if segments.is_empty() {
        return false;
    }
    let query_edge = v3_edge_region_ms(query_duration_ms);
    let candidate_edge = v3_edge_region_ms(candidate_duration_ms);
    segments.iter().all(|segment| {
        v3_range_is_edge_only(
            segment.query_start_ms,
            segment.query_end_ms,
            query_duration_ms,
            query_edge,
        ) && v3_range_is_edge_only(
            segment.candidate_start_ms,
            segment.candidate_end_ms,
            candidate_duration_ms,
            candidate_edge,
        )
    })
}

fn v3_edge_region_ms(duration_ms: Option<u32>) -> u32 {
    duration_ms
        .map(|duration| {
            ((f64::from(duration) * 0.15).round() as u32)
                .clamp(V3_EDGE_REGION_MIN_MS, V3_EDGE_REGION_MAX_MS)
        })
        .unwrap_or(V3_EDGE_REGION_MIN_MS)
}

fn v3_range_is_edge_only(
    start_ms: u32,
    end_ms: u32,
    duration_ms: Option<u32>,
    edge_ms: u32,
) -> bool {
    end_ms <= edge_ms
        || duration_ms.is_some_and(|duration| start_ms >= duration.saturating_sub(edge_ms))
}

fn v3_audio_video_conflict(
    _query: &MediaAnchorProfile,
    _candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
    _segments: &[V3SegmentCandidate],
) -> bool {
    let audio_offset = dominant_modality_offset_bin(pairs, AnchorModality::Audio);
    let video_offset = dominant_modality_offset_bin(pairs, AnchorModality::Video);
    matches!(
        (audio_offset, video_offset),
        (Some((audio_bin, audio_score)), Some((video_bin, video_score)))
            if audio_score >= V3_SEGMENT_AUDIO_MIN_PAIRS as u32
                && video_score >= V3_SEGMENT_VIDEO_MIN_PAIRS as u32
                && (audio_bin - video_bin).abs() * DEFAULT_ANCHOR_OFFSET_BIN_MS
                    > DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
    )
}

fn dominant_modality_offset_bin(
    pairs: &[AnchorMatchPair],
    modality: AnchorModality,
) -> Option<(i64, u32)> {
    let mut bins = HashMap::<i64, u32>::new();
    for pair in pairs.iter().filter(|pair| pair.modality == modality) {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        *bins.entry(rounded_offset_bin(offset)).or_default() += u32::from(pair.weight.max(1));
    }
    bins.into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
}

fn v3_segments_have_material_timeline_change(segments: &[V3SegmentCandidate]) -> bool {
    segments.windows(2).any(|pair| {
        let left = &pair[0];
        let right = &pair[1];
        let left_offset = i64::from(left.candidate_start_ms) - i64::from(left.query_start_ms);
        let right_offset = i64::from(right.candidate_start_ms) - i64::from(right.query_start_ms);
        (left_offset - right_offset).abs() > DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
            || (left.scale_ppm - right.scale_ppm).abs() > V3_SEGMENT_MERGE_SCALE_PPM
    })
}

fn classify_v3_timeline(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    analysis: &V3TimelineAnalysis,
    context: V3ClassificationContext,
) -> MatchClassV3 {
    if analysis.segments.is_empty() {
        return MatchClassV3::Reject;
    }
    if analysis.edge_only {
        return MatchClassV3::SharedIntroOutroOnly;
    }
    let has_query_audio = !query.audio_anchors.is_empty();
    let has_candidate_audio = !candidate.audio_anchors.is_empty();
    let audio_strong = analysis.audio_pairs >= V3_SEGMENT_AUDIO_MIN_PAIRS;
    let video_strong = analysis.video_pairs >= V3_SEGMENT_VIDEO_MIN_PAIRS;
    if analysis.audio_video_conflict && audio_strong && analysis.audio_pairs >= analysis.video_pairs
    {
        return MatchClassV3::SameAudioDifferentVideo;
    }
    if analysis.audio_video_conflict && video_strong {
        return MatchClassV3::SameVideoDifferentAudio;
    }
    if video_strong && (!has_query_audio || !has_candidate_audio) {
        return MatchClassV3::SameVideoDifferentAudio;
    }
    let query_coverage = query
        .duration_ms
        .filter(|duration| *duration > 0)
        .map(|duration| f64::from(analysis.total_aligned_span_ms) / f64::from(duration))
        .unwrap_or(1.0);
    let candidate_coverage = candidate
        .duration_ms
        .filter(|duration| *duration > 0)
        .map(|duration| f64::from(analysis.total_aligned_span_ms) / f64::from(duration))
        .unwrap_or(1.0);
    let broad_body_coverage = query_coverage.min(candidate_coverage) >= 0.25;
    let material_segment_change = v3_segments_have_material_timeline_change(&analysis.segments);
    let clear_edit = material_segment_change
        || (context.meaningful_span && broad_body_coverage && !context.duration_ok);
    if clear_edit && analysis.total_aligned_span_ms >= 120_000 {
        return MatchClassV3::SameMediaDifferentCut;
    }
    if context.meaningful_span
        && broad_body_coverage
        && context.drift_ok
        && context.margin_ok
        && context.continuity_ok
        && (audio_strong || video_strong)
    {
        return MatchClassV3::SameCutStrong;
    }
    if context.meaningful_span
        && broad_body_coverage
        && analysis.total_aligned_span_ms >= 120_000
        && (audio_strong || video_strong)
    {
        return MatchClassV3::SameCutProbable;
    }
    if analysis.total_aligned_span_ms >= 45_000 {
        return MatchClassV3::PartialOverlap;
    }
    MatchClassV3::Reject
}

fn media_timeline_map_v3_from_analysis(
    global_class: MatchClassV3,
    analysis: &V3TimelineAnalysis,
) -> MediaTimelineMapV3 {
    let segments = analysis
        .segments
        .iter()
        .map(|segment| {
            let total_pairs = (segment.audio_pairs + segment.video_pairs).max(1) as f32;
            AlignedSegmentV3 {
                query_start_ms: segment.query_start_ms,
                query_end_ms: segment.query_end_ms,
                candidate_start_ms: segment.candidate_start_ms,
                candidate_end_ms: segment.candidate_end_ms,
                scale_ppm: segment.scale_ppm,
                audio_pairs: segment.audio_pairs,
                video_pairs: segment.video_pairs,
                weighted_score: segment.weighted_score,
                residual_ms: segment.residual_ms,
                audio_score: segment.audio_pairs as f32 / total_pairs,
                video_score: segment.video_pairs as f32 / total_pairs,
                confidence: segment.confidence,
            }
        })
        .collect();
    MediaTimelineMapV3 {
        global_class,
        current_position_class: global_class,
        segments,
        total_aligned_span_ms: analysis.total_aligned_span_ms,
        largest_gap_ms: analysis.largest_gap_ms,
        edge_only: analysis.edge_only,
        audio_video_conflict: analysis.audio_video_conflict,
        best_segment_score: analysis.best_segment_score,
        second_best_segment_score: analysis.second_best_segment_score,
        piecewise_pair_count: analysis.piecewise_pair_count,
        piecewise_hypothesis_count: analysis.piecewise_hypothesis_count,
        piecewise_segment_candidate_count: analysis.piecewise_segment_candidate_count,
        piecewise_segment_chain_count: analysis.piecewise_segment_chain_count,
        piecewise_fit_millis: analysis.piecewise_fit_millis,
    }
}

fn anchor_coverage(aligned: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        aligned as f64 / total as f64
    }
}

fn unique_video_frame_anchor_count(anchors: &[VideoAnchor]) -> usize {
    anchors
        .iter()
        .map(|anchor| (anchor.t_ms, anchor.kind, anchor.hash64))
        .collect::<HashSet<_>>()
        .len()
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

fn bounded_time_distributed_video_anchors(
    anchors: &mut [VideoAnchor],
    max_anchors: usize,
) -> Vec<VideoAnchor> {
    anchors.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.hash64, anchor.kind));
    if anchors.len() <= max_anchors {
        return anchors.to_vec();
    }
    let stride = anchors.len() as f64 / max_anchors as f64;
    (0..max_anchors)
        .map(|index| anchors[(index as f64 * stride).floor() as usize])
        .collect()
}

fn bounded_time_distributed_audio_landmarks_v3(
    landmarks: &mut [AudioLandmarkV3],
    max_landmarks: usize,
) -> Vec<AudioLandmarkV3> {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    if max_landmarks == 0 {
        return Vec::new();
    }
    if landmarks.len() <= max_landmarks {
        return landmarks.to_vec();
    }
    let stride = landmarks.len() as f64 / max_landmarks as f64;
    (0..max_landmarks)
        .map(|index| landmarks[(index as f64 * stride).floor() as usize])
        .collect()
}

fn bounded_time_distributed_video_landmarks_v3(
    landmarks: &mut [VideoLandmarkV3],
    max_landmarks: usize,
) -> Vec<VideoLandmarkV3> {
    if max_landmarks == 0 {
        return Vec::new();
    }
    let mut valid = landmarks
        .iter()
        .copied()
        .filter(|landmark| {
            v3_video_kind_is_supported(landmark.kind)
                && v3_video_bucket_kind_matches(landmark.kind, landmark.bucket)
        })
        .collect::<Vec<_>>();
    sort_video_landmarks_for_bounding(&mut valid);
    if valid.len() <= max_landmarks {
        return valid;
    }

    let index_profile = max_landmarks <= V3_VIDEO_INDEX_LANDMARK_LIMIT;
    let kind_order = [
        V3_VIDEO_KIND_TEMPORAL_SHINGLE,
        V3_VIDEO_KIND_GLOBAL_DCT,
        V3_VIDEO_KIND_CENTER_DCT,
        V3_VIDEO_KIND_EDGE,
        V3_VIDEO_KIND_LEGACY_LUMA,
    ];
    let mut selected = Vec::with_capacity(max_landmarks);
    let mut seen = HashSet::new();

    for kind in kind_order {
        let candidates = valid
            .iter()
            .copied()
            .filter(|landmark| landmark.kind == kind)
            .collect::<Vec<_>>();
        if candidates.is_empty() || selected.len() >= max_landmarks {
            continue;
        }
        let quota = v3_video_kind_quota(max_landmarks, kind, index_profile)
            .max(usize::from(kind != V3_VIDEO_KIND_LEGACY_LUMA))
            .min(max_landmarks - selected.len())
            .min(candidates.len());
        for landmark in select_time_distributed_video_landmarks_v3(&candidates, quota) {
            if seen.insert(video_landmark_key(&landmark)) {
                selected.push(landmark);
            }
        }
    }

    while selected.len() < max_landmarks {
        let mut progressed = false;
        for kind in kind_order {
            if selected.len() >= max_landmarks {
                break;
            }
            let candidates = valid
                .iter()
                .copied()
                .filter(|landmark| {
                    landmark.kind == kind && !seen.contains(&video_landmark_key(landmark))
                })
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                continue;
            }
            let Some(landmark) = select_time_distributed_video_landmarks_v3(&candidates, 1)
                .into_iter()
                .next()
            else {
                continue;
            };
            seen.insert(video_landmark_key(&landmark));
            selected.push(landmark);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    sort_video_landmarks_for_bounding(&mut selected);
    selected.truncate(max_landmarks);
    selected
}

fn v3_video_kind_quota(max_landmarks: usize, kind: u8, index_profile: bool) -> usize {
    let percent = if index_profile {
        match kind {
            V3_VIDEO_KIND_TEMPORAL_SHINGLE => 50,
            V3_VIDEO_KIND_GLOBAL_DCT => 17,
            V3_VIDEO_KIND_CENTER_DCT => 17,
            V3_VIDEO_KIND_EDGE => 16,
            _ => 0,
        }
    } else {
        match kind {
            V3_VIDEO_KIND_TEMPORAL_SHINGLE => 40,
            V3_VIDEO_KIND_GLOBAL_DCT => 25,
            V3_VIDEO_KIND_CENTER_DCT => 20,
            V3_VIDEO_KIND_EDGE => 15,
            _ => 0,
        }
    };
    (max_landmarks * percent) / 100
}

fn select_time_distributed_video_landmarks_v3(
    landmarks: &[VideoLandmarkV3],
    limit: usize,
) -> Vec<VideoLandmarkV3> {
    if limit == 0 || landmarks.is_empty() {
        return Vec::new();
    }
    let mut sorted = landmarks.to_vec();
    sort_video_landmarks_for_bounding(&mut sorted);
    if sorted.len() <= limit {
        return sorted;
    }
    let stride = sorted.len() as f64 / limit as f64;
    (0..limit)
        .map(|index| sorted[(index as f64 * stride).floor() as usize])
        .collect()
}

fn sort_video_landmarks_for_bounding(landmarks: &mut [VideoLandmarkV3]) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            video_kind_bounding_priority(landmark.kind),
            landmark.bucket,
            landmark.hash64,
            std::cmp::Reverse(landmark.weight),
        )
    });
}

fn video_kind_bounding_priority(kind: u8) -> u8 {
    match kind {
        V3_VIDEO_KIND_TEMPORAL_SHINGLE => 0,
        V3_VIDEO_KIND_GLOBAL_DCT => 1,
        V3_VIDEO_KIND_CENTER_DCT => 2,
        V3_VIDEO_KIND_EDGE => 3,
        V3_VIDEO_KIND_LEGACY_LUMA => 4,
        _ => u8::MAX,
    }
}

fn video_landmark_key(landmark: &VideoLandmarkV3) -> (u8, u32, u64, u32) {
    (
        landmark.kind,
        landmark.bucket,
        landmark.hash64,
        landmark.t_ms,
    )
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

pub fn v3_video_bucket_for_kind(kind: u8, raw_bucket: u32) -> u32 {
    ((u32::from(kind) & 0x0f) << V3_VIDEO_BUCKET_KIND_SHIFT)
        | (raw_bucket & V3_VIDEO_BUCKET_VALUE_MASK)
}

pub fn v3_video_kind_is_supported(kind: u8) -> bool {
    matches!(
        kind,
        V3_VIDEO_KIND_LEGACY_LUMA
            | V3_VIDEO_KIND_GLOBAL_DCT
            | V3_VIDEO_KIND_CENTER_DCT
            | V3_VIDEO_KIND_EDGE
            | V3_VIDEO_KIND_TEMPORAL_SHINGLE
    )
}

pub fn v3_video_kind_from_bucket(bucket: u32) -> Option<u8> {
    let kind = (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8;
    v3_video_kind_is_supported(kind).then_some(kind)
}

pub fn v3_video_bucket_kind_matches(kind: u8, bucket: u32) -> bool {
    v3_video_kind_from_bucket(bucket).is_some_and(|bucket_kind| bucket_kind == kind)
}

pub fn validate_video_landmark_v3(landmark: &VideoLandmarkV3) -> Result<(), String> {
    if !v3_video_kind_is_supported(landmark.kind) {
        return Err(format!(
            "unsupported V3 video landmark kind {}",
            landmark.kind
        ));
    }
    let Some(bucket_kind) = v3_video_kind_from_bucket(landmark.bucket) else {
        return Err(format!(
            "unsupported V3 video landmark bucket kind {}",
            (landmark.bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8
        ));
    };
    if bucket_kind != landmark.kind {
        return Err(format!(
            "V3 video landmark kind {} does not match bucket kind {}",
            landmark.kind, bucket_kind
        ));
    }
    if landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
        let expected = v3_video_bucket_for_kind(landmark.kind, anchor_bucket(landmark.hash64));
        if landmark.bucket != expected {
            return Err(format!(
                "V3 temporal shingle bucket {} does not match exact hash bucket {}",
                landmark.bucket, expected
            ));
        }
    }
    Ok(())
}

pub fn validate_video_landmarks_v3(landmarks: &[VideoLandmarkV3]) -> Result<(), String> {
    for landmark in landmarks {
        validate_video_landmark_v3(landmark)?;
    }
    Ok(())
}

fn v3_video_lsh_buckets(kind: u8, hash: u64) -> Vec<u32> {
    if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
        return vec![v3_video_bucket_for_kind(kind, anchor_bucket(hash))];
    }
    video_lsh_buckets(hash)
        .into_iter()
        .map(|bucket| v3_video_bucket_for_kind(kind, bucket))
        .collect()
}

pub fn v3_video_hamming_threshold(kind: u8) -> u32 {
    let tuning = current_v3_tuning();
    match kind {
        V3_VIDEO_KIND_GLOBAL_DCT => tuning.video_hamming_global,
        V3_VIDEO_KIND_CENTER_DCT => tuning.video_hamming_center,
        V3_VIDEO_KIND_EDGE => tuning.video_hamming_edge,
        V3_VIDEO_KIND_TEMPORAL_SHINGLE => tuning.video_hamming_temporal,
        _ => DEFAULT_FRAME_HAMMING_THRESHOLD,
    }
}

fn v3_video_anchor_hashes_match(kind: u8, left: u64, right: u64) -> bool {
    frame_hash_distance(left, right) <= v3_video_hamming_threshold(kind)
}

pub fn detect_content_window_luma(width: usize, height: usize, luma: &[u8]) -> Option<LumaRect> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let full = LumaRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let row_is_content = |y: usize| {
        let start = y * width;
        !luma_slice_is_black(&luma[start..start + width])
    };
    let top = (0..height).find(|y| row_is_content(*y));
    let bottom = (0..height).rev().find(|y| row_is_content(*y));
    let (Some(top), Some(bottom)) = (top, bottom) else {
        return Some(full);
    };
    let column_is_content = |x: usize| {
        let mut values = Vec::with_capacity(bottom - top + 1);
        for y in top..=bottom {
            values.push(luma[y * width + x]);
        }
        !luma_slice_is_black(&values)
    };
    let left = (0..width).find(|x| column_is_content(*x));
    let right = (0..width).rev().find(|x| column_is_content(*x));
    let (Some(left), Some(right)) = (left, right) else {
        return Some(full);
    };
    let rect = LumaRect {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    };
    let uncertain = rect.width < width / 3
        || rect.height < height / 3
        || rect.width < 4
        || rect.height < 4
        || (rect.x <= 1
            && rect.y <= 1
            && rect.x + rect.width + 1 >= width
            && rect.y + rect.height + 1 >= height);
    if uncertain { Some(full) } else { Some(rect) }
}

fn luma_slice_is_black(values: &[u8]) -> bool {
    if values.is_empty() {
        return true;
    }
    let mean = values.iter().map(|value| f64::from(*value)).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = f64::from(*value) - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    mean < 18.0 && variance < 18.0
}

fn luma_rect_variance(width: usize, luma: &[u8], rect: LumaRect) -> f64 {
    let mut count = 0usize;
    let mut sum = 0f64;
    let mut sum_sq = 0f64;
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let value = f64::from(luma[y * width + x]);
            count += 1;
            sum += value;
            sum_sq += value * value;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let mean = sum / count as f64;
    (sum_sq / count as f64) - (mean * mean)
}

pub fn video_landmarks_v3_from_luma_frame(
    width: usize,
    height: usize,
    luma: &[u8],
    t_ms: u32,
) -> Vec<VideoLandmarkV3> {
    let Some(content) = detect_content_window_luma(width, height, luma) else {
        return Vec::new();
    };
    if luma_rect_variance(width, luma, content) < V3_VIDEO_MIN_VARIANCE {
        return Vec::new();
    }
    let mut landmarks = Vec::new();
    if let Some(hash) = dct_phash_luma(width, height, luma, content) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_GLOBAL_DCT, t_ms, hash, 2);
    }
    if let Some(hash) = dct_phash_luma(width, height, luma, center_crop_rect(content, 0.68)) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_CENTER_DCT, t_ms, hash, 2);
    }
    if let Some(hash) = edge_hash_luma(width, height, luma, content) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_EDGE, t_ms, hash, 2);
    }
    landmarks
}

pub fn video_landmarks_v3_from_luma_frames(
    width: usize,
    height: usize,
    frames: &[(u32, Vec<u8>)],
) -> Vec<VideoLandmarkV3> {
    let mut landmarks = Vec::new();
    for (t_ms, luma) in frames {
        landmarks.extend(video_landmarks_v3_from_luma_frame(
            width, height, luma, *t_ms,
        ));
    }
    add_v3_temporal_video_shingles(&mut landmarks);
    dedupe_video_landmarks_v3(&mut landmarks);
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_VERIFY_LANDMARK_LIMIT)
}

fn push_v3_video_landmarks_for_hash(
    landmarks: &mut Vec<VideoLandmarkV3>,
    kind: u8,
    t_ms: u32,
    hash64: u64,
    weight: u8,
) {
    for bucket in v3_video_lsh_buckets(kind, hash64) {
        landmarks.push(VideoLandmarkV3 {
            bucket,
            hash64,
            t_ms,
            kind,
            weight,
        });
    }
}

fn center_crop_rect(rect: LumaRect, scale: f64) -> LumaRect {
    let width = ((rect.width as f64 * scale).round() as usize)
        .clamp(4, rect.width.max(4))
        .min(rect.width);
    let height = ((rect.height as f64 * scale).round() as usize)
        .clamp(4, rect.height.max(4))
        .min(rect.height);
    LumaRect {
        x: rect.x + (rect.width - width) / 2,
        y: rect.y + (rect.height - height) / 2,
        width,
        height,
    }
}

fn sample_luma_rect_32(
    width: usize,
    luma: &[u8],
    rect: LumaRect,
) -> [f64; V3_VIDEO_PHASH_SIZE * V3_VIDEO_PHASH_SIZE] {
    let mut samples = [0f64; V3_VIDEO_PHASH_SIZE * V3_VIDEO_PHASH_SIZE];
    for y in 0..V3_VIDEO_PHASH_SIZE {
        let source_y = rect.y + ((y * rect.height) / V3_VIDEO_PHASH_SIZE).min(rect.height - 1);
        for x in 0..V3_VIDEO_PHASH_SIZE {
            let source_x = rect.x + ((x * rect.width) / V3_VIDEO_PHASH_SIZE).min(rect.width - 1);
            samples[y * V3_VIDEO_PHASH_SIZE + x] = f64::from(luma[source_y * width + source_x]);
        }
    }
    samples
}

fn dct_phash_luma(width: usize, height: usize, luma: &[u8], rect: LumaRect) -> Option<u64> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let samples = sample_luma_rect_32(width, luma, rect);
    let mut coeffs = [0f64; V3_VIDEO_PHASH_LOW_FREQ * V3_VIDEO_PHASH_LOW_FREQ];
    for v in 0..V3_VIDEO_PHASH_LOW_FREQ {
        for u in 0..V3_VIDEO_PHASH_LOW_FREQ {
            let mut sum = 0f64;
            for y in 0..V3_VIDEO_PHASH_SIZE {
                let cos_y = (((2 * y + 1) as f64 * v as f64 * std::f64::consts::PI)
                    / (2.0 * V3_VIDEO_PHASH_SIZE as f64))
                    .cos();
                for x in 0..V3_VIDEO_PHASH_SIZE {
                    let cos_x = (((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI)
                        / (2.0 * V3_VIDEO_PHASH_SIZE as f64))
                        .cos();
                    sum += samples[y * V3_VIDEO_PHASH_SIZE + x] * cos_x * cos_y;
                }
            }
            coeffs[v * V3_VIDEO_PHASH_LOW_FREQ + u] = sum;
        }
    }
    let mut comparable = coeffs[1..].to_vec();
    comparable.sort_by(|left, right| left.total_cmp(right));
    let median = comparable[comparable.len() / 2];
    let mut hash = 0u64;
    for (index, coeff) in coeffs.iter().enumerate().skip(1) {
        if *coeff >= median {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

fn edge_hash_luma(width: usize, height: usize, luma: &[u8], rect: LumaRect) -> Option<u64> {
    if width < 3 || height < 3 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let mut cells = [0f64; 64];
    for cell_y in 0..8 {
        for cell_x in 0..8 {
            let start_x = rect.x + cell_x * rect.width / 8;
            let end_x = (rect.x + ((cell_x + 1) * rect.width / 8))
                .max(start_x + 1)
                .min(rect.x + rect.width);
            let start_y = rect.y + cell_y * rect.height / 8;
            let end_y = (rect.y + ((cell_y + 1) * rect.height / 8))
                .max(start_y + 1)
                .min(rect.y + rect.height);
            let mut sum = 0f64;
            let mut count = 0f64;
            for y in start_y.max(1)..end_y.min(height - 1) {
                for x in start_x.max(1)..end_x.min(width - 1) {
                    let dx =
                        i16::from(luma[y * width + x + 1]) - i16::from(luma[y * width + x - 1]);
                    let dy =
                        i16::from(luma[(y + 1) * width + x]) - i16::from(luma[(y - 1) * width + x]);
                    sum += f64::from(dx.unsigned_abs() + dy.unsigned_abs());
                    count += 1.0;
                }
            }
            cells[cell_y * 8 + cell_x] = if count > 0.0 { sum / count } else { 0.0 };
        }
    }
    let mut sorted = cells;
    sorted.sort_by(|left, right| left.total_cmp(right));
    let median = sorted[sorted.len() / 2];
    if median <= 0.0 && cells.iter().all(|value| *value <= 0.0) {
        return None;
    }
    let mut hash = 0u64;
    for (index, value) in cells.iter().enumerate() {
        if *value >= median {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

fn add_v3_temporal_video_shingles(landmarks: &mut Vec<VideoLandmarkV3>) {
    let mut descriptors = landmarks
        .iter()
        .copied()
        .filter(|landmark| {
            matches!(
                landmark.kind,
                V3_VIDEO_KIND_GLOBAL_DCT | V3_VIDEO_KIND_CENTER_DCT | V3_VIDEO_KIND_EDGE
            )
        })
        .collect::<Vec<_>>();
    descriptors.sort_by_key(|landmark| (landmark.t_ms, landmark.kind, landmark.hash64));
    descriptors.dedup_by_key(|landmark| (landmark.t_ms, landmark.kind, landmark.hash64));
    let mut shingles = Vec::new();
    for (index, left) in descriptors.iter().enumerate() {
        let mut emitted = 0usize;
        for right in descriptors.iter().skip(index + 1) {
            let delta = right.t_ms.saturating_sub(left.t_ms);
            if delta > V3_VIDEO_TEMPORAL_MAX_DELTA_MS {
                break;
            }
            if delta < V3_VIDEO_TEMPORAL_MIN_DELTA_MS || left.kind != right.kind {
                continue;
            }
            let delta_bucket = delta / V3_VIDEO_TEMPORAL_DELTA_BUCKET_MS;
            let mut bytes = Vec::with_capacity(21);
            bytes.push(left.kind);
            bytes.extend_from_slice(&left.hash64.to_le_bytes());
            bytes.extend_from_slice(&right.hash64.to_le_bytes());
            bytes.extend_from_slice(&delta_bucket.to_le_bytes());
            let hash64 = stable_hash_u64(bytes);
            shingles.push(VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(
                    V3_VIDEO_KIND_TEMPORAL_SHINGLE,
                    anchor_bucket(hash64),
                ),
                hash64,
                t_ms: left.t_ms,
                kind: V3_VIDEO_KIND_TEMPORAL_SHINGLE,
                weight: 4,
            });
            emitted += 1;
            if emitted >= V3_VIDEO_TEMPORAL_FANOUT {
                break;
            }
        }
    }
    landmarks.extend(shingles);
}

fn dedupe_video_landmarks_v3(landmarks: &mut Vec<VideoLandmarkV3>) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.kind,
            landmark.bucket,
            landmark.hash64,
            std::cmp::Reverse(landmark.weight),
        )
    });
    landmarks.dedup_by(|left, right| {
        left.t_ms == right.t_ms
            && left.kind == right.kind
            && left.bucket == right.bucket
            && left.hash64 == right.hash64
    });
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
    mut evidence: MediaMatchEvidence,
    explanation: impl Into<String>,
) -> MediaMatchDecision {
    let timeline_map = evidence
        .timeline_map_v3
        .take()
        .unwrap_or_else(|| media_timeline_map_v3_from_evidence(tier, &evidence));
    evidence.v3_class = Some(timeline_map.global_class);
    evidence.timeline_map_v3 = Some(timeline_map);
    MediaMatchDecision {
        tier,
        evidence,
        explanation: explanation.into(),
    }
}

fn media_timeline_map_v3_from_evidence(
    tier: MediaMatchTier,
    evidence: &MediaMatchEvidence,
) -> MediaTimelineMapV3 {
    // Fallback for exact/non-anchor decisions; anchor decisions provide a true piecewise map.
    let global_class = media_match_class_v3_from_evidence(tier, evidence);
    let segments: Vec<AlignedSegmentV3> = evidence
        .alignment
        .as_ref()
        .map(|alignment| {
            let audio_score = evidence
                .audio
                .as_ref()
                .map(|audio| audio.similarity as f32)
                .unwrap_or(0.0);
            let video_score = evidence
                .video
                .as_ref()
                .map(|video| video.query_coverage.min(video.candidate_coverage) as f32)
                .unwrap_or(0.0);
            let confidence = match tier {
                MediaMatchTier::Exact | MediaMatchTier::Strong => 1.0,
                MediaMatchTier::Probable => 0.72,
                MediaMatchTier::Weak => 0.35,
                MediaMatchTier::Reject | MediaMatchTier::Unknown => 0.0,
            };
            AlignedSegmentV3 {
                query_start_ms: seconds_to_u32_ms(alignment.first_query_second),
                query_end_ms: seconds_to_u32_ms(alignment.last_query_second),
                candidate_start_ms: seconds_to_u32_ms(alignment.first_candidate_second),
                candidate_end_ms: seconds_to_u32_ms(alignment.last_candidate_second),
                scale_ppm: alignment.scale_ppm,
                audio_pairs: alignment.aligned_audio_anchors,
                video_pairs: alignment.aligned_video_anchors,
                weighted_score: alignment.aligned_pairs as u32,
                residual_ms: alignment.drift_ratio * alignment.aligned_span_seconds * 1000.0,
                audio_score,
                video_score,
                confidence,
            }
        })
        .into_iter()
        .collect();
    let total_aligned_span_ms = segments
        .iter()
        .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
        .sum();
    let best_segment_score = segments
        .iter()
        .map(|segment| segment.weighted_score)
        .max()
        .unwrap_or(0);
    let segment_count = segments.len();
    MediaTimelineMapV3 {
        global_class,
        current_position_class: global_class,
        segments,
        total_aligned_span_ms,
        largest_gap_ms: 0,
        edge_only: false,
        audio_video_conflict: false,
        best_segment_score,
        second_best_segment_score: 0,
        piecewise_pair_count: 0,
        piecewise_hypothesis_count: 0,
        piecewise_segment_candidate_count: 0,
        piecewise_segment_chain_count: segment_count,
        piecewise_fit_millis: 0,
    }
}

fn media_match_class_v3_from_evidence(
    tier: MediaMatchTier,
    evidence: &MediaMatchEvidence,
) -> MatchClassV3 {
    match tier {
        MediaMatchTier::Exact | MediaMatchTier::Strong => {
            if evidence
                .metadata
                .duration_within_tolerance
                .is_some_and(|value| !value)
            {
                MatchClassV3::SameMediaDifferentCut
            } else if evidence.video.is_some() && evidence.audio.is_none() {
                MatchClassV3::SameVideoDifferentAudio
            } else {
                MatchClassV3::SameCutStrong
            }
        }
        MediaMatchTier::Probable => {
            if evidence
                .metadata
                .duration_within_tolerance
                .is_some_and(|value| !value)
            {
                MatchClassV3::SameMediaDifferentCut
            } else {
                MatchClassV3::SameCutProbable
            }
        }
        MediaMatchTier::Weak => MatchClassV3::PartialOverlap,
        MediaMatchTier::Reject => {
            if evidence
                .alignment
                .as_ref()
                .is_some_and(|alignment| alignment.aligned_span_seconds < 120.0)
            {
                MatchClassV3::SharedIntroOutroOnly
            } else {
                MatchClassV3::Reject
            }
        }
        MediaMatchTier::Unknown => MatchClassV3::Unknown,
    }
}

fn seconds_to_u32_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round().min(f64::from(u32::MAX)) as u32
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

pub(crate) fn media_match_tier_rank(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Exact => 5,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Unknown => 1,
        MediaMatchTier::Reject => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_root(label: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-media-match-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).expect("test root should be created");
        root
    }

    fn write_fake_tool(root: &Path, name: &str, stdout_line: Option<&str>) -> PathBuf {
        #[cfg(windows)]
        let path = root.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let path = root.join(name);

        #[cfg(windows)]
        let script = match stdout_line {
            Some(line) => format!("@echo off\r\necho {line}\r\nexit /b 0\r\n"),
            None => "@echo off\r\nexit /b 0\r\n".to_owned(),
        };
        #[cfg(not(windows))]
        let script = match stdout_line {
            Some(line) => format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n"),
            None => "#!/bin/sh\nexit 0\n".to_owned(),
        };
        std::fs::write(&path, script).expect("fake tool should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path)
                .expect("fake tool metadata should load")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("fake tool should be executable");
        }
        path
    }

    fn record(
        path: &str,
        size: u64,
        duration: Option<f64>,
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
            extraction_settings: MediaExtractionSettings::combined_v3(),
            duration_seconds: duration,
            container_fingerprint: container_fingerprint_from_metadata(
                &normalized_path,
                1000,
                size,
                duration,
            ),
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
        video: Option<VideoFingerprint>,
        extraction_settings: MediaExtractionSettings,
    ) -> MediaFingerprintRecord {
        let mut record = record(path, size, duration, video);
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
        );
        record.extraction_settings = MediaExtractionSettings::audio_constellation_v3();
        record.audio_anchors = profile.audio_anchors;
        record.video_anchors = profile.video_anchors;
        record
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
            v3_landmarks: Vec::new(),
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

    fn synthetic_luma_pattern(width: usize, height: usize) -> Vec<u8> {
        synthetic_luma_pattern_seed(width, height, 0)
    }

    fn synthetic_luma_pattern_seed(width: usize, height: usize, seed: usize) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let base = ((x * (9 + seed % 5)
                        + y * (13 + seed % 7)
                        + ((x / 4 + y / 4 + seed) % 2) * 70
                        + seed * 17)
                        % 220) as u8;
                    base.saturating_add(20)
                })
            })
            .collect()
    }

    fn brightness_shift_luma(luma: &[u8], delta: i16) -> Vec<u8> {
        luma.iter()
            .map(|value| (i16::from(*value) + delta).clamp(0, i16::from(u8::MAX)) as u8)
            .collect()
    }

    fn v3_landmark_hash_for_kind(landmarks: &[VideoLandmarkV3], kind: u8) -> u64 {
        landmarks
            .iter()
            .find(|landmark| landmark.kind == kind)
            .map(|landmark| landmark.hash64)
            .expect("landmark kind should exist")
    }

    fn anchor_profile(
        duration_ms: u32,
        audio: &[(u32, u32)],
        video: &[(u32, u32, u64)],
    ) -> MediaAnchorProfile {
        MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
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
                    kind: V3_VIDEO_KIND_LEGACY_LUMA,
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

    fn audio_only_v3_anchor_profile(
        duration_ms: u32,
        offset_ms: i32,
        drift_ppm: i32,
    ) -> MediaAnchorProfile {
        let query_times = (0..24).map(|index| 60_000 + index * 45_000);
        let audio = query_times
            .map(|t_ms| AudioAnchor {
                bucket: t_ms / 45_000 + 1,
                t_ms: shifted_anchor_time(t_ms, offset_ms, drift_ppm),
                weight: 4,
            })
            .collect::<Vec<_>>();
        MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "audio-constellation-v3".to_owned(),
            duration_ms: Some(duration_ms),
            audio_anchors: audio,
            video_anchors: Vec::new(),
        }
    }

    fn v3_profile_from_times(
        duration_ms: u32,
        audio_times: &[(u32, u32)],
        video_times: &[(u32, u32, u64)],
    ) -> MediaAnchorProfile {
        MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(duration_ms),
            audio_anchors: audio_times
                .iter()
                .map(|(bucket, t_ms)| AudioAnchor {
                    bucket: *bucket,
                    t_ms: *t_ms,
                    weight: 4,
                })
                .collect(),
            video_anchors: video_times
                .iter()
                .map(|(bucket, t_ms, hash64)| VideoAnchor {
                    bucket: *bucket,
                    t_ms: *t_ms,
                    hash64: *hash64,
                    kind: V3_VIDEO_KIND_LEGACY_LUMA,
                    weight: 4,
                })
                .collect(),
        }
    }

    fn v3_audio_times(start_ms: u32, count: u32, step_ms: u32) -> Vec<(u32, u32)> {
        (0..count)
            .map(|index| (1_000 + index, start_ms + (index * step_ms)))
            .collect()
    }

    fn v3_shift_audio_times(
        times: &[(u32, u32)],
        offset_ms: i32,
        drift_ppm: i32,
    ) -> Vec<(u32, u32)> {
        times
            .iter()
            .map(|(bucket, t_ms)| (*bucket, shifted_anchor_time(*t_ms, offset_ms, drift_ppm)))
            .collect()
    }

    fn v3_video_times_from_audio(times: &[(u32, u32)]) -> Vec<(u32, u32, u64)> {
        times
            .iter()
            .map(|(bucket, t_ms)| (*bucket + 10_000, *t_ms, synthetic_hash(u64::from(*bucket))))
            .collect()
    }

    fn v3_shift_video_times(
        times: &[(u32, u32, u64)],
        offset_ms: i32,
        drift_ppm: i32,
    ) -> Vec<(u32, u32, u64)> {
        times
            .iter()
            .map(|(bucket, t_ms, hash)| {
                (
                    *bucket,
                    shifted_anchor_time(*t_ms, offset_ms, drift_ppm),
                    *hash,
                )
            })
            .collect()
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
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 9),
                t_ms: 1_000,
                hash64: 0x0123_4567_89ab_cdef,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 1,
            },
            VideoAnchor {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 10),
                t_ms: 3_000,
                hash64: 0xfedc_ba98_7654_3210,
                kind: V3_VIDEO_KIND_EDGE,
                weight: 3,
            },
        ];

        let encoded = encode_video_anchor_summary(&anchors);
        let decoded = decode_video_anchor_summary(&encoded).expect("video summary should decode");

        assert_eq!(decoded, anchors);
        assert!(encoded.len() < 84);
    }

    #[test]
    fn v3_blob_round_trips_delta_encoded_landmarks() {
        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(1_413_000),
            audio_landmarks: vec![
                AudioLandmarkV3 {
                    hash: 0x1234_5678,
                    t_ms: 10_000,
                    weight: 9,
                },
                AudioLandmarkV3 {
                    hash: 0x90ab_cdef,
                    t_ms: 42_000,
                    weight: 3,
                },
            ],
            video_landmarks: vec![VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_CENTER_DCT, 7),
                hash64: 0x0123_4567_89ab_cdef,
                t_ms: 48_000,
                kind: V3_VIDEO_KIND_CENTER_DCT,
                weight: 5,
            }],
        };

        let encoded = encode_media_fingerprint_blob_v3(&blob);
        let decoded = decode_media_fingerprint_blob_v3(&encoded).expect("v3 blob should decode");

        assert_eq!(decoded, blob);
        assert!(encoded.len() < 80);
    }

    #[test]
    fn v3_blob_rejects_corrupted_input() {
        assert!(matches!(
            decode_media_fingerprint_blob_v3(b"not-smm3"),
            Err(MediaFingerprintBlobV3DecodeError::InvalidMagic)
        ));

        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(1),
            audio_landmarks: vec![AudioLandmarkV3 {
                hash: 1,
                t_ms: 1,
                weight: 1,
            }],
            video_landmarks: Vec::new(),
        };
        let mut encoded = encode_media_fingerprint_blob_v3(&blob);
        encoded.truncate(encoded.len() - 1);

        assert!(matches!(
            decode_media_fingerprint_blob_v3(&encoded),
            Err(MediaFingerprintBlobV3DecodeError::InvalidLength)
        ));
    }

    #[test]
    fn v3_blob_rejects_unknown_video_kind() {
        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(1),
            audio_landmarks: Vec::new(),
            video_landmarks: vec![VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(9, 1),
                hash64: 1,
                t_ms: 1,
                kind: 9,
                weight: 1,
            }],
        };

        let encoded = encode_media_fingerprint_blob_v3(&blob);

        assert!(matches!(
            decode_media_fingerprint_blob_v3(&encoded),
            Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(9))
        ));
    }

    #[test]
    fn v3_blob_rejects_mismatched_video_bucket_kind() {
        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(1),
            audio_landmarks: Vec::new(),
            video_landmarks: vec![VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 1),
                hash64: 1,
                t_ms: 1,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 1,
            }],
        };

        let encoded = encode_media_fingerprint_blob_v3(&blob);

        assert!(matches!(
            decode_media_fingerprint_blob_v3(&encoded),
            Err(
                MediaFingerprintBlobV3DecodeError::MismatchedVideoBucketKind {
                    kind: V3_VIDEO_KIND_GLOBAL_DCT,
                    bucket_kind: V3_VIDEO_KIND_EDGE
                }
            )
        ));
    }

    #[test]
    fn v3_blob_rejects_invalid_temporal_video_bucket() {
        let hash64 = 0x1234_5678_9abc_def0;
        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(1),
            audio_landmarks: Vec::new(),
            video_landmarks: vec![VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_TEMPORAL_SHINGLE, 0xfeed_beef),
                hash64,
                t_ms: 1,
                kind: V3_VIDEO_KIND_TEMPORAL_SHINGLE,
                weight: 1,
            }],
        };

        let encoded = encode_media_fingerprint_blob_v3(&blob);

        assert!(matches!(
            decode_media_fingerprint_blob_v3(&encoded),
            Err(MediaFingerprintBlobV3DecodeError::InvalidTemporalVideoBucket { .. })
        ));
    }

    #[test]
    fn v3_wire_profile_rejects_unknown_video_kind() {
        let summary = encode_video_anchor_summary(&[VideoAnchor {
            bucket: v3_video_bucket_for_kind(9, 1),
            t_ms: 1,
            hash64: 1,
            kind: 9,
            weight: 1,
        }]);
        let profile = MediaMatchWireProfile {
            profile: "combined-v3".to_owned(),
            algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
            duration_ms: Some(1),
            audio: None,
            video: Some(MediaMatchWireAnchorBlock {
                algorithm: MediaExtractionSettings::combined_v3().video_algorithm,
                time_base_ms: 1,
                anchors: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    summary,
                ),
            }),
        };

        let error = media_anchor_profile_from_wire_profile(&profile).expect_err("invalid kind");

        assert!(error.contains("unsupported media v3 video landmark kind 9"));
    }

    #[test]
    fn video_landmark_with_bucket_kind_mismatch_is_not_matched() {
        let hash = 0x0123_4567_89ab_cdef;
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(60_000),
            audio_anchors: Vec::new(),
            video_anchors: vec![VideoAnchor {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 3),
                t_ms: 1_000,
                hash64: hash,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            }],
        };
        let candidate = MediaAnchorProfile {
            video_anchors: vec![VideoAnchor {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
                t_ms: 1_000,
                hash64: hash,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            }],
            ..query.clone()
        };

        assert!(collect_anchor_match_pairs(&query, &candidate).is_empty());
    }

    #[test]
    fn black_bar_detection_letterbox() {
        let width = 32;
        let height = 32;
        let mut luma = synthetic_luma_pattern(width, height);
        for y in 0..6 {
            for x in 0..width {
                luma[y * width + x] = 0;
                luma[(height - 1 - y) * width + x] = 0;
            }
        }

        let rect = detect_content_window_luma(width, height, &luma).expect("content rect");

        assert_eq!(rect.y, 6);
        assert_eq!(rect.height, 20);
    }

    #[test]
    fn black_bar_detection_pillarbox() {
        let width = 32;
        let height = 32;
        let mut luma = synthetic_luma_pattern(width, height);
        for y in 0..height {
            for x in 0..5 {
                luma[y * width + x] = 0;
                luma[y * width + (width - 1 - x)] = 0;
            }
        }

        let rect = detect_content_window_luma(width, height, &luma).expect("content rect");

        assert_eq!(rect.x, 5);
        assert_eq!(rect.width, 22);
    }

    #[test]
    fn all_black_frame_is_ignored_for_v3_video() {
        let luma = vec![0; VIDEO_FRAME_BYTES];
        let rect =
            detect_content_window_luma(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, &luma).unwrap();

        assert_eq!(
            rect,
            LumaRect {
                x: 0,
                y: 0,
                width: VIDEO_FRAME_WIDTH,
                height: VIDEO_FRAME_HEIGHT
            }
        );
        assert!(video_landmarks_v3_from_luma_frame(32, 32, &luma, 0).is_empty());
    }

    #[test]
    fn global_dct_hash_stable_under_brightness_shift() {
        let luma = synthetic_luma_pattern(32, 32);
        let brighter = brightness_shift_luma(&luma, 22);
        let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
        let right = video_landmarks_v3_from_luma_frame(32, 32, &brighter, 1_000);

        let distance = frame_hash_distance(
            v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_GLOBAL_DCT),
            v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_GLOBAL_DCT),
        );

        assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_GLOBAL_DCT));
    }

    #[test]
    fn center_crop_resists_hard_subtitle_band() {
        let luma = synthetic_luma_pattern(32, 32);
        let mut subtitled = luma.clone();
        for y in 25..30 {
            for x in 6..26 {
                subtitled[y * 32 + x] = 255;
            }
        }
        let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
        let right = video_landmarks_v3_from_luma_frame(32, 32, &subtitled, 1_000);

        let distance = frame_hash_distance(
            v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_CENTER_DCT),
            v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_CENTER_DCT),
        );

        assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_CENTER_DCT));
    }

    #[test]
    fn edge_hash_resists_brightness_shift() {
        let luma = synthetic_luma_pattern(32, 32);
        let brighter = brightness_shift_luma(&luma, 30);
        let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
        let right = video_landmarks_v3_from_luma_frame(32, 32, &brighter, 1_000);

        let distance = frame_hash_distance(
            v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_EDGE),
            v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_EDGE),
        );

        assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_EDGE));
    }

    #[test]
    fn temporal_shingle_requires_order() {
        let mut forward = vec![
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 1),
                hash64: 0x1000,
                t_ms: 0,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 2),
                hash64: 0x2000,
                t_ms: 10_000,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
                hash64: 0x3000,
                t_ms: 20_000,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
        ];
        let mut backward = vec![
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
                hash64: 0x3000,
                t_ms: 0,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 2),
                hash64: 0x2000,
                t_ms: 10_000,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
            VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 1),
                hash64: 0x1000,
                t_ms: 20_000,
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                weight: 2,
            },
        ];
        add_v3_temporal_video_shingles(&mut forward);
        add_v3_temporal_video_shingles(&mut backward);
        let forward_shingles = forward
            .iter()
            .filter(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
            .map(|landmark| landmark.hash64)
            .collect::<HashSet<_>>();
        let backward_shingles = backward
            .iter()
            .filter(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
            .map(|landmark| landmark.hash64)
            .collect::<HashSet<_>>();

        assert!(!forward_shingles.is_empty());
        assert!(forward_shingles.is_disjoint(&backward_shingles));
    }

    #[test]
    fn temporal_shingles_match_exactly() {
        let hash = 0x0123_4567_89ab_cdef;
        assert!(v3_video_anchor_hashes_match(
            V3_VIDEO_KIND_TEMPORAL_SHINGLE,
            hash,
            hash
        ));
        assert!(!v3_video_anchor_hashes_match(
            V3_VIDEO_KIND_TEMPORAL_SHINGLE,
            hash,
            hash ^ 1
        ));
    }

    #[test]
    fn video_descriptor_kinds_do_not_cross_match() {
        let hash = 0x0123_4567_89ab_cdef;
        let query = VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 7),
            t_ms: 1_000,
            hash64: hash,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        };
        let candidate = VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 7),
            t_ms: 1_000,
            hash64: hash,
            kind: V3_VIDEO_KIND_EDGE,
            weight: 2,
        };
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: vec![query],
        };
        let candidate = MediaAnchorProfile {
            video_anchors: vec![candidate],
            ..query.clone()
        };

        assert!(collect_anchor_match_pairs(&query, &candidate).is_empty());
    }

    #[test]
    fn combined_v3_video_landmarks_include_multiple_kinds() {
        let frames = vec![
            (0, synthetic_luma_pattern_seed(32, 32, 1)),
            (10_000, synthetic_luma_pattern_seed(32, 32, 2)),
            (20_000, synthetic_luma_pattern_seed(32, 32, 3)),
        ];
        let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
        let video = VideoFingerprint {
            duration_seconds: Some(30),
            frames: Vec::new(),
            v3_landmarks: landmarks,
        };
        let record = record_with_extraction_settings(
            "video.mkv",
            100,
            Some(30.0),
            Some(video),
            MediaExtractionSettings::combined_v3(),
        );
        let kinds = video_landmarks_v3_from_record(&record)
            .into_iter()
            .map(|landmark| landmark.kind)
            .collect::<HashSet<_>>();

        assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
        assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
    }

    #[test]
    fn combined_v3_video_bounding_preserves_descriptor_kinds() {
        let frames = (0..80)
            .map(|index| {
                (
                    index * 10_000,
                    synthetic_luma_pattern_seed(32, 32, index as usize + 100),
                )
            })
            .collect::<Vec<_>>();
        let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
        let kinds = landmarks
            .iter()
            .map(|landmark| landmark.kind)
            .collect::<HashSet<_>>();

        assert!(landmarks.len() <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
        assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
        assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
    }

    #[test]
    fn combined_v3_video_index_bounding_prefers_temporal_shingles() {
        let frames = (0..80)
            .map(|index| {
                (
                    index * 10_000,
                    synthetic_luma_pattern_seed(32, 32, index as usize + 200),
                )
            })
            .collect::<Vec<_>>();
        let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
        let video = VideoFingerprint {
            duration_seconds: Some(800),
            frames: Vec::new(),
            v3_landmarks: landmarks,
        };
        let record = record_with_extraction_settings(
            "index-bounds.mkv",
            100,
            Some(800.0),
            Some(video),
            MediaExtractionSettings::combined_v3(),
        );
        let index = video_index_landmarks_v3_from_record(&record);

        assert!(index.len() <= V3_VIDEO_INDEX_LANDMARK_LIMIT);
        assert!(
            index
                .iter()
                .any(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
        );
    }

    #[test]
    fn cropped_or_letterboxed_same_video_still_matches() {
        let content = synthetic_luma_pattern_seed(32, 20, 7);
        let mut letterboxed = vec![0u8; 32 * 32];
        for y in 0..20 {
            for x in 0..32 {
                letterboxed[(y + 6) * 32 + x] = content[y * 32 + x];
            }
        }
        let plain = video_landmarks_v3_from_luma_frame(32, 20, &content, 10_000);
        let boxed = video_landmarks_v3_from_luma_frame(32, 32, &letterboxed, 10_000);

        for kind in [
            V3_VIDEO_KIND_GLOBAL_DCT,
            V3_VIDEO_KIND_CENTER_DCT,
            V3_VIDEO_KIND_EDGE,
        ] {
            let distance = frame_hash_distance(
                v3_landmark_hash_for_kind(&plain, kind),
                v3_landmark_hash_for_kind(&boxed, kind),
            );
            assert!(
                distance <= v3_video_hamming_threshold(kind),
                "kind {kind} distance {distance} should stay matchable"
            );
        }
    }

    #[test]
    fn combined_v3_video_storage_limits_are_bounded() {
        let frames = (0..80)
            .map(|index| {
                (
                    index * 10_000,
                    synthetic_luma_pattern_seed(32, 32, index as usize),
                )
            })
            .collect::<Vec<_>>();
        let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
        let video = VideoFingerprint {
            duration_seconds: Some(800),
            frames: Vec::new(),
            v3_landmarks: landmarks,
        };
        let record = record_with_extraction_settings(
            "storage.mkv",
            100,
            Some(800.0),
            Some(video),
            MediaExtractionSettings::combined_v3(),
        );

        assert!(video_landmarks_v3_from_record(&record).len() <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
        assert!(
            video_index_landmarks_v3_from_record(&record).len() <= V3_VIDEO_INDEX_LANDMARK_LIMIT
        );
    }

    #[test]
    fn v3_record_diagnostics_report_blob_and_index_counts() {
        let audio = v3_audio_times(120_000, 12, 45_000)
            .into_iter()
            .map(|(bucket, t_ms)| AudioAnchor {
                bucket,
                t_ms,
                weight: 2,
            })
            .collect::<Vec<_>>();
        let frames = (0..12)
            .map(|index| {
                (
                    index * 10_000,
                    synthetic_luma_pattern_seed(32, 32, index as usize + 300),
                )
            })
            .collect::<Vec<_>>();
        let video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: Vec::new(),
            v3_landmarks: video_landmarks_v3_from_luma_frames(32, 32, &frames),
        };
        let mut record = record_with_extraction_settings(
            "diagnostics.mkv",
            100,
            Some(120.0),
            Some(video),
            MediaExtractionSettings::combined_v3(),
        );
        record.audio_anchors = audio;

        let summary = summarize_record_v3_diagnostics(&record);

        assert_eq!(summary.profile, "combined-v3");
        assert!(summary.audio_verify_count > 0);
        assert!(summary.video_verify_count > 0);
        assert!(summary.audio_index_count > 0);
        assert!(summary.video_index_count > 0);
        assert!(summary.audio_blob_bytes > 0);
        assert!(summary.video_blob_bytes > 0);
    }

    #[test]
    fn v3_diagnostics_serializes_stable_stream_metric_names() {
        let record = record_with_extraction_settings(
            "stream-diagnostics.mkv",
            100,
            Some(120.0),
            None,
            MediaExtractionSettings::audio_constellation_v3(),
        );
        let fingerprint = InstrumentedMediaFingerprint {
            record,
            report: MediaFingerprintExtractionReport {
                audio_stream: MediaAudioStreamMetrics {
                    streamed_bytes: 10_000,
                    streamed_samples: 5_000,
                    peak_frames: 12,
                    raw_landmarks_before_bounding: 300,
                    final_landmarks: 96,
                    max_buffer_samples: V3_AUDIO_WINDOW_SAMPLES + V3_AUDIO_HOP_SAMPLES,
                    max_raw_landmarks_seen: 1_100,
                    max_raw_landmarks_after_compaction: 512,
                    raw_landmark_compactions: 2,
                    ..MediaAudioStreamMetrics::default()
                },
                ..MediaFingerprintExtractionReport::default()
            },
        };

        let value =
            serde_json::to_value(summarize_instrumented_record_v3_diagnostics(&fingerprint))
                .expect("diagnostics should serialize");

        assert_eq!(value["streamedBytes"], 10_000);
        assert_eq!(value["streamedSamples"], 5_000);
        assert_eq!(value["peakFrames"], 12);
        assert_eq!(value["rawLandmarksBeforeBounding"], 300);
        assert_eq!(value["finalLandmarks"], 96);
        assert_eq!(
            value["maxBufferSamples"],
            V3_AUDIO_WINDOW_SAMPLES + V3_AUDIO_HOP_SAMPLES
        );
        assert_eq!(value["maxRawLandmarksSeen"], 1_100);
        assert_eq!(value["maxRawLandmarksAfterCompaction"], 512);
        assert_eq!(value["rawLandmarkCompactions"], 2);
    }

    #[test]
    fn v3_audio_constellation_generates_sparse_landmarks_from_pcm() {
        let sample_rate = V3_AUDIO_SAMPLE_RATE;
        let seconds = 8;
        let samples = (0..sample_rate as usize * seconds)
            .map(|index| {
                let t = index as f32 / sample_rate as f32;
                let frequency = 440.0 + ((t / 2.0).floor() * 110.0);
                (frequency.mul_add(std::f32::consts::TAU * t, 0.0).sin()
                    * f32::from(i16::MAX)
                    * 0.5)
                    .round() as i16
            })
            .collect::<Vec<_>>();

        let landmarks =
            audio_constellation_landmarks_v3_from_pcm(&samples, sample_rate, Some(seconds as f64));

        assert!(!landmarks.is_empty());
        assert!(landmarks.len() <= V3_AUDIO_VERIFY_LANDMARK_LIMIT);
        assert!(landmarks.iter().all(|landmark| landmark.weight > 0));
    }

    #[test]
    fn v3_audio_streaming_builder_is_chunk_boundary_stable() {
        let sample_rate = V3_AUDIO_SAMPLE_RATE;
        let seconds = 5;
        let samples = synthetic_audio_samples_v3(sample_rate, seconds);
        let full =
            audio_constellation_landmarks_v3_from_pcm(&samples, sample_rate, Some(seconds as f64));
        let uneven_chunks = samples.chunks(777).collect::<Vec<_>>();
        let tiny_chunks = samples.chunks(113).collect::<Vec<_>>();

        let (streamed, metrics) = audio_constellation_landmarks_v3_from_pcm_chunks(
            &uneven_chunks,
            sample_rate,
            Some(seconds as f64),
        );
        let (streamed_tiny, tiny_metrics) = audio_constellation_landmarks_v3_from_pcm_chunks(
            &tiny_chunks,
            sample_rate,
            Some(seconds as f64),
        );

        assert!(!streamed.is_empty());
        assert_eq!(streamed, streamed_tiny);
        assert!(audio_streaming_reference_overlap(&full, &streamed) >= 0.90);
        assert_eq!(metrics.streamed_samples, samples.len());
        assert_eq!(tiny_metrics.streamed_samples, samples.len());
    }

    #[test]
    fn v3_audio_streaming_rejects_odd_trailing_pcm_byte() {
        let error = audio_constellation_stream_rejects_odd_trailing_byte_for_test(&[1])
            .expect_err("odd trailing byte must fail");

        assert!(matches!(
            error,
            MediaFingerprintError::InvalidToolOutput { tool: "ffmpeg", .. }
        ));
    }

    #[test]
    fn v3_audio_streaming_decode_handles_split_pcm_samples() {
        let samples = synthetic_audio_samples_v3(V3_AUDIO_SAMPLE_RATE, 6);
        let bytes = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        let (landmarks, metrics) =
            audio_constellation_streaming_decode_pcm_bytes_for_test(&bytes).expect("decode");
        let split_metrics =
            audio_constellation_streaming_decode_split_bytes_for_test(&bytes).expect("split");

        assert!(!landmarks.is_empty());
        assert_eq!(metrics.streamed_bytes, bytes.len());
        assert_eq!(split_metrics.streamed_bytes, bytes.len());
        assert_eq!(split_metrics.streamed_samples, samples.len());
    }

    #[test]
    fn v3_audio_streaming_builder_keeps_rolling_buffer_bounded() {
        let sample_rate = V3_AUDIO_SAMPLE_RATE;
        let seconds = 45;
        let samples = synthetic_audio_samples_v3(sample_rate, seconds);

        let (_landmarks, metrics) =
            audio_constellation_landmarks_v3_from_pcm_streaming(&samples, sample_rate, Some(45.0));

        assert!(metrics.final_landmarks <= V3_AUDIO_VERIFY_LANDMARK_LIMIT);
        assert!(
            metrics.max_buffer_samples <= V3_AUDIO_WINDOW_SAMPLES,
            "{metrics:?}"
        );
        assert!(
            metrics.max_raw_landmarks_buffered <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
            "{metrics:?}"
        );
        let bounded_burst =
            V3_AUDIO_PAIR_FANOUT * V3_AUDIO_MAX_PEAKS_PER_FRAME * V3_AUDIO_PAIR_MAX_DELTA_FRAMES;
        assert!(
            metrics.max_raw_landmarks_seen <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT + bounded_burst,
            "{metrics:?}"
        );
        assert!(
            metrics.max_raw_landmarks_seen > V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
            "long synthetic audio should report a pre-compaction peak: {metrics:?}"
        );
        assert!(
            metrics.max_raw_landmarks_after_compaction <= V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
            "{metrics:?}"
        );
        assert!(
            metrics.raw_landmark_compactions > 0,
            "long synthetic audio should exercise raw landmark compaction: {metrics:?}"
        );
        assert!(metrics.streamed_samples > V3_AUDIO_WINDOW_SAMPLES * 100);
    }

    #[test]
    fn streaming_stdout_callback_error_aborts_promptly() {
        let (executable, args) = streaming_stdout_error_test_command();
        let started_at = Instant::now();
        let result = run_tool_streaming_stdout(
            "test-tool",
            &executable,
            args,
            None,
            Duration::from_secs(20),
            |_chunk| {
                Err(MediaFingerprintError::InvalidToolOutput {
                    tool: "test-tool",
                    reason: "intentional callback failure".to_owned(),
                })
            },
        );

        assert!(matches!(
            result,
            Err(MediaFingerprintError::InvalidToolOutput {
                tool: "test-tool",
                ..
            })
        ));
        assert!(
            started_at.elapsed() < Duration::from_secs(5),
            "callback failure should not wait for timeout"
        );
    }

    #[cfg(windows)]
    fn streaming_stdout_error_test_command() -> (PathBuf, Vec<OsString>) {
        (
            PathBuf::from("powershell"),
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "Write-Output chunk; Start-Sleep -Seconds 30".into(),
            ],
        )
    }

    #[cfg(not(windows))]
    fn streaming_stdout_error_test_command() -> (PathBuf, Vec<OsString>) {
        (
            PathBuf::from("sh"),
            vec!["-c".into(), "printf chunk; exec sleep 30".into()],
        )
    }

    fn synthetic_audio_samples_v3(sample_rate: u32, seconds: usize) -> Vec<i16> {
        (0..sample_rate as usize * seconds)
            .map(|index| {
                let t = index as f32 / sample_rate as f32;
                let frequency = 330.0 + ((t / 1.5).floor() * 77.0);
                ((frequency * std::f32::consts::TAU * t).sin() * f32::from(i16::MAX) * 0.45).round()
                    as i16
            })
            .collect()
    }

    #[test]
    fn same_cut_single_segment_maps_position() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameCutStrong,
            vec![test_segment_v3(10_000, 110_000, 12_000, 112_000, 1_000_000)],
        );

        let mapped = map_query_position_to_candidate_ms(&map, 60_000)
            .expect("position should map inside segment");

        assert_eq!(mapped.mapped_ms, 62_000);
        assert_eq!(mapped.segment_index, 0);
        assert_eq!(mapped.class_at_position, MatchClassV3::SameCutStrong);
        assert_eq!(mapped.local_offset_ms, 2_000);
        assert!(timeline_map_contains_query_position(&map, 60_000));
    }

    #[test]
    fn inserted_logo_two_segments_maps_each_segment() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameMediaDifferentCut,
            vec![
                test_segment_v3(0, 120_000, 0, 120_000, 1_000_000),
                test_segment_v3(120_000, 240_000, 180_000, 300_000, 1_000_000),
            ],
        );

        let first = map_query_position_to_candidate_ms(&map, 90_000).expect("first segment");
        let second = map_query_position_to_candidate_ms(&map, 180_000).expect("second segment");

        assert_eq!(first.mapped_ms, 90_000);
        assert_eq!(first.segment_index, 0);
        assert_eq!(second.mapped_ms, 240_000);
        assert_eq!(second.segment_index, 1);
        assert_eq!(
            second.class_at_position,
            MatchClassV3::SameMediaDifferentCut
        );
    }

    #[test]
    fn position_in_edit_gap_returns_none() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameMediaDifferentCut,
            vec![
                test_segment_v3(0, 90_000, 0, 90_000, 1_000_000),
                test_segment_v3(150_000, 240_000, 180_000, 270_000, 1_000_000),
            ],
        );

        assert!(map_query_position_to_candidate_ms(&map, 120_000).is_none());
        assert!(!timeline_map_contains_query_position(&map, 120_000));
    }

    #[test]
    fn reverse_mapping_round_trips_inside_segment() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameCutStrong,
            vec![test_segment_v3(10_000, 110_000, 20_000, 121_000, 1_010_000)],
        );

        let forward = map_query_position_to_candidate_ms(&map, 60_000).expect("forward map");
        let reverse =
            map_candidate_position_to_query_ms(&map, forward.mapped_ms).expect("reverse map");

        assert!((i64::from(reverse.mapped_ms) - 60_000).abs() <= 1);
        assert_eq!(reverse.segment_index, 0);
        assert!((reverse.local_offset_ms - (i64::from(forward.mapped_ms) - 60_000)).abs() <= 1);
    }

    #[test]
    fn same_media_different_cut_maps_but_not_autoplay() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameMediaDifferentCut,
            vec![test_segment_v3(0, 180_000, 30_000, 210_000, 1_000_000)],
        );
        let mapped =
            map_query_position_to_candidate_ms(&map, 90_000).expect("different cut maps locally");
        let decision = MediaMatchDecision {
            tier: MediaMatchTier::Strong,
            evidence: MediaMatchEvidence {
                v3_class: Some(MatchClassV3::SameMediaDifferentCut),
                timeline_map_v3: Some(map),
                ..MediaMatchEvidence::default()
            },
            explanation: "different cut".to_owned(),
        };
        let settings = MediaMatchSettings {
            autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ..MediaMatchSettings::default()
        };

        assert_eq!(mapped.mapped_ms, 120_000);
        assert_eq!(
            mapped.class_at_position,
            MatchClassV3::SameMediaDifferentCut
        );
        assert!(!decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn shared_intro_outro_maps_low_confidence() {
        let map = test_timeline_map_v3(
            MatchClassV3::SharedIntroOutroOnly,
            vec![test_segment_v3(0, 90_000, 0, 90_000, 1_000_000)],
        );

        let mapped = map_query_position_to_candidate_ms(&map, 30_000)
            .expect("edge segment maps diagnostically");

        assert_eq!(mapped.class_at_position, MatchClassV3::SharedIntroOutroOnly);
        assert!(mapped.confidence <= 0.25, "{mapped:?}");
    }

    #[test]
    fn timeline_mapping_rejects_non_positive_scale() {
        let zero = test_timeline_map_v3(
            MatchClassV3::SameCutStrong,
            vec![test_segment_v3(0, 90_000, 0, 90_000, 0)],
        );
        let negative = test_timeline_map_v3(
            MatchClassV3::SameCutStrong,
            vec![test_segment_v3(0, 90_000, 0, 90_000, -1)],
        );

        assert!(map_query_position_to_candidate_ms(&zero, 30_000).is_none());
        assert!(map_query_position_to_candidate_ms(&negative, 30_000).is_none());
        assert!(map_candidate_position_to_query_ms(&zero, 30_000).is_none());
        assert!(map_candidate_position_to_query_ms(&negative, 30_000).is_none());
    }

    #[test]
    fn timeline_mapping_absurd_scale_does_not_panic() {
        let map = test_timeline_map_v3(
            MatchClassV3::SameCutStrong,
            vec![test_segment_v3(0, u32::MAX, 0, u32::MAX, i32::MAX)],
        );

        let mapped = map_query_position_to_candidate_ms(&map, u32::MAX)
            .expect("i128 arithmetic should handle public extreme scale safely");

        assert_eq!(mapped.mapped_ms, u32::MAX);
    }

    fn test_segment_v3(
        query_start_ms: u32,
        query_end_ms: u32,
        candidate_start_ms: u32,
        candidate_end_ms: u32,
        scale_ppm: i32,
    ) -> AlignedSegmentV3 {
        AlignedSegmentV3 {
            query_start_ms,
            query_end_ms,
            candidate_start_ms,
            candidate_end_ms,
            scale_ppm,
            audio_pairs: 8,
            video_pairs: 0,
            weighted_score: 8,
            residual_ms: 0.0,
            audio_score: 1.0,
            video_score: 0.0,
            confidence: 1.0,
        }
    }

    fn test_timeline_map_v3(
        global_class: MatchClassV3,
        segments: Vec<AlignedSegmentV3>,
    ) -> MediaTimelineMapV3 {
        let total_aligned_span_ms = segments
            .iter()
            .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
            .sum();
        MediaTimelineMapV3 {
            global_class,
            current_position_class: global_class,
            segments,
            total_aligned_span_ms,
            largest_gap_ms: 0,
            edge_only: false,
            audio_video_conflict: false,
            best_segment_score: 8,
            second_best_segment_score: 0,
            piecewise_pair_count: 8,
            piecewise_hypothesis_count: 1,
            piecewise_segment_candidate_count: 1,
            piecewise_segment_chain_count: 1,
            piecewise_fit_millis: 0,
        }
    }

    #[test]
    fn v3_audio_only_offset_recovery_is_within_one_second() {
        let query = audio_only_v3_anchor_profile(1_200_000, 0, 0);
        let candidate = audio_only_v3_anchor_profile(1_201_000, 1_000, 0);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert_eq!(decision.tier, MediaMatchTier::Strong, "{decision:?}");
        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutStrong)
        );
        let alignment = decision.evidence.alignment.expect("alignment evidence");
        assert!((alignment.offset_seconds - 1.0).abs() <= 1.0);
        assert_eq!(alignment.aligned_video_anchors, 0);
        assert!(alignment.aligned_audio_anchors >= 16);
        assert!(
            decision
                .evidence
                .timeline_map_v3
                .as_ref()
                .is_some_and(|map| !map.segments.is_empty())
        );
    }

    #[test]
    fn v3_audio_only_drift_recovery_reports_affine_scale() {
        let query = audio_only_v3_anchor_profile(1_200_000, 0, 0);
        let candidate = audio_only_v3_anchor_profile(1_202_000, 0, 1_500);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(
            matches!(
                decision.tier,
                MediaMatchTier::Strong | MediaMatchTier::Probable
            ),
            "{decision:?}"
        );
        let alignment = decision.evidence.alignment.expect("alignment evidence");
        assert!(
            (alignment.scale_ppm - 1_001_500).abs() <= 300,
            "{alignment:?}"
        );
    }

    #[test]
    fn same_cut_strong_single_segment_is_autoplay_eligible() {
        let audio = v3_audio_times(120_000, 18, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate =
            v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);
        let mut settings = enabled_settings();
        settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

        let decision = decide_media_match_anchors(&query, &candidate, &settings);

        assert_eq!(decision.tier, MediaMatchTier::Strong);
        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutStrong)
        );
        assert!(decision.same_media_for_autoplay(&settings));
        let map = decision.evidence.timeline_map_v3.expect("timeline map");
        assert_eq!(map.global_class, MatchClassV3::SameCutStrong);
        assert_eq!(map.segments.len(), 1);
        assert!(map.total_aligned_span_ms >= 600_000);
    }

    #[test]
    fn v3_decision_diagnostics_include_class_and_segment_counts() {
        let audio = v3_audio_times(120_000, 18, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate =
            v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());
        let summary = summarize_decision_v3_diagnostics(&decision);

        assert_eq!(summary.decision_tier, Some(MediaMatchTier::Strong));
        assert_eq!(summary.decision_class, Some(MatchClassV3::SameCutStrong));
        assert_eq!(summary.piecewise_segment_count, Some(1));
        assert!(summary.piecewise_pair_count.unwrap_or_default() > 0);
        assert!(summary.notes.iter().any(|note| note.contains("segments=1")));
    }

    #[test]
    fn affine_drift_single_segment_reports_v3_scale() {
        let audio = v3_audio_times(120_000, 18, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate =
            v3_profile_from_times(1_202_000, &v3_shift_audio_times(&audio, 0, 1_500), &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(
            matches!(
                decision.tier,
                MediaMatchTier::Strong | MediaMatchTier::Probable
            ),
            "{decision:?}"
        );
        let map = decision.evidence.timeline_map_v3.expect("timeline map");
        assert_eq!(map.segments.len(), 1);
        assert!((map.segments[0].scale_ppm - 1_001_500).abs() <= 300);
    }

    #[test]
    fn piecewise_hypothesis_pair_selection_is_capped_and_preserves_modalities() {
        let pairs = (0..700)
            .map(|index| AnchorMatchPair {
                query_t_ms: 60_000 + index * 2_000,
                candidate_t_ms: 65_000 + index * 2_000,
                modality: if index % 5 == 0 {
                    AnchorModality::Video
                } else {
                    AnchorModality::Audio
                },
                weight: if index % 7 == 0 { 8 } else { 1 },
            })
            .collect::<Vec<_>>();

        let selected = select_v3_piecewise_hypothesis_pairs(&pairs);

        assert!(selected.len() <= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS);
        assert!(
            selected
                .iter()
                .any(|pair| pair.modality == AnchorModality::Audio)
        );
        assert!(
            selected
                .iter()
                .any(|pair| pair.modality == AnchorModality::Video)
        );
    }

    #[test]
    fn sparse_same_cut_common_gap_is_not_different_cut() {
        let mut audio = v3_audio_times(120_000, 8, 45_000);
        audio.extend(
            (0..8)
                .map(|index| (2_000 + index, 850_000 + (index * 45_000)))
                .collect::<Vec<_>>(),
        );
        let query = v3_profile_from_times(1_400_000, &audio, &[]);
        let candidate =
            v3_profile_from_times(1_400_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert_ne!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameMediaDifferentCut),
            "{decision:?}"
        );
        assert!(matches!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable)
        ));
        let map = decision.evidence.timeline_map_v3.expect("timeline map");
        assert!(map.segments.len() >= 2, "{map:?}");
    }

    #[test]
    fn trimmed_intro_maps_as_different_cut_not_autoplay() {
        let audio = v3_audio_times(60_000, 24, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate_audio = audio[4..].to_vec();
        let candidate = v3_profile_from_times(1_020_000, &candidate_audio, &[]);
        let mut settings = enabled_settings();
        settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

        let decision = decide_media_match_anchors(&query, &candidate, &settings);

        assert!(matches!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameMediaDifferentCut | MatchClassV3::PartialOverlap)
        ));
        assert_ne!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SharedIntroOutroOnly)
        );
        assert!(!decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn inserted_logo_piecewise_chain_maps_two_segments() {
        let audio = v3_audio_times(120_000, 20, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate_audio = audio
            .iter()
            .enumerate()
            .map(|(index, (bucket, t_ms))| {
                let offset = if index < 9 { 5_000 } else { 80_000 };
                (*bucket, shifted_anchor_time(*t_ms, offset, 0))
            })
            .collect::<Vec<_>>();
        let candidate = v3_profile_from_times(1_280_000, &candidate_audio, &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        let map = decision.evidence.timeline_map_v3.expect("timeline map");
        assert_eq!(map.global_class, MatchClassV3::SameMediaDifferentCut);
        assert!(map.segments.len() >= 2, "{map:?}");
    }

    #[test]
    fn removed_recap_piecewise_chain_maps_two_segments() {
        let mut audio = v3_audio_times(120_000, 8, 45_000);
        audio.extend(
            (0..12)
                .map(|index| (1_008 + index, 600_000 + (index * 45_000)))
                .collect::<Vec<_>>(),
        );
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate_audio = audio
            .iter()
            .enumerate()
            .map(|(index, (bucket, t_ms))| {
                let offset = if index < 8 { 5_000 } else { -65_000 };
                (*bucket, shifted_anchor_time(*t_ms, offset, 0))
            })
            .collect::<Vec<_>>();
        let candidate = v3_profile_from_times(1_130_000, &candidate_audio, &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        let map = decision.evidence.timeline_map_v3.expect("timeline map");
        assert_eq!(map.global_class, MatchClassV3::SameMediaDifferentCut);
        assert!(map.segments.len() >= 2, "{map:?}");
    }

    #[test]
    fn wrong_episode_shared_intro_outro_is_edge_only() {
        let audio = vec![
            (1_000, 0),
            (1_001, 30_000),
            (1_002, 60_000),
            (1_003, 1_100_000),
            (1_004, 1_130_000),
            (1_005, 1_160_000),
        ];
        let video = v3_video_times_from_audio(&audio);
        let query = v3_profile_from_times(1_200_000, &audio, &video);
        let candidate = query.clone();

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(!matches!(decision.tier, MediaMatchTier::Strong));
        assert!(matches!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject)
        ));
        assert!(
            decision
                .evidence
                .timeline_map_v3
                .as_ref()
                .is_some_and(|map| map.edge_only)
        );
    }

    #[test]
    fn partial_overlap_trailer_or_clip_is_not_same_cut() {
        let audio = v3_audio_times(420_000, 6, 30_000);
        let query = v3_profile_from_times(1_200_000, &audio, &[]);
        let candidate =
            v3_profile_from_times(240_000, &v3_shift_audio_times(&audio, -360_000, 0), &[]);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::PartialOverlap)
        );
        assert_ne!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutStrong)
        );
    }

    #[test]
    fn same_audio_weak_video_is_not_false_conflict() {
        let audio = v3_audio_times(120_000, 18, 45_000);
        let query_video = vec![(10_000, 120_000, synthetic_hash(1))];
        let candidate_video = vec![(20_000, 125_000, synthetic_hash(2))];
        let query = v3_profile_from_times(1_200_000, &audio, &query_video);
        let candidate = v3_profile_from_times(
            1_200_000,
            &v3_shift_audio_times(&audio, 5_000, 0),
            &candidate_video,
        );

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(
            matches!(
                decision.evidence.v3_class,
                Some(MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable)
            ),
            "{decision:?}"
        );
        assert_ne!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameAudioDifferentVideo)
        );
    }

    #[test]
    fn same_audio_different_video_is_not_autoplay() {
        let audio = v3_audio_times(120_000, 18, 45_000);
        let video = v3_video_times_from_audio(&audio);
        let query = v3_profile_from_times(1_200_000, &audio, &video);
        let candidate = v3_profile_from_times(
            1_200_000,
            &v3_shift_audio_times(&audio, 5_000, 0),
            &v3_shift_video_times(&video, 90_000, 0),
        );
        let mut settings = enabled_settings();
        settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

        let decision = decide_media_match_anchors(&query, &candidate, &settings);

        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameAudioDifferentVideo)
        );
        assert!(!decision.same_media_for_autoplay(&settings));
    }

    #[test]
    fn same_video_different_audio_requires_contradictory_audio() {
        let video_source = v3_audio_times(120_000, 18, 45_000);
        let video = v3_video_times_from_audio(&video_source);
        let audio = v3_audio_times(120_000, 6, 45_000);
        let query = v3_profile_from_times(1_200_000, &audio, &video);
        let candidate = v3_profile_from_times(
            1_200_000,
            &v3_shift_audio_times(&audio, 90_000, 0),
            &v3_shift_video_times(&video, 5_000, 0),
        );

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameVideoDifferentAudio)
        );
    }

    #[test]
    fn same_video_different_audio_is_not_autoplay() {
        let audio = v3_audio_times(120_000, 12, 45_000);
        let video = v3_video_times_from_audio(&audio);
        let query = v3_profile_from_times(1_200_000, &[], &video);
        let candidate =
            v3_profile_from_times(1_200_000, &[], &v3_shift_video_times(&video, 5_000, 0));
        let mut settings = enabled_settings();
        settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

        let decision = decide_media_match_anchors(&query, &candidate, &settings);

        assert_eq!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameVideoDifferentAudio)
        );
        assert!(!decision.same_media_for_autoplay(&settings));
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

        assert!(
            matches!(
                decision.tier,
                MediaMatchTier::Strong | MediaMatchTier::Probable
            ),
            "{decision:?}"
        );
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
            v3_landmarks: Vec::new(),
        };
        let candidate_video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![FrameFingerprint {
                timestamp_millis: 31_000,
                hash: candidate_hash,
            }],
            v3_landmarks: Vec::new(),
        };
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&query_video, 4),
        };
        let candidate = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
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
    fn video_matching_falls_back_when_hamming_near_hash_touches_every_lsh_band() {
        let query_hash = 0x0123_4567_89ab_cdef;
        let candidate_hash = query_hash ^ 0x0001_0001_0001_0001;
        assert!(video_anchor_hashes_match(query_hash, candidate_hash));
        assert!(
            video_lsh_buckets(query_hash)
                .iter()
                .all(|bucket| !video_lsh_buckets(candidate_hash).contains(bucket)),
            "fixture must touch every contiguous LSH band"
        );
        let query_video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![FrameFingerprint {
                timestamp_millis: 30_000,
                hash: query_hash,
            }],
            v3_landmarks: Vec::new(),
        };
        let candidate_video = VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![FrameFingerprint {
                timestamp_millis: 31_000,
                hash: candidate_hash,
            }],
            v3_landmarks: Vec::new(),
        };
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&query_video, 4),
        };
        let candidate = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
        };

        let pairs = collect_anchor_match_pairs(&query, &candidate);

        assert!(
            !pairs.is_empty(),
            "Hamming fallback should recover a near perceptual hash even when LSH buckets all differ"
        );
    }

    #[test]
    fn video_anchor_coverage_counts_unique_frames_not_lsh_bands() {
        let hash = synthetic_hash(42);
        let query_video = video_from_hashes(30, 10, &[hash]);
        let candidate_video = video_from_hashes(32, 10, &[hash]);
        let query = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&query_video, 4),
        };
        let candidate = MediaAnchorProfile {
            version: MEDIA_MATCH_ANCHOR_VERSION,
            profile: "combined-v3".to_owned(),
            duration_ms: Some(120_000),
            audio_anchors: Vec::new(),
            video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
        };

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());
        let video = decision
            .evidence
            .video
            .expect("video evidence should be present");

        assert_eq!(video.aligned_pairs, 1);
        assert_eq!(video.query_coverage, 1.0);
        assert_eq!(video.candidate_coverage, 1.0);
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

        assert!(
            matches!(
                decision.tier,
                MediaMatchTier::Strong | MediaMatchTier::Probable
            ),
            "{decision:?}"
        );
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
    fn anchor_matching_fits_long_duration_drift_when_offset_bins_are_spread() {
        let query_times = (0..20)
            .map(|index| 120_000 + index * 180_000)
            .collect::<Vec<_>>();
        let query_audio = query_times
            .iter()
            .enumerate()
            .map(|(index, t_ms)| (index as u32 + 1, *t_ms))
            .collect::<Vec<_>>();
        let query_video = query_times
            .iter()
            .enumerate()
            .map(|(index, t_ms)| (index as u32 + 100, *t_ms, synthetic_hash(index as u64 + 1)))
            .collect::<Vec<_>>();
        let candidate_audio = query_times
            .iter()
            .enumerate()
            .map(|(index, t_ms)| (index as u32 + 1, shifted_anchor_time(*t_ms, 0, 1_200)))
            .collect::<Vec<_>>();
        let candidate_video = query_times
            .iter()
            .enumerate()
            .map(|(index, t_ms)| {
                (
                    index as u32 + 100,
                    shifted_anchor_time(*t_ms, 0, 1_200),
                    synthetic_hash(index as u64 + 1),
                )
            })
            .collect::<Vec<_>>();
        let query = anchor_profile(3_800_000, &query_audio, &query_video);
        let candidate = anchor_profile(3_800_000, &candidate_audio, &candidate_video);

        let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

        assert!(
            matches!(
                decision.tier,
                MediaMatchTier::Strong | MediaMatchTier::Probable
            ),
            "{decision:?}"
        );
        let alignment = decision.evidence.alignment.expect("alignment evidence");
        assert!(alignment.aligned_pairs >= 30, "{alignment:?}");
        assert!(
            (alignment.scale_ppm - 1_001_200).abs() <= 250,
            "{alignment:?}"
        );
    }

    #[test]
    fn audio_constellation_v3_process_budget_is_audio_only() {
        let counts = expected_media_tool_invocation_counts(
            &MediaExtractionSettings::audio_constellation_v3(),
        );
        assert_eq!(counts.ffmpeg + counts.ffprobe, 2);
        assert_eq!(counts.ffmpeg, 1);
    }

    #[test]
    fn combined_v3_process_budget_includes_video() {
        let counts = expected_media_tool_invocation_counts(&MediaExtractionSettings::combined_v3());
        assert_eq!(counts.ffmpeg, 2);
        assert_eq!(counts.ffprobe, 1);
    }

    #[test]
    fn audio_constellation_v3_extraction_uses_ffmpeg_and_ffprobe_only() {
        let root = unique_test_root("audio-v3-tools");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("test media should be written");
        let tools = MediaMatchToolPaths {
            ffmpeg: write_fake_tool(&root, "ffmpeg", None),
            ffprobe: write_fake_tool(&root, "ffprobe", Some("1.0")),
        };

        let result = fingerprint_media_file_with_report(
            &media_path,
            &tools,
            &MediaExtractionSettings::audio_constellation_v3(),
            None,
        )
        .expect("V3 fingerprint should tolerate empty fake ffmpeg as a modality error");

        assert_eq!(result.report.invocations.ffprobe, 1);
        assert_eq!(result.report.invocations.ffmpeg, 1);
        assert!(result.record.audio_error.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn combined_v3_extraction_uses_ffmpeg_and_ffprobe_only() {
        let root = unique_test_root("combined-v3-tools");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("test media should be written");
        let tools = MediaMatchToolPaths {
            ffmpeg: write_fake_tool(&root, "ffmpeg", None),
            ffprobe: write_fake_tool(&root, "ffprobe", Some("1.0")),
        };

        let result = fingerprint_media_file_with_report(
            &media_path,
            &tools,
            &MediaExtractionSettings::combined_v3(),
            None,
        )
        .expect("combined V3 fingerprint should tolerate empty fake ffmpeg as modality errors");

        assert_eq!(result.report.invocations.ffprobe, 1);
        assert_eq!(result.report.invocations.ffmpeg, 2);
        assert!(result.record.audio_error.is_some());
        assert!(result.record.video_error.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wire_signature_round_trips_v3_audio_profile() {
        let mut record = record_with_extraction_settings(
            "[Judas] Show - S01E07.mkv",
            100,
            Some(1412.37),
            None,
            MediaExtractionSettings::audio_constellation_v3(),
        );
        record.audio_anchors = audio_only_v3_anchor_profile(1_412_370, 0, 0).audio_anchors;

        let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
            .expect("wire value should serialize");
        let signature =
            media_match_wire_signature_from_value(&value).expect("wire signature should parse");
        let profile = media_anchor_profile_from_wire_profile(&signature.profiles[0])
            .expect("v3 profile should decode");

        assert_eq!(signature.schema, MEDIA_MATCH_WIRE_SCHEMA_V3);
        assert_eq!(signature.profiles[0].profile, "audio-constellation-v3");
        assert!(!profile.audio_anchors.is_empty());
        assert!(profile.video_anchors.is_empty());
    }

    #[test]
    fn wire_signature_compares_local_record_to_remote_profile() {
        let query_profile = audio_only_v3_anchor_profile(1_412_000, 0, 0);
        let remote_profile = audio_only_v3_anchor_profile(1_413_000, 1_000, 0);
        let mut query = record_with_extraction_settings(
            "[Judas] Show - S01E07.mkv",
            100,
            Some(1412.0),
            None,
            MediaExtractionSettings::audio_constellation_v3(),
        );
        query.audio_anchors = query_profile.audio_anchors;
        let mut remote = record_with_extraction_settings(
            "[Erai-raws] Show - 07.mkv",
            200,
            Some(1413.0),
            None,
            MediaExtractionSettings::audio_constellation_v3(),
        );
        remote.audio_anchors = remote_profile.audio_anchors;
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
    }

    #[test]
    fn wire_signature_rejects_unsupported_v3_profile_fields() {
        let mut record = record_with_extraction_settings(
            "episode.mkv",
            100,
            Some(120.0),
            None,
            MediaExtractionSettings::audio_constellation_v3(),
        );
        record.audio_anchors = audio_only_v3_anchor_profile(120_000, 0, 0).audio_anchors;
        let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
            .expect("wire value should serialize");

        let mut unsupported_version = value.clone();
        unsupported_version["profiles"][0]["algorithmVersion"] =
            serde_json::json!(MEDIA_MATCH_ANCHOR_VERSION + 1);
        assert!(media_match_wire_signature_from_value(&unsupported_version).is_err());

        let mut unknown_profile = value.clone();
        unknown_profile["profiles"][0]["profile"] = serde_json::json!("audio-v999");
        assert!(media_match_wire_signature_from_value(&unknown_profile).is_err());

        let mut wrong_time_base = value.clone();
        wrong_time_base["profiles"][0]["audio"]["timeBaseMs"] = serde_json::json!(1000);
        assert!(media_match_wire_signature_from_value(&wrong_time_base).is_err());

        let mut wrong_algorithm = value;
        wrong_algorithm["profiles"][0]["audio"]["algorithm"] =
            serde_json::json!("unsupported-audio-anchor-algorithm");
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
    fn ffmpeg_rawvideo_parser_uses_showinfo_pts_for_full_profile_frames() {
        let mut stdout = vec![32u8; VIDEO_FRAME_BYTES];
        stdout.extend(std::iter::repeat_n(224u8, VIDEO_FRAME_BYTES));
        let stderr = "\
[Parsed_showinfo_1 @ 000001] n:   0 pts: 48000 pts_time:2.000 pos:0
[Parsed_showinfo_1 @ 000001] n:   1 pts: 103200 pts_time:4.300 pos:0
";

        let frames = video_frames_from_ffmpeg_rawvideo(&stdout, stderr.as_bytes())
            .expect("frames should decode");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].timestamp_millis, 2_000);
        assert_eq!(frames[1].timestamp_millis, 4_300);
    }

    #[test]
    #[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
    fn combined_v3_ffmpeg_generates_v3_video_kinds() {
        let Some(ffmpeg) = test_ffmpeg_path() else {
            eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
            return;
        };
        let Some(ffprobe) = test_ffprobe_path() else {
            eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
            return;
        };
        let media_path = temp_media_match_path("combined-v3-kinds", "mkv");
        let status = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=1:duration=90",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100:duration=90",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&media_path)
            .status()
            .expect("ffmpeg should create synthetic media");
        assert!(status.success(), "ffmpeg fixture generation failed");
        let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
        let fingerprint = fingerprint_media_file_with_report(
            &media_path,
            &tools,
            &MediaExtractionSettings::combined_v3(),
            None,
        )
        .expect("combined v3 fingerprint should extract");
        let _ = std::fs::remove_file(&media_path);
        let video_landmarks = fingerprint
            .record
            .video
            .as_ref()
            .map(|video| video.v3_landmarks.as_slice())
            .unwrap_or_default();
        let kinds = video_landmarks
            .iter()
            .map(|landmark| landmark.kind)
            .collect::<HashSet<_>>();

        assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
        assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
        assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
    }

    #[test]
    #[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
    fn audio_v3_streaming_extracts_synthetic_audio() {
        let Some(ffmpeg) = test_ffmpeg_path() else {
            eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
            return;
        };
        let Some(ffprobe) = test_ffprobe_path() else {
            eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
            return;
        };
        let media_path = temp_media_match_path("audio-v3-streaming", "wav");
        let status = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:sample_rate=44100:duration=12",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&media_path)
            .status()
            .expect("ffmpeg should create synthetic audio");
        assert!(status.success(), "ffmpeg fixture generation failed");
        let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
        let fingerprint = fingerprint_media_file_with_report(
            &media_path,
            &tools,
            &MediaExtractionSettings::audio_constellation_v3(),
            None,
        )
        .expect("audio v3 fingerprint should extract");
        let _ = std::fs::remove_file(&media_path);

        assert!(!audio_landmarks_v3_from_record(&fingerprint.record).is_empty());
        assert!(fingerprint.report.audio_stream.streamed_bytes > 0);
        assert!(fingerprint.report.audio_stream.max_buffer_samples <= V3_AUDIO_WINDOW_SAMPLES);
        assert!(
            fingerprint.report.audio_stream.max_raw_landmarks_buffered
                <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT
        );
    }

    #[test]
    #[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
    fn combined_v3_storage_bound_on_synthetic_media() {
        let Some(ffmpeg) = test_ffmpeg_path() else {
            eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
            return;
        };
        let Some(ffprobe) = test_ffprobe_path() else {
            eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
            return;
        };
        let media_path = temp_media_match_path("combined-v3-storage", "mkv");
        let status = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=1:duration=120",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=520:sample_rate=44100:duration=120",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&media_path)
            .status()
            .expect("ffmpeg should create synthetic media");
        assert!(status.success(), "ffmpeg fixture generation failed");
        let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
        let fingerprint = fingerprint_media_file_with_report(
            &media_path,
            &tools,
            &MediaExtractionSettings::combined_v3(),
            None,
        )
        .expect("combined v3 fingerprint should extract");
        let _ = std::fs::remove_file(&media_path);
        let diagnostics = summarize_record_v3_diagnostics(&fingerprint.record);

        assert!(diagnostics.video_verify_count <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
        assert!(diagnostics.video_index_count <= V3_VIDEO_INDEX_LANDMARK_LIMIT);
        assert!(diagnostics.audio_blob_bytes > 0);
        assert!(diagnostics.video_blob_bytes > 0);
    }

    fn test_ffmpeg_path() -> Option<PathBuf> {
        test_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg")
    }

    fn test_ffprobe_path() -> Option<PathBuf> {
        test_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe")
    }

    fn test_tool_path(env_key: &str, default_name: &str) -> Option<PathBuf> {
        let path = std::env::var_os(env_key)
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

    fn temp_media_match_path(prefix: &str, extension: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "sorotte-{prefix}-{}-{extension}.{extension}",
            std::process::id()
        ));
        path
    }

    #[test]
    fn exact_decision_uses_path_mtime_and_size() {
        let query = record("C:/Media/Movie.mkv", 100, Some(100.0), None);
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
            Some(shifted_video(0, &query_hashes)),
        );
        let candidate = record(
            "show-e02.mkv",
            100,
            Some(1200.0),
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
            v3_landmarks: Vec::new(),
        };
        let candidate = VideoFingerprint {
            duration_seconds: Some(86),
            frames: hashes
                .iter()
                .enumerate()
                .map(|(index, hash)| FrameFingerprint::new(index as f64 * 10.8, *hash))
                .collect(),
            v3_landmarks: Vec::new(),
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
    fn candidate_ranking_prefers_nearest_reject_with_timeline_evidence() {
        let query_profile = anchor_profile(900_000, &[(42, 10_000)], &[]);
        let nearest_profile = anchor_profile(900_000, &[(42, 12_000)], &[]);
        let unrelated_profile = anchor_profile(900_000, &[(84, 10_000)], &[]);
        let query = record_from_anchor_profile("episode.web.mkv", 100, query_profile);
        let nearest = record_from_anchor_profile("episode-nearest.mkv", 110, nearest_profile);
        let unrelated = record_from_anchor_profile("episode-unrelated.mkv", 120, unrelated_profile);

        let ranked =
            rank_media_match_candidates(&query, [&unrelated, &nearest], &enabled_settings());

        assert_eq!(ranked[0].decision.tier, MediaMatchTier::Reject);
        assert_eq!(
            ranked[0].candidate_path,
            normalize_media_path("episode-nearest.mkv")
        );
        assert_eq!(
            ranked[0]
                .decision
                .evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.aligned_pairs),
            Some(1)
        );
    }

    #[test]
    fn cache_invalidates_on_identity_and_algorithm_inputs() {
        let settings = MediaExtractionSettings::combined_v3();
        let audio_settings = MediaExtractionSettings::audio_constellation_v3();
        let mut cache = MediaMatchCache::default();
        let record = record("movie.mkv", 100, Some(10.0), None);
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
                    &audio_settings
                )
                .is_none()
        );
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
    fn pdq_style_luma_hash_is_stable_for_same_pixels() {
        let luma = (0u8..64).collect::<Vec<_>>();

        let left = pdq_style_luma_hash(8, 8, &luma).expect("hash");
        let right = pdq_style_luma_hash(8, 8, &luma).expect("hash");

        assert_eq!(left, right);
    }
}
