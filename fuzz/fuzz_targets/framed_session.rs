#![no_main]

use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use sorotte_cli::fuzz_support::{InboundProtocolLineReader, MAX_INBOUND_PROTOCOL_LINE_BYTES};
use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_player_api::PlayerAdapter;
use tokio::io::{AsyncBufRead, AsyncRead, ReadBuf};
use tokio::sync::Notify;

const CONTROL_BYTES: usize = 4;
const MAX_FRAMES: usize = 64;
const SIZE_SEAM_PREFIX: &[u8] = b"!SEAM";

thread_local! {
    static FUZZ_RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the in-memory framed-session fuzz runtime must initialize");
}

struct FramedSessionFuzzPlayer;

impl PlayerAdapter for FramedSessionFuzzPlayer {
    fn name(&self) -> &'static str {
        "framed-session-fuzz-player"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameFailure {
    InvalidUtf8,
    TooLong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReferenceFrame {
    line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FramingOutcome {
    lines: Vec<String>,
    failure: Option<FrameFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReferenceOutcome {
    frames: Vec<ReferenceFrame>,
    failure: Option<FrameFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplicationTrace {
    accepted: Vec<bool>,
    session_states: Vec<String>,
    pending_protocol_counts: Vec<usize>,
}

#[derive(Debug)]
struct CancellationGate {
    pause_at: usize,
    reached: AtomicBool,
    released: AtomicBool,
    notification: Notify,
}

impl CancellationGate {
    fn new(pause_at: usize) -> Self {
        Self {
            pause_at,
            reached: AtomicBool::new(false),
            released: AtomicBool::new(false),
            notification: Notify::new(),
        }
    }

    fn observe_pause(&self) {
        if !self.reached.swap(true, Ordering::SeqCst) {
            self.notification.notify_one();
        }
    }

    fn release(&self) {
        assert!(
            self.reached.load(Ordering::SeqCst),
            "the cancellation gate must be reached before release"
        );
        self.released.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct ScheduledReader {
    bytes: Vec<u8>,
    chunk_ends: Vec<usize>,
    position: usize,
    chunk_index: usize,
    gate: Option<Arc<CancellationGate>>,
}

impl ScheduledReader {
    fn new(bytes: Vec<u8>, chunk_lengths: Vec<usize>, cancellation_offset: Option<usize>) -> Self {
        let mut total = 0_usize;
        let chunk_ends = chunk_lengths
            .into_iter()
            .map(|length| {
                assert_ne!(length, 0, "scheduled fuzz chunks must not be empty");
                total = total
                    .checked_add(length)
                    .expect("bounded fuzz chunk lengths cannot overflow");
                total
            })
            .collect::<Vec<_>>();
        assert_eq!(
            total,
            bytes.len(),
            "scheduled fuzz chunks must cover the exact transport input"
        );
        let gate = cancellation_offset.map(|offset| {
            assert!(
                (1..=bytes.len()).contains(&offset),
                "cancellation must follow at least one consumed byte"
            );
            Arc::new(CancellationGate::new(offset))
        });
        Self {
            bytes,
            chunk_ends,
            position: 0,
            chunk_index: 0,
            gate,
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
        let scheduled_end = self
            .chunk_ends
            .get(self.chunk_index)
            .copied()
            .unwrap_or(self.bytes.len());
        match &self.gate {
            Some(gate)
                if !gate.released.load(Ordering::SeqCst) && self.position < gate.pause_at =>
            {
                scheduled_end.min(gate.pause_at)
            }
            _ => scheduled_end,
        }
    }

    fn cancellation_gate(&self) -> Option<Arc<CancellationGate>> {
        self.gate.clone()
    }
}

impl AsyncRead for ScheduledReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.as_mut().get_mut();
        this.advance_consumed_chunks();
        if let Some(gate) = &this.gate
            && !gate.released.load(Ordering::SeqCst)
            && this.position == gate.pause_at
        {
            gate.observe_pause();
            return Poll::Pending;
        }
        let available = this.current_chunk_end().saturating_sub(this.position);
        let read_len = available.min(buffer.remaining());
        buffer.put_slice(&this.bytes[this.position..this.position + read_len]);
        this.position += read_len;
        Poll::Ready(Ok(()))
    }
}

impl AsyncBufRead for ScheduledReader {
    fn poll_fill_buf(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        this.advance_consumed_chunks();
        if let Some(gate) = &this.gate
            && !gate.released.load(Ordering::SeqCst)
            && this.position == gate.pause_at
        {
            gate.observe_pause();
            return Poll::Pending;
        }
        let chunk_end = this.current_chunk_end();
        Poll::Ready(Ok(&this.bytes[this.position..chunk_end]))
    }

    fn consume(mut self: Pin<&mut Self>, amount: usize) {
        let this = self.as_mut().get_mut();
        let available = this.current_chunk_end().saturating_sub(this.position);
        assert!(
            amount <= available,
            "the production reader advanced beyond a scheduled fuzz chunk"
        );
        this.position += amount;
    }
}

fn reference_frame(raw: &[u8]) -> Result<ReferenceFrame, FrameFailure> {
    let payload = raw.strip_suffix(b"\r").unwrap_or(raw);
    if payload.len() > MAX_INBOUND_PROTOCOL_LINE_BYTES {
        return Err(FrameFailure::TooLong);
    }
    let line = String::from_utf8(payload.to_vec()).map_err(|_| FrameFailure::InvalidUtf8)?;
    Ok(ReferenceFrame { line })
}

fn reference_outcome(wire: &[u8]) -> ReferenceOutcome {
    let mut frames = Vec::new();
    let mut frame_start = 0_usize;
    for (index, byte) in wire.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        match reference_frame(&wire[frame_start..index]) {
            Ok(frame) => frames.push(frame),
            Err(failure) => {
                return ReferenceOutcome {
                    frames,
                    failure: Some(failure),
                };
            }
        }
        frame_start = index + 1;
    }
    if frame_start < wire.len() {
        match reference_frame(&wire[frame_start..]) {
            Ok(frame) => frames.push(frame),
            Err(failure) => {
                return ReferenceOutcome {
                    frames,
                    failure: Some(failure),
                };
            }
        }
    }
    ReferenceOutcome {
        frames,
        failure: None,
    }
}

fn classify_production_failure(error: &anyhow::Error) -> FrameFailure {
    if error.to_string().contains("Inbound protocol line too long") {
        FrameFailure::TooLong
    } else if error.downcast_ref::<std::string::FromUtf8Error>().is_some() {
        FrameFailure::InvalidUtf8
    } else {
        panic!("unexpected in-memory framing failure: {error:#}");
    }
}

async fn read_with_one_cancellation(
    line_reader: &mut InboundProtocolLineReader,
    reader: &mut ScheduledReader,
    gate: Arc<CancellationGate>,
) -> anyhow::Result<Option<String>> {
    enum ReadOutcome {
        Completed(anyhow::Result<Option<String>>),
        Cancelled,
    }

    let outcome = {
        let reached = gate.notification.notified();
        let read = line_reader.read_line(reader);
        tokio::pin!(reached);
        tokio::pin!(read);
        tokio::select! {
            result = &mut read => ReadOutcome::Completed(result),
            () = &mut reached => ReadOutcome::Cancelled,
        }
    };
    match outcome {
        ReadOutcome::Completed(result) => result,
        ReadOutcome::Cancelled => {
            gate.release();
            line_reader.read_line(reader).await
        }
    }
}

async fn production_outcome(
    wire: &[u8],
    chunks: Vec<usize>,
    cancellation_offset: Option<usize>,
    expected_frame_count: usize,
) -> FramingOutcome {
    let mut reader = ScheduledReader::new(wire.to_vec(), chunks, cancellation_offset);
    let cancellation_gate = reader.cancellation_gate();
    let mut line_reader = InboundProtocolLineReader::default();
    let mut lines = Vec::with_capacity(expected_frame_count);
    for frame_index in 0..expected_frame_count {
        let result = if frame_index == 0 {
            match cancellation_gate.clone() {
                Some(gate) => read_with_one_cancellation(&mut line_reader, &mut reader, gate).await,
                None => line_reader.read_line(&mut reader).await,
            }
        } else {
            line_reader.read_line(&mut reader).await
        };
        match result {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => panic!(
                "production framing reached EOF after {frame_index} of \
                 {expected_frame_count} input-derived frames"
            ),
            Err(error) => {
                return FramingOutcome {
                    lines,
                    failure: Some(classify_production_failure(&error)),
                };
            }
        }
    }
    match line_reader.read_line(&mut reader).await {
        Ok(None) => FramingOutcome {
            lines,
            failure: None,
        },
        Ok(Some(extra)) => panic!(
            "production framing exceeded the input-derived frame bound with \
             an extra {}-byte frame",
            extra.len()
        ),
        Err(error) => FramingOutcome {
            lines,
            failure: Some(classify_production_failure(&error)),
        },
    }
}

fn scheduled_chunks(total: usize, controls: &[u8; CONTROL_BYTES]) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    match controls[0] % 4 {
        0 => vec![total],
        1 => vec![1; total],
        2 => {
            let width = 1 + usize::from(controls[1] % 64);
            let mut remaining = total;
            let mut chunks = Vec::new();
            while remaining != 0 {
                let take = remaining.min(width);
                chunks.push(take);
                remaining -= take;
            }
            chunks
        }
        _ => {
            let mut state = u64::from_le_bytes([
                controls[0],
                controls[1],
                controls[2],
                controls[3],
                controls[3],
                controls[2],
                controls[1],
                controls[0],
            ]);
            let mut remaining = total;
            let mut chunks = Vec::new();
            while remaining != 0 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let width = 1 + ((state >> 32) as usize % 64);
                let take = remaining.min(width);
                chunks.push(take);
                remaining -= take;
            }
            chunks
        }
    }
}

fn cancellation_offset(wire: &[u8], control: u8) -> Option<usize> {
    if control & 1 == 0 || wire.is_empty() {
        return None;
    }
    let first_frame_prefix = wire
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(wire.len());
    if first_frame_prefix == 0 {
        return None;
    }
    Some(1 + (usize::from(control) % first_frame_prefix))
}

fn input_frame_count(wire: &[u8]) -> usize {
    wire.iter().filter(|byte| **byte == b'\n').count()
        + usize::from(wire.last().is_some_and(|byte| *byte != b'\n'))
}

fn session_state(runtime: &ClientApplication<FramedSessionFuzzPlayer>) -> String {
    format!("{:?}", runtime.session().model())
}

fn assert_session_invariants(runtime: &ClientApplication<FramedSessionFuzzPlayer>) {
    let session = runtime.session();
    if session.is_active() {
        assert!(
            session.username().is_some() && session.room().is_some(),
            "an active fuzzed session must retain both server identity fields"
        );
    }
    for room in session.room_names() {
        for username in session.usernames_in_room(&room) {
            assert_eq!(
                session.user_room(&username),
                Some(room.as_str()),
                "the public room and user projections must agree"
            );
        }
    }
}

fn apply_session_lines(frames: &[ReferenceFrame]) -> ApplicationTrace {
    let mut runtime = ClientApplication::with_default_session(FramedSessionFuzzPlayer);
    let mut accepted = Vec::new();
    let mut session_states = Vec::new();
    let mut pending_protocol_counts = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let before = session_state(&runtime);
        let syntactically_valid = serde_json::from_str::<Value>(&frame.line).is_ok();
        let result =
            runtime.apply_protocol_line(&frame.line, index as f64 + 1.0, false, false, false);
        if !syntactically_valid {
            assert!(
                result.is_err(),
                "invalid JSON in a complete frame must fail closed"
            );
            assert_eq!(
                session_state(&runtime),
                before,
                "invalid JSON must not partially mutate session state"
            );
        }
        accepted.push(result.is_ok());
        assert_session_invariants(&runtime);
        session_states.push(session_state(&runtime));
        pending_protocol_counts.push(runtime.pending_protocol_message_count());
    }
    ApplicationTrace {
        accepted,
        session_states,
        pending_protocol_counts,
    }
}

async fn exercise_wire(wire: &[u8], controls: &[u8; CONTROL_BYTES]) {
    let expected_frame_count = input_frame_count(wire);
    if expected_frame_count > MAX_FRAMES {
        return;
    }

    let reference = reference_outcome(wire);
    let coalesced = production_outcome(
        wire,
        if wire.is_empty() {
            Vec::new()
        } else {
            vec![wire.len()]
        },
        None,
        expected_frame_count,
    )
    .await;
    let scheduled = production_outcome(
        wire,
        scheduled_chunks(wire.len(), controls),
        cancellation_offset(wire, controls[2]),
        expected_frame_count,
    )
    .await;
    let expected_lines = reference
        .frames
        .iter()
        .map(|frame| frame.line.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        coalesced,
        FramingOutcome {
            lines: expected_lines.clone(),
            failure: reference.failure,
        },
        "coalesced production framing must match the independent byte oracle"
    );
    assert_eq!(
        scheduled,
        FramingOutcome {
            lines: expected_lines,
            failure: reference.failure,
        },
        "fragmented/cancelled production framing must match the independent byte oracle"
    );

    let reference_trace = apply_session_lines(&reference.frames);
    let scheduled_frames = scheduled
        .lines
        .iter()
        .map(|line| ReferenceFrame { line: line.clone() })
        .collect::<Vec<_>>();
    assert_eq!(
        apply_session_lines(&scheduled_frames),
        reference_trace,
        "framing schedules must preserve real session application outcomes"
    );
}

async fn exercise_size_seam(selector: u8) {
    let (payload_len, crlf) = match selector % 4 {
        0 => (MAX_INBOUND_PROTOCOL_LINE_BYTES, false),
        1 => (MAX_INBOUND_PROTOCOL_LINE_BYTES, true),
        2 => (MAX_INBOUND_PROTOCOL_LINE_BYTES + 1, false),
        _ => (MAX_INBOUND_PROTOCOL_LINE_BYTES + 1, true),
    };
    let mut wire = vec![b'x'; payload_len];
    if crlf {
        wire.extend_from_slice(b"\r\n");
    } else {
        wire.push(b'\n');
    }
    exercise_wire(&wire, &[1, 0, 1, selector]).await;
}

fuzz_target!(|bytes: &[u8]| {
    FUZZ_RUNTIME.with(|runtime| {
        if let Some(selector) = bytes.strip_prefix(SIZE_SEAM_PREFIX) {
            if let Some(selector) = selector.first() {
                runtime.block_on(exercise_size_seam(*selector));
            }
            return;
        }
        if bytes.len() < CONTROL_BYTES {
            return;
        }
        let controls: &[u8; CONTROL_BYTES] = bytes[..CONTROL_BYTES]
            .try_into()
            .expect("the fuzz header length is checked");
        runtime.block_on(exercise_wire(&bytes[CONTROL_BYTES..], controls));
    });
});
