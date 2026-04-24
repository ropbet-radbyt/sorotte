use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_paused_self_setby_slowdown_suppresses_restore_until_unpaused_after_do_seek()
 {
    #[derive(Clone, Copy)]
    enum CaseKind {
        StopWhilePaused,
        UnpauseAfterPaused,
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
            let prime_slowdown_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(0.0)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                ),
            ))
            .expect("slowdown-prime state should encode");
            writer
                .write_all(format!("{prime_slowdown_line}\n").as_bytes())
                .await
                .expect("slowdown-prime state write should succeed");
            writer
                .flush()
                .await
                .expect("slowdown-prime state flush should succeed");

            tokio::time::sleep(Duration::from_millis(180)).await;
            let do_seek_self_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                    .with_playstate(
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

            tokio::time::sleep(Duration::from_millis(180)).await;
            let paused_self_near_sync_line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new()
                    .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                    .with_playstate(
                        PlaystatePayload::new()
                            .with_position(1.95)
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

            if matches!(case, CaseKind::UnpauseAfterPaused) {
                tokio::time::sleep(Duration::from_millis(180)).await;
                let unpaused_self_near_sync_line = encode_message_line(&ProtocolMessage::state(
                    StatePayload::new()
                        .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
                        .with_playstate(
                            PlaystatePayload::new()
                                .with_position(1.95)
                                .with_paused(false)
                                .with_do_seek(false)
                                .with_set_by("cli-user"),
                        ),
                ))
                .expect("unpaused self near-sync state should encode");
                writer
                    .write_all(format!("{unpaused_self_near_sync_line}\n").as_bytes())
                    .await
                    .expect("unpaused self near-sync state write should succeed");
                writer
                    .flush()
                    .await
                    .expect("unpaused self near-sync state flush should succeed");
            }

            tokio::time::sleep(Duration::from_millis(220)).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let mut config = test_client_loop_config_with_addr(addr);
        config.max_connected_runtime_seconds = 2.2;
        config.readiness_supported_override = Some(false);
        config.local_can_control_override = Some(true);
        config.rewind_on_desync_override = Some(false);
        config.fastforward_on_desync_override = Some(false);
        config.slow_on_desync_override = Some(true);
        config.slowdown_threshold_seconds_override = Some(1.0);

        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .set_paused(false)
            .expect("stub player pause seed should succeed");
        runtime
            .player_mut()
            .set_playback_rate(1.0)
            .expect("stub player playback-rate seed should succeed");
        runtime
            .player_mut()
            .set_position(2.0)
            .expect("stub player position seed should succeed");
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(2.0),
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
        runtime.player().playback_rate()
    }

    let paused_rate = run_case(CaseKind::StopWhilePaused).await;
    let unpaused_rate = run_case(CaseKind::UnpauseAfterPaused).await;

    assert!(
        (paused_rate - 0.95).abs() < 1e-6,
        "paused room playstate should suppress slowdown restore before self-setBy slowdown branch; rate={paused_rate}"
    );
    assert!(
        (unpaused_rate - 1.0).abs() < 1e-6,
        "once unpaused (same self-setBy near-sync state), slowdown restore should apply; rate={unpaused_rate}"
    );
}
