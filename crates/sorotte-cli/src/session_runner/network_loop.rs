use super::*;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

fn cli_connect_timeout() -> Duration {
    env_non_negative_f64("SOROTTE_CLIENT_CONNECT_TIMEOUT_SECONDS")
        .filter(|seconds| *seconds > 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT)
}

#[cfg(test)]
mod connect_timeout_overflow_regression {
    use super::*;
    use std::{ffi::OsString, sync::Mutex};

    const KEY: &str = "SOROTTE_CLIENT_CONNECT_TIMEOUT_SECONDS";
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct RestoreEnv(Option<OsString>);

    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            // SAFETY: every mutation of this key in the module is serialized
            // by ENV_LOCK, which remains held until this restore guard drops.
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var(KEY, value),
                    None => std::env::remove_var(KEY),
                }
            }
        }
    }

    #[test]
    fn extreme_finite_connect_timeout_falls_back_without_panicking() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let _restore = RestoreEnv(std::env::var_os(KEY));
        // SAFETY: ENV_LOCK serializes this process-global mutation, and
        // RestoreEnv restores the prior value before the lock is released.
        unsafe {
            std::env::set_var(KEY, f64::MAX.to_string());
        }

        assert_eq!(cli_connect_timeout(), DEFAULT_CONNECT_TIMEOUT);
    }
}

fn ensure_application_command_succeeded(events: Vec<ClientEvent>) -> anyhow::Result<()> {
    if let Some(ClientEvent::OperationFailed { message, .. }) = events
        .into_iter()
        .find(|event| matches!(event, ClientEvent::OperationFailed { .. }))
    {
        return Err(anyhow!(message));
    }
    Ok(())
}

async fn wait_with_player_integration_maintenance(
    runtime: &mut ClientApplication<MpvAdapter>,
    duration: Duration,
) {
    await_with_player_integration_maintenance(runtime, tokio::time::sleep(duration)).await;
}

async fn run_reconnect_backoff(
    runtime: &mut ClientApplication<MpvAdapter>,
    retries: &mut u32,
) -> anyhow::Result<bool> {
    let _ = runtime.dispatch(ClientCommand::Reconnect { attempt: *retries });
    runtime.run_reconnect_retry(*retries)?;
    flush_reconnect_notifications(runtime, &mut emit_reconnect_transition_notification)?;
    let mut reconnect_delay = None;
    let mut stop_requested = false;
    runtime.drain_reconnect_intents(
        |delay_seconds| reconnect_delay = Some(delay_seconds),
        || stop_requested = true,
    );

    if stop_requested {
        return Ok(true);
    }
    let delay_seconds = reconnect_delay.unwrap_or(0.1);
    wait_with_player_integration_maintenance(runtime, Duration::from_secs_f64(delay_seconds)).await;
    *retries = retries.saturating_add(1);
    Ok(false)
}

#[derive(Debug)]
enum ConnectionAttemptOutcome {
    ConnectFailed(anyhow::Error),
    SessionFailed(anyhow::Error),
    TransportClosed,
    RuntimeWindowElapsed,
}

async fn finish_connection_attempt(
    runtime: &mut ClientApplication<MpvAdapter>,
    retries: &mut u32,
    network_start: &Instant,
    outcome: ConnectionAttemptOutcome,
) -> anyhow::Result<ControlFlow<()>> {
    let error = match outcome {
        ConnectionAttemptOutcome::RuntimeWindowElapsed => {
            *retries = 0;
            return Ok(ControlFlow::Break(()));
        }
        ConnectionAttemptOutcome::TransportClosed => {
            *retries = 0;
            ensure_application_command_succeeded(runtime.dispatch(ClientCommand::Disconnect {
                now_seconds: network_start.elapsed().as_secs_f64(),
            }))?;
            anyhow!("server connection closed and reconnect retries were exhausted")
        }
        ConnectionAttemptOutcome::SessionFailed(error) => {
            emit_application_service_events(runtime.shutdown_plex_service().await);
            ensure_application_command_succeeded(runtime.dispatch(ClientCommand::Disconnect {
                now_seconds: network_start.elapsed().as_secs_f64(),
            }))?;
            error
        }
        ConnectionAttemptOutcome::ConnectFailed(error) => error,
    };
    if run_reconnect_backoff(runtime, retries).await? {
        Err(error)
    } else {
        Ok(ControlFlow::Continue(()))
    }
}

struct ClientNetworkLoopTransportAttemptContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    endpoint: &'a str,
    launch: ConnectedSessionLaunchContext<'a, F, G>,
    retries: &'a mut u32,
    network_start: &'a Instant,
}

struct ClientNetworkLoopRetryState<F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    runtime: ClientApplication<MpvAdapter>,
    chat_message_on_connect: Option<String>,
    startup_playlist_file_on_connect: Option<String>,
    local_input_rx: Option<UnboundedReceiver<String>>,
    notification_sink: F,
    file_difference_sink: G,
    plex_config: PlexClientConfig,
    network_options_health_reporter: CliNetworkOptionsHealthReporter,
    tls_policy_override: Option<TlsPolicy>,
    retries: u32,
}

struct ClientNetworkLoopBootstrapState<F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    endpoint: String,
    retry_state: ClientNetworkLoopRetryState<F, G>,
    _managed_mpv_process_guard: Option<ManagedMpvProcessGuard>,
}

fn release_cli_runtime_sorotte_bridge_best_effort(runtime: &mut ClientApplication<MpvAdapter>) {
    runtime.with_player_io(MpvAdapter::release_sorotte_bridge_best_effort);
}

impl<F, G> Drop for ClientNetworkLoopBootstrapState<F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    fn drop(&mut self) {
        release_cli_runtime_sorotte_bridge_best_effort(&mut self.retry_state.runtime);
    }
}

fn bootstrap_client_network_loop_state<F, G>(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    argument_overrides: Option<&SyncplayClientArgOverrides>,
    stored_settings: Option<&StoredClientSettings>,
    notification_sink: F,
    file_difference_sink: G,
) -> anyhow::Result<ClientNetworkLoopBootstrapState<F, G>>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let endpoint = format!("{}:{}", config.host, config.port);
    let stdin_enabled = env_flag_enabled("SOROTTE_CLIENT_STDIN");
    let chat_message_on_connect = env_trimmed("SOROTTE_CLIENT_CHAT_MESSAGE");
    let startup_playlist_file_on_connect = startup_playlist_file_on_connect.map(str::to_owned);
    let (mut runtime, managed_mpv_process_guard) = create_client_runtime_with_managed_mpv_support(
        config,
        argument_overrides,
        stored_settings,
    )?;
    let _ = runtime.dispatch(ClientCommand::Connect {
        endpoint: endpoint.clone(),
    });
    if let Some(overrides) = argument_overrides
        && let Err(error) = runtime.with_player_io(|player| {
            apply_startup_file_to_attached_player_if_explicit_mpv_ipc(player, overrides)
        })
    {
        eprintln!("warning: failed explicit-mpv-IPC startup file open: {error}");
    }
    Ok(ClientNetworkLoopBootstrapState {
        endpoint,
        retry_state: ClientNetworkLoopRetryState {
            runtime,
            chat_message_on_connect,
            startup_playlist_file_on_connect,
            local_input_rx: stdin_enabled.then(crate::stdin_input::spawn_local_input_receiver),
            notification_sink,
            file_difference_sink,
            plex_config: cli_plex_config_from_env_and_stored_settings(stored_settings),
            network_options_health_reporter: CliNetworkOptionsHealthReporter::default(),
            tls_policy_override: stored_settings
                .and_then(|settings| settings.tls_policy.as_deref())
                .and_then(TlsPolicy::parse),
            retries: 0_u32,
        },
        _managed_mpv_process_guard: managed_mpv_process_guard,
    })
}

fn client_network_loop_transport_attempt_context_from_retry_state<'a, F, G>(
    endpoint: &'a str,
    config: &'a ClientLoopConfig,
    diagnostics_config: ClientLoopDiagnosticsConfig,
    network_start: &'a Instant,
    retry_state: &'a mut ClientNetworkLoopRetryState<F, G>,
) -> ClientNetworkLoopTransportAttemptContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    ClientNetworkLoopTransportAttemptContext {
        endpoint,
        launch: ConnectedSessionLaunchContext {
            runtime: &mut retry_state.runtime,
            config,
            chat_message_on_connect: retry_state.chat_message_on_connect.as_deref(),
            startup_playlist_file_on_connect: &mut retry_state.startup_playlist_file_on_connect,
            local_input_rx: retry_state.local_input_rx.as_mut(),
            notification_sink: &mut retry_state.notification_sink,
            file_difference_sink: &mut retry_state.file_difference_sink,
            diagnostics_config,
            plex_config: &retry_state.plex_config,
            network_options_health_reporter: &mut retry_state.network_options_health_reporter,
            tls_policy_override: retry_state.tls_policy_override,
        },
        retries: &mut retry_state.retries,
        network_start,
    }
}

async fn run_client_network_loop_retry_loop<F, G>(
    config: &ClientLoopConfig,
    diagnostics_config: ClientLoopDiagnosticsConfig,
    network_start: &Instant,
    mut bootstrap: ClientNetworkLoopBootstrapState<F, G>,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    loop {
        match run_client_network_loop_transport_attempt(
            client_network_loop_transport_attempt_context_from_retry_state(
                &bootstrap.endpoint,
                config,
                diagnostics_config,
                network_start,
                &mut bootstrap.retry_state,
            ),
        )
        .await?
        {
            ControlFlow::Break(()) => return Ok(()),
            ControlFlow::Continue(()) => {}
        }
    }
}

async fn connect_and_run_session<F, G>(
    endpoint: &str,
    launch: ConnectedSessionLaunchContext<'_, F, G>,
) -> ConnectionAttemptOutcome
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let connect_timeout = cli_connect_timeout();
    let connect_result = await_with_player_integration_maintenance(
        launch.runtime,
        tokio::time::timeout(connect_timeout, TcpStream::connect(endpoint)),
    )
    .await;
    match connect_result {
        Ok(Ok(stream)) => {
            match run_connected_client_session_with_startup_overrides_and_diagnostics(
                stream, launch,
            )
            .await
            {
                Ok(ConnectedSessionExit::TransportClosed) => {
                    ConnectionAttemptOutcome::TransportClosed
                }
                Ok(ConnectedSessionExit::RuntimeWindowElapsed) => {
                    ConnectionAttemptOutcome::RuntimeWindowElapsed
                }
                Err(error) => ConnectionAttemptOutcome::SessionFailed(error),
            }
        }
        Ok(Err(error)) => ConnectionAttemptOutcome::ConnectFailed(error.into()),
        Err(_) => ConnectionAttemptOutcome::ConnectFailed(anyhow!(
            "TCP connection to {endpoint} timed out after {:.1} seconds",
            connect_timeout.as_secs_f64()
        )),
    }
}

async fn run_client_network_loop_transport_attempt<F, G>(
    attempt: ClientNetworkLoopTransportAttemptContext<'_, F, G>,
) -> anyhow::Result<ControlFlow<()>>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ClientNetworkLoopTransportAttemptContext {
        endpoint,
        launch,
        retries,
        network_start,
    } = attempt;
    let ConnectedSessionLaunchContext {
        runtime,
        config,
        chat_message_on_connect,
        startup_playlist_file_on_connect,
        local_input_rx,
        notification_sink,
        file_difference_sink,
        diagnostics_config,
        plex_config,
        network_options_health_reporter,
        tls_policy_override,
    } = launch;
    let attempt_result = connect_and_run_session(
        endpoint,
        ConnectedSessionLaunchContext {
            runtime: &mut *runtime,
            config,
            chat_message_on_connect,
            startup_playlist_file_on_connect,
            local_input_rx,
            notification_sink,
            file_difference_sink,
            diagnostics_config,
            plex_config,
            network_options_health_reporter,
            tls_policy_override,
        },
    )
    .await;
    finish_connection_attempt(runtime, retries, network_start, attempt_result).await
}

#[cfg(test)]
pub(crate) async fn run_client_network_loop(config: &ClientLoopConfig) -> anyhow::Result<()> {
    run_client_network_loop_with_startup_overrides(config, None, None).await
}

/// Runs the production retry/connected-session loop around a caller-prepared
/// runtime and returns that same runtime after a normal outer-loop exit.
///
/// Lifecycle system seams need to preload deterministic player evidence,
/// force a real socket failure, and then inspect the surviving player/session
/// owner. Keeping this helper test-only avoids a second reconnect
/// implementation while preserving production construction and shutdown APIs.
#[cfg(test)]
pub(crate) async fn run_client_network_loop_with_prepared_runtime_for_test(
    config: &ClientLoopConfig,
    mut runtime: ClientApplication<MpvAdapter>,
    local_input_rx: Option<UnboundedReceiver<String>>,
) -> anyhow::Result<ClientApplication<MpvAdapter>> {
    let endpoint = format!("{}:{}", config.host, config.port);
    let _ = runtime.dispatch(ClientCommand::Connect {
        endpoint: endpoint.clone(),
    });
    let diagnostics_config = client_loop_diagnostics_config(None);
    let network_start = Instant::now();
    let mut retry_state = ClientNetworkLoopRetryState {
        runtime,
        chat_message_on_connect: None,
        startup_playlist_file_on_connect: None,
        local_input_rx,
        notification_sink: |_notification: &AutoplayCountdownNotification| Ok(()),
        file_difference_sink: |_summary: &str| Ok(()),
        plex_config: cli_plex_config_from_env_and_stored_settings(None),
        network_options_health_reporter: CliNetworkOptionsHealthReporter::default(),
        tls_policy_override: None,
        retries: 0,
    };

    loop {
        match run_client_network_loop_transport_attempt(
            client_network_loop_transport_attempt_context_from_retry_state(
                &endpoint,
                config,
                diagnostics_config,
                &network_start,
                &mut retry_state,
            ),
        )
        .await?
        {
            ControlFlow::Break(()) => {
                return Ok(retry_state.runtime);
            }
            ControlFlow::Continue(()) => {}
        }
    }
}

#[cfg(test)]
pub(crate) async fn run_client_network_loop_with_startup_overrides(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    argument_overrides: Option<&SyncplayClientArgOverrides>,
) -> anyhow::Result<()> {
    run_client_network_loop_with_startup_overrides_and_stored_settings(
        config,
        startup_playlist_file_on_connect,
        argument_overrides,
        None,
    )
    .await
}

pub(crate) async fn run_client_network_loop_with_startup_overrides_and_stored_settings(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    argument_overrides: Option<&SyncplayClientArgOverrides>,
    stored_settings: Option<&StoredClientSettings>,
) -> anyhow::Result<()> {
    let diagnostics_config = client_loop_diagnostics_config(argument_overrides);
    let bootstrap = bootstrap_client_network_loop_state(
        config,
        startup_playlist_file_on_connect,
        argument_overrides,
        stored_settings,
        emit_autoplay_countdown_notification,
        emit_file_difference_notification,
    )?;
    let network_start = Instant::now();
    run_client_network_loop_retry_loop(config, diagnostics_config, &network_start, bootstrap).await
}

#[cfg(test)]
mod deterministic_reconnect_time_tests {
    use super::*;
    use crate::{client_config::create_client_runtime, tests::test_client_loop_config};

    #[tokio::test(start_paused = true)]
    async fn runtime_window_completion_preserves_connection_and_does_not_back_off() {
        let config = test_client_loop_config();
        let mut runtime = create_client_runtime(&config);
        runtime.dispatch(ClientCommand::Connect {
            endpoint: "fixture:8999".to_owned(),
        });
        let before_phase = runtime.connection_phase().clone();
        let mut retries = 7;
        let started_at = Instant::now();
        let control = finish_connection_attempt(
            &mut runtime,
            &mut retries,
            &started_at,
            ConnectionAttemptOutcome::RuntimeWindowElapsed,
        )
        .await
        .unwrap();
        assert!(control.is_break());
        assert_eq!(retries, 0);
        assert_eq!(runtime.connection_phase(), &before_phase);
        assert_eq!(Instant::now().duration_since(started_at), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn transport_close_resets_failed_attempts_before_scheduling_reconnect() {
        let mut config = test_client_loop_config();
        config.max_retries = 0;
        let mut runtime = create_client_runtime(&config);
        let mut retries = 7;
        let started_at = Instant::now();
        let control = finish_connection_attempt(
            &mut runtime,
            &mut retries,
            &started_at,
            ConnectionAttemptOutcome::TransportClosed,
        )
        .await
        .unwrap();
        assert!(
            control.is_continue(),
            "a completed session starts a fresh retry budget"
        );
        assert_eq!(retries, 1);
        assert_eq!(
            Instant::now().duration_since(started_at),
            Duration::from_millis(100)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn connection_and_session_failures_preserve_original_error_and_retry_budget() {
        for session_failure in [false, true] {
            let mut config = test_client_loop_config();
            config.max_retries = 0;
            let mut runtime = create_client_runtime(&config);
            let mut retries = 1;
            let started_at = Instant::now();
            let original = std::io::Error::other("original attempt failure");
            let outcome = if session_failure {
                ConnectionAttemptOutcome::SessionFailed(original.into())
            } else {
                ConnectionAttemptOutcome::ConnectFailed(original.into())
            };
            let error = finish_connection_attempt(&mut runtime, &mut retries, &started_at, outcome)
                .await
                .expect_err("the existing retry budget must remain exhausted");
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().to_string(),
                "original attempt failure"
            );
            assert_eq!(retries, 1);
            assert_eq!(Instant::now().duration_since(started_at), Duration::ZERO);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_backoff_uses_exact_exponential_delays_and_no_terminal_sleep() {
        let mut config = test_client_loop_config();
        config.max_retries = 2;
        let mut runtime = create_client_runtime(&config);
        let mut retries = 0;
        let started_at = Instant::now();

        for (expected_retries, expected_elapsed) in [
            (1, Duration::from_millis(100)),
            (2, Duration::from_millis(300)),
            (3, Duration::from_millis(700)),
        ] {
            assert!(
                !run_reconnect_backoff(&mut runtime, &mut retries)
                    .await
                    .expect("an allowed reconnect attempt should schedule"),
                "attempt {expected_retries} should remain below the retry limit"
            );
            assert_eq!(retries, expected_retries);
            assert_eq!(
                Instant::now().duration_since(started_at),
                expected_elapsed,
                "reconnect attempt {expected_retries} should advance only its declared backoff"
            );
        }

        let elapsed_before_exhaustion = Instant::now().duration_since(started_at);
        assert!(
            run_reconnect_backoff(&mut runtime, &mut retries)
                .await
                .expect("retry exhaustion should be a normal scheduler outcome")
        );
        assert_eq!(retries, 3, "terminal evaluation must not invent an attempt");
        assert_eq!(
            Instant::now().duration_since(started_at),
            elapsed_before_exhaustion,
            "retry exhaustion must not add an unobservable terminal delay"
        );
    }
}

#[cfg(test)]
mod shutdown_release_tests {
    use super::*;

    #[test]
    fn cli_external_player_shutdown_restores_osd_before_releasing_bridge() {
        let (player, commands) = MpvAdapter::with_cleanup_recording_sorotte_bridge_test_ipc(
            sorotte_player_mpv::SyncplayUiSettings::default(),
            Some(("top".to_owned(), 16)),
        );
        assert_eq!(
            player.sorotte_bridge_health(),
            sorotte_player_mpv::SorotteBridgeHealth::Ready,
        );
        let mut runtime = ClientApplication::with_default_session(player);

        release_cli_runtime_sorotte_bridge_best_effort(&mut runtime);

        let commands = commands
            .lock()
            .expect("cleanup command log should not be poisoned")
            .clone();
        assert_eq!(commands.len(), 3, "CLI cleanup should queue three commands");
        assert_eq!(
            commands[0],
            serde_json::json!(["set_property", "osd-align-y", "top"])
        );
        assert_eq!(
            commands[1],
            serde_json::json!(["set_property", "osd-margin-y", 16])
        );
        assert_eq!(commands[2][2], "sorotte_syncplayintf_release");

        assert_eq!(
            runtime.player().sorotte_bridge_health(),
            sorotte_player_mpv::SorotteBridgeHealth::Disabled
        );
        assert!(!runtime.player().syncplayintf_options_ready());
    }
}
