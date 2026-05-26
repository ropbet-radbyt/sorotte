use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const FRAME_HASH_BITS: u32 = 64;
pub(crate) const DEFAULT_FRAME_HAMMING_THRESHOLD: u32 = 10;
pub(crate) const DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS: i64 = 1_000;
pub(crate) const DEFAULT_ANCHOR_OFFSET_BIN_MS: i64 = 1_000;

// V3 piecewise timeline fitting thresholds. These are deliberately conservative:
// segments need enough local evidence to map time, while gaps remain explicit.
pub(crate) const V3_SEGMENT_MIN_PAIR_DELTA_MS: u32 = 30_000;
pub(crate) const V3_SEGMENT_SPLIT_GAP_MS: u32 = 75_000;
pub(crate) const V3_SEGMENT_AUDIO_MIN_PAIRS: usize = 6;
pub(crate) const V3_SEGMENT_AUDIO_MIN_SPAN_MS: u32 = 60_000;
pub(crate) const V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS: usize = 3;
pub(crate) const V3_SEGMENT_AUDIO_VIDEO_MIN_SPAN_MS: u32 = 45_000;
pub(crate) const V3_SEGMENT_VIDEO_MIN_PAIRS: usize = 5;
pub(crate) const V3_SEGMENT_VIDEO_MIN_SPAN_MS: u32 = 60_000;
pub(crate) const V3_SEGMENT_MERGE_GAP_MS: u32 = 45_000;
pub(crate) const V3_SEGMENT_MERGE_SCALE_PPM: i32 = 2_500;
pub(crate) const V3_EDGE_REGION_MIN_MS: u32 = 120_000;
pub(crate) const V3_EDGE_REGION_MAX_MS: u32 = 300_000;
pub(crate) const V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS: usize = 512;
pub(crate) const MAX_BROAD_SCALE_FIT_PAIRS: usize = 128;

// V3 native audio constellation extraction thresholds.
pub(crate) const V3_AUDIO_SAMPLE_RATE: u32 = 11_025;
pub(crate) const V3_AUDIO_WINDOW_SAMPLES: usize = 2048;
pub(crate) const V3_AUDIO_HOP_SAMPLES: usize = 512;
pub(crate) const V3_AUDIO_MIN_FREQ_HZ: f32 = 250.0;
pub(crate) const V3_AUDIO_MAX_FREQ_HZ: f32 = 5_000.0;
pub(crate) const V3_AUDIO_MAX_PEAKS_PER_FRAME: usize = 6;
pub(crate) const V3_AUDIO_PEAK_NEIGHBORHOOD: usize = 2;
pub(crate) const V3_AUDIO_PAIR_MIN_DELTA_FRAMES: usize = 8;
pub(crate) const V3_AUDIO_PAIR_MAX_DELTA_FRAMES: usize = 108;
pub(crate) const V3_AUDIO_PAIR_FANOUT: usize = 8;
pub(crate) const V3_AUDIO_VERIFY_LANDMARK_LIMIT: usize = 768;
pub(crate) const V3_AUDIO_INDEX_LANDMARK_LIMIT: usize = 192;
// Streaming audio keeps only a winnowed raw landmark buffer; this bounds noisy/long files
// while preserving enough oversampling for the final time-distributed selector.
pub(crate) const V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 8;
pub(crate) const V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 4;

// V3 video retrieval and descriptor thresholds.
pub(crate) const VIDEO_LSH_BANDS: u32 = 4;
pub(crate) const VIDEO_LSH_BITS_PER_BAND: u32 = 16;
pub(crate) const V3_VIDEO_BUCKET_KIND_SHIFT: u32 = 28;
pub(crate) const V3_VIDEO_BUCKET_VALUE_MASK: u32 = 0x0fff_ffff;
pub(crate) const V3_VIDEO_VERIFY_LANDMARK_LIMIT: usize = 192;
pub(crate) const V3_VIDEO_INDEX_LANDMARK_LIMIT: usize = 64;
pub(crate) const V3_VIDEO_PHASH_SIZE: usize = 32;
pub(crate) const V3_VIDEO_PHASH_LOW_FREQ: usize = 8;
pub(crate) const V3_VIDEO_MIN_VARIANCE: f64 = 6.0;
pub(crate) const V3_VIDEO_TEMPORAL_MIN_DELTA_MS: u32 = 5_000;
pub(crate) const V3_VIDEO_TEMPORAL_MAX_DELTA_MS: u32 = 60_000;
pub(crate) const V3_VIDEO_TEMPORAL_DELTA_BUCKET_MS: u32 = 5_000;
pub(crate) const V3_VIDEO_TEMPORAL_FANOUT: usize = 2;
pub(crate) const V3_VIDEO_HAMMING_GLOBAL: u32 = 10;
pub(crate) const V3_VIDEO_HAMMING_CENTER: u32 = 10;
pub(crate) const V3_VIDEO_HAMMING_EDGE: u32 = 12;
pub(crate) const V3_VIDEO_HAMMING_TEMPORAL: u32 = 0;

pub(crate) const VIDEO_FRAME_WIDTH: usize = 32;
pub(crate) const VIDEO_FRAME_HEIGHT: usize = 32;
pub(crate) const VIDEO_FRAME_BYTES: usize = VIDEO_FRAME_WIDTH * VIDEO_FRAME_HEIGHT;

pub(crate) const MEDIA_TOOL_POLL_INTERVAL: Duration = Duration::from_millis(25);
pub(crate) const FFPROBE_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const FFMPEG_AUDIO_V3_TIMEOUT: Duration = Duration::from_secs(180);
pub(crate) const FFMPEG_FULL_VIDEO_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V3Tuning {
    pub segment_min_pair_delta_ms: u32,
    pub segment_split_gap_ms: u32,
    pub segment_audio_min_pairs: usize,
    pub segment_video_min_pairs: usize,
    pub piecewise_max_hypothesis_pairs: usize,
    pub audio_raw_landmark_buffer_limit: usize,
    pub audio_raw_landmark_retain_limit: usize,
    pub video_hamming_global: u32,
    pub video_hamming_center: u32,
    pub video_hamming_edge: u32,
    pub video_hamming_temporal: u32,
}

pub fn current_v3_tuning() -> V3Tuning {
    V3Tuning {
        segment_min_pair_delta_ms: V3_SEGMENT_MIN_PAIR_DELTA_MS,
        segment_split_gap_ms: V3_SEGMENT_SPLIT_GAP_MS,
        segment_audio_min_pairs: V3_SEGMENT_AUDIO_MIN_PAIRS,
        segment_video_min_pairs: V3_SEGMENT_VIDEO_MIN_PAIRS,
        piecewise_max_hypothesis_pairs: V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS,
        audio_raw_landmark_buffer_limit: V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        audio_raw_landmark_retain_limit: V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
        video_hamming_global: V3_VIDEO_HAMMING_GLOBAL,
        video_hamming_center: V3_VIDEO_HAMMING_CENTER,
        video_hamming_edge: V3_VIDEO_HAMMING_EDGE,
        video_hamming_temporal: V3_VIDEO_HAMMING_TEMPORAL,
    }
}
