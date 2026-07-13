use super::*;

mod execution;

use std::sync::{Arc, OnceLock};

use self::execution::{
    ConnectedSessionBranchExecutionContext, ConnectedSessionEventExecutionContext,
    run_connected_session_event_plan_legacy_compatible,
};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;

trait ConnectedSessionAsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> ConnectedSessionAsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type ConnectedSessionWriteHalf = tokio::io::WriteHalf<Box<dyn ConnectedSessionAsyncStream>>;

const CLI_PLEX_CLIENT_IDENTIFIER: &str = "sorotte-cli";
const CLI_PLEX_CACHE_FILE_NAME: &str = "plex-watch-cache.json";

fn client_runtime_now_seconds() -> f64 {
    static CLIENT_RUNTIME_CLOCK_EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
    CLIENT_RUNTIME_CLOCK_EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

fn normalized_tls_server_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn cli_plex_cache_path() -> Option<std::path::PathBuf> {
    crate::config_paths::resolve_sorotte_cli_config_path().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("cache").join(CLI_PLEX_CACHE_FILE_NAME))
    })
}

fn emit_application_service_events(events: Vec<ClientEvent>) {
    for event in events {
        match event {
            ClientEvent::Notification(message) => eprintln!("warning: {message}"),
            ClientEvent::OperationFailed { message, .. } => eprintln!("warning: {message}"),
            _ => {}
        }
    }
}

fn ensure_connected_application_command_succeeded(events: Vec<ClientEvent>) -> anyhow::Result<()> {
    if let Some(ClientEvent::OperationFailed { operation, message }) = events
        .into_iter()
        .find(|event| matches!(event, ClientEvent::OperationFailed { .. }))
    {
        return Err(anyhow!("{operation}: {message}"));
    }
    Ok(())
}

fn ensure_rustls_crypto_provider() {
    static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();
    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn default_tls_client_config() -> Arc<ClientConfig> {
    static TLS_CLIENT_CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    TLS_CLIENT_CONFIG
        .get_or_init(|| {
            ensure_rustls_crypto_provider();
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn start_tls_negotiation_enabled_legacy_compatible() -> bool {
    #[cfg(test)]
    const DEFAULT_START_TLS_NEGOTIATION_ENABLED: bool = false;
    #[cfg(not(test))]
    const DEFAULT_START_TLS_NEGOTIATION_ENABLED: bool = true;

    env_flag_override("SOROTTE_CLIENT_STARTTLS").unwrap_or(DEFAULT_START_TLS_NEGOTIATION_ENABLED)
}

fn decode_inbound_message_prefix_legacy_compatible(
    line: &str,
) -> (Vec<ProtocolMessage>, Option<ProtocolError>) {
    let items = match decode_message_line_items(line) {
        Ok(items) => items,
        Err(error) => return (Vec::new(), Some(error)),
    };
    let mut messages = Vec::new();
    for item in items {
        match item.message {
            Ok(message) => messages.push(message),
            Err(error) => return (messages, Some(error)),
        }
    }
    (messages, None)
}

async fn negotiate_start_tls_legacy_compatible(
    mut stream: TcpStream,
    host: &str,
) -> anyhow::Result<Box<dyn ConnectedSessionAsyncStream>> {
    let tls_request_line = encode_message_line(&ProtocolMessage::start_tls("send"))?;
    write_protocol_line(&mut stream, &tls_request_line).await?;

    let mut reader = BufReader::new(stream);
    let Some(tls_response_line) = read_inbound_protocol_line(&mut reader).await? else {
        return Err(anyhow!(
            "server closed connection before TLS negotiation completed"
        ));
    };

    let upgrade_to_tls = matches!(
        decode_message_line(tls_response_line.trim()),
        Ok(ProtocolMessage::Tls(tls_message)) if tls_message.tls.start_tls.contains("true")
    );
    let stream = reader.into_inner();
    if !upgrade_to_tls {
        return Ok(Box::new(stream));
    }

    let server_name = ServerName::try_from(normalized_tls_server_host(host).trim().to_owned())
        .map_err(|error| {
            anyhow!("client TLS negotiation failed because the server name is invalid: {error}")
        })?;
    let tls_stream = TlsConnector::from(default_tls_client_config())
        .connect(server_name, stream)
        .await?;
    Ok(Box::new(tls_stream))
}

#[cfg(test)]
pub(crate) async fn run_connected_client_session<F, G>(
    stream: TcpStream,
    runtime: &mut ClientApplication<MpvAdapter>,
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
    runtime: &mut ClientApplication<MpvAdapter>,
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
    let plex_config = cli_plex_config_from_env_and_stored_settings(None);
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
            plex_config: &plex_config,
        },
    )
    .await
}

pub(crate) struct ConnectedSessionLaunchContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    pub(crate) runtime: &'a mut ClientApplication<MpvAdapter>,
    pub(crate) config: &'a ClientLoopConfig,
    pub(crate) chat_message_on_connect: Option<&'a str>,
    pub(crate) startup_playlist_file_on_connect: &'a mut Option<String>,
    pub(crate) local_input_rx: Option<&'a mut UnboundedReceiver<String>>,
    pub(crate) notification_sink: &'a mut F,
    pub(crate) file_difference_sink: &'a mut G,
    pub(crate) diagnostics_config: ClientLoopDiagnosticsConfig,
    pub(crate) plex_config: &'a PlexClientConfig,
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
        plex_config,
    } = launch;
    let mut local_input_rx = local_input_rx;
    let mut hello_payload = HelloPayload::new(
        config.username.clone(),
        config.room.clone(),
        config.version.clone(),
    )
    .with_realversion(SYNCPLAY_COMPAT_VERSION_LEGACY);
    if let Some(server_password) = config.server_password.as_ref()
        && !server_password.is_empty()
    {
        hello_payload.extra.insert(
            "password".to_owned(),
            Value::String(legacy_server_password_token(
                server_password.expose_secret(),
            )),
        );
    }
    hello_payload.features = Some(client_hello_features_legacy_compatible(config));
    let hello_message = ProtocolMessage::hello(hello_payload);
    ensure_connected_application_command_succeeded(
        runtime.dispatch(ClientCommand::BeginConnecting),
    )?;
    ensure_connected_application_command_succeeded(runtime.dispatch(
        ClientCommand::InitializeSessionIdentity {
            username: config.username.clone(),
            room: config.room.clone(),
        },
    ))?;
    ensure_connected_application_command_succeeded(
        runtime.dispatch(ClientCommand::TransportConnected),
    )?;

    let hello_line = encode_message_line(&hello_message)?;
    let stream: Box<dyn ConnectedSessionAsyncStream> =
        if start_tls_negotiation_enabled_legacy_compatible() {
            negotiate_start_tls_legacy_compatible(stream, &config.host).await?
        } else {
            Box::new(stream)
        };
    let (reader, mut writer) = tokio::io::split(stream);
    emit_application_service_events(
        runtime
            .configure_plex_service(
                plex_config,
                CLI_PLEX_CLIENT_IDENTIFIER,
                cli_plex_cache_path(),
            )
            .await,
    );
    write_protocol_line(&mut writer, &hello_line).await?;
    let mut pending_chat_message_on_connect = chat_message_on_connect.map(str::to_owned);
    publish_pending_local_file_updates(runtime, config)?;
    flush_runtime_protocol_lines(runtime, &mut writer).await?;
    emit_application_service_events(runtime.pump_plex_service().await);

    let mut reader = BufReader::new(reader);
    let connected_start = Instant::now();
    let mut autoplay_tick =
        tokio::time::interval(Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS));
    let mut plex_tick = tokio::time::interval(Duration::from_secs(10));
    let mut player_chat_input_tick =
        tokio::time::interval(Duration::from_millis(PLAYER_CHAT_INPUT_POLL_INTERVAL_MS));
    let mut file_difference_state = FileDifferenceNotificationState::default();
    let mut reconnect_correction_diagnostics_state = ReconnectCorrectionDiagnosticsState::default();
    let mut local_user_offset_seconds = 0.0f64;
    let mut pending_ready_at_start_on_server_hello =
        Some(config.ready_at_start_override.unwrap_or(false));
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
            emit_application_service_events(runtime.shutdown_plex_service().await);
            return Ok(ConnectedSessionExit::RuntimeWindowElapsed);
        }

        let playback_barrier_retry_delay =
            runtime.pending_playback_barrier_retry_delay_at(client_runtime_now_seconds());
        let playback_barrier_retry_timer = async move {
            match playback_barrier_retry_delay {
                Some(delay_seconds) => {
                    tokio::time::sleep(Duration::from_secs_f64(delay_seconds.max(0.0))).await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(playback_barrier_retry_timer);

        tokio::select! {
            line = read_inbound_protocol_line(&mut reader) => {
                match line? {
                    Some(line) => {
                        let (decoded_inbound_messages, predecoded_inbound_error) =
                            decode_inbound_message_prefix_legacy_compatible(&line);
                        let inbound_is_server_hello = pending_ready_at_start_on_server_hello.is_some()
                            && decoded_inbound_messages
                                .iter()
                                .any(|message| matches!(message, ProtocolMessage::Hello(_)));
                        let now_seconds = client_runtime_now_seconds();
                        let event_execution_plan =
                            connected_session_inbound_message_event_execution_plan_legacy_compatible(
                                inbound_is_server_hello,
                                pending_chat_message_on_connect.is_some(),
                                decoded_inbound_messages
                                    .iter()
                                    .any(|message| matches!(message, ProtocolMessage::State(_))),
                                ConnectedSessionSharedExecutionInputs {
                                    shared_playlists_enabled,
                                    diagnostics: branch_diagnostics_plan,
                                    outbound_state_sync_enabled,
                                },
                            );
                        let event_result = run_connected_session_event_plan_legacy_compatible(
                            runtime,
                            Some(&line),
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
                        .await;
                        if let Err(error) = event_result {
                            emit_application_service_events(runtime.shutdown_plex_service().await);
                            return Err(error);
                        }
                        if let Some(error) = predecoded_inbound_error {
                            emit_application_service_events(runtime.shutdown_plex_service().await);
                            return Err(error.into());
                        }
                        emit_application_service_events(runtime.pump_plex_service().await);
                    }
                    None => {
                        emit_application_service_events(runtime.shutdown_plex_service().await);
                        return Ok(ConnectedSessionExit::TransportClosed);
                    }
                }
            }
            _ = autoplay_tick.tick() => {
                let now_seconds = client_runtime_now_seconds();
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
                emit_application_service_events(runtime.pump_plex_service().await);
            }
            _ = &mut playback_barrier_retry_timer => {
                let now_seconds = client_runtime_now_seconds();
                runtime.run_pending_playback_barrier_retry_at(now_seconds)?;
                flush_runtime_protocol_lines(runtime, &mut writer).await?;
            }
            _ = plex_tick.tick(), if runtime.plex_service_enabled() => {
                emit_application_service_events(runtime.pump_plex_service().await);
            }
            _ = player_chat_input_tick.tick() => {
                if drain_player_chat_input_legacy_compatible(runtime)? {
                    flush_runtime_protocol_lines(runtime, &mut writer).await?;
                }
                emit_application_service_events(runtime.pump_plex_service().await);
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
                            current_room: runtime.session().room(),
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
                        client_runtime_now_seconds(),
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
                    emit_application_service_events(runtime.pump_plex_service().await);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn starttls_negotiation_sends_request_and_accepts_plain_fallback() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let tls_line = lines
                .next_line()
                .await
                .expect("TLS line read should succeed")
                .expect("TLS line should be present");
            assert!(
                tls_line.contains(r#""TLS":{"startTLS":"send"}"#),
                "client should request StartTLS before Hello"
            );
            writer
                .write_all(b"{\"TLS\":{\"startTLS\":\"false\"}}\n")
                .await
                .expect("server TLS fallback write should succeed");
            writer
                .flush()
                .await
                .expect("server TLS fallback flush should succeed");
        });

        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test server");
        let _stream = negotiate_start_tls_legacy_compatible(stream, "127.0.0.1")
            .await
            .expect("plain TLS fallback should complete");

        server_task
            .await
            .expect("server task should complete without panic");
    }
}
