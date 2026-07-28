use super::*;

#[tokio::test]
async fn connected_client_session_switches_and_identifies_on_new_controlled_room() {
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
                    br#"{"Hello":{"username":"cli-user","room":{"name":"room1"},"version":"1.2.255","features":{"managedRooms":true}}}
"#,
                )
                .await
                .expect("server should write hello response");
        writer
                .write_all(
                    br#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}
"#,
                )
                .await
                .expect("server should write new controlled room payload");
        writer.flush().await.expect("server flush should succeed");

        let room_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .expect("room update read should not timeout")
            .expect("room update read should succeed")
            .expect("room update line should be present");
        let room_message = decode_message_line(&room_line).expect("room update should decode");
        let ProtocolMessage::Set(room_set) = room_message else {
            panic!("second client line should be Set.room");
        };
        let room_payload = room_set
            .set
            .room
            .expect("second client line should include room payload");
        assert_eq!(room_payload.name, "+room:ABCDEF123456");

        let list_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .expect("list request read should not timeout")
            .expect("list request read should succeed")
            .expect("list request line should be present");
        let list_message =
            decode_message_line(&list_line).expect("list request line should decode");
        let ProtocolMessage::List(list_payload) = list_message else {
            panic!("third client line should be List.request");
        };
        assert!(matches!(list_payload.list, ListPayload::Request(_)));

        let auth_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .expect("controller auth read should not timeout")
            .expect("controller auth read should succeed")
            .expect("controller auth line should be present");
        let auth_message = decode_message_line(&auth_line).expect("controller auth should decode");
        let ProtocolMessage::Set(auth_set) = auth_message else {
            panic!("fourth client line should be Set.controllerAuth");
        };
        let controller_auth = auth_set
            .set
            .controller_auth
            .expect("fourth client line should include controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(
            controller_auth
                .password
                .as_ref()
                .map(|password| password.expose_secret()),
            Some("AB-123-456")
        );
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 0.5,
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
}

#[tokio::test]
async fn client_network_loop_reconnects_after_transport_close() {
    let _env = TestEnvGuard::lock(&CLIENT_CONNECTION_PHASE_ENV_LOCK);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        {
            let (socket_1, _) = listener
                .accept()
                .await
                .expect("first accept should succeed");
            let (reader_1, mut writer_1) = socket_1.into_split();
            let mut first_lines = BufReader::new(reader_1).lines();
            let first_tls_request = first_lines
                .next_line()
                .await
                .expect("first TLS request read should succeed")
                .expect("first TLS request should be present");
            assert!(
                first_tls_request.contains(r#""TLS":{"startTLS":"send"}"#),
                "first connection should request STARTTLS before Hello"
            );
            writer_1
                .write_all(b"{\"TLS\":{\"startTLS\":\"false\"}}\n")
                .await
                .expect("first STARTTLS decline should write");
            writer_1
                .flush()
                .await
                .expect("first STARTTLS decline should flush");
            let first_hello = first_lines
                .next_line()
                .await
                .expect("first hello read should succeed")
                .expect("first hello should be present");
            assert!(
                first_hello.contains("\"Hello\""),
                "first connection should receive hello"
            );
        }

        let (socket_2, _) = listener
            .accept()
            .await
            .expect("second accept should succeed");
        let (reader_2, mut writer_2) = socket_2.into_split();
        let mut second_lines = BufReader::new(reader_2).lines();
        let second_tls_request = second_lines
            .next_line()
            .await
            .expect("second TLS request read should succeed")
            .expect("second TLS request should be present");
        assert!(
            second_tls_request.contains(r#""TLS":{"startTLS":"send"}"#),
            "second connection should request STARTTLS before Hello"
        );
        writer_2
            .write_all(b"{\"TLS\":{\"startTLS\":\"false\"}}\n")
            .await
            .expect("second STARTTLS decline should write");
        writer_2
            .flush()
            .await
            .expect("second STARTTLS decline should flush");
        let second_hello = second_lines
            .next_line()
            .await
            .expect("second hello read should succeed")
            .expect("second hello should be present");
        assert!(
            second_hello.contains("\"Hello\""),
            "second connection should receive hello"
        );

        writer_2
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write ready update");
        writer_2.flush().await.expect("server flush should succeed");
        tokio::time::sleep(Duration::from_millis(250)).await;
        writer_2
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write second ready update");
        writer_2
            .flush()
            .await
            .expect("server second flush should succeed");
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "cli-room".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 3,
        max_connected_runtime_seconds: 0.2,
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

    run_client_network_loop(&config)
        .await
        .expect("network loop should reconnect and finish");
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn client_network_loop_retries_starttls_timeout_before_exhaustion() {
    let timeout_key = "SOROTTE_CLIENT_STARTTLS_TIMEOUT_SECONDS";
    let previous_timeout = std::env::var_os(timeout_key);
    let env = TestEnvGuard::lock(&CLIENT_CONNECTION_PHASE_ENV_LOCK);
    env.set_var(timeout_key, "0.025");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("STARTTLS timeout listener should bind");
    let addr = listener
        .local_addr()
        .expect("STARTTLS timeout listener should expose its address");
    let server_task = tokio::spawn(async move {
        let mut observed_tls_requests = Vec::new();
        for attempt in 1..=2 {
            let (socket, _) = listener.accept().await.unwrap_or_else(|error| {
                panic!("STARTTLS timeout attempt {attempt} should accept: {error}")
            });
            let (reader, _writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            let tls_request = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .unwrap_or_else(|_| {
                    panic!("STARTTLS timeout attempt {attempt} should receive a request")
                })
                .unwrap_or_else(|error| {
                    panic!("STARTTLS timeout attempt {attempt} request should read: {error}")
                })
                .unwrap_or_else(|| {
                    panic!("STARTTLS timeout attempt {attempt} request should exist")
                });
            observed_tls_requests.push(tls_request);

            let application_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .unwrap_or_else(|_| panic!("STARTTLS timeout attempt {attempt} should close"))
                .unwrap_or_else(|error| {
                    panic!("STARTTLS timeout attempt {attempt} close should read: {error}")
                });
            assert_eq!(
                application_line, None,
                "credentials/Hello must not be sent while STARTTLS is unresolved"
            );
        }
        observed_tls_requests
    });

    let config = ClientLoopConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        server_password: Some("saved-secret".into()),
        max_retries: 0,
        ..test_client_loop_config()
    };
    let result = tokio::time::timeout(Duration::from_secs(3), run_client_network_loop(&config))
        .await
        .expect("STARTTLS timeout retries should remain bounded")
        .expect_err("required STARTTLS timeouts should exhaust reconnects");
    let observed_tls_requests = tokio::time::timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server should observe the retry")
        .expect("server task should complete without panic");

    match previous_timeout {
        Some(value) => env.set_var(timeout_key, value),
        None => env.remove_var(timeout_key),
    }

    assert!(
        result.to_string().contains("STARTTLS response timed out"),
        "the final timeout should be preserved after reconnect exhaustion: {result:#}"
    );
    assert_eq!(
        observed_tls_requests.len(),
        2,
        "one initial attempt plus one reconnect should reach the server"
    );
    assert!(
        observed_tls_requests
            .iter()
            .all(|line| line.contains(r#""TLS":{"startTLS":"send"}"#))
    );
}
