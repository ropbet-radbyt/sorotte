use super::*;
use sorotte_player_mpv::managed_process::{ManagedMpvCommand, OwnedMpvProcess};

#[derive(Debug)]
pub(crate) struct ManagedMpvProcessGuard {
    child: OwnedMpvProcess,
}

struct FinishedClientRuntime {
    runtime: ClientApplication<MpvAdapter>,
    managed_guard: Option<ManagedMpvProcessGuard>,
    bridge_health: SorotteBridgeHealth,
    streaming_warning: Option<String>,
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
            MpvAdapter::disconnected_with_json_ipc_retry(ipc_path)
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

    let mut command = ManagedMpvCommand::new(&mpv_bin);
    if let Some(parent) = mpv_bin.parent() {
        command.current_dir(parent);
    }
    command.args(managed_mpv_launch_base_args_legacy_compatible(&ipc_path));
    if !config.extra_args.is_empty() {
        command.args(&config.extra_args);
    }
    let child = command.spawn(ipc_cleanup_path)?;
    let mut guard = ManagedMpvProcessGuard { child };
    let adapter = connect_managed_mpv_adapter_with_retry(
        &ipc_path,
        connect_timeout,
        connect_poll_interval,
        &mut guard.child,
    )
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

#[cfg(all(test, windows))]
pub(crate) fn connect_mpv_adapter_with_retry(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<MpvAdapter> {
    connect_mpv_adapter_with_retry_using(ipc_path, timeout, poll_interval, |path| {
        MpvAdapter::with_json_ipc(path)
    })
}

fn connect_managed_mpv_adapter_with_retry(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
    child: &mut OwnedMpvProcess,
) -> anyhow::Result<MpvAdapter> {
    connect_mpv_adapter_with_retry_using_and_health_check(
        ipc_path,
        timeout,
        poll_interval,
        |path| MpvAdapter::with_json_ipc(path),
        || match child.try_wait() {
            Ok(Some(status)) => Err(anyhow!(
                "managed mpv exited before JSON IPC became available (status={status})"
            )),
            Ok(None) => Ok(()),
            Err(error) => Err(anyhow!(
                "failed checking managed mpv process while waiting for JSON IPC: {error}"
            )),
        },
    )
}

#[cfg(test)]
fn connect_mpv_adapter_with_retry_using<F>(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
    connect: F,
) -> anyhow::Result<MpvAdapter>
where
    F: FnMut(&str) -> Result<MpvAdapter, PlayerError>,
{
    connect_mpv_adapter_with_retry_using_and_health_check(
        ipc_path,
        timeout,
        poll_interval,
        connect,
        || Ok(()),
    )
}

fn connect_mpv_adapter_with_retry_using_and_health_check<F, H>(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut connect: F,
    mut check_health: H,
) -> anyhow::Result<MpvAdapter>
where
    F: FnMut(&str) -> Result<MpvAdapter, PlayerError>,
    H: FnMut() -> anyhow::Result<()>,
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
                check_health()?;
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break;
                }
                std::thread::sleep(poll_interval.min(remaining));
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

#[cfg(test)]
mod process_supervision_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const MANAGED_PROCESS_FIXTURE_TEST: &str = "mpv_startup::managed_process::process_supervision_tests::managed_process_fixture_entrypoint";
    const MANAGED_PROCESS_FIXTURE_ROLE: &str = "SOROTTE_TEST_MANAGED_PROCESS_FIXTURE_ROLE";
    const MANAGED_PROCESS_FIXTURE_ROOT: &str = "SOROTTE_TEST_MANAGED_PROCESS_FIXTURE_ROOT";

    static PROCESS_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct ProcessFixtureDirectory {
        path: PathBuf,
    }

    impl ProcessFixtureDirectory {
        fn new(case: &str) -> Self {
            let sequence = PROCESS_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sorotte-cli-managed-process-{}-{sequence}-{case}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("process fixture directory should be created");
            Self { path }
        }

        fn marker(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for ProcessFixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn wait_for_process_marker(path: &Path, description: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {description}: {}",
                path.display()
            );
            std::thread::yield_now();
        }
    }

    fn spawn_managed_process_fixture(role: &str, fixture: &ProcessFixtureDirectory) -> Child {
        Command::new(std::env::current_exe().expect("current test executable should be available"))
            .args(["--exact", MANAGED_PROCESS_FIXTURE_TEST, "--nocapture"])
            .env(MANAGED_PROCESS_FIXTURE_ROLE, role)
            .env(MANAGED_PROCESS_FIXTURE_ROOT, &fixture.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("managed process fixture should spawn")
    }

    fn drop_guard_with_deadline(guard: ManagedMpvProcessGuard) {
        let (finished_tx, finished_rx) = std::sync::mpsc::sync_channel(1);
        let drop_thread = std::thread::spawn(move || {
            drop(guard);
            let _ = finished_tx.send(());
        });
        finished_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("managed process termination and reap should be bounded");
        drop_thread
            .join()
            .expect("managed process drop thread should not panic");
    }

    #[test]
    fn managed_process_fixture_entrypoint() {
        let Some(role) = std::env::var_os(MANAGED_PROCESS_FIXTURE_ROLE) else {
            return;
        };
        let root = PathBuf::from(
            std::env::var_os(MANAGED_PROCESS_FIXTURE_ROOT)
                .expect("process fixture root should accompany fixture role"),
        );
        match role.to_string_lossy().as_ref() {
            "wait-for-release" => {
                std::fs::write(root.join("child-started"), b"started")
                    .expect("managed child should publish its start barrier");
                wait_for_process_marker(&root.join("release-child"), "managed child release");
                std::fs::write(root.join("child-finished"), b"finished")
                    .expect("managed child should publish voluntary completion");
            }
            "exit-immediately" => {
                std::fs::write(root.join("child-started"), b"started")
                    .expect("early-exit child should publish its start barrier");
            }
            unexpected => panic!("unknown managed process fixture role: {unexpected}"),
        }
    }

    #[test]
    fn managed_guard_drop_kills_waits_reaps_and_removes_ipc_artifact() {
        let fixture = ProcessFixtureDirectory::new("guard-drop");
        let child = spawn_managed_process_fixture("wait-for-release", &fixture);
        wait_for_process_marker(&fixture.marker("child-started"), "managed child start");
        let ipc_artifact = fixture.marker("managed-ipc.sock");
        std::fs::write(&ipc_artifact, b"stale endpoint")
            .expect("test IPC artifact should be created");
        let guard = ManagedMpvProcessGuard {
            child: OwnedMpvProcess::from_test_child(child, Some(ipc_artifact.clone())).unwrap(),
        };

        assert!(
            !fixture.marker("child-finished").exists(),
            "managed child should remain owned and running before guard shutdown"
        );
        drop_guard_with_deadline(guard);

        assert!(
            !fixture.marker("child-finished").exists(),
            "managed shutdown should kill the child instead of releasing it voluntarily"
        );
        assert!(
            !ipc_artifact.exists(),
            "managed shutdown should remove the owned IPC artifact"
        );
    }

    #[test]
    fn managed_guard_cleanup_is_idempotent_after_child_already_exited() {
        let fixture = ProcessFixtureDirectory::new("early-exit-cleanup");
        let mut child = spawn_managed_process_fixture("exit-immediately", &fixture);
        wait_for_process_marker(&fixture.marker("child-started"), "early-exit child start");
        let status = child
            .wait()
            .expect("early-exit child should be deterministically reaped");
        assert!(status.success(), "fixture child should exit successfully");
        let ipc_artifact = fixture.marker("managed-ipc.sock");
        std::fs::write(&ipc_artifact, b"stale endpoint")
            .expect("test IPC artifact should be created");

        drop_guard_with_deadline(ManagedMpvProcessGuard {
            child: OwnedMpvProcess::from_test_child(child, Some(ipc_artifact.clone())).unwrap(),
        });

        assert!(
            !ipc_artifact.exists(),
            "cleanup should still remove IPC state after the child has exited"
        );
    }

    #[test]
    fn managed_launch_missing_binary_fails_before_process_ownership_is_created() {
        let fixture = ProcessFixtureDirectory::new("missing-binary");
        let missing_binary = fixture.marker("missing-mpv");
        let error = spawn_managed_mpv_and_attach(ManagedMpvLaunchEnvConfig {
            enabled: true,
            mpv_bin: Some(missing_binary.clone()),
            ipc_path: Some(fixture.marker("unused-ipc").to_string_lossy().into_owned()),
            ..Default::default()
        })
        .expect_err("missing managed mpv binary must fail")
        .to_string();

        assert!(
            error.contains("managed mpv binary does not exist"),
            "missing-binary failure should identify the validation boundary: {error}"
        );
        assert!(
            error.contains(missing_binary.to_string_lossy().as_ref()),
            "missing-binary failure should identify the requested program: {error}"
        );
    }

    #[test]
    fn managed_attach_stops_retrying_when_its_child_exits() {
        let fixture = ProcessFixtureDirectory::new("attach-after-child-exit");
        let ipc_path = fixture.marker("never-created-ipc");
        let timeout = Duration::from_secs(1);
        let error = spawn_managed_mpv_and_attach(ManagedMpvLaunchEnvConfig {
            enabled: true,
            mpv_bin: Some(
                std::env::current_exe().expect("current test executable should be available"),
            ),
            ipc_path: Some(ipc_path.to_string_lossy().into_owned()),
            connect_timeout_ms: Some(
                u32::try_from(timeout.as_millis()).expect("test timeout should fit u32"),
            ),
            connect_poll_interval_ms: Some(1),
            ..Default::default()
        })
        .expect_err("test harness child cannot create mpv JSON IPC")
        .to_string();
        assert!(
            error.contains("managed mpv launched but JSON IPC attach failed"),
            "failure should reach the managed attach boundary: {error}"
        );
        assert!(
            error.contains("managed mpv exited before JSON IPC became available")
                && error.contains("status="),
            "failure should identify the exited child and preserve its status: {error}"
        );
        assert!(
            !error.contains("timed out"),
            "managed attach must stop retrying as soon as its child exits: {error}"
        );
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
