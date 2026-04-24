use super::*;

#[tokio::test]
async fn connected_client_session_leading_tab_known_aliases_do_not_emit_outbound_actions() {
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

        let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(400);
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
                "leading-tab known alias input should not emit outbound chat"
            );
            assert!(
                !matches!(message, ProtocolMessage::State(_)),
                "leading-tab known alias input should not emit outbound state"
            );
            assert!(
                !matches!(message, ProtocolMessage::List(_)),
                "leading-tab known alias input should not emit outbound list requests"
            );
            if let ProtocolMessage::Set(ref payload) = message {
                assert!(
                    payload.set.room.is_none()
                        && payload.set.ready.is_none()
                        && payload.set.playlist_change.is_none()
                        && payload.set.playlist_index.is_none()
                        && payload.set.controller_auth.is_none(),
                    "leading-tab known alias input should not emit local command set messages: {payload:?}"
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
        max_connected_runtime_seconds: 0.6,
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
        tokio::time::sleep(Duration::from_millis(60)).await;
        sender
            .send("\u{000B}hello room".to_owned())
            .expect("vertical-tab plain message should queue");
        sender
            .send("\thello room".to_owned())
            .expect("plain message should queue");
        sender
            .send("\thelp".to_owned())
            .expect("help command should queue");
        sender
            .send("\tlist".to_owned())
            .expect("list command should queue");
        sender
            .send("\troom room2".to_owned())
            .expect("room command should queue");
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
