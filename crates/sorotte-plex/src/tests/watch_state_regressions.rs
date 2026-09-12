use super::*;

fn watch(file: LocalFileUpdate, position: f64) -> PlexWatchEvent {
    PlexWatchEvent::new(file)
        .with_position_seconds(position)
        .with_paused(false)
}

fn select_fixture_movie(transport: &FakeTransport, file: &LocalFileUpdate, rating_key: &str) {
    *transport.file_search_results.borrow_mut() = vec![PlexMediaSearchResult {
        rating_key: rating_key.to_owned(),
        title: file.name.clone(),
        parent_title: None,
        grandparent_title: None,
        media_type: PlexMediaType::Movie,
        duration_millis: file.duration_seconds.and_then(seconds_to_millis),
        file_paths: vec![file.path.clone().unwrap_or_else(|| file.name.clone())],
    }];
}

#[test]
fn same_path_replacement_retires_the_old_item_and_refreshes_its_cached_match() {
    for (new_size, new_duration) in [(2_000, 300.0), (1_000, 600.0)] {
        let old = LocalFileUpdate::new("Current.mkv")
            .with_path("C:/Media/Current.mkv")
            .with_size_bytes(1_000)
            .with_duration_seconds(300.0);
        let new = old
            .clone()
            .with_size_bytes(new_size)
            .with_duration_seconds(new_duration);
        let transport = FakeTransport::default();
        select_fixture_movie(&transport, &old, "old");
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        engine.tick(Some(watch(old.clone(), 100.0)), now);
        engine.tick(Some(watch(old, 109.0)), now + Duration::from_secs(9));
        select_fixture_movie(&transport, &new, "replacement");
        let status = engine.tick(Some(watch(new.clone(), 1.0)), now + Duration::from_secs(10));
        assert_eq!(status.current_item.unwrap().rating_key, "replacement");
        assert_eq!(transport.file_searches.borrow().len(), 2);
        let reports = transport.reports.borrow();
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[1].rating_key, "old");
        assert_eq!(reports[1].state, PlexTimelineState::Stopped);
        assert_eq!(reports[1].time_millis, 109_000);
        assert_eq!(reports[2].rating_key, "replacement");
        drop(reports);
        // Persisted caches retain the same corroborating identity after a restart.
        let cache: PlexMatchCache =
            serde_json::from_str(&serde_json::to_string(engine.cache()).unwrap()).unwrap();
        let mut restored = PlexSyncEngine::new(engine.config().clone(), transport.clone(), cache);
        assert_eq!(
            restored
                .tick(Some(watch(new, 2.0)), now)
                .current_item
                .unwrap()
                .rating_key,
            "replacement"
        );
        assert_eq!(transport.file_searches.borrow().len(), 2);
    }
}

#[test]
fn metadata_enrichment_and_later_absence_keep_the_current_item_and_report_throttle() {
    for path in [None, Some("C:/Media/Current.mkv")] {
        let mut sparse = LocalFileUpdate::new("Current.mkv");
        sparse.path = path.map(str::to_owned);
        let known = sparse
            .clone()
            .with_size_bytes(1000)
            .with_duration_seconds(300.0);
        let transport = FakeTransport::default();
        select_fixture_movie(&transport, &known, "same");
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        for (seconds, file) in [(0, sparse.clone()), (1, known), (2, sparse)] {
            let status = engine.tick(
                Some(watch(file, 10.0 + seconds as f64)),
                now + Duration::from_secs(seconds),
            );
            assert_eq!(status.current_item.unwrap().rating_key, "same");
        }
        assert_eq!(transport.file_searches.borrow().len(), 1);
        assert_eq!(transport.reports.borrow().len(), 1);
        engine.tick(None, now + Duration::from_secs(3));
        assert_eq!(
            transport.reports.borrow().last().unwrap().time_millis,
            12_000
        );
    }
}

#[test]
fn final_stop_retries_the_latest_observation_after_periodic_and_terminal_send_failures() {
    let file = movie_file();
    let transport = FakeTransport::default();
    select_fixture_movie(&transport, &file, "movie");
    let mut engine = configured_engine(transport.clone());
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    engine.tick(Some(watch(file.clone(), 100.0)), now);
    *transport.failed_reports_remaining.borrow_mut() = 2;
    assert_eq!(
        engine
            .tick(Some(watch(file, 125.0)), now + Duration::from_secs(1))
            .state,
        PlexSyncState::Error
    );
    assert_eq!(
        engine.tick(None, now + Duration::from_secs(2)).state,
        PlexSyncState::Error
    );
    assert_eq!(
        engine.tick(None, now + Duration::from_secs(3)).state,
        PlexSyncState::Ready
    );
    engine.tick(None, now + Duration::from_secs(4));
    let reports = transport.reports.borrow();
    assert_eq!(reports.len(), 2, "a successful terminal is not repeated");
    assert_eq!(reports[1].state, PlexTimelineState::Stopped);
    assert_eq!(reports[1].time_millis, 125_000);
}

#[test]
fn old_cache_without_file_identity_is_rejected_for_reconstruction() {
    let old = r#"{"entries":{"path:movie":{"rating_key":"old","title":"Movie","media_type":"Movie","duration_millis":300000}}}"#;
    assert!(serde_json::from_str::<PlexMatchCache>(old).is_err());
}

#[test]
fn disabling_and_reenabling_sync_discards_the_previous_pending_progress() {
    let file = movie_file();
    let transport = FakeTransport::default();
    select_fixture_movie(&transport, &file, "movie");
    let mut engine = configured_engine(transport.clone());
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    engine.tick(Some(watch(file.clone(), 100.0)), now);
    engine.tick(
        Some(watch(file.clone(), 109.0)),
        now + Duration::from_secs(9),
    );
    assert_eq!(transport.reports.borrow().len(), 1);

    let enabled = engine.config().clone();
    let mut disabled = enabled.clone();
    disabled.enabled = false;
    engine.set_config(disabled);
    engine.tick(None, now + Duration::from_secs(10));
    engine.set_config(enabled);
    engine.tick(None, now + Duration::from_secs(11));
    assert_eq!(transport.reports.borrow().len(), 1);
    assert!(engine.status.current_item.is_none());

    engine.tick(Some(watch(file, 3.0)), now + Duration::from_secs(12));
    let reports = transport.reports.borrow();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].state, PlexTimelineState::Playing);
    assert_eq!(reports[1].time_millis, 3_000);
}

#[test]
fn stream_resolution_rechecks_replaced_local_file_even_when_old_metadata_still_exists() {
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir(self.0.parent().unwrap());
        }
    }
    let fixture = Fixture(plex_cache_test_path("stream-replacement").with_file_name("Current.mkv"));
    fs::write(&fixture.0, b"old").unwrap();
    let file = LocalFileUpdate::new("Current.mkv").with_path(fixture.0.to_string_lossy());
    let transport = FakeTransport::default();
    select_fixture_movie(&transport, &file, "old");
    for key in ["old", "replacement"] {
        let mut metadata = metadata_for_rating_key(key);
        metadata.parts[0].file_name = Some("Current.mkv".to_owned());
        transport
            .metadata_results
            .borrow_mut()
            .insert(key.to_owned(), metadata);
    }
    let mut resolver = PlexMediaResolver::new(
        stream_resolver_config(),
        transport.clone(),
        PlexMatchCache::default(),
    );
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(
        resolver
            .resolve_stream_target(&fixture.0.to_string_lossy(), now)
            .unwrap()
            .unwrap()
            .matched_item
            .rating_key,
        "old"
    );
    fs::write(&fixture.0, b"replacement contents").unwrap();
    select_fixture_movie(&transport, &file, "replacement");
    assert_eq!(
        resolver
            .resolve_stream_target(&fixture.0.to_string_lossy(), now)
            .unwrap()
            .unwrap()
            .matched_item
            .rating_key,
        "replacement"
    );
    assert_eq!(transport.file_searches.borrow().len(), 2);
}
