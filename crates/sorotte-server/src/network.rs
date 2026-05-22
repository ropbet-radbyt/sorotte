use super::*;
use std::net::SocketAddr;

type AcceptedClient = io::Result<(TcpStream, SocketAddr)>;

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
        dispatch_client_event(
            client_event_senders,
            &line.client_id,
            ClientOutboundEvent::Line(line.line),
        )
        .await;
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
    if let Some(event_sender) = event_sender
        && event_sender.try_send(event).is_err()
    {
        let mut senders = client_event_senders.lock().await;
        senders.remove(client_id);
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

pub(crate) async fn read_network_line_from_stream<S>(stream: &mut S) -> io::Result<Option<String>>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
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

    String::from_utf8(bytes).map(Some).map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("inbound protocol line is not valid utf-8: {source}"),
        )
    })
}

#[derive(Debug)]
enum ServerNetworkTransport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Closed,
    #[cfg(test)]
    StalledWrite,
}

impl ServerNetworkTransport {
    fn is_tls(&self) -> bool {
        matches!(self, Self::Tls(_))
    }

    async fn read_line(&mut self) -> io::Result<Option<String>> {
        match self {
            Self::Plain(stream) => read_network_line_from_stream(stream).await,
            Self::Tls(stream) => read_network_line_from_stream(stream.as_mut()).await,
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
            Self::Plain(stream) => write_network_line_to_stream(stream, line).await,
            Self::Tls(stream) => write_network_line_to_stream(stream.as_mut(), line).await,
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
            Self::Plain(stream) => stream.shutdown().await,
            Self::Tls(stream) => stream.shutdown().await,
            Self::Closed => Ok(()),
            #[cfg(test)]
            Self::StalledWrite => Ok(()),
        }
    }

    async fn upgrade_to_tls(self, acceptor: TlsAcceptor) -> io::Result<Self> {
        match self {
            Self::Plain(stream) => {
                let tls_stream = acceptor.accept(stream).await?;
                Ok(Self::Tls(Box::new(tls_stream)))
            }
            Self::Tls(stream) => Ok(Self::Tls(stream)),
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

async fn tls_acceptor_from_runtime(runtime: &Arc<Mutex<ServerRuntime>>) -> io::Result<TlsAcceptor> {
    let tls_server_config = {
        let runtime_guard = runtime.lock().await;
        runtime_guard.tls_server_config()
    };
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
    runtime: &Arc<Mutex<ServerRuntime>>,
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
    runtime: Arc<Mutex<ServerRuntime>>,
    client_event_senders: SharedClientEventSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    timeouts: ServerNetworkClientSessionTimeouts,
) -> Result<(), ServerNetworkError> {
    let (event_tx, mut event_rx): (Sender<ClientOutboundEvent>, Receiver<ClientOutboundEvent>) =
        channel(CLIENT_OUTBOUND_QUEUE_CAPACITY);
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert(client_id.clone(), event_tx);
    }

    let mut transport = ServerNetworkTransport::Plain(stream);
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
                let dispatch = {
                    let mut runtime_guard = runtime.lock().await;
                    let dispatch = runtime_guard.handle_line_fanout_with_transport_actions_for_peer(
                        &client_id,
                        inbound_line,
                        peer_ip.as_deref(),
                    );
                    let session_exists = runtime_guard.session(&client_id).is_some();
                    (dispatch, session_exists)
                };
                let (dispatch, session_exists) = dispatch;
                session_known = session_known || session_exists;
                let dispatch = match dispatch {
                    Ok(dispatch) => dispatch,
                    Err(source) => {
                        session_error = Some(ServerNetworkError::Runtime(source));
                        break;
                    }
                };
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
            outbound_event = event_rx.recv() => {
                let Some(outbound_event) = outbound_event else {
                    break;
                };
                match outbound_event {
                    ClientOutboundEvent::Line(outbound_line) => {
                        if let Err(source) =
                            transport
                                .write_line_with_timeout(&outbound_line, timeouts.write)
                                .await
                        {
                            session_error = Some(ServerNetworkError::Io(source));
                            break;
                        }
                    }
                    ClientOutboundEvent::TransportAction(ServerTransportAction::Close) => {
                        break;
                    }
                    ClientOutboundEvent::TransportAction(ServerTransportAction::StartTls) => {
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
        }
    }

    {
        let mut senders = client_event_senders.lock().await;
        senders.remove(&client_id);
    }
    if let Err(source) = transport.shutdown().await
        && session_error.is_none()
    {
        session_error = Some(ServerNetworkError::Io(source));
    }

    let disconnect_fanout = {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.handle_transport_disconnect_fanout(&client_id)
    };
    match disconnect_fanout {
        Ok(outbound_lines) => {
            dispatch_outbound_lines_to_clients(&client_event_senders, outbound_lines).await;
        }
        Err(source) => {
            if session_error.is_none() {
                session_error = Some(ServerNetworkError::Runtime(source));
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
    runtime: Arc<Mutex<ServerRuntime>>,
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
        }],
        write_timeout,
    )
    .await
}

async fn run_server_network_client_session(
    stream: TcpStream,
    peer_ip: Option<String>,
    client_id: String,
    runtime: Arc<Mutex<ServerRuntime>>,
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
    runtime: Arc<Mutex<ServerRuntime>>,
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
                let dispatch = {
                    let mut runtime_guard = runtime.lock().await;
                    runtime_guard.collect_dispatch_at(current_unix_timestamp_seconds())
                };
                let dispatch = match dispatch {
                    Ok(dispatch) => dispatch,
                    Err(source) => {
                        loop_error = Some(ServerNetworkError::Runtime(source));
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
    runtime: Arc<Mutex<ServerRuntime>>,
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
