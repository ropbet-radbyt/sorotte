//! Physical file watch-progress contracts through a loopback Plex timeline endpoint.
use super::*;
use crate::app::testing::support::pump_worker_state;
use sorotte_client_app::app_boundary::commands::LocalOffsetCommand;
use std::io::{Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct TimelineServer {
    url: String,
    requests: std::sync::mpsc::Receiver<String>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TimelineServer {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let (tx, requests) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let len = stream.read(&mut buffer).unwrap();
                    assert!(len > 0 && request.len() < 65_536);
                    request.extend_from_slice(&buffer[..len]);
                }
                let line = String::from_utf8(request)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                assert!(
                    line.starts_with("GET /:/timeline?"),
                    "unexpected network request: {line}"
                );
                stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}").unwrap();
                tx.send(line).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn next(&self, rating_key: &str, state: &str) -> u64 {
        let request = self
            .requests
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        eprintln!("captured HTTP: {request}");
        let target = request.split_whitespace().nth(1).unwrap();
        let query = target.split_once('?').unwrap().1;
        let field = |key: &str| {
            query
                .split('&')
                .find_map(|part| {
                    let (name, value) = part.split_once('=')?;
                    (name == key).then_some(value)
                })
                .unwrap()
                .to_owned()
        };
        assert_eq!(field("ratingKey"), rating_key);
        assert_eq!(field("state"), state);
        field("time").parse().unwrap()
    }
}

impl Drop for TimelineServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn report_current_player(
    owner: &GuiPersistedConfigRuntimeOwner,
    engine: &mut PlexSyncEngine<PlexHttpClient>,
    tick: u64,
) {
    let status = engine.tick(
        owner.plex_watch_event_for_current_player(),
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100 + tick),
    );
    assert_ne!(
        status.state,
        sorotte_plex::PlexSyncState::Error,
        "{status:?}"
    );
}

fn watch_position_after_offset(offset: f64) -> (u64, u64) {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("movie.mkv");
    let next_path = fixture.path().join("next.mkv");
    std::fs::write(&path, b"fixture").unwrap();
    std::fs::write(&next_path, b"next fixture").unwrap();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(
        sorotte_player_mpv::MpvAdapter::simulated(),
    )));
    owner
        .player
        .as_mut()
        .unwrap()
        .open_file(&path.to_string_lossy())
        .unwrap();
    owner.player.as_mut().unwrap().set_position(30.0).unwrap();
    owner.player.as_mut().unwrap().set_paused(false).unwrap();
    owner.refresh_player_state_impl();
    assert_eq!(owner.player_position_seconds, Some(30.0));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = GuiRuntimeState::from_stored_settings(&StoredClientSettings::default());
    handle.push_request(GuiRuntimeRequest::SetOffset(LocalOffsetCommand::Absolute(
        offset,
    )));
    pump_worker_state(&mut owner, &handle, &mut state);
    assert_eq!(owner.user_offset_seconds, offset);
    let physical_position = owner
        .player
        .as_mut()
        .unwrap()
        .as_mpv_mut()
        .unwrap()
        .position_seconds();
    assert_eq!(physical_position, 30.0 + offset);
    assert_eq!(
        owner.player_position_seconds,
        Some(30.0),
        "room projection remains offset-independent"
    );
    let server = TimelineServer::start();
    let config = sorotte_plex::PlexClientConfig {
        enabled: true,
        selected_server_id: Some("fixture-server".into()),
        selected_server_url: Some(server.url.clone()),
        selected_server_token: Some("fixture-token".into()),
        ..Default::default()
    };
    let mut cache = PlexMatchCache::default();
    for (media_path, rating_key) in [(&path, "movie"), (&next_path, "next")] {
        let file = LocalFileUpdate::new(media_path.file_name().unwrap().to_string_lossy())
            .with_path(media_path.to_string_lossy());
        cache.entries.insert(
            sorotte_plex::server_scoped_cache_key_for_file(&config, &file).unwrap(),
            sorotte_plex::PlexCachedMatch {
                rating_key: rating_key.into(),
                title: rating_key.into(),
                media_type: sorotte_plex::PlexMediaType::Movie,
                duration_millis: Some(300_000),
                file_identity: Default::default(),
            },
        );
    }
    let mut engine = PlexSyncEngine::new(
        config,
        PlexHttpClient::new("watch-coordinate-test").unwrap(),
        cache,
    );
    report_current_player(&owner, &mut engine, 0);
    let time_millis = server.next("movie", "playing");
    eprintln!(
        "offset={offset}, physical={physical_position}, room={:?}, Plex report={}ms",
        owner.player_position_seconds, time_millis
    );

    // Ordinary pause/resume preserves item attribution and exposes the same timeline domain.
    for (tick, paused, status) in [(1, true, "paused"), (2, false, "playing")] {
        handle.push_request(GuiRuntimeRequest::SetPlaybackPaused(paused));
        pump_worker_state(&mut owner, &handle, &mut state);
        report_current_player(&owner, &mut engine, tick);
        assert_eq!(server.next("movie", status), time_millis);
    }

    // An ordinary external player OpenFile observation stops the old item and starts the new one.
    // The offset-only detached session has no room transport, so this models the mpv-side switch.
    owner
        .player
        .as_mut()
        .unwrap()
        .open_file(&next_path.to_string_lossy())
        .unwrap();
    owner.refresh_player_state_impl();
    let loaded_path = owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.as_ref())
        .unwrap();
    assert_eq!(
        sorotte_media_match::normalize_media_path(std::path::Path::new(loaded_path)),
        sorotte_media_match::normalize_media_path(&next_path)
    );
    handle.push_request(GuiRuntimeRequest::SeekToPosition(50.0));
    handle.push_request(GuiRuntimeRequest::SetPlaybackPaused(false));
    pump_worker_state(&mut owner, &handle, &mut state);
    assert_eq!(
        owner
            .player
            .as_mut()
            .unwrap()
            .as_mpv_mut()
            .unwrap()
            .position_seconds(),
        50.0 + offset
    );
    report_current_player(&owner, &mut engine, 3);
    assert_eq!(server.next("movie", "stopped"), time_millis);
    assert_eq!(
        server.next("next", "playing"),
        ((50.0 + offset) * 1000.0) as u64
    );
    // A later ordinary offset reset must still work, independently of the failure oracle.
    handle.push_request(GuiRuntimeRequest::SetOffset(LocalOffsetCommand::Absolute(
        0.0,
    )));
    pump_worker_state(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SeekToPosition(70.0));
    pump_worker_state(&mut owner, &handle, &mut state);
    assert_eq!(
        owner
            .player
            .as_mut()
            .unwrap()
            .as_mpv_mut()
            .unwrap()
            .position_seconds(),
        70.0
    );
    // A ten-second movement need not bypass Plex's ordinary report throttle.
    report_current_player(&owner, &mut engine, 14);
    assert_eq!(server.next("next", "playing"), 70_000);
    (time_millis, (physical_position * 1000.0) as u64)
}

#[test]
fn plex_progress_uses_physical_position_with_positive_offset() {
    let (actual, expected) = watch_position_after_offset(10.0);
    assert_eq!(
        actual, expected,
        "Plex progress must refer to the watched file timeline"
    );
}

#[test]
fn plex_progress_uses_physical_position_with_negative_offset() {
    let (actual, expected) = watch_position_after_offset(-10.0);
    assert_eq!(
        actual, expected,
        "Plex progress must refer to the watched file timeline"
    );
}

#[test]
fn plex_progress_zero_offset_control() {
    let (actual, expected) = watch_position_after_offset(0.0);
    assert_eq!(actual, expected);
}
