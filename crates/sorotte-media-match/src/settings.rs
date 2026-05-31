use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION, MEDIA_MATCH_WIRE_SCHEMA_V3,
    tuning::{
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT, V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS,
        V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS, V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
        V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS, V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
        V3_AUDIO_SAMPLED_MIN_BODY_REGIONS, current_v3_tuning,
    },
};

pub const MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaFingerprintProfile {
    AudioConstellationV3,
    CombinedV3,
}

impl MediaFingerprintProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AudioConstellationV3 => "audio-constellation-v3",
            Self::CombinedV3 => "combined-v3",
        }
    }

    pub fn is_fast(self) -> bool {
        matches!(self, Self::AudioConstellationV3)
    }

    pub fn uses_video_by_default(self) -> bool {
        matches!(self, Self::CombinedV3)
    }
}

fn default_media_fingerprint_profile() -> MediaFingerprintProfile {
    MediaFingerprintProfile::AudioConstellationV3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaAudioIndexMode {
    FullVerify,
    SparseFull,
    SampledFast,
    SampledNormal,
}

impl MediaAudioIndexMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullVerify => "full-verify",
            Self::SparseFull => "sparse-full",
            Self::SampledFast => "sampled-fast",
            Self::SampledNormal => "sampled-normal",
        }
    }

    pub fn is_sampled(self) -> bool {
        matches!(self, Self::SampledFast | Self::SampledNormal)
    }

    pub fn is_dense_full_verify(self) -> bool {
        matches!(self, Self::FullVerify)
    }
}

fn default_media_audio_index_mode() -> MediaAudioIndexMode {
    MediaAudioIndexMode::FullVerify
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaDenseAudioProfile {
    #[default]
    DenseCurrent,
    DenseRealfft,
    Dense8k,
    DenseHop2048,
    Dense8kHop2048,
    Dense8kWindow1024Hop1024,
    DenseMaxPeaks4,
    DensePairRetain16,
    DenseGated,
    DenseFastCombinedCandidate,
}

impl MediaDenseAudioProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::DenseCurrent => "dense-current",
            Self::DenseRealfft => "dense-realfft",
            Self::Dense8k => "dense-8k",
            Self::DenseHop2048 => "dense-hop2048",
            Self::Dense8kHop2048 => "dense-8k-hop2048",
            Self::Dense8kWindow1024Hop1024 => "dense-8k-window1024-hop1024",
            Self::DenseMaxPeaks4 => "dense-max-peaks-4",
            Self::DensePairRetain16 => "dense-pair-retain-16",
            Self::DenseGated => "dense-gated",
            Self::DenseFastCombinedCandidate => "dense-fast-combined-candidate",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaSampledAudioSourceStrategy {
    #[default]
    Current,
    SingleProcessFilter,
    FastSeekPerWindow,
    OutputSeekPerWindow,
    FfprobeProbe,
    PacketMap,
    MkvAudioRanges,
    SampledPcmCache,
    Auto,
}

impl MediaSampledAudioSourceStrategy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::SingleProcessFilter => "single-process-filter",
            Self::FastSeekPerWindow => "fast-seek-per-window",
            Self::OutputSeekPerWindow => "output-seek-per-window",
            Self::FfprobeProbe => "ffprobe-probe",
            Self::PacketMap => "packet-map",
            Self::MkvAudioRanges => "mkv-audio-ranges",
            Self::SampledPcmCache => "sampled-pcm-cache",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaSampledFfmpegWindowStrategy {
    #[default]
    CurrentThreeInvocations,
    SingleProcessFilter,
    FastSeekPerWindow,
    OutputSeekPerWindow,
    FfprobeProbe,
    PacketMap,
    MkvAudioRanges,
    SampledPcmCache,
    Auto,
}

impl MediaSampledFfmpegWindowStrategy {
    pub fn label(self) -> &'static str {
        match self {
            Self::CurrentThreeInvocations => "current-three-invocations",
            Self::SingleProcessFilter => "single-process-filter",
            Self::FastSeekPerWindow => "fast-seek-per-window",
            Self::OutputSeekPerWindow => "output-seek-per-window",
            Self::FfprobeProbe => "ffprobe-probe",
            Self::PacketMap => "packet-map",
            Self::MkvAudioRanges => "mkv-audio-ranges",
            Self::SampledPcmCache => "sampled-pcm-cache",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaSampledWindowPlacementAlgorithm {
    #[default]
    BodyDistributedV1,
}

impl MediaSampledWindowPlacementAlgorithm {
    pub fn label(self) -> &'static str {
        match self {
            Self::BodyDistributedV1 => "body-distributed-v1",
        }
    }
}

fn default_sampled_policy_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSampledAudioPolicy {
    #[serde(default = "default_sampled_policy_version")]
    pub policy_version: u32,
    #[serde(default)]
    pub ffmpeg_window_strategy: MediaSampledFfmpegWindowStrategy,
    #[serde(default)]
    pub window_placement_algorithm: MediaSampledWindowPlacementAlgorithm,
    pub sampled_fast_window_seconds: u32,
    pub sampled_fast_min_windows: usize,
    pub sampled_fast_max_windows: usize,
    pub sampled_fast_target_landmarks: usize,
    pub sampled_fast_index_landmark_limit: usize,
    pub sampled_fast_min_body_regions: usize,
    pub sampled_fast_sample_rate: u32,
}

impl Default for MediaSampledAudioPolicy {
    fn default() -> Self {
        Self::fixed_sampled_fast_current()
    }
}

impl MediaSampledAudioPolicy {
    pub fn fixed_sampled_fast_current() -> Self {
        Self {
            policy_version: default_sampled_policy_version(),
            ffmpeg_window_strategy: MediaSampledFfmpegWindowStrategy::CurrentThreeInvocations,
            window_placement_algorithm: MediaSampledWindowPlacementAlgorithm::BodyDistributedV1,
            sampled_fast_window_seconds: V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
            sampled_fast_min_windows: V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS,
            sampled_fast_max_windows: V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS,
            sampled_fast_target_landmarks: V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS,
            sampled_fast_index_landmark_limit: V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
            sampled_fast_min_body_regions: V3_AUDIO_SAMPLED_MIN_BODY_REGIONS
                .min(V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS),
            sampled_fast_sample_rate: V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
        }
    }

    pub fn for_sampled_fast_source_strategy(
        source_strategy: MediaSampledAudioSourceStrategy,
    ) -> Self {
        let mut policy = Self::fixed_sampled_fast_current();
        policy.ffmpeg_window_strategy = match source_strategy {
            MediaSampledAudioSourceStrategy::SingleProcessFilter => {
                MediaSampledFfmpegWindowStrategy::SingleProcessFilter
            }
            MediaSampledAudioSourceStrategy::FastSeekPerWindow => {
                MediaSampledFfmpegWindowStrategy::FastSeekPerWindow
            }
            MediaSampledAudioSourceStrategy::OutputSeekPerWindow => {
                MediaSampledFfmpegWindowStrategy::OutputSeekPerWindow
            }
            MediaSampledAudioSourceStrategy::FfprobeProbe => {
                MediaSampledFfmpegWindowStrategy::FfprobeProbe
            }
            MediaSampledAudioSourceStrategy::PacketMap => {
                MediaSampledFfmpegWindowStrategy::PacketMap
            }
            MediaSampledAudioSourceStrategy::MkvAudioRanges => {
                MediaSampledFfmpegWindowStrategy::MkvAudioRanges
            }
            MediaSampledAudioSourceStrategy::SampledPcmCache => {
                MediaSampledFfmpegWindowStrategy::SampledPcmCache
            }
            MediaSampledAudioSourceStrategy::Auto => MediaSampledFfmpegWindowStrategy::Auto,
            MediaSampledAudioSourceStrategy::Current => {
                MediaSampledFfmpegWindowStrategy::CurrentThreeInvocations
            }
        };
        policy
    }

    pub fn label(&self) -> String {
        format!(
            "sampled-fast-fixed-{}-{}",
            self.sampled_fast_min_windows,
            self.ffmpeg_window_strategy.label()
        )
    }

    pub fn is_production_compatible(&self) -> bool {
        self == &Self::fixed_sampled_fast_current()
    }

    pub fn is_default_policy(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaExtractionSettings {
    #[serde(default = "default_media_fingerprint_profile")]
    pub profile: MediaFingerprintProfile,
    #[serde(default = "default_media_audio_index_mode")]
    pub audio_index_mode: MediaAudioIndexMode,
    #[serde(default)]
    pub dense_audio_profile: MediaDenseAudioProfile,
    #[serde(
        default,
        skip_serializing_if = "MediaSampledAudioPolicy::is_default_policy"
    )]
    pub sampled_audio_policy: MediaSampledAudioPolicy,
    pub frame_sample_interval_seconds: u32,
    pub max_frames: usize,
    pub audio_algorithm: String,
    pub video_algorithm: String,
}

impl Default for MediaExtractionSettings {
    fn default() -> Self {
        Self::audio_constellation_v3()
    }
}

impl MediaExtractionSettings {
    pub fn audio_constellation_v3() -> Self {
        Self {
            profile: MediaFingerprintProfile::AudioConstellationV3,
            audio_index_mode: MediaAudioIndexMode::FullVerify,
            dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
            sampled_audio_policy: MediaSampledAudioPolicy::default(),
            frame_sample_interval_seconds: 0,
            max_frames: 0,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "none".to_owned(),
        }
    }

    pub fn combined_v3() -> Self {
        Self {
            profile: MediaFingerprintProfile::CombinedV3,
            audio_index_mode: MediaAudioIndexMode::FullVerify,
            dense_audio_profile: MediaDenseAudioProfile::DenseCurrent,
            sampled_audio_policy: MediaSampledAudioPolicy::default(),
            frame_sample_interval_seconds: 10,
            max_frames: 64,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "sorotte-video-scene-v3".to_owned(),
        }
    }

    pub fn sampled_audio_index_v3() -> Self {
        Self {
            audio_index_mode: MediaAudioIndexMode::SampledNormal,
            audio_algorithm: "sorotte-audio-constellation-v3-sampled-normal".to_owned(),
            ..Self::audio_constellation_v3()
        }
    }

    pub fn sampled_fast_audio_index_v3() -> Self {
        Self {
            audio_index_mode: MediaAudioIndexMode::SampledFast,
            audio_algorithm: "sorotte-audio-constellation-v3-sampled-fast".to_owned(),
            ..Self::audio_constellation_v3()
        }
    }

    pub fn sparse_full_audio_v3() -> Self {
        Self {
            audio_index_mode: MediaAudioIndexMode::SparseFull,
            audio_algorithm: "sorotte-audio-constellation-v3-sparse-full".to_owned(),
            ..Self::audio_constellation_v3()
        }
    }

    pub fn with_dense_audio_profile(mut self, profile: MediaDenseAudioProfile) -> Self {
        self.dense_audio_profile = profile;
        if self.audio_index_mode.is_dense_full_verify() {
            self.audio_algorithm = if profile == MediaDenseAudioProfile::DenseCurrent {
                "sorotte-audio-constellation-v3".to_owned()
            } else {
                format!("sorotte-audio-constellation-v3-{}", profile.label())
            };
        }
        self
    }

    pub fn with_sampled_audio_policy(mut self, policy: MediaSampledAudioPolicy) -> Self {
        self.sampled_audio_policy = policy;
        self
    }
}

pub fn media_extraction_settings_hash(settings: &MediaExtractionSettings) -> [u8; 32] {
    media_match_v3_fingerprint_config_hash(settings)
}

pub fn media_match_v3_fingerprint_config_hash(settings: &MediaExtractionSettings) -> [u8; 32] {
    media_match_v3_fingerprint_config_hash_with_version_and_tuning(
        settings,
        MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
        current_v3_tuning(),
    )
}

fn media_match_v3_fingerprint_config_hash_with_version_and_tuning(
    settings: &MediaExtractionSettings,
    cache_version: u32,
    tuning: crate::V3Tuning,
) -> [u8; 32] {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct FingerprintConfig<'a> {
        cache_version: u32,
        algorithm_version: u32,
        anchor_version: u32,
        wire_schema: &'static str,
        settings: &'a MediaExtractionSettings,
        tuning: crate::V3Tuning,
    }

    // During V3 development the cache namespace intentionally includes the
    // full tuning snapshot, including retrieval-only values, so diagnostic
    // cache reuse is conservative across calibration and algorithm changes.
    let config = FingerprintConfig {
        cache_version,
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        anchor_version: MEDIA_MATCH_ANCHOR_VERSION,
        wire_schema: MEDIA_MATCH_WIRE_SCHEMA_V3,
        settings,
        tuning,
    };
    let bytes = serde_json::to_vec(&config).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_config_hash_changes_with_profile() {
        let audio = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::audio_constellation_v3(),
        );
        let combined =
            media_match_v3_fingerprint_config_hash(&MediaExtractionSettings::combined_v3());

        assert_ne!(audio, combined);
    }

    #[test]
    fn fingerprint_config_hash_changes_with_audio_index_mode() {
        let full = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::audio_constellation_v3(),
        );
        let sampled = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::sampled_audio_index_v3(),
        );
        let sampled_fast = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );
        let sparse_full = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::sparse_full_audio_v3(),
        );

        assert_ne!(full, sampled);
        assert_ne!(sampled_fast, sampled);
        assert_ne!(sparse_full, full);
    }

    #[test]
    fn fingerprint_config_hash_changes_with_dense_audio_profile() {
        let current = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::audio_constellation_v3(),
        );
        let dense_8k = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::audio_constellation_v3()
                .with_dense_audio_profile(MediaDenseAudioProfile::Dense8k),
        );

        assert_ne!(current, dense_8k);
    }

    #[test]
    fn fingerprint_config_hash_changes_with_sampled_audio_policy() {
        let fixed = media_match_v3_fingerprint_config_hash(
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );
        for strategy in non_current_sampled_audio_source_strategies() {
            let experimental = media_match_v3_fingerprint_config_hash(
                &MediaExtractionSettings::sampled_fast_audio_index_v3().with_sampled_audio_policy(
                    MediaSampledAudioPolicy::for_sampled_fast_source_strategy(strategy),
                ),
            );
            assert_ne!(
                fixed,
                experimental,
                "sampled source strategy {} must not share the production fingerprint config hash",
                strategy.label()
            );
        }
    }

    #[test]
    fn sampled_audio_policy_separates_all_experimental_source_strategies() {
        let current = MediaSampledAudioPolicy::fixed_sampled_fast_current();
        assert!(current.is_production_compatible());
        assert_eq!(
            current,
            MediaSampledAudioPolicy::for_sampled_fast_source_strategy(
                MediaSampledAudioSourceStrategy::Current,
            )
        );
        for strategy in non_current_sampled_audio_source_strategies() {
            let policy = MediaSampledAudioPolicy::for_sampled_fast_source_strategy(strategy);
            assert_ne!(
                current,
                policy,
                "sampled source strategy {} must not share the production policy",
                strategy.label()
            );
            assert!(
                !policy.is_production_compatible(),
                "sampled source strategy {} must not be production-compatible",
                strategy.label()
            );
        }
    }

    fn non_current_sampled_audio_source_strategies() -> [MediaSampledAudioSourceStrategy; 8] {
        [
            MediaSampledAudioSourceStrategy::SingleProcessFilter,
            MediaSampledAudioSourceStrategy::FastSeekPerWindow,
            MediaSampledAudioSourceStrategy::OutputSeekPerWindow,
            MediaSampledAudioSourceStrategy::FfprobeProbe,
            MediaSampledAudioSourceStrategy::PacketMap,
            MediaSampledAudioSourceStrategy::MkvAudioRanges,
            MediaSampledAudioSourceStrategy::SampledPcmCache,
            MediaSampledAudioSourceStrategy::Auto,
        ]
    }

    #[test]
    fn fingerprint_config_hash_changes_with_cache_version() {
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let current = media_match_v3_fingerprint_config_hash_with_version_and_tuning(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
            current_v3_tuning(),
        );
        let bumped = media_match_v3_fingerprint_config_hash_with_version_and_tuning(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION + 1,
            current_v3_tuning(),
        );

        assert_ne!(current, bumped);
    }

    #[test]
    fn fingerprint_config_hash_changes_with_tuning_snapshot() {
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let tuning = current_v3_tuning();
        let current = media_match_v3_fingerprint_config_hash_with_version_and_tuning(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
            tuning,
        );
        let mut changed = tuning;
        changed.retrieval_prefilter_limit += 1;
        let bumped = media_match_v3_fingerprint_config_hash_with_version_and_tuning(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
            changed,
        );

        assert_ne!(current, bumped);
    }
}
