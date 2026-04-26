use super::*;

mod execution;

use self::execution::{
    ConnectedSessionBranchExecutionContext, ConnectedSessionEventExecutionContext,
    run_connected_session_event_plan_legacy_compatible,
};

#[cfg(test)]
pub(crate) async fn run_connected_client_session<F, G>(
    stream: TcpStream,
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
    chat_message_on_connect: Option<&str>,
    local_input_rx: Option<&mut UnboundedReceiver<String>>,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
) -> anyhow::Result<ConnectedSessionExit>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let mut no_playlist = None;
    let exit = run_connected_client_session_with_legacy_startup_overrides(
        stream,
        runtime,
        config,
        chat_message_on_connect,
        &mut no_playlist,
        local_input_rx,
        notification_sink,
        file_difference_sink,
    )
    .await?;
    Ok(exit)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_connected_client_session_with_legacy_startup_overrides<F, G>(
    stream: TcpStream,
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
    chat_message_on_connect: Option<&str>,
    startup_playlist_file_on_connect: &mut Option<String>,
    local_input_rx: Option<&mut UnboundedReceiver<String>>,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
) -> anyhow::Result<ConnectedSessionExit>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let diagnostics_config = client_loop_diagnostics_config(None);
    run_connected_client_session_with_legacy_startup_overrides_and_diagnostics(
        stream,
        ConnectedSessionLaunchContext {
            runtime,
            config,
            chat_message_on_connect,
            startup_playlist_file_on_connect,
            local_input_rx,
            notification_sink,
            file_difference_sink,
            diagnostics_config,
        },
    )
    .await
}

pub(crate) struct ConnectedSessionLaunchContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    pub(crate) runtime: &'a mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    pub(crate) config: &'a ClientLoopConfig,
    pub(crate) chat_message_on_connect: Option<&'a str>,
    pub(crate) startup_playlist_file_on_connect: &'a mut Option<String>,
    pub(crate) local_input_rx: Option<&'a mut UnboundedReceiver<String>>,
    pub(crate) notification_sink: &'a mut F,
    pub(crate) file_difference_sink: &'a mut G,
    pub(crate) diagnostics_config: ClientLoopDiagnosticsConfig,
}

pub(crate) async fn run_connected_client_session_with_legacy_startup_overrides_and_diagnostics<
    F,
    G,
>(
    stream: TcpStream,
    launch: ConnectedSessionLaunchContext<'_, F, G>,
) -> anyhow::Result<ConnectedSessionExit>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ConnectedSessionLaunchContext {
        runtime,
        config,
        chat_message_on_connect,
        startup_playlist_file_on_connect,
        local_input_rx,
        notification_sink,
        file_difference_sink,
        diagnostics_config,
    } = launch;
    let mut local_input_rx = local_input_rx;
    let mut hello_payload = HelloPayload::new(
        config.username.clone(),
        config.room.clone(),
        config.version.clone(),
    )
    .with_realversion(SYNCPLAY_COMPAT_VERSION_LEGACY);
    if let Some(server_password) = config.server_password.as_deref()
        && !server_password.is_empty()
    {
        hello_payload.extra.insert(
            "password".to_owned(),
            Value::String(legacy_server_password_token(server_password)),
        );
    }
    hello_payload.features = Some(client_hello_features_legacy_compatible(config));
    let hello_message = ProtocolMessage::hello(hello_payload);
    runtime
        .session_mut()
        .apply_protocol_message(hello_message.clone())?;
    runtime.session_mut().clear_server_feature_support_state();

    let hello_line = encode_message_line(&hello_message)?;
    let (reader, mut writer) = stream.into_split();
    write_protocol_line(&mut writer, &hello_line).await?;
    let mut pending_chat_message_on_connect = chat_message_on_connect.map(str::to_owned);
    publish_pending_local_file_updates(runtime, config)?;
    flush_runtime_protocol_lines(runtime, &mut writer).await?;

    let mut reader = BufReader::new(reader).lines();
    let connected_start = Instant::now();
    let mut autoplay_tick =
        tokio::time::interval(Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS));
    let mut player_chat_input_tick =
        tokio::time::interval(Duration::from_millis(PLAYER_CHAT_INPUT_POLL_INTERVAL_MS));
    let mut file_difference_state = FileDifferenceNotificationState::default();
    let mut reconnect_correction_diagnostics_state = ReconnectCorrectionDiagnosticsState::default();
    let mut local_user_offset_seconds = 0.0f64;
    let mut pending_ready_at_start_on_server_hello =
        config.ready_at_start_override.unwrap_or(false);
    let mut outbound_state_sync_enabled = false;
    let branch_diagnostics_plan = ConnectedSessionDiagnosticsPlan {
        log_player_telemetry: diagnostics_config.log_player_telemetry,
        log_player_drift: diagnostics_config.log_player_drift,
        reconnect_correction_diagnostics_format: diagnostics_config
            .reconnect_correction_diagnostics_format,
    };
    let shared_playlists_enabled = shared_playlists_enabled_cli_legacy_compatible(config);
    let dont_slow_down_with_me = config.dont_slow_down_with_me_override.unwrap_or(false);

    loop {
        if connected_start.elapsed().as_secs_f64() >= config.max_connected_runtime_seconds {
            return Ok(ConnectedSessionExit::RuntimeWindowElapsed);
        }

        tokio::select! {
            line = reader.next_line() => {
                match line? {
                    Some(line) => {
                        let decoded_inbound_message = decode_message_line(&line).ok();
                        let inbound_is_server_hello = pending_ready_at_start_on_server_hello
                            && matches!(
                                decoded_inbound_message.as_ref(),
                                Some(ProtocolMessage::Hello(_))
                            );
                        let now_seconds = connected_start.elapsed().as_secs_f64();
                        let event_execution_plan =
                            connected_session_inbound_message_event_execution_plan_legacy_compatible(
                                inbound_is_server_hello,
                                pending_chat_message_on_connect.is_some(),
                                matches!(
                                    decoded_inbound_message.as_ref(),
                                    Some(ProtocolMessage::State(_))
                                ),
                                ConnectedSessionSharedExecutionInputs {
                                    shared_playlists_enabled,
                                    diagnostics: branch_diagnostics_plan,
                                    outbound_state_sync_enabled,
                                },
                            );
                        run_connected_session_event_plan_legacy_compatible(
                            runtime,
                            Some(&line),
                            decoded_inbound_message.as_ref(),
                            now_seconds,
                            dont_slow_down_with_me,
                            event_execution_plan,
                            ConnectedSessionEventExecutionContext {
                                pending_ready_at_start_on_server_hello: &mut pending_ready_at_start_on_server_hello,
                                pending_chat_message_on_connect: &mut pending_chat_message_on_connect,
                                outbound_state_sync_enabled: &mut outbound_state_sync_enabled,
                                branch: ConnectedSessionBranchExecutionContext {
                                    config,
                                    writer: &mut writer,
                                    startup_playlist_file_on_connect,
                                    diagnostics_config: &diagnostics_config,
                                    reconnect_correction_diagnostics_state: &mut reconnect_correction_diagnostics_state,
                                    file_difference_state: &mut file_difference_state,
                                    notification_sink,
                                    file_difference_sink,
                                },
                            },
                        )
                        .await?;
                    }
                    None => return Ok(ConnectedSessionExit::TransportClosed),
                }
            }
            _ = autoplay_tick.tick() => {
                let now_seconds = connected_start.elapsed().as_secs_f64();
                let event_execution_plan =
                    connected_session_autoplay_tick_event_execution_plan_legacy_compatible(
                        ConnectedSessionSharedExecutionInputs {
                            shared_playlists_enabled,
                            diagnostics: branch_diagnostics_plan,
                            outbound_state_sync_enabled,
                        },
                    );
                run_connected_session_event_plan_legacy_compatible(
                    runtime,
                    None,
                    None,
                    now_seconds,
                    dont_slow_down_with_me,
                    event_execution_plan,
                    ConnectedSessionEventExecutionContext {
                        pending_ready_at_start_on_server_hello: &mut pending_ready_at_start_on_server_hello,
                        pending_chat_message_on_connect: &mut pending_chat_message_on_connect,
                        outbound_state_sync_enabled: &mut outbound_state_sync_enabled,
                        branch: ConnectedSessionBranchExecutionContext {
                            config,
                            writer: &mut writer,
                            startup_playlist_file_on_connect,
                            diagnostics_config: &diagnostics_config,
                            reconnect_correction_diagnostics_state: &mut reconnect_correction_diagnostics_state,
                            file_difference_state: &mut file_difference_state,
                            notification_sink,
                            file_difference_sink,
                        },
                    },
                )
                .await?;
            }
            _ = player_chat_input_tick.tick() => {
                if drain_player_chat_input_legacy_compatible(runtime)? {
                    flush_runtime_protocol_lines(runtime, &mut writer).await?;
                }
            }
            local_line = recv_local_input_line(&mut local_input_rx) => {
                let Some(local_line) = local_line else {
                    local_input_rx = None;
                    continue;
                };

                if let Some(command) = parse_local_input_command(&local_line) {
                    let command = plan_local_input_command_legacy_compatible(
                        command,
                        &LocalInputCommandPlanningContext {
                            current_room: runtime.session().room.as_deref(),
                            configured_room: &config.room,
                        },
                    );
                    let dispatch =
                        plan_local_input_dispatch_legacy_compatible(command, shared_playlists_enabled);
                    let help_version = config.version.as_str();
                    let emitted = {
                        let language = current_legacy_runtime_language_tag_legacy_compatible();
                        if let Some(lines) = shared_render_local_input_display_lines_legacy_compatible(
                            &dispatch,
                            runtime.session(),
                            language.as_deref(),
                            help_version,
                        ) {
                            for line in lines {
                                println!("{line}");
                            }
                            false
                        } else {
                            match dispatch {
                                PlannedLocalInputDispatch::Suppressed => false,
                                PlannedLocalInputDispatch::Run(action) => {
                                    run_planned_local_runtime_action_legacy_compatible(
                                        runtime,
                                        &mut local_user_offset_seconds,
                                        action,
                                    )?
                                }
                                _ => false,
                            }
                        }
                    };
                    let event_execution_plan =
                        connected_session_local_input_event_execution_plan_legacy_compatible(
                            emitted,
                            ConnectedSessionSharedExecutionInputs {
                                shared_playlists_enabled,
                                diagnostics: branch_diagnostics_plan,
                                outbound_state_sync_enabled,
                            },
                        );
                    run_connected_session_event_plan_legacy_compatible(
                        runtime,
                        None,
                        None,
                        connected_start.elapsed().as_secs_f64(),
                        dont_slow_down_with_me,
                        event_execution_plan,
                        ConnectedSessionEventExecutionContext {
                            pending_ready_at_start_on_server_hello: &mut pending_ready_at_start_on_server_hello,
                            pending_chat_message_on_connect: &mut pending_chat_message_on_connect,
                            outbound_state_sync_enabled: &mut outbound_state_sync_enabled,
                            branch: ConnectedSessionBranchExecutionContext {
                                config,
                                writer: &mut writer,
                                startup_playlist_file_on_connect,
                                diagnostics_config: &diagnostics_config,
                                reconnect_correction_diagnostics_state: &mut reconnect_correction_diagnostics_state,
                                file_difference_state: &mut file_difference_state,
                                notification_sink,
                                file_difference_sink,
                            },
                        },
                    )
                    .await?;
                }
            }
        }
    }
}
