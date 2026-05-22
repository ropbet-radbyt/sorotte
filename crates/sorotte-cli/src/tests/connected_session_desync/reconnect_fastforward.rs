use super::*;

#[tokio::test]
async fn connected_client_session_reconnect_clears_stale_self_setby_fastforward_suppression_window_before_second_session_desync_evaluation()
 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        // First connection: create a self-setBy fastforward suppression window in session state.
        {
            let (socket_1, _) = listener
                .accept()
                .await
                .expect("first accept should succeed");
            let (reader_1, mut writer_1) = socket_1.into_split();
            let mut lines_1 = BufReader::new(reader_1).lines();
            expect_client_hello_and_send_standard_test_server_hello(&mut lines_1, &mut writer_1)
                .await;

            tokio::time::sleep(Duration::from_millis(100)).await;
            let self_behind_1 = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("cli-user"),
                ),
            ))
            .expect("first self behind state should encode");
            writer_1
                .write_all(format!("{self_behind_1}\n").as_bytes())
                .await
                .expect("first self behind state write should succeed");
            writer_1
                .flush()
                .await
                .expect("first self behind state flush should succeed");

            tokio::time::sleep(Duration::from_millis(320)).await;
            let self_behind_2 = encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                    .with_playstate(
                        PlaystatePayload::new()
                            .with_position(10.0)
                            .with_paused(false)
                            .with_do_seek(false)
                            .with_set_by("cli-user"),
                    ),
            ))
            .expect("second self behind state should encode");
            writer_1
                .write_all(format!("{self_behind_2}\n").as_bytes())
                .await
                .expect("second self behind state write should succeed");
            writer_1
                .flush()
                .await
                .expect("second self behind state flush should succeed");

            tokio::time::sleep(Duration::from_millis(10)).await;
            writer_1
                .shutdown()
                .await
                .expect("first writer shutdown should succeed");
        }

        // Second connection: after reconnect reset, a fresh behind timer should start and
        // then trigger again within the new session's threshold window.
        let (socket_2, _) = listener
            .accept()
            .await
            .expect("second accept should succeed");
        let (reader_2, mut writer_2) = socket_2.into_split();
        let mut lines_2 = BufReader::new(reader_2).lines();
        expect_client_hello_and_send_standard_test_server_hello(&mut lines_2, &mut writer_2).await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let validation_match_state = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.2)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("validation-match state should encode");
        writer_2
            .write_all(format!("{validation_match_state}\n").as_bytes())
            .await
            .expect("validation-match state write should succeed");
        writer_2
            .flush()
            .await
            .expect("validation-match state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let remote_behind_1 = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("first remote behind state should encode");
        writer_2
            .write_all(format!("{remote_behind_1}\n").as_bytes())
            .await
            .expect("first remote behind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("first remote behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let remote_behind_2 = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
        ))
        .expect("second remote behind state should encode");
        writer_2
            .write_all(format!("{remote_behind_2}\n").as_bytes())
            .await
            .expect("second remote behind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second remote behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(10)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
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

    let stream_1 = TcpStream::connect(addr)
        .await
        .expect("client should connect for first session");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let first_exit = run_connected_client_session(
        stream_1,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("first connected session should run");
    assert_eq!(first_exit, ConnectedSessionExit::TransportClosed);
    assert!(
        runtime.player().position_seconds() < 1.0,
        "precondition: self-setBy fastforward candidate should be suppressed before reconnect; position={}",
        runtime.player().position_seconds()
    );

    runtime
        .run_disconnect(0.1)
        .expect("disconnect transition should be applied between sessions");
    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry planning should succeed");

    // Re-seed local behind position after reconnect reset so second-session behavior is isolated
    // from first-session player state.
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 0.2);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        runtime.player().position_seconds() > 10.0,
        "reconnect reset should clear stale self-setBy fastforward suppression window so a fresh second-session timer can trigger; position={}",
        runtime.player().position_seconds()
    );
}

#[tokio::test]
async fn connected_client_session_reconnect_clears_stale_fastforward_action_cooldown_window_before_second_session_desync_evaluation()
 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        // First connection: trigger a real fastforward action to leave a cooldown window.
        {
            let (socket_1, _) = listener
                .accept()
                .await
                .expect("first accept should succeed");
            let (reader_1, mut writer_1) = socket_1.into_split();
            let mut lines_1 = BufReader::new(reader_1).lines();
            expect_client_hello_and_send_standard_test_server_hello(&mut lines_1, &mut writer_1)
                .await;

            tokio::time::sleep(Duration::from_millis(100)).await;
            let remote_behind_1 = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("first remote behind state should encode");
            writer_1
                .write_all(format!("{remote_behind_1}\n").as_bytes())
                .await
                .expect("first remote behind state write should succeed");
            writer_1
                .flush()
                .await
                .expect("first remote behind state flush should succeed");

            tokio::time::sleep(Duration::from_millis(320)).await;
            let remote_behind_2 = encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                    .with_playstate(
                        PlaystatePayload::new()
                            .with_position(10.0)
                            .with_paused(false)
                            .with_do_seek(false)
                            .with_set_by("remote-user"),
                    ),
            ))
            .expect("second remote behind state should encode");
            writer_1
                .write_all(format!("{remote_behind_2}\n").as_bytes())
                .await
                .expect("second remote behind state write should succeed");
            writer_1
                .flush()
                .await
                .expect("second remote behind state flush should succeed");

            tokio::time::sleep(Duration::from_millis(10)).await;
            writer_1
                .shutdown()
                .await
                .expect("first writer shutdown should succeed");
        }

        // Second connection: reset should clear the stale cooldown window so fastforward can
        // retrigger after the new session's threshold timing instead of waiting several seconds.
        let (socket_2, _) = listener
            .accept()
            .await
            .expect("second accept should succeed");
        let (reader_2, mut writer_2) = socket_2.into_split();
        let mut lines_2 = BufReader::new(reader_2).lines();
        expect_client_hello_and_send_standard_test_server_hello(&mut lines_2, &mut writer_2).await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let validation_match_state = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.2)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("validation-match state should encode");
        writer_2
            .write_all(format!("{validation_match_state}\n").as_bytes())
            .await
            .expect("validation-match state write should succeed");
        writer_2
            .flush()
            .await
            .expect("validation-match state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let remote_behind_1 = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("first remote behind state should encode");
        writer_2
            .write_all(format!("{remote_behind_1}\n").as_bytes())
            .await
            .expect("first remote behind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("first remote behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let remote_behind_2 = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
        ))
        .expect("second remote behind state should encode");
        writer_2
            .write_all(format!("{remote_behind_2}\n").as_bytes())
            .await
            .expect("second remote behind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second remote behind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(10)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
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

    let stream_1 = TcpStream::connect(addr)
        .await
        .expect("client should connect for first session");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let first_exit = run_connected_client_session(
        stream_1,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("first connected session should run");
    assert_eq!(first_exit, ConnectedSessionExit::TransportClosed);
    assert!(
        runtime.player().position_seconds() > 10.0,
        "precondition: first session should trigger fastforward and leave a cooldown window; position={}",
        runtime.player().position_seconds()
    );

    runtime
        .run_disconnect(0.1)
        .expect("disconnect transition should be applied between sessions");
    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry planning should succeed");

    seed_stub_player_pause_position_telemetry(&mut runtime, false, 0.2);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        runtime.player().position_seconds() > 10.0,
        "reconnect reset should clear stale fastforward cooldown window so fastforward can retrigger in the second session's fresh threshold window; position={}",
        runtime.player().position_seconds()
    );
}
