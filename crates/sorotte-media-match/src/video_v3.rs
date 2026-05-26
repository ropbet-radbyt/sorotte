use serde::{Deserialize, Serialize};

pub const V3_VIDEO_KIND_LUMA_FRAME: u8 = 0;
pub const V3_VIDEO_KIND_GLOBAL_DCT: u8 = 1;
pub const V3_VIDEO_KIND_CENTER_DCT: u8 = 2;
pub const V3_VIDEO_KIND_EDGE: u8 = 3;
pub const V3_VIDEO_KIND_TEMPORAL_SHINGLE: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFingerprint {
    pub duration_seconds: Option<u32>,
    pub frames: Vec<FrameFingerprint>,
    #[serde(default)]
    pub v3_landmarks: Vec<VideoLandmarkV3>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameFingerprint {
    pub timestamp_millis: u64,
    pub hash: u64,
}

impl FrameFingerprint {
    pub fn new(timestamp_seconds: f64, hash: u64) -> Self {
        let timestamp_millis = if timestamp_seconds.is_finite() && timestamp_seconds > 0.0 {
            (timestamp_seconds * 1000.0).round() as u64
        } else {
            0
        };
        Self {
            timestamp_millis,
            hash,
        }
    }

    pub fn timestamp_seconds(self) -> f64 {
        self.timestamp_millis as f64 / 1000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LumaRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoLandmarkV3 {
    pub bucket: u32,
    pub hash64: u64,
    pub t_ms: u32,
    pub kind: u8,
    pub weight: u8,
}
