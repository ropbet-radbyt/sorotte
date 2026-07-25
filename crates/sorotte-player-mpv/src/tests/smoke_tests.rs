use super::*;
use crate::MpvNetworkOptionsHookHealth;

#[test]
#[ignore = "opt-in real-mpv bridge test; set SOROTTE_TEST_MPV_BIN"]
fn real_mpv_bridge_lifecycle_over_json_ipc() {
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{JoinHandle, sleep},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    struct RealMpvGuard {
        child: Child,
        cleanup_path: Option<PathBuf>,
    }

    impl Drop for RealMpvGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(path) = self.cleanup_path.as_ref() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    struct LoopbackMediaServerGuard {
        address: SocketAddr,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for LoopbackMediaServerGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.address);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn silent_wav_bytes() -> Vec<u8> {
        const SAMPLE_RATE: u32 = 8_000;
        const CHANNELS: u16 = 1;
        const BITS_PER_SAMPLE: u16 = 16;
        const DURATION_SECONDS: u32 = 2;
        let data_len =
            SAMPLE_RATE * DURATION_SECONDS * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&CHANNELS.to_le_bytes());
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(
            &(SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8)).to_le_bytes(),
        );
        wav.extend_from_slice(&(CHANNELS * (BITS_PER_SAMPLE / 8)).to_le_bytes());
        wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        wav
    }

    fn start_loopback_media_server() -> (LoopbackMediaServerGuard, String) {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("loopback media listener should bind");
        let address = listener
            .local_addr()
            .expect("loopback media listener should have an address");
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let body = Arc::new(silent_wav_bytes());
        let server_body = Arc::clone(&body);
        let thread = std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                if server_stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut request = [0_u8; 4_096];
                let _ = stream.read(&mut request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    server_body.len()
                );
                if stream.write_all(response.as_bytes()).is_ok() {
                    let _ = stream.write_all(&server_body);
                }
            }
        });
        let url = format!("http://{address}/sorotte-network-hook-smoke.wav");
        (
            LoopbackMediaServerGuard {
                address,
                stop,
                thread: Some(thread),
            },
            url,
        )
    }

    fn connect_with_retry(endpoint: &str) -> MpvAdapter {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_error = None;
        while Instant::now() < deadline {
            match MpvAdapter::with_json_ipc(endpoint) {
                Ok(adapter) => return adapter,
                Err(error) => {
                    last_error = Some(error.to_string());
                    sleep(Duration::from_millis(50));
                }
            }
        }
        panic!(
            "real mpv JSON IPC did not become ready: {}",
            last_error.as_deref().unwrap_or("<no attempt>")
        );
    }

    fn wait_for_network_outcome(
        adapter: &mut MpvAdapter,
    ) -> MpvNetworkMediaOptionsTransitionOutcome {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(outcome) = adapter.take_network_media_options_transition_outcome() {
                return outcome;
            }
            sleep(Duration::from_millis(25));
        }
        panic!("real mpv did not publish a network-options transition outcome");
    }

    let mpv_bin = std::env::var_os("SOROTTE_TEST_MPV_BIN")
        .map(PathBuf::from)
        .expect("set SOROTTE_TEST_MPV_BIN to an mpv executable before running this ignored test");
    assert!(
        mpv_bin.is_file(),
        "mpv binary must exist: {}",
        mpv_bin.display()
    );
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    #[cfg(windows)]
    let (endpoint, cleanup_path) = (
        format!(
            r"\\.\pipe\sorotte-real-bridge-{}-{unique}",
            std::process::id()
        ),
        None,
    );
    #[cfg(not(windows))]
    let (endpoint, cleanup_path) = {
        let path = std::env::temp_dir().join(format!(
            "sorotte-real-bridge-{}-{unique}.sock",
            std::process::id()
        ));
        (path.to_string_lossy().into_owned(), Some(path))
    };

    let child = Command::new(&mpv_bin)
        .arg("--no-config")
        .arg("--no-terminal")
        .arg("--idle=yes")
        .arg("--force-window=no")
        .arg(format!("--input-ipc-server={endpoint}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("real mpv should launch");
    let _guard = RealMpvGuard {
        child,
        cleanup_path,
    };

    let mut owner = connect_with_retry(&endpoint);
    let mut settings = LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..LegacySyncplayUiSettings::default()
    };
    owner
        .configure_legacy_syncplay_ui_settings(settings.clone())
        .expect("core mpv UI settings should apply");
    assert_eq!(
        owner.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready,
        "load-script, canonical ping/pong discovery, and exact settings acknowledgement must work"
    );

    settings.chat_input_enabled = false;
    owner
        .configure_legacy_syncplay_ui_settings(settings.clone())
        .expect("dynamic input disable should apply");
    assert_eq!(
        owner.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready
    );
    settings.chat_input_enabled = true;
    owner
        .configure_legacy_syncplay_ui_settings(settings.clone())
        .expect("dynamic input re-enable should apply");
    assert_eq!(
        owner.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready
    );

    let mut contender = connect_with_retry(&endpoint);
    contender.set_test_sorotte_bridge_owner_id("real-mpv-contending-owner");
    contender
        .configure_legacy_syncplay_ui_settings(settings.clone())
        .expect("contender core mpv settings should apply independently");
    let contender_task = std::thread::spawn(move || {
        let health = contender.configure_bundled_sorotte_bridge();
        (contender, health)
    });
    let heartbeat_deadline = Instant::now() + Duration::from_secs(4);
    while !contender_task.is_finished() && Instant::now() < heartbeat_deadline {
        let _ = owner.take_playback_telemetry_update();
        sleep(Duration::from_millis(100));
    }
    let (mut contender, contender_health) = contender_task
        .join()
        .expect("contender bridge task should not panic");
    assert!(
        matches!(
            &contender_health,
            SorotteBridgeHealth::Degraded(failure)
                if failure.kind == SorotteBridgeFailureKind::LeaseBusy
        ),
        "a duplicate live owner must degrade only bridge integration: {contender_health:?}"
    );

    owner.release_sorotte_bridge_best_effort();
    sleep(Duration::from_millis(200));
    assert_eq!(
        contender.retry_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready,
        "graceful release should allow immediate in-place takeover"
    );

    contender.configure_network_media_options([("cache-secs", "75")]);
    assert_eq!(
        contender
            .apply_network_media_options_to_active_media_classified()
            .expect("the core network hook should load, acknowledge, and own the idle player"),
        MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia
    );

    let mut hook_contender = connect_with_retry(&endpoint);
    hook_contender.set_test_sorotte_bridge_owner_id("real-mpv-network-hook-contender");
    hook_contender.configure_network_media_options([("cache-secs", "90")]);
    let busy_error = hook_contender
        .apply_network_media_options_to_active_media_classified()
        .expect_err("a live different core-hook owner must reject takeover");
    assert!(busy_error.to_string().contains("owned by"));
    assert!(hook_contender.is_connected());

    let (_network_media_server, network_media_url) = start_loopback_media_server();
    hook_contender
        .open_file(&network_media_url)
        .expect("real mpv should accept the asynchronous network load request");
    let network_outcome = wait_for_network_outcome(&mut contender);
    if let MpvNetworkMediaOptionsTransitionOutcome::Failed(error)
    | MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error) = &network_outcome
    {
        panic!("the real-mpv network hook failed: {error}");
    }
    assert_eq!(
        network_outcome,
        MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated,
        "the on-load hook should apply the owned option map to network media"
    );

    let missing_local = std::env::temp_dir().join(format!(
        "sorotte-network-hook-local-{}-{unique}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing_local);
    hook_contender
        .open_file(&missing_local.to_string_lossy())
        .expect("real mpv should accept the asynchronous local load request");
    assert_eq!(
        wait_for_network_outcome(&mut contender),
        MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged,
        "local on-load must complete the installed policy without a file-local write"
    );

    sleep(Duration::from_millis(10_200));
    let takeover_deadline = Instant::now() + Duration::from_secs(3);
    let _takeover = loop {
        match hook_contender.apply_network_media_options_to_active_media_classified() {
            Ok(takeover) => break takeover,
            Err(error)
                if error.to_string().contains("owned by") && Instant::now() < takeover_deadline =>
            {
                sleep(Duration::from_millis(100));
            }
            Err(error) => panic!("an expired lease should allow takeover: {error}"),
        }
    };
    assert_eq!(
        hook_contender
            .network_options_runtime_health_snapshot()
            .hook_health,
        MpvNetworkOptionsHookHealth::Ready
    );
    assert!(matches!(
        wait_for_network_outcome(&mut contender),
        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)
            if error.to_string().contains("ownership")
                || error.to_string().contains("lease expired")
    ));
    assert!(
        contender.is_connected(),
        "lease replacement must not detach playback"
    );
    hook_contender.release_sorotte_bridge_best_effort();

    settings.chat_input_enabled = false;
    contender
        .configure_legacy_syncplay_ui_settings(settings)
        .expect("new owner should dynamically disable input");
    assert_eq!(
        contender.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready
    );
    contender.release_sorotte_bridge_best_effort();
}

#[cfg(windows)]
#[test]
#[ignore = "local smoke test; requires standalone mpv binary and media file"]
fn local_standalone_mpv_smoke_reports_file_metadata() {
    use std::{
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread::sleep,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
    }

    fn default_mpv_bin(root: &Path) -> PathBuf {
        root.join("mpv").join("mpv.exe")
    }

    fn first_media_file(media_dir: &Path) -> Option<PathBuf> {
        let mut entries = std::fs::read_dir(media_dir).ok()?;
        while let Some(Ok(entry)) = entries.next() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase());
            let Some(ext) = ext else { continue };
            if matches!(ext.as_str(), "mkv" | "mp4" | "avi" | "webm" | "mov" | "m4v") {
                return Some(path);
            }
        }
        None
    }

    fn env_duration_ms(name: &str, default_ms: u64) -> Duration {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(default_ms))
    }

    struct MpvChildGuard(Child);

    impl Drop for MpvChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = repo_root();
    let mpv_bin = std::env::var_os("SOROTTE_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_mpv_bin(&root));
    let media_file = std::env::var_os("SOROTTE_MPV_SMOKE_MEDIA")
        .map(PathBuf::from)
        .or_else(|| first_media_file(&root.join("media")))
        .expect("expected media file in ./media or SOROTTE_MPV_SMOKE_MEDIA");

    if !mpv_bin.exists() {
        panic!(
            "mpv binary not found at {} (override with SOROTTE_MPV_SMOKE_BIN)",
            mpv_bin.display()
        );
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis();
    let pipe_path = format!(
        r"\\.\pipe\sorotte-mpv-smoke-{}-{unique}",
        std::process::id()
    );
    let connect_timeout = env_duration_ms("SOROTTE_MPV_SMOKE_CONNECT_TIMEOUT_MS", 5_000);
    let metadata_timeout = env_duration_ms("SOROTTE_MPV_SMOKE_METADATA_TIMEOUT_MS", 10_000);
    let poll_interval = env_duration_ms("SOROTTE_MPV_SMOKE_POLL_INTERVAL_MS", 50);

    let child = MpvChildGuard(
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
            .expect("standalone mpv should start for local smoke test"),
    );

    let mut adapter = None;
    let connect_started = Instant::now();
    let mut last_connect_error = None;
    while connect_started.elapsed() < connect_timeout {
        match MpvAdapter::with_json_ipc(&pipe_path) {
            Ok(attached) => {
                adapter = Some(attached);
                break;
            }
            Err(err) => {
                last_connect_error = Some(err.to_string());
                sleep(poll_interval);
            }
        }
    }
    let mut adapter = match adapter {
        Some(adapter) => adapter,
        None => panic!(
            "expected to connect to mpv JSON IPC pipe within {:?} (pipe={}, mpv_bin={}, media={}); last error: {}",
            connect_timeout,
            pipe_path,
            mpv_bin.display(),
            media_file.display(),
            last_connect_error.as_deref().unwrap_or("<none>")
        ),
    };

    let mut observed_update = None;
    let mut last_update = None;
    let mut last_telemetry = None;
    let metadata_started = Instant::now();
    while metadata_started.elapsed() < metadata_timeout {
        if let Some(update) = adapter.take_local_file_update() {
            last_update = Some(update.clone());
            let has_duration = update
                .duration_seconds
                .is_some_and(|duration| duration > 1.0);
            let has_path = update.path.is_some();
            if has_path && has_duration {
                observed_update = Some(update);
                break;
            }
        }
        while let Some(telemetry) = adapter.take_playback_telemetry_update() {
            last_telemetry = Some(telemetry);
        }
        sleep(poll_interval);
    }

    drop(child);

    let update = observed_update.unwrap_or_else(|| {
        panic!(
            "expected mpv telemetry-driven LocalFileUpdate within {:?} (poll_interval={:?}, pipe={}, mpv_bin={}, media={}); last_update={:?}; last_telemetry={:?}",
            metadata_timeout,
            poll_interval,
            pipe_path,
            mpv_bin.display(),
            media_file.display(),
            last_update,
            last_telemetry
        )
    });
    let expected_name = media_file
        .file_name()
        .and_then(|name| name.to_str())
        .expect("media file should have a UTF-8 filename");
    assert_eq!(update.name, expected_name);
    assert!(
        update
            .duration_seconds
            .is_some_and(|duration| duration > 60.0),
        "expected realistic media duration from mpv telemetry, got {:?}",
        update.duration_seconds
    );
    assert!(
        update.path.is_some(),
        "expected mpv to report a path for the loaded file"
    );
}
