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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaExtractionSettings {
    #[serde(default = "default_media_fingerprint_profile")]
    pub profile: MediaFingerprintProfile,
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
            frame_sample_interval_seconds: 0,
            max_frames: 0,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "none".to_owned(),
        }
    }

    pub fn combined_v3() -> Self {
        Self {
            profile: MediaFingerprintProfile::CombinedV3,
            frame_sample_interval_seconds: 10,
            max_frames: 64,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "sorotte-video-scene-v3".to_owned(),
        }
    }
}

pub fn media_extraction_settings_hash(settings: &MediaExtractionSettings) -> [u8; 32] {
    media_match_v3_fingerprint_config_hash(settings)
}

pub fn media_match_v3_fingerprint_config_hash(settings: &MediaExtractionSettings) -> [u8; 32] {
    media_match_v3_fingerprint_config_hash_with_version(
        settings,
        MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
    )
}

fn media_match_v3_fingerprint_config_hash_with_version(
    settings: &MediaExtractionSettings,
    cache_version: u32,
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
        tuning: current_v3_tuning(),
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
    fn fingerprint_config_hash_changes_with_cache_version() {
        let settings = MediaExtractionSettings::audio_constellation_v3();
        let current = media_match_v3_fingerprint_config_hash_with_version(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION,
        );
        let bumped = media_match_v3_fingerprint_config_hash_with_version(
            &settings,
            MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION + 1,
        );

        assert_ne!(current, bumped);
    }
}
