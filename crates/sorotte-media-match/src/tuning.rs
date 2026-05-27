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
pub(crate) const V3_PIECEWISE_MAX_HYPOTHESES: usize = 2_000;
pub(crate) const MAX_BROAD_SCALE_FIT_PAIRS: usize = 128;
pub(crate) const V3_FAST_AUDIO_TOP_OFFSET_BINS: usize = 6;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_PAIRS: usize = 24;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_REGIONS: usize = 4;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_SPAN_MS: u32 = 300_000;

// V3 native audio constellation extraction thresholds.
pub(crate) const V3_AUDIO_SAMPLE_RATE: u32 = 11_025;
pub(crate) const V3_AUDIO_WINDOW_SAMPLES: usize = 2048;
pub(crate) const V3_AUDIO_HOP_SAMPLES: usize = 1024;
pub(crate) const V3_AUDIO_MIN_FREQ_HZ: f32 = 250.0;
pub(crate) const V3_AUDIO_MAX_FREQ_HZ: f32 = 5_000.0;
pub(crate) const V3_AUDIO_MAX_PEAKS_PER_FRAME: usize = 6;
pub(crate) const V3_AUDIO_PEAK_NEIGHBORHOOD: usize = 2;
pub(crate) const V3_AUDIO_PAIR_MIN_DELTA_FRAMES: usize = 4;
pub(crate) const V3_AUDIO_PAIR_MAX_DELTA_FRAMES: usize = 54;
pub(crate) const V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES: usize = 2;
pub(crate) const V3_AUDIO_PAIR_FANOUT: usize = 8;
pub(crate) const V3_AUDIO_PAIR_CANDIDATE_RETAIN: usize = V3_AUDIO_PAIR_FANOUT * 4;
pub(crate) const V3_AUDIO_VERIFY_LANDMARK_LIMIT: usize = 2048;
pub(crate) const V3_AUDIO_INDEX_LANDMARK_LIMIT: usize = 512;
pub(crate) const V3_AUDIO_SAMPLED_INDEX_LANDMARK_LIMIT: usize = 512;
pub(crate) const V3_AUDIO_SAMPLED_INDEX_WINDOW_SECONDS: u32 = 30;
pub(crate) const V3_AUDIO_SAMPLED_INDEX_WINDOW_COUNT: usize = 5;
// Streaming audio keeps only a winnowed raw landmark buffer; this bounds noisy/long files
// while preserving enough oversampling for the final time-distributed selector.
pub(crate) const V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 8;
pub(crate) const V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT: usize = V3_AUDIO_VERIFY_LANDMARK_LIMIT * 4;
pub(crate) const V3_AUDIO_RAW_REGION_RETAIN_LIMIT: usize = 512;
pub(crate) const V3_AUDIO_RAW_REGION_TRIM_BURST: usize = 128;

// V3 video retrieval and descriptor thresholds.
pub(crate) const VIDEO_LSH_BANDS: u32 = 4;
pub(crate) const VIDEO_LSH_BITS_PER_BAND: u32 = 16;
pub(crate) const V3_RETRIEVAL_PREFILTER_LIMIT: usize = 24;
pub(crate) const V3_RETRIEVAL_OFFSET_BIN_MS: i64 = 1_000;
pub(crate) const V3_RETRIEVAL_REGION_MS: i64 = 60_000;
pub(crate) const V3_RETRIEVAL_GAP_MS: i64 = 120_000;
pub(crate) const V3_COMMON_BUCKET_MIN_SKIP_DF: i64 = 256;
pub(crate) const V3_COMMON_BUCKET_FILE_DIVISOR: i64 = 20;
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
    pub piecewise_max_hypotheses: usize,
    pub fast_audio_top_offset_bins: usize,
    pub fast_audio_min_body_pairs: usize,
    pub fast_audio_min_body_regions: usize,
    pub fast_audio_min_body_span_ms: u32,
    pub audio_hop_samples: usize,
    pub audio_pair_min_delta_frames: usize,
    pub audio_pair_max_delta_frames: usize,
    pub audio_pair_delta_stride_frames: usize,
    pub audio_pair_candidate_retain: usize,
    pub audio_verify_landmark_limit: usize,
    pub audio_index_landmark_limit: usize,
    pub audio_sampled_index_landmark_limit: usize,
    pub audio_sampled_index_window_seconds: u32,
    pub audio_sampled_index_window_count: usize,
    pub audio_raw_landmark_buffer_limit: usize,
    pub audio_raw_landmark_retain_limit: usize,
    pub audio_raw_region_retain_limit: usize,
    pub video_verify_landmark_limit: usize,
    pub video_index_landmark_limit: usize,
    pub retrieval_prefilter_limit: usize,
    pub retrieval_offset_bin_ms: i64,
    pub retrieval_region_ms: i64,
    pub retrieval_gap_ms: i64,
    pub common_bucket_min_skip_df: i64,
    pub common_bucket_file_divisor: i64,
    pub video_lsh_bands: u32,
    pub video_lsh_bits_per_band: u32,
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
        piecewise_max_hypotheses: V3_PIECEWISE_MAX_HYPOTHESES,
        fast_audio_top_offset_bins: V3_FAST_AUDIO_TOP_OFFSET_BINS,
        fast_audio_min_body_pairs: V3_FAST_AUDIO_MIN_BODY_PAIRS,
        fast_audio_min_body_regions: V3_FAST_AUDIO_MIN_BODY_REGIONS,
        fast_audio_min_body_span_ms: V3_FAST_AUDIO_MIN_BODY_SPAN_MS,
        audio_hop_samples: V3_AUDIO_HOP_SAMPLES,
        audio_pair_min_delta_frames: V3_AUDIO_PAIR_MIN_DELTA_FRAMES,
        audio_pair_max_delta_frames: V3_AUDIO_PAIR_MAX_DELTA_FRAMES,
        audio_pair_delta_stride_frames: V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES,
        audio_pair_candidate_retain: V3_AUDIO_PAIR_CANDIDATE_RETAIN,
        audio_verify_landmark_limit: V3_AUDIO_VERIFY_LANDMARK_LIMIT,
        audio_index_landmark_limit: V3_AUDIO_INDEX_LANDMARK_LIMIT,
        audio_sampled_index_landmark_limit: V3_AUDIO_SAMPLED_INDEX_LANDMARK_LIMIT,
        audio_sampled_index_window_seconds: V3_AUDIO_SAMPLED_INDEX_WINDOW_SECONDS,
        audio_sampled_index_window_count: V3_AUDIO_SAMPLED_INDEX_WINDOW_COUNT,
        audio_raw_landmark_buffer_limit: V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        audio_raw_landmark_retain_limit: V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
        audio_raw_region_retain_limit: V3_AUDIO_RAW_REGION_RETAIN_LIMIT,
        video_verify_landmark_limit: V3_VIDEO_VERIFY_LANDMARK_LIMIT,
        video_index_landmark_limit: V3_VIDEO_INDEX_LANDMARK_LIMIT,
        retrieval_prefilter_limit: V3_RETRIEVAL_PREFILTER_LIMIT,
        retrieval_offset_bin_ms: V3_RETRIEVAL_OFFSET_BIN_MS,
        retrieval_region_ms: V3_RETRIEVAL_REGION_MS,
        retrieval_gap_ms: V3_RETRIEVAL_GAP_MS,
        common_bucket_min_skip_df: V3_COMMON_BUCKET_MIN_SKIP_DF,
        common_bucket_file_divisor: V3_COMMON_BUCKET_FILE_DIVISOR,
        video_lsh_bands: VIDEO_LSH_BANDS,
        video_lsh_bits_per_band: VIDEO_LSH_BITS_PER_BAND,
        video_hamming_global: V3_VIDEO_HAMMING_GLOBAL,
        video_hamming_center: V3_VIDEO_HAMMING_CENTER,
        video_hamming_edge: V3_VIDEO_HAMMING_EDGE,
        video_hamming_temporal: V3_VIDEO_HAMMING_TEMPORAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_tuning_serializes_stable_calibration_fields() {
        let value = serde_json::to_value(current_v3_tuning()).expect("V3 tuning should serialize");

        assert_eq!(
            value["audioVerifyLandmarkLimit"].as_u64(),
            Some(V3_AUDIO_VERIFY_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["audioIndexLandmarkLimit"].as_u64(),
            Some(V3_AUDIO_INDEX_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["audioSampledIndexLandmarkLimit"].as_u64(),
            Some(V3_AUDIO_SAMPLED_INDEX_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["audioSampledIndexWindowSeconds"].as_u64(),
            Some(u64::from(V3_AUDIO_SAMPLED_INDEX_WINDOW_SECONDS))
        );
        assert_eq!(
            value["audioSampledIndexWindowCount"].as_u64(),
            Some(V3_AUDIO_SAMPLED_INDEX_WINDOW_COUNT as u64)
        );
        assert_eq!(
            value["audioRawRegionRetainLimit"].as_u64(),
            Some(V3_AUDIO_RAW_REGION_RETAIN_LIMIT as u64)
        );
        assert_eq!(
            value["videoVerifyLandmarkLimit"].as_u64(),
            Some(V3_VIDEO_VERIFY_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["videoIndexLandmarkLimit"].as_u64(),
            Some(V3_VIDEO_INDEX_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["retrievalPrefilterLimit"].as_u64(),
            Some(V3_RETRIEVAL_PREFILTER_LIMIT as u64)
        );
        assert_eq!(
            value["retrievalOffsetBinMs"].as_i64(),
            Some(V3_RETRIEVAL_OFFSET_BIN_MS)
        );
        assert_eq!(
            value["retrievalRegionMs"].as_i64(),
            Some(V3_RETRIEVAL_REGION_MS)
        );
        assert_eq!(value["retrievalGapMs"].as_i64(), Some(V3_RETRIEVAL_GAP_MS));
        assert_eq!(
            value["commonBucketMinSkipDf"].as_i64(),
            Some(V3_COMMON_BUCKET_MIN_SKIP_DF)
        );
        assert_eq!(
            value["commonBucketFileDivisor"].as_i64(),
            Some(V3_COMMON_BUCKET_FILE_DIVISOR)
        );
        assert_eq!(
            value["videoLshBands"].as_u64(),
            Some(u64::from(VIDEO_LSH_BANDS))
        );
        assert_eq!(
            value["videoLshBitsPerBand"].as_u64(),
            Some(u64::from(VIDEO_LSH_BITS_PER_BAND))
        );
    }
}
