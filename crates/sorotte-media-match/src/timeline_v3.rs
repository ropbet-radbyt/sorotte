use crate::{AlignedSegmentV3, MatchClassV3, MediaTimelineMapV3, TimelinePositionMapResult};

// Runtime integration note:
// Given a V3 timeline map and the local playback position, map to a peer/candidate
// position only when that timestamp is inside an aligned segment. Do not infer
// across edit gaps, and keep SameMediaDifferentCut diagnostic-only for autoplay.
// The runtime should expose this mapping as debug evidence before using it for
// automatic seek/sync behavior.

pub fn classify_timeline_at_query_ms(map: &MediaTimelineMapV3, query_t_ms: u32) -> MatchClassV3 {
    if map
        .segments
        .iter()
        .any(|segment| segment.query_start_ms <= query_t_ms && query_t_ms <= segment.query_end_ms)
    {
        map.global_class
    } else if map.segments.is_empty() {
        MatchClassV3::Unknown
    } else {
        MatchClassV3::PartialOverlap
    }
}

pub fn timeline_map_contains_query_position(map: &MediaTimelineMapV3, query_t_ms: u32) -> bool {
    map.segments
        .iter()
        .any(|segment| segment.query_start_ms <= query_t_ms && query_t_ms <= segment.query_end_ms)
}

pub fn map_query_position_to_candidate_ms(
    map: &MediaTimelineMapV3,
    query_t_ms: u32,
) -> Option<TimelinePositionMapResult> {
    let (segment_index, segment) = map.segments.iter().enumerate().find(|(_, segment)| {
        segment.query_start_ms <= query_t_ms && query_t_ms <= segment.query_end_ms
    })?;
    let delta_ms = query_t_ms.saturating_sub(segment.query_start_ms);
    let mapped_delta_ms = affine_delta_ms(delta_ms, i64::from(segment.scale_ppm))?;
    let mapped_ms = clamp_i64_to_u32(
        i64::from(segment.candidate_start_ms) + mapped_delta_ms,
        segment.candidate_start_ms,
        segment.candidate_end_ms,
    );
    let confidence = timeline_position_confidence(map, segment);
    Some(TimelinePositionMapResult {
        mapped_ms,
        class_at_position: map.global_class,
        segment_index,
        confidence,
        local_offset_ms: i64::from(mapped_ms) - i64::from(query_t_ms),
        scale_ppm: segment.scale_ppm,
    })
}

pub fn map_candidate_position_to_query_ms(
    map: &MediaTimelineMapV3,
    candidate_t_ms: u32,
) -> Option<TimelinePositionMapResult> {
    let (segment_index, segment) = map.segments.iter().enumerate().find(|(_, segment)| {
        segment.candidate_start_ms <= candidate_t_ms
            && candidate_t_ms <= segment.candidate_end_ms
            && segment.scale_ppm > 0
    })?;
    let delta_ms = candidate_t_ms.saturating_sub(segment.candidate_start_ms);
    let mapped_delta_ms = affine_delta_ms(
        delta_ms,
        1_000_000_000_000i64 / i64::from(segment.scale_ppm),
    )?;
    let mapped_ms = clamp_i64_to_u32(
        i64::from(segment.query_start_ms) + mapped_delta_ms,
        segment.query_start_ms,
        segment.query_end_ms,
    );
    let confidence = timeline_position_confidence(map, segment);
    Some(TimelinePositionMapResult {
        mapped_ms,
        class_at_position: map.global_class,
        segment_index,
        confidence,
        local_offset_ms: i64::from(candidate_t_ms) - i64::from(mapped_ms),
        scale_ppm: segment.scale_ppm,
    })
}

fn affine_delta_ms(delta_ms: u32, scale_ppm: i64) -> Option<i64> {
    if scale_ppm <= 0 {
        return None;
    }
    Some((i64::from(delta_ms) * scale_ppm) / 1_000_000)
}

fn clamp_i64_to_u32(value: i64, min: u32, max: u32) -> u32 {
    value.clamp(i64::from(min), i64::from(max)) as u32
}

fn timeline_position_confidence(map: &MediaTimelineMapV3, segment: &AlignedSegmentV3) -> f32 {
    let multiplier = match map.global_class {
        MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable => 1.0,
        MatchClassV3::SameMediaDifferentCut | MatchClassV3::PartialOverlap => 0.85,
        MatchClassV3::SameAudioDifferentVideo | MatchClassV3::SameVideoDifferentAudio => 0.65,
        MatchClassV3::SharedIntroOutroOnly => 0.25,
        MatchClassV3::Reject | MatchClassV3::Unknown => 0.0,
    };
    (segment.confidence * multiplier).clamp(0.0, 1.0)
}
