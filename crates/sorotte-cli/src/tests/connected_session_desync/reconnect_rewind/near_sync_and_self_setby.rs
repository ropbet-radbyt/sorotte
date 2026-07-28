use super::*;

#[tokio::test]
async fn connected_client_session_reconnect_prevents_stale_speed_restore_when_second_session_rewind_precedes_near_sync()
 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        // First connection: trigger slowdown to prime stale speed_changed before reconnect.
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
            let remote_slowdown_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("first-session remote slowdown state should encode");
            writer_1
                .write_all(format!("{remote_slowdown_line}\n").as_bytes())
                .await
                .expect("first-session remote slowdown state write should succeed");
            writer_1
                .flush()
                .await
                .expect("first-session remote slowdown state flush should succeed");

            tokio::time::sleep(Duration::from_millis(180)).await;
            writer_1
                .shutdown()
                .await
                .expect("first writer shutdown should succeed");
        }

        // Second connection: clear validation, then force a rewind and follow with a
        // near-sync state. Near-sync must not emit a stale restore-speed action.
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
                    .with_position(10.0)
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

        tokio::time::sleep(Duration::from_millis(160)).await;
        let remote_rewind_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("second-session remote rewind state should encode");
        writer_2
            .write_all(format!("{remote_rewind_line}\n").as_bytes())
            .await
            .expect("second-session remote rewind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second-session remote rewind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(160)).await;
        let near_sync_after_rewind_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.05)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
        ))
        .expect("second-session near-sync-after-rewind state should encode");
        writer_2
            .write_all(format!("{near_sync_after_rewind_line}\n").as_bytes())
            .await
            .expect("second-session near-sync-after-rewind state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second-session near-sync-after-rewind state flush should succeed");

        tokio::time::sleep(Duration::from_millis(80)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.4;
    config.readiness_supported_override = Some(false);
    config.local_can_control_override = Some(true);
    config.rewind_on_desync_override = Some(true);
    config.fastforward_on_desync_override = Some(false);
    config.slow_on_desync_override = Some(true);
    config.rewind_threshold_seconds_override = Some(6.0);
    config.slowdown_threshold_seconds_override = Some(1.0);

    let mut runtime = create_client_runtime(&config);
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 2.0);
    seed_stub_player_playback_rate(&mut runtime, 1.0);

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
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "precondition: first session should trigger slowdown and set playback rate=0.95; rate={}",
        runtime.player().playback_rate()
    );

    runtime
        .run_disconnect(0.1)
        .expect("disconnect transition should be applied between sessions");
    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry planning should succeed");

    assert!(
        (runtime.player().playback_rate() - 1.0).abs() < 1e-6,
        "reconnect must neutralize the first session's Sorotte-owned slowdown"
    );
    // Reintroduce 0.95 as an unowned sentinel after reconnect. The second
    // session must not mistake the old ownership bit for authority to restore it.
    seed_stub_player_playback_rate(&mut runtime, 0.95);
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        runtime.player().position_seconds() < 1.0,
        "second-session remote rewind should still apply after reconnect reset; position={}",
        runtime.player().position_seconds()
    );
    assert!(
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "near-sync after post-reconnect rewind should not emit stale restore-speed action; rate={}",
        runtime.player().playback_rate()
    );
}

#[tokio::test]
async fn connected_client_session_reconnect_prevents_stale_speed_restore_when_second_session_self_setby_rewind_is_suppressed_before_near_sync()
 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let server_task = tokio::spawn(async move {
        // First connection: trigger slowdown to prime stale speed_changed before reconnect.
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
            let remote_slowdown_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("first-session remote slowdown state should encode");
            writer_1
                .write_all(format!("{remote_slowdown_line}\n").as_bytes())
                .await
                .expect("first-session remote slowdown state write should succeed");
            writer_1
                .flush()
                .await
                .expect("first-session remote slowdown state flush should succeed");

            tokio::time::sleep(Duration::from_millis(180)).await;
            writer_1
                .shutdown()
                .await
                .expect("first writer shutdown should succeed");
        }

        // Second connection: clear validation, then send a self-setBy rewind candidate and a
        // self-setBy near-sync state. Both should avoid stale restore-speed behavior.
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
                    .with_position(10.0)
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

        tokio::time::sleep(Duration::from_millis(160)).await;
        let self_rewind_candidate_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("second-session self rewind-candidate state should encode");
        writer_2
            .write_all(format!("{self_rewind_candidate_line}\n").as_bytes())
            .await
            .expect("second-session self rewind-candidate state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second-session self rewind-candidate state flush should succeed");

        tokio::time::sleep(Duration::from_millis(160)).await;
        let self_near_sync_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(9.95)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("second-session self near-sync state should encode");
        writer_2
            .write_all(format!("{self_near_sync_line}\n").as_bytes())
            .await
            .expect("second-session self near-sync state write should succeed");
        writer_2
            .flush()
            .await
            .expect("second-session self near-sync state flush should succeed");

        tokio::time::sleep(Duration::from_millis(80)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.4;
    config.readiness_supported_override = Some(false);
    config.local_can_control_override = Some(true);
    config.rewind_on_desync_override = Some(true);
    config.fastforward_on_desync_override = Some(false);
    config.slow_on_desync_override = Some(true);
    config.rewind_threshold_seconds_override = Some(6.0);
    config.slowdown_threshold_seconds_override = Some(1.0);

    let mut runtime = create_client_runtime(&config);
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 2.0);
    seed_stub_player_playback_rate(&mut runtime, 1.0);

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
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "precondition: first session should trigger slowdown and set playback rate=0.95; rate={}",
        runtime.player().playback_rate()
    );

    runtime
        .run_disconnect(0.1)
        .expect("disconnect transition should be applied between sessions");
    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry planning should succeed");

    assert!(
        (runtime.player().playback_rate() - 1.0).abs() < 1e-6,
        "reconnect must neutralize the first session's Sorotte-owned slowdown"
    );
    // Reintroduce 0.95 as an unowned sentinel after reconnect. The second
    // session must not emit a stale restore-speed action.
    seed_stub_player_playback_rate(&mut runtime, 0.95);
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        runtime.player().position_seconds() > 9.0,
        "post-reconnect self-setBy rewind candidate should remain suppressed; position={}",
        runtime.player().position_seconds()
    );
    assert!(
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "near-sync after post-reconnect self-setBy rewind suppression should not emit stale restore-speed action; rate={}",
        runtime.player().playback_rate()
    );
}
