use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_player_api::PlayerAdapter;
use sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const MAX_INBOUND_PROTOCOL_LINE_BYTES: usize = DEFAULT_MAX_PROTOCOL_LINE_BYTES;

pub(crate) async fn read_inbound_protocol_line<R>(reader: &mut R) -> anyhow::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }

        if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
            let raw_line_len = line.len() + newline_index;
            let line_len = if newline_index == 0 {
                raw_line_len.saturating_sub(usize::from(line.last() == Some(&b'\r')))
            } else {
                raw_line_len.saturating_sub(usize::from(available[newline_index - 1] == b'\r'))
            };
            if line_len > MAX_INBOUND_PROTOCOL_LINE_BYTES {
                return Err(anyhow::anyhow!(
                    "Inbound protocol line too long: exceeded {} bytes",
                    MAX_INBOUND_PROTOCOL_LINE_BYTES
                ));
            }

            let take = newline_index + 1;
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            break;
        }

        if line.len() + available.len() > MAX_INBOUND_PROTOCOL_LINE_BYTES {
            return Err(anyhow::anyhow!(
                "Inbound protocol line too long: exceeded {} bytes",
                MAX_INBOUND_PROTOCOL_LINE_BYTES
            ));
        }

        let take = available.len();
        line.extend_from_slice(available);
        reader.consume(take);
    }

    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(String::from_utf8(line)?))
}

pub(crate) async fn write_protocol_line<W>(writer: &mut W, line: &str) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\r\n").await?;
    writer.flush().await?;
    Ok(())
}

pub(super) async fn flush_runtime_protocol_lines<P>(
    runtime: &mut ClientApplication<P>,
    writer: &mut (impl AsyncWrite + Unpin),
) -> anyhow::Result<()>
where
    P: PlayerAdapter,
{
    while let Some(pending) = runtime.pending_protocol_line()? {
        if let Err(error) = write_protocol_line(writer, pending.line()).await {
            let _ = runtime.release_protocol_line(pending.lease());
            return Err(error);
        }
        let acknowledged = runtime.acknowledge_protocol_line(pending.lease());
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_client_core::{
        ClientEffect, ClientEffectSink, ClientRuntime, ClientSession, ConnectionPhase,
        LogicalMediaId, MediaTransportKind, PlaybackBarrierStartConfig,
        PlaybackBarrierTimeoutAction, QueuedRuntimeControl,
    };
    use sorotte_protocol::{
        PlaybackBarrierPolicy, PlaybackBarrierRequestResultPayload, PlaybackBarrierSetExtension,
        ProtocolMessage, SetPayload, decode_message_line_items, encode_message_line,
    };
    use tokio::io::BufReader;

    struct ProtocolIoTestPlayer;

    impl PlayerAdapter for ProtocolIoTestPlayer {
        fn name(&self) -> &'static str {
            "protocol-io-test-player"
        }
    }

    #[tokio::test]
    async fn cli_connected_session_rejects_inbound_line_over_max_bytes() {
        let input = vec![b'a'; MAX_INBOUND_PROTOCOL_LINE_BYTES + 1];
        let mut reader = BufReader::new(&input[..]);

        let error = read_inbound_protocol_line(&mut reader)
            .await
            .expect_err("oversized inbound line should fail");

        assert!(
            error.to_string().contains("Inbound protocol line too long"),
            "oversized inbound line should produce a clear error"
        );
    }

    #[tokio::test]
    async fn cli_connected_session_accepts_batched_valid_line() {
        let input = br#"{"Chat":"hello","List":null}"#.to_vec();
        let mut framed = input.clone();
        framed.extend_from_slice(b"\r\n");
        let mut reader = BufReader::new(&framed[..]);

        let line = read_inbound_protocol_line(&mut reader)
            .await
            .expect("batched line read should succeed")
            .expect("batched line should be present");

        assert_eq!(line.as_bytes(), input);
        let items = decode_message_line_items(&line).expect("batched line should decode");
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn write_protocol_line_uses_crlf_framing() {
        let mut output = Vec::new();

        write_protocol_line(&mut output, r#"{"List":null}"#)
            .await
            .expect("protocol line should write");

        assert_eq!(output, b"{\"List\":null}\r\n");
    }

    #[tokio::test]
    async fn cli_writer_failure_leaves_protocol_message_queued() {
        let mut control = QueuedRuntimeControl::default();
        control
            .emit(ClientEffect::SendChat("retry me".to_owned()))
            .expect("chat effect should be supported");
        let runtime = ClientRuntime::new(ClientSession::default(), ProtocolIoTestPlayer, control);
        let mut runtime = ClientApplication::from_runtime(runtime);
        let (reader, mut writer) = tokio::io::duplex(64);
        drop(reader);

        flush_runtime_protocol_lines(&mut runtime, &mut writer)
            .await
            .expect_err("closed transport should reject the protocol line");

        assert_eq!(runtime.pending_protocol_message_count(), 1);
        assert!(
            runtime
                .pending_protocol_line()
                .expect("pending line should still serialize")
                .expect("failed line should remain pending")
                .line()
                .contains("retry me")
        );
    }

    #[test]
    fn cli_application_boundary_keeps_retry_later_nonfatal_and_retries_same_attempt_once() {
        let mut runtime = ClientApplication::with_default_session(ProtocolIoTestPlayer);
        runtime
            .apply_protocol_line(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect("barrier-aware server Hello should apply through the CLI boundary");
        runtime
            .apply_protocol_line(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"controller":true}}}}"#,
                2.0,
                false,
                false,
                false,
            )
            .expect("local controller projection should apply through the CLI boundary");
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            timeout_action: PlaybackBarrierTimeoutAction::Continue,
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("sha256:cli-retry-later")
                .expect("logical media ID should be valid"),
            MediaTransportKind::NetworkVod,
            3.0,
        );

        let pending = runtime
            .pending_protocol_line()
            .expect("CLI request should serialize")
            .expect("CLI media preparation should queue a barrier request");
        let original = decode_message_line_items(pending.line())
            .expect("CLI request should decode")
            .into_iter()
            .find_map(|item| match item.message.ok()? {
                ProtocolMessage::Set(set) => set.set.playback_barrier_v1().ok()??.prepare,
                _ => None,
            })
            .expect("CLI request should contain PrepareMedia");
        let lease = pending.lease();
        assert!(
            runtime.acknowledge_protocol_line(lease).is_some(),
            "the local write receipt should release only the serialized request"
        );

        let retry_later = ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_request_result(
                    PlaybackBarrierRequestResultPayload::retry_later(
                        original
                            .request_id
                            .clone()
                            .expect("CLI barrier request should have an operation ID"),
                        original.request_nonce,
                        1_000,
                    ),
                ),
            ),
        );
        runtime
            .apply_protocol_line(
                &encode_message_line(&retry_later).expect("retryLater should encode"),
                10.0,
                false,
                false,
                false,
            )
            .expect("retryLater should be nonfatal through the CLI application boundary");

        assert!(matches!(
            runtime.connection_phase(),
            ConnectionPhase::Active(_)
        ));
        assert!(
            !runtime.take_stop_reconnect_requested(),
            "retryLater must not stop the CLI reconnect loop"
        );
        assert_eq!(
            runtime.pending_playback_barrier_retry_delay_at(10.0),
            Some(1.0),
            "the CLI must retain the current semantic intent behind the server delay"
        );
        runtime
            .run_pending_playback_barrier_retry_at(10.999)
            .expect("an early CLI retry pump should be harmless");
        assert_eq!(runtime.pending_protocol_message_count(), 0);

        runtime
            .run_pending_playback_barrier_retry_at(11.0)
            .expect("the due CLI retry should dispatch");
        assert_eq!(runtime.pending_protocol_message_count(), 1);
        let retried = runtime
            .pending_protocol_messages()
            .iter()
            .find_map(|message| match message {
                ProtocolMessage::Set(set) => set
                    .set
                    .playback_barrier_v1()
                    .ok()?
                    .and_then(|extension| extension.prepare),
                _ => None,
            })
            .expect("CLI retry should contain PrepareMedia");
        assert_eq!(retried.request_id, original.request_id);
        assert_eq!(retried.request_nonce, original.request_nonce);
        assert_eq!(retried.logical_media_id, original.logical_media_id);

        runtime
            .run_pending_playback_barrier_retry_at(12.0)
            .expect("a repeated CLI retry pump should be harmless");
        assert_eq!(
            runtime.pending_protocol_message_count(),
            1,
            "the CLI retry pump must emit exactly one attempt"
        );
    }
}
