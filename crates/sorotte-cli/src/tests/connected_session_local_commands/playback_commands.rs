use super::*;

#[tokio::test]
async fn connected_client_session_sets_pause_from_local_input_channel() {
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
                b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
            )
            .await
            .expect("server hello write should succeed");
        let _ = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
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
    runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default().with_paused(false),
        );
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let (sender, mut receiver) = unbounded_channel::<String>();
    sender
        .send("pause".to_owned())
        .expect("pause command should queue");
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
    assert!(
        runtime.player().paused(),
        "local pause command should set player paused state"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_seeks_from_local_input_channel() {
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
            "a pre-Hello seek must remain queued instead of publishing an ineligible state: {early_line:?}"
        );
        writer
            .write_all(
                b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
            .await
            .expect("server hello write should succeed");
        let causal_seek = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("post-Hello seek read should succeed")
                    .expect("client should remain connected until the queued seek is published");
                let message = decode_message_line(&line).expect("post-Hello line should decode");
                if let ProtocolMessage::State(state) = message
                    && state.state.playstate.as_ref().is_some_and(|playstate| {
                        playstate.position == Some(42.0) && playstate.do_seek == Some(true)
                    })
                {
                    return;
                }
            }
        })
        .await;
        assert!(
            causal_seek.is_ok(),
            "the queued seek must cross the protocol boundary after the server Hello"
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
    sender
        .send("seek 42".to_owned())
        .expect("seek command should queue");
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
    assert_eq!(
        runtime.player().position_seconds(),
        42.0,
        "local seek command should update player position"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_holds_seek_until_playlist_selection_authority_arrives() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");
    let (sender, mut receiver) = unbounded_channel::<String>();
    let server_sender = sender.clone();

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

        let initial_barrier = 7_001.0;
        let initial_frames = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"],"user":"remote-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":0,"user":"remote-user"}}}"#,
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"remote-user","sorotteTransportRevision":1}}}"#,
            encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ping(PingPayload::new().with_latency_calculation(initial_barrier)),
            ))
            .expect("initial barrier should encode"),
        );
        writer
            .write_all(initial_frames.as_bytes())
            .await
            .expect("initial canonical frames should write");
        writer
            .flush()
            .await
            .expect("initial canonical frames should flush");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("initial barrier read should succeed")
                    .expect("client should remain connected through the initial barrier");
                let message =
                    decode_message_line(&line).expect("initial outbound line should decode");
                if let ProtocolMessage::State(state) = message
                    && state
                        .state
                        .ping
                        .as_ref()
                        .and_then(|ping| ping.latency_calculation)
                        == Some(initial_barrier)
                {
                    break;
                }
            }
        })
        .await
        .expect("initial causal barrier should complete");

        let selection_barrier = 7_002.0;
        let selection_frames = format!(
            "{}\n{}\n",
            r#"{"Set":{"playlistIndex":{"index":1,"user":"remote-user"}}}"#,
            encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ping(PingPayload::new().with_latency_calculation(selection_barrier)),
            ))
            .expect("selection barrier should encode"),
        );
        writer
            .write_all(selection_frames.as_bytes())
            .await
            .expect("successor selection frames should write");
        writer
            .flush()
            .await
            .expect("successor selection frames should flush");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("selection barrier read should succeed")
                    .expect("client should remain connected through the selection barrier");
                let message =
                    decode_message_line(&line).expect("selection outbound line should decode");
                if let ProtocolMessage::State(state) = message
                    && state
                        .state
                        .ping
                        .as_ref()
                        .and_then(|ping| ping.latency_calculation)
                        == Some(selection_barrier)
                {
                    break;
                }
            }
        })
        .await
        .expect("selection causal barrier should complete");

        server_sender
            .send("seek 42".to_owned())
            .expect("seek should queue while successor authority is pending");
        tokio::time::sleep(Duration::from_millis(100)).await;

        let held_seek_barrier = 7_003.0;
        writer
            .write_all(
                format!(
                    "{}\n",
                    encode_message_line(&ProtocolMessage::state(StatePayload::new().with_ping(
                        PingPayload::new().with_latency_calculation(held_seek_barrier),
                    ),))
                    .expect("held-seek barrier should encode"),
                )
                .as_bytes(),
            )
            .await
            .expect("held-seek barrier should write");
        writer
            .flush()
            .await
            .expect("held-seek barrier should flush");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("held-seek barrier read should succeed")
                    .expect("client should remain connected while the seek is held");
                let message =
                    decode_message_line(&line).expect("held-seek outbound line should decode");
                if let ProtocolMessage::State(state) = &message
                    && state.state.playstate.as_ref().is_some_and(|playstate| {
                        playstate.position == Some(42.0) && playstate.do_seek == Some(true)
                    })
                {
                    panic!(
                        "seek must not publish under predecessor transport authority: {message:?}"
                    );
                }
                if let ProtocolMessage::State(state) = message
                    && state
                        .state
                        .ping
                        .as_ref()
                        .and_then(|ping| ping.latency_calculation)
                        == Some(held_seek_barrier)
                {
                    break;
                }
            }
        })
        .await
        .expect("the session should remain responsive while the seek is causally fenced");

        writer
            .write_all(
                b"{\"State\":{\"playstate\":{\"position\":0.0,\"paused\":true,\"doSeek\":false,\"setBy\":\"remote-user\",\"sorotteTransportRevision\":2}}}\n",
            )
            .await
            .expect("successor canonical State should write");
        writer
            .flush()
            .await
            .expect("successor canonical State should flush");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("causal seek read should succeed")
                    .expect("client should remain connected until the held seek publishes");
                let message = decode_message_line(&line).expect("causal seek line should decode");
                if let ProtocolMessage::State(state) = message
                    && let Some(playstate) = state.state.playstate
                    && playstate.position == Some(42.0)
                    && playstate.do_seek == Some(true)
                {
                    assert_eq!(
                        playstate
                            .transport_revision()
                            .expect("causal seek transport revision should decode"),
                        Some(2),
                        "held seek must publish against successor transport authority"
                    );
                    break;
                }
            }
        })
        .await
        .expect("held seek should publish after successor authority arrives");

        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let config = ClientLoopConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        max_retries: 0,
        max_connected_runtime_seconds: 4.0,
        ..test_client_loop_config()
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
        Some(&mut receiver),
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("connected session should run");
    assert_eq!(exit, ConnectedSessionExit::TransportClosed);
    assert_eq!(
        runtime.player().position_seconds(),
        42.0,
        "held seek should reach the physical player after the fence opens"
    );
    assert!(
        !runtime.session().has_pending_playlist_index_reset_intent(),
        "successor authority should complete the selection transition"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_applies_offset_commands_from_local_input_channel() {
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
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
        let _ = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
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
    sender
        .send("offset 5".to_owned())
        .expect("absolute offset command should queue");
    sender
        .send("o +2".to_owned())
        .expect("relative offset command should queue");
    sender
        .send("o /3".to_owned())
        .expect("slash-relative offset command should queue");
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
    assert_eq!(
        runtime.player().position_seconds(),
        4.0,
        "offset command sequence should adjust local player position with legacy-like math"
    );
    server_task.await.expect("server task join should succeed");
}

#[tokio::test]
async fn connected_client_session_undoes_seek_from_local_input_channel() {
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
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
        let _ = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
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
    sender
        .send("seek 12".to_owned())
        .expect("seek command should queue");
    sender
        .send("undo".to_owned())
        .expect("undo command should queue");
    sender
        .send("undo".to_owned())
        .expect("second undo command should queue");
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
    assert_eq!(
        runtime.player().position_seconds(),
        12.0,
        "seek + undo + undo sequence should restore the seek target"
    );
    server_task.await.expect("server task join should succeed");
}
