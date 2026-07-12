use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Hello feature and nested Set/State extension key for the additive Sorotte
/// playback-start barrier protocol.
pub const SOROTTE_PLAYBACK_BARRIER_V1: &str = "sorottePlaybackBarrierV1";

/// Room-level behavior when capable participants report an ongoing transport
/// stall after playback has started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomBufferingPolicy {
    Independent,
    PauseController,
    PauseAnyEligible,
    Quorum,
}

/// Controller-authored, generation-scoped room buffering policy.
///
/// Timing values are requests. Servers normalize them to bounded values before
/// publishing the active policy in [`RoomBufferingStatusPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomBufferingPolicyPayload {
    pub media_generation: u64,
    /// Strictly increasing connection-scoped request nonce when
    /// `media_generation` is zero. The server echoes it on canonical config.
    #[serde(default)]
    pub request_nonce: u64,
    /// Whether a zero-generation request starts a new playback episode or is
    /// merely refreshing transport for the current episode.
    #[serde(default)]
    pub load_intent: MediaLoadIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<u64>,
    pub policy: RoomBufferingPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_hysteresis_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pause_ms: Option<u64>,
}

impl RoomBufferingPolicyPayload {
    pub fn new(media_generation: u64, policy: RoomBufferingPolicy) -> Self {
        Self {
            media_generation,
            request_nonce: 0,
            load_intent: MediaLoadIntent::NewPlayback,
            state_revision: None,
            policy,
            quorum_percent: None,
            debounce_ms: None,
            resume_hysteresis_ms: None,
            max_pause_ms: None,
        }
    }

    pub fn with_request_nonce(mut self, request_nonce: u64) -> Self {
        self.request_nonce = request_nonce;
        self
    }

    pub fn with_load_intent(mut self, load_intent: MediaLoadIntent) -> Self {
        self.load_intent = load_intent;
        self
    }

    pub fn with_state_revision(mut self, state_revision: u64) -> Self {
        self.state_revision = Some(state_revision);
        self
    }

    pub fn with_quorum_percent(mut self, quorum_percent: u32) -> Self {
        self.quorum_percent = Some(quorum_percent);
        self
    }

    pub fn with_debounce_ms(mut self, debounce_ms: u64) -> Self {
        self.debounce_ms = Some(debounce_ms);
        self
    }

    pub fn with_resume_hysteresis_ms(mut self, resume_hysteresis_ms: u64) -> Self {
        self.resume_hysteresis_ms = Some(resume_hysteresis_ms);
        self
    }

    pub fn with_max_pause_ms(mut self, max_pause_ms: u64) -> Self {
        self.max_pause_ms = Some(max_pause_ms);
        self
    }
}

/// Session-bound transport observation. Identity is derived from the
/// authenticated connection; clients cannot name another participant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportBufferingReportPayload {
    pub media_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<u64>,
    pub buffering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<f64>,
}

impl TransportBufferingReportPayload {
    pub fn new(media_generation: u64, buffering: bool) -> Self {
        Self {
            media_generation,
            state_revision: None,
            buffering,
            buffered_seconds: None,
            observed_at: None,
        }
    }

    pub fn with_state_revision(mut self, state_revision: u64) -> Self {
        self.state_revision = Some(state_revision);
        self
    }

    pub fn with_buffered_seconds(mut self, buffered_seconds: f64) -> Self {
        self.buffered_seconds = Some(buffered_seconds);
        self
    }

    pub fn with_observed_at(mut self, observed_at: f64) -> Self {
        self.observed_at = Some(observed_at);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomBufferingPhase {
    Independent,
    Monitoring,
    DebouncingPause,
    Paused,
    DebouncingResume,
    FailOpen,
}

/// Server-owned projection of the active buffering policy and eligible cohort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomBufferingStatusPayload {
    pub config: RoomBufferingPolicyPayload,
    pub phase: RoomBufferingPhase,
    pub eligible_clients: u32,
    pub required_buffering_clients: u32,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub buffering_clients: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_deadline: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackBarrierPolicy {
    AllEligible,
    Controller,
    Quorum,
}

/// Why a media load is being requested. Only new playback episodes allocate
/// a new authoritative room generation; transport refreshes retain it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MediaLoadIntent {
    #[default]
    NewPlayback,
    Replay,
    TransportRefresh,
}

/// Server-enforced behavior when the prepare cohort misses its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackBarrierTimeoutAction {
    #[default]
    Continue,
    RemainPaused,
    AskController,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareMediaPayload {
    /// Zero in a client request and assigned monotonically by the server in
    /// the canonical prepare broadcast.
    pub media_generation: u64,
    /// Strictly increasing, connection-scoped idempotency key. This is
    /// deliberately independent of the client's wall clock.
    #[serde(default)]
    pub request_nonce: u64,
    #[serde(default)]
    pub load_intent: MediaLoadIntent,
    pub logical_media_id: String,
    pub target_position: f64,
    pub policy: PlaybackBarrierPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
    /// Preferred cohort-relative quorum. New servers normalize this percentage
    /// to the absolute `quorum` after capturing the capable participant set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_action: Option<PlaybackBarrierTimeoutAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<f64>,
}

impl std::fmt::Debug for PrepareMediaPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrepareMediaPayload")
            .field("media_generation", &self.media_generation)
            .field("request_nonce", &self.request_nonce)
            .field("load_intent", &self.load_intent)
            .field("logical_media_id", &"<redacted>")
            .field("target_position", &self.target_position)
            .field("policy", &self.policy)
            .field("quorum", &self.quorum)
            .field("quorum_percent", &self.quorum_percent)
            .field("timeout_ms", &self.timeout_ms)
            .field("timeout_action", &self.timeout_action)
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl PrepareMediaPayload {
    pub fn new(
        media_generation: u64,
        logical_media_id: impl Into<String>,
        target_position: f64,
        policy: PlaybackBarrierPolicy,
    ) -> Self {
        Self {
            media_generation,
            request_nonce: 0,
            load_intent: MediaLoadIntent::NewPlayback,
            logical_media_id: logical_media_id.into(),
            target_position,
            policy,
            quorum: None,
            quorum_percent: None,
            timeout_ms: None,
            timeout_action: None,
            deadline: None,
        }
    }

    pub fn request(
        request_nonce: u64,
        logical_media_id: impl Into<String>,
        target_position: f64,
        policy: PlaybackBarrierPolicy,
        load_intent: MediaLoadIntent,
    ) -> Self {
        Self::new(0, logical_media_id, target_position, policy)
            .with_request_nonce(request_nonce)
            .with_load_intent(load_intent)
    }

    pub fn with_request_nonce(mut self, request_nonce: u64) -> Self {
        self.request_nonce = request_nonce;
        self
    }

    pub fn with_load_intent(mut self, load_intent: MediaLoadIntent) -> Self {
        self.load_intent = load_intent;
        self
    }

    pub fn with_quorum(mut self, quorum: u32) -> Self {
        self.quorum = Some(quorum);
        self
    }

    pub fn with_quorum_percent(mut self, quorum_percent: u32) -> Self {
        self.quorum_percent = Some(quorum_percent);
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    pub fn with_timeout_action(mut self, timeout_action: PlaybackBarrierTimeoutAction) -> Self {
        self.timeout_action = Some(timeout_action);
        self
    }

    pub fn with_deadline(mut self, deadline: f64) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReadyPayload {
    pub media_generation: u64,
    pub loaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seekable: Option<bool>,
    pub buffer_ready: bool,
}

impl MediaReadyPayload {
    pub fn new(media_generation: u64, loaded: bool, buffer_ready: bool) -> Self {
        Self {
            media_generation,
            loaded,
            seekable: None,
            buffer_ready,
        }
    }

    pub fn with_seekable(mut self, seekable: bool) -> Self {
        self.seekable = Some(seekable);
        self
    }

    pub fn is_ready(&self) -> bool {
        self.loaded && self.buffer_ready
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitStartPayload {
    pub media_generation: u64,
    pub state_revision: u64,
    pub anchor_position: f64,
    pub anchor_server_time: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<f64>,
    pub started_deadline: f64,
}

impl CommitStartPayload {
    pub fn new(
        media_generation: u64,
        state_revision: u64,
        anchor_position: f64,
        anchor_server_time: f64,
        started_deadline: f64,
    ) -> Self {
        Self {
            media_generation,
            state_revision,
            anchor_position,
            anchor_server_time,
            start_at: None,
            started_deadline,
        }
    }

    pub fn with_start_at(mut self, start_at: f64) -> Self {
        self.start_at = Some(start_at);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedAckPayload {
    pub media_generation: u64,
    pub state_revision: u64,
    pub observed_position: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<f64>,
}

impl StartedAckPayload {
    pub fn new(media_generation: u64, state_revision: u64, observed_position: f64) -> Self {
        Self {
            media_generation,
            state_revision,
            observed_position,
            observed_at: None,
        }
    }

    pub fn with_observed_at(mut self, observed_at: f64) -> Self {
        self.observed_at = Some(observed_at);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackBarrierPhase {
    Preparing,
    Committed,
    AwaitingDecision,
    Complete,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackBarrierParticipantPhase {
    Pending,
    Ready,
    Started,
    Degraded,
    PrepareTimedOut,
    StartedAckTimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackBarrierDegradedReason {
    PrepareTimeout,
    NotReadyAtCommit,
    StartedTimeout,
    Disconnected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackBarrierParticipantStatus {
    pub phase: PlaybackBarrierParticipantPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<MediaReadyPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<PlaybackBarrierDegradedReason>,
}

impl PlaybackBarrierParticipantStatus {
    pub fn pending() -> Self {
        Self {
            phase: PlaybackBarrierParticipantPhase::Pending,
            readiness: None,
            observed_position: None,
            degraded_reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackBarrierStatusPayload {
    pub media_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<u64>,
    pub phase: PlaybackBarrierPhase,
    pub policy: PlaybackBarrierPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
    pub deadline: f64,
    pub participants: BTreeMap<String, PlaybackBarrierParticipantStatus>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub excluded_legacy_clients: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackBarrierSetExtension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare: Option<PrepareMediaPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitStartPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PlaybackBarrierStatusPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffering_policy: Option<RoomBufferingPolicyPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffering_status: Option<RoomBufferingStatusPayload>,
}

impl PlaybackBarrierSetExtension {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prepare(mut self, prepare: PrepareMediaPayload) -> Self {
        self.prepare = Some(prepare);
        self
    }

    pub fn with_commit(mut self, commit: CommitStartPayload) -> Self {
        self.commit = Some(commit);
        self
    }

    pub fn with_status(mut self, status: PlaybackBarrierStatusPayload) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_buffering_policy(mut self, policy: RoomBufferingPolicyPayload) -> Self {
        self.buffering_policy = Some(policy);
        self
    }

    pub fn with_buffering_status(mut self, status: RoomBufferingStatusPayload) -> Self {
        self.buffering_status = Some(status);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackBarrierStateExtension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<MediaReadyPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<StartedAckPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportBufferingReportPayload>,
}

impl PlaybackBarrierStateExtension {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ready(mut self, ready: MediaReadyPayload) -> Self {
        self.ready = Some(ready);
        self
    }

    pub fn with_started(mut self, started: StartedAckPayload) -> Self {
        self.started = Some(started);
        self
    }

    pub fn with_transport(mut self, transport: TransportBufferingReportPayload) -> Self {
        self.transport = Some(transport);
        self
    }
}

pub(crate) fn insert_extension<T: Serialize>(extra: &mut BTreeMap<String, Value>, extension: &T) {
    let value = serde_json::to_value(extension)
        .expect("playback barrier extension types must serialize to JSON");
    extra.insert(SOROTTE_PLAYBACK_BARRIER_V1.to_owned(), value);
}

pub(crate) fn decode_extension<T: DeserializeOwned>(
    extra: &BTreeMap<String, Value>,
) -> serde_json::Result<Option<T>> {
    extra
        .get(SOROTTE_PLAYBACK_BARRIER_V1)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
}
