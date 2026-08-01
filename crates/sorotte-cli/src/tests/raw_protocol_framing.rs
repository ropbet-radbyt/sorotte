use super::*;
use crate::protocol_io::{InboundProtocolReadObservation, observe_inbound_protocol_reads};
use std::net::SocketAddr;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

const RAW_SOCKET_TIMEOUT: Duration = Duration::from_secs(3);
const WIRE_USERNAME: &str = "wire-user";
const WIRE_ROOM: &str = "wire-room";
const SERVER_HELLO: &[u8] = br#"{"Hello":{"username":"wire-user","room":{"name":"wire-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#;
const SERVER_READY: &[u8] =
    br#"{"Set":{"ready":{"isReady":true,"username":"wire-user","manuallyInitiated":false}}}"#;

enum RawServerWrites {
    Chunks(Vec<Vec<u8>>),
    GatedFragment {
        prefix: Vec<u8>,
        remaining: Vec<u8>,
        release_remaining: oneshot::Receiver<()>,
    },
}

fn crlf_framed(payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(payload.len() + 2);
    framed.extend_from_slice(payload);
    framed.extend_from_slice(b"\r\n");
    framed
}

fn coalesced_frames(frames: &[&[u8]]) -> Vec<u8> {
    let capacity = frames.iter().map(|frame| frame.len() + 2).sum();
    let mut coalesced = Vec::with_capacity(capacity);
    for frame in frames {
        coalesced.extend_from_slice(frame);
        coalesced.extend_from_slice(b"\r\n");
    }
    coalesced
}

async fn read_client_hello(reader: OwnedReadHalf) -> BufReader<OwnedReadHalf> {
    let mut reader = BufReader::new(reader);
    let mut hello = String::new();
    let bytes_read = tokio::time::timeout(RAW_SOCKET_TIMEOUT, reader.read_line(&mut hello))
        .await
        .expect("raw server should receive the client Hello before the boundary timeout")
        .expect("raw server should read the client Hello");
    assert_ne!(bytes_read, 0, "client should send a Hello frame");
    assert!(
        hello.contains("\"Hello\""),
        "first plaintext client frame should be Hello, got: {hello}"
    );
    reader
}

async fn spawn_raw_loopback_server(
    writes: RawServerWrites,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test-owned loopback listener should bind");
    let address = listener
        .local_addr()
        .expect("test-owned loopback listener should expose an address");
    assert!(address.ip().is_loopback());

    let task = tokio::spawn(async move {
        let (socket, peer_address) = tokio::time::timeout(RAW_SOCKET_TIMEOUT, listener.accept())
            .await
            .expect("raw client should connect before the boundary timeout")
            .expect("raw server should accept the client");
        assert!(peer_address.ip().is_loopback());
        socket
            .set_nodelay(true)
            .expect("raw server should enable TCP_NODELAY");
        let (reader, mut writer) = socket.into_split();
        let mut reader = read_client_hello(reader).await;

        match writes {
            RawServerWrites::Chunks(chunks) => {
                for chunk in chunks {
                    writer
                        .write_all(&chunk)
                        .await
                        .expect("raw server chunk write should succeed");
                }
            }
            RawServerWrites::GatedFragment {
                prefix,
                remaining,
                release_remaining,
            } => {
                writer
                    .write_all(&prefix)
                    .await
                    .expect("fragment prefix write should succeed");
                writer.flush().await.expect("fragment prefix should flush");
                tokio::time::timeout(RAW_SOCKET_TIMEOUT, release_remaining)
                    .await
                    .expect("client should release the remaining fragment bytes")
                    .expect("client-side fragment release should remain present");
                for byte in remaining {
                    writer
                        .write_all(&[byte])
                        .await
                        .expect("one-byte remainder write should succeed");
                    tokio::task::yield_now().await;
                }
            }
        }
        writer.flush().await.expect("raw server should flush");
        writer
            .shutdown()
            .await
            .expect("raw server should half-close its write side");
        let mut trailing_client_bytes = Vec::new();
        tokio::time::timeout(
            RAW_SOCKET_TIMEOUT,
            reader.read_to_end(&mut trailing_client_bytes),
        )
        .await
        .expect("raw client should close after observing the server half-close")
        .expect("raw server should drain the client receive half");
    });

    (address, task)
}

async fn join_raw_server(task: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(RAW_SOCKET_TIMEOUT, task)
        .await
        .expect("raw server task should finish before the boundary timeout")
        .expect("raw server task should not panic");
}

async fn run_raw_connected_session(
    writes: RawServerWrites,
) -> (
    anyhow::Result<ConnectedSessionExit>,
    ClientApplication<MpvAdapter>,
) {
    let (address, server_task) = spawn_raw_loopback_server(writes).await;
    let mut config = test_client_loop_config_with_addr(address);
    config.max_connected_runtime_seconds = 2.0;
    let mut runtime = create_client_runtime(&config);
    let stream = TcpStream::connect(address)
        .await
        .expect("raw client should connect to its test-owned loopback listener");
    assert!(
        stream
            .peer_addr()
            .expect("raw client peer address should be available")
            .ip()
            .is_loopback()
    );
    stream
        .set_nodelay(true)
        .expect("raw client should enable TCP_NODELAY");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let result = tokio::time::timeout(
        RAW_SOCKET_TIMEOUT,
        run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        ),
    )
    .await
    .expect("raw connected session should finish before the boundary timeout");
    join_raw_server(server_task).await;
    (result, runtime)
}

fn assert_server_hello_applied(runtime: &ClientApplication<MpvAdapter>) {
    assert_eq!(runtime.session().username(), Some(WIRE_USERNAME));
    assert_eq!(runtime.session().room(), Some(WIRE_ROOM));
    assert!(
        runtime.session().is_active(),
        "the inbound server Hello should activate the client session"
    );
}

async fn run_forced_cancelled_fragment(prefix_len: usize) -> anyhow::Result<ConnectedSessionExit> {
    let framed = crlf_framed(SERVER_HELLO);
    assert!(
        prefix_len < framed.len(),
        "the gated prefix must leave at least one byte for release"
    );
    let (release_remaining_tx, release_remaining_rx) = oneshot::channel();
    let (address, server_task) = spawn_raw_loopback_server(RawServerWrites::GatedFragment {
        prefix: framed[..prefix_len].to_vec(),
        remaining: framed[prefix_len..].to_vec(),
        release_remaining: release_remaining_rx,
    })
    .await;
    let mut config = test_client_loop_config_with_addr(address);
    config.max_connected_runtime_seconds = 2.0;
    let mut runtime = create_client_runtime(&config);
    let stream = TcpStream::connect(address)
        .await
        .expect("fragmented client should connect");
    let (local_input_tx, mut local_input_rx) = unbounded_channel::<String>();
    let (observation_tx, mut observation_rx) = unbounded_channel();
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let result = {
        let session = observe_inbound_protocol_reads(
            observation_tx,
            run_connected_client_session(
                stream,
                &mut runtime,
                &config,
                None,
                Some(&mut local_input_rx),
                &mut notification_sink,
                &mut file_difference_sink,
            ),
        );
        tokio::pin!(session);

        loop {
            tokio::select! {
                observation = observation_rx.recv() => {
                    if matches!(
                        observation,
                        Some(InboundProtocolReadObservation::ConsumedPartial(bytes))
                            if bytes >= prefix_len
                    ) {
                        break;
                    }
                }
                completed = &mut session => {
                    panic!("fragmented session completed before consuming its gated prefix: {completed:?}");
                }
            }
        }

        drop(local_input_tx);
        loop {
            tokio::select! {
                observation = observation_rx.recv() => {
                    if matches!(
                        observation,
                        Some(InboundProtocolReadObservation::CancelledPartial(bytes))
                            if bytes >= prefix_len
                    ) {
                        break;
                    }
                }
                completed = &mut session => {
                    panic!("fragmented session completed before the read cancellation barrier: {completed:?}");
                }
            }
        }
        release_remaining_tx
            .send(())
            .expect("raw server should still be waiting to release the remaining bytes");
        tokio::time::timeout(RAW_SOCKET_TIMEOUT, &mut session)
            .await
            .expect("fragmented session should reach its bounded outcome")
    };
    join_raw_server(server_task).await;
    result
}

fn assert_cancelled_fragment_survives(result: anyhow::Result<ConnectedSessionExit>) {
    assert_eq!(
        result.expect("the complete released frame should remain valid after read cancellation"),
        ConnectedSessionExit::TransportClosed,
        "a cancellation-safe reader should accept the complete released frame"
    );
}

#[test]
fn one_byte_fragmentation_survives_select_cancellation() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("raw framing test runtime should build")
        .block_on(async {
            assert_cancelled_fragment_survives(run_forced_cancelled_fragment(4).await);
        });
}

#[test]
fn split_crlf_survives_select_cancellation() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("raw framing test runtime should build")
        .block_on(async {
            assert_cancelled_fragment_survives(
                run_forced_cancelled_fragment(SERVER_HELLO.len() + 1).await,
            );
        });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_client_session_accepts_coalesced_raw_frames() {
    let (result, runtime) = run_raw_connected_session(RawServerWrites::Chunks(vec![
        coalesced_frames(&[SERVER_HELLO, SERVER_READY]),
    ]))
    .await;
    assert_eq!(
        result.expect("coalesced server frames should be accepted"),
        ConnectedSessionExit::TransportClosed
    );
    assert_server_hello_applied(&runtime);
    assert_eq!(runtime.session().user_ready(WIRE_USERNAME), Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_prefix_commits_before_malformed_invalid_utf8_and_oversized_suffixes() {
    let mut malformed = crlf_framed(SERVER_HELLO);
    malformed.extend_from_slice(b"{not-json}\r\n");
    let (result, runtime) =
        run_raw_connected_session(RawServerWrites::Chunks(vec![malformed])).await;
    let error = result.expect_err("malformed suffix should fail the connected session");
    assert!(
        matches!(
            error.downcast_ref::<sorotte_protocol::ProtocolError>(),
            Some(sorotte_protocol::ProtocolError::InvalidJson(_))
        ),
        "malformed suffix should surface a typed JSON error, got: {error:#}"
    );
    assert_server_hello_applied(&runtime);

    let mut invalid_utf8 = crlf_framed(SERVER_HELLO);
    invalid_utf8.extend_from_slice(&[0xff, b'\r', b'\n']);
    let (result, runtime) =
        run_raw_connected_session(RawServerWrites::Chunks(vec![invalid_utf8])).await;
    let error = result.expect_err("invalid UTF-8 suffix should fail the connected session");
    assert!(
        error.downcast_ref::<std::string::FromUtf8Error>().is_some(),
        "invalid UTF-8 suffix should preserve its typed source, got: {error:#}"
    );
    assert_server_hello_applied(&runtime);

    let mut oversized = crlf_framed(SERVER_HELLO);
    oversized.extend(std::iter::repeat_n(
        b'x',
        crate::protocol_io::MAX_INBOUND_PROTOCOL_LINE_BYTES + 1,
    ));
    oversized.extend_from_slice(b"\r\n");
    let (result, runtime) =
        run_raw_connected_session(RawServerWrites::Chunks(vec![oversized])).await;
    let error = result.expect_err("oversized suffix should fail the connected session");
    assert!(
        error.to_string().contains("Inbound protocol line too long"),
        "oversized suffix should report the framing limit, got: {error:#}"
    );
    assert_server_hello_applied(&runtime);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncated_and_valid_unterminated_frames_have_distinct_bounded_outcomes() {
    let (result, runtime) = run_raw_connected_session(RawServerWrites::Chunks(vec![
        b"{\"Set\":{\"ready\":".to_vec(),
    ]))
    .await;
    let error = result.expect_err("truncated final JSON frame should fail at EOF");
    assert!(
        matches!(
            error.downcast_ref::<sorotte_protocol::ProtocolError>(),
            Some(sorotte_protocol::ProtocolError::InvalidJson(_))
        ),
        "truncated frame should surface a typed JSON error, got: {error:#}"
    );
    assert_eq!(runtime.session().username(), Some("cli-user"));
    assert_eq!(runtime.session().room(), Some("cli-room"));
    assert!(
        !runtime.session().is_active(),
        "truncated pre-Hello input must not activate the session"
    );

    let (result, runtime) =
        run_raw_connected_session(RawServerWrites::Chunks(vec![SERVER_HELLO.to_vec()])).await;
    assert_eq!(
        result.expect("valid unterminated final frame should be accepted at EOF"),
        ConnectedSessionExit::TransportClosed
    );
    assert_server_hello_applied(&runtime);
}
