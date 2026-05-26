use serde::{Deserialize, Serialize};

use crate::{FAST_AUDIO_SAMPLE_SECONDS, FAST_VIDEO_SAMPLE_FRAMES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaFingerprintProfile {
    FastAnchorV2,
    FullAnchorV2,
    AudioConstellationV3,
    CombinedV3,
}

impl MediaFingerprintProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FastAnchorV2 => "fast-anchor-v2",
            Self::FullAnchorV2 => "full-anchor-v2",
            Self::AudioConstellationV3 => "audio-constellation-v3",
            Self::CombinedV3 => "combined-v3",
        }
    }

    pub fn is_fast(self) -> bool {
        matches!(self, Self::FastAnchorV2 | Self::AudioConstellationV3)
    }

    pub fn is_v3(self) -> bool {
        matches!(self, Self::AudioConstellationV3 | Self::CombinedV3)
    }

    pub fn uses_v3_audio_constellation(self) -> bool {
        self.is_v3()
    }

    pub fn uses_video_by_default(self) -> bool {
        matches!(
            self,
            Self::FastAnchorV2 | Self::FullAnchorV2 | Self::CombinedV3
        )
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
    pub fn fast_anchor_v2() -> Self {
        Self {
            profile: MediaFingerprintProfile::FastAnchorV2,
            frame_sample_interval_seconds: 0,
            max_frames: FAST_VIDEO_SAMPLE_FRAMES,
            audio_sample_seconds: FAST_AUDIO_SAMPLE_SECONDS,
            audio_algorithm: format!("chromaprint-anchor-v2-{FAST_AUDIO_SAMPLE_SECONDS}s"),
            video_algorithm: "sorotte-luma-anchor-v2-fast".to_owned(),
        }
    }

    pub fn full_anchor_v2() -> Self {
        Self {
            profile: MediaFingerprintProfile::FullAnchorV2,
            frame_sample_interval_seconds: 10,
            max_frames: 720,
            audio_sample_seconds: 0,
            audio_algorithm: "chromaprint-anchor-v2-full".to_owned(),
            video_algorithm: "sorotte-luma-anchor-v2-full".to_owned(),
        }
    }

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
