use super::*;

#[derive(Debug)]
struct FragmentedAsyncReader {
    bytes: Vec<u8>,
    offset: usize,
    max_fragment_bytes: usize,
    read_calls: usize,
}

impl FragmentedAsyncReader {
    fn new(bytes: impl Into<Vec<u8>>, max_fragment_bytes: usize) -> Self {
        assert!(max_fragment_bytes > 0);
        Self {
            bytes: bytes.into(),
            offset: 0,
            max_fragment_bytes,
            read_calls: 0,
        }
    }
}

impl tokio::io::AsyncRead for FragmentedAsyncReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        self.read_calls += 1;
        if self.offset == self.bytes.len() {
            return std::task::Poll::Ready(Ok(()));
        }

        let bytes_to_copy = self
            .max_fragment_bytes
            .min(buffer.remaining())
            .min(self.bytes.len() - self.offset);
        let end = self.offset + bytes_to_copy;
        buffer.put_slice(&self.bytes[self.offset..end]);
        self.offset = end;
        std::task::Poll::Ready(Ok(()))
    }
}

async fn connected_tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let (client_stream, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
    let (server_stream, _) = accepted.expect("server should accept client");
    (client_stream.expect("client should connect"), server_stream)
}

fn server_actor_with_tls_cert_path(cert_path: &Path) -> ServerActorHandle {
    let mut model = ServerRuntime::new();
    model.set_tls_cert_path(Some(cert_path.to_path_buf()));
    ServerActorHandle::spawn(model)
}

#[tokio::test]
async fn server_network_rejects_line_over_max_bytes() {
    let (mut client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let session_task = tokio::spawn(
        crate::network::run_server_network_client_session_with_pre_hello_timeout(
            server_stream,
            None,
            "client-1".to_owned(),
            runtime,
            client_event_senders,
            None,
            Duration::from_secs(2),
        ),
    );

    let too_long_line = vec![b'x'; crate::MAX_PROTOCOL_LINE_BYTES + 1];
    client_stream
        .write_all(&too_long_line)
        .await
        .expect("oversized line bytes should write");
    client_stream
        .flush()
        .await
        .expect("oversized line should flush");

    let response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut client_stream),
    )
    .await
    .expect("error response should arrive before timeout")
    .expect("error response read should succeed")
    .expect("error response line should be present");
    let response = decode_message_line(&response_line).expect("error response should decode");
    let ProtocolMessage::Error(payload) = response else {
        panic!("oversized line should receive protocol Error");
    };
    assert_eq!(payload.error.message, crate::PROTOCOL_LINE_TOO_LONG_ERROR);

    let mut byte = [0_u8; 1];
    let bytes_read = timeout(Duration::from_secs(2), client_stream.read(&mut byte))
        .await
        .expect("connection close should arrive before timeout")
        .expect("closed connection read should succeed");
    assert_eq!(bytes_read, 0);
    assert!(
        session_task
            .await
            .expect("session task should join")
            .is_err(),
        "oversized line should end the session with an IO error"
    );
}

#[tokio::test]
async fn server_network_accepts_media_match_line_above_default_protocol_limit() {
    let (mut client_stream, mut server_stream) = connected_tcp_pair().await;
    let signature = "a".repeat(sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES + 1024);
    let line = format!(
        r#"{{"Set":{{"file":{{"name":"episode.mkv","duration":100.0,"mediaMatch":{{"schema":"sorotte.mediaMatch.v3","profiles":[{{"profile":"audio-constellation-v3","algorithmVersion":3,"durationMs":100000,"audio":{{"algorithm":"sorotte-audio-constellation-v3-sampled-fast","timeBaseMs":1,"anchors":"{signature}"}}}}]}}}}}}}}"#
    );
    assert!(line.len() > sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    assert!(line.len() <= crate::MAX_PROTOCOL_LINE_BYTES);

    client_stream
        .write_all(line.as_bytes())
        .await
        .expect("large media-match line bytes should write");
    client_stream
        .write_all(b"\n")
        .await
        .expect("large media-match line should terminate");
    client_stream
        .flush()
        .await
        .expect("large media-match line should flush");

    let received_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut server_stream),
    )
    .await
    .expect("large media-match line should arrive before timeout")
    .expect("large media-match line read should succeed")
    .expect("large media-match line should be present");

    assert_eq!(received_line, line);
    decode_message_line(&received_line).expect("large media-match line should decode");
}

#[tokio::test]
async fn server_network_buffered_line_reader_keeps_partial_line_after_cancel() {
    let (mut client_stream, mut server_stream) = connected_tcp_pair().await;
    let mut read_buffer = Vec::new();
    let prefix = br#"{"State":{"ping":{"clientRtt":0.0446634"#;
    let suffix = br#"29260253906}}}"#;
    let expected_line = r#"{"State":{"ping":{"clientRtt":0.044663429260253906}}}"#;

    client_stream
        .write_all(prefix)
        .await
        .expect("partial line prefix should write");
    client_stream
        .flush()
        .await
        .expect("partial line prefix should flush");

    let cancelled = timeout(
        Duration::from_millis(50),
        crate::network::read_network_line_from_stream_with_buffer(
            &mut server_stream,
            &mut read_buffer,
        ),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "read without a newline should remain pending until cancelled"
    );
    assert_eq!(
        read_buffer, prefix,
        "cancelled reads must not discard bytes already consumed from the stream"
    );

    client_stream
        .write_all(suffix)
        .await
        .expect("partial line suffix should write");
    client_stream
        .write_all(b"\r\n")
        .await
        .expect("line terminator should write");
    client_stream
        .flush()
        .await
        .expect("completed line should flush");

    let received_line = timeout(
        Duration::from_secs(2),
        crate::network::read_network_line_from_stream_with_buffer(
            &mut server_stream,
            &mut read_buffer,
        ),
    )
    .await
    .expect("completed line should arrive before timeout")
    .expect("completed line read should succeed")
    .expect("completed line should be present");

    assert_eq!(received_line, expected_line);
    serde_json::from_str::<Value>(&received_line).expect("completed line should be valid JSON");
    assert!(
        read_buffer.is_empty(),
        "successful reads should clear the persistent line buffer"
    );
}

#[tokio::test]
async fn server_network_buffered_line_reader_accepts_one_byte_fragmentation() {
    let input = b"fragmented\r\n";
    let mut stream = FragmentedAsyncReader::new(input, 1);
    let mut read_buffer = Vec::new();

    let line =
        crate::network::read_network_line_from_stream_with_buffer(&mut stream, &mut read_buffer)
            .await
            .expect("fragmented line read should succeed")
            .expect("fragmented line should be present");

    assert_eq!(line, "fragmented");
    assert_eq!(stream.read_calls, input.len());
    assert!(read_buffer.is_empty());
}

#[tokio::test]
async fn server_network_buffered_line_reader_retains_multiple_lines_from_one_read() {
    let input = b"bare-lf\ncrlf\r\n";
    let mut stream = FragmentedAsyncReader::new(input, input.len());
    let mut read_buffer = Vec::new();

    let first =
        crate::network::read_network_line_from_stream_with_buffer(&mut stream, &mut read_buffer)
            .await
            .expect("first buffered line read should succeed");
    assert_eq!(first.as_deref(), Some("bare-lf"));
    assert_eq!(stream.read_calls, 1);
    assert_eq!(read_buffer, b"crlf\r\n");

    let second =
        crate::network::read_network_line_from_stream_with_buffer(&mut stream, &mut read_buffer)
            .await
            .expect("second buffered line read should succeed");
    assert_eq!(second.as_deref(), Some("crlf"));
    assert_eq!(
        stream.read_calls, 1,
        "a complete buffered line must not perform another asynchronous read"
    );
    assert!(read_buffer.is_empty());
}

#[tokio::test]
async fn server_network_buffered_line_reader_enforces_hard_limit() {
    let mut at_limit = vec![b'x'; crate::MAX_PROTOCOL_LINE_BYTES];
    at_limit.push(b'\n');
    let mut at_limit_stream = FragmentedAsyncReader::new(at_limit, usize::MAX);
    let mut read_buffer = Vec::new();
    let line = crate::network::read_network_line_from_stream_with_buffer(
        &mut at_limit_stream,
        &mut read_buffer,
    )
    .await
    .expect("line exactly at the hard limit should succeed")
    .expect("line exactly at the hard limit should be present");
    assert_eq!(line.len(), crate::MAX_PROTOCOL_LINE_BYTES);

    let mut at_limit_crlf = vec![b'y'; crate::MAX_PROTOCOL_LINE_BYTES];
    at_limit_crlf.extend_from_slice(b"\r\n");
    let mut at_limit_crlf_stream = FragmentedAsyncReader::new(at_limit_crlf, usize::MAX);
    let line = crate::network::read_network_line_from_stream_with_buffer(
        &mut at_limit_crlf_stream,
        &mut read_buffer,
    )
    .await
    .expect("CRLF line exactly at the hard limit should succeed")
    .expect("CRLF line exactly at the hard limit should be present");
    assert_eq!(line.len(), crate::MAX_PROTOCOL_LINE_BYTES);
    assert!(line.bytes().all(|byte| byte == b'y'));

    let mut over_limit = vec![b'x'; crate::MAX_PROTOCOL_LINE_BYTES + 1];
    over_limit.push(b'\n');
    let mut over_limit_stream = FragmentedAsyncReader::new(over_limit, usize::MAX);
    let error = crate::network::read_network_line_from_stream_with_buffer(
        &mut over_limit_stream,
        &mut read_buffer,
    )
    .await
    .expect_err("line over the hard limit should fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), crate::PROTOCOL_LINE_TOO_LONG_ERROR);
    assert!(
        read_buffer.len() <= crate::MAX_PROTOCOL_LINE_BYTES + 2,
        "the bounded reader must not accumulate beyond framing and one sentinel byte"
    );
}

#[tokio::test]
async fn server_network_buffered_line_reader_accepts_final_line_at_eof() {
    let input = b"final line without newline";
    let mut stream = FragmentedAsyncReader::new(input, input.len());
    let mut read_buffer = Vec::new();

    let line =
        crate::network::read_network_line_from_stream_with_buffer(&mut stream, &mut read_buffer)
            .await
            .expect("final unterminated line read should succeed");
    assert_eq!(line.as_deref(), Some("final line without newline"));
    assert!(read_buffer.is_empty());
}

#[tokio::test]
async fn server_network_buffered_line_reader_rejects_invalid_utf8() {
    let mut stream = FragmentedAsyncReader::new(vec![0xff, b'\n'], 2);
    let mut read_buffer = Vec::new();

    let error =
        crate::network::read_network_line_from_stream_with_buffer(&mut stream, &mut read_buffer)
            .await
            .expect_err("invalid UTF-8 should fail");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("not valid utf-8"));
}

#[tokio::test]
async fn server_network_closes_pre_hello_idle_client() {
    let (mut client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let session_task = tokio::spawn(
        crate::network::run_server_network_client_session_with_pre_hello_timeout(
            server_stream,
            None,
            "client-1".to_owned(),
            runtime,
            client_event_senders,
            None,
            Duration::from_millis(20),
        ),
    );

    let mut byte = [0_u8; 1];
    let bytes_read = timeout(Duration::from_secs(2), client_stream.read(&mut byte))
        .await
        .expect("idle close should arrive before timeout")
        .expect("closed connection read should succeed");
    assert_eq!(bytes_read, 0);
    session_task
        .await
        .expect("session task should join")
        .expect("idle pre-hello close should not be an error");
}

#[tokio::test]
async fn server_network_does_not_create_session_for_pre_hello_idle_client() {
    let (_client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));

    crate::network::run_server_network_client_session_with_pre_hello_timeout(
        server_stream,
        None,
        "client-1".to_owned(),
        runtime.clone(),
        client_event_senders,
        None,
        Duration::from_millis(20),
    )
    .await
    .expect("idle pre-hello close should not be an error");

    assert!(
        runtime
            .session("client-1")
            .await
            .expect("server actor should answer session query")
            .is_none(),
        "idle pre-hello clients should never create a runtime session"
    );
}

#[tokio::test]
async fn server_network_prunes_finished_session_tasks() {
    let mut session_tasks = vec![tokio::spawn(async {})];
    tokio::task::yield_now().await;

    timeout(
        Duration::from_secs(1),
        crate::network::prune_finished_session_tasks(&mut session_tasks),
    )
    .await
    .expect("session task pruning should complete before timeout");

    assert!(session_tasks.is_empty());
}

#[tokio::test]
async fn server_network_tick_does_not_accumulate_simulated_time() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let mut model = ServerRuntime::new();
    model.set_time_now_override_seconds(Some(50.0));
    let runtime = ServerActorHandle::spawn(model);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime.clone(),
        None,
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(600)).await;
    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    assert_eq!(
        runtime
            .time_now_override_seconds()
            .await
            .expect("server actor should answer time query"),
        Some(50.0),
        "production network ticks should not advance the deterministic override"
    );
}

#[tokio::test]
async fn server_network_shutdown_closes_and_awaits_active_sessions() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime.clone(),
        None,
        shutdown_rx,
    ));

    let stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello should write");
    writer
        .write_all(b"\n")
        .await
        .expect("hello newline should write");
    writer.flush().await.expect("hello should flush");

    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let read = timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .expect("hello response should arrive before timeout")
            .expect("hello response should read");
        assert!(read > 0, "server should not close before Hello response");
        if matches!(
            decode_message_line(line.trim_end()).expect("response should decode"),
            ProtocolMessage::Hello(_)
        ) {
            break;
        }
    }
    assert!(
        runtime
            .session("client-1")
            .await
            .expect("actor should answer before shutdown")
            .is_some(),
        "active network client should own a runtime session"
    );

    shutdown_tx.send(true).expect("shutdown signal should send");
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("network shutdown should stay within its grace deadline")
        .expect("server network task should join")
        .expect("server network shutdown should succeed");

    timeout(Duration::from_secs(1), async {
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .await
                .expect("shutdown read should succeed")
                == 0
            {
                break;
            }
        }
    })
    .await
    .expect("active client transport should receive EOF during shutdown");
    assert!(
        runtime
            .session("client-1")
            .await
            .expect("actor should answer after network shutdown")
            .is_none(),
        "network owner must await the session's normal disconnect cleanup"
    );
    assert!(
        TcpStream::connect(address).await.is_err(),
        "listener must be closed before network shutdown returns"
    );

    runtime
        .shutdown()
        .await
        .expect("test actor should shut down cleanly");
}

#[tokio::test]
async fn server_lifecycle_explicitly_shuts_down_actor_after_network_teardown() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let retained_probe = runtime.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    shutdown_tx.send(true).expect("shutdown signal should send");

    run_server_network_loops_and_shutdown_actor(vec![listener], runtime, None, shutdown_rx)
        .await
        .expect("lifecycle should drain the network and actor");

    assert!(
        matches!(
            retained_probe.session("probe").await,
            Err(ServerActorError::Unavailable)
        ),
        "a retained sender clone proves the actor stopped explicitly rather than through Drop"
    );
}

#[tokio::test]
async fn server_lifecycle_preserves_network_and_actor_shutdown_failures() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    runtime
        .clone()
        .shutdown()
        .await
        .expect("precondition actor shutdown should succeed");
    let (_shutdown_tx, shutdown_rx) = watch::channel(true);

    let error = run_server_network_loops_and_shutdown_actor(vec![], runtime, None, shutdown_rx)
        .await
        .expect_err("network and actor shutdown failures should be observable");

    assert!(
        matches!(
            error,
            ServerLifecycleError::NetworkAndShutdown {
                network: ServerNetworkError::Io(_),
                shutdown: ServerActorError::Unavailable,
            }
        ),
        "lifecycle errors must retain both the network failure and durability-barrier failure"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_actor_remains_responsive_while_room_persistence_waits_on_sqlite() {
    let db_path = temporary_sqlite_path("actor-sqlite-contention");
    let _ = fs::remove_file(&db_path);
    let mut model = ServerRuntime::with_persistent_rooms_enabled(true);
    model
        .set_persistent_rooms_db_path(Some(db_path.clone()))
        .expect("room persistence should initialize");
    let runtime = ServerActorHandle::spawn(model);
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
            None,
        )
        .await
        .expect("hello command should succeed");

    let blocker = Connection::open(&db_path).expect("blocking sqlite connection should open");
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .expect("blocking sqlite transaction should begin");

    timeout(
        Duration::from_millis(250),
        runtime.handle_line(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"]}}}"#,
            None,
        ),
    )
    .await
    .expect("model transition must not wait for sqlite busy timeout")
    .expect("playlist transition should succeed");
    timeout(
        Duration::from_millis(250),
        runtime.handle_line("client-1", r#"{"List":null}"#, None),
    )
    .await
    .expect("a second command must progress while persistence is blocked")
    .expect("list command should succeed");

    blocker
        .execute_batch("ROLLBACK")
        .expect("blocking sqlite transaction should release");
    drop(blocker);
    runtime
        .shutdown()
        .await
        .expect("actor shutdown should flush and join persistence workers");
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

fn test_outbound_line(
    line: impl Into<String>,
    delivery: ServerOutboundDelivery,
) -> DirectedOutboundLine {
    DirectedOutboundLine {
        client_id: "client-1".to_owned(),
        line: line.into(),
        delivery,
    }
}

async fn fill_reliable_outbound_queue(
    client_event_senders: &crate::network::SharedClientEventSenders,
) {
    let lines = (0..crate::CLIENT_OUTBOUND_QUEUE_CAPACITY).map(|index| {
        test_outbound_line(format!("queued-{index}"), ServerOutboundDelivery::Reliable)
    });
    crate::network::dispatch_outbound_lines_to_clients(client_event_senders, lines.collect()).await;
}

#[tokio::test]
async fn server_network_transient_full_queue_keeps_live_client_registered() {
    let metrics = crate::ServerOutboundBackpressureMetrics::default();
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, mut event_rx) = crate::network::client_event_queue(metrics.clone());
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert("client-1".to_owned(), event_tx);
    }
    fill_reliable_outbound_queue(&client_event_senders).await;

    let dispatch_senders = client_event_senders.clone();
    let overflow_dispatch = tokio::spawn(async move {
        crate::network::dispatch_outbound_lines_to_clients(
            &dispatch_senders,
            vec![test_outbound_line(
                r#"{"Chat":"retained"}"#,
                ServerOutboundDelivery::Reliable,
            )],
        )
        .await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        event_rx.receive_reliable_line_for_test().await.is_some(),
        "draining one queued line should release reliable capacity"
    );
    overflow_dispatch
        .await
        .expect("overflow dispatch task should join");

    assert!(client_event_senders.lock().await.contains_key("client-1"));
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.full_queue_events, 1);
    assert_eq!(snapshot.overload_disconnects, 0);
    assert_eq!(snapshot.dropped_messages, 0);
}

#[tokio::test]
async fn server_network_sustained_full_queue_signals_explicit_overload_close() {
    let metrics = crate::ServerOutboundBackpressureMetrics::default();
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, event_rx) = crate::network::client_event_queue(metrics.clone());
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert("client-1".to_owned(), event_tx);
    }
    fill_reliable_outbound_queue(&client_event_senders).await;

    crate::network::dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![test_outbound_line(
            r#"{"Chat":"overload"}"#,
            ServerOutboundDelivery::Reliable,
        )],
    )
    .await;

    assert!(
        !client_event_senders.lock().await.contains_key("client-1"),
        "an overloaded sender should be removed only after close is signalled"
    );
    assert_eq!(
        event_rx.overload_queue_depth_for_test(),
        Some(crate::CLIENT_OUTBOUND_QUEUE_CAPACITY)
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.full_queue_events, 1);
    assert_eq!(snapshot.overload_disconnects, 1);
    assert_eq!(snapshot.closed_channel_events, 0);
    assert_eq!(snapshot.dropped_messages, 1);
}

#[tokio::test]
async fn server_network_full_queue_drops_periodic_state_without_disconnect() {
    let metrics = crate::ServerOutboundBackpressureMetrics::default();
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, _event_rx) = crate::network::client_event_queue(metrics.clone());
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert("client-1".to_owned(), event_tx);
    }
    fill_reliable_outbound_queue(&client_event_senders).await;

    crate::network::dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![test_outbound_line(
            "periodic-state",
            ServerOutboundDelivery::CoalesciblePeriodicState,
        )],
    )
    .await;

    assert!(
        client_event_senders.lock().await.contains_key("client-1"),
        "a disposable periodic state must not overload-disconnect a client whose reliable lane is full"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.full_queue_events, 1);
    assert_eq!(snapshot.overload_disconnects, 0);
    assert_eq!(snapshot.dropped_messages, 1);
}

#[tokio::test]
async fn server_network_closed_queue_is_not_reported_as_overload() {
    let metrics = crate::ServerOutboundBackpressureMetrics::default();
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, event_rx) = crate::network::client_event_queue(metrics.clone());
    drop(event_rx);
    client_event_senders
        .lock()
        .await
        .insert("client-1".to_owned(), event_tx);

    crate::network::dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![test_outbound_line(
            r#"{"Chat":"closed"}"#,
            ServerOutboundDelivery::Reliable,
        )],
    )
    .await;

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.closed_channel_events, 1);
    assert_eq!(snapshot.full_queue_events, 0);
    assert_eq!(snapshot.overload_disconnects, 0);
    assert_eq!(snapshot.dropped_messages, 1);
}

#[tokio::test]
async fn server_network_periodic_state_updates_coalesce_to_latest() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let metrics = runtime.outbound_backpressure_metrics();
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, mut event_rx) = crate::network::client_event_queue(metrics.clone());
    client_event_senders
        .lock()
        .await
        .insert("client-1".to_owned(), event_tx);

    crate::network::dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![
            test_outbound_line("state-1", ServerOutboundDelivery::CoalesciblePeriodicState),
            test_outbound_line("state-2", ServerOutboundDelivery::CoalesciblePeriodicState),
        ],
    )
    .await;

    assert_eq!(
        event_rx.receive_periodic_state_for_test().await.as_deref(),
        Some("state-2")
    );
    let snapshot = runtime.outbound_backpressure_snapshot();
    assert_eq!(snapshot.coalesced_state_updates, 1);
    assert_eq!(snapshot.queue_depth, 0);
    assert_eq!(snapshot.dropped_messages, 0);
    runtime
        .shutdown()
        .await
        .expect("server actor should shut down cleanly");
}

#[tokio::test]
async fn actor_commit_order_is_preserved_when_session_dispatches_resume_out_of_order() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    for (client_id, username) in [
        ("source-a", "alice"),
        ("source-b", "bob"),
        ("observer", "carol"),
    ] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"room"}},"version":"1.7.5"}}}}"#
                ),
                None,
            )
            .await
            .expect("hello actor command should succeed");
    }

    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, mut event_rx) =
        crate::network::client_event_queue(crate::ServerOutboundBackpressureMetrics::default());
    client_event_senders
        .lock()
        .await
        .insert("observer".to_owned(), event_tx);
    let dispatch_order: crate::network::SharedNetworkDispatchOrder = Arc::new(Mutex::new(()));

    let first_committed = Arc::new(tokio::sync::Notify::new());
    let release_first_session = Arc::new(tokio::sync::Notify::new());
    let first_runtime = runtime.clone();
    let first_senders = client_event_senders.clone();
    let first_dispatch_order = dispatch_order.clone();
    let first_committed_task = first_committed.clone();
    let release_first_session_task = release_first_session.clone();
    let first_session = tokio::spawn(async move {
        crate::network::handle_line_and_queue_peer_dispatch_after_commit_for_test(
            &first_runtime,
            &first_dispatch_order,
            "source-a",
            r#"{"Set":{"playlistChange":{"files":["first.mkv"]}}}"#,
            &first_senders,
            async move {
                first_committed_task.notify_one();
                // Hold the production dispatch helper across the exact
                // suspension point that previously let a later actor commit
                // overtake peer fanout.
                release_first_session_task.notified().await;
            },
        )
        .await
        .expect("first actor mutation and dispatch should succeed");
    });
    first_committed.notified().await;

    let second_runtime = runtime.clone();
    let second_senders = client_event_senders.clone();
    let second_dispatch_order = dispatch_order.clone();
    let second_session = tokio::spawn(async move {
        crate::network::handle_line_and_queue_peer_dispatch_after_commit_for_test(
            &second_runtime,
            &second_dispatch_order,
            "source-b",
            r#"{"Set":{"playlistChange":{"files":["second.mkv"]}}}"#,
            &second_senders,
            std::future::ready(()),
        )
        .await
        .expect("second actor mutation and dispatch should succeed");
    });
    tokio::task::yield_now().await;
    release_first_session.notify_one();
    first_session
        .await
        .expect("first session dispatch task should join");
    second_session
        .await
        .expect("second session dispatch task should join");

    let mut observed_playlists = Vec::new();
    while observed_playlists.len() < 2 {
        let line = timeout(
            Duration::from_secs(1),
            event_rx.receive_reliable_line_for_test(),
        )
        .await
        .expect("observer should receive both committed playlist changes")
        .expect("observer queue should remain open");
        if let ProtocolMessage::Set(payload) =
            decode_message_line(&line).expect("observer fanout should decode")
            && let Some(playlist) = payload.set.playlist_change
        {
            observed_playlists.push(playlist.files);
        }
    }

    assert_eq!(
        observed_playlists,
        vec![vec!["first.mkv".to_owned()], vec!["second.mkv".to_owned()]],
        "network fanout must preserve the actor's authoritative mutation order"
    );

    runtime
        .shutdown()
        .await
        .expect("server actor should shut down cleanly");
}

#[tokio::test]
async fn server_network_accept_queue_is_bounded() {
    let (accepted_tx, _accepted_rx): (
        mpsc::Sender<io::Result<(TcpStream, std::net::SocketAddr)>>,
        _,
    ) = mpsc::channel(crate::ACCEPTED_CLIENT_QUEUE_CAPACITY);
    for index in 0..crate::ACCEPTED_CLIENT_QUEUE_CAPACITY {
        accepted_tx
            .try_send(Err(io::Error::other(format!("fill-{index}"))))
            .expect("bounded accepted-client queue should accept entries up to capacity");
    }

    let overflow = accepted_tx.try_send(Err(io::Error::other("overflow")));

    assert!(
        matches!(overflow, Err(mpsc::error::TrySendError::Full(_))),
        "accepted-client queue should be bounded at configured capacity"
    );
}

#[tokio::test]
async fn server_network_write_timeout_closes_stalled_client() {
    let error = crate::network::stalled_transport_write_for_test(Duration::from_millis(20))
        .await
        .expect_err("stalled transport write should time out");

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        error.to_string().contains("server protocol write"),
        "write timeout error should name the failed operation"
    );
}

#[tokio::test]
async fn server_network_error_response_write_timeout_does_not_hang_session() {
    let error =
        crate::network::stalled_transport_error_response_write_for_test(Duration::from_millis(20))
            .await
            .expect_err("stalled error-response write should time out");

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn server_network_direct_response_write_timeout_does_not_block_loop() {
    let error =
        crate::network::stalled_transport_direct_response_write_for_test(Duration::from_millis(20))
            .await
            .expect_err("stalled direct response write should time out");

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn committed_peer_fanout_is_queued_before_a_stalled_source_write() {
    let (source_result, peer_line, committed_files) =
        crate::network::stalled_source_write_still_queues_peer_fanout_for_test(
            Duration::from_millis(20),
        )
        .await;

    assert_eq!(
        source_result
            .expect_err("source write should still report its timeout")
            .kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(committed_files, vec!["committed.mkv".to_owned()]);
    let peer_message = decode_message_line(
        peer_line
            .as_deref()
            .expect("peer mutation should be queued before the source timeout"),
    )
    .expect("queued peer mutation should decode");
    assert!(matches!(
        peer_message,
        ProtocolMessage::Set(payload)
            if payload.set.playlist_change.as_ref().is_some_and(|playlist| {
                playlist.files == ["committed.mkv"]
            })
    ));
}

#[tokio::test]
async fn server_network_loop_routes_hello_response_to_connected_client() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write");
    writer
        .write_all(b"\n")
        .await
        .expect("hello newline should write");
    writer.flush().await.expect("hello write should flush");

    let mut buffered_reader = BufReader::new(reader);
    let mut saw_hello = false;
    for _ in 0..4 {
        let mut line = String::new();
        let read_bytes = timeout(Duration::from_secs(2), buffered_reader.read_line(&mut line))
            .await
            .expect("server response should arrive before timeout")
            .expect("server response read should succeed");
        if read_bytes == 0 {
            break;
        }
        assert!(
            line.ends_with("\r\n"),
            "server protocol responses should use CRLF framing"
        );
        let message = decode_message_line(line.trim_end()).expect("response line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "network loop should route runtime hello response to connected client"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");
}

#[tokio::test]
async fn recovery_on_new_socket_closes_superseded_socket_through_cross_client_routing() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime.clone(),
        None,
        shutdown_rx,
    ));

    let old_stream = TcpStream::connect(address)
        .await
        .expect("old client should connect");
    let (old_reader, mut old_writer) = old_stream.into_split();
    let mut old_reader = BufReader::new(old_reader);
    old_writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}
"#)
        .await
        .expect("old Hello should write");
    old_writer.flush().await.expect("old Hello should flush");
    timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            assert!(
                old_reader
                    .read_line(&mut line)
                    .await
                    .expect("old Hello response should read")
                    > 0,
                "old socket should remain open through Hello"
            );
            if matches!(
                decode_message_line(line.trim_end()).expect("old response should decode"),
                ProtocolMessage::Hello(_)
            ) {
                break;
            }
        }
    })
    .await
    .expect("old Hello response should arrive");

    old_writer
        .write_all(br#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":501,"requestId":"network-overlap-operation","loadIntent":"newPlayback","logicalMediaId":"network-overlap-media","targetPosition":1.0,"policy":"controller"}}}}
"#)
        .await
        .expect("old prepare should write");
    old_writer.flush().await.expect("old prepare should flush");
    timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            assert!(
                old_reader
                    .read_line(&mut line)
                    .await
                    .expect("canonical prepare should read")
                    > 0,
                "old socket should remain open until recovery"
            );
            let message =
                decode_message_line(line.trim_end()).expect("old prepare response should decode");
            let ProtocolMessage::Set(set) = message else {
                continue;
            };
            if set
                .set
                .playback_barrier_v1()
                .ok()
                .flatten()
                .is_some_and(|extension| extension.prepare.is_some())
            {
                break;
            }
        }
    })
    .await
    .expect("canonical prepare should prove server acceptance");

    let new_stream = TcpStream::connect(address)
        .await
        .expect("replacement client should connect");
    let (new_reader, mut new_writer) = new_stream.into_split();
    let mut new_reader = BufReader::new(new_reader);
    new_writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}
"#)
        .await
        .expect("replacement Hello should write");
    new_writer
        .flush()
        .await
        .expect("replacement Hello should flush");
    timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            assert!(
                new_reader
                    .read_line(&mut line)
                    .await
                    .expect("replacement Hello response should read")
                    > 0,
                "replacement socket should stay connected"
            );
            if matches!(
                decode_message_line(line.trim_end()).expect("replacement response should decode"),
                ProtocolMessage::Hello(_)
            ) {
                break;
            }
        }
    })
    .await
    .expect("replacement Hello response should arrive");

    new_writer
        .write_all(br#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"network-overlap-operation","originalRequestNonce":501,"recoveryNonce":502,"logicalMediaId":"network-overlap-media"}}}}
"#)
        .await
        .expect("replacement recovery should write");
    new_writer
        .flush()
        .await
        .expect("replacement recovery should flush");

    timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            if old_reader
                .read_line(&mut line)
                .await
                .expect("old socket close should read cleanly")
                == 0
            {
                break;
            }
        }
    })
    .await
    .expect("recovery processed on client-2 must close client-1's socket");

    timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .session("client-1")
                .await
                .expect("actor should answer old-session query")
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("old socket EOF should complete model disconnect cleanup");
    assert!(
        runtime
            .session("client-2")
            .await
            .expect("actor should answer replacement-session query")
            .is_some(),
        "cross-client Close must not close the recovery source"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");
    runtime
        .shutdown()
        .await
        .expect("test actor should shut down cleanly");
}

#[tokio::test]
async fn server_network_loop_passes_peer_ip_to_motd_template() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let mut model = ServerRuntime::new();
    model.set_motd_template(Some("Peer=$userIp User=$username Room=$room".to_owned()));
    let runtime = ServerActorHandle::spawn(model);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#)
        .await
        .expect("hello line should write");
    writer
        .write_all(b"\n")
        .await
        .expect("hello newline should write");
    writer.flush().await.expect("hello write should flush");

    let mut buffered_reader = BufReader::new(reader);
    let mut motd = None;
    for _ in 0..4 {
        let mut line = String::new();
        let read_bytes = timeout(Duration::from_secs(2), buffered_reader.read_line(&mut line))
            .await
            .expect("server response should arrive before timeout")
            .expect("server response read should succeed");
        if read_bytes == 0 {
            break;
        }
        let message = decode_message_line(line.trim_end()).expect("response line should decode");
        if let ProtocolMessage::Hello(_) = message {
            let hello = extract_hello_from_message(message).expect("hello should extract");
            motd = hello
                .extra
                .get("motd")
                .and_then(Value::as_str)
                .map(str::to_owned);
            break;
        }
    }
    assert_eq!(
        motd.as_deref(),
        Some("Peer=127.0.0.1 User=alice Room=room1")
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");
}

#[tokio::test]
async fn server_network_loop_ignores_whitespace_only_lines() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(b"   \t\r\n")
        .await
        .expect("whitespace line should write");
    writer
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write");
    writer
        .write_all(b"\n")
        .await
        .expect("hello newline should write");
    writer.flush().await.expect("client writes should flush");

    let mut buffered_reader = BufReader::new(reader);
    let mut saw_hello = false;
    for _ in 0..5 {
        let mut line = String::new();
        let read_bytes = timeout(Duration::from_secs(2), buffered_reader.read_line(&mut line))
            .await
            .expect("server response should arrive before timeout")
            .expect("server response read should succeed");
        if read_bytes == 0 {
            break;
        }
        let message = decode_message_line(line.trim_end()).expect("response line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "whitespace-only protocol lines should be ignored before the next command"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");
}

#[tokio::test]
async fn server_network_loop_sends_error_for_invalid_utf8_line() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let mut stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    stream
        .write_all(&[0xff, b'\n'])
        .await
        .expect("invalid utf8 line should write");
    stream.flush().await.expect("invalid line should flush");

    let response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut stream),
    )
    .await
    .expect("error response should arrive before timeout")
    .expect("error response read should succeed")
    .expect("error response line should be present");
    let error_response = decode_message_line(&response_line).expect("error response should decode");
    let ProtocolMessage::Error(payload) = error_response else {
        panic!("invalid utf8 should receive protocol Error");
    };
    assert_eq!(payload.error.message, LEGACY_SERVER_LINE_DECODE_ERROR);

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");
}

#[tokio::test]
async fn server_network_closes_starttls_client_that_never_handshakes() {
    let cert_path = temporary_directory_path("tls-handshake-timeout-close");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = server_actor_with_tls_cert_path(&cert_path);
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let session_task = tokio::spawn(
        crate::network::run_server_network_client_session_with_timeouts(
            server_stream,
            None,
            "client-1".to_owned(),
            runtime,
            client_event_senders,
            None,
            crate::network::ServerNetworkClientSessionTimeouts::new(
                Duration::from_secs(2),
                Duration::from_millis(20),
                Duration::from_secs(2),
            ),
        ),
    );

    client_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    client_stream
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    client_stream
        .flush()
        .await
        .expect("tls request should flush");

    let tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut client_stream),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed")
    .expect("tls response line should be present");
    let ProtocolMessage::Tls(payload) =
        decode_message_line(&tls_response_line).expect("tls response should decode")
    else {
        panic!("server should respond with TLS payload");
    };
    assert_eq!(payload.tls.start_tls, "true");

    let mut byte = [0_u8; 1];
    let bytes_read = timeout(Duration::from_secs(2), client_stream.read(&mut byte))
        .await
        .expect("TLS handshake timeout close should arrive before timeout")
        .expect("closed connection read should succeed");
    assert_eq!(bytes_read, 0);

    let error = session_task
        .await
        .expect("session task should join")
        .expect_err("TLS handshake timeout should end the session with an error");
    let ServerNetworkError::Io(source) = error else {
        panic!("TLS handshake timeout should be an IO error");
    };
    assert_eq!(source.kind(), io::ErrorKind::TimedOut);

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_starttls_handshake_timeout_does_not_create_session() {
    let cert_path = temporary_directory_path("tls-handshake-timeout-no-session");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = server_actor_with_tls_cert_path(&cert_path);
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let session_task = tokio::spawn(
        crate::network::run_server_network_client_session_with_timeouts(
            server_stream,
            None,
            "client-1".to_owned(),
            runtime.clone(),
            client_event_senders,
            None,
            crate::network::ServerNetworkClientSessionTimeouts::new(
                Duration::from_secs(2),
                Duration::from_millis(20),
                Duration::from_secs(2),
            ),
        ),
    );

    client_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    client_stream
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    client_stream
        .flush()
        .await
        .expect("tls request should flush");

    let _ = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut client_stream),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed")
    .expect("tls response line should be present");

    session_task
        .await
        .expect("session task should join")
        .expect_err("TLS handshake timeout should end the session with an error");
    assert!(
        runtime
            .session("client-1")
            .await
            .expect("server actor should answer session query")
            .is_none(),
        "a client that only requests StartTLS should not create a runtime session"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_starttls_success_still_allows_hello() {
    let cert_path = temporary_directory_path("tls-handshake-timeout-success");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut client_stream, server_stream) = connected_tcp_pair().await;
    let runtime = server_actor_with_tls_cert_path(&cert_path);
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let session_task = tokio::spawn(
        crate::network::run_server_network_client_session_with_timeouts(
            server_stream,
            None,
            "client-1".to_owned(),
            runtime.clone(),
            client_event_senders,
            None,
            crate::network::ServerNetworkClientSessionTimeouts::new(
                Duration::from_secs(2),
                Duration::from_secs(2),
                Duration::from_secs(2),
            ),
        ),
    );

    client_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    client_stream
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    client_stream
        .flush()
        .await
        .expect("tls request should flush");

    let tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut client_stream),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed")
    .expect("tls response line should be present");
    let ProtocolMessage::Tls(payload) =
        decode_message_line(&tls_response_line).expect("tls response should decode")
    else {
        panic!("server should respond with TLS payload");
    };
    assert_eq!(payload.tls.start_tls, "true");

    let connector = tls_client_connector_for_test_fixture();
    let server_name = ServerName::try_from("localhost").expect("server name should parse");
    let mut tls_stream = timeout(
        Duration::from_secs(2),
        connector.connect(server_name, client_stream),
    )
    .await
    .expect("tls handshake should complete before timeout")
    .expect("tls handshake should succeed");

    tls_stream
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write over tls");
    tls_stream
        .write_all(b"\n")
        .await
        .expect("hello newline should write over tls");
    tls_stream
        .flush()
        .await
        .expect("hello line should flush over tls");

    let mut saw_hello = false;
    for _ in 0..4 {
        let maybe_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut tls_stream),
        )
        .await
        .expect("post-upgrade response should arrive before timeout")
        .expect("post-upgrade response read should succeed");
        let Some(line) = maybe_line else {
            break;
        };
        let message = decode_message_line(&line).expect("post-upgrade line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(saw_hello, "successful StartTLS should preserve Hello flow");
    assert!(
        runtime
            .session("client-1")
            .await
            .expect("server actor should answer session query")
            .is_some(),
        "successful StartTLS Hello should create the runtime session"
    );

    tls_stream
        .shutdown()
        .await
        .expect("tls stream should shut down");
    session_task
        .await
        .expect("session task should join")
        .expect("clean TLS client close should not fail the session");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_loop_forwards_tls_start_transport_action_to_sink() {
    let cert_path = temporary_directory_path("tls-network-loop");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = server_actor_with_tls_cert_path(&cert_path);
    let (transport_action_tx, mut transport_action_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        Some(transport_action_tx),
        shutdown_rx,
    ));

    let stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    writer
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    writer.flush().await.expect("tls request should flush");

    let mut buffered_reader = BufReader::new(reader);
    let mut response_line = String::new();
    timeout(
        Duration::from_secs(2),
        buffered_reader.read_line(&mut response_line),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed");
    let tls_response =
        decode_message_line(response_line.trim_end()).expect("tls response should decode");
    let ProtocolMessage::Tls(payload) = tls_response else {
        panic!("server should respond with TLS payload");
    };
    assert_eq!(payload.tls.start_tls, "true");

    let action = timeout(Duration::from_secs(2), transport_action_rx.recv())
        .await
        .expect("transport action should arrive before timeout")
        .expect("transport action channel should deliver StartTls");
    assert_eq!(action.client_id, "client-1");
    assert_eq!(action.action, ServerTransportAction::StartTls);

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_loop_tls_upgrade_preserves_post_upgrade_protocol_flow() {
    let cert_path = temporary_directory_path("tls-network-upgrade-flow");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = server_actor_with_tls_cert_path(&cert_path);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let mut stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    stream
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    stream.flush().await.expect("tls request should flush");

    let tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut stream),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed")
    .expect("tls response line should be present");
    let tls_response =
        decode_message_line(&tls_response_line).expect("tls response line should decode");
    let ProtocolMessage::Tls(payload) = tls_response else {
        panic!("server should respond with TLS payload");
    };
    assert_eq!(payload.tls.start_tls, "true");

    let connector = tls_client_connector_for_test_fixture();
    let server_name = ServerName::try_from("localhost").expect("server name should parse");
    let mut tls_stream = timeout(
        Duration::from_secs(2),
        connector.connect(server_name, stream),
    )
    .await
    .expect("tls handshake should complete before timeout")
    .expect("tls handshake should succeed");

    tls_stream
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write over tls");
    tls_stream
        .write_all(b"\n")
        .await
        .expect("hello newline should write over tls");
    tls_stream
        .flush()
        .await
        .expect("hello line should flush over tls");

    let mut saw_hello = false;
    for _ in 0..4 {
        let maybe_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut tls_stream),
        )
        .await
        .expect("post-upgrade response should arrive before timeout")
        .expect("post-upgrade response read should succeed");
        let Some(line) = maybe_line else {
            break;
        };
        if line.is_empty() {
            continue;
        }
        let message = decode_message_line(&line).expect("post-upgrade line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "server should continue protocol flow over upgraded TLS transport"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_loop_tls_upgrade_uses_cached_context_when_files_disappear() {
    let cert_path = temporary_directory_path("tls-network-upgrade-cached-context");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = server_actor_with_tls_cert_path(&cert_path);

    fs::remove_file(cert_path.join("privkey.pem")).expect("privkey file should be removable");
    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    fs::remove_file(cert_path.join("cert.pem")).expect("cert file should be removable");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let mut stream = TcpStream::connect(address)
        .await
        .expect("client should connect");
    stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("tls request line should write");
    stream
        .write_all(b"\n")
        .await
        .expect("tls request newline should write");
    stream.flush().await.expect("tls request should flush");

    let tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut stream),
    )
    .await
    .expect("tls response should arrive before timeout")
    .expect("tls response read should succeed")
    .expect("tls response line should be present");
    let tls_response =
        decode_message_line(&tls_response_line).expect("tls response line should decode");
    let ProtocolMessage::Tls(payload) = tls_response else {
        panic!("server should respond with TLS payload");
    };
    assert_eq!(
        payload.tls.start_tls, "true",
        "server should still accept TLS using cached loaded context"
    );

    let connector = tls_client_connector_for_test_fixture();
    let server_name = ServerName::try_from("localhost").expect("server name should parse");
    let mut tls_stream = timeout(
        Duration::from_secs(2),
        connector.connect(server_name, stream),
    )
    .await
    .expect("tls handshake should complete before timeout")
    .expect("tls handshake should succeed with cached context");

    tls_stream
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write over tls");
    tls_stream
        .write_all(b"\n")
        .await
        .expect("hello newline should write over tls");
    tls_stream
        .flush()
        .await
        .expect("hello line should flush over tls");

    let mut saw_hello = false;
    for _ in 0..4 {
        let maybe_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut tls_stream),
        )
        .await
        .expect("post-upgrade response should arrive before timeout")
        .expect("post-upgrade response read should succeed");
        let Some(line) = maybe_line else {
            break;
        };
        if line.is_empty() {
            continue;
        }
        let message = decode_message_line(&line).expect("post-upgrade line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "server should continue protocol flow over cached-context upgraded tls transport"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_loop_tls_upgrade_keeps_inflight_handshake_when_bundle_rotates_invalid_after_starttls_true()
 {
    let cert_path = temporary_directory_path("tls-network-upgrade-rotation-window");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = server_actor_with_tls_cert_path(&cert_path);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let mut first_stream = TcpStream::connect(address)
        .await
        .expect("first client should connect");
    first_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("first tls request line should write");
    first_stream
        .write_all(b"\n")
        .await
        .expect("first tls request newline should write");
    first_stream
        .flush()
        .await
        .expect("first tls request should flush");

    let first_tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut first_stream),
    )
    .await
    .expect("first tls response should arrive before timeout")
    .expect("first tls response read should succeed")
    .expect("first tls response line should be present");
    let first_tls_response = decode_message_line(&first_tls_response_line)
        .expect("first tls response line should decode");
    let ProtocolMessage::Tls(payload) = first_tls_response else {
        panic!("server should respond with TLS payload for first client");
    };
    assert_eq!(payload.tls.start_tls, "true");

    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    overwrite_file_until_modified_time_changes(
        &cert_path.join("cert.pem"),
        "rotated-after-starttls-true",
    );

    let connector = tls_client_connector_for_test_fixture();
    let server_name = ServerName::try_from("localhost").expect("server name should parse");
    let mut first_tls_stream = timeout(
        Duration::from_secs(2),
        connector.connect(server_name, first_stream),
    )
    .await
    .expect("first tls handshake should complete before timeout")
    .expect("first tls handshake should succeed with cached context");

    first_tls_stream
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write over first tls stream");
    first_tls_stream
        .write_all(b"\n")
        .await
        .expect("hello newline should write over first tls stream");
    first_tls_stream
        .flush()
        .await
        .expect("hello line should flush over first tls stream");

    let mut saw_hello = false;
    for _ in 0..4 {
        let maybe_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut first_tls_stream),
        )
        .await
        .expect("first post-upgrade response should arrive before timeout")
        .expect("first post-upgrade response read should succeed");
        let Some(line) = maybe_line else {
            break;
        };
        if line.is_empty() {
            continue;
        }
        let message = decode_message_line(&line).expect("first post-upgrade line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "first client should complete protocol flow over cached-context upgraded tls transport"
    );

    let mut second_stream = TcpStream::connect(address)
        .await
        .expect("second client should connect");
    second_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("second tls request line should write");
    second_stream
        .write_all(b"\n")
        .await
        .expect("second tls request newline should write");
    second_stream
        .flush()
        .await
        .expect("second tls request should flush");

    let second_tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut second_stream),
    )
    .await
    .expect("second tls response should arrive before timeout")
    .expect("second tls response read should succeed")
    .expect("second tls response line should be present");
    let second_tls_response = decode_message_line(&second_tls_response_line)
        .expect("second tls response line should decode");
    let ProtocolMessage::Tls(payload) = second_tls_response else {
        panic!("server should respond with TLS payload for second client");
    };
    assert_eq!(
        payload.tls.start_tls, "false",
        "second client should be denied TLS after cert rotation makes bundle invalid"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[tokio::test]
async fn server_network_loop_tls_upgrade_recovers_after_invalid_rotation_bundle_is_restored() {
    let cert_path = temporary_directory_path("tls-network-upgrade-rotation-recovery");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = server_actor_with_tls_cert_path(&cert_path);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        runtime,
        None,
        shutdown_rx,
    ));

    let mut first_stream = TcpStream::connect(address)
        .await
        .expect("first client should connect");
    first_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("first tls request line should write");
    first_stream
        .write_all(b"\n")
        .await
        .expect("first tls request newline should write");
    first_stream
        .flush()
        .await
        .expect("first tls request should flush");
    let first_tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut first_stream),
    )
    .await
    .expect("first tls response should arrive before timeout")
    .expect("first tls response read should succeed")
    .expect("first tls response line should be present");
    let first_tls_response = decode_message_line(&first_tls_response_line)
        .expect("first tls response line should decode");
    let ProtocolMessage::Tls(first_tls_payload) = first_tls_response else {
        panic!("server should respond with TLS payload for first client");
    };
    assert_eq!(first_tls_payload.tls.start_tls, "true");

    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    overwrite_file_until_modified_time_changes(
        &cert_path.join("cert.pem"),
        "rotated-invalid-before-second-client",
    );

    let mut second_stream = TcpStream::connect(address)
        .await
        .expect("second client should connect");
    second_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("second tls request line should write");
    second_stream
        .write_all(b"\n")
        .await
        .expect("second tls request newline should write");
    second_stream
        .flush()
        .await
        .expect("second tls request should flush");
    let second_tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut second_stream),
    )
    .await
    .expect("second tls response should arrive before timeout")
    .expect("second tls response read should succeed")
    .expect("second tls response line should be present");
    let second_tls_response = decode_message_line(&second_tls_response_line)
        .expect("second tls response line should decode");
    let ProtocolMessage::Tls(second_tls_payload) = second_tls_response else {
        panic!("server should respond with TLS payload for second client");
    };
    assert_eq!(
        second_tls_payload.tls.start_tls, "false",
        "second client should be denied TLS after invalid cert rotation"
    );

    write_valid_tls_bundle(&cert_path);
    rewrite_file_until_modified_time_changes(&cert_path.join("cert.pem"), TEST_TLS_CERT_PEM);

    let mut third_stream = TcpStream::connect(address)
        .await
        .expect("third client should connect");
    third_stream
        .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
        .await
        .expect("third tls request line should write");
    third_stream
        .write_all(b"\n")
        .await
        .expect("third tls request newline should write");
    third_stream
        .flush()
        .await
        .expect("third tls request should flush");
    let third_tls_response_line = timeout(
        Duration::from_secs(2),
        super::read_network_line_from_stream(&mut third_stream),
    )
    .await
    .expect("third tls response should arrive before timeout")
    .expect("third tls response read should succeed")
    .expect("third tls response line should be present");
    let third_tls_response = decode_message_line(&third_tls_response_line)
        .expect("third tls response line should decode");
    let ProtocolMessage::Tls(third_tls_payload) = third_tls_response else {
        panic!("server should respond with TLS payload for third client");
    };
    assert_eq!(
        third_tls_payload.tls.start_tls, "true",
        "third client should be allowed TLS after valid bundle restoration"
    );

    let connector = tls_client_connector_for_test_fixture();
    let server_name = ServerName::try_from("localhost").expect("server name should parse");
    let mut third_tls_stream = timeout(
        Duration::from_secs(2),
        connector.connect(server_name, third_stream),
    )
    .await
    .expect("third tls handshake should complete before timeout")
    .expect("third tls handshake should succeed after bundle restoration");

    third_tls_stream
        .write_all(br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#)
        .await
        .expect("hello line should write over third tls stream");
    third_tls_stream
        .write_all(b"\n")
        .await
        .expect("hello newline should write over third tls stream");
    third_tls_stream
        .flush()
        .await
        .expect("hello line should flush over third tls stream");

    let mut saw_hello = false;
    for _ in 0..4 {
        let maybe_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut third_tls_stream),
        )
        .await
        .expect("third post-upgrade response should arrive before timeout")
        .expect("third post-upgrade response read should succeed");
        let Some(line) = maybe_line else {
            break;
        };
        if line.is_empty() {
            continue;
        }
        let message = decode_message_line(&line).expect("third post-upgrade line should decode");
        if matches!(message, ProtocolMessage::Hello(_)) {
            saw_hello = true;
            break;
        }
    }
    assert!(
        saw_hello,
        "third client should complete protocol flow over restored TLS context"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown signal should send successfully");
    server_task
        .await
        .expect("server task should join cleanly")
        .expect("server loop should exit without error");

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}
