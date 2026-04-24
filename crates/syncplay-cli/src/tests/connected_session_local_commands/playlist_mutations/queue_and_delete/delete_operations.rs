use super::*;

#[tokio::test]
async fn connected_client_session_deletes_playlist_item_from_local_input_channel() {
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
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":2,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

        let mut deleted_files = None;
        let mut deleted_index = None;
        for _ in 0..8 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("playlist delete line read should not timeout")
                .expect("playlist delete line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if deleted_files.is_none()
                && let Some(change) = payload.set.playlist_change.as_ref()
            {
                deleted_files = Some(change.files.clone());
                continue;
            }
            if deleted_index.is_none()
                && let Some(index) = payload.set.playlist_index.as_ref()
            {
                deleted_index = Some(index.index);
            }
            if deleted_files.is_some() && deleted_index.is_some() {
                break;
            }
        }
        assert_eq!(
            deleted_files,
            Some(vec!["episode1.mkv".to_owned(), "episode3.mkv".to_owned()]),
            "delete command should emit playlistChange without removed file"
        );
        assert_eq!(
            deleted_index,
            Some(1),
            "delete command should emit playlistIndex adjusted to remaining file"
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
            .send("delete 2".to_owned())
            .expect("delete command should queue");
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
