use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use syncplay_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;
use syncplay_protocol::{
    ProtocolMessage, decode_message_line, decode_message_line_items, encode_message_line,
};

use super::handle::{GuiQueuedSessionTransportHandle, GuiSessionTransportDriver};

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

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiTcpSessionTlsNegotiationState {
    PendingRequest,
    AwaitingResponse,
    Disabled,
    Active,
}

pub(in crate::app) struct GuiTcpSessionTransportDriver {
    host: String,
    port: u16,
    transport: Option<GuiTcpSessionNetworkTransport>,
    pending_transport_outbound_lines: VecDeque<Vec<u8>>,
    pending_transport_outbound_offset: usize,
    pending_outbound_lines: VecDeque<Vec<u8>>,
    pending_outbound_offset: usize,
    inbound_buffer: Vec<u8>,
    inbound_idle_timeout: Duration,
    last_inbound_activity_at: Instant,
    tls_negotiation_state: GuiTcpSessionTlsNegotiationState,
    tls_client_config: Arc<ClientConfig>,
}

impl GuiTcpSessionTransportDriver {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
    const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);
    const INBOUND_IDLE_TIMEOUT: Duration = Duration::from_millis(12_500);

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

    fn ordered_connect_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        let normalized_host = Self::normalized_connect_host(host).trim();
        if normalized_host.is_empty() {
            return Err("Session transport TCP host resolution failed: host was empty.".to_owned());
        }

        let mut addresses = (normalized_host, port)
            .to_socket_addrs()
            .map_err(|error| {
                format!(
                    "Session transport TCP address resolution for {normalized_host}:{port} failed: {error}"
                )
            })?
            .collect::<Vec<_>>();
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

    fn connect_stream(host: &str, port: u16) -> Result<TcpStream, String> {
        let addresses = Self::ordered_connect_addresses(host, port)?;
        let deadline = Instant::now() + Self::CONNECT_TIMEOUT;
        let mut failures = Vec::new();

        for address in &addresses {
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(now);
            let timeout = remaining.min(Self::CONNECT_ATTEMPT_TIMEOUT);
            match TcpStream::connect_timeout(address, timeout) {
                Ok(stream) => return Ok(stream),
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

    fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = Self::connect_stream(host, port)?;
        Self::configure_connected_stream(&stream)?;
        Ok(Self {
            host: host.to_owned(),
            port,
            transport: Some(GuiTcpSessionNetworkTransport::Plain(stream)),
            pending_transport_outbound_lines: VecDeque::new(),
            pending_transport_outbound_offset: 0,
            pending_outbound_lines: VecDeque::new(),
            pending_outbound_offset: 0,
            inbound_buffer: Vec::new(),
            inbound_idle_timeout: Self::INBOUND_IDLE_TIMEOUT,
            last_inbound_activity_at: Instant::now(),
            tls_negotiation_state: GuiTcpSessionTlsNegotiationState::PendingRequest,
            tls_client_config: Self::default_tls_client_config(),
        })
    }

    pub(in crate::app) fn connect_from_host_arg(host_arg: &str) -> Result<Self, String> {
        let (host, port) = parse_host_and_optional_port_from_host_arg_legacy_compatible(host_arg);
        let Some(port) = port.or(Some(8999)) else {
            return Err("Session transport TCP port resolution failed.".to_owned());
        };
        Self::connect(&host, port)
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

    fn disconnect_with_error(&mut self, message: String) -> Result<(), String> {
        self.transport = None;
        self.pending_transport_outbound_lines.clear();
        self.pending_transport_outbound_offset = 0;
        self.pending_outbound_lines.clear();
        self.pending_outbound_offset = 0;
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
        let stream = Self::connect_stream(&self.host, self.port)?;
        Self::configure_connected_stream(&stream)?;
        self.transport = Some(GuiTcpSessionNetworkTransport::Plain(stream));
        self.pending_transport_outbound_lines.clear();
        self.pending_transport_outbound_offset = 0;
        self.pending_outbound_lines.clear();
        self.pending_outbound_offset = 0;
        self.inbound_buffer.clear();
        self.last_inbound_activity_at = Instant::now();
        self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::PendingRequest;
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
        for line in transport.drain_outbound_protocol_lines() {
            let mut encoded_line = line.into_bytes();
            encoded_line.extend_from_slice(b"\r\n");
            self.pending_outbound_lines.push_back(encoded_line);
        }
    }

    fn flush_queue(
        queue: &mut VecDeque<Vec<u8>>,
        offset: &mut usize,
        transport: &mut GuiTcpSessionNetworkTransport,
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

    fn flush_outbound_lines(&mut self) -> Result<(), String> {
        let Some(transport) = self.transport.as_mut() else {
            return Ok(());
        };
        if let Err(error) = Self::flush_queue(
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
        if let Err(error) = Self::flush_queue(
            &mut self.pending_outbound_lines,
            &mut self.pending_outbound_offset,
            transport,
            "Session transport TCP connection closed while writing.",
            "Session transport TCP write failed",
        ) {
            return self.disconnect_with_error(error);
        }
        Ok(())
    }

    fn next_complete_inbound_line(inbound_buffer: &mut Vec<u8>) -> Result<Option<String>, String> {
        while let Some(newline_index) = inbound_buffer.iter().position(|byte| *byte == b'\n') {
            let mut raw_line: Vec<u8> = inbound_buffer.drain(..=newline_index).collect();
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
            return Ok(());
        };
        match message {
            ProtocolMessage::Tls(tls_message) if tls_message.tls.start_tls.contains("true") => {
                let server_name = self.server_name()?;
                let transport = self.transport.take().ok_or_else(|| {
                    "Session transport TCP TLS upgrade failed because the socket was unavailable."
                        .to_owned()
                })?;
                self.transport =
                    Some(transport.upgrade_to_tls(self.tls_client_config.clone(), server_name)?);
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Active;
            }
            ProtocolMessage::Tls(_) => {
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Disabled;
            }
            _ => {
                self.tls_negotiation_state = GuiTcpSessionTlsNegotiationState::Disabled;
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
        self.queue_tls_negotiation_request_if_needed()?;
        self.queue_outbound_lines(transport);
        self.flush_outbound_lines()?;
        self.drain_inbound_lines(transport)?;
        if self.tls_negotiation_state != GuiTcpSessionTlsNegotiationState::AwaitingResponse {
            self.flush_outbound_lines()?;
        }
        Ok(())
    }

    fn reconnect(&mut self) -> Result<(), String> {
        self.reconnect_stream()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
