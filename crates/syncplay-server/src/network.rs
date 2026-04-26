use super::*;

async fn dispatch_outbound_lines_to_clients(
    client_line_senders: &SharedClientLineSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
) {
    for line in outbound_lines {
        let line_sender = {
            let senders = client_line_senders.lock().await;
            senders.get(&line.client_id).cloned()
        };
        if let Some(line_sender) = line_sender {
            let _ = line_sender.send(line.line);
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

async fn write_network_line_to_stream<S>(stream: &mut S, line: &str) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
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
        }
    }

    async fn write_line(&mut self, line: &str) -> io::Result<()> {
        match self {
            Self::Plain(stream) => write_network_line_to_stream(stream, line).await,
            Self::Tls(stream) => write_network_line_to_stream(stream.as_mut(), line).await,
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.shutdown().await,
            Self::Tls(stream) => stream.shutdown().await,
            Self::Closed => Ok(()),
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
        }
    }
}

async fn route_outbound_lines_for_client_session(
    transport: &mut ServerNetworkTransport,
    client_id: &str,
    client_line_senders: &SharedClientLineSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
) -> io::Result<()> {
    let mut peer_outbound_lines = Vec::new();
    for line in outbound_lines {
        if line.client_id == client_id {
            transport.write_line(&line.line).await?;
        } else {
            peer_outbound_lines.push(line);
        }
    }
    dispatch_outbound_lines_to_clients(client_line_senders, peer_outbound_lines).await;
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
) -> io::Result<()> {
    let should_start_tls = transport_actions.iter().any(|action| {
        action.client_id == client_id && action.action == ServerTransportAction::StartTls
    });
    if !should_start_tls || transport.is_tls() {
        return Ok(());
    }
    let tls_acceptor = tls_acceptor_from_runtime(runtime).await?;
    let current_transport = std::mem::replace(transport, ServerNetworkTransport::Closed);
    *transport = current_transport.upgrade_to_tls(tls_acceptor).await?;
    Ok(())
}

async fn run_server_network_client_session(
    stream: TcpStream,
    client_id: String,
    runtime: Arc<Mutex<ServerRuntime>>,
    client_line_senders: SharedClientLineSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
) -> Result<(), ServerNetworkError> {
    let (line_tx, mut line_rx): (UnboundedSender<String>, UnboundedReceiver<String>) =
        unbounded_channel();
    {
        let mut senders = client_line_senders.lock().await;
        senders.insert(client_id.clone(), line_tx);
    }

    let mut transport = ServerNetworkTransport::Plain(stream);
    let mut session_error: Option<ServerNetworkError> = None;
    loop {
        tokio::select! {
            inbound_line_result = transport.read_line() => {
                let inbound_line = match inbound_line_result {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(source) => {
                        session_error = Some(ServerNetworkError::Io(source));
                        break;
                    }
                };
                if inbound_line.is_empty() {
                    continue;
                }
                let dispatch = {
                    let mut runtime_guard = runtime.lock().await;
                    runtime_guard.handle_line_fanout_with_transport_actions(&client_id, &inbound_line)
                };
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
                    &client_line_senders,
                    dispatch.outbound_lines,
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
            outbound_line = line_rx.recv() => {
                let Some(outbound_line) = outbound_line else {
                    break;
                };
                if let Err(source) = transport.write_line(&outbound_line).await {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
            }
        }
    }

    {
        let mut senders = client_line_senders.lock().await;
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
            dispatch_outbound_lines_to_clients(&client_line_senders, outbound_lines).await;
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

pub async fn run_server_network_loop_until_shutdown(
    listener: TcpListener,
    runtime: Arc<Mutex<ServerRuntime>>,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    let client_line_senders: SharedClientLineSenders = Arc::new(Mutex::new(BTreeMap::new()));
    let mut session_tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut next_client_number: u64 = 1;
    let mut tick = time::interval(std::time::Duration::from_secs_f64(
        SERVER_NETWORK_TICK_INTERVAL_SECONDS,
    ));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let outbound_lines = {
                    let mut runtime_guard = runtime.lock().await;
                    runtime_guard.advance_time_and_collect_fanout(SERVER_NETWORK_TICK_INTERVAL_SECONDS)?
                };
                dispatch_outbound_lines_to_clients(&client_line_senders, outbound_lines).await;
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let client_id = format!("client-{next_client_number}");
                next_client_number = next_client_number.saturating_add(1);
                let runtime = runtime.clone();
                let client_line_senders = client_line_senders.clone();
                let transport_action_sink = transport_action_sink.clone();
                session_tasks.push(tokio::spawn(async move {
                    let _ = run_server_network_client_session(
                        stream,
                        client_id,
                        runtime,
                        client_line_senders,
                        transport_action_sink,
                    )
                    .await;
                }));
            }
        }
    }

    for task in session_tasks {
        task.abort();
    }

    Ok(())
}
