use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use sorotte_client_app::app_boundary::state::{
    TlsPolicy, parse_host_and_optional_port_from_host_arg_legacy_compatible,
};
use sorotte_protocol::{
    DEFAULT_MAX_PROTOCOL_LINE_BYTES, PingPayload, ProtocolMessage, StatePayload,
    decode_message_line, decode_message_line_items, encode_message_line,
};

use super::handle::{
    GuiOutboundProtocolDeliveryResult, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
};

pub(in crate::app::runtime_stack::transport) const MAX_INBOUND_PROTOCOL_LINE_BYTES: usize =
    // Server List snapshots aggregate per-user file metadata; media-match signatures are capped
    // per file, so a valid multi-user snapshot can exceed the base single-line protocol default.
    DEFAULT_MAX_PROTOCOL_LINE_BYTES * 8;

enum GuiTcpSessionNetworkTransport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl GuiTcpSessionNetworkTransport {
    fn plain_mut(&mut self) -> Option<&mut TcpStream> {
        match self {
            Self::Plain(stream) => Some(stream),
            Self::Tls(_) => None,
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }

    fn tls_handshake_in_progress(&self) -> bool {
        matches!(self, Self::Tls(stream) if stream.conn.is_handshaking())
    }

    fn upgrade_to_tls(
        self,
        tls_client_config: Arc<ClientConfig>,
        server_name: ServerName<'static>,
    ) -> Result<Self, String> {
        let Self::Plain(stream) = self else {
            return Err(
                "Session transport TCP TLS upgrade requested after TLS was already active."
                    .to_owned(),
            );
        };
        let client = ClientConnection::new(tls_client_config, server_name)
            .map_err(|error| format!("Session transport TCP TLS client setup failed: {error}"))?;
        Ok(Self::Tls(Box::new(StreamOwned::new(client, stream))))
    }
}

impl Write for GuiTcpSessionNetworkTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiTcpSessionTlsNegotiationState {
    PendingRequest,
    AwaitingResponse,
    Disabled,
    Active,
}

struct GuiTcpPendingOutboundFrame {
    token: Option<u64>,
    bytes: Vec<u8>,
    offset: usize,
}

type GuiDnsResolver = Box<dyn FnOnce(String, u16) -> io::Result<Vec<SocketAddr>> + Send + 'static>;

struct GuiDnsResolutionRequest {
    host: String,
    port: u16,
    resolver: GuiDnsResolver,
    shared_result: Arc<GuiDnsResolutionSharedResult>,
}

#[derive(Default)]
struct GuiDnsResolutionSharedResult {
    result: Mutex<Option<Result<Vec<SocketAddr>, String>>>,
    ready: Condvar,
}

struct GuiDnsPendingResolution {
    host: String,
    port: u16,
    shared_result: Arc<GuiDnsResolutionSharedResult>,
}

struct GuiDnsResolverService {
    request_tx: mpsc::SyncSender<GuiDnsResolutionRequest>,
    pending: Mutex<Option<GuiDnsPendingResolution>>,
}

impl GuiDnsResolverService {
    fn start() -> Result<Self, String> {
        let (request_tx, request_rx) = mpsc::sync_channel::<GuiDnsResolutionRequest>(1);
        thread::Builder::new()
            .name("sorotte-gui-dns-resolver".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (request.resolver)(request.host, request.port)
                    }))
                    .unwrap_or_else(|_| Err(io::Error::other("DNS resolver worker panicked")))
                    .map_err(|error| error.to_string());
                    *request
                        .shared_result
                        .result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
                    request.shared_result.ready.notify_all();
                }
            })
            .map_err(|error| {
                format!("Session transport DNS resolver worker spawn failed: {error}")
            })?;
        Ok(Self {
            request_tx,
            pending: Mutex::new(None),
        })
    }

    fn begin_resolution<R>(
        &self,
        host: String,
        port: u16,
        resolver: R,
    ) -> Result<Arc<GuiDnsResolutionSharedResult>, String>
    where
        R: FnOnce(String, u16) -> io::Result<Vec<SocketAddr>> + Send + 'static,
    {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = pending.as_ref() {
            if existing.host == host && existing.port == port {
                return Ok(existing.shared_result.clone());
            }
            let existing_finished = existing
                .shared_result
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some();
            if !existing_finished {
                return Err(format!(
                    "Session transport TCP address resolution for {host}:{port} cannot start because a different resolution is still in flight."
                ));
            }
            *pending = None;
        }
        let shared_result = Arc::new(GuiDnsResolutionSharedResult::default());
        let request = GuiDnsResolutionRequest {
            host: host.clone(),
            port,
            resolver: Box::new(resolver),
            shared_result: shared_result.clone(),
        };
        if let Err(error) = self.request_tx.try_send(request) {
            return Err(format!(
                "Session transport TCP address resolution worker is unavailable: {error}"
            ));
        }
        *pending = Some(GuiDnsPendingResolution {
            host,
            port,
            shared_result: shared_result.clone(),
        });
        Ok(shared_result)
    }

    fn clear_pending_if_same(&self, shared_result: &Arc<GuiDnsResolutionSharedResult>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(&pending.shared_result, shared_result))
        {
            *pending = None;
        }
    }
}

pub(in crate::app) struct GuiTcpSessionTransportDriver {
    host: String,
    port: u16,
    transport: Option<GuiTcpSessionNetworkTransport>,
    pending_transport_outbound_lines: VecDeque<Vec<u8>>,
    pending_transport_outbound_offset: usize,
    pending_outbound_lines: VecDeque<GuiTcpPendingOutboundFrame>,
    pending_outbound_liveness_lines: VecDeque<GuiTcpPendingOutboundFrame>,
    transport_handle: Option<GuiQueuedSessionTransportHandle>,
    inbound_buffer: Vec<u8>,
    inbound_idle_timeout: Duration,
    last_inbound_activity_at: Instant,
    tls_negotiation_state: GuiTcpSessionTlsNegotiationState,
    tls_policy: TlsPolicy,
    tls_response_started_at: Option<Instant>,
    tls_handshake_started_at: Option<Instant>,
    initial_hello_started_at: Option<Instant>,
    server_handshake_completed: bool,
    starttls_response_timeout: Duration,
    tls_handshake_timeout: Duration,
    initial_hello_timeout: Duration,
    tls_client_config: Arc<ClientConfig>,
    resolver_service: Arc<GuiDnsResolverService>,
}

impl GuiTcpSessionTransportDriver {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
    const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);
    const INBOUND_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
    const STARTTLS_RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);
    const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
    const INITIAL_HELLO_TIMEOUT: Duration = Duration::from_secs(10);

    fn normalized_connect_host(host: &str) -> &str {
        host.strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host)
    }

    pub(in crate::app::runtime_stack::transport) fn ensure_rustls_crypto_provider() {
        static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();
        RUSTLS_PROVIDER_INIT.get_or_init(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    fn default_tls_client_config() -> Arc<ClientConfig> {
        static TLS_CLIENT_CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
        TLS_CLIENT_CONFIG
            .get_or_init(|| {
                Self::ensure_rustls_crypto_provider();
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

    fn ordered_connect_addresses_with_deadline_using<R>(
        host: &str,
        port: u16,
        deadline: Instant,
        resolver_service: &GuiDnsResolverService,
        resolver: R,
    ) -> Result<Vec<SocketAddr>, String>
    where
        R: FnOnce(String, u16) -> io::Result<Vec<SocketAddr>> + Send + 'static,
    {
        let normalized_host = Self::normalized_connect_host(host).trim();
        if normalized_host.is_empty() {
            return Err("Session transport TCP host resolution failed: host was empty.".to_owned());
        }
        let normalized_host = normalized_host.to_owned();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "Session transport TCP address resolution for {normalized_host}:{port} timed out within {:?}.",
                Self::CONNECT_TIMEOUT,
            ));
        }
        let shared_result =
            resolver_service.begin_resolution(normalized_host.clone(), port, resolver)?;
        let result_guard = shared_result
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut result_guard, wait_result) = shared_result
            .ready
            .wait_timeout_while(result_guard, remaining, |result| result.is_none())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = result_guard.take();
        drop(result_guard);
        let mut addresses = match result {
            Some(Ok(addresses)) => {
                resolver_service.clear_pending_if_same(&shared_result);
                addresses
            }
            Some(Err(error)) => {
                resolver_service.clear_pending_if_same(&shared_result);
                return Err(format!(
                    "Session transport TCP address resolution for {normalized_host}:{port} failed: {error}"
                ));
            }
            None if wait_result.timed_out() => {
                return Err(format!(
                    "Session transport TCP address resolution for {normalized_host}:{port} timed out within {:?}.",
                    Self::CONNECT_TIMEOUT,
                ));
            }
            None => {
                return Err(format!(
                    "Session transport TCP address resolution worker for {normalized_host}:{port} did not publish a result."
                ));
            }
        };
        if addresses.is_empty() {
            return Err(format!(
                "Session transport TCP address resolution for {normalized_host}:{port} returned no addresses."
            ));
        }

        addresses.sort_by_key(|address| match address {
            SocketAddr::V4(_) => 0_u8,
            SocketAddr::V6(_) => 1_u8,
        });
        addresses.dedup();
        Ok(addresses)
    }

    fn connect_stream(
        host: &str,
        port: u16,
        resolver_service: &GuiDnsResolverService,
    ) -> Result<TcpStream, String> {
        let deadline = Instant::now() + Self::CONNECT_TIMEOUT;
        let addresses = Self::ordered_connect_addresses_with_deadline_using(
            host,
            port,
            deadline,
            resolver_service,
            |host, port| {
                (host.as_str(), port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect())
            },
        )?;
        let mut failures = Vec::new();

        for address in &addresses {
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(now);
            let timeout = remaining.min(Self::CONNECT_ATTEMPT_TIMEOUT);
            match TcpStream::connect_timeout(address, timeout) {
                Ok(stream) => {
                    Self::configure_connected_stream(&stream)?;
                    if Instant::now() > deadline {
                        return Err(format!(
                            "Session transport TCP connect to {host}:{port} exceeded the total {:?} deadline during socket configuration.",
                            Self::CONNECT_TIMEOUT,
                        ));
                    }
                    return Ok(stream);
                }
                Err(error) => failures.push(format!("{address}: {error}")),
            }
        }

        Err(format!(
            "Session transport TCP connect to {host}:{port} failed after trying {} resolved addresses within {:?}: {}",
            addresses.len(),
            Self::CONNECT_TIMEOUT,
            failures.join("; "),
        ))
    }

    fn configure_connected_stream(stream: &TcpStream) -> Result<(), String> {
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("Session transport TCP nonblocking setup failed: {error}"))?;
        stream
            .set_nodelay(true)
            .map_err(|error| format!("Session transport TCP nodelay setup failed: {error}"))?;
        Ok(())
    }

    fn connect(
        host: &str,
        port: u16,
        tls_policy: TlsPolicy,
        resolver_service: Arc<GuiDnsResolverService>,
    ) -> Result<Self, String> {
        let stream = Self::connect_stream(host, port, &resolver_service)?;
        let now = Instant::now();
        Ok(Self {
            host: host.to_owned(),
            port,
            transport: Some(GuiTcpSessionNetworkTransport::Plain(stream)),
            pending_transport_outbound_lines: VecDeque::new(),
            pending_transport_outbound_offset: 0,
            pending_outbound_lines: VecDeque::new(),
            pending_outbound_liveness_lines: VecDeque::new(),
            transport_handle: None,
            inbound_buffer: Vec::new(),
            inbound_idle_timeout: Self::INBOUND_IDLE_TIMEOUT,
            last_inbound_activity_at: now,
            tls_negotiation_state: if tls_policy == TlsPolicy::Plaintext {
                GuiTcpSessionTlsNegotiationState::Disabled
            } else {
                GuiTcpSessionTlsNegotiationState::PendingRequest
            },
            tls_policy,
            tls_response_started_at: (tls_policy != TlsPolicy::Plaintext).then_some(now),
            tls_handshake_started_at: None,
            initial_hello_started_at: (tls_policy == TlsPolicy::Plaintext).then_some(now),
            server_handshake_completed: false,
            starttls_response_timeout: Self::STARTTLS_RESPONSE_TIMEOUT,
            tls_handshake_timeout: Self::TLS_HANDSHAKE_TIMEOUT,
            initial_hello_timeout: Self::INITIAL_HELLO_TIMEOUT,
            tls_client_config: Self::default_tls_client_config(),
            resolver_service,
        })
    }

    #[cfg(test)]
    pub(in crate::app) fn connect_from_host_arg_with_tls_policy(
        host_arg: &str,
        tls_policy: TlsPolicy,
    ) -> Result<Self, String> {
        Self::connect_from_host_arg_with_tls_policy_and_resolver(
            host_arg,
            tls_policy,
            Arc::new(GuiDnsResolverService::start()?),
        )
    }

    fn connect_from_host_arg_with_tls_policy_and_resolver(
        host_arg: &str,
        tls_policy: TlsPolicy,
        resolver_service: Arc<GuiDnsResolverService>,
    ) -> Result<Self, String> {
        let (host, port) = parse_host_and_optional_port_from_host_arg_legacy_compatible(host_arg);
        let Some(port) = port.or(Some(8999)) else {
            return Err("Session transport TCP port resolution failed.".to_owned());
        };
        Self::connect(&host, port, tls_policy, resolver_service)
    }

    #[cfg(test)]
    pub(in crate::app::runtime_stack::transport) fn with_tls_client_config(
        mut self,
        tls_client_config: Arc<ClientConfig>,
    ) -> Self {
        self.tls_client_config = tls_client_config;
        self
    }

    #[cfg(test)]
    pub(in crate::app) fn with_inbound_idle_timeout(mut self, timeout: Duration) -> Self {
        self.inbound_idle_timeout = timeout;
        self
    }

    #[cfg(test)]
    pub(in crate::app::runtime_stack::transport) fn with_connection_phase_timeouts(
        mut self,
        starttls_response_timeout: Duration,
        tls_handshake_timeout: Duration,
        initial_hello_timeout: Duration,
    ) -> Self {
        self.starttls_response_timeout = starttls_response_timeout;
        self.tls_handshake_timeout = tls_handshake_timeout;
        self.initial_hello_timeout = initial_hello_timeout;
        self
    }

    fn set_server_handshake_completed(&mut self, completed: bool) {
        self.server_handshake_completed = completed;
        if completed {
            self.initial_hello_started_at = None;
        }
    }

    fn warn_prefer_tls_plaintext_fallback(
        transport_handle: &GuiQueuedSessionTransportHandle,
        reason: &str,
    ) {
        let warning = format!(
            "Security warning: {reason} The connection is continuing without encryption because TLS policy is PreferTls; credentials and session data may be exposed. Set tlsPolicy = RequireTls to refuse this connection."
        );
        eprintln!("warning: {warning}");
        transport_handle.push_transport_warning(warning);
    }

    fn fail_pending_outbound_deliveries(&mut self, message: &str) {
        let Some(transport_handle) = self.transport_handle.as_ref() else {
            return;
        };
        for frame in &self.pending_outbound_lines {
            if let Some(token) = frame.token {
                transport_handle.publish_outbound_protocol_delivery_result(
                    GuiOutboundProtocolDeliveryResult::FrameFailed {
                        token,
                        bytes_written: frame.offset,
                        message: message.to_owned(),
                    },
                );
            }
        }
        transport_handle.fail_pending_outbound_protocol_delivery(0, message.to_owned());
    }

    fn disconnect_with_error(&mut self, message: String) -> Result<(), String> {
        self.fail_pending_outbound_deliveries(&message);
        self.transport = None;
        self.pending_transport_outbound_lines.clear();
        self.pending_transport_outbound_offset = 0;
        self.pending_outbound_lines.clear();
        self.pending_outbound_liveness_lines.clear();
        self.inbound_buffer.clear();
        Err(message)
    }

    fn note_inbound_activity(&mut self) {
        self.last_inbound_activity_at = Instant::now();
    }

    fn disconnect_if_inbound_idle(&mut self) -> Result<(), String> {
        if self.transport.is_none()
            || self.last_inbound_activity_at.elapsed() < self.inbound_idle_timeout
        {
            return Ok(());
        }
        self.disconnect_with_error(format!(
            "Session transport TCP timed out after {:.1} seconds without inbound traffic.",
            self.inbound_idle_timeout.as_secs_f64()
        ))
    }

    fn reconnect_stream(&mut self) -> Result<(), String> {
        let reset_message =
            "Outbound protocol delivery was interrupted while reconnecting the TCP transport.";
        self.fail_pending_outbound_deliveries(reset_message);
        self.transport = None;
        self.pending_transport_outbound_lines.clear();
        self.pending_transport_outbound_offset = 0;
        self.pending_outbound_lines.clear();
        self.pending_outbound_liveness_lines.clear();
        self.inbound_buffer.clear();

        let stream = Self::connect_stream(&self.host, self.port, &self.resolver_service)?;
        self.transport = Some(GuiTcpSessionNetworkTransport::Plain(stream));
        let now = Instant::now();
        self.last_inbound_activity_at = now;
        self.tls_negotiation_state = if self.tls_policy == TlsPolicy::Plaintext {
            GuiTcpSessionTlsNegotiationState::Disabled
        } else {
            GuiTcpSessionTlsNegotiationState::PendingRequest
        };
        self.tls_response_started_at = (self.tls_policy != TlsPolicy::Plaintext).then_some(now);
        self.tls_handshake_started_at = None;
        self.initial_hello_started_at = (self.tls_policy == TlsPolicy::Plaintext).then_some(now);
        self.server_handshake_completed = false;
        Ok(())
    }

    fn enforce_connection_phase_deadlines(&mut self) -> Result<(), String> {
        if self.tls_negotiation_state == GuiTcpSessionTlsNegotiationState::AwaitingResponse
            && self
                .tls_response_started_at
                .is_some_and(|started| started.elapsed() >= self.starttls_response_timeout)
        {
            return Err(format!(
                "Session transport TCP STARTTLS response timed out after {:.1} seconds.",
                self.starttls_response_timeout.as_secs_f64()
            ));
        }

        if self.tls_negotiation_state == GuiTcpSessionTlsNegotiationState::Active {
            let handshaking = self
                .transport
                .as_ref()
                .is_some_and(GuiTcpSessionNetworkTransport::tls_handshake_in_progress);
            if handshaking
                && self
                    .tls_handshake_started_at
                    .is_some_and(|started| started.elapsed() >= self.tls_handshake_timeout)
            {
                return Err(format!(
                    "Session transport TCP TLS handshake timed out after {:.1} seconds.",
                    self.tls_handshake_timeout.as_secs_f64()
                ));
            }
            if !handshaking && self.initial_hello_started_at.is_none() {
                self.tls_handshake_started_at = None;
                self.initial_hello_started_at = Some(Instant::now());
            }
        }

        if !self.server_handshake_completed
            && self
                .initial_hello_started_at
                .is_some_and(|started| started.elapsed() >= self.initial_hello_timeout)
        {
            return Err(format!(
                "Session transport TCP initial Hello timed out after {:.1} seconds.",
                self.initial_hello_timeout.as_secs_f64()
            ));
        }
        Ok(())
    }

    fn queue_tls_negotiation_request_if_needed(&mut self) -> Result<(), String> {
        if self.tls_negotiation_state != GuiTcpSessionTlsNegotiationState::PendingRequest {
            return Ok(());
        }
        let mut encoded_line = encode_message_line(&ProtocolMessage::start_tls("send"))
            .map_err(|error| format!("Session transport TCP TLS request encode failed: {error}"))?
            .into_bytes();
        encoded_line.extend_from_slice(b"\r\n");
        self.pending_transport_outbound_lines
            .push_back(encoded_line);
        self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::AwaitingResponse;
        Ok(())
    }

    fn queue_outbound_lines(&mut self, transport: &GuiQueuedSessionTransportHandle) {
        if let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() {
            let mut encoded_line = delivery.line.into_bytes();
            encoded_line.extend_from_slice(b"\r\n");
            self.pending_outbound_lines
                .push_back(GuiTcpPendingOutboundFrame {
                    token: Some(delivery.token),
                    bytes: encoded_line,
                    offset: 0,
                });
        }
        for line in transport.drain_untracked_outbound_protocol_lines_for_driver() {
            let mut encoded_line = line.into_bytes();
            encoded_line.extend_from_slice(b"\r\n");
            self.pending_outbound_lines
                .push_back(GuiTcpPendingOutboundFrame {
                    token: None,
                    bytes: encoded_line,
                    offset: 0,
                });
        }
        Self::queue_outbound_liveness_line(&mut self.pending_outbound_liveness_lines, transport);
    }

    fn queue_outbound_liveness_line(
        pending_liveness_lines: &mut VecDeque<GuiTcpPendingOutboundFrame>,
        transport: &GuiQueuedSessionTransportHandle,
    ) {
        if !pending_liveness_lines.is_empty() {
            return;
        }
        let Some(line) = transport.take_outbound_liveness_protocol_line_for_driver() else {
            return;
        };
        let mut encoded_line = line.into_bytes();
        encoded_line.extend_from_slice(b"\r\n");
        pending_liveness_lines.push_back(GuiTcpPendingOutboundFrame {
            token: None,
            bytes: encoded_line,
            offset: 0,
        });
    }

    fn flush_transport_queue(
        queue: &mut VecDeque<Vec<u8>>,
        offset: &mut usize,
        transport: &mut impl Write,
        closed_message: &'static str,
        error_prefix: &'static str,
    ) -> Result<(), String> {
        while !queue.is_empty() {
            let Some(front) = queue.front() else {
                break;
            };
            let front_len = front.len();
            let pending_slice = &front[*offset..];
            match transport.write(pending_slice) {
                Ok(0) => return Err(closed_message.to_owned()),
                Ok(written) => {
                    *offset += written;
                    if *offset >= front_len {
                        queue.pop_front();
                        *offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("{error_prefix}: {error}")),
            }
        }
        Ok(())
    }

    fn flush_outbound_frame_queue(
        queue: &mut VecDeque<GuiTcpPendingOutboundFrame>,
        transport: &mut impl Write,
        mut frame_written: impl FnMut(u64),
    ) -> Result<(), String> {
        while let Some(front) = queue.front_mut() {
            let pending_slice = &front.bytes[front.offset..];
            match transport.write(pending_slice) {
                Ok(0) => {
                    return Err("Session transport TCP connection closed while writing.".to_owned());
                }
                Ok(written) => {
                    front.offset += written;
                    if front.offset >= front.bytes.len() {
                        let token = front.token;
                        queue.pop_front();
                        if let Some(token) = token {
                            frame_written(token);
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return Err(format!("Session transport TCP write failed: {error}"));
                }
            }
        }
        Ok(())
    }

    fn flush_ordered_protocol_frame_queues(
        pending_outbound_lines: &mut VecDeque<GuiTcpPendingOutboundFrame>,
        pending_liveness_lines: &mut VecDeque<GuiTcpPendingOutboundFrame>,
        transport: &mut impl Write,
        mut frame_written: impl FnMut(u64),
    ) -> Result<(), String> {
        if pending_liveness_lines
            .front()
            .is_some_and(|frame| frame.offset > 0)
        {
            Self::flush_outbound_frame_queue(pending_liveness_lines, transport, |_| {})?;
            if !pending_liveness_lines.is_empty() {
                return Ok(());
            }
        }

        Self::flush_outbound_frame_queue(pending_outbound_lines, transport, &mut frame_written)?;
        if !pending_outbound_lines.is_empty() {
            return Ok(());
        }
        Self::flush_outbound_frame_queue(pending_liveness_lines, transport, |_| {})
    }

    fn flush_outbound_lines(&mut self) -> Result<(), String> {
        let Some(transport) = self.transport.as_mut() else {
            return Ok(());
        };
        if let Err(error) = Self::flush_transport_queue(
            &mut self.pending_transport_outbound_lines,
            &mut self.pending_transport_outbound_offset,
            transport,
            "Session transport TCP connection closed while writing the TLS negotiation frame.",
            "Session transport TCP TLS negotiation write failed",
        ) {
            return self.disconnect_with_error(error);
        }
        if self.tls_negotiation_state == GuiTcpSessionTlsNegotiationState::AwaitingResponse {
            return Ok(());
        }
        let transport_handle = self.transport_handle.clone();
        if let Err(error) = Self::flush_ordered_protocol_frame_queues(
            &mut self.pending_outbound_lines,
            &mut self.pending_outbound_liveness_lines,
            transport,
            |token| {
                if let Some(transport_handle) = transport_handle.as_ref() {
                    transport_handle.publish_outbound_protocol_delivery_result(
                        GuiOutboundProtocolDeliveryResult::FrameWritten { token },
                    );
                }
            },
        ) {
            return self.disconnect_with_error(error);
        }
        Ok(())
    }

    fn inbound_protocol_line_too_long_error() -> String {
        format!(
            "Session transport TCP inbound protocol line exceeded {MAX_INBOUND_PROTOCOL_LINE_BYTES} bytes."
        )
    }

    fn raw_protocol_line_len(raw_line: &[u8]) -> usize {
        let without_lf = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        without_lf.strip_suffix(b"\r").unwrap_or(without_lf).len()
    }

    fn current_unterminated_inbound_line_len(inbound_buffer: &[u8]) -> usize {
        inbound_buffer
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(inbound_buffer.len(), |newline_index| {
                inbound_buffer.len().saturating_sub(newline_index + 1)
            })
    }

    fn next_complete_inbound_line(inbound_buffer: &mut Vec<u8>) -> Result<Option<String>, String> {
        while let Some(newline_index) = inbound_buffer.iter().position(|byte| *byte == b'\n') {
            let mut raw_line: Vec<u8> = inbound_buffer.drain(..=newline_index).collect();
            if Self::raw_protocol_line_len(&raw_line) > MAX_INBOUND_PROTOCOL_LINE_BYTES {
                return Err(Self::inbound_protocol_line_too_long_error());
            }
            if raw_line.last() == Some(&b'\n') {
                raw_line.pop();
            }
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
            if raw_line.is_empty() {
                continue;
            }
            let line = String::from_utf8(raw_line).map_err(|error| {
                format!("Session transport TCP received a non-UTF-8 line: {error}")
            })?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let items = decode_message_line_items(line).map_err(|error| {
                format!("Session transport TCP received an invalid protocol line: {error}")
            })?;
            match items.first().map(|item| &item.message) {
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    return Err(format!(
                        "Session transport TCP received an invalid protocol line: {error}"
                    ));
                }
                None => {
                    return Err(
                        "Session transport TCP received an invalid empty protocol line".to_owned(),
                    );
                }
            }
            return Ok(Some(line.to_owned()));
        }
        Ok(None)
    }

    fn server_name(&self) -> Result<ServerName<'static>, String> {
        let host = Self::normalized_connect_host(&self.host).trim().to_owned();
        ServerName::try_from(host.clone()).map_err(|error| {
            format!("Session transport TCP TLS server name '{host}' was invalid: {error}")
        })
    }

    fn drain_tls_negotiation_response_line(&mut self) -> Result<Option<String>, String> {
        let mut closed_by_server = false;
        while !self.inbound_buffer.contains(&b'\n') {
            let Some(stream) = self
                .transport
                .as_mut()
                .and_then(GuiTcpSessionNetworkTransport::plain_mut)
            else {
                break;
            };
            let mut byte = [0_u8; 1];
            match stream.read(&mut byte) {
                Ok(0) => {
                    self.transport = None;
                    closed_by_server = true;
                    break;
                }
                Ok(_) => {
                    self.inbound_buffer.push(byte[0]);
                    self.note_inbound_activity();
                    if byte[0] != b'\n'
                        && Self::current_unterminated_inbound_line_len(&self.inbound_buffer)
                            > MAX_INBOUND_PROTOCOL_LINE_BYTES
                    {
                        let message = Self::inbound_protocol_line_too_long_error();
                        let _ = self.disconnect_with_error(message.clone());
                        return Err(message);
                    }
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    let message =
                        format!("Session transport TCP TLS negotiation read failed: {error}");
                    let _ = self.disconnect_with_error(message.clone());
                    return Err(message);
                }
            }
        }

        let line = match Self::next_complete_inbound_line(&mut self.inbound_buffer) {
            Ok(line) => line,
            Err(error) => {
                let _ = self.disconnect_with_error(error.clone());
                return Err(error);
            }
        };
        if closed_by_server {
            if !self.inbound_buffer.is_empty() {
                self.inbound_buffer.clear();
                return Err(
                    "Session transport TCP connection closed with an incomplete inbound line."
                        .to_owned(),
                );
            }
            return Err("Session transport TCP connection closed by the server.".to_owned());
        }
        Ok(line)
    }

    fn handle_tls_negotiation_response_line(
        &mut self,
        transport_handle: &GuiQueuedSessionTransportHandle,
        line: String,
    ) -> Result<(), String> {
        let Ok(message) = decode_message_line(&line) else {
            return if self.tls_policy == TlsPolicy::RequireTls {
                Err("Session transport TCP received a malformed response instead of accepting required TLS.".to_owned())
            } else {
                Self::warn_prefer_tls_plaintext_fallback(
                    transport_handle,
                    "The server returned a malformed STARTTLS response.",
                );
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Disabled;
                self.tls_response_started_at = None;
                self.initial_hello_started_at = Some(Instant::now());
                Ok(())
            };
        };
        match message {
            ProtocolMessage::Tls(tls_message) if tls_message.tls.start_tls == "true" => {
                let server_name = self.server_name()?;
                let transport = self.transport.take().ok_or_else(|| {
                    "Session transport TCP TLS upgrade failed because the socket was unavailable."
                        .to_owned()
                })?;
                self.transport =
                    Some(transport.upgrade_to_tls(self.tls_client_config.clone(), server_name)?);
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Active;
                self.tls_response_started_at = None;
                self.tls_handshake_started_at = Some(Instant::now());
            }
            ProtocolMessage::Tls(_) => {
                if self.tls_policy == TlsPolicy::RequireTls {
                    return Err(
                        "Session transport TCP server refused required TLS negotiation.".to_owned(),
                    );
                }
                Self::warn_prefer_tls_plaintext_fallback(
                    transport_handle,
                    "The server declined STARTTLS.",
                );
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Disabled;
                self.tls_response_started_at = None;
                self.initial_hello_started_at = Some(Instant::now());
            }
            _ => {
                if self.tls_policy == TlsPolicy::RequireTls {
                    return Err(format!(
                        "Session transport TCP server returned unexpected {} message instead of accepting required TLS.",
                        message.kind()
                    ));
                }
                Self::warn_prefer_tls_plaintext_fallback(
                    transport_handle,
                    "The server returned an unexpected message instead of a STARTTLS response.",
                );
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Disabled;
                self.tls_response_started_at = None;
                self.initial_hello_started_at = Some(Instant::now());
                transport_handle.push_inbound_protocol_line(line);
            }
        }
        Ok(())
    }

    fn drain_inbound_lines(
        &mut self,
        transport_handle: &GuiQueuedSessionTransportHandle,
    ) -> Result<(), String> {
        while self.tls_negotiation_state == GuiTcpSessionTlsNegotiationState::AwaitingResponse {
            let Some(line) = self.drain_tls_negotiation_response_line()? else {
                return self.disconnect_if_inbound_idle();
            };
            self.handle_tls_negotiation_response_line(transport_handle, line)?;
        }

        let mut read_buffer = [0_u8; 4096];
        let mut closed_by_server = false;
        while let Some(transport) = self.transport.as_mut() {
            match transport.read(&mut read_buffer) {
                Ok(0) => {
                    self.transport = None;
                    closed_by_server = true;
                    break;
                }
                Ok(read_bytes) => {
                    self.inbound_buffer
                        .extend_from_slice(&read_buffer[..read_bytes]);
                    self.note_inbound_activity();
                    if Self::current_unterminated_inbound_line_len(&self.inbound_buffer)
                        > MAX_INBOUND_PROTOCOL_LINE_BYTES
                    {
                        return self
                            .disconnect_with_error(Self::inbound_protocol_line_too_long_error());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return self.disconnect_with_error(format!(
                        "Session transport TCP read failed: {error}"
                    ));
                }
            }
        }

        let mut complete_lines = Vec::new();
        loop {
            let next_line = match Self::next_complete_inbound_line(&mut self.inbound_buffer) {
                Ok(line) => line,
                Err(error) => return self.disconnect_with_error(error),
            };
            let Some(line) = next_line else {
                break;
            };
            complete_lines.push(line);
        }
        if !complete_lines.is_empty() {
            transport_handle.push_inbound_protocol_lines(complete_lines);
        }
        if closed_by_server {
            if !self.inbound_buffer.is_empty() {
                self.inbound_buffer.clear();
                return Err(
                    "Session transport TCP connection closed with an incomplete inbound line."
                        .to_owned(),
                );
            }
            return Err("Session transport TCP connection closed by the server.".to_owned());
        }
        self.disconnect_if_inbound_idle()
    }
}

impl GuiSessionTransportDriver for GuiTcpSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if self.transport_handle.is_none() {
            self.transport_handle = Some(transport.clone());
        }
        let result = (|| {
            self.queue_tls_negotiation_request_if_needed()?;
            self.enforce_connection_phase_deadlines()?;
            self.queue_outbound_lines(transport);
            self.flush_outbound_lines()?;
            self.drain_inbound_lines(transport)?;
            if self.tls_negotiation_state != GuiTcpSessionTlsNegotiationState::AwaitingResponse {
                self.flush_outbound_lines()?;
            }
            self.enforce_connection_phase_deadlines()?;
            Ok(())
        })();
        if let Err(error) = result {
            return self.disconnect_with_error(error);
        }
        Ok(())
    }

    fn reconnect(&mut self) -> Result<(), String> {
        self.reconnect_stream()
    }

    fn set_protocol_liveness_enabled(&mut self, enabled: bool) {
        self.set_server_handshake_completed(enabled);
    }
}

impl Drop for GuiTcpSessionTransportDriver {
    fn drop(&mut self) {
        self.fail_pending_outbound_deliveries(
            "Outbound protocol delivery was interrupted while dropping the TCP transport.",
        );
    }
}

pub(in crate::app) struct GuiThreadedTcpSessionTransportDriver {
    host_arg: String,
    tls_policy: TlsPolicy,
    transport_handle: Option<GuiQueuedSessionTransportHandle>,
    worker: Option<GuiThreadedTcpSessionTransportWorker>,
    liveness_enabled: Arc<AtomicBool>,
    resolver_service: Arc<GuiDnsResolverService>,
    worker_failed: bool,
}

struct GuiThreadedTcpSessionTransportWorker {
    stop_tx: mpsc::Sender<()>,
    pump_tx: mpsc::Sender<()>,
    pump_result_rx: mpsc::Receiver<Result<(), String>>,
    error_rx: mpsc::Receiver<String>,
    join_handle: Option<thread::JoinHandle<()>>,
    pump_request_in_flight: bool,
}

impl GuiThreadedTcpSessionTransportDriver {
    const WORKER_PUMP_INTERVAL: Duration = Duration::from_millis(25);
    const LIVENESS_INTERVAL: Duration = Duration::from_secs(1);
    const LIVENESS_INITIAL_DELAY: Duration = Duration::from_secs(2);

    pub(in crate::app) fn connect_from_host_arg_with_tls_policy(
        host_arg: &str,
        tls_policy: TlsPolicy,
    ) -> Result<Self, String> {
        let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(host_arg);
        if host.trim().is_empty() {
            return Err("Session transport TCP host resolution failed: host was empty.".to_owned());
        }
        Ok(Self {
            host_arg: host_arg.to_owned(),
            tls_policy,
            transport_handle: None,
            worker: None,
            liveness_enabled: Arc::new(AtomicBool::new(false)),
            resolver_service: Arc::new(GuiDnsResolverService::start()?),
            worker_failed: false,
        })
    }

    fn liveness_protocol_line() -> Result<String, String> {
        encode_message_line(&ProtocolMessage::state(
            StatePayload::new().with_ping(PingPayload::new()),
        ))
        .map_err(|error| format!("Session transport TCP liveness encode failed: {error}"))
    }

    fn start_worker(&mut self, transport: GuiQueuedSessionTransportHandle) -> Result<(), String> {
        let host_arg = self.host_arg.clone();
        let tls_policy = self.tls_policy;
        let liveness_line = Self::liveness_protocol_line()?;
        let liveness_enabled = self.liveness_enabled.clone();
        let resolver_service = self.resolver_service.clone();
        let (stop_tx, stop_rx) = mpsc::channel();
        let (pump_tx, pump_rx) = mpsc::channel();
        let (pump_result_tx, pump_result_rx) = mpsc::channel();
        let (error_tx, error_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("sorotte-gui-tcp-transport".to_owned())
            .spawn(move || {
                let mut driver =
                    match GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy_and_resolver(
                        &host_arg,
                        tls_policy,
                        resolver_service,
                    ) {
                        Ok(driver) => driver,
                        Err(error) => {
                            let _ = error_tx.send(error);
                            return;
                        }
                    };
                let mut liveness_was_enabled = false;
                let mut observed_outbound_activity =
                    transport.outbound_protocol_activity_revision();
                let mut next_liveness_at = Instant::now() + Self::LIVENESS_INITIAL_DELAY;
                loop {
                    let pump_requested = match pump_rx.recv_timeout(Self::WORKER_PUMP_INTERVAL) {
                        Ok(()) => true,
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            driver.fail_pending_outbound_deliveries(
                                "Outbound protocol delivery was interrupted while stopping the TCP transport worker.",
                            );
                            break;
                        }
                    };
                    if stop_rx.try_recv().is_ok() {
                        driver.fail_pending_outbound_deliveries(
                            "Outbound protocol delivery was interrupted while stopping the TCP transport worker.",
                        );
                        break;
                    }

                    let now = Instant::now();
                    let current_outbound_activity = transport.outbound_protocol_activity_revision();
                    if current_outbound_activity != observed_outbound_activity {
                        observed_outbound_activity = current_outbound_activity;
                        next_liveness_at = now + Self::LIVENESS_INITIAL_DELAY;
                    }

                    let liveness_is_enabled = liveness_enabled.load(Ordering::Relaxed);
                    driver.set_server_handshake_completed(liveness_is_enabled);
                    if liveness_is_enabled {
                        if !liveness_was_enabled {
                            next_liveness_at = now + Self::LIVENESS_INITIAL_DELAY;
                        }
                        if now >= next_liveness_at {
                            transport.push_outbound_liveness_protocol_line(liveness_line.clone());
                            next_liveness_at = now + Self::LIVENESS_INTERVAL;
                        }
                    } else {
                        next_liveness_at = now + Self::LIVENESS_INITIAL_DELAY;
                    }
                    liveness_was_enabled = liveness_is_enabled;

                    if let Err(error) = driver.pump(&transport) {
                        let _ = error_tx.send(error.clone());
                        if pump_requested {
                            let _ = pump_result_tx.send(Err(error));
                        }
                        break;
                    }
                    if pump_requested && pump_result_tx.send(Ok(())).is_err() {
                        driver.fail_pending_outbound_deliveries(
                            "Outbound protocol delivery was interrupted after the TCP transport owner disconnected.",
                        );
                        break;
                    }
                }
            })
            .map_err(|error| format!("Session transport TCP worker spawn failed: {error}"))?;

        self.worker = Some(GuiThreadedTcpSessionTransportWorker {
            stop_tx,
            pump_tx,
            pump_result_rx,
            error_rx,
            join_handle: Some(join_handle),
            pump_request_in_flight: false,
        });
        Ok(())
    }

    fn ensure_worker_started(
        &mut self,
        transport: &GuiQueuedSessionTransportHandle,
    ) -> Result<(), String> {
        if self.worker.is_some() || self.worker_failed {
            return Ok(());
        }
        self.start_worker(transport.clone())
    }

    fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
        if let Some(transport_handle) = self.transport_handle.as_ref() {
            transport_handle.clear_outbound_liveness_protocol_line();
            transport_handle.fail_pending_outbound_protocol_delivery(
                0,
                "Outbound protocol delivery was interrupted while stopping the TCP transport worker.",
            );
        }
    }

    fn take_worker_error(&mut self) -> Option<String> {
        let worker = self.worker.as_mut()?;
        match worker.error_rx.try_recv() {
            Ok(error) => {
                self.stop_worker();
                self.worker_failed = true;
                Some(error)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.stop_worker();
                self.worker_failed = true;
                Some("Session transport TCP worker exited unexpectedly.".to_owned())
            }
        }
    }
}

impl GuiSessionTransportDriver for GuiThreadedTcpSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if self.transport_handle.is_none() {
            self.transport_handle = Some(transport.clone());
        }
        if let Some(error) = self.take_worker_error() {
            transport.fail_pending_outbound_protocol_delivery(0, error.clone());
            return Err(error);
        }
        if let Err(error) = self.ensure_worker_started(transport) {
            transport.fail_pending_outbound_protocol_delivery(0, error.clone());
            return Err(error);
        }
        if self.worker_failed {
            // The owner already observed this generation's failure and owns
            // the reconnect deadline. Stay quiescent until reconnect()
            // installs a replacement worker instead of reporting a fresh
            // failure on every UI pump.
            return Ok(());
        }
        let pump_result = self
            .worker
            .as_mut()
            .ok_or_else(|| "Session transport TCP worker is unavailable.".to_owned())?
            .pump();
        if let Err(error) = pump_result {
            transport.fail_pending_outbound_protocol_delivery(0, error.clone());
            self.stop_worker();
            self.worker_failed = true;
            return Err(error);
        }
        if let Some(error) = self.take_worker_error() {
            transport.fail_pending_outbound_protocol_delivery(0, error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn set_protocol_liveness_enabled(&mut self, enabled: bool) {
        self.liveness_enabled.store(enabled, Ordering::Relaxed);
        if !enabled && let Some(transport_handle) = self.transport_handle.as_ref() {
            transport_handle.clear_outbound_liveness_protocol_line();
        }
    }

    fn reconnect(&mut self) -> Result<(), String> {
        self.set_protocol_liveness_enabled(false);
        self.stop_worker();
        if let Some(transport_handle) = self.transport_handle.as_ref() {
            transport_handle.clear_outbound_liveness_protocol_line();
        }
        self.worker_failed = false;
        if let Some(transport) = self.transport_handle.clone() {
            self.start_worker(transport)
        } else {
            Ok(())
        }
    }
}

impl GuiThreadedTcpSessionTransportWorker {
    fn pump(&mut self) -> Result<(), String> {
        if self.pump_request_in_flight {
            match self.pump_result_rx.try_recv() {
                Ok(result) => {
                    self.pump_request_in_flight = false;
                    result?;
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("Session transport TCP worker exited unexpectedly.".to_owned());
                }
            }
        }
        self.pump_tx
            .send(())
            .map_err(|_| "Session transport TCP worker exited unexpectedly.".to_owned())?;
        self.pump_request_in_flight = true;
        Ok(())
    }

    fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(join_handle) = self
            .join_handle
            .take()
            .filter(|handle| handle.is_finished())
        {
            let _ = join_handle.join();
        }
    }
}

impl Drop for GuiThreadedTcpSessionTransportDriver {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

#[cfg(test)]
mod tests {
    use super::super::handle::GuiOutboundProtocolDelivery;
    use super::*;

    #[test]
    fn address_resolution_is_bounded_by_the_total_connect_deadline() {
        let resolver_service =
            GuiDnsResolverService::start().expect("test resolver service should start");
        let timeout = Duration::from_millis(25);
        let started_at = Instant::now();

        let error = GuiTcpSessionTransportDriver::ordered_connect_addresses_with_deadline_using(
            "resolver.example",
            8999,
            started_at + timeout,
            &resolver_service,
            |_host, _port| {
                thread::sleep(Duration::from_millis(200));
                Ok(Vec::new())
            },
        )
        .expect_err("a stalled resolver must time out");

        assert!(error.contains("address resolution"));
        assert!(error.contains("timed out"));
        assert!(started_at.elapsed() < Duration::from_millis(150));
    }

    #[test]
    fn timed_out_resolution_is_reused_by_the_next_attempt() {
        let resolver_service =
            GuiDnsResolverService::start().expect("test resolver service should start");
        let first_deadline = Instant::now() + Duration::from_millis(25);

        GuiTcpSessionTransportDriver::ordered_connect_addresses_with_deadline_using(
            "resolver.example",
            8999,
            first_deadline,
            &resolver_service,
            move |_host, _port| {
                thread::sleep(Duration::from_millis(150));
                Ok(vec!["127.0.0.1:8999".parse().expect("test address")])
            },
        )
        .expect_err("the first stalled resolution should time out");

        let duplicate_ran = Arc::new(AtomicBool::new(false));
        let worker_duplicate_ran = duplicate_ran.clone();
        let addresses =
            GuiTcpSessionTransportDriver::ordered_connect_addresses_with_deadline_using(
                "resolver.example",
                8999,
                Instant::now() + Duration::from_millis(250),
                &resolver_service,
                move |_host, _port| {
                    worker_duplicate_ran.store(true, Ordering::Release);
                    Ok(Vec::new())
                },
            )
            .expect("the next attempt should await and reuse the in-flight resolution");

        assert_eq!(
            addresses,
            vec!["127.0.0.1:8999".parse().expect("test address")]
        );
        assert!(!duplicate_ran.load(Ordering::Acquire));
    }

    #[test]
    fn threaded_transport_construction_does_not_resolve_or_connect_on_the_owner_thread() {
        let started_at = Instant::now();

        let driver = GuiThreadedTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
            "resolver-will-run-in-worker.invalid:8999",
            TlsPolicy::PreferTls,
        )
        .expect("threaded transport construction should only validate the host argument");

        assert!(started_at.elapsed() < Duration::from_millis(50));
        drop(driver);
    }

    struct PartialThenErrorWriter {
        prefix_bytes: usize,
        calls: usize,
        written: Vec<u8>,
    }

    struct PartialThenWouldBlockWriter {
        prefix_bytes: usize,
        calls: usize,
        written: Vec<u8>,
    }

    impl Write for PartialThenWouldBlockWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.calls > 1 {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            let written = self.prefix_bytes.min(buffer.len());
            self.written.extend_from_slice(&buffer[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Write for PartialThenErrorWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.calls > 1 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "scripted write failure",
                ));
            }
            let written = self.prefix_bytes.min(buffer.len());
            self.written.extend_from_slice(&buffer[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn next_complete_inbound_line_accepts_valid_prefix_before_unknown_command() {
        let mut inbound_buffer = br#"{"Set":{"features":{"chat":true}},"Bogus":{"x":1}}"#.to_vec();
        inbound_buffer.push(b'\n');

        let line = GuiTcpSessionTransportDriver::next_complete_inbound_line(&mut inbound_buffer)
            .expect("mixed batched line should not fail transport pre-validation")
            .expect("mixed batched line should be returned");

        assert!(line.contains("\"Set\""));
        assert!(line.contains("\"Bogus\""));
        assert!(inbound_buffer.is_empty());
    }

    #[test]
    fn partial_frame_write_is_failed_without_ack_and_can_be_retried_in_full() {
        let transport = GuiQueuedSessionTransportHandle::default();
        let line = r#"{"Chat":"retry me"}"#;
        transport
            .try_push_outbound_protocol_delivery(GuiOutboundProtocolDelivery::new(41, line))
            .expect("first tracked delivery should fit");
        let delivery = transport
            .take_outbound_protocol_delivery_for_driver()
            .expect("driver should claim the tracked delivery");
        let mut bytes = delivery.line.into_bytes();
        bytes.extend_from_slice(b"\r\n");
        let expected_frame = bytes.clone();
        let mut queue = VecDeque::from([GuiTcpPendingOutboundFrame {
            token: Some(delivery.token),
            bytes,
            offset: 0,
        }]);
        let mut writer = PartialThenErrorWriter {
            prefix_bytes: 7,
            calls: 0,
            written: Vec::new(),
        };
        let mut written_tokens = Vec::new();

        let error = GuiTcpSessionTransportDriver::flush_outbound_frame_queue(
            &mut queue,
            &mut writer,
            |token| written_tokens.push(token),
        )
        .expect_err("the scripted second write should fail");

        assert!(error.contains("scripted write failure"));
        assert_eq!(writer.written, expected_frame[..7]);
        assert_eq!(queue.front().map(|frame| frame.offset), Some(7));
        assert!(written_tokens.is_empty());
        transport.publish_outbound_protocol_delivery_result(
            GuiOutboundProtocolDeliveryResult::FrameFailed {
                token: 41,
                bytes_written: 7,
                message: error,
            },
        );
        let blocked = GuiOutboundProtocolDelivery::new(42, line);
        assert_eq!(
            transport.try_push_outbound_protocol_delivery(blocked.clone()),
            Err(blocked),
            "the tracked slot must remain occupied until the owner drains its result"
        );
        assert_eq!(
            transport.drain_outbound_protocol_delivery_results(),
            vec![GuiOutboundProtocolDeliveryResult::FrameFailed {
                token: 41,
                bytes_written: 7,
                message: "Session transport TCP write failed: scripted write failure".to_owned(),
            }]
        );

        transport
            .try_push_outbound_protocol_delivery(GuiOutboundProtocolDelivery::new(42, line))
            .expect("replacement generation should accept the retry");
        let retry = transport
            .take_outbound_protocol_delivery_for_driver()
            .expect("driver should claim the retry");
        let mut retry_bytes = retry.line.into_bytes();
        retry_bytes.extend_from_slice(b"\r\n");
        let mut retry_queue = VecDeque::from([GuiTcpPendingOutboundFrame {
            token: Some(retry.token),
            bytes: retry_bytes,
            offset: 0,
        }]);
        let mut replacement_wire = Vec::new();
        GuiTcpSessionTransportDriver::flush_outbound_frame_queue(
            &mut retry_queue,
            &mut replacement_wire,
            |token| {
                transport.publish_outbound_protocol_delivery_result(
                    GuiOutboundProtocolDeliveryResult::FrameWritten { token },
                );
            },
        )
        .expect("replacement transport should write the complete frame");

        assert!(retry_queue.is_empty());
        assert_eq!(replacement_wire, expected_frame);
        assert_eq!(
            transport.drain_outbound_protocol_delivery_results(),
            vec![GuiOutboundProtocolDeliveryResult::FrameWritten { token: 42 }]
        );
    }

    #[test]
    fn blocked_tcp_writer_keeps_only_one_pending_and_one_latest_liveness_state() {
        let transport = GuiQueuedSessionTransportHandle::default();
        let mut pending_liveness_lines = VecDeque::new();

        transport.push_outbound_liveness_protocol_line("first");
        GuiTcpSessionTransportDriver::queue_outbound_liveness_line(
            &mut pending_liveness_lines,
            &transport,
        );
        transport.push_outbound_liveness_protocol_line("second");
        transport.push_outbound_liveness_protocol_line("latest");
        GuiTcpSessionTransportDriver::queue_outbound_liveness_line(
            &mut pending_liveness_lines,
            &transport,
        );

        assert_eq!(pending_liveness_lines.len(), 1);
        assert_eq!(pending_liveness_lines[0].bytes, b"first\r\n");

        pending_liveness_lines.clear();
        GuiTcpSessionTransportDriver::queue_outbound_liveness_line(
            &mut pending_liveness_lines,
            &transport,
        );
        assert_eq!(pending_liveness_lines.len(), 1);
        assert_eq!(pending_liveness_lines[0].bytes, b"latest\r\n");
        assert!(
            transport
                .take_outbound_liveness_protocol_line_for_driver()
                .is_none()
        );
    }

    #[test]
    fn partially_written_liveness_frame_finishes_before_new_reliable_frame() {
        let liveness = b"liveness\r\n".to_vec();
        let reliable = b"reliable\r\n".to_vec();
        let mut pending_liveness_lines = VecDeque::from([GuiTcpPendingOutboundFrame {
            token: None,
            bytes: liveness.clone(),
            offset: 0,
        }]);
        let mut pending_outbound_lines = VecDeque::new();
        let mut blocked_writer = PartialThenWouldBlockWriter {
            prefix_bytes: 4,
            calls: 0,
            written: Vec::new(),
        };

        GuiTcpSessionTransportDriver::flush_ordered_protocol_frame_queues(
            &mut pending_outbound_lines,
            &mut pending_liveness_lines,
            &mut blocked_writer,
            |_| {},
        )
        .expect("WouldBlock should retain the partial liveness frame");
        assert_eq!(pending_liveness_lines[0].offset, 4);

        pending_outbound_lines.push_back(GuiTcpPendingOutboundFrame {
            token: Some(7),
            bytes: reliable.clone(),
            offset: 0,
        });
        let mut resumed_wire = Vec::new();
        let mut written_tokens = Vec::new();
        GuiTcpSessionTransportDriver::flush_ordered_protocol_frame_queues(
            &mut pending_outbound_lines,
            &mut pending_liveness_lines,
            &mut resumed_wire,
            |token| written_tokens.push(token),
        )
        .expect("resumed writer should finish both complete frames");

        let mut complete_wire = blocked_writer.written;
        complete_wire.extend(resumed_wire);
        assert_eq!(complete_wire, [liveness, reliable].concat());
        assert_eq!(written_tokens, vec![7]);
        assert!(pending_liveness_lines.is_empty());
        assert!(pending_outbound_lines.is_empty());
    }

    #[test]
    fn partially_written_reliable_frame_finishes_before_liveness_frame() {
        let reliable = b"reliable\r\n".to_vec();
        let liveness = b"liveness\r\n".to_vec();
        let mut pending_outbound_lines = VecDeque::from([GuiTcpPendingOutboundFrame {
            token: Some(7),
            bytes: reliable.clone(),
            offset: 0,
        }]);
        let mut pending_liveness_lines = VecDeque::from([GuiTcpPendingOutboundFrame {
            token: None,
            bytes: liveness.clone(),
            offset: 0,
        }]);
        let mut blocked_writer = PartialThenWouldBlockWriter {
            prefix_bytes: 4,
            calls: 0,
            written: Vec::new(),
        };
        let mut written_tokens = Vec::new();

        GuiTcpSessionTransportDriver::flush_ordered_protocol_frame_queues(
            &mut pending_outbound_lines,
            &mut pending_liveness_lines,
            &mut blocked_writer,
            |token| written_tokens.push(token),
        )
        .expect("WouldBlock should retain the partial reliable frame");
        assert_eq!(pending_outbound_lines[0].offset, 4);
        assert_eq!(pending_liveness_lines[0].offset, 0);
        assert!(written_tokens.is_empty());

        let mut resumed_wire = Vec::new();
        GuiTcpSessionTransportDriver::flush_ordered_protocol_frame_queues(
            &mut pending_outbound_lines,
            &mut pending_liveness_lines,
            &mut resumed_wire,
            |token| written_tokens.push(token),
        )
        .expect("resumed writer should finish both complete frames");

        let mut complete_wire = blocked_writer.written;
        complete_wire.extend(resumed_wire);
        assert_eq!(complete_wire, [reliable, liveness].concat());
        assert_eq!(written_tokens, vec![7]);
        assert!(pending_outbound_lines.is_empty());
        assert!(pending_liveness_lines.is_empty());
    }
}
