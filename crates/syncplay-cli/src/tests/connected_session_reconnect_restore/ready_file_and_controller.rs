use super::*;

#[tokio::test]
async fn connected_client_session_restores_ready_and_file_after_reconnect_hello() {
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
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255"}}
"#,
                )
                .await
                .expect("server should write reconnect hello");
        writer.flush().await.expect("server flush should succeed");

        let mut outbound_messages = Vec::new();
        for _ in 0..2 {
            let line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("outbound line read should not timeout")
                .expect("outbound line read should succeed")
                .expect("outbound line should be present");
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
                                .ready
                                .as_ref()
                                .is_some_and(|ready| ready.is_ready && ready.manually_initiated == Some(false))
                    )
                }),
                "reconnect restore should emit Set.ready with restored value"
            );
        assert!(
            outbound_messages.iter().any(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(set_message)
                        if set_message
                            .set
                            .file
                            .as_ref()
                            .is_some_and(|file| file.name.as_deref() == Some("movie.mkv"))
                )
            }),
            "reconnect restore should emit Set.file with restored metadata"
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
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"cli-user"}}}"#)
        .expect("precondition ready should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"cli-user":{"room":{"name":"cli-room"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("precondition file metadata should apply");
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

#[tokio::test]
async fn connected_client_session_reidentifies_controller_when_password_is_configured() {
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
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255","features":{"managedRooms":true}}}
"#,
                )
                .await
                .expect("server should write hello response");
        writer.flush().await.expect("server flush should succeed");

        let controller_auth_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .expect("controller auth read should not timeout")
            .expect("controller auth read should succeed")
            .expect("controller auth line should be present");
        let controller_auth_message =
            decode_message_line(&controller_auth_line).expect("controller auth line should decode");
        let ProtocolMessage::Set(set_message) = controller_auth_message else {
            panic!("second client line should be Set.controllerAuth");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .expect("controller auth message should include controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "+room:ABCDEF123456".to_owned(),
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
        controlled_room_password_override: Some("ab-123-456".to_owned()),
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
