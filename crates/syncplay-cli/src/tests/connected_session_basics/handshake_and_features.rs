use super::*;

#[tokio::test]
async fn connected_client_session_sends_hello_and_applies_inbound_set_ready() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(
            hello_line.contains("\"Hello\""),
            "first client line should be a Hello message"
        );

        writer
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write ready update");
        writer.flush().await.expect("server flush should succeed");
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "cli-room".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 2.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
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
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    server_task.await.expect("server task join should succeed");

    assert_eq!(runtime.session().user_ready("cli-user"), Some(true));
}

#[tokio::test]
async fn connected_client_session_sends_ready_on_server_hello_when_ready_at_start_enabled() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

        let mut ready_payload = None;
        for _ in 0..4 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("ready line read should not timeout")
                .expect("ready line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if let Some(ready) = payload.set.ready {
                ready_payload = Some(ready);
                break;
            }
        }

        let Some(ready_payload) = ready_payload else {
            panic!("client should emit Set.ready when readyAtStart is enabled");
        };
        assert_eq!(ready_payload.is_ready, Some(true));
        assert_eq!(ready_payload.manually_initiated, Some(false));
        assert!(
            ready_payload.username.is_none(),
            "auto-ready uses local Set.ready payload without explicit username"
        );
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "cli-room".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 0.5,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: Some(true),
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
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
    assert!(
        matches!(
            exit,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "connected session should either observe peer close or exit on runtime window"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_sends_ready_when_server_hello_is_not_first_batched_command() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
            .write_all(
                br#"{"Set":{"features":{"chat":true}},"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"readiness":true}}}
"#,
            )
            .await
            .expect("batched server line write should succeed");

        let mut ready_payload = None;
        for _ in 0..4 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("ready line read should not timeout")
                .expect("ready line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if let Some(ready) = payload.set.ready {
                ready_payload = Some(ready);
                break;
            }
        }

        let Some(ready_payload) = ready_payload else {
            panic!("client should emit Set.ready when batched Hello enables ready-at-start");
        };
        assert_eq!(ready_payload.is_ready, Some(true));
        assert_eq!(ready_payload.manually_initiated, Some(false));
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.ready_at_start_override = Some(true);
    config.max_connected_runtime_seconds = 0.5;

    let mut runtime = create_client_runtime(&config);
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
    assert!(
        matches!(
            exit,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "connected session should either observe peer close or exit on runtime window"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_replies_when_state_is_not_first_batched_command() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
            .write_all(
                br#"{"Set":{"features":{"chat":true}},"State":{"ping":{"latencyCalculation":1.25},"playstate":{"position":5.0,"paused":true,"doSeek":true}}}
"#,
            )
            .await
            .expect("batched server line write should succeed");

        let mut state_response = None;
        for _ in 0..4 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("state response read should not timeout")
                .expect("state response read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            if let ProtocolMessage::State(payload) = message {
                state_response = Some(payload);
                break;
            }
        }

        let Some(state_response) = state_response else {
            panic!("client should emit State response for non-first batched State");
        };
        assert!(
            state_response
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation)
                .is_some(),
            "State response should carry client latency telemetry"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 0.1;

    let mut runtime = create_client_runtime(&config);
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
    assert!(
        matches!(
            exit,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "connected session should either observe peer close or exit on runtime window"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_processes_valid_batched_prefix_before_unknown_command() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
            .write_all(
                br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"readiness":true}},"State":{"ping":{"latencyCalculation":1.25},"playstate":{"position":5.0,"paused":true,"doSeek":true}},"Bogus":{"x":1}}
"#,
            )
            .await
            .expect("mixed batched server line write should succeed");

        let mut ready_payload = None;
        let mut state_response = None;
        for _ in 0..6 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("response line read should not timeout")
                .expect("response line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            match message {
                ProtocolMessage::Set(payload) => {
                    if let Some(ready) = payload.set.ready {
                        ready_payload = Some(ready);
                    }
                }
                ProtocolMessage::State(payload) => {
                    state_response = Some(payload);
                }
                _ => {}
            }
            if ready_payload.is_some() && state_response.is_some() {
                break;
            }
        }

        let Some(ready_payload) = ready_payload else {
            panic!("client should emit Set.ready before dropping the mixed batched line");
        };
        assert_eq!(ready_payload.is_ready, Some(true));
        assert_eq!(ready_payload.manually_initiated, Some(false));
        let Some(state_response) = state_response else {
            panic!("client should emit State response before dropping the mixed batched line");
        };
        assert!(
            state_response
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation)
                .is_some(),
            "State response should carry client latency telemetry"
        );
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.ready_at_start_override = Some(true);
    config.max_connected_runtime_seconds = 0.5;

    let mut runtime = create_client_runtime(&config);
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let result = run_connected_client_session(
        stream,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await;
    assert!(
        result.is_err(),
        "client should drop after reaching the trailing unknown command"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_includes_shared_playlists_feature_in_hello_when_configured() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, _writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        let message = decode_message_line(&hello_line).expect("hello line should decode");
        let ProtocolMessage::Hello(hello_message) = message else {
            panic!("first client line should be a Hello message");
        };
        assert_eq!(hello_message.hello.version, "1.2.255");
        assert_eq!(
            hello_message.hello.realversion.as_deref(),
            Some(syncplay_client_core::SYNCPLAY_COMPAT_VERSION_LEGACY)
        );
        let features = hello_message
            .hello
            .features
            .as_ref()
            .and_then(Value::as_object)
            .expect("hello should advertise a feature map");
        assert_eq!(
            features.get("sharedPlaylists").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(features.get("chat").and_then(Value::as_bool), Some(true));
        assert_eq!(features.get("uiMode").and_then(Value::as_str), Some("CLI"));
        assert_eq!(
            features.get("featureList").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            features.get("readiness").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            features.get("managedRooms").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            features.get("persistentRooms").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            features.get("setOthersReadiness").and_then(Value::as_bool),
            Some(true)
        );
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.shared_playlists_enabled_override = Some(false);
    config.max_connected_runtime_seconds = 2.0;

    let mut runtime = create_client_runtime(&config);
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
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_suppresses_playlist_commands_when_shared_playlists_disabled() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"sharedPlaylists\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        let mut saw_playlist_set = false;
        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let maybe_line = tokio::time::timeout(remaining, lines.next_line()).await;
            let Some(line) = (match maybe_line {
                Ok(Ok(line)) => line,
                Ok(Err(err)) => panic!("client line read should succeed: {err}"),
                Err(_) => None,
            }) else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if payload.set.playlist_change.is_some() || payload.set.playlist_index.is_some() {
                saw_playlist_set = true;
                break;
            }
        }
        assert!(
            !saw_playlist_set,
            "shared playlist commands should be suppressed when sharedPlaylistEnabled=false"
        );
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.shared_playlists_enabled_override = Some(false);
    config.max_connected_runtime_seconds = 0.5;

    let mut runtime = create_client_runtime(&config);
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let (sender, mut receiver) = unbounded_channel::<String>();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        sender
            .send("qa episode3.mkv".to_owned())
            .expect("queue command should queue");
    });
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session(
        stream,
        &mut runtime,
        &config,
        None,
        Some(&mut receiver),
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("connected session should run");
    assert!(
        matches!(
            exit,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "connected session should either observe peer close or exit on runtime window"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_includes_hashed_server_password_in_hello_when_configured() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, _writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        let message = decode_message_line(&hello_line).expect("hello line should decode");
        let ProtocolMessage::Hello(hello_message) = message else {
            panic!("first client line should be a Hello message");
        };
        assert_eq!(
            hello_message.hello.extra.get("password"),
            Some(&Value::String(
                "e8e1176287cec19598090813ad01afab".to_owned()
            ))
        );
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.server_password = Some("server-secret".to_owned());
    config.max_connected_runtime_seconds = 2.0;

    let mut runtime = create_client_runtime(&config);
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
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    server_task.await.expect("server task join should succeed");
}
