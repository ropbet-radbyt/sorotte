use super::*;

#[test]
fn managed_mpv_launch_base_args_preserve_local_cache_defaults() {
    let args = managed_mpv_launch_base_args_legacy_compatible(r"\\.\pipe\sorotte-cli-mpv-test");

    assert_eq!(
        args,
        vec![
            "--pause".to_owned(),
            "--force-window=no".to_owned(),
            "--idle=yes".to_owned(),
            r"--input-ipc-server=\\.\pipe\sorotte-cli-mpv-test".to_owned(),
        ]
    );
}

#[test]
fn managed_mpv_launch_env_config_from_env_parses_values() {
    let key_enabled = "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH";
    let key_bin = "SOROTTE_CLIENT_MPV_MANAGED_BIN";
    let key_media = "SOROTTE_CLIENT_MPV_MANAGED_MEDIA";
    let key_ipc = "SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH";
    let key_timeout = "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS";
    let key_poll = "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS";
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let old_enabled = std::env::var(key_enabled).ok();
    let old_bin = std::env::var(key_bin).ok();
    let old_media = std::env::var(key_media).ok();
    let old_ipc = std::env::var(key_ipc).ok();
    let old_timeout = std::env::var(key_timeout).ok();
    let old_poll = std::env::var(key_poll).ok();
    env.set_var(key_enabled, "1");
    env.set_var(key_bin, "C:\\tmp\\mpv.exe");
    env.set_var(key_media, "C:\\tmp\\video.mkv");
    env.set_var(key_ipc, "\\\\.\\pipe\\syncplay-test");
    env.set_var(key_timeout, "7000");
    env.set_var(key_poll, "25");

    let config = managed_mpv_launch_env_config_from_env();
    assert!(config.enabled);
    assert_eq!(
        config.mpv_bin,
        Some(std::path::PathBuf::from("C:\\tmp\\mpv.exe"))
    );
    assert_eq!(
        config.media_file,
        Some(std::path::PathBuf::from("C:\\tmp\\video.mkv"))
    );
    assert_eq!(
        config.ipc_path.as_deref(),
        Some("\\\\.\\pipe\\syncplay-test")
    );
    assert_eq!(config.connect_timeout_ms, Some(7000));
    assert_eq!(config.connect_poll_interval_ms, Some(25));
    env.remove_var(key_enabled);
    env.remove_var(key_bin);
    env.remove_var(key_media);
    env.remove_var(key_ipc);
    env.remove_var(key_timeout);
    env.remove_var(key_poll);

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
    if let Some(value) = old_timeout {
        env.set_var(key_timeout, value);
    }
    if let Some(value) = old_poll {
        env.set_var(key_poll, value);
    }
}

#[test]
fn managed_mpv_startup_keeps_initial_network_url_for_post_attach_load() {
    let mut config = ManagedMpvLaunchEnvConfig::default();
    let overrides = LegacyClientArgOverrides {
        player_path: Some("mpv".to_owned()),
        file: Some("https://media.example/video.m3u8".to_owned()),
        player_args: vec!["--cache-secs=75".to_owned()],
        ..LegacyClientArgOverrides::default()
    };

    apply_legacy_client_arg_managed_mpv_overrides(&mut config, Some(&overrides));

    assert!(config.enabled);
    assert_eq!(
        config
            .media_file
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned()),
        Some("https://media.example/video.m3u8".to_owned())
    );
    assert_eq!(config.extra_args, vec!["--cache-secs=75".to_owned()]);
}
