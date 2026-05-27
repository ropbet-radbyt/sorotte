use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::{
    anchors::{AudioAnchor, MediaAnchorProfile, VideoAnchor, media_anchor_profile_from_record},
    tuning::{
        DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS, DEFAULT_ANCHOR_OFFSET_BIN_MS,
        MAX_BROAD_SCALE_FIT_PAIRS, V3_EDGE_REGION_MAX_MS, V3_EDGE_REGION_MIN_MS,
        V3_FAST_AUDIO_MIN_BODY_PAIRS, V3_FAST_AUDIO_MIN_BODY_REGIONS,
        V3_FAST_AUDIO_MIN_BODY_SPAN_MS, V3_FAST_AUDIO_TOP_OFFSET_BINS, V3_PIECEWISE_MAX_HYPOTHESES,
        V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS, V3_SEGMENT_AUDIO_MIN_PAIRS,
        V3_SEGMENT_AUDIO_MIN_SPAN_MS, V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS,
        V3_SEGMENT_AUDIO_VIDEO_MIN_SPAN_MS, V3_SEGMENT_MERGE_GAP_MS, V3_SEGMENT_MERGE_SCALE_PPM,
        V3_SEGMENT_MIN_PAIR_DELTA_MS, V3_SEGMENT_SPLIT_GAP_MS, V3_SEGMENT_VIDEO_MIN_PAIRS,
        V3_SEGMENT_VIDEO_MIN_SPAN_MS,
    },
    types::{
        AlignedSegmentV3, AudioMatchEvidence, MatchClassV3, MediaFingerprintRecord,
        MediaMatchDecision, MediaMatchEvidence, MediaMatchSettings, MediaMatchTier,
        MediaTimelineAlignment, MediaTimelineMapV3, MetadataMatchEvidence, VideoMatchEvidence,
    },
    video_v3::{
        v3_video_anchor_hashes_match, v3_video_bucket_kind_matches, v3_video_kind_is_supported,
    },
};

#[cfg(test)]
use crate::{
    tuning::DEFAULT_FRAME_HAMMING_THRESHOLD,
    video_v3::{VideoFingerprint, frame_hash_distance},
};

#[derive(Debug, Clone, PartialEq)]
pub struct MediaMatchCandidateDecision {
    pub candidate_path: String,
    pub decision: MediaMatchDecision,
}

pub(crate) fn media_match_tier_rank(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Exact => 5,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Unknown => 1,
        MediaMatchTier::Reject => 0,
    }
}

pub fn rank_media_match_candidates<'a>(
    query: &MediaFingerprintRecord,
    candidates: impl IntoIterator<Item = &'a MediaFingerprintRecord>,
    settings: &MediaMatchSettings,
) -> Vec<MediaMatchCandidateDecision> {
    let mut decisions = candidates
        .into_iter()
        .map(|candidate| MediaMatchCandidateDecision {
            candidate_path: candidate.identity.normalized_path.clone(),
            decision: decide_media_match(query, candidate, settings),
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| {
        media_match_tier_rank(right.decision.tier)
            .cmp(&media_match_tier_rank(left.decision.tier))
            .then_with(|| {
                media_match_candidate_aligned_pairs(&right.decision)
                    .cmp(&media_match_candidate_aligned_pairs(&left.decision))
            })
            .then_with(|| {
                media_match_candidate_aligned_span(&right.decision)
                    .total_cmp(&media_match_candidate_aligned_span(&left.decision))
            })
            .then_with(|| {
                right
                    .decision
                    .evidence
                    .video
                    .as_ref()
                    .map(|video| video.query_coverage.min(video.candidate_coverage))
                    .unwrap_or(0.0)
                    .total_cmp(
                        &left
                            .decision
                            .evidence
                            .video
                            .as_ref()
                            .map(|video| video.query_coverage.min(video.candidate_coverage))
                            .unwrap_or(0.0),
                    )
            })
            .then_with(|| left.candidate_path.cmp(&right.candidate_path))
    });
    decisions
}

fn media_match_candidate_aligned_pairs(decision: &MediaMatchDecision) -> usize {
    decision
        .evidence
        .alignment
        .as_ref()
        .map(|alignment| alignment.aligned_pairs)
        .or_else(|| {
            decision
                .evidence
                .video
                .as_ref()
                .map(|video| video.aligned_pairs)
        })
        .unwrap_or(0)
}

fn media_match_candidate_aligned_span(decision: &MediaMatchDecision) -> f64 {
    decision
        .evidence
        .alignment
        .as_ref()
        .map(|alignment| alignment.aligned_span_seconds)
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnchorMatchPair {
    pub(crate) query_t_ms: u32,
    pub(crate) candidate_t_ms: u32,
    pub(crate) modality: AnchorModality,
    pub(crate) weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchorModality {
    Audio,
    Video,
}

impl AnchorMatchPair {
    fn modality_order(self) -> u8 {
        match self.modality {
            AnchorModality::Audio => 0,
            AnchorModality::Video => 1,
        }
    }
}

#[derive(Debug, Clone)]
struct AnchorScaleOffsetFit {
    offset_ms: i64,
    scale_ppm: i32,
    drift_ratio: f64,
    aligned: Vec<AnchorMatchPair>,
}

#[derive(Debug, Clone)]
struct AnchorFitCandidate {
    score: u32,
    inlier_count: usize,
    span_ms: u32,
    max_residual_ms: f64,
    scale: f64,
    offset: f64,
    aligned: Vec<AnchorMatchPair>,
}

#[derive(Debug, Clone)]
struct V3SegmentCandidate {
    query_start_ms: u32,
    query_end_ms: u32,
    candidate_start_ms: u32,
    candidate_end_ms: u32,
    scale_ppm: i32,
    audio_pairs: usize,
    video_pairs: usize,
    weighted_score: u32,
    residual_ms: f64,
    confidence: f32,
}

#[derive(Debug, Clone)]
struct V3TimelineAnalysis {
    segments: Vec<V3SegmentCandidate>,
    total_aligned_span_ms: u32,
    largest_gap_ms: u32,
    edge_only: bool,
    audio_video_conflict: bool,
    best_segment_score: u32,
    second_best_segment_score: u32,
    audio_pairs: usize,
    video_pairs: usize,
    piecewise_pair_count: usize,
    piecewise_hypothesis_count: usize,
    piecewise_segment_candidate_count: usize,
    piecewise_segment_chain_count: usize,
    piecewise_fit_millis: u64,
}

#[derive(Debug, Clone, Copy)]
struct V3ClassificationContext {
    duration_ok: bool,
    meaningful_span: bool,
    drift_ok: bool,
    margin_ok: bool,
    continuity_ok: bool,
}

#[derive(Debug, Clone)]
struct V3AudioOffsetBinDiagnostic {
    offset_ms: i64,
    weighted_score: u32,
    pair_count: usize,
    query_span_ms: u32,
    candidate_span_ms: u32,
    body_pair_count: usize,
    edge_pair_count: usize,
    body_span_ms: u32,
    edge_span_ms: u32,
    body_region_count: usize,
    largest_body_gap_ms: u32,
}

#[derive(Debug, Clone)]
struct V3FastAudioProof {
    class: MatchClassV3,
    analysis: V3TimelineAnalysis,
    note: String,
}

pub fn decide_media_match_anchors(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let mut evidence = MediaMatchEvidence {
        metadata: MetadataMatchEvidence {
            duration_delta_seconds: query.duration_ms.zip(candidate.duration_ms).map(
                |(left, right)| (i64::from(left) - i64::from(right)).unsigned_abs() as f64 / 1000.0,
            ),
            duration_within_tolerance: query.duration_ms.zip(candidate.duration_ms).map(
                |(left, right)| {
                    !settings.runtime_tolerance_enabled
                        || (i64::from(left) - i64::from(right)).abs() as f64 / 1000.0
                            <= settings.runtime_tolerance_seconds
                },
            ),
            ..MetadataMatchEvidence::default()
        },
        audio: None,
        video: None,
        alignment: None,
        v3_class: None,
        timeline_map_v3: None,
        notes: Vec::new(),
    };

    if !settings.fingerprinting_enabled {
        evidence
            .notes
            .push("fingerprinting disabled; metadata is diagnostic only".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "fingerprinting disabled; no same-media decision",
        );
    }

    if query.is_empty() || candidate.is_empty() {
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "no comparable media match anchors",
        );
    }

    let pairs = collect_anchor_match_pairs(query, candidate);
    if pairs.is_empty() {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "anchor lookup found no shared timeline evidence",
        );
    }

    let Some((best_offset_ms, best_weight, second_weight)) = dominant_anchor_offset(&pairs) else {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "anchor offsets did not form a dominant hypothesis",
        );
    };
    let Some(fit) = fit_anchor_scale_offset(&pairs, best_offset_ms) else {
        return decision(
            MediaMatchTier::Reject,
            evidence,
            "no anchors fit the dominant offset",
        );
    };
    let aligned = fit.aligned.clone();

    let audio_pairs = aligned
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Audio)
        .count();
    let video_pairs = aligned
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Video)
        .count();
    let span_ms = aligned_anchor_span_ms(&aligned);
    let drift_ratio = fit.drift_ratio;
    let second_best_offset_margin = if best_weight > 0 {
        1.0 - (f64::from(second_weight) / f64::from(best_weight))
    } else {
        0.0
    };
    let query_audio_coverage = anchor_coverage(audio_pairs, query.audio_anchors.len());
    let candidate_audio_coverage = anchor_coverage(audio_pairs, candidate.audio_anchors.len());
    let query_video_coverage = anchor_coverage(
        video_pairs,
        unique_video_frame_anchor_count(&query.video_anchors),
    );
    let candidate_video_coverage = anchor_coverage(
        video_pairs,
        unique_video_frame_anchor_count(&candidate.video_anchors),
    );
    if !query.audio_anchors.is_empty() && !candidate.audio_anchors.is_empty() {
        evidence.audio = Some(AudioMatchEvidence {
            similarity: query_audio_coverage.min(candidate_audio_coverage),
            shared_anchor_ratio: query_audio_coverage.min(candidate_audio_coverage),
            duration_delta_seconds: evidence.metadata.duration_delta_seconds,
        });
    }
    if !query.video_anchors.is_empty() && !candidate.video_anchors.is_empty() {
        evidence.video = Some(VideoMatchEvidence {
            aligned_pairs: video_pairs,
            query_coverage: query_video_coverage,
            candidate_coverage: candidate_video_coverage,
            best_offset_seconds: fit.offset_ms as f64 / 1000.0,
            drift_ratio,
            mean_hamming_distance: 0.0,
        });
    }
    let (first_query_ms, last_query_ms, first_candidate_ms, last_candidate_ms) =
        aligned_anchor_bounds(&aligned);
    evidence.alignment = Some(MediaTimelineAlignment {
        offset_seconds: fit.offset_ms as f64 / 1000.0,
        scale_ppm: fit.scale_ppm,
        drift_ratio,
        aligned_pairs: aligned.len(),
        aligned_audio_anchors: audio_pairs,
        aligned_video_anchors: video_pairs,
        aligned_span_seconds: span_ms as f64 / 1000.0,
        second_best_offset_margin,
        first_query_second: first_query_ms as f64 / 1000.0,
        last_query_second: last_query_ms as f64 / 1000.0,
        first_candidate_second: first_candidate_ms as f64 / 1000.0,
        last_candidate_second: last_candidate_ms as f64 / 1000.0,
    });

    let duration_ok = evidence.metadata.duration_within_tolerance.unwrap_or(true);
    let drift_ok = drift_ratio <= settings.max_alignment_drift_ratio;
    let margin_ok = second_best_offset_margin >= 0.35 || second_weight == 0;
    let span_seconds = span_ms as f64 / 1000.0;
    let continuity_ok = aligned_anchor_largest_gap_ratio(&aligned) <= 0.65;
    let shorter_duration_seconds = query
        .duration_ms
        .zip(candidate.duration_ms)
        .map(|(left, right)| left.min(right) as f64 / 1000.0);
    let meaningful_span = shorter_duration_seconds
        .map(|duration| {
            let target = if duration >= 2400.0 {
                (duration * 0.30).min(600.0)
            } else {
                (duration * 0.25).min(300.0)
            };
            span_seconds >= target.clamp(30.0, 300.0)
        })
        .unwrap_or(span_seconds >= 30.0);
    let both_modalities = audio_pairs >= 3 && video_pairs >= 3;
    let very_strong_single_modality =
        (audio_pairs >= 16 || video_pairs >= 10) && meaningful_span && margin_ok && continuity_ok;
    let weak_evidence = audio_pairs >= 2 || video_pairs >= 2 || aligned.len() >= 3;
    if !query.audio_anchors.is_empty() && !candidate.audio_anchors.is_empty() {
        evidence.notes.push(format_audio_offset_bin_diagnostics(
            query, candidate, &pairs,
        ));
    }
    let classification_context = V3ClassificationContext {
        duration_ok,
        meaningful_span,
        drift_ok,
        margin_ok,
        continuity_ok,
    };
    let fast_audio_proof = fast_audio_same_cut_proof(
        query,
        candidate,
        &pairs,
        &fit,
        second_best_offset_margin,
        classification_context,
    );
    let (timeline_analysis, v3_class) = if let Some(proof) = fast_audio_proof {
        evidence.notes.push(proof.note);
        (proof.analysis, proof.class)
    } else {
        let timeline_analysis = build_v3_timeline_analysis(query, candidate, &pairs);
        let v3_class =
            classify_v3_timeline(query, candidate, &timeline_analysis, classification_context);
        (timeline_analysis, v3_class)
    };
    let video_inconclusive = !query.video_anchors.is_empty()
        && !candidate.video_anchors.is_empty()
        && timeline_analysis.video_pairs < V3_SEGMENT_VIDEO_MIN_PAIRS
        && !timeline_analysis.audio_video_conflict;
    evidence.notes.push(format!(
        "v3 segments={} span={:.1}s largest_gap={:.1}s edge_only={} audio_video_conflict={} video_inconclusive={} best_segment_score={} second_segment_score={} pair_count={} hypotheses={} segment_candidates={} chained_segments={} piecewise_fit_ms={}",
        timeline_analysis.segments.len(),
        f64::from(timeline_analysis.total_aligned_span_ms) / 1000.0,
        f64::from(timeline_analysis.largest_gap_ms) / 1000.0,
        timeline_analysis.edge_only,
        timeline_analysis.audio_video_conflict,
        video_inconclusive,
        timeline_analysis.best_segment_score,
        timeline_analysis.second_best_segment_score,
        timeline_analysis.piecewise_pair_count,
        timeline_analysis.piecewise_hypothesis_count,
        timeline_analysis.piecewise_segment_candidate_count,
        timeline_analysis.piecewise_segment_chain_count,
        timeline_analysis.piecewise_fit_millis
    ));

    let tier = match v3_class {
        MatchClassV3::SameCutStrong
            if (both_modalities
                && meaningful_span
                && drift_ok
                && margin_ok
                && duration_ok
                && continuity_ok)
                || (very_strong_single_modality
                    && drift_ok
                    && (duration_ok || span_seconds >= 300.0)) =>
        {
            MediaMatchTier::Strong
        }
        MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable => MediaMatchTier::Probable,
        MatchClassV3::SameMediaDifferentCut
        | MatchClassV3::SameAudioDifferentVideo
        | MatchClassV3::SameVideoDifferentAudio => MediaMatchTier::Probable,
        MatchClassV3::PartialOverlap => MediaMatchTier::Weak,
        MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject => MediaMatchTier::Reject,
        MatchClassV3::Unknown => {
            if weak_evidence {
                MediaMatchTier::Weak
            } else {
                MediaMatchTier::Unknown
            }
        }
    };
    let timeline_map = media_timeline_map_v3_from_analysis(v3_class, &timeline_analysis);
    evidence.timeline_map_v3 = Some(timeline_map);

    let explanation = match v3_class {
        MatchClassV3::SameCutStrong => "anchor timelines strongly align across the same cut",
        MatchClassV3::SameCutProbable => {
            "anchor timelines align but evidence is below same-cut strong confidence"
        }
        MatchClassV3::SameMediaDifferentCut => {
            "anchor timelines align in multiple body segments with edit differences"
        }
        MatchClassV3::SameAudioDifferentVideo => {
            "audio timeline aligns but video evidence conflicts or is absent"
        }
        MatchClassV3::SameVideoDifferentAudio => {
            "video timeline aligns but audio evidence conflicts or is absent"
        }
        MatchClassV3::PartialOverlap => "partial anchor timeline overlap",
        MatchClassV3::SharedIntroOutroOnly => {
            "anchor timeline evidence is concentrated at shared edges"
        }
        MatchClassV3::Reject => "anchor timeline evidence is insufficient",
        MatchClassV3::Unknown => "insufficient comparable anchor evidence",
    };
    decision(tier, evidence, explanation)
}

pub fn decide_media_match(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let mut evidence = MediaMatchEvidence {
        metadata: metadata_evidence(query, candidate, settings),
        audio: None,
        video: None,
        alignment: None,
        v3_class: None,
        timeline_map_v3: None,
        notes: Vec::new(),
    };

    if query.algorithm_version != candidate.algorithm_version {
        evidence.notes.push("algorithm version mismatch".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "algorithm version mismatch",
        );
    }

    if query.identity.normalized_path == candidate.identity.normalized_path
        && query.identity.modified_unix_millis == candidate.identity.modified_unix_millis
        && query.identity.size_bytes == candidate.identity.size_bytes
    {
        return decision(
            MediaMatchTier::Exact,
            evidence,
            "same path, modified time, and size",
        );
    }

    if !settings.fingerprinting_enabled {
        evidence
            .notes
            .push("fingerprinting disabled; metadata is diagnostic only".to_owned());
        return decision(
            MediaMatchTier::Unknown,
            evidence,
            "fingerprinting disabled; no same-media decision",
        );
    }

    if query.container_fingerprint == candidate.container_fingerprint
        && query.identity.size_bytes == candidate.identity.size_bytes
        && query.identity.size_bytes > 0
    {
        return decision(
            MediaMatchTier::Exact,
            evidence,
            "same container fingerprint and size",
        );
    }

    let query_profile = media_anchor_profile_from_record(query);
    let candidate_profile = media_anchor_profile_from_record(candidate);
    let decision = decide_media_match_anchors(&query_profile, &candidate_profile, settings);
    cap_sampled_audio_record_decision_if_needed(decision, query, candidate)
}

fn cap_sampled_audio_record_decision_if_needed(
    mut decision: MediaMatchDecision,
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
) -> MediaMatchDecision {
    let requires_dense_full_verify = !query
        .extraction_settings
        .audio_index_mode
        .is_dense_full_verify()
        || !candidate
            .extraction_settings
            .audio_index_mode
            .is_dense_full_verify();
    if !requires_dense_full_verify || decision.tier != MediaMatchTier::Strong {
        return decision;
    }
    decision.tier = MediaMatchTier::Probable;
    if decision.evidence.v3_class == Some(MatchClassV3::SameCutStrong) {
        decision.evidence.v3_class = Some(MatchClassV3::SameCutProbable);
    }
    if let Some(map) = &mut decision.evidence.timeline_map_v3
        && map.global_class == MatchClassV3::SameCutStrong
    {
        map.global_class = MatchClassV3::SameCutProbable;
        map.current_position_class = MatchClassV3::SameCutProbable;
    }
    decision.evidence.notes.push(
        "non-dense audio index record caps direct decision below Strong; dense full verification is required for SameCutStrong autoplay".to_owned(),
    );
    decision.explanation = format!(
        "{}; dense full verification is required for Strong",
        decision.explanation
    );
    decision
}

/// Legacy diagnostic helper for direct frame-hash sequence alignment.
///
/// Media Matching decisions use compact time-local anchors via
/// [`decide_media_match_anchors`] instead of this non-queryable comparison.
#[cfg(test)]
pub(crate) fn align_video_fingerprints(
    query: &VideoFingerprint,
    candidate: &VideoFingerprint,
) -> Option<VideoMatchEvidence> {
    if query.frames.is_empty() || candidate.frames.is_empty() {
        return None;
    }

    let mut all_pairs = Vec::new();
    for (query_index, query_frame) in query.frames.iter().enumerate() {
        for (candidate_index, candidate_frame) in candidate.frames.iter().enumerate() {
            let distance = frame_hash_distance(query_frame.hash, candidate_frame.hash);
            if distance <= DEFAULT_FRAME_HAMMING_THRESHOLD {
                all_pairs.push((query_index, candidate_index, distance));
            }
        }
    }

    all_pairs.sort_by_key(|(query_index, candidate_index, distance)| {
        (*query_index, *distance, *candidate_index)
    });

    let mut used_query = HashSet::new();
    let mut used_candidate = HashSet::new();
    let mut aligned = Vec::new();
    for (query_index, candidate_index, distance) in all_pairs {
        if used_query.contains(&query_index) || used_candidate.contains(&candidate_index) {
            continue;
        }
        let query_time = query.frames[query_index].timestamp_seconds();
        let candidate_time = candidate.frames[candidate_index].timestamp_seconds();
        used_query.insert(query_index);
        used_candidate.insert(candidate_index);
        aligned.push((query_time, candidate_time, distance));
    }

    if aligned.is_empty() {
        return None;
    }

    aligned.sort_by(|left, right| left.0.total_cmp(&right.0));
    let first = aligned[0];
    let last = aligned[aligned.len() - 1];
    let mut offsets = aligned
        .iter()
        .map(|(query_time, candidate_time, _)| candidate_time - query_time)
        .collect::<Vec<_>>();
    offsets.sort_by(f64::total_cmp);
    let best_offset_seconds = offsets[offsets.len() / 2];
    let query_span = (last.0 - first.0).abs().max(1.0);
    let offset_drift = ((last.1 - last.0) - (first.1 - first.0)).abs();
    let drift_ratio = offset_drift / query_span;
    let distance_sum: u32 = aligned.iter().map(|(_, _, distance)| *distance).sum();

    Some(VideoMatchEvidence {
        aligned_pairs: aligned.len(),
        query_coverage: aligned.len() as f64 / query.frames.len() as f64,
        candidate_coverage: aligned.len() as f64 / candidate.frames.len() as f64,
        best_offset_seconds,
        drift_ratio,
        mean_hamming_distance: distance_sum as f64 / aligned.len() as f64,
    })
}

pub(crate) fn collect_anchor_match_pairs(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
) -> Vec<AnchorMatchPair> {
    let mut candidate_audio = HashMap::<u32, Vec<&AudioAnchor>>::new();
    for anchor in &candidate.audio_anchors {
        candidate_audio
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
    }
    let mut candidate_video = HashMap::<u32, Vec<&VideoAnchor>>::new();
    let mut candidate_video_all = Vec::new();
    for anchor in &candidate.video_anchors {
        candidate_video
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
        candidate_video_all.push(anchor);
    }
    let mut pairs = Vec::new();
    for query_anchor in &query.audio_anchors {
        if let Some(candidate_anchors) = candidate_audio.get(&query_anchor.bucket) {
            for candidate_anchor in candidate_anchors {
                pairs.push(AnchorMatchPair {
                    query_t_ms: query_anchor.t_ms,
                    candidate_t_ms: candidate_anchor.t_ms,
                    modality: AnchorModality::Audio,
                    weight: query_anchor.weight.min(candidate_anchor.weight),
                });
            }
        }
    }
    let mut seen_video_pairs = HashSet::<(u32, u32, u8, u64, u64)>::new();
    for query_anchor in &query.video_anchors {
        if let Some(candidate_anchors) = candidate_video.get(&query_anchor.bucket) {
            for candidate_anchor in candidate_anchors {
                push_video_anchor_match_pair(
                    &mut pairs,
                    &mut seen_video_pairs,
                    query_anchor,
                    candidate_anchor,
                );
            }
        }
        for candidate_anchor in &candidate_video_all {
            push_video_anchor_match_pair(
                &mut pairs,
                &mut seen_video_pairs,
                query_anchor,
                candidate_anchor,
            );
        }
    }
    pairs
}

fn push_video_anchor_match_pair(
    pairs: &mut Vec<AnchorMatchPair>,
    seen_video_pairs: &mut HashSet<(u32, u32, u8, u64, u64)>,
    query_anchor: &VideoAnchor,
    candidate_anchor: &VideoAnchor,
) -> bool {
    if !v3_video_kind_is_supported(query_anchor.kind)
        || !v3_video_kind_is_supported(candidate_anchor.kind)
        || !v3_video_bucket_kind_matches(query_anchor.kind, query_anchor.bucket)
        || !v3_video_bucket_kind_matches(candidate_anchor.kind, candidate_anchor.bucket)
    {
        return false;
    }
    if query_anchor.kind != candidate_anchor.kind {
        return false;
    }
    if !v3_video_anchor_hashes_match(
        query_anchor.kind,
        query_anchor.hash64,
        candidate_anchor.hash64,
    ) {
        return false;
    }
    if !seen_video_pairs.insert((
        query_anchor.t_ms,
        candidate_anchor.t_ms,
        query_anchor.kind,
        query_anchor.hash64,
        candidate_anchor.hash64,
    )) {
        return true;
    }
    pairs.push(AnchorMatchPair {
        query_t_ms: query_anchor.t_ms,
        candidate_t_ms: candidate_anchor.t_ms,
        modality: AnchorModality::Video,
        weight: query_anchor.weight.min(candidate_anchor.weight),
    });
    true
}

fn dominant_anchor_offset(pairs: &[AnchorMatchPair]) -> Option<(i64, u32, u32)> {
    let mut bins = HashMap::<i64, u32>::new();
    for pair in pairs {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        let bin = rounded_offset_bin(offset);
        *bins.entry(bin).or_default() += u32::from(pair.weight.max(1));
    }
    let mut ranked = bins.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (best_bin, best_weight) = ranked.first().copied()?;
    let second_weight = ranked.get(1).map(|(_, weight)| *weight).unwrap_or(0);
    Some((
        best_bin * DEFAULT_ANCHOR_OFFSET_BIN_MS,
        best_weight,
        second_weight,
    ))
}

fn rounded_offset_bin(offset_ms: i64) -> i64 {
    if offset_ms >= 0 {
        (offset_ms + (DEFAULT_ANCHOR_OFFSET_BIN_MS / 2)) / DEFAULT_ANCHOR_OFFSET_BIN_MS
    } else {
        (offset_ms - (DEFAULT_ANCHOR_OFFSET_BIN_MS / 2)) / DEFAULT_ANCHOR_OFFSET_BIN_MS
    }
}

fn fit_anchor_scale_offset(
    pairs: &[AnchorMatchPair],
    voted_offset_ms: i64,
) -> Option<AnchorScaleOffsetFit> {
    let seeded = pairs
        .iter()
        .copied()
        .filter(|pair| {
            let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
            (offset - voted_offset_ms).abs() <= DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
        })
        .collect::<Vec<_>>();
    if seeded.is_empty() {
        return None;
    }

    let mut candidates = vec![(1.0, voted_offset_ms as f64)];
    let seeded_candidates = broad_scale_fit_sample(&seeded);
    add_scale_offset_candidates_from_pairs(&seeded_candidates, &mut candidates);
    if seeded.len() < pairs.len() {
        let broad_pairs = broad_scale_fit_sample(pairs);
        add_scale_offset_candidates_from_pairs(&broad_pairs, &mut candidates);
    }

    let mut best: Option<AnchorFitCandidate> = None;
    for (scale, offset) in candidates {
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.is_empty() {
            continue;
        }
        let (scale, offset) = least_squares_anchor_fit(&inliers).unwrap_or((scale, offset));
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.is_empty() {
            continue;
        }
        let score = inliers
            .iter()
            .map(|pair| u32::from(pair.weight.max(1)))
            .sum::<u32>();
        let span = aligned_anchor_span_ms(&inliers);
        let max_residual = max_anchor_fit_residual_ms(&inliers, scale, offset);
        let candidate = AnchorFitCandidate {
            score,
            inlier_count: inliers.len(),
            span_ms: span,
            max_residual_ms: max_residual,
            scale,
            offset,
            aligned: inliers,
        };
        let replace = best.as_ref().is_none_or(|current| {
            candidate
                .score
                .cmp(&current.score)
                .then_with(|| candidate.inlier_count.cmp(&current.inlier_count))
                .then_with(|| candidate.span_ms.cmp(&current.span_ms))
                .then_with(|| {
                    current
                        .max_residual_ms
                        .total_cmp(&candidate.max_residual_ms)
                })
                .is_gt()
        });
        if replace {
            best = Some(candidate);
        }
    }

    let best = best?;
    let drift_ratio = if best.span_ms > 0 {
        best.max_residual_ms / f64::from(best.span_ms)
    } else {
        0.0
    };
    let scale_ppm = (best.scale * 1_000_000.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    Some(AnchorScaleOffsetFit {
        offset_ms: best.offset.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64,
        scale_ppm,
        drift_ratio,
        aligned: best.aligned,
    })
}

fn add_scale_offset_candidates_from_pairs(
    pairs: &[AnchorMatchPair],
    candidates: &mut Vec<(f64, f64)>,
) {
    for (left_index, left) in pairs.iter().enumerate() {
        for right in pairs.iter().skip(left_index + 1) {
            let query_delta = f64::from(right.query_t_ms) - f64::from(left.query_t_ms);
            if query_delta.abs() < 10_000.0 {
                continue;
            }
            let candidate_delta = f64::from(right.candidate_t_ms) - f64::from(left.candidate_t_ms);
            let scale = candidate_delta / query_delta;
            if !(0.95..=1.05).contains(&scale) {
                continue;
            }
            let offset = f64::from(left.candidate_t_ms) - (scale * f64::from(left.query_t_ms));
            candidates.push((scale, offset));
        }
    }
}

fn broad_scale_fit_sample(pairs: &[AnchorMatchPair]) -> Vec<AnchorMatchPair> {
    if pairs.len() <= MAX_BROAD_SCALE_FIT_PAIRS {
        return pairs.to_vec();
    }
    let stride = pairs.len() as f64 / MAX_BROAD_SCALE_FIT_PAIRS as f64;
    (0..MAX_BROAD_SCALE_FIT_PAIRS)
        .map(|index| pairs[(index as f64 * stride).floor() as usize])
        .collect()
}

fn anchor_fit_inliers(pairs: &[AnchorMatchPair], scale: f64, offset: f64) -> Vec<AnchorMatchPair> {
    pairs
        .iter()
        .copied()
        .filter(|pair| {
            let predicted = (scale * f64::from(pair.query_t_ms)) + offset;
            (f64::from(pair.candidate_t_ms) - predicted).abs()
                <= DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS as f64
        })
        .collect()
}

fn least_squares_anchor_fit(pairs: &[AnchorMatchPair]) -> Option<(f64, f64)> {
    if pairs.len() < 2 {
        return None;
    }
    let mut sum_weight = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    for pair in pairs {
        let weight = f64::from(pair.weight.max(1));
        let x = f64::from(pair.query_t_ms);
        let y = f64::from(pair.candidate_t_ms);
        sum_weight += weight;
        sum_x += weight * x;
        sum_y += weight * y;
        sum_xx += weight * x * x;
        sum_xy += weight * x * y;
    }
    let denominator = (sum_weight * sum_xx) - (sum_x * sum_x);
    if denominator.abs() < f64::EPSILON {
        return None;
    }
    let scale = ((sum_weight * sum_xy) - (sum_x * sum_y)) / denominator;
    if !(0.95..=1.05).contains(&scale) {
        return None;
    }
    let offset = (sum_y - (scale * sum_x)) / sum_weight;
    Some((scale, offset))
}

fn max_anchor_fit_residual_ms(pairs: &[AnchorMatchPair], scale: f64, offset: f64) -> f64 {
    pairs
        .iter()
        .map(|pair| {
            let predicted = (scale * f64::from(pair.query_t_ms)) + offset;
            (f64::from(pair.candidate_t_ms) - predicted).abs()
        })
        .fold(0.0, f64::max)
}

fn fast_audio_same_cut_proof(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
    fit: &AnchorScaleOffsetFit,
    second_best_offset_margin: f64,
    context: V3ClassificationContext,
) -> Option<V3FastAudioProof> {
    if !query.video_anchors.is_empty() || !candidate.video_anchors.is_empty() {
        return None;
    }
    let diagnostics = audio_offset_bin_diagnostics(query, candidate, pairs);
    let best = diagnostics
        .iter()
        .take(V3_FAST_AUDIO_TOP_OFFSET_BINS)
        .next()?;
    let enough_body_pairs = best.body_pair_count >= V3_FAST_AUDIO_MIN_BODY_PAIRS
        && best.body_region_count >= V3_FAST_AUDIO_MIN_BODY_REGIONS;
    let enough_body_span = best.body_span_ms >= V3_FAST_AUDIO_MIN_BODY_SPAN_MS;
    let largest_body_gap_ratio = if best.body_span_ms > 0 {
        f64::from(best.largest_body_gap_ms) / f64::from(best.body_span_ms)
    } else {
        1.0
    };
    let continuity_ok = context.continuity_ok && largest_body_gap_ratio <= 0.65;
    let edge_only = best.body_pair_count == 0 || best.body_span_ms < 45_000;
    let mut blocked = Vec::new();
    if edge_only {
        blocked.push("edge_only");
    }
    if !enough_body_span {
        blocked.push("insufficient_body_span");
    }
    if second_best_offset_margin < 0.35 {
        blocked.push("weak_margin");
    }
    if !context.drift_ok {
        blocked.push("drift");
    }
    if !context.duration_ok {
        blocked.push("duration");
    }
    if !continuity_ok {
        blocked.push("continuity");
    }
    if edge_only || !enough_body_pairs || !enough_body_span || !context.duration_ok {
        return None;
    }
    let class = if blocked.is_empty() {
        MatchClassV3::SameCutStrong
    } else if context.drift_ok && continuity_ok && second_best_offset_margin >= 0.20 {
        MatchClassV3::SameCutProbable
    } else {
        return None;
    };
    let inliers = audio_pairs_for_offset_ms(pairs, best.offset_ms);
    let (scale, offset) = least_squares_anchor_fit(&inliers)
        .unwrap_or((f64::from(fit.scale_ppm) / 1_000_000.0, fit.offset_ms as f64));
    let segment = v3_segment_candidate_from_pairs(&inliers, scale, offset)?;
    let weighted_score = segment.weighted_score;
    let analysis = V3TimelineAnalysis {
        segments: vec![segment],
        total_aligned_span_ms: best.body_span_ms.max(best.query_span_ms),
        largest_gap_ms: best.largest_body_gap_ms,
        edge_only: false,
        audio_video_conflict: false,
        best_segment_score: weighted_score,
        second_best_segment_score: diagnostics
            .get(1)
            .map(|diagnostic| diagnostic.weighted_score)
            .unwrap_or(0),
        audio_pairs: inliers.len(),
        video_pairs: 0,
        piecewise_pair_count: pairs.len(),
        piecewise_hypothesis_count: 0,
        piecewise_segment_candidate_count: 1,
        piecewise_segment_chain_count: 1,
        piecewise_fit_millis: 0,
    };
    let note = format!(
        "fast_audio_verifier class={:?} total_audio_pairs={} best_offset_ms={} body_pairs={} body_regions={} body_span={:.1}s edge_span={:.1}s largest_body_gap={:.1}s margin={:.3} blocked=[{}]",
        class,
        pairs
            .iter()
            .filter(|pair| pair.modality == AnchorModality::Audio)
            .count(),
        best.offset_ms,
        best.body_pair_count,
        best.body_region_count,
        f64::from(best.body_span_ms) / 1000.0,
        f64::from(best.edge_span_ms) / 1000.0,
        f64::from(best.largest_body_gap_ms) / 1000.0,
        second_best_offset_margin,
        blocked.join(",")
    );
    Some(V3FastAudioProof {
        class,
        analysis,
        note,
    })
}

fn audio_pairs_for_offset_ms(pairs: &[AnchorMatchPair], offset_ms: i64) -> Vec<AnchorMatchPair> {
    pairs
        .iter()
        .copied()
        .filter(|pair| {
            pair.modality == AnchorModality::Audio
                && (i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms) - offset_ms).abs()
                    <= DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS
        })
        .collect()
}

fn format_audio_offset_bin_diagnostics(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
) -> String {
    let diagnostics = audio_offset_bin_diagnostics(query, candidate, pairs);
    let total_audio_pairs = pairs
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Audio)
        .count();
    let bins = diagnostics
        .iter()
        .take(10)
        .map(|diagnostic| {
            format!(
                "{{offset_ms:{} score:{} pairs:{} q_span_ms:{} c_span_ms:{} body_pairs:{} edge_pairs:{} body_span_ms:{} edge_span_ms:{} largest_body_gap_ms:{}}}",
                diagnostic.offset_ms,
                diagnostic.weighted_score,
                diagnostic.pair_count,
                diagnostic.query_span_ms,
                diagnostic.candidate_span_ms,
                diagnostic.body_pair_count,
                diagnostic.edge_pair_count,
                diagnostic.body_span_ms,
                diagnostic.edge_span_ms,
                diagnostic.largest_body_gap_ms
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("audio_pair_diagnostics total_audio_pairs={total_audio_pairs} top_offset_bins=[{bins}]")
}

fn audio_offset_bin_diagnostics(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
) -> Vec<V3AudioOffsetBinDiagnostic> {
    let mut bins = HashMap::<i64, Vec<AnchorMatchPair>>::new();
    for pair in pairs
        .iter()
        .copied()
        .filter(|pair| pair.modality == AnchorModality::Audio)
    {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        bins.entry(rounded_offset_bin(offset))
            .or_default()
            .push(pair);
    }
    let mut diagnostics = bins
        .into_iter()
        .map(|(bin, pairs)| {
            let weighted_score = pairs
                .iter()
                .map(|pair| u32::from(pair.weight.max(1)))
                .sum::<u32>();
            let body_pairs = pairs
                .iter()
                .copied()
                .filter(|pair| audio_pair_is_body(query, candidate, *pair))
                .collect::<Vec<_>>();
            let edge_pair_count = pairs.len().saturating_sub(body_pairs.len());
            let mut body_regions = body_pairs
                .iter()
                .map(|pair| pair.query_t_ms / 60_000)
                .collect::<Vec<_>>();
            body_regions.sort_unstable();
            body_regions.dedup();
            V3AudioOffsetBinDiagnostic {
                offset_ms: bin * DEFAULT_ANCHOR_OFFSET_BIN_MS,
                weighted_score,
                pair_count: pairs.len(),
                query_span_ms: aligned_anchor_span_ms(&pairs),
                candidate_span_ms: candidate_anchor_span_ms(&pairs),
                body_pair_count: body_pairs.len(),
                edge_pair_count,
                body_span_ms: aligned_anchor_span_ms(&body_pairs),
                edge_span_ms: audio_edge_span_ms(query, candidate, &pairs),
                body_region_count: body_regions.len(),
                largest_body_gap_ms: largest_query_gap_ms(&body_pairs),
            }
        })
        .collect::<Vec<_>>();
    diagnostics.sort_by(|left, right| {
        right
            .weighted_score
            .cmp(&left.weighted_score)
            .then_with(|| right.body_span_ms.cmp(&left.body_span_ms))
            .then_with(|| right.body_pair_count.cmp(&left.body_pair_count))
            .then_with(|| left.offset_ms.cmp(&right.offset_ms))
    });
    diagnostics
}

fn build_v3_timeline_analysis(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
) -> V3TimelineAnalysis {
    let started_at = Instant::now();
    let mut hypotheses = Vec::<(f64, f64)>::new();
    let mut offset_bins = pairs
        .iter()
        .map(|pair| rounded_offset_bin(i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms)))
        .collect::<Vec<_>>();
    offset_bins.sort_unstable();
    offset_bins.dedup();
    for offset_bin in offset_bins {
        hypotheses.push((1.0, (offset_bin * DEFAULT_ANCHOR_OFFSET_BIN_MS) as f64));
    }
    let hypothesis_pairs = select_v3_piecewise_hypothesis_pairs(pairs);
    add_v3_piecewise_hypotheses_from_pairs(&hypothesis_pairs, &mut hypotheses);
    let piecewise_hypothesis_count = hypotheses.len();

    let mut segment_candidates = Vec::<V3SegmentCandidate>::new();
    for (scale, offset) in hypotheses {
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        if inliers.len() < 2 {
            continue;
        }
        let (scale, offset) = least_squares_anchor_fit(&inliers).unwrap_or((scale, offset));
        let inliers = anchor_fit_inliers(pairs, scale, offset);
        segment_candidates.extend(v3_segments_from_inliers(&inliers, scale, offset));
    }
    let piecewise_segment_candidate_count = segment_candidates.len();
    let mut segments = chain_v3_segments(dedup_v3_segments(segment_candidates));
    let piecewise_segment_chain_count = segments.len();
    merge_adjacent_v3_segments(&mut segments);
    let total_aligned_span_ms = segments
        .iter()
        .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
        .sum::<u32>();
    let largest_gap_ms = largest_v3_segment_gap_ms(&segments);
    let mut scores = segments
        .iter()
        .map(|segment| segment.weighted_score)
        .collect::<Vec<_>>();
    scores.sort_unstable_by(|left, right| right.cmp(left));
    let best_segment_score = scores.first().copied().unwrap_or(0);
    let second_best_segment_score = scores.get(1).copied().unwrap_or(0);
    let audio_pairs = segments.iter().map(|segment| segment.audio_pairs).sum();
    let video_pairs = segments.iter().map(|segment| segment.video_pairs).sum();
    let edge_only = v3_segments_are_edge_only(&segments, query.duration_ms, candidate.duration_ms);
    let audio_video_conflict = v3_audio_video_conflict(query, candidate, pairs, &segments);
    V3TimelineAnalysis {
        segments,
        total_aligned_span_ms,
        largest_gap_ms,
        edge_only,
        audio_video_conflict,
        best_segment_score,
        second_best_segment_score,
        audio_pairs,
        video_pairs,
        piecewise_pair_count: pairs.len(),
        piecewise_hypothesis_count,
        piecewise_segment_candidate_count,
        piecewise_segment_chain_count,
        piecewise_fit_millis: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

pub(crate) fn select_v3_piecewise_hypothesis_pairs(
    pairs: &[AnchorMatchPair],
) -> Vec<AnchorMatchPair> {
    if pairs.len() <= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS {
        return pairs.to_vec();
    }
    let mut bin_scores = HashMap::<i64, u32>::new();
    for pair in pairs {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        *bin_scores.entry(rounded_offset_bin(offset)).or_default() += u32::from(pair.weight.max(1));
    }
    let mut bins = bin_scores.into_iter().collect::<Vec<_>>();
    bins.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut selected = Vec::with_capacity(V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS);
    let mut seen = HashSet::<(u32, u32, u8)>::new();
    for modality in [AnchorModality::Audio, AnchorModality::Video] {
        let modality_quota = V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS / 8;
        for (bin, _) in &bins {
            let mut candidates = pairs
                .iter()
                .copied()
                .filter(|pair| {
                    pair.modality == modality
                        && rounded_offset_bin(
                            i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms),
                        ) == *bin
                })
                .collect::<Vec<_>>();
            candidates.sort_by(v3_hypothesis_pair_order);
            for pair in candidates {
                push_v3_hypothesis_pair(&mut selected, &mut seen, pair);
                if selected
                    .iter()
                    .filter(|selected| selected.modality == modality)
                    .count()
                    >= modality_quota
                    || selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS
                {
                    break;
                }
            }
            if selected
                .iter()
                .filter(|selected| selected.modality == modality)
                .count()
                >= modality_quota
                || selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS
            {
                break;
            }
        }
    }
    for (bin, _) in bins {
        let mut candidates = pairs
            .iter()
            .copied()
            .filter(|pair| {
                rounded_offset_bin(i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms))
                    == bin
            })
            .collect::<Vec<_>>();
        candidates.sort_by(v3_hypothesis_pair_order);
        for pair in candidates {
            push_v3_hypothesis_pair(&mut selected, &mut seen, pair);
            if selected.len() >= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS {
                return selected;
            }
        }
    }
    selected
}

fn push_v3_hypothesis_pair(
    selected: &mut Vec<AnchorMatchPair>,
    seen: &mut HashSet<(u32, u32, u8)>,
    pair: AnchorMatchPair,
) {
    if seen.insert((pair.query_t_ms, pair.candidate_t_ms, pair.modality_order())) {
        selected.push(pair);
    }
}

fn v3_hypothesis_pair_order(left: &AnchorMatchPair, right: &AnchorMatchPair) -> std::cmp::Ordering {
    right
        .weight
        .cmp(&left.weight)
        .then_with(|| left.query_t_ms.cmp(&right.query_t_ms))
        .then_with(|| left.candidate_t_ms.cmp(&right.candidate_t_ms))
        .then_with(|| left.modality_order().cmp(&right.modality_order()))
}

fn add_v3_piecewise_hypotheses_from_pairs(
    pairs: &[AnchorMatchPair],
    hypotheses: &mut Vec<(f64, f64)>,
) {
    'outer: for (left_index, left) in pairs.iter().enumerate() {
        for right in pairs.iter().skip(left_index + 1) {
            if hypotheses.len() >= V3_PIECEWISE_MAX_HYPOTHESES {
                break 'outer;
            }
            let query_delta = right.query_t_ms.abs_diff(left.query_t_ms);
            if query_delta < V3_SEGMENT_MIN_PAIR_DELTA_MS {
                continue;
            }
            let query_delta = f64::from(right.query_t_ms) - f64::from(left.query_t_ms);
            let candidate_delta = f64::from(right.candidate_t_ms) - f64::from(left.candidate_t_ms);
            if query_delta.abs() < f64::EPSILON {
                continue;
            }
            let scale = candidate_delta / query_delta;
            if !(0.95..=1.05).contains(&scale) {
                continue;
            }
            let offset = f64::from(left.candidate_t_ms) - (scale * f64::from(left.query_t_ms));
            hypotheses.push((scale, offset));
        }
    }
    hypotheses.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    hypotheses.dedup_by(|left, right| {
        (left.0 - right.0).abs() < 0.000_1 && (left.1 - right.1).abs() < 250.0
    });
}

fn v3_segments_from_inliers(
    inliers: &[AnchorMatchPair],
    scale: f64,
    offset: f64,
) -> Vec<V3SegmentCandidate> {
    let mut sorted = inliers.to_vec();
    sorted.sort_by_key(|pair| (pair.query_t_ms, pair.candidate_t_ms, pair.modality_order()));
    let mut segments = Vec::new();
    let mut current = Vec::<AnchorMatchPair>::new();
    for pair in sorted {
        if let Some(previous) = current.last().copied() {
            let query_gap = pair.query_t_ms.saturating_sub(previous.query_t_ms);
            let candidate_gap = pair.candidate_t_ms.saturating_sub(previous.candidate_t_ms);
            let current_span = aligned_anchor_span_ms(&current);
            let gap_threshold =
                V3_SEGMENT_SPLIT_GAP_MS.max((f64::from(current_span) * 0.15).round() as u32);
            let gap_delta = query_gap.abs_diff(candidate_gap);
            let large_common_gap = query_gap.max(candidate_gap) > 300_000;
            if gap_delta > gap_threshold || large_common_gap {
                if let Some(segment) = v3_segment_candidate_from_pairs(&current, scale, offset) {
                    segments.push(segment);
                }
                current.clear();
            }
        }
        current.push(pair);
    }
    if let Some(segment) = v3_segment_candidate_from_pairs(&current, scale, offset) {
        segments.push(segment);
    }
    segments
}

fn v3_segment_candidate_from_pairs(
    pairs: &[AnchorMatchPair],
    scale: f64,
    offset: f64,
) -> Option<V3SegmentCandidate> {
    if pairs.is_empty() {
        return None;
    }
    let audio_pairs = pairs
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Audio)
        .count();
    let video_pairs = pairs
        .iter()
        .filter(|pair| pair.modality == AnchorModality::Video)
        .count();
    let span_ms = aligned_anchor_span_ms(pairs);
    let enough = if audio_pairs >= V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS
        && video_pairs >= V3_SEGMENT_AUDIO_VIDEO_MIN_PAIRS
    {
        span_ms >= V3_SEGMENT_AUDIO_VIDEO_MIN_SPAN_MS
    } else if audio_pairs >= V3_SEGMENT_AUDIO_MIN_PAIRS {
        span_ms >= V3_SEGMENT_AUDIO_MIN_SPAN_MS
    } else if video_pairs >= V3_SEGMENT_VIDEO_MIN_PAIRS {
        span_ms >= V3_SEGMENT_VIDEO_MIN_SPAN_MS
    } else {
        false
    };
    if !enough {
        return None;
    }
    let (query_start_ms, query_end_ms, candidate_start_ms, candidate_end_ms) =
        aligned_anchor_bounds(pairs);
    let weighted_score = pairs
        .iter()
        .map(|pair| u32::from(pair.weight.max(1)))
        .sum::<u32>();
    let residual_ms = max_anchor_fit_residual_ms(pairs, scale, offset);
    let scale_ppm = (scale * 1_000_000.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let confidence = (weighted_score as f32 / 64.0)
        .min(1.0)
        .max((span_ms as f32 / 600_000.0).min(1.0));
    Some(V3SegmentCandidate {
        query_start_ms,
        query_end_ms,
        candidate_start_ms,
        candidate_end_ms,
        scale_ppm,
        audio_pairs,
        video_pairs,
        weighted_score,
        residual_ms,
        confidence,
    })
}

fn dedup_v3_segments(mut segments: Vec<V3SegmentCandidate>) -> Vec<V3SegmentCandidate> {
    segments.sort_by(|left, right| {
        left.query_start_ms
            .cmp(&right.query_start_ms)
            .then_with(|| left.query_end_ms.cmp(&right.query_end_ms))
            .then_with(|| left.candidate_start_ms.cmp(&right.candidate_start_ms))
            .then_with(|| left.candidate_end_ms.cmp(&right.candidate_end_ms))
            .then_with(|| right.weighted_score.cmp(&left.weighted_score))
    });
    let mut deduped = Vec::<V3SegmentCandidate>::new();
    for segment in segments {
        let duplicate = deduped.iter().any(|current| {
            current.query_start_ms.abs_diff(segment.query_start_ms) <= 1_000
                && current.query_end_ms.abs_diff(segment.query_end_ms) <= 1_000
                && current
                    .candidate_start_ms
                    .abs_diff(segment.candidate_start_ms)
                    <= 1_000
                && current.candidate_end_ms.abs_diff(segment.candidate_end_ms) <= 1_000
        });
        if !duplicate {
            deduped.push(segment);
        }
    }
    deduped
}

fn chain_v3_segments(mut segments: Vec<V3SegmentCandidate>) -> Vec<V3SegmentCandidate> {
    if segments.len() <= 1 {
        return segments;
    }
    segments.sort_by(|left, right| {
        left.query_start_ms
            .cmp(&right.query_start_ms)
            .then_with(|| left.candidate_start_ms.cmp(&right.candidate_start_ms))
            .then_with(|| right.weighted_score.cmp(&left.weighted_score))
    });
    let mut best_scores = vec![0i64; segments.len()];
    let mut previous = vec![None; segments.len()];
    for index in 0..segments.len() {
        best_scores[index] = v3_segment_chain_score(&segments[index]);
        for prev_index in 0..index {
            if !v3_segments_are_chain_compatible(&segments[prev_index], &segments[index]) {
                continue;
            }
            let candidate_score =
                best_scores[prev_index] + v3_segment_chain_score(&segments[index]);
            if candidate_score > best_scores[index]
                || (candidate_score == best_scores[index]
                    && previous[index].is_none_or(|current| prev_index < current))
            {
                best_scores[index] = candidate_score;
                previous[index] = Some(prev_index);
            }
        }
    }
    let Some((mut index, _)) = best_scores
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(&left.0)))
    else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    loop {
        chain.push(segments[index].clone());
        let Some(prev_index) = previous[index] else {
            break;
        };
        index = prev_index;
    }
    chain.reverse();
    chain
}

fn v3_segment_chain_score(segment: &V3SegmentCandidate) -> i64 {
    i64::from(segment.weighted_score) * 1_000
        + i64::from(segment.query_end_ms.saturating_sub(segment.query_start_ms) / 1_000)
}

fn v3_segments_are_chain_compatible(left: &V3SegmentCandidate, right: &V3SegmentCandidate) -> bool {
    left.query_end_ms <= right.query_start_ms && left.candidate_end_ms <= right.candidate_start_ms
}

fn merge_adjacent_v3_segments(segments: &mut Vec<V3SegmentCandidate>) {
    if segments.len() < 2 {
        return;
    }
    let mut merged = Vec::<V3SegmentCandidate>::new();
    for segment in segments.drain(..) {
        if let Some(previous) = merged.last_mut()
            && v3_segments_can_merge(previous, &segment)
        {
            previous.query_end_ms = previous.query_end_ms.max(segment.query_end_ms);
            previous.candidate_end_ms = previous.candidate_end_ms.max(segment.candidate_end_ms);
            previous.audio_pairs += segment.audio_pairs;
            previous.video_pairs += segment.video_pairs;
            previous.weighted_score += segment.weighted_score;
            previous.residual_ms = previous.residual_ms.max(segment.residual_ms);
            previous.confidence = previous.confidence.max(segment.confidence);
            continue;
        }
        merged.push(segment);
    }
    *segments = merged;
}

fn v3_segments_can_merge(left: &V3SegmentCandidate, right: &V3SegmentCandidate) -> bool {
    left.query_end_ms <= right.query_start_ms
        && left.candidate_end_ms <= right.candidate_start_ms
        && right.query_start_ms.saturating_sub(left.query_end_ms) <= V3_SEGMENT_MERGE_GAP_MS
        && right
            .candidate_start_ms
            .saturating_sub(left.candidate_end_ms)
            <= V3_SEGMENT_MERGE_GAP_MS
        && (left.scale_ppm - right.scale_ppm).abs() <= V3_SEGMENT_MERGE_SCALE_PPM
}

fn largest_v3_segment_gap_ms(segments: &[V3SegmentCandidate]) -> u32 {
    segments
        .windows(2)
        .map(|pair| {
            pair[1]
                .query_start_ms
                .saturating_sub(pair[0].query_end_ms)
                .max(
                    pair[1]
                        .candidate_start_ms
                        .saturating_sub(pair[0].candidate_end_ms),
                )
        })
        .max()
        .unwrap_or(0)
}

fn v3_segments_are_edge_only(
    segments: &[V3SegmentCandidate],
    query_duration_ms: Option<u32>,
    candidate_duration_ms: Option<u32>,
) -> bool {
    if segments.is_empty() {
        return false;
    }
    let query_edge = v3_edge_region_ms(query_duration_ms);
    let candidate_edge = v3_edge_region_ms(candidate_duration_ms);
    segments.iter().all(|segment| {
        v3_range_is_edge_only(
            segment.query_start_ms,
            segment.query_end_ms,
            query_duration_ms,
            query_edge,
        ) && v3_range_is_edge_only(
            segment.candidate_start_ms,
            segment.candidate_end_ms,
            candidate_duration_ms,
            candidate_edge,
        )
    })
}

fn v3_edge_region_ms(duration_ms: Option<u32>) -> u32 {
    duration_ms
        .map(|duration| {
            ((f64::from(duration) * 0.15).round() as u32)
                .clamp(V3_EDGE_REGION_MIN_MS, V3_EDGE_REGION_MAX_MS)
        })
        .unwrap_or(V3_EDGE_REGION_MIN_MS)
}

fn v3_range_is_edge_only(
    start_ms: u32,
    end_ms: u32,
    duration_ms: Option<u32>,
    edge_ms: u32,
) -> bool {
    end_ms <= edge_ms
        || duration_ms.is_some_and(|duration| start_ms >= duration.saturating_sub(edge_ms))
}

fn v3_audio_video_conflict(
    _query: &MediaAnchorProfile,
    _candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
    _segments: &[V3SegmentCandidate],
) -> bool {
    let audio_offset = dominant_modality_offset_bin(pairs, AnchorModality::Audio);
    let video_offset = dominant_modality_offset_bin(pairs, AnchorModality::Video);
    matches!(
        (audio_offset, video_offset),
        (Some((audio_bin, audio_score)), Some((video_bin, video_score)))
            if audio_score >= V3_SEGMENT_AUDIO_MIN_PAIRS as u32
                && video_score >= V3_SEGMENT_VIDEO_MIN_PAIRS as u32
                && (audio_bin - video_bin).abs() * DEFAULT_ANCHOR_OFFSET_BIN_MS
                    > DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
    )
}

fn dominant_modality_offset_bin(
    pairs: &[AnchorMatchPair],
    modality: AnchorModality,
) -> Option<(i64, u32)> {
    let mut bins = HashMap::<i64, u32>::new();
    for pair in pairs.iter().filter(|pair| pair.modality == modality) {
        let offset = i64::from(pair.candidate_t_ms) - i64::from(pair.query_t_ms);
        *bins.entry(rounded_offset_bin(offset)).or_default() += u32::from(pair.weight.max(1));
    }
    bins.into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
}

fn v3_segments_have_material_timeline_change(segments: &[V3SegmentCandidate]) -> bool {
    segments.windows(2).any(|pair| {
        let left = &pair[0];
        let right = &pair[1];
        let left_offset = i64::from(left.candidate_start_ms) - i64::from(left.query_start_ms);
        let right_offset = i64::from(right.candidate_start_ms) - i64::from(right.query_start_ms);
        (left_offset - right_offset).abs() > DEFAULT_ANCHOR_ALIGNMENT_TOLERANCE_MS * 2
            || (left.scale_ppm - right.scale_ppm).abs() > V3_SEGMENT_MERGE_SCALE_PPM
    })
}

fn classify_v3_timeline(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    analysis: &V3TimelineAnalysis,
    context: V3ClassificationContext,
) -> MatchClassV3 {
    if analysis.segments.is_empty() {
        return MatchClassV3::Reject;
    }
    if analysis.edge_only {
        return MatchClassV3::SharedIntroOutroOnly;
    }
    let has_query_audio = !query.audio_anchors.is_empty();
    let has_candidate_audio = !candidate.audio_anchors.is_empty();
    let audio_strong = analysis.audio_pairs >= V3_SEGMENT_AUDIO_MIN_PAIRS;
    let video_strong = analysis.video_pairs >= V3_SEGMENT_VIDEO_MIN_PAIRS;
    if analysis.audio_video_conflict && audio_strong && analysis.audio_pairs >= analysis.video_pairs
    {
        return MatchClassV3::SameAudioDifferentVideo;
    }
    if analysis.audio_video_conflict && video_strong {
        return MatchClassV3::SameVideoDifferentAudio;
    }
    if video_strong && (!has_query_audio || !has_candidate_audio) {
        return MatchClassV3::SameVideoDifferentAudio;
    }
    let query_coverage = query
        .duration_ms
        .filter(|duration| *duration > 0)
        .map(|duration| f64::from(analysis.total_aligned_span_ms) / f64::from(duration))
        .unwrap_or(1.0);
    let candidate_coverage = candidate
        .duration_ms
        .filter(|duration| *duration > 0)
        .map(|duration| f64::from(analysis.total_aligned_span_ms) / f64::from(duration))
        .unwrap_or(1.0);
    let broad_body_coverage = query_coverage.min(candidate_coverage) >= 0.25;
    let material_segment_change = v3_segments_have_material_timeline_change(&analysis.segments);
    let clear_edit = material_segment_change
        || (context.meaningful_span && broad_body_coverage && !context.duration_ok);
    if clear_edit && analysis.total_aligned_span_ms >= 120_000 {
        return MatchClassV3::SameMediaDifferentCut;
    }
    if context.meaningful_span
        && broad_body_coverage
        && context.drift_ok
        && context.margin_ok
        && context.continuity_ok
        && (audio_strong || video_strong)
    {
        return MatchClassV3::SameCutStrong;
    }
    if context.meaningful_span
        && broad_body_coverage
        && analysis.total_aligned_span_ms >= 120_000
        && (audio_strong || video_strong)
    {
        return MatchClassV3::SameCutProbable;
    }
    if analysis.total_aligned_span_ms >= 45_000 {
        return MatchClassV3::PartialOverlap;
    }
    MatchClassV3::Reject
}

fn media_timeline_map_v3_from_analysis(
    global_class: MatchClassV3,
    analysis: &V3TimelineAnalysis,
) -> MediaTimelineMapV3 {
    let segments = analysis
        .segments
        .iter()
        .map(|segment| {
            let total_pairs = (segment.audio_pairs + segment.video_pairs).max(1) as f32;
            AlignedSegmentV3 {
                query_start_ms: segment.query_start_ms,
                query_end_ms: segment.query_end_ms,
                candidate_start_ms: segment.candidate_start_ms,
                candidate_end_ms: segment.candidate_end_ms,
                scale_ppm: segment.scale_ppm,
                audio_pairs: segment.audio_pairs,
                video_pairs: segment.video_pairs,
                weighted_score: segment.weighted_score,
                residual_ms: segment.residual_ms,
                audio_score: segment.audio_pairs as f32 / total_pairs,
                video_score: segment.video_pairs as f32 / total_pairs,
                confidence: segment.confidence,
            }
        })
        .collect();
    MediaTimelineMapV3 {
        global_class,
        current_position_class: global_class,
        segments,
        total_aligned_span_ms: analysis.total_aligned_span_ms,
        largest_gap_ms: analysis.largest_gap_ms,
        edge_only: analysis.edge_only,
        audio_video_conflict: analysis.audio_video_conflict,
        best_segment_score: analysis.best_segment_score,
        second_best_segment_score: analysis.second_best_segment_score,
        piecewise_pair_count: analysis.piecewise_pair_count,
        piecewise_hypothesis_count: analysis.piecewise_hypothesis_count,
        piecewise_segment_candidate_count: analysis.piecewise_segment_candidate_count,
        piecewise_segment_chain_count: analysis.piecewise_segment_chain_count,
        piecewise_fit_millis: analysis.piecewise_fit_millis,
    }
}

fn anchor_coverage(aligned: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        aligned as f64 / total as f64
    }
}

fn unique_video_frame_anchor_count(anchors: &[VideoAnchor]) -> usize {
    anchors
        .iter()
        .map(|anchor| (anchor.t_ms, anchor.kind, anchor.hash64))
        .collect::<HashSet<_>>()
        .len()
}

fn aligned_anchor_bounds(pairs: &[AnchorMatchPair]) -> (u32, u32, u32, u32) {
    let first_query = pairs.iter().map(|pair| pair.query_t_ms).min().unwrap_or(0);
    let last_query = pairs.iter().map(|pair| pair.query_t_ms).max().unwrap_or(0);
    let first_candidate = pairs
        .iter()
        .map(|pair| pair.candidate_t_ms)
        .min()
        .unwrap_or(0);
    let last_candidate = pairs
        .iter()
        .map(|pair| pair.candidate_t_ms)
        .max()
        .unwrap_or(0);
    (first_query, last_query, first_candidate, last_candidate)
}

fn aligned_anchor_span_ms(pairs: &[AnchorMatchPair]) -> u32 {
    let (first_query, last_query, _, _) = aligned_anchor_bounds(pairs);
    last_query.saturating_sub(first_query)
}

fn candidate_anchor_span_ms(pairs: &[AnchorMatchPair]) -> u32 {
    let (_, _, first_candidate, last_candidate) = aligned_anchor_bounds(pairs);
    last_candidate.saturating_sub(first_candidate)
}

fn audio_pair_is_body(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pair: AnchorMatchPair,
) -> bool {
    !v3_time_is_edge(pair.query_t_ms, query.duration_ms)
        && !v3_time_is_edge(pair.candidate_t_ms, candidate.duration_ms)
}

fn v3_time_is_edge(t_ms: u32, duration_ms: Option<u32>) -> bool {
    let edge_ms = v3_edge_region_ms(duration_ms);
    t_ms <= edge_ms || duration_ms.is_some_and(|duration| t_ms >= duration.saturating_sub(edge_ms))
}

fn audio_edge_span_ms(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    pairs: &[AnchorMatchPair],
) -> u32 {
    let edge_pairs = pairs
        .iter()
        .copied()
        .filter(|pair| !audio_pair_is_body(query, candidate, *pair))
        .collect::<Vec<_>>();
    aligned_anchor_span_ms(&edge_pairs)
}

fn largest_query_gap_ms(pairs: &[AnchorMatchPair]) -> u32 {
    if pairs.len() < 2 {
        return 0;
    }
    let mut times = pairs.iter().map(|pair| pair.query_t_ms).collect::<Vec<_>>();
    times.sort_unstable();
    times.dedup();
    times
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .max()
        .unwrap_or(0)
}

fn aligned_anchor_largest_gap_ratio(pairs: &[AnchorMatchPair]) -> f64 {
    if pairs.len() < 3 {
        return 1.0;
    }
    let mut times = pairs.iter().map(|pair| pair.query_t_ms).collect::<Vec<_>>();
    times.sort_unstable();
    times.dedup();
    if times.len() < 3 {
        return 1.0;
    }
    let span = times[times.len() - 1].saturating_sub(times[0]).max(1);
    let largest_gap = times
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .max()
        .unwrap_or(span);
    largest_gap as f64 / span as f64
}

fn metadata_evidence(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MetadataMatchEvidence {
    let duration_delta_seconds = query
        .duration_seconds
        .zip(candidate.duration_seconds)
        .map(|(left, right)| (left - right).abs());
    MetadataMatchEvidence {
        same_normalized_path: query.identity.normalized_path == candidate.identity.normalized_path,
        same_size: Some(query.identity.size_bytes == candidate.identity.size_bytes),
        duration_delta_seconds,
        duration_within_tolerance: duration_delta_seconds.map(|delta| {
            !settings.runtime_tolerance_enabled || delta <= settings.runtime_tolerance_seconds
        }),
        extension_match: extension(&query.identity.normalized_path)
            .zip(extension(&candidate.identity.normalized_path))
            .map(|(left, right)| left == right),
    }
}

fn extension(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(_, extension)| extension)
}

fn decision(
    tier: MediaMatchTier,
    mut evidence: MediaMatchEvidence,
    explanation: impl Into<String>,
) -> MediaMatchDecision {
    let timeline_map = evidence
        .timeline_map_v3
        .take()
        .unwrap_or_else(|| media_timeline_map_v3_from_evidence(tier, &evidence));
    evidence.v3_class = Some(timeline_map.global_class);
    evidence.timeline_map_v3 = Some(timeline_map);
    MediaMatchDecision {
        tier,
        evidence,
        explanation: explanation.into(),
    }
}

fn media_timeline_map_v3_from_evidence(
    tier: MediaMatchTier,
    evidence: &MediaMatchEvidence,
) -> MediaTimelineMapV3 {
    // Fallback for exact/non-anchor decisions; anchor decisions provide a true piecewise map.
    let global_class = media_match_class_v3_from_evidence(tier, evidence);
    let segments: Vec<AlignedSegmentV3> = evidence
        .alignment
        .as_ref()
        .map(|alignment| {
            let audio_score = evidence
                .audio
                .as_ref()
                .map(|audio| audio.similarity as f32)
                .unwrap_or(0.0);
            let video_score = evidence
                .video
                .as_ref()
                .map(|video| video.query_coverage.min(video.candidate_coverage) as f32)
                .unwrap_or(0.0);
            let confidence = match tier {
                MediaMatchTier::Exact | MediaMatchTier::Strong => 1.0,
                MediaMatchTier::Probable => 0.72,
                MediaMatchTier::Weak => 0.35,
                MediaMatchTier::Reject | MediaMatchTier::Unknown => 0.0,
            };
            AlignedSegmentV3 {
                query_start_ms: seconds_to_u32_ms(alignment.first_query_second),
                query_end_ms: seconds_to_u32_ms(alignment.last_query_second),
                candidate_start_ms: seconds_to_u32_ms(alignment.first_candidate_second),
                candidate_end_ms: seconds_to_u32_ms(alignment.last_candidate_second),
                scale_ppm: alignment.scale_ppm,
                audio_pairs: alignment.aligned_audio_anchors,
                video_pairs: alignment.aligned_video_anchors,
                weighted_score: alignment.aligned_pairs as u32,
                residual_ms: alignment.drift_ratio * alignment.aligned_span_seconds * 1000.0,
                audio_score,
                video_score,
                confidence,
            }
        })
        .into_iter()
        .collect();
    let total_aligned_span_ms = segments
        .iter()
        .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
        .sum();
    let best_segment_score = segments
        .iter()
        .map(|segment| segment.weighted_score)
        .max()
        .unwrap_or(0);
    let segment_count = segments.len();
    MediaTimelineMapV3 {
        global_class,
        current_position_class: global_class,
        segments,
        total_aligned_span_ms,
        largest_gap_ms: 0,
        edge_only: false,
        audio_video_conflict: false,
        best_segment_score,
        second_best_segment_score: 0,
        piecewise_pair_count: 0,
        piecewise_hypothesis_count: 0,
        piecewise_segment_candidate_count: 0,
        piecewise_segment_chain_count: segment_count,
        piecewise_fit_millis: 0,
    }
}

fn media_match_class_v3_from_evidence(
    tier: MediaMatchTier,
    evidence: &MediaMatchEvidence,
) -> MatchClassV3 {
    match tier {
        MediaMatchTier::Exact | MediaMatchTier::Strong => {
            if evidence
                .metadata
                .duration_within_tolerance
                .is_some_and(|value| !value)
            {
                MatchClassV3::SameMediaDifferentCut
            } else if evidence.video.is_some() && evidence.audio.is_none() {
                MatchClassV3::SameVideoDifferentAudio
            } else {
                MatchClassV3::SameCutStrong
            }
        }
        MediaMatchTier::Probable => {
            if evidence
                .metadata
                .duration_within_tolerance
                .is_some_and(|value| !value)
            {
                MatchClassV3::SameMediaDifferentCut
            } else {
                MatchClassV3::SameCutProbable
            }
        }
        MediaMatchTier::Weak => MatchClassV3::PartialOverlap,
        MediaMatchTier::Reject => {
            if evidence
                .alignment
                .as_ref()
                .is_some_and(|alignment| alignment.aligned_span_seconds < 120.0)
            {
                MatchClassV3::SharedIntroOutroOnly
            } else {
                MatchClassV3::Reject
            }
        }
        MediaMatchTier::Unknown => MatchClassV3::Unknown,
    }
}

fn seconds_to_u32_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round().min(f64::from(u32::MAX)) as u32
}
