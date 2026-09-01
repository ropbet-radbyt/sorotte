use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_player_api::PlayerAdapter;
use sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

use crate::local_runtime_actions::PLAYER_CHAT_INPUT_POLL_INTERVAL_MS;

pub const MAX_INBOUND_PROTOCOL_LINE_BYTES: usize = DEFAULT_MAX_PROTOCOL_LINE_BYTES;

const LIFECYCLE_WRITE_BARRIER_ENV: &str = "SOROTTE_LIFECYCLE_WRITE_BARRIER";
const LIFECYCLE_WRITE_BARRIER_MODE: &str = "leased-oversized-frame";
const LIFECYCLE_WRITE_BARRIER_MIN_FRAME_BYTES: usize = 256 * 1024;
const LIFECYCLE_WRITE_BARRIER_TIMEOUT: Duration = Duration::from_secs(15);
const LIFECYCLE_WRITE_BARRIER_READY_SUFFIX: &str = ".leased-frame-ready";
const LIFECYCLE_WRITE_BARRIER_RELEASE_SUFFIX: &str = ".leased-frame-release";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LifecycleWriteBarrier {
    ready_path: PathBuf,
    release_path: PathBuf,
}

fn lifecycle_write_barrier_marker_path(
    evidence_path: &Path,
    suffix: &str,
) -> anyhow::Result<PathBuf> {
    let file_name = evidence_path.file_name().ok_or_else(|| {
        anyhow::anyhow!("{LIFECYCLE_WRITE_BARRIER_ENV} requires a lifecycle evidence file path")
    })?;
    let mut marker_name = OsString::from(file_name);
    marker_name.push(suffix);
    Ok(evidence_path.with_file_name(marker_name))
}

fn parse_lifecycle_write_barrier(
    evidence_enabled: bool,
    configured: Option<&str>,
    evidence_path: Option<&Path>,
    frame_bytes: usize,
) -> anyhow::Result<Option<LifecycleWriteBarrier>> {
    let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !evidence_enabled {
        return Err(anyhow::anyhow!(
            "{LIFECYCLE_WRITE_BARRIER_ENV} requires lifecycle evidence to be enabled"
        ));
    }
    if configured != LIFECYCLE_WRITE_BARRIER_MODE {
        return Err(anyhow::anyhow!(
            "{LIFECYCLE_WRITE_BARRIER_ENV} must be {LIFECYCLE_WRITE_BARRIER_MODE:?}"
        ));
    }
    let evidence_path = evidence_path.ok_or_else(|| {
        anyhow::anyhow!(
            "{LIFECYCLE_WRITE_BARRIER_ENV} requires {}",
            sorotte_lifecycle_evidence::EVIDENCE_PATH_ENV
        )
    })?;
    if frame_bytes < LIFECYCLE_WRITE_BARRIER_MIN_FRAME_BYTES {
        return Ok(None);
    }
    Ok(Some(LifecycleWriteBarrier {
        ready_path: lifecycle_write_barrier_marker_path(
            evidence_path,
            LIFECYCLE_WRITE_BARRIER_READY_SUFFIX,
        )?,
        release_path: lifecycle_write_barrier_marker_path(
            evidence_path,
            LIFECYCLE_WRITE_BARRIER_RELEASE_SUFFIX,
        )?,
    }))
}

fn lifecycle_write_barrier_for_frame(
    frame_bytes: usize,
) -> anyhow::Result<Option<LifecycleWriteBarrier>> {
    let configured = std::env::var(LIFECYCLE_WRITE_BARRIER_ENV).ok();
    let evidence_path =
        std::env::var_os(sorotte_lifecycle_evidence::EVIDENCE_PATH_ENV).map(PathBuf::from);
    parse_lifecycle_write_barrier(
        sorotte_lifecycle_evidence::global_enabled(),
        configured.as_deref(),
        evidence_path.as_deref(),
        frame_bytes,
    )
}

async fn await_lifecycle_write_barrier(frame_bytes: usize) -> anyhow::Result<()> {
    let Some(barrier) = lifecycle_write_barrier_for_frame(frame_bytes)? else {
        return Ok(());
    };
    await_configured_lifecycle_write_barrier(barrier).await
}

async fn await_configured_lifecycle_write_barrier(
    barrier: LifecycleWriteBarrier,
) -> anyhow::Result<()> {
    if barrier.release_path.exists() && !barrier.ready_path.exists() {
        return Err(anyhow::anyhow!(
            "lifecycle write barrier release marker existed before the leased frame arrived"
        ));
    }
    let mut ready = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&barrier.ready_path)
    {
        Ok(ready) => ready,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // The barrier is one-shot. A reconnect may retry the semantic
            // frame after the first transport has failed.
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "failed to create lifecycle write barrier readiness marker: {error}"
            ));
        }
    };
    ready.write_all(b"leased\n").map_err(|error| {
        anyhow::anyhow!("failed to write lifecycle write barrier readiness marker: {error}")
    })?;
    ready.sync_all().map_err(|error| {
        anyhow::anyhow!("failed to sync lifecycle write barrier readiness marker: {error}")
    })?;
    drop(ready);

    let deadline = Instant::now() + LIFECYCLE_WRITE_BARRIER_TIMEOUT;
    loop {
        if barrier.release_path.is_file() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!(
                "timed out waiting for lifecycle write barrier release"
            ));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InboundProtocolReadObservation {
    ConsumedPartial(usize),
    CancelledPartial(usize),
}

#[cfg(test)]
tokio::task_local! {
    static INBOUND_PROTOCOL_READ_OBSERVER:
        tokio::sync::mpsc::UnboundedSender<InboundProtocolReadObservation>;
}

#[cfg(test)]
pub(crate) async fn observe_inbound_protocol_reads<F>(
    observer: tokio::sync::mpsc::UnboundedSender<InboundProtocolReadObservation>,
    future: F,
) -> F::Output
where
    F: std::future::Future,
{
    INBOUND_PROTOCOL_READ_OBSERVER.scope(observer, future).await
}

#[cfg(test)]
struct InboundProtocolReadGuard {
    partial_bytes: usize,
    completed: bool,
}

#[cfg(test)]
impl InboundProtocolReadGuard {
    fn new(partial_bytes: usize) -> Self {
        Self {
            partial_bytes,
            completed: false,
        }
    }

    fn consumed_partial(&mut self, bytes: usize) {
        self.partial_bytes = bytes;
        let _ = INBOUND_PROTOCOL_READ_OBSERVER.try_with(|observer| {
            observer.send(InboundProtocolReadObservation::ConsumedPartial(bytes))
        });
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

#[cfg(test)]
impl Drop for InboundProtocolReadGuard {
    fn drop(&mut self) {
        if self.partial_bytes == 0 || self.completed {
            return;
        }
        let _ = INBOUND_PROTOCOL_READ_OBSERVER.try_with(|observer| {
            observer.send(InboundProtocolReadObservation::CancelledPartial(
                self.partial_bytes,
            ))
        });
    }
}

#[derive(Debug, Default)]
pub struct InboundProtocolLineReader {
    partial_line: Vec<u8>,
}

impl InboundProtocolLineReader {
    pub async fn read_line<R>(&mut self, reader: &mut R) -> anyhow::Result<Option<String>>
    where
        R: AsyncBufRead + Unpin,
    {
        #[cfg(test)]
        let mut read_guard = InboundProtocolReadGuard::new(self.partial_line.len());
        loop {
            let available = match reader.fill_buf().await {
                Ok(available) => available,
                Err(error) => {
                    self.partial_line.clear();
                    #[cfg(test)]
                    read_guard.complete();
                    return Err(error.into());
                }
            };
            if available.is_empty() {
                if self.partial_line.is_empty() {
                    #[cfg(test)]
                    read_guard.complete();
                    return Ok(None);
                }
                break;
            }

            if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
                let raw_line_len = self.partial_line.len() + newline_index;
                let line_len = if newline_index == 0 {
                    raw_line_len
                        .saturating_sub(usize::from(self.partial_line.last() == Some(&b'\r')))
                } else {
                    raw_line_len.saturating_sub(usize::from(available[newline_index - 1] == b'\r'))
                };
                if line_len > MAX_INBOUND_PROTOCOL_LINE_BYTES {
                    self.partial_line.clear();
                    #[cfg(test)]
                    read_guard.complete();
                    return Err(anyhow::anyhow!(
                        "Inbound protocol line too long: exceeded {} bytes",
                        MAX_INBOUND_PROTOCOL_LINE_BYTES
                    ));
                }

                let take = newline_index + 1;
                self.partial_line.extend_from_slice(&available[..take]);
                reader.consume(take);
                break;
            }

            let buffered_len = self.partial_line.len() + available.len();
            let ends_with_framing_cr = available
                .last()
                .or_else(|| self.partial_line.last())
                .is_some_and(|byte| *byte == b'\r');
            let payload_len = buffered_len.saturating_sub(usize::from(ends_with_framing_cr));
            if payload_len > MAX_INBOUND_PROTOCOL_LINE_BYTES {
                self.partial_line.clear();
                #[cfg(test)]
                read_guard.complete();
                return Err(anyhow::anyhow!(
                    "Inbound protocol line too long: exceeded {} bytes",
                    MAX_INBOUND_PROTOCOL_LINE_BYTES
                ));
            }

            let take = available.len();
            self.partial_line.extend_from_slice(available);
            reader.consume(take);
            #[cfg(test)]
            read_guard.consumed_partial(self.partial_line.len());
        }

        let mut line = std::mem::take(&mut self.partial_line);
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        #[cfg(test)]
        read_guard.complete();
        Ok(Some(String::from_utf8(line)?))
    }
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

async fn flush_runtime_protocol_lines_with_deadline<P>(
    runtime: &mut ClientApplication<P>,
    writer: &mut (impl AsyncWrite + Unpin),
    deadline: Option<Instant>,
) -> anyhow::Result<bool>
where
    P: PlayerAdapter,
{
    while let Some(pending) = runtime.pending_protocol_line()? {
        let write = async {
            await_lifecycle_write_barrier(pending.line().len().saturating_add(2)).await?;
            write_protocol_line(writer, pending.line()).await
        };
        tokio::pin!(write);
        let mut maintenance_tick = tokio::time::interval(std::time::Duration::from_millis(
            PLAYER_CHAT_INPUT_POLL_INTERVAL_MS,
        ));
        maintenance_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let deadline_elapsed = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(deadline_elapsed);
        let write_result = loop {
            tokio::select! {
                result = &mut write => break Some(result),
                _ = &mut deadline_elapsed => break None,
                _ = maintenance_tick.tick() => {
                    runtime.with_player_io(PlayerAdapter::maintain_runtime_leases_nonblocking);
                }
            }
        };
        match write_result {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                let _ = runtime.release_protocol_line(pending.lease());
                return Err(error);
            }
            None => {
                let _ = runtime.release_protocol_line(pending.lease());
                return Ok(false);
            }
        }
        let acknowledged = runtime.acknowledge_protocol_line(pending.lease());
        debug_assert!(acknowledged.is_some());
    }
    Ok(true)
}

pub(super) async fn flush_runtime_protocol_lines<P>(
    runtime: &mut ClientApplication<P>,
    writer: &mut (impl AsyncWrite + Unpin),
) -> anyhow::Result<()>
where
    P: PlayerAdapter,
{
    let completed = flush_runtime_protocol_lines_with_deadline(runtime, writer, None).await?;
    debug_assert!(completed, "a flush without a deadline cannot time out");
    Ok(())
}

pub(super) async fn flush_runtime_protocol_lines_until<P>(
    runtime: &mut ClientApplication<P>,
    writer: &mut (impl AsyncWrite + Unpin),
    deadline: Instant,
) -> anyhow::Result<bool>
where
    P: PlayerAdapter,
{
    flush_runtime_protocol_lines_with_deadline(runtime, writer, Some(deadline)).await
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::AsyncReadExt;
    use tokio::io::BufReader;

    struct ProtocolIoTestPlayer;

    async fn read_one_protocol_line_for_test<R>(reader: &mut R) -> anyhow::Result<Option<String>>
    where
        R: AsyncBufRead + Unpin,
    {
        InboundProtocolLineReader::default().read_line(reader).await
    }

    impl PlayerAdapter for ProtocolIoTestPlayer {
        fn name(&self) -> &'static str {
            "protocol-io-test-player"
        }
    }

    struct MaintainingProtocolIoTestPlayer(Arc<AtomicUsize>);

    impl PlayerAdapter for MaintainingProtocolIoTestPlayer {
        fn name(&self) -> &'static str {
            "maintaining-protocol-io-test-player"
        }

        fn maintain_runtime_leases_nonblocking(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn maintain_runtime_integrations(&mut self) {
            panic!("async protocol writes must not invoke blocking player maintenance");
        }
    }

    #[test]
    fn lifecycle_write_barrier_is_evidence_gated_bounded_and_path_derived() {
        let evidence_path = Path::new("evidence/client-follower.jsonl");
        assert_eq!(
            parse_lifecycle_write_barrier(false, None, None, usize::MAX)
                .expect("an absent barrier must be inert"),
            None
        );
        assert!(
            parse_lifecycle_write_barrier(
                false,
                Some(LIFECYCLE_WRITE_BARRIER_MODE),
                Some(evidence_path),
                usize::MAX,
            )
            .expect_err("ordinary clients must reject the verification-only barrier")
            .to_string()
            .contains("requires lifecycle evidence")
        );
        assert!(
            parse_lifecycle_write_barrier(
                true,
                Some("unknown-mode"),
                Some(evidence_path),
                usize::MAX,
            )
            .expect_err("unknown barrier modes must fail closed")
            .to_string()
            .contains(LIFECYCLE_WRITE_BARRIER_MODE)
        );
        assert!(
            parse_lifecycle_write_barrier(
                true,
                Some(LIFECYCLE_WRITE_BARRIER_MODE),
                None,
                usize::MAX,
            )
            .expect_err("the marker path must derive from lifecycle evidence")
            .to_string()
            .contains(sorotte_lifecycle_evidence::EVIDENCE_PATH_ENV)
        );
        assert_eq!(
            parse_lifecycle_write_barrier(
                true,
                Some(LIFECYCLE_WRITE_BARRIER_MODE),
                Some(evidence_path),
                LIFECYCLE_WRITE_BARRIER_MIN_FRAME_BYTES - 1,
            )
            .expect("ordinary frames must bypass the barrier"),
            None
        );

        let barrier = parse_lifecycle_write_barrier(
            true,
            Some(LIFECYCLE_WRITE_BARRIER_MODE),
            Some(evidence_path),
            LIFECYCLE_WRITE_BARRIER_MIN_FRAME_BYTES,
        )
        .expect("valid barrier configuration")
        .expect("the threshold frame must be gated");
        assert_eq!(
            barrier.ready_path,
            Path::new("evidence/client-follower.jsonl.leased-frame-ready")
        );
        assert_eq!(
            barrier.release_path,
            Path::new("evidence/client-follower.jsonl.leased-frame-release")
        );
    }

    #[tokio::test]
    async fn lifecycle_write_barrier_waits_once_for_an_explicit_release() {
        static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sorotte-cli-lifecycle-write-barrier-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&root).expect("unique barrier test directory should be created");
        let barrier = LifecycleWriteBarrier {
            ready_path: root.join("ready"),
            release_path: root.join("release"),
        };
        let waiting_barrier = barrier.clone();
        let waiter =
            tokio::spawn(
                async move { await_configured_lifecycle_write_barrier(waiting_barrier).await },
            );
        tokio::time::timeout(Duration::from_secs(2), async {
            while !barrier.ready_path.is_file() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the leased-frame marker should appear");
        assert!(
            !waiter.is_finished(),
            "the write must remain behind the barrier"
        );
        std::fs::write(&barrier.release_path, b"reset\n")
            .expect("the harness release marker should be written");
        waiter
            .await
            .expect("barrier task should not panic")
            .expect("explicit release should unblock the write");

        tokio::time::timeout(
            Duration::from_millis(100),
            await_configured_lifecycle_write_barrier(barrier),
        )
        .await
        .expect("a semantic retry must bypass the one-shot barrier")
        .expect("a semantic retry must remain valid");
        std::fs::remove_dir_all(root).expect("barrier test directory should be removed");
    }

    #[tokio::test]
    async fn cli_connected_session_rejects_inbound_line_over_max_bytes() {
        let input = vec![b'a'; MAX_INBOUND_PROTOCOL_LINE_BYTES + 1];
        let mut reader = BufReader::new(&input[..]);

        let error = read_one_protocol_line_for_test(&mut reader)
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

        let line = read_one_protocol_line_for_test(&mut reader)
            .await
            .expect("batched line read should succeed")
            .expect("batched line should be present");

        assert_eq!(line.as_bytes(), input);
        let items = decode_message_line_items(&line).expect("batched line should decode");
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn cli_accepts_exact_limit_payload_when_crlf_is_split_between_reads() {
        let payload = vec![b'a'; MAX_INBOUND_PROTOCOL_LINE_BYTES];
        let mut framed = payload.clone();
        framed.extend_from_slice(b"\r\n");
        let mut reader = BufReader::with_capacity(MAX_INBOUND_PROTOCOL_LINE_BYTES + 1, &framed[..]);

        let line = read_one_protocol_line_for_test(&mut reader)
            .await
            .expect("exact-limit split CRLF line should be accepted")
            .expect("exact-limit line should be present");

        assert_eq!(line.as_bytes(), payload);
    }

    #[tokio::test]
    async fn write_protocol_line_uses_crlf_framing() {
        let mut output = Vec::new();

        write_protocol_line(&mut output, r#"{"List":null}"#)
            .await
            .expect("protocol line should write");

        assert_eq!(output, b"{\"List\":null}\r\n");
    }

    #[test]
    fn production_connected_session_has_no_reusable_one_shot_reader_wrapper() {
        const CONNECTED_SESSION_SOURCE: &str = include_str!("session_runner/connected_session.rs");
        assert!(CONNECTED_SESSION_SOURCE.contains("InboundProtocolLineReader::default()"));
        assert!(
            !CONNECTED_SESSION_SOURCE.contains("read_inbound_protocol_line"),
            "reusable connection paths must own InboundProtocolLineReader state"
        );
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

    #[tokio::test]
    async fn blocked_protocol_write_maintains_player_integrations_until_acknowledged() {
        let maintenance_calls = Arc::new(AtomicUsize::new(0));
        let mut control = QueuedRuntimeControl::default();
        control
            .emit(ClientEffect::SendChat("x".repeat(512)))
            .expect("chat effect should be supported");
        let runtime = ClientRuntime::new(
            ClientSession::default(),
            MaintainingProtocolIoTestPlayer(Arc::clone(&maintenance_calls)),
            control,
        );
        let mut runtime = ClientApplication::from_runtime(runtime);
        let (mut reader, mut writer) = tokio::io::duplex(1);
        let reader_maintenance_calls = Arc::clone(&maintenance_calls);
        let drain = tokio::spawn(async move {
            while reader_maintenance_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            let mut received = Vec::new();
            let mut byte = [0_u8; 1];
            while !received.ends_with(b"\r\n") {
                reader
                    .read_exact(&mut byte)
                    .await
                    .expect("blocked protocol writer should remain readable");
                received.push(byte[0]);
            }
            received
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            flush_runtime_protocol_lines(&mut runtime, &mut writer),
        )
        .await
        .expect("maintenance should unblock the delayed reader")
        .expect("delayed transport reader should eventually accept the line");

        assert_eq!(runtime.pending_protocol_message_count(), 0);
        assert!(
            maintenance_calls.load(Ordering::SeqCst) >= 1,
            "maintenance must run before the blocked writer is allowed to continue"
        );
        let received = drain.await.expect("reader task should finish");
        assert!(received.ends_with(b"\r\n"));
    }

    #[tokio::test]
    async fn startup_protocol_write_deadline_releases_the_pending_frame_for_reconnect() {
        let mut control = QueuedRuntimeControl::default();
        control
            .emit(ClientEffect::SendChat("x".repeat(512)))
            .expect("chat effect should be supported");
        let runtime = ClientRuntime::new(ClientSession::default(), ProtocolIoTestPlayer, control);
        let mut runtime = ClientApplication::from_runtime(runtime);
        let (_reader, mut writer) = tokio::io::duplex(1);

        let completed = flush_runtime_protocol_lines_until(
            &mut runtime,
            &mut writer,
            Instant::now() + std::time::Duration::from_millis(25),
        )
        .await
        .expect("deadline should stop a blocked startup write cleanly");

        assert!(!completed);
        assert_eq!(
            runtime.pending_protocol_message_count(),
            1,
            "the canceled startup frame must remain queued for reconnect"
        );
        assert!(
            runtime
                .pending_protocol_line()
                .expect("pending frame should remain leasable")
                .is_some(),
            "deadline cancellation must release the frame lease"
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
