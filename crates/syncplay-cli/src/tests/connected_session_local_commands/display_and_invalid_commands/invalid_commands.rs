use super::*;

#[tokio::test]
async fn connected_client_session_invalid_playlist_commands_do_not_fall_back_to_chat_or_emit_playlist_updates()
 {
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
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

        let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
        loop {
            let remaining = scan_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
            let Ok(Ok(Some(line))) = next_line else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            assert!(
                !matches!(message, ProtocolMessage::Chat(_)),
                "invalid local playlist commands should not fall back to chat messages"
            );
            if let ProtocolMessage::Set(ref payload) = message {
                assert!(
                    payload.set.playlist_change.is_none() && payload.set.playlist_index.is_none(),
                    "invalid local playlist commands should not emit outbound playlist set messages"
                );
            }
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
        tokio::time::sleep(Duration::from_millis(120)).await;
        sender
            .send("queue".to_owned())
            .expect("queue command should queue");
        sender
            .send("select".to_owned())
            .expect("select command should queue");
        sender
            .send("select 999".to_owned())
            .expect("out-of-range select command should queue");
        sender
            .send("delete".to_owned())
            .expect("delete command should queue");
        sender
            .send("delete 999".to_owned())
            .expect("out-of-range delete command should queue");
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
async fn connected_client_session_invalid_seek_offset_commands_show_help_without_falling_back_to_chat()
 {
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
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

        let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
        loop {
            let remaining = scan_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
            let Ok(Ok(Some(line))) = next_line else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            assert!(
                !matches!(message, ProtocolMessage::Chat(_)),
                "invalid seek/offset commands should not fall back to chat messages"
            );
            if let ProtocolMessage::Set(ref payload) = message {
                assert!(
                    payload.set.room.is_none()
                        && payload.set.ready.is_none()
                        && payload.set.playlist_change.is_none()
                        && payload.set.playlist_index.is_none()
                        && payload.set.controller_auth.is_none(),
                    "invalid seek/offset commands should not emit local-command set messages: {payload:?}"
                );
            }
            assert!(
                !matches!(message, ProtocolMessage::State(_)),
                "invalid seek/offset commands should not emit outbound state messages"
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
        readiness_supported_override: Some(false),
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
        tokio::time::sleep(Duration::from_millis(120)).await;
        sender.send("seek".to_owned()).expect("seek should queue");
        sender
            .send("seek nope".to_owned())
            .expect("seek with invalid parameter should queue");
        sender
            .send("offset".to_owned())
            .expect("offset should queue");
        sender
            .send("o+oops".to_owned())
            .expect("offset shorthand with invalid parameter should queue");
        sender
            .send("s+oops".to_owned())
            .expect("seek shorthand with invalid parameter should queue");
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
