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

trait ServerNetworkAcceptor: Send + Sync {
    fn accept(&self) -> std::pin::Pin<Box<dyn Future<Output = AcceptedClient> + Send + '_>>;
}

impl ServerNetworkAcceptor for TcpListener {
    fn accept(&self) -> std::pin::Pin<Box<dyn Future<Output = AcceptedClient> + Send + '_>> {
        Box::pin(TcpListener::accept(self))
    }
}

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
struct ProtocolLineQueueState {
    latest_periodic_generation: u64,
    tail_periodic: Option<Arc<StdMutex<PeriodicStateUpdate>>>,
    closed: bool,
}

enum QueuedProtocolLine {
    Reliable(String),
    Periodic(Arc<StdMutex<PeriodicStateUpdate>>),
    TransportAction(ServerTransportAction),
}

enum ResolvedClientEvent {
    ProtocolLine(String),
    TransportAction(ServerTransportAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientEventSendOutcome {
    Sent,
    Coalesced,
    DroppedPeriodic,
    Closed,
    Overloaded,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientEventSender {
    // Every protocol line shares one causal lane. A periodic state remains
    // replaceable only while its marker is the queue tail; a later reliable
    // line seals it so no older state can arrive after that reliable line.
    protocol_lines: Sender<QueuedProtocolLine>,
    protocol_line_queue: Arc<Mutex<ProtocolLineQueueState>>,
    overload_close: watch::Sender<Option<usize>>,
    overload_signalled: Arc<AtomicBool>,
    metrics: ServerOutboundBackpressureMetrics,
}

pub(crate) struct ClientEventReceiver {
    protocol_lines: Receiver<QueuedProtocolLine>,
    protocol_line_queue: Arc<Mutex<ProtocolLineQueueState>>,
    overload_close: watch::Receiver<Option<usize>>,
    metrics: ServerOutboundBackpressureMetrics,
}

pub(crate) type SharedClientEventSenders = Arc<Mutex<BTreeMap<String, ClientEventSender>>>;
pub(crate) type SharedNetworkDispatchOrder = Arc<Mutex<()>>;

#[derive(Clone)]
struct ServerNetworkDispatchContext {
    client_event_senders: SharedClientEventSenders,
    dispatch_order: SharedNetworkDispatchOrder,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
}

pub(crate) async fn with_network_dispatch_order<T>(
    dispatch_order: &SharedNetworkDispatchOrder,
    operation: impl Future<Output = T>,
) -> T {
    let _guard = dispatch_order.lock().await;
    operation.await
}

pub(crate) fn client_event_queue(
    metrics: ServerOutboundBackpressureMetrics,
) -> (ClientEventSender, ClientEventReceiver) {
    let (protocol_tx, protocol_rx) = channel(CLIENT_OUTBOUND_QUEUE_CAPACITY);
    let (overload_tx, overload_rx) = watch::channel(None);
    let protocol_line_queue = Arc::new(Mutex::new(ProtocolLineQueueState::default()));
    (
        ClientEventSender {
            protocol_lines: protocol_tx,
            protocol_line_queue: protocol_line_queue.clone(),
            overload_close: overload_tx,
            overload_signalled: Arc::new(AtomicBool::new(false)),
            metrics: metrics.clone(),
        },
        ClientEventReceiver {
            protocol_lines: protocol_rx,
            protocol_line_queue,
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
            ClientOutboundEvent::PeriodicStateLine(line) => self.send_periodic_state(line).await,
            ClientOutboundEvent::TransportAction(action) => {
                self.send_reliable_event(QueuedProtocolLine::TransportAction(action))
                    .await
            }
        }
    }

    async fn send_reliable_line(&self, line: String) -> ClientEventSendOutcome {
        self.send_reliable_event(QueuedProtocolLine::Reliable(line))
            .await
    }

    async fn send_reliable_event(&self, item: QueuedProtocolLine) -> ClientEventSendOutcome {
        let mut queue = self.protocol_line_queue.lock().await;
        if queue.closed || self.protocol_lines.is_closed() {
            queue.closed = true;
            self.metrics.closed();
            self.metrics.dropped();
            return ClientEventSendOutcome::Closed;
        }
        match self.protocol_lines.try_send(item) {
            Ok(()) => {
                queue.tail_periodic = None;
                self.metrics.enqueued();
                ClientEventSendOutcome::Sent
            }
            Err(TrySendError::Closed(_item)) => {
                queue.closed = true;
                self.metrics.closed();
                self.metrics.dropped();
                ClientEventSendOutcome::Closed
            }
            Err(TrySendError::Full(item)) => {
                self.metrics.full();
                let grace = std::time::Duration::from_millis(CLIENT_OUTBOUND_OVERLOAD_GRACE_MILLIS);
                match time::timeout(grace, self.protocol_lines.send(item)).await {
                    Ok(Ok(())) => {
                        queue.tail_periodic = None;
                        self.metrics.enqueued();
                        ClientEventSendOutcome::Sent
                    }
                    Ok(Err(_closed)) => {
                        queue.closed = true;
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

    async fn send_periodic_state(&self, line: String) -> ClientEventSendOutcome {
        let mut queue = self.protocol_line_queue.lock().await;
        if queue.closed || self.protocol_lines.is_closed() {
            queue.closed = true;
            self.metrics.closed();
            self.metrics.dropped();
            return ClientEventSendOutcome::Closed;
        }
        let generation = queue.latest_periodic_generation.saturating_add(1);
        queue.latest_periodic_generation = generation;
        if let Some(tail_periodic) = queue.tail_periodic.as_ref() {
            *tail_periodic
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                PeriodicStateUpdate { generation, line };
            self.metrics.coalesced();
            return ClientEventSendOutcome::Coalesced;
        }

        let update = Arc::new(StdMutex::new(PeriodicStateUpdate { generation, line }));
        let item = QueuedProtocolLine::Periodic(update.clone());
        match self.protocol_lines.try_send(item) {
            Ok(()) => {
                queue.tail_periodic = Some(update);
                self.metrics.enqueued();
                ClientEventSendOutcome::Sent
            }
            Err(TrySendError::Closed(_item)) => {
                queue.closed = true;
                self.metrics.closed();
                self.metrics.dropped();
                ClientEventSendOutcome::Closed
            }
            Err(TrySendError::Full(_item)) => {
                self.metrics.full();
                self.metrics.dropped();
                ClientEventSendOutcome::DroppedPeriodic
            }
        }
    }

    fn signal_overload(&self) {
        if !self.overload_signalled.swap(true, Ordering::AcqRel) {
            self.metrics.overload_disconnect();
            let queue_depth =
                CLIENT_OUTBOUND_QUEUE_CAPACITY.saturating_sub(self.protocol_lines.capacity());
            let _ = self.overload_close.send(Some(queue_depth));
        }
    }
}

impl ClientEventReceiver {
    async fn resolve_event(&self, item: QueuedProtocolLine) -> ResolvedClientEvent {
        let event = match item {
            QueuedProtocolLine::Reliable(line) => ResolvedClientEvent::ProtocolLine(line),
            QueuedProtocolLine::Periodic(update) => {
                let mut queue = self.protocol_line_queue.lock().await;
                if queue
                    .tail_periodic
                    .as_ref()
                    .is_some_and(|tail| Arc::ptr_eq(tail, &update))
                {
                    queue.tail_periodic = None;
                }
                let line = update
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .line
                    .clone();
                ResolvedClientEvent::ProtocolLine(line)
            }
            QueuedProtocolLine::TransportAction(action) => {
                ResolvedClientEvent::TransportAction(action)
            }
        };
        self.metrics.dequeued();
        event
    }

    #[cfg(test)]
    async fn resolve_protocol_line(&self, item: QueuedProtocolLine) -> String {
        match self.resolve_event(item).await {
            ResolvedClientEvent::ProtocolLine(line) => line,
            ResolvedClientEvent::TransportAction(action) => {
                panic!("expected protocol line, received transport action {action:?}")
            }
        }
    }

    #[cfg(test)]
    async fn receive_protocol_line(&mut self) -> Option<String> {
        let item = self.protocol_lines.recv().await?;
        Some(self.resolve_protocol_line(item).await)
    }

    async fn close_and_record_discarded(&mut self) {
        {
            let mut queue = self.protocol_line_queue.lock().await;
            queue.closed = true;
            queue.tail_periodic = None;
            self.protocol_lines.close();
        }
        let discarded = self.protocol_lines.len();
        self.metrics.discarded(discarded);
    }

    #[cfg(test)]
    pub(crate) async fn receive_reliable_line_for_test(&mut self) -> Option<String> {
        self.receive_protocol_line().await
    }

    #[cfg(test)]
    pub(crate) async fn receive_periodic_state_for_test(&mut self) -> Option<String> {
        self.receive_protocol_line().await
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
    let mut events_by_client = BTreeMap::<String, Vec<ClientOutboundEvent>>::new();
    for line in outbound_lines {
        let event = match line.delivery {
            ServerOutboundDelivery::Reliable => ClientOutboundEvent::ReliableLine(line.line),
            ServerOutboundDelivery::CoalesciblePeriodicState => {
                ClientOutboundEvent::PeriodicStateLine(line.line)
            }
        };
        events_by_client
            .entry(line.client_id)
            .or_default()
            .push(event);
    }

    let mut dispatch_tasks = JoinSet::new();
    for (client_id, events) in events_by_client {
        let client_event_senders = Arc::clone(client_event_senders);
        dispatch_tasks.spawn(async move {
            for event in events {
                dispatch_client_event(&client_event_senders, &client_id, event).await;
            }
        });
    }
    while let Some(result) = dispatch_tasks.join_next().await {
        result.expect("client outbound dispatch task should not panic");
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
        ClientEventSendOutcome::Sent
        | ClientEventSendOutcome::Coalesced
        | ClientEventSendOutcome::DroppedPeriodic => {}
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

struct OrderedClientDispatch {
    session_exists: bool,
}

async fn handle_line_and_queue_peer_dispatch_after_commit(
    runtime: &ServerActorHandle,
    dispatch_context: &ServerNetworkDispatchContext,
    client_id: &str,
    inbound_line: &str,
    peer_ip: Option<&str>,
    after_actor_commit: impl Future<Output = ()>,
) -> Result<OrderedClientDispatch, ServerActorError> {
    with_network_dispatch_order(&dispatch_context.dispatch_order, async {
        let (dispatch, session_exists) = runtime
            .handle_line(client_id, inbound_line, peer_ip)
            .await?;
        after_actor_commit.await;
        dispatch_outbound_lines_to_clients(
            &dispatch_context.client_event_senders,
            dispatch.outbound_lines,
        )
        .await;
        dispatch_transport_actions_to_sink(
            dispatch_context.transport_action_sink.as_ref(),
            &dispatch.transport_actions,
        );
        dispatch_transport_actions_to_clients(
            &dispatch_context.client_event_senders,
            &dispatch.transport_actions,
        )
        .await;
        Ok(OrderedClientDispatch { session_exists })
    })
    .await
}

async fn handle_line_and_queue_peer_dispatch(
    runtime: &ServerActorHandle,
    dispatch_context: &ServerNetworkDispatchContext,
    client_id: &str,
    inbound_line: &str,
    peer_ip: Option<&str>,
) -> Result<OrderedClientDispatch, ServerActorError> {
    handle_line_and_queue_peer_dispatch_after_commit(
        runtime,
        dispatch_context,
        client_id,
        inbound_line,
        peer_ip,
        std::future::ready(()),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn handle_line_and_queue_peer_dispatch_after_commit_for_test(
    runtime: &ServerActorHandle,
    dispatch_order: &SharedNetworkDispatchOrder,
    client_id: &str,
    inbound_line: &str,
    client_event_senders: &SharedClientEventSenders,
    after_actor_commit: impl Future<Output = ()>,
) -> Result<(), ServerActorError> {
    let dispatch_context = ServerNetworkDispatchContext {
        client_event_senders: client_event_senders.clone(),
        dispatch_order: dispatch_order.clone(),
        transport_action_sink: None,
    };
    handle_line_and_queue_peer_dispatch_after_commit(
        runtime,
        &dispatch_context,
        client_id,
        inbound_line,
        None,
        after_actor_commit,
    )
    .await
    .map(|_| ())
}

async fn disconnect_and_queue_peer_dispatch(
    runtime: &ServerActorHandle,
    dispatch_context: &ServerNetworkDispatchContext,
    client_id: &str,
) -> Result<(), ServerActorError> {
    with_network_dispatch_order(&dispatch_context.dispatch_order, async {
        let outbound_lines = runtime.disconnect(client_id).await?;
        dispatch_outbound_lines_to_clients(&dispatch_context.client_event_senders, outbound_lines)
            .await;
        Ok(())
    })
    .await
}

async fn collect_and_queue_periodic_dispatch(
    runtime: &ServerActorHandle,
    dispatch_context: &ServerNetworkDispatchContext,
    now_seconds: f64,
) -> Result<(), ServerActorError> {
    with_network_dispatch_order(&dispatch_context.dispatch_order, async {
        let dispatch = runtime.collect_dispatch(now_seconds).await?;
        dispatch_outbound_lines_to_clients(
            &dispatch_context.client_event_senders,
            dispatch.outbound_lines,
        )
        .await;
        dispatch_transport_actions_to_sink(
            dispatch_context.transport_action_sink.as_ref(),
            &dispatch.transport_actions,
        );
        dispatch_transport_actions_to_clients(
            &dispatch_context.client_event_senders,
            &dispatch.transport_actions,
        )
        .await;
        Ok(())
    })
    .await
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
    dispatch_context: ServerNetworkDispatchContext,
    timeouts: ServerNetworkClientSessionTimeouts,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    let ServerNetworkClientIdentity { peer_ip, client_id } = identity;
    let (event_tx, mut event_rx) = client_event_queue(runtime.outbound_backpressure_metrics());
    {
        let mut senders = dispatch_context.client_event_senders.lock().await;
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
            biased;

            _ = wait_for_shutdown(&mut shutdown_rx) => {
                shutdown_requested = true;
                break;
            },
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
            outbound_item = event_rx.protocol_lines.recv() => {
                let Some(outbound_item) = outbound_item else {
                    break;
                };
                match event_rx.resolve_event(outbound_item).await {
                    ResolvedClientEvent::ProtocolLine(outbound_line) => {
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
                    ResolvedClientEvent::TransportAction(ServerTransportAction::Close) => break,
                    ResolvedClientEvent::TransportAction(ServerTransportAction::StartTls) => {
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
                    handle_line_and_queue_peer_dispatch(
                        &runtime,
                        &dispatch_context,
                        &client_id,
                        inbound_line,
                        peer_ip.as_deref(),
                    ),
                )
                .await
                else {
                    shutdown_requested = true;
                    break;
                };
                let ordered_dispatch = handle_result?;
                session_known = session_known || ordered_dispatch.session_exists;
            }
        }
    }

    {
        let mut senders = dispatch_context.client_event_senders.lock().await;
        senders.remove(&client_id);
    }
    event_rx.close_and_record_discarded().await;
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

    let disconnect_result =
        disconnect_and_queue_peer_dispatch(&runtime, &dispatch_context, &client_id).await;
    match disconnect_result {
        Ok(()) => {}
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
    let dispatch_context = ServerNetworkDispatchContext {
        client_event_senders,
        dispatch_order: Arc::new(Mutex::new(())),
        transport_action_sink,
    };
    run_server_network_client_session_with_timeouts_until_shutdown(
        stream,
        ServerNetworkClientIdentity { peer_ip, client_id },
        runtime,
        dispatch_context,
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
    let (client_tx, mut client_rx) =
        client_event_queue(ServerOutboundBackpressureMetrics::default());
    client_event_senders
        .lock()
        .await
        .insert("client-1".to_owned(), client_tx);
    dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![DirectedOutboundLine {
            client_id: "client-1".to_owned(),
            line: r#"{"Chat":"direct"}"#.to_owned(),
            delivery: ServerOutboundDelivery::Reliable,
        }],
    )
    .await;
    let line = client_rx
        .receive_reliable_line_for_test()
        .await
        .expect("source response should be queued");
    transport
        .write_line_with_timeout(&line, write_timeout)
        .await
}

#[cfg(test)]
pub(crate) async fn stalled_source_write_still_queues_peer_fanout_for_test(
    write_timeout: std::time::Duration,
) -> (io::Result<()>, Option<String>, Vec<String>) {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "source-client",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .expect("source test session should initialize");
    runtime
        .handle_line(
            "peer-client",
            r#"{"Hello":{"username":"bob","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .expect("peer test session should initialize");
    let outbound_lines = runtime
        .handle_line_fanout(
            "source-client",
            r#"{"Set":{"playlistChange":{"files":["committed.mkv"]}}}"#,
        )
        .expect("playlist mutation should commit before network routing");
    let committed_files = runtime.room_playlist_state("room").files;

    let mut transport = ServerNetworkTransport::StalledWrite;
    let client_event_senders: SharedClientEventSenders = Arc::new(Mutex::new(BTreeMap::new()));
    let (source_tx, mut source_rx) =
        client_event_queue(ServerOutboundBackpressureMetrics::default());
    let (peer_tx, mut peer_rx) = client_event_queue(ServerOutboundBackpressureMetrics::default());
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert("source-client".to_owned(), source_tx);
        senders.insert("peer-client".to_owned(), peer_tx);
    }
    dispatch_outbound_lines_to_clients(&client_event_senders, outbound_lines).await;
    let peer_line = peer_rx.receive_reliable_line_for_test().await;
    let source_line = source_rx
        .receive_reliable_line_for_test()
        .await
        .expect("source response should be queued");
    let route_result = transport
        .write_line_with_timeout(&source_line, write_timeout)
        .await;
    (route_result, peer_line, committed_files)
}

async fn run_server_network_client_session(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: ServerActorHandle,
    dispatch_context: ServerNetworkDispatchContext,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    run_server_network_client_session_with_timeouts_until_shutdown(
        stream,
        ServerNetworkClientIdentity { peer_ip, client_id },
        runtime,
        dispatch_context,
        ServerNetworkClientSessionTimeouts::production_with_pre_hello(
            std::time::Duration::from_secs_f64(PROTOCOL_TIMEOUT_SECONDS),
        ),
        shutdown_rx,
    )
    .await
}

fn accept_error_is_transient(source: &io::Error) -> bool {
    // Invalid/closed listener handles are permanent. Other accept failures,
    // including descriptor/resource pressure, are retried because they do not
    // invalidate the bound socket and commonly recover once load subsides.
    if matches!(source.raw_os_error(), Some(9 | 995 | 10022 | 10038)) {
        return false;
    }
    !matches!(
        source.kind(),
        io::ErrorKind::PermissionDenied
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::AddrNotAvailable
            | io::ErrorKind::NotConnected
            | io::ErrorKind::Unsupported
    )
}

async fn accept_server_network_clients_with_until_shutdown<A>(
    acceptor: A,
    accepted_clients: Sender<AcceptedClient>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> io::Result<()>
where
    A: ServerNetworkAcceptor,
{
    let initial_backoff = std::time::Duration::from_millis(ACCEPT_RETRY_INITIAL_BACKOFF_MILLIS);
    let max_backoff = std::time::Duration::from_millis(ACCEPT_RETRY_MAX_BACKOFF_MILLIS);
    let mut retry_backoff = initial_backoff;
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown_rx) => break,
            accepted = acceptor.accept() => {
                let accepted_client = match accepted {
                    Ok(accepted_client) => {
                        retry_backoff = initial_backoff;
                        Ok(accepted_client)
                    }
                    Err(source) if accept_error_is_transient(&source) => {
                        tokio::select! {
                            _ = time::sleep(retry_backoff) => {}
                            _ = wait_for_shutdown(&mut shutdown_rx) => break,
                        }
                        retry_backoff = retry_backoff.saturating_mul(2).min(max_backoff);
                        continue;
                    }
                    Err(source) => Err(source),
                };
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

async fn accept_server_network_clients_until_shutdown(
    listener: TcpListener,
    accepted_clients: Sender<AcceptedClient>,
    shutdown_rx: watch::Receiver<bool>,
) -> io::Result<()> {
    accept_server_network_clients_with_until_shutdown(listener, accepted_clients, shutdown_rx).await
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

    let dispatch_context = ServerNetworkDispatchContext {
        client_event_senders: Arc::new(Mutex::new(BTreeMap::new())),
        dispatch_order: Arc::new(Mutex::new(())),
        transport_action_sink,
    };
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
                match collect_and_queue_periodic_dispatch(
                    &runtime,
                    &dispatch_context,
                    current_unix_timestamp_seconds(),
                )
                .await
                {
                    Ok(()) => {}
                    Err(source) => {
                        loop_error = Some(ServerNetworkError::Actor(source));
                        break;
                    }
                }
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
                let dispatch_context = dispatch_context.clone();
                let session_shutdown_rx = local_shutdown_rx.clone();
                let peer_ip = Some(address.ip().to_string());
                let task_client_id = client_id.clone();
                session_tasks.spawn(async move {
                    if let Err(source) = run_server_network_client_session(
                        stream,
                        peer_ip,
                        client_id,
                        runtime,
                        dispatch_context,
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

    struct ScriptedAcceptor {
        outcomes: StdMutex<std::collections::VecDeque<AcceptedClient>>,
    }

    impl ServerNetworkAcceptor for ScriptedAcceptor {
        fn accept(&self) -> std::pin::Pin<Box<dyn Future<Output = AcceptedClient> + Send + '_>> {
            let outcome = self
                .outcomes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front();
            Box::pin(async move {
                match outcome {
                    Some(outcome) => outcome,
                    None => std::future::pending::<AcceptedClient>().await,
                }
            })
        }
    }

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

    #[tokio::test]
    async fn repro_pending_periodic_state_never_follows_newer_reliable_state() {
        let (sender, mut receiver) =
            client_event_queue(ServerOutboundBackpressureMetrics::default());
        assert_eq!(
            sender
                .send(ClientOutboundEvent::PeriodicStateLine(
                    "old-periodic-state".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::ReliableLine(
                    "new-forced-state".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );

        assert_eq!(
            receiver.receive_protocol_line().await.as_deref(),
            Some("old-periodic-state")
        );
        assert_eq!(
            receiver.receive_protocol_line().await.as_deref(),
            Some("new-forced-state")
        );
    }

    #[tokio::test]
    async fn periodic_state_coalescing_is_scoped_to_the_protocol_queue_tail() {
        let metrics = ServerOutboundBackpressureMetrics::default();
        let (sender, mut receiver) = client_event_queue(metrics.clone());
        assert_eq!(
            sender
                .send(ClientOutboundEvent::PeriodicStateLine(
                    "periodic-before-reliable".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::ReliableLine("reliable".to_owned()))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::PeriodicStateLine(
                    "periodic-after-reliable-1".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::PeriodicStateLine(
                    "periodic-after-reliable-2".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Coalesced
        );

        assert_eq!(
            receiver.receive_protocol_line().await.as_deref(),
            Some("periodic-before-reliable")
        );
        assert_eq!(
            receiver.receive_protocol_line().await.as_deref(),
            Some("reliable")
        );
        assert_eq!(
            receiver.receive_protocol_line().await.as_deref(),
            Some("periodic-after-reliable-2")
        );
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.coalesced_state_updates, 1);
        assert_eq!(snapshot.queue_depth, 0);
    }

    #[tokio::test]
    async fn pending_periodic_state_is_not_written_after_a_newer_direct_local_state() {
        let (sender, mut receiver) =
            client_event_queue(ServerOutboundBackpressureMetrics::default());

        assert_eq!(
            sender
                .send(ClientOutboundEvent::PeriodicStateLine(
                    "old-periodic-state".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::ReliableLine(
                    "new-forced-state".to_owned(),
                ))
                .await,
            ClientEventSendOutcome::Sent
        );

        let first = receiver
            .receive_protocol_line()
            .await
            .expect("old periodic state should remain first");
        let second = receiver
            .receive_protocol_line()
            .await
            .expect("new reliable state should remain second");
        assert_eq!(
            [first.as_str(), second.as_str()],
            ["old-periodic-state", "new-forced-state"],
            "an older queued state must not arrive after the newer authoritative local response"
        );
    }

    #[tokio::test]
    async fn transport_action_cannot_overtake_its_preceding_protocol_line() {
        let (sender, mut receiver) =
            client_event_queue(ServerOutboundBackpressureMetrics::default());
        assert_eq!(
            sender
                .send(ClientOutboundEvent::ReliableLine("tls-ack".to_owned()))
                .await,
            ClientEventSendOutcome::Sent
        );
        assert_eq!(
            sender
                .send(ClientOutboundEvent::TransportAction(
                    ServerTransportAction::StartTls,
                ))
                .await,
            ClientEventSendOutcome::Sent
        );

        let first = receiver
            .protocol_lines
            .recv()
            .await
            .expect("TLS acknowledgement should be queued");
        let second = receiver
            .protocol_lines
            .recv()
            .await
            .expect("TLS action should be queued");
        assert!(matches!(
            receiver.resolve_event(first).await,
            ResolvedClientEvent::ProtocolLine(line) if line == "tls-ack"
        ));
        assert!(matches!(
            receiver.resolve_event(second).await,
            ResolvedClientEvent::TransportAction(ServerTransportAction::StartTls)
        ));
    }

    #[tokio::test]
    async fn transient_accept_error_retries_and_delivers_the_next_connection() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have an address");
        let client = TcpStream::connect(address);
        let accepted = listener.accept();
        let (client_result, accepted_result) = tokio::join!(client, accepted);
        let client_stream = client_result.expect("test client should connect");
        let (server_stream, peer_address) = accepted_result.expect("test server should accept");

        let acceptor = ScriptedAcceptor {
            outcomes: StdMutex::new(std::collections::VecDeque::from([
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "injected transient accept failure",
                )),
                Ok((server_stream, peer_address)),
            ])),
        };
        let (accepted_tx, mut accepted_rx) = channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let accept_task = tokio::spawn(accept_server_network_clients_with_until_shutdown(
            acceptor,
            accepted_tx,
            shutdown_rx,
        ));

        let (_, delivered_address) =
            time::timeout(std::time::Duration::from_secs(1), accepted_rx.recv())
                .await
                .expect("accept retry should complete before timeout")
                .expect("accept channel should remain open")
                .expect("the second accept result should be delivered");
        assert_eq!(delivered_address, peer_address);

        let _ = shutdown_tx.send(true);
        accept_task
            .await
            .expect("accept retry task should join")
            .expect("shutdown after recovery should succeed");
        drop(client_stream);
    }
}
