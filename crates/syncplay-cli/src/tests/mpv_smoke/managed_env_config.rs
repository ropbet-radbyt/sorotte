use super::*;

#[test]
fn managed_mpv_launch_env_config_from_env_parses_values() {
    let key_enabled = "SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH";
    let key_bin = "SYNCPLAY_CLIENT_MPV_MANAGED_BIN";
    let key_media = "SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA";
    let key_ipc = "SYNCPLAY_CLIENT_MPV_MANAGED_IPC_PATH";
    let key_timeout = "SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS";
    let key_poll = "SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS";
    let old_enabled = std::env::var(key_enabled).ok();
    let old_bin = std::env::var(key_bin).ok();
    let old_media = std::env::var(key_media).ok();
    let old_ipc = std::env::var(key_ipc).ok();
    let old_timeout = std::env::var(key_timeout).ok();
    let old_poll = std::env::var(key_poll).ok();

    // SAFETY: Scoped unit-test env mutation with restoration before return.
    unsafe {
        std::env::set_var(key_enabled, "1");
        std::env::set_var(key_bin, "C:\\tmp\\mpv.exe");
        std::env::set_var(key_media, "C:\\tmp\\video.mkv");
        std::env::set_var(key_ipc, "\\\\.\\pipe\\syncplay-test");
        std::env::set_var(key_timeout, "7000");
        std::env::set_var(key_poll, "25");
    }

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

    // SAFETY: Scoped unit-test env restoration.
    unsafe {
        std::env::remove_var(key_enabled);
        std::env::remove_var(key_bin);
        std::env::remove_var(key_media);
        std::env::remove_var(key_ipc);
        std::env::remove_var(key_timeout);
        std::env::remove_var(key_poll);
    }
    if let Some(value) = old_enabled {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_enabled, value);
        }
    }
    if let Some(value) = old_bin {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_bin, value);
        }
    }
    if let Some(value) = old_media {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_media, value);
        }
    }
    if let Some(value) = old_ipc {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_ipc, value);
        }
    }
    if let Some(value) = old_timeout {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_timeout, value);
        }
    }
    if let Some(value) = old_poll {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_poll, value);
        }
    }
}
