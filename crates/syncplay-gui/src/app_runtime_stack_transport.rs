use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use syncplay_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;
use syncplay_protocol::decode_message_line;

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(in super::super) struct GuiQueuedSessionTransportHandle {
    queued_inbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
    queued_outbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
}

#[allow(dead_code)]
impl GuiQueuedSessionTransportHandle {
    pub(in super::super) fn push_inbound_protocol_line(&self, line: impl Into<String>) {
        self.push_inbound_protocol_lines([line.into()]);
    }

    pub(in super::super) fn push_inbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(lines);
    }

    pub(in super::super) fn drain_inbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(in super::super) fn push_outbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(lines);
    }

    pub(in super::super) fn drain_outbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(in super::super) fn clear_protocol_lines(&self) {
        self.queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

pub(in super::super) trait GuiSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String>;
}

#[allow(dead_code)]
pub(in super::super) struct GuiLoopbackSessionTransportDriver {
    echo_username: String,
}

#[allow(dead_code)]
impl GuiLoopbackSessionTransportDriver {
    pub(in super::super) fn new(echo_username: impl Into<String>) -> Self {
        Self {
            echo_username: echo_username.into(),
        }
    }

    fn json_string_literal(input: &str) -> Option<&str> {
        let mut characters = input.char_indices();
        match characters.next() {
            Some((_, '"')) => {}
            _ => return None,
        }

        let mut escaped = false;
        for (index, character) in characters {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => return Some(&input[..=index]),
                _ => {}
            }
        }
        None
    }

    fn chat_message_literal(line: &str) -> Option<&str> {
        let rest = line.strip_prefix("{\"Chat\":")?.strip_suffix('}')?;
        if rest.starts_with('"') {
            return Self::json_string_literal(rest);
        }

        let message_key = "\"message\":";
        let message_index = rest.find(message_key)?;
        let message_start = message_index + message_key.len();
        Self::json_string_literal(rest.get(message_start..)?)
    }

    fn translated_inbound_line(&self, outbound_line: &str) -> String {
        let Some(message_literal) = Self::chat_message_literal(outbound_line) else {
            return outbound_line.to_owned();
        };
        format!(
            r#"{{"Chat":{{"username":{:?},"message":{message_literal}}}}}"#,
            self.echo_username
        )
    }
}

impl GuiSessionTransportDriver for GuiLoopbackSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        let outbound_protocol_lines = transport.drain_outbound_protocol_lines();
        if outbound_protocol_lines.is_empty() {
            return Ok(());
        }
        transport.push_inbound_protocol_lines(
            outbound_protocol_lines
                .into_iter()
                .map(|line| self.translated_inbound_line(&line)),
        );
        Ok(())
    }
}

pub(in super::super) struct GuiTcpSessionTransportDriver {
    stream: Option<TcpStream>,
    pending_outbound_lines: VecDeque<Vec<u8>>,
    pending_outbound_offset: usize,
    inbound_buffer: Vec<u8>,
}

impl GuiTcpSessionTransportDriver {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
    const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);

    fn normalized_connect_host(host: &str) -> &str {
        host.strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host)
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

    fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = Self::connect_stream(host, port)?;
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("Session transport TCP nonblocking setup failed: {error}"))?;
        stream
            .set_nodelay(true)
            .map_err(|error| format!("Session transport TCP nodelay setup failed: {error}"))?;
        Ok(Self {
            stream: Some(stream),
            pending_outbound_lines: VecDeque::new(),
            pending_outbound_offset: 0,
            inbound_buffer: Vec::new(),
        })
    }

    pub(in super::super) fn connect_from_host_arg(host_arg: &str) -> Result<Self, String> {
        let (host, port) = parse_host_and_optional_port_from_host_arg_legacy_compatible(host_arg);
        let Some(port) = port.or(Some(8999)) else {
            return Err("Session transport TCP port resolution failed.".to_owned());
        };
        Self::connect(&host, port)
    }

    fn disconnect_with_error(&mut self, message: String) -> Result<(), String> {
        self.stream = None;
        self.pending_outbound_lines.clear();
        self.pending_outbound_offset = 0;
        self.inbound_buffer.clear();
        Err(message)
    }

    fn queue_outbound_lines(&mut self, transport: &GuiQueuedSessionTransportHandle) {
        for line in transport.drain_outbound_protocol_lines() {
            let mut encoded_line = line.into_bytes();
            encoded_line.extend_from_slice(b"\r\n");
            self.pending_outbound_lines.push_back(encoded_line);
        }
    }

    fn flush_outbound_lines(&mut self) -> Result<(), String> {
        while !self.pending_outbound_lines.is_empty() {
            let Some(stream) = self.stream.as_mut() else {
                return Ok(());
            };
            let Some(front) = self.pending_outbound_lines.front() else {
                break;
            };
            let front_len = front.len();
            let pending_slice = &front[self.pending_outbound_offset..];
            match stream.write(pending_slice) {
                Ok(0) => {
                    return self.disconnect_with_error(
                        "Session transport TCP connection closed while writing.".to_owned(),
                    );
                }
                Ok(written) => {
                    self.pending_outbound_offset += written;
                    if self.pending_outbound_offset >= front_len {
                        self.pending_outbound_lines.pop_front();
                        self.pending_outbound_offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return self.disconnect_with_error(format!(
                        "Session transport TCP write failed: {error}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn drain_inbound_lines(
        &mut self,
        transport: &GuiQueuedSessionTransportHandle,
    ) -> Result<(), String> {
        let mut read_buffer = [0_u8; 4096];
        let mut closed_by_server = false;
        loop {
            let Some(stream) = self.stream.as_mut() else {
                break;
            };
            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    self.stream = None;
                    closed_by_server = true;
                    break;
                }
                Ok(read_bytes) => self
                    .inbound_buffer
                    .extend_from_slice(&read_buffer[..read_bytes]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return self.disconnect_with_error(format!(
                        "Session transport TCP read failed: {error}"
                    ));
                }
            }
        }

        let mut complete_lines = Vec::new();
        while let Some(newline_index) = self.inbound_buffer.iter().position(|byte| *byte == b'\n') {
            let mut raw_line: Vec<u8> = self.inbound_buffer.drain(..=newline_index).collect();
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
            if line.is_empty() || decode_message_line(line).is_err() {
                continue;
            }
            complete_lines.push(line.to_owned());
        }
        if !complete_lines.is_empty() {
            transport.push_inbound_protocol_lines(complete_lines);
        }
        if closed_by_server && !self.inbound_buffer.is_empty() {
            self.inbound_buffer.clear();
            return Err(
                "Session transport TCP connection closed with an incomplete inbound line."
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl GuiSessionTransportDriver for GuiTcpSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        self.queue_outbound_lines(transport);
        self.flush_outbound_lines()?;
        self.drain_inbound_lines(transport)
    }
}
