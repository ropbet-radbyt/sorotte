use crate::{
    AudioAnchor, MatchClassV3, MediaDurationCompatibility, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexBuildTransaction, MediaIndexInventoryEntry,
    MediaIndexService, MediaMatchAutoplayPolicy, MediaMatchSettings, MediaMatchTier,
    decide_media_match, decide_media_match_against_wire_signature,
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
fn media_index_inventory_change_invalidates_fingerprint_and_anchors() {
    let root = media_index_test_root("changed-invalidation");
    let session = MediaIndexService::new(&root)
        .open()
        .expect("index session should open");
    let mut original = record("/media/episode.mkv", 0);
    original.identity.modified_unix_millis = 10;
    original.identity.size_bytes = 100;
    let settings_hash = crate::media_extraction_settings_hash(&original.extraction_settings);
    session
        .save_record(&original, None)
        .expect("original fingerprint should save");

    session
        .refresh_inventory(
            &[MediaIndexInventoryEntry::new(
                original.identity.normalized_path.clone(),
                20,
                200,
            )],
            std::slice::from_ref(&original.identity.normalized_path),
            &["/media".to_owned()],
            || false,
        )
        .expect("changed inventory should refresh");

    assert!(
        session
            .load_cache(&original.extraction_settings)
            .expect("cache should load")
            .records
            .is_empty(),
        "a changed file must not retain its fingerprint"
    );
    assert_eq!(
        session.inventory_paths().expect("inventory should load"),
        vec![original.identity.normalized_path]
    );
    assert!(
        session
            .anchor_stats_dirty(&settings_hash)
            .expect("dirty marker should load"),
        "removing a changed file's anchors must invalidate aggregate anchor stats"
    );
    drop(session);
    std::fs::remove_dir_all(root).expect("temporary index directory should be removable");
}

#[test]
fn media_index_inventory_prunes_only_scanned_roots_and_reports_summary() {
    let root = media_index_test_root("prune-summary");
    let session = MediaIndexService::new(&root)
        .open()
        .expect("index session should open");
    let kept = record("/library/kept.mkv", 0);
    let stale = record("/library/stale.mkv", 0);
    let outside = record("/other/outside.mkv", 0);
    for record in [&kept, &stale, &outside] {
        session
            .save_record(record, None)
            .expect("fixture fingerprint should save");
    }

    session
        .refresh_inventory(
            &[MediaIndexInventoryEntry::new(
                kept.identity.normalized_path.clone(),
                kept.identity.modified_unix_millis,
                kept.identity.size_bytes,
            )],
            std::slice::from_ref(&kept.identity.normalized_path),
            &["/library".to_owned()],
            || false,
        )
        .expect("inventory should prune stale scanned rows");

    assert_eq!(
        session.inventory_paths().expect("inventory should load"),
        vec![
            kept.identity.normalized_path.clone(),
            outside.identity.normalized_path.clone(),
        ]
    );
    let summary = session
        .summary(&kept.extraction_settings)
        .expect("summary should load");
    assert_eq!(summary.inventory_count, 2);
    assert_eq!(summary.fixed_settings_fingerprint_count, 2);
    assert_eq!(summary.current_settings_fingerprint_count, 2);
    assert_eq!(summary.v3_fingerprint_row_count, 2);
    assert!(summary.database_bytes > 0);
    assert!(summary.v3_audio_blob_bytes > 0);
    assert!(summary.v3_audio_verify_count > 0);
    assert!(summary.v3_audio_index_count > 0);
    drop(session);
    std::fs::remove_dir_all(root).expect("temporary index directory should be removable");
}

#[test]
fn media_index_inventory_cancellation_rolls_back_every_change() {
    let root = media_index_test_root("cancel-rollback");
    let session = MediaIndexService::new(&root)
        .open()
        .expect("index session should open");
    let original = record("/media/original.mkv", 0);
    session
        .save_record(&original, None)
        .expect("original fingerprint should save");
    let entries = [
        MediaIndexInventoryEntry::new(
            original.identity.normalized_path.clone(),
            original.identity.modified_unix_millis + 1,
            original.identity.size_bytes + 1,
        ),
        MediaIndexInventoryEntry::new("/media/new.mkv", 2, 200),
    ];
    let seen = entries
        .iter()
        .map(|entry| entry.normalized_path.clone())
        .collect::<Vec<_>>();
    let mut cancellation_checks = 0;

    let error = session
        .refresh_inventory(&entries, &seen, &["/media".to_owned()], || {
            cancellation_checks += 1;
            cancellation_checks >= 2
        })
        .expect_err("refresh should be canceled after its first staged change");

    assert!(error.contains("canceled"));
    assert_eq!(
        session.inventory_paths().expect("inventory should load"),
        vec![original.identity.normalized_path.clone()],
        "the first staged upsert and invalidation must roll back"
    );
    assert!(
        session
            .load_record(
                &original.identity.normalized_path,
                &original.extraction_settings,
                original.identity.modified_unix_millis,
                original.identity.size_bytes,
            )
            .expect("record lookup should succeed")
            .is_some(),
        "the original fingerprint must survive cancellation"
    );
    drop(session);
    std::fs::remove_dir_all(root).expect("temporary index directory should be removable");
}

#[test]
fn media_index_build_begin_copies_committed_wal_pages_with_online_backup() {
    let live_root = media_index_test_root("wal-backup-live");
    let staging_root = media_index_test_root("wal-backup-staging");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("live index should initialize"));
    let connection =
        rusqlite::Connection::open(service.index_path()).expect("live SQLite database should open");
    connection
        .execute_batch(
            "PRAGMA wal_autocheckpoint = 0;
             INSERT INTO media_files_v3 (
                normalized_path,
                modified_unix_millis,
                size_bytes,
                duration_ms,
                container_fingerprint,
                updated_unix_millis
             ) VALUES ('wal-only.mkv', 1, 2, NULL, '', 3);",
        )
        .expect("WAL-only fixture row should commit");
    assert!(
        std::path::PathBuf::from(format!("{}-wal", service.index_path().display())).exists(),
        "fixture should retain a WAL sidecar while the writer is open"
    );

    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("online staging backup should succeed");
    let staged = MediaIndexService::new(&staging_root)
        .open()
        .expect("staged index should open");
    assert_eq!(
        staged
            .inventory_paths()
            .expect("staged inventory should load"),
        vec!["wal-only.mkv".to_owned()]
    );

    drop(staged);
    transaction.abort().expect("staging should abort cleanly");
    drop(connection);
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_build_commit_validates_before_replacing_live_index() {
    let live_root = media_index_test_root("validated-swap-live");
    let staging_root = media_index_test_root("validated-swap-staging");
    let live_service = MediaIndexService::new(&live_root);
    let live_session = live_service.open().expect("live index should open");
    live_session
        .save_record(&record("live.mkv", 0), None)
        .expect("live fixture should save");
    drop(live_session);

    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    let staged = MediaIndexService::new(&staging_root)
        .open()
        .expect("staged index should open");
    staged
        .save_record(&record("staged.mkv", 1), None)
        .expect("staged fixture should save");
    drop(staged);
    let old_reader = live_service
        .open()
        .expect("an old-generation reader should remain open during commit");
    assert_eq!(
        old_reader
            .inventory_paths()
            .expect("old-generation inventory should load"),
        vec!["live.mkv".to_owned()]
    );
    transaction
        .commit()
        .expect("validated staged index should become live");

    let current = live_service.open().expect("new live index should open");
    let paths = current
        .inventory_paths()
        .expect("new inventory should load");
    assert!(paths.contains(&"live.mkv".to_owned()));
    assert!(paths.contains(&"staged.mkv".to_owned()));
    drop(current);
    drop(old_reader);
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_build_validation_failure_leaves_live_index_unchanged() {
    let live_root = media_index_test_root("invalid-stage-live");
    let staging_root = media_index_test_root("invalid-stage-staging");
    let live_service = MediaIndexService::new(&live_root);
    let live_session = live_service.open().expect("live index should open");
    live_session
        .save_record(&record("live.mkv", 0), None)
        .expect("live fixture should save");
    drop(live_session);

    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    let staging_path = MediaIndexService::new(&staging_root).index_path();
    std::fs::remove_file(&staging_path).expect("staged database should be removable");
    std::fs::write(&staging_path, b"not a sqlite database")
        .expect("invalid staged database should be writable");
    let error = transaction
        .commit()
        .expect_err("invalid staged database must not replace the live index");
    assert!(error.contains("validating") || error.contains("database"));

    let current = live_service
        .open()
        .expect("original live index should still open");
    assert_eq!(
        current
            .inventory_paths()
            .expect("live inventory should load"),
        vec!["live.mkv".to_owned()]
    );
    drop(current);
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_manifest_activation_failure_leaves_previous_generation_active() {
    let live_root = media_index_test_root("manifest-failure-live");
    let staging_root = media_index_test_root("manifest-failure-staging");
    let live_service = MediaIndexService::new(&live_root);
    let live_session = live_service.open().expect("live index should open");
    live_session
        .save_record(&record("live.mkv", 0), None)
        .expect("live fixture should save");
    drop(live_session);

    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    let staged = MediaIndexService::new(&staging_root)
        .open()
        .expect("staged index should open");
    staged
        .save_record(&record("staged.mkv", 1), None)
        .expect("staged fixture should save");
    drop(staged);
    std::fs::create_dir_all(live_root.join("current.json"))
        .expect("manifest destination conflict should be created");

    transaction
        .commit()
        .expect_err("manifest replacement failure must leave the old generation active");
    let current = live_service
        .open()
        .expect("previous live index should still open");
    assert_eq!(
        current
            .inventory_paths()
            .expect("live inventory should load"),
        vec!["live.mkv".to_owned()]
    );

    drop(current);
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

fn media_index_test_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "sorotte-media-index-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos()
    ))
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
