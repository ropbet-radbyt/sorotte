use super::*;

#[tokio::test]
async fn connected_client_session_inbound_state_paused_rewind_desync_applies_remote_pause_and_seek_but_skips_self_setby()
 {
    async fn run_case(set_by: &str) -> (bool, f64) {
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
                        .with_paused(true)
                        .with_do_seek(false)
                        .with_set_by(set_by.as_str()),
                ),
            ))
            .expect("inbound paused rewind state should encode");
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
        seed_stub_player_pause_position_telemetry(&mut runtime, false, 10.0);

        run_connected_client_session_expect_normal_exit(addr, &mut runtime, &config).await;
        server_task.await.expect("server task join should succeed");
        (
            runtime.player().paused(),
            runtime.player().position_seconds(),
        )
    }

    let (remote_paused, remote_position) = run_case("remote-user").await;
    let (self_paused, self_position) = run_case("cli-user").await;

    assert!(
        remote_paused,
        "remote paused room state should pause local player before/alongside desync correction"
    );
    assert!(
        remote_position < 1.0,
        "remote paused room state should still apply rewind desync correction seek; position={remote_position}"
    );
    assert!(
        !self_paused,
        "self-attributed paused room state should not trigger local pause sync"
    );
    assert!(
        self_position > 9.0,
        "self-attributed paused room state should suppress rewind desync correction; position={self_position}"
    );
}

#[tokio::test]
async fn connected_client_session_inbound_state_paused_fastforward_desync_applies_remote_pause_and_seek_but_skips_self_setby()
 {
    async fn run_case(set_by: &str) -> (bool, f64) {
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
                        .with_position(10.0)
                        .with_paused(true)
                        .with_do_seek(false)
                        .with_set_by(set_by.as_str()),
                ),
            ))
            .expect("inbound paused fastforward state should encode");
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
        config.max_connected_runtime_seconds = 1.6;
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
        (
            runtime.player().paused(),
            runtime.player().position_seconds(),
        )
    }

    let (remote_paused, remote_position) = run_case("remote-user").await;
    let (self_paused, self_position) = run_case("cli-user").await;

    assert!(
        remote_paused,
        "remote paused room state should pause local player before/alongside desync correction"
    );
    assert!(
        remote_position >= 10.0,
        "remote paused room state should seek to the room position before pausing; position={remote_position}"
    );
    assert!(
        !self_paused,
        "self-attributed paused room state should not trigger local pause sync"
    );
    assert!(
        self_position < 1.0,
        "self-attributed paused room state should suppress fastforward desync correction; position={self_position}"
    );
}
