use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn close_before_or_after_hello(completed_membership: bool, max_retries: u32) {
    let env = TestEnvGuard::lock(&CLIENT_CONNECTION_PHASE_ENV_LOCK);
    env.set_var("SOROTTE_CLIENT_TLS_POLICY", "Plaintext");
    env.set_var("SOROTTE_CLIENT_PLEX_SYNC", "false");
    env.set_var("SOROTTE_CLIENT_PLEX_STREAMING", "false");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let server_count = count.clone();
    let (input_tx, input_rx) = unbounded_channel();
    let server = tokio::spawn(async move {
        for attempt in 1..=4 {
            let (socket, _) = listener.accept().await.unwrap();
            server_count.fetch_add(1, Ordering::SeqCst);
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            let hello = lines.next_line().await.unwrap().unwrap();
            assert!(hello.contains("\"Hello\""));
            if completed_membership || attempt == 4 {
                writer.write_all(b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"room1\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n").await.unwrap();
                writer.flush().await.unwrap();
            }
            if attempt < 4 {
                writer.shutdown().await.unwrap();
                while lines.next_line().await.unwrap().is_some() {}
                continue;
            }
            writer
                .write_all(b"{\"State\":{\"ping\":{\"latencyCalculation\":37.0}}}\n")
                .await
                .unwrap();
            writer.flush().await.unwrap();
            loop {
                let line = lines.next_line().await.unwrap().unwrap();
                if let ProtocolMessage::State(message) = decode_message_line(&line).unwrap()
                    && message
                        .state
                        .ping
                        .as_ref()
                        .and_then(|ping| ping.latency_calculation)
                        == Some(37.0)
                {
                    break;
                }
            }
            input_tx.send("ch after recovery".to_owned()).unwrap();
            loop {
                let line = lines.next_line().await.unwrap().unwrap();
                if let ProtocolMessage::Chat(message) = decode_message_line(&line).unwrap() {
                    assert_eq!(
                        message.chat,
                        sorotte_protocol::ChatPayload::text("after recovery")
                    );
                    break;
                }
            }
            while lines.next_line().await.unwrap().is_some() {}
        }
    });
    let mut config = test_client_loop_config_with_addr(address);
    config.max_retries = max_retries;
    config.max_connected_runtime_seconds = 0.3;
    let runtime = create_client_runtime(&config);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_client_network_loop_with_prepared_runtime_for_test(&config, runtime, Some(input_rx)),
    )
    .await;
    let attempts = count.load(Ordering::SeqCst);
    if completed_membership {
        let recovered = result
            .expect("bounded sessions finish")
            .expect("healthy reconnection succeeds");
        assert_eq!(recovered.session().room(), Some("room1"));
        server.await.unwrap();
        assert_eq!(attempts, 4);
    } else {
        // The bounded fixture would accept a fourth healthy connection if the
        // failure budget were bypassed. Stop its waiting accept after exhaustion.
        server.abort();
        let stopped = server.await;
        assert!(stopped.is_ok() || stopped.unwrap_err().is_cancelled());
        let error = result
            .expect("incomplete handshakes exhaust their budget")
            .err()
            .expect("clean EOF without Hello is a failed attempt");
        assert!(
            error
                .to_string()
                .contains("before startup handshake completed"),
            "{error:#}"
        );
        assert_eq!(attempts, max_retries as usize + 2);
    }
}

#[tokio::test]
async fn prehello_clean_close_consumes_zero_retry_budget() {
    close_before_or_after_hello(false, 0).await;
    close_before_or_after_hello(true, 0).await;
}

#[tokio::test]
async fn prehello_clean_close_consumes_nonzero_retry_budget() {
    close_before_or_after_hello(false, 1).await;
    close_before_or_after_hello(true, 1).await;
}
