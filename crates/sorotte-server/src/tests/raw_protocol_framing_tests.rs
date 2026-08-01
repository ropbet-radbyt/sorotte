use super::*;
use std::collections::BTreeMap;

const RAW_SOCKET_TIMEOUT: Duration = Duration::from_secs(3);

struct RawLoopbackClient {
    client_id: String,
    stream: BufReader<TcpStream>,
    session_task: tokio::task::JoinHandle<Result<(), ServerNetworkError>>,
}

impl RawLoopbackClient {
    async fn connect(
        runtime: ServerActorHandle,
        client_event_senders: crate::network::SharedClientEventSenders,
        client_id: impl Into<String>,
    ) -> Self {
        let client_id = client_id.into();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test-owned loopback listener should bind");
        let address = listener
            .local_addr()
            .expect("test-owned listener should have an address");
        assert!(address.ip().is_loopback());
        let (client_result, accepted_result) =
            tokio::join!(TcpStream::connect(address), listener.accept());
        let client_stream = client_result.expect("raw test client should connect");
        let (server_stream, peer_address) = accepted_result.expect("raw test server should accept");
        assert!(peer_address.ip().is_loopback());
        client_stream
            .set_nodelay(true)
            .expect("raw test client should enable TCP_NODELAY");
        server_stream
            .set_nodelay(true)
            .expect("raw test server should enable TCP_NODELAY");

        let task_client_id = client_id.clone();
        let session_task = tokio::spawn(
            crate::network::run_server_network_client_session_with_pre_hello_timeout(
                server_stream,
                Some(peer_address.ip().to_string()),
                task_client_id,
                runtime,
                client_event_senders,
                None,
                RAW_SOCKET_TIMEOUT,
            ),
        );
        Self {
            client_id,
            stream: BufReader::new(client_stream),
            session_task,
        }
    }

    async fn write(&mut self, bytes: &[u8]) {
        self.stream
            .get_mut()
            .write_all(bytes)
            .await
            .expect("raw loopback bytes should write");
        self.stream
            .get_mut()
            .flush()
            .await
            .expect("raw loopback bytes should flush");
    }

    async fn write_one_byte_fragments(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write(std::slice::from_ref(byte)).await;
            tokio::task::yield_now().await;
        }
    }

    async fn half_close_write(&mut self) {
        self.stream
            .get_mut()
            .shutdown()
            .await
            .expect("raw loopback write half should close");
    }

    async fn read_until(
        &mut self,
        description: &str,
        mut predicate: impl FnMut(&ProtocolMessage) -> bool,
    ) -> Vec<ProtocolMessage> {
        timeout(RAW_SOCKET_TIMEOUT, async {
            let mut messages = Vec::new();
            for _ in 0..32 {
                let mut line = String::new();
                let bytes_read = self
                    .stream
                    .read_line(&mut line)
                    .await
                    .expect("server response bytes should read");
                assert_ne!(
                    bytes_read, 0,
                    "server closed before {description}; received {messages:?}"
                );
                let line = line.trim_end_matches(['\r', '\n']);
                let message =
                    decode_message_line(line).expect("server response line should decode");
                let done = predicate(&message);
                messages.push(message);
                if done {
                    return messages;
                }
            }
            panic!("server did not emit {description} within 32 protocol lines");
        })
        .await
        .unwrap_or_else(|_| panic!("server did not emit {description} before timeout"))
    }

    async fn read_to_eof(&mut self) -> Vec<ProtocolMessage> {
        timeout(RAW_SOCKET_TIMEOUT, async {
            let mut messages = Vec::new();
            loop {
                let mut line = String::new();
                let bytes_read = self
                    .stream
                    .read_line(&mut line)
                    .await
                    .expect("server tail should read");
                if bytes_read == 0 {
                    return messages;
                }
                messages.push(
                    decode_message_line(line.trim_end_matches(['\r', '\n']))
                        .expect("server tail should contain protocol lines"),
                );
            }
        })
        .await
        .expect("server connection should reach EOF before timeout")
    }

    async fn join(self) -> Result<(), ServerNetworkError> {
        timeout(RAW_SOCKET_TIMEOUT, self.session_task)
            .await
            .expect("server client-session task should finish before timeout")
            .expect("server client-session task should join")
    }
}

fn hello_line(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
    )
}

async fn assert_session_absent(runtime: &ServerActorHandle, client_id: &str) {
    assert!(
        runtime
            .session(client_id)
            .await
            .expect("server actor should answer session query")
            .is_none(),
        "client {client_id} should not retain a server session"
    );
}

async fn close_successful_client(runtime: &ServerActorHandle, mut client: RawLoopbackClient) {
    let client_id = client.client_id.clone();
    client.half_close_write().await;
    assert!(
        client.read_to_eof().await.is_empty(),
        "a clean read half-close should not create trailing protocol output"
    );
    client
        .join()
        .await
        .expect("clean read half-close should end the server session normally");
    assert_session_absent(runtime, &client_id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_server_session_accepts_fragmented_split_and_coalesced_raw_frames() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(BTreeMap::new()));

    let mut one_byte = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders.clone(),
        "raw-one-byte",
    )
    .await;
    let one_byte_hello = format!("{}\r\n", hello_line("one-byte", "raw-room"));
    one_byte
        .write_one_byte_fragments(one_byte_hello.as_bytes())
        .await;
    one_byte
        .read_until("Hello after one-byte fragmentation", |message| {
            matches!(message, ProtocolMessage::Hello(_))
        })
        .await;
    assert_eq!(
        runtime
            .session("raw-one-byte")
            .await
            .expect("server actor should answer session query")
            .map(|session| session.username),
        Some("one-byte".to_owned())
    );
    close_successful_client(&runtime, one_byte).await;

    let mut split_crlf = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders.clone(),
        "raw-split-crlf",
    )
    .await;
    split_crlf
        .write(hello_line("split-crlf", "raw-room").as_bytes())
        .await;
    split_crlf.write(b"\r").await;
    assert_session_absent(&runtime, "raw-split-crlf").await;
    split_crlf.write(b"\n").await;
    split_crlf
        .read_until("Hello after split CRLF", |message| {
            matches!(message, ProtocolMessage::Hello(_))
        })
        .await;
    close_successful_client(&runtime, split_crlf).await;

    let mut coalesced = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders.clone(),
        "raw-coalesced",
    )
    .await;
    let coalesced_frames = format!(
        "{}\r\n{{\"List\":null}}\r\n",
        hello_line("coalesced", "raw-room")
    );
    coalesced.write(coalesced_frames.as_bytes()).await;
    let responses = coalesced
        .read_until("List after coalesced Hello and List frames", |message| {
            matches!(message, ProtocolMessage::List(_))
        })
        .await;
    let hello_index = responses
        .iter()
        .position(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("coalesced Hello should produce a Hello response");
    let list_index = responses
        .iter()
        .position(|message| matches!(message, ProtocolMessage::List(_)))
        .expect("coalesced List should produce a List response");
    assert!(
        hello_index < list_index,
        "coalesced frames must preserve application order: {responses:?}"
    );
    close_successful_client(&runtime, coalesced).await;

    runtime
        .shutdown()
        .await
        .expect("server actor should shut down cleanly");
}

#[derive(Clone, Copy)]
enum FaultSuffix {
    MalformedJson,
    InvalidUtf8,
    Oversized,
}

impl FaultSuffix {
    fn label(self) -> &'static str {
        match self {
            Self::MalformedJson => "malformed-json",
            Self::InvalidUtf8 => "invalid-utf8",
            Self::Oversized => "oversized",
        }
    }

    fn bytes(self) -> Vec<u8> {
        match self {
            Self::MalformedJson => b"{\"State\":\r\n".to_vec(),
            Self::InvalidUtf8 => vec![0xff, b'\r', b'\n'],
            Self::Oversized => {
                let mut bytes = vec![b'x'; crate::MAX_PROTOCOL_LINE_BYTES + 1];
                bytes.extend_from_slice(b"\r\n");
                bytes
            }
        }
    }

    fn expected_error(self) -> &'static str {
        match self {
            Self::MalformedJson => crate::LEGACY_SERVER_NOT_JSON_ERROR_PREFIX,
            Self::InvalidUtf8 => crate::LEGACY_SERVER_LINE_DECODE_ERROR,
            Self::Oversized => crate::PROTOCOL_LINE_TOO_LONG_ERROR,
        }
    }

    fn expects_session_io_error(self) -> bool {
        matches!(self, Self::InvalidUtf8 | Self::Oversized)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn valid_prefix_commits_before_fault_suffix_and_bad_connection_is_isolated() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(BTreeMap::new()));
    let mut sentinel = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders.clone(),
        "raw-sentinel",
    )
    .await;
    sentinel
        .write(format!("{}\r\n", hello_line("sentinel", "isolation-room")).as_bytes())
        .await;
    sentinel
        .read_until("sentinel Hello", |message| {
            matches!(message, ProtocolMessage::Hello(_))
        })
        .await;

    for fault in [
        FaultSuffix::MalformedJson,
        FaultSuffix::InvalidUtf8,
        FaultSuffix::Oversized,
    ] {
        let client_id = format!("raw-fault-{}", fault.label());
        let username = format!("fault-{}", fault.label());
        let mut faulty = RawLoopbackClient::connect(
            runtime.clone(),
            client_event_senders.clone(),
            client_id.clone(),
        )
        .await;
        let mut coalesced = format!("{}\r\n", hello_line(&username, "isolation-room")).into_bytes();
        coalesced.extend_from_slice(&fault.bytes());
        faulty.write(&coalesced).await;

        let responses = faulty
            .read_until("Error after a valid prefix frame", |message| {
                matches!(message, ProtocolMessage::Error(_))
            })
            .await;
        let hello_index = responses
            .iter()
            .position(|message| matches!(message, ProtocolMessage::Hello(_)))
            .unwrap_or_else(|| {
                panic!(
                    "{} suffix discarded the valid Hello prefix: {responses:?}",
                    fault.label()
                )
            });
        let (error_index, error_message) = responses
            .iter()
            .enumerate()
            .find_map(|(index, message)| {
                let ProtocolMessage::Error(payload) = message else {
                    return None;
                };
                Some((index, payload.error.message.as_str()))
            })
            .expect("fault suffix should produce Error");
        assert!(
            hello_index < error_index,
            "{} suffix reordered prefix response and Error: {responses:?}",
            fault.label()
        );
        assert!(
            error_message.starts_with(fault.expected_error()),
            "{} suffix emitted unexpected error {error_message:?}",
            fault.label()
        );
        assert!(
            faulty.read_to_eof().await.is_empty(),
            "{} suffix should close immediately after Error",
            fault.label()
        );

        let result = faulty.join().await;
        if fault.expects_session_io_error() {
            let Err(ServerNetworkError::Io(source)) = result else {
                panic!(
                    "{} suffix should terminate with an IO error, got {result:?}",
                    fault.label()
                );
            };
            assert_eq!(source.kind(), io::ErrorKind::InvalidData);
        } else {
            result.expect("malformed JSON closes through the protocol action");
        }
        assert_session_absent(&runtime, &client_id).await;

        sentinel.write(b"{\"List\":null}\r\n").await;
        sentinel
            .read_until("sentinel List after peer fault", |message| {
                matches!(message, ProtocolMessage::List(_))
            })
            .await;
        assert!(
            runtime
                .session("raw-sentinel")
                .await
                .expect("server actor should answer sentinel session query")
                .is_some(),
            "{} peer fault must not close the healthy connection",
            fault.label()
        );
    }

    close_successful_client(&runtime, sentinel).await;
    runtime
        .shutdown()
        .await
        .expect("server actor should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn truncation_and_read_half_close_have_distinct_bounded_outcomes() {
    let runtime = ServerActorHandle::spawn(ServerRuntime::new());
    let client_event_senders = Arc::new(Mutex::new(BTreeMap::new()));

    let mut truncated = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders.clone(),
        "raw-truncated",
    )
    .await;
    truncated
        .write(br#"{"Hello":{"username":"truncated""#)
        .await;
    truncated.half_close_write().await;
    let responses = truncated
        .read_until("Error for truncated final frame", |message| {
            matches!(message, ProtocolMessage::Error(_))
        })
        .await;
    assert!(responses.iter().any(|message| {
        matches!(
            message,
            ProtocolMessage::Error(payload)
                if payload
                    .error
                    .message
                    .starts_with(crate::LEGACY_SERVER_NOT_JSON_ERROR_PREFIX)
        )
    }));
    assert!(truncated.read_to_eof().await.is_empty());
    truncated
        .join()
        .await
        .expect("truncated UTF-8 JSON should close through the protocol action");
    assert_session_absent(&runtime, "raw-truncated").await;

    let mut final_unterminated = RawLoopbackClient::connect(
        runtime.clone(),
        client_event_senders,
        "raw-final-unterminated",
    )
    .await;
    final_unterminated
        .write(hello_line("final-unterminated", "raw-room").as_bytes())
        .await;
    final_unterminated.half_close_write().await;
    final_unterminated
        .read_until("Hello for valid unterminated final frame", |message| {
            matches!(message, ProtocolMessage::Hello(_))
        })
        .await;
    assert!(final_unterminated.read_to_eof().await.is_empty());
    final_unterminated
        .join()
        .await
        .expect("valid unterminated final frame should commit before read EOF");
    assert_session_absent(&runtime, "raw-final-unterminated").await;

    runtime
        .shutdown()
        .await
        .expect("server actor should shut down cleanly");
}
