use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::normalize_media_path;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchClassV3 {
    SameCutStrong,
    SameCutProbable,
    SameMediaDifferentCut,
    SameVideoDifferentAudio,
    SameAudioDifferentVideo,
    PartialOverlap,
    SharedIntroOutroOnly,
    Reject,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedSegmentV3 {
    pub query_start_ms: u32,
    pub query_end_ms: u32,
    pub candidate_start_ms: u32,
    pub candidate_end_ms: u32,
    pub scale_ppm: i32,
    #[serde(default)]
    pub audio_pairs: usize,
    #[serde(default)]
    pub video_pairs: usize,
    #[serde(default)]
    pub weighted_score: u32,
    #[serde(default)]
    pub residual_ms: f64,
    pub audio_score: f32,
    pub video_score: f32,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTimelineMapV3 {
    pub global_class: MatchClassV3,
    pub current_position_class: MatchClassV3,
    pub segments: Vec<AlignedSegmentV3>,
    #[serde(default)]
    pub total_aligned_span_ms: u32,
    #[serde(default)]
    pub largest_gap_ms: u32,
    #[serde(default)]
    pub edge_only: bool,
    #[serde(default)]
    pub audio_video_conflict: bool,
    #[serde(default)]
    pub best_segment_score: u32,
    #[serde(default)]
    pub second_best_segment_score: u32,
    #[serde(default)]
    pub piecewise_pair_count: usize,
    #[serde(default)]
    pub piecewise_hypothesis_count: usize,
    #[serde(default)]
    pub piecewise_segment_candidate_count: usize,
    #[serde(default)]
    pub piecewise_segment_chain_count: usize,
    #[serde(default)]
    pub piecewise_fit_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelinePositionMapResult {
    pub mapped_ms: u32,
    pub class_at_position: MatchClassV3,
    pub segment_index: usize,
    pub confidence: f32,
    pub local_offset_ms: i64,
    pub scale_ppm: i32,
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
        if !settings.autoplay_allows_strong_same_media() {
            return false;
        }
        match self.tier {
            MediaMatchTier::Exact => true,
            MediaMatchTier::Strong => self.evidence.v3_class == Some(MatchClassV3::SameCutStrong),
            MediaMatchTier::Probable
            | MediaMatchTier::Weak
            | MediaMatchTier::Reject
            | MediaMatchTier::Unknown => false,
        }
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
                v3_class: None,
                timeline_map_v3: None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v3_class: Option<MatchClassV3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_map_v3: Option<MediaTimelineMapV3>,
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

    pub fn valid_for(
        &self,
        normalized_path: &str,
        modified_unix_millis: u64,
        size_bytes: u64,
    ) -> bool {
        self.normalized_path == normalized_path
            && self.modified_unix_millis == modified_unix_millis
            && self.size_bytes == size_bytes
    }
}
