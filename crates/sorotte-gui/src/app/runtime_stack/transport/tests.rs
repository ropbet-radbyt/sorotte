use super::tcp::MAX_INBOUND_PROTOCOL_LINE_BYTES;
use super::*;

use std::{
    io::{self, BufRead, BufReader, Write},
    net::{Shutdown, TcpListener},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use rustls::{
    ClientConfig, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    pki_types::CertificateDer,
};
use sorotte_client_app::app_boundary::state::TlsPolicy;
use sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES;

const TEST_TLS_CERT_PEM: &str = include_str!("../../../../../../fixtures/tls/test_cert.pem");
const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../../../../fixtures/tls/test_chain.pem");
const TEST_TLS_PRIVATE_KEY_PEM: &str =
    include_str!("../../../../../../fixtures/tls/test_privkey.pem");

fn test_tls_client_config() -> Arc<ClientConfig> {
    GuiTcpSessionTransportDriver::ensure_rustls_crypto_provider();
    let mut cert_reader = io::BufReader::new(TEST_TLS_CERT_PEM.as_bytes());
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .expect("test TLS certificate fixture should parse");
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .expect("test TLS certificate should be trusted by the client");
    }
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn test_tls_server_config() -> Arc<ServerConfig> {
    GuiTcpSessionTransportDriver::ensure_rustls_crypto_provider();
    let mut cert_reader = io::BufReader::new(TEST_TLS_CERT_PEM.as_bytes());
    let mut certificate_chain = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<CertificateDer<'static>>, _>>()
        .expect("test TLS certificate fixture should parse");
    let mut chain_reader = io::BufReader::new(TEST_TLS_CHAIN_PEM.as_bytes());
    certificate_chain.extend(
        rustls_pemfile::certs(&mut chain_reader)
            .collect::<Result<Vec<CertificateDer<'static>>, _>>()
            .expect("test TLS chain fixture should parse"),
    );
    let mut key_reader = io::BufReader::new(TEST_TLS_PRIVATE_KEY_PEM.as_bytes());
    let private_key = rustls_pemfile::private_key(&mut key_reader)
        .expect("test TLS private key fixture should parse")
        .expect("test TLS private key fixture should contain a key");
    Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certificate_chain, private_key)
            .expect("test TLS server config should build"),
    )
}

fn hello_line() -> String {
    r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned()
}

fn credential_hello_line() -> String {
    r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","password":"credential-secret","features":{"chat":true}}}"#
        .to_owned()
}

fn valid_chat_line_with_len(line_len: usize) -> String {
    let prefix = r#"{"Chat":""#;
    let suffix = r#""}"#;
    assert!(line_len >= prefix.len() + suffix.len());
    let message_len = line_len - prefix.len() - suffix.len();
    let line = format!("{prefix}{}{suffix}", "a".repeat(message_len));
    assert_eq!(line.len(), line_len);
    line
}

fn oversized_media_match_list_snapshot_line() -> String {
    let signature = "A".repeat(32 * 1024);
    let line = format!(
        r#"{{"List":{{"room1":{{"alice":{{"file":{{"name":"episode1.mkv","mediaMatch":{{"schema":"sorotte.mediaMatch.v3","profiles":[{{"profile":"audio-constellation-v3","algorithmVersion":3,"durationMs":100000,"audio":{{"algorithm":"sorotte-audio-constellation-v3-sampled-fast","timeBaseMs":1,"anchors":"{signature}"}}}}]}}}}}},"bob":{{"file":{{"name":"episode2.mkv","mediaMatch":{{"schema":"sorotte.mediaMatch.v3","profiles":[{{"profile":"audio-constellation-v3","algorithmVersion":3,"durationMs":100000,"audio":{{"algorithm":"sorotte-audio-constellation-v3-sampled-fast","timeBaseMs":1,"anchors":"{signature}"}}}}]}}}}}}}}}}}}"#
    );
    assert!(line.len() > DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    assert!(line.len() <= MAX_INBOUND_PROTOCOL_LINE_BYTES);
    line
}

fn write_plaintext_tls_fallback(stream: &mut std::net::TcpStream) {
    stream
        .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
        .expect("test server should write the TLS decline");
    stream
        .write_all(b"\n")
        .expect("test server should terminate the TLS decline");
}

fn read_line_until_timeout(
    reader: &mut BufReader<std::net::TcpStream>,
    timeout: Duration,
    context: &str,
) -> Option<String> {
    reader
        .get_mut()
        .set_read_timeout(Some(timeout))
        .unwrap_or_else(|error| panic!("{context} should configure a read timeout: {error}"));
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            None
        }
        Err(error) => panic!("{context} should read a protocol line or time out: {error}"),
    }
}

fn connect_gui_transport_driver(port: u16) -> GuiTcpSessionTransportDriver {
    GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{port}"),
        TlsPolicy::PreferTls,
    )
    .expect("transport test client driver should connect")
    .with_inbound_idle_timeout(Duration::from_secs(2))
}

#[test]
fn gui_tcp_rejects_inbound_line_over_max_bytes() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("oversized-line transport test server should bind");
    let address = listener
        .local_addr()
        .expect("oversized-line transport test server should expose its address");
    let (first_line_tx, first_line_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("oversized-line transport test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("oversized-line transport test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("oversized-line transport test server should read the TLS request");
        first_line_tx
            .send(first_line)
            .expect("oversized-line transport test server should report the TLS request");
        write_plaintext_tls_fallback(&mut stream);
        stream
            .write_all(&vec![b'a'; MAX_INBOUND_PROTOCOL_LINE_BYTES + 1])
            .expect("oversized-line transport test server should write the oversized line");
        thread::sleep(Duration::from_millis(250));
    });

    let mut driver = connect_gui_transport_driver(address.port());
    let transport = GuiQueuedSessionTransportHandle::default();

    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        match driver.pump(&transport) {
            Ok(()) => {}
            Err(error) => break error,
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the oversized inbound line to fail the TCP transport",
        );
        thread::sleep(Duration::from_millis(10));
    };

    let first_line = first_line_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("oversized-line transport test server should receive the TLS request");
    assert!(first_line.contains(r#""TLS""#));
    assert!(
        error.contains("inbound protocol line exceeded"),
        "oversized inbound line should surface a clear transport error"
    );
    assert!(transport.drain_inbound_protocol_lines().is_empty());

    server_thread
        .join()
        .expect("oversized-line transport test server thread should join");
}

#[test]
fn gui_tcp_accepts_line_at_or_under_max_bytes() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("max-line transport test server should bind");
    let address = listener
        .local_addr()
        .expect("max-line transport test server should expose its address");
    let expected_line = valid_chat_line_with_len(MAX_INBOUND_PROTOCOL_LINE_BYTES);
    let server_line = expected_line.clone();
    let (line_observed_tx, line_observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("max-line transport test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("max-line transport test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("max-line transport test server should read the TLS request");
        write_plaintext_tls_fallback(&mut stream);
        stream
            .write_all(server_line.as_bytes())
            .expect("max-line transport test server should write the max-sized line");
        stream
            .write_all(b"\n")
            .expect("max-line transport test server should terminate the max-sized line");
        let _ = line_observed_rx.recv_timeout(Duration::from_secs(5));
    });

    let mut driver = connect_gui_transport_driver(address.port());
    let transport = GuiQueuedSessionTransportHandle::default();

    let deadline = Instant::now() + Duration::from_secs(2);
    let inbound_lines = loop {
        driver
            .pump(&transport)
            .expect("max-sized inbound line should not fail the TCP transport");
        let inbound_lines = transport.drain_inbound_protocol_lines();
        if !inbound_lines.is_empty() {
            break inbound_lines;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the max-sized inbound line",
        );
        thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(inbound_lines.len(), 1);
    assert_eq!(inbound_lines[0].line, expected_line);
    assert!(inbound_lines[0].received_at_seconds > 0.0);
    let _ = line_observed_tx.send(());

    server_thread
        .join()
        .expect("max-line transport test server thread should join");
}

#[test]
fn gui_tcp_accepts_media_match_room_snapshot_above_default_protocol_limit() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("above-default-line transport test server should bind");
    let address = listener
        .local_addr()
        .expect("above-default-line transport test server should expose its address");
    let expected_line = oversized_media_match_list_snapshot_line();
    let server_line = expected_line.clone();
    let (line_observed_tx, line_observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("above-default-line transport test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("above-default-line transport test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("above-default-line transport test server should read the TLS request");
        write_plaintext_tls_fallback(&mut stream);
        stream
            .write_all(server_line.as_bytes())
            .expect("above-default-line transport test server should write the large line");
        stream
            .write_all(b"\n")
            .expect("above-default-line transport test server should terminate the large line");
        let _ = line_observed_rx.recv_timeout(Duration::from_secs(5));
    });

    let mut driver = connect_gui_transport_driver(address.port());
    let transport = GuiQueuedSessionTransportHandle::default();

    let deadline = Instant::now() + Duration::from_secs(2);
    let inbound_lines = loop {
        driver
            .pump(&transport)
            .expect("above-default inbound line should not fail the TCP transport");
        let inbound_lines = transport.drain_inbound_protocol_lines();
        if !inbound_lines.is_empty() {
            break inbound_lines;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the above-default inbound line",
        );
        thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(inbound_lines.len(), 1);
    assert_eq!(inbound_lines[0].line, expected_line);
    assert!(inbound_lines[0].received_at_seconds > 0.0);
    let _ = line_observed_tx.send(());

    server_thread
        .join()
        .expect("above-default-line transport test server thread should join");
}

#[test]
fn tcp_session_transport_driver_rejects_invalid_inbound_protocol_lines() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("invalid-line transport test server should bind");
    let address = listener
        .local_addr()
        .expect("invalid-line transport test server should expose its address");
    let (first_line_tx, first_line_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("invalid-line transport test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("invalid-line transport test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("invalid-line transport test server should read the TLS request");
        first_line_tx
            .send(first_line)
            .expect("invalid-line transport test server should report the TLS request");
        stream
            .write_all(br#"{"status":"connected"}"#)
            .expect("invalid-line transport test server should write the invalid line");
        stream
            .write_all(b"\n")
            .expect("invalid-line transport test server should terminate the invalid line");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("invalid-line transport test client driver should connect")
    .with_inbound_idle_timeout(Duration::from_secs(2));
    let transport = GuiQueuedSessionTransportHandle::default();

    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        match driver.pump(&transport) {
            Ok(()) => {}
            Err(error) => break error,
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the invalid inbound line to fail the TCP transport",
        );
        thread::sleep(Duration::from_millis(10));
    };

    let first_line = first_line_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("invalid-line transport test server should receive the TLS request");
    assert!(first_line.contains(r#""TLS""#));
    assert!(first_line.contains(r#""startTLS":"send""#));
    assert!(
        error.contains("Session transport TCP received an invalid protocol line"),
        "invalid inbound protocol lines should surface a fatal transport error"
    );
    assert!(transport.drain_inbound_protocol_lines().is_empty());

    server_thread
        .join()
        .expect("invalid-line transport test server thread should join");
}

#[test]
fn tcp_session_transport_driver_falls_back_to_plaintext_when_server_declines_start_tls() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("plaintext TLS-fallback test server should bind");
    let address = listener
        .local_addr()
        .expect("plaintext TLS-fallback test server should expose its address");
    let (first_line_tx, first_line_rx) = mpsc::channel();
    let (hello_tx, hello_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("plaintext TLS-fallback test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("plaintext TLS-fallback test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("plaintext TLS-fallback test server should read the TLS request");
        first_line_tx
            .send(first_line)
            .expect("plaintext TLS-fallback test server should report the TLS request");
        stream
            .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
            .expect("plaintext TLS-fallback test server should write the TLS decline");
        stream
            .write_all(b"\n")
            .expect("plaintext TLS-fallback test server should terminate the TLS decline");
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("plaintext TLS-fallback test server should read the plaintext hello");
        hello_tx
            .send(hello)
            .expect("plaintext TLS-fallback test server should report the plaintext hello");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("plaintext TLS-fallback client driver should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([credential_hello_line()]);

    let deadline = Instant::now() + Duration::from_secs(2);
    let hello = loop {
        if let Ok(hello) = hello_rx.try_recv() {
            break hello;
        }
        if let Err(error) = driver.pump(&transport) {
            if let Ok(hello) = hello_rx.try_recv() {
                break hello;
            }
            panic!("session transport driver should pump: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the plaintext hello after TLS fallback"
        );
        thread::sleep(Duration::from_millis(10));
    };

    let first_line = first_line_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("plaintext TLS-fallback server should receive the TLS request");
    assert!(first_line.contains(r#""TLS""#));
    assert!(first_line.contains(r#""startTLS":"send""#));
    assert!(hello.contains(r#""Hello""#));
    assert!(hello.contains(r#""alice""#));
    let warnings = transport.drain_transport_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("continuing without encryption"));
    assert!(warnings[0].contains("tlsPolicy = RequireTls"));

    server_thread
        .join()
        .expect("plaintext TLS-fallback server thread should join");
}

#[test]
fn tcp_session_transport_require_tls_rejects_refusal_without_sending_hello() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("required TLS refusal test server should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    let (observed_tx, observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read TLS request");
        write_plaintext_tls_fallback(&mut stream);
        let application_line = read_line_until_timeout(
            &mut reader,
            Duration::from_millis(250),
            "required TLS refusal test server",
        );
        observed_tx
            .send((tls_request, application_line))
            .expect("server should report observations");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::RequireTls,
    )
    .expect("required TLS client should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([credential_hello_line()]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(
            Instant::now() < deadline,
            "required TLS refusal should fail"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("refused required TLS"));
    let (tls_request, application_line) = observed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("server should report observations");
    assert!(tls_request.contains(r#""startTLS":"send""#));
    assert_eq!(
        application_line, None,
        "credentials/Hello must not cross the downgraded socket"
    );
    server_thread.join().expect("server thread should join");
}

#[test]
fn tcp_session_transport_prefer_tls_preserves_hello_bundled_after_refusal() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("bundled TLS-refusal test server should bind");
    let address = listener
        .local_addr()
        .expect("bundled TLS-refusal test server should expose address");
    let (stop_tx, stop_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read TLS request");
        assert!(tls_request.contains(r#""startTLS":"send""#));
        stream
            .write_all(
                br#"{"TLS":{"startTLS":"false"},"Hello":{"username":"server","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("bundled refusal and Hello should write");
        stream
            .write_all(b"\n")
            .expect("bundled refusal and Hello should terminate");
        stream.flush().expect("bundled response should flush");
        let _ = stop_rx.recv_timeout(Duration::from_secs(1));
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("PreferTls client should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut inbound = Vec::new();
    while Instant::now() < deadline && inbound.is_empty() {
        driver
            .pump(&transport)
            .expect("bundled refusal must keep the plaintext transport usable");
        inbound = transport.drain_inbound_protocol_lines();
        thread::sleep(Duration::from_millis(5));
    }
    let _ = stop_tx.send(());
    server_thread.join().expect("server thread should join");

    assert_eq!(
        inbound.len(),
        1,
        "the valid bundled Hello must be re-injected into normal inbound handling"
    );
    let message = sorotte_protocol::decode_message_line(&inbound[0].line)
        .expect("the re-injected application line should decode");
    assert!(matches!(
        message,
        sorotte_protocol::ProtocolMessage::Hello(_)
    ));
}

#[test]
fn tcp_session_transport_require_tls_rejects_substituted_message() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("required TLS substitution test server should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    let (observed_tx, observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read TLS request");
        stream
            .write_all(hello_line().as_bytes())
            .expect("substituted message should write");
        stream.write_all(b"\n").expect("message should terminate");
        let application_line = read_line_until_timeout(
            &mut reader,
            Duration::from_millis(250),
            "required TLS substitution test server",
        );
        observed_tx
            .send((tls_request, application_line))
            .expect("server should report observations");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::RequireTls,
    )
    .expect("required TLS client should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([credential_hello_line()]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(Instant::now() < deadline, "TLS substitution should fail");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("unexpected Hello message"));
    assert!(transport.drain_inbound_protocol_lines().is_empty());
    let (tls_request, application_line) = observed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("server should report observations");
    assert!(tls_request.contains(r#""startTLS":"send""#));
    assert_eq!(
        application_line, None,
        "credentials/Hello must not follow a substituted STARTTLS response"
    );
    server_thread.join().expect("server thread should join");
}

#[test]
fn tcp_session_transport_require_tls_rejects_truncated_response_without_sending_credentials() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("required TLS truncation test server should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    let (observed_tx, observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read TLS request");
        stream
            .write_all(br#"{"TLS":{"startTLS":"tru"#)
            .expect("truncated response should write");
        stream
            .shutdown(Shutdown::Write)
            .expect("server should half-close its truncated response");
        let application_line = read_line_until_timeout(
            &mut reader,
            Duration::from_millis(250),
            "required TLS truncation test server",
        );
        observed_tx
            .send((tls_request, application_line))
            .expect("server should report observations");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::RequireTls,
    )
    .expect("required TLS client should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([credential_hello_line()]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(Instant::now() < deadline, "TLS truncation should fail");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("incomplete inbound line"));
    let (tls_request, application_line) = observed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("server should report observations");
    assert!(tls_request.contains(r#""startTLS":"send""#));
    assert_eq!(
        application_line, None,
        "credentials/Hello must not follow a truncated STARTTLS response"
    );
    server_thread.join().expect("server thread should join");
}

#[test]
fn tcp_session_transport_require_tls_rejects_invalid_certificate_without_sending_credentials() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("invalid-certificate TLS test server should bind");
    let address = listener
        .local_addr()
        .expect("invalid-certificate listener should expose address");
    let server_config = test_tls_server_config();
    let (observed_tx, observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read TLS request");
        stream
            .write_all(br#"{"TLS":{"startTLS":"true"}}"#)
            .expect("TLS acceptance should write");
        stream
            .write_all(b"\n")
            .expect("TLS acceptance should terminate");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("TLS server should configure a read timeout");

        let mut tls_stream = StreamOwned::new(
            ServerConnection::new(server_config).expect("TLS server connection should build"),
            stream,
        );
        let mut hello = String::new();
        let read_result = BufReader::new(&mut tls_stream)
            .read_line(&mut hello)
            .map_err(|error| error.to_string());
        observed_tx
            .send((tls_request, hello, read_result))
            .expect("TLS server should report observations");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::RequireTls,
    )
    .expect("required TLS client should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([credential_hello_line()]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(
            Instant::now() < deadline,
            "invalid certificate should fail the TLS handshake"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let error_lower = error.to_ascii_lowercase();
    assert!(
        error_lower.contains("certificate") || error_lower.contains("unknownissuer"),
        "invalid-certificate failure should identify certificate verification: {error}"
    );
    let (tls_request, hello, _server_read_result) = observed_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("TLS server should report observations");
    assert!(tls_request.contains(r#""startTLS":"send""#));
    assert!(
        hello.is_empty(),
        "credentials/Hello must not be released before certificate verification: {hello:?}"
    );
    assert!(!hello.contains("credential-secret"));
    server_thread.join().expect("server thread should join");
}

#[test]
fn tcp_session_transport_enforces_starttls_and_initial_hello_deadlines() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("STARTTLS deadline test server should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    let server_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("server should accept");
        thread::sleep(Duration::from_millis(150));
        drop(stream);
    });
    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::RequireTls,
    )
    .expect("deadline client should connect")
    .with_connection_phase_timeouts(
        Duration::from_millis(25),
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let transport = GuiQueuedSessionTransportHandle::default();
    let deadline = Instant::now() + Duration::from_secs(1);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(
            Instant::now() < deadline,
            "STARTTLS response should time out"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert!(error.contains("STARTTLS response timed out"));
    server_thread.join().expect("server thread should join");

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("initial Hello deadline test server should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("TLS request should read");
        write_plaintext_tls_fallback(&mut stream);
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("client Hello should read");
        thread::sleep(Duration::from_millis(150));
    });
    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("initial Hello deadline client should connect")
    .with_connection_phase_timeouts(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_millis(25),
    );
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([hello_line()]);
    let deadline = Instant::now() + Duration::from_secs(1);
    let error = loop {
        if let Err(error) = driver.pump(&transport) {
            break error;
        }
        assert!(Instant::now() < deadline, "initial Hello should time out");
        thread::sleep(Duration::from_millis(5));
    };
    assert!(error.contains("initial Hello timed out"));
    server_thread.join().expect("server thread should join");
}

#[test]
fn tcp_session_transport_driver_upgrades_to_tls_before_sending_hello() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("TLS upgrade test server should bind");
    let address = listener
        .local_addr()
        .expect("TLS upgrade test server should expose its address");
    let server_config = test_tls_server_config();
    let (first_line_tx, first_line_rx) = mpsc::channel();
    let (hello_tx, hello_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("TLS upgrade test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("TLS upgrade test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("TLS upgrade test server should read the TLS request");
        first_line_tx
            .send(first_line)
            .expect("TLS upgrade test server should report the TLS request");
        stream
            .write_all(br#"{"TLS":{"startTLS":"true"}}"#)
            .expect("TLS upgrade test server should accept protocol TLS");
        stream
            .write_all(b"\n")
            .expect("TLS upgrade test server should terminate the TLS response");

        let mut tls_stream = StreamOwned::new(
            ServerConnection::new(server_config)
                .expect("TLS upgrade test server connection should build"),
            stream,
        );
        let mut hello_reader = BufReader::new(&mut tls_stream);
        let mut hello = String::new();
        hello_reader
            .read_line(&mut hello)
            .expect("TLS upgrade test server should read the hello over TLS");
        hello_tx
            .send(hello)
            .expect("TLS upgrade test server should report the TLS hello");
    });

    let mut driver = GuiTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("TLS upgrade client driver should connect")
    .with_tls_client_config(test_tls_client_config());
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([hello_line()]);

    let deadline = Instant::now() + Duration::from_secs(2);
    let hello = loop {
        if let Ok(hello) = hello_rx.try_recv() {
            break hello;
        }
        if let Err(error) = driver.pump(&transport) {
            if let Ok(hello) = hello_rx.try_recv() {
                break hello;
            }
            panic!("session transport driver should pump: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the hello after the TLS upgrade"
        );
        thread::sleep(Duration::from_millis(10));
    };

    let first_line = first_line_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("TLS upgrade test server should receive the TLS request");
    assert!(first_line.contains(r#""TLS""#));
    assert!(first_line.contains(r#""startTLS":"send""#));
    assert!(hello.contains(r#""Hello""#));
    assert!(hello.contains(r#""alice""#));

    server_thread
        .join()
        .expect("TLS upgrade test server thread should join");
}

#[test]
fn threaded_tcp_session_transport_does_not_send_liveness_before_enabled() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("threaded TCP liveness gating test server should bind");
    let address = listener
        .local_addr()
        .expect("threaded TCP liveness gating test server should expose its address");
    let (observed_tx, observed_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("threaded TCP liveness gating test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("threaded TCP liveness gating test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("threaded TCP liveness gating test server should read the TLS request");
        write_plaintext_tls_fallback(&mut stream);
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("threaded TCP liveness gating test server should read the hello");
        let extra = read_line_until_timeout(
            &mut reader,
            Duration::from_millis(1300),
            "threaded TCP liveness gating test server",
        );
        observed_tx
            .send((tls_request, hello, extra))
            .expect("threaded TCP liveness gating test server should report observed lines");
    });

    let mut driver = GuiThreadedTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("threaded TCP liveness gating client driver should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([hello_line()]);
    driver
        .pump(&transport)
        .expect("threaded TCP liveness gating driver should start");

    let (tls_request, hello, extra) = observed_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("threaded TCP liveness gating server should report observed lines");
    assert!(tls_request.contains(r#""TLS""#));
    assert!(tls_request.contains(r#""startTLS":"send""#));
    assert!(hello.contains(r#""Hello""#));
    assert!(
        extra.is_none(),
        "threaded TCP transport must not send liveness State before it is enabled"
    );

    drop(driver);
    server_thread
        .join()
        .expect("threaded TCP liveness gating test server thread should join");
}

#[test]
fn threaded_tcp_session_transport_sends_liveness_without_gui_pump() {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("threaded TCP liveness test server should bind");
    let address = listener
        .local_addr()
        .expect("threaded TCP liveness test server should expose its address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (state_tx, state_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("threaded TCP liveness test server should accept a client");
        let reader_stream = stream
            .try_clone()
            .expect("threaded TCP liveness test server should clone the socket");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("threaded TCP liveness test server should read the TLS request");
        write_plaintext_tls_fallback(&mut stream);
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("threaded TCP liveness test server should read the hello");
        hello_tx
            .send((tls_request, hello))
            .expect("threaded TCP liveness test server should report the hello");
        let state = read_line_until_timeout(
            &mut reader,
            Duration::from_secs(3),
            "threaded TCP liveness test server",
        );
        state_tx
            .send(state)
            .expect("threaded TCP liveness test server should report the liveness line");
    });

    let mut driver = GuiThreadedTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
        &format!("localhost:{}", address.port()),
        TlsPolicy::PreferTls,
    )
    .expect("threaded TCP liveness client driver should connect");
    let transport = GuiQueuedSessionTransportHandle::default();
    transport.push_outbound_protocol_lines([hello_line()]);
    driver
        .pump(&transport)
        .expect("threaded TCP liveness driver should start");

    let (tls_request, hello) = hello_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("threaded TCP liveness server should receive the startup lines");
    assert!(tls_request.contains(r#""TLS""#));
    assert!(hello.contains(r#""Hello""#));

    driver.set_protocol_liveness_enabled(true);
    let state = state_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("threaded TCP liveness server should report a liveness line")
        .expect("threaded TCP transport should send a liveness line without another GUI pump");
    assert!(state.contains(r#""State""#));
    assert!(state.contains(r#""ping""#));

    drop(driver);
    server_thread
        .join()
        .expect("threaded TCP liveness test server thread should join");
}
