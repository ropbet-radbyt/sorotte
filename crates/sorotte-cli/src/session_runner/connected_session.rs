use super::*;

mod execution;

use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use self::execution::{
    ConnectedSessionBranchExecutionContext, ConnectedSessionEventExecutionContext,
    planned_local_runtime_action_is_player_bound,
    report_contained_connected_session_player_failure,
    run_connected_session_event_plan_legacy_compatible, run_contained_planned_local_runtime_action,
};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use sorotte_client_app::app_boundary::commands::PlannedLocalRuntimeAction;
use sorotte_player_mpv::SorotteBridgeHealth;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;

trait ConnectedSessionAsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> ConnectedSessionAsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type ConnectedSessionWriteHalf = tokio::io::WriteHalf<Box<dyn ConnectedSessionAsyncStream>>;

const CLI_PLEX_CLIENT_IDENTIFIER: &str = "sorotte-cli";
const CLI_PLEX_CACHE_FILE_NAME: &str = "plex-watch-cache.json";
const DEFAULT_STARTTLS_RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_INITIAL_HELLO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingReadyAtStart {
    desired: bool,
    had_current_v2_membership: bool,
}

pub(crate) fn client_runtime_now_seconds() -> f64 {
    // Client-core also has legacy-compatible entry points whose default clock
    // is Unix wall time. Keep this process-monotonic clock in the same numeric
    // domain so a local command cannot rebase fresh mpv observations from a
    // process-relative timestamp to epoch seconds. The Instant component keeps
    // retry/status deadlines monotonic after startup even if the wall clock is
    // adjusted.
    static CLIENT_RUNTIME_CLOCK_ORIGIN: OnceLock<(std::time::Instant, f64)> = OnceLock::new();
    let (origin, unix_seconds_at_origin) = CLIENT_RUNTIME_CLOCK_ORIGIN.get_or_init(|| {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        (std::time::Instant::now(), unix_seconds)
    });
    unix_seconds_at_origin + origin.elapsed().as_secs_f64()
}

fn insert_readiness_reconnect_token(
    hello: &mut HelloPayload,
    session: &sorotte_client_core::ClientSession,
    room: &str,
) {
    if let Some(reconnect_token) = session.readiness_reconnect_token_for_room(room) {
        hello.extra.insert(
            sorotte_protocol::SOROTTE_READINESS_RECONNECT_TOKEN.to_owned(),
            Value::String(reconnect_token.to_owned()),
        );
    }
}

fn normalized_tls_server_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn cli_plex_cache_path() -> anyhow::Result<Option<std::path::PathBuf>> {
    Ok(
        crate::config_paths::resolve_sorotte_cli_config_path()?.and_then(|path| {
            path.parent()
                .map(|parent| parent.join("cache").join(CLI_PLEX_CACHE_FILE_NAME))
        }),
    )
}

pub(super) fn emit_application_service_events(events: Vec<ClientEvent>) {
    for event in events {
        match event {
            ClientEvent::Notification(message) => eprintln!("warning: {message}"),
            ClientEvent::OperationFailed { message, .. } => eprintln!("warning: {message}"),
            _ => {}
        }
    }
}

#[derive(Debug)]
struct CliBridgeRuntimeHealthReporter {
    degradation_reported: bool,
}

impl CliBridgeRuntimeHealthReporter {
    fn new(initial_health: &SorotteBridgeHealth) -> Self {
        Self {
            degradation_reported: matches!(initial_health, SorotteBridgeHealth::Degraded(_)),
        }
    }

    fn line_for_transition(&mut self, health: &SorotteBridgeHealth) -> Option<String> {
        match health {
            SorotteBridgeHealth::Degraded(failure) if !self.degradation_reported => {
                self.degradation_reported = true;
                Some(format!(
                    "warning: mpv remains ready, but Chat/OSD integration degraded at runtime: {}",
                    failure.reason
                ))
            }
            SorotteBridgeHealth::Ready if self.degradation_reported => {
                self.degradation_reported = false;
                Some("info: mpv Chat/OSD integration recovered".to_owned())
            }
            SorotteBridgeHealth::Disabled => {
                self.degradation_reported = false;
                None
            }
            SorotteBridgeHealth::Ready
            | SorotteBridgeHealth::Recovering
            | SorotteBridgeHealth::Degraded(_) => None,
        }
    }
}

fn drain_cli_bridge_runtime_health_transitions_to_sink(
    runtime: &mut ClientApplication<MpvAdapter>,
    reporter: &mut CliBridgeRuntimeHealthReporter,
    mut emit: impl FnMut(String),
) {
    let transitions = runtime.with_player_io(|player| {
        std::iter::from_fn(|| player.take_sorotte_bridge_health_transition_nonblocking())
            .collect::<Vec<_>>()
    });
    for health in transitions {
        if let Some(line) = reporter.line_for_transition(&health) {
            emit(line);
        }
    }
}

fn report_cli_bridge_runtime_health_transitions(
    runtime: &mut ClientApplication<MpvAdapter>,
    reporter: &mut CliBridgeRuntimeHealthReporter,
) {
    drain_cli_bridge_runtime_health_transitions_to_sink(runtime, reporter, |line| {
        eprintln!("{line}");
    });
}

#[cfg(test)]
mod bridge_runtime_health_reporter_tests {
    use super::*;
    use sorotte_player_mpv::{
        LegacySyncplayUiSettings, SorotteBridgeFailure, SorotteBridgeFailureKind,
    };

    fn degraded(reason: &str) -> SorotteBridgeHealth {
        SorotteBridgeHealth::Degraded(SorotteBridgeFailure {
            kind: SorotteBridgeFailureKind::LeaseBusy,
            reason: reason.to_owned(),
        })
    }

    #[test]
    fn cli_runtime_health_reporter_logs_each_degradation_and_recovery_once() {
        let mut reporter = CliBridgeRuntimeHealthReporter::new(&SorotteBridgeHealth::Ready);

        assert!(
            reporter
                .line_for_transition(&SorotteBridgeHealth::Ready)
                .is_none()
        );
        assert!(
            reporter
                .line_for_transition(&SorotteBridgeHealth::Recovering)
                .is_none()
        );
        assert_eq!(
            reporter
                .line_for_transition(&degraded("another owner holds the input lease"))
                .as_deref(),
            Some(
                "warning: mpv remains ready, but Chat/OSD integration degraded at runtime: another owner holds the input lease"
            )
        );
        assert!(
            reporter
                .line_for_transition(&degraded("same degradation repeated"))
                .is_none()
        );
        assert_eq!(
            reporter
                .line_for_transition(&SorotteBridgeHealth::Ready)
                .as_deref(),
            Some("info: mpv Chat/OSD integration recovered")
        );
        assert!(
            reporter
                .line_for_transition(&SorotteBridgeHealth::Ready)
                .is_none()
        );
    }

    #[test]
    fn cli_player_pump_consumes_runtime_degradation_transition_only_once() {
        let (mut player, _release_count) =
            MpvAdapter::with_release_recording_sorotte_bridge_test_ipc(LegacySyncplayUiSettings {
                chat_move_osd: false,
                ..LegacySyncplayUiSettings::default()
            });
        let mut reporter = CliBridgeRuntimeHealthReporter::new(&player.sorotte_bridge_health());
        player.mark_sorotte_bridge_degraded(
            SorotteBridgeFailureKind::LeaseBusy,
            "runtime lease reacquisition was rejected",
        );
        let mut runtime = ClientApplication::with_default_session(player);
        let mut lines = Vec::new();

        drain_cli_bridge_runtime_health_transitions_to_sink(&mut runtime, &mut reporter, |line| {
            lines.push(line)
        });
        drain_cli_bridge_runtime_health_transitions_to_sink(&mut runtime, &mut reporter, |line| {
            lines.push(line)
        });

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("runtime lease reacquisition was rejected"));
        assert!(runtime.player().is_connected());
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

fn positive_timeout_from_env(name: &str, default: Duration) -> Duration {
    positive_timeout_or_default(env_non_negative_f64(name), default)
}

fn positive_timeout_or_default(seconds: Option<f64>, default: Duration) -> Duration {
    seconds
        .filter(|seconds| *seconds > 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .unwrap_or(default)
}

fn client_tls_policy(
    config: &ClientLoopConfig,
    persisted_override: Option<TlsPolicy>,
) -> TlsPolicy {
    if let Some(policy) = env_trimmed("SOROTTE_CLIENT_TLS_POLICY")
        .as_deref()
        .and_then(TlsPolicy::parse)
    {
        return policy;
    }
    if let Some(start_tls) = env_flag_override("SOROTTE_CLIENT_STARTTLS") {
        return if start_tls {
            TlsPolicy::PreferTls
        } else {
            TlsPolicy::Plaintext
        };
    }
    inferred_client_tls_policy(config, persisted_override)
}

fn inferred_client_tls_policy(
    config: &ClientLoopConfig,
    persisted_override: Option<TlsPolicy>,
) -> TlsPolicy {
    if let Some(policy) = persisted_override {
        return policy;
    }
    let has_credentials = config
        .server_password
        .as_ref()
        .is_some_and(|password| !password.expose_secret().is_empty())
        || config
            .controlled_room_password_override
            .as_ref()
            .is_some_and(|password| !password.expose_secret().is_empty());
    TlsPolicy::default_for_credentials(has_credentials)
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

async fn negotiate_start_tls_with_policy(
    mut stream: TcpStream,
    host: &str,
    policy: TlsPolicy,
    response_timeout: Duration,
    handshake_timeout: Duration,
) -> anyhow::Result<StartTlsNegotiationResult> {
    debug_assert_ne!(policy, TlsPolicy::Plaintext);
    let tls_response_line = tokio::time::timeout(response_timeout, async {
        let tls_request_line = encode_message_line(&ProtocolMessage::start_tls("send"))?;
        write_protocol_line(&mut stream, &tls_request_line).await?;
        let mut reader = BufReader::new(stream);
        // This reader is terminal to the STARTTLS response attempt: timeout
        // cancellation drops the connection, while successful fallback keeps
        // the BufReader and its prefetched bytes as the transport.
        let mut line_reader = InboundProtocolLineReader::default();
        let response = line_reader.read_line(&mut reader).await?;
        // Keep the reader as the transport. PreferTls fallback must preserve
        // any later plaintext protocol bytes that arrived in the same socket
        // read as the STARTTLS response.
        Ok::<_, anyhow::Error>((reader, response))
    })
    .await
    .map_err(|_| {
        anyhow!(
            "server STARTTLS response timed out after {:.1} seconds",
            response_timeout.as_secs_f64()
        )
    })??;
    let (stream, tls_response_line) = tls_response_line;
    let Some(tls_response_line) = tls_response_line else {
        return Err(anyhow!(
            "server closed connection before TLS negotiation completed"
        ));
    };

    let mut prefetched_plaintext_lines = VecDeque::new();
    let decoded_items = decode_message_line_items(tls_response_line.trim());
    let upgrade_to_tls = match decoded_items {
        Ok(items) if items.iter().all(|item| item.message.is_ok()) => {
            let messages = items
                .into_iter()
                .map(|item| item.message.expect("validated protocol item"))
                .collect::<Vec<_>>();
            if matches!(
                messages.as_slice(),
                [ProtocolMessage::Tls(tls_message)] if tls_message.tls.start_tls == "true"
            ) {
                true
            } else if matches!(messages.as_slice(), [ProtocolMessage::Tls(_)]) {
                false
            } else {
                if policy == TlsPolicy::RequireTls {
                    if let [message] = messages.as_slice() {
                        return Err(anyhow!(
                            "server returned unexpected {} message instead of accepting required TLS",
                            message.kind()
                        ));
                    }
                    return Err(anyhow!(
                        "server bundled additional protocol messages with its STARTTLS response instead of providing a standalone required TLS acceptance"
                    ));
                }
                eprintln!(
                    "warning: server returned an unexpected STARTTLS response; continuing over plaintext because TLS policy is PreferTls"
                );
                for message in messages
                    .into_iter()
                    .filter(|message| !matches!(message, ProtocolMessage::Tls(_)))
                {
                    prefetched_plaintext_lines.push_back(encode_message_line(&message)?);
                }
                false
            }
        }
        Ok(_) => {
            if policy == TlsPolicy::RequireTls {
                return Err(anyhow!(
                    "server returned a malformed response instead of accepting required TLS"
                ));
            }
            eprintln!(
                "warning: server returned a malformed STARTTLS response; continuing over plaintext because TLS policy is PreferTls"
            );
            false
        }
        Err(error) => {
            if policy == TlsPolicy::RequireTls {
                return Err(anyhow!(
                    "server returned a malformed response instead of accepting required TLS: {error}"
                ));
            }
            eprintln!(
                "warning: server returned a malformed STARTTLS response; continuing over plaintext because TLS policy is PreferTls"
            );
            false
        }
    };
    if !upgrade_to_tls {
        if policy == TlsPolicy::RequireTls {
            return Err(anyhow!("server refused required TLS negotiation"));
        }
        eprintln!(
            "warning: server declined STARTTLS; continuing over plaintext because TLS policy is PreferTls"
        );
        return Ok(StartTlsNegotiationResult {
            stream: Box::new(stream),
            prefetched_plaintext_lines,
        });
    }

    let server_name = ServerName::try_from(normalized_tls_server_host(host).trim().to_owned())
        .map_err(|error| {
            anyhow!("client TLS negotiation failed because the server name is invalid: {error}")
        })?;
    let tls_stream = tokio::time::timeout(
        handshake_timeout,
        TlsConnector::from(default_tls_client_config()).connect(server_name, stream),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "client TLS handshake timed out after {:.1} seconds",
            handshake_timeout.as_secs_f64()
        )
    })??;
    Ok(StartTlsNegotiationResult {
        stream: Box::new(tls_stream),
        prefetched_plaintext_lines: VecDeque::new(),
    })
}

struct StartTlsNegotiationResult {
    stream: Box<dyn ConnectedSessionAsyncStream>,
    prefetched_plaintext_lines: VecDeque<String>,
}

async fn read_inbound_or_prefetched_protocol_line<R>(
    reader: &mut R,
    prefetched_lines: &mut VecDeque<String>,
    line_reader: &mut InboundProtocolLineReader,
) -> anyhow::Result<Option<String>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    if let Some(line) = prefetched_lines.pop_front() {
        return Ok(Some(line));
    }
    line_reader.read_line(reader).await
}

#[cfg(test)]
async fn negotiate_start_tls_legacy_compatible(
    stream: TcpStream,
    host: &str,
) -> anyhow::Result<Box<dyn ConnectedSessionAsyncStream>> {
    negotiate_start_tls_with_policy(
        stream,
        host,
        TlsPolicy::PreferTls,
        DEFAULT_STARTTLS_RESPONSE_TIMEOUT,
        DEFAULT_TLS_HANDSHAKE_TIMEOUT,
    )
    .await
    .map(|negotiation| negotiation.stream)
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
pub(crate) async fn run_connected_client_session_with_plex_config_for_test<F, G>(
    stream: TcpStream,
    runtime: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
    chat_message_on_connect: Option<&str>,
    local_input_rx: Option<&mut UnboundedReceiver<String>>,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
    plex_config: &PlexClientConfig,
) -> anyhow::Result<ConnectedSessionExit>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let mut no_playlist = None;
    let diagnostics_config = client_loop_diagnostics_config(None);
    let mut network_options_health_reporter = CliNetworkOptionsHealthReporter::default();
    run_connected_client_session_with_legacy_startup_overrides_and_diagnostics(
        stream,
        ConnectedSessionLaunchContext {
            runtime,
            config,
            chat_message_on_connect,
            startup_playlist_file_on_connect: &mut no_playlist,
            local_input_rx,
            notification_sink,
            file_difference_sink,
            diagnostics_config,
            plex_config,
            network_options_health_reporter: &mut network_options_health_reporter,
            tls_policy_override: Some(TlsPolicy::Plaintext),
        },
    )
    .await
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
    let mut network_options_health_reporter = CliNetworkOptionsHealthReporter::default();
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
            network_options_health_reporter: &mut network_options_health_reporter,
            // The legacy connected-session tests use plaintext protocol fixtures. Keep that
            // test-only helper pinned to plaintext; production callers resolve the configured
            // TLS policy through the network-loop launch context below.
            tls_policy_override: Some(TlsPolicy::Plaintext),
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
    pub(crate) network_options_health_reporter: &'a mut CliNetworkOptionsHealthReporter,
    pub(crate) tls_policy_override: Option<TlsPolicy>,
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
        network_options_health_reporter,
        tls_policy_override,
    } = launch;
    network_options_health_reporter
        .set_player_telemetry_diagnostics_enabled(diagnostics_config.log_player_telemetry);
    let had_current_v2_membership = runtime.session().room() == Some(config.room.as_str())
        && (runtime.session().readiness_snapshot().is_some()
            || runtime.session().pending_readiness_intent().is_some());
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
    insert_readiness_reconnect_token(&mut hello_payload, runtime.session(), &config.room);
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
    let tls_policy = client_tls_policy(config, tls_policy_override);
    let (stream, mut prefetched_inbound_lines): (
        Box<dyn ConnectedSessionAsyncStream>,
        VecDeque<String>,
    ) = if tls_policy != TlsPolicy::Plaintext {
        let negotiation = await_with_player_integration_maintenance(
            runtime,
            negotiate_start_tls_with_policy(
                stream,
                &config.host,
                tls_policy,
                positive_timeout_from_env(
                    "SOROTTE_CLIENT_STARTTLS_TIMEOUT_SECONDS",
                    DEFAULT_STARTTLS_RESPONSE_TIMEOUT,
                ),
                positive_timeout_from_env(
                    "SOROTTE_CLIENT_TLS_HANDSHAKE_TIMEOUT_SECONDS",
                    DEFAULT_TLS_HANDSHAKE_TIMEOUT,
                ),
            ),
        )
        .await?;
        (negotiation.stream, negotiation.prefetched_plaintext_lines)
    } else {
        (Box::new(stream), VecDeque::new())
    };
    let (reader, mut writer) = tokio::io::split(stream);
    emit_application_service_events(
        runtime
            .configure_plex_service(
                plex_config,
                CLI_PLEX_CLIENT_IDENTIFIER,
                cli_plex_cache_path()?,
            )
            .await,
    );
    let initial_hello_timeout = positive_timeout_from_env(
        "SOROTTE_CLIENT_INITIAL_HELLO_TIMEOUT_SECONDS",
        DEFAULT_INITIAL_HELLO_TIMEOUT,
    );
    let initial_hello_deadline = Instant::now() + initial_hello_timeout;
    tokio::time::timeout_at(
        initial_hello_deadline,
        await_with_player_integration_maintenance(
            runtime,
            write_protocol_line(&mut writer, &hello_line),
        ),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "server initial Hello timed out after {:.1} seconds while sending the client Hello",
            initial_hello_timeout.as_secs_f64()
        )
    })??;
    let mut pending_chat_message_on_connect = chat_message_on_connect.map(str::to_owned);
    publish_pending_local_file_updates(
        runtime,
        config,
        network_options_health_reporter,
        client_runtime_now_seconds(),
    )?;
    if !flush_runtime_protocol_lines_until(runtime, &mut writer, initial_hello_deadline).await? {
        return Err(anyhow!(
            "server initial Hello timed out after {:.1} seconds while sending startup protocol messages",
            initial_hello_timeout.as_secs_f64()
        ));
    }
    emit_application_service_events(runtime.pump_plex_service().await);

    let mut reader = BufReader::new(reader);
    let mut inbound_line_reader = InboundProtocolLineReader::default();
    let connected_start = Instant::now();
    let mut autoplay_tick =
        tokio::time::interval(Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS));
    let mut plex_tick = tokio::time::interval(Duration::from_secs(10));
    let mut player_integration_tick =
        tokio::time::interval(Duration::from_millis(PLAYER_CHAT_INPUT_POLL_INTERVAL_MS));
    let mut file_difference_state = FileDifferenceNotificationState::default();
    let mut reconnect_correction_diagnostics_state = ReconnectCorrectionDiagnosticsState::default();
    let mut seek_preparation_notification_state = SeekPreparationNotificationState::default();
    let mut readiness_notification_state = ReadinessNotificationState::default();
    let mut local_user_offset_seconds = 0.0f64;
    let mut pending_ready_at_start_on_server_hello = Some(PendingReadyAtStart {
        desired: config.ready_at_start_override.unwrap_or(false),
        had_current_v2_membership,
    });
    let mut pending_causally_fenced_player_input = None;
    let initial_hello_deadline = tokio::time::sleep_until(initial_hello_deadline);
    tokio::pin!(initial_hello_deadline);
    let mut outbound_state_sync_enabled = false;
    let branch_diagnostics_plan = ConnectedSessionDiagnosticsPlan {
        log_player_telemetry: diagnostics_config.log_player_telemetry,
        log_player_drift: diagnostics_config.log_player_drift,
        reconnect_correction_diagnostics_format: diagnostics_config
            .reconnect_correction_diagnostics_format,
    };
    let shared_playlists_enabled = shared_playlists_enabled_cli_legacy_compatible(config);
    let dont_slow_down_with_me = config.dont_slow_down_with_me_override.unwrap_or(false);
    let mut bridge_health_reporter =
        CliBridgeRuntimeHealthReporter::new(&runtime.player().sorotte_bridge_health());

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
        let player_input_fence_active = pending_ready_at_start_on_server_hello.is_some()
            || runtime.session().has_pending_playlist_index_reset_intent();
        let may_poll_local_input =
            pending_causally_fenced_player_input.is_none() || !player_input_fence_active;

        tokio::select! {
            _ = &mut initial_hello_deadline, if pending_ready_at_start_on_server_hello.is_some() => {
                emit_application_service_events(runtime.shutdown_plex_service().await);
                return Err(anyhow!(
                    "server initial Hello timed out after {:.1} seconds",
                    initial_hello_timeout.as_secs_f64()
                ));
            }
            line = read_inbound_or_prefetched_protocol_line(
                &mut reader,
                &mut prefetched_inbound_lines,
                &mut inbound_line_reader,
            ) => {
                match line? {
                    Some(line) => {
                        let (decoded_inbound_messages, predecoded_inbound_error) =
                            decode_inbound_message_prefix_legacy_compatible(&line);
                        let inbound_is_server_hello = pending_ready_at_start_on_server_hello.is_some()
                            && (decoded_inbound_messages
                                .iter()
                                .any(|message| matches!(message, ProtocolMessage::Hello(_)))
                                || runtime.session().server_readiness_v2_supported());
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
                                    seek_preparation_notification_state: &mut seek_preparation_notification_state,
                                    readiness_notification_state: &mut readiness_notification_state,
                                    file_difference_state: &mut file_difference_state,
                                    network_options_health_reporter,
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
                            seek_preparation_notification_state: &mut seek_preparation_notification_state,
                            readiness_notification_state: &mut readiness_notification_state,
                            file_difference_state: &mut file_difference_state,
                            network_options_health_reporter,
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
            _ = player_integration_tick.tick() => {
                report_cli_bridge_runtime_health_transitions(
                    runtime,
                    &mut bridge_health_reporter,
                );
                let _ = drain_player_chat_input_legacy_compatible(runtime)?;
                let now_seconds = client_runtime_now_seconds();
                let event_execution_plan =
                    connected_session_player_coordination_tick_event_execution_plan_legacy_compatible(
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
                            seek_preparation_notification_state: &mut seek_preparation_notification_state,
                            readiness_notification_state: &mut readiness_notification_state,
                            file_difference_state: &mut file_difference_state,
                            network_options_health_reporter,
                            notification_sink,
                            file_difference_sink,
                        },
                    },
                )
                .await?;
                emit_application_service_events(runtime.pump_plex_service().await);
            }
            local_line = async {
                if let Some(line) = pending_causally_fenced_player_input.take() {
                    Some(line)
                } else {
                    recv_local_input_line(&mut local_input_rx).await
                }
            }, if may_poll_local_input => {
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
                    if player_input_fence_active
                        && matches!(
                            &dispatch,
                            PlannedLocalInputDispatch::Run(action)
                                if planned_local_runtime_action_is_player_bound(action)
                        )
                    {
                        // A connected socket is not yet room authority, and a
                        // playlist selection does not own its successor
                        // transport revision until the paired canonical State
                        // arrives. Keep player-bound input in FIFO order across
                        // either causal fence; otherwise the physical player can
                        // change without an eligible successor State.
                        pending_causally_fenced_player_input = Some(local_line);
                        continue;
                    }
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
                                    let publish_local_player_state =
                                        planned_local_runtime_action_is_player_bound(&action);
                                    let (emitted, failure) =
                                        run_contained_planned_local_runtime_action(
                                        runtime,
                                        &mut local_user_offset_seconds,
                                        client_runtime_now_seconds(),
                                        action,
                                    )?;
                                    if let Some(failure) = failure {
                                        report_contained_connected_session_player_failure(&failure);
                                        flush_runtime_protocol_lines(runtime, &mut writer).await?;
                                    }
                                    if emitted && publish_local_player_state {
                                        // Publish a player-bound local mutation in the same event
                                        // turn. If canonical room state has not arrived yet this may
                                        // be ping-only; the core's generation-scoped local intent
                                        // then carries Play/Pause into the first canonical response.
                                        let _ = runtime
                                            .run_state_sync_heartbeat_legacy_ping_compatible(
                                                dont_slow_down_with_me,
                                            );
                                    }
                                    emitted
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
                                seek_preparation_notification_state: &mut seek_preparation_notification_state,
                                readiness_notification_state: &mut readiness_notification_state,
                                file_difference_state: &mut file_difference_state,
                                network_options_health_reporter,
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
    use sorotte_client_core::ClientSession;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tokio::io::AsyncBufReadExt;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    struct StartTlsMaintenancePlayer(Arc<AtomicUsize>);

    impl PlayerAdapter for StartTlsMaintenancePlayer {
        fn name(&self) -> &'static str {
            "starttls-maintenance-test-player"
        }

        fn maintain_runtime_leases_nonblocking(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn maintain_runtime_integrations(&mut self) {
            panic!("async connection waits must not invoke blocking player maintenance");
        }
    }

    async fn wait_for_protocol_barrier(flag: &AtomicBool, description: &str) {
        let watchdog_started_at = std::time::Instant::now();
        while !flag.load(Ordering::SeqCst) {
            assert!(
                watchdog_started_at.elapsed() < Duration::from_secs(5),
                "real-time harness watchdog expired while waiting for {description}"
            );
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn reconnect_token_is_added_only_to_same_room_hello() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true},"sorotteReadinessReconnectToken":"cli-reconnect-token"}}"#,
            )
            .expect("server Hello should store the reconnect token");

        let mut same_room = HelloPayload::new("alice", "room1", "1.2.255");
        insert_readiness_reconnect_token(&mut same_room, &session, "room1");
        assert_eq!(
            same_room
                .extra
                .get(sorotte_protocol::SOROTTE_READINESS_RECONNECT_TOKEN),
            Some(&Value::String("cli-reconnect-token".to_owned()))
        );

        let mut other_room = HelloPayload::new("alice", "room2", "1.2.255");
        insert_readiness_reconnect_token(&mut other_room, &session, "room2");
        assert!(
            !other_room
                .extra
                .contains_key(sorotte_protocol::SOROTTE_READINESS_RECONNECT_TOKEN)
        );
    }

    #[test]
    fn persisted_tls_policy_overrides_credential_inference() {
        let mut config = crate::tests::test_client_loop_config();
        config.server_password = Some("saved-secret".into());
        assert_eq!(
            inferred_client_tls_policy(&config, None),
            TlsPolicy::RequireTls
        );
        assert_eq!(
            inferred_client_tls_policy(&config, Some(TlsPolicy::Plaintext)),
            TlsPolicy::Plaintext
        );
        assert_eq!(
            inferred_client_tls_policy(&config, Some(TlsPolicy::PreferTls)),
            TlsPolicy::PreferTls
        );
    }

    #[test]
    fn timeout_parser_falls_back_for_extreme_finite_values() {
        let default = Duration::from_secs(7);
        assert_eq!(
            positive_timeout_or_default(Some(f64::MAX), default),
            default
        );
        assert_eq!(
            positive_timeout_or_default(Some(1.5), default),
            Duration::from_millis(1_500)
        );
    }

    #[tokio::test]
    async fn starttls_negotiation_sends_request_and_accepts_plain_fallback() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let maintenance_calls = Arc::new(AtomicUsize::new(0));
        let server_maintenance_calls = Arc::clone(&maintenance_calls);
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
            while server_maintenance_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
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
        let mut runtime = ClientApplication::with_default_session(StartTlsMaintenancePlayer(
            Arc::clone(&maintenance_calls),
        ));
        let _stream = tokio::time::timeout(
            Duration::from_secs(2),
            await_with_player_integration_maintenance(
                &mut runtime,
                negotiate_start_tls_legacy_compatible(stream, "127.0.0.1"),
            ),
        )
        .await
        .expect("STARTTLS fallback should remain serviced")
        .expect("plain TLS fallback should complete");
        assert!(
            maintenance_calls.load(Ordering::SeqCst) >= 1,
            "STARTTLS response wait must maintain player integrations"
        );

        server_task
            .await
            .expect("server task should complete without panic");
    }

    async fn required_tls_error_for_response(response: &'static [u8]) -> String {
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
            lines
                .next_line()
                .await
                .expect("TLS request read should succeed")
                .expect("TLS request should be present");
            writer
                .write_all(response)
                .await
                .expect("test response should write");
            writer.flush().await.expect("test response should flush");
        });
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test server");
        let error = negotiate_start_tls_with_policy(
            stream,
            "127.0.0.1",
            TlsPolicy::RequireTls,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .err()
        .expect("required TLS must reject a downgrade response")
        .to_string();
        server_task.await.expect("server task should complete");
        error
    }

    #[tokio::test]
    async fn require_tls_rejects_refusal_substitution_and_truncation() {
        let refused =
            required_tls_error_for_response(b"{\"TLS\":{\"startTLS\":\"false\"}}\n").await;
        assert!(refused.contains("refused required TLS"));

        let substituted = required_tls_error_for_response(
            b"{\"Hello\":{\"username\":\"mallory\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n",
        )
        .await;
        assert!(substituted.contains("unexpected Hello message"));

        let truncated = required_tls_error_for_response(b"{\"TLS\":{\"startTLS\":\"tru").await;
        assert!(truncated.contains("malformed response"));
    }

    async fn prefer_tls_prefetched_lines_for_response(response: &'static [u8]) -> VecDeque<String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");
        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            lines
                .next_line()
                .await
                .expect("TLS request read should succeed")
                .expect("TLS request should be present");
            writer
                .write_all(response)
                .await
                .expect("test response should write");
            writer.flush().await.expect("test response should flush");
        });
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect");
        let negotiation = negotiate_start_tls_with_policy(
            stream,
            "127.0.0.1",
            TlsPolicy::PreferTls,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .expect("PreferTls should retain a plaintext connection");
        server_task.await.expect("server task should complete");
        negotiation.prefetched_plaintext_lines
    }

    #[tokio::test]
    async fn prefer_tls_reinjects_unexpected_hello_and_error_lines() {
        for (expected, response) in [
            (
                r#"{"Hello":{"username":"server","room":{"name":"room"},"version":"1.7.5"}}"#,
                b"{\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n"
                    .as_slice(),
            ),
            (
                r#"{"Error":{"message":"something went wrong"}}"#,
                b"{\"Error\":{\"message\":\"something went wrong\"}}\n".as_slice(),
            ),
        ] {
            let mut pending = prefer_tls_prefetched_lines_for_response(response).await;
            let mut reader = BufReader::new(tokio::io::empty());
            let mut line_reader = InboundProtocolLineReader::default();
            let reinjected = read_inbound_or_prefetched_protocol_line(
                &mut reader,
                &mut pending,
                &mut line_reader,
            )
            .await
            .expect("prefetched protocol line should be readable")
            .expect("prefetched protocol line should be present");
            assert_eq!(
                decode_message_line(&reinjected)
                    .expect("re-injected line should enter normal protocol decoding"),
                decode_message_line(expected).expect("expected protocol fixture should decode"),
            );
            assert!(
                decode_message_line(&reinjected).is_ok(),
                "re-injected line should enter normal protocol decoding"
            );
            assert!(pending.is_empty());
        }
    }

    #[tokio::test]
    async fn prefer_tls_preserves_protocol_lines_read_ahead_with_unexpected_response() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");
        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            lines
                .next_line()
                .await
                .expect("TLS request read should succeed")
                .expect("TLS request should be present");
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n{\"Chat\":{\"username\":\"server\",\"message\":\"preserve read ahead\"}}\n",
                )
                .await
                .expect("two-line fallback response should write in one operation");
            writer
                .flush()
                .await
                .expect("fallback response should flush");
        });

        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect");
        let mut negotiation = negotiate_start_tls_with_policy(
            stream,
            "127.0.0.1",
            TlsPolicy::PreferTls,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .expect("PreferTls should retain a plaintext connection");
        server_task.await.expect("server task should complete");

        let mut reader = BufReader::new(negotiation.stream);
        let mut line_reader = InboundProtocolLineReader::default();
        let first = read_inbound_or_prefetched_protocol_line(
            &mut reader,
            &mut negotiation.prefetched_plaintext_lines,
            &mut line_reader,
        )
        .await
        .expect("prefetched Hello read should succeed")
        .expect("prefetched Hello should be present");
        assert!(matches!(
            decode_message_line(&first),
            Ok(ProtocolMessage::Hello(_))
        ));

        let second = tokio::time::timeout(
            Duration::from_secs(1),
            read_inbound_or_prefetched_protocol_line(
                &mut reader,
                &mut negotiation.prefetched_plaintext_lines,
                &mut line_reader,
            ),
        )
        .await
        .expect("the already-sent second line should not time out")
        .expect("second line read should succeed")
        .expect("the read-ahead Chat line must not be discarded");
        assert!(matches!(
            decode_message_line(&second),
            Ok(ProtocolMessage::Chat(_))
        ));
    }

    #[tokio::test]
    async fn prefer_tls_reinjects_only_application_messages_bundled_with_a_tls_refusal() {
        for response in [
            b"{\"TLS\":{\"startTLS\":\"false\"},\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n"
                .as_slice(),
            b"{\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
            b"{\"TLS\":{\"startTLS\":\"false\"},\"Error\":{\"message\":\"something went wrong\"}}\n"
                .as_slice(),
            b"{\"Error\":{\"message\":\"something went wrong\"},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
            b"{\"TLS\":{\"startTLS\":\"false\"},\"State\":{\"playstate\":{\"position\":5.0,\"paused\":false,\"doSeek\":false}}}\n"
                .as_slice(),
            b"{\"State\":{\"playstate\":{\"position\":5.0,\"paused\":false,\"doSeek\":false}},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
        ] {
            let prefetched = prefer_tls_prefetched_lines_for_response(response).await;
            let messages = prefetched
                .iter()
                .map(|line| {
                    decode_message_line(line)
                        .expect("each re-injected application message should decode")
                })
                .collect::<Vec<_>>();

            assert_eq!(messages.len(), 1);
            assert!(
                messages
                    .iter()
                    .all(|message| !matches!(message, ProtocolMessage::Tls(_)))
            );
            assert!(
                messages
                    .iter()
                    .any(|message| matches!(
                        message,
                        ProtocolMessage::Hello(_)
                            | ProtocolMessage::Error(_)
                            | ProtocolMessage::State(_)
                    ))
            );
        }
    }

    #[tokio::test]
    async fn prefer_tls_bundled_refusal_reinjects_only_application_messages() {
        let response =
            b"{\"TLS\":{\"startTLS\":\"false\"},\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n";
        let mut prefetched = prefer_tls_prefetched_lines_for_response(response).await;
        let mut session = sorotte_client_core::ClientSession::default();

        session
            .apply_message_json(
                &prefetched
                    .pop_front()
                    .expect("the bundled Hello must be preserved"),
            )
            .expect("negotiation control must not poison normal plaintext inbound handling");
        assert!(prefetched.is_empty());
        assert_eq!(session.username(), Some("server"));
        assert_eq!(session.room(), Some("room"));
    }

    #[tokio::test]
    async fn require_tls_rejects_messages_bundled_with_a_tls_refusal_in_either_order() {
        for response in [
            b"{\"TLS\":{\"startTLS\":\"false\"},\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"}}\n"
                .as_slice(),
            b"{\"Hello\":{\"username\":\"server\",\"room\":{\"name\":\"room\"},\"version\":\"1.7.5\"},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
            b"{\"TLS\":{\"startTLS\":\"false\"},\"Error\":{\"message\":\"something went wrong\"}}\n"
                .as_slice(),
            b"{\"Error\":{\"message\":\"something went wrong\"},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
            b"{\"TLS\":{\"startTLS\":\"false\"},\"State\":{\"playstate\":{\"position\":5.0,\"paused\":false,\"doSeek\":false}}}\n"
                .as_slice(),
            b"{\"State\":{\"playstate\":{\"position\":5.0,\"paused\":false,\"doSeek\":false}},\"TLS\":{\"startTLS\":\"false\"}}\n"
                .as_slice(),
        ] {
            let error = required_tls_error_for_response(response).await;
            assert!(error.contains("bundled additional protocol messages"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn starttls_response_and_handshake_have_independent_deadlines() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");
        let request_observed = Arc::new(AtomicBool::new(false));
        let server_request_observed = Arc::clone(&request_observed);
        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, _writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            lines
                .next_line()
                .await
                .expect("TLS request read should succeed")
                .expect("TLS request should be present");
            server_request_observed.store(true, Ordering::SeqCst);
            assert!(
                lines
                    .next_line()
                    .await
                    .expect("timeout connection close should be readable")
                    .is_none(),
                "the client must close without sending application protocol"
            );
        });
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect");
        let response_started_at = tokio::time::Instant::now();
        let response_task = tokio::spawn(negotiate_start_tls_with_policy(
            stream,
            "127.0.0.1",
            TlsPolicy::RequireTls,
            Duration::from_millis(25),
            Duration::from_secs(1),
        ));
        wait_for_protocol_barrier(&request_observed, "the server to read STARTTLS").await;
        assert_eq!(
            tokio::time::Instant::now(),
            response_started_at,
            "protocol progress must not require wall-clock advancement"
        );
        tokio::time::advance(Duration::from_millis(25)).await;
        let response_timeout = response_task
            .await
            .expect("response timeout task should not panic")
            .err()
            .expect("silent STARTTLS endpoint should time out");
        assert_eq!(
            tokio::time::Instant::now().duration_since(response_started_at),
            Duration::from_millis(25),
            "the STARTTLS response phase must consume exactly its own deadline"
        );
        assert!(
            response_timeout
                .to_string()
                .contains("STARTTLS response timed out")
        );
        server_task.await.expect("server task should complete");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");
        let client_hello_observed = Arc::new(AtomicBool::new(false));
        let server_client_hello_observed = Arc::clone(&client_hello_observed);
        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut reader = BufReader::new(reader);
            let mut request = String::new();
            assert_ne!(
                reader
                    .read_line(&mut request)
                    .await
                    .expect("TLS request read should succeed"),
                0,
                "TLS request should be present"
            );
            writer
                .write_all(b"{\"TLS\":{\"startTLS\":\"true\"}}\n")
                .await
                .expect("TLS acceptance should write");
            writer.flush().await.expect("TLS acceptance should flush");
            let mut first_client_hello_byte = [0_u8; 1];
            reader
                .read_exact(&mut first_client_hello_byte)
                .await
                .expect("the TLS handshake should send a ClientHello");
            server_client_hello_observed.store(true, Ordering::SeqCst);
            let mut remainder = Vec::new();
            reader
                .read_to_end(&mut remainder)
                .await
                .expect("timed-out handshake should close the connection");
        });
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect");
        let handshake_started_at = tokio::time::Instant::now();
        let handshake_task = tokio::spawn(negotiate_start_tls_with_policy(
            stream,
            "127.0.0.1",
            TlsPolicy::RequireTls,
            Duration::from_secs(1),
            Duration::from_millis(25),
        ));
        wait_for_protocol_barrier(
            &client_hello_observed,
            "the server to receive the TLS ClientHello",
        )
        .await;
        assert_eq!(
            tokio::time::Instant::now(),
            handshake_started_at,
            "STARTTLS acceptance and ClientHello delivery must not consume virtual time"
        );
        tokio::time::advance(Duration::from_millis(25)).await;
        let handshake_timeout = handshake_task
            .await
            .expect("handshake timeout task should not panic")
            .err()
            .expect("silent TLS handshake should time out");
        assert_eq!(
            tokio::time::Instant::now().duration_since(handshake_started_at),
            Duration::from_millis(25),
            "a completed STARTTLS response must not consume the handshake deadline"
        );
        assert!(
            handshake_timeout
                .to_string()
                .contains("TLS handshake timed out")
        );
        server_task.await.expect("server task should complete");
    }
}
