use super::*;

#[tokio::test]
async fn connected_client_session_reconnect_prevents_stale_speed_restore_across_second_session_do_seek_paused_and_self_setby_slowdown_suppression_branches()
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

        // Second connection: clear validation, then walk doSeek -> paused -> self-setBy ->
        // near-sync sequence and verify it does not restore speed from stale state.
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
                    .with_position(2.0)
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
        let do_seek_self_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(true)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("doSeek self state should encode");
        writer_2
            .write_all(format!("{do_seek_self_line}\n").as_bytes())
            .await
            .expect("doSeek self state write should succeed");
        writer_2
            .flush()
            .await
            .expect("doSeek self state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let paused_self_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("paused self state should encode");
        writer_2
            .write_all(format!("{paused_self_line}\n").as_bytes())
            .await
            .expect("paused self state write should succeed");
        writer_2
            .flush()
            .await
            .expect("paused self state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let unpaused_self_slowdown_candidate_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("unpaused self slowdown-candidate state should encode");
        writer_2
            .write_all(format!("{unpaused_self_slowdown_candidate_line}\n").as_bytes())
            .await
            .expect("unpaused self slowdown-candidate state write should succeed");
        writer_2
            .flush()
            .await
            .expect("unpaused self slowdown-candidate state flush should succeed");

        tokio::time::sleep(Duration::from_millis(180)).await;
        let self_near_sync_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(1.95)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("self near-sync state should encode");
        writer_2
            .write_all(format!("{self_near_sync_line}\n").as_bytes())
            .await
            .expect("self near-sync state write should succeed");
        writer_2
            .flush()
            .await
            .expect("self near-sync state flush should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
        writer_2
            .shutdown()
            .await
            .expect("second writer shutdown should succeed");
    });

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 2.8;
    config.readiness_supported_override = Some(false);
    config.local_can_control_override = Some(true);
    config.rewind_on_desync_override = Some(false);
    config.fastforward_on_desync_override = Some(false);
    config.slow_on_desync_override = Some(true);
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
    seed_stub_player_pause_position_telemetry(&mut runtime, false, 2.0);

    run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
    server_task.await.expect("server task join should succeed");

    assert!(
        (runtime.player().playback_rate() - 0.95).abs() < 1e-6,
        "reconnect reset + doSeek/paused/self-setBy slowdown-suppression sequence should not emit stale restore-speed action; rate={}",
        runtime.player().playback_rate()
    );
}

#[tokio::test]
async fn connected_client_session_reconnect_applies_remote_seek_after_do_seek_transition() {
    async fn run_case(send_do_seek_clear: bool) -> (f64, f64) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            // First connection: self-setBy rewind candidate should be suppressed.
            {
                let (socket_1, _) = listener
                    .accept()
                    .await
                    .expect("first accept should succeed");
                let (reader_1, mut writer_1) = socket_1.into_split();
                let mut lines_1 = BufReader::new(reader_1).lines();
                expect_client_hello_and_send_standard_test_server_hello(
                    &mut lines_1,
                    &mut writer_1,
                )
                .await;

                tokio::time::sleep(Duration::from_millis(100)).await;
                let self_rewind_candidate_line = encode_message_line(&ProtocolMessage::state(
                    StatePayload::new().with_playstate(
                        PlaystatePayload::new()
                            .with_position(0.0)
                            .with_paused(false)
                            .with_do_seek(false)
                            .with_set_by("cli-user"),
                    ),
                ))
                .expect("first-session self rewind-candidate state should encode");
                writer_1
                    .write_all(format!("{self_rewind_candidate_line}\n").as_bytes())
                    .await
                    .expect("first-session self rewind-candidate state write should succeed");
                writer_1
                    .flush()
                    .await
                    .expect("first-session self rewind-candidate state flush should succeed");

                tokio::time::sleep(Duration::from_millis(120)).await;
                writer_1
                    .shutdown()
                    .await
                    .expect("first writer shutdown should succeed");
            }

            // Second connection: reconnect reset should preserve rewind suppression ordering
            // across a doSeek transition in the new session.
            let (socket_2, _) = listener
                .accept()
                .await
                .expect("second accept should succeed");
            let (reader_2, mut writer_2) = socket_2.into_split();
            let mut lines_2 = BufReader::new(reader_2).lines();
            expect_client_hello_and_send_standard_test_server_hello(&mut lines_2, &mut writer_2)
                .await;

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
            let do_seek_remote_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(true)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("second-session doSeek remote state should encode");
            writer_2
                .write_all(format!("{do_seek_remote_line}\n").as_bytes())
                .await
                .expect("second-session doSeek remote state write should succeed");
            writer_2
                .flush()
                .await
                .expect("second-session doSeek remote state flush should succeed");

            if send_do_seek_clear {
                tokio::time::sleep(Duration::from_millis(160)).await;
                let do_seek_clear_remote_line = encode_message_line(&ProtocolMessage::state(
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
                .expect("second-session doSeek-clear remote state should encode");
                writer_2
                    .write_all(format!("{do_seek_clear_remote_line}\n").as_bytes())
                    .await
                    .expect("second-session doSeek-clear remote state write should succeed");
                writer_2
                    .flush()
                    .await
                    .expect("second-session doSeek-clear remote state flush should succeed");
            }

            tokio::time::sleep(Duration::from_millis(220)).await;
            writer_2
                .shutdown()
                .await
                .expect("second writer shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 2.3;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(false);
        config.rewind_on_desync_override = Some(true);
        config.fastforward_on_desync_override = Some(false);
        config.slow_on_desync_override = Some(false);
        config.rewind_threshold_seconds_override = Some(1.0);

        let mut runtime = create_client_runtime(&config);
        seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

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
        let first_session_position = runtime.player().position_seconds();
        assert!(
            first_session_position > 9.0,
            "precondition: first-session self-setBy rewind candidate should be suppressed; position={first_session_position}"
        );

        runtime
            .run_disconnect(0.1)
            .expect("disconnect transition should be applied between sessions");
        runtime
            .run_reconnect_retry(0)
            .expect("reconnect retry planning should succeed");

        seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

        run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
        server_task.await.expect("server task join should succeed");

        (first_session_position, runtime.player().position_seconds())
    }

    let (_precondition_position, position_without_do_seek_clear) = run_case(false).await;
    let (_precondition_position, position_with_do_seek_clear) = run_case(true).await;

    assert!(
        position_without_do_seek_clear < 1.0,
        "post-reconnect remote doSeek should seek immediately without waiting for a later doSeek-clear state; position={position_without_do_seek_clear}"
    );
    assert!(
        position_with_do_seek_clear < 1.0,
        "post-reconnect remote doSeek should remain synced after a later doSeek-clear state; position={position_with_do_seek_clear}"
    );
}
