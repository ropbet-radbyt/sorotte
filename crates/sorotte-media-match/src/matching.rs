use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::{
    anchors::{AudioAnchor, MediaAnchorProfile, media_anchor_profile_from_record},
    identity::duration_seconds_to_millis,
    tuning::DEFAULT_ANCHOR_OFFSET_BIN_MS,
    types::{
        AlignedSegmentV3, AudioMatchEvidence, MatchClassV3, MediaDurationCompatibility,
        MediaFingerprintRecord, MediaMatchDecision, MediaMatchEvidence, MediaMatchSettings,
        MediaMatchTier, MediaTimelineAlignment, MediaTimelineMapV3, MetadataMatchEvidence,
        media_duration_compatibility_ms, media_duration_ratio_ms,
    },
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
                media_match_class_rank(right.decision.evidence.v3_class)
                    .cmp(&media_match_class_rank(left.decision.evidence.v3_class))
            })
            .then_with(|| {
                media_match_candidate_aligned_pairs(&right.decision)
                    .cmp(&media_match_candidate_aligned_pairs(&left.decision))
            })
            .then_with(|| {
                media_match_duration_rank(
                    right
                        .decision
                        .evidence
                        .metadata
                        .duration_compatibility
                        .unwrap_or(MediaDurationCompatibility::Unknown),
                )
                .cmp(&media_match_duration_rank(
                    left.decision
                        .evidence
                        .metadata
                        .duration_compatibility
                        .unwrap_or(MediaDurationCompatibility::Unknown),
                ))
            })
            .then_with(|| {
                media_match_filename_similarity(&right.decision)
                    .total_cmp(&media_match_filename_similarity(&left.decision))
            })
            .then_with(|| {
                media_match_same_size_rank(&right.decision)
                    .cmp(&media_match_same_size_rank(&left.decision))
            })
            .then_with(|| left.candidate_path.cmp(&right.candidate_path))
    });
    decisions
}

fn media_match_class_rank(class: Option<MatchClassV3>) -> u8 {
    match class.unwrap_or(MatchClassV3::Unknown) {
        MatchClassV3::SameCutStrong => 5,
        MatchClassV3::SameCutProbable => 4,
        MatchClassV3::SameMediaDifferentCut => 3,
        MatchClassV3::PartialOverlap => 2,
        MatchClassV3::SharedIntroOutroOnly => 1,
        MatchClassV3::Reject | MatchClassV3::Unknown => 0,
    }
}

fn media_match_duration_rank(compatibility: MediaDurationCompatibility) -> u8 {
    match compatibility {
        MediaDurationCompatibility::SameCutCompatible => 4,
        MediaDurationCompatibility::NearCompatible => 3,
        MediaDurationCompatibility::Unknown => 2,
        MediaDurationCompatibility::ContainedOrPartial => 1,
        MediaDurationCompatibility::IncompatibleSameCut => 0,
    }
}

fn media_match_filename_similarity(decision: &MediaMatchDecision) -> f64 {
    decision
        .evidence
        .metadata
        .filename_stem_similarity
        .unwrap_or(0.0)
}

fn media_match_same_size_rank(decision: &MediaMatchDecision) -> u8 {
    u8::from(decision.evidence.metadata.same_size.unwrap_or(false))
}

fn media_match_candidate_aligned_pairs(decision: &MediaMatchDecision) -> usize {
    decision
        .evidence
        .alignment
        .as_ref()
        .map(|alignment| alignment.aligned_pairs)
        .unwrap_or(0)
}

pub fn decide_media_match(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let mut evidence = MediaMatchEvidence {
        metadata: metadata_evidence(query, candidate, settings),
        ..MediaMatchEvidence::default()
    };

    if query.identity.normalized_path == candidate.identity.normalized_path {
        push_metadata_notes(&mut evidence);
        evidence.v3_class = Some(MatchClassV3::SameCutStrong);
        evidence.notes.push("same normalized path".to_owned());
        return MediaMatchDecision {
            tier: MediaMatchTier::Exact,
            evidence,
            explanation: "same normalized media path".to_owned(),
        };
    }

    let query_profile = media_anchor_profile_from_record(query);
    let candidate_profile = media_anchor_profile_from_record(candidate);
    decide_media_match_anchors_with_evidence(query_profile, candidate_profile, evidence)
}

pub(crate) fn decide_media_match_anchors(
    query: &MediaAnchorProfile,
    candidate: &MediaAnchorProfile,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    decide_media_match_anchors_with_evidence(
        query.clone(),
        candidate.clone(),
        MediaMatchEvidence {
            metadata: metadata_evidence_for_duration_ms(
                query.duration_ms,
                candidate.duration_ms,
                settings,
            ),
            ..MediaMatchEvidence::default()
        },
    )
}

fn decide_media_match_anchors_with_evidence(
    query: MediaAnchorProfile,
    candidate: MediaAnchorProfile,
    mut evidence: MediaMatchEvidence,
) -> MediaMatchDecision {
    let Some(alignment) = align_audio_anchors(&query.audio_anchors, &candidate.audio_anchors)
    else {
        evidence.v3_class = Some(MatchClassV3::Reject);
        evidence
            .notes
            .push("no coherent sampled-fast audio offset".to_owned());
        return MediaMatchDecision {
            tier: MediaMatchTier::Reject,
            evidence,
            explanation: "sampled-fast audio did not align".to_owned(),
        };
    };

    let query_count = unique_audio_anchor_count(&query.audio_anchors).max(1);
    let candidate_count = unique_audio_anchor_count(&candidate.audio_anchors).max(1);
    let shared_anchor_ratio =
        alignment.aligned_audio_anchors as f64 / query_count.min(candidate_count).max(1) as f64;
    evidence.audio = Some(AudioMatchEvidence {
        similarity: shared_anchor_ratio,
        shared_anchor_ratio,
        duration_delta_seconds: query
            .duration_ms
            .zip(candidate.duration_ms)
            .map(|(left, right)| (f64::from(left) - f64::from(right)).abs() / 1000.0),
    });
    evidence.alignment = Some(alignment.clone());
    let audio_class = classify_sampled_audio_alignment(&alignment);
    push_metadata_notes(&mut evidence);
    let class = apply_duration_compatibility(audio_class, &alignment, &mut evidence);
    evidence.v3_class = Some(class);
    evidence.timeline_map_v3 = Some(timeline_map_from_alignment(&alignment, class));
    evidence.notes.push(format!(
        "sampled-fast audio aligned pairs={} span={:.1}s offset={:.3}s margin={:.2}",
        alignment.aligned_pairs,
        alignment.aligned_span_seconds,
        alignment.offset_seconds,
        alignment.second_best_offset_margin
    ));

    let tier = match class {
        MatchClassV3::SameCutStrong => MediaMatchTier::Probable,
        MatchClassV3::SameCutProbable => MediaMatchTier::Probable,
        MatchClassV3::SameMediaDifferentCut
        | MatchClassV3::PartialOverlap
        | MatchClassV3::SharedIntroOutroOnly => MediaMatchTier::Weak,
        MatchClassV3::Reject | MatchClassV3::Unknown => MediaMatchTier::Reject,
    };
    MediaMatchDecision {
        tier,
        evidence,
        explanation: match tier {
            MediaMatchTier::Probable => "sampled-fast audio suggests same media; full autoplay verification is not available".to_owned(),
            MediaMatchTier::Weak => "sampled-fast audio found only partial/shared evidence".to_owned(),
            _ => "sampled-fast audio rejected candidate".to_owned(),
        },
    }
}

fn metadata_evidence(
    query: &MediaFingerprintRecord,
    candidate: &MediaFingerprintRecord,
    settings: &MediaMatchSettings,
) -> MetadataMatchEvidence {
    let query_duration_ms = query.duration_seconds.and_then(duration_seconds_to_millis);
    let candidate_duration_ms = candidate
        .duration_seconds
        .and_then(duration_seconds_to_millis);
    let duration_delta_seconds = query
        .duration_seconds
        .zip(candidate.duration_seconds)
        .map(|(left, right)| (left - right).abs());
    MetadataMatchEvidence {
        same_normalized_path: query.identity.normalized_path == candidate.identity.normalized_path,
        same_size: Some(query.identity.size_bytes == candidate.identity.size_bytes),
        duration_delta_seconds,
        duration_within_tolerance: duration_delta_seconds
            .map(|delta| delta <= settings.runtime_tolerance_seconds),
        extension_match: Some(
            media_path_extension(&query.identity.normalized_path)
                == media_path_extension(&candidate.identity.normalized_path),
        ),
        duration_compatibility: Some(media_duration_compatibility_ms(
            query_duration_ms,
            candidate_duration_ms,
        )),
        duration_ratio: media_duration_ratio_ms(query_duration_ms, candidate_duration_ms),
        filename_stem_similarity: Some(filename_stem_similarity(
            &query.identity.normalized_path,
            &candidate.identity.normalized_path,
        )),
    }
}

fn metadata_evidence_for_duration_ms(
    query_duration_ms: Option<u32>,
    candidate_duration_ms: Option<u32>,
    settings: &MediaMatchSettings,
) -> MetadataMatchEvidence {
    let duration_delta_seconds = query_duration_ms
        .zip(candidate_duration_ms)
        .map(|(left, right)| f64::from(left.abs_diff(right)) / 1000.0);
    MetadataMatchEvidence {
        same_normalized_path: false,
        same_size: None,
        duration_delta_seconds,
        duration_within_tolerance: duration_delta_seconds
            .map(|delta| delta <= settings.runtime_tolerance_seconds),
        extension_match: None,
        duration_compatibility: Some(media_duration_compatibility_ms(
            query_duration_ms,
            candidate_duration_ms,
        )),
        duration_ratio: media_duration_ratio_ms(query_duration_ms, candidate_duration_ms),
        filename_stem_similarity: None,
    }
}

fn push_metadata_notes(evidence: &mut MediaMatchEvidence) {
    if let Some(compatibility) = evidence.metadata.duration_compatibility {
        match compatibility {
            MediaDurationCompatibility::SameCutCompatible => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration compatible delta={}s",
                        format_seconds(delta)
                    ));
                }
            }
            MediaDurationCompatibility::NearCompatible => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration near-compatible delta={}s",
                        format_seconds(delta)
                    ));
                }
            }
            MediaDurationCompatibility::ContainedOrPartial => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration suggests contained/partial overlap delta={}s ratio={}",
                        format_seconds(delta),
                        format_ratio(evidence.metadata.duration_ratio)
                    ));
                }
            }
            MediaDurationCompatibility::IncompatibleSameCut
            | MediaDurationCompatibility::Unknown => {}
        }
    }
    if let Some(false) = evidence.metadata.extension_match {
        evidence.notes.push("extension mismatch".to_owned());
    } else if let Some(true) = evidence.metadata.extension_match {
        evidence.notes.push("extension match".to_owned());
    }
    if let Some(similarity) = evidence.metadata.filename_stem_similarity {
        evidence
            .notes
            .push(format!("filename stem similarity={similarity:.2}"));
    }
}

fn align_audio_anchors(
    query: &[AudioAnchor],
    candidate: &[AudioAnchor],
) -> Option<MediaTimelineAlignment> {
    if query.is_empty() || candidate.is_empty() {
        return None;
    }
    let mut candidate_by_bucket = HashMap::<u32, Vec<&AudioAnchor>>::new();
    for anchor in candidate {
        candidate_by_bucket
            .entry(anchor.bucket)
            .or_default()
            .push(anchor);
    }

    let mut offset_bins = BTreeMap::<i64, OffsetBin>::new();
    let mut seen = HashSet::<(u32, u32, u32)>::new();
    for query_anchor in query {
        let Some(candidate_anchors) = candidate_by_bucket.get(&query_anchor.bucket) else {
            continue;
        };
        for candidate_anchor in candidate_anchors {
            if !seen.insert((
                query_anchor.bucket,
                query_anchor.t_ms,
                candidate_anchor.t_ms,
            )) {
                continue;
            }
            let offset = i64::from(candidate_anchor.t_ms) - i64::from(query_anchor.t_ms);
            let bin = rounded_offset_bin(offset);
            let weight = u32::from(query_anchor.weight.min(candidate_anchor.weight).max(1));
            let entry = offset_bins.entry(bin).or_default();
            entry.score = entry.score.saturating_add(weight);
            entry.pair_count += 1;
            entry.query_times.insert(query_anchor.t_ms);
            entry.candidate_times.insert(candidate_anchor.t_ms);
        }
    }

    let mut bins = offset_bins.into_iter().collect::<Vec<_>>();
    bins.sort_by(|(left_bin, left), (right_bin, right)| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.pair_count.cmp(&left.pair_count))
            .then_with(|| left_bin.cmp(right_bin))
    });
    let (best_bin, best) = bins.first()?.clone();
    let second_score = bins.get(1).map(|(_, bin)| bin.score).unwrap_or(0);
    if best.pair_count < 8 || best.score < 12 {
        return None;
    }
    let query_start = *best.query_times.first().unwrap_or(&0);
    let query_end = *best.query_times.last().unwrap_or(&query_start);
    let candidate_start = *best.candidate_times.first().unwrap_or(&0);
    let candidate_end = *best.candidate_times.last().unwrap_or(&candidate_start);
    let span_ms = query_end
        .saturating_sub(query_start)
        .max(candidate_end.saturating_sub(candidate_start));
    let margin = if second_score == 0 {
        best.score as f64
    } else {
        best.score as f64 / second_score.max(1) as f64
    };
    Some(MediaTimelineAlignment {
        offset_seconds: best_bin as f64 / 1000.0,
        scale_ppm: 0,
        drift_ratio: 0.0,
        aligned_pairs: best.pair_count,
        aligned_audio_anchors: best.pair_count,
        aligned_span_seconds: f64::from(span_ms) / 1000.0,
        second_best_offset_margin: margin,
        first_query_second: f64::from(query_start) / 1000.0,
        last_query_second: f64::from(query_end) / 1000.0,
        first_candidate_second: f64::from(candidate_start) / 1000.0,
        last_candidate_second: f64::from(candidate_end) / 1000.0,
    })
}

#[derive(Debug, Clone, Default)]
struct OffsetBin {
    score: u32,
    pair_count: usize,
    query_times: BTreeSet<u32>,
    candidate_times: BTreeSet<u32>,
}

fn rounded_offset_bin(offset_ms: i64) -> i64 {
    let bin = DEFAULT_ANCHOR_OFFSET_BIN_MS.max(1);
    ((offset_ms + bin / 2).div_euclid(bin)) * bin
}

fn classify_sampled_audio_alignment(alignment: &MediaTimelineAlignment) -> MatchClassV3 {
    let enough_span = alignment.aligned_span_seconds >= 20.0;
    let enough_pairs = alignment.aligned_pairs >= 16;
    let margin_ok = alignment.second_best_offset_margin >= 1.15;
    if enough_span && enough_pairs && margin_ok {
        MatchClassV3::SameCutProbable
    } else if enough_pairs && alignment.aligned_span_seconds >= 5.0 {
        MatchClassV3::PartialOverlap
    } else {
        MatchClassV3::SharedIntroOutroOnly
    }
}

fn apply_duration_compatibility(
    class: MatchClassV3,
    alignment: &MediaTimelineAlignment,
    evidence: &mut MediaMatchEvidence,
) -> MatchClassV3 {
    let compatibility = evidence
        .metadata
        .duration_compatibility
        .unwrap_or(MediaDurationCompatibility::Unknown);
    match compatibility {
        MediaDurationCompatibility::Unknown | MediaDurationCompatibility::SameCutCompatible => {
            class
        }
        MediaDurationCompatibility::NearCompatible => {
            if class == MatchClassV3::SameCutProbable
                && sampled_audio_strong_enough_for_near_duration(alignment)
            {
                class
            } else if class == MatchClassV3::SameCutProbable {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration near mismatch delta={}s downgraded same-cut audio to different-cut",
                        format_seconds(delta)
                    ));
                }
                MatchClassV3::SameMediaDifferentCut
            } else {
                class
            }
        }
        MediaDurationCompatibility::ContainedOrPartial => match class {
            MatchClassV3::SameCutStrong
            | MatchClassV3::SameCutProbable
            | MatchClassV3::SameMediaDifferentCut
            | MatchClassV3::PartialOverlap => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration contained/partial delta={}s downgraded same-cut audio to partial-overlap",
                        format_seconds(delta)
                    ));
                }
                MatchClassV3::PartialOverlap
            }
            MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject | MatchClassV3::Unknown => {
                class
            }
        },
        MediaDurationCompatibility::IncompatibleSameCut => match class {
            MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration mismatch delta={}s downgraded same-cut audio to partial-overlap",
                        format_seconds(delta)
                    ));
                }
                MatchClassV3::PartialOverlap
            }
            MatchClassV3::SameMediaDifferentCut
            | MatchClassV3::PartialOverlap
            | MatchClassV3::SharedIntroOutroOnly => {
                if let Some(delta) = evidence.metadata.duration_delta_seconds {
                    evidence.notes.push(format!(
                        "duration mismatch delta={}s rejected weak sampled-fast audio",
                        format_seconds(delta)
                    ));
                }
                MatchClassV3::Reject
            }
            MatchClassV3::Reject | MatchClassV3::Unknown => class,
        },
    }
}

fn sampled_audio_strong_enough_for_near_duration(alignment: &MediaTimelineAlignment) -> bool {
    alignment.aligned_span_seconds >= 20.0
        && alignment.aligned_pairs >= 24
        && alignment.second_best_offset_margin >= 1.15
}

fn timeline_map_from_alignment(
    alignment: &MediaTimelineAlignment,
    class: MatchClassV3,
) -> MediaTimelineMapV3 {
    const UNITY_SCALE_PPM: i64 = 1_000_000;

    let query_start_ms = (alignment.first_query_second * 1000.0)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32;
    let candidate_start_ms = (alignment.first_candidate_second * 1000.0)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32;
    let span_ms = (alignment.aligned_span_seconds * 1000.0)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32;
    let affine_scale_ppm = i32::try_from(UNITY_SCALE_PPM + i64::from(alignment.scale_ppm))
        .ok()
        .filter(|scale_ppm| *scale_ppm > 0)
        .unwrap_or(0);
    MediaTimelineMapV3 {
        global_class: class,
        current_position_class: class,
        segments: vec![AlignedSegmentV3 {
            query_start_ms,
            query_end_ms: query_start_ms.saturating_add(span_ms),
            candidate_start_ms,
            candidate_end_ms: candidate_start_ms.saturating_add(span_ms),
            scale_ppm: affine_scale_ppm,
            audio_pairs: alignment.aligned_audio_anchors,
            weighted_score: alignment.aligned_audio_anchors as u32,
            residual_ms: 0.0,
            audio_score: 1.0,
            confidence: 0.75,
        }],
        total_aligned_span_ms: span_ms,
        largest_gap_ms: 0,
        edge_only: false,
        best_segment_score: alignment.aligned_audio_anchors as u32,
        second_best_segment_score: 0,
    }
}

fn unique_audio_anchor_count(anchors: &[AudioAnchor]) -> usize {
    anchors
        .iter()
        .map(|anchor| (anchor.bucket, anchor.t_ms))
        .collect::<HashSet<_>>()
        .len()
}

fn media_path_extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).trim();
    let dot = name.rfind('.')?;
    (dot + 1 < name.len()).then(|| name[dot + 1..].to_ascii_lowercase())
}

fn filename_stem_similarity(left: &str, right: &str) -> f64 {
    let left_episode = episode_token_from_stem(left);
    let right_episode = episode_token_from_stem(right);
    let left_stem = normalized_filename_stem(left);
    let right_stem = normalized_filename_stem(right);
    let mut score = if left_stem == right_stem && !left_stem.is_empty() {
        1.0
    } else {
        token_jaccard_similarity(&left_stem, &right_stem)
    };
    if left_episode.is_some() && right_episode.is_some() && left_episode != right_episode {
        score = score.min(0.2);
    }
    score
}

fn normalized_filename_stem(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = name
        .rfind('.')
        .filter(|dot| *dot > 0)
        .map(|dot| &name[..dot])
        .unwrap_or(name);
    let mut cleaned = String::with_capacity(stem.len());
    let mut bracket_depth = 0u8;
    for character in stem.to_ascii_lowercase().chars() {
        match character {
            '[' | '(' | '{' => bracket_depth = bracket_depth.saturating_add(1),
            ']' | ')' | '}' => bracket_depth = bracket_depth.saturating_sub(1),
            _ if bracket_depth > 0 => {}
            '.' | '_' | '-' => cleaned.push(' '),
            _ => cleaned.push(character),
        }
    }
    cleaned
        .split_whitespace()
        .filter(|token| !is_filename_noise_token(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_filename_noise_token(token: &str) -> bool {
    matches!(
        token,
        "480p"
            | "720p"
            | "1080p"
            | "2160p"
            | "x264"
            | "x265"
            | "h264"
            | "h265"
            | "hevc"
            | "bluray"
            | "web"
            | "webrip"
            | "webdl"
            | "dl"
            | "hdr"
            | "dv"
            | "aac"
            | "flac"
    )
}

fn token_jaccard_similarity(left: &str, right: &str) -> f64 {
    let left_tokens = left.split_whitespace().collect::<BTreeSet<_>>();
    let right_tokens = right.split_whitespace().collect::<BTreeSet<_>>();
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count();
    intersection as f64 / union.max(1) as f64
}

fn episode_token_from_stem(path: &str) -> Option<(u32, u32)> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = name
        .rfind('.')
        .filter(|dot| *dot > 0)
        .map(|dot| &name[..dot])
        .unwrap_or(name)
        .to_ascii_lowercase();
    let bytes = stem.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b's'
            && let Some((season, after_season)) = parse_digits(bytes, index + 1, 2)
            && after_season < bytes.len()
            && bytes[after_season] == b'e'
            && let Some((episode, _)) = parse_digits(bytes, after_season + 1, 2)
        {
            return Some((season, episode));
        }
        if bytes[index].is_ascii_digit()
            && (index == 0 || !bytes[index - 1].is_ascii_digit())
            && let Some((season, after_season)) = parse_digits(bytes, index, 2)
            && after_season < bytes.len()
            && bytes[after_season] == b'x'
            && let Some((episode, _)) = parse_digits(bytes, after_season + 1, 2)
        {
            return Some((season, episode));
        }
    }
    None
}

fn parse_digits(bytes: &[u8], start: usize, max_len: usize) -> Option<(u32, usize)> {
    let mut value = 0u32;
    let mut end = start;
    while end < bytes.len() && end - start < max_len && bytes[end].is_ascii_digit() {
        value = value * 10 + u32::from(bytes[end] - b'0');
        end += 1;
    }
    (end > start).then_some((value, end))
}

fn format_seconds(seconds: f64) -> String {
    if seconds >= 100.0 {
        format!("{seconds:.0}")
    } else {
        format!("{seconds:.1}")
    }
}

fn format_ratio(ratio: Option<f64>) -> String {
    ratio
        .map(|ratio| format!("{ratio:.2}"))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchors(offset: u32) -> Vec<AudioAnchor> {
        (0..32)
            .map(|index| AudioAnchor {
                bucket: 10_000 + index,
                t_ms: index * 2_000 + offset,
                weight: 8,
            })
            .collect()
    }

    #[test]
    fn sampled_audio_alignment_produces_probable_not_strong() {
        let query = anchors(0);
        let candidate = anchors(1_000);
        let alignment = align_audio_anchors(&query, &candidate).expect("aligned");

        assert_eq!(alignment.aligned_pairs, 32);
        assert!(alignment.aligned_span_seconds >= 20.0);
    }

    #[test]
    fn weak_shared_audio_is_rejected_when_offset_is_not_coherent() {
        let query = anchors(0);
        let mut candidate = anchors(1_000);
        for (index, anchor) in candidate.iter_mut().enumerate() {
            anchor.t_ms = anchor.t_ms.saturating_add(index as u32 * 700);
        }

        assert!(align_audio_anchors(&query, &candidate).is_none());
    }

    #[test]
    fn exact_path_is_autoplay_eligible_only_through_exact_tier() {
        let record = MediaFingerprintRecord {
            identity: crate::MediaFileIdentity::new("a.mkv", 1, 2),
            algorithm_version: crate::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: crate::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(120.0),
            container_fingerprint: "container".to_owned(),
            audio_anchors: anchors(0),
            audio_error: None,
        };
        let settings = MediaMatchSettings {
            autoplay_policy: crate::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ..MediaMatchSettings::default()
        };

        let decision = decide_media_match(&record, &record, &settings);

        assert_eq!(decision.tier, MediaMatchTier::Exact);
        assert!(decision.same_media_for_autoplay(&settings));
    }
}
