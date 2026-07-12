use super::*;
use std::{
    future::Future,
    net::SocketAddr,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::mpsc::error::TrySendError;

type AcceptedClient = io::Result<(TcpStream, SocketAddr)>;

#[derive(Clone, PartialEq, Eq)]
enum ClientOutboundEvent {
    ReliableLine(String),
    PeriodicStateLine(String),
    TransportAction(ServerTransportAction),
}

impl std::fmt::Debug for ClientOutboundEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReliableLine(line) => formatter
                .debug_struct("ReliableLine")
                .field("line_bytes", &line.len())
                .finish(),
            Self::PeriodicStateLine(line) => formatter
                .debug_struct("PeriodicStateLine")
                .field("line_bytes", &line.len())
                .finish(),
            Self::TransportAction(action) => formatter
                .debug_tuple("TransportAction")
                .field(action)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct PeriodicStateUpdate {
    generation: u64,
    line: String,
}

impl std::fmt::Debug for PeriodicStateUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeriodicStateUpdate")
            .field("generation", &self.generation)
            .field("line_bytes", &self.line.len())
            .finish()
    }
}

#[derive(Debug, Default)]
struct PeriodicStateQueueState {
    latest_sent_generation: u64,
    latest_delivered_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientEventSendOutcome {
    Sent,
    Coalesced,
    Closed,
    Overloaded,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientEventSender {
    // Protocol control lines are bounded and reliable until an explicit
    // overload disconnect. Rare transport actions use their own reliable lane,
    // while periodic state uses a single replaceable watch slot.
    reliable_lines: Sender<String>,
    transport_actions: UnboundedSender<ServerTransportAction>,
    periodic_state: watch::Sender<Option<PeriodicStateUpdate>>,
    periodic_state_queue: Arc<StdMutex<PeriodicStateQueueState>>,
    overload_close: watch::Sender<Option<usize>>,
    overload_signalled: Arc<AtomicBool>,
    metrics: ServerOutboundBackpressureMetrics,
}

pub(crate) struct ClientEventReceiver {
    reliable_lines: Receiver<String>,
    transport_actions: tokio::sync::mpsc::UnboundedReceiver<ServerTransportAction>,
    periodic_state: watch::Receiver<Option<PeriodicStateUpdate>>,
    periodic_state_queue: Arc<StdMutex<PeriodicStateQueueState>>,
    overload_close: watch::Receiver<Option<usize>>,
    metrics: ServerOutboundBackpressureMetrics,
}

pub(crate) type SharedClientEventSenders = Arc<Mutex<BTreeMap<String, ClientEventSender>>>;

pub(crate) fn client_event_queue(
    metrics: ServerOutboundBackpressureMetrics,
) -> (ClientEventSender, ClientEventReceiver) {
    let (reliable_tx, reliable_rx) = channel(CLIENT_OUTBOUND_QUEUE_CAPACITY);
    let (transport_tx, transport_rx) = tokio::sync::mpsc::unbounded_channel();
    let (periodic_tx, periodic_rx) = watch::channel(None);
    let (overload_tx, overload_rx) = watch::channel(None);
    let periodic_state_queue = Arc::new(StdMutex::new(PeriodicStateQueueState::default()));
    (
        ClientEventSender {
            reliable_lines: reliable_tx,
            transport_actions: transport_tx,
            periodic_state: periodic_tx,
            periodic_state_queue: periodic_state_queue.clone(),
            overload_close: overload_tx,
            overload_signalled: Arc::new(AtomicBool::new(false)),
            metrics: metrics.clone(),
        },
        ClientEventReceiver {
            reliable_lines: reliable_rx,
            transport_actions: transport_rx,
            periodic_state: periodic_rx,
            periodic_state_queue,
            overload_close: overload_rx,
            metrics,
        },
    )
}

impl ClientEventSender {
    async fn send(&self, event: ClientOutboundEvent) -> ClientEventSendOutcome {
        if self.overload_signalled.load(Ordering::Acquire) {
            self.metrics.dropped();
            return ClientEventSendOutcome::Overloaded;
        }
        match event {
            ClientOutboundEvent::ReliableLine(line) => self.send_reliable_line(line).await,
            ClientOutboundEvent::PeriodicStateLine(line) => self.send_periodic_state(line),
            ClientOutboundEvent::TransportAction(action) => self.send_transport_action(action),
        }
    }

    async fn send_reliable_line(&self, line: String) -> ClientEventSendOutcome {
        match self.reliable_lines.try_send(line) {
            Ok(()) => {
                self.metrics.enqueued();
                ClientEventSendOutcome::Sent
            }
            Err(TrySendError::Closed(_line)) => {
                self.metrics.closed();
                self.metrics.dropped();
                ClientEventSendOutcome::Closed
            }
            Err(TrySendError::Full(line)) => {
                self.metrics.full();
                let grace = std::time::Duration::from_millis(CLIENT_OUTBOUND_OVERLOAD_GRACE_MILLIS);
                match time::timeout(grace, self.reliable_lines.send(line)).await {
                    Ok(Ok(())) => {
                        self.metrics.enqueued();
                        ClientEventSendOutcome::Sent
                    }
                    Ok(Err(_closed)) => {
                        self.metrics.closed();
                        self.metrics.dropped();
                        ClientEventSendOutcome::Closed
                    }
                    Err(_) => {
                        self.metrics.dropped();
                        self.signal_overload();
                        ClientEventSendOutcome::Overloaded
                    }
                }
            }
        }
    }

    fn send_periodic_state(&self, line: String) -> ClientEventSendOutcome {
        let mut queue = self
            .periodic_state_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = queue.latest_sent_generation.saturating_add(1);
        let was_pending = queue.latest_sent_generation > queue.latest_delivered_generation;
        let update = PeriodicStateUpdate { generation, line };
        if self.periodic_state.send(Some(update)).is_err() {
            self.metrics.closed();
            self.metrics.dropped();
            return ClientEventSendOutcome::Closed;
        }
        queue.latest_sent_generation = generation;
        if was_pending {
            self.metrics.coalesced();
            ClientEventSendOutcome::Coalesced
        } else {
            self.metrics.enqueued();
            ClientEventSendOutcome::Sent
        }
    }

    fn send_transport_action(&self, action: ServerTransportAction) -> ClientEventSendOutcome {
        match self.transport_actions.send(action) {
            Ok(()) => {
                self.metrics.enqueued();
                ClientEventSendOutcome::Sent
            }
            Err(_) => {
                self.metrics.closed();
                self.metrics.dropped();
                ClientEventSendOutcome::Closed
            }
        }
    }

    fn signal_overload(&self) {
        if !self.overload_signalled.swap(true, Ordering::AcqRel) {
            self.metrics.overload_disconnect();
            let queue_depth =
                CLIENT_OUTBOUND_QUEUE_CAPACITY.saturating_sub(self.reliable_lines.capacity());
            let _ = self.overload_close.send(Some(queue_depth));
        }
    }
}

impl ClientEventReceiver {
    fn periodic_state_delivered(&self, generation: u64) {
        let mut queue = self
            .periodic_state_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_pending = queue.latest_sent_generation > queue.latest_delivered_generation;
        queue.latest_delivered_generation = queue.latest_delivered_generation.max(generation);
        let is_pending = queue.latest_sent_generation > queue.latest_delivered_generation;
        if was_pending && !is_pending {
            self.metrics.dequeued();
        }
    }

    fn close_and_record_discarded(&mut self) {
        self.reliable_lines.close();
        self.transport_actions.close();
        let mut discarded = self.reliable_lines.len() + self.transport_actions.len();
        let mut periodic_queue = self
            .periodic_state_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if periodic_queue.latest_sent_generation > periodic_queue.latest_delivered_generation {
            discarded += 1;
            periodic_queue.latest_delivered_generation = periodic_queue.latest_sent_generation;
        }
        self.metrics.discarded(discarded);
    }

    #[cfg(test)]
    pub(crate) async fn receive_reliable_line_for_test(&mut self) -> Option<String> {
        let line = self.reliable_lines.recv().await;
        if line.is_some() {
            self.metrics.dequeued();
        }
        line
    }

    #[cfg(test)]
    pub(crate) async fn receive_periodic_state_for_test(&mut self) -> Option<String> {
        self.periodic_state.changed().await.ok()?;
        let update = self.periodic_state.borrow_and_update().clone()?;
        self.periodic_state_delivered(update.generation);
        Some(update.line)
    }

    #[cfg(test)]
    pub(crate) fn overload_queue_depth_for_test(&self) -> Option<usize> {
        *self.overload_close.borrow()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ServerNetworkClientSessionTimeouts {
    pre_hello: std::time::Duration,
    tls_handshake: std::time::Duration,
    write: std::time::Duration,
}

struct ServerNetworkClientIdentity {
    peer_ip: Option<String>,
    client_id: String,
}

impl ServerNetworkClientSessionTimeouts {
    pub(crate) fn new(
        pre_hello: std::time::Duration,
        tls_handshake: std::time::Duration,
        write: std::time::Duration,
    ) -> Self {
        Self {
            pre_hello,
            tls_handshake,
            write,
        }
    }

    fn production_with_pre_hello(pre_hello: std::time::Duration) -> Self {
        Self::new(
            pre_hello,
            std::time::Duration::from_secs_f64(TLS_HANDSHAKE_TIMEOUT_SECONDS),
            std::time::Duration::from_secs_f64(SERVER_WRITE_TIMEOUT_SECONDS),
        )
    }
}

pub(crate) async fn dispatch_outbound_lines_to_clients(
    client_event_senders: &SharedClientEventSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
) {
    for line in outbound_lines {
        let event = match line.delivery {
            ServerOutboundDelivery::Reliable => ClientOutboundEvent::ReliableLine(line.line),
            ServerOutboundDelivery::CoalesciblePeriodicState => {
                ClientOutboundEvent::PeriodicStateLine(line.line)
            }
        };
        dispatch_client_event(client_event_senders, &line.client_id, event).await;
    }
}

async fn dispatch_client_event(
    client_event_senders: &SharedClientEventSenders,
    client_id: &str,
    event: ClientOutboundEvent,
) {
    let event_sender = {
        let senders = client_event_senders.lock().await;
        senders.get(client_id).cloned()
    };
    let Some(event_sender) = event_sender else {
        return;
    };
    match event_sender.send(event).await {
        ClientEventSendOutcome::Sent | ClientEventSendOutcome::Coalesced => {}
        ClientEventSendOutcome::Closed | ClientEventSendOutcome::Overloaded => {
            let mut senders = client_event_senders.lock().await;
            senders.remove(client_id);
        }
    }
}

async fn dispatch_transport_actions_to_clients(
    client_event_senders: &SharedClientEventSenders,
    transport_actions: &[DirectedTransportAction],
) {
    for action in transport_actions {
        dispatch_client_event(
            client_event_senders,
            &action.client_id,
            ClientOutboundEvent::TransportAction(action.action.clone()),
        )
        .await;
    }
}

#[cfg(test)]
pub(crate) async fn prune_finished_session_tasks(session_tasks: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < session_tasks.len() {
        if session_tasks[index].is_finished() {
            let task = session_tasks.swap_remove(index);
            if let Err(source) = task.await {
                eprintln!("Sorotte server client session task ended unexpectedly: {source}");
            }
        } else {
            index += 1;
        }
    }
}

fn dispatch_transport_actions_to_sink(
    transport_action_sink: Option<&UnboundedSender<DirectedTransportAction>>,
    transport_actions: &[DirectedTransportAction],
) {
    if let Some(transport_action_sink) = transport_action_sink {
        for action in transport_actions {
            let _ = transport_action_sink.send(action.clone());
        }
    }
}

fn transport_actions_close_client(
    transport_actions: &[DirectedTransportAction],
    client_id: &str,
) -> bool {
    transport_actions.iter().any(|action| {
        action.client_id == client_id && action.action == ServerTransportAction::Close
    })
}

pub(crate) async fn write_network_line_to_stream<S>(stream: &mut S, line: &str) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    Ok(())
}

fn timeout_io_error(operation: &str, timeout_duration: std::time::Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "{operation} timed out after {:.1} seconds",
            timeout_duration.as_secs_f64()
        ),
    )
}

pub(crate) async fn read_network_line_from_stream_with_buffer<S>(
    stream: &mut S,
    bytes: &mut Vec<u8>,
) -> io::Result<Option<String>>
where
    S: AsyncRead + Unpin,
{
    const READ_CHUNK_BYTES: usize = 8 * 1024;
    let mut newline_search_start = 0;

    loop {
        if let Some(relative_newline_index) = bytes[newline_search_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let newline_index = newline_search_start + relative_newline_index;
            let payload_bytes =
                newline_index - usize::from(newline_index > 0 && bytes[newline_index - 1] == b'\r');
            if payload_bytes > MAX_PROTOCOL_LINE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    PROTOCOL_LINE_TOO_LONG_ERROR,
                ));
            }

            let mut line_bytes = bytes.drain(..=newline_index).collect::<Vec<_>>();
            line_bytes.pop();
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            return String::from_utf8(line_bytes).map(Some).map_err(|source| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("inbound protocol line is not valid utf-8: {source}"),
                )
            });
        }

        let buffered_payload_bytes = bytes.len() - usize::from(bytes.last() == Some(&b'\r'));
        if buffered_payload_bytes > MAX_PROTOCOL_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                PROTOCOL_LINE_TOO_LONG_ERROR,
            ));
        }

        // Allow one optional CR plus the LF framing beyond the payload limit.
        // Any other byte in those slots is rejected on the next iteration, so
        // a peer still cannot grow this buffer without bound.
        newline_search_start = bytes.len();
        let remaining_capacity = MAX_PROTOCOL_LINE_BYTES + 2 - bytes.len();
        let mut chunk = [0_u8; READ_CHUNK_BYTES];
        let bytes_read = stream
            .read(&mut chunk[..remaining_capacity.min(READ_CHUNK_BYTES)])
            .await?;
        if bytes_read == 0 {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }

        bytes.extend_from_slice(&chunk[..bytes_read]);
    }

    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }

    let line_bytes = std::mem::take(bytes);
    String::from_utf8(line_bytes).map(Some).map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("inbound protocol line is not valid utf-8: {source}"),
        )
    })
}

#[cfg(test)]
pub(crate) async fn read_network_line_from_stream<S>(stream: &mut S) -> io::Result<Option<String>>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    read_network_line_from_stream_with_buffer(stream, &mut bytes).await
}

#[derive(Debug)]
struct PrefixedTcpStream {
    stream: TcpStream,
    prefix: Vec<u8>,
    prefix_offset: usize,
}

impl PrefixedTcpStream {
    fn new(stream: TcpStream, prefix: Vec<u8>) -> Self {
        Self {
            stream,
            prefix,
            prefix_offset: 0,
        }
    }
}

impl AsyncRead for PrefixedTcpStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.prefix_offset < this.prefix.len() {
            let end = (this.prefix_offset + buffer.remaining()).min(this.prefix.len());
            buffer.put_slice(&this.prefix[this.prefix_offset..end]);
            this.prefix_offset = end;
            if this.prefix_offset == this.prefix.len() {
                this.prefix.clear();
                this.prefix_offset = 0;
            }
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut this.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for PrefixedTcpStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_write(context, buffer)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

#[derive(Debug)]
enum ServerNetworkTransport {
    Plain {
        stream: TcpStream,
        read_buffer: Vec<u8>,
    },
    Tls {
        stream: Box<TlsStream<PrefixedTcpStream>>,
        read_buffer: Vec<u8>,
    },
    Closed,
    #[cfg(test)]
    StalledWrite,
}

impl ServerNetworkTransport {
    fn is_tls(&self) -> bool {
        matches!(self, Self::Tls { .. })
    }

    async fn read_line(&mut self) -> io::Result<Option<String>> {
        match self {
            Self::Plain {
                stream,
                read_buffer,
            } => read_network_line_from_stream_with_buffer(stream, read_buffer).await,
            Self::Tls {
                stream,
                read_buffer,
            } => read_network_line_from_stream_with_buffer(stream.as_mut(), read_buffer).await,
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
            #[cfg(test)]
            Self::StalledWrite => std::future::pending().await,
        }
    }

    async fn write_line_without_timeout(&mut self, line: &str) -> io::Result<()> {
        match self {
            Self::Plain { stream, .. } => write_network_line_to_stream(stream, line).await,
            Self::Tls { stream, .. } => write_network_line_to_stream(stream.as_mut(), line).await,
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
            #[cfg(test)]
            Self::StalledWrite => std::future::pending().await,
        }
    }

    async fn write_line_with_timeout(
        &mut self,
        line: &str,
        timeout_duration: std::time::Duration,
    ) -> io::Result<()> {
        match time::timeout(timeout_duration, self.write_line_without_timeout(line)).await {
            Ok(result) => result,
            Err(_) => Err(timeout_io_error("server protocol write", timeout_duration)),
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            Self::Plain { stream, .. } => stream.shutdown().await,
            Self::Tls { stream, .. } => stream.shutdown().await,
            Self::Closed => Ok(()),
            #[cfg(test)]
            Self::StalledWrite => Ok(()),
        }
    }

    async fn upgrade_to_tls(self, acceptor: TlsAcceptor) -> io::Result<Self> {
        match self {
            Self::Plain {
                stream,
                read_buffer,
            } => {
                // A chunked plaintext read may also receive the beginning of an
                // optimistic TLS handshake. Replay those prefetched bytes into
                // rustls so buffering does not change StartTLS behavior.
                let tls_stream = acceptor
                    .accept(PrefixedTcpStream::new(stream, read_buffer))
                    .await?;
                Ok(Self::Tls {
                    stream: Box::new(tls_stream),
                    read_buffer: Vec::new(),
                })
            }
            Self::Tls {
                stream,
                read_buffer,
            } => Ok(Self::Tls {
                stream,
                read_buffer,
            }),
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
            #[cfg(test)]
            Self::StalledWrite => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is stalled",
            )),
        }
    }

    async fn upgrade_to_tls_with_timeout(
        self,
        acceptor: TlsAcceptor,
        timeout_duration: std::time::Duration,
    ) -> io::Result<Self> {
        match time::timeout(timeout_duration, self.upgrade_to_tls(acceptor)).await {
            Ok(result) => result,
            Err(_) => Err(timeout_io_error("server TLS handshake", timeout_duration)),
        }
    }
}

async fn route_outbound_lines_for_client_session(
    transport: &mut ServerNetworkTransport,
    client_id: &str,
    client_event_senders: &SharedClientEventSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
    write_timeout: std::time::Duration,
) -> io::Result<()> {
    let mut peer_outbound_lines = Vec::new();
    for line in outbound_lines {
        if line.client_id == client_id {
            transport
                .write_line_with_timeout(&line.line, write_timeout)
                .await?;
        } else {
            peer_outbound_lines.push(line);
        }
    }
    dispatch_outbound_lines_to_clients(client_event_senders, peer_outbound_lines).await;
    Ok(())
}

async fn tls_acceptor_from_runtime(runtime: &ServerActorHandle) -> io::Result<TlsAcceptor> {
    let tls_server_config = runtime
        .tls_server_config()
        .await
        .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error))?;
    let Some(tls_server_config) = tls_server_config else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "tls server config is not available",
        ));
    };
    Ok(TlsAcceptor::from(tls_server_config))
}

async fn apply_local_transport_actions(
    transport: &mut ServerNetworkTransport,
    client_id: &str,
    runtime: &ServerActorHandle,
    transport_actions: &[DirectedTransportAction],
    tls_handshake_timeout: std::time::Duration,
) -> io::Result<()> {
    let should_start_tls = transport_actions.iter().any(|action| {
        action.client_id == client_id && action.action == ServerTransportAction::StartTls
    });
    if !should_start_tls || transport.is_tls() {
        return Ok(());
    }
    let tls_acceptor = tls_acceptor_from_runtime(runtime).await?;
    let current_transport = std::mem::replace(transport, ServerNetworkTransport::Closed);
    *transport = current_transport
        .upgrade_to_tls_with_timeout(tls_acceptor, tls_handshake_timeout)
        .await?;
    Ok(())
}

async fn run_server_network_client_session_with_timeouts_until_shutdown(
    stream: TcpStream,
    identity: ServerNetworkClientIdentity,
    runtime: ServerActorHandle,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    timeouts: ServerNetworkClientSessionTimeouts,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    let ServerNetworkClientIdentity { peer_ip, client_id } = identity;
    let (event_tx, mut event_rx) = client_event_queue(runtime.outbound_backpressure_metrics());
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert(client_id.clone(), event_tx);
    }

    let mut transport = ServerNetworkTransport::Plain {
        stream,
        read_buffer: Vec::new(),
    };
    let mut session_error: Option<ServerNetworkError> = None;
    let pre_hello_timer = time::sleep(timeouts.pre_hello);
    tokio::pin!(pre_hello_timer);
    let mut session_known = false;
    let mut shutdown_requested = false;
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown_rx) => {
                shutdown_requested = true;
                break;
            },
            _ = &mut pre_hello_timer, if !session_known => {
                break;
            }
            inbound_line_result = transport.read_line() => {
                let inbound_line = match inbound_line_result {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(source) => {
                        if source.kind() == io::ErrorKind::InvalidData
                            && let Ok(error_line) = encode_message_line(
                                &ProtocolMessage::error_message(
                                    if source.to_string() == PROTOCOL_LINE_TOO_LONG_ERROR {
                                        PROTOCOL_LINE_TOO_LONG_ERROR
                                    } else {
                                        LEGACY_SERVER_LINE_DECODE_ERROR
                                    },
                                ),
                            )
                            && run_until_shutdown(
                                &mut shutdown_rx,
                                transport.write_line_with_timeout(&error_line, timeouts.write),
                            )
                            .await
                            .is_none()
                        {
                            shutdown_requested = true;
                            break;
                        }
                        session_error = Some(ServerNetworkError::Io(source));
                        break;
                    }
                };
                let inbound_line = inbound_line.trim();
                if inbound_line.is_empty() {
                    continue;
                }
                let Some(handle_result) = run_until_shutdown(
                    &mut shutdown_rx,
                    runtime.handle_line(&client_id, inbound_line, peer_ip.as_deref()),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                let (dispatch, session_exists) = handle_result?;
                session_known = session_known || session_exists;
                let close_after_dispatch =
                    transport_actions_close_client(&dispatch.transport_actions, &client_id);
                let Some(route_result) = run_until_shutdown(
                    &mut shutdown_rx,
                    route_outbound_lines_for_client_session(
                        &mut transport,
                        &client_id,
                        &client_event_senders,
                        dispatch.outbound_lines,
                        timeouts.write,
                    ),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                if let Err(source) = route_result {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
                dispatch_transport_actions_to_sink(
                    transport_action_sink.as_ref(),
                    &dispatch.transport_actions,
                );
                let Some(action_result) = run_until_shutdown(
                    &mut shutdown_rx,
                    apply_local_transport_actions(
                        &mut transport,
                        &client_id,
                        &runtime,
                        &dispatch.transport_actions,
                        timeouts.tls_handshake,
                    ),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                if let Err(source) = action_result {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
                if close_after_dispatch {
                    break;
                }
            }
            outbound_line = event_rx.reliable_lines.recv() => {
                let Some(outbound_line) = outbound_line else {
                    break;
                };
                event_rx.metrics.dequeued();
                let Some(write_result) = run_until_shutdown(
                    &mut shutdown_rx,
                    transport.write_line_with_timeout(&outbound_line, timeouts.write),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                if let Err(source) = write_result {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
            }
            transport_action = event_rx.transport_actions.recv() => {
                let Some(transport_action) = transport_action else {
                    break;
                };
                event_rx.metrics.dequeued();
                match transport_action {
                    ServerTransportAction::Close => {
                        break;
                    }
                    ServerTransportAction::StartTls => {
                        let action = DirectedTransportAction::new(
                            &client_id,
                            ServerTransportAction::StartTls,
                        );
                        let Some(action_result) = run_until_shutdown(
                            &mut shutdown_rx,
                            apply_local_transport_actions(
                                &mut transport,
                                &client_id,
                                &runtime,
                                &[action],
                                timeouts.tls_handshake,
                            ),
                        )
                        .await
                        else {
                            shutdown_requested = true;
                            break;
                        };
                        if let Err(source) = action_result {
                            session_error = Some(ServerNetworkError::Io(source));
                            break;
                        }
                    }
                }
            }
            periodic_state_changed = event_rx.periodic_state.changed() => {
                if periodic_state_changed.is_err() {
                    break;
                }
                let update = event_rx.periodic_state.borrow_and_update().clone();
                let Some(update) = update else {
                    continue;
                };
                event_rx.periodic_state_delivered(update.generation);
                let Some(write_result) = run_until_shutdown(
                    &mut shutdown_rx,
                    transport.write_line_with_timeout(&update.line, timeouts.write),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                if let Err(source) = write_result {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
            }
            overload_changed = event_rx.overload_close.changed() => {
                if overload_changed.is_err() {
                    break;
                }
                let Some(queue_depth) = *event_rx.overload_close.borrow_and_update() else {
                    continue;
                };
                session_error = Some(ServerNetworkError::OutboundOverload {
                    client_id: client_id.clone(),
                    queue_depth,
                });
                break;
            }
        }
    }

    {
        let mut senders = client_event_senders.lock().await;
        senders.remove(&client_id);
    }
    event_rx.close_and_record_discarded();
    let transport_shutdown_timeout = timeouts.write.min(std::time::Duration::from_secs(1));
    match time::timeout(transport_shutdown_timeout, transport.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(source)) if !shutdown_requested && session_error.is_none() => {
            session_error = Some(ServerNetworkError::Io(source));
        }
        Err(_) if !shutdown_requested && session_error.is_none() => {
            session_error = Some(ServerNetworkError::Io(timeout_io_error(
                "transport shutdown",
                transport_shutdown_timeout,
            )));
        }
        Ok(Err(_)) | Err(_) => {}
    }

    let disconnect_fanout = runtime.disconnect(&client_id).await;
    match disconnect_fanout {
        Ok(outbound_lines) => {
            dispatch_outbound_lines_to_clients(&client_event_senders, outbound_lines).await;
        }
        Err(source) => {
            if session_error.is_none() {
                session_error = Some(ServerNetworkError::Actor(source));
            }
        }
    }

    if let Some(session_error) = session_error {
        return Err(session_error);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn run_server_network_client_session_with_timeouts(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: ServerActorHandle,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    timeouts: ServerNetworkClientSessionTimeouts,
) -> Result<(), ServerNetworkError> {
    // Direct session callers retain the historical API and run until their
    // transport closes. The production network owner supplies a real receiver
    // through the internal helper below.
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    run_server_network_client_session_with_timeouts_until_shutdown(
        stream,
        ServerNetworkClientIdentity { peer_ip, client_id },
        runtime,
        client_event_senders,
        transport_action_sink,
        timeouts,
        shutdown_rx,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_server_network_client_session_with_pre_hello_timeout(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: ServerActorHandle,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    pre_hello_timeout: std::time::Duration,
) -> Result<(), ServerNetworkError> {
    run_server_network_client_session_with_timeouts(
        stream,
        peer_ip,
        client_id,
        runtime,
        client_event_senders,
        transport_action_sink,
        ServerNetworkClientSessionTimeouts::production_with_pre_hello(pre_hello_timeout),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn stalled_transport_write_for_test(
    write_timeout: std::time::Duration,
) -> io::Result<()> {
    let mut transport = ServerNetworkTransport::StalledWrite;
    transport
        .write_line_with_timeout(r#"{"Chat":"stalled"}"#, write_timeout)
        .await
}

#[cfg(test)]
pub(crate) async fn stalled_transport_error_response_write_for_test(
    write_timeout: std::time::Duration,
) -> io::Result<()> {
    let mut transport = ServerNetworkTransport::StalledWrite;
    let error_line = encode_message_line(&ProtocolMessage::error_message(
        LEGACY_SERVER_LINE_DECODE_ERROR,
    ))
    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    transport
        .write_line_with_timeout(&error_line, write_timeout)
        .await
}

#[cfg(test)]
pub(crate) async fn stalled_transport_direct_response_write_for_test(
    write_timeout: std::time::Duration,
) -> io::Result<()> {
    let mut transport = ServerNetworkTransport::StalledWrite;
    let client_event_senders: SharedClientEventSenders = Arc::new(Mutex::new(BTreeMap::new()));
    route_outbound_lines_for_client_session(
        &mut transport,
        "client-1",
        &client_event_senders,
        vec![DirectedOutboundLine {
            client_id: "client-1".to_owned(),
            line: r#"{"Chat":"direct"}"#.to_owned(),
            delivery: ServerOutboundDelivery::Reliable,
        }],
        write_timeout,
    )
    .await
}

async fn run_server_network_client_session(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: ServerActorHandle,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    run_server_network_client_session_with_timeouts_until_shutdown(
        stream,
        ServerNetworkClientIdentity { peer_ip, client_id },
        runtime,
        client_event_senders,
        transport_action_sink,
        ServerNetworkClientSessionTimeouts::production_with_pre_hello(
            std::time::Duration::from_secs_f64(PROTOCOL_TIMEOUT_SECONDS),
        ),
        shutdown_rx,
    )
    .await
}

async fn accept_server_network_clients_until_shutdown(
    listener: TcpListener,
    accepted_clients: Sender<AcceptedClient>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown_rx) => break,
            accepted = listener.accept() => {
                let accepted_client = accepted;
                let accepted_error = accepted_client.is_err();
                tokio::select! {
                    sent = accepted_clients.send(accepted_client) => {
                        if sent.is_err() {
                            break;
                        }
                    }
                    _ = wait_for_shutdown(&mut shutdown_rx) => break,
                }
                if accepted_error {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown_rx.borrow() || shutdown_rx.changed().await.is_err() {
            return;
        }
    }
}

async fn run_until_shutdown<T>(
    shutdown_rx: &mut watch::Receiver<bool>,
    operation: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown_rx) => None,
        result = operation => Some(result),
    }
}

fn record_finished_tasks(tasks: &mut JoinSet<()>, task_kind: &str) {
    while let Some(result) = tasks.try_join_next() {
        if let Err(source) = result {
            eprintln!("Sorotte server {task_kind} task ended unexpectedly: {source}");
        }
    }
}

async fn await_tasks_until_deadline(
    tasks: &mut JoinSet<()>,
    deadline: time::Instant,
    task_kind: &str,
) -> usize {
    while !tasks.is_empty() {
        match time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(Ok(()))) => {}
            Ok(Some(Err(source))) => {
                eprintln!("Sorotte server {task_kind} task ended unexpectedly: {source}");
            }
            Ok(None) => break,
            Err(_) => {
                return tasks.len();
            }
        }
    }
    0
}

pub async fn run_server_network_loops_until_shutdown(
    listeners: Vec<TcpListener>,
    runtime: ServerActorHandle,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    if listeners.is_empty() {
        return Err(ServerNetworkError::Io(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no server listeners are available",
        )));
    }

    let client_event_senders: SharedClientEventSenders = Arc::new(Mutex::new(BTreeMap::new()));
    let (accepted_tx, mut accepted_rx): (Sender<AcceptedClient>, Receiver<AcceptedClient>) =
        channel(ACCEPTED_CLIENT_QUEUE_CAPACITY);
    let (local_shutdown_tx, local_shutdown_rx) = watch::channel(false);
    let mut accept_tasks = JoinSet::new();
    for listener in listeners {
        let accepted_tx = accepted_tx.clone();
        let listener_shutdown_rx = local_shutdown_rx.clone();
        accept_tasks.spawn(async move {
            if let Err(source) = accept_server_network_clients_until_shutdown(
                listener,
                accepted_tx,
                listener_shutdown_rx,
            )
            .await
            {
                eprintln!("Sorotte server acceptor ended with error: {source}");
            }
        });
    }
    drop(accepted_tx);

    let mut session_tasks = JoinSet::new();
    let mut next_client_number: u64 = 1;
    let mut tick = time::interval(std::time::Duration::from_secs_f64(
        SERVER_NETWORK_TICK_INTERVAL_SECONDS,
    ));
    let mut loop_error: Option<ServerNetworkError> = None;

    loop {
        record_finished_tasks(&mut session_tasks, "client session");
        record_finished_tasks(&mut accept_tasks, "acceptor");
        tokio::select! {
            _ = tick.tick() => {
                let dispatch = match runtime
                    .collect_dispatch(current_unix_timestamp_seconds())
                    .await
                {
                    Ok(dispatch) => dispatch,
                    Err(source) => {
                        loop_error = Some(ServerNetworkError::Actor(source));
                        break;
                    }
                };
                dispatch_outbound_lines_to_clients(
                    &client_event_senders,
                    dispatch.outbound_lines,
                )
                .await;
                dispatch_transport_actions_to_sink(
                    transport_action_sink.as_ref(),
                    &dispatch.transport_actions,
                );
                dispatch_transport_actions_to_clients(
                    &client_event_senders,
                    &dispatch.transport_actions,
                )
                .await;
            }
            _ = wait_for_shutdown(&mut shutdown_rx) => break,
            accepted = accepted_rx.recv() => {
                let Some(accepted) = accepted else {
                    break;
                };
                let (stream, address) = match accepted {
                    Ok(accepted) => accepted,
                    Err(source) => {
                        loop_error = Some(ServerNetworkError::Io(source));
                        break;
                    }
                };
                let client_id = format!("client-{next_client_number}");
                next_client_number = next_client_number.saturating_add(1);
                let runtime = runtime.clone();
                let client_event_senders = client_event_senders.clone();
                let transport_action_sink = transport_action_sink.clone();
                let session_shutdown_rx = local_shutdown_rx.clone();
                let peer_ip = Some(address.ip().to_string());
                let task_client_id = client_id.clone();
                session_tasks.spawn(async move {
                    if let Err(source) = run_server_network_client_session(
                        stream,
                        peer_ip,
                        client_id,
                        runtime,
                        client_event_senders,
                        transport_action_sink,
                        session_shutdown_rx,
                    )
                    .await
                    {
                        eprintln!(
                            "Sorotte server client session {task_client_id} ended with error: {source}"
                        );
                    }
                });
            }
        }
    }

    // Stop the listeners first, then let every active session run its normal
    // transport shutdown and runtime-disconnect path. A single bounded grace
    // period covers both phases; remaining tasks are explicitly aborted after
    // the deadline rather than detached.
    let _ = local_shutdown_tx.send(true);
    drop(accepted_rx);
    let shutdown_grace = std::time::Duration::from_secs_f64(SERVER_NETWORK_SHUTDOWN_GRACE_SECONDS);
    let shutdown_deadline = time::Instant::now() + shutdown_grace;
    // Reserve a small slice of the same bounded grace period for joining
    // forced cancellations. This keeps actor shutdown from racing session
    // futures in the normal timeout case without extending the deadline.
    let forced_join_reserve = std::time::Duration::from_millis(100);
    let graceful_deadline = shutdown_deadline - forced_join_reserve;
    let timed_out_acceptors =
        await_tasks_until_deadline(&mut accept_tasks, graceful_deadline, "acceptor").await;
    let timed_out_sessions =
        await_tasks_until_deadline(&mut session_tasks, graceful_deadline, "client session").await;

    if timed_out_acceptors > 0 || timed_out_sessions > 0 {
        accept_tasks.abort_all();
        session_tasks.abort_all();
        let undrained_acceptors =
            await_tasks_until_deadline(&mut accept_tasks, shutdown_deadline, "acceptor").await;
        let undrained_sessions =
            await_tasks_until_deadline(&mut session_tasks, shutdown_deadline, "client session")
                .await;
        if undrained_acceptors > 0 || undrained_sessions > 0 {
            // Dropping a JoinSet aborts every task still registered in it. The
            // sets are locals and are dropped before this function returns to
            // the lifecycle owner, so no task handle is silently detached.
            eprintln!(
                "Sorotte server forced shutdown could not join {undrained_acceptors} acceptor task(s) and {undrained_sessions} client session task(s) before the deadline"
            );
        }
        let timeout_error = ServerNetworkError::ShutdownTimeout {
            timeout_millis: shutdown_grace.as_millis().try_into().unwrap_or(u64::MAX),
            acceptor_tasks: timed_out_acceptors,
            session_tasks: timed_out_sessions,
        };
        if let Some(loop_error) = loop_error {
            eprintln!("Sorotte server network teardown also failed: {timeout_error}");
            return Err(loop_error);
        }
        return Err(timeout_error);
    }

    if let Some(loop_error) = loop_error {
        return Err(loop_error);
    }

    Ok(())
}

/// Owns the production server lifecycle through the explicit durability
/// barrier. Actor shutdown is attempted even when network startup or teardown
/// fails, and dual failures retain both causes.
pub async fn run_server_network_loops_and_shutdown_actor(
    listeners: Vec<TcpListener>,
    runtime: ServerActorHandle,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerLifecycleError> {
    let network_result = run_server_network_loops_until_shutdown(
        listeners,
        runtime.clone(),
        transport_action_sink,
        shutdown_rx,
    )
    .await;
    let shutdown_result = runtime.shutdown().await;

    match (network_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(network), Ok(())) => Err(ServerLifecycleError::Network(network)),
        (Ok(()), Err(shutdown)) => Err(ServerLifecycleError::Shutdown(shutdown)),
        (Err(network), Err(shutdown)) => {
            Err(ServerLifecycleError::NetworkAndShutdown { network, shutdown })
        }
    }
}

pub async fn run_server_network_loop_until_shutdown(
    listener: TcpListener,
    runtime: ServerActorHandle,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    run_server_network_loops_until_shutdown(
        vec![listener],
        runtime,
        transport_action_sink,
        shutdown_rx,
    )
    .await
}

#[cfg(test)]
mod credential_debug_tests {
    use super::*;

    #[test]
    fn outbound_network_event_debug_never_prints_protocol_lines() {
        let secret = "network-line-password-canary";
        let line = format!(r#"{{\"Hello\":{{\"password\":\"{secret}\"}}}}"#);
        let event = ClientOutboundEvent::ReliableLine(line.clone());
        let periodic = PeriodicStateUpdate {
            generation: 7,
            line,
        };

        assert!(!format!("{event:?}").contains(secret));
        assert!(!format!("{periodic:?}").contains(secret));
    }

    #[tokio::test]
    async fn session_shutdown_wait_is_bounded_and_reports_remaining_tasks() {
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::pending::<()>());

        let remaining = await_tasks_until_deadline(
            &mut tasks,
            time::Instant::now() + std::time::Duration::from_millis(20),
            "test session",
        )
        .await;

        assert_eq!(remaining, 1);
        tasks.abort_all();
        time::timeout(std::time::Duration::from_secs(1), tasks.shutdown())
            .await
            .expect("aborted task should be joinable after the bounded deadline");
    }
}
