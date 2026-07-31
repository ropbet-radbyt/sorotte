use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    anchors::AudioAnchor, identity::normalize_media_path, settings::MediaExtractionSettings,
};

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
    PartialOverlap,
    SharedIntroOutroOnly,
    Reject,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaDurationCompatibility {
    #[default]
    Unknown,
    #[serde(alias = "compatible")]
    SameCutCompatible,
    NearCompatible,
    #[serde(alias = "query-full-candidate-short")]
    ContainedOrPartial,
    #[serde(alias = "incompatible")]
    IncompatibleSameCut,
}

pub fn media_duration_compatibility_ms<T, U>(
    left_duration_ms: Option<T>,
    right_duration_ms: Option<U>,
) -> MediaDurationCompatibility
where
    T: Into<i64> + Copy,
    U: Into<i64> + Copy,
{
    let Some((shorter, longer)) = ordered_positive_durations(left_duration_ms, right_duration_ms)
    else {
        return MediaDurationCompatibility::Unknown;
    };
    let delta = longer - shorter;
    let same_cut_threshold = 3_000.max((shorter as f64 * 0.005).ceil() as i64);
    if delta <= same_cut_threshold {
        return MediaDurationCompatibility::SameCutCompatible;
    }
    let near_threshold = 10_000.max((shorter as f64 * 0.01).ceil() as i64);
    if delta <= near_threshold {
        return MediaDurationCompatibility::NearCompatible;
    }

    let ratio = shorter as f64 / longer as f64;
    let large_full_length_mismatch =
        shorter >= 10 * 60 * 1000 && longer >= 20 * 60 * 1000 && delta >= 5 * 60 * 1000;
    if large_full_length_mismatch && ratio >= 0.45 {
        MediaDurationCompatibility::IncompatibleSameCut
    } else if ratio >= 0.08 || shorter >= 60_000 {
        MediaDurationCompatibility::ContainedOrPartial
    } else {
        MediaDurationCompatibility::IncompatibleSameCut
    }
}

pub fn media_duration_ratio_ms<T, U>(
    left_duration_ms: Option<T>,
    right_duration_ms: Option<U>,
) -> Option<f64>
where
    T: Into<i64> + Copy,
    U: Into<i64> + Copy,
{
    let (shorter, longer) = ordered_positive_durations(left_duration_ms, right_duration_ms)?;
    Some(shorter as f64 / longer as f64)
}

fn ordered_positive_durations<T, U>(
    left_duration_ms: Option<T>,
    right_duration_ms: Option<U>,
) -> Option<(i64, i64)>
where
    T: Into<i64> + Copy,
    U: Into<i64> + Copy,
{
    let left = left_duration_ms?.into();
    let right = right_duration_ms?.into();
    if left <= 0 || right <= 0 {
        return None;
    }
    Some(if left <= right {
        (left, right)
    } else {
        (right, left)
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedSegmentV3 {
    pub query_start_ms: u32,
    pub query_end_ms: u32,
    pub candidate_start_ms: u32,
    pub candidate_end_ms: u32,
    /// Absolute affine scale in parts per million; `1_000_000` is unity.
    pub scale_ppm: i32,
    #[serde(default)]
    pub audio_pairs: usize,
    #[serde(default)]
    pub weighted_score: u32,
    #[serde(default)]
    pub residual_ms: f64,
    pub audio_score: f32,
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
    pub best_segment_score: u32,
    #[serde(default)]
    pub second_best_segment_score: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelinePositionMapResult {
    pub mapped_ms: u32,
    pub class_at_position: MatchClassV3,
    pub segment_index: usize,
    pub confidence: f32,
    pub local_offset_ms: i64,
    /// Absolute affine scale copied from the selected segment.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_size: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_delta_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_within_tolerance: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_compatibility: Option<MediaDurationCompatibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename_stem_similarity: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioMatchEvidence {
    pub similarity: f64,
    pub shared_anchor_ratio: f64,
    pub duration_delta_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFingerprintRecord {
    pub identity: MediaFileIdentity,
    pub algorithm_version: u32,
    pub extraction_settings: MediaExtractionSettings,
    pub duration_seconds: Option<f64>,
    pub container_fingerprint: String,
    #[serde(default)]
    pub audio_anchors: Vec<AudioAnchor>,
    #[serde(default)]
    pub audio_error: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTimelineAlignment {
    pub offset_seconds: f64,
    /// Drift from affine unity in parts per million; `0` is unity.
    pub scale_ppm: i32,
    pub drift_ratio: f64,
    pub aligned_pairs: usize,
    pub aligned_audio_anchors: usize,
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
