mod anchors;
mod audio_v3;
mod diagnostic_harness;
mod diagnostics;
mod extraction;
mod identity;
mod matching;
mod report_compare;
mod settings;
mod timeline_v3;
mod tuning;
mod types;
mod v3_index;
mod video_v3;
mod wire;

// Anchor row types are public because they are part of `MediaFingerprintRecord`.
pub use anchors::{AudioAnchor, VideoAnchor};
pub use diagnostic_harness::{
    MediaMatchV3DiagnosticCandidateReport, MediaMatchV3DiagnosticCaseReport,
    MediaMatchV3DiagnosticDecisionReport, MediaMatchV3DiagnosticExpectation,
    MediaMatchV3DiagnosticFingerprintReport, MediaMatchV3DiagnosticManifest,
    MediaMatchV3DiagnosticManifestCase, MediaMatchV3DiagnosticReport,
    MediaMatchV3DiagnosticRetrievalReport, MediaMatchV3DiagnosticRunOptions,
    MediaMatchV3DiagnosticSummaryReport, MediaMatchV3ResolvedManifest,
    MediaMatchV3ResolvedManifestCandidate, MediaMatchV3ResolvedManifestCase,
    media_match_v3_diagnostic_manifest_from_json, media_match_v3_diagnostic_manifest_report_json,
    resolve_media_match_v3_diagnostic_manifest, run_media_match_v3_diagnostic_manifest,
    validate_media_match_v3_diagnostic_manifest,
};
pub use diagnostics::{
    MediaMatchV3DiagnosticSummary, summarize_decision_v3_diagnostics,
    summarize_instrumented_record_v3_diagnostics, summarize_record_v3_diagnostics,
};
pub use extraction::{
    InstrumentedMediaFingerprint, MediaAudioStreamMetrics, MediaExtractionTimings,
    MediaFingerprintError, MediaFingerprintExtractionReport, MediaMatchToolPaths,
    MediaToolInvocationCounts, fingerprint_media_file, fingerprint_media_file_cancellable,
    fingerprint_media_file_cancellable_with_report, fingerprint_media_file_with_report,
};
pub use identity::normalize_media_path;
pub use matching::{MediaMatchCandidateDecision, decide_media_match, rank_media_match_candidates};
pub use report_compare::{
    MediaMatchV3ReportComparison, MediaMatchV3ReportComparisonSummary,
    MediaMatchV3ReportCompatibility, MediaMatchV3ReportCompatibilityOptions,
    MediaMatchV3ReportMetricDelta, MediaMatchV3ReportPairKey, MediaMatchV3ReportStatusChange,
    MediaMatchV3ReportValueChange, compare_media_match_v3_reports,
    compare_media_match_v3_reports_with_options, validate_media_match_v3_diagnostic_report,
    validate_media_match_v3_report_pair_compatible,
};
pub use settings::{
    MEDIA_MATCH_V3_FINGERPRINT_CACHE_VERSION, MediaExtractionSettings, MediaFingerprintProfile,
    media_extraction_settings_hash, media_match_v3_fingerprint_config_hash,
};
pub use timeline_v3::{
    classify_timeline_at_query_ms, map_candidate_position_to_query_ms,
    map_query_position_to_candidate_ms, timeline_map_contains_query_position,
};
pub use tuning::{V3Tuning, current_v3_tuning};
pub use types::{
    AlignedSegmentV3, AudioMatchEvidence, MatchClassV3, MediaFileIdentity, MediaFingerprintRecord,
    MediaMatchAutoplayPolicy, MediaMatchCache, MediaMatchDecision, MediaMatchEvidence,
    MediaMatchSettings, MediaMatchTier, MediaTimelineAlignment, MediaTimelineMapV3,
    MetadataMatchEvidence, TimelinePositionMapResult, VideoMatchEvidence,
};
// The GUI/runtime owns cache location and rebuild lifecycle; these wrappers keep the SQL/index
// implementation centralized in this crate while still letting runtime code maintain the cache.
pub use v3_index::{
    MediaMatchV3RetrievalStats, anchor_stats_v3_dirty, clear_all_anchor_stats_v3_dirty,
    clear_anchor_stats_v3_dirty, delete_media_match_v3_file_and_fingerprints,
    delete_media_match_v3_fingerprints_and_anchors, initialize_media_match_v3_index,
    load_media_match_v3_cache_for_settings, load_media_match_v3_record_for_path,
    mark_anchor_stats_v3_dirty, mark_anchor_stats_v3_dirty_for_file,
    media_match_v3_anchor_candidate_paths_with_stats, media_match_v3_index_path,
    open_media_match_v3_index, refresh_all_anchor_stats_v3, refresh_anchor_stats_v3,
    refresh_dirty_anchor_stats_v3_if_needed, save_media_match_v3_record,
};
// These descriptor types and kind helpers are intentionally public: GUI/runtime tests build
// V3 fixture records directly, and valid test fixtures need kind-encoded video buckets.
pub use video_v3::{
    FrameFingerprint, V3_VIDEO_KIND_CENTER_DCT, V3_VIDEO_KIND_EDGE, V3_VIDEO_KIND_GLOBAL_DCT,
    V3_VIDEO_KIND_LUMA_FRAME, V3_VIDEO_KIND_TEMPORAL_SHINGLE, VideoFingerprint, VideoLandmarkV3,
    v3_video_bucket_for_kind,
};
pub use wire::{
    MediaMatchWireAnchorBlock, MediaMatchWireProfile, MediaMatchWireSignature,
    decide_media_match_against_wire_signature, media_anchor_profile_from_wire_profile,
    media_match_wire_anchor_profile_from_anchor_profile, media_match_wire_signature_from_records,
    media_match_wire_signature_from_value, media_match_wire_value_from_records,
};

pub const MEDIA_MATCH_ALGORITHM_VERSION: u32 = 3;
pub const MEDIA_MATCH_FILE_PAYLOAD_KEY: &str = "mediaMatch";
pub const MEDIA_MATCH_WIRE_SCHEMA_V3: &str = "sorotte.mediaMatch.v3";
pub const MEDIA_MATCH_WIRE_MAX_BYTES: usize = 32 * 1024;
pub const MEDIA_MATCH_ANCHOR_VERSION: u32 = 3;

#[cfg(test)]
mod tests;
