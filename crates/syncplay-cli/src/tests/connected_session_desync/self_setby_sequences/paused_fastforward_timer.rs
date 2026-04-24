use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_paused_self_setby_fastforward_candidate_primes_timer_for_next_remote_state()
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

        expect_client_hello_and_send_standard_test_server_hello(&mut lines, &mut writer).await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let paused_self_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("paused self fastforward-candidate state should encode");
        writer
            .write_all(format!("{paused_self_line}\n").as_bytes())
            .await
            .expect("paused self state write should succeed");
        writer
            .flush()
            .await
            .expect("paused self state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let paused_remote_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(true)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
        ))
        .expect("paused remote fastforward-candidate state should encode");
        writer
            .write_all(format!("{paused_remote_line}\n").as_bytes())
            .await
            .expect("paused remote state write should succeed");
        writer
            .flush()
            .await
            .expect("paused remote state flush should succeed");

        tokio::time::sleep(Duration::from_millis(10)).await;
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.0;
    config.readiness_supported_override = Some(false);
    config.local_can_control_override = Some(false);
    config.rewind_on_desync_override = Some(false);
    config.fastforward_on_desync_override = Some(true);
    config.slow_on_desync_override = Some(false);
    config.fastforward_threshold_seconds_override = Some(2.0);

    let mut runtime = create_client_runtime(&config);
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 0.2);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");
    assert!(
        runtime.player().paused(),
        "remote paused state should still pause local player"
    );
    assert!(
        runtime.player().position_seconds() >= 10.0,
        "remote paused state should sync to the room position even after a prior self-setBy behind sample; position={}",
        runtime.player().position_seconds()
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_paused_self_setby_near_sync_clears_fastforward_timer_before_next_remote_state()
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
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true,\"readiness\":false}}}\n",
                )
                .await
                .expect("server hello write should succeed");
        writer
            .flush()
            .await
            .expect("server hello flush should succeed");

        tokio::time::sleep(Duration::from_millis(100)).await;
        let paused_self_behind_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("paused self behind state should encode");
        writer
            .write_all(format!("{paused_self_behind_line}\n").as_bytes())
            .await
            .expect("paused self behind state write should succeed");
        writer
            .flush()
            .await
            .expect("paused self behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let paused_self_near_sync_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.3)
                        .with_paused(true)
                        .with_do_seek(false)
                        .with_set_by("cli-user"),
                ),
        ))
        .expect("paused self near-sync state should encode");
        writer
            .write_all(format!("{paused_self_near_sync_line}\n").as_bytes())
            .await
            .expect("paused self near-sync state write should succeed");
        writer
            .flush()
            .await
            .expect("paused self near-sync state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let paused_remote_behind_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(true)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
        ))
        .expect("paused remote behind state should encode");
        writer
            .write_all(format!("{paused_remote_behind_line}\n").as_bytes())
            .await
            .expect("paused remote behind state write should succeed");
        writer
            .flush()
            .await
            .expect("paused remote behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(10)).await;
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.2;
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
    assert!(
        runtime.player().paused(),
        "remote paused state should still pause local player"
    );
    assert!(
        runtime.player().position_seconds() >= 10.0,
        "remote paused state should sync to the room position after a prior self-setBy near-sync sample; position={}",
        runtime.player().position_seconds()
    );
}
