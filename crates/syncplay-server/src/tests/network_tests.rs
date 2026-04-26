use super::*;

#[tokio::test]
async fn server_network_loop_routes_hello_response_to_connected_client() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
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
async fn server_network_loop_sends_error_for_invalid_utf8_line() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local address");
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
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
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
    {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
    }
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
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
    {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
    }
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
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
    {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
    }

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
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
    {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
    }

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
    let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
    {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
    }

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
