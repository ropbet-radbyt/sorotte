use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_pause_sync_applies_remote_mismatch_but_skips_self_setby()
 {
    async fn run_case(set_by: &str) -> bool {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");
        let set_by = set_by.to_owned();

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            expect_client_hello_and_send_standard_test_server_hello(&mut lines, &mut writer).await;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let inbound_state_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(5.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by(set_by.as_str()),
                ),
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

            tokio::time::sleep(Duration::from_millis(150)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 1.0;
        config.readiness_supported_override = Some(false);

        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .set_paused(true)
            .expect("stub player pause seed should succeed");
        runtime
            .player_mut()
            .set_position(5.0)
            .expect("stub player position seed should succeed");
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(5.0),
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
        runtime.player().paused()
    }

    let remote_set_by_paused = run_case("remote-user").await;
    let self_set_by_paused = run_case("cli-user").await;

    assert!(
        !remote_set_by_paused,
        "remote pause mismatch should unpause local player in normal connected session"
    );
    assert!(
        self_set_by_paused,
        "self-originated inbound playstate should not trigger local pause correction"
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_do_seek_applies_remote_seek_immediately() {
    async fn run_case(send_do_seek_clear: bool) -> f64 {
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
                        b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true,\"readiness\":false}}}\n",
                    )
                    .await
                    .expect("server hello write should succeed");
            writer
                .flush()
                .await
                .expect("server hello flush should succeed");

            tokio::time::sleep(Duration::from_millis(100)).await;

            let do_seek_state_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(true)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("doSeek state line should encode");
            writer
                .write_all(format!("{do_seek_state_line}\n").as_bytes())
                .await
                .expect("doSeek state write should succeed");
            writer
                .flush()
                .await
                .expect("doSeek state flush should succeed");

            if send_do_seek_clear {
                tokio::time::sleep(Duration::from_millis(120)).await;
                let clear_state_line = encode_message_line(&ProtocolMessage::state(
                    StatePayload::new()
                        .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                        .with_playstate(
                            PlaystatePayload::new()
                                .with_position(0.0)
                                .with_paused(false)
                                .with_do_seek(false)
                                .with_set_by("remote-user"),
                        ),
                ))
                .expect("doSeek-clear state line should encode");
                writer
                    .write_all(format!("{clear_state_line}\n").as_bytes())
                    .await
                    .expect("doSeek-clear state write should succeed");
                writer
                    .flush()
                    .await
                    .expect("doSeek-clear state flush should succeed");
            }

            tokio::time::sleep(Duration::from_millis(220)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 1.2;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(false);
        config.rewind_on_desync_override = Some(true);
        config.fastforward_on_desync_override = Some(false);
        config.slow_on_desync_override = Some(false);
        config.rewind_threshold_seconds_override = Some(1.0);

        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .set_paused(false)
            .expect("stub player pause seed should succeed");
        runtime
            .player_mut()
            .set_position(10.0)
            .expect("stub player position seed should succeed");
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(10.0),
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

    let position_without_do_seek_clear = run_case(false).await;
    let position_with_do_seek_clear = run_case(true).await;

    assert!(
        position_without_do_seek_clear < 1.0,
        "remote doSeek should seek immediately even before a later doSeek-clear update; position={position_without_do_seek_clear}"
    );
    assert!(
        position_with_do_seek_clear < 1.0,
        "remote doSeek should remain synced after a later doSeek-clear update; position={position_with_do_seek_clear}"
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_rewind_desync_skips_self_setby_but_applies_remote_setby()
 {
    async fn run_case(set_by: &str) -> f64 {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");
        let set_by = set_by.to_owned();

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
                        b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true,\"readiness\":false}}}\n",
                    )
                    .await
                    .expect("server hello write should succeed");
            writer
                .flush()
                .await
                .expect("server hello flush should succeed");

            tokio::time::sleep(Duration::from_millis(100)).await;

            let inbound_state_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by(set_by.as_str()),
                ),
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

            tokio::time::sleep(Duration::from_millis(180)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 1.0;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(false);
        config.rewind_on_desync_override = Some(true);
        config.fastforward_on_desync_override = Some(false);
        config.slow_on_desync_override = Some(false);
        config.rewind_threshold_seconds_override = Some(1.0);

        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .set_paused(false)
            .expect("stub player pause seed should succeed");
        runtime
            .player_mut()
            .set_position(10.0)
            .expect("stub player position seed should succeed");
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(10.0),
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

    let remote_set_by_position = run_case("remote-user").await;
    let self_set_by_position = run_case("cli-user").await;

    assert!(
        remote_set_by_position < 1.0,
        "remote-originated ahead desync should rewind local player; position={remote_set_by_position}"
    );
    assert!(
        self_set_by_position > 9.0,
        "self-originated room playstate should suppress rewind desync correction; position={self_set_by_position}"
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_fastforward_desync_skips_self_setby_but_applies_remote_setby()
 {
    async fn run_case(set_by: &str) -> f64 {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");
        let set_by = set_by.to_owned();

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
                        b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true,\"readiness\":false}}}\n",
                    )
                    .await
                    .expect("server hello write should succeed");
            writer
                .flush()
                .await
                .expect("server hello flush should succeed");

            tokio::time::sleep(Duration::from_millis(100)).await;

            let inbound_state_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by(set_by.as_str()),
                ),
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

            tokio::time::sleep(Duration::from_millis(900)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 1.5;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(false);
        config.rewind_on_desync_override = Some(false);
        config.fastforward_on_desync_override = Some(true);
        config.slow_on_desync_override = Some(false);
        config.fastforward_threshold_seconds_override = Some(2.0);

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

    let remote_set_by_position = run_case("remote-user").await;
    let self_set_by_position = run_case("cli-user").await;

    assert!(
        remote_set_by_position > 10.0,
        "remote-originated behind desync should fastforward local player; position={remote_set_by_position}"
    );
    assert!(
        self_set_by_position < 1.0,
        "self-originated room playstate should suppress fastforward desync correction; position={self_set_by_position}"
    );
}
