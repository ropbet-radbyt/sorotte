use serde::{Deserialize, Serialize};

use crate::{
    InstrumentedMediaFingerprint, MatchClassV3, MediaFingerprintExtractionReport,
    MediaFingerprintRecord, MediaMatchDecision, MediaMatchTier,
    anchors::{
        MediaFingerprintBlobV3, audio_index_landmarks_v3_from_record,
        audio_landmarks_v3_from_record, encode_media_fingerprint_blob_v3,
    },
    identity::duration_seconds_to_millis,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummary {
    pub file_path: Option<String>,
    pub profile: String,
    pub index_quality: String,
    pub duration_ms: Option<u32>,
    pub extraction_total_millis: Option<u128>,
    pub extraction_audio_millis: Option<u128>,
    pub audio_verify_count: usize,
    pub audio_index_count: usize,
    pub audio_blob_bytes: usize,
    pub retrieval_candidates_count: Option<usize>,
    pub decision_tier: Option<MediaMatchTier>,
    pub decision_class: Option<MatchClassV3>,
    pub source_path_root: Option<String>,
    pub source_path_kind: Option<String>,
    pub source_volume_id: Option<String>,
    pub ffmpeg_command_kind: Option<String>,
    pub ffmpeg_selected_stream: Option<String>,
    pub ffmpeg_disabled_non_audio_streams: bool,
    pub ffmpeg_disabled_subtitles: bool,
    pub ffmpeg_disabled_data: bool,
    pub streamed_bytes: Option<usize>,
    pub streamed_samples: Option<usize>,
    pub peak_frames: Option<usize>,
    pub raw_landmarks_emitted: Option<usize>,
    pub raw_landmarks_before_bounding: Option<usize>,
    pub raw_landmarks_kept_before_final: Option<usize>,
    pub final_landmarks: Option<usize>,
    pub max_buffer_samples: Option<usize>,
    pub raw_landmark_compactions: Option<usize>,
    pub ffmpeg_process_wall_millis: Option<u128>,
    pub ffmpeg_input_read_bytes: Option<u64>,
    pub ffmpeg_input_read_ops: Option<u64>,
    pub ffmpeg_output_pcm_bytes: Option<u64>,
    pub read_amplification_ratio: Option<f64>,
    pub ffmpeg_invocation_count: Option<usize>,
    pub sampled_window_decode_millis: Option<u128>,
    pub ffmpeg_exit_millis: Option<u128>,
    pub pcm_decode_drain_millis: Option<u128>,
    pub ffmpeg_decode_stream_millis: Option<u128>,
    pub analyzer_millis: Option<u128>,
    pub peak_selection_millis: Option<u128>,
    pub pairing_millis: Option<u128>,
    pub reservoir_millis: Option<u128>,
    pub final_selection_millis: Option<u128>,
    pub candidate_pairs_considered: Option<usize>,
    pub candidate_pairs_skipped_by_anchor_gate: Option<usize>,
    pub candidate_pairs_skipped_by_target_gate: Option<usize>,
    pub candidate_pairs_skipped_by_saturation: Option<usize>,
    pub candidate_pairs_emitted: Option<usize>,
    pub landmarks_accepted_into_reservoir: Option<usize>,
    pub landmarks_rejected_by_reservoir: Option<usize>,
    pub reservoir_acceptance_ratio: Option<f64>,
    pub sampled_audio_seconds_decoded: Option<u32>,
    pub sampled_audio_windows_decoded: Option<usize>,
    pub notes: Vec<String>,
}

pub fn summarize_record_v3_diagnostics(
    record: &MediaFingerprintRecord,
) -> MediaMatchV3DiagnosticSummary {
    summarize_record_v3_diagnostics_with_report(record, None)
}

pub fn summarize_instrumented_record_v3_diagnostics(
    fingerprint: &InstrumentedMediaFingerprint,
) -> MediaMatchV3DiagnosticSummary {
    summarize_record_v3_diagnostics_with_report(&fingerprint.record, Some(&fingerprint.report))
}

fn summarize_record_v3_diagnostics_with_report(
    record: &MediaFingerprintRecord,
    report: Option<&MediaFingerprintExtractionReport>,
) -> MediaMatchV3DiagnosticSummary {
    let duration_ms = record.duration_seconds.and_then(duration_seconds_to_millis);
    let audio_landmarks = audio_landmarks_v3_from_record(record);
    let audio_index_landmarks = audio_index_landmarks_v3_from_record(record);
    let audio_blob_bytes = encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
        duration_ms: duration_ms.map(u64::from),
        audio_landmarks: audio_landmarks.clone(),
    })
    .len();
    let audio_stream = report.map(|report| &report.audio_stream);
    let mut notes = Vec::new();
    notes.push(format!(
        "audioRegionCounts60s verify=[{}] index=[{}] averageWeight={:.2}",
        format_audio_region_counts(&audio_landmarks),
        format_audio_region_counts(&audio_index_landmarks),
        average_audio_landmark_weight(&audio_landmarks),
    ));
    MediaMatchV3DiagnosticSummary {
        file_path: Some(record.identity.normalized_path.clone()),
        profile: record.extraction_settings.profile.label().to_owned(),
        index_quality: "sampled-fast".to_owned(),
        duration_ms,
        extraction_total_millis: report.map(|report| report.timings.total_millis),
        extraction_audio_millis: report.map(|report| report.timings.audio_millis),
        audio_verify_count: audio_landmarks.len(),
        audio_index_count: audio_index_landmarks.len(),
        audio_blob_bytes,
        retrieval_candidates_count: None,
        decision_tier: None,
        decision_class: None,
        source_path_root: audio_stream.and_then(|stream| stream.source_path_root.clone()),
        source_path_kind: audio_stream.and_then(|stream| stream.source_path_kind.clone()),
        source_volume_id: audio_stream.and_then(|stream| stream.source_volume_id.clone()),
        ffmpeg_command_kind: audio_stream.and_then(|stream| stream.ffmpeg_command_kind.clone()),
        ffmpeg_selected_stream: audio_stream
            .and_then(|stream| stream.ffmpeg_selected_stream.clone()),
        ffmpeg_disabled_non_audio_streams: audio_stream
            .map(|stream| stream.ffmpeg_disabled_non_audio_streams)
            .unwrap_or(false),
        ffmpeg_disabled_subtitles: audio_stream
            .map(|stream| stream.ffmpeg_disabled_subtitles)
            .unwrap_or(false),
        ffmpeg_disabled_data: audio_stream
            .map(|stream| stream.ffmpeg_disabled_data)
            .unwrap_or(false),
        streamed_bytes: audio_stream.map(|stream| stream.streamed_bytes),
        streamed_samples: audio_stream.map(|stream| stream.streamed_samples),
        peak_frames: audio_stream.map(|stream| stream.peak_frames),
        raw_landmarks_emitted: audio_stream.map(|stream| stream.raw_landmarks_emitted),
        raw_landmarks_before_bounding: audio_stream
            .map(|stream| stream.raw_landmarks_before_bounding),
        raw_landmarks_kept_before_final: audio_stream
            .map(|stream| stream.raw_landmarks_kept_before_final),
        final_landmarks: audio_stream.map(|stream| stream.final_landmarks),
        max_buffer_samples: audio_stream.map(|stream| stream.max_buffer_samples),
        raw_landmark_compactions: audio_stream.map(|stream| stream.raw_landmark_compactions),
        ffmpeg_process_wall_millis: audio_stream.map(|stream| stream.ffmpeg_process_wall_millis),
        ffmpeg_input_read_bytes: audio_stream.and_then(|stream| stream.ffmpeg_input_read_bytes),
        ffmpeg_input_read_ops: audio_stream.and_then(|stream| stream.ffmpeg_input_read_ops),
        ffmpeg_output_pcm_bytes: audio_stream.map(|stream| stream.ffmpeg_output_pcm_bytes),
        read_amplification_ratio: audio_stream.and_then(read_amplification_ratio),
        ffmpeg_invocation_count: audio_stream.map(|stream| stream.ffmpeg_invocation_count),
        sampled_window_decode_millis: audio_stream
            .map(|stream| stream.sampled_window_decode_millis),
        ffmpeg_exit_millis: audio_stream.map(|stream| stream.ffmpeg_exit_millis),
        pcm_decode_drain_millis: audio_stream.map(|stream| stream.pcm_decode_drain_millis),
        ffmpeg_decode_stream_millis: audio_stream.map(|stream| stream.ffmpeg_decode_stream_millis),
        analyzer_millis: audio_stream.map(|stream| stream.analyzer_millis),
        peak_selection_millis: audio_stream.map(|stream| stream.peak_selection_millis),
        pairing_millis: audio_stream.map(|stream| stream.pairing_millis),
        reservoir_millis: audio_stream.map(|stream| stream.reservoir_millis),
        final_selection_millis: audio_stream.map(|stream| stream.final_selection_millis),
        candidate_pairs_considered: audio_stream.map(|stream| stream.candidate_pairs_considered),
        candidate_pairs_skipped_by_anchor_gate: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_anchor_gate),
        candidate_pairs_skipped_by_target_gate: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_target_gate),
        candidate_pairs_skipped_by_saturation: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_saturation),
        candidate_pairs_emitted: audio_stream.map(|stream| stream.candidate_pairs_emitted),
        landmarks_accepted_into_reservoir: audio_stream
            .map(|stream| stream.landmarks_accepted_into_reservoir),
        landmarks_rejected_by_reservoir: audio_stream
            .map(|stream| stream.landmarks_rejected_by_reservoir),
        reservoir_acceptance_ratio: audio_stream.and_then(reservoir_acceptance_ratio),
        sampled_audio_seconds_decoded: audio_stream
            .map(|stream| stream.sampled_audio_seconds_decoded),
        sampled_audio_windows_decoded: audio_stream
            .map(|stream| stream.sampled_audio_windows_decoded),
        notes,
    }
}

pub fn summarize_decision_v3_diagnostics(
    decision: &MediaMatchDecision,
) -> MediaMatchV3DiagnosticSummary {
    MediaMatchV3DiagnosticSummary {
        retrieval_candidates_count: Some(1),
        decision_tier: Some(decision.tier),
        decision_class: decision.evidence.v3_class,
        notes: decision.evidence.notes.clone(),
        ..MediaMatchV3DiagnosticSummary::default()
    }
}

fn format_audio_region_counts(landmarks: &[crate::audio_v3::AudioLandmarkV3]) -> String {
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    for landmark in landmarks {
        *counts.entry(landmark.t_ms / 60_000).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(region, count)| format!("{region}:{count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn average_audio_landmark_weight(landmarks: &[crate::audio_v3::AudioLandmarkV3]) -> f64 {
    if landmarks.is_empty() {
        return 0.0;
    }
    landmarks
        .iter()
        .map(|landmark| f64::from(landmark.weight))
        .sum::<f64>()
        / landmarks.len() as f64
}

fn read_amplification_ratio(stream: &crate::MediaAudioStreamMetrics) -> Option<f64> {
    let input = stream.ffmpeg_input_read_bytes?;
    let output = stream.ffmpeg_output_pcm_bytes;
    (output > 0).then(|| input as f64 / output as f64)
}

fn reservoir_acceptance_ratio(stream: &crate::MediaAudioStreamMetrics) -> Option<f64> {
    let total = stream.landmarks_accepted_into_reservoir + stream.landmarks_rejected_by_reservoir;
    (total > 0).then(|| stream.landmarks_accepted_into_reservoir as f64 / total as f64)
}
