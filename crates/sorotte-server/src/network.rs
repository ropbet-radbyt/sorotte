use super::*;
use std::{
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
    let mut byte = [0_u8; 1];
    loop {
        let bytes_read = stream.read(&mut byte).await?;
        if bytes_read == 0 {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
        if bytes.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                PROTOCOL_LINE_TOO_LONG_ERROR,
            ));
        }
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
enum ServerNetworkTransport {
    Plain {
        stream: TcpStream,
        read_buffer: Vec<u8>,
    },
    Tls {
        stream: Box<TlsStream<TcpStream>>,
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
                let tls_stream = acceptor.accept(stream).await?;
                Ok(Self::Tls {
                    stream: Box::new(tls_stream),
                    read_buffer,
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

pub(crate) async fn run_server_network_client_session_with_timeouts(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: ServerActorHandle,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    timeouts: ServerNetworkClientSessionTimeouts,
) -> Result<(), ServerNetworkError> {
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
    loop {
        tokio::select! {
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
                        {
                            let _ = transport
                                .write_line_with_timeout(&error_line, timeouts.write)
                                .await;
                        }
                        session_error = Some(ServerNetworkError::Io(source));
                        break;
                    }
                };
                let inbound_line = inbound_line.trim();
                if inbound_line.is_empty() {
                    continue;
                }
                let (dispatch, session_exists) = runtime
                    .handle_line(&client_id, inbound_line, peer_ip.as_deref())
                    .await?;
                session_known = session_known || session_exists;
                let close_after_dispatch =
                    transport_actions_close_client(&dispatch.transport_actions, &client_id);
                if let Err(source) = route_outbound_lines_for_client_session(
                    &mut transport,
                    &client_id,
                    &client_event_senders,
                    dispatch.outbound_lines,
                    timeouts.write,
                )
                .await
                {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
                dispatch_transport_actions_to_sink(
                    transport_action_sink.as_ref(),
                    &dispatch.transport_actions,
                );
                if let Err(source) = apply_local_transport_actions(
                    &mut transport,
                    &client_id,
                    &runtime,
                    &dispatch.transport_actions,
                    timeouts.tls_handshake,
                )
                .await
                {
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
                if let Err(source) =
                    transport
                        .write_line_with_timeout(&outbound_line, timeouts.write)
                        .await
                {
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
                        if let Err(source) = apply_local_transport_actions(
                            &mut transport,
                            &client_id,
                            &runtime,
                            &[action],
                            timeouts.tls_handshake,
                        )
                        .await
                        {
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
                if let Err(source) =
                    transport
                        .write_line_with_timeout(&update.line, timeouts.write)
                        .await
                {
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
    if let Err(source) = transport.shutdown().await
        && session_error.is_none()
    {
        session_error = Some(ServerNetworkError::Io(source));
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
) -> Result<(), ServerNetworkError> {
    run_server_network_client_session_with_pre_hello_timeout(
        stream,
        peer_ip,
        client_id,
        runtime,
        client_event_senders,
        transport_action_sink,
        std::time::Duration::from_secs_f64(PROTOCOL_TIMEOUT_SECONDS),
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
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let accepted_client = accepted;
                let accepted_error = accepted_client.is_err();
                tokio::select! {
                    sent = accepted_clients.send(accepted_client) => {
                        if sent.is_err() {
                            break;
                        }
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
                if accepted_error {
                    break;
                }
            }
        }
    }
    Ok(())
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
    let mut accept_tasks: Vec<JoinHandle<()>> = Vec::new();
    for listener in listeners {
        let accepted_tx = accepted_tx.clone();
        let listener_shutdown_rx = shutdown_rx.clone();
        accept_tasks.push(tokio::spawn(async move {
            let _ = accept_server_network_clients_until_shutdown(
                listener,
                accepted_tx,
                listener_shutdown_rx,
            )
            .await;
        }));
    }
    drop(accepted_tx);

    let mut session_tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut next_client_number: u64 = 1;
    let mut tick = time::interval(std::time::Duration::from_secs_f64(
        SERVER_NETWORK_TICK_INTERVAL_SECONDS,
    ));
    let mut loop_error: Option<ServerNetworkError> = None;

    loop {
        prune_finished_session_tasks(&mut session_tasks).await;
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
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
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
                let peer_ip = Some(address.ip().to_string());
                let task_client_id = client_id.clone();
                session_tasks.push(tokio::spawn(async move {
                    if let Err(source) = run_server_network_client_session(
                        stream,
                        peer_ip,
                        client_id,
                        runtime,
                        client_event_senders,
                        transport_action_sink,
                    )
                    .await
                    {
                        eprintln!(
                            "Sorotte server client session {task_client_id} ended with error: {source}"
                        );
                    }
                }));
            }
        }
    }

    for task in accept_tasks {
        task.abort();
    }
    for task in session_tasks {
        task.abort();
    }

    if let Some(loop_error) = loop_error {
        return Err(loop_error);
    }

    Ok(())
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
}
