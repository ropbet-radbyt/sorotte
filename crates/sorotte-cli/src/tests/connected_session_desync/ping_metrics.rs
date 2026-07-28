use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_ping_updates_outbound_state_ping_metrics() {
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

        let now_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        let inbound_latency_calculation = now_seconds - 0.05;
        let inbound_client_latency_calculation = now_seconds - 0.08;
        let inbound_state_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(1.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                )
                .with_ping(
                    PingPayload::new()
                        .with_latency_calculation(inbound_latency_calculation)
                        .with_client_latency_calculation(inbound_client_latency_calculation)
                        .with_server_rtt(0.02),
                ),
        ))
        .expect("inbound state line should encode");
        writer
            .write_all(format!("{inbound_state_line}\n").as_bytes())
            .await
            .expect("inbound state write should succeed");

        let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(600);
        let mut observed_state_ping = None;
        while tokio::time::Instant::now() < scan_deadline {
            let remaining = scan_deadline.saturating_duration_since(tokio::time::Instant::now());
            let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
            let Ok(Ok(Some(line))) = next_line else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::State(state_message) = message else {
                continue;
            };
            observed_state_ping = state_message.state.ping;
            if observed_state_ping.is_some() {
                break;
            }
        }

        let ping = observed_state_ping.expect("client should emit outbound state with ping");
        let echoed_latency = ping
            .latency_calculation
            .expect("outbound state should echo inbound latencyCalculation");
        assert!(
            (echoed_latency - inbound_latency_calculation).abs() < 1e-6,
            "outbound state should echo inbound latencyCalculation exactly"
        );

        let client_latency = ping
            .client_latency_calculation
            .expect("outbound state should include clientLatencyCalculation");
        assert!(
            client_latency > 0.0,
            "outbound state should include a non-zero clientLatencyCalculation"
        );

        let client_rtt = ping
            .client_rtt
            .expect("outbound state should include clientRtt");
        assert!(
            (0.0..2.0).contains(&client_rtt),
            "outbound state should include a plausible clientRtt, got {client_rtt}"
        );

        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
        // On Windows, dropping a socket with unread client writes after the
        // write-half shutdown can turn the intended EOF into WSAECONNABORTED
        // for the peer. Drain until the client observes EOF and closes, while
        // keeping the fixture bounded if the peer fails to do so.
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            while let Ok(Some(_)) = lines.next_line().await {}
        })
        .await;
    });

    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: addr.port(),
        server_password: None,
        username: "cli-user".to_owned(),
        room: "cli-room".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 0.8,
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
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(12.5),
        );

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
async fn connected_client_session_inbound_state_ping_server_rtt_enables_borderline_fastforward_desync_correction()
 {
    async fn run_case(include_server_rtt: bool) -> f64 {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");
        // Keep the raw gap below the sustained fast-forward threshold even after room time
        // advances, while the server-RTT correction crosses it early enough to act.
        let target_position = 1.2_f64;

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            expect_client_hello_and_send_standard_test_server_hello(&mut lines, &mut writer).await;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let inbound_timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0)
                - 1.0;
            let mut inbound_ping = PingPayload::new().with_latency_calculation(inbound_timestamp);
            if include_server_rtt {
                inbound_ping = inbound_ping
                    .with_client_latency_calculation(inbound_timestamp)
                    .with_server_rtt(0.05);
            }
            let inbound_state_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_playstate(
                        PlaystatePayload::new()
                            .with_position(target_position)
                            .with_paused(false)
                            .with_do_seek(false)
                            .with_set_by("remote-user"),
                    )
                    .with_ping(inbound_ping),
            ))
            .expect("inbound state line should encode");
            writer
                .write_all(format!("{inbound_state_line}\n").as_bytes())
                .await
                .expect("inbound state write should succeed");
            writer
                .flush()
                .await
                .expect("inbound state flush should succeed");

            tokio::time::sleep(Duration::from_millis(5300)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 8.0;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(false);
        config.rewind_on_desync_override = Some(false);
        config.fastforward_on_desync_override = Some(true);
        config.slow_on_desync_override = Some(false);
        config.fastforward_threshold_seconds_override = Some(6.0);

        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .set_paused(false)
            .expect("stub player pause seed should succeed");
        runtime
            .player_mut()
            .set_position(0.2)
            .expect("stub player position seed should succeed");
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(0.2),
            );

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
        runtime.player().position_seconds()
    }

    let without_server_rtt_position = run_case(false).await;
    let with_server_rtt_position = run_case(true).await;

    assert!(
        without_server_rtt_position < 1.0,
        "without serverRtt, borderline case should not fastforward-seek; position={without_server_rtt_position}"
    );
    assert!(
        with_server_rtt_position > 1.0,
        "with serverRtt, forward-delay compensation should trigger a fastforward seek; position={with_server_rtt_position}"
    );
}
