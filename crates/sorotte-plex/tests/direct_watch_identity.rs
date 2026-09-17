//! Direct Plex identity survives resolution, watch reporting, and server changes.
use std::{cell::RefCell, rc::Rc, time::SystemTime};

use sorotte_player_api::LocalFileUpdate;
use sorotte_plex::{
    PlexClientConfig, PlexError, PlexMatchCache, PlexMediaMetadata, PlexMediaResolver,
    PlexMediaSearchResult, PlexMediaType, PlexMetadataTransport, PlexPlayablePart, PlexResult,
    PlexServerConnection, PlexServerConnectionKind, PlexServerDiscoveryTransport, PlexSyncEngine,
    PlexSyncState, PlexSyncTransport, PlexTimelineReport, PlexTimelineState, PlexWatchEvent,
    SecretPlexPlaybackUrl,
};
use sorotte_secret::SecretValue;

const FILE_NAME: &str = "Twin.Movie.mkv";
const SERVER: &str = "http://plex.fixture:32400";
const SHARED_SERVER: &str = "http://shared.fixture:32400";

#[derive(Clone, Default)]
struct RecordingTransport {
    library: Vec<PlexMediaSearchResult>,
    filename_queries: Rc<RefCell<Vec<String>>>,
    title_queries: Rc<RefCell<Vec<String>>>,
    reports: Rc<RefCell<Vec<PlexTimelineReport>>>,
    report_servers: Rc<RefCell<Vec<String>>>,
    metadata_lookups: Rc<RefCell<Vec<(String, String)>>>,
    metadata_unavailable: Rc<RefCell<bool>>,
    accessible_servers: Vec<PlexServerConnection>,
    discovery_count: Rc<RefCell<usize>>,
}

fn library_item(rating_key: &str, edition: &str) -> PlexMediaSearchResult {
    PlexMediaSearchResult {
        rating_key: rating_key.to_owned(),
        title: "Twin Movie".to_owned(),
        parent_title: None,
        grandparent_title: None,
        media_type: PlexMediaType::Movie,
        duration_millis: Some(240_000),
        file_paths: vec![format!("/library/{edition}/{FILE_NAME}")],
    }
}

impl PlexSyncTransport for RecordingTransport {
    fn search_media(
        &self,
        server_url: &str,
        _token: &str,
        query: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        assert_eq!(server_url, SERVER);
        self.title_queries.borrow_mut().push(query.to_owned());
        // Both independent entries really match this title query.
        assert_eq!(query, "twin movie");
        Ok(self.library.clone())
    }

    fn search_media_by_file_name(
        &self,
        server_url: &str,
        _token: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        assert_eq!(server_url, SERVER);
        self.filename_queries
            .borrow_mut()
            .push(file_name.to_owned());
        Ok(self
            .library
            .iter()
            .filter(|item| {
                item.file_paths
                    .iter()
                    .any(|path| path.rsplit('/').next() == Some(file_name))
            })
            .cloned()
            .collect())
    }

    fn report_timeline(
        &self,
        server_url: &str,
        _token: &str,
        report: &PlexTimelineReport,
    ) -> PlexResult<()> {
        assert!([SERVER, SHARED_SERVER].contains(&server_url));
        self.report_servers.borrow_mut().push(server_url.to_owned());
        self.reports.borrow_mut().push(report.clone());
        Ok(())
    }
}

impl PlexMetadataTransport for RecordingTransport {
    fn metadata_by_rating_key(
        &self,
        server_url: &str,
        _token: &str,
        rating_key: &str,
    ) -> PlexResult<PlexMediaMetadata> {
        assert!([SERVER, SHARED_SERVER].contains(&server_url));
        assert_eq!(rating_key, "101");
        self.metadata_lookups
            .borrow_mut()
            .push((server_url.to_owned(), rating_key.to_owned()));
        if *self.metadata_unavailable.borrow() {
            return Err(PlexError::InvalidResponse(
                "selected item is no longer available".to_owned(),
            ));
        }
        Ok(PlexMediaMetadata {
            rating_key: "101".to_owned(),
            title: "Twin Movie".to_owned(),
            media_type: PlexMediaType::Movie,
            duration_millis: Some(240_000),
            parts: vec![PlexPlayablePart {
                id: "part-101".to_owned(),
                key: "/library/parts/101/file.mkv".to_owned(),
                file_name: Some(FILE_NAME.to_owned()),
                duration_millis: Some(240_000),
                size_bytes: Some(10_000),
                container: Some("mkv".to_owned()),
            }],
        })
    }

    fn build_part_stream_url(
        &self,
        server_url: &str,
        _token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl> {
        Ok(SecretPlexPlaybackUrl::new(format!(
            "{server_url}{}",
            part.key
        )))
    }

    fn server_machine_identifier(&self, _server_url: &str, _token: &str) -> PlexResult<String> {
        Ok("fixture-machine".to_owned())
    }
}

impl PlexServerDiscoveryTransport for RecordingTransport {
    fn discover_servers(&self, _token: &SecretValue) -> PlexResult<Vec<PlexServerConnection>> {
        *self.discovery_count.borrow_mut() += 1;
        Ok(self.accessible_servers.clone())
    }

    fn verify_server_connection(&self, _server: &PlexServerConnection) -> PlexResult<()> {
        Ok(())
    }

    fn server_machine_identifier(
        &self,
        _server_url: &str,
        _token: &SecretValue,
    ) -> PlexResult<String> {
        Ok("fixture-machine".to_owned())
    }
}

fn config() -> PlexClientConfig {
    PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        selected_server_id: Some("fixture-machine".to_owned()),
        selected_server_url: Some(SERVER.to_owned()),
        selected_server_token: Some("fixture-only".into()),
        ..PlexClientConfig::default()
    }
}

fn resolved_watch(library: Vec<PlexMediaSearchResult>) -> RecordingTransport {
    let transport = RecordingTransport {
        library,
        ..Default::default()
    };
    let mut resolver =
        PlexMediaResolver::new(config(), transport.clone(), PlexMatchCache::default());
    let target = resolver
        .resolve_stream_target(
            "plex://fixture-machine/metadata/101",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
        .expect("exact direct item must resolve");
    assert_eq!(target.matched_item.rating_key, "101");
    assert_eq!(target.logical_file.name, FILE_NAME);
    assert!(transport.filename_queries.borrow().is_empty());
    assert!(transport.title_queries.borrow().is_empty());
    // Same handoff as GUI stream load -> observed logical file -> watch engine.
    let mut engine = PlexSyncEngine::new(config(), transport.clone(), resolver.cache().clone());
    let status = engine.tick(
        Some(
            PlexWatchEvent::new(target.logical_file)
                .with_position_seconds(120.0)
                .with_paused(false),
        ),
        SystemTime::UNIX_EPOCH,
    );
    assert!(status.last_error.is_none());
    assert!(transport.filename_queries.borrow().is_empty());
    assert!(transport.title_queries.borrow().is_empty());
    transport
}

fn direct_event(machine: &str, position: f64) -> PlexWatchEvent {
    PlexWatchEvent::new(
        LocalFileUpdate::new(FILE_NAME)
            .with_path(format!("plex://{machine}/metadata/101"))
            .with_duration_seconds(240.0),
    )
    .with_position_seconds(position)
    .with_paused(false)
}

#[test]
fn direct_watch_keeps_server_identity_across_progress_stop_and_same_key_switch() {
    let transport = RecordingTransport {
        accessible_servers: vec![PlexServerConnection {
            name: "Shared".to_owned(),
            machine_identifier: "shared-machine".to_owned(),
            uri: SHARED_SERVER.to_owned(),
            access_token: "shared-fixture-token".into(),
            owned: false,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        }],
        ..Default::default()
    };
    let mut config = config();
    config.user_token = Some("fixture-account".into());
    let mut engine = PlexSyncEngine::new(config, transport.clone(), PlexMatchCache::default());
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(
        engine
            .tick(Some(direct_event("shared-machine", 120.0)), now)
            .state,
        PlexSyncState::Syncing
    );
    engine.tick(
        Some(direct_event("shared-machine", 121.0)),
        now + std::time::Duration::from_secs(1),
    );
    assert_eq!(
        *transport.discovery_count.borrow(),
        1,
        "keep the resolved endpoint for the active item"
    );
    assert_eq!(transport.metadata_lookups.borrow().len(), 1);
    assert_eq!(
        transport.reports.borrow().len(),
        1,
        "ordinary timeline throttling remains active"
    );

    // Identical numeric rating keys on different servers are different items.
    engine.tick(
        Some(direct_event("fixture-machine", 5.0)),
        now + std::time::Duration::from_secs(2),
    );
    engine.tick(None, now + std::time::Duration::from_secs(3));
    assert_eq!(
        *transport.report_servers.borrow(),
        [SHARED_SERVER, SHARED_SERVER, SERVER, SERVER]
    );
    let reports = transport.reports.borrow();
    assert_eq!(
        reports
            .iter()
            .map(|report| report.state)
            .collect::<Vec<_>>(),
        [
            PlexTimelineState::Playing,
            PlexTimelineState::Stopped,
            PlexTimelineState::Playing,
            PlexTimelineState::Stopped
        ]
    );
    assert_eq!(reports[1].time_millis, 121_000);
    assert_eq!(reports[3].time_millis, 5_000);
    assert_eq!(transport.metadata_lookups.borrow().len(), 2);
    assert!(transport.filename_queries.borrow().is_empty());
    assert!(transport.title_queries.borrow().is_empty());
}

#[test]
fn unavailable_direct_identity_does_not_fall_back_to_a_same_title_item() {
    let transport = RecordingTransport {
        library: vec![library_item("102", "edition-b")],
        ..Default::default()
    };
    *transport.metadata_unavailable.borrow_mut() = true;
    let mut engine = PlexSyncEngine::new(config(), transport.clone(), PlexMatchCache::default());
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(
        engine
            .tick(Some(direct_event("fixture-machine", 120.0)), now)
            .state,
        PlexSyncState::Error
    );
    assert!(transport.reports.borrow().is_empty());
    assert!(transport.filename_queries.borrow().is_empty());
    assert!(transport.title_queries.borrow().is_empty());
    *transport.metadata_unavailable.borrow_mut() = false;
    assert_eq!(
        engine
            .tick(
                Some(direct_event("fixture-machine", 121.0)),
                now + std::time::Duration::from_secs(1)
            )
            .state,
        PlexSyncState::Syncing
    );
    assert_eq!(transport.reports.borrow()[0].rating_key, "101");
}

#[test]
fn inaccessible_direct_server_does_not_report_its_key_to_selected_server() {
    let transport = RecordingTransport {
        library: vec![library_item("101", "edition-a")],
        ..Default::default()
    };
    let mut config = config();
    config.user_token = Some("fixture-account".into());
    let mut engine = PlexSyncEngine::new(config, transport.clone(), PlexMatchCache::default());
    assert_eq!(
        engine
            .tick(
                Some(direct_event("unavailable-machine", 120.0)),
                SystemTime::UNIX_EPOCH
            )
            .state,
        PlexSyncState::Error
    );
    assert!(transport.reports.borrow().is_empty());
    assert!(transport.metadata_lookups.borrow().is_empty());
    assert!(transport.filename_queries.borrow().is_empty());
    assert!(transport.title_queries.borrow().is_empty());
}

#[test]
fn direct_watch_does_not_require_a_selected_server() {
    let transport = RecordingTransport {
        accessible_servers: vec![PlexServerConnection {
            name: "Shared".to_owned(),
            machine_identifier: "shared-machine".to_owned(),
            uri: SHARED_SERVER.to_owned(),
            access_token: "shared-fixture-token".into(),
            owned: false,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        }],
        ..Default::default()
    };
    let config = PlexClientConfig {
        enabled: true,
        user_token: Some("fixture-account".into()),
        ..Default::default()
    };
    let mut engine = PlexSyncEngine::new(config, transport.clone(), PlexMatchCache::default());
    assert_eq!(
        engine
            .tick(
                Some(direct_event("shared-machine", 12.0)),
                SystemTime::UNIX_EPOCH
            )
            .state,
        PlexSyncState::Syncing
    );
    engine.tick(None, SystemTime::UNIX_EPOCH);
    assert_eq!(
        *transport.report_servers.borrow(),
        [SHARED_SERVER, SHARED_SERVER]
    );
    assert_eq!(
        transport.reports.borrow()[1].state,
        PlexTimelineState::Stopped
    );
}

#[test]
fn direct_plex_known_item_should_report_despite_duplicate_search_matches() {
    let transport = resolved_watch(vec![
        library_item("101", "edition-a"),
        library_item("102", "edition-b"),
    ]);
    let reports = transport.reports.borrow();
    assert_eq!(
        reports.len(),
        1,
        "exact item 101 was already resolved; ambiguous rediscovery must not suppress its watch progress"
    );
    assert_eq!(reports[0].rating_key, "101");
}

#[test]
fn direct_plex_unique_title_control_reports_progress() {
    let transport = resolved_watch(vec![library_item("101", "edition-a")]);
    let reports = transport.reports.borrow();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].rating_key, "101");
    assert_eq!(reports[0].time_millis, 120_000);
}

#[test]
fn local_file_exact_path_control_selects_among_duplicate_titles() {
    let transport = RecordingTransport {
        library: vec![
            library_item("101", "edition-a"),
            library_item("102", "edition-b"),
        ],
        ..Default::default()
    };
    let file = LocalFileUpdate::new(FILE_NAME)
        .with_path(format!("/library/edition-a/{FILE_NAME}"))
        .with_duration_seconds(240.0);
    let mut engine = PlexSyncEngine::new(config(), transport.clone(), PlexMatchCache::default());
    engine.tick(
        Some(
            PlexWatchEvent::new(file)
                .with_position_seconds(120.0)
                .with_paused(false),
        ),
        SystemTime::UNIX_EPOCH,
    );
    let reports = transport.reports.borrow();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].rating_key, "101");
    assert_eq!(
        *transport.filename_queries.borrow(),
        vec![FILE_NAME.to_owned()]
    );
    assert!(transport.title_queries.borrow().is_empty());
}

struct LoopbackPlex {
    url: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackPlex {
    fn new(duplicate: bool) -> Self {
        use std::io::{Read, Write};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_requests = requests.clone();
        let worker_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let item = |key: &str, edition: &str| {
                serde_json::json!({
                    "ratingKey": key, "title": "Twin Movie", "type": "movie", "duration": 240000,
                    "Media": [{"duration":240000, "container":"mkv", "Part":[{
                        "id":format!("part-{key}"), "key":format!("/library/parts/{key}/file.mkv"),
                        "file":format!("/library/{edition}/{FILE_NAME}"), "size":10000
                    }]}]
                })
            };
            let first = item("101", "edition-a");
            let mut search = vec![first.clone()];
            if duplicate {
                search.push(item("102", "edition-b"));
            }
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            request.push(byte[0]);
                        }
                        let request = String::from_utf8_lossy(&request);
                        let path = request.split_whitespace().nth(1).unwrap_or("/");
                        worker_requests.lock().unwrap().push(path.to_owned());
                        let json = if path == "/library/metadata/101" {
                            serde_json::json!({"MediaContainer":{"Metadata":[first]}})
                        } else if path == "/library/sections" {
                            serde_json::json!({"MediaContainer":{"Directory":[{"key":"1","type":"movie"}]}})
                        } else if path.starts_with("/search?")
                            || (path.starts_with("/library/sections/1/all?")
                                && path.contains("file=Twin.Movie.mkv"))
                        {
                            serde_json::json!({"MediaContainer":{"Metadata":search}})
                        } else {
                            serde_json::json!({"MediaContainer":{"Metadata":[]}})
                        };
                        let body = json.to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for LoopbackPlex {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn http_direct_watch(duplicate: bool) {
    let server = LoopbackPlex::new(duplicate);
    let mut config = config();
    config.selected_server_url = Some(server.url.clone());
    let transport = sorotte_plex::PlexHttpClient::new("direct-watch-loopback").unwrap();
    let mut resolver =
        PlexMediaResolver::new(config.clone(), transport.clone(), PlexMatchCache::default());
    let target = resolver
        .resolve_stream_target(
            "plex://fixture-machine/metadata/101",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
        .unwrap();
    assert_eq!(target.matched_item.rating_key, "101");
    let mut engine = PlexSyncEngine::new(config, transport, resolver.cache().clone());
    let status = engine.tick(
        Some(
            PlexWatchEvent::new(target.logical_file)
                .with_position_seconds(120.0)
                .with_paused(false),
        ),
        SystemTime::UNIX_EPOCH,
    );
    let requests = server.requests.lock().unwrap().clone();
    assert!(status.last_error.is_none());
    assert!(
        !requests
            .iter()
            .any(|path| path.starts_with("/search?") || path.starts_with("/library/sections"))
    );
    assert!(
        requests
            .iter()
            .any(|path| path.starts_with("/:/timeline?") && path.contains("ratingKey=101")),
        "a directly resolved item must receive its watch timeline; duplicate titles cannot erase known identity"
    );
}

#[test]
fn http_direct_known_item_should_report_despite_duplicate_titles() {
    http_direct_watch(true);
}

#[test]
fn http_direct_unique_title_control_reports_progress() {
    http_direct_watch(false);
}
