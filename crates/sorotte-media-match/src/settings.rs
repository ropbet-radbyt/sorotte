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

pub const MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaFingerprintProfile {
    AudioConstellationV3,
}

impl MediaFingerprintProfile {
    pub fn label(self) -> &'static str {
        "audio-constellation-v3"
    }
}

fn default_media_fingerprint_profile() -> MediaFingerprintProfile {
    MediaFingerprintProfile::AudioConstellationV3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaAudioIndexMode {
    SampledFast,
}

impl MediaAudioIndexMode {
    pub fn label(self) -> &'static str {
        "sampled-fast"
    }
}

fn default_media_audio_index_mode() -> MediaAudioIndexMode {
    MediaAudioIndexMode::SampledFast
}

fn default_sampled_policy_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSampledAudioPolicy {
    #[serde(default = "default_sampled_policy_version")]
    pub policy_version: u32,
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

    pub fn label(&self) -> String {
        format!(
            "sampled-fast-fixed-{}x{}s-current",
            self.sampled_fast_max_windows, self.sampled_fast_window_seconds
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
    #[serde(
        default,
        skip_serializing_if = "MediaSampledAudioPolicy::is_default_policy"
    )]
    pub sampled_audio_policy: MediaSampledAudioPolicy,
    pub audio_algorithm: String,
}

impl Default for MediaExtractionSettings {
    fn default() -> Self {
        Self::audio_constellation_v3()
    }
}

impl MediaExtractionSettings {
    pub fn audio_constellation_v3() -> Self {
        Self::sampled_fast_audio_index_v3()
    }

    pub fn sampled_fast_audio_index_v3() -> Self {
        Self {
            profile: MediaFingerprintProfile::AudioConstellationV3,
            audio_index_mode: MediaAudioIndexMode::SampledFast,
            sampled_audio_policy: MediaSampledAudioPolicy::fixed_sampled_fast_current(),
            audio_algorithm: "sorotte-audio-constellation-v3-sampled-fast".to_owned(),
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
    fn production_settings_are_fixed_sampled_fast() {
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();

        assert_eq!(settings.profile.label(), "audio-constellation-v3");
        assert_eq!(settings.audio_index_mode.label(), "sampled-fast");
        assert!(settings.sampled_audio_policy.is_production_compatible());
        assert_eq!(
            settings.sampled_audio_policy.sampled_fast_max_windows,
            V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS
        );
        assert_eq!(
            settings.sampled_audio_policy.sampled_fast_window_seconds,
            20
        );
        assert_eq!(
            settings.sampled_audio_policy.sampled_fast_sample_rate,
            8_000
        );
        assert_eq!(
            settings
                .sampled_audio_policy
                .sampled_fast_index_landmark_limit,
            384
        );
    }

    #[test]
    fn fingerprint_config_hash_changes_with_cache_version() {
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
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
}
