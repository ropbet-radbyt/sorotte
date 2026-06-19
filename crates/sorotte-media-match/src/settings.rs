use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION, MEDIA_MATCH_WIRE_SCHEMA_V3,
    tuning::{
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT, V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
        V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT, V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
        current_v3_tuning,
    },
};

pub const MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION: u32 = 2;
pub const MEDIA_MATCH_V3_PROFILE_LABEL: &str = "audio-constellation-v3";
pub const MEDIA_MATCH_V3_AUDIO_ALGORITHM: &str = "sorotte-audio-constellation-v3-sampled-fast";

fn default_sampled_policy_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSampledAudioPolicy {
    #[serde(default = "default_sampled_policy_version")]
    pub policy_version: u32,
    pub window_seconds: u32,
    pub window_count: usize,
    pub sample_rate: u32,
    pub landmark_limit: usize,
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
            window_seconds: V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
            window_count: V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT,
            sample_rate: V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
            landmark_limit: V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
        }
    }

    pub fn label(&self) -> String {
        format!(
            "sampled-fast-fixed-{}x{}s-current",
            self.window_count, self.window_seconds
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
    #[serde(
        default,
        skip_serializing_if = "MediaSampledAudioPolicy::is_default_policy"
    )]
    pub sampled_audio_policy: MediaSampledAudioPolicy,
}

impl Default for MediaExtractionSettings {
    fn default() -> Self {
        Self::sampled_fast_audio_index_v3()
    }
}

impl MediaExtractionSettings {
    pub fn sampled_fast_audio_index_v3() -> Self {
        Self {
            sampled_audio_policy: MediaSampledAudioPolicy::fixed_sampled_fast_current(),
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

        assert!(settings.sampled_audio_policy.is_production_compatible());
        assert_eq!(
            settings.sampled_audio_policy.window_count,
            V3_AUDIO_SAMPLED_FAST_WINDOW_COUNT
        );
        assert_eq!(settings.sampled_audio_policy.window_seconds, 20);
        assert_eq!(settings.sampled_audio_policy.sample_rate, 8_000);
        assert_eq!(settings.sampled_audio_policy.landmark_limit, 384);
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
