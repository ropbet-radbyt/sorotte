use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    DEFAULT_FRAME_HAMMING_THRESHOLD, FRAME_HASH_BITS, V3_VIDEO_BUCKET_KIND_SHIFT,
    V3_VIDEO_BUCKET_VALUE_MASK, V3_VIDEO_INDEX_LANDMARK_LIMIT, V3_VIDEO_MIN_VARIANCE,
    V3_VIDEO_PHASH_LOW_FREQ, V3_VIDEO_PHASH_SIZE, V3_VIDEO_TEMPORAL_DELTA_BUCKET_MS,
    V3_VIDEO_TEMPORAL_FANOUT, V3_VIDEO_TEMPORAL_MAX_DELTA_MS, V3_VIDEO_TEMPORAL_MIN_DELTA_MS,
    V3_VIDEO_VERIFY_LANDMARK_LIMIT, VIDEO_LSH_BANDS, VIDEO_LSH_BITS_PER_BAND, VideoAnchor,
    current_v3_tuning,
};

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

pub(crate) fn bounded_time_distributed_video_anchors(
    anchors: &mut [VideoAnchor],
    max_anchors: usize,
) -> Vec<VideoAnchor> {
    anchors.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.hash64, anchor.kind));
    if anchors.len() <= max_anchors {
        return anchors.to_vec();
    }
    let stride = anchors.len() as f64 / max_anchors as f64;
    (0..max_anchors)
        .map(|index| anchors[(index as f64 * stride).floor() as usize])
        .collect()
}

pub(crate) fn bounded_time_distributed_video_landmarks_v3(
    landmarks: &mut [VideoLandmarkV3],
    max_landmarks: usize,
) -> Vec<VideoLandmarkV3> {
    if max_landmarks == 0 {
        return Vec::new();
    }
    let mut valid = landmarks
        .iter()
        .copied()
        .filter(|landmark| {
            v3_video_kind_is_supported(landmark.kind)
                && v3_video_bucket_kind_matches(landmark.kind, landmark.bucket)
        })
        .collect::<Vec<_>>();
    sort_video_landmarks_for_bounding(&mut valid);
    if valid.len() <= max_landmarks {
        return valid;
    }

    let index_profile = max_landmarks <= V3_VIDEO_INDEX_LANDMARK_LIMIT;
    let kind_order = [
        V3_VIDEO_KIND_TEMPORAL_SHINGLE,
        V3_VIDEO_KIND_GLOBAL_DCT,
        V3_VIDEO_KIND_CENTER_DCT,
        V3_VIDEO_KIND_EDGE,
        V3_VIDEO_KIND_LUMA_FRAME,
    ];
    let mut selected = Vec::with_capacity(max_landmarks);
    let mut seen = HashSet::new();

    for kind in kind_order {
        let candidates = valid
            .iter()
            .copied()
            .filter(|landmark| landmark.kind == kind)
            .collect::<Vec<_>>();
        if candidates.is_empty() || selected.len() >= max_landmarks {
            continue;
        }
        let quota = v3_video_kind_quota(max_landmarks, kind, index_profile)
            .max(usize::from(kind != V3_VIDEO_KIND_LUMA_FRAME))
            .min(max_landmarks - selected.len())
            .min(candidates.len());
        for landmark in select_time_distributed_video_landmarks_v3(&candidates, quota) {
            if seen.insert(video_landmark_key(&landmark)) {
                selected.push(landmark);
            }
        }
    }

    while selected.len() < max_landmarks {
        let mut progressed = false;
        for kind in kind_order {
            if selected.len() >= max_landmarks {
                break;
            }
            let candidates = valid
                .iter()
                .copied()
                .filter(|landmark| {
                    landmark.kind == kind && !seen.contains(&video_landmark_key(landmark))
                })
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                continue;
            }
            let Some(landmark) = select_time_distributed_video_landmarks_v3(&candidates, 1)
                .into_iter()
                .next()
            else {
                continue;
            };
            seen.insert(video_landmark_key(&landmark));
            selected.push(landmark);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    sort_video_landmarks_for_bounding(&mut selected);
    selected.truncate(max_landmarks);
    selected
}

fn v3_video_kind_quota(max_landmarks: usize, kind: u8, index_profile: bool) -> usize {
    let percent = if index_profile {
        match kind {
            V3_VIDEO_KIND_TEMPORAL_SHINGLE => 50,
            V3_VIDEO_KIND_GLOBAL_DCT => 17,
            V3_VIDEO_KIND_CENTER_DCT => 17,
            V3_VIDEO_KIND_EDGE => 16,
            _ => 0,
        }
    } else {
        match kind {
            V3_VIDEO_KIND_TEMPORAL_SHINGLE => 40,
            V3_VIDEO_KIND_GLOBAL_DCT => 25,
            V3_VIDEO_KIND_CENTER_DCT => 20,
            V3_VIDEO_KIND_EDGE => 15,
            _ => 0,
        }
    };
    (max_landmarks * percent) / 100
}

fn select_time_distributed_video_landmarks_v3(
    landmarks: &[VideoLandmarkV3],
    limit: usize,
) -> Vec<VideoLandmarkV3> {
    if limit == 0 || landmarks.is_empty() {
        return Vec::new();
    }
    let mut sorted = landmarks.to_vec();
    sort_video_landmarks_for_bounding(&mut sorted);
    if sorted.len() <= limit {
        return sorted;
    }
    let stride = sorted.len() as f64 / limit as f64;
    (0..limit)
        .map(|index| sorted[(index as f64 * stride).floor() as usize])
        .collect()
}

fn sort_video_landmarks_for_bounding(landmarks: &mut [VideoLandmarkV3]) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            video_kind_bounding_priority(landmark.kind),
            landmark.bucket,
            landmark.hash64,
            std::cmp::Reverse(landmark.weight),
        )
    });
}

fn video_kind_bounding_priority(kind: u8) -> u8 {
    match kind {
        V3_VIDEO_KIND_TEMPORAL_SHINGLE => 0,
        V3_VIDEO_KIND_GLOBAL_DCT => 1,
        V3_VIDEO_KIND_CENTER_DCT => 2,
        V3_VIDEO_KIND_EDGE => 3,
        V3_VIDEO_KIND_LUMA_FRAME => 4,
        _ => u8::MAX,
    }
}

fn video_landmark_key(landmark: &VideoLandmarkV3) -> (u8, u32, u64, u32) {
    (
        landmark.kind,
        landmark.bucket,
        landmark.hash64,
        landmark.t_ms,
    )
}

pub(crate) fn stable_hash_u64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hasher = Sha256::new();
    for byte in bytes {
        hasher.update([byte]);
    }
    let digest = hasher.finalize();
    u64::from_le_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

pub(crate) fn anchor_bucket(hash: u64) -> u32 {
    (hash >> 32) as u32
}

pub(crate) fn video_lsh_buckets(hash: u64) -> [u32; VIDEO_LSH_BANDS as usize] {
    let mask = (1u64 << VIDEO_LSH_BITS_PER_BAND) - 1;
    let mut buckets = [0u32; VIDEO_LSH_BANDS as usize];
    for band in 0..VIDEO_LSH_BANDS {
        let shift = band * VIDEO_LSH_BITS_PER_BAND;
        let band_bits = ((hash >> shift) & mask) as u32;
        buckets[band as usize] = (band << VIDEO_LSH_BITS_PER_BAND) | band_bits;
    }
    buckets
}

pub fn video_anchor_hashes_match(left: u64, right: u64) -> bool {
    frame_hash_distance(left, right) <= DEFAULT_FRAME_HAMMING_THRESHOLD
}

pub fn v3_video_bucket_for_kind(kind: u8, raw_bucket: u32) -> u32 {
    ((u32::from(kind) & 0x0f) << V3_VIDEO_BUCKET_KIND_SHIFT)
        | (raw_bucket & V3_VIDEO_BUCKET_VALUE_MASK)
}

pub fn v3_video_kind_is_supported(kind: u8) -> bool {
    matches!(
        kind,
        V3_VIDEO_KIND_LUMA_FRAME
            | V3_VIDEO_KIND_GLOBAL_DCT
            | V3_VIDEO_KIND_CENTER_DCT
            | V3_VIDEO_KIND_EDGE
            | V3_VIDEO_KIND_TEMPORAL_SHINGLE
    )
}

pub fn v3_video_kind_from_bucket(bucket: u32) -> Option<u8> {
    let kind = (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8;
    v3_video_kind_is_supported(kind).then_some(kind)
}

pub fn v3_video_bucket_kind_matches(kind: u8, bucket: u32) -> bool {
    v3_video_kind_from_bucket(bucket).is_some_and(|bucket_kind| bucket_kind == kind)
}

pub fn validate_video_landmark_v3(landmark: &VideoLandmarkV3) -> Result<(), String> {
    if !v3_video_kind_is_supported(landmark.kind) {
        return Err(format!(
            "unsupported V3 video landmark kind {}",
            landmark.kind
        ));
    }
    let Some(bucket_kind) = v3_video_kind_from_bucket(landmark.bucket) else {
        return Err(format!(
            "unsupported V3 video landmark bucket kind {}",
            (landmark.bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8
        ));
    };
    if bucket_kind != landmark.kind {
        return Err(format!(
            "V3 video landmark kind {} does not match bucket kind {}",
            landmark.kind, bucket_kind
        ));
    }
    if landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
        let expected = v3_video_bucket_for_kind(landmark.kind, anchor_bucket(landmark.hash64));
        if landmark.bucket != expected {
            return Err(format!(
                "V3 temporal shingle bucket {} does not match exact hash bucket {}",
                landmark.bucket, expected
            ));
        }
    }
    Ok(())
}

pub fn validate_video_landmarks_v3(landmarks: &[VideoLandmarkV3]) -> Result<(), String> {
    for landmark in landmarks {
        validate_video_landmark_v3(landmark)?;
    }
    Ok(())
}

pub(crate) fn v3_video_lsh_buckets(kind: u8, hash: u64) -> Vec<u32> {
    if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
        return vec![v3_video_bucket_for_kind(kind, anchor_bucket(hash))];
    }
    video_lsh_buckets(hash)
        .into_iter()
        .map(|bucket| v3_video_bucket_for_kind(kind, bucket))
        .collect()
}

pub fn v3_video_hamming_threshold(kind: u8) -> u32 {
    let tuning = current_v3_tuning();
    match kind {
        V3_VIDEO_KIND_GLOBAL_DCT => tuning.video_hamming_global,
        V3_VIDEO_KIND_CENTER_DCT => tuning.video_hamming_center,
        V3_VIDEO_KIND_EDGE => tuning.video_hamming_edge,
        V3_VIDEO_KIND_TEMPORAL_SHINGLE => tuning.video_hamming_temporal,
        _ => DEFAULT_FRAME_HAMMING_THRESHOLD,
    }
}

pub(crate) fn v3_video_anchor_hashes_match(kind: u8, left: u64, right: u64) -> bool {
    frame_hash_distance(left, right) <= v3_video_hamming_threshold(kind)
}

pub fn detect_content_window_luma(width: usize, height: usize, luma: &[u8]) -> Option<LumaRect> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let full = LumaRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let row_is_content = |y: usize| {
        let start = y * width;
        !luma_slice_is_black(&luma[start..start + width])
    };
    let top = (0..height).find(|y| row_is_content(*y));
    let bottom = (0..height).rev().find(|y| row_is_content(*y));
    let (Some(top), Some(bottom)) = (top, bottom) else {
        return Some(full);
    };
    let column_is_content = |x: usize| {
        let mut values = Vec::with_capacity(bottom - top + 1);
        for y in top..=bottom {
            values.push(luma[y * width + x]);
        }
        !luma_slice_is_black(&values)
    };
    let left = (0..width).find(|x| column_is_content(*x));
    let right = (0..width).rev().find(|x| column_is_content(*x));
    let (Some(left), Some(right)) = (left, right) else {
        return Some(full);
    };
    let rect = LumaRect {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    };
    let uncertain = rect.width < width / 3
        || rect.height < height / 3
        || rect.width < 4
        || rect.height < 4
        || (rect.x <= 1
            && rect.y <= 1
            && rect.x + rect.width + 1 >= width
            && rect.y + rect.height + 1 >= height);
    if uncertain { Some(full) } else { Some(rect) }
}

fn luma_slice_is_black(values: &[u8]) -> bool {
    if values.is_empty() {
        return true;
    }
    let mean = values.iter().map(|value| f64::from(*value)).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = f64::from(*value) - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    mean < 18.0 && variance < 18.0
}

fn luma_rect_variance(width: usize, luma: &[u8], rect: LumaRect) -> f64 {
    let mut count = 0usize;
    let mut sum = 0f64;
    let mut sum_sq = 0f64;
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let value = f64::from(luma[y * width + x]);
            count += 1;
            sum += value;
            sum_sq += value * value;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let mean = sum / count as f64;
    (sum_sq / count as f64) - (mean * mean)
}

pub fn video_landmarks_v3_from_luma_frame(
    width: usize,
    height: usize,
    luma: &[u8],
    t_ms: u32,
) -> Vec<VideoLandmarkV3> {
    let Some(content) = detect_content_window_luma(width, height, luma) else {
        return Vec::new();
    };
    if luma_rect_variance(width, luma, content) < V3_VIDEO_MIN_VARIANCE {
        return Vec::new();
    }
    let mut landmarks = Vec::new();
    if let Some(hash) = dct_phash_luma(width, height, luma, content) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_GLOBAL_DCT, t_ms, hash, 2);
    }
    if let Some(hash) = dct_phash_luma(width, height, luma, center_crop_rect(content, 0.68)) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_CENTER_DCT, t_ms, hash, 2);
    }
    if let Some(hash) = edge_hash_luma(width, height, luma, content) {
        push_v3_video_landmarks_for_hash(&mut landmarks, V3_VIDEO_KIND_EDGE, t_ms, hash, 2);
    }
    landmarks
}

pub fn video_landmarks_v3_from_luma_frames(
    width: usize,
    height: usize,
    frames: &[(u32, Vec<u8>)],
) -> Vec<VideoLandmarkV3> {
    let mut landmarks = Vec::new();
    for (t_ms, luma) in frames {
        landmarks.extend(video_landmarks_v3_from_luma_frame(
            width, height, luma, *t_ms,
        ));
    }
    add_v3_temporal_video_shingles(&mut landmarks);
    dedupe_video_landmarks_v3(&mut landmarks);
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_VERIFY_LANDMARK_LIMIT)
}

fn push_v3_video_landmarks_for_hash(
    landmarks: &mut Vec<VideoLandmarkV3>,
    kind: u8,
    t_ms: u32,
    hash64: u64,
    weight: u8,
) {
    for bucket in v3_video_lsh_buckets(kind, hash64) {
        landmarks.push(VideoLandmarkV3 {
            bucket,
            hash64,
            t_ms,
            kind,
            weight,
        });
    }
}

fn center_crop_rect(rect: LumaRect, scale: f64) -> LumaRect {
    let width = ((rect.width as f64 * scale).round() as usize)
        .clamp(4, rect.width.max(4))
        .min(rect.width);
    let height = ((rect.height as f64 * scale).round() as usize)
        .clamp(4, rect.height.max(4))
        .min(rect.height);
    LumaRect {
        x: rect.x + (rect.width - width) / 2,
        y: rect.y + (rect.height - height) / 2,
        width,
        height,
    }
}

fn sample_luma_rect_32(
    width: usize,
    luma: &[u8],
    rect: LumaRect,
) -> [f64; V3_VIDEO_PHASH_SIZE * V3_VIDEO_PHASH_SIZE] {
    let mut samples = [0f64; V3_VIDEO_PHASH_SIZE * V3_VIDEO_PHASH_SIZE];
    for y in 0..V3_VIDEO_PHASH_SIZE {
        let source_y = rect.y + ((y * rect.height) / V3_VIDEO_PHASH_SIZE).min(rect.height - 1);
        for x in 0..V3_VIDEO_PHASH_SIZE {
            let source_x = rect.x + ((x * rect.width) / V3_VIDEO_PHASH_SIZE).min(rect.width - 1);
            samples[y * V3_VIDEO_PHASH_SIZE + x] = f64::from(luma[source_y * width + source_x]);
        }
    }
    samples
}

fn dct_phash_luma(width: usize, height: usize, luma: &[u8], rect: LumaRect) -> Option<u64> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let samples = sample_luma_rect_32(width, luma, rect);
    let mut coeffs = [0f64; V3_VIDEO_PHASH_LOW_FREQ * V3_VIDEO_PHASH_LOW_FREQ];
    for v in 0..V3_VIDEO_PHASH_LOW_FREQ {
        for u in 0..V3_VIDEO_PHASH_LOW_FREQ {
            let mut sum = 0f64;
            for y in 0..V3_VIDEO_PHASH_SIZE {
                let cos_y = (((2 * y + 1) as f64 * v as f64 * std::f64::consts::PI)
                    / (2.0 * V3_VIDEO_PHASH_SIZE as f64))
                    .cos();
                for x in 0..V3_VIDEO_PHASH_SIZE {
                    let cos_x = (((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI)
                        / (2.0 * V3_VIDEO_PHASH_SIZE as f64))
                        .cos();
                    sum += samples[y * V3_VIDEO_PHASH_SIZE + x] * cos_x * cos_y;
                }
            }
            coeffs[v * V3_VIDEO_PHASH_LOW_FREQ + u] = sum;
        }
    }
    let mut comparable = coeffs[1..].to_vec();
    comparable.sort_by(|left, right| left.total_cmp(right));
    let median = comparable[comparable.len() / 2];
    let mut hash = 0u64;
    for (index, coeff) in coeffs.iter().enumerate().skip(1) {
        if *coeff >= median {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

fn edge_hash_luma(width: usize, height: usize, luma: &[u8], rect: LumaRect) -> Option<u64> {
    if width < 3 || height < 3 || luma.len() < width.saturating_mul(height) {
        return None;
    }
    let mut cells = [0f64; 64];
    for cell_y in 0..8 {
        for cell_x in 0..8 {
            let start_x = rect.x + cell_x * rect.width / 8;
            let end_x = (rect.x + ((cell_x + 1) * rect.width / 8))
                .max(start_x + 1)
                .min(rect.x + rect.width);
            let start_y = rect.y + cell_y * rect.height / 8;
            let end_y = (rect.y + ((cell_y + 1) * rect.height / 8))
                .max(start_y + 1)
                .min(rect.y + rect.height);
            let mut sum = 0f64;
            let mut count = 0f64;
            for y in start_y.max(1)..end_y.min(height - 1) {
                for x in start_x.max(1)..end_x.min(width - 1) {
                    let dx =
                        i16::from(luma[y * width + x + 1]) - i16::from(luma[y * width + x - 1]);
                    let dy =
                        i16::from(luma[(y + 1) * width + x]) - i16::from(luma[(y - 1) * width + x]);
                    sum += f64::from(dx.unsigned_abs() + dy.unsigned_abs());
                    count += 1.0;
                }
            }
            cells[cell_y * 8 + cell_x] = if count > 0.0 { sum / count } else { 0.0 };
        }
    }
    let mut sorted = cells;
    sorted.sort_by(|left, right| left.total_cmp(right));
    let median = sorted[sorted.len() / 2];
    if median <= 0.0 && cells.iter().all(|value| *value <= 0.0) {
        return None;
    }
    let mut hash = 0u64;
    for (index, value) in cells.iter().enumerate() {
        if *value >= median {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn add_v3_temporal_video_shingles(landmarks: &mut Vec<VideoLandmarkV3>) {
    let mut descriptors = landmarks
        .iter()
        .copied()
        .filter(|landmark| {
            matches!(
                landmark.kind,
                V3_VIDEO_KIND_GLOBAL_DCT | V3_VIDEO_KIND_CENTER_DCT | V3_VIDEO_KIND_EDGE
            )
        })
        .collect::<Vec<_>>();
    descriptors.sort_by_key(|landmark| (landmark.t_ms, landmark.kind, landmark.hash64));
    descriptors.dedup_by_key(|landmark| (landmark.t_ms, landmark.kind, landmark.hash64));
    let mut shingles = Vec::new();
    for (index, left) in descriptors.iter().enumerate() {
        let mut emitted = 0usize;
        for right in descriptors.iter().skip(index + 1) {
            let delta = right.t_ms.saturating_sub(left.t_ms);
            if delta > V3_VIDEO_TEMPORAL_MAX_DELTA_MS {
                break;
            }
            if delta < V3_VIDEO_TEMPORAL_MIN_DELTA_MS || left.kind != right.kind {
                continue;
            }
            let delta_bucket = delta / V3_VIDEO_TEMPORAL_DELTA_BUCKET_MS;
            let mut bytes = Vec::with_capacity(21);
            bytes.push(left.kind);
            bytes.extend_from_slice(&left.hash64.to_le_bytes());
            bytes.extend_from_slice(&right.hash64.to_le_bytes());
            bytes.extend_from_slice(&delta_bucket.to_le_bytes());
            let hash64 = stable_hash_u64(bytes);
            shingles.push(VideoLandmarkV3 {
                bucket: v3_video_bucket_for_kind(
                    V3_VIDEO_KIND_TEMPORAL_SHINGLE,
                    anchor_bucket(hash64),
                ),
                hash64,
                t_ms: left.t_ms,
                kind: V3_VIDEO_KIND_TEMPORAL_SHINGLE,
                weight: 4,
            });
            emitted += 1;
            if emitted >= V3_VIDEO_TEMPORAL_FANOUT {
                break;
            }
        }
    }
    landmarks.extend(shingles);
}

fn dedupe_video_landmarks_v3(landmarks: &mut Vec<VideoLandmarkV3>) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.kind,
            landmark.bucket,
            landmark.hash64,
            std::cmp::Reverse(landmark.weight),
        )
    });
    landmarks.dedup_by(|left, right| {
        left.t_ms == right.t_ms
            && left.kind == right.kind
            && left.bucket == right.bucket
            && left.hash64 == right.hash64
    });
}

pub fn pdq_style_luma_hash(width: usize, height: usize, luma: &[u8]) -> Option<u64> {
    if width == 0 || height == 0 || luma.len() < width.saturating_mul(height) {
        return None;
    }

    let mut cells = [0u32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let start_x = x * width / 8;
            let end_x = ((x + 1) * width / 8).max(start_x + 1).min(width);
            let start_y = y * height / 8;
            let end_y = ((y + 1) * height / 8).max(start_y + 1).min(height);
            let mut sum = 0u32;
            let mut count = 0u32;
            for source_y in start_y..end_y {
                let row = source_y * width;
                for source_x in start_x..end_x {
                    sum += u32::from(luma[row + source_x]);
                    count += 1;
                }
            }
            cells[y * 8 + x] = sum.checked_div(count).unwrap_or(0);
        }
    }

    let mean = cells.iter().sum::<u32>() / FRAME_HASH_BITS;
    let mut hash = 0u64;
    for (index, cell) in cells.iter().enumerate() {
        if *cell >= mean {
            hash |= 1u64 << index;
        }
    }
    Some(hash)
}

pub fn frame_hash_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}
