//! Independent watch-server failure and recovery regression coverage.
use super::*;

#[derive(Clone, Default)]
struct FailingOldServer {
    inner: RecordingTransport,
    old_server_down: Rc<RefCell<bool>>,
    attempts: Rc<RefCell<Vec<(String, PlexTimelineState, u64)>>>,
}

impl PlexSyncTransport for FailingOldServer {
    fn search_media(
        &self,
        server: &str,
        token: &str,
        query: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        self.inner.search_media(server, token, query)
    }
    fn search_media_by_file_name(
        &self,
        server: &str,
        token: &str,
        name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        self.inner.search_media_by_file_name(server, token, name)
    }
    fn report_timeline(
        &self,
        server: &str,
        token: &str,
        report: &PlexTimelineReport,
    ) -> PlexResult<()> {
        self.attempts
            .borrow_mut()
            .push((server.into(), report.state, report.time_millis));
        if server == SHARED_SERVER && *self.old_server_down.borrow() {
            return Err(PlexError::Http("old server temporarily unreachable".into()));
        }
        self.inner.report_timeline(server, token, report)
    }
}

impl PlexMetadataTransport for FailingOldServer {
    fn metadata_by_rating_key(
        &self,
        server: &str,
        token: &str,
        key: &str,
    ) -> PlexResult<PlexMediaMetadata> {
        self.inner.metadata_by_rating_key(server, token, key)
    }
    fn build_part_stream_url(
        &self,
        server: &str,
        token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl> {
        self.inner.build_part_stream_url(server, token, part)
    }
    fn server_machine_identifier(&self, server: &str, token: &str) -> PlexResult<String> {
        PlexMetadataTransport::server_machine_identifier(&self.inner, server, token)
    }
}

impl PlexServerDiscoveryTransport for FailingOldServer {
    fn discover_servers(&self, token: &SecretValue) -> PlexResult<Vec<PlexServerConnection>> {
        self.inner.discover_servers(token)
    }
    fn verify_server_connection(&self, server: &PlexServerConnection) -> PlexResult<()> {
        self.inner.verify_server_connection(server)
    }
    fn server_machine_identifier(&self, server: &str, token: &SecretValue) -> PlexResult<String> {
        PlexServerDiscoveryTransport::server_machine_identifier(&self.inner, server, token)
    }
}

fn fixture_engine() -> (FailingOldServer, PlexSyncEngine<FailingOldServer>) {
    let transport = FailingOldServer {
        inner: RecordingTransport {
            accessible_servers: vec![PlexServerConnection {
                name: "Shared".into(),
                machine_identifier: "shared-machine".into(),
                uri: SHARED_SERVER.into(),
                access_token: "shared-fixture-token".into(),
                owned: false,
                has_local_connection: false,
                connection_kind: PlexServerConnectionKind::Remote,
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut config = config();
    config.user_token = Some("fixture-account".into());
    let engine = PlexSyncEngine::new(config, transport.clone(), PlexMatchCache::default());
    (transport, engine)
}

fn switch_to_reachable_server(old_server_down: bool) -> usize {
    let (transport, mut engine) = fixture_engine();
    let at = |seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    assert_eq!(
        engine
            .tick(Some(direct_event("shared-machine", 120.0)), at(100))
            .state,
        PlexSyncState::Syncing
    );
    engine.tick(Some(direct_event("shared-machine", 129.0)), at(109));
    *transport.old_server_down.borrow_mut() = old_server_down;

    // A is now unreachable. B is independent, reachable, and physically being watched.
    for tick in 0..12 {
        engine.tick(
            Some(direct_event("fixture-machine", 5.0 + tick as f64).with_paused(tick == 5)),
            at(110 + tick),
        );
    }
    let reports_to_new_server = transport
        .inner
        .report_servers
        .borrow()
        .iter()
        .filter(|server| server.as_str() == SERVER)
        .count();
    eprintln!(
        "old_server_down={old_server_down}, successful new-server reports={reports_to_new_server}, attempts={:?}",
        transport.attempts.borrow()
    );
    if !old_server_down {
        assert!(
            transport
                .attempts
                .borrow()
                .iter()
                .any(|(server, state, position)| {
                    server == SERVER && *state == PlexTimelineState::Paused && *position == 10_000
                }),
            "the reachable-server control reports the new player's pause"
        );
        assert!(
            transport
                .attempts
                .borrow()
                .iter()
                .any(|(server, state, position)| {
                    server == SERVER && *state == PlexTimelineState::Playing && *position == 11_000
                }),
            "the reachable-server control reports the new player's resume"
        );
    }

    // The next valid retry after A recovers can stop A and report B normally.
    *transport.old_server_down.borrow_mut() = false;
    engine.tick(Some(direct_event("fixture-machine", 20.0)), at(130));
    let reports = transport.inner.reports.borrow();
    let servers = transport.inner.report_servers.borrow();
    assert!(servers.iter().zip(reports.iter()).any(|(server, report)| {
        server == SERVER
            && report.state == PlexTimelineState::Playing
            && report.time_millis == 20_000
    }));
    drop(servers);
    drop(reports);
    assert_eq!(
        transport
            .inner
            .report_servers
            .borrow()
            .iter()
            .zip(transport.inner.reports.borrow().iter())
            .filter(|(server, report)| {
                server.as_str() == SHARED_SERVER
                    && report.state == PlexTimelineState::Stopped
                    && report.time_millis == 129_000
            })
            .count(),
        1,
        "the old server receives one successful final report at its latest observation"
    );
    engine.tick(None, at(131));
    assert_eq!(
        transport
            .inner
            .report_servers
            .borrow()
            .last()
            .map(String::as_str),
        Some(SERVER)
    );
    let stopped = transport.inner.reports.borrow().last().cloned().unwrap();
    assert_eq!(stopped.state, PlexTimelineState::Stopped);
    assert_eq!(stopped.time_millis, 20_000);
    reports_to_new_server
}

#[test]
fn old_server_failure_does_not_block_new_server_progress() {
    assert!(
        switch_to_reachable_server(true) > 0,
        "an unreachable previous Plex server must not prevent watch progress for an independent reachable current server"
    );
}

#[test]
fn two_reachable_servers_switch_control() {
    assert!(switch_to_reachable_server(false) > 0);
}

#[test]
fn resuming_an_item_supersedes_its_failed_old_stop() {
    let (transport, mut engine) = fixture_engine();
    let at = |seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    engine.tick(Some(direct_event("shared-machine", 120.0)), at(100));
    engine.tick(Some(direct_event("shared-machine", 129.0)), at(109));
    *transport.old_server_down.borrow_mut() = true;
    engine.tick(Some(direct_event("fixture-machine", 5.0)), at(110));
    engine.tick(Some(direct_event("shared-machine", 1.0)), at(111));
    *transport.old_server_down.borrow_mut() = false;
    engine.tick(Some(direct_event("shared-machine", 2.0)), at(121));
    let reports = transport.inner.reports.borrow();
    assert_eq!(reports.last().unwrap().state, PlexTimelineState::Playing);
    assert_eq!(reports.last().unwrap().time_millis, 2_000);
    assert!(
        !reports
            .iter()
            .any(|report| report.state == PlexTimelineState::Stopped
                && report.time_millis == 129_000)
    );
    drop(reports);
    engine.tick(None, at(122));
    let reports = transport.inner.reports.borrow();
    assert_eq!(reports.last().unwrap().state, PlexTimelineState::Stopped);
    assert_eq!(reports.last().unwrap().time_millis, 2_000);
}

#[test]
fn disabling_sync_discards_deferred_stops_with_the_old_context() {
    let (transport, mut engine) = fixture_engine();
    let at = |seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    engine.tick(Some(direct_event("shared-machine", 120.0)), at(100));
    *transport.old_server_down.borrow_mut() = true;
    engine.tick(Some(direct_event("fixture-machine", 5.0)), at(110));
    let enabled = engine.config().clone();
    let mut disabled = enabled.clone();
    disabled.enabled = false;
    engine.set_config(disabled);
    *transport.old_server_down.borrow_mut() = false;
    let count = transport.inner.reports.borrow().len();
    engine.tick(None, at(150));
    engine.set_config(enabled);
    engine.tick(None, at(151));
    assert_eq!(transport.inner.reports.borrow().len(), count);
    engine.tick(Some(direct_event("shared-machine", 1.0)), at(152));
    assert_eq!(
        transport.inner.reports.borrow().last().unwrap().time_millis,
        1_000
    );
}
