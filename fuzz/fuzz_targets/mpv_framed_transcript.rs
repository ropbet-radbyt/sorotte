#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};
use sorotte_player_api::{PlayerAttachmentEpoch, PlayerCommandId, SnapshotField};
use sorotte_player_mpv::{
    MpvLifecycleVerificationHarness,
    fuzz_support::{FuzzMpvIpcOutcome, FuzzMpvIpcScriptEnd, run_in_memory_mpv_ipc_fuzz_case},
    transcript::{MpvTranscript, MpvTranscriptRecord, MpvTranscriptReplaySink},
};

const HEADER_BYTES: usize = 4;
const MAX_DERIVED_FRAMES: usize = 64;
const MAX_TRANSCRIPT_RECORDS: usize = 16;
const REQUEST_ID: u64 = 1;

#[derive(Clone, Debug, PartialEq)]
enum ReferenceOutcome {
    Succeeded(Value),
    ServerRejected,
    TimedOut,
    Disconnected,
    ProtocolCorruption,
}

#[derive(Debug, PartialEq)]
struct ReferenceRun {
    outcome: ReferenceOutcome,
    queued_events: Vec<Value>,
}

#[derive(Clone)]
struct TranscriptSpec {
    attachment_epoch: u64,
    ingress_sequence: u64,
    receipt_tick: u64,
    command_id: Option<u64>,
    playlist_entry_id: Option<i64>,
    raw_json: Value,
}

#[derive(Default)]
struct ReplayProjection {
    records: Vec<ReplayRecord>,
}

#[derive(Debug, PartialEq)]
struct ReplayRecord {
    attachment_epoch: u64,
    ingress_sequence: u64,
    receipt_tick: u64,
    command_id: Option<u64>,
    playlist_entry_id: Option<i64>,
    raw_json_sha256: String,
}

impl MpvTranscriptReplaySink for ReplayProjection {
    fn consume_batch(&mut self, records: &[MpvTranscriptRecord]) {
        self.records
            .extend(records.iter().map(|record| ReplayRecord {
                attachment_epoch: record.attachment_epoch.get(),
                ingress_sequence: record.ingress_sequence,
                receipt_tick: record.monotonic_receipt_tick,
                command_id: record.command_id.map(PlayerCommandId::get),
                playlist_entry_id: record.playlist_entry_id,
                raw_json_sha256: record.raw_json_sha256(),
            }));
    }
}

fn bounded_payload(bytes: &[u8]) -> Vec<u8> {
    let payload = bytes.get(HEADER_BYTES..).unwrap_or_default();
    let mut newline_count = 0;
    for (index, byte) in payload.iter().enumerate() {
        if *byte == b'\n' {
            newline_count += 1;
            if newline_count == MAX_DERIVED_FRAMES {
                return payload[..=index].to_vec();
            }
        }
    }
    payload.to_vec()
}

fn scheduled_chunks(payload: &[u8], mode: u8, salt: u8) -> Vec<Vec<u8>> {
    if payload.is_empty() {
        return Vec::new();
    }
    match mode % 4 {
        0 => vec![payload.to_vec()],
        1 => payload.iter().map(|byte| vec![*byte]).collect(),
        2 => {
            let width = usize::from(salt % 31) + 1;
            payload.chunks(width).map(<[u8]>::to_vec).collect()
        }
        _ => {
            let mut state = u64::from(salt).wrapping_add(1);
            let mut chunks = Vec::new();
            let mut offset = 0;
            while offset < payload.len() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let width = usize::try_from(state % 47).expect("bounded chunk width") + 1;
                let end = offset.saturating_add(width).min(payload.len());
                chunks.push(payload[offset..end].to_vec());
                offset = end;
            }
            chunks
        }
    }
}

fn script_end(control: u8) -> FuzzMpvIpcScriptEnd {
    match control % 5 {
        0 => FuzzMpvIpcScriptEnd::Eof,
        1 => FuzzMpvIpcScriptEnd::ReadTimedOut,
        2 => FuzzMpvIpcScriptEnd::ReadDisconnected,
        3 => FuzzMpvIpcScriptEnd::WriteTimedOut,
        _ => FuzzMpvIpcScriptEnd::WriteDisconnected,
    }
}

fn reference_run(payload: &[u8], end: FuzzMpvIpcScriptEnd) -> ReferenceRun {
    match end {
        FuzzMpvIpcScriptEnd::WriteTimedOut => {
            return ReferenceRun {
                outcome: ReferenceOutcome::TimedOut,
                queued_events: Vec::new(),
            };
        }
        FuzzMpvIpcScriptEnd::WriteDisconnected => {
            return ReferenceRun {
                outcome: ReferenceOutcome::Disconnected,
                queued_events: Vec::new(),
            };
        }
        _ => {}
    }

    let mut queued_events = Vec::new();
    let mut offset = 0;
    while let Some(relative_newline) = payload[offset..].iter().position(|byte| *byte == b'\n') {
        let end_offset = offset + relative_newline + 1;
        if let Some(outcome) = reference_line(&payload[offset..end_offset], &mut queued_events) {
            return ReferenceRun {
                outcome,
                queued_events,
            };
        }
        offset = end_offset;
    }

    if offset < payload.len()
        && end == FuzzMpvIpcScriptEnd::Eof
        && let Some(outcome) = reference_line(&payload[offset..], &mut queued_events)
    {
        return ReferenceRun {
            outcome,
            queued_events,
        };
    }

    let outcome = match end {
        FuzzMpvIpcScriptEnd::Eof | FuzzMpvIpcScriptEnd::ReadDisconnected => {
            ReferenceOutcome::Disconnected
        }
        FuzzMpvIpcScriptEnd::ReadTimedOut => ReferenceOutcome::TimedOut,
        FuzzMpvIpcScriptEnd::WriteTimedOut | FuzzMpvIpcScriptEnd::WriteDisconnected => {
            unreachable!("write termination is classified before parsing")
        }
    };
    ReferenceRun {
        outcome,
        queued_events,
    }
}

fn reference_line(raw_line: &[u8], queued_events: &mut Vec<Value>) -> Option<ReferenceOutcome> {
    let Ok(decoded) = std::str::from_utf8(raw_line) else {
        // UTF-8 decoding is part of the transport reader. The worker classifies
        // every non-timeout read error as a disconnected transport, while valid
        // UTF-8 containing malformed JSON is protocol corruption.
        return Some(ReferenceOutcome::Disconnected);
    };
    let line = decoded.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(line) else {
        return Some(ReferenceOutcome::ProtocolCorruption);
    };
    if parsed.get("event").and_then(Value::as_str).is_some() {
        queued_events.push(parsed);
        return None;
    }
    let Some(request_id) = parsed.get("request_id").and_then(Value::as_u64) else {
        return Some(ReferenceOutcome::ProtocolCorruption);
    };
    if request_id != REQUEST_ID {
        return Some(ReferenceOutcome::ProtocolCorruption);
    }
    let Some(error) = parsed.get("error").and_then(Value::as_str) else {
        return Some(ReferenceOutcome::ProtocolCorruption);
    };
    if error == "success" {
        Some(ReferenceOutcome::Succeeded(parsed))
    } else {
        Some(ReferenceOutcome::ServerRejected)
    }
}

fn assert_worker_matches_reference(
    production_outcome: &FuzzMpvIpcOutcome,
    reference_outcome: &ReferenceOutcome,
) {
    match (production_outcome, reference_outcome) {
        (FuzzMpvIpcOutcome::Succeeded(production), ReferenceOutcome::Succeeded(reference)) => {
            assert_eq!(
                production, reference,
                "production success payload must match the independent response oracle"
            );
        }
        (FuzzMpvIpcOutcome::ServerRejected, ReferenceOutcome::ServerRejected)
        | (FuzzMpvIpcOutcome::TimedOut, ReferenceOutcome::TimedOut)
        | (FuzzMpvIpcOutcome::Disconnected, ReferenceOutcome::Disconnected)
        | (FuzzMpvIpcOutcome::ProtocolCorruption, ReferenceOutcome::ProtocolCorruption) => {}
        (FuzzMpvIpcOutcome::CommandFailed, _) => {
            panic!("the fixed serializable command must not fail before transport")
        }
        _ => panic!(
            "production outcome {production_outcome:?} disagrees with reference {reference_outcome:?}"
        ),
    }
}

fn transcript_specs(
    control: u8,
    queued_events: &[Value],
    outcome: &ReferenceOutcome,
) -> Vec<TranscriptSpec> {
    let mut values = queued_events
        .iter()
        .take(MAX_TRANSCRIPT_RECORDS)
        .cloned()
        .collect::<Vec<_>>();
    if let ReferenceOutcome::Succeeded(response) = outcome
        && values.len() < MAX_TRANSCRIPT_RECORDS
    {
        values.push(response.clone());
    }
    if values.is_empty() {
        values.push(json!({ "event": "idle" }));
    }

    let mut next_sequence = BTreeMap::<u64, u64>::new();
    let mut specs = values
        .into_iter()
        .enumerate()
        .map(|(index, raw_json)| {
            let epoch = u64::from((control.wrapping_add(index as u8) % 3) + 1);
            let sequence = next_sequence.entry(epoch).or_insert(0);
            *sequence += 1;
            TranscriptSpec {
                attachment_epoch: epoch,
                ingress_sequence: *sequence,
                receipt_tick: u64::try_from(index).expect("bounded transcript index") + 1,
                command_id: (index % 2 == 0)
                    .then_some(u64::try_from(index).expect("bounded transcript index") + 1),
                playlist_entry_id: (index % 3 == 0)
                    .then_some(i64::try_from(index).expect("bounded transcript index")),
                raw_json,
            }
        })
        .collect::<Vec<_>>();

    match control % 8 {
        0 | 6 => {}
        1 => {
            if specs.len() == 1 {
                specs[0].ingress_sequence = 0;
            } else {
                specs[1].attachment_epoch = specs[0].attachment_epoch;
                specs[1].ingress_sequence = specs[0].ingress_sequence;
            }
        }
        2 => {
            if specs.len() == 1 {
                specs[0].attachment_epoch = 0;
            } else {
                specs[1].receipt_tick = 0;
            }
        }
        3 => specs[0].attachment_epoch = 0,
        4 => specs[0].ingress_sequence = 0,
        5 => {
            if specs.len() == 1 {
                specs[0].raw_json = Value::String("not-an-object".to_owned());
            } else {
                specs.swap(0, 1);
            }
        }
        7 => specs[0].raw_json = Value::Array(Vec::new()),
        _ => unreachable!(),
    }
    specs
}

fn transcript_specs_are_valid(specs: &[TranscriptSpec]) -> bool {
    let mut prior_tick = None;
    let mut prior_sequence = BTreeMap::<u64, u64>::new();
    for spec in specs {
        if spec.attachment_epoch == 0
            || spec.ingress_sequence == 0
            || !spec.raw_json.is_object()
            || prior_tick.is_some_and(|tick| spec.receipt_tick < tick)
            || prior_sequence
                .get(&spec.attachment_epoch)
                .is_some_and(|sequence| spec.ingress_sequence <= *sequence)
        {
            return false;
        }
        prior_tick = Some(spec.receipt_tick);
        prior_sequence.insert(spec.attachment_epoch, spec.ingress_sequence);
    }
    true
}

fn assert_transcript_projection(control: u8, queued_events: &[Value], outcome: &ReferenceOutcome) {
    let specs = transcript_specs(control, queued_events, outcome);
    let expected_valid = transcript_specs_are_valid(&specs);
    let records = specs
        .iter()
        .map(|spec| {
            MpvTranscriptRecord::sanitized(
                PlayerAttachmentEpoch::new(spec.attachment_epoch),
                spec.ingress_sequence,
                spec.receipt_tick,
                spec.command_id.map(PlayerCommandId::new),
                spec.playlist_entry_id,
                spec.raw_json.clone(),
            )
        })
        .collect::<Vec<_>>();
    let transcript = MpvTranscript::new(records);
    assert_eq!(
        transcript.is_ok(),
        expected_valid,
        "transcript acceptance must match the independent attachment/order oracle"
    );
    let Ok(transcript) = transcript else {
        return;
    };

    let encoded = transcript
        .to_json_lines()
        .expect("validated transcript must serialize");
    let restored =
        MpvTranscript::from_json_lines(&encoded).expect("serialized transcript must parse");
    assert_eq!(
        transcript, restored,
        "transcript JSON-lines must round-trip exactly after sanitization"
    );

    let mut one_batch = ReplayProjection::default();
    transcript
        .replay_partitioned(&[transcript.len()], &mut one_batch)
        .expect("one complete replay batch must be valid");
    let mut individual = ReplayProjection::default();
    transcript
        .replay_partitioned(&vec![1; transcript.len()], &mut individual)
        .expect("one-record replay batches must be valid");
    assert_eq!(
        one_batch.records, individual.records,
        "replay partitions must not change attachment or ingress order"
    );
}

fn known_epoch(field: &SnapshotField<PlayerAttachmentEpoch>) -> PlayerAttachmentEpoch {
    match field {
        SnapshotField::Known(value) => *value,
        other => panic!("attachment epoch must be known, got {other:?}"),
    }
}

fn assert_lifecycle_fencing(control: u8) {
    let mut harness = MpvLifecycleVerificationHarness::new();
    let first_epoch = known_epoch(&harness.projection().attachment_epoch);
    let first = harness.accept_tracked_load("fuzz-generation-one", []);

    if control & 0b0000_0001 != 0 {
        harness.apply_authoritative_snapshot(
            [sorotte_player_mpv::LifecycleVerificationPlaylistEntry::new(
                11,
                Some("fuzz-generation-one".to_owned()),
                true,
            )],
            Some("fuzz-generation-one".to_owned()),
        );
    }
    let start = json!({ "event": "start-file", "playlist_entry_id": 11 });
    let loaded = json!({ "event": "file-loaded" });
    if control & 0b0000_0010 != 0 {
        harness.ingest_decoded_mpv_json(loaded.clone());
        harness.ingest_decoded_mpv_json(start.clone());
    } else {
        harness.ingest_decoded_mpv_json(start.clone());
        if control & 0b0000_0100 == 0 {
            harness.ingest_decoded_mpv_json(loaded.clone());
        }
    }
    if control & 0b0000_1000 != 0 {
        harness.ingest_decoded_mpv_json(start);
        harness.ingest_decoded_mpv_json(loaded);
    }
    if control & 0b0001_0000 != 0 {
        harness.advance_clock(60_000);
    }
    let before_replacement = harness.projection();
    assert!(
        before_replacement.attempts.contains_key(&first.attempt_id),
        "the current attachment may only reference its own first attempt"
    );
    assert_ne!(first.media_generation.get(), 0);

    harness.replace_attachment();
    let replacement = harness.projection();
    let replacement_epoch = known_epoch(&replacement.attachment_epoch);
    assert_ne!(
        replacement_epoch, first_epoch,
        "attachment replacement must allocate a new epoch"
    );
    assert!(
        !replacement.attempts.contains_key(&first.attempt_id),
        "attachment replacement must fence the prior attempt"
    );
    assert_eq!(
        replacement.physical_media_generation,
        SnapshotField::KnownAbsent,
        "attachment replacement must clear the prior physical generation"
    );

    let second = harness.accept_tracked_load("fuzz-generation-two", []);
    let second_projection = harness.projection();
    assert_ne!(
        second.media_generation, first.media_generation,
        "successive logical loads must use distinct media generations"
    );
    assert!(
        !second_projection.attempts.contains_key(&first.attempt_id),
        "a new attachment must not resurrect a prior attempt"
    );
    assert!(
        second_projection.attempts.contains_key(&second.attempt_id),
        "the replacement attachment must retain its own attempt"
    );
}

fuzz_target!(|bytes: &[u8]| {
    let controls = [
        bytes.first().copied().unwrap_or_default(),
        bytes.get(1).copied().unwrap_or_default(),
        bytes.get(2).copied().unwrap_or_default(),
        bytes.get(3).copied().unwrap_or_default(),
    ];
    let payload = bounded_payload(bytes);
    let end = script_end(controls[1]);
    let chunks = scheduled_chunks(&payload, controls[0], controls[3]);
    let reference = reference_run(&payload, end);
    let production = run_in_memory_mpv_ipc_fuzz_case(chunks, end, REQUEST_ID);

    assert_eq!(
        production.request_lines.len(),
        1,
        "the fixed command must produce one attempted request"
    );
    let request: Value = serde_json::from_str(production.request_lines[0].trim_end())
        .expect("the production fixed command must serialize as JSON");
    assert_eq!(
        request,
        json!({
            "command": ["get_property", "pause"],
            "request_id": REQUEST_ID,
        }),
        "the feature-gated seam must issue only its fixed command"
    );
    assert_worker_matches_reference(&production.outcome, &reference.outcome);
    assert_eq!(
        production.queued_events, reference.queued_events,
        "event ordering, duplication, and response barriers must match the independent oracle"
    );
    assert_transcript_projection(controls[2], &production.queued_events, &reference.outcome);
    assert_lifecycle_fencing(controls[3]);
});
