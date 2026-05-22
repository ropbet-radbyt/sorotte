use super::*;

#[test]
#[ignore = "requires local standalone mpv binary and media asset"]
fn managed_mpv_cli_smoke_publishes_local_file_metadata_without_external_ipc_setup() {
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

    let key_enabled = "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH";
    let key_bin = "SOROTTE_CLIENT_MPV_MANAGED_BIN";
    let key_media = "SOROTTE_CLIENT_MPV_MANAGED_MEDIA";
    let key_ipc = "SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH";
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let old_enabled = std::env::var(key_enabled).ok();
    let old_bin = std::env::var(key_bin).ok();
    let old_media = std::env::var(key_media).ok();
    let old_ipc = std::env::var(key_ipc).ok();
    let old_client_ipc = std::env::var(key_client_ipc).ok();
    let old_fallback_ipc = std::env::var(key_fallback_ipc).ok();
    env.set_var(key_enabled, "1");
    env.set_var(key_bin, mpv_bin.as_os_str());
    env.set_var(key_media, media_file.as_os_str());
    env.remove_var(key_ipc);
    env.remove_var(key_client_ipc);
    env.remove_var(key_fallback_ipc);

    let result = (|| {
        let config = test_client_loop_config();
        let (mut runtime, _managed_guard) =
            create_client_runtime_with_managed_mpv_support(&config, None, None)
                .expect("managed mpv runtime creation should succeed");

        let expected_name = media_file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("media file should have utf-8 filename")
            .to_owned();
        let metadata_timeout = Duration::from_secs(12);
        let poll_interval = Duration::from_millis(50);
        let started = std::time::Instant::now();
        let mut published_lines = Vec::new();
        let mut last_telemetry = None;

        while started.elapsed() < metadata_timeout {
            for telemetry in runtime.drain_player_playback_telemetry_updates() {
                last_telemetry = Some(telemetry);
            }

            let published = runtime
                .publish_pending_local_file_update_legacy_compatible(
                    PrivacyMode::SendRaw,
                    PrivacyMode::SendRaw,
                )
                .expect("publishing pending local file update should not fail");

            if published {
                runtime
                    .flush_queued_protocol_lines_to_transport(|line| {
                        published_lines.push(line.to_owned());
                        Ok(())
                    })
                    .expect("queued protocol line flush should succeed");
                if published_lines
                    .iter()
                    .any(|line| line.contains(&expected_name))
                {
                    return Ok((published_lines, last_telemetry, expected_name));
                }
            }

            std::thread::sleep(poll_interval);
        }

        Err(anyhow::anyhow!(
            "expected managed mpv CLI smoke to publish local file metadata within {:?} (mpv_bin={}, media={}); lines={:?}; last_telemetry={:?}",
            metadata_timeout,
            mpv_bin.display(),
            media_file.display(),
            published_lines,
            last_telemetry
        ))
    })();
    env.remove_var(key_enabled);
    env.remove_var(key_bin);
    env.remove_var(key_media);
    env.remove_var(key_ipc);
    env.remove_var(key_client_ipc);
    env.remove_var(key_fallback_ipc);

    if let Some(value) = old_enabled {
        env.set_var(key_enabled, value);
    }
    if let Some(value) = old_bin {
        env.set_var(key_bin, value);
    }
    if let Some(value) = old_media {
        env.set_var(key_media, value);
    }
    if let Some(value) = old_ipc {
        env.set_var(key_ipc, value);
    }
    if let Some(value) = old_client_ipc {
        env.set_var(key_client_ipc, value);
    }
    if let Some(value) = old_fallback_ipc {
        env.set_var(key_fallback_ipc, value);
    }

    let (published_lines, _last_telemetry, expected_name) =
        result.expect("managed mpv CLI smoke should publish local file metadata");
    assert!(
        published_lines.iter().any(|line| line.contains("\"Set\"")),
        "expected queued Set message from local file publish, got {published_lines:?}"
    );
    assert!(
        published_lines
            .iter()
            .any(|line| line.contains(&expected_name)),
        "expected queued local file publish to include media filename '{expected_name}'; lines={published_lines:?}"
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires local standalone mpv binary and media asset"]
fn unmanaged_external_mpv_smoke_launch_spec_and_spawn_apply_file_and_player_args() {
    use std::process::Child;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        r"\\.\pipe\sorotte-cli-unmanaged-smoke-{}-{unique}",
        std::process::id()
    );

    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: Some(mpv_bin.to_string_lossy().into_owned()),
        file: Some(media_file.to_string_lossy().into_owned()),
        player_args: vec![
            "--force-window=no".to_owned(),
            "--idle=yes".to_owned(),
            "--pause".to_owned(),
            format!("--input-ipc-server={pipe_path}"),
            "--start=1.5".to_owned(),
            "--speed=1.25".to_owned(),
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

    let spec = legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides)
        .expect("player-path should produce unmanaged external launch spec");
    assert_eq!(
        spec.program, mpv_bin,
        "launch spec should preserve the requested external player path"
    );
    assert_eq!(
        spec.args.last(),
        Some(&media_file.to_string_lossy().into_owned()),
        "legacy unmanaged startup should append file after player args"
    );

    let mut child = MpvChildGuard(
        spawn_legacy_external_player_from_spec_legacy_compatible(&spec)
            .expect("legacy unmanaged external spawn should start real mpv"),
    );

    let mut adapter = crate::connect_mpv_adapter_with_retry(
        &pipe_path,
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
    .expect("should attach to unmanaged mpv JSON IPC provided via forwarded player args");

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

        if saw_local_file && saw_pause_true && saw_speed && saw_position {
            return;
        }
        std::thread::sleep(poll_interval);
    }

    // Keep child alive until after diagnostics are captured; kill on drop.
    let _ = &mut child;
    panic!(
        "expected unmanaged external mpv launch to apply file/start/pause/speed within {:?} (mpv_bin={}, media={}, pipe={}); state: saw_local_file={}, saw_pause_true={}, saw_speed={}, saw_position={}; adapter_path={:?}; paused={}; position={}; speed={}; last_update={:?}; last_telemetry={:?}",
        timeout,
        spec.program.display(),
        media_file.display(),
        pipe_path,
        saw_local_file,
        saw_pause_true,
        saw_speed,
        saw_position,
        adapter.current_path(),
        adapter.paused(),
        adapter.position_seconds(),
        adapter.playback_rate(),
        last_update,
        last_telemetry
    );
}
