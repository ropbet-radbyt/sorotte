#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires local standalone mpv binary and media asset"]
#[allow(clippy::await_holding_lock)]
async fn connected_client_session_real_mpv_explicit_ipc_smoke_applies_inbound_server_playstate_fastforward_via_ping_forward_delay_to_real_player()
 {
    use sorotte_protocol::{PingPayload, PlaystatePayload, StatePayload};
    use std::process::{Child, Command, Stdio};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    struct MpvChildGuard(Child);

    impl Drop for MpvChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = cli_smoke_repo_root();
    let mpv_bin = std::env::var_os("SOROTTE_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .or_else(crate::find_default_managed_mpv_bin)
        .expect("expected mpv binary in ./mpv or SOROTTE_MPV_SMOKE_BIN");
    let media_file = std::env::var_os("SOROTTE_MPV_SMOKE_MEDIA")
        .map(PathBuf::from)
        .or_else(|| first_media_file(&root.join("media")))
        .expect("expected media file in ./media or SOROTTE_MPV_SMOKE_MEDIA");

    if !mpv_bin.exists() {
        panic!("mpv binary not found at {}", mpv_bin.display());
    }
    if !media_file.exists() {
        panic!("media file not found at {}", media_file.display());
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis();
    let pipe_path = format!(
        r"\\.\pipe\sorotte-cli-state-fastforward-smoke-{}-{unique}",
        std::process::id()
    );

    let _mpv_child = MpvChildGuard(
        Command::new(&mpv_bin)
            .current_dir(
                mpv_bin
                    .parent()
                    .expect("mpv binary path should have a parent directory"),
            )
            .arg("--pause")
            .arg("--force-window=no")
            .arg("--idle=yes")
            .arg(format!("--input-ipc-server={pipe_path}"))
            .arg(&media_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("standalone mpv should start for explicit-IPC fastforward smoke"),
    );
    let _warmup_attach = crate::connect_mpv_adapter_with_retry(
        &pipe_path,
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
    .expect("pre-runtime explicit mpv JSON IPC attach should succeed");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let expected_name = media_file
        .file_name()
        .and_then(|name| name.to_str())
        .expect("media file should have utf-8 filename")
        .to_owned();
    let target_position = 6.0_f64;

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

        let server_hello = encode_message_line(&ProtocolMessage::hello(
            HelloPayload::new("server", "cli-room", "1.7.5").with_features(json!({
                "chat": true,
                "readiness": false
            })),
        ))
        .expect("server hello should encode");
        writer
            .write_all(format!("{server_hello}\n").as_bytes())
            .await
            .expect("server hello write should succeed");
        writer
            .flush()
            .await
            .expect("server hello flush should succeed");

        let mut saw_file_publish = false;
        let publish_started = Instant::now();
        while publish_started.elapsed() < Duration::from_secs(8) {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(500), lines.next_line()).await;
            let line = match maybe_line {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => panic!("client line read should succeed: {error}"),
                Err(_) => continue,
            };
            let message = decode_message_line(&line).expect("client line should decode");
            if let ProtocolMessage::Set(payload) = message
                && let Some(file) = payload.set.file
                && file.name.as_deref() == Some(expected_name.as_str())
            {
                saw_file_publish = true;
                break;
            }
        }
        assert!(
            saw_file_publish,
            "expected client to publish local file metadata before inbound state"
        );

        // Let telemetry and the first autoplay tick settle before sending the borderline state.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        let inbound_latency_calculation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_secs_f64()
            - 0.35;
        let state_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new()
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(target_position)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("remote-user"),
                )
                .with_ping(
                    PingPayload::new()
                        .with_latency_calculation(inbound_latency_calculation)
                        .with_server_rtt(0.05),
                ),
        ))
        .expect("server state should encode");
        writer
            .write_all(format!("{state_line}\n").as_bytes())
            .await
            .expect("server state write should succeed");
        writer
            .flush()
            .await
            .expect("server state flush should succeed");

        // Keep the connection alive long enough for the sustained fast-forward window to elapse.
        tokio::time::sleep(Duration::from_millis(5600)).await;
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let key_managed = "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    let old_managed = std::env::var_os(key_managed);
    env.set_var(key_client_ipc, &pipe_path);
    env.remove_var(key_fallback_ipc);
    env.remove_var(key_managed);

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 10.0;
    config.readiness_supported_override = Some(false);
    config.local_can_control_override = Some(false);
    config.rewind_on_desync_override = Some(false);
    config.fastforward_on_desync_override = Some(true);
    config.slow_on_desync_override = Some(false);
    config.fastforward_threshold_seconds_override = Some(6.0);

    let (mut runtime, _managed_guard) =
        create_client_runtime_with_managed_mpv_support(&config, None, None)
            .expect("runtime creation with explicit mpv IPC should succeed");

    // Seed a borderline-behind local position and keep playback almost stationary so the
    // forward-delay compensation (from inbound ping/serverRtt) is what crosses the threshold.
    crate::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
        runtime.player_mut().set_paused(true)
    })
    .expect("real mpv pause seed should succeed");
    crate::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
        runtime.player_mut().set_playback_rate(0.01)
    })
    .expect("real mpv slow playback-rate seed should succeed");
    crate::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
        runtime.player_mut().set_position(0.2)
    })
    .expect("real mpv position seed should succeed");

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
        "connected session should observe peer close or runtime timeout"
    );
    server_task.await.expect("server task join should succeed");

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }
    match old_managed {
        Some(value) => env.set_var(key_managed, value),
        None => env.remove_var(key_managed),
    }

    let final_position = runtime.player().position_seconds();
    assert!(
        final_position >= target_position - 0.5,
        "expected ping forward-delay compensated fastforward to seek real mpv near/above target; target={target_position}; position={final_position}; speed={}",
        runtime.player().playback_rate()
    );
    assert!(
        final_position >= target_position + 0.1,
        "expected ping forward-delay compensated fastforward to overshoot target (fastforward extra); target={target_position}; position={final_position}"
    );
}
