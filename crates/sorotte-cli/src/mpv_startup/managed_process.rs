use super::*;

#[derive(Debug)]
pub(crate) struct ManagedMpvProcessGuard {
    child: Child,
    ipc_cleanup_path: Option<PathBuf>,
}

impl Drop for ManagedMpvProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(path) = self.ipc_cleanup_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(crate) fn create_client_runtime_with_managed_mpv_support(
    config: &ClientLoopConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
) -> anyhow::Result<(
    ClientApplication<MpvAdapter>,
    Option<ManagedMpvProcessGuard>,
)> {
    let session = create_client_session(config);
    let (mut player, managed_guard, managed_startup_media) =
        create_mpv_adapter_and_optional_managed_process_from_env(legacy_overrides)?;
    apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
        &mut player,
        stored_settings,
    )?;
    let streaming = stored_settings
        .map(ClientConfig::resolve)
        .map(|resolution| resolution.config.playback.streaming)
        .unwrap_or_default();
    let advanced_arguments = legacy_overrides
        .map(|overrides| overrides.player_args.as_slice())
        .unwrap_or_default();
    let effective_options = streaming.effective_mpv_options(advanced_arguments);
    player.configure_network_media_options(
        effective_options
            .iter()
            .map(|option| (option.name.clone(), option.effective_value.clone())),
    );
    player
        .apply_network_media_options_to_active_media()
        .map_err(|error| anyhow!("failed updating active mpv network-media options: {error}"))?;
    if let Some(media) = managed_startup_media {
        player
            .open_file(&media)
            .map_err(|error| anyhow!("failed opening managed mpv startup media: {error}"))?;
    }
    Ok((ClientApplication::new(session, player), managed_guard))
}

fn create_mpv_adapter_and_optional_managed_process_from_env(
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> anyhow::Result<(MpvAdapter, Option<ManagedMpvProcessGuard>, Option<String>)> {
    let explicit_ipc_path = explicit_mpv_ipc_path_from_env();
    if let Some(ipc_path) = explicit_ipc_path {
        let mut adapter = create_mpv_adapter_from_path_or_disconnected(&ipc_path);
        let ytdl_probe_executable = legacy_overrides
            .and_then(|overrides| ytdl_probe_executable_from_mpv_args(&overrides.player_args));
        adapter.configure_ytdl_live_probe_executable(ytdl_probe_executable);
        return Ok((adapter, None, None));
    }

    let mut managed_config = managed_mpv_launch_env_config_from_env();
    apply_legacy_client_arg_managed_mpv_overrides(&mut managed_config, legacy_overrides);
    if !managed_config.enabled {
        #[cfg(test)]
        return Ok((SimulatedPlayer::new().into_inner(), None, None));
        #[cfg(not(test))]
        return Ok((MpvAdapter::default(), None, None));
    }

    let startup_media = managed_config
        .media_file
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let (adapter, guard) = spawn_managed_mpv_and_attach(managed_config)?;
    Ok((adapter, Some(guard), startup_media))
}

fn create_mpv_adapter_from_path_or_disconnected(ipc_path: &str) -> MpvAdapter {
    match MpvAdapter::with_json_ipc(ipc_path) {
        Ok(adapter) => adapter,
        Err(err) => {
            eprintln!(
                "warning: failed to connect mpv JSON IPC at '{ipc_path}': {err}; player is disconnected"
            );
            MpvAdapter::default()
        }
    }
}

fn spawn_managed_mpv_and_attach(
    config: ManagedMpvLaunchEnvConfig,
) -> anyhow::Result<(MpvAdapter, ManagedMpvProcessGuard)> {
    let requested_mpv_bin = config.mpv_bin.or_else(find_default_managed_mpv_bin).ok_or_else(|| {
        anyhow!(
            "managed mpv launch requested but no mpv binary was found; set SOROTTE_CLIENT_MPV_MANAGED_BIN"
        )
    })?;
    let mpv_bin = resolve_managed_mpv_launch_program_legacy_compatible(&requested_mpv_bin);
    if managed_mpv_launch_program_requires_existing_file_legacy_compatible(&mpv_bin)
        && !mpv_bin.is_file()
    {
        return Err(anyhow!(
            "managed mpv binary does not exist: {}",
            mpv_bin.display()
        ));
    }
    if let Some(media_file) = config.media_file.as_ref()
        && !media_file.to_string_lossy().contains("://")
        && !media_file.exists()
    {
        return Err(anyhow!(
            "managed mpv media file does not exist: {}",
            media_file.display()
        ));
    }

    let (ipc_path, ipc_cleanup_path) = if let Some(ipc_path) = config.ipc_path {
        let ipc_cleanup_path = ipc_cleanup_path_for_platform(&ipc_path);
        if let Some(path) = ipc_cleanup_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
        (ipc_path, ipc_cleanup_path)
    } else {
        generate_managed_mpv_ipc_path()?
    };

    let connect_timeout =
        Duration::from_millis(u64::from(config.connect_timeout_ms.unwrap_or(5_000).max(1)));
    let connect_poll_interval = Duration::from_millis(u64::from(
        config.connect_poll_interval_ms.unwrap_or(50).max(1),
    ));

    let mut command = Command::new(&mpv_bin);
    if let Some(parent) = mpv_bin.parent() {
        command.current_dir(parent);
    }
    command.args(managed_mpv_launch_base_args_legacy_compatible(&ipc_path));
    if !config.extra_args.is_empty() {
        command.args(&config.extra_args);
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let guard = ManagedMpvProcessGuard {
        child,
        ipc_cleanup_path,
    };
    let mut adapter =
        connect_mpv_adapter_with_retry(&ipc_path, connect_timeout, connect_poll_interval).map_err(
            |err| {
                anyhow!(
                    "managed mpv launched but JSON IPC attach failed (mpv_bin={}, ipc={}): {err}",
                    mpv_bin.display(),
                    ipc_path
                )
            },
        )?;
    adapter.configure_ytdl_live_probe_executable(ytdl_probe_executable_from_mpv_args(
        &config.extra_args,
    ));

    eprintln!("info: started managed mpv and attached JSON IPC at '{ipc_path}'");
    Ok((adapter, guard))
}

fn ytdl_probe_executable_from_mpv_args(args: &[String]) -> Option<PathBuf> {
    args.iter().enumerate().find_map(|(index, argument)| {
        let option_value = ["--script-opts-append=", "--script-opts="]
            .iter()
            .find_map(|prefix| argument.strip_prefix(prefix))
            .or_else(|| {
                matches!(argument.as_str(), "--script-opts-append" | "--script-opts")
                    .then(|| args.get(index + 1).map(String::as_str))
                    .flatten()
            })?;
        option_value.split(',').find_map(|entry| {
            entry
                .trim()
                .strip_prefix("ytdl_hook-ytdl_path=")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
    })
}

#[cfg(test)]
pub(crate) fn managed_mpv_launch_base_args_legacy_compatible(ipc_path: &str) -> Vec<String> {
    managed_mpv_launch_base_args(ipc_path)
}

#[cfg(not(test))]
fn managed_mpv_launch_base_args_legacy_compatible(ipc_path: &str) -> Vec<String> {
    managed_mpv_launch_base_args(ipc_path)
}

fn managed_mpv_launch_base_args(ipc_path: &str) -> Vec<String> {
    vec![
        "--pause".to_owned(),
        "--force-window=no".to_owned(),
        "--idle=yes".to_owned(),
        format!("--input-ipc-server={ipc_path}"),
    ]
}

pub(crate) fn connect_mpv_adapter_with_retry(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<MpvAdapter> {
    let started = std::time::Instant::now();
    let mut last_error = None;
    while started.elapsed() < timeout {
        match MpvAdapter::with_json_ipc(ipc_path) {
            Ok(adapter) => return Ok(adapter),
            Err(err) => {
                last_error = Some(err.to_string());
                std::thread::sleep(poll_interval);
            }
        }
    }

    Err(anyhow!(
        "timed out after {:?} waiting for mpv JSON IPC at '{}' (poll={:?}); last error: {}",
        timeout,
        ipc_path,
        poll_interval,
        last_error.as_deref().unwrap_or("<none>")
    ))
}

fn generate_managed_mpv_ipc_path() -> anyhow::Result<(String, Option<PathBuf>)> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| anyhow!("system time should be after unix epoch: {err}"))?
        .as_millis();
    #[cfg(windows)]
    {
        Ok((
            format!(r"\\.\pipe\sorotte-cli-mpv-{}-{unique}", std::process::id()),
            None,
        ))
    }
    #[cfg(not(windows))]
    {
        let path = std::env::temp_dir().join(format!(
            "sorotte-cli-mpv-{}-{unique}.sock",
            std::process::id()
        ));
        let path_str = path.to_string_lossy().into_owned();
        Ok((path_str, Some(path)))
    }
}

fn ipc_cleanup_path_for_platform(path: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let _ = path;
        None
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from(path))
    }
}

#[cfg(test)]
mod ytdl_probe_configuration_tests {
    use super::*;

    #[test]
    fn extracts_ytdl_hook_path_from_inline_and_separate_script_options() {
        assert_eq!(
            ytdl_probe_executable_from_mpv_args(&[
                "--script-opts-append=ytdl_hook-ytdl_path=C:/Tools/yt-dlp.exe".to_owned(),
            ]),
            Some(PathBuf::from("C:/Tools/yt-dlp.exe"))
        );
        assert_eq!(
            ytdl_probe_executable_from_mpv_args(&[
                "--script-opts".to_owned(),
                "osc-visibility=never,ytdl_hook-ytdl_path=/opt/sorotte/yt-dlp".to_owned(),
            ]),
            Some(PathBuf::from("/opt/sorotte/yt-dlp"))
        );
    }

    #[test]
    fn absent_or_empty_ytdl_hook_path_uses_adapter_path_fallback() {
        assert_eq!(
            ytdl_probe_executable_from_mpv_args(&["--fullscreen".to_owned()]),
            None
        );
        assert_eq!(
            ytdl_probe_executable_from_mpv_args(&[
                "--script-opts-append=ytdl_hook-ytdl_path=".to_owned(),
            ]),
            None
        );
    }
}
