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
use url::Url;

use crate::constants::{
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT, LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED,
    LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED, LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG,
    MPV_EVENT_CLIENT_MESSAGE, SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_ACTIVE_RESULT,
    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_CONFIGURED,
    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP,
    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT,
};

const REDACTED: &str = "<redacted>";
const ANONYMIZED_PREFIX: &str = "anon_";

/// One sanitized decoded mpv event or synthetic lifecycle-model input.
///
/// The opt-in adapter capture supplies decoded event-pump items only. Tests and
/// fixture tooling may construct other model inputs explicitly.
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
    /// Sanitized raw JSON. Sensitive keys, opaque third-party payloads, media
    /// paths, headers, and URL credentials are removed before this value enters
    /// the transcript.
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

/// Incremental model/test recorder that validates attachment-scoped order and
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SanitizationContext {
    Structured,
    Opaque,
    SorotteControl,
}

fn sanitize_json(value: Value) -> Value {
    sanitize_value(value, SanitizationContext::Structured)
}

fn sanitize_value(value: Value, context: SanitizationContext) -> Value {
    match value {
        Value::Object(object) => sanitize_object(object, context),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_value(value, context))
                .collect(),
        ),
        Value::String(value) => Value::String(sanitize_string(&value, context)),
        other => other,
    }
}

fn sanitize_object(object: Map<String, Value>, context: SanitizationContext) -> Value {
    let is_client_message =
        object.get("event").and_then(Value::as_str) == Some(MPV_EVENT_CLIENT_MESSAGE);
    Value::Object(
        object
            .into_iter()
            .map(|(key, value)| {
                let sanitized = if sensitive_key(&key) {
                    Value::String(REDACTED.to_owned())
                } else if is_client_message && key == "args" {
                    sanitize_client_message_args(value)
                } else if location_key(&key) {
                    sanitize_location_value(value, context)
                } else {
                    sanitize_value(value, context)
                };
                (key, sanitized)
            })
            .collect::<Map<_, _>>(),
    )
}

fn sanitize_client_message_args(value: Value) -> Value {
    let Value::Array(values) = value else {
        return sanitize_value(value, SanitizationContext::Opaque);
    };
    let Some(message_name) = values.first().and_then(Value::as_str) else {
        return Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_value(value, SanitizationContext::Opaque))
                .collect(),
        );
    };
    let Some(payload_context) = sorotte_client_message_context(message_name) else {
        return Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_value(value, SanitizationContext::Opaque))
                .collect(),
        );
    };

    Value::Array(
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                if index == 0 {
                    value
                } else {
                    sanitize_value(value, payload_context)
                }
            })
            .collect(),
    )
}

fn sorotte_client_message_context(message_name: &str) -> Option<SanitizationContext> {
    match message_name {
        LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT => Some(SanitizationContext::Opaque),
        LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED
        | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG
        | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED
        | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_CONFIGURED
        | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP
        | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT
        | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_ACTIVE_RESULT
        | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT => {
            Some(SanitizationContext::SorotteControl)
        }
        _ => None,
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = normalized_key(key);
    let compact = normalized.replace('_', "");
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
        || matches!(
            compact.as_str(),
            "accesstoken" | "refreshtoken" | "apikey" | "httpheaderfields" | "proxyauthorization"
        )
        || compact.ends_with("token")
        || compact.ends_with("secret")
        || compact.ends_with("password")
}

fn location_key(key: &str) -> bool {
    let normalized = normalized_key(key);
    let compact = normalized.replace('_', "");
    matches!(
        normalized.as_str(),
        "path"
            | "filename"
            | "file_name"
            | "file"
            | "directory"
            | "dir"
            | "cwd"
            | "working_directory"
            | "url"
            | "uri"
            | "target"
            | "media_path"
            | "media_url"
            | "stream_url"
    ) || normalized.ends_with("_path")
        || normalized.ends_with("_filename")
        || normalized.ends_with("_directory")
        || normalized.ends_with("_url")
        || normalized.ends_with("_uri")
        || compact.ends_with("path")
        || compact.ends_with("filename")
        || compact.ends_with("directory")
        || compact.ends_with("url")
        || compact.ends_with("uri")
}

fn normalized_key(key: &str) -> String {
    key.to_ascii_lowercase().replace('-', "_")
}

fn sanitize_location_value(value: Value, context: SanitizationContext) -> Value {
    match value {
        Value::String(value) => Value::String(sanitize_location_string(&value)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_location_value(value, context))
                .collect(),
        ),
        Value::Object(object) => sanitize_object(object, context),
        other => other,
    }
}

fn sanitize_location_string(value: &str) -> String {
    if is_sanitized_marker(value) {
        return value.to_owned();
    }
    if looks_like_url(value) {
        return sanitize_url(value).unwrap_or_else(|| anonymize(value));
    }
    if value.is_empty() {
        return String::new();
    }
    anonymize(value)
}

fn sanitize_string(value: &str, context: SanitizationContext) -> String {
    if is_sanitized_marker(value) {
        return value.to_owned();
    }
    if let Some(embedded) = parse_embedded_json(value) {
        let embedded_context = match context {
            SanitizationContext::SorotteControl => SanitizationContext::SorotteControl,
            SanitizationContext::Structured | SanitizationContext::Opaque => {
                SanitizationContext::Opaque
            }
        };
        return serde_json::to_string(&sanitize_value(embedded, embedded_context))
            .expect("a sanitized serde_json::Value should always serialize");
    }
    if looks_like_url(value)
        && let Some(sanitized) = sanitize_url(value)
    {
        return sanitized;
    }
    if let Some(sanitized) = sanitize_header(value, context == SanitizationContext::Opaque) {
        return sanitized;
    }
    if looks_like_filesystem_path(value) {
        return anonymize(value);
    }
    if context == SanitizationContext::Opaque && !value.is_empty() {
        return anonymize(value);
    }
    value.to_owned()
}

fn parse_embedded_json(value: &str) -> Option<Value> {
    let trimmed = value.trim();
    if !trimmed.starts_with(['{', '[']) {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn looks_like_url(value: &str) -> bool {
    let trimmed = value.trim();
    let Some(scheme_end) = trimmed.find("://") else {
        return false;
    };
    scheme_end > 0
        && trimmed[..scheme_end].chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn sanitize_url(value: &str) -> Option<String> {
    let mut parsed = Url::parse(value.trim()).ok()?;

    if !parsed.username().is_empty() || parsed.password().is_some() {
        parsed.set_username(REDACTED).ok()?;
        parsed.set_password(None).ok()?;
    }
    if let Some(host) = parsed.host_str().map(str::to_owned)
        && !is_anonymized_host(&host)
    {
        parsed.set_host(Some(&anonymize_host(&host))).ok()?;
    }
    parsed.set_port(None).ok()?;

    let sanitized_path = parsed
        .path()
        .split('/')
        .map(|segment| {
            if segment.is_empty() || is_sanitized_marker(segment) {
                segment.to_owned()
            } else {
                anonymize(segment)
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    parsed.set_path(&sanitized_path);

    if parsed.query().is_some() {
        let query_pairs = parsed
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        parsed.set_query(None);
        {
            let mut output = parsed.query_pairs_mut();
            for (key, value) in query_pairs {
                let key = if value.is_empty() && looks_like_opaque_identifier(&key) {
                    anonymize(&key)
                } else {
                    key
                };
                let value = if value.is_empty() {
                    value
                } else {
                    anonymize(&value)
                };
                output.append_pair(&key, &value);
            }
        }
    }

    if let Some(fragment) = parsed.fragment().map(str::to_owned)
        && !fragment.is_empty()
    {
        parsed.set_fragment(Some(&anonymize(&fragment)));
    }
    Some(parsed.into())
}

fn sanitize_header(value: &str, redact_all: bool) -> Option<String> {
    let (name, raw_value) = value.split_once(':')?;
    let name = name.trim();
    let normalized = normalized_key(name);
    let valid_name = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    let sensitive = matches!(
        normalized.as_str(),
        "authorization"
            | "proxy_authorization"
            | "cookie"
            | "set_cookie"
            | "x_api_key"
            | "x_auth_token"
            | "x_amz_security_token"
    ) || normalized.contains("credential")
        || normalized.ends_with("_token");
    if !valid_name || (!redact_all && !sensitive) {
        return None;
    }

    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return Some(format!("{name}:"));
    }
    Some(format!("{name}: {}", anonymize(trimmed)))
}

fn looks_like_filesystem_path(value: &str) -> bool {
    let value = value.trim();
    let relative_forward_path = !value.chars().any(char::is_whitespace)
        && value.rsplit_once('/').is_some_and(|(parent, filename)| {
            !parent.is_empty()
                && !filename.is_empty()
                && (parent.contains('/') || filename.contains('.'))
        });
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || value.starts_with("./")
        || value.starts_with(".\\")
        || value.contains('\\')
        || relative_forward_path
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn looks_like_opaque_identifier(value: &str) -> bool {
    value.len() >= 24
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn is_anonymized_host(value: &str) -> bool {
    let Some(digest) = value
        .strip_prefix("anon-")
        .and_then(|value| value.strip_suffix(".invalid"))
    else {
        return false;
    };
    digest.len() == 16 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn anonymize_host(value: &str) -> String {
    if is_anonymized_host(value) {
        return value.to_owned();
    }
    let pseudonym = anonymize(value);
    format!(
        "anon-{}.invalid",
        pseudonym
            .strip_prefix(ANONYMIZED_PREFIX)
            .expect("an anonymized value must carry its prefix")
    )
}

fn is_sanitized_marker(value: &str) -> bool {
    if value == REDACTED {
        return true;
    }
    let Some(digest) = value.strip_prefix(ANONYMIZED_PREFIX) else {
        return false;
    };
    digest.len() == 16 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn anonymize(value: &str) -> String {
    if is_sanitized_marker(value) {
        return value.to_owned();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"sorotte-transcript-anonymization-v1\0");
    hasher.update(value.as_bytes());
    let digest = hex_digest(hasher.finalize());
    format!("{ANONYMIZED_PREFIX}{}", &digest[..16])
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
        assert!(!exported.contains("media.invalid"));
        assert!(!exported.contains("private/video.mkv"));
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
    fn third_party_client_messages_anonymize_embedded_json_headers_paths_and_signed_urls() {
        let private_text = "private-chat-canary-4f1a";
        let private_header = "Bearer private-header-canary-b821";
        let private_path = r"C:\Users\Yuuki\Videos\private-episode.mkv";
        let private_signature = "renewable-signature-canary-c309";
        let private_url = format!(
            "https://viewer:password@private-media.example:8443/users/yuuki/episode.mkv?X-Amz-Signature={private_signature}&quality=1080p#private-fragment"
        );
        let embedded = json!({
            "text": private_text,
            "path": private_path,
            "url": private_url,
            "details": [
                format!("Authorization: {private_header}"),
                private_path,
            ],
        })
        .to_string();
        let input = json!({
            "event": MPV_EVENT_CLIENT_MESSAGE,
            "args": [
                "third-party-chat",
                embedded,
                private_text,
                format!("Cookie: session={private_signature}"),
                private_url,
            ],
        });

        let sanitized = sanitize_json(input);
        assert_eq!(
            sanitize_json(sanitized.clone()),
            sanitized,
            "sanitization must be idempotent so fixture round trips stay stable"
        );
        let exported = sanitized.to_string();
        for private in [
            private_text,
            private_header,
            private_path,
            private_signature,
            "viewer:password",
            "private-media.example",
            "users/yuuki/episode.mkv",
            "private-fragment",
        ] {
            assert!(
                !exported.contains(private),
                "sanitized third-party payload retained {private}"
            );
        }

        let args = sanitized["args"]
            .as_array()
            .expect("client-message args should remain an array");
        assert!(is_sanitized_marker(
            args[0].as_str().expect("message name pseudonym")
        ));
        assert_eq!(
            args[2].as_str(),
            Some(anonymize(private_text).as_str()),
            "equal private strings should retain a useful deterministic pseudonym"
        );
        assert!(
            args[3]
                .as_str()
                .expect("header pseudonym")
                .starts_with("Cookie: anon_")
        );

        let sanitized_embedded: Value =
            serde_json::from_str(args[1].as_str().expect("embedded JSON string"))
                .expect("embedded JSON should remain valid");
        assert_eq!(
            sanitized_embedded["text"].as_str(),
            Some(anonymize(private_text).as_str())
        );
        assert_eq!(
            sanitized_embedded["path"].as_str(),
            Some(anonymize(private_path).as_str())
        );
        assert!(
            sanitized_embedded["details"][0]
                .as_str()
                .expect("embedded header")
                .starts_with("Authorization: anon_")
        );
        assert!(
            sanitized_embedded["url"]
                .as_str()
                .expect("embedded URL")
                .contains("anon-")
        );
    }

    #[test]
    fn recognized_sorotte_client_messages_keep_routing_and_control_payload_shape() {
        let control_messages = [
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED,
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG,
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED,
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_CONFIGURED,
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP,
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_ACTIVE_RESULT,
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT,
        ];
        let private_path = r"C:\Users\Shaun\private-network-options.json";
        for message_name in control_messages {
            let sanitized = sanitize_json(json!({
                "event": MPV_EVENT_CLIENT_MESSAGE,
                "args": [
                    message_name,
                    json!({
                        "protocol": "sorotte-network-options-v3",
                        "status": "ready",
                        "heartbeatNonce": 551,
                        "path": private_path,
                    }).to_string(),
                ],
            }));
            let args = sanitized["args"]
                .as_array()
                .expect("control message args should remain an array");
            assert_eq!(
                args[0].as_str(),
                Some(message_name),
                "Sorotte routing name must remain recognizable"
            );
            let payload: Value = serde_json::from_str(
                args[1]
                    .as_str()
                    .expect("control payload should remain encoded JSON"),
            )
            .expect("control payload should remain valid JSON");
            assert_eq!(payload["protocol"], "sorotte-network-options-v3");
            assert_eq!(payload["status"], "ready");
            assert_eq!(payload["heartbeatNonce"], 551);
            assert_eq!(payload["path"], anonymize(private_path));
        }

        let private_chat = "private-syncplay-chat-canary";
        let sanitized_chat = sanitize_json(json!({
            "event": MPV_EVENT_CLIENT_MESSAGE,
            "args": [LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT, private_chat],
        }));
        assert_eq!(
            sanitized_chat["args"][0], LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT,
            "recognized chat routing must remain intact"
        );
        assert_eq!(
            sanitized_chat["args"][1],
            anonymize(private_chat),
            "chat contents must still be anonymized"
        );
    }

    #[test]
    fn generic_event_arrays_anonymize_relative_paths_and_header_credentials() {
        let private_path = "private/show.mkv";
        let private_bare_filename = "private-episode-canary.mkv";
        let private_header = "Bearer generic-array-canary";
        let sanitized = sanitize_json(json!({
            "event": "property-change",
            "name": "third-party-metadata",
            "data": [
                private_path,
                format!(" Authorization: {private_header}"),
                {"mediaPath": private_bare_filename},
            ],
        }));

        assert_eq!(sanitized["data"][0], anonymize(private_path));
        assert_eq!(
            sanitized["data"][1],
            format!("Authorization: {}", anonymize(private_header))
        );
        assert_eq!(
            sanitized["data"][2]["mediaPath"],
            anonymize(private_bare_filename)
        );
        assert!(!sanitized.to_string().contains(private_path));
        assert!(!sanitized.to_string().contains(private_bare_filename));
        assert!(!sanitized.to_string().contains(private_header));
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
