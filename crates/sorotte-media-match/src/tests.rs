use crate::{
    AudioAnchor, MatchClassV3, MediaDurationCompatibility, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexBuildTransaction, MediaIndexCommitError,
    MediaIndexInventoryEntry, MediaIndexService, MediaMatchAutoplayPolicy, MediaMatchSettings,
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
    let MediaIndexCommitError::NotActivated(error) = error;
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
    let generation_count = std::fs::read_dir(live_root.join("generations"))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    assert_eq!(
        generation_count, 0,
        "pre-activation failure must remove its orphan generation"
    );
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

#[test]
fn media_index_pre_activation_failure_matrix_preserves_live_and_removes_artifacts() {
    use crate::media_index::MediaIndexCommitFailurePoint;

    for failure_point in [
        MediaIndexCommitFailurePoint::BeforeGenerationCreation,
        MediaIndexCommitFailurePoint::DuringGenerationCopy,
        MediaIndexCommitFailurePoint::DuringReplacementValidation,
        MediaIndexCommitFailurePoint::DuringManifestReplacement,
    ] {
        let label = format!("commit-failure-{failure_point:?}");
        let live_root = media_index_test_root(&format!("{label}-live"));
        let staging_root = media_index_test_root(&format!("{label}-staging"));
        let service = MediaIndexService::new(&live_root);
        let live = service.open().expect("live index should open");
        live.save_record(&record("live.mkv", 0), None)
            .expect("live fixture should save");
        drop(live);

        let mut transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
            .expect("staging transaction should begin");
        let staged = MediaIndexService::new(&staging_root)
            .open()
            .expect("staged index should open");
        staged
            .save_record(&record("staged.mkv", 1), None)
            .expect("staged fixture should save");
        drop(staged);
        transaction.set_test_failure_point(failure_point);

        let error = transaction
            .commit()
            .expect_err("pre-activation failure must be typed as not activated");
        assert!(matches!(error, MediaIndexCommitError::NotActivated(_)));
        assert!(
            !staging_root.exists(),
            "failed transaction must remove staging at {failure_point:?}"
        );
        let generation_count = std::fs::read_dir(live_root.join("generations"))
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0);
        assert_eq!(
            generation_count, 0,
            "failed transaction must remove orphan generations at {failure_point:?}"
        );
        let current = service
            .open()
            .expect("previous live index should remain readable");
        assert_eq!(
            current
                .inventory_paths()
                .expect("previous live inventory should load"),
            vec!["live.mkv".to_owned()]
        );
        drop(current);
        assert!(
            !live_root.join("current.json").exists(),
            "pre-activation failure must not publish a manifest at {failure_point:?}"
        );
        std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
    }
}

#[test]
fn media_index_post_activation_cleanup_failure_reports_success_with_warning() {
    use crate::media_index::MediaIndexCommitFailurePoint;

    let live_root = media_index_test_root("cleanup-failure-live");
    let staging_root = media_index_test_root("cleanup-failure-staging");
    let service = MediaIndexService::new(&live_root);
    let live = service.open().expect("live index should open");
    live.save_record(&record("live.mkv", 0), None)
        .expect("live fixture should save");
    drop(live);

    let mut transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    let staged = MediaIndexService::new(&staging_root)
        .open()
        .expect("staged index should open");
    staged
        .save_record(&record("staged.mkv", 1), None)
        .expect("staged fixture should save");
    drop(staged);
    transaction.set_test_failure_point(MediaIndexCommitFailurePoint::DuringStagingCleanup);

    let outcome = transaction
        .commit()
        .expect("post-activation cleanup failure must still report activation");
    assert!(matches!(
        outcome,
        crate::MediaIndexCommitOutcome::Activated {
            cleanup_warning: Some(_),
        }
    ));
    assert!(
        staging_root.exists(),
        "injected cleanup failure should leave staging for later maintenance"
    );
    let current = service.open().expect("activated index should open");
    let paths = current
        .inventory_paths()
        .expect("activated inventory should load");
    assert!(paths.contains(&"live.mkv".to_owned()));
    assert!(paths.contains(&"staged.mkv".to_owned()));
    drop(current);

    std::fs::remove_dir_all(staging_root).expect("staging fixture should be removable");
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_commits_retain_only_current_and_previous_generations() {
    let live_root = media_index_test_root("bounded-generations-live");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("initial index should open"));

    for index in 0..100 {
        let staging_root = media_index_test_root(&format!("bounded-generations-stage-{index}"));
        let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
            .expect("staging transaction should begin");
        let staged = MediaIndexService::new(&staging_root)
            .open()
            .expect("staged index should open");
        staged
            .save_record(&record(&format!("episode-{index}.mkv"), index), None)
            .expect("staged record should save");
        drop(staged);
        let outcome = transaction.commit().expect("generation should activate");
        assert!(matches!(
            outcome,
            crate::MediaIndexCommitOutcome::Activated { .. }
        ));
    }

    let generations = std::fs::read_dir(live_root.join("generations"))
        .expect("generation directory should exist")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert!(
        generations <= 2,
        "current plus one previous generation should be retained, found {generations}"
    );
    let retained_sizes = std::fs::read_dir(live_root.join("generations"))
        .expect("generation directory should exist")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            std::fs::metadata(entry.path().join("media-match-v3.sqlite3"))
                .ok()
                .map(|metadata| metadata.len())
        })
        .collect::<Vec<_>>();
    let retained_bytes = retained_sizes.iter().sum::<u64>();
    let largest_generation = retained_sizes.iter().copied().max().unwrap_or(0);
    assert!(
        retained_bytes <= largest_generation.saturating_mul(2),
        "retained generation bytes should be bounded by two full indexes"
    );
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[cfg(windows)]
#[test]
fn media_index_locked_old_generation_cleanup_is_deferred_without_failing_activation() {
    let live_root = media_index_test_root("locked-old-generation-live");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("initial index should open"));

    let first_staging_root = media_index_test_root("locked-old-generation-stage-1");
    let first_transaction = MediaIndexBuildTransaction::begin(&live_root, &first_staging_root)
        .expect("first staging transaction should begin");
    let first_staged = MediaIndexService::new(&first_staging_root)
        .open()
        .expect("first staged index should open");
    first_staged
        .save_record(&record("episode-1.mkv", 1), None)
        .expect("first staged record should save");
    drop(first_staged);
    first_transaction
        .commit()
        .expect("first generation should activate");

    let old_reader = service
        .open()
        .expect("reader should pin the first generation");
    for index in 2..=3 {
        let staging_root = media_index_test_root(&format!("locked-old-generation-stage-{index}"));
        let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
            .expect("staging transaction should begin");
        let staged = MediaIndexService::new(&staging_root)
            .open()
            .expect("staged index should open");
        staged
            .save_record(&record(&format!("episode-{index}.mkv"), index), None)
            .expect("staged record should save");
        drop(staged);
        let outcome = transaction
            .commit()
            .expect("locked old reader must not turn cleanup into activation failure");
        if index == 3 {
            assert!(matches!(
                outcome,
                crate::MediaIndexCommitOutcome::Activated {
                    cleanup_warning: Some(_),
                }
            ));
        }
    }

    assert!(
        old_reader
            .inventory_paths()
            .expect("pinned reader should remain readable")
            .contains(&"episode-1.mkv".to_owned())
    );
    let generations_while_locked = std::fs::read_dir(live_root.join("generations"))
        .expect("generation directory should exist")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert_eq!(
        generations_while_locked, 3,
        "locked generation should remain until its reader closes"
    );

    drop(old_reader);
    drop(
        service
            .open()
            .expect("reopen should retry deferred generation cleanup"),
    );
    let generations_after_release = std::fs::read_dir(live_root.join("generations"))
        .expect("generation directory should exist")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert!(
        generations_after_release <= 2,
        "current plus one previous generation should remain after retry, found {generations_after_release}"
    );
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_missing_or_corrupt_manifest_recovers_newest_valid_generation() {
    for corrupt in [false, true] {
        let live_root = media_index_test_root(if corrupt {
            "corrupt-manifest-recovery"
        } else {
            "missing-manifest-recovery"
        });
        let service = MediaIndexService::new(&live_root);
        let initial = service.open().expect("initial index should open");
        initial
            .save_record(&record("base.mkv", 0), None)
            .expect("base record should save");
        drop(initial);
        for (index, path) in [(1, "first.mkv"), (2, "newest.mkv")] {
            let staging_root =
                media_index_test_root(&format!("manifest-recovery-stage-{corrupt}-{index}"));
            let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
                .expect("staging transaction should begin");
            let staged = MediaIndexService::new(&staging_root)
                .open()
                .expect("staged index should open");
            staged
                .save_record(&record(path, index), None)
                .expect("staged record should save");
            drop(staged);
            transaction.commit().expect("generation should activate");
        }
        let manifest_path = live_root.join("current.json");
        if corrupt {
            std::fs::write(&manifest_path, b"{not-json").expect("manifest should be corruptible");
        } else {
            std::fs::remove_file(&manifest_path).expect("manifest should be removable");
        }

        let recovered = service
            .open()
            .expect("newest valid generation should recover");
        assert!(
            recovered
                .inventory_paths()
                .expect("inventory should load")
                .contains(&"newest.mkv".to_owned())
        );
        drop(recovered);
        let repaired: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&manifest_path).expect("manifest should be repaired"),
        )
        .expect("repaired manifest should be valid JSON");
        assert_eq!(repaired["version"], 2);
        std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
    }
}

#[test]
fn media_index_open_migrates_legacy_generation_manifest_to_bounded_v2() {
    let live_root = media_index_test_root("legacy-manifest-migration");
    let staging_root = media_index_test_root("legacy-manifest-migration-stage");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("initial index should open"));
    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    let staged = MediaIndexService::new(&staging_root)
        .open()
        .expect("staged index should open");
    staged
        .save_record(&record("episode.mkv", 1), None)
        .expect("staged record should save");
    drop(staged);
    transaction.commit().expect("generation should activate");

    let manifest_path = live_root.join("current.json");
    let current_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("V2 manifest should exist"))
            .expect("V2 manifest should parse");
    let generation = current_manifest["current"]
        .as_str()
        .expect("V2 manifest should name current generation");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "generation": generation,
        }))
        .expect("legacy manifest should serialize"),
    )
    .expect("legacy manifest should be written");

    drop(service.open().expect("legacy manifest should migrate"));
    let migrated: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&manifest_path).expect("migrated manifest should exist"),
    )
    .expect("migrated manifest should parse");
    assert_eq!(migrated["version"].as_u64(), Some(2));
    assert_eq!(migrated["current"].as_str(), Some(generation));
    assert!(migrated.get("previous").is_some());
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_missing_current_falls_back_to_previous_without_initializing() {
    let live_root = media_index_test_root("previous-generation-recovery");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("initial index should open"));
    for (index, path) in [(1, "previous.mkv"), (2, "current-only.mkv")] {
        let staging_root = media_index_test_root(&format!("previous-generation-stage-{index}"));
        let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
            .expect("staging transaction should begin");
        let staged = MediaIndexService::new(&staging_root)
            .open()
            .expect("staged index should open");
        staged
            .save_record(&record(path, index), None)
            .expect("staged record should save");
        drop(staged);
        transaction.commit().expect("generation should activate");
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(live_root.join("current.json")).expect("manifest should exist"),
    )
    .expect("manifest should parse");
    let current = manifest["current"]
        .as_str()
        .expect("current generation should be named");
    std::fs::remove_dir_all(live_root.join("generations").join(current))
        .expect("current generation should be removable");

    let recovered = service
        .open()
        .expect("previous valid generation should recover");
    let paths = recovered
        .inventory_paths()
        .expect("recovered inventory should load");
    assert!(paths.contains(&"previous.mkv".to_owned()));
    assert!(!paths.contains(&"current-only.mkv".to_owned()));
    drop(recovered);
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_missing_all_referenced_generations_fails_without_recreating_database() {
    let live_root = media_index_test_root("missing-all-generations");
    let staging_root = media_index_test_root("missing-all-generations-stage");
    let service = MediaIndexService::new(&live_root);
    drop(service.open().expect("initial index should open"));
    let transaction = MediaIndexBuildTransaction::begin(&live_root, &staging_root)
        .expect("staging transaction should begin");
    drop(
        MediaIndexService::new(&staging_root)
            .open()
            .expect("staged index should open"),
    );
    transaction.commit().expect("generation should activate");
    std::fs::remove_dir_all(live_root.join("generations"))
        .expect("all generations should be removable");

    let error = match service.open() {
        Ok(_) => panic!("broken activated references must fail closed"),
        Err(error) => error,
    };
    assert!(error.contains("no valid activated index"));
    assert!(
        !live_root.join("generations").exists(),
        "open-existing recovery must not recreate a missing generation"
    );
    std::fs::remove_dir_all(live_root).expect("temporary live index should be removable");
}

#[test]
fn media_index_open_removes_abandoned_gui_staging_directories() {
    let cache_root = media_index_test_root("abandoned-build-cache");
    let live_root = cache_root.join("media-match");
    let abandoned = cache_root
        .join(".media-match-build-dead")
        .join("cache")
        .join("media-match");
    std::fs::create_dir_all(&abandoned).expect("abandoned staging tree should be created");
    std::fs::write(abandoned.join("partial"), b"partial")
        .expect("abandoned staging content should be written");

    drop(
        MediaIndexService::new(&live_root)
            .open()
            .expect("live index should initialize"),
    );

    assert!(
        !cache_root.join(".media-match-build-dead").exists(),
        "startup should remove abandoned build roots"
    );
    std::fs::remove_dir_all(cache_root).expect("temporary cache root should be removable");
}

#[test]
fn media_index_open_preserves_gui_staging_owned_by_a_running_process() {
    let cache_root = media_index_test_root("active-build-cache");
    let live_root = cache_root.join("media-match");
    let active_build_root =
        cache_root.join(format!(".media-match-build-{}-active", std::process::id()));
    std::fs::create_dir_all(active_build_root.join("cache").join("media-match"))
        .expect("active staging tree should be created");
    std::fs::write(active_build_root.join("in-progress"), b"in progress")
        .expect("active staging marker should be written");

    drop(
        MediaIndexService::new(&live_root)
            .open()
            .expect("live index should initialize"),
    );

    assert!(
        active_build_root.exists(),
        "opening the live index must not remove an active rebuild"
    );
    std::fs::remove_dir_all(cache_root).expect("temporary cache root should be removable");
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
