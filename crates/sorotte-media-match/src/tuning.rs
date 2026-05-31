use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_ANCHOR_OFFSET_BIN_MS: i64 = 1_000;

pub(crate) const V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS: usize = 512;
pub(crate) const V3_PIECEWISE_MAX_HYPOTHESES: usize = 2_000;
pub(crate) const V3_FAST_AUDIO_TOP_OFFSET_BINS: usize = 6;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_PAIRS: usize = 24;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_REGIONS: usize = 4;
pub(crate) const V3_FAST_AUDIO_MIN_BODY_SPAN_MS: u32 = 300_000;

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
pub(crate) const V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT: usize = 384;
pub(crate) const V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS: usize = 320;
pub(crate) const V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS: u32 = 20;
pub(crate) const V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS: usize = 3;
pub(crate) const V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS: usize = 3;
pub(crate) const V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE: u32 = 8_000;
pub(crate) const V3_AUDIO_SAMPLED_MIN_BODY_REGIONS: usize = 4;
pub(crate) const V3_AUDIO_RAW_REGION_RETAIN_LIMIT: usize = 256;

pub(crate) const V3_RETRIEVAL_PREFILTER_LIMIT: usize = 24;
pub(crate) const V3_RETRIEVAL_OFFSET_BIN_MS: i64 = 1_000;
pub(crate) const V3_RETRIEVAL_REGION_MS: i64 = 60_000;
pub(crate) const V3_RETRIEVAL_GAP_MS: i64 = 120_000;
pub(crate) const V3_COMMON_BUCKET_MIN_SKIP_DF: i64 = 256;
pub(crate) const V3_COMMON_BUCKET_FILE_DIVISOR: i64 = 20;

pub(crate) const MEDIA_TOOL_POLL_INTERVAL: Duration = Duration::from_millis(25);
pub(crate) const FFPROBE_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const FFMPEG_AUDIO_V3_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V3Tuning {
    pub piecewise_max_hypothesis_pairs: usize,
    pub piecewise_max_hypotheses: usize,
    pub fast_audio_top_offset_bins: usize,
    pub fast_audio_min_body_pairs: usize,
    pub fast_audio_min_body_regions: usize,
    pub fast_audio_min_body_span_ms: u32,
    pub audio_sample_rate: u32,
    pub audio_window_samples: usize,
    pub audio_hop_samples: usize,
    pub audio_max_peaks_per_frame: usize,
    pub audio_pair_min_delta_frames: usize,
    pub audio_pair_max_delta_frames: usize,
    pub audio_pair_delta_stride_frames: usize,
    pub audio_pair_candidate_retain: usize,
    pub audio_verify_landmark_limit: usize,
    pub audio_index_landmark_limit: usize,
    pub audio_sampled_fast_index_landmark_limit: usize,
    pub audio_sampled_fast_target_landmarks: usize,
    pub audio_sampled_fast_window_seconds: u32,
    pub audio_sampled_fast_min_windows: usize,
    pub audio_sampled_fast_max_windows: usize,
    pub audio_sampled_fast_sample_rate: u32,
    pub audio_sampled_min_body_regions: usize,
    pub audio_raw_region_retain_limit: usize,
    pub retrieval_prefilter_limit: usize,
    pub retrieval_offset_bin_ms: i64,
    pub retrieval_region_ms: i64,
    pub retrieval_gap_ms: i64,
    pub common_bucket_min_skip_df: i64,
    pub common_bucket_file_divisor: i64,
}

pub fn current_v3_tuning() -> V3Tuning {
    V3Tuning {
        piecewise_max_hypothesis_pairs: V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS,
        piecewise_max_hypotheses: V3_PIECEWISE_MAX_HYPOTHESES,
        fast_audio_top_offset_bins: V3_FAST_AUDIO_TOP_OFFSET_BINS,
        fast_audio_min_body_pairs: V3_FAST_AUDIO_MIN_BODY_PAIRS,
        fast_audio_min_body_regions: V3_FAST_AUDIO_MIN_BODY_REGIONS,
        fast_audio_min_body_span_ms: V3_FAST_AUDIO_MIN_BODY_SPAN_MS,
        audio_sample_rate: V3_AUDIO_SAMPLE_RATE,
        audio_window_samples: V3_AUDIO_WINDOW_SAMPLES,
        audio_hop_samples: V3_AUDIO_HOP_SAMPLES,
        audio_max_peaks_per_frame: V3_AUDIO_MAX_PEAKS_PER_FRAME,
        audio_pair_min_delta_frames: V3_AUDIO_PAIR_MIN_DELTA_FRAMES,
        audio_pair_max_delta_frames: V3_AUDIO_PAIR_MAX_DELTA_FRAMES,
        audio_pair_delta_stride_frames: V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES,
        audio_pair_candidate_retain: V3_AUDIO_PAIR_CANDIDATE_RETAIN,
        audio_verify_landmark_limit: V3_AUDIO_VERIFY_LANDMARK_LIMIT,
        audio_index_landmark_limit: V3_AUDIO_INDEX_LANDMARK_LIMIT,
        audio_sampled_fast_index_landmark_limit: V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
        audio_sampled_fast_target_landmarks: V3_AUDIO_SAMPLED_FAST_TARGET_LANDMARKS,
        audio_sampled_fast_window_seconds: V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS,
        audio_sampled_fast_min_windows: V3_AUDIO_SAMPLED_FAST_MIN_WINDOWS,
        audio_sampled_fast_max_windows: V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS,
        audio_sampled_fast_sample_rate: V3_AUDIO_SAMPLED_FAST_SAMPLE_RATE,
        audio_sampled_min_body_regions: V3_AUDIO_SAMPLED_MIN_BODY_REGIONS,
        audio_raw_region_retain_limit: V3_AUDIO_RAW_REGION_RETAIN_LIMIT,
        retrieval_prefilter_limit: V3_RETRIEVAL_PREFILTER_LIMIT,
        retrieval_offset_bin_ms: V3_RETRIEVAL_OFFSET_BIN_MS,
        retrieval_region_ms: V3_RETRIEVAL_REGION_MS,
        retrieval_gap_ms: V3_RETRIEVAL_GAP_MS,
        common_bucket_min_skip_df: V3_COMMON_BUCKET_MIN_SKIP_DF,
        common_bucket_file_divisor: V3_COMMON_BUCKET_FILE_DIVISOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_tuning_serializes_fixed_sampled_fast_fields() {
        let value = serde_json::to_value(current_v3_tuning()).expect("V3 tuning should serialize");

        assert_eq!(
            value["audioSampledFastIndexLandmarkLimit"].as_u64(),
            Some(V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT as u64)
        );
        assert_eq!(
            value["audioSampledFastWindowSeconds"].as_u64(),
            Some(u64::from(V3_AUDIO_SAMPLED_FAST_WINDOW_SECONDS))
        );
        assert_eq!(
            value["audioSampledFastMaxWindows"].as_u64(),
            Some(V3_AUDIO_SAMPLED_FAST_MAX_WINDOWS as u64)
        );
        assert_eq!(
            value["retrievalPrefilterLimit"].as_u64(),
            Some(V3_RETRIEVAL_PREFILTER_LIMIT as u64)
        );
        assert_eq!(value.as_object().map(|object| object.len()), Some(30));
    }
}
