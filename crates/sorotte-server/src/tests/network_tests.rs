use super::*;

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

#[tokio::test]
async fn server_network_closes_or_drops_slow_client_when_outbound_queue_full() {
    let client_event_senders = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let (event_tx, _event_rx) = mpsc::channel(crate::CLIENT_OUTBOUND_QUEUE_CAPACITY);
    for index in 0..crate::CLIENT_OUTBOUND_QUEUE_CAPACITY {
        event_tx
            .try_send(crate::ClientOutboundEvent::Line(format!("queued-{index}")))
            .expect("test queue should accept initial fill");
    }
    {
        let mut senders = client_event_senders.lock().await;
        senders.insert("client-1".to_owned(), event_tx);
    }

    crate::network::dispatch_outbound_lines_to_clients(
        &client_event_senders,
        vec![DirectedOutboundLine {
            client_id: "client-1".to_owned(),
            line: r#"{"Chat":"overflow"}"#.to_owned(),
        }],
    )
    .await;

    assert!(
        !client_event_senders.lock().await.contains_key("client-1"),
        "a full outbound queue should close/drop the slow client sender"
    );
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
