use crate::{
    AudioAnchor, MatchClassV3, MediaDurationCompatibility, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexService, MediaMatchAutoplayPolicy, MediaMatchSettings,
    MediaMatchTier, decide_media_match, decide_media_match_against_wire_signature,
    media_match_wire_signature_from_records, rank_media_match_candidates,
    settings::MediaExtractionSettings,
};

#[test]
fn media_index_service_owns_record_round_trip() {
    let root = std::env::temp_dir().join(format!(
        "sorotte-media-index-service-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos()
    ));
    let service = MediaIndexService::new(&root);
    let session = service.open().expect("index session should open");
    let record = record("episode.mkv", 0);

    session
        .save_record(&record, None)
        .expect("record should save through the service");
    let loaded = session
        .load_record(
            "episode.mkv",
            &record.extraction_settings,
            record.identity.modified_unix_millis,
            record.identity.size_bytes,
        )
        .expect("record should load through the service")
        .expect("saved record should exist");
    assert_eq!(loaded.identity, record.identity);

    session
        .delete_file("episode.mkv")
        .expect("record should delete through the service");
    assert!(
        session
            .load_record(
                "episode.mkv",
                &record.extraction_settings,
                record.identity.modified_unix_millis,
                record.identity.size_bytes,
            )
            .expect("deleted record lookup should succeed")
            .is_none()
    );
    drop(session);
    std::fs::remove_dir_all(root).expect("temporary index directory should be removable");
}

#[test]
fn fixed_sampled_fast_is_the_only_normal_settings_path() {
    let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();

    assert!(settings.sampled_audio_policy.is_production_compatible());
    assert_eq!(settings.sampled_audio_policy.window_count, 3);
    assert_eq!(settings.sampled_audio_policy.window_seconds, 20);
    assert_eq!(settings.sampled_audio_policy.landmark_limit, 384);
}

#[test]
fn exact_same_path_can_autoplay_without_fingerprint_strength() {
    let record = record("same.mkv", 0);
    let settings = autoplay_settings();
    let decision = decide_media_match(&record, &record, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Exact);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong)
    );
    assert!(decision.same_media_for_autoplay(&settings));
}

#[test]
fn sampled_audio_match_is_probable_and_not_autoplay_eligible() {
    let query = record("query.mkv", 0);
    let candidate = record("candidate.mkv", 400);
    let settings = autoplay_settings();
    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
    assert_eq!(
        decision.evidence.metadata.duration_compatibility,
        Some(MediaDurationCompatibility::SameCutCompatible)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn same_audio_large_duration_mismatch_is_not_same_cut_or_top_rank() {
    let query = record("query.mkv", 0);
    let short = record_with_duration("a-short.mkv", 400, Some(12.0 * 60.0), 100, 48, 0);
    let same_duration =
        record_with_duration("z-same-duration.mkv", 400, Some(24.0 * 60.0), 101, 48, 0);
    let settings = autoplay_settings();

    let decision = decide_media_match(&query, &short, &settings);

    assert_eq!(
        decision.evidence.metadata.duration_compatibility,
        Some(MediaDurationCompatibility::IncompatibleSameCut)
    );
    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
    assert!(!decision.same_media_for_autoplay(&settings));

    let ranked = rank_media_match_candidates(&query, [&short, &same_duration], &settings);
    assert_eq!(
        ranked.first().unwrap().candidate_path,
        same_duration.identity.normalized_path
    );
}

#[test]
fn missing_duration_is_neutral_for_audio_match() {
    let query = record_with_duration("query.mkv", 0, None, 100, 48, 0);
    let candidate = record("candidate.mkv", 400);
    let settings = autoplay_settings();

    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(
        decision.evidence.metadata.duration_compatibility,
        Some(MediaDurationCompatibility::Unknown)
    );
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
}

#[test]
fn extension_mismatch_does_not_reject_strong_audio_match() {
    let query = record("query.mkv", 0);
    let candidate = record("candidate.mp4", 400);
    let decision = decide_media_match(&query, &candidate, &autoplay_settings());

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(decision.evidence.metadata.extension_match, Some(false));
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
}

#[test]
fn same_size_and_duration_do_not_outrank_stronger_audio() {
    let query = record("query.mkv", 0);
    let weak_same_metadata =
        record_with_duration("a-same-metadata.mkv", 400, Some(24.0 * 60.0), 100, 16, 0);
    let strong_different_size =
        record_with_duration("z-stronger-audio.mkv", 400, Some(24.0 * 60.0), 200, 48, 0);

    let ranked = rank_media_match_candidates(
        &query,
        [&weak_same_metadata, &strong_different_size],
        &autoplay_settings(),
    );

    assert_eq!(
        ranked.first().unwrap().candidate_path,
        strong_different_size.identity.normalized_path
    );
}

#[test]
fn filename_match_does_not_override_audio_rejection() {
    let query = record("Show.S01E01.mkv", 0);
    let candidate = record_with_duration("Show.S01E01.mp4", 0, Some(24.0 * 60.0), 100, 48, 10_000);

    let decision = decide_media_match(&query, &candidate, &autoplay_settings());

    assert_eq!(decision.tier, MediaMatchTier::Reject);
    assert!(decision.evidence.metadata.filename_stem_similarity.unwrap() > 0.9);
}

#[test]
fn filename_match_only_breaks_otherwise_comparable_ties() {
    let query = record("Z.Show.S01E01.mkv", 0);
    let filename_match = record_with_duration(
        "z.show.s01e01.1080p.mkv",
        400,
        Some(24.0 * 60.0),
        100,
        48,
        0,
    );
    let filename_mismatch =
        record_with_duration("a.other.s01e01.mkv", 400, Some(24.0 * 60.0), 100, 48, 0);

    let ranked = rank_media_match_candidates(
        &query,
        [&filename_mismatch, &filename_match],
        &autoplay_settings(),
    );

    assert_eq!(
        ranked.first().unwrap().candidate_path,
        filename_match.identity.normalized_path
    );
}

#[test]
fn wire_duration_mismatch_uses_shared_duration_compatibility() {
    let query = record("query.mkv", 0);
    let short = record_with_duration("short.mkv", 400, Some(12.0 * 60.0), 100, 48, 0);
    let signature = media_match_wire_signature_from_records(&[short]);

    let decision =
        decide_media_match_against_wire_signature(&query, &signature, &autoplay_settings());

    assert_eq!(
        decision.evidence.metadata.duration_compatibility,
        Some(MediaDurationCompatibility::IncompatibleSameCut)
    );
    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
}

#[test]
fn unrelated_sampled_audio_rejects() {
    let query = record("query.mkv", 0);
    let mut candidate = record("candidate.mkv", 0);
    for anchor in &mut candidate.audio_anchors {
        anchor.bucket += 10_000;
    }
    let decision = decide_media_match(&query, &candidate, &autoplay_settings());

    assert_eq!(decision.tier, MediaMatchTier::Reject);
}

fn record(path: &str, offset_ms: u32) -> MediaFingerprintRecord {
    record_with_duration(path, offset_ms, Some(24.0 * 60.0), 100, 48, 0)
}

fn record_with_duration(
    path: &str,
    offset_ms: u32,
    duration_seconds: Option<f64>,
    size_bytes: u64,
    anchor_count: u32,
    bucket_offset: u32,
) -> MediaFingerprintRecord {
    MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: path.to_owned(),
            modified_unix_millis: 1,
            size_bytes,
        },
        algorithm_version: crate::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: MediaExtractionSettings::sampled_fast_audio_index_v3(),
        duration_seconds,
        container_fingerprint: format!("fingerprint-{path}"),
        audio_anchors: (0..anchor_count)
            .map(|index| AudioAnchor {
                bucket: 100 + (index % 12) + bucket_offset,
                t_ms: index * 1_000 + offset_ms,
                weight: 10,
            })
            .collect(),
        audio_error: None,
    }
}

fn autoplay_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    }
}
