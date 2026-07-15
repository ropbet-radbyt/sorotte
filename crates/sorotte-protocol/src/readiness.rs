use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Hello feature and nested Set/State extension key for the additive Sorotte
/// readiness-intent protocol.
pub const SOROTTE_READINESS_V2: &str = "sorotteReadinessV2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserReadinessIntent {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DirectReadinessSurface {
    GuiButton,
    GuiMenu,
    CliCommand,
    ChatCommand,
    KeyboardShortcut,
    RemoteControlSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerReadinessAction {
    Play,
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerInteractionSurface {
    SorottePlaybackControl,
    NativePlayerControl,
    MediaKey,
    PlayerIpcUserCommand,
}

/// A client-assertable readiness source. Controller identity is deliberately
/// absent: a target username requests an override, while the server derives
/// and publishes the authenticated actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UserReadinessMutationSource {
    /// Explicit room-entry policy such as a configured ready-at-start value.
    /// This is distinct from a user operating a readiness control after join.
    Initialization,
    DirectUser {
        surface: DirectReadinessSurface,
    },
    IndirectPlayer {
        action: PlayerReadinessAction,
        surface: PlayerInteractionSurface,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TechnicalBlockCause {
    Loading,
    Seeking,
    Prebuffering,
    Rebuffering,
    CachePause,
    RoomBufferingPolicy,
    TransportRefresh,
    MediaGenerationReplacement,
    AdapterReplacement,
    Recovery,
    EndOfFile,
    MediaUnavailable,
    MediaMappingUnavailable,
    PlayerFailure,
    AdapterFailure,
    RecoveryExhausted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryStage {
    NotStarted,
    Waiting,
    Retrying,
    ReloadingMedia,
    RestartingPlayer,
    ReplacingAdapter,
}

/// Canonical source published by the server. Unlike
/// [`UserReadinessMutationSource`], this includes authenticated controller and
/// system-authored transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReadinessMutationSource {
    DirectUser {
        surface: DirectReadinessSurface,
    },
    IndirectPlayer {
        action: PlayerReadinessAction,
        surface: PlayerInteractionSurface,
    },
    ControllerOverride {
        actor: String,
    },
    SystemTechnical {
        reason: TechnicalBlockCause,
    },
    Initialization,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessMutationMetadata {
    pub source: ReadinessMutationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub room_readiness_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_observed_at: Option<f64>,
}

impl std::fmt::Debug for ReadinessMutationMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadinessMutationMetadata")
            .field("source", &self.source)
            .field("actor", &self.actor)
            .field(
                "operation_id",
                &self.operation_id.as_ref().map(|_| "<redacted>"),
            )
            .field("room_readiness_revision", &self.room_readiness_revision)
            .field("server_observed_at", &self.server_observed_at)
            .finish()
    }
}

impl ReadinessMutationMetadata {
    pub fn new(source: ReadinessMutationSource, room_readiness_revision: u64) -> Self {
        Self {
            source,
            actor: None,
            operation_id: None,
            room_readiness_revision,
            server_observed_at: None,
        }
    }

    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    pub fn with_operation_id(mut self, operation_id: impl Into<String>) -> Self {
        self.operation_id = Some(operation_id.into());
        self
    }

    pub fn with_server_observed_at(mut self, server_observed_at: f64) -> Self {
        self.server_observed_at = Some(server_observed_at);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TechnicalPlayabilityPhase {
    Unknown,
    Preparing,
    Playable,
    TemporarilyBlocked,
    TerminallyBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TechnicalPlayability {
    Unknown,
    Preparing {
        media_generation: u64,
    },
    Playable {
        media_generation: u64,
    },
    TemporarilyBlocked {
        media_generation: u64,
        cause: TechnicalBlockCause,
        recovery: RecoveryStage,
    },
    TerminallyBlocked {
        media_generation: u64,
        cause: TechnicalBlockCause,
    },
}

impl TechnicalPlayability {
    pub fn phase(&self) -> TechnicalPlayabilityPhase {
        match self {
            Self::Unknown => TechnicalPlayabilityPhase::Unknown,
            Self::Preparing { .. } => TechnicalPlayabilityPhase::Preparing,
            Self::Playable { .. } => TechnicalPlayabilityPhase::Playable,
            Self::TemporarilyBlocked { .. } => TechnicalPlayabilityPhase::TemporarilyBlocked,
            Self::TerminallyBlocked { .. } => TechnicalPlayabilityPhase::TerminallyBlocked,
        }
    }

    pub fn media_generation(&self) -> Option<u64> {
        match self {
            Self::Unknown => None,
            Self::Preparing { media_generation }
            | Self::Playable { media_generation }
            | Self::TemporarilyBlocked {
                media_generation, ..
            }
            | Self::TerminallyBlocked {
                media_generation, ..
            } => Some(*media_generation),
        }
    }

    pub fn summary(&self) -> TechnicalPlayabilitySummary {
        TechnicalPlayabilitySummary::from(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalPlayabilitySummary {
    pub phase: TechnicalPlayabilityPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<TechnicalBlockCause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryStage>,
}

impl From<&TechnicalPlayability> for TechnicalPlayabilitySummary {
    fn from(playability: &TechnicalPlayability) -> Self {
        let (reason, recovery) = match playability {
            TechnicalPlayability::TemporarilyBlocked {
                cause, recovery, ..
            } => (Some(*cause), Some(*recovery)),
            TechnicalPlayability::TerminallyBlocked { cause, .. } => (Some(*cause), None),
            TechnicalPlayability::Unknown
            | TechnicalPlayability::Preparing { .. }
            | TechnicalPlayability::Playable { .. } => (None, None),
        };
        Self {
            phase: playability.phase(),
            media_generation: playability.media_generation(),
            reason,
            recovery,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalReadinessBlock {
    pub media_generation: u64,
    pub cause: TechnicalBlockCause,
}

impl TechnicalReadinessBlock {
    pub fn new(media_generation: u64, cause: TechnicalBlockCause) -> Self {
        Self {
            media_generation,
            cause,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StartParticipationRole {
    #[default]
    Required,
    Spectator,
    ExcludedLegacy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(
    tag = "owner",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RoomPauseOwner {
    #[default]
    None,
    User {
        actor: String,
    },
    ReadinessStartGate {
        media_generation: u64,
    },
    RoomBufferingPolicy {
        media_generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state_revision: Option<u64>,
    },
    Recovery,
    EndOfPlaylist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartGateDegradedReason {
    Superseded,
    ReadinessChanged,
    TechnicalFailure,
    UserPaused,
    PauseOwnershipLost,
    Cancelled,
    TimedOut,
    NoRequiredParticipants,
    IncompatibleLegacyParticipant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RoomStartGatePhase {
    #[default]
    Inactive,
    WaitingForIntent {
        media_generation: u64,
    },
    WaitingForTechnicalReadiness {
        media_generation: u64,
    },
    ReadyToCommit {
        media_generation: u64,
        readiness_revision: u64,
    },
    Committed {
        media_generation: u64,
        readiness_revision: u64,
        playback_revision: u64,
    },
    Degraded {
        media_generation: u64,
        reason: StartGateDegradedReason,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessIntentRequest {
    pub operation_id: String,
    pub request_nonce: u64,
    pub membership_epoch: u64,
    pub desired: UserReadinessIntent,
    pub source: UserReadinessMutationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

impl std::fmt::Debug for ReadinessIntentRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadinessIntentRequest")
            .field("operation_id", &"<redacted>")
            .field("request_nonce", &self.request_nonce)
            .field("membership_epoch", &self.membership_epoch)
            .field("desired", &self.desired)
            .field("source", &self.source)
            .field("target_username", &self.target_username)
            .field("expected_revision", &self.expected_revision)
            .finish()
    }
}

impl ReadinessIntentRequest {
    pub fn new(
        operation_id: impl Into<String>,
        request_nonce: u64,
        membership_epoch: u64,
        desired: UserReadinessIntent,
        source: UserReadinessMutationSource,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            request_nonce,
            membership_epoch,
            desired,
            source,
            target_username: None,
            expected_revision: None,
        }
    }

    pub fn with_target_username(mut self, target_username: impl Into<String>) -> Self {
        self.target_username = Some(target_username.into());
        self
    }

    pub fn with_expected_revision(mut self, expected_revision: u64) -> Self {
        self.expected_revision = Some(expected_revision);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalReadinessReport {
    pub media_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_state_revision: Option<u64>,
    pub phase: TechnicalPlayabilityPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<TechnicalBlockCause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<f64>,
}

impl TechnicalReadinessReport {
    pub fn new(media_generation: u64, phase: TechnicalPlayabilityPhase) -> Self {
        Self {
            media_generation,
            playback_state_revision: None,
            phase,
            reason: None,
            recovery: None,
            observed_at: None,
        }
    }

    pub fn with_playback_state_revision(mut self, playback_state_revision: u64) -> Self {
        self.playback_state_revision = Some(playback_state_revision);
        self
    }

    pub fn with_reason(mut self, reason: TechnicalBlockCause) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn with_recovery(mut self, recovery: RecoveryStage) -> Self {
        self.recovery = Some(recovery);
        self
    }

    pub fn with_observed_at(mut self, observed_at: f64) -> Self {
        self.observed_at = Some(observed_at);
        self
    }
}

/// Canonical server-side participant record. The full playability state is
/// useful to reducers; participant fanout uses its compact summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantReadiness {
    pub membership_epoch: u64,
    pub user_intent: UserReadinessIntent,
    pub user_intent_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_mutation: Option<ReadinessMutationMetadata>,
    pub technical_state: TechnicalPlayability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_technical_block: Option<TechnicalReadinessBlock>,
    #[serde(default)]
    pub participation_role: StartParticipationRole,
    pub room_ready: bool,
    pub start_eligible: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantReadinessUpdate {
    pub room_readiness_revision: u64,
    pub membership_epoch: u64,
    pub username: String,
    pub user_intent: UserReadinessIntent,
    pub user_intent_revision: u64,
    pub user_intent_source: ReadinessMutationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_mutation: Option<ReadinessMutationMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_technical_block: Option<TechnicalReadinessBlock>,
    pub technical_state: TechnicalPlayabilitySummary,
    #[serde(default)]
    pub participation_role: StartParticipationRole,
    pub room_ready: bool,
    pub start_eligible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_operation_id: Option<String>,
}

impl std::fmt::Debug for ParticipantReadinessUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParticipantReadinessUpdate")
            .field("room_readiness_revision", &self.room_readiness_revision)
            .field("membership_epoch", &self.membership_epoch)
            .field("username", &self.username)
            .field("user_intent", &self.user_intent)
            .field("user_intent_revision", &self.user_intent_revision)
            .field("user_intent_source", &self.user_intent_source)
            .field("last_user_mutation", &self.last_user_mutation)
            .field("terminal_technical_block", &self.terminal_technical_block)
            .field("technical_state", &self.technical_state)
            .field("participation_role", &self.participation_role)
            .field("room_ready", &self.room_ready)
            .field("start_eligible", &self.start_eligible)
            .field(
                "accepted_operation_id",
                &self.accepted_operation_id.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomReadinessSnapshot {
    pub room_readiness_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_generation: Option<u64>,
    pub start_gate_phase: RoomStartGatePhase,
    #[serde(default)]
    pub pause_owner: RoomPauseOwner,
    pub participants: BTreeMap<String, ParticipantReadinessUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadinessRequestResultStatus {
    Accepted,
    Duplicate,
    Superseded,
    RejectedStaleMembership,
    RejectedStaleNonce,
    RejectedRevisionConflict,
    RejectedUnauthorized,
    RejectedInvalid,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessRequestResultPayload {
    pub operation_id: String,
    pub request_nonce: u64,
    pub status: ReadinessRequestResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_readiness_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership_epoch: Option<u64>,
}

impl std::fmt::Debug for ReadinessRequestResultPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadinessRequestResultPayload")
            .field("operation_id", &"<redacted>")
            .field("request_nonce", &self.request_nonce)
            .field("status", &self.status)
            .field("room_readiness_revision", &self.room_readiness_revision)
            .field("membership_epoch", &self.membership_epoch)
            .finish()
    }
}

impl ReadinessRequestResultPayload {
    pub fn new(
        operation_id: impl Into<String>,
        request_nonce: u64,
        status: ReadinessRequestResultStatus,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            request_nonce,
            status,
            room_readiness_revision: None,
            membership_epoch: None,
        }
    }

    pub fn with_room_readiness_revision(mut self, room_readiness_revision: u64) -> Self {
        self.room_readiness_revision = Some(room_readiness_revision);
        self
    }

    pub fn with_membership_epoch(mut self, membership_epoch: u64) -> Self {
        self.membership_epoch = Some(membership_epoch);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessSetExtension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<ReadinessIntentRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<ParticipantReadinessUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<RoomReadinessSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_result: Option<ReadinessRequestResultPayload>,
}

impl ReadinessSetExtension {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_intent(mut self, intent: ReadinessIntentRequest) -> Self {
        self.intent = Some(intent);
        self
    }

    pub fn with_participant(mut self, participant: ParticipantReadinessUpdate) -> Self {
        self.participant = Some(participant);
        self
    }

    pub fn with_snapshot(mut self, snapshot: RoomReadinessSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    pub fn with_request_result(mut self, request_result: ReadinessRequestResultPayload) -> Self {
        self.request_result = Some(request_result);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessStateExtension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<TechnicalReadinessReport>,
}

impl ReadinessStateExtension {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_technical(mut self, technical: TechnicalReadinessReport) -> Self {
        self.technical = Some(technical);
        self
    }
}

pub(crate) fn insert_extension<T: Serialize>(extra: &mut BTreeMap<String, Value>, extension: &T) {
    let value = serde_json::to_value(extension)
        .expect("readiness V2 extension types must serialize to JSON");
    extra.insert(SOROTTE_READINESS_V2.to_owned(), value);
}

pub(crate) fn decode_extension<T: DeserializeOwned>(
    extra: &BTreeMap<String, Value>,
) -> serde_json::Result<Option<T>> {
    extra
        .get(SOROTTE_READINESS_V2)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
}
