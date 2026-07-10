use super::*;

const MANAGED_MPV_BUFFERING_DEFAULT_ARGS: &[&str] = &[
    "--cache-pause=yes",
    "--cache-pause-initial=yes",
    "--cache-pause-wait=5",
];

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
    let (mut player, managed_guard) =
        create_mpv_adapter_and_optional_managed_process_from_env(legacy_overrides)?;
    apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
        &mut player,
        stored_settings,
    )?;
    Ok((ClientApplication::new(session, player), managed_guard))
}

fn create_mpv_adapter_and_optional_managed_process_from_env(
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> anyhow::Result<(MpvAdapter, Option<ManagedMpvProcessGuard>)> {
    let explicit_ipc_path = explicit_mpv_ipc_path_from_env();
    if let Some(ipc_path) = explicit_ipc_path {
        return Ok((
            create_mpv_adapter_from_path_or_disconnected(&ipc_path),
            None,
        ));
    }

    let mut managed_config = managed_mpv_launch_env_config_from_env();
    apply_legacy_client_arg_managed_mpv_overrides(&mut managed_config, legacy_overrides);
    if !managed_config.enabled {
        #[cfg(test)]
        return Ok((SimulatedPlayer::new().into_inner(), None));
        #[cfg(not(test))]
        return Ok((MpvAdapter::default(), None));
    }

    let (adapter, guard) = spawn_managed_mpv_and_attach(managed_config)?;
    Ok((adapter, Some(guard)))
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
    if let Some(media_file) = config.media_file.as_ref() {
        command.arg(media_file);
    }

    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let guard = ManagedMpvProcessGuard {
        child,
        ipc_cleanup_path,
    };
    let adapter = connect_mpv_adapter_with_retry(&ipc_path, connect_timeout, connect_poll_interval)
        .map_err(|err| {
            anyhow!(
                "managed mpv launched but JSON IPC attach failed (mpv_bin={}, ipc={}): {err}",
                mpv_bin.display(),
                ipc_path
            )
        })?;

    eprintln!("info: started managed mpv and attached JSON IPC at '{ipc_path}'");
    Ok((adapter, guard))
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
    let mut args = vec![
        "--pause".to_owned(),
        "--force-window=no".to_owned(),
        "--idle=yes".to_owned(),
    ];
    args.extend(
        MANAGED_MPV_BUFFERING_DEFAULT_ARGS
            .iter()
            .map(|arg| (*arg).to_owned()),
    );
    args.push(format!("--input-ipc-server={ipc_path}"));
    args
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
