use super::*;

#[tokio::test]
async fn connected_client_session_publishes_pending_local_file_update() {
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
        assert!(
            hello_line.contains("\"Hello\""),
            "first client line should be a Hello message"
        );

        let set_file_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .expect("set file line read should not timeout")
            .expect("set file line read should succeed")
            .expect("set file line should be present");
        let set_file_message =
            decode_message_line(&set_file_line).expect("set file line should decode");
        let ProtocolMessage::Set(set_message) = set_file_message else {
            panic!("second client line should be Set.file");
        };
        let file = set_message
            .set
            .file
            .expect("second client line should include file payload");
        assert_eq!(file.name.as_deref(), Some("movie.mkv"));
        assert_eq!(file.size.as_ref().and_then(|value| value.as_u64()), Some(0));
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
    runtime
        .player_mut()
        .open_file("movie.mkv")
        .expect("mpv adapter should accept local file open");
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
    assert_eq!(runtime.session().user_has_file("cli-user"), Some(true));
}

#[tokio::test]
async fn connected_client_session_restores_playlist_after_reconnect_empty_snapshot() {
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
                br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"sharedPlaylists":true}}}
{"Set":{"playlistChange":{"files":[]}}}
"#,
            )
            .await
            .expect("server should write reconnect Hello and empty playlist snapshot");
        writer.flush().await.expect("server flush should succeed");

        let mut outbound_messages = Vec::new();
        for _ in 0..4 {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(300), lines.next_line()).await;
            let line = match maybe_line {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break,
                Ok(Err(read_err)) => panic!("outbound line read should succeed: {read_err}"),
                Err(_) => break,
            };
            outbound_messages
                .push(decode_message_line(&line).expect("outbound line should decode"));
        }

        assert!(
                outbound_messages.iter().any(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(set_message)
                            if set_message
                                .set
                                .playlist_change
                                .as_ref()
                                .is_some_and(|payload| payload.files == vec!["episode1.mkv", "episode2.mkv"])
                    )
                }),
                "reconnect restore should emit playlistChange with cached files"
            );
        assert!(
            outbound_messages.iter().any(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(set_message)
                        if set_message
                            .set
                            .playlist_index
                            .as_ref()
                            .is_some_and(|payload| payload.index == 1)
                )
            }),
            "reconnect restore should emit playlistIndex with cached index"
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
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255"}}"#,
        )
        .expect("precondition hello should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}"#,
            )
            .expect("precondition playlist change should apply");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}"#)
        .expect("precondition playlist index should apply");
    runtime.session_mut().reset_sync_state_for_reconnect();

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
