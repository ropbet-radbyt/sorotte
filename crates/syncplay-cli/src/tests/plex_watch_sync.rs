use super::*;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::mpsc;
use std::thread;
use syncplay_player_api::LocalFileUpdate;

fn restore_env_key(env: &TestEnvGuard<'_>, key: &str, prior: Option<OsString>) {
    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
}

fn spawn_test_plex_server() -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
    listener
        .set_nonblocking(true)
        .expect("Plex test listener should become nonblocking");
    let addr = listener
        .local_addr()
        .expect("Plex test listener should have local addr");
    let (request_tx, request_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut accepted = 0_usize;
        while accepted < 3 && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            accepted += 1;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        request.extend_from_slice(&buffer[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
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
        }
    });
    (format!("http://{addr}"), request_rx, handle)
}

#[test]
fn cli_plex_config_uses_stored_settings_unless_env_overrides() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let keys = [
        "SYNCPLAY_CLIENT_PLEX_SYNC",
        "SYNCPLAY_CLIENT_PLEX_TOKEN",
        "SYNCPLAY_CLIENT_PLEX_SERVER_ID",
        "SYNCPLAY_CLIENT_PLEX_SERVER_URL",
        "SYNCPLAY_CLIENT_PLEX_SERVER_TOKEN",
    ];
    let prior = keys
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        env.remove_var(key);
    }
    env.set_var("SYNCPLAY_CLIENT_PLEX_SERVER_URL", "http://env-plex:32400");

    let config = cli_plex_config_from_env_and_stored_settings(Some(&StoredClientSettingsMvp {
        plex_sync_enabled: Some(true),
        plex_user_token: Some("stored-user-token".to_owned()),
        plex_selected_server_id: Some("stored-machine".to_owned()),
        plex_selected_server_url: Some("http://stored-plex:32400".to_owned()),
        plex_selected_server_token: Some("stored-server-token".to_owned()),
        ..StoredClientSettingsMvp::default()
    }));

    assert!(config.enabled);
    assert_eq!(config.user_token.as_deref(), Some("stored-user-token"));
    assert_eq!(config.selected_server_id.as_deref(), Some("stored-machine"));
    assert_eq!(
        config.selected_server_url.as_deref(),
        Some("http://env-plex:32400")
    );
    assert_eq!(
        config.selected_server_token.as_deref(),
        Some("stored-server-token")
    );

    for (key, value) in prior {
        restore_env_key(&env, key, value);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_session_reports_plex_timeline_from_player_telemetry() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let keys = [
        "SYNCPLAY_CLIENT_CONFIG_PATH",
        "SYNCPLAY_CLIENT_PLEX_SYNC",
        "SYNCPLAY_CLIENT_PLEX_TOKEN",
        "SYNCPLAY_CLIENT_PLEX_SERVER_URL",
        "SYNCPLAY_CLIENT_PLEX_SERVER_TOKEN",
    ];
    let prior = keys
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        env.remove_var(key);
    }

    let cache_root = std::env::temp_dir().join(format!(
        "syncplay-rs-cli-plex-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should be after epoch")
            .as_nanos()
    ));
    let config_path = cache_root.join("syncplay.ini");
    let (plex_url, plex_requests, plex_thread) = spawn_test_plex_server();
    env.set_var("SYNCPLAY_CLIENT_CONFIG_PATH", config_path.as_os_str());
    env.set_var("SYNCPLAY_CLIENT_PLEX_SYNC", "1");
    env.set_var("SYNCPLAY_CLIENT_PLEX_TOKEN", "user-token");
    env.set_var("SYNCPLAY_CLIENT_PLEX_SERVER_URL", &plex_url);
    env.set_var("SYNCPLAY_CLIENT_PLEX_SERVER_TOKEN", "server-token");

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
        tokio::time::sleep(Duration::from_millis(250)).await;
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 0.4;
    let mut runtime = create_client_runtime(&config);
    runtime.player_mut().queue_local_file_update(
        LocalFileUpdate::new("Movie Name.mkv")
            .with_duration_seconds(95.5)
            .with_path("C:/media/Movie Name.mkv"),
    );
    runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(12.5),
        );
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session(
        stream,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("connected session should run");
    assert!(matches!(
        exit,
        ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
    ));
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
        "first Plex request should list library sections"
    );
    assert!(
        sections_request
            .to_ascii_lowercase()
            .contains("x-plex-token: server-token")
    );
    assert!(
        file_lookup_request.starts_with("GET /library/sections/1/all?"),
        "second Plex request should look up the local file"
    );
    assert!(file_lookup_request.contains("file=Movie+Name.mkv"));
    assert!(
        timeline_request.starts_with("GET /:/timeline?"),
        "third Plex request should report timeline"
    );
    assert!(timeline_request.contains("ratingKey=99"));
    assert!(timeline_request.contains("state=playing"));
    assert!(timeline_request.contains("time=12500"));
    assert!(timeline_request.contains("duration=95500"));
    assert!(
        timeline_request
            .to_ascii_lowercase()
            .contains("x-plex-token: server-token")
    );

    plex_thread
        .join()
        .expect("Plex test server thread should join");
    let _ = std::fs::remove_dir_all(cache_root);
    for (key, value) in prior {
        restore_env_key(&env, key, value);
    }
}
