use super::*;

pub(crate) fn managed_mpv_launch_env_config_from_env() -> ManagedMpvLaunchEnvConfig {
    ManagedMpvLaunchEnvConfig {
        enabled: env_flag_enabled("SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH"),
        mpv_bin: env_trimmed("SYNCPLAY_CLIENT_MPV_MANAGED_BIN").map(PathBuf::from),
        media_file: env_trimmed("SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA").map(PathBuf::from),
        extra_args: Vec::new(),
        ipc_path: env_trimmed("SYNCPLAY_CLIENT_MPV_MANAGED_IPC_PATH"),
        connect_timeout_ms: env_u32("SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS"),
        connect_poll_interval_ms: env_u32("SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS"),
    }
}

pub(crate) fn apply_legacy_client_arg_managed_mpv_overrides(
    managed_config: &mut ManagedMpvLaunchEnvConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) {
    let Some(overrides) = legacy_overrides else {
        return;
    };
    let legacy_player_path = overrides.player_path.as_deref();
    let legacy_player_requests_managed_mpv =
        legacy_player_path.is_some_and(legacy_player_path_requests_managed_mpv_legacy_compatible);

    if !managed_config.enabled && legacy_player_requests_managed_mpv {
        managed_config.enabled = true;
    }

    if managed_config.mpv_bin.is_none()
        && legacy_player_requests_managed_mpv
        && let Some(player_path) = legacy_player_path
    {
        managed_config.mpv_bin = Some(PathBuf::from(player_path));
    }
    if managed_config.media_file.is_none()
        && let Some(file) = overrides.file.as_deref()
    {
        managed_config.media_file = Some(PathBuf::from(file));
    }
    if managed_config.extra_args.is_empty() && !overrides.player_args.is_empty() {
        managed_config.extra_args = overrides.player_args.clone();
    }
}

pub(crate) fn explicit_mpv_ipc_path_from_env() -> Option<String> {
    env_trimmed("SYNCPLAY_CLIENT_MPV_IPC_PATH").or_else(|| env_trimmed("SYNCPLAY_MPV_IPC_PATH"))
}
