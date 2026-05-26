use serde::Serialize;

use crate::{
    InstrumentedMediaFingerprint, MatchClassV3, MediaFingerprintBlobV3,
    MediaFingerprintExtractionReport, MediaFingerprintRecord, MediaMatchDecision, MediaMatchTier,
    anchors::{
        audio_index_landmarks_v3_from_record, audio_landmarks_v3_from_record,
        encode_media_fingerprint_blob_v3, video_index_landmarks_v3_from_record,
        video_landmarks_v3_from_record,
    },
    identity::duration_seconds_to_millis,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchV3DiagnosticSummary {
    pub file_path: Option<String>,
    pub profile: String,
    pub duration_ms: Option<u32>,
    pub extraction_total_millis: Option<u128>,
    pub extraction_audio_millis: Option<u128>,
    pub extraction_video_millis: Option<u128>,
    pub audio_verify_count: usize,
    pub video_verify_count: usize,
    pub audio_index_count: usize,
    pub video_index_count: usize,
    pub audio_blob_bytes: usize,
    pub video_blob_bytes: usize,
    pub retrieval_candidates_count: Option<usize>,
    pub piecewise_pair_count: Option<usize>,
    pub piecewise_hypothesis_count: Option<usize>,
    pub piecewise_segment_count: Option<usize>,
    pub piecewise_fit_millis: Option<u64>,
    pub decision_tier: Option<MediaMatchTier>,
    pub decision_class: Option<MatchClassV3>,
    pub streamed_bytes: Option<usize>,
    pub streamed_samples: Option<usize>,
    pub peak_frames: Option<usize>,
    pub raw_landmarks_before_bounding: Option<usize>,
    pub final_landmarks: Option<usize>,
    pub max_buffer_samples: Option<usize>,
    pub max_raw_landmarks_seen: Option<usize>,
    pub max_raw_landmarks_after_compaction: Option<usize>,
    pub raw_landmark_compactions: Option<usize>,
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
    let video_landmarks = video_landmarks_v3_from_record(record);
    let duration_ms_u64 = duration_ms.map(u64::from);
    let audio_blob_bytes = encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
        duration_ms: duration_ms_u64,
        audio_landmarks: audio_landmarks.clone(),
        video_landmarks: Vec::new(),
    })
    .len();
    let video_blob_bytes = encode_media_fingerprint_blob_v3(&MediaFingerprintBlobV3 {
        duration_ms: duration_ms_u64,
        audio_landmarks: Vec::new(),
        video_landmarks: video_landmarks.clone(),
    })
    .len();
    let mut notes = Vec::new();
    let audio_stream = report.map(|report| &report.audio_stream);
    if let Some(report) = report {
        notes.push(format!(
            "streamedBytes={} streamedSamples={} peakFrames={} rawLandmarksBeforeBounding={} finalLandmarks={} maxBufferSamples={} maxRawLandmarksSeen={} maxRawLandmarksAfterCompaction={} rawLandmarkCompactions={}",
            report.audio_stream.streamed_bytes,
            report.audio_stream.streamed_samples,
            report.audio_stream.peak_frames,
            report.audio_stream.raw_landmarks_before_bounding,
            report.audio_stream.final_landmarks,
            report.audio_stream.max_buffer_samples,
            report.audio_stream.max_raw_landmarks_seen,
            report.audio_stream.max_raw_landmarks_after_compaction,
            report.audio_stream.raw_landmark_compactions
        ));
    }
    MediaMatchV3DiagnosticSummary {
        file_path: Some(record.identity.normalized_path.clone()),
        profile: record.extraction_settings.profile.label().to_owned(),
        duration_ms,
        extraction_total_millis: report.map(|report| report.timings.total_millis),
        extraction_audio_millis: report.map(|report| report.timings.audio_millis),
        extraction_video_millis: report.map(|report| report.timings.video_millis),
        audio_verify_count: audio_landmarks.len(),
        video_verify_count: video_landmarks.len(),
        audio_index_count: audio_index_landmarks_v3_from_record(record).len(),
        video_index_count: video_index_landmarks_v3_from_record(record).len(),
        audio_blob_bytes,
        video_blob_bytes,
        retrieval_candidates_count: None,
        piecewise_pair_count: None,
        piecewise_hypothesis_count: None,
        piecewise_segment_count: None,
        piecewise_fit_millis: None,
        decision_tier: None,
        decision_class: None,
        streamed_bytes: audio_stream.map(|stream| stream.streamed_bytes),
        streamed_samples: audio_stream.map(|stream| stream.streamed_samples),
        peak_frames: audio_stream.map(|stream| stream.peak_frames),
        raw_landmarks_before_bounding: audio_stream
            .map(|stream| stream.raw_landmarks_before_bounding),
        final_landmarks: audio_stream.map(|stream| stream.final_landmarks),
        max_buffer_samples: audio_stream.map(|stream| stream.max_buffer_samples),
        max_raw_landmarks_seen: audio_stream.map(|stream| stream.max_raw_landmarks_seen),
        max_raw_landmarks_after_compaction: audio_stream
            .map(|stream| stream.max_raw_landmarks_after_compaction),
        raw_landmark_compactions: audio_stream.map(|stream| stream.raw_landmark_compactions),
        notes,
    }
}

pub fn summarize_decision_v3_diagnostics(
    decision: &MediaMatchDecision,
) -> MediaMatchV3DiagnosticSummary {
    let map = decision.evidence.timeline_map_v3.as_ref();
    let mut notes = decision.evidence.notes.clone();
    if let Some(map) = map {
        notes.push(format!(
            "v3_class={:?} segments={} span_ms={} largest_gap_ms={} edge_only={} audio_video_conflict={} best_segment_score={} second_segment_score={}",
            map.global_class,
            map.segments.len(),
            map.total_aligned_span_ms,
            map.largest_gap_ms,
            map.edge_only,
            map.audio_video_conflict,
            map.best_segment_score,
            map.second_best_segment_score
        ));
    }
    MediaMatchV3DiagnosticSummary {
        file_path: None,
        profile: "decision".to_owned(),
        duration_ms: None,
        extraction_total_millis: None,
        extraction_audio_millis: None,
        extraction_video_millis: None,
        audio_verify_count: 0,
        video_verify_count: 0,
        audio_index_count: 0,
        video_index_count: 0,
        audio_blob_bytes: 0,
        video_blob_bytes: 0,
        retrieval_candidates_count: None,
        piecewise_pair_count: map.map(|map| map.piecewise_pair_count),
        piecewise_hypothesis_count: map.map(|map| map.piecewise_hypothesis_count),
        piecewise_segment_count: map.map(|map| map.segments.len()),
        piecewise_fit_millis: map.map(|map| map.piecewise_fit_millis),
        decision_tier: Some(decision.tier),
        decision_class: decision
            .evidence
            .v3_class
            .or_else(|| map.map(|map| map.global_class)),
        streamed_bytes: None,
        streamed_samples: None,
        peak_frames: None,
        raw_landmarks_before_bounding: None,
        final_landmarks: None,
        max_buffer_samples: None,
        max_raw_landmarks_seen: None,
        max_raw_landmarks_after_compaction: None,
        raw_landmark_compactions: None,
        notes,
    }
}
