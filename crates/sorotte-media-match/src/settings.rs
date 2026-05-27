use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION, MEDIA_MATCH_WIRE_SCHEMA_V3,
    tuning::current_v3_tuning,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaExtractionSettings {
    #[serde(default = "default_media_fingerprint_profile")]
    pub profile: MediaFingerprintProfile,
    #[serde(default = "default_media_audio_index_mode")]
    pub audio_index_mode: MediaAudioIndexMode,
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
