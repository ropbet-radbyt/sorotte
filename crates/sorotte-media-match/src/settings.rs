use serde::{Deserialize, Serialize};

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

fn default_audio_sample_seconds() -> u32 {
    0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaExtractionSettings {
    #[serde(default = "default_media_fingerprint_profile")]
    pub profile: MediaFingerprintProfile,
    pub frame_sample_interval_seconds: u32,
    pub max_frames: usize,
    #[serde(default = "default_audio_sample_seconds")]
    pub audio_sample_seconds: u32,
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
            audio_sample_seconds: 0,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "none".to_owned(),
        }
    }

    pub fn combined_v3() -> Self {
        Self {
            profile: MediaFingerprintProfile::CombinedV3,
            frame_sample_interval_seconds: 10,
            max_frames: 64,
            audio_sample_seconds: 0,
            audio_algorithm: "sorotte-audio-constellation-v3".to_owned(),
            video_algorithm: "sorotte-video-scene-v3".to_owned(),
        }
    }
}
