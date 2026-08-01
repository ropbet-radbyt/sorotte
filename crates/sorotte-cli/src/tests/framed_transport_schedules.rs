use super::*;
use crate::protocol_io::{
    InboundProtocolLineReader, InboundProtocolReadObservation, MAX_INBOUND_PROTOCOL_LINE_BYTES,
    observe_inbound_protocol_reads,
};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWriteExt, ReadBuf};

const SCHEDULE_TIMEOUT: Duration = Duration::from_secs(1);
const WIRE_USERNAME: &str = "schedule-user";
const WIRE_ROOM: &str = "schedule-room";
const SERVER_HELLO: &[u8] = br#"{"Hello":{"username":"schedule-user","room":{"name":"schedule-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#;
const SERVER_READY: &[u8] =
    br#"{"Set":{"ready":{"isReady":true,"username":"schedule-user","manuallyInitiated":false}}}"#;

struct FramedTransportTestPlayer;

impl PlayerAdapter for FramedTransportTestPlayer {
    fn name(&self) -> &'static str {
        "framed-transport-test-player"
    }
}

#[derive(Debug)]
struct ScheduledReader {
    bytes: Vec<u8>,
    chunk_ends: Vec<usize>,
    position: usize,
    chunk_index: usize,
}

impl ScheduledReader {
    fn new(bytes: Vec<u8>, chunk_lengths: Vec<usize>) -> Self {
        let mut total = 0;
        let chunk_ends = chunk_lengths
            .into_iter()
            .map(|length| {
                assert_ne!(length, 0, "scheduled chunks must not be empty");
                total += length;
                total
            })
            .collect::<Vec<_>>();
        assert_eq!(
            total,
            bytes.len(),
            "scheduled chunks must cover the complete transport input"
        );
        Self {
            bytes,
            chunk_ends,
            position: 0,
            chunk_index: 0,
        }
    }

    fn advance_consumed_chunks(&mut self) {
        while self
            .chunk_ends
            .get(self.chunk_index)
            .is_some_and(|end| self.position >= *end)
        {
            self.chunk_index += 1;
        }
    }

    fn current_chunk_end(&self) -> usize {
        self.chunk_ends
            .get(self.chunk_index)
            .copied()
            .unwrap_or(self.bytes.len())
    }
}

impl AsyncRead for ScheduledReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        this.advance_consumed_chunks();
        let available = this.current_chunk_end().saturating_sub(this.position);
        let read_len = available.min(buffer.remaining());
        buffer.put_slice(&this.bytes[this.position..this.position + read_len]);
        this.position += read_len;
        Poll::Ready(Ok(()))
    }
}

impl AsyncBufRead for ScheduledReader {
    fn poll_fill_buf(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.get_mut();
        this.advance_consumed_chunks();
        let chunk_end = this.current_chunk_end();
        Poll::Ready(Ok(&this.bytes[this.position..chunk_end]))
    }

    fn consume(mut self: Pin<&mut Self>, amount: usize) {
        let this = self.as_mut().get_mut();
        let available = this.current_chunk_end().saturating_sub(this.position);
        assert!(
            amount <= available,
            "consumer advanced beyond the scheduled chunk boundary"
        );
        this.position += amount;
    }
}

fn crlf_framed(payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(payload.len() + 2);
    framed.extend_from_slice(payload);
    framed.extend_from_slice(b"\r\n");
    framed
}

fn coalesced_server_frames() -> Vec<u8> {
    let mut batch = crlf_framed(SERVER_HELLO);
    batch.extend_from_slice(&crlf_framed(SERVER_READY));
    batch
}

fn fixed_width_chunks(total: usize, width: usize) -> Vec<usize> {
    assert_ne!(width, 0);
    let mut remaining = total;
    let mut chunks = Vec::new();
    while remaining != 0 {
        let take = width.min(remaining);
        chunks.push(take);
        remaining -= take;
    }
    chunks
}

fn generated_chunks(total: usize, seed: u64) -> Vec<usize> {
    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut remaining = total;
    let mut chunks = Vec::new();
    while remaining != 0 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let width = 1 + ((state >> 32) as usize % 31);
        let take = width.min(remaining);
        chunks.push(take);
        remaining -= take;
    }
    chunks
}

fn chunk_schedule_matrix(total: usize) -> Vec<Vec<usize>> {
    let mut schedules = vec![vec![total], vec![1; total]];
    schedules.extend((2..=17).map(|width| fixed_width_chunks(total, width)));
    schedules.extend((0..64).map(|seed| generated_chunks(total, seed)));
    schedules
}

fn input_frame_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
        + usize::from(bytes.last().is_some_and(|byte| *byte != b'\n'))
}

async fn read_bounded_lines<R>(
    line_reader: &mut InboundProtocolLineReader,
    transport: &mut R,
    expected_frame_count: usize,
) -> anyhow::Result<Vec<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut lines = Vec::with_capacity(expected_frame_count);
    for frame_index in 0..expected_frame_count {
        let Some(line) = line_reader.read_line(transport).await? else {
            anyhow::bail!(
                "framed reader reached EOF after {frame_index} of {expected_frame_count} \
                 input-derived frames"
            );
        };
        lines.push(line);
    }
    if let Some(extra_line) = line_reader.read_line(transport).await? {
        anyhow::bail!(
            "framed reader exceeded the {expected_frame_count}-frame input-derived bound \
             with an extra {}-byte frame",
            extra_line.len()
        );
    }
    Ok(lines)
}

async fn read_all_scheduled_lines(
    bytes: Vec<u8>,
    chunk_lengths: Vec<usize>,
) -> anyhow::Result<Vec<String>> {
    let expected_frame_count = input_frame_count(&bytes);
    let mut transport = ScheduledReader::new(bytes, chunk_lengths);
    let mut line_reader = InboundProtocolLineReader::default();
    read_bounded_lines(&mut line_reader, &mut transport, expected_frame_count).await
}

fn apply_session_lines(
    lines: &[String],
) -> anyhow::Result<ClientApplication<FramedTransportTestPlayer>> {
    let mut runtime = ClientApplication::with_default_session(FramedTransportTestPlayer);
    for (index, line) in lines.iter().enumerate() {
        runtime.apply_protocol_line(line, index as f64 + 1.0, false, false, false)?;
    }
    Ok(runtime)
}

fn assert_complete_session(runtime: &ClientApplication<FramedTransportTestPlayer>) {
    assert_eq!(runtime.session().username(), Some(WIRE_USERNAME));
    assert_eq!(runtime.session().room(), Some(WIRE_ROOM));
    assert!(runtime.session().is_active());
    assert_eq!(runtime.session().user_ready(WIRE_USERNAME), Some(true));
}

async fn read_after_cancelling_at(batch: &[u8], cancellation_offset: usize) -> Vec<String> {
    let (mut writer, reader) = tokio::io::duplex(batch.len() + 1);
    writer
        .write_all(&batch[..cancellation_offset])
        .await
        .expect("the cancellation prefix should fit in the in-memory transport");
    writer
        .flush()
        .await
        .expect("the cancellation prefix should flush");

    let mut transport = BufReader::new(reader);
    let mut line_reader = InboundProtocolLineReader::default();
    let (observation_tx, mut observation_rx) = unbounded_channel();
    {
        let pending_read =
            observe_inbound_protocol_reads(observation_tx, line_reader.read_line(&mut transport));
        tokio::pin!(pending_read);
        tokio::select! {
            observation = observation_rx.recv() => {
                assert_eq!(
                    observation,
                    Some(InboundProtocolReadObservation::ConsumedPartial(
                        cancellation_offset
                    ))
                );
            }
            completed = &mut pending_read => {
                panic!(
                    "read completed before cancellation at byte {cancellation_offset}: \
                     {completed:?}"
                );
            }
        }
    }

    assert_eq!(
        tokio::time::timeout(SCHEDULE_TIMEOUT, observation_rx.recv())
            .await
            .expect("dropping the pending read should synchronously report cancellation"),
        Some(InboundProtocolReadObservation::CancelledPartial(
            cancellation_offset
        ))
    );

    writer
        .write_all(&batch[cancellation_offset..])
        .await
        .expect("the remaining coalesced frames should write after cancellation");
    writer
        .shutdown()
        .await
        .expect("the in-memory server should half-close");

    read_bounded_lines(&mut line_reader, &mut transport, input_frame_count(batch))
        .await
        .expect("the resumed framed read should produce exactly the input-derived frame count")
}

#[tokio::test]
async fn generated_fragmentation_and_coalescing_preserve_session_semantics() {
    let batch = coalesced_server_frames();
    let schedules = chunk_schedule_matrix(batch.len());
    assert_eq!(schedules.len(), 82);

    for (schedule_index, chunks) in schedules.into_iter().enumerate() {
        let lines = read_all_scheduled_lines(batch.clone(), chunks)
            .await
            .unwrap_or_else(|error| {
                panic!("chunk schedule {schedule_index} should remain readable: {error:#}")
            });
        assert_eq!(
            lines,
            vec![
                String::from_utf8(SERVER_HELLO.to_vec()).expect("Hello fixture should be UTF-8"),
                String::from_utf8(SERVER_READY.to_vec()).expect("Ready fixture should be UTF-8"),
            ],
            "chunk schedule {schedule_index} changed frame boundaries"
        );
        let runtime = apply_session_lines(&lines).unwrap_or_else(|error| {
            panic!("chunk schedule {schedule_index} should apply: {error:#}")
        });
        assert_complete_session(&runtime);
    }
}

#[tokio::test]
async fn split_lf_and_crlf_payload_limits_use_exact_accumulated_length() {
    let exact_payload = vec![b'x'; MAX_INBOUND_PROTOCOL_LINE_BYTES];
    let over_limit_payload = vec![b'x'; MAX_INBOUND_PROTOCOL_LINE_BYTES + 1];

    let mut exact_lf = exact_payload.clone();
    exact_lf.push(b'\n');
    let exact_lf_lines =
        read_all_scheduled_lines(exact_lf, vec![MAX_INBOUND_PROTOCOL_LINE_BYTES - 1, 2])
            .await
            .expect(
                "an exact-limit LF frame split before its last payload byte should be accepted",
            );
    assert_eq!(exact_lf_lines.len(), 1);
    assert_eq!(exact_lf_lines[0].as_bytes(), exact_payload);

    let exact_crlf = crlf_framed(&exact_payload);
    let exact_crlf_len = exact_crlf.len();
    let exact_crlf_lines = read_all_scheduled_lines(exact_crlf, vec![exact_crlf_len])
        .await
        .expect("an exact-limit CRLF frame in one chunk should exclude its framing CR");
    assert_eq!(exact_crlf_lines.len(), 1);
    assert_eq!(exact_crlf_lines[0].as_bytes(), exact_payload);

    let mut over_limit_lf = over_limit_payload.clone();
    over_limit_lf.push(b'\n');
    let over_limit_lf_error =
        read_all_scheduled_lines(over_limit_lf, vec![MAX_INBOUND_PROTOCOL_LINE_BYTES, 2])
            .await
            .expect_err("a split MAX+1 LF payload must not lose its last non-CR byte");
    assert!(
        over_limit_lf_error
            .to_string()
            .contains("Inbound protocol line too long"),
        "MAX+1 LF rejection should identify the framing limit: {over_limit_lf_error:#}"
    );

    let over_limit_crlf = crlf_framed(&over_limit_payload);
    let over_limit_crlf_len = over_limit_crlf.len();
    let over_limit_crlf_error =
        read_all_scheduled_lines(over_limit_crlf, vec![1, over_limit_crlf_len - 1])
            .await
            .expect_err(
                "a MAX+1 CRLF payload split after one byte must use additive accumulated length",
            );
    assert!(
        over_limit_crlf_error
            .to_string()
            .contains("Inbound protocol line too long"),
        "MAX+1 CRLF rejection should identify the framing limit: {over_limit_crlf_error:#}"
    );
}

#[tokio::test]
async fn every_first_frame_cancellation_point_preserves_coalesced_session_semantics() {
    let batch = coalesced_server_frames();
    let first_frame_len = SERVER_HELLO.len() + 2;

    for cancellation_offset in 1..first_frame_len {
        let lines = read_after_cancelling_at(&batch, cancellation_offset).await;
        assert_eq!(
            lines,
            vec![
                String::from_utf8(SERVER_HELLO.to_vec()).expect("Hello fixture should be UTF-8"),
                String::from_utf8(SERVER_READY.to_vec()).expect("Ready fixture should be UTF-8"),
            ],
            "cancellation at byte {cancellation_offset} lost, duplicated, or joined a frame"
        );
        let runtime = apply_session_lines(&lines).unwrap_or_else(|error| {
            panic!("cancellation offset {cancellation_offset} should apply: {error:#}")
        });
        assert_complete_session(&runtime);
    }
}

#[tokio::test]
async fn generated_truncation_and_eof_schedules_commit_only_complete_messages() {
    let hello_frame = crlf_framed(SERVER_HELLO);

    for truncation_offset in 1..SERVER_READY.len() {
        let mut batch = hello_frame.clone();
        batch.extend_from_slice(&SERVER_READY[..truncation_offset]);
        for chunks in [
            fixed_width_chunks(batch.len(), 1),
            generated_chunks(batch.len(), truncation_offset as u64),
        ] {
            let lines = read_all_scheduled_lines(batch.clone(), chunks)
                .await
                .expect("valid UTF-8 truncation should remain a readable final frame");
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].as_bytes(), SERVER_HELLO);
            assert_eq!(lines[1].as_bytes(), &SERVER_READY[..truncation_offset]);

            let mut runtime =
                apply_session_lines(&lines[..1]).expect("the complete Hello prefix should apply");
            runtime
                .apply_protocol_line(&lines[1], 2.0, false, false, false)
                .expect_err("a truncated Ready message should fail JSON decoding");
            assert_eq!(runtime.session().username(), Some(WIRE_USERNAME));
            assert_eq!(runtime.session().room(), Some(WIRE_ROOM));
            assert!(runtime.session().is_active());
            assert_ne!(runtime.session().user_ready(WIRE_USERNAME), Some(true));
        }
    }

    let eof_variants = [
        SERVER_READY.to_vec(),
        [SERVER_READY, b"\r"].concat(),
        crlf_framed(SERVER_READY),
    ];
    for (variant_index, suffix) in eof_variants.into_iter().enumerate() {
        let mut batch = hello_frame.clone();
        batch.extend_from_slice(&suffix);
        for seed in 0..16 {
            let lines =
                read_all_scheduled_lines(batch.clone(), generated_chunks(batch.len(), seed))
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "EOF variant {variant_index}, schedule {seed} should read: {error:#}"
                        )
                    });
            let runtime = apply_session_lines(&lines).unwrap_or_else(|error| {
                panic!("EOF variant {variant_index}, schedule {seed} should apply: {error:#}")
            });
            assert_complete_session(&runtime);
        }
    }

    let mut empty = ScheduledReader::new(Vec::new(), Vec::new());
    let mut line_reader = InboundProtocolLineReader::default();
    assert_eq!(
        line_reader
            .read_line(&mut empty)
            .await
            .expect("empty EOF should be readable"),
        None
    );
    assert_eq!(
        line_reader
            .read_line(&mut empty)
            .await
            .expect("repeated empty EOF should remain readable"),
        None
    );
}
