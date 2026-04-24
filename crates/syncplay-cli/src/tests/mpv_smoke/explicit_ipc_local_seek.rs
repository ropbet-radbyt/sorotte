use super::*;

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires local standalone mpv binary and media asset"]
#[allow(clippy::await_holding_lock)]
async fn connected_client_session_real_mpv_explicit_ipc_smoke_publishes_local_file_and_applies_local_seek_command()
 {
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
        r"\\.\pipe\syncplay-rust-cli-e2e-smoke-{}-{unique}",
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
            .expect("standalone mpv should start for e2e explicit-IPC smoke"),
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
    let (file_publish_tx, file_publish_rx) = tokio::sync::oneshot::channel::<()>();

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
            HelloPayload::new("server", "cli-room", "1.7.5")
                .with_features(json!({"chat": true, "readiness": true})),
        ))
        .expect("server hello should encode");
        let server_hello = format!("{server_hello}\n");
        writer
            .write_all(server_hello.as_bytes())
            .await
            .expect("server hello write should succeed");
        writer
            .flush()
            .await
            .expect("server hello flush should succeed");

        let mut saw_file_publish = false;
        let publish_wait = Duration::from_secs(8);
        let publish_started = Instant::now();
        while publish_started.elapsed() < publish_wait {
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
            }
            if saw_file_publish {
                break;
            }
        }
        assert!(
            saw_file_publish,
            "expected client to publish local file metadata"
        );
        let _ = file_publish_tx.send(());

        tokio::time::sleep(Duration::from_secs(1)).await;
        writer
            .shutdown()
            .await
            .expect("server shutdown should succeed");
    });

    let _env_lock = LEGACY_EXTERNAL_PLAYER_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key_client_ipc = "SYNCPLAY_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SYNCPLAY_MPV_IPC_PATH";
    let key_managed = "SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    let old_managed = std::env::var_os(key_managed);
    unsafe {
        std::env::set_var(key_client_ipc, &pipe_path);
        std::env::remove_var(key_fallback_ipc);
        std::env::remove_var(key_managed);
    }

    let mut config = test_client_loop_config_with_addr(addr);
    config.max_connected_runtime_seconds = 6.0;
    let (mut runtime, _managed_guard) =
        create_client_runtime_with_managed_mpv_support(&config, None, None)
            .expect("runtime creation with explicit mpv IPC should succeed");
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let (sender, mut receiver) = unbounded_channel::<String>();
    tokio::spawn(async move {
        let _ = file_publish_rx.await;
        sender
            .send("seek 2".to_owned())
            .expect("seek command should queue");
    });
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session(
        stream,
        &mut runtime,
        &config,
        None,
        Some(&mut receiver),
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
        Some(value) => unsafe { std::env::set_var(key_client_ipc, value) },
        None => unsafe { std::env::remove_var(key_client_ipc) },
    }
    match old_fallback_ipc {
        Some(value) => unsafe { std::env::set_var(key_fallback_ipc, value) },
        None => unsafe { std::env::remove_var(key_fallback_ipc) },
    }
    match old_managed {
        Some(value) => unsafe { std::env::set_var(key_managed, value) },
        None => unsafe { std::env::remove_var(key_managed) },
    }
    assert!(
        runtime.player().position_seconds() >= 1.5,
        "expected local seek command to move real mpv via runtime loop; position={}",
        runtime.player().position_seconds()
    );
    assert!(
        runtime.player().paused(),
        "expected real mpv to remain paused after local seek command; position={}; speed={}",
        runtime.player().position_seconds(),
        runtime.player().playback_rate()
    );
}
