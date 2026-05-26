use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{AudioLandmarkV3, VideoLandmarkV3};

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
            Self::TooManyLandmarks(count) => write!(
                formatter,
                "media fingerprint v3 blob has too many landmarks ({count})"
            ),
            Self::InvalidSection(section) => {
                write!(
                    formatter,
                    "invalid media fingerprint v3 blob section {section}"
                )
            }
            Self::NonMonotonicTime => write!(
                formatter,
                "media fingerprint v3 blob timestamps are not monotonic"
            ),
            Self::UnsupportedVideoKind(kind) => {
                write!(
                    formatter,
                    "unsupported media fingerprint v3 video landmark kind {kind}"
                )
            }
            Self::MismatchedVideoBucketKind { kind, bucket_kind } => {
                write!(
                    formatter,
                    "media fingerprint v3 video kind {kind} does not match bucket kind {bucket_kind}"
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
