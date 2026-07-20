use super::*;

#[derive(Debug)]
pub(crate) struct ManagedMpvProcessGuard {
    child: Child,
    ipc_cleanup_path: Option<PathBuf>,
}

struct FinishedClientRuntime {
    runtime: ClientApplication<MpvAdapter>,
    managed_guard: Option<ManagedMpvProcessGuard>,
    bridge_health: SorotteBridgeHealth,
    streaming_warning: Option<String>,
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
    let (player, managed_guard, managed_startup_media) =
        create_mpv_adapter_and_optional_managed_process_from_env(legacy_overrides)?;
    let FinishedClientRuntime {
        runtime,
        managed_guard,
        bridge_health,
        streaming_warning,
    } = finish_client_runtime_with_mpv(
        config,
        legacy_overrides,
        stored_settings,
        player,
        managed_guard,
        managed_startup_media,
    )?;
    if let Some(warning) = streaming_warning {
        eprintln!("{warning}");
    }
    if let Some(warning) = sorotte_bridge_warning_line(&bridge_health) {
        eprintln!("{warning}");
    }
    Ok((runtime, managed_guard))
}

fn finish_client_runtime_with_mpv(
    config: &ClientLoopConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
    player: MpvAdapter,
    managed_guard: Option<ManagedMpvProcessGuard>,
    managed_startup_media: Option<String>,
) -> anyhow::Result<FinishedClientRuntime> {
    finish_client_runtime_with_mpv_and_bridge_setup(
        config,
        legacy_overrides,
        stored_settings,
        player,
        managed_guard,
        managed_startup_media,
        apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible,
    )
}

fn finish_client_runtime_with_mpv_and_bridge_setup<F>(
    config: &ClientLoopConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
    mut player: MpvAdapter,
    managed_guard: Option<ManagedMpvProcessGuard>,
    managed_startup_media: Option<String>,
    configure_bridge: F,
) -> anyhow::Result<FinishedClientRuntime>
where
    F: FnOnce(&mut MpvAdapter, Option<&StoredClientSettingsMvp>) -> SorotteBridgeHealth,
{
    let session = create_client_session(config);
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
    let streaming_warning = match player.apply_network_media_options_to_active_media() {
        Ok(()) => None,
        Err(error) if player.is_connected() => Some(active_network_options_warning_line(&error)),
        Err(error) => {
            return Err(anyhow!(
                "failed updating active mpv network-media options: {error}"
            ));
        }
    };
    if let Some(media) = managed_startup_media {
        player
            .open_file(&media)
            .map_err(|error| anyhow!("failed opening managed mpv startup media: {error}"))?;
    }
    let player_was_connected = player.is_connected();
    let bridge_health = configure_bridge(&mut player, stored_settings);
    if player_was_connected && !player.is_connected() {
        let detail = match &bridge_health {
            SorotteBridgeHealth::Degraded(failure) => failure.reason.as_str(),
            SorotteBridgeHealth::Disabled
            | SorotteBridgeHealth::Ready
            | SorotteBridgeHealth::Recovering => "the mpv JSON IPC transport became unhealthy",
        };
        return Err(anyhow!(
            "mpv JSON IPC became unavailable while configuring optional Chat/OSD integration: {detail}"
        ));
    }
    Ok(FinishedClientRuntime {
        runtime: ClientApplication::new(session, player),
        managed_guard,
        bridge_health,
        streaming_warning,
    })
}

fn active_network_options_warning_line(error: &PlayerError) -> String {
    format!(
        "warning: mpv playback is ready, but streaming options could not be applied to the active media: {error}; desired options will be used for future network loads"
    )
}

fn sorotte_bridge_warning_line(health: &SorotteBridgeHealth) -> Option<String> {
    let SorotteBridgeHealth::Degraded(failure) = health else {
        return None;
    };
    Some(format!(
        "warning: mpv is ready, but Chat/OSD integration could not be configured: {}",
        failure.reason
    ))
}

#[cfg(test)]
pub(crate) fn create_client_runtime_with_prepared_mpv_for_test(
    config: &ClientLoopConfig,
    stored_settings: Option<&StoredClientSettingsMvp>,
    player: MpvAdapter,
) -> anyhow::Result<(ClientApplication<MpvAdapter>, SorotteBridgeHealth)> {
    let FinishedClientRuntime {
        runtime,
        managed_guard,
        bridge_health,
        ..
    } = finish_client_runtime_with_mpv(config, None, stored_settings, player, None, None)?;
    debug_assert!(managed_guard.is_none());
    Ok((runtime, bridge_health))
}

#[cfg(test)]
pub(crate) fn create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test<F>(
    config: &ClientLoopConfig,
    stored_settings: Option<&StoredClientSettingsMvp>,
    player: MpvAdapter,
    configure_bridge: F,
) -> anyhow::Result<(ClientApplication<MpvAdapter>, SorotteBridgeHealth)>
where
    F: FnOnce(&mut MpvAdapter, Option<&StoredClientSettingsMvp>) -> SorotteBridgeHealth,
{
    let FinishedClientRuntime {
        runtime,
        managed_guard,
        bridge_health,
        ..
    } = finish_client_runtime_with_mpv_and_bridge_setup(
        config,
        None,
        stored_settings,
        player,
        None,
        None,
        configure_bridge,
    )?;
    debug_assert!(managed_guard.is_none());
    Ok((runtime, bridge_health))
}

#[cfg(test)]
pub(crate) fn create_client_runtime_with_prepared_mpv_and_startup_health_for_test<F>(
    config: &ClientLoopConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
    player: MpvAdapter,
    configure_bridge: F,
) -> anyhow::Result<(
    ClientApplication<MpvAdapter>,
    SorotteBridgeHealth,
    Option<String>,
)>
where
    F: FnOnce(&mut MpvAdapter, Option<&StoredClientSettingsMvp>) -> SorotteBridgeHealth,
{
    let FinishedClientRuntime {
        runtime,
        managed_guard,
        bridge_health,
        streaming_warning,
    } = finish_client_runtime_with_mpv_and_bridge_setup(
        config,
        legacy_overrides,
        stored_settings,
        player,
        None,
        None,
        configure_bridge,
    )?;
    debug_assert!(managed_guard.is_none());
    Ok((runtime, bridge_health, streaming_warning))
}

fn create_mpv_adapter_and_optional_managed_process_from_env(
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> anyhow::Result<(MpvAdapter, Option<ManagedMpvProcessGuard>, Option<String>)> {
    let explicit_ipc_path = explicit_mpv_ipc_path_from_env();
    if let Some(ipc_path) = explicit_ipc_path {
        return Ok((
            create_mpv_adapter_from_path_or_disconnected(&ipc_path),
            None,
            None,
        ));
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
            eprintln!("{}", explicit_mpv_ipc_connection_warning(ipc_path, &err));
            MpvAdapter::default()
        }
    }
}

fn explicit_mpv_ipc_connection_warning(ipc_path: &str, error: &PlayerError) -> String {
    format!(
        "warning: failed to connect mpv JSON IPC at '{ipc_path}': {error}; player is disconnected"
    )
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
    connect_mpv_adapter_with_retry_using(ipc_path, timeout, poll_interval, |path| {
        MpvAdapter::with_json_ipc(path)
    })
}

fn connect_mpv_adapter_with_retry_using<F>(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut connect: F,
) -> anyhow::Result<MpvAdapter>
where
    F: FnMut(&str) -> Result<MpvAdapter, PlayerError>,
{
    let started = std::time::Instant::now();
    let mut last_error = None;
    while started.elapsed() < timeout {
        match connect(ipc_path) {
            Ok(adapter) => return Ok(adapter),
            Err(error) if sorotte_player_mpv::is_unsupported_mpv_version_error(&error) => {
                return Err(anyhow!(error));
            }
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

#[cfg(test)]
mod version_requirement_tests {
    use super::*;

    #[test]
    fn managed_attach_fails_fast_with_clear_mpv_upgrade_guidance() {
        let mut attempts = 0;
        let result = connect_mpv_adapter_with_retry_using(
            "test-mpv-ipc",
            Duration::from_secs(5),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Err(PlayerError::OperationFailed(format!(
                    "Sorotte requires mpv {} or newer; upgrade mpv and try again",
                    sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
                )))
            },
        );

        let error = match result {
            Ok(_) => panic!("an unsupported mpv version must be rejected"),
            Err(error) => error.to_string(),
        };
        assert_eq!(
            attempts, 1,
            "a permanent version failure must not be retried"
        );
        assert!(error.contains(&format!(
            "requires mpv {} or newer",
            sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
        )));
        assert!(error.contains("upgrade mpv"));
        assert!(!error.contains("timed out"));
    }

    #[test]
    fn explicit_attach_warning_preserves_mpv_upgrade_guidance() {
        let error = PlayerError::OperationFailed(format!(
            "Sorotte requires mpv {} or newer; upgrade mpv and try again",
            sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
        ));

        let warning = explicit_mpv_ipc_connection_warning("test-mpv-ipc", &error);

        assert!(warning.starts_with("warning: failed to connect mpv JSON IPC"));
        assert!(warning.contains(&format!(
            "requires mpv {} or newer",
            sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
        )));
        assert!(warning.contains("upgrade mpv"));
        assert!(warning.ends_with("player is disconnected"));
    }

    #[test]
    fn managed_attach_still_retries_transient_connection_failures() {
        let mut attempts = 0;
        let result = connect_mpv_adapter_with_retry_using(
            "test-mpv-ipc",
            Duration::from_secs(5),
            Duration::ZERO,
            |_| {
                attempts += 1;
                if attempts == 1 {
                    Err(PlayerError::OperationFailed(
                        "mpv endpoint is still starting".to_owned(),
                    ))
                } else {
                    Ok(MpvAdapter::default())
                }
            },
        );

        assert!(result.is_ok());
        assert_eq!(attempts, 2);
    }
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
mod startup_health_tests {
    use super::*;

    #[test]
    fn degraded_bridge_health_produces_a_scoped_nonfatal_warning() {
        let health = SorotteBridgeHealth::Degraded(sorotte_player_mpv::SorotteBridgeFailure {
            kind: sorotte_player_mpv::SorotteBridgeFailureKind::AcknowledgementTimeout,
            reason: "settings acknowledgement timed out".to_owned(),
        });

        assert_eq!(
            sorotte_bridge_warning_line(&health).as_deref(),
            Some(
                "warning: mpv is ready, but Chat/OSD integration could not be configured: settings acknowledgement timed out"
            )
        );
        assert!(sorotte_bridge_warning_line(&SorotteBridgeHealth::Ready).is_none());
        assert!(sorotte_bridge_warning_line(&SorotteBridgeHealth::Disabled).is_none());
    }
}
