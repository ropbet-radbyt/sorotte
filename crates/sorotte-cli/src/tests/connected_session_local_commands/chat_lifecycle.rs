use super::*;

#[tokio::test]
async fn connected_client_session_drops_local_chat_before_server_hello() {
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

        let early_line = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
        assert!(
            early_line.is_err(),
            "pre-hello local chat should not produce outbound protocol lines; observed={early_line:?}"
        );

        writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

        for _ in 0..3 {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            let Ok(Ok(Some(line))) = maybe_line else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            assert!(
                !matches!(message, ProtocolMessage::Chat(_)),
                "pre-hello local chat should not be queued and sent after server hello"
            );
        }
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
    let (sender, mut receiver) = unbounded_channel::<String>();
    tokio::spawn(async move {
        sender
            .send("chat hello too soon".to_owned())
            .expect("chat command should queue");
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
async fn connected_client_session_drops_local_chat_queued_between_disconnect_and_reconnect() {
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
            let first_hello = first_lines
                .next_line()
                .await
                .expect("first hello read should succeed")
                .expect("first hello should be present");
            assert!(
                first_hello.contains("\"Hello\""),
                "first connection should receive hello"
            );
            writer_1
                .shutdown()
                .await
                .expect("first writer shutdown should succeed");
        }

        let (socket_2, _) = listener
            .accept()
            .await
            .expect("second accept should succeed");
        let (reader_2, mut writer_2) = socket_2.into_split();
        let mut second_lines = BufReader::new(reader_2).lines();
        let second_hello = second_lines
            .next_line()
            .await
            .expect("second hello read should succeed")
            .expect("second hello should be present");
        assert!(
            second_hello.contains("\"Hello\""),
            "second connection should receive hello"
        );

        tokio::time::sleep(Duration::from_millis(200)).await;
        writer_2
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("second server hello write should succeed");
        writer_2
            .flush()
            .await
            .expect("second server hello flush should succeed");

        for _ in 0..3 {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(200), second_lines.next_line()).await;
            let Ok(Ok(Some(line))) = maybe_line else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            assert!(
                !matches!(message, ProtocolMessage::Chat(_)),
                "chat queued between disconnect and reconnect should be dropped before second server hello"
            );
        }

        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_retries = 0;
    config.max_connected_runtime_seconds = 0.5;
    let mut runtime = create_client_runtime(&config);
    let (sender, mut receiver) = unbounded_channel::<String>();
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let first_stream = TcpStream::connect(addr)
        .await
        .expect("first client should connect to test listener");
    let first_exit = run_connected_client_session(
        first_stream,
        &mut runtime,
        &config,
        None,
        Some(&mut receiver),
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("first connected session should run");
    assert_eq!(first_exit, ConnectedSessionExit::TransportClosed);

    sender
        .send("chat hello between sessions".to_owned())
        .expect("between-session chat command should queue");

    let second_stream = TcpStream::connect(addr)
        .await
        .expect("second client should connect to test listener");
    let second_exit = run_connected_client_session(
        second_stream,
        &mut runtime,
        &config,
        None,
        Some(&mut receiver),
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("second connected session should run");
    assert_eq!(second_exit, ConnectedSessionExit::TransportClosed);

    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_truncates_chat_message_to_session_max_length() {
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
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

        let mut chat_payload = None;
        for _ in 0..4 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("chat line read should not timeout")
                .expect("chat line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            if let ProtocolMessage::Chat(payload) = message {
                chat_payload = Some(payload.chat);
                break;
            }
        }
        let Some(chat_payload) = chat_payload else {
            panic!("client should emit chat line after server hello");
        };
        assert_eq!(
            chat_payload,
            sorotte_protocol::ChatPayload::Text("hello".to_owned())
        );
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
    let mut session_update = runtime.session_mut();
    let chat_config = session_update.chat_config_mut();
    chat_config.max_chat_message_length = 5;
    chat_config.apply_server_max_chat_message_length = false;
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session(
        stream,
        &mut runtime,
        &config,
        Some("hello room"),
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("connected session should run");
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    server_task.await.expect("server task join should succeed");
}
