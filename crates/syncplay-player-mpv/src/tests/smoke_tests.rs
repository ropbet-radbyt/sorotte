use super::*;

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
    let mpv_bin = std::env::var_os("SYNCPLAY_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_mpv_bin(&root));
    let media_file = std::env::var_os("SYNCPLAY_MPV_SMOKE_MEDIA")
        .map(PathBuf::from)
        .or_else(|| first_media_file(&root.join("media")))
        .expect("expected media file in ./media or SYNCPLAY_MPV_SMOKE_MEDIA");

    if !mpv_bin.exists() {
        panic!(
            "mpv binary not found at {} (override with SYNCPLAY_MPV_SMOKE_BIN)",
            mpv_bin.display()
        );
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis();
    let pipe_path = format!(
        r"\\.\pipe\syncplay-rust-mpv-smoke-{}-{unique}",
        std::process::id()
    );
    let connect_timeout = env_duration_ms("SYNCPLAY_MPV_SMOKE_CONNECT_TIMEOUT_MS", 5_000);
    let metadata_timeout = env_duration_ms("SYNCPLAY_MPV_SMOKE_METADATA_TIMEOUT_MS", 10_000);
    let poll_interval = env_duration_ms("SYNCPLAY_MPV_SMOKE_POLL_INTERVAL_MS", 50);

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
