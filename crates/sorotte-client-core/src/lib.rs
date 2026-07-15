use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::Md5;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sorotte_core::SyncDomain;
use sorotte_media_match::{MEDIA_MATCH_FILE_PAYLOAD_KEY, MediaMatchTier, MediaMatchWireSignature};
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
};
use sorotte_protocol::{
    ChatPayload, ControllerAuthPayload, FilePayload, IgnoringOnTheFlyPayload, ListPayload,
    ParticipantReadinessUpdate, PingPayload, PlayerInteractionSurface, PlaylistIndexPayload,
    PlaystatePayload, ProtocolError, ProtocolMessage, ReadinessIntentRequest,
    ReadinessRequestResultStatus, ReadinessSetExtension, ReadinessStateExtension, ReadyPayload,
    RoomReadinessSnapshot, RoomRef, SOROTTE_PLEX_PLAYLIST_URIS_FEATURE, SOROTTE_READINESS_V2,
    SetPayload, StatePayload, TechnicalReadinessReport, UserReadinessIntent,
    UserReadinessMutationSource, canonical_playlist_files_from_change, decode_message_line,
    decode_message_line_items, encode_message_line, playlist_change_with_plex_sidecar,
};
use sorotte_secret::SecretValue;

const SEEK_THRESHOLD_SECONDS: f64 = 1.0;
const DEFAULT_REWIND_THRESHOLD_SECONDS: f64 = 4.0;
const DEFAULT_FASTFORWARD_THRESHOLD_SECONDS: f64 = 5.0;
const FASTFORWARD_BEHIND_THRESHOLD_SECONDS: f64 = 1.75;
const FASTFORWARD_EXTRA_SECONDS: f64 = 0.25;
const FASTFORWARD_RESET_THRESHOLD_SECONDS: f64 = 3.0;
const DEFAULT_SLOWDOWN_THRESHOLD_SECONDS: f64 = 1.5;
const SLOWDOWN_RESET_THRESHOLD_SECONDS: f64 = 0.1;
const SLOWDOWN_RATE: f64 = 0.95;
const NORMAL_PLAYBACK_RATE: f64 = 1.0;
const DEFAULT_MAX_RECONNECT_RETRIES: u32 = 999;
const DEFAULT_RECONNECT_BASE_DELAY_SECONDS: f64 = 0.1;
const DEFAULT_RECONNECT_BACKOFF_MAX_EXPONENT: u32 = 5;
const DEFAULT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS: f64 = 2.0;
const DEFAULT_AUTOPLAY_DELAY_SECONDS: f64 = 3.0;
const AUTOPLAY_COUNTDOWN_STEP_SECONDS: f64 = 1.0;
const RECENTLY_ADVANCED_GRACE_SECONDS: f64 = 5.0;
const RECENT_REWIND_SEEK_SUPPRESSION_SECONDS: f64 = 1.0;
const RECENT_REWIND_SEEK_IGNORE_POSITION_SECONDS: f64 = 5.0;
const LEGACY_SHOW_DURATION_NOTIFICATION: bool = true;
const LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS: f64 = 2.5;
const LEGACY_CHAT_MAX_MESSAGE_LENGTH: usize = 150;
const LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH: usize = 50;
const LEGACY_FALLBACK_MAX_USERNAME_LENGTH: usize = 16;
const LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH: usize = 35;
const LEGACY_FALLBACK_MAX_FILENAME_LENGTH: usize = 250;
pub const SYNCPLAY_WIRE_VERSION_LEGACY: &str = "1.2.255";
pub const SYNCPLAY_COMPAT_VERSION_LEGACY: &str = "1.7.5";
const LEGACY_CHAT_MIN_VERSION: &str = "1.5.0";
const LEGACY_USER_READY_MIN_VERSION: &str = "1.3.0";
const LEGACY_MANAGED_ROOMS_MIN_VERSION: &str = "1.3.0";
const LEGACY_SHARED_PLAYLIST_MIN_VERSION: &str = "1.4.0";
const MAX_PENDING_LOCAL_PLAYLIST_ECHOES: usize = 64;
const LEGACY_SET_OTHERS_READINESS_MIN_VERSION: &str = "1.7.2";
const LEGACY_SHOW_SAME_ROOM_OSD: bool = true;
const LEGACY_SHOW_OSD_WARNINGS: bool = true;
const LEGACY_SHOW_NONCONTROLLER_OSD: bool = false;
const LEGACY_SHOW_DIFFERENT_ROOM_OSD: bool = false;
const LEGACY_ONLY_SWITCH_TO_TRUSTED_DOMAINS: bool = true;
const LEGACY_DEFAULT_TRUSTED_DOMAINS: [&str; 2] = ["youtube.com", "youtu.be"];
const DEFAULT_RECONNECT_STATE_RESTORE_AUTOCORRECT: bool = true;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS: u32 = 1;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS: u32 = 8;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES: u32 = 0;
const ROUND_HALF_EPSILON: f64 = 1e-12;
const PRIVACY_HIDDEN_FILENAME: &str = "**Hidden filename**";
const MUSIC_FORMATS: [&str; 8] = [
    ".mp3", ".m4a", ".m4p", ".wav", ".aiff", ".r", ".ogg", ".flac",
];
pub const AUTOPLAY_TICK_INTERVAL_SECONDS: f64 = AUTOPLAY_COUNTDOWN_STEP_SECONDS;
const LEGACY_PING_MOVING_AVERAGE_WEIGHT: f64 = 0.85;

pub fn legacy_server_password_token(password: &str) -> String {
    format!("{:x}", Md5::digest(password.as_bytes()))
}

/// Builds the privacy-safe, source-stable identity used by playback
/// coordination. Local paths are deliberately excluded so peers with
/// different library roots still agree; known YouTube URL forms collapse to
/// the video ID so harmless URL/query variations do not create generations.
pub fn logical_media_id_for_local_file_update(update: &LocalFileUpdate) -> LogicalMediaId {
    let source_identity = youtube_video_id(&update.name)
        .or_else(|| update.path.as_deref().and_then(youtube_video_id))
        .map(|video_id| format!("youtube:{video_id}"))
        .unwrap_or_else(|| update.name.trim().to_owned());
    let mut digest = Sha256::new();
    digest.update(b"sorotte-logical-media-v1\0");
    digest.update(source_identity.as_bytes());
    if !source_identity.starts_with("youtube:") {
        digest.update(b"\0");
        digest.update(update.size_bytes.unwrap_or_default().to_le_bytes());
    }
    LogicalMediaId::new(format!("media-sha256:{:x}", digest.finalize()))
        .expect("SHA-256 logical media identity is non-empty")
}

fn youtube_video_id(value: &str) -> Option<&str> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let host_start = lower.find("://").map_or(0, |index| index + 3);
    let host_and_path = &value[host_start..];
    let lower_host_and_path = &lower[host_start..];
    let host_end = lower_host_and_path
        .find(['/', '?', '#'])
        .unwrap_or(lower_host_and_path.len());
    let host = lower_host_and_path[..host_end]
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let path_and_query = &host_and_path[host_end..];

    let candidate = if host == "youtu.be" {
        path_and_query
            .trim_start_matches('/')
            .split(['?', '#'])
            .next()
    } else if matches!(host, "youtube.com" | "music.youtube.com") {
        let path = path_and_query.split(['?', '#']).next().unwrap_or_default();
        let path_candidate = ["/shorts/", "/embed/", "/live/"]
            .into_iter()
            .find_map(|prefix| {
                path.strip_prefix(prefix)
                    .and_then(|rest| rest.split('/').next())
            });
        path_candidate.or_else(|| {
            path_and_query
                .split_once('?')
                .map(|(_, query)| query)
                .into_iter()
                .flat_map(|query| query.split('&'))
                .find_map(|part| part.strip_prefix("v="))
        })
    } else {
        None
    }?;
    let candidate = candidate.trim();
    (!candidate.is_empty()
        && candidate.len() <= 64
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    .then_some(candidate)
}

mod config;
mod control;
mod inbound;
mod model;
mod notifications;
mod outbox;
mod ping;
mod playback_coordinator;
mod player_transition;
mod runtime;
mod session;
mod views;

pub use self::config::{
    ChatConfig, DesyncCorrectionAction, DesyncCorrectionConfig, PrivacyMode,
    ReadinessAutoplayConfig, ReconnectPolicyConfig, ReconnectRetryDecision,
    ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionPolicyMode,
    ReconnectStateRestoreCorrectionStateSnapshot, SessionBehaviorConfig, UnpauseActionMode,
};
pub use self::control::{
    ClientEffect, ClientEffectError, ClientEffectSink, ClientRuntimeAction, PendingProtocolLine,
    PlaybackBarrierRequestScope, QueuedRuntimeControl, ReadinessIntentScope,
};
pub use self::inbound::{
    ClientCompatibilityFallback, FileDuration, FileSize, PeerCapabilities, SharedFile,
};
pub(crate) use self::inbound::{
    ClientHello, ClientInboundCommand, ClientListUser, ClientPlaystate, ClientSetCommand,
    ClientStateUpdate, normalize_client_protocol_message, normalize_client_state_payload,
};
pub use self::model::{
    ClientEvent, ClientModel, ConnectionPhase, ConnectionState, ControllerState,
    LocalPauseChangeHealth, PendingReadinessIntent, PlaybackSyncState, PlaylistState,
    ReadinessState, ReconnectState, RoomState, ServerCapabilities,
};
pub use self::notifications::{
    AutoplayCountdownNotification, ChatNotification, ControlledRoomCreationNotification,
    ControllerAuthTransitionNotification, FileDifferenceSummary, ReconnectPlaylistRestoreIntent,
    ReconnectTransitionNotification, UserChangeNotification,
};
pub use self::outbox::ProtocolLineLease;
pub use self::ping::ClientPingMetricsLegacyCompatible;
pub(crate) use self::ping::unix_wall_clock_time_seconds_legacy_compatible;
pub use self::playback_coordinator::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DegradedPlaybackReason, DesiredRoomPlayback,
    DesiredRoomPlaybackUpdateKind, LogicalMediaId, MediaLoadIntent, MediaLoadPlan,
    MediaTransportKind, PlaybackCoordinator, PlaybackCoordinatorAction, PlaybackCoordinatorConfig,
    PlaybackCoordinatorMetrics, PlaybackDiagnostic, PlayerTransportObservation,
    RecoveryEpisodeSnapshot, RecoveryPolicy, SeekPreparationDegradedReason, SeekPreparationPhase,
    SeekPreparationSnapshot, SeekPreparationTerminalOutcome, SeekTargetAvailability,
};
pub use self::player_transition::{
    NativePlayerAction, PlayerCommandCause, PlayerCommandCompletion, PlayerCommandRegistration,
    PlayerLogicalPauseObservation, PlayerTransitionClassification, PlayerTransitionClassifier,
    PlayerTransitionClassifierConfig, PlayerTransitionContext, PlayerTransitionIgnoredReason,
    PlayerTransitionTechnicalReason, PlayerTransitionUnknownReason,
};
pub use self::runtime::{
    ClientPlayerIo, ClientRuntime, ClientSessionUpdate, PlaybackBarrierRoomBufferingConfig,
    PlaybackBarrierStartConfig, PlaybackBarrierTimeoutAction, PlaybackCoordinationSnapshot,
};
pub use self::session::ClientSession;
pub(crate) use self::session::ClientSessionLocalActionSnapshot;
pub use self::views::{
    ClientMediaMatchPeerFileState, ClientUserView, RoomPlaylistView, RoomPlaystateAuthority,
    RoomPlaystateView,
};
