//! Sanitized mpv IPC transcript capture and deterministic replay.
//!
//! Transcripts retain the physical attachment and ingress identities needed to
//! reproduce lifecycle failures without retaining credentials or private media
//! locations. Callers may replay the same records with arbitrary pump
//! partitions; the record order never depends on those partitions.

use std::{
    error::Error,
    fmt::{self, Write as _},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sorotte_player_api::{PlayerAttachmentEpoch, PlayerCommandId};

const REDACTED: &str = "<redacted>";

/// One sanitized raw mpv JSON IPC item at the earliest reliable ingress
/// boundary.
#[derive(Clone, PartialEq)]
pub struct MpvTranscriptRecord {
    pub attachment_epoch: PlayerAttachmentEpoch,
    pub ingress_sequence: u64,
    /// Monotonic elapsed time in caller-defined ticks. The unit is stored by
    /// transcript metadata or the capture site; deterministic replay never
    /// interprets it as wall-clock time.
    pub monotonic_receipt_tick: u64,
    pub command_id: Option<PlayerCommandId>,
    pub playlist_entry_id: Option<i64>,
    /// Sanitized raw JSON. Sensitive keys and URL query credentials are
    /// redacted before this value enters the transcript.
    pub raw_json: Value,
}

impl fmt::Debug for MpvTranscriptRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MpvTranscriptRecord")
            .field("attachment_epoch", &self.attachment_epoch)
            .field("ingress_sequence", &self.ingress_sequence)
            .field("monotonic_receipt_tick", &self.monotonic_receipt_tick)
            .field("command_id", &self.command_id)
            .field("playlist_entry_id", &self.playlist_entry_id)
            .field("event", &self.event_name())
            .field("raw_json_sha256", &self.raw_json_sha256())
            .finish()
    }
}

impl MpvTranscriptRecord {
    /// Constructs a record and sanitizes its raw JSON before retaining it.
    pub fn sanitized(
        attachment_epoch: PlayerAttachmentEpoch,
        ingress_sequence: u64,
        monotonic_receipt_tick: u64,
        command_id: Option<PlayerCommandId>,
        playlist_entry_id: Option<i64>,
        raw_json: Value,
    ) -> Self {
        Self {
            attachment_epoch,
            ingress_sequence,
            monotonic_receipt_tick,
            command_id,
            playlist_entry_id,
            raw_json: sanitize_json(raw_json),
        }
    }

    /// Returns mpv's event name, or a compact classification for a command
    /// response/request.
    pub fn event_name(&self) -> Option<&str> {
        self.raw_json
            .get("event")
            .and_then(Value::as_str)
            .or_else(|| self.raw_json.get("command").is_some().then_some("command"))
            .or_else(|| {
                self.raw_json
                    .get("request_id")
                    .is_some()
                    .then_some("command-response")
            })
    }

    /// Stable digest of the sanitized JSON for privacy-safe trace correlation.
    pub fn raw_json_sha256(&self) -> String {
        let encoded = serde_json::to_vec(&self.raw_json)
            .expect("a serde_json::Value should always serialize successfully");
        hex_digest(Sha256::digest(encoded))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvTranscriptError {
    message: String,
}

impl MpvTranscriptError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MpvTranscriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for MpvTranscriptError {}

/// An ordered, validated set of sanitized IPC records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MpvTranscript {
    records: Vec<MpvTranscriptRecord>,
}

impl MpvTranscript {
    pub fn new(records: Vec<MpvTranscriptRecord>) -> Result<Self, MpvTranscriptError> {
        validate_records(&records)?;
        Ok(Self { records })
    }

    pub fn from_json_lines(input: &str) -> Result<Self, MpvTranscriptError> {
        let mut records = Vec::new();
        for (index, line) in input.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let wire: WireRecord = serde_json::from_str(line).map_err(|error| {
                MpvTranscriptError::new(format!(
                    "invalid transcript JSON on line {}: {error}",
                    index + 1
                ))
            })?;
            records.push(wire.into_record());
        }
        Self::new(records)
    }

    pub fn to_json_lines(&self) -> Result<String, MpvTranscriptError> {
        let mut output = String::new();
        for record in &self.records {
            let line =
                serde_json::to_string(&WireRecord::from_record(record)).map_err(|error| {
                    MpvTranscriptError::new(format!(
                        "failed to serialize transcript record: {error}"
                    ))
                })?;
            output.push_str(&line);
            output.push('\n');
        }
        Ok(output)
    }

    pub fn records(&self) -> &[MpvTranscriptRecord] {
        &self.records
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Replays the transcript through explicit pump partitions.
    ///
    /// Partition lengths must consume the whole transcript exactly. Each
    /// record remains in authoritative ingress order within and across batches.
    pub fn replay_partitioned(
        &self,
        partition_lengths: &[usize],
        sink: &mut impl MpvTranscriptReplaySink,
    ) -> Result<(), MpvTranscriptError> {
        if self.records.is_empty() && partition_lengths.is_empty() {
            return Ok(());
        }
        if partition_lengths.contains(&0) {
            return Err(MpvTranscriptError::new(
                "transcript replay partitions must be nonzero",
            ));
        }
        let partition_total = partition_lengths.iter().try_fold(0usize, |total, length| {
            total
                .checked_add(*length)
                .ok_or_else(|| MpvTranscriptError::new("transcript partition length overflow"))
        })?;
        if partition_total != self.records.len() {
            return Err(MpvTranscriptError::new(format!(
                "transcript partitions consume {partition_total} records, expected {}",
                self.records.len()
            )));
        }

        let mut offset = 0;
        for length in partition_lengths {
            let end = offset + length;
            sink.consume_batch(&self.records[offset..end]);
            offset = end;
        }
        Ok(())
    }

    /// Emits a compact privacy-safe dump suitable for a bug report. Raw JSON,
    /// media targets, headers, and credentials are never included.
    pub fn redacted_debug_dump(&self) -> String {
        let mut output = String::new();
        for record in &self.records {
            let event = record.event_name().unwrap_or("unknown");
            let command_id = record
                .command_id
                .map(|id| id.get().to_string())
                .unwrap_or_else(|| "-".to_owned());
            let playlist_entry_id = record
                .playlist_entry_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_owned());
            let _ = writeln!(
                output,
                "epoch={} seq={} tick={} command={} playlist={} kind={} json_sha256={}",
                record.attachment_epoch.get(),
                record.ingress_sequence,
                record.monotonic_receipt_tick,
                command_id,
                playlist_entry_id,
                event,
                record.raw_json_sha256(),
            );
        }
        output
    }
}

/// Batch-aware replay target. Implementations should reduce records in slice
/// order and must not infer causality from the slice boundary.
pub trait MpvTranscriptReplaySink {
    fn consume_batch(&mut self, records: &[MpvTranscriptRecord]);
}

/// Incremental recorder that validates attachment-scoped ingress order and
/// global monotonic receipt ticks.
#[derive(Debug, Default)]
pub struct MpvTranscriptRecorder {
    records: Vec<MpvTranscriptRecord>,
}

impl MpvTranscriptRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(
        &mut self,
        attachment_epoch: PlayerAttachmentEpoch,
        ingress_sequence: u64,
        monotonic_receipt_tick: u64,
        command_id: Option<PlayerCommandId>,
        playlist_entry_id: Option<i64>,
        raw_json: Value,
    ) -> Result<(), MpvTranscriptError> {
        let record = MpvTranscriptRecord::sanitized(
            attachment_epoch,
            ingress_sequence,
            monotonic_receipt_tick,
            command_id,
            playlist_entry_id,
            raw_json,
        );
        validate_append(&self.records, &record)?;
        self.records.push(record);
        Ok(())
    }

    pub fn finish(self) -> MpvTranscript {
        MpvTranscript {
            records: self.records,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WireRecord {
    attachment_epoch: u64,
    ingress_sequence: u64,
    monotonic_receipt_tick: u64,
    command_id: Option<u64>,
    playlist_entry_id: Option<i64>,
    raw_json: Value,
}

impl WireRecord {
    fn from_record(record: &MpvTranscriptRecord) -> Self {
        Self {
            attachment_epoch: record.attachment_epoch.get(),
            ingress_sequence: record.ingress_sequence,
            monotonic_receipt_tick: record.monotonic_receipt_tick,
            command_id: record.command_id.map(PlayerCommandId::get),
            playlist_entry_id: record.playlist_entry_id,
            raw_json: record.raw_json.clone(),
        }
    }

    fn into_record(self) -> MpvTranscriptRecord {
        MpvTranscriptRecord::sanitized(
            PlayerAttachmentEpoch::new(self.attachment_epoch),
            self.ingress_sequence,
            self.monotonic_receipt_tick,
            self.command_id.map(PlayerCommandId::new),
            self.playlist_entry_id,
            self.raw_json,
        )
    }
}

fn validate_records(records: &[MpvTranscriptRecord]) -> Result<(), MpvTranscriptError> {
    for (index, record) in records.iter().enumerate() {
        validate_record(record)?;
        validate_append(&records[..index], record)?;
    }
    Ok(())
}

fn validate_append(
    existing: &[MpvTranscriptRecord],
    record: &MpvTranscriptRecord,
) -> Result<(), MpvTranscriptError> {
    validate_record(record)?;
    if let Some(previous) = existing.last()
        && record.monotonic_receipt_tick < previous.monotonic_receipt_tick
    {
        return Err(MpvTranscriptError::new(format!(
            "receipt tick {} precedes prior tick {}",
            record.monotonic_receipt_tick, previous.monotonic_receipt_tick
        )));
    }
    if let Some(previous) = existing
        .iter()
        .rev()
        .find(|previous| previous.attachment_epoch == record.attachment_epoch)
        && record.ingress_sequence <= previous.ingress_sequence
    {
        return Err(MpvTranscriptError::new(format!(
            "attachment {} ingress sequence {} does not follow {}",
            record.attachment_epoch.get(),
            record.ingress_sequence,
            previous.ingress_sequence
        )));
    }
    Ok(())
}

fn validate_record(record: &MpvTranscriptRecord) -> Result<(), MpvTranscriptError> {
    if record.attachment_epoch.get() == 0 {
        return Err(MpvTranscriptError::new("attachment epoch must be nonzero"));
    }
    if record.ingress_sequence == 0 {
        return Err(MpvTranscriptError::new("ingress sequence must be nonzero"));
    }
    if !record.raw_json.is_object() {
        return Err(MpvTranscriptError::new(
            "raw transcript JSON must be an object",
        ));
    }
    Ok(())
}

fn sanitize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let sanitized = if sensitive_key(&key) {
                        Value::String(REDACTED.to_owned())
                    } else {
                        sanitize_json(value)
                    };
                    (key, sanitized)
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize_json).collect()),
        Value::String(value) => Value::String(sanitize_string(&value)),
        other => other,
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "cookies"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "access_token"
            | "refresh_token"
            | "api_key"
            | "http_header_fields"
            | "headers"
    ) || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
}

fn sanitize_string(value: &str) -> String {
    let trimmed = value.trim();
    let Some(scheme_end) = trimmed.find("://") else {
        return value.to_owned();
    };
    let authority_start = scheme_end + 3;
    let path_start = trimmed[authority_start..]
        .find(['/', '?', '#'])
        .map(|index| authority_start + index)
        .unwrap_or(trimmed.len());
    let mut sanitized = trimmed.to_owned();
    if let Some(userinfo_end) = trimmed[authority_start..path_start].rfind('@') {
        sanitized.replace_range(authority_start..authority_start + userinfo_end, REDACTED);
    }
    redact_sensitive_query_values(&sanitized)
}

fn redact_sensitive_query_values(value: &str) -> String {
    let Some(query_start) = value.find('?') else {
        return value.to_owned();
    };
    let fragment_start = value[query_start..]
        .find('#')
        .map(|index| query_start + index)
        .unwrap_or(value.len());
    let query = &value[query_start + 1..fragment_start];
    let mut pairs = Vec::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            pairs.push(pair.to_owned());
            continue;
        };
        if sensitive_key(key) || key.eq_ignore_ascii_case("sig") {
            pairs.push(format!("{key}={REDACTED}"));
        } else {
            pairs.push(format!("{key}={value}"));
        }
    }
    let mut sanitized = String::with_capacity(value.len());
    sanitized.push_str(&value[..query_start + 1]);
    sanitized.push_str(&pairs.join("&"));
    sanitized.push_str(&value[fragment_start..]);
    sanitized
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let mut encoded = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{
        AuthoritativePlaylistEntry, PlayerLifecycleInput, PlayerLifecycleState,
        reduce_player_lifecycle,
    };
    use serde_json::json;
    use sorotte_player_api::{
        PlayerMediaGeneration, PlayerMediaLoadFailureKind, PlayerPhysicalLoadOutcome,
    };
    use std::collections::{BTreeMap, BTreeSet};

    const FIXTURES: [(&str, &str); 9] = [
        (
            "local-file",
            include_str!("../../../fixtures/player-lifecycle-transcripts/local-file.jsonl"),
        ),
        (
            "http-load",
            include_str!("../../../fixtures/player-lifecycle-transcripts/http-load.jsonl"),
        ),
        (
            "youtube-like",
            include_str!("../../../fixtures/player-lifecycle-transcripts/youtube-like.jsonl"),
        ),
        (
            "cache-stall-recovery",
            include_str!(
                "../../../fixtures/player-lifecycle-transcripts/cache-stall-recovery.jsonl"
            ),
        ),
        (
            "premature-eof-recovery",
            include_str!(
                "../../../fixtures/player-lifecycle-transcripts/premature-eof-recovery.jsonl"
            ),
        ),
        (
            "seek-during-buffering",
            include_str!(
                "../../../fixtures/player-lifecycle-transcripts/seek-during-buffering.jsonl"
            ),
        ),
        (
            "rapid-a-b-c",
            include_str!("../../../fixtures/player-lifecycle-transcripts/rapid-a-b-c.jsonl"),
        ),
        (
            "keep-open",
            include_str!("../../../fixtures/player-lifecycle-transcripts/keep-open.jsonl"),
        ),
        (
            "reattachment",
            include_str!("../../../fixtures/player-lifecycle-transcripts/reattachment.jsonl"),
        ),
    ];

    #[derive(Default)]
    struct DigestSink {
        hasher: Sha256,
        record_count: usize,
        epochs: BTreeMap<u64, usize>,
    }

    impl MpvTranscriptReplaySink for DigestSink {
        fn consume_batch(&mut self, records: &[MpvTranscriptRecord]) {
            for record in records {
                self.hasher
                    .update(record.attachment_epoch.get().to_le_bytes());
                self.hasher.update(record.ingress_sequence.to_le_bytes());
                self.hasher
                    .update(record.monotonic_receipt_tick.to_le_bytes());
                self.hasher.update(
                    record
                        .command_id
                        .map(PlayerCommandId::get)
                        .unwrap_or_default()
                        .to_le_bytes(),
                );
                self.hasher
                    .update(record.playlist_entry_id.unwrap_or_default().to_le_bytes());
                self.hasher.update(
                    serde_json::to_vec(&record.raw_json).expect("fixture JSON should serialize"),
                );
                *self
                    .epochs
                    .entry(record.attachment_epoch.get())
                    .or_default() += 1;
                self.record_count += 1;
            }
        }
    }

    struct LifecycleReducerSink {
        state: PlayerLifecycleState,
        next_media_generation: u64,
        command_targets: BTreeMap<PlayerCommandId, String>,
        seek_commands: BTreeSet<PlayerCommandId>,
        last_position_seconds: Option<f64>,
        record_count: usize,
        epochs: BTreeMap<u64, usize>,
    }

    impl Default for LifecycleReducerSink {
        fn default() -> Self {
            Self {
                state: PlayerLifecycleState::default(),
                next_media_generation: 1,
                command_targets: BTreeMap::new(),
                seek_commands: BTreeSet::new(),
                last_position_seconds: None,
                record_count: 0,
                epochs: BTreeMap::new(),
            }
        }
    }

    impl LifecycleReducerSink {
        fn reduce(&mut self, input: PlayerLifecycleInput) {
            let state = std::mem::take(&mut self.state);
            let (state, _) = reduce_player_lifecycle(state, input);
            state
                .assert_invariants()
                .expect("transcript replay must preserve lifecycle invariants");
            self.state = state;
        }

        fn ensure_epoch(&mut self, epoch: PlayerAttachmentEpoch) {
            while self.state.attachment_epoch.get() < epoch.get() {
                self.reduce(PlayerLifecycleInput::AttachmentReplaced);
            }
            assert_eq!(
                self.state.attachment_epoch, epoch,
                "transcript epochs must not move backwards or skip reducer ownership"
            );
        }

        fn next_generation(&mut self) -> PlayerMediaGeneration {
            let generation = PlayerMediaGeneration::new(self.next_media_generation);
            self.next_media_generation = self.next_media_generation.saturating_add(1).max(1);
            generation
        }

        fn replay_record(&mut self, record: &MpvTranscriptRecord) {
            self.ensure_epoch(record.attachment_epoch);
            *self
                .epochs
                .entry(record.attachment_epoch.get())
                .or_default() += 1;
            self.record_count += 1;

            if let Some(command) = record.raw_json.get("command").and_then(Value::as_array) {
                self.replay_command(record, command);
                return;
            }
            if record.raw_json.get("request_id").is_some() {
                self.replay_response(record);
                return;
            }

            match record.event_name() {
                Some("start-file") => self.replay_start_file(record),
                Some("file-loaded") => self.replay_file_loaded(record),
                Some("end-file") => self.replay_end_file(record),
                Some("playback-restart") => {
                    self.reduce(PlayerLifecycleInput::PlaybackRestart {
                        attachment_epoch: record.attachment_epoch,
                        playlist_entry_id: record.playlist_entry_id,
                    });
                }
                Some("property-change") => self.replay_property(record),
                Some("shutdown") => {
                    self.reduce(PlayerLifecycleInput::TransportDisconnected {
                        attachment_epoch: record.attachment_epoch,
                    });
                }
                _ => {}
            }
        }

        fn replay_command(&mut self, record: &MpvTranscriptRecord, command: &[Value]) {
            let Some(name) = command.first().and_then(Value::as_str) else {
                return;
            };
            match name {
                "loadfile" => {
                    let Some(command_id) = record.command_id else {
                        return;
                    };
                    let target = command
                        .get(1)
                        .and_then(Value::as_str)
                        .unwrap_or("<unknown-target>")
                        .to_owned();
                    let recovery = command.get(4).is_some();
                    let generation = if recovery {
                        self.state
                            .active_media_generation()
                            .unwrap_or_else(|| self.next_generation())
                    } else {
                        self.next_generation()
                    };
                    self.command_targets.insert(command_id, target.clone());
                    let baseline_playlist_entry_ids =
                        self.state.playlist_entry_attempts.keys().copied().collect();
                    self.reduce(PlayerLifecycleInput::LoadAttemptSubmitted {
                        command_id: Some(command_id),
                        media_generation: generation,
                        requested_target: target,
                        baseline_playlist_entry_ids,
                    });
                }
                "set_property" if command.get(1).and_then(Value::as_str) == Some("time-pos") => {
                    let Some(command_id) = record.command_id else {
                        return;
                    };
                    let Some(media_generation) = self.state.active_media_generation() else {
                        return;
                    };
                    let target = command.get(2).and_then(Value::as_f64).unwrap_or_default();
                    self.seek_commands.insert(command_id);
                    self.reduce(PlayerLifecycleInput::SeekCommandSubmitted {
                        command_id,
                        media_generation,
                        raw_player_target_seconds: target,
                        effective_room_target_seconds: target,
                        dispatch_sequence_boundary: record.ingress_sequence,
                    });
                }
                _ => {}
            }
        }

        fn replay_response(&mut self, record: &MpvTranscriptRecord) {
            let Some(command_id) = record.command_id.or_else(|| {
                record
                    .raw_json
                    .get("request_id")
                    .and_then(Value::as_u64)
                    .map(PlayerCommandId::new)
            }) else {
                return;
            };
            let succeeded = record.raw_json.get("error").and_then(Value::as_str) == Some("success");
            if let Some(attempt_id) = self.state.attempt_for_command(command_id) {
                if succeeded {
                    self.reduce(PlayerLifecycleInput::LoadAttemptAccepted {
                        attachment_epoch: record.attachment_epoch,
                        attempt_id,
                    });
                    let entries = playlist_entries(&record.raw_json);
                    if !entries.is_empty() {
                        let current_path = entries
                            .iter()
                            .find(|entry| entry.current)
                            .and_then(|entry| entry.original_filename.clone());
                        self.reduce(PlayerLifecycleInput::PlaylistSnapshot {
                            attachment_epoch: record.attachment_epoch,
                            entries,
                            current_path,
                        });
                    }
                } else {
                    self.reduce(PlayerLifecycleInput::LoadAttemptRejected {
                        attachment_epoch: record.attachment_epoch,
                        attempt_id,
                        failure: sorotte_player_api::PlayerCommandFailureKind::Unknown,
                    });
                }
            } else if self.seek_commands.contains(&command_id) {
                if succeeded {
                    self.reduce(PlayerLifecycleInput::SeekCommandAccepted {
                        attachment_epoch: record.attachment_epoch,
                        command_id,
                    });
                } else {
                    self.reduce(PlayerLifecycleInput::SeekCommandRejected {
                        attachment_epoch: record.attachment_epoch,
                        command_id,
                        failure: sorotte_player_api::PlayerCommandFailureKind::Unknown,
                    });
                }
            }
        }

        fn replay_start_file(&mut self, record: &MpvTranscriptRecord) {
            let Some(playlist_entry_id) = record.playlist_entry_id else {
                return;
            };
            if !self
                .state
                .playlist_entry_attempts
                .contains_key(&playlist_entry_id)
                && let Some(command_id) = record.command_id
                && self.state.attempt_for_command(command_id).is_some()
            {
                let target = self
                    .command_targets
                    .get(&command_id)
                    .cloned()
                    .unwrap_or_else(|| format!("<entry-{playlist_entry_id}>"));
                self.reduce(PlayerLifecycleInput::PlaylistSnapshot {
                    attachment_epoch: record.attachment_epoch,
                    entries: vec![AuthoritativePlaylistEntry::new(
                        playlist_entry_id,
                        Some(target.clone()),
                        true,
                    )],
                    current_path: Some(target),
                });
            }
            if !self
                .state
                .playlist_entry_attempts
                .contains_key(&playlist_entry_id)
            {
                let generation = self.next_generation();
                self.reduce(PlayerLifecycleInput::ExternalLoadObserved {
                    attachment_epoch: record.attachment_epoch,
                    media_generation: generation,
                    playlist_entry_id,
                    observed_target: format!("<external-entry-{playlist_entry_id}>"),
                    file_loaded: false,
                });
            }
            self.reduce(PlayerLifecycleInput::StartFile {
                attachment_epoch: record.attachment_epoch,
                playlist_entry_id,
            });
        }

        fn replay_file_loaded(&mut self, record: &MpvTranscriptRecord) {
            let Some(playlist_entry_id) = record.playlist_entry_id else {
                return;
            };
            if !self
                .state
                .playlist_entry_attempts
                .contains_key(&playlist_entry_id)
            {
                let generation = self.next_generation();
                self.reduce(PlayerLifecycleInput::ExternalLoadObserved {
                    attachment_epoch: record.attachment_epoch,
                    media_generation: generation,
                    playlist_entry_id,
                    observed_target: format!("<external-entry-{playlist_entry_id}>"),
                    file_loaded: true,
                });
                return;
            }
            self.reduce(PlayerLifecycleInput::FileLoaded {
                attachment_epoch: record.attachment_epoch,
                playlist_entry_id: Some(playlist_entry_id),
                loaded_target: record
                    .command_id
                    .and_then(|command_id| self.command_targets.get(&command_id).cloned()),
            });
        }

        fn replay_end_file(&mut self, record: &MpvTranscriptRecord) {
            let Some(playlist_entry_id) = record.playlist_entry_id else {
                return;
            };
            let outcome = if record.raw_json.get("reason").and_then(Value::as_str) == Some("error")
            {
                PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network)
            } else {
                PlayerPhysicalLoadOutcome::Ended
            };
            self.reduce(PlayerLifecycleInput::EndFile {
                attachment_epoch: record.attachment_epoch,
                playlist_entry_id,
                outcome,
            });
        }

        fn replay_property(&mut self, record: &MpvTranscriptRecord) {
            let name = record.raw_json.get("name").and_then(Value::as_str);
            let data = record.raw_json.get("data");
            match name {
                Some("eof-reached") => {
                    self.reduce(PlayerLifecycleInput::EofObserved {
                        attachment_epoch: record.attachment_epoch,
                        playlist_entry_id: record.playlist_entry_id,
                        reached: data.and_then(Value::as_bool).unwrap_or(false),
                        position_seconds: self.last_position_seconds,
                    });
                }
                Some("time-pos") => {
                    let Some(position_seconds) = data.and_then(Value::as_f64) else {
                        return;
                    };
                    self.last_position_seconds = Some(position_seconds);
                    if let Some(media_generation) = self.state.active_media_generation() {
                        self.reduce(PlayerLifecycleInput::PositionObserved {
                            attachment_epoch: record.attachment_epoch,
                            media_generation,
                            observed_sequence: record.ingress_sequence,
                            position_seconds,
                        });
                    }
                }
                Some("seeking") => {
                    let Some(media_generation) = self.state.active_media_generation() else {
                        return;
                    };
                    self.reduce(PlayerLifecycleInput::SeekingObserved {
                        attachment_epoch: record.attachment_epoch,
                        media_generation,
                        observed_sequence: record.ingress_sequence,
                        seeking: data.and_then(Value::as_bool).unwrap_or(false),
                    });
                }
                _ => {}
            }
        }

        fn finish(self) -> (String, usize, BTreeMap<u64, usize>) {
            self.state
                .assert_invariants()
                .expect("final transcript lifecycle state must be valid");
            let mut playlist_entry_attempts: Vec<_> = self
                .state
                .playlist_entry_attempts
                .iter()
                .map(|(entry_id, attempt_id)| (*entry_id, *attempt_id))
                .collect();
            playlist_entry_attempts.sort_unstable();
            let mut batch_state = self.state.clone();
            let pending_batch = batch_state.peek_event_batch();
            let semantic_state = format!(
                "epoch={:?}|attempts={:?}|entry_attempts={playlist_entry_attempts:?}|\
                 active={:?}|commands={:?}|seeks={:?}|reconciliation={:?}/{:?}/{:?}/{:?}|\
                 terminal={:?}|pending_batch={pending_batch:?}",
                self.state.attachment_epoch,
                self.state.load_attempts,
                self.state.active_load_attempt,
                self.state.commands,
                self.state.seek_ownership,
                self.state.reconciliation_required,
                self.state.last_reconciliation,
                self.state.next_reconciliation_tick,
                self.state.reconciliation_backoff_ticks,
                self.state.logical_terminal,
            );
            (
                hex_digest(Sha256::digest(semantic_state.as_bytes())),
                self.record_count,
                self.epochs,
            )
        }
    }

    impl MpvTranscriptReplaySink for LifecycleReducerSink {
        fn consume_batch(&mut self, records: &[MpvTranscriptRecord]) {
            for record in records {
                self.replay_record(record);
            }
        }
    }

    fn playlist_entries(raw_json: &Value) -> Vec<AuthoritativePlaylistEntry> {
        raw_json
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some(AuthoritativePlaylistEntry::new(
                    entry.get("id")?.as_i64()?,
                    entry
                        .get("filename")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    entry
                        .get("current")
                        .or_else(|| entry.get("playing"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ))
            })
            .collect()
    }

    #[test]
    fn sanitized_fixtures_parse_round_trip_and_replay_under_any_partition() {
        for (name, fixture) in FIXTURES {
            let transcript = MpvTranscript::from_json_lines(fixture)
                .unwrap_or_else(|error| panic!("{name} fixture should parse: {error}"));
            assert!(!transcript.is_empty(), "{name} fixture must not be empty");
            let round_trip = transcript
                .to_json_lines()
                .unwrap_or_else(|error| panic!("{name} fixture should serialize: {error}"));
            assert_eq!(
                MpvTranscript::from_json_lines(&round_trip).expect("round trip"),
                transcript,
                "{name} round trip"
            );

            let mut individual = LifecycleReducerSink::default();
            transcript
                .replay_partitioned(&vec![1; transcript.len()], &mut individual)
                .expect("individual replay");
            let mut all = LifecycleReducerSink::default();
            transcript
                .replay_partitioned(&[transcript.len()], &mut all)
                .expect("single replay");
            let varied = varied_partitions(transcript.len());
            let mut randomized_shape = LifecycleReducerSink::default();
            transcript
                .replay_partitioned(&varied, &mut randomized_shape)
                .expect("varied replay");

            assert_eq!(
                individual.finish(),
                all.finish(),
                "{name} must be pump-partition invariant"
            );
            let mut expected = LifecycleReducerSink::default();
            transcript
                .replay_partitioned(&[transcript.len()], &mut expected)
                .expect("expected replay");
            assert_eq!(
                expected.finish(),
                randomized_shape.finish(),
                "{name} varied partitions"
            );
        }
    }

    #[test]
    fn fixtures_are_synthetic_and_cover_required_lifecycle_shapes() {
        let forbidden = [
            "youtube.com",
            "googlevideo.com",
            "authorization",
            "cookie",
            "bearer ",
            "c:\\users\\",
            "/home/",
        ];
        for (name, fixture) in FIXTURES {
            let normalized = fixture.to_ascii_lowercase();
            for marker in forbidden {
                assert!(
                    !normalized.contains(marker),
                    "{name} fixture contains forbidden marker {marker}"
                );
            }
        }

        let reattachment =
            MpvTranscript::from_json_lines(FIXTURES[8].1).expect("reattachment fixture");
        let reused: Vec<_> = reattachment
            .records()
            .iter()
            .filter(|record| record.playlist_entry_id == Some(1))
            .map(|record| record.attachment_epoch.get())
            .collect();
        assert!(reused.contains(&1));
        assert!(reused.contains(&2));

        let rapid = MpvTranscript::from_json_lines(FIXTURES[6].1).expect("rapid fixture");
        assert!(rapid.records().iter().any(|record| {
            record.event_name() == Some("end-file") && record.playlist_entry_id == Some(20)
        }));
        assert!(rapid.records().iter().any(|record| {
            record.event_name() == Some("file-loaded") && record.playlist_entry_id == Some(30)
        }));
    }

    #[test]
    fn recorder_redacts_secrets_before_storage_and_debug_dump_omits_raw_media() {
        let secret = "do-not-retain-this-secret";
        let private_media = "https://user:password@media.invalid/private/video.mkv";
        let mut recorder = MpvTranscriptRecorder::new();
        recorder
            .record(
                PlayerAttachmentEpoch::new(1),
                1,
                10,
                Some(PlayerCommandId::new(7)),
                Some(41),
                json!({
                    "event": "start-file",
                    "playlist_entry_id": 41,
                    "path": format!("{private_media}?token={secret}&quality=1080p"),
                    "authorization": format!("Bearer {secret}"),
                    "nested": {"api-key": secret},
                }),
            )
            .expect("record should be accepted");
        let transcript = recorder.finish();
        let exported = transcript.to_json_lines().expect("transcript export");
        let dump = transcript.redacted_debug_dump();

        assert!(!exported.contains(secret));
        assert!(!exported.contains("user:password"));
        assert!(exported.contains(REDACTED));
        assert!(!dump.contains(secret));
        assert!(!dump.contains("private/video.mkv"));
        assert!(!dump.contains("media.invalid"));
        assert!(dump.contains("epoch=1 seq=1 tick=10"));
        assert!(dump.contains("kind=start-file"));
        assert!(dump.contains("json_sha256="));
        assert!(!format!("{:?}", transcript.records()[0]).contains(secret));
    }

    #[test]
    fn validation_rejects_nonmonotonic_identity_or_time() {
        let record = |epoch, sequence, tick| {
            MpvTranscriptRecord::sanitized(
                PlayerAttachmentEpoch::new(epoch),
                sequence,
                tick,
                None,
                None,
                json!({"event": "idle"}),
            )
        };
        assert!(MpvTranscript::new(vec![record(1, 1, 1), record(1, 1, 2)]).is_err());
        assert!(MpvTranscript::new(vec![record(1, 2, 2), record(1, 1, 3)]).is_err());
        assert!(MpvTranscript::new(vec![record(1, 1, 2), record(2, 1, 1)]).is_err());
        assert!(MpvTranscript::new(vec![record(0, 1, 1)]).is_err());
        assert!(MpvTranscript::new(vec![record(1, 0, 1)]).is_err());
    }

    #[test]
    fn partition_contract_rejects_partial_or_empty_batches() {
        let transcript =
            MpvTranscript::from_json_lines(FIXTURES[0].1).expect("local transcript fixture");
        assert!(
            transcript
                .replay_partitioned(&[transcript.len() - 1], &mut DigestSink::default())
                .is_err()
        );
        assert!(
            transcript
                .replay_partitioned(&[0, transcript.len()], &mut DigestSink::default())
                .is_err()
        );
    }

    fn varied_partitions(length: usize) -> Vec<usize> {
        let mut remaining = length;
        let mut next = 1usize;
        let mut partitions = Vec::new();
        while remaining > 0 {
            let length = next.min(remaining);
            partitions.push(length);
            remaining -= length;
            next = if next == 3 { 1 } else { next + 1 };
        }
        partitions
    }
}
