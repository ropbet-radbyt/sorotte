#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires local standalone mpv binary and media asset"]
#[allow(clippy::await_holding_lock)]
async fn connected_client_session_real_mpv_explicit_ipc_reconnect_validation_smoke_applies_server_playstate_to_real_player()
 {
    use std::process::{Child, Command, Stdio};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use syncplay_protocol::{PlaystatePayload, StatePayload};

    struct MpvChildGuard(Child);

    impl Drop for MpvChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = cli_smoke_repo_root();
    let mpv_bin = std::env::var_os("SYNCPLAY_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .or_else(crate::find_default_managed_mpv_bin)
        .expect("expected mpv binary in ./mpv or SYNCPLAY_MPV_SMOKE_BIN");
    let media_file = std::env::var_os("SYNCPLAY_MPV_SMOKE_MEDIA")
        .map(PathBuf::from)
        .or_else(|| first_media_file(&root.join("media")))
        .expect("expected media file in ./media or SYNCPLAY_MPV_SMOKE_MEDIA");

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
        r"\\.\pipe\syncplay-rust-cli-e2e-reconnect-smoke-{}-{unique}",
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
            .expect("standalone mpv should start for reconnect validation explicit-IPC smoke"),
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
    let reconnect_target_position = 2.0_f64;

    let server_task = tokio::spawn(async move {
        // First connection: normal publish + disconnect to trigger reconnect path.
        let (socket1, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("first accept should not timeout")
            .expect("first accept should succeed");
        let (reader1, mut writer1) = socket1.into_split();
        let mut lines1 = BufReader::new(reader1).lines();

        let hello1 = lines1
            .next_line()
            .await
            .expect("first hello line read should succeed")
            .expect("first hello line should be present");
        assert!(
            hello1.contains("\"Hello\""),
            "first client line on first connection should be Hello"
        );

        let server_hello = encode_message_line(&ProtocolMessage::hello(
            HelloPayload::new("server", "cli-room", "1.7.5")
                .with_features(json!({"chat": true, "readiness": true})),
        ))
        .expect("server hello should encode");
        writer1
            .write_all(format!("{server_hello}\n").as_bytes())
            .await
            .expect("first server hello write should succeed");
        writer1
            .flush()
            .await
            .expect("first server hello flush should succeed");

        let mut saw_first_file_publish = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(500), lines1.next_line()).await;
            let line = match maybe_line {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => panic!("first connection client line read failed: {error}"),
                Err(_) => continue,
            };
            let message = decode_message_line(&line).expect("first connection line should decode");
            if let ProtocolMessage::Set(payload) = message
                && let Some(file) = payload.set.file
                && file.name.as_deref() == Some(expected_name.as_str())
            {
                saw_first_file_publish = true;
                break;
            }
        }
        assert!(
            saw_first_file_publish,
            "expected first connection to publish local file metadata"
        );

        writer1
            .shutdown()
            .await
            .expect("first connection shutdown should succeed");

        // Second connection: send room playstate after reconnect so validation corrects player.
        let (socket2, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("second accept should not timeout")
            .expect("second accept should succeed");
        let (reader2, mut writer2) = socket2.into_split();
        let mut lines2 = BufReader::new(reader2).lines();

        let hello2 = lines2
            .next_line()
            .await
            .expect("second hello line read should succeed")
            .expect("second hello line should be present");
        assert!(
            hello2.contains("\"Hello\""),
            "first client line on second connection should be Hello"
        );

        writer2
            .write_all(format!("{server_hello}\n").as_bytes())
            .await
            .expect("second server hello write should succeed");
        writer2
            .flush()
            .await
            .expect("second server hello flush should succeed");

        let observe_started = Instant::now();
        while observe_started.elapsed() < Duration::from_millis(500) {
            let maybe_line =
                tokio::time::timeout(Duration::from_millis(300), lines2.next_line()).await;
            let line = match maybe_line {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => panic!("second connection client line read failed: {error}"),
                Err(_) => continue,
            };
            let _ = decode_message_line(&line).expect("second connection line should decode");
        }

        // Let the runtime hit at least one tick to sync fresh player telemetry into session
        // before we send the reconnect room playstate. Reconnect file restore/republish
        // timing is covered elsewhere and can race with this smoke's server pacing.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        let state_line = encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(reconnect_target_position)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("remote-user"),
            ),
        ))
        .expect("server state playstate should encode");
        writer2
            .write_all(format!("{state_line}\n").as_bytes())
            .await
            .expect("second server state write should succeed");
        writer2
            .flush()
            .await
            .expect("second server state flush should succeed");

        tokio::time::sleep(Duration::from_secs(1)).await;
        writer2
            .shutdown()
            .await
            .expect("second connection shutdown should succeed");
    });

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SYNCPLAY_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SYNCPLAY_MPV_IPC_PATH";
    let key_managed = "SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    let old_managed = std::env::var_os(key_managed);
    env.set_var(key_client_ipc, &pipe_path);
    env.remove_var(key_fallback_ipc);
    env.remove_var(key_managed);

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 8.0;
    config.readiness_supported_override = Some(false);
    let (mut runtime, _managed_guard) =
        create_client_runtime_with_managed_mpv_support(&config, None, None)
            .expect("runtime creation with explicit mpv IPC should succeed");

    let stream1 = TcpStream::connect(addr)
        .await
        .expect("client should connect for first session");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let exit1 = run_connected_client_session(
        stream1,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("first connected session should run");
    assert_eq!(
        exit1,
        ConnectedSessionExit::TransportClosed,
        "first session should close to trigger reconnect path"
    );

    runtime
        .run_disconnect(0.5)
        .expect("disconnect handling should succeed");
    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry planning should succeed");

    // Seed a local mismatch after reconnect reset so validation has something to correct.
    crate::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
        runtime.player_mut().set_position(0.0)
    })
    .expect("real mpv position seed should succeed");
    crate::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
        runtime.player_mut().set_paused(false)
    })
    .expect("real mpv pause seed should succeed");

    let stream2 = TcpStream::connect(addr)
        .await
        .expect("client should connect for second session");
    let exit2 = run_connected_client_session(
        stream2,
        &mut runtime,
        &config,
        None,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("second connected session should run");
    assert!(
        matches!(
            exit2,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "second session should observe peer close or runtime timeout"
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
        runtime.player().paused(),
        "expected reconnect validation to pause real mpv from server room playstate; position={final_position}; speed={}",
        runtime.player().playback_rate()
    );
    assert!(
        (final_position - reconnect_target_position).abs() <= 0.9,
        "expected reconnect validation to seek real mpv near target from server room playstate; target={reconnect_target_position}; position={final_position}"
    );
}
