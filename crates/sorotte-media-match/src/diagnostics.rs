use serde::{Deserialize, Serialize};

use crate::{
    InstrumentedMediaFingerprint, MatchClassV3, MediaFingerprintExtractionReport,
    MediaFingerprintRecord, MediaMatchDecision, MediaMatchTier,
    anchors::{
        MediaFingerprintBlobV3, audio_index_landmarks_v3_from_record,
        audio_landmarks_v3_from_record, encode_media_fingerprint_blob_v3,
        video_index_landmarks_v3_from_record, video_landmarks_v3_from_record,
    },
    audio_v3::AudioLandmarkV3,
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
    pub decision_pair_collection_millis: Option<u64>,
    pub fast_audio_verifier_millis: Option<u64>,
    pub global_fit_millis: Option<u64>,
    pub offset_histogram_millis: Option<u64>,
    pub fast_global_fit_millis: Option<u64>,
    pub broad_global_fit_millis: Option<u64>,
    pub global_fit_candidate_count: Option<usize>,
    pub global_fit_inlier_count: Option<usize>,
    pub global_fit_fallback_used: Option<bool>,
    pub timeline_map_millis: Option<u64>,
    pub evidence_formatting_millis: Option<u64>,
    pub total_decision_millis: Option<u64>,
    pub decision_tier: Option<MediaMatchTier>,
    pub decision_class: Option<MatchClassV3>,
    pub source_path_root: Option<String>,
    pub source_path_kind: Option<String>,
    pub source_volume_id: Option<String>,
    pub ffmpeg_command_kind: Option<String>,
    pub ffmpeg_selected_stream: Option<String>,
    pub ffmpeg_disabled_video: bool,
    pub ffmpeg_disabled_subtitles: bool,
    pub ffmpeg_disabled_data: bool,
    pub container_format: Option<String>,
    pub audio_stream_index: Option<usize>,
    pub audio_codec: Option<String>,
    pub audio_bitrate_bps: Option<u64>,
    pub audio_duration_millis: Option<u64>,
    pub audio_start_time_millis: Option<i64>,
    pub audio_packet_positions_available: Option<bool>,
    pub audio_packet_position_completeness_per_mille: Option<u16>,
    pub audio_packet_positions_monotonic: Option<bool>,
    pub average_audio_packet_size_bytes: Option<u64>,
    pub audio_packet_count_in_sampled_windows: Option<usize>,
    pub audio_packet_probe_millis: Option<u128>,
    pub audio_packet_probe_read_bytes: Option<u64>,
    pub audio_packet_window_compressed_bytes: Option<u64>,
    pub audio_packet_window_coalesced_range_bytes: Option<u64>,
    pub audio_packet_read_savings_estimate_bytes: Option<i64>,
    pub selected_sampled_audio_source_strategy: Option<String>,
    pub source_strategy_decision_reason: Option<String>,
    pub source_strategy_fallback_count: Option<u32>,
    pub audio_packet_map_cache_hit: Option<bool>,
    pub audio_packet_map_build_millis: Option<u128>,
    pub audio_packet_map_packet_count: Option<usize>,
    pub audio_packet_map_bytes: Option<u64>,
    pub audio_packet_map_complete: Option<bool>,
    pub audio_packet_map_fallback_reason: Option<String>,
    pub audio_packet_window_count: Option<usize>,
    pub audio_packet_ranges: Option<usize>,
    pub audio_packet_range_bytes: Option<u64>,
    pub audio_packet_coalesced_range_bytes: Option<u64>,
    pub audio_packet_range_read_millis: Option<u128>,
    pub audio_packet_range_read_ops: Option<u64>,
    pub audio_packet_read_amplification_vs_pcm: Option<f64>,
    pub audio_packet_estimated_savings_vs_current: Option<i64>,
    pub sampled_pcm_cache_hit: Option<bool>,
    pub sampled_pcm_cache_bytes: Option<u64>,
    pub sampled_pcm_cache_read_millis: Option<u128>,
    pub sampled_pcm_cache_write_millis: Option<u128>,
    pub sampled_pcm_cache_saved_millis: Option<i64>,
    pub audio_sidecar_mode: Option<String>,
    pub audio_sidecar_fallback_reason: Option<String>,
    pub streamed_bytes: Option<usize>,
    pub streamed_samples: Option<usize>,
    pub peak_frames: Option<usize>,
    pub raw_landmarks_emitted: Option<usize>,
    pub raw_landmarks_before_bounding: Option<usize>,
    pub raw_landmarks_kept_before_final: Option<usize>,
    pub final_landmarks: Option<usize>,
    pub max_buffer_samples: Option<usize>,
    pub max_raw_landmarks_seen: Option<usize>,
    pub max_raw_landmarks_after_compaction: Option<usize>,
    pub raw_landmark_compactions: Option<usize>,
    pub ffmpeg_process_wall_millis: Option<u128>,
    pub ffmpeg_input_read_bytes: Option<u64>,
    pub ffmpeg_input_read_ops: Option<u64>,
    pub ffmpeg_output_pcm_bytes: Option<u64>,
    pub read_amplification_ratio: Option<f64>,
    pub ffmpeg_invocation_count: Option<usize>,
    pub sampled_window_seek_millis: Option<u128>,
    pub sampled_window_decode_millis: Option<u128>,
    pub ffmpeg_open_probe_millis: Option<u128>,
    pub ffmpeg_exit_millis: Option<u128>,
    pub pcm_decode_drain_millis: Option<u128>,
    pub analyzer_millis: Option<u128>,
    pub peak_selection_millis: Option<u128>,
    pub pairing_millis: Option<u128>,
    pub compaction_millis: Option<u128>,
    pub reservoir_millis: Option<u128>,
    pub final_selection_millis: Option<u128>,
    pub pcm_drain_thread_millis: Option<u128>,
    pub analyzer_thread_millis: Option<u128>,
    pub channel_backpressure_millis: Option<u128>,
    pub max_queued_pcm_bytes: Option<usize>,
    pub candidate_pairs_considered: Option<usize>,
    pub candidate_pairs_skipped_by_anchor_gate: Option<usize>,
    pub candidate_pairs_skipped_by_target_gate: Option<usize>,
    pub candidate_pairs_skipped_by_saturation: Option<usize>,
    pub candidate_pairs_emitted: Option<usize>,
    pub anchor_peaks_considered: Option<usize>,
    pub anchor_peaks_selected: Option<usize>,
    pub anchor_peaks_skipped_by_gate: Option<usize>,
    pub target_peaks_considered: Option<usize>,
    pub target_peaks_selected: Option<usize>,
    pub landmarks_accepted_into_reservoir: Option<usize>,
    pub landmarks_rejected_by_reservoir: Option<usize>,
    pub reservoir_acceptance_ratio: Option<f64>,
    pub sampled_audio_seconds_decoded: Option<u32>,
    pub sampled_audio_windows_decoded: Option<usize>,
    pub full_audio_seconds_decoded: Option<u32>,
    pub effective_decoded_seconds_per_second: Option<f64>,
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
    let audio_index_landmarks = audio_index_landmarks_v3_from_record(record);
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
            "sourcePathRoot={} sourcePathKind={} ffmpegCommandKind={} ffmpegSelectedStream={} ffmpegDisabledVideo={} ffmpegDisabledSubtitles={} ffmpegDisabledData={} containerFormat={} audioStreamIndex={} audioCodec={} audioBitrateBps={} audioDurationMillis={} audioStartTimeMillis={} audioPacketPositionsAvailable={} audioPacketPositionCompletenessPerMille={} audioPacketPositionsMonotonic={} averageAudioPacketSizeBytes={} audioPacketCountInSampledWindows={} audioPacketProbeMillis={} audioPacketProbeReadBytes={} audioPacketWindowCompressedBytes={} audioPacketWindowCoalescedRangeBytes={} audioPacketReadSavingsEstimateBytes={} streamedBytes={} streamedSamples={} peakFrames={} rawLandmarksEmitted={} rawLandmarksBeforeBounding={} rawLandmarksKeptBeforeFinal={} finalLandmarks={} maxBufferSamples={} maxRawLandmarksSeen={} maxRawLandmarksAfterCompaction={} rawLandmarkCompactions={} ffmpegProcessWallMillis={} ffmpegInputReadBytes={} ffmpegInputReadOps={} ffmpegOutputPcmBytes={} readAmplificationRatio={:.4} ffmpegInvocationCount={} sampledWindowSeekMillis={} sampledWindowDecodeMillis={} ffmpegOpenProbeMillis={} ffmpegExitMillis={} pcmDecodeDrainMillis={} pcmDrainThreadMillis={} analyzerThreadMillis={} channelBackpressureMillis={} maxQueuedPcmBytes={} analyzerMillis={} peakSelectionMillis={} pairingMillis={} compactionMillis={} reservoirMillis={} finalSelectionMillis={} anchorPeaksConsidered={} anchorPeaksSelected={} anchorPeaksSkippedByGate={} targetPeaksConsidered={} targetPeaksSelected={} candidatePairsConsidered={} candidatePairsSkippedByAnchorGate={} candidatePairsSkippedByTargetGate={} candidatePairsSkippedBySaturation={} candidatePairsEmitted={} landmarksAcceptedIntoReservoir={} landmarksRejectedByReservoir={} reservoirAcceptanceRatio={:.4} ffmpegDecodeStreamMillis={} sampledAudioSecondsDecoded={} sampledAudioWindowsDecoded={} fullAudioSecondsDecoded={} effectiveDecodedSecondsPerSecond={:.2}",
            report.audio_stream.source_path_root.as_deref().unwrap_or(""),
            report.audio_stream.source_path_kind.as_deref().unwrap_or("unknown"),
            report.audio_stream.ffmpeg_command_kind.as_deref().unwrap_or(""),
            report.audio_stream.ffmpeg_selected_stream.as_deref().unwrap_or(""),
            report.audio_stream.ffmpeg_disabled_video,
            report.audio_stream.ffmpeg_disabled_subtitles,
            report.audio_stream.ffmpeg_disabled_data,
            report.audio_stream.container_format.as_deref().unwrap_or(""),
            report.audio_stream.audio_stream_index.unwrap_or(0),
            report.audio_stream.audio_codec.as_deref().unwrap_or(""),
            report.audio_stream.audio_bitrate_bps.unwrap_or(0),
            report.audio_stream.audio_duration_millis.unwrap_or(0),
            report.audio_stream.audio_start_time_millis.unwrap_or(0),
            report.audio_stream.audio_packet_positions_available.unwrap_or(false),
            report
                .audio_stream
                .audio_packet_position_completeness_per_mille
                .unwrap_or(0),
            report.audio_stream.audio_packet_positions_monotonic.unwrap_or(false),
            report.audio_stream.average_audio_packet_size_bytes.unwrap_or(0),
            report
                .audio_stream
                .audio_packet_count_in_sampled_windows
                .unwrap_or(0),
            report.audio_stream.audio_packet_probe_millis.unwrap_or(0),
            report.audio_stream.audio_packet_probe_read_bytes.unwrap_or(0),
            report
                .audio_stream
                .audio_packet_window_compressed_bytes
                .unwrap_or(0),
            report
                .audio_stream
                .audio_packet_window_coalesced_range_bytes
                .unwrap_or(0),
            report
                .audio_stream
                .audio_packet_read_savings_estimate_bytes
                .unwrap_or(0),
            report.audio_stream.streamed_bytes,
            report.audio_stream.streamed_samples,
            report.audio_stream.peak_frames,
            report.audio_stream.raw_landmarks_emitted,
            report.audio_stream.raw_landmarks_before_bounding,
            report.audio_stream.raw_landmarks_before_bounding,
            report.audio_stream.final_landmarks,
            report.audio_stream.max_buffer_samples,
            report.audio_stream.max_raw_landmarks_seen,
            report.audio_stream.max_raw_landmarks_after_compaction,
            report.audio_stream.raw_landmark_compactions,
            report.audio_stream.ffmpeg_process_wall_millis,
            report.audio_stream.ffmpeg_input_read_bytes.unwrap_or(0),
            report.audio_stream.ffmpeg_input_read_ops.unwrap_or(0),
            report.audio_stream.ffmpeg_output_pcm_bytes,
            read_amplification_ratio(&report.audio_stream).unwrap_or(0.0),
            report.audio_stream.ffmpeg_invocation_count,
            report.audio_stream.sampled_window_seek_millis,
            report.audio_stream.sampled_window_decode_millis,
            report.audio_stream.ffmpeg_open_probe_millis,
            report.audio_stream.ffmpeg_exit_millis,
            report.audio_stream.pcm_decode_drain_millis,
            report.audio_stream.pcm_drain_thread_millis,
            report.audio_stream.analyzer_thread_millis,
            report.audio_stream.channel_backpressure_millis,
            report.audio_stream.max_queued_pcm_bytes,
            report.audio_stream.analyzer_millis,
            report.audio_stream.peak_selection_millis,
            report.audio_stream.pairing_millis,
            report.audio_stream.compaction_millis,
            report.audio_stream.reservoir_millis,
            report.audio_stream.final_selection_millis,
            report.audio_stream.anchor_peaks_considered,
            report.audio_stream.anchor_peaks_selected,
            report.audio_stream.anchor_peaks_skipped_by_gate,
            report.audio_stream.target_peaks_considered,
            report.audio_stream.target_peaks_selected,
            report.audio_stream.candidate_pairs_considered,
            report.audio_stream.candidate_pairs_skipped_by_anchor_gate,
            report.audio_stream.candidate_pairs_skipped_by_target_gate,
            report.audio_stream.candidate_pairs_skipped_by_saturation,
            report.audio_stream.candidate_pairs_emitted,
            report.audio_stream.landmarks_accepted_into_reservoir,
            report.audio_stream.landmarks_rejected_by_reservoir,
            reservoir_acceptance_ratio(&report.audio_stream).unwrap_or(0.0),
            report.audio_stream.ffmpeg_decode_stream_millis,
            report.audio_stream.sampled_audio_seconds_decoded,
            report.audio_stream.sampled_audio_windows_decoded,
            report.audio_stream.full_audio_seconds_decoded,
            effective_decoded_seconds_per_second(&report.audio_stream).unwrap_or(0.0)
        ));
    }
    let (audio_edge_count, audio_body_count) =
        audio_edge_body_counts(&audio_landmarks, duration_ms);
    notes.push(format!(
        "audioRegionCounts60s verify=[{}] index=[{}] edgeLandmarks={} bodyLandmarks={} averageWeight={:.2} medianWeight={:.2}",
        format_audio_region_counts(&audio_landmarks),
        format_audio_region_counts(&audio_index_landmarks),
        audio_edge_count,
        audio_body_count,
        average_audio_landmark_weight(&audio_landmarks),
        median_audio_landmark_weight(&audio_landmarks)
    ));
    MediaMatchV3DiagnosticSummary {
        file_path: Some(record.identity.normalized_path.clone()),
        profile: record.extraction_settings.profile.label().to_owned(),
        index_quality: record
            .extraction_settings
            .audio_index_mode
            .label()
            .to_owned(),
        duration_ms,
        extraction_total_millis: report.map(|report| report.timings.total_millis),
        extraction_audio_millis: report.map(|report| report.timings.audio_millis),
        extraction_video_millis: report.map(|report| report.timings.video_millis),
        audio_verify_count: audio_landmarks.len(),
        video_verify_count: video_landmarks.len(),
        audio_index_count: audio_index_landmarks.len(),
        video_index_count: video_index_landmarks_v3_from_record(record).len(),
        audio_blob_bytes,
        video_blob_bytes,
        retrieval_candidates_count: None,
        piecewise_pair_count: None,
        piecewise_hypothesis_count: None,
        piecewise_segment_count: None,
        piecewise_fit_millis: None,
        decision_pair_collection_millis: None,
        fast_audio_verifier_millis: None,
        global_fit_millis: None,
        offset_histogram_millis: None,
        fast_global_fit_millis: None,
        broad_global_fit_millis: None,
        global_fit_candidate_count: None,
        global_fit_inlier_count: None,
        global_fit_fallback_used: None,
        timeline_map_millis: None,
        evidence_formatting_millis: None,
        total_decision_millis: None,
        decision_tier: None,
        decision_class: None,
        source_path_root: audio_stream.and_then(|stream| stream.source_path_root.clone()),
        source_path_kind: audio_stream.and_then(|stream| stream.source_path_kind.clone()),
        source_volume_id: audio_stream.and_then(|stream| stream.source_volume_id.clone()),
        ffmpeg_command_kind: audio_stream.and_then(|stream| stream.ffmpeg_command_kind.clone()),
        ffmpeg_selected_stream: audio_stream
            .and_then(|stream| stream.ffmpeg_selected_stream.clone()),
        ffmpeg_disabled_video: audio_stream
            .map(|stream| stream.ffmpeg_disabled_video)
            .unwrap_or(false),
        ffmpeg_disabled_subtitles: audio_stream
            .map(|stream| stream.ffmpeg_disabled_subtitles)
            .unwrap_or(false),
        ffmpeg_disabled_data: audio_stream
            .map(|stream| stream.ffmpeg_disabled_data)
            .unwrap_or(false),
        container_format: audio_stream.and_then(|stream| stream.container_format.clone()),
        audio_stream_index: audio_stream.and_then(|stream| stream.audio_stream_index),
        audio_codec: audio_stream.and_then(|stream| stream.audio_codec.clone()),
        audio_bitrate_bps: audio_stream.and_then(|stream| stream.audio_bitrate_bps),
        audio_duration_millis: audio_stream.and_then(|stream| stream.audio_duration_millis),
        audio_start_time_millis: audio_stream.and_then(|stream| stream.audio_start_time_millis),
        audio_packet_positions_available: audio_stream
            .and_then(|stream| stream.audio_packet_positions_available),
        audio_packet_position_completeness_per_mille: audio_stream
            .and_then(|stream| stream.audio_packet_position_completeness_per_mille),
        audio_packet_positions_monotonic: audio_stream
            .and_then(|stream| stream.audio_packet_positions_monotonic),
        average_audio_packet_size_bytes: audio_stream
            .and_then(|stream| stream.average_audio_packet_size_bytes),
        audio_packet_count_in_sampled_windows: audio_stream
            .and_then(|stream| stream.audio_packet_count_in_sampled_windows),
        audio_packet_probe_millis: audio_stream.and_then(|stream| stream.audio_packet_probe_millis),
        audio_packet_probe_read_bytes: audio_stream
            .and_then(|stream| stream.audio_packet_probe_read_bytes),
        audio_packet_window_compressed_bytes: audio_stream
            .and_then(|stream| stream.audio_packet_window_compressed_bytes),
        audio_packet_window_coalesced_range_bytes: audio_stream
            .and_then(|stream| stream.audio_packet_window_coalesced_range_bytes),
        audio_packet_read_savings_estimate_bytes: audio_stream
            .and_then(|stream| stream.audio_packet_read_savings_estimate_bytes),
        selected_sampled_audio_source_strategy: audio_stream
            .and_then(|stream| stream.selected_sampled_audio_source_strategy.clone()),
        source_strategy_decision_reason: audio_stream
            .and_then(|stream| stream.source_strategy_decision_reason.clone()),
        source_strategy_fallback_count: audio_stream
            .map(|stream| stream.source_strategy_fallback_count),
        audio_packet_map_cache_hit: audio_stream
            .and_then(|stream| stream.audio_packet_map_cache_hit),
        audio_packet_map_build_millis: audio_stream
            .and_then(|stream| stream.audio_packet_map_build_millis),
        audio_packet_map_packet_count: audio_stream
            .and_then(|stream| stream.audio_packet_map_packet_count),
        audio_packet_map_bytes: audio_stream.and_then(|stream| stream.audio_packet_map_bytes),
        audio_packet_map_complete: audio_stream.and_then(|stream| stream.audio_packet_map_complete),
        audio_packet_map_fallback_reason: audio_stream
            .and_then(|stream| stream.audio_packet_map_fallback_reason.clone()),
        audio_packet_window_count: audio_stream.and_then(|stream| stream.audio_packet_window_count),
        audio_packet_ranges: audio_stream.and_then(|stream| stream.audio_packet_ranges),
        audio_packet_range_bytes: audio_stream.and_then(|stream| stream.audio_packet_range_bytes),
        audio_packet_coalesced_range_bytes: audio_stream
            .and_then(|stream| stream.audio_packet_coalesced_range_bytes),
        audio_packet_range_read_millis: audio_stream
            .and_then(|stream| stream.audio_packet_range_read_millis),
        audio_packet_range_read_ops: audio_stream
            .and_then(|stream| stream.audio_packet_range_read_ops),
        audio_packet_read_amplification_vs_pcm: audio_stream
            .and_then(|stream| stream.audio_packet_read_amplification_vs_pcm),
        audio_packet_estimated_savings_vs_current: audio_stream
            .and_then(|stream| stream.audio_packet_estimated_savings_vs_current),
        sampled_pcm_cache_hit: audio_stream.and_then(|stream| stream.sampled_pcm_cache_hit),
        sampled_pcm_cache_bytes: audio_stream.and_then(|stream| stream.sampled_pcm_cache_bytes),
        sampled_pcm_cache_read_millis: audio_stream
            .and_then(|stream| stream.sampled_pcm_cache_read_millis),
        sampled_pcm_cache_write_millis: audio_stream
            .and_then(|stream| stream.sampled_pcm_cache_write_millis),
        sampled_pcm_cache_saved_millis: audio_stream
            .and_then(|stream| stream.sampled_pcm_cache_saved_millis),
        audio_sidecar_mode: audio_stream.and_then(|stream| stream.audio_sidecar_mode.clone()),
        audio_sidecar_fallback_reason: audio_stream
            .and_then(|stream| stream.audio_sidecar_fallback_reason.clone()),
        streamed_bytes: audio_stream.map(|stream| stream.streamed_bytes),
        streamed_samples: audio_stream.map(|stream| stream.streamed_samples),
        peak_frames: audio_stream.map(|stream| stream.peak_frames),
        raw_landmarks_emitted: audio_stream.map(|stream| stream.raw_landmarks_emitted),
        raw_landmarks_before_bounding: audio_stream
            .map(|stream| stream.raw_landmarks_before_bounding),
        raw_landmarks_kept_before_final: audio_stream
            .map(|stream| stream.raw_landmarks_before_bounding),
        final_landmarks: audio_stream.map(|stream| stream.final_landmarks),
        max_buffer_samples: audio_stream.map(|stream| stream.max_buffer_samples),
        max_raw_landmarks_seen: audio_stream.map(|stream| stream.max_raw_landmarks_seen),
        max_raw_landmarks_after_compaction: audio_stream
            .map(|stream| stream.max_raw_landmarks_after_compaction),
        raw_landmark_compactions: audio_stream.map(|stream| stream.raw_landmark_compactions),
        ffmpeg_process_wall_millis: audio_stream.map(|stream| stream.ffmpeg_process_wall_millis),
        ffmpeg_input_read_bytes: audio_stream.and_then(|stream| stream.ffmpeg_input_read_bytes),
        ffmpeg_input_read_ops: audio_stream.and_then(|stream| stream.ffmpeg_input_read_ops),
        ffmpeg_output_pcm_bytes: audio_stream.map(|stream| stream.ffmpeg_output_pcm_bytes),
        read_amplification_ratio: audio_stream.and_then(read_amplification_ratio),
        ffmpeg_invocation_count: audio_stream.map(|stream| stream.ffmpeg_invocation_count),
        sampled_window_seek_millis: audio_stream.map(|stream| stream.sampled_window_seek_millis),
        sampled_window_decode_millis: audio_stream
            .map(|stream| stream.sampled_window_decode_millis),
        ffmpeg_open_probe_millis: audio_stream.map(|stream| stream.ffmpeg_open_probe_millis),
        ffmpeg_exit_millis: audio_stream.map(|stream| stream.ffmpeg_exit_millis),
        pcm_decode_drain_millis: audio_stream.map(|stream| stream.pcm_decode_drain_millis),
        analyzer_millis: audio_stream.map(|stream| stream.analyzer_millis),
        peak_selection_millis: audio_stream.map(|stream| stream.peak_selection_millis),
        pairing_millis: audio_stream.map(|stream| stream.pairing_millis),
        compaction_millis: audio_stream.map(|stream| stream.compaction_millis),
        reservoir_millis: audio_stream.map(|stream| stream.reservoir_millis),
        final_selection_millis: audio_stream.map(|stream| stream.final_selection_millis),
        pcm_drain_thread_millis: audio_stream.map(|stream| stream.pcm_drain_thread_millis),
        analyzer_thread_millis: audio_stream.map(|stream| stream.analyzer_thread_millis),
        channel_backpressure_millis: audio_stream.map(|stream| stream.channel_backpressure_millis),
        max_queued_pcm_bytes: audio_stream.map(|stream| stream.max_queued_pcm_bytes),
        candidate_pairs_considered: audio_stream.map(|stream| stream.candidate_pairs_considered),
        candidate_pairs_skipped_by_anchor_gate: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_anchor_gate),
        candidate_pairs_skipped_by_target_gate: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_target_gate),
        candidate_pairs_skipped_by_saturation: audio_stream
            .map(|stream| stream.candidate_pairs_skipped_by_saturation),
        candidate_pairs_emitted: audio_stream.map(|stream| stream.candidate_pairs_emitted),
        anchor_peaks_considered: audio_stream.map(|stream| stream.anchor_peaks_considered),
        anchor_peaks_selected: audio_stream.map(|stream| stream.anchor_peaks_selected),
        anchor_peaks_skipped_by_gate: audio_stream
            .map(|stream| stream.anchor_peaks_skipped_by_gate),
        target_peaks_considered: audio_stream.map(|stream| stream.target_peaks_considered),
        target_peaks_selected: audio_stream.map(|stream| stream.target_peaks_selected),
        landmarks_accepted_into_reservoir: audio_stream
            .map(|stream| stream.landmarks_accepted_into_reservoir),
        landmarks_rejected_by_reservoir: audio_stream
            .map(|stream| stream.landmarks_rejected_by_reservoir),
        reservoir_acceptance_ratio: audio_stream.and_then(reservoir_acceptance_ratio),
        sampled_audio_seconds_decoded: audio_stream
            .map(|stream| stream.sampled_audio_seconds_decoded),
        sampled_audio_windows_decoded: audio_stream
            .map(|stream| stream.sampled_audio_windows_decoded),
        full_audio_seconds_decoded: audio_stream.map(|stream| stream.full_audio_seconds_decoded),
        effective_decoded_seconds_per_second: audio_stream
            .and_then(effective_decoded_seconds_per_second),
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
        index_quality: "decision".to_owned(),
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
        decision_pair_collection_millis: map.map(|map| map.decision_pair_collection_millis),
        fast_audio_verifier_millis: map.map(|map| map.fast_audio_verifier_millis),
        global_fit_millis: map.map(|map| map.global_fit_millis),
        offset_histogram_millis: map.map(|map| map.offset_histogram_millis),
        fast_global_fit_millis: map.map(|map| map.fast_global_fit_millis),
        broad_global_fit_millis: map.map(|map| map.broad_global_fit_millis),
        global_fit_candidate_count: map.map(|map| map.global_fit_candidate_count),
        global_fit_inlier_count: map.map(|map| map.global_fit_inlier_count),
        global_fit_fallback_used: map.map(|map| map.global_fit_fallback_used),
        timeline_map_millis: map.map(|map| map.timeline_map_millis),
        evidence_formatting_millis: map.map(|map| map.evidence_formatting_millis),
        total_decision_millis: map.map(|map| map.total_decision_millis),
        decision_tier: Some(decision.tier),
        decision_class: decision
            .evidence
            .v3_class
            .or_else(|| map.map(|map| map.global_class)),
        source_path_root: None,
        source_path_kind: None,
        source_volume_id: None,
        ffmpeg_command_kind: None,
        ffmpeg_selected_stream: None,
        ffmpeg_disabled_video: false,
        ffmpeg_disabled_subtitles: false,
        ffmpeg_disabled_data: false,
        container_format: None,
        audio_stream_index: None,
        audio_codec: None,
        audio_bitrate_bps: None,
        audio_duration_millis: None,
        audio_start_time_millis: None,
        audio_packet_positions_available: None,
        audio_packet_position_completeness_per_mille: None,
        audio_packet_positions_monotonic: None,
        average_audio_packet_size_bytes: None,
        audio_packet_count_in_sampled_windows: None,
        audio_packet_probe_millis: None,
        audio_packet_probe_read_bytes: None,
        audio_packet_window_compressed_bytes: None,
        audio_packet_window_coalesced_range_bytes: None,
        audio_packet_read_savings_estimate_bytes: None,
        selected_sampled_audio_source_strategy: None,
        source_strategy_decision_reason: None,
        source_strategy_fallback_count: None,
        audio_packet_map_cache_hit: None,
        audio_packet_map_build_millis: None,
        audio_packet_map_packet_count: None,
        audio_packet_map_bytes: None,
        audio_packet_map_complete: None,
        audio_packet_map_fallback_reason: None,
        audio_packet_window_count: None,
        audio_packet_ranges: None,
        audio_packet_range_bytes: None,
        audio_packet_coalesced_range_bytes: None,
        audio_packet_range_read_millis: None,
        audio_packet_range_read_ops: None,
        audio_packet_read_amplification_vs_pcm: None,
        audio_packet_estimated_savings_vs_current: None,
        sampled_pcm_cache_hit: None,
        sampled_pcm_cache_bytes: None,
        sampled_pcm_cache_read_millis: None,
        sampled_pcm_cache_write_millis: None,
        sampled_pcm_cache_saved_millis: None,
        audio_sidecar_mode: None,
        audio_sidecar_fallback_reason: None,
        streamed_bytes: None,
        streamed_samples: None,
        peak_frames: None,
        raw_landmarks_emitted: None,
        raw_landmarks_before_bounding: None,
        raw_landmarks_kept_before_final: None,
        final_landmarks: None,
        max_buffer_samples: None,
        max_raw_landmarks_seen: None,
        max_raw_landmarks_after_compaction: None,
        raw_landmark_compactions: None,
        ffmpeg_process_wall_millis: None,
        ffmpeg_input_read_bytes: None,
        ffmpeg_input_read_ops: None,
        ffmpeg_output_pcm_bytes: None,
        read_amplification_ratio: None,
        ffmpeg_invocation_count: None,
        sampled_window_seek_millis: None,
        sampled_window_decode_millis: None,
        ffmpeg_open_probe_millis: None,
        ffmpeg_exit_millis: None,
        pcm_decode_drain_millis: None,
        analyzer_millis: None,
        peak_selection_millis: None,
        pairing_millis: None,
        compaction_millis: None,
        reservoir_millis: None,
        final_selection_millis: None,
        pcm_drain_thread_millis: None,
        analyzer_thread_millis: None,
        channel_backpressure_millis: None,
        max_queued_pcm_bytes: None,
        candidate_pairs_considered: None,
        candidate_pairs_skipped_by_anchor_gate: None,
        candidate_pairs_skipped_by_target_gate: None,
        candidate_pairs_skipped_by_saturation: None,
        candidate_pairs_emitted: None,
        anchor_peaks_considered: None,
        anchor_peaks_selected: None,
        anchor_peaks_skipped_by_gate: None,
        target_peaks_considered: None,
        target_peaks_selected: None,
        landmarks_accepted_into_reservoir: None,
        landmarks_rejected_by_reservoir: None,
        reservoir_acceptance_ratio: None,
        sampled_audio_seconds_decoded: None,
        sampled_audio_windows_decoded: None,
        full_audio_seconds_decoded: None,
        effective_decoded_seconds_per_second: None,
        notes,
    }
}

fn reservoir_acceptance_ratio(stream: &crate::MediaAudioStreamMetrics) -> Option<f64> {
    let total = stream
        .landmarks_accepted_into_reservoir
        .saturating_add(stream.landmarks_rejected_by_reservoir);
    if total == 0 {
        return None;
    }
    Some(stream.landmarks_accepted_into_reservoir as f64 / total as f64)
}

fn effective_decoded_seconds_per_second(stream: &crate::MediaAudioStreamMetrics) -> Option<f64> {
    let decoded_seconds = u64::from(stream.sampled_audio_seconds_decoded)
        + u64::from(stream.full_audio_seconds_decoded);
    if decoded_seconds == 0 || stream.ffmpeg_process_wall_millis == 0 {
        return None;
    }
    Some(decoded_seconds as f64 / (stream.ffmpeg_process_wall_millis as f64 / 1000.0))
}

fn read_amplification_ratio(stream: &crate::MediaAudioStreamMetrics) -> Option<f64> {
    let input = stream.ffmpeg_input_read_bytes?;
    let output = stream.ffmpeg_output_pcm_bytes;
    if output == 0 {
        return None;
    }
    Some(input as f64 / output as f64)
}

fn format_audio_region_counts(landmarks: &[AudioLandmarkV3]) -> String {
    if landmarks.is_empty() {
        return String::new();
    }
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    for landmark in landmarks {
        *counts.entry(landmark.t_ms / 60_000).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(region, count)| format!("{}:{}", region * 60, count))
        .collect::<Vec<_>>()
        .join(",")
}

fn audio_edge_body_counts(
    landmarks: &[AudioLandmarkV3],
    duration_ms: Option<u32>,
) -> (usize, usize) {
    let Some(duration_ms) = duration_ms else {
        return (0, landmarks.len());
    };
    let edge_ms = ((f64::from(duration_ms) * 0.10).round() as u32).clamp(120_000, 240_000);
    let edge_count = landmarks
        .iter()
        .filter(|landmark| {
            landmark.t_ms < edge_ms || landmark.t_ms >= duration_ms.saturating_sub(edge_ms)
        })
        .count();
    (edge_count, landmarks.len().saturating_sub(edge_count))
}

fn average_audio_landmark_weight(landmarks: &[AudioLandmarkV3]) -> f64 {
    if landmarks.is_empty() {
        return 0.0;
    }
    landmarks
        .iter()
        .map(|landmark| f64::from(landmark.weight))
        .sum::<f64>()
        / landmarks.len() as f64
}

fn median_audio_landmark_weight(landmarks: &[AudioLandmarkV3]) -> f64 {
    if landmarks.is_empty() {
        return 0.0;
    }
    let mut weights = landmarks
        .iter()
        .map(|landmark| landmark.weight)
        .collect::<Vec<_>>();
    weights.sort_unstable();
    f64::from(weights[weights.len() / 2])
}
