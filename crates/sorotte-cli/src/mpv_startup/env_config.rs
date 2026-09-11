use super::*;

pub(crate) fn managed_mpv_launch_env_config_from_env() -> ManagedMpvLaunchEnvConfig {
    ManagedMpvLaunchEnvConfig {
        enabled: env_flag_enabled("SOROTTE_CLIENT_MPV_MANAGED_LAUNCH"),
        mpv_bin: env_trimmed("SOROTTE_CLIENT_MPV_MANAGED_BIN").map(PathBuf::from),
        media_file: env_trimmed("SOROTTE_CLIENT_MPV_MANAGED_MEDIA").map(PathBuf::from),
        extra_args: Vec::new(),
        ipc_path: env_trimmed("SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH"),
        connect_timeout_ms: env_u32("SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS"),
        connect_poll_interval_ms: env_u32("SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS"),
    }
}

pub(crate) fn apply_syncplay_client_arg_managed_mpv_overrides(
    managed_config: &mut ManagedMpvLaunchEnvConfig,
    argument_overrides: Option<&SyncplayClientArgOverrides>,
) {
    let Some(overrides) = argument_overrides else {
        return;
    };
    let player_path = overrides.player_path.as_deref();
    let player_requests_managed_mpv = player_path.is_some_and(player_path_requests_managed_mpv);

    if !managed_config.enabled && player_requests_managed_mpv {
        managed_config.enabled = true;
    }

    if managed_config.mpv_bin.is_none()
        && player_requests_managed_mpv
        && let Some(player_path) = player_path
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
    env_trimmed("SOROTTE_CLIENT_MPV_IPC_PATH").or_else(|| env_trimmed("SOROTTE_MPV_IPC_PATH"))
}
