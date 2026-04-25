#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[test]
#[ignore = "requires local standalone mpv binary and media asset"]
fn explicit_mpv_ipc_cli_startup_smoke_applies_file_and_supported_player_args_to_real_mpv() {
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        r"\\.\pipe\syncplay-rust-cli-explicit-ipc-smoke-{}-{unique}",
        std::process::id()
    );

    let _child = MpvChildGuard(
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("standalone mpv should start for explicit IPC smoke test"),
    );

    let mut adapter = crate::connect_mpv_adapter_with_retry(
        &pipe_path,
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
    .expect("should attach to explicit mpv JSON IPC");

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SYNCPLAY_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SYNCPLAY_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.set_var(key_client_ipc, &pipe_path);
    env.remove_var(key_fallback_ipc);

    let result = (|| {
        let overrides = LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: false,
            clear_gui_data_requested: false,
            language: None,
            player_path: None,
            file: Some(media_file.to_string_lossy().into_owned()),
            player_args: vec![
                "--profile=fast".to_owned(), // unsupported in explicit-IPC subset; should be ignored
                "--start=1.5".to_owned(),
                "--pause".to_owned(),
                "--speed=1.25".to_owned(),
                "--volume=33".to_owned(),
                "--mute".to_owned(),
                "--deinterlace".to_owned(),
                "--keepaspect".to_owned(),
                "--keepaspect-window".to_owned(),
                "--border".to_owned(),
                "--force-window".to_owned(),
                "--keep-open".to_owned(),
                "--keep-open-pause".to_owned(),
                "--cursor-autohide-fs-only".to_owned(),
                "--stop-screensaver".to_owned(),
                "--sub-visibility".to_owned(),
                "--osd-bar".to_owned(),
                "--window-maximized".to_owned(),
                "--window-minimized".to_owned(),
            ],
            load_playlist_from_file: None,
            host: None,
            port: None,
            username: None,
            room: None,
            controlled_room_password_override: None,
            show_help: false,
            show_version: false,
            unknown_options: vec![],
        };

        let applied =
            apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
                &mut adapter,
                &overrides,
            )
            .expect("explicit-mpv-IPC startup helper should succeed against real mpv");
        assert!(
            applied,
            "startup helper should report that it applied actions"
        );

        let expected_name = media_file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("media file should have utf-8 filename")
            .to_owned();
        let started = Instant::now();
        let timeout = Duration::from_secs(12);
        let poll_interval = Duration::from_millis(50);
        let mut saw_local_file = false;
        let mut saw_pause_true = false;
        let mut saw_speed = false;
        let mut saw_position = false;
        let mut saw_volume = false;
        let mut saw_muted = false;
        let mut saw_deinterlace = false;
        let mut saw_keepaspect = false;
        let mut saw_keepaspect_window = false;
        let mut saw_border = false;
        let mut saw_force_window = false;
        let mut saw_keep_open = false;
        let mut saw_keep_open_pause = false;
        let mut saw_cursor_autohide_fs_only = false;
        let mut saw_stop_screensaver = false;
        let mut saw_sub_visibility = false;
        let mut saw_osd_bar = false;
        let mut saw_window_maximized = false;
        let mut saw_window_minimized = false;
        let mut last_update = None;
        let mut last_telemetry = None;

        while started.elapsed() < timeout {
            if let Some(update) = adapter.take_local_file_update() {
                if update.name == expected_name {
                    saw_local_file = true;
                }
                last_update = Some(update);
            }
            while let Some(telemetry) = adapter.take_playback_telemetry_update() {
                last_telemetry = Some(telemetry);
            }

            if adapter.paused() {
                saw_pause_true = true;
            }
            if (adapter.playback_rate() - 1.25).abs() < 0.05 {
                saw_speed = true;
            }
            if adapter.position_seconds() >= 1.0 {
                saw_position = true;
            }
            if (adapter.volume() - 33.0).abs() < 0.5 {
                saw_volume = true;
            }
            if adapter.muted() {
                saw_muted = true;
            }
            if adapter.deinterlace() {
                saw_deinterlace = true;
            }
            if adapter.keepaspect() {
                saw_keepaspect = true;
            }
            if adapter.keepaspect_window() {
                saw_keepaspect_window = true;
            }
            if adapter.border() {
                saw_border = true;
            }
            if adapter.force_window() {
                saw_force_window = true;
            }
            if adapter.keep_open() {
                saw_keep_open = true;
            }
            if adapter.keep_open_pause() {
                saw_keep_open_pause = true;
            }
            if adapter.cursor_autohide_fs_only() {
                saw_cursor_autohide_fs_only = true;
            }
            if adapter.stop_screensaver() {
                saw_stop_screensaver = true;
            }
            if adapter.sub_visibility() {
                saw_sub_visibility = true;
            }
            if adapter.osd_bar() {
                saw_osd_bar = true;
            }
            if adapter.window_maximized() {
                saw_window_maximized = true;
            }
            if adapter.window_minimized() {
                saw_window_minimized = true;
            }

            if saw_local_file
                && saw_pause_true
                && saw_speed
                && saw_position
                && saw_volume
                && saw_muted
                && saw_deinterlace
                && saw_keepaspect
                && saw_keepaspect_window
                && saw_border
                && saw_force_window
                && saw_keep_open
                && saw_keep_open_pause
                && saw_cursor_autohide_fs_only
                && saw_stop_screensaver
                && saw_sub_visibility
                && saw_osd_bar
                && saw_window_maximized
                && saw_window_minimized
            {
                return Ok(());
            }
            std::thread::sleep(poll_interval);
        }

        Err(anyhow::anyhow!(
            "expected explicit-mpv-IPC startup helper to apply file/start/pause/speed/volume/mute/deinterlace/keepaspect/keepaspect-window/border/force-window/keep-open/keep-open-pause/cursor-autohide-fs-only/stop-screensaver/sub-visibility/osd-bar/window-maximized/window-minimized within {:?} (mpv_bin={}, media={}, pipe={}); state: saw_local_file={}, saw_pause_true={}, saw_speed={}, saw_position={}, saw_volume={}, saw_muted={}, saw_deinterlace={}, saw_keepaspect={}, saw_keepaspect_window={}, saw_border={}, saw_force_window={}, saw_keep_open={}, saw_keep_open_pause={}, saw_cursor_autohide_fs_only={}, saw_stop_screensaver={}, saw_sub_visibility={}, saw_osd_bar={}, saw_window_maximized={}, saw_window_minimized={}; adapter_path={:?}; paused={}; position={}; speed={}; volume={}; muted={}; deinterlace={}; keepaspect={}; keepaspect_window={}; border={}; force_window={}; keep_open={}; keep_open_pause={}; cursor_autohide_fs_only={}; stop_screensaver={}; sub_visibility={}; osd_bar={}; window_maximized={}; window_minimized={}; last_update={:?}; last_telemetry={:?}",
            timeout,
            mpv_bin.display(),
            media_file.display(),
            pipe_path,
            saw_local_file,
            saw_pause_true,
            saw_speed,
            saw_position,
            saw_volume,
            saw_muted,
            saw_deinterlace,
            saw_keepaspect,
            saw_keepaspect_window,
            saw_border,
            saw_force_window,
            saw_keep_open,
            saw_keep_open_pause,
            saw_cursor_autohide_fs_only,
            saw_stop_screensaver,
            saw_sub_visibility,
            saw_osd_bar,
            saw_window_maximized,
            saw_window_minimized,
            adapter.current_path(),
            adapter.paused(),
            adapter.position_seconds(),
            adapter.playback_rate(),
            adapter.volume(),
            adapter.muted(),
            adapter.deinterlace(),
            adapter.keepaspect(),
            adapter.keepaspect_window(),
            adapter.border(),
            adapter.force_window(),
            adapter.keep_open(),
            adapter.keep_open_pause(),
            adapter.cursor_autohide_fs_only(),
            adapter.stop_screensaver(),
            adapter.sub_visibility(),
            adapter.osd_bar(),
            adapter.window_maximized(),
            adapter.window_minimized(),
            last_update,
            last_telemetry
        ))
    })();

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }

    result.expect("explicit-mpv-IPC startup smoke should apply supported subset to real mpv");
}
