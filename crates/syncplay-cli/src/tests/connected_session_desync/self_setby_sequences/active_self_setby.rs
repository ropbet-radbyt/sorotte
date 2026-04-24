use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_fastforward_do_seek_then_self_setby_sequence_preserves_self_setby_suppression_window_for_next_remote_state()
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
        let do_seek_self_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(true)
                    .with_set_by("cli-user"),
            ),
        ))
        .expect("doSeek self state should encode");
        writer
            .write_all(format!("{do_seek_self_line}\n").as_bytes())
            .await
            .expect("doSeek self state write should succeed");
        writer
            .flush()
            .await
            .expect("doSeek self state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let self_clear_line = encode_message_line(&ProtocolMessage::state(
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
        .expect("self doSeek-clear state should encode");
        writer
            .write_all(format!("{self_clear_line}\n").as_bytes())
            .await
            .expect("self doSeek-clear state write should succeed");
        writer
            .flush()
            .await
            .expect("self doSeek-clear state flush should succeed");

        tokio::time::sleep(Duration::from_millis(320)).await;
        let remote_line = encode_message_line(&ProtocolMessage::state(
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
        .expect("remote state should encode");
        writer
            .write_all(format!("{remote_line}\n").as_bytes())
            .await
            .expect("remote state write should succeed");
        writer
            .flush()
            .await
            .expect("remote state flush should succeed");

        // Close almost immediately after the first remote non-doSeek state. If the connected
        // loop preserves the post-doSeek self-setBy suppression-window timing semantics from
        // client-core, this remote state can fast-forward immediately without a fresh sustain
        // window because the self-setBy branch leaves a future suppression-window timer.
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
        runtime.player().position_seconds() > 10.0,
        "after doSeek clears, self-setBy fastforward suppression-window timing should carry into the next remote state; position={}",
        runtime.player().position_seconds()
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_slowdown_do_seek_and_self_setby_sequence_stays_suppressed_until_remote_case()
 {
    #[derive(Clone, Copy)]
    enum CaseKind {
        DoSeekThenSelf,
        RemoteControl,
    }

    async fn run_case(case: CaseKind) -> f64 {
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
            match case {
                CaseKind::DoSeekThenSelf => {
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
                    writer
                        .write_all(format!("{do_seek_self_line}\n").as_bytes())
                        .await
                        .expect("doSeek self state write should succeed");
                    writer
                        .flush()
                        .await
                        .expect("doSeek self state flush should succeed");

                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let self_clear_line = encode_message_line(&ProtocolMessage::state(
                        StatePayload::new()
                            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                            .with_playstate(
                                PlaystatePayload::new()
                                    .with_position(0.0)
                                    .with_paused(false)
                                    .with_do_seek(false)
                                    .with_set_by("cli-user"),
                            ),
                    ))
                    .expect("self doSeek-clear state should encode");
                    writer
                        .write_all(format!("{self_clear_line}\n").as_bytes())
                        .await
                        .expect("self doSeek-clear state write should succeed");
                    writer
                        .flush()
                        .await
                        .expect("self doSeek-clear state flush should succeed");

                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                CaseKind::RemoteControl => {
                    let remote_line = encode_message_line(&ProtocolMessage::state(
                        StatePayload::new().with_playstate(
                            PlaystatePayload::new()
                                .with_position(0.0)
                                .with_paused(false)
                                .with_do_seek(false)
                                .with_set_by("remote-user"),
                        ),
                    ))
                    .expect("remote state should encode");
                    writer
                        .write_all(format!("{remote_line}\n").as_bytes())
                        .await
                        .expect("remote state write should succeed");
                    writer
                        .flush()
                        .await
                        .expect("remote state flush should succeed");

                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }

            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 1.5;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(true);
        config.rewind_on_desync_override = Some(false);
        config.fastforward_on_desync_override = Some(false);
        config.slow_on_desync_override = Some(true);
        config.slowdown_threshold_seconds_override = Some(1.0);

        let mut runtime = create_client_runtime(&config);
        seed_stub_player_pause_position_telemetry(&mut runtime, false, 2.0);
        seed_stub_player_playback_rate(&mut runtime, 1.0);

        run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
        server_task.await.expect("server task join should succeed");
        runtime.player().playback_rate()
    }

    let suppressed_rate = run_case(CaseKind::DoSeekThenSelf).await;
    let remote_rate = run_case(CaseKind::RemoteControl).await;

    assert!(
        (suppressed_rate - 1.0).abs() < 1e-6,
        "doSeek+self-setBy slowdown branch sequence should remain suppressed; rate={suppressed_rate}"
    );
    assert!(
        (remote_rate - 0.95).abs() < 1e-6,
        "remote slowdown case should apply slowdown playback rate; rate={remote_rate}"
    );
}
