use std::{fmt, path::PathBuf};

use crate::{MediaExtractionSettings, MediaFingerprintRecord};

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
    pub raw_landmarks_before_bounding: usize,
    pub final_landmarks: usize,
    pub max_buffer_samples: usize,
    pub max_raw_landmarks_seen: usize,
    pub max_raw_landmarks_after_compaction: usize,
    pub raw_landmark_compactions: usize,
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

pub fn expected_media_tool_invocation_counts(
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
