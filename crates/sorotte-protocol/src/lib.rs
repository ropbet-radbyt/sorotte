use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sorotte_secret::SecretValue;

mod chat;
mod codec;
mod envelope;
mod hello;
mod list;
mod message;
mod playback_barrier;
mod readiness;
mod redacted_debug;
mod room;
mod set;
mod state;

pub use chat::{ChatMessagePayload, ChatPayload, ErrorPayload, TlsPayload};
pub use codec::{
    DEFAULT_MAX_PROTOCOL_LINE_BYTES, DecodedMessageLineItem, ProtocolError, decode_line,
    decode_message_line, decode_message_line_items, decode_message_lines, encode_line,
    encode_message_line, extract_hello, extract_hello_from_message,
};
pub use envelope::{
    ChatMessage, ErrorMessage, HelloMessage, ListMessage, SetMessage, StateMessage, TlsMessage,
};
pub use hello::HelloPayload;
pub use list::{ListPayload, ListUserEntry};
pub use message::ProtocolMessage;
pub use playback_barrier::{
    CommitStartPayload, MediaLoadIntent, MediaReadyPayload, PlaybackBarrierDegradedReason,
    PlaybackBarrierParticipantPhase, PlaybackBarrierParticipantStatus, PlaybackBarrierPhase,
    PlaybackBarrierPolicy, PlaybackBarrierRecoveryDisposition, PlaybackBarrierRecoveryPayload,
    PlaybackBarrierRequestResultPayload, PlaybackBarrierRequestResultStatus,
    PlaybackBarrierSetExtension, PlaybackBarrierStateExtension, PlaybackBarrierStatusPayload,
    PlaybackBarrierTimeoutAction, PrepareMediaPayload, RoomBufferingPhase, RoomBufferingPolicy,
    RoomBufferingPolicyPayload, RoomBufferingStatusPayload, SOROTTE_PLAYBACK_BARRIER_V1,
    StartedAckPayload, TransportBufferingReportPayload,
};
pub use readiness::{
    DirectReadinessSurface, ParticipantReadiness, ParticipantReadinessUpdate,
    PlayerInteractionSurface, PlayerReadinessAction, ReadinessIntentRequest,
    ReadinessMutationMetadata, ReadinessMutationSource, ReadinessRequestResultPayload,
    ReadinessRequestResultStatus, ReadinessSetExtension, ReadinessStateExtension, RecoveryStage,
    RoomPauseOwner, RoomReadinessSnapshot, RoomStartGatePhase, SOROTTE_READINESS_V2,
    StartGateDegradedReason, StartParticipationRole, TechnicalBlockCause, TechnicalPlayability,
    TechnicalPlayabilityPhase, TechnicalPlayabilitySummary, TechnicalReadinessBlock,
    TechnicalReadinessReport, UserReadinessIntent, UserReadinessMutationSource,
};
pub use room::RoomRef;
pub use set::{
    ControllerAuthPayload, FilePayload, NewControlledRoomPayload, PlaylistChangePayload,
    PlaylistIndexPayload, ReadyPayload, SOROTTE_PLEX_PLAYLIST_URIS_FEATURE,
    SOROTTE_PLEX_PLAYLIST_URIS_KEY, SetPayload, UserSetPayload,
    canonical_playlist_files_from_change, is_sorotte_plex_playlist_uri,
    playlist_change_with_plex_sidecar, split_playlist_files_for_syncplay,
    syncplay_playlist_file_name,
};
pub use state::{IgnoringOnTheFlyPayload, PingPayload, PlaystatePayload, StatePayload};

#[cfg(test)]
mod tests;
