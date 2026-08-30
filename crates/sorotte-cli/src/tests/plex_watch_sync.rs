use super::*;
use sorotte_player_api::LocalFileUpdate;
use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::mpsc;
use std::thread;
use tokio::sync::oneshot;

const TEST_PLEX_FIXTURE_DEADLINE: Duration = Duration::from_secs(15);

fn read_test_plex_request(
    stream: &mut std::net::TcpStream,
    overall_timeout: Duration,
) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .ok()?;
    read_test_plex_request_with(overall_timeout, |buffer| stream.read(buffer))
}

fn read_test_plex_request_with(
    overall_timeout: Duration,
    mut read: impl FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Option<String> {
    let deadline = std::time::Instant::now() + overall_timeout;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while request.len() < 64 * 1024 && std::time::Instant::now() < deadline {
        match read(&mut buffer) {
            Ok(0) => return None,
            Ok(read) => {
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    return Some(String::from_utf8_lossy(&request).into_owned());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return None,
        }
    }
    None
}

fn spawn_test_plex_server() -> (
    String,
    mpsc::Receiver<String>,
    oneshot::Receiver<()>,
    thread::JoinHandle<()>,
) {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
    listener
        .set_nonblocking(true)
        .expect("Plex test listener should become nonblocking");
    let addr = listener
        .local_addr()
        .expect("Plex test listener should have local addr");
    let (request_tx, request_rx) = mpsc::channel();
    let (timeline_served_tx, timeline_served_rx) = oneshot::channel();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + TEST_PLEX_FIXTURE_DEADLINE;
        let mut accepted = 0_usize;
        let mut timeline_served_tx = Some(timeline_served_tx);
        while accepted < 3 && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let Some(request) = read_test_plex_request(&mut stream, remaining) else {
                continue;
            };
            accepted += 1;
            let _ = request_tx.send(request.clone());
            let request_line = request.lines().next().unwrap_or_default();
            let body = if request_line.starts_with("GET /library/sections ") {
                r#"{"MediaContainer":{"Directory":[{"key":"1","type":"show","title":"Shows"}]}}"#
            } else if request_line.starts_with("GET /library/sections/1/all") {
                r#"{"MediaContainer":{"Metadata":[{"ratingKey":"99","type":"episode","title":"Movie Name","duration":95500,"Media":[{"Part":[{"file":"C:/media/Movie Name.mkv"}]}]}]}}"#
            } else if request_line.starts_with("GET /search") {
                r#"{"MediaContainer":{"Metadata":[{"ratingKey":"99","type":"movie","title":"Movie Name","duration":95500}]}}"#
            } else {
                "{}"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            if request_line.starts_with("GET /:/timeline?")
                && let Some(timeline_served_tx) = timeline_served_tx.take()
            {
                let _ = timeline_served_tx.send(());
            }
        }
    });
    (
        format!("http://{addr}"),
        request_rx,
        timeline_served_rx,
        handle,
    )
}

#[test]
fn plex_fixture_accumulates_complete_headers_across_transient_reads() {
    let mut reads = std::collections::VecDeque::from([
        Err(std::io::ErrorKind::Interrupted),
        Err(std::io::ErrorKind::WouldBlock),
        Ok(b"GET /library/sections HTTP/1.1\r\nHost:".to_vec()),
        Err(std::io::ErrorKind::TimedOut),
        Ok(b" localhost\r\nX-Test: yes\r\n\r\n".to_vec()),
    ]);
    let mut read_count = 0_usize;
    let request = read_test_plex_request_with(Duration::from_secs(1), |buffer| {
        read_count += 1;
        match reads
            .pop_front()
            .expect("scripted fixture reader should not need another read")
        {
            Ok(bytes) => {
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Err(kind) => Err(std::io::Error::from(kind)),
        }
    })
    .expect("fixture should retain partial headers across transient read errors");

    assert_eq!(read_count, 5);
    assert!(request.starts_with("GET /library/sections HTTP/1.1"));
    assert!(request.ends_with("X-Test: yes\r\n\r\n"));
    assert!(reads.is_empty());
}

#[test]
fn cli_plex_config_uses_stored_settings_unless_env_overrides() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let keys = [
        "SOROTTE_CLIENT_PLEX_SYNC",
        "SOROTTE_CLIENT_PLEX_STREAMING",
        "SOROTTE_CLIENT_PLEX_TOKEN",
        "SOROTTE_CLIENT_PLEX_SERVER_ID",
        "SOROTTE_CLIENT_PLEX_SERVER_URL",
        "SOROTTE_CLIENT_PLEX_SERVER_TOKEN",
    ];
    for key in keys {
        env.remove_var(key);
    }
    env.set_var("SOROTTE_CLIENT_PLEX_SERVER_URL", "http://env-plex:32400");

    let config = cli_plex_config_from_env_and_stored_settings(Some(&StoredClientSettingsMvp {
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("stored-user-token".into()),
        plex_selected_server_id: Some("stored-machine".to_owned()),
        plex_selected_server_url: Some("http://stored-plex:32400".to_owned()),
        plex_selected_server_token: Some("stored-server-token".into()),
        ..StoredClientSettingsMvp::default()
    }));

    assert!(config.enabled);
    assert!(config.streaming_enabled);
    assert_eq!(
        config
            .user_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("stored-user-token")
    );
    assert_eq!(config.selected_server_id.as_deref(), Some("stored-machine"));
    assert_eq!(
        config.selected_server_url.as_deref(),
        Some("http://env-plex:32400")
    );
    assert_eq!(
        config
            .selected_server_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("stored-server-token")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_session_reports_plex_timeline_from_player_telemetry() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let keys = [
        "SOROTTE_CLIENT_CONFIG_PATH",
        "SOROTTE_CLIENT_PLEX_SYNC",
        "SOROTTE_CLIENT_PLEX_TOKEN",
        "SOROTTE_CLIENT_PLEX_SERVER_URL",
        "SOROTTE_CLIENT_PLEX_SERVER_TOKEN",
    ];
    for key in keys {
        env.remove_var(key);
    }

    let cache_root = std::env::temp_dir().join(format!(
        "sorotte-cli-plex-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should be after epoch")
            .as_nanos()
    ));
    let config_path = cache_root.join("sorotte.ini");
    let (plex_url, plex_requests, plex_timeline_served, plex_thread) = spawn_test_plex_server();
    env.set_var("SOROTTE_CLIENT_CONFIG_PATH", config_path.as_os_str());
    // Pass Plex configuration through the production launch boundary rather
    // than publishing it in process-global environment variables for the
    // duration of this async test. Parallel connected-session tests otherwise
    // become accidental Plex clients and can consume this fixture's requests.
    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: false,
        user_token: Some("user-token".into()),
        selected_server_id: None,
        selected_server_url: Some(plex_url),
        selected_server_token: Some("server-token".into()),
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Syncplay test listener should bind");
    let addr = listener
        .local_addr()
        .expect("Syncplay test listener should have local addr");
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, _writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        let file_line = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .expect("file line read should not timeout")
            .expect("file line read should succeed")
            .expect("file line should be present");
        assert!(file_line.contains("\"Set\""));
        tokio::time::timeout(TEST_PLEX_FIXTURE_DEADLINE, plex_timeline_served)
            .await
            .expect("Plex timeline should be served before the fixture deadline")
            .expect("Plex fixture must signal timeline completion");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 20.0;
    let mut runtime = create_client_runtime(&config);
    runtime.with_player_io(|player| {
        player
            .open_file("C:/media/Movie Name.mkv")
            .expect("simulated player should accept the owned local file");
        player
            .set_paused(false)
            .expect("simulated player should accept the physical playing state");
        player
            .set_position(12.5)
            .expect("simulated player should accept the physical playback position");
        player.queue_local_file_update(
            LocalFileUpdate::new("Movie Name.mkv")
                .with_duration_seconds(95.5)
                .with_path("C:/media/Movie Name.mkv"),
        );
    });
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session_with_plex_config_for_test(
        stream,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
        &plex_config,
    )
    .await
    .expect("connected session should run");
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    server_task.await.expect("server task should join");

    let sections_request = plex_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("Plex library sections request should be sent");
    let file_lookup_request = plex_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("Plex file lookup request should be sent");
    let timeline_request = plex_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("Plex timeline request should be sent");
    assert!(
        sections_request.starts_with("GET /library/sections "),
        "first Plex request should list library sections, got {sections_request:?}"
    );
    assert!(
        sections_request
            .to_ascii_lowercase()
            .contains("x-plex-token: server-token")
    );
    assert!(
        file_lookup_request.starts_with("GET /library/sections/1/all?"),
        "second Plex request should look up the local file, got {file_lookup_request:?}"
    );
    assert!(file_lookup_request.contains("file=Movie+Name.mkv"));
    assert!(
        timeline_request.starts_with("GET /:/timeline?"),
        "third Plex request should report timeline, got {timeline_request:?}"
    );
    assert!(
        timeline_request.contains("ratingKey=99"),
        "timeline should identify the resolved item, got {timeline_request:?}"
    );
    assert!(
        timeline_request.contains("state=playing"),
        "timeline should retain the sampled playing state, got {timeline_request:?}"
    );
    assert!(
        timeline_request.contains("time=12500"),
        "timeline should retain the sampled position, got {timeline_request:?}"
    );
    assert!(
        timeline_request.contains("duration=95500"),
        "timeline should retain the sampled duration, got {timeline_request:?}"
    );
    assert!(
        timeline_request
            .to_ascii_lowercase()
            .contains("x-plex-token: server-token")
    );

    plex_thread
        .join()
        .expect("Plex test server thread should join");
    let plex_cache_path = cache_root.join("cache").join("plex-watch-cache.json");
    assert!(
        plex_cache_path.is_file(),
        "Plex watch cache should be saved under the Sorotte cache directory"
    );
    assert!(
        !cache_root.join("plex-watch-cache.json").exists(),
        "Plex watch cache should not be written next to sorotte.ini"
    );
    let _ = std::fs::remove_dir_all(cache_root);
}
