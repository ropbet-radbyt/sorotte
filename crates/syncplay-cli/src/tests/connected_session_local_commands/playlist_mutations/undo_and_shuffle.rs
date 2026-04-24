use super::*;

#[tokio::test]
async fn connected_client_session_undoes_playlist_from_local_input_channel() {
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
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

        let mut undone_files = None;
        for _ in 0..8 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("playlist undo line read should not timeout")
                .expect("playlist undo line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if let Some(change) = payload.set.playlist_change.as_ref() {
                undone_files = Some(change.files.clone());
                break;
            }
        }
        assert_eq!(
            undone_files,
            Some(Vec::<String>::new()),
            "playlist undo command should emit playlistChange with previous snapshot"
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
            .send("undoplaylist".to_owned())
            .expect("undo playlist command should queue");
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
async fn connected_client_session_shuffles_remaining_playlist_from_local_input_channel() {
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
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv","episode4.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

        let mut shuffled_files = None;
        let mut shuffled_index = None;
        for _ in 0..24 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("playlist shuffle-remaining line read should not timeout")
                .expect("playlist shuffle-remaining line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if shuffled_files.is_none()
                && let Some(change) = payload.set.playlist_change.as_ref()
            {
                shuffled_files = Some(change.files.clone());
                continue;
            }
            if shuffled_index.is_none()
                && let Some(index) = payload.set.playlist_index.as_ref()
            {
                shuffled_index = Some(index.index);
            }
            if shuffled_files.is_some() && shuffled_index.is_some() {
                break;
            }
        }
        let Some(shuffled_files) = shuffled_files else {
            panic!("shuffle remaining command should emit Set.playlistChange");
        };
        assert_eq!(
            &shuffled_files[..2],
            &["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
            "shuffle remaining command should keep entries up to current index unchanged"
        );
        let mut expected_tail = vec!["episode3.mkv".to_owned(), "episode4.mkv".to_owned()];
        let mut actual_tail = shuffled_files[2..].to_vec();
        expected_tail.sort();
        actual_tail.sort();
        assert_eq!(
            actual_tail, expected_tail,
            "shuffle remaining command should only permute remaining entries"
        );
        assert_eq!(
            shuffled_index,
            Some(1),
            "shuffle remaining command should preserve current playlist index"
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
        max_connected_runtime_seconds: 0.7,
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
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_millis(60)).await;
            sender
                .send("shuffleremainingplaylist".to_owned())
                .expect("shuffle remaining command should queue");
        }
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
async fn connected_client_session_shuffles_entire_playlist_from_local_input_channel() {
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

        let mut shuffled_files = None;
        let mut saw_index_reset = false;
        for _ in 0..12 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("playlist shuffle-entire line read should not timeout")
                .expect("playlist shuffle-entire line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(payload) = message else {
                continue;
            };
            if shuffled_files.is_none()
                && let Some(change) = payload.set.playlist_change.as_ref()
            {
                shuffled_files = Some(change.files.clone());
                continue;
            }
            if let Some(index) = payload.set.playlist_index.as_ref()
                && index.index == 0
            {
                saw_index_reset = true;
            }
            if saw_index_reset {
                break;
            }
        }
        assert!(
            saw_index_reset,
            "shuffle entire command should emit Set.playlistIndex resetting index to zero"
        );
        if let Some(shuffled_files) = shuffled_files {
            let mut expected = vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
                "episode3.mkv".to_owned(),
            ];
            let mut actual = shuffled_files;
            expected.sort();
            actual.sort();
            assert_eq!(
                actual, expected,
                "shuffle entire command should keep playlist membership unchanged"
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
        tokio::time::sleep(Duration::from_millis(120)).await;
        sender
            .send("shuffleentireplaylist".to_owned())
            .expect("shuffle entire command should queue");
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
