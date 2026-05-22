use super::*;

#[tokio::test]
async fn connected_client_session_reconnect_prevents_stale_speed_restore_across_second_session_do_seek_paused_and_self_setby_rewind_suppression_branches()
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

        // Second connection: clear validation, then walk doSeek -> paused -> self-setBy
        // rewind suppression -> near-sync sequence and verify no stale restore-speed action.
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

        tokio::time::sleep(Duration::from_millis(180)).await;
        let paused_do_seek_self_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("paused doSeek self state should encode");
        writer_2
            .write_all(format!("{paused_do_seek_self_line}\n").as_bytes())
            .await
            .expect("paused doSeek self state write should succeed");
        writer_2
            .flush()
            .await
            .expect("paused doSeek self state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let paused_self_rewind_candidate_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("paused self rewind-candidate state should encode");
        writer_2
            .write_all(format!("{paused_self_rewind_candidate_line}\n").as_bytes())
            .await
            .expect("paused self rewind-candidate state write should succeed");
        writer_2
            .flush()
            .await
            .expect("paused self rewind-candidate state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let unpaused_self_near_sync_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(9.95)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("unpaused self near-sync state should encode");
        writer_2
            .write_all(format!("{unpaused_self_near_sync_line}\n").as_bytes())
            .await
            .expect("unpaused self near-sync state write should succeed");
        writer_2
            .flush()
            .await
            .expect("unpaused self near-sync state flush should succeed");

        tokio::time::sleep(Duration::from_millis(80)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.8;
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

    // Re-seed telemetry after reconnect reset while keeping visible playback-rate=0.95 so
    // stale restore-speed behavior would be observable.
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        runtime.player().position_seconds() > 9.0,
        "post-reconnect doSeek+paused+self-setBy rewind suppression sequence should keep rewind suppressed before near-sync; position={}",
        runtime.player().position_seconds()
    );
    assert!(
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "near-sync after doSeek+paused+self-setBy rewind-suppression sequence should not emit stale restore-speed action; rate={}",
        runtime.player().playback_rate()
    );
}
