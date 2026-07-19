use super::*;

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
    flush_reconnect_notifications_legacy_compatible(runtime)?;
    let mut reconnect_delay = None;
    let mut stop_requested = false;
    runtime.drain_reconnect_intents(
        |delay_seconds| reconnect_delay = Some(delay_seconds),
        || stop_requested = true,
    );

    let plan =
        client_reconnect_backoff_plan_legacy_compatible(*retries, stop_requested, reconnect_delay);

    if plan.stop_retrying {
        return Ok(true);
    }

    let Some(delay_seconds) = plan.sleep_delay_seconds else {
        return Err(anyhow!(
            "active reconnect backoff plan did not include a sleep delay"
        ));
    };
    wait_with_player_integration_maintenance(runtime, Duration::from_secs_f64(delay_seconds)).await;
    *retries = plan.next_retries;
    Ok(false)
}

async fn run_client_network_loop_event_plan_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    retries: &mut u32,
    network_start: &Instant,
    plan: ClientNetworkLoopEventPlan,
) -> anyhow::Result<ClientNetworkLoopExecutionOutcome> {
    if plan.return_success {
        return Ok(client_network_loop_execution_outcome_legacy_compatible(
            plan, false,
        ));
    }
    if plan.run_disconnect {
        ensure_application_command_succeeded(runtime.dispatch(ClientCommand::Disconnect {
            now_seconds: network_start.elapsed().as_secs_f64(),
        }))?;
    }
    let reconnect_exhausted =
        plan.run_reconnect_backoff && run_reconnect_backoff(runtime, retries).await?;
    Ok(client_network_loop_execution_outcome_legacy_compatible(
        plan,
        reconnect_exhausted,
    ))
}

async fn run_client_network_loop_attempt_plan_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    retries: &mut u32,
    network_start: &Instant,
    plan: ClientNetworkLoopAttemptPlan,
) -> anyhow::Result<ClientNetworkLoopExecutionOutcome> {
    if plan.reset_retries_before_event {
        *retries = 0;
    }
    run_client_network_loop_event_plan_legacy_compatible(
        runtime,
        retries,
        network_start,
        plan.event,
    )
    .await
}

fn reconnect_exhausted_error_from_attempt_disposition_legacy_compatible(
    kind: ClientNetworkLoopReconnectExhaustedErrorKind,
    connect_error: Option<anyhow::Error>,
) -> anyhow::Error {
    match client_network_loop_reconnect_exhausted_error_action_legacy_compatible(kind) {
        ClientNetworkLoopReconnectExhaustedErrorAction::UseConnectError => connect_error
            .unwrap_or_else(|| {
                anyhow!("connect-failure exhaustion did not include the original connect error")
            }),
        ClientNetworkLoopReconnectExhaustedErrorAction::StaticMessage(message) => {
            anyhow!(message)
        }
    }
}

enum ClientNetworkLoopTransportAttemptOutcome {
    ReturnSuccess,
    Continue,
    ReconnectExhausted(anyhow::Error),
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

struct ClientNetworkLoopStartupExecutionPlan {
    diagnostics_config: ClientLoopDiagnosticsConfig,
    startup_plan: ClientNetworkLoopStartupPlan,
}

fn client_network_loop_startup_execution_plan_legacy_compatible(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> ClientNetworkLoopStartupExecutionPlan {
    ClientNetworkLoopStartupExecutionPlan {
        diagnostics_config: client_loop_diagnostics_config(legacy_overrides),
        startup_plan: client_network_loop_startup_plan_legacy_compatible(
            ClientNetworkLoopStartupPlanInputs {
                endpoint_host: &config.host,
                endpoint_port: config.port,
                stdin_enabled: env_flag_enabled("SOROTTE_CLIENT_STDIN"),
                has_legacy_overrides: legacy_overrides.is_some(),
                chat_message_on_connect: env_trimmed("SOROTTE_CLIENT_CHAT_MESSAGE").as_deref(),
                startup_playlist_file_on_connect,
            },
        ),
    }
}

fn bootstrap_client_network_loop_state_legacy_compatible<F, G>(
    config: &ClientLoopConfig,
    startup_plan: ClientNetworkLoopStartupPlan,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
    notification_sink: F,
    file_difference_sink: G,
) -> anyhow::Result<ClientNetworkLoopBootstrapState<F, G>>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ClientNetworkLoopStartupPlan {
        endpoint,
        spawn_local_input_receiver,
        apply_legacy_explicit_mpv_ipc_startup,
        chat_message_on_connect,
        startup_playlist_file_on_connect,
    } = startup_plan;
    let (mut runtime, managed_mpv_process_guard) =
        create_client_runtime_with_managed_mpv_support(config, legacy_overrides, stored_settings)?;
    let _ = runtime.dispatch(ClientCommand::Connect {
        endpoint: endpoint.clone(),
    });
    if apply_legacy_explicit_mpv_ipc_startup
        && let Some(overrides) = legacy_overrides
        && let Err(error) = runtime.with_player_io(|player| {
            apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
                player, overrides,
            )
        })
    {
        eprintln!("warning: failed legacy explicit-mpv-IPC startup file open: {error}");
    }
    Ok(ClientNetworkLoopBootstrapState {
        endpoint,
        retry_state: ClientNetworkLoopRetryState {
            runtime,
            chat_message_on_connect,
            startup_playlist_file_on_connect,
            local_input_rx: spawn_local_input_receiver
                .then(spawn_local_input_receiver_legacy_compatible),
            notification_sink,
            file_difference_sink,
            plex_config: cli_plex_config_from_env_and_stored_settings(stored_settings),
            retries: 0_u32,
        },
        _managed_mpv_process_guard: managed_mpv_process_guard,
    })
}

fn client_network_loop_transport_attempt_context_from_retry_state_legacy_compatible<'a, F, G>(
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
        },
        retries: &mut retry_state.retries,
        network_start,
    }
}

async fn run_client_network_loop_retry_loop_legacy_compatible<F, G>(
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
        match run_client_network_loop_transport_attempt_legacy_compatible(
            client_network_loop_transport_attempt_context_from_retry_state_legacy_compatible(
                &bootstrap.endpoint,
                config,
                diagnostics_config,
                network_start,
                &mut bootstrap.retry_state,
            ),
        )
        .await?
        {
            ClientNetworkLoopTransportAttemptOutcome::ReturnSuccess => return Ok(()),
            ClientNetworkLoopTransportAttemptOutcome::Continue => {}
            ClientNetworkLoopTransportAttemptOutcome::ReconnectExhausted(error) => {
                return Err(error);
            }
        }
    }
}

async fn run_client_network_loop_from_startup_execution_plan_legacy_compatible(
    config: &ClientLoopConfig,
    startup: ClientNetworkLoopStartupExecutionPlan,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
) -> anyhow::Result<()> {
    let ClientNetworkLoopStartupExecutionPlan {
        diagnostics_config,
        startup_plan,
    } = startup;
    let bootstrap = bootstrap_client_network_loop_state_legacy_compatible(
        config,
        startup_plan,
        legacy_overrides,
        stored_settings,
        emit_autoplay_countdown_notification,
        emit_file_difference_notification,
    )?;
    let network_start = Instant::now();
    run_client_network_loop_retry_loop_legacy_compatible(
        config,
        diagnostics_config,
        &network_start,
        bootstrap,
    )
    .await
}

async fn client_network_loop_transport_attempt_execution_plan_legacy_compatible<F, G>(
    endpoint: &str,
    launch: ConnectedSessionLaunchContext<'_, F, G>,
) -> anyhow::Result<(ClientNetworkLoopAttemptExecutionPlan, Option<anyhow::Error>)>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let connect_result =
        await_with_player_integration_maintenance(launch.runtime, TcpStream::connect(endpoint))
            .await;
    Ok(match connect_result {
        Ok(stream) => (
            client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible(
                run_connected_client_session_with_legacy_startup_overrides_and_diagnostics(
                    stream, launch,
                )
                .await?,
            ),
            None,
        ),
        Err(connect_err) => (
            client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible(),
            Some(connect_err.into()),
        ),
    })
}

async fn run_client_network_loop_transport_attempt_legacy_compatible<F, G>(
    attempt: ClientNetworkLoopTransportAttemptContext<'_, F, G>,
) -> anyhow::Result<ClientNetworkLoopTransportAttemptOutcome>
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
    } = launch;
    let (attempt_execution_plan, connect_error) =
        client_network_loop_transport_attempt_execution_plan_legacy_compatible(
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
            },
        )
        .await?;
    let attempt_disposition =
        client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible(
            attempt_execution_plan,
            run_client_network_loop_attempt_plan_legacy_compatible(
                runtime,
                retries,
                network_start,
                attempt_execution_plan.attempt_plan,
            )
            .await?,
        );
    Ok(match attempt_disposition {
        ClientNetworkLoopAttemptDisposition::ReturnSuccess => {
            ClientNetworkLoopTransportAttemptOutcome::ReturnSuccess
        }
        ClientNetworkLoopAttemptDisposition::Continue => {
            ClientNetworkLoopTransportAttemptOutcome::Continue
        }
        ClientNetworkLoopAttemptDisposition::ReconnectExhausted(kind) => {
            ClientNetworkLoopTransportAttemptOutcome::ReconnectExhausted(
                reconnect_exhausted_error_from_attempt_disposition_legacy_compatible(
                    kind,
                    connect_error,
                ),
            )
        }
    })
}

#[cfg(test)]
pub(crate) async fn run_client_network_loop(config: &ClientLoopConfig) -> anyhow::Result<()> {
    run_client_network_loop_with_legacy_startup_overrides(config, None, None).await
}

#[cfg(test)]
pub(crate) async fn run_client_network_loop_with_legacy_startup_overrides(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> anyhow::Result<()> {
    run_client_network_loop_with_legacy_startup_overrides_and_stored_settings(
        config,
        startup_playlist_file_on_connect,
        legacy_overrides,
        None,
    )
    .await
}

pub(crate) async fn run_client_network_loop_with_legacy_startup_overrides_and_stored_settings(
    config: &ClientLoopConfig,
    startup_playlist_file_on_connect: Option<&str>,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
    stored_settings: Option<&StoredClientSettingsMvp>,
) -> anyhow::Result<()> {
    run_client_network_loop_from_startup_execution_plan_legacy_compatible(
        config,
        client_network_loop_startup_execution_plan_legacy_compatible(
            config,
            startup_playlist_file_on_connect,
            legacy_overrides,
        ),
        legacy_overrides,
        stored_settings,
    )
    .await
}

#[cfg(test)]
mod shutdown_release_tests {
    use super::*;

    #[test]
    fn cli_external_player_shutdown_restores_osd_before_releasing_bridge() {
        let (player, commands) = MpvAdapter::with_cleanup_recording_sorotte_bridge_test_ipc(
            sorotte_player_mpv::LegacySyncplayUiSettings::default(),
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
        assert!(!runtime.player().legacy_syncplayintf_options_ready());
    }
}
