#[cfg(test)]
mod command_ack_tests;
mod player_adapter;
mod state;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    path::{Path, PathBuf},
    process,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sorotte_player_api::{
    LoadAttemptId, LocalFileUpdate, PlayerActiveLoadSnapshot, PlayerAdapter, PlayerAttachmentEpoch,
    PlayerAuthoritativeSnapshot, PlayerCacheTelemetryUpdate, PlayerCommandFailureKind,
    PlayerCommandId, PlayerCommandProgress, PlayerCommandResult, PlayerError, PlayerEventSequence,
    PlayerLocalFileObservation, PlayerMediaGeneration, PlayerMediaLoadFailureKind,
    PlayerMediaLoadObservation, PlayerMediaLoadOutcome, PlayerObservationBatch,
    PlayerObservationTimestamp, PlayerOrderedEvent, PlayerOrderedEventKind,
    PlayerPhysicalLoadOutcome, PlayerPlayIntent, PlayerPlaybackTelemetryUpdate,
    PlayerSeekableRange, PlayerSequenceBoundary, PlayerTimelineKind, PlayerTransportDelta,
    PlayerTransportPhase, PlayerTransportSnapshot, PlayerTransportTelemetryUpdate, SnapshotField,
};
use sorotte_secret::SecretValue;

use self::state::MpvObservedState;
use crate::bridge::{SorotteBridgeFailure, SorotteBridgeFailureKind, SorotteBridgeHealth};
use crate::bridge_resource::{
    materialize_bundled_sorotte_bridge, materialize_bundled_sorotte_network_options_hook,
};
use crate::constants::*;
#[cfg(test)]
use crate::ipc::MpvJsonIpcTransport;
use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use crate::legacy_ui::{
    LegacySyncplayOsdKind, LegacySyncplayUiSettings, sanitize_legacy_syncplay_script_message_text,
};
use crate::lifecycle::{
    AuthoritativePlaylistEntry, PlayerLifecycleEffect, PlayerLifecycleInput, PlayerLifecycleState,
    reduce_player_lifecycle,
};
use crate::transcript::{MpvTranscript, MpvTranscriptError, MpvTranscriptRecorder};

const PAUSED_POSITION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES: usize = 64;
const MAX_PENDING_COMMAND_PROGRESS_UPDATES: usize = 128;
const MAX_PENDING_ORDERED_PLAYER_EVENTS: usize = 256;
const MAX_UNACKNOWLEDGED_MEDIA_LOAD_OUTCOMES: usize = MAX_PENDING_ORDERED_PLAYER_EVENTS;
const MAX_PENDING_NETWORK_MEDIA_OPTIONS_TRANSITION_OUTCOMES: usize = 16;
const PLAYER_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const PLAYER_LOAD_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PLAYBACK_ADVANCEMENT_EPSILON_SECONDS: f64 = 0.01;
const INTERRUPTED_NETWORK_STREAM_MINIMUM_REMAINING_SECONDS: f64 = 15.0;
const INTERRUPTED_NETWORK_STREAM_RECOVERY_PROGRESS_SECONDS: f64 = 2.0;
const MAX_CONSECUTIVE_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS: usize = 2;
const MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS: usize = 5;
const NETWORK_CACHE_STALL_RECOVERY_DELAY: Duration = Duration::from_secs(20);
const NETWORK_CACHE_STALL_RECOVERY_MARGIN: Duration = Duration::from_secs(5);
const LEGACY_SYNCPLAYINTF_OWNER_LEASE_MS: u64 = 2_000;
const LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL: Duration = Duration::from_secs(2);
const LEGACY_SYNCPLAYINTF_RUNTIME_RECOVERY_ATTEMPTS: usize = 3;
const LEGACY_SYNCPLAYINTF_DISCOVERY_ATTEMPTS: usize = 3;
const LEGACY_SYNCPLAYINTF_REGISTRATION_ATTEMPTS: usize = 20;
const LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW: Duration = Duration::from_millis(2_500);
const LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_WINDOW: Duration = Duration::from_millis(2_500);
const NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_INTERVAL: Duration = Duration::from_millis(25);
// GUI rendering, synchronous settings work, and OS scheduling can legitimately delay the
// integration pump for more than two seconds. Keep ownership failover bounded while leaving
// enough room for those transient stalls; heartbeats still run every 500 ms.
const NETWORK_OPTIONS_HOOK_OWNER_LEASE_MS: u64 = 10_000;
const NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT: Duration = Duration::from_millis(750);
const MINIMUM_SUPPORTED_MPV_VERSION_COMPONENTS: (u64, u64, u64) = (0, 41, 0);
const NETWORK_MEDIA_OPTION_READBACK_ALLOWLIST: [&str; 8] = [
    "cache",
    "cache-pause",
    "cache-pause-initial",
    "cache-pause-wait",
    "cache-secs",
    "demuxer-max-bytes",
    "demuxer-max-back-bytes",
    "cache-on-disk",
];
static NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT: AtomicU64 = AtomicU64::new(1);
static LEGACY_SYNCPLAYINTF_OWNER_ID: LazyLock<String> = LazyLock::new(|| {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("sorotte-{}-{started_at}", process::id())
});

/// Describes how applying configured network-media options affected mpv's authoritative
/// active-media state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpvActiveNetworkMediaOptionsApplyOutcome {
    /// mpv currently has no active media path.
    NoActiveMedia,
    /// mpv's active path is local, so network-only options were intentionally left unchanged.
    LocalMediaUnchanged,
    /// mpv's active path is network media and all configured file-local options were accepted.
    NetworkMediaUpdated,
    /// A newer authoritative path replaced the path being applied. Its ordered transition
    /// outcome is authoritative and will be reported separately by the adapter.
    Superseded,
}

/// Reports a change in the availability of Sorotte's mpv network-options hook.
#[derive(Debug, PartialEq, Eq)]
pub enum MpvNetworkOptionsHookHealthTransition {
    /// A previously degraded core hook was positively reconfigured or responded successfully.
    Recovered,
    /// The core hook is unavailable or this adapter lost its lease. Playback and JSON IPC remain
    /// attached, but applying network-only policy requires an explicit retry or hook recovery.
    Degraded(PlayerError),
}

/// Reports the result of applying configured options to authoritative active media.
#[derive(Debug, PartialEq, Eq)]
pub enum MpvNetworkMediaPolicyOutcome {
    /// The active media ended and there is currently no file-specific policy to apply.
    NoActiveMedia,
    /// The authoritative hook classified the active media as local, so network options are idle.
    LocalMediaUnchanged,
    /// Every configured file-local network option was accepted for the active network media.
    NetworkMediaUpdated,
    /// At least one option write failed. IPC health determines whether the failure is retryable.
    Failed(PlayerError),
}

/// Authoritative current health of Sorotte's mpv network-options hook.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MpvNetworkOptionsHookHealth {
    /// Hook configuration or ownership has not yet been positively acknowledged.
    #[default]
    Pending,
    /// Hook configuration and ownership are currently acknowledged.
    Ready,
    /// The hook is unavailable. The message is safe for user-visible diagnostics.
    Degraded(String),
}

/// Authoritative current state of active-media network-option policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MpvNetworkMediaPolicyState {
    /// No authoritative media-policy result has been observed for this attachment or generation.
    #[default]
    Unknown,
    /// An explicit apply was superseded and is waiting for the successor load's hook result.
    AwaitingAuthoritativeLoad,
    /// There is no active media requiring file-specific policy.
    NoActiveMedia,
    /// Active media is local and network-only options are intentionally idle.
    LocalMediaUnchanged,
    /// Active network media accepted every configured option.
    NetworkMediaUpdated,
    /// Active-media policy failed. The message is safe for user-visible diagnostics.
    Failed(String),
}

/// Revisioned authoritative network-options state. Consumers can reconcile this snapshot every
/// pump so a dropped notification can never leave their stored health divergent from the adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MpvNetworkOptionsRuntimeHealthSnapshot {
    pub revision: u64,
    pub hook_health: MpvNetworkOptionsHookHealth,
    pub media_policy: MpvNetworkMediaPolicyState,
}

/// Classification of one configured option in the authoritative network-policy result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpvNetworkOptionApplyStatus {
    /// mpv accepted the option and, when read-back was available, reported the desired value.
    Applied,
    /// mpv rejected the file-local option write.
    Rejected,
    /// mpv accepted the write but authoritative read-back differed or was unavailable.
    Mismatched,
}

/// Privacy-safe per-option result. Values live only in the allowlisted cache-option maps on the
/// diagnostic snapshot, so arbitrary advanced arguments cannot leak into diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MpvNetworkOptionApplyResult {
    pub name: String,
    pub status: MpvNetworkOptionApplyStatus,
}

/// Aggregate result of applying the current network policy to one media generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpvNetworkMediaPolicyApplicationState {
    Applied,
    PartiallyApplied,
    Failed,
}

/// Generation-scoped, privacy-safe network-policy and cache diagnostic state.
///
/// Media targets, URLs, local paths, credentials, and arbitrary advanced option values are never
/// retained here. Only the fixed cache-option allowlist can contribute desired/effective values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MpvNetworkMediaDiagnosticSnapshot {
    pub media_generation: Option<PlayerMediaGeneration>,
    pub network_policy_generation: u64,
    pub load_sequence: Option<u64>,
    pub application_state: Option<MpvNetworkMediaPolicyApplicationState>,
    pub verification_complete: bool,
    pub option_results: Vec<MpvNetworkOptionApplyResult>,
    pub desired_cache_options: BTreeMap<String, String>,
    pub effective_cache_options: BTreeMap<String, String>,
    pub observed_at: Option<PlayerObservationTimestamp>,
    pub transport_phase: PlayerTransportPhase,
    pub paused_for_cache: Option<bool>,
    pub demuxer_cache_idle: Option<bool>,
    pub cache_duration_seconds: Option<f64>,
    pub forward_bytes: Option<u64>,
    pub raw_input_rate_bytes_per_second: Option<u64>,
    pub reader_position_seconds: Option<f64>,
    pub cache_end_seconds: Option<f64>,
    pub cache_eof: Option<bool>,
    pub cache_underrun: Option<bool>,
}

/// Compatibility view that merges the two independent event channels in production order.
/// New consumers should use the split transition/outcome APIs and the authoritative snapshot.
#[derive(Debug, PartialEq, Eq)]
pub enum MpvNetworkMediaOptionsTransitionOutcome {
    /// A previously degraded core hook was positively reconfigured or responded successfully.
    HookRecovered,
    /// The active media ended and there is currently no file-specific policy to apply.
    NoActiveMedia,
    /// The authoritative hook classified the active media as local, so network options are idle.
    LocalMediaUnchanged,
    /// Every configured file-local network option was accepted for the active network media.
    NetworkMediaUpdated,
    /// The core hook is unavailable or this adapter lost its lease. Playback and JSON IPC remain
    /// attached, but applying network-only policy requires an explicit retry or hook recovery.
    HookDegraded(PlayerError),
    /// At least one option write failed. IPC health determines whether the failure is retryable.
    Failed(PlayerError),
}

struct SequencedNetworkOptionsEvent<T> {
    sequence: u64,
    value: T,
}

#[derive(Clone, PartialEq, Eq)]
struct NetworkMediaOptionsApplyIdentity {
    attempt_id: u64,
    media_generation: Option<PlayerMediaGeneration>,
    path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ExpectedNetworkOptionsTransition {
    media_generation: PlayerMediaGeneration,
    load_sequence: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct EmbeddedNetworkMediaOptions {
    media_generation: PlayerMediaGeneration,
    requested_target: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InterruptedNetworkStreamRecovery {
    media_generation: PlayerMediaGeneration,
    latest_attempt_id: LoadAttemptId,
    resume_position_seconds: f64,
    consecutive_attempts: usize,
    total_attempts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NetworkCacheStall {
    media_generation: PlayerMediaGeneration,
    last_progress_at: Instant,
    last_sample: NetworkCacheProgressSample,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct NetworkCacheProgressSample {
    position_seconds: Option<f64>,
    buffered_ahead_seconds: Option<f64>,
    buffered_ahead_bytes: Option<u64>,
    cache_reader_position_seconds: Option<f64>,
    cache_end_seconds: Option<f64>,
}

impl NetworkCacheProgressSample {
    fn from_observed_state(state: &MpvObservedState) -> Self {
        Self {
            position_seconds: state.position_seconds,
            buffered_ahead_seconds: state.buffered_ahead_seconds,
            buffered_ahead_bytes: state.buffered_ahead_bytes,
            cache_reader_position_seconds: state.cache_reader_position_seconds,
            cache_end_seconds: state.cache_end_seconds,
        }
    }

    fn made_progress_since(self, previous: Self) -> bool {
        fn f64_increased(current: Option<f64>, previous: Option<f64>) -> bool {
            match (current, previous) {
                (Some(current), Some(previous)) => {
                    current > previous + PLAYBACK_ADVANCEMENT_EPSILON_SECONDS
                }
                (Some(current), None) => current > PLAYBACK_ADVANCEMENT_EPSILON_SECONDS,
                _ => false,
            }
        }

        fn u64_increased(current: Option<u64>, previous: Option<u64>) -> bool {
            match (current, previous) {
                (Some(current), Some(previous)) => current > previous,
                (Some(current), None) => current > 0,
                _ => false,
            }
        }

        f64_increased(self.position_seconds, previous.position_seconds)
            || f64_increased(self.buffered_ahead_seconds, previous.buffered_ahead_seconds)
            || u64_increased(self.buffered_ahead_bytes, previous.buffered_ahead_bytes)
            || f64_increased(
                self.cache_reader_position_seconds,
                previous.cache_reader_position_seconds,
            )
            || f64_increased(self.cache_end_seconds, previous.cache_end_seconds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NetworkOptionsHookApplyStatus {
    NoActiveMedia,
    LocalMediaUnchanged,
    NetworkMediaUpdated,
    PartiallyApplied,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkOptionsHookOptionApplyStatus {
    Applied,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkOptionsHookOptionResult {
    name: String,
    status: NetworkOptionsHookOptionApplyStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkOptionsHookActiveResult {
    attempt_id: u64,
    generation: u64,
    load_sequence: u64,
    source_path: Option<SecretValue>,
    source_kind: NetworkOptionsMediaTargetKind,
    stream_target_kind: NetworkOptionsMediaTargetKind,
    status: NetworkOptionsHookApplyStatus,
    verification_complete: bool,
    option_results: Vec<NetworkOptionsHookOptionResult>,
    effective_options: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkOptionsHookTransitionResult {
    generation: u64,
    load_sequence: u64,
    source_path: Option<SecretValue>,
    source_kind: NetworkOptionsMediaTargetKind,
    stream_target_kind: NetworkOptionsMediaTargetKind,
    status: NetworkOptionsHookApplyStatus,
    verification_complete: bool,
    option_results: Vec<NetworkOptionsHookOptionResult>,
    effective_options: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkOptionsMediaTargetKind {
    Absent,
    LocalPath,
    FileUrl,
    Http,
    Https,
    Edl,
    OtherProtocol,
}

impl NetworkOptionsMediaTargetKind {
    fn from_target(target: Option<&str>) -> Self {
        let Some(target) = target.map(str::trim).filter(|target| !target.is_empty()) else {
            return Self::Absent;
        };
        let Some((scheme, _)) = target.split_once("://") else {
            return Self::LocalPath;
        };
        match scheme.to_ascii_lowercase().as_str() {
            "file" => Self::FileUrl,
            "http" => Self::Http,
            "https" => Self::Https,
            "edl" => Self::Edl,
            _ => Self::OtherProtocol,
        }
    }
}

impl fmt::Display for NetworkOptionsMediaTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "none",
            Self::LocalPath => "local path",
            Self::FileUrl => "file URL",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Edl => "EDL",
            Self::OtherProtocol => "other protocol",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NetworkOptionsApplyDiagnostic {
    load_sequence: u64,
    source_kind: NetworkOptionsMediaTargetKind,
    stream_target_kind: NetworkOptionsMediaTargetKind,
}

impl NetworkOptionsApplyDiagnostic {
    fn player_error(
        load_sequence: u64,
        source_kind: NetworkOptionsMediaTargetKind,
        stream_target_kind: NetworkOptionsMediaTargetKind,
    ) -> PlayerError {
        PlayerError::OperationFailed(
            Self {
                load_sequence,
                source_kind,
                stream_target_kind,
            }
            .to_string(),
        )
    }
}

impl fmt::Display for NetworkOptionsApplyDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "mpv rejected a network-media option for hook load {} (source: {}, resolved target: {})",
            self.load_sequence, self.source_kind, self.stream_target_kind
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingNetworkOptionsHookHeartbeat {
    nonce: u64,
    /// Present only for heartbeats sent through the asynchronous control lane. Synchronous
    /// maintenance observes delivery directly and therefore does not need completion identity.
    command_id: Option<u64>,
    /// Set only after mpv has accepted the heartbeat command. A nonblocking IPC command can
    /// remain in flight longer than the hook acknowledgement window, so starting that window at
    /// enqueue time would falsely degrade an otherwise healthy hook.
    sent_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthoritativePathObservationOrigin {
    StartFilePending,
    PathEvent,
    Poll,
    EndFileIdle,
}

struct DeferredAuthoritativePathObservation {
    path: Option<String>,
    origin: AuthoritativePathObservationOrigin,
}

fn uses_network_media_options(path: &str) -> bool {
    let Some((scheme, _)) = path.trim().split_once("://") else {
        return false;
    };
    !scheme.eq_ignore_ascii_case("file")
}

fn classify_sorotte_bridge_configuration_failure(
    reason: &str,
    acknowledged_rejection: bool,
) -> SorotteBridgeFailureKind {
    let normalized = reason.to_ascii_lowercase();
    if normalized.contains("another sorotte owner") || normalized.contains("live bridge lease") {
        SorotteBridgeFailureKind::LeaseBusy
    } else if acknowledged_rejection {
        SorotteBridgeFailureKind::SettingsRejected
    } else if normalized.contains("json ipc")
        || normalized.contains("not connected")
        || normalized.contains("command queue")
    {
        SorotteBridgeFailureKind::IpcCommand
    } else {
        SorotteBridgeFailureKind::AcknowledgementTimeout
    }
}

#[derive(Debug)]
struct PendingTrackedCommand {
    id: PlayerCommandId,
    media_generation: Option<PlayerMediaGeneration>,
    accepted_at: Option<Instant>,
    deferred_result: Option<PlayerCommandResult>,
    kind: TrackedCommandKind,
}

#[derive(Debug)]
enum TrackedCommandKind {
    Load {
        file_loaded: bool,
        ready: bool,
    },
    Seek {
        target_seconds: f64,
        seeking_finished: bool,
        position_in_tolerance: bool,
    },
    Pause {
        logical_pause_observed: bool,
    },
    Play {
        intent: PlayerPlayIntent,
        restart_sequence_baseline: u64,
        position_baseline: Option<f64>,
        logical_play_observed: bool,
        cache_clear_observed: bool,
        restart_observed: bool,
        forward_advancement_observed: bool,
    },
}

impl TrackedCommandKind {
    fn timeout(&self) -> Duration {
        match self {
            Self::Load { .. } => PLAYER_LOAD_COMMAND_TIMEOUT,
            Self::Seek { .. } | Self::Pause { .. } | Self::Play { .. } => PLAYER_COMMAND_TIMEOUT,
        }
    }

    fn completed(&self) -> bool {
        match self {
            Self::Load { file_loaded, ready } => *file_loaded && *ready,
            Self::Seek {
                seeking_finished,
                position_in_tolerance,
                ..
            } => *seeking_finished && *position_in_tolerance,
            Self::Pause {
                logical_pause_observed,
            } => *logical_pause_observed,
            Self::Play {
                intent,
                logical_play_observed,
                cache_clear_observed,
                restart_observed,
                forward_advancement_observed,
                ..
            } => {
                let restart_satisfied =
                    matches!(intent, PlayerPlayIntent::Resume) || *restart_observed;
                *logical_play_observed
                    && *cache_clear_observed
                    && restart_satisfied
                    && *forward_advancement_observed
            }
        }
    }

    fn is_load_seek_or_play(&self) -> bool {
        matches!(
            self,
            Self::Load { .. } | Self::Seek { .. } | Self::Play { .. }
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum TrackedCommandObservation {
    FileLoaded,
    Phase(PlayerTransportPhase),
    LogicalPause(bool),
    CachePause(bool),
    Seeking(bool),
    Position(f64),
    PlaybackRestart(u64),
}

#[derive(Debug, Clone, Copy)]
enum TrackedCommandSupersession {
    Load,
    Seek,
    PauseOrPlay,
}

pub struct MpvAdapter {
    paused: bool,
    logical_pause_explicit: bool,
    position_seconds: f64,
    playback_rate: f64,
    paused_for_cache: bool,
    cache_buffering_percent: Option<f64>,
    muted: bool,
    volume: Option<f64>,
    deinterlace: bool,
    keepaspect: bool,
    keepaspect_window: bool,
    fullscreen: bool,
    ontop: bool,
    border: bool,
    force_window: bool,
    keep_open: bool,
    keep_open_pause: bool,
    cursor_autohide_fs_only: bool,
    stop_screensaver: bool,
    sub_visibility: bool,
    osd_bar: bool,
    window_maximized: bool,
    window_minimized: bool,
    current_path: Option<String>,
    network_media_options: BTreeMap<String, String>,
    network_media_options_hook_enabled: bool,
    network_media_options_hook_loaded: bool,
    network_media_options_generation: u64,
    network_media_options_hook_configured_generation: Option<u64>,
    network_media_options_hook_configuration_error: Option<String>,
    network_media_options_hook_last_heartbeat_at: Option<Instant>,
    network_media_options_hook_pending_heartbeat: Option<PendingNetworkOptionsHookHeartbeat>,
    network_media_options_hook_pending_event_poll_command_id: Option<u64>,
    next_network_media_options_hook_heartbeat_nonce: u64,
    network_media_options_hook_instance_id: Option<String>,
    network_media_options_hook_last_accepted_load_sequence: Option<u64>,
    network_media_options_hook_latest_started_load_sequence: Option<u64>,
    network_media_options_expected_transition: Option<ExpectedNetworkOptionsTransition>,
    network_media_options_hook_health: MpvNetworkOptionsHookHealth,
    network_media_options_hook_ownership_possible: bool,
    network_media_options_hook_configuration_in_progress: bool,
    network_media_options_policy_state: MpvNetworkMediaPolicyState,
    network_media_options_runtime_health_revision: u64,
    network_media_options_application_state: Option<MpvNetworkMediaPolicyApplicationState>,
    network_media_options_diagnostic_load_sequence: Option<u64>,
    network_media_options_verification_complete: bool,
    network_media_options_option_results: Vec<MpvNetworkOptionApplyResult>,
    network_media_options_effective_cache_options: BTreeMap<String, String>,
    pending_network_media_options_hook_active_result: Option<NetworkOptionsHookActiveResult>,
    deferred_network_media_options_hook_transition_result:
        Option<NetworkOptionsHookTransitionResult>,
    network_media_options_embedded_load: Option<EmbeddedNetworkMediaOptions>,
    network_media_options_apply_identity: Option<NetworkMediaOptionsApplyIdentity>,
    next_network_media_options_apply_attempt_id: u64,
    network_media_options_event_batch_depth: usize,
    deferred_network_media_options_observation: Option<DeferredAuthoritativePathObservation>,
    next_network_options_event_sequence: u64,
    pending_network_options_hook_health_transitions:
        VecDeque<SequencedNetworkOptionsEvent<MpvNetworkOptionsHookHealthTransition>>,
    pending_network_media_policy_outcomes:
        VecDeque<SequencedNetworkOptionsEvent<MpvNetworkMediaPolicyOutcome>>,
    pending_local_file_update: Option<LocalFileUpdate>,
    pending_local_file_generation: Option<PlayerMediaGeneration>,
    pending_local_file_observed_at: Option<PlayerObservationTimestamp>,
    pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
    pending_transport_telemetry_updates: VecDeque<PlayerTransportTelemetryUpdate>,
    pending_cache_telemetry_updates: VecDeque<PlayerCacheTelemetryUpdate>,
    pending_tracked_commands: VecDeque<PendingTrackedCommand>,
    pending_command_progress_updates: VecDeque<PlayerCommandProgress>,
    pending_media_load_outcomes: VecDeque<PlayerMediaLoadObservation>,
    next_ordered_player_event_sequence: u64,
    pending_ordered_player_events: VecDeque<PlayerOrderedEvent>,
    ordered_player_event_reacquisition_required: bool,
    ordered_player_event_reacquisition_requested_by_consumer: bool,
    last_delivered_ordered_command_progress: Vec<PlayerCommandProgress>,
    last_delivered_ordered_media_load_outcomes: Vec<PlayerMediaLoadObservation>,
    unacknowledged_terminal_command_progress: BTreeMap<PlayerCommandId, PlayerCommandProgress>,
    unacknowledged_media_load_outcomes: VecDeque<PlayerMediaLoadObservation>,
    pending_chat_requests: VecDeque<String>,
    pending_load_request: Option<String>,
    pending_load_generation: Option<PlayerMediaGeneration>,
    last_polled_local_file_update: Option<LocalFileUpdate>,
    last_paused_position_poll_at: Option<Instant>,
    observed_state: MpvObservedState,
    observers_registered: bool,
    transport_observers_registered: bool,
    observation_clock_origin: Instant,
    current_ipc_event_observed_at: Option<PlayerObservationTimestamp>,
    lifecycle_transcript_recorder: Option<MpvTranscriptRecorder>,
    next_lifecycle_transcript_ingress_sequence: u64,
    next_media_generation: u64,
    player_lifecycle: PlayerLifecycleState,
    lifecycle_reconciliation_due: bool,
    interrupted_network_stream_recovery: Option<InterruptedNetworkStreamRecovery>,
    network_cache_stall: Option<NetworkCacheStall>,
    active_media_generation: Option<PlayerMediaGeneration>,
    active_playlist_entry_id: Option<u64>,
    transport_phase: PlayerTransportPhase,
    active_file_loaded: bool,
    active_generation_has_restarted: bool,
    timeline_kind: PlayerTimelineKind,
    ytdl_is_live: bool,
    ytdl_is_live_metadata_generation: Option<PlayerMediaGeneration>,
    latest_cached_seekable_window: Option<PlayerSeekableRange>,
    path_metadata_generation: Option<PlayerMediaGeneration>,
    duration_metadata_generation: Option<PlayerMediaGeneration>,
    playback_restart_sequence: u64,
    next_command_id: u64,
    legacy_syncplay_ui_settings: LegacySyncplayUiSettings,
    last_simulated_legacy_syncplay_osd_message: Option<(String, LegacySyncplayOsdKind)>,
    legacy_syncplay_osd_placement_restore: Option<(String, i64)>,
    legacy_syncplayintf_script_loaded: bool,
    legacy_syncplayintf_options_applied: bool,
    legacy_syncplayintf_script_name: String,
    legacy_syncplayintf_bridge_instance_id: Option<String>,
    legacy_syncplayintf_owner_id: String,
    legacy_syncplayintf_attachment_id: String,
    legacy_syncplayintf_next_options_generation: u64,
    legacy_syncplayintf_pending_options_generation: Option<u64>,
    legacy_syncplayintf_acknowledged_options_generation: Option<u64>,
    legacy_syncplayintf_options_ack_error: Option<String>,
    legacy_syncplayintf_next_ping_nonce: u64,
    legacy_syncplayintf_pending_ping_nonce: Option<u64>,
    legacy_syncplayintf_last_heartbeat_at: Option<Instant>,
    legacy_syncplayintf_pending_heartbeat_command_id: Option<u64>,
    legacy_syncplayintf_last_discovery_at: Option<Instant>,
    legacy_syncplayintf_lease_reacquire_required: bool,
    legacy_syncplayintf_runtime_rediscovery_required: bool,
    legacy_syncplayintf_runtime_recovery_attempts: usize,
    legacy_syncplayintf_runtime_recovery_failure: Option<SorotteBridgeFailure>,
    sorotte_bridge_health: SorotteBridgeHealth,
    pending_sorotte_bridge_health_transitions: VecDeque<SorotteBridgeHealth>,
    ipc_endpoint: Option<PathBuf>,
    simulation_mode: bool,
    ipc_client: Option<MpvJsonIpcClient>,
    pending_ipc_connection_events: VecDeque<MpvIpcConnectionEvent>,
}

impl MpvAdapter {
    /// Enables sanitized raw mpv lifecycle transcript capture for debugging or
    /// deterministic regression-fixture generation.
    pub fn enable_lifecycle_transcript_capture(&mut self) {
        self.lifecycle_transcript_recorder = Some(MpvTranscriptRecorder::new());
        self.next_lifecycle_transcript_ingress_sequence = 1;
    }

    /// Records one decoded raw mpv IPC item at the adapter ingress boundary.
    ///
    /// Adapter-polled events call this automatically when capture is enabled.
    /// Debug transports may call it for synchronous command responses, which
    /// are consumed below the ordinary event pump.
    pub fn record_lifecycle_transcript_input(
        &mut self,
        command_id: Option<PlayerCommandId>,
        playlist_entry_id: Option<i64>,
        raw_json: Value,
    ) -> Result<(), MpvTranscriptError> {
        let Some(recorder) = self.lifecycle_transcript_recorder.as_mut() else {
            return Ok(());
        };
        let ingress_sequence = self.next_lifecycle_transcript_ingress_sequence.max(1);
        let monotonic_receipt_tick =
            u64::try_from(self.observation_clock_origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        recorder.record(
            self.player_lifecycle.attachment_epoch,
            ingress_sequence,
            monotonic_receipt_tick,
            command_id,
            playlist_entry_id,
            raw_json,
        )?;
        self.next_lifecycle_transcript_ingress_sequence = ingress_sequence.saturating_add(1).max(1);
        Ok(())
    }

    /// Stops capture and returns the validated sanitized transcript.
    pub fn take_lifecycle_transcript(&mut self) -> Option<MpvTranscript> {
        self.lifecycle_transcript_recorder
            .take()
            .map(MpvTranscriptRecorder::finish)
    }

    pub fn with_json_ipc(path: impl AsRef<Path>) -> Result<Self, PlayerError> {
        let mut adapter = Self::default();
        adapter.connect_json_ipc(path)?;
        Ok(adapter)
    }

    pub fn connect_json_ipc(&mut self, path: impl AsRef<Path>) -> Result<(), PlayerError> {
        let endpoint = path.as_ref().to_path_buf();
        let client = MpvJsonIpcClient::connect(&endpoint).map_err(PlayerError::OperationFailed)?;
        self.initialize_json_ipc_attachment(endpoint, client)
    }

    fn initialize_json_ipc_attachment(
        &mut self,
        endpoint: PathBuf,
        mut client: MpvJsonIpcClient,
    ) -> Result<(), PlayerError> {
        Self::require_supported_mpv_version(&mut client)?;
        let replacing_attachment = self.ipc_client.is_some();
        self.release_sorotte_bridge_best_effort();
        self.collect_ipc_connection_events();
        if replacing_attachment {
            self.reset_player_state_for_new_attachment();
        }
        self.apply_lifecycle_input(PlayerLifecycleInput::AttachmentReplaced);
        self.next_lifecycle_transcript_ingress_sequence = 1;
        self.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);
        self.pending_tracked_commands.clear();
        self.pending_load_request = None;
        self.pending_load_generation = None;
        self.interrupted_network_stream_recovery = None;
        self.network_cache_stall = None;
        self.active_media_generation = None;
        self.active_playlist_entry_id = None;
        self.active_file_loaded = false;
        self.active_generation_has_restarted = false;
        self.current_path = None;
        self.pending_local_file_update = None;
        self.last_polled_local_file_update = None;
        self.pending_playback_telemetry_update = None;
        self.pending_transport_telemetry_updates.clear();
        self.pending_cache_telemetry_updates.clear();
        self.pending_media_load_outcomes.clear();
        self.observed_state = MpvObservedState::default();
        self.transport_phase = PlayerTransportPhase::Empty;
        self.reset_timeline_metadata();
        self.simulation_mode = false;
        self.ipc_client = Some(client);
        self.ipc_endpoint = Some(endpoint);
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
        self.observers_registered = false;
        self.transport_observers_registered = false;
        self.reset_network_media_options_attachment_state();
        self.legacy_syncplay_osd_placement_restore = None;
        #[cfg(not(test))]
        {
            self.ensure_observers_registered_if_attached();
            self.reconcile_lifecycle_from_authority();
        }
        Ok(())
    }

    fn reset_player_state_for_new_attachment(&mut self) {
        let accepted_command_ids = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some())
            .map(|command| command.id)
            .collect::<BTreeSet<_>>();
        self.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);
        let handoff_progress = accepted_command_ids
            .iter()
            .filter_map(|command_id| {
                self.unacknowledged_terminal_command_progress
                    .get(command_id)
                    .copied()
            })
            .collect::<Vec<_>>();

        self.pending_tracked_commands.clear();
        self.pending_command_progress_updates.clear();
        self.pending_media_load_outcomes.clear();
        self.pending_ordered_player_events.clear();
        self.last_delivered_ordered_command_progress.clear();
        self.last_delivered_ordered_media_load_outcomes.clear();
        self.unacknowledged_terminal_command_progress.clear();
        self.unacknowledged_media_load_outcomes.clear();
        self.pending_local_file_update = None;
        self.pending_local_file_generation = None;
        self.pending_local_file_observed_at = None;
        self.pending_playback_telemetry_update = None;
        self.pending_transport_telemetry_updates.clear();
        self.pending_cache_telemetry_updates.clear();
        self.pending_ipc_connection_events.clear();

        self.pending_load_request = None;
        self.pending_load_generation = None;
        self.active_playlist_entry_id = None;
        self.active_media_generation = None;
        self.interrupted_network_stream_recovery = None;
        self.network_cache_stall = None;
        self.current_path = None;
        self.last_polled_local_file_update = None;
        self.active_file_loaded = false;
        self.active_generation_has_restarted = false;
        self.transport_phase = PlayerTransportPhase::Empty;
        self.observed_state = MpvObservedState::default();
        self.paused = false;
        self.logical_pause_explicit = false;
        self.position_seconds = 0.0;
        self.playback_rate = 0.0;
        self.paused_for_cache = false;
        self.cache_buffering_percent = None;
        self.last_paused_position_poll_at = None;
        self.playback_restart_sequence = 0;
        self.reset_timeline_metadata();
        self.ordered_player_event_reacquisition_required = true;
        self.ordered_player_event_reacquisition_requested_by_consumer = false;
        for progress in handoff_progress {
            self.queue_command_progress(progress);
        }
    }

    fn require_supported_mpv_version(client: &mut MpvJsonIpcClient) -> Result<(), PlayerError> {
        let reported_version = match client.get_property_string_classified(MPV_PROPERTY_VERSION) {
            Ok(Some(version)) => version,
            Ok(None) => {
                return Err(PlayerError::OperationFailed(format!(
                    "{}{minimum} or newer, but the connected mpv did not report an mpv-version; upgrade mpv and try again",
                    crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                    minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
                )));
            }
            Err(error) if error.is_property_unavailable() => {
                return Err(PlayerError::OperationFailed(format!(
                    "{}{minimum} or newer, but the connected mpv does not expose the mpv-version property; upgrade mpv and try again",
                    crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                    minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
                )));
            }
            Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
        };
        let parsed_version = Self::parse_mpv_version(&reported_version).ok_or_else(|| {
            PlayerError::OperationFailed(format!(
                "{}{minimum} or newer, but the connected mpv reported an unrecognized mpv-version; install an official supported mpv build and try again",
                crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
            ))
        })?;
        if parsed_version < MINIMUM_SUPPORTED_MPV_VERSION_COMPONENTS {
            let (major, minor, patch) = parsed_version;
            return Err(PlayerError::OperationFailed(format!(
                "{}{minimum} or newer, but the connected mpv reports mpv {major}.{minor}.{patch}; upgrade mpv and try again",
                crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
            )));
        }
        Ok(())
    }

    fn reset_network_media_options_attachment_state(&mut self) {
        self.network_media_options_hook_loaded = false;
        self.network_media_options_hook_configured_generation = None;
        self.network_media_options_hook_configuration_error = None;
        self.network_media_options_hook_last_heartbeat_at = None;
        self.network_media_options_hook_pending_heartbeat = None;
        self.network_media_options_hook_pending_event_poll_command_id = None;
        self.next_network_media_options_hook_heartbeat_nonce = 1;
        self.network_media_options_hook_instance_id = None;
        self.network_media_options_hook_last_accepted_load_sequence = None;
        self.network_media_options_hook_latest_started_load_sequence = None;
        self.network_media_options_expected_transition = None;
        self.network_media_options_hook_health = MpvNetworkOptionsHookHealth::Pending;
        self.network_media_options_hook_ownership_possible = false;
        self.network_media_options_hook_configuration_in_progress = false;
        self.network_media_options_policy_state = MpvNetworkMediaPolicyState::Unknown;
        self.reset_network_media_policy_diagnostics();
        self.bump_network_options_runtime_health_revision();
        self.pending_network_media_options_hook_active_result = None;
        self.deferred_network_media_options_hook_transition_result = None;
        self.network_media_options_embedded_load = None;
        self.network_media_options_apply_identity = None;
        self.network_media_options_event_batch_depth = 0;
        self.deferred_network_media_options_observation = None;
        self.pending_network_options_hook_health_transitions.clear();
        self.pending_network_media_policy_outcomes.clear();
    }

    fn reset_legacy_syncplayintf_attachment_for_new_ipc(&mut self) {
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_last_discovery_at = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        // Health transitions are scoped to one IPC endpoint and must never outlive it.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        self.pending_chat_requests.clear();
        let connection_generation = self
            .ipc_client
            .as_ref()
            .map(MpvJsonIpcClient::generation)
            .unwrap_or_else(|| NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT.fetch_add(1, Ordering::Relaxed));
        self.legacy_syncplayintf_attachment_id = format!(
            "{}-{connection_generation}",
            self.legacy_syncplayintf_owner_id
        );
    }

    pub fn is_connected(&self) -> bool {
        self.ipc_client
            .as_ref()
            .is_some_and(MpvJsonIpcClient::is_healthy)
    }

    pub(crate) fn simulated() -> Self {
        Self {
            simulation_mode: true,
            ..Self::default()
        }
    }

    /// Builds a connected adapter whose test transport accepts mpv commands but never emits the
    /// Lua settings acknowledgement.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_unacknowledging_syncplayintf_test_ipc(settings: LegacySyncplayUiSettings) -> Self {
        Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            ipc_client: Some(crate::test_support::unacknowledging_syncplayintf_client()),
            ..Self::default()
        }
    }

    /// Builds a connected adapter whose test transport accepts bridge discovery commands but
    /// never emits the canonical pong.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_undiscoverable_sorotte_bridge_test_ipc(settings: LegacySyncplayUiSettings) -> Self {
        let mut adapter = Self {
            legacy_syncplay_ui_settings: settings,
            ipc_client: Some(crate::test_support::undiscoverable_syncplayintf_client()),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    /// Builds a connected adapter whose fake mpv rejects only canonical bridge discovery while
    /// leaving the core JSON IPC transport healthy.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_rejected_sorotte_bridge_discovery_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> Self {
        let mut adapter = Self {
            legacy_syncplay_ui_settings: settings,
            ipc_client: Some(crate::test_support::rejecting_syncplayintf_discovery_client()),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    /// Builds a ready bridge attachment and returns a counter incremented when its terminal
    /// release reaches the fake IPC transport.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_release_recording_sorotte_bridge_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let (ipc_client, release_count) =
            crate::test_support::release_recording_syncplayintf_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, release_count)
    }

    /// Builds a ready bridge attachment whose terminal cleanup commands are recorded in order.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_cleanup_recording_sorotte_bridge_test_ipc(
        settings: LegacySyncplayUiSettings,
        osd_placement_restore: Option<(String, i64)>,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let (ipc_client, commands) = crate::test_support::cleanup_recording_syncplayintf_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplay_osd_placement_restore: osd_placement_restore,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, commands)
    }

    /// Builds a ready simulated bridge over connected IPC that rejects the first active-network
    /// option write while accepting a later retry.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_first_active_network_option_rejection_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> Self {
        Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(crate::test_support::reject_first_active_network_option_client()),
            ..Self::default()
        }
    }

    /// Builds a ready simulated bridge over connected IPC that rejects exactly the Nth
    /// active-network option write while recording the partial apply and later retry.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_nth_active_network_option_rejection_test_ipc(
        settings: LegacySyncplayUiSettings,
        rejected_write: usize,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let (ipc_client, commands) =
            crate::test_support::reject_nth_active_network_option_client(rejected_write);
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, commands)
    }

    /// Builds a ready simulated bridge whose authoritative path is initially absent and becomes
    /// network media on the next query, recording accepted active-network option writes.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_delayed_active_network_media_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let (ipc_client, commands) = crate::test_support::delayed_active_network_media_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, commands)
    }

    /// Builds a ready simulated bridge whose active path starts local and transitions to a
    /// network path after the returned trigger is armed. The command log contains only
    /// `file-local-options/*` writes.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_external_network_media_transition_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        Self::with_external_network_media_transition_test_ipc_mode(settings, false)
    }

    /// Builds the same triggered local-to-network transition fixture while making mpv reject
    /// the first transition-time file-local option write without disconnecting IPC.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_rejected_external_network_media_transition_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        Self::with_external_network_media_transition_test_ipc_mode(settings, true)
    }

    /// Builds a ready simulated bridge whose explicit apply starts on network A and is
    /// superseded during its first write by network B, which accepts the complete option map.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_active_network_media_supersession_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let (ipc_client, commands) =
            crate::test_support::active_network_media_supersession_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, commands)
    }

    /// Injects a scoped core-hook degradation into a feature-gated adapter fixture.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_hook_degradation(
        &mut self,
        reason: impl Into<String>,
    ) {
        self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(reason.into()));
    }

    /// Injects terminal-idle media policy state without changing hook health.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_no_active_media(&mut self) {
        self.queue_network_media_policy_outcome(MpvNetworkMediaPolicyOutcome::NoActiveMedia);
    }

    /// Injects a local-media policy result without changing hook health.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_local_media_unchanged(&mut self) {
        self.queue_network_media_policy_outcome(MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged);
    }

    /// Positively recovers a feature-gated hook-health fixture.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_hook_recovery(&mut self) {
        self.queue_network_media_options_hook_recovered();
    }

    /// Marks the feature-gated adapter fixture as waiting for an authoritative media result.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_awaiting_authoritative_transition(&mut self) {
        self.set_network_media_policy_state(MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad);
    }

    /// Injects a sanitized active-media policy failure from credential-bearing raw targets.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_network_media_options_policy_failure(
        &mut self,
        load_sequence: u64,
        source_path: &str,
        stream_open_filename: &str,
    ) {
        self.queue_network_media_policy_outcome(MpvNetworkMediaPolicyOutcome::Failed(
            NetworkOptionsApplyDiagnostic::player_error(
                load_sequence,
                NetworkOptionsMediaTargetKind::from_target(Some(source_path)),
                NetworkOptionsMediaTargetKind::from_target(Some(stream_open_filename)),
            ),
        ));
    }

    /// Establishes a successful privacy-safe network/cache diagnostic fixture.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn inject_test_verified_network_media_diagnostic_snapshot(&mut self, load_sequence: u64) {
        if self.observation_media_generation().is_none() {
            self.establish_test_external_media_lifecycle(
                1,
                "https://media.example.test/diagnostic.m3u8",
            );
        }
        self.network_media_options_application_state =
            Some(MpvNetworkMediaPolicyApplicationState::Applied);
        self.network_media_options_diagnostic_load_sequence = Some(load_sequence);
        self.network_media_options_verification_complete = true;
        self.network_media_options_option_results = self
            .network_media_options
            .keys()
            .map(|name| MpvNetworkOptionApplyResult {
                name: name.clone(),
                status: MpvNetworkOptionApplyStatus::Applied,
            })
            .collect();
        self.network_media_options_effective_cache_options =
            self.network_media_options_desired_cache_options();
        self.transport_phase = PlayerTransportPhase::ReadyPaused;
        self.observed_state.paused_for_cache = Some(false);
        self.observed_state.demuxer_cache_idle = Some(true);
        self.observed_state.buffered_ahead_seconds = Some(30.0);
        self.observed_state.buffered_ahead_bytes = Some(524_288);
        self.observed_state.input_rate_bytes_per_second = Some(2_000_000);
        self.observed_state.cache_reader_position_seconds = Some(10.0);
        self.observed_state.cache_end_seconds = Some(40.0);
        self.observed_state.cache_eof = Some(false);
        self.observed_state.cache_underrun = Some(false);
        self.observed_state.cache_metrics_observed_at = Some(self.observation_timestamp());
    }

    #[cfg(test)]
    pub(crate) fn inject_test_cache_telemetry_update(&mut self) {
        let generation = self
            .observation_media_generation()
            .or_else(|| Some(PlayerMediaGeneration::new(1)));
        self.queue_cache_telemetry_update(PlayerCacheTelemetryUpdate {
            media_generation: generation,
            observed_at: Some(self.observation_timestamp()),
            buffered_ahead_seconds: Some(5.0),
            ..PlayerCacheTelemetryUpdate::default()
        });
    }

    #[cfg(feature = "test-support")]
    fn establish_test_external_media_lifecycle(&mut self, playlist_entry_id: i64, target: &str) {
        let attachment_epoch = self.lifecycle_epoch();
        let media_generation = self.allocate_media_generation();
        self.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation,
            playlist_entry_id,
            observed_target: target.to_owned(),
            file_loaded: true,
        });
        self.active_playlist_entry_id = u64::try_from(playlist_entry_id).ok();
        self.active_media_generation = self.player_lifecycle.active_media_generation();
        self.active_file_loaded = true;
        self.current_path = Some(target.to_owned());
        self.observed_state.path = Some(target.to_owned());
        self.transport_phase = PlayerTransportPhase::Playing;
        debug_assert!(self.player_lifecycle.assert_invariants().is_ok());
    }

    #[cfg(feature = "test-support")]
    fn with_external_network_media_transition_test_ipc_mode(
        settings: LegacySyncplayUiSettings,
        reject_option_write: bool,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (ipc_client, commands, transition_trigger) =
            crate::test_support::external_network_media_transition_client(reject_option_write);
        let mut adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        adapter.establish_test_external_media_lifecycle(43, "C:/media/local-intro.mkv");
        (adapter, commands, transition_trigger)
    }

    /// Marks a feature-gated fake IPC client unhealthy so higher-layer tests can distinguish a
    /// fatal player transport loss from optional bridge degradation.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn mark_test_ipc_unhealthy(&mut self, reason: impl Into<String>) {
        if let Some(client) = self.ipc_client.as_mut() {
            client.mark_unhealthy_for_test(reason);
        }
        self.collect_ipc_connection_events();
    }

    pub fn take_ipc_connection_events(&mut self) -> Vec<MpvIpcConnectionEvent> {
        self.maintain_runtime_integrations();
        self.collect_ipc_connection_events();
        self.pending_ipc_connection_events.drain(..).collect()
    }

    fn collect_ipc_connection_events(&mut self) {
        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };
        self.pending_ipc_connection_events
            .extend(ipc_client.take_connection_events());
    }

    pub fn current_path(&self) -> Option<&str> {
        self.current_path.as_deref()
    }

    /// Configures options that mpv should apply only while playing network media.
    ///
    /// The options are attached to Sorotte-issued `loadfile` commands as mpv
    /// per-file options. mpv restores the user's prior values when that media
    /// ends, so a later local file keeps its normal mpv/user cache policy.
    pub fn configure_network_media_options<I, K, V>(&mut self, options: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let options = options
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        if options != self.network_media_options {
            self.network_media_options = options;
            self.network_media_options_generation =
                self.network_media_options_generation.wrapping_add(1).max(1);
            self.network_media_options_hook_configured_generation = None;
            self.network_media_options_hook_configuration_error = None;
            self.network_media_options_hook_last_heartbeat_at = None;
            self.network_media_options_hook_pending_heartbeat = None;
            self.network_media_options_hook_pending_event_poll_command_id = None;
            if !matches!(
                self.network_media_options_hook_health,
                MpvNetworkOptionsHookHealth::Degraded(_)
            ) {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
            }
            self.pending_network_media_options_hook_active_result = None;
            self.deferred_network_media_options_hook_transition_result = None;
            self.network_media_options_embedded_load = None;
            self.network_media_options_apply_identity = None;
            self.network_media_options_expected_transition = None;
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
            self.reset_network_media_policy_diagnostics();
            self.deferred_network_media_options_observation = None;
            // File-policy results belong to the superseded option generation. Hook-health
            // transitions describe the adapter-wide hook lease and must survive unchanged,
            // including Degraded -> Recovered -> Degraded sequences that have not yet drained.
            self.pending_network_media_policy_outcomes.clear();
        }
    }

    /// Returns the oldest unconsumed hook-health transition.
    pub fn take_network_options_hook_health_transition(
        &mut self,
    ) -> Option<MpvNetworkOptionsHookHealthTransition> {
        self.maintain_runtime_integrations();
        self.take_network_options_hook_health_transition_nonblocking()
    }

    /// Pure queue pop for async wait loops that already service leases explicitly.
    pub fn take_network_options_hook_health_transition_nonblocking(
        &mut self,
    ) -> Option<MpvNetworkOptionsHookHealthTransition> {
        self.pending_network_options_hook_health_transitions
            .pop_front()
            .map(|event| event.value)
    }

    /// Returns the oldest unconsumed active-media policy outcome.
    pub fn take_network_media_policy_outcome(&mut self) -> Option<MpvNetworkMediaPolicyOutcome> {
        self.maintain_runtime_integrations();
        self.take_network_media_policy_outcome_nonblocking()
    }

    /// Pure queue pop for async wait loops that already service leases explicitly.
    pub fn take_network_media_policy_outcome_nonblocking(
        &mut self,
    ) -> Option<MpvNetworkMediaPolicyOutcome> {
        self.pending_network_media_policy_outcomes
            .pop_front()
            .map(|event| event.value)
    }

    /// Returns the authoritative current network-options state without consuming notifications.
    pub fn network_options_runtime_health_snapshot(
        &self,
    ) -> MpvNetworkOptionsRuntimeHealthSnapshot {
        MpvNetworkOptionsRuntimeHealthSnapshot {
            revision: self.network_media_options_runtime_health_revision,
            hook_health: self.network_media_options_hook_health.clone(),
            media_policy: self.network_media_options_policy_state.clone(),
        }
    }

    /// Returns generation-correlated effective policy and cache state without retaining media
    /// targets or arbitrary advanced option values.
    pub fn network_media_diagnostic_snapshot(&self) -> MpvNetworkMediaDiagnosticSnapshot {
        MpvNetworkMediaDiagnosticSnapshot {
            media_generation: self.observation_media_generation(),
            network_policy_generation: self.network_media_options_generation,
            load_sequence: self.network_media_options_diagnostic_load_sequence,
            application_state: self.network_media_options_application_state,
            verification_complete: self.network_media_options_verification_complete,
            option_results: self.network_media_options_option_results.clone(),
            desired_cache_options: self.network_media_options_desired_cache_options(),
            effective_cache_options: self.network_media_options_effective_cache_options.clone(),
            observed_at: self.observed_state.cache_metrics_observed_at,
            transport_phase: self.transport_phase,
            paused_for_cache: self.observed_state.paused_for_cache,
            demuxer_cache_idle: self.observed_state.demuxer_cache_idle,
            cache_duration_seconds: self.observed_state.buffered_ahead_seconds,
            forward_bytes: self.observed_state.buffered_ahead_bytes,
            raw_input_rate_bytes_per_second: self.observed_state.input_rate_bytes_per_second,
            reader_position_seconds: self.observed_state.cache_reader_position_seconds,
            cache_end_seconds: self.observed_state.cache_end_seconds,
            cache_eof: self.observed_state.cache_eof,
            cache_underrun: self.observed_state.cache_underrun,
        }
    }

    /// Returns the next production-ordered compatibility outcome across the two independent
    /// channels. New consumers should drain each typed channel and reconcile the snapshot.
    pub fn take_network_media_options_transition_outcome(
        &mut self,
    ) -> Option<MpvNetworkMediaOptionsTransitionOutcome> {
        self.maintain_runtime_integrations();
        let hook_sequence = self
            .pending_network_options_hook_health_transitions
            .front()
            .map(|event| event.sequence);
        let policy_sequence = self
            .pending_network_media_policy_outcomes
            .front()
            .map(|event| event.sequence);
        match (hook_sequence, policy_sequence) {
            (Some(hook), Some(policy)) if hook <= policy => self
                .pending_network_options_hook_health_transitions
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkOptionsHookHealthTransition::Recovered => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookRecovered
                    }
                    MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)
                    }
                }),
            (Some(_), Some(_)) | (None, Some(_)) => self
                .pending_network_media_policy_outcomes
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkMediaPolicyOutcome::NoActiveMedia => {
                        MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia
                    }
                    MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged => {
                        MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged
                    }
                    MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                        MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated
                    }
                    MpvNetworkMediaPolicyOutcome::Failed(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::Failed(error)
                    }
                }),
            (Some(_), None) => self
                .pending_network_options_hook_health_transitions
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkOptionsHookHealthTransition::Recovered => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookRecovered
                    }
                    MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)
                    }
                }),
            (None, None) => None,
        }
    }

    /// Advances the bounded maintenance required by Sorotte-owned mpv integrations.
    ///
    /// Runtime owners should call this from their regular tick even when they are not currently
    /// consuming playback telemetry. Every adapter observation getter also invokes it so an
    /// active transport-only or command-only consumer keeps hook leases alive.
    pub fn maintain_runtime_integrations(&mut self) {
        self.drain_ipc_events_if_attached();
        self.recover_network_media_options_hook_ownership_if_needed();
        self.maintain_network_cache_stall_recovery();
        self.maintain_network_media_options_hook_lease();
        self.maintain_legacy_syncplayintf_lease();
        // Synchronous heartbeat commands can themselves harvest a bounded batch of events. Flush
        // their control faults and ordinary observations, then recover an ownership loss in the
        // same pump so a transient lease expiry never reaches the GUI as incomplete setup.
        self.drain_ipc_events_if_attached();
        self.recover_network_media_options_hook_ownership_if_needed();
        self.maintain_network_cache_stall_recovery();
        let now_tick =
            u64::try_from(self.observation_clock_origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced { now_tick });
        if self.lifecycle_reconciliation_due {
            #[cfg(not(test))]
            {
                self.lifecycle_reconciliation_due = false;
                self.reconcile_lifecycle_from_authority();
            }
        }
    }

    /// Applies the configured network options to an already-active network file.
    ///
    /// mpv's `file-local-options` namespace snapshots the prior option values
    /// and restores them when the file ends. This is useful when Sorotte
    /// attaches to an existing mpv session or changes settings in place. Local
    /// files are deliberately left untouched.
    pub fn apply_network_media_options_to_active_media(&mut self) -> Result<(), PlayerError> {
        self.apply_network_media_options_to_active_media_classified()
            .map(|_| ())
    }

    /// Applies configured network options and reports whether mpv had no active media, local
    /// media that was intentionally unchanged, network media that accepted every option, or a
    /// newer authoritative path superseded the explicit attempt.
    pub fn apply_network_media_options_to_active_media_classified(
        &mut self,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        let mut result = self.apply_network_media_options_to_active_media_classified_inner();
        if result
            .as_ref()
            .is_err_and(Self::network_options_hook_ownership_failure)
        {
            // A lease can expire between the last runtime pump and an explicit settings apply.
            // The Lua hook has already released the old owner, so retry the full configure/apply
            // transaction once instead of surfacing a sticky error that only an app restart can
            // clear.
            self.network_media_options_hook_configuration_error = None;
            result = self.apply_network_media_options_to_active_media_classified_inner();
        }
        if let Err(error) = &result {
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Failed(
                error.to_string(),
            ));
        }
        result
    }

    fn network_options_hook_ownership_failure(error: &PlayerError) -> bool {
        let PlayerError::OperationFailed(reason) = error else {
            return false;
        };
        Self::network_options_hook_ownership_failure_reason(reason)
    }

    fn network_options_hook_ownership_failure_reason(reason: &str) -> bool {
        reason.contains("network-options hook lease expired")
            || reason.contains("network-options hook ownership was replaced")
            || reason.contains("network-options hook ownership was lost")
            || reason.contains("network-options hook did not acknowledge heartbeat nonce")
    }

    fn recover_network_media_options_hook_ownership_if_needed(&mut self) {
        let should_recover = matches!(
            &self.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Degraded(reason)
                if Self::network_options_hook_ownership_failure_reason(reason)
        );
        if !should_recover || !self.network_media_options_hook_should_run() {
            return;
        }

        if let Err(error) = self.apply_network_media_options_to_active_media_classified() {
            // Do not enter a blocking retry loop on every runtime tick when another live owner
            // legitimately won the lease or mpv stopped answering. The original degradation is
            // already queued for observers; record the failed automatic attempt as the current
            // authoritative health and leave subsequent recovery to the explicit retry action.
            self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Degraded(format!(
                "automatic network-options hook ownership recovery failed: {error}"
            )));
        }
    }

    fn apply_network_media_options_to_active_media_classified_inner(
        &mut self,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        // An explicit file-policy operation must not discard adapter-wide hook-health events or
        // authoritative path results already queued by early maintenance in this same pump.
        self.ensure_network_media_options_hook_configured()?;
        // `current_path` may describe a requested load or a prior externally
        // replaced playlist entry. An attached mpv is authoritative; the cache
        // is safe only for simulation or other no-IPC operation.
        let active_path = match self.ipc_client.as_mut() {
            Some(client) => match client.get_property_string_classified(MPV_PROPERTY_PATH) {
                Ok(path) => path,
                Err(error) if error.is_property_unavailable() => None,
                Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
            },
            None => self.current_path.clone(),
        };
        let Some(active_path) = active_path else {
            if self.network_media_options_hook_should_run()
                && (self.pending_load_request().is_some()
                    || self.pending_load_generation().is_some()
                    || matches!(
                        self.transport_phase,
                        PlayerTransportPhase::Loading | PlayerTransportPhase::Prebuffering
                    ))
            {
                self.set_network_media_policy_state(
                    MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad,
                );
                return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
            }
            self.clear_network_media_options_path_identity();
            self.reset_network_media_policy_diagnostics();
            self.record_network_media_options_policy_applied(
                MpvNetworkMediaPolicyState::NoActiveMedia,
                None,
            );
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia);
        };

        let attempt_id = self
            .begin_network_media_options_apply_attempt(self.active_media_generation, &active_path);
        if self.network_media_options_hook_should_run() {
            if !self.network_media_options_hook_is_ready() {
                return Err(PlayerError::OperationFailed(
                    "Sorotte's mpv network-options hook is not ready after configuration"
                        .to_owned(),
                ));
            }
            return self
                .apply_network_media_options_to_active_media_via_hook(&active_path, attempt_id);
        }
        if !uses_network_media_options(&active_path) {
            self.clear_network_media_options_path_identity();
            self.reset_network_media_policy_diagnostics();
            self.record_network_media_options_policy_applied(
                MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                None,
            );
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged);
        }
        // Direct file-local writes are intentionally limited to simulation and explicit test
        // fixtures. A real JSON IPC attachment must never re-enter the cross-file fallback when
        // the core hook is enabled but unavailable.
        if !self.apply_network_media_options_for_attempt(&active_path, attempt_id)? {
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
        }
        self.record_unverified_network_media_options_applied();
        self.record_network_media_options_policy_applied(
            MpvNetworkMediaPolicyState::NetworkMediaUpdated,
            None,
        );
        Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated)
    }

    fn network_media_options_hook_should_run(&self) -> bool {
        self.network_media_options_hook_enabled
            && !self.simulation_mode
            && self.ipc_client.is_some()
    }

    fn network_media_options_hook_is_ready(&self) -> bool {
        self.network_media_options_hook_should_run()
            && matches!(
                self.network_media_options_hook_health,
                MpvNetworkOptionsHookHealth::Ready
            )
            && self.network_media_options_hook_loaded
            && self.network_media_options_hook_configured_generation
                == Some(self.network_media_options_generation)
    }

    fn invalidate_network_media_options_hook_delivery(&mut self) {
        if matches!(
            self.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        ) {
            self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
        }
        self.network_media_options_hook_loaded = false;
        self.network_media_options_hook_configured_generation = None;
        self.network_media_options_hook_last_heartbeat_at = None;
        self.network_media_options_hook_pending_heartbeat = None;
        self.network_media_options_hook_pending_event_poll_command_id = None;
        self.pending_network_media_options_hook_active_result = None;
        self.deferred_network_media_options_hook_transition_result = None;
        self.network_media_options_expected_transition = None;
    }

    fn network_media_options_hook_controller_payload(&self) -> String {
        json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
        })
        .to_string()
    }

    fn maintain_network_media_options_hook_lease(&mut self) {
        if !self.network_media_options_hook_is_ready() {
            return;
        }
        if let Some(pending) = self.network_media_options_hook_pending_heartbeat {
            if pending.sent_at.is_some_and(|sent_at| {
                sent_at.elapsed() >= NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT
            }) {
                let reason = format!(
                    "Sorotte's mpv network-options hook did not acknowledge heartbeat nonce {}",
                    pending.nonce
                );
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
                    reason,
                ));
            }
            return;
        }
        if self
            .network_media_options_hook_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let nonce = self.next_network_media_options_hook_heartbeat_nonce;
        self.next_network_media_options_hook_heartbeat_nonce = self
            .next_network_media_options_hook_heartbeat_nonce
            .wrapping_add(1)
            .max(1);
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "heartbeatNonce": nonce,
        });
        self.network_media_options_hook_pending_heartbeat =
            Some(PendingNetworkOptionsHookHeartbeat {
                nonce,
                command_id: None,
                sent_at: Some(Instant::now()),
            });
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_HEARTBEAT_MESSAGE,
            payload.to_string(),
        ]);
        match self.send_ipc_command_if_attached(command) {
            Ok(()) => {}
            Err(error) => {
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(error);
            }
        }
    }

    fn ensure_network_media_options_hook_configured(&mut self) -> Result<(), PlayerError> {
        if !self.network_media_options_hook_should_run() {
            return Ok(());
        }
        if self.network_media_options_hook_is_ready() {
            return Ok(());
        }
        if self.network_media_options_hook_configuration_in_progress {
            return Err(PlayerError::OperationFailed(
                "Sorotte's mpv network-options hook configuration is already in progress"
                    .to_owned(),
            ));
        }
        self.network_media_options_hook_configuration_in_progress = true;
        let result = self.ensure_network_media_options_hook_configured_inner();
        self.network_media_options_hook_configuration_in_progress = false;
        result
    }

    fn ensure_network_media_options_hook_configured_inner(&mut self) -> Result<(), PlayerError> {
        // Ownership/configuration failures describe the previous transaction. Leaving one set
        // here lets a delayed lease-expired event abort a new configuration before its positive
        // acknowledgement is reduced.
        self.network_media_options_hook_configuration_error = None;
        if !self.network_media_options_hook_loaded {
            let path = materialize_bundled_sorotte_network_options_hook().map_err(|error| {
                PlayerError::OperationFailed(format!(
                    "failed to materialize Sorotte's mpv network-options hook: {error}"
                ))
            })?;
            if let Err(error) = self.send_ipc_command_if_attached(json!([
                MPV_COMMAND_LOAD_SCRIPT,
                path.to_string_lossy()
            ])) {
                self.invalidate_network_media_options_hook_delivery();
                return Err(error);
            }
            self.network_media_options_hook_loaded = true;
        }
        let generation = self.network_media_options_generation;
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": generation,
            "leaseMs": NETWORK_OPTIONS_HOOK_OWNER_LEASE_MS,
            "options": self.network_media_options_map(),
        })
        .to_string();
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_CONFIGURE_MESSAGE,
            payload
        ]);
        let deadline = Instant::now() + NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_WINDOW;
        loop {
            if let Err(error) = self.send_ipc_command_if_attached(command.clone()) {
                self.invalidate_network_media_options_hook_delivery();
                return Err(error);
            }
            self.network_media_options_hook_ownership_possible = true;
            if self.network_media_options_hook_configured_generation == Some(generation) {
                self.network_media_options_hook_last_heartbeat_at = Some(Instant::now());
                self.network_media_options_hook_pending_heartbeat = None;
                self.network_media_options_hook_pending_event_poll_command_id = None;
                return Ok(());
            }
            if let Some(error) = self.network_media_options_hook_configuration_error.take() {
                return Err(PlayerError::OperationFailed(error));
            }
            if Instant::now() >= deadline {
                self.invalidate_network_media_options_hook_delivery();
                return Err(PlayerError::OperationFailed(format!(
                    "Sorotte's mpv network-options hook did not acknowledge generation {generation}"
                )));
            }
            std::thread::sleep(NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_INTERVAL);
        }
    }

    fn apply_network_media_options_to_active_media_via_hook(
        &mut self,
        initial_path: &str,
        attempt_id: u64,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        self.pending_network_media_options_hook_active_result = None;
        self.set_network_media_policy_state(MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad);
        let generation = self.network_media_options_generation;
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": generation,
            "attempt": attempt_id,
        })
        .to_string();
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_APPLY_ACTIVE_MESSAGE,
            payload
        ]);
        let deadline = Instant::now() + NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_WINDOW;
        let result = loop {
            if let Err(error) = self.send_ipc_command_if_attached(command.clone()) {
                self.invalidate_network_media_options_hook_delivery();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(error);
            }
            if let Some(result) = self.pending_network_media_options_hook_active_result.take()
                && result.attempt_id == attempt_id
                && result.generation == generation
            {
                break result;
            }
            if let Some(error) = self.network_media_options_hook_configuration_error.take() {
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(PlayerError::OperationFailed(error));
            }
            if Instant::now() >= deadline {
                self.invalidate_network_media_options_hook_delivery();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(PlayerError::OperationFailed(format!(
                    "Sorotte's mpv network-options hook did not report active apply attempt {attempt_id}"
                )));
            }
            std::thread::sleep(NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_INTERVAL);
        };

        let superseded = result.source_path.as_ref().map(SecretValue::expose_secret)
            != Some(initial_path)
            || !self.network_media_options_apply_attempt_is_current(attempt_id);
        if superseded {
            // The returned status belongs to the old sampled path. Only the newer authoritative
            // network/local/idle transition may publish the final outcome.
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
        }

        self.network_media_options_hook_last_accepted_load_sequence = Some(
            self.network_media_options_hook_last_accepted_load_sequence
                .map_or(result.load_sequence, |accepted| {
                    accepted.max(result.load_sequence)
                }),
        );
        self.queue_network_media_options_hook_recovered();

        match result.status {
            NetworkOptionsHookApplyStatus::NoActiveMedia => {
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::NoActiveMedia,
                    Some(result.load_sequence),
                );
                Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia)
            }
            NetworkOptionsHookApplyStatus::LocalMediaUnchanged => {
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                    Some(result.load_sequence),
                );
                Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged)
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated
            | NetworkOptionsHookApplyStatus::PartiallyApplied
            | NetworkOptionsHookApplyStatus::Failed => {
                let application_state = self.record_network_media_option_application(
                    result.load_sequence,
                    result.status,
                    result.verification_complete,
                    result.option_results,
                    result.effective_options,
                );
                if application_state == MpvNetworkMediaPolicyApplicationState::Applied {
                    self.record_network_media_options_policy_applied(
                        MpvNetworkMediaPolicyState::NetworkMediaUpdated,
                        Some(result.load_sequence),
                    );
                    return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated);
                }
                let error = self.network_media_option_application_error(
                    result.load_sequence,
                    result.source_kind,
                    result.stream_target_kind,
                    application_state,
                );
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Failed(
                    error.to_string(),
                ));
                Err(error)
            }
        }
    }

    fn begin_network_media_options_apply_attempt(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        path: &str,
    ) -> u64 {
        let attempt_id = self.next_network_media_options_apply_attempt_id;
        self.next_network_media_options_apply_attempt_id = self
            .next_network_media_options_apply_attempt_id
            .wrapping_add(1)
            .max(1);
        self.network_media_options_apply_identity = Some(NetworkMediaOptionsApplyIdentity {
            attempt_id,
            media_generation,
            path: path.to_owned(),
        });
        attempt_id
    }

    fn network_media_options_apply_attempt_is_current(&self, attempt_id: u64) -> bool {
        self.network_media_options_apply_identity
            .as_ref()
            .is_some_and(|identity| identity.attempt_id == attempt_id)
    }

    fn apply_network_media_options_for_attempt(
        &mut self,
        path: &str,
        attempt_id: u64,
    ) -> Result<bool, PlayerError> {
        if !uses_network_media_options(path) {
            return Ok(true);
        }
        for (name, value) in self.network_media_options.clone() {
            if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                return Ok(false);
            }
            let result = self.send_ipc_command_if_attached(json!([
                MPV_COMMAND_SET_PROPERTY,
                format!("file-local-options/{name}"),
                value
            ]));
            if let Err(error) = result {
                // Command rejection returns before the generic sender drains events that arrived
                // ahead of the response. Process them before attributing the error so a newer
                // authoritative path can supersede this attempt without a stale failure outcome.
                self.drain_ipc_events_if_attached();
                // Supersession can make a healthy rejection irrelevant to the new file, but an
                // unhealthy transport is adapter-wide and must remain observable to its owner.
                if !self.is_connected() {
                    return Err(error);
                }
                if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                    return Ok(false);
                }
                return Err(error);
            }
            if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn embedded_network_media_options_belong_to_pending_load(&self) -> bool {
        self.network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| {
                self.pending_load_generation() == Some(embedded.media_generation)
                    || (self.pending_load_request().is_some()
                        && self.active_media_generation == Some(embedded.media_generation))
            })
    }

    fn embedded_network_media_options_apply_to_path(
        &self,
        media_generation: Option<PlayerMediaGeneration>,
        path: &str,
    ) -> bool {
        self.network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| {
                Some(embedded.media_generation) == media_generation
                    && Self::media_target_matches(path, &embedded.requested_target)
            })
    }

    fn clear_network_media_options_path_identity(&mut self) {
        self.network_media_options_apply_identity = None;
        if !self.embedded_network_media_options_belong_to_pending_load() {
            self.network_media_options_embedded_load = None;
        }
    }

    fn record_network_media_options_policy_applied(
        &mut self,
        state: MpvNetworkMediaPolicyState,
        load_sequence: Option<u64>,
    ) {
        self.set_network_media_policy_state(state);
        if let Some(load_sequence) = load_sequence {
            self.network_media_options_hook_last_accepted_load_sequence = Some(
                self.network_media_options_hook_last_accepted_load_sequence
                    .map_or(load_sequence, |accepted| accepted.max(load_sequence)),
            );
        }
    }

    fn observe_authoritative_path_for_network_options(
        &mut self,
        path: Option<&str>,
        origin: AuthoritativePathObservationOrigin,
    ) {
        if self.network_media_options_event_batch_depth != 0 {
            if origin == AuthoritativePathObservationOrigin::StartFilePending {
                self.deferred_network_media_options_hook_transition_result = None;
            }

            if path.is_none() {
                if origin == AuthoritativePathObservationOrigin::EndFileIdle
                    && self
                        .deferred_network_media_options_observation
                        .as_ref()
                        .is_some_and(|observation| {
                            observation.origin == AuthoritativePathObservationOrigin::Poll
                                && observation.path.is_none()
                        })
                {
                    self.deferred_network_media_options_observation =
                        Some(DeferredAuthoritativePathObservation {
                            path: None,
                            origin: AuthoritativePathObservationOrigin::EndFileIdle,
                        });
                    return;
                }
                if origin != AuthoritativePathObservationOrigin::StartFilePending
                    && self
                        .deferred_network_media_options_observation
                        .as_ref()
                        .is_some_and(|observation| {
                            observation.origin == AuthoritativePathObservationOrigin::EndFileIdle
                                && observation.path.is_none()
                        })
                {
                    return;
                }
            }
            // A poll issued while reducing an already-buffered event batch completes after every
            // event already present in that batch. Preserve that newer snapshot over events that
            // are merely handled later from the older local batch vector.
            if self
                .deferred_network_media_options_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.origin == AuthoritativePathObservationOrigin::Poll
                        && origin != AuthoritativePathObservationOrigin::Poll
                })
            {
                return;
            }
            self.deferred_network_media_options_observation =
                Some(DeferredAuthoritativePathObservation {
                    path: path.map(ToOwned::to_owned),
                    origin,
                });
            return;
        }
        self.apply_authoritative_path_for_network_options(path, origin);
    }

    fn apply_authoritative_path_for_network_options(
        &mut self,
        path: Option<&str>,
        origin: AuthoritativePathObservationOrigin,
    ) {
        let Some(path) = path else {
            let completes_pending_policy = self.network_media_options_apply_identity.is_some()
                || matches!(
                    self.network_media_options_policy_state,
                    MpvNetworkMediaPolicyState::Failed(_)
                        | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
                );
            self.clear_network_media_options_path_identity();
            if origin == AuthoritativePathObservationOrigin::EndFileIdle {
                self.reset_network_media_policy_diagnostics();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::NoActiveMedia);
            }
            if self.network_media_options_hook_should_run()
                && origin == AuthoritativePathObservationOrigin::EndFileIdle
                && completes_pending_policy
                && self
                    .deferred_network_media_options_hook_transition_result
                    .is_none()
            {
                self.queue_network_media_policy_outcome(
                    MpvNetworkMediaPolicyOutcome::NoActiveMedia,
                );
            }
            return;
        };
        let media_generation = self.active_media_generation;
        if self.network_media_options.is_empty() {
            return;
        }

        if self.network_media_options_hook_should_run() {
            let duplicate = self
                .network_media_options_apply_identity
                .as_ref()
                .is_some_and(|identity| {
                    identity.path == path
                        && (identity.media_generation == media_generation
                            || identity.media_generation.is_none())
                });
            if duplicate {
                return;
            }

            let recovered_after_on_load = !self.network_media_options_hook_is_ready();
            if recovered_after_on_load && self.network_media_options_hook_configuration_in_progress
            {
                return;
            }
            if recovered_after_on_load
                && let Err(error) = self.ensure_network_media_options_hook_configured()
            {
                self.queue_network_media_options_hook_degraded(error);
                return;
            }
            let attempt_id = self.begin_network_media_options_apply_attempt(media_generation, path);
            if self.embedded_network_media_options_apply_to_path(media_generation, path) {
                self.network_media_options_embedded_load = None;
            }
            if recovered_after_on_load {
                let outcome = match self
                    .apply_network_media_options_to_active_media_via_hook(path, attempt_id)
                {
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded) => return,
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia) => {
                        MpvNetworkMediaPolicyOutcome::NoActiveMedia
                    }
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged) => {
                        MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged
                    }
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated) => {
                        MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated
                    }
                    Err(error) if !self.network_media_options_hook_is_ready() => {
                        self.queue_network_media_options_hook_degraded(error);
                        return;
                    }
                    Err(error) => MpvNetworkMediaPolicyOutcome::Failed(error),
                };
                self.queue_network_media_policy_outcome(outcome);
            }
            return;
        }

        if !uses_network_media_options(path) {
            self.network_media_options_apply_identity = None;
            self.reset_network_media_policy_diagnostics();
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::LocalMediaUnchanged);
            let embedded_generation_is_current = self
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| Some(embedded.media_generation) == media_generation);
            if embedded_generation_is_current
                && !self.embedded_network_media_options_belong_to_pending_load()
            {
                self.network_media_options_embedded_load = None;
            }
            return;
        }

        let duplicate = self
            .network_media_options_apply_identity
            .as_ref()
            .is_some_and(|identity| {
                identity.path == path
                    && (identity.media_generation == media_generation
                        || identity.media_generation.is_none())
            });
        if duplicate {
            if let Some(identity) = self.network_media_options_apply_identity.as_mut()
                && identity.media_generation.is_none()
            {
                identity.media_generation = media_generation;
            }
            return;
        }

        if self.embedded_network_media_options_apply_to_path(media_generation, path) {
            self.begin_network_media_options_apply_attempt(media_generation, path);
            self.network_media_options_embedded_load = None;
            self.record_unverified_network_media_options_applied();
            self.queue_network_media_policy_outcome(
                MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated,
            );
            return;
        }
        if origin == AuthoritativePathObservationOrigin::PathEvent
            && self.embedded_network_media_options_belong_to_pending_load()
        {
            // Until a matching target establishes the pending load's generation, any event-time
            // network path can belong to the file being replaced. A later property poll can
            // safely establish that a mismatched external path is still authoritative.
            return;
        }
        let embedded_generation_is_current = self
            .network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| Some(embedded.media_generation) == media_generation);
        // Poll-time mismatches apply to the current external path but retain a pending embedded
        // marker in case Sorotte's requested target appears later. Only an orphaned marker is
        // obsolete here.
        if embedded_generation_is_current
            && !self.embedded_network_media_options_belong_to_pending_load()
        {
            self.network_media_options_embedded_load = None;
        }

        let attempt_id = self.begin_network_media_options_apply_attempt(media_generation, path);

        let outcome = match self.apply_network_media_options_for_attempt(path, attempt_id) {
            Ok(true) => {
                self.record_unverified_network_media_options_applied();
                MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated
            }
            Ok(false) => return,
            Err(error) => MpvNetworkMediaPolicyOutcome::Failed(error),
        };
        self.queue_network_media_policy_outcome(outcome);
    }

    fn bump_network_options_runtime_health_revision(&mut self) {
        self.network_media_options_runtime_health_revision = self
            .network_media_options_runtime_health_revision
            .wrapping_add(1)
            .max(1);
    }

    fn set_network_options_hook_health(&mut self, health: MpvNetworkOptionsHookHealth) {
        if self.network_media_options_hook_health != health {
            self.network_media_options_hook_health = health;
            self.bump_network_options_runtime_health_revision();
        }
    }

    fn set_network_media_policy_state(&mut self, state: MpvNetworkMediaPolicyState) {
        if self.network_media_options_policy_state != state {
            self.network_media_options_policy_state = state;
            self.bump_network_options_runtime_health_revision();
        }
    }

    fn next_network_options_event_sequence(&mut self) -> u64 {
        let sequence = self.next_network_options_event_sequence;
        self.next_network_options_event_sequence = self
            .next_network_options_event_sequence
            .wrapping_add(1)
            .max(1);
        sequence
    }

    fn queue_network_options_hook_health_transition(
        &mut self,
        transition: MpvNetworkOptionsHookHealthTransition,
    ) {
        match &transition {
            MpvNetworkOptionsHookHealthTransition::Recovered => {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
            }
            MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                if matches!(
                    self.network_media_options_hook_health,
                    MpvNetworkOptionsHookHealth::Degraded(_)
                ) {
                    return;
                }
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Degraded(
                    error.to_string(),
                ));
            }
        }
        if self.pending_network_options_hook_health_transitions.len()
            == MAX_PENDING_NETWORK_MEDIA_OPTIONS_TRANSITION_OUTCOMES
        {
            self.pending_network_options_hook_health_transitions
                .pop_front();
        }
        let sequence = self.next_network_options_event_sequence();
        self.pending_network_options_hook_health_transitions
            .push_back(SequencedNetworkOptionsEvent {
                sequence,
                value: transition,
            });
    }

    fn queue_network_media_policy_outcome(&mut self, outcome: MpvNetworkMediaPolicyOutcome) {
        let state = match &outcome {
            MpvNetworkMediaPolicyOutcome::NoActiveMedia => {
                MpvNetworkMediaPolicyState::NoActiveMedia
            }
            MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged => {
                MpvNetworkMediaPolicyState::LocalMediaUnchanged
            }
            MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                MpvNetworkMediaPolicyState::NetworkMediaUpdated
            }
            MpvNetworkMediaPolicyOutcome::Failed(error) => {
                MpvNetworkMediaPolicyState::Failed(error.to_string())
            }
        };
        self.set_network_media_policy_state(state);
        if self.pending_network_media_policy_outcomes.len()
            == MAX_PENDING_NETWORK_MEDIA_OPTIONS_TRANSITION_OUTCOMES
        {
            self.pending_network_media_policy_outcomes.pop_front();
        }
        let sequence = self.next_network_options_event_sequence();
        self.pending_network_media_policy_outcomes
            .push_back(SequencedNetworkOptionsEvent {
                sequence,
                value: outcome,
            });
    }

    fn queue_network_media_options_hook_degraded(&mut self, error: PlayerError) {
        self.queue_network_options_hook_health_transition(
            MpvNetworkOptionsHookHealthTransition::Degraded(error),
        );
    }

    fn queue_network_media_options_hook_recovered(&mut self) {
        let was_degraded = matches!(
            self.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Degraded(_)
        );
        self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        if was_degraded {
            self.queue_network_options_hook_health_transition(
                MpvNetworkOptionsHookHealthTransition::Recovered,
            );
        }
    }

    fn network_media_option_allows_diagnostic_value(name: &str) -> bool {
        NETWORK_MEDIA_OPTION_READBACK_ALLOWLIST.contains(&name)
    }

    fn network_media_options_desired_cache_options(&self) -> BTreeMap<String, String> {
        self.network_media_options
            .iter()
            .filter_map(|(name, value)| {
                Self::canonical_network_media_diagnostic_value(name, value)
                    .map(|value| (name.clone(), value))
            })
            .collect()
    }

    fn reset_network_media_policy_diagnostics(&mut self) {
        self.network_media_options_application_state = None;
        self.network_media_options_diagnostic_load_sequence = None;
        self.network_media_options_verification_complete = false;
        self.network_media_options_option_results.clear();
        self.network_media_options_effective_cache_options.clear();
    }

    fn record_unverified_network_media_options_applied(&mut self) {
        self.network_media_options_application_state =
            Some(MpvNetworkMediaPolicyApplicationState::Applied);
        self.network_media_options_diagnostic_load_sequence = None;
        self.network_media_options_verification_complete = false;
        self.network_media_options_option_results = self
            .network_media_options
            .keys()
            .map(|name| MpvNetworkOptionApplyResult {
                name: name.clone(),
                status: MpvNetworkOptionApplyStatus::Applied,
            })
            .collect();
        self.network_media_options_effective_cache_options.clear();
    }

    fn normalize_mpv_boolean(value: &str) -> Option<bool> {
        match value.trim().to_ascii_lowercase().as_str() {
            "yes" | "true" | "on" | "1" => Some(true),
            "no" | "false" | "off" | "0" => Some(false),
            _ => None,
        }
    }

    fn parse_mpv_byte_quantity(value: &str) -> Option<f64> {
        let normalized = value.trim().to_ascii_lowercase().replace(' ', "");
        let (number, multiplier) = [
            ("gib", 1024.0 * 1024.0 * 1024.0),
            ("mib", 1024.0 * 1024.0),
            ("kib", 1024.0),
            ("gb", 1_000_000_000.0),
            ("mb", 1_000_000.0),
            ("kb", 1_000.0),
            ("b", 1.0),
        ]
        .into_iter()
        .find_map(|(suffix, multiplier)| {
            normalized
                .strip_suffix(suffix)
                .map(|number| (number, multiplier))
        })
        .unwrap_or((normalized.as_str(), 1.0));
        let bytes = number.parse::<f64>().ok()? * multiplier;
        (bytes.is_finite() && bytes >= 0.0).then_some(bytes)
    }

    fn canonical_network_media_diagnostic_value(name: &str, value: &str) -> Option<String> {
        let trimmed = value.trim();
        match name {
            "cache" => match trimmed.to_ascii_lowercase().as_str() {
                "yes" => Some("yes".to_owned()),
                "no" => Some("no".to_owned()),
                "auto" => Some("auto".to_owned()),
                "auto-safe" => Some("auto-safe".to_owned()),
                _ => None,
            },
            "cache-pause" | "cache-pause-initial" | "cache-on-disk" => {
                Self::normalize_mpv_boolean(trimmed)
                    .map(|enabled| if enabled { "yes" } else { "no" }.to_owned())
            }
            "cache-pause-wait" | "cache-secs" => {
                let number = trimmed.parse::<f64>().ok()?;
                (number.is_finite() && number >= 0.0).then(|| number.to_string())
            }
            "demuxer-max-bytes" | "demuxer-max-back-bytes" => {
                let bytes = Self::parse_mpv_byte_quantity(trimmed)?;
                (bytes <= u64::MAX as f64).then(|| bytes.to_string())
            }
            _ => None,
        }
    }

    fn network_media_option_values_match(name: &str, desired: &str, effective: &str) -> bool {
        if desired.trim().eq_ignore_ascii_case(effective.trim()) {
            return true;
        }
        match name {
            "cache-pause" | "cache-pause-initial" | "cache-on-disk" => {
                Self::normalize_mpv_boolean(desired) == Self::normalize_mpv_boolean(effective)
                    && Self::normalize_mpv_boolean(desired).is_some()
            }
            "cache-pause-wait" | "cache-secs" => {
                let Some(desired) = desired.trim().parse::<f64>().ok() else {
                    return false;
                };
                let Some(effective) = effective.trim().parse::<f64>().ok() else {
                    return false;
                };
                desired.is_finite()
                    && effective.is_finite()
                    && (desired - effective).abs() <= 0.000_001
            }
            "demuxer-max-bytes" | "demuxer-max-back-bytes" => {
                let Some(desired) = Self::parse_mpv_byte_quantity(desired) else {
                    return false;
                };
                let Some(effective) = Self::parse_mpv_byte_quantity(effective) else {
                    return false;
                };
                (desired - effective).abs() <= 1.0
            }
            _ => false,
        }
    }

    fn record_network_media_option_application(
        &mut self,
        load_sequence: u64,
        hook_status: NetworkOptionsHookApplyStatus,
        verification_complete: bool,
        hook_results: Vec<NetworkOptionsHookOptionResult>,
        effective_options: BTreeMap<String, String>,
    ) -> MpvNetworkMediaPolicyApplicationState {
        let mut results = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for hook_result in hook_results {
            if !self.network_media_options.contains_key(&hook_result.name)
                || !seen.insert(hook_result.name.clone())
            {
                continue;
            }
            let status = match hook_result.status {
                NetworkOptionsHookOptionApplyStatus::Rejected => {
                    MpvNetworkOptionApplyStatus::Rejected
                }
                NetworkOptionsHookOptionApplyStatus::Applied
                    if verification_complete
                        && Self::network_media_option_allows_diagnostic_value(
                            &hook_result.name,
                        ) =>
                {
                    match (
                        self.network_media_options.get(&hook_result.name),
                        effective_options.get(&hook_result.name),
                    ) {
                        (Some(desired), Some(effective))
                            if Self::network_media_option_values_match(
                                &hook_result.name,
                                desired,
                                effective,
                            ) =>
                        {
                            MpvNetworkOptionApplyStatus::Applied
                        }
                        _ => MpvNetworkOptionApplyStatus::Mismatched,
                    }
                }
                NetworkOptionsHookOptionApplyStatus::Applied => {
                    MpvNetworkOptionApplyStatus::Applied
                }
            };
            results.push(MpvNetworkOptionApplyResult {
                name: hook_result.name,
                status,
            });
        }

        if verification_complete {
            for name in self.network_media_options.keys() {
                if seen.insert(name.clone()) {
                    results.push(MpvNetworkOptionApplyResult {
                        name: name.clone(),
                        status: MpvNetworkOptionApplyStatus::Mismatched,
                    });
                }
            }
        }

        let applied = results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Applied)
            .count();
        let problematic = results.len().saturating_sub(applied);
        let state = match hook_status {
            NetworkOptionsHookApplyStatus::Failed => MpvNetworkMediaPolicyApplicationState::Failed,
            NetworkOptionsHookApplyStatus::PartiallyApplied => {
                if applied == 0 {
                    MpvNetworkMediaPolicyApplicationState::Failed
                } else {
                    MpvNetworkMediaPolicyApplicationState::PartiallyApplied
                }
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated if problematic == 0 => {
                MpvNetworkMediaPolicyApplicationState::Applied
            }
            _ if applied == 0 => MpvNetworkMediaPolicyApplicationState::Failed,
            _ => MpvNetworkMediaPolicyApplicationState::PartiallyApplied,
        };

        self.network_media_options_application_state = Some(state);
        self.network_media_options_diagnostic_load_sequence = Some(load_sequence);
        self.network_media_options_verification_complete = verification_complete;
        self.network_media_options_option_results = results;
        self.network_media_options_effective_cache_options = effective_options
            .into_iter()
            .filter(|(name, _)| {
                self.network_media_options.contains_key(name)
                    && Self::network_media_option_allows_diagnostic_value(name)
            })
            .collect();
        state
    }

    fn network_media_option_application_error(
        &self,
        load_sequence: u64,
        source_kind: NetworkOptionsMediaTargetKind,
        stream_target_kind: NetworkOptionsMediaTargetKind,
        state: MpvNetworkMediaPolicyApplicationState,
    ) -> PlayerError {
        if state == MpvNetworkMediaPolicyApplicationState::Failed
            && self.network_media_options_option_results.is_empty()
        {
            return NetworkOptionsApplyDiagnostic::player_error(
                load_sequence,
                source_kind,
                stream_target_kind,
            );
        }
        let applied = self
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Applied)
            .count();
        let rejected = self
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Rejected)
            .count();
        let mismatched = self
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Mismatched)
            .count();
        let classification = match state {
            MpvNetworkMediaPolicyApplicationState::Applied => "applied",
            MpvNetworkMediaPolicyApplicationState::PartiallyApplied => "partially applied",
            MpvNetworkMediaPolicyApplicationState::Failed => "failed to apply",
        };
        PlayerError::OperationFailed(format!(
            "mpv {classification} the network-media policy for hook load {load_sequence} (source: {source_kind}, resolved target: {stream_target_kind}; {applied} applied, {rejected} rejected, {mismatched} mismatched)"
        ))
    }

    fn network_media_options_map(&self) -> serde_json::Map<String, Value> {
        self.network_media_options
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect()
    }

    fn parse_mpv_version(version: &str) -> Option<(u64, u64, u64)> {
        version
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .filter(|part| part.bytes().filter(|byte| *byte == b'.').count() >= 2)
            .find_map(|part| {
                let mut components = part.split('.');
                Some((
                    components.next()?.parse::<u64>().ok()?,
                    components.next()?.parse::<u64>().ok()?,
                    components.next()?.parse::<u64>().ok()?,
                ))
            })
    }

    fn send_network_media_loadfile(&mut self, path: &str) -> Result<(), PlayerError> {
        let options = Value::Object(self.network_media_options_map());
        self.send_ipc_command_if_attached_without_draining_events(json!([
            MPV_COMMAND_LOADFILE,
            path,
            MPV_LOADFILE_REPLACE,
            -1,
            options
        ]))
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_seconds
    }

    pub fn playback_rate(&self) -> f64 {
        if self.playback_rate == 0.0 {
            1.0
        } else {
            self.playback_rate
        }
    }

    pub fn paused_for_cache(&self) -> bool {
        self.paused_for_cache
    }

    pub fn cache_buffering_percent(&self) -> Option<f64> {
        self.cache_buffering_percent
    }

    pub fn media_generation(&self) -> Option<PlayerMediaGeneration> {
        self.player_lifecycle.current_media_generation()
    }

    pub fn transport_phase(&self) -> PlayerTransportPhase {
        self.transport_phase
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn volume(&self) -> f64 {
        self.volume.unwrap_or(100.0)
    }

    pub fn deinterlace(&self) -> bool {
        self.deinterlace
    }

    pub fn keepaspect(&self) -> bool {
        self.keepaspect
    }

    pub fn keepaspect_window(&self) -> bool {
        self.keepaspect_window
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn ontop(&self) -> bool {
        self.ontop
    }

    pub fn border(&self) -> bool {
        self.border
    }

    pub fn force_window(&self) -> bool {
        self.force_window
    }

    pub fn keep_open(&self) -> bool {
        self.keep_open
    }

    pub fn keep_open_pause(&self) -> bool {
        self.keep_open_pause
    }

    pub fn cursor_autohide_fs_only(&self) -> bool {
        self.cursor_autohide_fs_only
    }

    pub fn stop_screensaver(&self) -> bool {
        self.stop_screensaver
    }

    pub fn sub_visibility(&self) -> bool {
        self.sub_visibility
    }

    pub fn osd_bar(&self) -> bool {
        self.osd_bar
    }

    pub fn window_maximized(&self) -> bool {
        self.window_maximized
    }

    pub fn window_minimized(&self) -> bool {
        self.window_minimized
    }

    fn queue_ordered_player_event(&mut self, kind: PlayerOrderedEventKind) {
        let sequence = PlayerEventSequence::new(self.next_ordered_player_event_sequence);
        self.next_ordered_player_event_sequence = self
            .next_ordered_player_event_sequence
            .checked_add(1)
            .expect("mpv ordered player event sequence exhausted");
        if self.pending_ordered_player_events.len() >= MAX_PENDING_ORDERED_PLAYER_EVENTS {
            self.pending_ordered_player_events.pop_front();
            self.ordered_player_event_reacquisition_required = true;
        }
        self.pending_ordered_player_events
            .push_back(PlayerOrderedEvent::new(sequence, kind));
    }

    fn authoritative_ordered_player_snapshot(
        &self,
        authoritative_local_file: Option<LocalFileUpdate>,
        interrupted_command_progress: Vec<PlayerCommandProgress>,
        interrupted_media_load_outcomes: Vec<PlayerMediaLoadObservation>,
        authoritative_generation: Option<PlayerMediaGeneration>,
    ) -> Vec<PlayerOrderedEventKind> {
        let mut snapshot = Vec::with_capacity(
            interrupted_command_progress.len()
                + interrupted_media_load_outcomes.len()
                + usize::from(authoritative_local_file.is_some())
                + usize::from(authoritative_generation.is_some()),
        );
        // Semantic command/load outcomes are not ordinary state fields. Replay their exact
        // terminal meaning before the physical snapshot so a failed pre-start load cannot
        // disappear behind the absence of an active media generation, and a completed load is
        // never rewritten as an unknown failure.
        for progress in interrupted_command_progress {
            snapshot.push(PlayerOrderedEventKind::CommandProgress(progress));
        }
        for observation in interrupted_media_load_outcomes {
            snapshot.push(PlayerOrderedEventKind::MediaLoad(observation));
        }
        if let Some(update) = authoritative_local_file {
            snapshot.push(PlayerOrderedEventKind::LocalFile(
                PlayerLocalFileObservation::new(
                    update,
                    self.observation_media_generation(),
                    Some(self.observation_timestamp()),
                ),
            ));
        }
        let Some(generation) = authoritative_generation else {
            return snapshot;
        };
        let mut update = self.transport_update_for(generation);
        update.phase = Some(self.transport_phase);
        update.position_seconds = self.observed_state.position_seconds;
        update.playback_rate = self.observed_state.playback_rate;
        update.logical_pause = self.observed_state.logical_pause;
        update.paused_for_cache = self.observed_state.paused_for_cache;
        update.cache_buffering_percent = self.observed_state.cache_buffering_percent;
        update.seeking = self.observed_state.seeking;
        update.seekable = self.observed_state.seekable;
        update.seekable_ranges = Some(
            self.observed_state
                .seekable_ranges
                .clone()
                .unwrap_or_default(),
        );
        update.core_idle = self.observed_state.core_idle;
        update.demuxer_cache_idle = self.observed_state.demuxer_cache_idle;
        update.playback_restart_sequence = Some(self.playback_restart_sequence);
        update.eof_reached = self.observed_state.eof_reached;
        update.buffered_ahead_seconds = self.observed_state.buffered_ahead_seconds;
        update.buffered_ahead_bytes = self.observed_state.buffered_ahead_bytes;
        update.input_rate_bytes_per_second = self.observed_state.input_rate_bytes_per_second;
        snapshot.push(PlayerOrderedEventKind::Transport(update));
        snapshot
    }

    fn authoritative_reacquisition_command_progress(&self) -> Vec<PlayerCommandProgress> {
        let mut progress: Vec<_> = self
            .unacknowledged_terminal_command_progress
            .values()
            .copied()
            .collect();
        for pending in self
            .pending_tracked_commands
            .iter()
            .filter(|pending| pending.accepted_at.is_some())
        {
            if progress
                .iter()
                .any(|existing| existing.command_id == pending.id)
            {
                continue;
            }
            progress.push(PlayerCommandProgress::accepted(
                pending.id,
                pending.media_generation,
                Some(self.observation_timestamp()),
            ));
        }
        progress
    }

    fn acknowledge_last_delivered_ordered_semantic_outcomes(&mut self) {
        if self.ordered_player_event_reacquisition_requested_by_consumer {
            return;
        }
        for progress in self.last_delivered_ordered_command_progress.drain(..) {
            if progress.is_terminal() {
                self.unacknowledged_terminal_command_progress
                    .remove(&progress.command_id);
            }
        }
        for observation in self.last_delivered_ordered_media_load_outcomes.drain(..) {
            if let Some(index) =
                self.unacknowledged_media_load_outcomes
                    .iter()
                    .position(|pending| {
                        pending.media_generation == observation.media_generation
                            && pending.outcome == observation.outcome
                    })
            {
                self.unacknowledged_media_load_outcomes.remove(index);
            }
        }
    }

    pub fn queue_local_file_update(&mut self, update: LocalFileUpdate) {
        let media_generation = self.observation_media_generation();
        let observed_at = Some(self.observation_timestamp());
        self.queue_ordered_player_event(PlayerOrderedEventKind::LocalFile(
            PlayerLocalFileObservation::new(update.clone(), media_generation, observed_at),
        ));
        self.pending_local_file_generation = media_generation;
        self.pending_local_file_observed_at = observed_at;
        if let Some((attachment_epoch, attempt_id, media_generation)) =
            self.player_lifecycle.active_attempt().map(|attempt| {
                (
                    attempt.attachment_epoch,
                    attempt.id,
                    attempt.media_generation,
                )
            })
        {
            self.apply_lifecycle_input(PlayerLifecycleInput::LocalFileChanged {
                attachment_epoch,
                attempt_id,
                media_generation,
                update: update.clone(),
            });
        }
        self.pending_local_file_update = Some(update);
    }

    fn queue_media_load_outcome(&mut self, outcome: PlayerMediaLoadOutcome) {
        self.queue_media_load_outcome_for_generation(outcome, self.observation_media_generation());
    }

    fn queue_media_load_outcome_for_generation(
        &mut self,
        outcome: PlayerMediaLoadOutcome,
        media_generation: Option<PlayerMediaGeneration>,
    ) {
        let observation = PlayerMediaLoadObservation::new(
            outcome,
            media_generation,
            Some(self.observation_timestamp()),
        );
        self.queue_media_load_observation(observation);
    }

    fn queue_media_load_observation(&mut self, observation: PlayerMediaLoadObservation) {
        self.queue_ordered_player_event(PlayerOrderedEventKind::MediaLoad(observation.clone()));
        self.pending_media_load_outcomes
            .push_back(observation.clone());
        if !self
            .unacknowledged_media_load_outcomes
            .iter()
            .any(|pending| pending == &observation)
        {
            if self.unacknowledged_media_load_outcomes.len()
                >= MAX_UNACKNOWLEDGED_MEDIA_LOAD_OUTCOMES
            {
                self.unacknowledged_media_load_outcomes.pop_front();
            }
            self.unacknowledged_media_load_outcomes
                .push_back(observation);
        }
    }

    pub fn legacy_syncplay_ui_settings(&self) -> &LegacySyncplayUiSettings {
        &self.legacy_syncplay_ui_settings
    }

    pub fn last_simulated_legacy_syncplay_osd_message(
        &self,
    ) -> Option<&(String, LegacySyncplayOsdKind)> {
        self.last_simulated_legacy_syncplay_osd_message.as_ref()
    }

    pub fn legacy_syncplayintf_options_ready(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_bridge_instance_id.is_some()
            && self.legacy_syncplayintf_options_applied
            && self
                .legacy_syncplayintf_pending_options_generation
                .is_none()
    }

    pub fn legacy_syncplayintf_script_loaded(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
    }

    pub fn apply_pending_legacy_syncplayintf_options(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the syncplayintf bridge is not loaded".to_owned(),
            ));
        }
        if self.legacy_syncplayintf_options_applied {
            return Ok(());
        }
        self.send_legacy_syncplayintf_options_if_loaded()
    }

    pub fn legacy_syncplay_osd_placement_restore(&self) -> Option<(String, i64)> {
        self.legacy_syncplay_osd_placement_restore.clone()
    }

    pub fn set_legacy_syncplay_osd_placement_restore(&mut self, restore: Option<(String, i64)>) {
        self.legacy_syncplay_osd_placement_restore = restore;
    }

    pub fn set_property_string(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn set_property_i64(&mut self, name: &str, value: i64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn show_text(
        &mut self,
        text: &str,
        duration_ms: u64,
        level: i64,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SHOW_TEXT, text, duration_ms, level]))
    }

    pub fn load_legacy_syncplayintf_script(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), PlayerError> {
        if self.ipc_client.is_none() {
            if self.simulation_mode {
                self.legacy_syncplayintf_script_loaded = true;
                self.legacy_syncplayintf_bridge_instance_id =
                    Some("simulated-sorotte-syncplayintf".to_owned());
                self.legacy_syncplayintf_options_applied = true;
            }
            return Ok(());
        }

        if self.discover_legacy_syncplayintf_bridge(false)? {
            self.try_send_legacy_syncplayintf_options_if_pending();
            return Ok(());
        }

        let script_path = path.as_ref().to_string_lossy().into_owned();
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_LOAD_SCRIPT, script_path]))?;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        if !self.discover_legacy_syncplayintf_bridge(true)? {
            return Err(PlayerError::OperationFailed(
                "loaded the Sorotte syncplayintf resource, but its stable bridge did not answer discovery"
                    .to_owned(),
            ));
        }
        self.try_send_legacy_syncplayintf_options_if_pending();
        Ok(())
    }

    pub fn configure_legacy_syncplay_ui_settings(
        &mut self,
        settings: LegacySyncplayUiSettings,
    ) -> Result<(), PlayerError> {
        let syncplayintf_options_changed = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_differ(&settings);
        let placement_available = self.ipc_client.is_some() || self.simulation_mode;
        if placement_available && settings.should_move_osd() {
            if self.legacy_syncplay_osd_placement_restore.is_none() {
                let restore = match self.ipc_client.as_mut() {
                    Some(client) => {
                        let align = client
                            .get_property_string(MPV_PROPERTY_OSD_ALIGN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical alignment".to_owned(),
                                )
                            })?;
                        let margin = client
                            .get_property_i64(MPV_PROPERTY_OSD_MARGIN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical margin".to_owned(),
                                )
                            })?;
                        (align, margin)
                    }
                    None => ("top".to_owned(), 0),
                };
                self.legacy_syncplay_osd_placement_restore = Some(restore);
            }
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, "bottom")?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, settings.chat_osd_margin)?;
        } else if placement_available
            && let Some((align, margin)) =
                self.legacy_syncplay_osd_placement_restore.as_ref().cloned()
        {
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, &align)?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, margin)?;
            self.legacy_syncplay_osd_placement_restore = None;
        }
        self.legacy_syncplay_ui_settings = settings;
        if syncplayintf_options_changed {
            let runtime_bridge_was_active = matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
            );
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
            if runtime_bridge_was_active {
                self.legacy_syncplayintf_runtime_recovery_attempts = 0;
                self.legacy_syncplayintf_runtime_recovery_failure = None;
                self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::AcknowledgementTimeout,
                    "updated Chat/OSD settings are awaiting bridge acknowledgement",
                    false,
                );
                self.attempt_sorotte_bridge_runtime_recovery();
            } else {
                self.try_send_legacy_syncplayintf_options_if_pending();
            }
        }
        Ok(())
    }

    pub fn configure_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn retry_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn sorotte_bridge_health(&self) -> SorotteBridgeHealth {
        self.sorotte_bridge_health.clone()
    }

    /// Returns the exact settings generation acknowledged by the current bridge attachment.
    pub fn sorotte_bridge_acknowledged_generation(&self) -> Option<u64> {
        self.legacy_syncplayintf_options_applied
            .then_some(self.legacy_syncplayintf_acknowledged_options_generation)
            .flatten()
    }

    /// Advances bounded bridge maintenance and returns the oldest unconsumed health transition.
    ///
    /// Bridge transitions are independent of core mpv JSON IPC health. A `Recovering` or
    /// `Degraded` transition gates player chat and causes OSD output to use mpv's `show-text`, but
    /// does not detach the adapter or make playback commands unavailable.
    pub fn take_sorotte_bridge_health_transition(&mut self) -> Option<SorotteBridgeHealth> {
        self.maintain_runtime_integrations();
        self.pending_sorotte_bridge_health_transitions.pop_front()
    }

    /// Services only nonblocking lease/event work and returns the oldest bridge-health change.
    /// Async owners should use this variant so draining notifications cannot enter configuration
    /// retry loops or sleep while unrelated I/O futures are waiting to be polled.
    pub fn take_sorotte_bridge_health_transition_nonblocking(
        &mut self,
    ) -> Option<SorotteBridgeHealth> {
        PlayerAdapter::maintain_runtime_leases_nonblocking(self);
        self.pending_sorotte_bridge_health_transitions.pop_front()
    }

    /// Services only nonblocking lease/event work and returns the oldest player-chat request.
    /// This is the async-owner counterpart to [`PlayerAdapter::take_pending_chat_request`].
    pub fn take_pending_chat_request_nonblocking(&mut self) -> Option<String> {
        PlayerAdapter::maintain_runtime_leases_nonblocking(self);
        self.pending_chat_requests.pop_front()
    }

    pub fn mark_sorotte_bridge_degraded(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.degrade_sorotte_bridge(kind, reason)
    }

    fn configure_bundled_sorotte_bridge_inner(
        &mut self,
        retry_window: Duration,
    ) -> SorotteBridgeHealth {
        let bridge_requested = self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge();
        if !bridge_requested && !self.legacy_syncplayintf_script_loaded {
            return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        }

        if !self.legacy_syncplayintf_script_loaded {
            match self.discover_loaded_legacy_syncplayintf_script() {
                Ok(true) => {}
                Ok(false) if bridge_requested => {
                    let script_path = match materialize_bundled_sorotte_bridge() {
                        Ok(path) => path,
                        Err(error) => {
                            return self.degrade_sorotte_bridge(
                                SorotteBridgeFailureKind::ResourceMaterialization,
                                format!(
                                    "failed to materialize Sorotte's bundled mpv bridge: {error}"
                                ),
                            );
                        }
                    };
                    if let Err(error) = self.load_legacy_syncplayintf_script(&script_path) {
                        return self.degrade_sorotte_bridge(
                            SorotteBridgeFailureKind::ScriptLoad,
                            format!(
                                "failed to load Sorotte's bundled mpv bridge from '{}': {error}",
                                script_path.display()
                            ),
                        );
                    }
                }
                Ok(false) => {
                    return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
                }
                Err(error) => {
                    return self.degrade_sorotte_bridge(
                        SorotteBridgeFailureKind::Discovery,
                        format!("failed to discover Sorotte's mpv bridge: {error}"),
                    );
                }
            }
        }

        let deadline = Instant::now() + retry_window;
        let mut last_acknowledged_error = None;
        let last_error = loop {
            let error = match self.apply_pending_legacy_syncplayintf_options() {
                Ok(()) if self.legacy_syncplayintf_options_ready() => {
                    let health = if bridge_requested {
                        SorotteBridgeHealth::Ready
                    } else {
                        SorotteBridgeHealth::Disabled
                    };
                    return self.set_sorotte_bridge_health(health);
                }
                Ok(()) => {
                    "Sorotte's mpv bridge did not report that its settings are ready".to_owned()
                }
                Err(error) => error.to_string(),
            };
            if let Some(acknowledged_error) = self.legacy_syncplayintf_options_ack_error.clone() {
                last_acknowledged_error = Some(acknowledged_error);
            }
            if Instant::now() >= deadline {
                break error;
            }
            std::thread::sleep(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_INTERVAL);
        };

        let acknowledged_error = self
            .legacy_syncplayintf_options_ack_error
            .clone()
            .or(last_acknowledged_error);
        let reason = acknowledged_error.clone().unwrap_or(last_error);
        let kind =
            classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some());
        self.degrade_sorotte_bridge(kind, reason)
    }

    fn set_sorotte_bridge_health(&mut self, health: SorotteBridgeHealth) -> SorotteBridgeHealth {
        if self.sorotte_bridge_health == health {
            return health;
        }
        self.sorotte_bridge_health = health.clone();
        if self.pending_sorotte_bridge_health_transitions.back() != Some(&health) {
            self.pending_sorotte_bridge_health_transitions
                .push_back(health.clone());
        }
        if matches!(
            health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Disabled
        ) {
            self.legacy_syncplayintf_runtime_rediscovery_required = false;
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
            self.legacy_syncplayintf_runtime_recovery_failure = None;
        }
        health
    }

    fn degrade_sorotte_bridge(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required =
            kind == SorotteBridgeFailureKind::LeaseBusy;
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Degraded(SorotteBridgeFailure::new(
            kind, reason,
        )))
    }

    pub fn show_syncplay_legacy_message(
        &mut self,
        message: &str,
        kind: LegacySyncplayOsdKind,
    ) -> Result<(), PlayerError> {
        if message.trim().is_empty() || !self.legacy_syncplay_ui_settings.show_osd {
            return Ok(());
        }
        if self.simulation_mode {
            self.last_simulated_legacy_syncplay_osd_message = Some((message.to_owned(), kind));
        }

        let duration_ms = match kind {
            LegacySyncplayOsdKind::Notification => {
                self.legacy_syncplay_ui_settings.notification_timeout_ms
            }
            LegacySyncplayOsdKind::Alert => self.legacy_syncplay_ui_settings.alert_timeout_ms,
        };
        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            let script_message_name = match kind {
                LegacySyncplayOsdKind::Notification => "notification-osd-neutral",
                LegacySyncplayOsdKind::Alert => "alert-osd-neutral",
            };
            match self.send_syncplayintf_script_message(
                script_message_name,
                &sanitize_legacy_syncplay_script_message_text(message),
            ) {
                Ok(()) => return Ok(()),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::IpcCommand,
                    format!("Sorotte's mpv bridge rejected {script_message_name}: {error}"),
                    true,
                ),
            }
        }
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    pub fn show_syncplay_legacy_chat_message(&mut self, message: &str) -> Result<(), PlayerError> {
        if message.trim().is_empty() {
            return Ok(());
        }

        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            match self.send_syncplayintf_script_message(
                "chat",
                &sanitize_legacy_syncplay_script_message_text(message),
            ) {
                Ok(()) => return Ok(()),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::IpcCommand,
                    format!("Sorotte's mpv bridge rejected chat output: {error}"),
                    true,
                ),
            }
        }

        let maybe_duration_ms = if self.legacy_syncplay_ui_settings.chat_output_enabled {
            Some(self.legacy_syncplay_ui_settings.chat_timeout_ms)
        } else if self.legacy_syncplay_ui_settings.show_osd {
            Some(self.legacy_syncplay_ui_settings.notification_timeout_ms)
        } else {
            None
        };

        let Some(duration_ms) = maybe_duration_ms else {
            return Ok(());
        };
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    fn send_syncplayintf_script_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            self.legacy_syncplayintf_script_name.as_str(),
            message_name,
            payload
        ]))
    }

    fn send_syncplayintf_probe_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<bool, PlayerError> {
        let result = match self.ipc_client.as_mut() {
            Some(client) => client.send_compatibility_probe_expect_success(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
                message_name,
                payload
            ])),
            None if self.simulation_mode => return Ok(true),
            None => return Err(PlayerError::NotConnected),
        };
        self.drain_ipc_events_if_attached();
        match result {
            Ok(()) => Ok(true),
            Err(error) if error.is_server_rejection() => Ok(false),
            Err(error) => Err(PlayerError::OperationFailed(error.into_message())),
        }
    }

    pub fn discover_loaded_legacy_syncplayintf_script(&mut self) -> Result<bool, PlayerError> {
        self.discover_legacy_syncplayintf_bridge(false)
    }

    fn discover_legacy_syncplayintf_bridge(
        &mut self,
        wait_for_registration: bool,
    ) -> Result<bool, PlayerError> {
        if self.simulation_mode {
            self.legacy_syncplayintf_script_loaded = true;
            self.legacy_syncplayintf_bridge_instance_id =
                Some("simulated-sorotte-syncplayintf".to_owned());
            self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
            return Ok(true);
        }

        let nonce = self.legacy_syncplayintf_next_ping_nonce;
        self.legacy_syncplayintf_next_ping_nonce = self
            .legacy_syncplayintf_next_ping_nonce
            .wrapping_add(1)
            .max(1);
        self.legacy_syncplayintf_pending_ping_nonce = Some(nonce);
        let payload = json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "nonce": nonce,
        })
        .to_string();
        let mut target_accepted_a_ping = false;
        let attempts = if wait_for_registration {
            LEGACY_SYNCPLAYINTF_REGISTRATION_ATTEMPTS
        } else {
            LEGACY_SYNCPLAYINTF_DISCOVERY_ATTEMPTS
        };
        for _ in 0..attempts {
            let ping_accepted =
                self.send_syncplayintf_probe_message(LEGACY_SYNCPLAYINTF_PING_MESSAGE, &payload)?;
            target_accepted_a_ping |= ping_accepted;
            if !ping_accepted {
                if !wait_for_registration {
                    self.legacy_syncplayintf_pending_ping_nonce = None;
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_some()
                && let Some(client) = self.ipc_client.as_mut()
            {
                let _ = client.get_property(MPV_PROPERTY_PAUSE);
                self.drain_ipc_events_if_attached();
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_none()
                && self.legacy_syncplayintf_bridge_instance_id.is_some()
            {
                self.legacy_syncplayintf_script_loaded = true;
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.legacy_syncplayintf_pending_ping_nonce = None;
        if target_accepted_a_ping {
            return Err(PlayerError::OperationFailed(
                "the stable Sorotte syncplayintf target accepted discovery messages but did not return a valid pong; refusing to load a duplicate bridge"
                    .to_owned(),
            ));
        }
        Ok(false)
    }

    fn send_legacy_syncplayintf_options_if_loaded(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the Sorotte syncplayintf bridge has not been discovered".to_owned(),
            ));
        }
        if self.simulation_mode {
            let generation = self.legacy_syncplayintf_next_options_generation;
            self.legacy_syncplayintf_next_options_generation = self
                .legacy_syncplayintf_next_options_generation
                .wrapping_add(1)
                .max(1);
            self.legacy_syncplayintf_options_applied = true;
            self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
            return Ok(());
        }

        let bridge_instance_id = self
            .legacy_syncplayintf_bridge_instance_id
            .clone()
            .ok_or_else(|| {
                PlayerError::OperationFailed(
                    "the Sorotte syncplayintf bridge instance is unknown".to_owned(),
                )
            })?;
        let generation = match self.legacy_syncplayintf_pending_options_generation {
            Some(generation) => generation,
            None => {
                let generation = self.legacy_syncplayintf_next_options_generation;
                self.legacy_syncplayintf_next_options_generation = self
                    .legacy_syncplayintf_next_options_generation
                    .wrapping_add(1)
                    .max(1);
                self.legacy_syncplayintf_pending_options_generation = Some(generation);
                generation
            }
        };
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_options_ack_error = None;
        let payload = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_payload(
                &bridge_instance_id,
                &self.legacy_syncplayintf_owner_id,
                &self.legacy_syncplayintf_attachment_id,
                generation,
                LEGACY_SYNCPLAYINTF_OWNER_LEASE_MS,
            );

        self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_SET_OPTIONS_MESSAGE, &payload)?;
        if !self.legacy_syncplayintf_options_applied
            && let Some(client) = self.ipc_client.as_mut()
        {
            let _ = client.get_property(MPV_PROPERTY_PAUSE);
            self.drain_ipc_events_if_attached();
        }
        if self.legacy_syncplayintf_options_applied {
            self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            return Ok(());
        }
        Err(PlayerError::OperationFailed(
            self.legacy_syncplayintf_options_ack_error
                .clone()
                .unwrap_or_else(|| {
                    format!(
                        "Sorotte syncplayintf did not acknowledge settings generation {generation}"
                    )
                }),
        ))
    }

    fn try_send_legacy_syncplayintf_options_if_pending(&mut self) {
        if self.legacy_syncplayintf_options_applied
            || self.legacy_syncplayintf_lease_reacquire_required
            || matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Recovering | SorotteBridgeHealth::Degraded(_)
            )
        {
            return;
        }

        let _ = self.send_legacy_syncplayintf_options_if_loaded();
    }

    fn ensure_legacy_syncplayintf_ready(&mut self) -> bool {
        self.try_send_legacy_syncplayintf_options_if_pending();
        self.legacy_syncplayintf_options_ready()
    }

    fn legacy_syncplayintf_controller_payload(&self) -> Option<String> {
        Some(
            json!({
                "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
                "bridgeInstanceId": self.legacy_syncplayintf_bridge_instance_id.as_deref()?,
                "ownerId": self.legacy_syncplayintf_owner_id.as_str(),
                "attachmentId": self.legacy_syncplayintf_attachment_id.as_str(),
            })
            .to_string(),
        )
    }

    fn begin_sorotte_bridge_runtime_recovery(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
        rediscovery_required: bool,
    ) {
        if !matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
        ) {
            return;
        }
        let reason = reason.into();
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        }
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required = true;
        self.legacy_syncplayintf_runtime_rediscovery_required |= rediscovery_required;
        self.legacy_syncplayintf_runtime_recovery_failure =
            Some(SorotteBridgeFailure::new(kind, reason));
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Recovering);
    }

    fn attempt_sorotte_bridge_runtime_recovery(&mut self) {
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering)
            || (self.legacy_syncplayintf_runtime_recovery_attempts > 0
                && self
                    .legacy_syncplayintf_last_heartbeat_at
                    .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL))
        {
            return;
        }
        self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());

        let mut forced_failure_kind = None;
        let result = if self.legacy_syncplayintf_runtime_rediscovery_required {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {
                    self.legacy_syncplayintf_runtime_rediscovery_required = false;
                    self.send_legacy_syncplayintf_options_if_loaded()
                }
                Ok(false) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(PlayerError::OperationFailed(
                        "Sorotte's stable mpv bridge target is no longer registered".to_owned(),
                    ))
                }
                Err(error) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(error)
                }
            }
        } else {
            self.send_legacy_syncplayintf_options_if_loaded()
        };

        if result.is_ok() && self.legacy_syncplayintf_options_ready() {
            let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                SorotteBridgeHealth::Ready
            } else {
                SorotteBridgeHealth::Disabled
            };
            self.set_sorotte_bridge_health(health);
            return;
        }
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            return;
        }

        self.legacy_syncplayintf_runtime_recovery_attempts += 1;
        if let Err(error) = result {
            let acknowledged_error = self.legacy_syncplayintf_options_ack_error.clone();
            let reason = acknowledged_error
                .clone()
                .unwrap_or_else(|| error.to_string());
            let kind = forced_failure_kind.unwrap_or_else(|| {
                classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some())
            });
            self.legacy_syncplayintf_runtime_recovery_failure =
                Some(SorotteBridgeFailure::new(kind, reason));
        }

        if self.legacy_syncplayintf_runtime_recovery_attempts
            >= LEGACY_SYNCPLAYINTF_RUNTIME_RECOVERY_ATTEMPTS
        {
            let failure = self
                .legacy_syncplayintf_runtime_recovery_failure
                .clone()
                .unwrap_or_else(|| {
                    SorotteBridgeFailure::new(
                        SorotteBridgeFailureKind::AcknowledgementTimeout,
                        "Sorotte's mpv bridge did not acknowledge bounded runtime recovery",
                    )
                });
            self.degrade_sorotte_bridge(failure.kind, failure.reason);
        }
    }

    fn maintain_legacy_syncplayintf_lease(&mut self) {
        self.drain_ipc_events_if_attached();
        if matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Disabled | SorotteBridgeHealth::Degraded(_)
        ) {
            return;
        }
        if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }

        if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge()
            && self
                .legacy_syncplayintf_last_discovery_at
                .is_none_or(|last| last.elapsed() >= LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL)
        {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {}
                Ok(false) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    "Sorotte's stable mpv bridge target is no longer registered",
                    true,
                ),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    format!("failed to rediscover Sorotte's mpv bridge: {error}"),
                    true,
                ),
            }
            if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
                self.attempt_sorotte_bridge_runtime_recovery();
                return;
            }
        }

        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            self.legacy_syncplayintf_last_heartbeat_at = None;
            self.legacy_syncplayintf_pending_heartbeat_command_id = None;
            return;
        }
        if !self.legacy_syncplayintf_options_ready() {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::AcknowledgementTimeout,
                "Sorotte's mpv bridge lost its acknowledged runtime settings",
                false,
            );
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }
        if self
            .legacy_syncplayintf_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let Some(payload) = self.legacy_syncplayintf_controller_payload() else {
            return;
        };
        match self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_HEARTBEAT_MESSAGE, &payload)
        {
            Ok(()) if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Ready) => {
                self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            }
            Ok(()) => {
                self.legacy_syncplayintf_last_heartbeat_at = None;
                self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                self.attempt_sorotte_bridge_runtime_recovery();
            }
            Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::IpcCommand,
                format!("failed to renew Sorotte's mpv bridge lease: {error}"),
                true,
            ),
        }
    }

    /// Queues terminal, one-way releases for Sorotte's core hook and optional bridge, then clears
    /// their local attachment state.
    ///
    /// This is a shutdown-only operation. If an IPC final write is queued, the current JSON IPC
    /// client becomes unusable; callers should invoke this immediately before detaching or
    /// replacing the adapter. Lease expiry remains the fallback when the best-effort write cannot
    /// be queued or completed.
    pub fn release_sorotte_bridge_best_effort(&mut self) {
        let mut final_commands = Vec::with_capacity(4);
        if let Some((align_y, margin_y)) = self.legacy_syncplay_osd_placement_restore.take() {
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_ALIGN_Y,
                align_y
            ]));
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_MARGIN_Y,
                margin_y
            ]));
        }
        if self.network_media_options_hook_ownership_possible {
            final_commands.push(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
                SOROTTE_NETWORK_OPTIONS_RELEASE_MESSAGE,
                self.network_media_options_hook_controller_payload(),
            ]));
        }
        if self.legacy_syncplayintf_script_loaded
            && let Some(payload) = self.legacy_syncplayintf_controller_payload()
        {
            final_commands.push(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                self.legacy_syncplayintf_script_name.as_str(),
                LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE,
                payload
            ]));
        }
        if !final_commands.is_empty()
            && let Some(client) = self.ipc_client.as_mut()
        {
            client.send_final_commands_best_effort(final_commands);
        }
        self.reset_network_media_options_attachment_state();
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.pending_chat_requests.clear();
        // Release is terminal for this endpoint; queued observations are no longer actionable.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.sorotte_bridge_health = SorotteBridgeHealth::Disabled;
    }

    fn ensure_observers_registered_if_attached(&mut self) {
        if self.observers_registered {
            return;
        }
        if self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_PATH_ID, MPV_PROPERTY_PATH),
            (MPV_OBS_DURATION_ID, MPV_PROPERTY_DURATION),
            (MPV_OBS_FILE_SIZE_ID, MPV_PROPERTY_FILE_SIZE),
            (MPV_OBS_PAUSE_ID, MPV_PROPERTY_PAUSE),
            (MPV_OBS_TIME_POS_ID, MPV_PROPERTY_TIME_POS),
            (MPV_OBS_SPEED_ID, MPV_PROPERTY_SPEED),
            (MPV_OBS_PAUSED_FOR_CACHE_ID, MPV_PROPERTY_PAUSED_FOR_CACHE),
            (
                MPV_OBS_CACHE_BUFFERING_STATE_ID,
                MPV_PROPERTY_CACHE_BUFFERING_STATE,
            ),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            let registration_result = ipc_client.observe_property(observer_id, property_name);
            if registration_result.is_err() {
                return;
            }
            self.drain_ipc_events_if_attached();
        }
        self.observers_registered = true;
    }

    fn ensure_transport_observers_registered_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        if self.transport_observers_registered || self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_SEEKING_ID, MPV_PROPERTY_SEEKING),
            (MPV_OBS_SEEKABLE_ID, MPV_PROPERTY_SEEKABLE),
            (MPV_OBS_CORE_IDLE_ID, MPV_PROPERTY_CORE_IDLE),
            (
                MPV_OBS_DEMUXER_CACHE_STATE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_STATE,
            ),
            (
                MPV_OBS_DEMUXER_CACHE_IDLE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_IDLE,
            ),
            // Observe both forms: the full metadata map provides a resilient
            // fallback while the narrower subproperty avoids retransmitting
            // unrelated tags.
            (MPV_OBS_YTDL_IS_LIVE_ID, MPV_PROPERTY_YTDL_IS_LIVE),
            (MPV_OBS_METADATA_ID, MPV_PROPERTY_METADATA),
            (MPV_OBS_EOF_REACHED_ID, MPV_PROPERTY_EOF_REACHED),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            // Individual properties can be unavailable for a particular
            // media source or build. One rejection must not prevent the
            // remaining lifecycle properties from registering.
            let _ = ipc_client.observe_property(observer_id, property_name);
            self.drain_ipc_events_if_attached();
        }
        self.transport_observers_registered = true;
    }

    fn poll_ipc_local_file_update_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_local_file_update.is_some() {
            return;
        }
        if self.pending_load_request.is_none() && self.last_polled_local_file_update.is_some() {
            return;
        }

        let polled_update = self.poll_local_file_update_from_mpv_coherent();

        let Ok(polled_update) = polled_update else {
            return;
        };
        let Some(polled_update) = polled_update else {
            self.observe_authoritative_path_for_network_options(
                None,
                AuthoritativePathObservationOrigin::Poll,
            );
            return;
        };

        if self.pending_load_request().is_some() {
            let authoritative_update = polled_update.clone();
            let authoritative_path = polled_update.path.clone();
            let pending_load_completed =
                self.complete_pending_load_request_from_polled_update_if_ready(polled_update);
            if !pending_load_completed {
                self.current_path = authoritative_update.path.clone();
                self.observed_state.path = authoritative_update.path;
                self.observed_state.duration_seconds = authoritative_update.duration_seconds;
                self.observed_state.size_bytes = authoritative_update.size_bytes;
                self.path_metadata_generation = self.observation_media_generation();
                self.duration_metadata_generation = self.observation_media_generation();
                if self.refresh_timeline_kind_from_metadata() {
                    let update = self.transport_update();
                    self.queue_transport_telemetry_update(update);
                }
            }
            if let Some(path) = authoritative_path.as_deref() {
                self.observe_authoritative_path_for_network_options(
                    Some(path),
                    AuthoritativePathObservationOrigin::Poll,
                );
            }
            self.drain_ipc_events_if_attached();
            return;
        }

        let authoritative_path = polled_update.path.clone();
        self.observed_state.path = authoritative_path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.current_path = polled_update.path.clone();
        self.path_metadata_generation = self.observation_media_generation();
        self.duration_metadata_generation = self.observation_media_generation();
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        if Self::local_file_update_ready_for_sync(&polled_update) {
            self.record_local_file_update_if_changed(polled_update);
        }
        if let Some(path) = authoritative_path.as_deref() {
            self.observe_authoritative_path_for_network_options(
                Some(path),
                AuthoritativePathObservationOrigin::Poll,
            );
        }
        self.drain_ipc_events_if_attached();
    }

    fn poll_local_file_update_from_mpv(
        ipc_client: &mut MpvJsonIpcClient,
    ) -> Result<Option<LocalFileUpdate>, String> {
        let Some(path) = ipc_client.get_property_string(MPV_PROPERTY_PATH)? else {
            return Ok(None);
        };

        let mut local_file_update = Self::local_file_update_for_path(path.as_str());

        if let Some(duration_seconds) = ipc_client.get_property_f64(MPV_PROPERTY_DURATION)? {
            local_file_update = local_file_update.with_duration_seconds(duration_seconds);
        }

        if let Some(size_bytes) = ipc_client.get_property_u64(MPV_PROPERTY_FILE_SIZE)? {
            local_file_update = local_file_update.with_size_bytes(size_bytes);
        }

        Ok(Some(local_file_update))
    }

    fn poll_local_file_update_from_mpv_coherent(
        &mut self,
    ) -> Result<Option<LocalFileUpdate>, String> {
        let generation_before_poll = self.observation_media_generation();
        let initial_update = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return Ok(None);
            };
            Self::poll_local_file_update_from_mpv(ipc_client)
        };

        // Events collected by the composite property reads precede the response that collected
        // them, but some can be newer than the initial path response because duration and size
        // are queried afterward. Reduce them without firing network-option side effects, then
        // re-read path to establish one final authoritative boundary.
        let observed_interleaved_events = self.drain_ipc_events_without_network_options_flush();
        let initial_update = match initial_update {
            Ok(update) => update,
            Err(error) => {
                self.deferred_network_media_options_observation = None;
                return Err(error);
            }
        };
        if !observed_interleaved_events {
            self.deferred_network_media_options_observation = None;
            return Ok(initial_update);
        }

        let final_path = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                self.deferred_network_media_options_observation = None;
                return Ok(None);
            };
            ipc_client.get_property_string(MPV_PROPERTY_PATH)
        };
        // Direct events collected before this response are older than it. A handler can,
        // however, issue its own nested poll while reducing those events; that nested Poll
        // observation completes afterward and must outrank this captured response.
        self.deferred_network_media_options_observation = None;
        self.drain_ipc_events_without_network_options_flush();
        let nested_poll_observation = self
            .deferred_network_media_options_observation
            .take()
            .filter(|observation| observation.origin == AuthoritativePathObservationOrigin::Poll);
        let final_path = match nested_poll_observation {
            Some(observation) => observation.path,
            None => final_path?,
        };
        let Some(final_path) = final_path else {
            return Ok(None);
        };

        let initial_path = initial_update
            .as_ref()
            .and_then(|update| update.path.as_deref());
        if initial_path == Some(final_path.as_str())
            && self.observation_media_generation() == generation_before_poll
        {
            return Ok(initial_update);
        }

        // Metadata read before a path/generation change can describe the replaced file. Keep the
        // final path but let a later poll repopulate duration/size for its authoritative media.
        Ok(Some(Self::local_file_update_for_path(&final_path)))
    }

    fn poll_paused_position_telemetry_if_attached(&mut self) {
        if !self.paused {
            return;
        }

        let now = Instant::now();
        if self
            .last_paused_position_poll_at
            .is_some_and(|last_poll| now.duration_since(last_poll) < PAUSED_POSITION_POLL_INTERVAL)
        {
            return;
        }
        self.last_paused_position_poll_at = Some(now);

        let polled_position = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            ipc_client.get_property_f64(MPV_PROPERTY_TIME_POS)
        };
        self.drain_ipc_events_if_attached();

        let Ok(Some(position_seconds)) = polled_position else {
            return;
        };
        if !position_seconds.is_finite() || (self.position_seconds - position_seconds).abs() < 1e-6
        {
            return;
        }

        self.position_seconds = position_seconds;
        self.observed_state.position_seconds = Some(position_seconds);
        self.observe_interrupted_network_stream_recovery_progress(position_seconds);
        self.queue_playback_telemetry_update(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(position_seconds),
        );
        let update = self
            .transport_update()
            .with_position_seconds(position_seconds);
        self.queue_transport_telemetry_update(update);
        if let Some(media_generation) = self.player_lifecycle.active_media_generation() {
            let lifecycle_epoch = self.lifecycle_epoch();
            let observed_sequence = self.player_lifecycle.last_event_sequence();
            self.apply_lifecycle_input(PlayerLifecycleInput::PositionObserved {
                attachment_epoch: lifecycle_epoch,
                media_generation,
                observed_sequence,
                position_seconds,
            });
        }
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Position(position_seconds),
        );
    }

    fn record_local_file_update_if_changed(&mut self, update: LocalFileUpdate) {
        if self.last_polled_local_file_update.as_ref() != Some(&update) {
            self.last_polled_local_file_update = Some(update.clone());
            self.queue_local_file_update(update);
        }
    }

    fn complete_pending_load_request_from_polled_update_if_ready(
        &mut self,
        polled_update: LocalFileUpdate,
    ) -> bool {
        let Some(requested_target) = self.pending_load_request.clone() else {
            return false;
        };
        if !Self::local_file_update_matches_request(&polled_update, &requested_target)
            || !Self::local_file_update_ready_for_sync(&polled_update)
        {
            return false;
        }
        #[cfg(not(test))]
        let (attempt_id, generation, playlist_entry_id) = {
            let Some(attempt) = self.player_lifecycle.active_attempt() else {
                self.lifecycle_reconciliation_due = true;
                return false;
            };
            let Some(playlist_entry_id) = attempt.playlist_entry_id else {
                self.lifecycle_reconciliation_due = true;
                return false;
            };
            if attempt.requested_target != requested_target || attempt.state.is_terminal() {
                return false;
            }
            (attempt.id, attempt.media_generation, playlist_entry_id)
        };
        #[cfg(test)]
        let generation = self
            .pending_load_generation
            .expect("legacy scripted polling requires its submitted generation");
        let requested_target = self
            .pending_load_request
            .take()
            .expect("pending request should still be present");
        #[cfg(not(test))]
        {
            let attachment_epoch = self.lifecycle_epoch();
            self.apply_lifecycle_input(PlayerLifecycleInput::FileLoaded {
                attachment_epoch,
                playlist_entry_id: Some(playlist_entry_id),
                loaded_target: polled_update.path.clone(),
            });
            if self.player_lifecycle.active_load_attempt != Some(attempt_id) {
                self.pending_load_request = Some(requested_target);
                return false;
            }
        }
        self.pending_load_generation = None;
        self.active_media_generation = Some(generation);
        self.active_file_loaded = true;
        self.current_path = polled_update.path.clone();
        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.path_metadata_generation = Some(generation);
        self.duration_metadata_generation = Some(generation);
        self.refresh_timeline_kind_from_metadata();
        self.record_local_file_update_if_changed(polled_update.clone());
        self.queue_media_load_outcome(PlayerMediaLoadOutcome::success(
            requested_target,
            polled_update.path,
        ));
        self.refresh_inferred_transport_phase();
        true
    }

    fn queue_playback_telemetry_update(&mut self, update: PlayerPlaybackTelemetryUpdate) {
        match self.pending_playback_telemetry_update.as_mut() {
            Some(pending) => {
                if let Some(paused) = update.paused
                    && !(paused && pending.paused_for_cache == Some(true))
                {
                    pending.paused = Some(paused);
                }
                if let Some(position_seconds) = update.position_seconds {
                    pending.position_seconds = Some(position_seconds);
                }
                if let Some(playback_rate) = update.playback_rate {
                    pending.playback_rate = Some(playback_rate);
                }
                if let Some(paused_for_cache) = update.paused_for_cache {
                    pending.paused_for_cache = Some(paused_for_cache);
                    if paused_for_cache && pending.paused == Some(true) {
                        pending.paused = None;
                    }
                }
                if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                    pending.cache_buffering_percent = Some(cache_buffering_percent);
                }
            }
            None => {
                let mut update = update;
                if update.paused_for_cache == Some(true) && update.paused == Some(true) {
                    update.paused = None;
                }
                self.pending_playback_telemetry_update = Some(update);
            }
        }
    }

    fn allocate_media_generation(&mut self) -> PlayerMediaGeneration {
        let generation = self.next_media_generation.max(1);
        self.next_media_generation = generation.wrapping_add(1).max(1);
        PlayerMediaGeneration::new(generation)
    }

    fn pending_load_generation(&self) -> Option<PlayerMediaGeneration> {
        self.pending_load_generation
    }

    fn pending_load_request(&self) -> Option<&str> {
        self.pending_load_request.as_deref()
    }

    fn lifecycle_epoch(&self) -> PlayerAttachmentEpoch {
        self.player_lifecycle.attachment_epoch
    }

    fn apply_lifecycle_input(&mut self, input: PlayerLifecycleInput) -> Vec<PlayerLifecycleEffect> {
        let current = std::mem::take(&mut self.player_lifecycle);
        let (next, effects) = reduce_player_lifecycle(current, input);
        self.player_lifecycle = next;
        if effects.iter().any(|effect| {
            matches!(
                effect,
                PlayerLifecycleEffect::RequestLifecycleReconciliation
            )
        }) {
            self.lifecycle_reconciliation_due = true;
        }
        effects
    }

    fn submit_lifecycle_load(
        &mut self,
        command_id: Option<PlayerCommandId>,
        generation: PlayerMediaGeneration,
        requested_target: &str,
        baseline_playlist_entry_ids: BTreeSet<i64>,
    ) -> sorotte_player_api::LoadAttemptId {
        self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id,
            media_generation: generation,
            requested_target: requested_target.to_owned(),
            baseline_playlist_entry_ids,
        })
        .into_iter()
        .find_map(|effect| match effect {
            PlayerLifecycleEffect::LoadAttemptAllocated { attempt_id, .. } => Some(attempt_id),
            _ => None,
        })
        .expect("a fresh media generation must allocate one physical load attempt")
    }

    fn tracked_load_command_id_for_generation(
        &self,
        generation: PlayerMediaGeneration,
    ) -> Option<PlayerCommandId> {
        self.pending_tracked_commands
            .iter()
            .find(|command| {
                command.media_generation == Some(generation)
                    && matches!(command.kind, TrackedCommandKind::Load { .. })
            })
            .map(|command| command.id)
    }

    #[cfg_attr(test, allow(dead_code))]
    fn authoritative_playlist_entries(playlist: &Value) -> Vec<AuthoritativePlaylistEntry> {
        let Some(entries) = playlist.as_array() else {
            return Vec::new();
        };
        let mut parsed = entries
            .iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_i64().or_else(|| {
                    entry
                        .get("id")
                        .and_then(Value::as_u64)
                        .and_then(|value| value.try_into().ok())
                })?;
                Some(AuthoritativePlaylistEntry::new(
                    id,
                    entry
                        .get("filename")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    entry.get("current").and_then(Value::as_bool) == Some(true)
                        || entry.get("playing").and_then(Value::as_bool) == Some(true),
                ))
            })
            .collect::<Vec<_>>();
        if parsed.len() == 1 {
            parsed[0].current = true;
        }
        parsed
    }

    fn capture_authoritative_playlist_baseline(&mut self) -> BTreeSet<i64> {
        // Most legacy unit-test transports script only the command under test.
        // Keep their fixtures deterministic while production always captures
        // the real pre-command playlist identity set.
        #[cfg(test)]
        {
            self.player_lifecycle
                .playlist_entry_attempts
                .keys()
                .copied()
                .collect()
        }

        #[cfg(not(test))]
        let authoritative = self
            .ipc_client
            .as_mut()
            .and_then(|client| client.get_property(MPV_PROPERTY_PLAYLIST).ok().flatten())
            .map(|playlist| Self::authoritative_playlist_entries(&playlist));
        #[cfg(not(test))]
        authoritative
            .map(|entries| entries.into_iter().map(|entry| entry.id).collect())
            .unwrap_or_else(|| {
                self.player_lifecycle
                    .playlist_entry_attempts
                    .keys()
                    .copied()
                    .collect()
            })
    }

    #[cfg(test)]
    fn bind_single_pending_test_load(&mut self, playlist_entry_id: i64) -> bool {
        let candidates = self
            .player_lifecycle
            .load_attempts
            .values()
            .filter(|attempt| !attempt.state.is_terminal() && attempt.playlist_entry_id.is_none())
            .map(|attempt| (attempt.id, attempt.requested_target.clone()))
            .collect::<Vec<_>>();
        let [(attempt_id, requested_target)] = candidates.as_slice() else {
            return false;
        };
        let attachment_epoch = self.lifecycle_epoch();
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch,
            entries: vec![AuthoritativePlaylistEntry::new(
                playlist_entry_id,
                Some(requested_target.clone()),
                true,
            )],
            current_path: Some(requested_target.clone()),
        });
        self.apply_lifecycle_input(PlayerLifecycleInput::StartFile {
            attachment_epoch,
            playlist_entry_id,
        });
        if self
            .player_lifecycle
            .attempt_for_playlist_entry(playlist_entry_id)
            != Some(*attempt_id)
        {
            return false;
        }
        self.active_playlist_entry_id = u64::try_from(playlist_entry_id).ok();
        self.active_media_generation = self
            .player_lifecycle
            .load_attempts
            .get(attempt_id)
            .map(|attempt| attempt.media_generation);
        true
    }

    #[cfg_attr(test, allow(dead_code))]
    fn reconcile_lifecycle_from_authority(&mut self) {
        let epoch = self.lifecycle_epoch();
        let Some(client) = self.ipc_client.as_mut() else {
            return;
        };
        if !client.is_healthy() {
            self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                attachment_epoch: epoch,
            });
            return;
        }
        let Some(playlist) = client.get_property(MPV_PROPERTY_PLAYLIST).ok().flatten() else {
            self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                attachment_epoch: epoch,
            });
            return;
        };
        let path = client
            .get_property(MPV_PROPERTY_PATH)
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(ToOwned::to_owned));
        let paused = client
            .get_property(MPV_PROPERTY_PAUSE)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let position_seconds = client
            .get_property(MPV_PROPERTY_TIME_POS)
            .ok()
            .flatten()
            .and_then(|value| value.as_f64());
        let playback_rate = client
            .get_property(MPV_PROPERTY_SPEED)
            .ok()
            .flatten()
            .and_then(|value| value.as_f64());
        let paused_for_cache = client
            .get_property(MPV_PROPERTY_PAUSED_FOR_CACHE)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let cache_buffering_percent = client
            .get_property(MPV_PROPERTY_CACHE_BUFFERING_STATE)
            .ok()
            .flatten()
            .and_then(|value| value.as_f64());
        let seeking = client
            .get_property(MPV_PROPERTY_SEEKING)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let seekable = client
            .get_property(MPV_PROPERTY_SEEKABLE)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let core_idle = client
            .get_property(MPV_PROPERTY_CORE_IDLE)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let demuxer_cache_idle = client
            .get_property(MPV_PROPERTY_DEMUXER_CACHE_IDLE)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let eof_reached = client
            .get_property(MPV_PROPERTY_EOF_REACHED)
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let entries = Self::authoritative_playlist_entries(&playlist);
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries: entries.clone(),
            current_path: path.clone(),
        });
        if let Some(current) = entries.iter().find(|entry| entry.current)
            && self
                .player_lifecycle
                .attempt_for_playlist_entry(current.id)
                .is_none()
        {
            let target = current
                .original_filename
                .clone()
                .or(path.clone())
                .unwrap_or_default();
            let could_be_pending = self.player_lifecycle.load_attempts.values().any(|attempt| {
                !attempt.state.is_terminal()
                    && attempt.playlist_entry_id.is_none()
                    && (attempt.requested_target == target
                        || current.original_filename.as_deref()
                            == Some(attempt.requested_target.as_str()))
            });
            if !could_be_pending && !target.is_empty() {
                let generation = self.allocate_media_generation();
                self.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
                    attachment_epoch: epoch,
                    media_generation: generation,
                    playlist_entry_id: current.id,
                    observed_target: target,
                    file_loaded: self.active_file_loaded,
                });
            }
        }
        // Bind identities before reducing buffered start-file/file-loaded
        // notifications so a fast load cannot lose its physical observations.
        self.drain_ipc_events_if_attached();
        self.observed_state.path = path.clone();
        self.observed_state.paused = paused;
        self.observed_state.logical_pause =
            paused.map(|paused| paused && paused_for_cache != Some(true));
        self.observed_state.position_seconds = position_seconds;
        self.observed_state.playback_rate = playback_rate;
        self.observed_state.paused_for_cache = paused_for_cache;
        self.observed_state.cache_buffering_percent = cache_buffering_percent;
        self.observed_state.seeking = seeking;
        self.observed_state.seekable = seekable;
        self.observed_state.core_idle = core_idle;
        self.observed_state.demuxer_cache_idle = demuxer_cache_idle;
        self.observed_state.eof_reached = eof_reached;
        self.current_path = path.clone();
        if entries.iter().all(|entry| !entry.current) {
            self.lifecycle_reconciliation_due = false;
            if self.player_lifecycle.requires_authoritative_snapshot() {
                self.publish_authoritative_lifecycle_snapshot();
            }
            return;
        }
        self.lifecycle_reconciliation_due = false;
        if self.player_lifecycle.requires_authoritative_snapshot() {
            self.publish_authoritative_lifecycle_snapshot();
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn lifecycle_snapshot_field<T>(value: Option<T>) -> SnapshotField<T> {
        value.map_or(SnapshotField::Unavailable, SnapshotField::Known)
    }

    #[cfg_attr(test, allow(dead_code))]
    fn publish_authoritative_lifecycle_snapshot(&mut self) {
        let attachment_epoch = self.lifecycle_epoch();
        let active = self.player_lifecycle.active_attempt();
        let active_load = active.map(|attempt| PlayerActiveLoadSnapshot {
            attempt_id: attempt.id,
            media_generation: attempt.media_generation,
            command_id: attempt.command_id,
            playlist_entry_id: attempt.playlist_entry_id,
        });
        let active_attempt_id = active.map(|attempt| attempt.id);
        let active_generation = active.map(|attempt| attempt.media_generation);
        let active_playlist_entry_id = active.and_then(|attempt| attempt.playlist_entry_id);
        let current_path = self
            .observed_state
            .path
            .clone()
            .or_else(|| self.current_path.clone());
        let snapshot = PlayerAuthoritativeSnapshot {
            attachment_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(
                attachment_epoch,
                self.player_lifecycle.last_event_sequence(),
            ),
            transport: PlayerTransportSnapshot {
                load_attempt_id: active_attempt_id
                    .map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
                media_generation: active_generation
                    .map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
                observed_at: SnapshotField::Known(self.observation_timestamp()),
                phase: SnapshotField::Known(self.transport_phase),
                position_seconds: Self::lifecycle_snapshot_field(
                    self.observed_state.position_seconds,
                ),
                playback_rate: Self::lifecycle_snapshot_field(self.observed_state.playback_rate),
                logical_pause: Self::lifecycle_snapshot_field(self.observed_state.logical_pause),
                paused_for_cache: Self::lifecycle_snapshot_field(
                    self.observed_state.paused_for_cache,
                ),
                cache_percentage: Self::lifecycle_snapshot_field(
                    self.observed_state.cache_buffering_percent,
                ),
                seeking: Self::lifecycle_snapshot_field(self.observed_state.seeking),
                seekable: Self::lifecycle_snapshot_field(self.observed_state.seekable),
                timeline_kind: SnapshotField::Known(self.timeline_kind),
                core_idle: Self::lifecycle_snapshot_field(self.observed_state.core_idle),
                demuxer_cache_idle: Self::lifecycle_snapshot_field(
                    self.observed_state.demuxer_cache_idle,
                ),
                playback_restart_sequence: SnapshotField::Known(self.playback_restart_sequence),
                eof_reached: Self::lifecycle_snapshot_field(self.observed_state.eof_reached),
                seekable_ranges: SnapshotField::Unavailable,
                known_live_seekable_window: self
                    .latest_cached_seekable_window
                    .map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
                buffered_duration_seconds: Self::lifecycle_snapshot_field(
                    self.observed_state.buffered_ahead_seconds,
                ),
                buffered_bytes: Self::lifecycle_snapshot_field(
                    self.observed_state.buffered_ahead_bytes,
                ),
                input_rate_bytes_per_second: Self::lifecycle_snapshot_field(
                    self.observed_state.input_rate_bytes_per_second,
                ),
                error_kind: SnapshotField::KnownAbsent,
            },
            active_load: active_load.map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
            current_playlist_entry_id: active_playlist_entry_id
                .map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
            current_path: current_path.map_or(SnapshotField::KnownAbsent, SnapshotField::Known),
        };
        self.apply_lifecycle_input(PlayerLifecycleInput::AuthoritativeSnapshotApplied(snapshot));
    }

    fn interrupted_network_stream_recovery_load_command(
        &self,
        path: &str,
        resume_position_seconds: f64,
    ) -> Value {
        let mut options = self.network_media_options_map();
        options.insert(
            "start".to_owned(),
            Value::String(resume_position_seconds.to_string()),
        );
        json!([
            MPV_COMMAND_LOADFILE,
            path,
            MPV_LOADFILE_REPLACE,
            -1,
            Value::Object(options)
        ])
    }

    fn observe_interrupted_network_stream_recovery_progress(&mut self, position_seconds: f64) {
        let generation = self.observation_media_generation();
        if let Some(recovery) =
            self.interrupted_network_stream_recovery
                .as_mut()
                .filter(|recovery| {
                    Some(recovery.media_generation) == generation
                        && position_seconds
                            >= recovery.resume_position_seconds
                                + INTERRUPTED_NETWORK_STREAM_RECOVERY_PROGRESS_SECONDS
                })
        {
            recovery.resume_position_seconds = position_seconds;
            recovery.consecutive_attempts = 0;
        }
    }

    fn try_recover_interrupted_network_stream(
        &mut self,
        generation: PlayerMediaGeneration,
    ) -> bool {
        self.try_recover_network_stream_with_minimum_remaining(
            generation,
            INTERRUPTED_NETWORK_STREAM_MINIMUM_REMAINING_SECONDS,
        )
    }

    fn try_recover_stalled_network_stream(&mut self, generation: PlayerMediaGeneration) -> bool {
        // A sustained, progress-free cache pause is independent evidence that
        // the request is dead, including near the media tail.
        self.try_recover_network_stream_with_minimum_remaining(
            generation,
            PLAYBACK_ADVANCEMENT_EPSILON_SECONDS,
        )
    }

    fn try_recover_network_stream_with_minimum_remaining(
        &mut self,
        generation: PlayerMediaGeneration,
        minimum_remaining_seconds: f64,
    ) -> bool {
        if self.timeline_kind != PlayerTimelineKind::Vod {
            return false;
        }
        let Some(active_attempt) = self.player_lifecycle.active_attempt().cloned() else {
            return false;
        };
        if active_attempt.media_generation != generation
            || active_attempt.state.is_terminal()
            || active_attempt.superseded_by.is_some()
        {
            return false;
        }
        let Some(path) = self
            .current_path
            .clone()
            .filter(|path| uses_network_media_options(path))
        else {
            return false;
        };
        let Some(duration_seconds) = self
            .observed_state
            .duration_seconds
            .filter(|duration| duration.is_finite() && *duration > 0.0)
        else {
            return false;
        };
        let Some(position_seconds) = self
            .observed_state
            .position_seconds
            .filter(|position| position.is_finite() && *position >= 0.0)
        else {
            return false;
        };
        if duration_seconds - position_seconds <= minimum_remaining_seconds {
            return false;
        }

        let (consecutive_attempts, total_attempts) = self
            .interrupted_network_stream_recovery
            .filter(|recovery| recovery.media_generation == generation)
            .map_or((1, 1), |recovery| {
                (
                    recovery.consecutive_attempts.saturating_add(1),
                    recovery.total_attempts.saturating_add(1),
                )
            });
        if consecutive_attempts > MAX_CONSECUTIVE_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
            || total_attempts > MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
        {
            return false;
        }

        let attachment_epoch = self.lifecycle_epoch();
        let baseline_playlist_entry_ids = self.capture_authoritative_playlist_baseline();
        let attempt_id =
            self.submit_lifecycle_load(None, generation, &path, baseline_playlist_entry_ids);
        self.interrupted_network_stream_recovery = Some(InterruptedNetworkStreamRecovery {
            media_generation: generation,
            latest_attempt_id: attempt_id,
            resume_position_seconds: position_seconds,
            consecutive_attempts,
            total_attempts,
        });
        let command =
            self.interrupted_network_stream_recovery_load_command(&path, position_seconds);
        if self
            .send_ipc_command_if_attached_without_draining_events(command)
            .is_err()
        {
            self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch,
                attempt_id,
                failure: PlayerCommandFailureKind::TransportDisconnected,
            });
            return false;
        }

        self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch,
            attempt_id,
        });
        self.network_media_options_embedded_load = (!self.network_media_options.is_empty())
            .then_some(EmbeddedNetworkMediaOptions {
                media_generation: generation,
                requested_target: path,
            });
        self.pending_transport_telemetry_updates.retain(|update| {
            update.media_generation != Some(generation)
                || (update.phase != Some(PlayerTransportPhase::Ended)
                    && update.phase != Some(PlayerTransportPhase::Failed)
                    && update.eof_reached != Some(true))
        });
        self.pending_cache_telemetry_updates
            .retain(|update| update.media_generation != Some(generation));
        self.transport_phase = PlayerTransportPhase::Loading;
        self.active_file_loaded = false;
        self.observed_state.eof_reached = Some(false);
        self.queue_cache_telemetry_update(self.cleared_cache_telemetry_update(Some(generation)));
        let mut update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading)
            .with_position_seconds(position_seconds);
        update.eof_reached = Some(false);
        self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        self.lifecycle_reconciliation_due = true;
        #[cfg(not(test))]
        self.reconcile_lifecycle_from_authority();
        self.drain_ipc_events_if_attached();
        true
    }

    fn observe_network_cache_pause_for_recovery(&mut self, paused_for_cache: bool) {
        if !paused_for_cache {
            self.network_cache_stall = None;
            return;
        }
        let Some(media_generation) = self.observation_media_generation() else {
            return;
        };
        let is_rebuffering_network_vod = self.active_file_loaded
            && self.active_generation_has_restarted
            && self.timeline_kind == PlayerTimelineKind::Vod
            && self.observed_state.seeking != Some(true)
            && self
                .current_path
                .as_deref()
                .is_some_and(uses_network_media_options);
        if !is_rebuffering_network_vod {
            return;
        }
        if self
            .network_cache_stall
            .is_none_or(|stall| stall.media_generation != media_generation)
        {
            self.network_cache_stall = Some(NetworkCacheStall {
                media_generation,
                last_progress_at: Instant::now(),
                last_sample: NetworkCacheProgressSample::from_observed_state(&self.observed_state),
            });
        }
    }

    fn network_cache_stall_recovery_delay(&self) -> Duration {
        let configured_wait = self
            .network_media_options
            .get("cache-pause-wait")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .and_then(|value| Duration::try_from_secs_f64(value).ok())
            .unwrap_or_default();
        NETWORK_CACHE_STALL_RECOVERY_DELAY.max(
            configured_wait
                .checked_add(NETWORK_CACHE_STALL_RECOVERY_MARGIN)
                .unwrap_or(Duration::MAX),
        )
    }

    fn maintain_network_cache_stall_recovery(&mut self) {
        let Some(stall) = self.network_cache_stall else {
            return;
        };
        let still_stalled = self.observation_media_generation() == Some(stall.media_generation)
            && self.active_file_loaded
            && self.active_generation_has_restarted
            && self.timeline_kind == PlayerTimelineKind::Vod
            && self.observed_state.paused_for_cache == Some(true)
            && self.observed_state.seeking != Some(true)
            && self.observed_state.eof_reached != Some(true)
            && self.observed_state.cache_eof != Some(true)
            && self
                .current_path
                .as_deref()
                .is_some_and(uses_network_media_options);
        if !still_stalled {
            self.network_cache_stall = None;
            return;
        }

        let sample = NetworkCacheProgressSample::from_observed_state(&self.observed_state);
        if let Some(active_stall) = self
            .network_cache_stall
            .as_mut()
            .filter(|active| active.media_generation == stall.media_generation)
        {
            if sample.made_progress_since(active_stall.last_sample) {
                active_stall.last_progress_at = Instant::now();
                active_stall.last_sample = sample;
                return;
            }
            active_stall.last_sample = sample;
        }
        if stall.last_progress_at.elapsed() < self.network_cache_stall_recovery_delay() {
            return;
        }

        if self.try_recover_stalled_network_stream(stall.media_generation) {
            self.network_cache_stall = None;
        } else if let Some(active_stall) = self
            .network_cache_stall
            .as_mut()
            .filter(|active| active.media_generation == stall.media_generation)
        {
            // A rejected recovery stays bounded and backs off for another
            // watchdog interval instead of spinning in every runtime pump.
            active_stall.last_progress_at = Instant::now();
        }
    }

    /// Returns the number of bounded same-generation network reload attempts
    /// made since the most recent logical media load.
    pub fn network_stream_recovery_attempt_count(&self) -> usize {
        self.interrupted_network_stream_recovery
            .map_or(0, |recovery| recovery.total_attempts)
    }

    fn allocate_command_id(&mut self) -> PlayerCommandId {
        let command_id = self.next_command_id.max(1);
        self.next_command_id = command_id.wrapping_add(1).max(1);
        PlayerCommandId::new(command_id)
    }

    fn register_tracked_command(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        kind: TrackedCommandKind,
    ) -> PlayerCommandId {
        let id = self.allocate_command_id();
        let lifecycle_dispatch_boundary = self.player_lifecycle.last_event_sequence();
        match &kind {
            TrackedCommandKind::Load { .. } => {}
            TrackedCommandKind::Seek { target_seconds, .. } => {
                if let Some(media_generation) = media_generation {
                    self.apply_lifecycle_input(PlayerLifecycleInput::SeekCommandSubmitted {
                        command_id: id,
                        media_generation,
                        raw_player_target_seconds: *target_seconds,
                        effective_room_target_seconds: *target_seconds,
                        dispatch_sequence_boundary: lifecycle_dispatch_boundary,
                    });
                } else {
                    self.apply_lifecycle_input(PlayerLifecycleInput::CommandSubmitted {
                        command_id: id,
                        media_generation,
                        kind: crate::lifecycle::LifecycleCommandKind::Seek,
                    });
                }
            }
            TrackedCommandKind::Pause { .. } => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandSubmitted {
                    command_id: id,
                    media_generation,
                    kind: crate::lifecycle::LifecycleCommandKind::Pause,
                });
            }
            TrackedCommandKind::Play { .. } => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandSubmitted {
                    command_id: id,
                    media_generation,
                    kind: crate::lifecycle::LifecycleCommandKind::Play,
                });
            }
        }
        self.pending_tracked_commands
            .push_back(PendingTrackedCommand {
                id,
                media_generation,
                accepted_at: None,
                deferred_result: None,
                kind,
            });
        id
    }

    fn accept_tracked_command(&mut self, command_id: PlayerCommandId) {
        let Some(command) = self
            .pending_tracked_commands
            .iter_mut()
            .find(|command| command.id == command_id)
        else {
            return;
        };
        if command.accepted_at.is_some() {
            return;
        }
        command.accepted_at = Some(Instant::now());
        let media_generation = command.media_generation;
        let deferred_result = command.deferred_result;
        let is_load = matches!(command.kind, TrackedCommandKind::Load { .. });
        let is_seek = matches!(command.kind, TrackedCommandKind::Seek { .. });
        let lifecycle_epoch = self.lifecycle_epoch();
        if is_seek && media_generation.is_some() {
            self.apply_lifecycle_input(PlayerLifecycleInput::SeekCommandAccepted {
                attachment_epoch: lifecycle_epoch,
                command_id,
            });
        } else if !is_load {
            self.apply_lifecycle_input(PlayerLifecycleInput::CommandAccepted {
                attachment_epoch: lifecycle_epoch,
                command_id,
            });
        }
        self.queue_command_progress(PlayerCommandProgress::accepted(
            command_id,
            media_generation,
            Some(self.observation_timestamp()),
        ));
        if let Some(result) = deferred_result {
            self.finish_tracked_command(command_id, result);
        } else {
            self.finish_completed_tracked_commands();
        }
    }

    fn discard_unaccepted_tracked_command(&mut self, command_id: PlayerCommandId) {
        let kind = self
            .pending_tracked_commands
            .iter()
            .find(|command| command.id == command_id)
            .and_then(|command| match &command.kind {
                TrackedCommandKind::Load { .. } => None,
                TrackedCommandKind::Seek { .. } => {
                    Some(crate::lifecycle::LifecycleCommandKind::Seek)
                }
                TrackedCommandKind::Pause { .. } => {
                    Some(crate::lifecycle::LifecycleCommandKind::Pause)
                }
                TrackedCommandKind::Play { .. } => {
                    Some(crate::lifecycle::LifecycleCommandKind::Play)
                }
            });
        let lifecycle_epoch = self.lifecycle_epoch();
        match kind {
            Some(crate::lifecycle::LifecycleCommandKind::Seek) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::SeekCommandRejected {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                    failure: PlayerCommandFailureKind::Unknown,
                });
            }
            Some(_) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandRejected {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                    failure: PlayerCommandFailureKind::Unknown,
                });
            }
            None => {}
        }
        self.pending_tracked_commands
            .retain(|command| command.id != command_id);
    }

    fn queue_command_progress(&mut self, progress: PlayerCommandProgress) {
        self.queue_ordered_player_event(PlayerOrderedEventKind::CommandProgress(progress));
        if progress.is_terminal() {
            self.unacknowledged_terminal_command_progress
                .insert(progress.command_id, progress);
        }
        if self.pending_command_progress_updates.len() >= MAX_PENDING_COMMAND_PROGRESS_UPDATES {
            self.pending_command_progress_updates.pop_front();
        }
        self.pending_command_progress_updates.push_back(progress);
    }

    fn finish_tracked_command(&mut self, command_id: PlayerCommandId, result: PlayerCommandResult) {
        let Some(index) = self
            .pending_tracked_commands
            .iter()
            .position(|command| command.id == command_id)
        else {
            return;
        };
        let command = self
            .pending_tracked_commands
            .remove(index)
            .expect("tracked command index should remain valid");
        let lifecycle_epoch = self.lifecycle_epoch();
        match result {
            PlayerCommandResult::Completed => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandCompleted {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                });
            }
            PlayerCommandResult::Superseded => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandSuperseded {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                });
            }
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut)
                if matches!(command.kind, TrackedCommandKind::Seek { .. }) =>
            {
                self.apply_lifecycle_input(
                    PlayerLifecycleInput::SeekCommandCompletionNotObserved {
                        attachment_epoch: lifecycle_epoch,
                        command_id,
                    },
                );
            }
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandCompletionNotObserved {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                });
            }
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TransportDisconnected) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandTransportDisconnected {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                });
            }
            PlayerCommandResult::Failed(failure) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::CommandRejected {
                    attachment_epoch: lifecycle_epoch,
                    command_id,
                    failure,
                });
            }
        }
        let observed_position_seconds = (command.media_generation
            == self.observation_media_generation())
        .then_some(self.observed_state.position_seconds)
        .flatten();
        self.queue_command_progress(PlayerCommandProgress::finished(
            command.id,
            command.media_generation,
            Some(self.observation_timestamp()),
            observed_position_seconds,
            result,
        ));
    }

    fn finish_completed_tracked_commands(&mut self) {
        let completed: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some() && command.kind.completed())
            .map(|command| command.id)
            .collect();
        for command_id in completed {
            self.finish_tracked_command(command_id, PlayerCommandResult::Completed);
        }
    }

    fn observe_tracked_commands(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observation: TrackedCommandObservation,
    ) {
        let ready_paused_observed = self.observed_state.logical_pause == Some(true)
            && self.observed_state.paused_for_cache == Some(false);
        let logical_pause_observed_independently =
            self.observed_state.paused_for_cache == Some(false);
        let pause_property_current = self.observed_state.paused == Some(true);
        for command in &mut self.pending_tracked_commands {
            if command.media_generation != media_generation {
                continue;
            }
            match (&mut command.kind, observation) {
                (
                    TrackedCommandKind::Load { file_loaded, .. },
                    TrackedCommandObservation::FileLoaded,
                ) => {
                    *file_loaded = true;
                }
                (
                    TrackedCommandKind::Load { ready, .. },
                    TrackedCommandObservation::Phase(phase),
                ) => {
                    // Readiness is generation-scoped evidence; a later internal seek must not
                    // erase it while the loadfile acknowledgement is still buffered.
                    *ready |= phase == PlayerTransportPhase::Playing
                        || (phase == PlayerTransportPhase::ReadyPaused && ready_paused_observed);
                }
                (
                    TrackedCommandKind::Seek {
                        seeking_finished, ..
                    },
                    TrackedCommandObservation::Seeking(seeking),
                ) => {
                    *seeking_finished = !seeking;
                }
                (
                    TrackedCommandKind::Seek {
                        target_seconds,
                        position_in_tolerance,
                        ..
                    },
                    TrackedCommandObservation::Position(position_seconds),
                ) => {
                    *position_in_tolerance = (position_seconds - *target_seconds).abs()
                        <= crate::MPV_SEEK_COMPLETION_TOLERANCE_SECONDS;
                }
                (
                    TrackedCommandKind::Pause {
                        logical_pause_observed,
                    },
                    TrackedCommandObservation::LogicalPause(logical_pause),
                ) => {
                    *logical_pause_observed = logical_pause && logical_pause_observed_independently;
                }
                (
                    TrackedCommandKind::Pause {
                        logical_pause_observed,
                    },
                    TrackedCommandObservation::CachePause(paused_for_cache),
                ) => {
                    *logical_pause_observed = !paused_for_cache && pause_property_current;
                }
                (
                    TrackedCommandKind::Play {
                        logical_play_observed,
                        ..
                    },
                    TrackedCommandObservation::LogicalPause(logical_pause),
                ) => {
                    *logical_play_observed = !logical_pause;
                }
                (
                    TrackedCommandKind::Play {
                        cache_clear_observed,
                        ..
                    },
                    TrackedCommandObservation::CachePause(paused_for_cache),
                ) => {
                    *cache_clear_observed = !paused_for_cache;
                }
                (
                    TrackedCommandKind::Play {
                        restart_sequence_baseline,
                        restart_observed,
                        ..
                    },
                    TrackedCommandObservation::PlaybackRestart(sequence),
                ) => {
                    *restart_observed = sequence > *restart_sequence_baseline;
                }
                (
                    TrackedCommandKind::Play {
                        intent,
                        position_baseline,
                        restart_observed,
                        forward_advancement_observed,
                        ..
                    },
                    TrackedCommandObservation::Position(position_seconds),
                ) => match position_baseline {
                    Some(baseline) => {
                        if (matches!(intent, PlayerPlayIntent::Resume) || *restart_observed)
                            && position_seconds > *baseline + PLAYBACK_ADVANCEMENT_EPSILON_SECONDS
                        {
                            *forward_advancement_observed = true;
                        }
                    }
                    None => *position_baseline = Some(position_seconds),
                },
                _ => {}
            }
        }
        self.finish_completed_tracked_commands();
    }

    fn supersede_tracked_commands(
        &mut self,
        except: Option<PlayerCommandId>,
        predicate: impl Fn(&TrackedCommandKind) -> bool,
    ) {
        let superseded: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| Some(command.id) != except && predicate(&command.kind))
            .map(|command| command.id)
            .collect();
        for command_id in superseded {
            self.finish_tracked_command(command_id, PlayerCommandResult::Superseded);
        }
    }

    fn fail_tracked_commands_for_generation(
        &mut self,
        media_generation: PlayerMediaGeneration,
        failure: PlayerCommandFailureKind,
    ) {
        let result = PlayerCommandResult::Failed(failure);
        let mut failed = Vec::new();
        for command in &mut self.pending_tracked_commands {
            if command.media_generation != Some(media_generation) {
                continue;
            }
            if command.accepted_at.is_some() {
                failed.push(command.id);
            } else {
                command.deferred_result = Some(result);
            }
        }
        for command_id in failed {
            self.finish_tracked_command(command_id, result);
        }
    }

    fn fail_all_accepted_tracked_commands(&mut self, failure: PlayerCommandFailureKind) {
        let failed: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some())
            .map(|command| command.id)
            .collect();
        for command_id in failed {
            self.finish_tracked_command(command_id, PlayerCommandResult::Failed(failure));
        }
    }

    fn expire_tracked_commands(&mut self) {
        let now = Instant::now();
        let timed_out: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| {
                command.accepted_at.is_some_and(|accepted_at| {
                    now.saturating_duration_since(accepted_at) >= command.kind.timeout()
                })
            })
            .map(|command| command.id)
            .collect();
        for command_id in timed_out {
            self.finish_tracked_command(
                command_id,
                PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
            );
        }
    }

    fn observation_timestamp(&self) -> PlayerObservationTimestamp {
        self.current_ipc_event_observed_at.unwrap_or_else(|| {
            PlayerObservationTimestamp::from_adapter_start(self.observation_clock_origin.elapsed())
        })
    }

    fn observation_timestamp_for(&self, observed_at: Instant) -> PlayerObservationTimestamp {
        PlayerObservationTimestamp::from_adapter_observation(
            observed_at.saturating_duration_since(self.observation_clock_origin),
            self.observation_clock_origin.elapsed(),
        )
    }

    fn transport_update(&self) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate {
            media_generation: self.observation_media_generation(),
            observed_at: Some(self.observation_timestamp()),
            ..PlayerTransportTelemetryUpdate::default()
        };
        update.timeline_kind = Some(self.timeline_kind);
        if self.timeline_kind == PlayerTimelineKind::SlidingLive {
            update.known_live_seekable_window = self.latest_cached_seekable_window;
        }
        update
    }

    fn observation_media_generation(&self) -> Option<PlayerMediaGeneration> {
        self.player_lifecycle.active_media_generation()
    }

    fn transport_update_for(
        &self,
        generation: PlayerMediaGeneration,
    ) -> PlayerTransportTelemetryUpdate {
        let mut update =
            PlayerTransportTelemetryUpdate::new(generation, self.observation_timestamp());
        if self.observation_media_generation() == Some(generation) {
            update.timeline_kind = Some(self.timeline_kind);
            if self.timeline_kind == PlayerTimelineKind::SlidingLive {
                update.known_live_seekable_window = self.latest_cached_seekable_window;
            }
        }
        update
    }

    fn queue_transport_telemetry_update(&mut self, update: PlayerTransportTelemetryUpdate) {
        self.queue_transport_telemetry_update_for_attempt(update, None);
    }

    fn queue_transport_telemetry_update_for_attempt(
        &mut self,
        mut update: PlayerTransportTelemetryUpdate,
        owning_attempt_id: Option<LoadAttemptId>,
    ) {
        if update.media_generation.is_none() {
            update.media_generation = self.observation_media_generation();
        }
        if update.observed_at.is_none() {
            update.observed_at = Some(self.observation_timestamp());
        }
        let mut ordered_delta = PlayerTransportDelta::from(update.clone());
        ordered_delta.load_attempt_id = owning_attempt_id.or_else(|| {
            update.media_generation.and_then(|generation| {
                self.player_lifecycle
                    .active_attempt()
                    .filter(|attempt| attempt.media_generation == generation)
                    .map(|attempt| attempt.id)
            })
        });
        let lifecycle_epoch = self.lifecycle_epoch();
        let observed_phase = ordered_delta.phase;
        self.apply_lifecycle_input(PlayerLifecycleInput::TransportDelta {
            attachment_epoch: lifecycle_epoch,
            delta: ordered_delta,
        });
        if let Some(phase) = observed_phase {
            self.apply_lifecycle_input(PlayerLifecycleInput::PhaseObserved {
                attachment_epoch: lifecycle_epoch,
                phase,
            });
        }
        if update.paused_for_cache == Some(true) {
            for pending in self.pending_transport_telemetry_updates.iter_mut().rev() {
                if pending.media_generation != update.media_generation {
                    break;
                }
                if pending.logical_pause == Some(true) {
                    pending.logical_pause = None;
                }
                if pending.phase == Some(PlayerTransportPhase::ReadyPaused)
                    && update.phase.is_some()
                {
                    pending.phase = update.phase;
                }
                if pending.playback_restart_sequence.is_some() {
                    break;
                }
            }
            for event in self.pending_ordered_player_events.iter_mut().rev() {
                let PlayerOrderedEventKind::Transport(pending) = &mut event.kind else {
                    continue;
                };
                if pending.media_generation != update.media_generation {
                    break;
                }
                if pending.logical_pause == Some(true) {
                    pending.logical_pause = None;
                }
                if pending.phase == Some(PlayerTransportPhase::ReadyPaused)
                    && update.phase.is_some()
                {
                    pending.phase = update.phase;
                }
                if pending.playback_restart_sequence.is_some() {
                    break;
                }
            }
        }
        self.queue_ordered_player_event(PlayerOrderedEventKind::Transport(update.clone()));

        let update_has_cache_metrics = update.cache_buffering_percent.is_some()
            || update.buffered_ahead_seconds.is_some()
            || update.buffered_ahead_bytes.is_some()
            || update.input_rate_bytes_per_second.is_some();
        let cache_position_boundary =
            self.pending_transport_telemetry_updates
                .back()
                .is_some_and(|pending| {
                    let pending_has_cache_metrics = pending.cache_buffering_percent.is_some()
                        || pending.buffered_ahead_seconds.is_some()
                        || pending.buffered_ahead_bytes.is_some()
                        || pending.input_rate_bytes_per_second.is_some();
                    (update.position_seconds.is_some()
                        && !update_has_cache_metrics
                        && pending_has_cache_metrics)
                        || (update_has_cache_metrics
                            && update.position_seconds.is_none()
                            && pending.position_seconds.is_some())
                });
        let projection_clock_boundary = self
            .pending_transport_telemetry_updates
            .back()
            .is_some_and(|pending| {
                update.playback_rate.is_some()
                    || pending.playback_rate.is_some()
                    || (pending.position_seconds.is_some() && update.position_seconds.is_none())
            });
        let lifecycle_boundary = cache_position_boundary
            || projection_clock_boundary
            || self
                .pending_transport_telemetry_updates
                .back()
                .is_none_or(|pending| {
                    pending.media_generation != update.media_generation
                        || update.playback_restart_sequence.is_some()
                        || update.error_kind.is_some()
                        || update.eof_reached == Some(true)
                        || update
                            .phase
                            .is_some_and(|phase| pending.phase != Some(phase))
                });
        if !lifecycle_boundary
            && let Some(pending) = self.pending_transport_telemetry_updates.back_mut()
        {
            pending.merge_from(update);
            return;
        }

        if self.pending_transport_telemetry_updates.len() >= MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES
        {
            self.pending_transport_telemetry_updates.pop_front();
        }
        self.pending_transport_telemetry_updates.push_back(update);
    }

    fn queue_cache_telemetry_update(&mut self, mut update: PlayerCacheTelemetryUpdate) {
        if update.media_generation.is_none() {
            update.media_generation = self.observation_media_generation();
        }
        if update.observed_at.is_none() {
            update.observed_at = Some(self.observation_timestamp());
        }
        if self.pending_cache_telemetry_updates.len() >= MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES {
            self.pending_cache_telemetry_updates.pop_front();
        }
        self.pending_cache_telemetry_updates.push_back(update);
    }

    fn cleared_cache_telemetry_update(
        &self,
        generation: Option<PlayerMediaGeneration>,
    ) -> PlayerCacheTelemetryUpdate {
        PlayerCacheTelemetryUpdate {
            media_generation: generation,
            observed_at: Some(self.observation_timestamp()),
            ..PlayerCacheTelemetryUpdate::default()
        }
    }

    fn begin_seek_cache_evidence_epoch(&mut self) {
        let generation = self.observation_media_generation();
        self.cache_buffering_percent = None;
        self.observed_state.cache_buffering_percent = None;
        self.observed_state.buffered_ahead_seconds = None;
        self.observed_state.buffered_ahead_bytes = None;
        self.observed_state.input_rate_bytes_per_second = None;
        self.observed_state.cache_reader_position_seconds = None;
        self.observed_state.cache_end_seconds = None;
        self.observed_state.cache_eof = None;
        self.observed_state.cache_underrun = None;
        self.observed_state.cache_metrics_observed_at = None;
        self.queue_cache_telemetry_update(self.cleared_cache_telemetry_update(generation));
    }

    fn set_transport_phase(&mut self, phase: PlayerTransportPhase) {
        self.transport_phase = phase;
        let mut update = self.transport_update();
        update.phase = Some(phase);
        self.queue_transport_telemetry_update(update);
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Phase(phase),
        );
    }

    fn inferred_transport_phase(&self) -> PlayerTransportPhase {
        if self.active_media_generation.is_none() && self.pending_load_generation().is_none() {
            return PlayerTransportPhase::Empty;
        }
        if self.observed_state.seeking == Some(true) {
            return PlayerTransportPhase::Seeking;
        }
        if self.observed_state.paused_for_cache == Some(true) {
            return if self.active_generation_has_restarted {
                PlayerTransportPhase::Rebuffering
            } else {
                PlayerTransportPhase::Prebuffering
            };
        }
        if !self.active_file_loaded {
            return if self.active_media_generation.is_some() {
                PlayerTransportPhase::Loading
            } else {
                PlayerTransportPhase::Empty
            };
        }
        if self.observed_state.logical_pause == Some(true) {
            return PlayerTransportPhase::ReadyPaused;
        }
        if self.observed_state.core_idle == Some(true) {
            return if self.active_generation_has_restarted {
                PlayerTransportPhase::Rebuffering
            } else {
                PlayerTransportPhase::Prebuffering
            };
        }
        if self.observed_state.core_idle == Some(false) || self.active_generation_has_restarted {
            return PlayerTransportPhase::Playing;
        }
        PlayerTransportPhase::Prebuffering
    }

    fn refresh_inferred_transport_phase(&mut self) {
        let phase = self.inferred_transport_phase();
        if phase != self.transport_phase {
            self.set_transport_phase(phase);
        }
    }

    fn cache_state_telemetry_update(&mut self, data: &Value) -> PlayerTransportTelemetryUpdate {
        let mut update = self.transport_update();
        let Some(cache_state) = data.as_object() else {
            self.observed_state.seekable_ranges = None;
            self.latest_cached_seekable_window = None;
            self.observed_state.buffered_ahead_seconds = None;
            self.observed_state.buffered_ahead_bytes = None;
            self.observed_state.input_rate_bytes_per_second = None;
            self.observed_state.cache_reader_position_seconds = None;
            self.observed_state.cache_end_seconds = None;
            self.observed_state.cache_eof = None;
            self.observed_state.cache_underrun = None;
            self.observed_state.cache_metrics_observed_at = update.observed_at;
            self.queue_cache_telemetry_update(PlayerCacheTelemetryUpdate {
                media_generation: update.media_generation,
                observed_at: update.observed_at,
                ..PlayerCacheTelemetryUpdate::default()
            });
            return update;
        };

        update.seekable_ranges = cache_state
            .get("seekable-ranges")
            .and_then(Value::as_array)
            .map(|ranges| {
                ranges
                    .iter()
                    .filter_map(|range| {
                        let start_seconds = range.get("start")?.as_f64()?;
                        let end_seconds = range.get("end")?.as_f64()?;
                        (start_seconds.is_finite()
                            && end_seconds.is_finite()
                            && start_seconds <= end_seconds)
                            .then_some(PlayerSeekableRange::new(start_seconds, end_seconds))
                    })
                    .collect()
            });
        self.observed_state.seekable_ranges = update.seekable_ranges.clone();
        if let Some(ranges) = update.seekable_ranges.as_deref() {
            self.latest_cached_seekable_window = ranges
                .iter()
                .copied()
                .filter(|range| {
                    range.start_seconds.is_finite()
                        && range.end_seconds.is_finite()
                        && range.end_seconds > range.start_seconds
                })
                .max_by(|left, right| left.end_seconds.total_cmp(&right.end_seconds));
        } else {
            self.latest_cached_seekable_window = None;
        }
        update.timeline_kind = Some(self.timeline_kind);
        if self.timeline_kind == PlayerTimelineKind::SlidingLive {
            update.known_live_seekable_window = self.latest_cached_seekable_window;
        }
        update.buffered_ahead_seconds = cache_state
            .get("cache-duration")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        update.buffered_ahead_bytes = cache_state
            .get("fw-bytes")
            .and_then(Self::nonnegative_u64_from_json);
        update.input_rate_bytes_per_second = cache_state
            .get("raw-input-rate")
            .and_then(Self::nonnegative_u64_from_json);
        let reader_position_seconds = cache_state
            .get("reader-pts")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite());
        let cache_end_seconds = cache_state
            .get("cache-end")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite());
        let cache_eof = cache_state
            .get("eof")
            .or_else(|| cache_state.get("eof-cached"))
            .and_then(Value::as_bool);
        let cache_underrun = cache_state.get("underrun").and_then(Value::as_bool);
        self.queue_cache_telemetry_update(PlayerCacheTelemetryUpdate {
            media_generation: update.media_generation,
            observed_at: update.observed_at,
            buffered_ahead_seconds: update.buffered_ahead_seconds,
            buffered_ahead_bytes: update.buffered_ahead_bytes,
            input_rate_bytes_per_second: update.input_rate_bytes_per_second,
            reader_position_seconds,
            cache_end_seconds,
            eof: cache_eof,
            underrun: cache_underrun,
        });

        self.observed_state.buffered_ahead_seconds = update.buffered_ahead_seconds;
        self.observed_state.buffered_ahead_bytes = update.buffered_ahead_bytes;
        self.observed_state.input_rate_bytes_per_second = update.input_rate_bytes_per_second;
        self.observed_state.cache_reader_position_seconds = reader_position_seconds;
        self.observed_state.cache_end_seconds = cache_end_seconds;
        self.observed_state.cache_eof = cache_eof;
        self.observed_state.cache_underrun = cache_underrun;
        self.observed_state.cache_metrics_observed_at = update.observed_at;
        update
    }

    fn refresh_timeline_kind_from_metadata(&mut self) -> bool {
        let previous = self.timeline_kind;
        let Some(generation) = self.observation_media_generation() else {
            return false;
        };
        if !self.active_file_loaded || self.path_metadata_generation != Some(generation) {
            return false;
        }
        let Some(path) = self.observed_state.path.as_deref() else {
            return false;
        };
        self.timeline_kind = if !uses_network_media_options(path) {
            PlayerTimelineKind::Vod
        } else if self.ytdl_is_live && self.ytdl_is_live_metadata_generation == Some(generation) {
            // Supported mpv releases publish yt-dlp's per-file live flag as
            // metadata. Only positive evidence bound to this load is
            // sufficient for a sliding timeline.
            PlayerTimelineKind::SlidingLive
        } else if self.duration_metadata_generation != Some(generation) {
            return false;
        } else if self.observed_state.duration_seconds.is_some() {
            PlayerTimelineKind::Vod
        } else {
            // mpv's cache ranges are local cache state, not a source/DVR
            // window. A durationless network source remains explicitly
            // Unknown unless an upstream integration supplies positive live
            // timeline evidence; guessing here can turn valid VOD seeks into
            // destructive live-edge clamps.
            PlayerTimelineKind::Unknown
        };
        previous != self.timeline_kind
    }

    fn reset_timeline_metadata(&mut self) {
        self.timeline_kind = PlayerTimelineKind::Unknown;
        self.ytdl_is_live = false;
        self.ytdl_is_live_metadata_generation = None;
        self.latest_cached_seekable_window = None;
        self.observed_state.seekable_ranges = None;
        self.path_metadata_generation = None;
        self.duration_metadata_generation = None;
    }

    fn nonnegative_u64_from_json(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_i64()?.try_into().ok())
    }

    fn metadata_boolean_is_true(value: &Value) -> bool {
        value.as_bool() == Some(true)
            || value
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    fn full_metadata_ytdl_is_live(value: Option<&Value>) -> bool {
        value
            .and_then(Value::as_object)
            .and_then(|metadata| {
                metadata.iter().find_map(|(key, value)| {
                    key.eq_ignore_ascii_case("ytdl_is_live").then_some(value)
                })
            })
            .is_some_and(Self::metadata_boolean_is_true)
    }

    fn observe_ytdl_is_live_for_current_generation(&mut self, is_live: bool) {
        let Some(generation) = self.observation_media_generation() else {
            return;
        };
        if self.ytdl_is_live_metadata_generation != Some(generation) {
            self.ytdl_is_live = false;
            self.ytdl_is_live_metadata_generation = Some(generation);
        }
        // Once observed, positive per-file live evidence remains
        // authoritative for this generation. mpv can briefly report either
        // the full metadata map or its subproperty as unavailable during
        // demuxer changes; that must not turn a sliding timeline back into
        // VOD.
        self.ytdl_is_live |= is_live;
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
    }

    fn chat_input_polling_enabled(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_options_applied
            && self.legacy_syncplay_ui_settings.chat_input_enabled
    }

    fn poll_ipc_events_for_chat_input_if_enabled(&mut self) {
        if !self.chat_input_polling_enabled() {
            return;
        }

        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };

        let _ = ipc_client.get_property(MPV_PROPERTY_PAUSE);
        self.drain_ipc_events_if_attached();
    }

    fn drain_ipc_events_if_attached(&mut self) {
        // A prior nonblocking command can have harvested events followed by its completion.
        // Reduce that ordered stream first so the ordinary full pump cannot leapfrog the
        // completion with a newer event. Path observations remain deferred until the full flush
        // below, where potentially blocking hook recovery is allowed.
        self.drain_runtime_lease_events_nonblocking();
        self.reduce_pending_ipc_events(false);
        // `take_pending_events` above can be the poll that observes a just-finished nonblocking
        // command. Its events have now reduced in order; consume the following completion before
        // flushing deferred path work.
        self.drain_runtime_lease_events_nonblocking();
        self.flush_deferred_network_media_options_observation();
    }

    fn drain_ipc_events_without_network_options_flush(&mut self) -> bool {
        self.reduce_pending_ipc_events(false)
    }

    fn reduce_pending_ipc_events(&mut self, flush_network_options: bool) -> bool {
        let outermost_batch = self.network_media_options_event_batch_depth == 0;
        self.network_media_options_event_batch_depth += 1;
        let mut processed_any = false;
        loop {
            let pending_events = match self.ipc_client.as_mut() {
                Some(ipc_client) => ipc_client.take_pending_timed_events(),
                None => Vec::new(),
            };
            if pending_events.is_empty() {
                break;
            }
            processed_any = true;
            for event in pending_events {
                let previous_observed_at = self
                    .current_ipc_event_observed_at
                    .replace(self.observation_timestamp_for(event.received_at));
                self.handle_ipc_event(&event.value);
                self.current_ipc_event_observed_at = previous_observed_at;
            }
        }
        self.network_media_options_event_batch_depth -= 1;

        if outermost_batch && flush_network_options {
            self.flush_deferred_network_media_options_observation();
        }
        processed_any
    }

    fn flush_deferred_network_media_options_observation(&mut self) {
        let observation = self.deferred_network_media_options_observation.take();
        let observed_path = observation
            .as_ref()
            .map(|observation| observation.path.clone());
        if let Some(observation) = observation {
            self.apply_authoritative_path_for_network_options(
                observation.path.as_deref(),
                observation.origin,
            );
        }
        if let Some(result) = self
            .deferred_network_media_options_hook_transition_result
            .take()
        {
            self.apply_network_options_hook_transition_result(result, observed_path);
        }
    }

    fn observe_unhealthy_ipc_transport(&mut self) {
        let disconnected = self
            .ipc_client
            .as_ref()
            .is_some_and(|ipc_client| !ipc_client.is_healthy());
        if !disconnected
            || matches!(
                self.transport_phase,
                PlayerTransportPhase::Empty
                    | PlayerTransportPhase::Ended
                    | PlayerTransportPhase::Failed
            )
        {
            return;
        }
        let active_owner = self
            .player_lifecycle
            .active_attempt()
            .map(|attempt| (attempt.id, attempt.media_generation));
        let attachment_epoch = self.lifecycle_epoch();
        self.apply_lifecycle_input(PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch,
        });
        self.transport_phase = PlayerTransportPhase::Failed;
        self.active_file_loaded = false;
        if let Some((attempt_id, generation)) = active_owner {
            let update = self
                .transport_update_for(generation)
                .with_phase(PlayerTransportPhase::Failed);
            self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        }
    }

    fn handle_ipc_event(&mut self, event: &Value) {
        // This is the first adapter-owned boundary after mpv's JSON stream has
        // been decoded. Capture before classification so ignored or malformed
        // lifecycle input remains available to deterministic replay.
        let command_id = event
            .get("request_id")
            .and_then(Value::as_u64)
            .map(PlayerCommandId::new);
        let playlist_entry_id = event.get("playlist_entry_id").and_then(Value::as_i64);
        let _ =
            self.record_lifecycle_transcript_input(command_id, playlist_entry_id, event.clone());

        let Some(event_name) = event.get("event").and_then(Value::as_str) else {
            return;
        };

        match event_name {
            MPV_EVENT_START_FILE => {
                self.handle_start_file_event(event);
                return;
            }
            MPV_EVENT_FILE_LOADED => {
                self.handle_file_loaded_event();
                return;
            }
            MPV_EVENT_SEEK => {
                self.handle_seek_event();
                return;
            }
            MPV_EVENT_PLAYBACK_RESTART => {
                self.handle_playback_restart_event();
                return;
            }
            MPV_EVENT_END_FILE => {
                self.handle_end_file_event(event);
                return;
            }
            MPV_EVENT_PROPERTY_CHANGE => {}
            MPV_EVENT_CLIENT_MESSAGE => {
                self.handle_client_message_event(event);
                return;
            }
            _ => return,
        }

        let Some(property_name) = event.get("name").and_then(Value::as_str) else {
            return;
        };
        let data = event.get("data");

        let mut authoritative_path_observed = false;
        let mut authoritative_path = None;
        let file_metadata_changed = match property_name {
            MPV_PROPERTY_PATH => {
                authoritative_path_observed = true;
                let next_path = data.and_then(Value::as_str).map(ToOwned::to_owned);
                #[cfg(test)]
                if let Some(next_path) = next_path.as_deref() {
                    let matching_unbound = self
                        .player_lifecycle
                        .load_attempts
                        .values()
                        .filter(|attempt| {
                            !attempt.state.is_terminal()
                                && attempt.playlist_entry_id.is_none()
                                && Self::media_target_matches(next_path, &attempt.requested_target)
                        })
                        .count();
                    if matching_unbound == 1 {
                        let test_entry_id = self
                            .active_playlist_entry_id
                            .and_then(|entry_id| i64::try_from(entry_id).ok())
                            .or_else(|| {
                                self.player_lifecycle
                                    .load_attempts
                                    .values()
                                    .find(|attempt| {
                                        !attempt.state.is_terminal()
                                            && attempt.playlist_entry_id.is_none()
                                            && Self::media_target_matches(
                                                next_path,
                                                &attempt.requested_target,
                                            )
                                    })
                                    .and_then(|attempt| i64::try_from(attempt.id.get()).ok())
                            });
                        if let Some(test_entry_id) = test_entry_id
                            && self.bind_single_pending_test_load(test_entry_id)
                        {
                            self.active_file_loaded = false;
                        }
                    }
                }
                #[cfg(test)]
                if next_path.is_some() && self.active_media_generation.is_none() {
                    // Legacy scripted transports omit the authoritative playlist
                    // query used in production. This compatibility projection is
                    // test-only; production ownership is established by the
                    // reducer from playlist-entry evidence before metadata is
                    // correlated.
                    let generation = self
                        .pending_load_generation()
                        .unwrap_or_else(|| self.allocate_media_generation());
                    self.active_media_generation = Some(generation);
                    self.active_file_loaded = true;
                    self.active_generation_has_restarted = false;
                    self.transport_phase = PlayerTransportPhase::Prebuffering;
                    let update = self
                        .transport_update_for(generation)
                        .with_phase(PlayerTransportPhase::Prebuffering);
                    self.queue_transport_telemetry_update(update);
                }
                self.current_path = next_path.clone();
                self.observed_state.path = next_path.clone();
                authoritative_path = next_path.clone();
                self.path_metadata_generation = self.observation_media_generation();
                true
            }
            MPV_PROPERTY_DURATION => {
                self.observed_state.duration_seconds = data.and_then(Value::as_f64);
                self.duration_metadata_generation = self.observation_media_generation();
                true
            }
            MPV_PROPERTY_FILE_SIZE => {
                self.observed_state.size_bytes = data
                    .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()));
                true
            }
            MPV_PROPERTY_PAUSE => {
                if let Some(paused) = data.and_then(Value::as_bool) {
                    self.paused = paused;
                    self.observed_state.paused = Some(paused);
                    if !paused {
                        self.logical_pause_explicit = false;
                    } else if matches!(
                        self.transport_phase,
                        PlayerTransportPhase::Empty
                            | PlayerTransportPhase::Loading
                            | PlayerTransportPhase::ReadyPaused
                    ) || self
                        .pending_tracked_commands
                        .iter()
                        .any(|command| matches!(command.kind, TrackedCommandKind::Pause { .. }))
                    {
                        self.logical_pause_explicit = true;
                    }
                    let logical_pause = (!paused
                        || self.observed_state.paused_for_cache != Some(true))
                    .then_some(paused);
                    self.observed_state.logical_pause = logical_pause;
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_paused(paused),
                    );
                    if let Some(logical_pause) = logical_pause {
                        let update = self.transport_update().with_logical_pause(logical_pause);
                        self.queue_transport_telemetry_update(update);
                        self.observe_tracked_commands(
                            self.observation_media_generation(),
                            TrackedCommandObservation::LogicalPause(logical_pause),
                        );
                    }
                    self.refresh_inferred_transport_phase();
                } else {
                    self.observed_state.paused = None;
                    self.observed_state.logical_pause = None;
                }
                false
            }
            MPV_PROPERTY_TIME_POS => {
                if let Some(position_seconds) = data.and_then(Value::as_f64) {
                    self.position_seconds = position_seconds;
                    self.observed_state.position_seconds = Some(position_seconds);
                    self.observe_interrupted_network_stream_recovery_progress(position_seconds);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_position_seconds(position_seconds),
                    );
                    let update = self
                        .transport_update()
                        .with_position_seconds(position_seconds);
                    self.queue_transport_telemetry_update(update);
                    if let Some(media_generation) = self.player_lifecycle.active_media_generation()
                    {
                        let lifecycle_epoch = self.lifecycle_epoch();
                        let observed_sequence = self.player_lifecycle.last_event_sequence();
                        self.apply_lifecycle_input(PlayerLifecycleInput::PositionObserved {
                            attachment_epoch: lifecycle_epoch,
                            media_generation,
                            observed_sequence,
                            position_seconds,
                        });
                    }
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Position(position_seconds),
                    );
                } else {
                    self.observed_state.position_seconds = None;
                }
                false
            }
            MPV_PROPERTY_SPEED => {
                if let Some(speed) = data.and_then(Value::as_f64) {
                    self.playback_rate = speed;
                    self.observed_state.playback_rate = Some(speed);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_playback_rate(speed),
                    );
                    let mut update = self.transport_update();
                    update.playback_rate = Some(speed);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.playback_rate = None;
                }
                false
            }
            MPV_PROPERTY_PAUSED_FOR_CACHE => {
                if let Some(paused_for_cache) = data.and_then(Value::as_bool) {
                    self.paused_for_cache = paused_for_cache;
                    self.observed_state.paused_for_cache = Some(paused_for_cache);
                    self.observe_network_cache_pause_for_recovery(paused_for_cache);
                    let logical_pause = match self.observed_state.paused {
                        Some(true) if paused_for_cache => None,
                        Some(true) if self.logical_pause_explicit => Some(true),
                        Some(true) => None,
                        paused => paused,
                    };
                    self.observed_state.logical_pause = logical_pause;
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_paused_for_cache(paused_for_cache),
                    );
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.paused_for_cache = Some(paused_for_cache);
                    update.logical_pause = logical_pause;
                    self.observed_state.cache_metrics_observed_at = update.observed_at;
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::CachePause(paused_for_cache),
                    );
                    if let Some(logical_pause) = logical_pause {
                        self.observe_tracked_commands(
                            self.observation_media_generation(),
                            TrackedCommandObservation::LogicalPause(logical_pause),
                        );
                    }
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.paused_for_cache = None;
                }
                false
            }
            MPV_PROPERTY_CACHE_BUFFERING_STATE => {
                if let Some(cache_buffering_percent) = data.and_then(Value::as_f64) {
                    self.cache_buffering_percent = Some(cache_buffering_percent);
                    self.observed_state.cache_buffering_percent = Some(cache_buffering_percent);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_cache_buffering_percent(cache_buffering_percent),
                    );
                    let mut update = self.transport_update();
                    update.cache_buffering_percent = Some(cache_buffering_percent);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.cache_buffering_percent = None;
                    self.observed_state.cache_buffering_percent = None;
                }
                false
            }
            MPV_PROPERTY_SEEKING => {
                if let Some(seeking) = data.and_then(Value::as_bool) {
                    self.observed_state.seeking = Some(seeking);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.seeking = Some(seeking);
                    self.queue_transport_telemetry_update(update);
                    if let Some(media_generation) = self.player_lifecycle.active_media_generation()
                    {
                        let lifecycle_epoch = self.lifecycle_epoch();
                        let observed_sequence = self.player_lifecycle.last_event_sequence();
                        self.apply_lifecycle_input(PlayerLifecycleInput::SeekingObserved {
                            attachment_epoch: lifecycle_epoch,
                            media_generation,
                            observed_sequence,
                            seeking,
                        });
                    }
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Seeking(seeking),
                    );
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.seeking = None;
                }
                false
            }
            MPV_PROPERTY_SEEKABLE => {
                if let Some(seekable) = data.and_then(Value::as_bool) {
                    self.observed_state.seekable = Some(seekable);
                    let mut update = self.transport_update();
                    update.seekable = Some(seekable);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.seekable = None;
                }
                false
            }
            MPV_PROPERTY_CORE_IDLE => {
                if let Some(core_idle) = data.and_then(Value::as_bool) {
                    self.observed_state.core_idle = Some(core_idle);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.core_idle = Some(core_idle);
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.core_idle = None;
                }
                false
            }
            MPV_PROPERTY_DEMUXER_CACHE_STATE => {
                let update = self.cache_state_telemetry_update(data.unwrap_or(&Value::Null));
                self.queue_transport_telemetry_update(update);
                false
            }
            MPV_PROPERTY_YTDL_IS_LIVE => {
                self.observe_ytdl_is_live_for_current_generation(
                    data.is_some_and(Self::metadata_boolean_is_true),
                );
                false
            }
            MPV_PROPERTY_METADATA => {
                self.observe_ytdl_is_live_for_current_generation(Self::full_metadata_ytdl_is_live(
                    data,
                ));
                false
            }
            MPV_PROPERTY_DEMUXER_CACHE_IDLE => {
                if let Some(demuxer_cache_idle) = data.and_then(Value::as_bool) {
                    self.observed_state.demuxer_cache_idle = Some(demuxer_cache_idle);
                    let mut update = self.transport_update();
                    update.demuxer_cache_idle = Some(demuxer_cache_idle);
                    self.observed_state.cache_metrics_observed_at = update.observed_at;
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.demuxer_cache_idle = None;
                }
                false
            }
            MPV_PROPERTY_EOF_REACHED => {
                if let Some(eof_reached) = data.and_then(Value::as_bool) {
                    self.observed_state.eof_reached = Some(eof_reached);
                    let lifecycle_epoch = self.lifecycle_epoch();
                    let lifecycle_playlist_entry_id = self
                        .active_playlist_entry_id
                        .and_then(|entry_id| i64::try_from(entry_id).ok());
                    let position_seconds = self.observed_state.position_seconds;
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.eof_reached = Some(false);
                    self.queue_transport_telemetry_update(update);
                    // The sanitized transport delta deliberately remains non-terminal. Reduce
                    // the physical EOF observation afterwards so that delta's retained Playing
                    // phase cannot erase the provisional candidate it is meant to fence.
                    self.apply_lifecycle_input(PlayerLifecycleInput::EofObserved {
                        attachment_epoch: lifecycle_epoch,
                        playlist_entry_id: lifecycle_playlist_entry_id,
                        reached: eof_reached,
                        position_seconds,
                    });
                } else {
                    self.observed_state.eof_reached = None;
                }
                false
            }
            _ => false,
        };

        if file_metadata_changed {
            if self.refresh_timeline_kind_from_metadata() {
                let update = self.transport_update();
                self.queue_transport_telemetry_update(update);
            }
            self.maybe_emit_local_file_update_from_observed_state();
        }
        if authoritative_path_observed {
            self.observe_authoritative_path_for_network_options(
                authoritative_path.as_deref(),
                AuthoritativePathObservationOrigin::PathEvent,
            );
        }
    }

    fn handle_start_file_event(&mut self, event: &Value) {
        // `pause`, `speed`, and `core-idle` are player/core properties rather
        // than file metadata. mpv does not necessarily emit another property
        // change when an already-paused player begins a new file, so retain
        // their last observations across the media-generation boundary.
        let retained_paused = self.observed_state.paused;
        let retained_logical_pause = self.observed_state.logical_pause;
        let retained_playback_rate = self.observed_state.playback_rate;
        let retained_core_idle = self.observed_state.core_idle;
        let playlist_entry_id = event.get("playlist_entry_id").and_then(Value::as_u64);
        let Some(lifecycle_playlist_entry_id) =
            playlist_entry_id.and_then(|entry_id| i64::try_from(entry_id).ok())
        else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        let lifecycle_epoch = self.lifecycle_epoch();
        self.apply_lifecycle_input(PlayerLifecycleInput::StartFile {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: lifecycle_playlist_entry_id,
        });
        if self
            .player_lifecycle
            .attempt_for_playlist_entry(lifecycle_playlist_entry_id)
            .is_none()
        {
            let has_unbound_candidate =
                self.player_lifecycle.load_attempts.values().any(|attempt| {
                    !attempt.state.is_terminal() && attempt.playlist_entry_id.is_none()
                });
            if has_unbound_candidate {
                #[cfg(test)]
                {
                    // Legacy scripted unit transports do not expose an
                    // authoritative playlist. Remember only the latest observed
                    // entry; matching path/file-loaded evidence performs the
                    // actual binding without weakening production ownership.
                    self.active_playlist_entry_id = playlist_entry_id;
                }
                #[cfg(not(test))]
                {
                    self.lifecycle_reconciliation_due = true;
                }
            } else {
                let generation = self.allocate_media_generation();
                self.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
                    attachment_epoch: lifecycle_epoch,
                    media_generation: generation,
                    playlist_entry_id: lifecycle_playlist_entry_id,
                    observed_target: String::new(),
                    file_loaded: false,
                });
            }
        }
        let Some(lifecycle_attempt_id) = self
            .player_lifecycle
            .attempt_for_playlist_entry(lifecycle_playlist_entry_id)
        else {
            return;
        };
        let Some(generation) = self
            .player_lifecycle
            .load_attempts
            .get(&lifecycle_attempt_id)
            .map(|attempt| attempt.media_generation)
        else {
            return;
        };

        if self.network_media_options_hook_instance_id.is_some()
            && self.network_media_options_hook_configured_generation
                == Some(self.network_media_options_generation)
        {
            let base_sequence = self
                .network_media_options_hook_latest_started_load_sequence
                .or(self.network_media_options_hook_last_accepted_load_sequence)
                .unwrap_or(0);
            let load_sequence = base_sequence.wrapping_add(1).max(1);
            self.network_media_options_hook_latest_started_load_sequence = Some(load_sequence);
            self.network_media_options_expected_transition =
                Some(ExpectedNetworkOptionsTransition {
                    media_generation: generation,
                    load_sequence,
                });
        } else {
            self.network_media_options_expected_transition = None;
        }

        self.network_media_options_apply_identity = None;
        self.reset_network_media_policy_diagnostics();

        self.active_playlist_entry_id = playlist_entry_id;
        self.active_media_generation = Some(generation);
        self.active_file_loaded = false;
        self.active_generation_has_restarted = false;
        self.network_cache_stall = None;
        self.reset_timeline_metadata();
        self.current_path = None;
        self.paused_for_cache = false;
        self.cache_buffering_percent = None;
        self.observed_state.paused = retained_paused;
        self.observed_state.logical_pause = retained_logical_pause;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.observed_state.position_seconds = None;
        self.observed_state.playback_rate = retained_playback_rate;
        self.observed_state.paused_for_cache = None;
        self.observed_state.cache_buffering_percent = None;
        self.observed_state.seeking = None;
        self.observed_state.seekable = None;
        self.observed_state.seekable_ranges = None;
        self.observed_state.core_idle = retained_core_idle;
        self.observed_state.demuxer_cache_idle = None;
        self.observed_state.eof_reached = Some(false);
        self.observed_state.buffered_ahead_seconds = None;
        self.observed_state.buffered_ahead_bytes = None;
        self.observed_state.input_rate_bytes_per_second = None;
        self.observed_state.cache_reader_position_seconds = None;
        self.observed_state.cache_end_seconds = None;
        self.observed_state.cache_eof = None;
        self.observed_state.cache_underrun = None;
        self.observed_state.cache_metrics_observed_at = None;
        self.transport_phase = PlayerTransportPhase::Loading;

        let mut update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading);
        update.logical_pause = retained_logical_pause;
        update.playback_rate = retained_playback_rate;
        update.core_idle = retained_core_idle;
        update.eof_reached = Some(false);
        self.queue_transport_telemetry_update_for_attempt(update, Some(lifecycle_attempt_id));
        self.queue_cache_telemetry_update(self.cleared_cache_telemetry_update(Some(generation)));
        // A later start-file in the same buffered batch invalidates every earlier path even when
        // mpv has not emitted the replacement path yet. The batch reducer will let a following
        // path observation supersede this idle/loading marker before any option write begins.
        self.observe_authoritative_path_for_network_options(
            None,
            AuthoritativePathObservationOrigin::StartFilePending,
        );
    }

    fn handle_seek_event(&mut self) {
        self.network_cache_stall = None;
        self.begin_seek_cache_evidence_epoch();
        self.observed_state.seeking = Some(true);
        self.transport_phase = PlayerTransportPhase::Seeking;
        let mut update = self
            .transport_update()
            .with_phase(PlayerTransportPhase::Seeking);
        update.seeking = Some(true);
        self.queue_transport_telemetry_update(update);
        if let Some(media_generation) = self.player_lifecycle.active_media_generation() {
            let lifecycle_epoch = self.lifecycle_epoch();
            let observed_sequence = self.player_lifecycle.last_event_sequence();
            self.apply_lifecycle_input(PlayerLifecycleInput::SeekingObserved {
                attachment_epoch: lifecycle_epoch,
                media_generation,
                observed_sequence,
                seeking: true,
            });
        }
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Seeking(true),
        );
    }

    fn handle_playback_restart_event(&mut self) {
        self.network_cache_stall = None;
        let lifecycle_epoch = self.lifecycle_epoch();
        let lifecycle_playlist_entry_id = self
            .active_playlist_entry_id
            .and_then(|entry_id| i64::try_from(entry_id).ok());
        #[cfg(test)]
        if let Some(playlist_entry_id) = lifecycle_playlist_entry_id
            && self
                .player_lifecycle
                .attempt_for_playlist_entry(playlist_entry_id)
                .is_none()
        {
            self.bind_single_pending_test_load(playlist_entry_id);
        }
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: lifecycle_playlist_entry_id,
        });
        let Some(active_attempt) = self.player_lifecycle.active_attempt().cloned() else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        if lifecycle_playlist_entry_id.is_some()
            && active_attempt.playlist_entry_id != lifecycle_playlist_entry_id
        {
            self.lifecycle_reconciliation_due = true;
            return;
        }
        self.active_media_generation = Some(active_attempt.media_generation);
        self.active_file_loaded = true;
        self.active_generation_has_restarted = true;
        self.playback_restart_sequence = self.playback_restart_sequence.wrapping_add(1).max(1);
        self.observed_state.seeking = Some(false);
        self.observed_state.eof_reached = Some(false);
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;

        let mut update = self
            .transport_update_for(active_attempt.media_generation)
            .with_phase(phase);
        update.seeking = Some(false);
        update.eof_reached = Some(false);
        update.playback_restart_sequence = Some(self.playback_restart_sequence);
        self.queue_transport_telemetry_update_for_attempt(update, Some(active_attempt.id));
        let media_generation = self.observation_media_generation();
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Seeking(false));
        self.observe_tracked_commands(
            media_generation,
            TrackedCommandObservation::PlaybackRestart(self.playback_restart_sequence),
        );
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Phase(phase));
    }

    fn handle_file_loaded_event(&mut self) {
        #[allow(unused_mut)]
        let mut playlist_entry_id = self
            .active_playlist_entry_id
            .and_then(|entry_id| i64::try_from(entry_id).ok());
        #[cfg(test)]
        if playlist_entry_id.is_none_or(|entry_id| {
            self.player_lifecycle
                .attempt_for_playlist_entry(entry_id)
                .is_none()
        }) {
            let test_entry_id = self
                .active_playlist_entry_id
                .and_then(|entry_id| i64::try_from(entry_id).ok())
                .or_else(|| {
                    self.player_lifecycle
                        .load_attempts
                        .values()
                        .find(|attempt| {
                            !attempt.state.is_terminal() && attempt.playlist_entry_id.is_none()
                        })
                        .and_then(|attempt| i64::try_from(attempt.id.get()).ok())
                });
            if let Some(test_entry_id) = test_entry_id
                && self.bind_single_pending_test_load(test_entry_id)
            {
                playlist_entry_id = Some(test_entry_id);
            }
        }
        let Some(playlist_entry_id) = playlist_entry_id else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        let lifecycle_epoch = self.lifecycle_epoch();
        let Some(attempt_id) = self
            .player_lifecycle
            .attempt_for_playlist_entry(playlist_entry_id)
        else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        let Some(generation) = self
            .player_lifecycle
            .load_attempts
            .get(&attempt_id)
            .map(|attempt| attempt.media_generation)
        else {
            return;
        };
        self.apply_lifecycle_input(PlayerLifecycleInput::FileLoaded {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: Some(playlist_entry_id),
            loaded_target: self.observed_state.path.clone(),
        });
        self.active_media_generation = Some(generation);
        self.active_file_loaded = true;
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;
        let update = self.transport_update().with_phase(phase);
        self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        let media_generation = self.observation_media_generation();
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::FileLoaded);
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Phase(phase));

        if self.pending_load_generation != Some(generation) {
            return;
        }
        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };
        self.pending_load_generation = None;

        let polled_update = self
            .poll_local_file_update_from_mpv_coherent()
            .ok()
            .flatten();
        let metadata_is_current = polled_update.is_some();
        let loaded_update =
            polled_update.unwrap_or_else(|| Self::local_file_update_for_path(&requested_target));
        self.current_path = loaded_update.path.clone();
        self.observed_state.path = loaded_update.path.clone();
        self.observed_state.duration_seconds = loaded_update.duration_seconds;
        self.observed_state.size_bytes = loaded_update.size_bytes;
        if metadata_is_current {
            self.path_metadata_generation = self.active_media_generation;
            self.duration_metadata_generation = self.active_media_generation;
        }
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        if let Some(path) = loaded_update.path.as_deref() {
            self.observe_authoritative_path_for_network_options(
                Some(path),
                AuthoritativePathObservationOrigin::Poll,
            );
        }
        if Self::local_file_update_ready_for_sync(&loaded_update) {
            self.record_local_file_update_if_changed(loaded_update.clone());
        }
        self.queue_media_load_outcome_for_generation(
            PlayerMediaLoadOutcome::success(requested_target, loaded_update.path.clone()),
            Some(generation),
        );
    }

    fn handle_end_file_event(&mut self, event: &Value) {
        let reason = event
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        #[allow(unused_mut)]
        let mut playlist_entry_id = event.get("playlist_entry_id").and_then(Value::as_u64);
        #[cfg(test)]
        if playlist_entry_id.is_none() {
            playlist_entry_id = self.active_playlist_entry_id;
        }
        #[cfg(test)]
        if playlist_entry_id.is_none() {
            let test_entry_id = self
                .player_lifecycle
                .load_attempts
                .values()
                .find(|attempt| !attempt.state.is_terminal() && attempt.playlist_entry_id.is_none())
                .and_then(|attempt| i64::try_from(attempt.id.get()).ok());
            if let Some(test_entry_id) = test_entry_id
                && self.bind_single_pending_test_load(test_entry_id)
            {
                playlist_entry_id = u64::try_from(test_entry_id).ok();
            }
        }
        let Some(lifecycle_playlist_entry_id) =
            playlist_entry_id.and_then(|entry_id| i64::try_from(entry_id).ok())
        else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        #[cfg(test)]
        if self
            .player_lifecycle
            .attempt_for_playlist_entry(lifecycle_playlist_entry_id)
            .is_none()
        {
            self.bind_single_pending_test_load(lifecycle_playlist_entry_id);
        }
        let Some(lifecycle_attempt_id) = self
            .player_lifecycle
            .attempt_for_playlist_entry(lifecycle_playlist_entry_id)
        else {
            let already_terminal = self.player_lifecycle.load_attempts.values().any(|attempt| {
                attempt.playlist_entry_id == Some(lifecycle_playlist_entry_id)
                    && attempt.state.is_terminal()
            });
            if !already_terminal {
                self.lifecycle_reconciliation_due = true;
            }
            return;
        };
        let generation = self
            .player_lifecycle
            .load_attempts
            .get(&lifecycle_attempt_id)
            .map(|attempt| attempt.media_generation);
        let message = (reason == MPV_END_FILE_REASON_ERROR).then(|| {
            event
                .get("file_error")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    event.get("error").and_then(|value| match value {
                        Value::String(message) => Some(message.trim().to_owned()),
                        Value::Number(number) => Some(format!("mpv error code {number}")),
                        _ => None,
                    })
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "mpv failed to load the requested media.".to_owned())
        });
        let error_kind = message
            .as_deref()
            .map(Self::media_load_failure_kind_from_message);
        let phase = if error_kind.is_some() {
            PlayerTransportPhase::Failed
        } else {
            PlayerTransportPhase::Ended
        };
        let physical_outcome = error_kind.map_or(
            PlayerPhysicalLoadOutcome::Ended,
            PlayerPhysicalLoadOutcome::Failed,
        );
        let recovery_started = if reason == "eof"
            && self.player_lifecycle.active_load_attempt == Some(lifecycle_attempt_id)
        {
            generation
                .is_some_and(|generation| self.try_recover_interrupted_network_stream(generation))
        } else {
            false
        };
        let lifecycle_epoch = self.lifecycle_epoch();
        let lifecycle_effects = self.apply_lifecycle_input(PlayerLifecycleInput::EndFile {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: lifecycle_playlist_entry_id,
            outcome: physical_outcome,
        });
        let logical_terminal = lifecycle_effects.iter().any(|effect| {
            matches!(
                effect,
                PlayerLifecycleEffect::LogicalPlaybackTerminal {
                    attempt_id,
                    ..
                } if *attempt_id == lifecycle_attempt_id
            )
        });
        if recovery_started {
            // The old physical attempt is now terminal in the reducer, while
            // the accepted same-generation successor owns logical playback.
            // Its Loading delta was queued by the recovery transaction.
            return;
        }

        if logical_terminal && let Some(generation) = generation {
            let mut update = self.transport_update_for(generation).with_phase(phase);
            update.eof_reached = Some(true);
            update.error_kind = error_kind;
            self.queue_transport_telemetry_update_for_attempt(update, Some(lifecycle_attempt_id));
            if logical_terminal {
                self.queue_cache_telemetry_update(
                    self.cleared_cache_telemetry_update(Some(generation)),
                );
                self.fail_tracked_commands_for_generation(
                    generation,
                    PlayerCommandFailureKind::MediaEnded,
                );
            }
        }

        let affects_current_generation = logical_terminal;
        if affects_current_generation {
            self.interrupted_network_stream_recovery = None;
            self.network_cache_stall = None;
            if self
                .network_media_options_expected_transition
                .is_some_and(|expected| Some(expected.media_generation) == generation)
            {
                self.network_media_options_expected_transition = None;
            }
            self.transport_phase = phase;
            self.active_file_loaded = false;
            self.active_media_generation = None;
            self.reset_timeline_metadata();
            self.observed_state.eof_reached = Some(true);
            self.observed_state.buffered_ahead_seconds = None;
            self.observed_state.buffered_ahead_bytes = None;
            self.observed_state.input_rate_bytes_per_second = None;
            self.observed_state.cache_reader_position_seconds = None;
            self.observed_state.cache_end_seconds = None;
            self.observed_state.cache_eof = None;
            self.observed_state.cache_underrun = None;
            self.observed_state.cache_metrics_observed_at = None;
            if self.active_playlist_entry_id == playlist_entry_id {
                self.active_playlist_entry_id = None;
            }
            if self
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| Some(embedded.media_generation) == generation)
            {
                self.network_media_options_embedded_load = None;
            }
            // end-file is authoritative for the matching active generation even when mpv does
            // not emit a separate path=null observation. It must supersede a network path seen
            // earlier in the same buffered batch before that path can trigger an option write.
            self.observe_authoritative_path_for_network_options(
                None,
                AuthoritativePathObservationOrigin::EndFileIdle,
            );
        }

        if !logical_terminal || reason != MPV_END_FILE_REASON_ERROR {
            return;
        }

        if self.pending_load_generation != generation {
            return;
        }
        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };
        self.pending_load_generation = None;
        let message = message.expect("error end-file events should have a fallback message");
        self.current_path = None;
        self.pending_local_file_update = None;
        self.pending_local_file_generation = None;
        self.pending_local_file_observed_at = None;
        self.last_polled_local_file_update = None;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.reset_timeline_metadata();
        self.queue_media_load_outcome_for_generation(
            PlayerMediaLoadOutcome::failure(
                requested_target,
                None,
                error_kind.unwrap_or(PlayerMediaLoadFailureKind::Unknown),
                message,
            ),
            generation,
        );
    }

    fn handle_client_message_event(&mut self, event: &Value) {
        let Some(args) = event.get("args").and_then(Value::as_array) else {
            return;
        };
        let Some(message_name) = args.first().and_then(Value::as_str) else {
            return;
        };
        let payload = args.get(1).and_then(Value::as_str);
        match message_name {
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_CONFIGURED => {
                self.handle_network_options_hook_configured(payload);
            }
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP => {
                self.handle_network_options_hook_ownership(payload);
            }
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT => {
                self.handle_network_options_hook_heartbeat(payload);
            }
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_ACTIVE_RESULT => {
                self.handle_network_options_hook_active_result(payload);
            }
            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT => {
                self.handle_network_options_hook_transition_result(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED => {
                self.handle_legacy_syncplayintf_options_ack(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG => {
                self.handle_legacy_syncplayintf_pong(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED => {
                self.handle_legacy_syncplayintf_lease_expired(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT => {
                self.handle_legacy_syncplayintf_chat_request(payload);
            }
            _ => {}
        }
    }

    fn parse_network_options_hook_status(parsed: &Value) -> Option<NetworkOptionsHookApplyStatus> {
        let wire_status = parsed.get("status").and_then(Value::as_str)?;
        match (
            wire_status,
            parsed.get("applicationState").and_then(Value::as_str),
        ) {
            ("network-updated", Some("applied")) => {
                return Some(NetworkOptionsHookApplyStatus::NetworkMediaUpdated);
            }
            ("failed", Some("partially-applied")) => {
                return Some(NetworkOptionsHookApplyStatus::PartiallyApplied);
            }
            ("failed", Some("failed")) => return Some(NetworkOptionsHookApplyStatus::Failed),
            (_, Some(_)) => return None,
            _ => {}
        }
        match wire_status {
            "no-active" => Some(NetworkOptionsHookApplyStatus::NoActiveMedia),
            "local" => Some(NetworkOptionsHookApplyStatus::LocalMediaUnchanged),
            "network-updated" => Some(NetworkOptionsHookApplyStatus::NetworkMediaUpdated),
            // Accepted for compatibility with short-lived development builds. The bundled v3
            // hook uses legacy `failed` plus `applicationState=partially-applied`, so an older
            // v3 adapter still fails closed instead of silently ignoring a new wire status.
            "partially-applied" => Some(NetworkOptionsHookApplyStatus::PartiallyApplied),
            "failed" => Some(NetworkOptionsHookApplyStatus::Failed),
            _ => None,
        }
    }

    fn parse_network_options_hook_option_results(
        parsed: &Value,
    ) -> Vec<NetworkOptionsHookOptionResult> {
        parsed
            .get("optionResults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|result| {
                let name = result
                    .get("name")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_owned();
                if name.is_empty() {
                    return None;
                }
                let status = match result.get("status").and_then(Value::as_str)? {
                    "applied" => NetworkOptionsHookOptionApplyStatus::Applied,
                    "rejected" => NetworkOptionsHookOptionApplyStatus::Rejected,
                    _ => return None,
                };
                Some(NetworkOptionsHookOptionResult { name, status })
            })
            .collect()
    }

    fn parse_network_options_hook_effective_options(parsed: &Value) -> BTreeMap<String, String> {
        parsed
            .get("effectiveOptions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| {
                Self::canonical_network_media_diagnostic_value(name, value.as_str()?)
                    .map(|value| (name.clone(), value))
            })
            .collect()
    }

    fn network_options_hook_verification_complete(parsed: &Value) -> bool {
        parsed.get("verification").and_then(Value::as_str) == Some("complete")
    }

    fn parse_network_options_hook_payload(&self, payload: Option<&str>) -> Option<Value> {
        let parsed = serde_json::from_str::<Value>(payload?).ok()?;
        (parsed.get("protocol").and_then(Value::as_str) == Some(SOROTTE_NETWORK_OPTIONS_PROTOCOL)
            && parsed.get("ownerId").and_then(Value::as_str)
                == Some(self.legacy_syncplayintf_owner_id.as_str())
            && parsed.get("attachmentId").and_then(Value::as_str)
                == Some(self.legacy_syncplayintf_attachment_id.as_str()))
        .then_some(parsed)
    }

    fn network_options_hook_generation(parsed: &Value) -> Option<u64> {
        parsed.get("configurationGeneration").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    fn network_options_hook_load_sequence(parsed: &Value) -> Option<u64> {
        parsed.get("loadSequence").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    fn network_options_hook_current_load_sequence(parsed: &Value) -> Option<u64> {
        parsed.get("currentLoadSequence").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    fn network_options_hook_instance_id(parsed: &Value) -> Option<&str> {
        parsed
            .get("hookInstanceId")
            .and_then(Value::as_str)
            .filter(|instance_id| !instance_id.is_empty())
    }

    fn network_options_hook_matches_configured_instance(&self, parsed: &Value) -> bool {
        matches!(
            (
                Self::network_options_hook_instance_id(parsed),
                self.network_media_options_hook_instance_id.as_deref(),
            ),
            (Some(received), Some(configured)) if received == configured
        )
    }

    fn network_options_hook_path(parsed: &Value, key: &str) -> Option<String> {
        parsed
            .get(key)
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
    }

    fn handle_network_options_hook_configured(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            self.network_media_options_hook_configuration_error =
                Some("Sorotte's mpv network-options hook omitted a valid generation".to_owned());
            return;
        };
        if generation != self.network_media_options_generation {
            return;
        }
        match parsed.get("status").and_then(Value::as_str) {
            Some("configured") => {
                let Some(hook_instance_id) = Self::network_options_hook_instance_id(&parsed) else {
                    self.network_media_options_hook_configuration_error = Some(
                        "Sorotte's mpv network-options hook omitted its instance id".to_owned(),
                    );
                    return;
                };
                let Some(current_load_sequence) =
                    Self::network_options_hook_current_load_sequence(&parsed)
                else {
                    self.network_media_options_hook_configuration_error = Some(
                        "Sorotte's mpv network-options hook omitted its current load sequence"
                            .to_owned(),
                    );
                    return;
                };
                if self.network_media_options_hook_instance_id.as_deref() == Some(hook_instance_id)
                {
                    if self
                        .network_media_options_hook_last_accepted_load_sequence
                        .is_some_and(|accepted| current_load_sequence < accepted)
                    {
                        let accepted = self
                            .network_media_options_hook_last_accepted_load_sequence
                            .expect("the regression guard established an accepted sequence");
                        let reason = format!(
                            "Sorotte's mpv network-options hook reported a regressed load sequence ({current_load_sequence} below {accepted}) for the same instance"
                        );
                        self.invalidate_network_media_options_hook_delivery();
                        self.pending_network_media_options_hook_active_result = None;
                        self.deferred_network_media_options_hook_transition_result = None;
                        self.network_media_options_hook_configuration_error = Some(reason.clone());
                        self.queue_network_media_options_hook_degraded(
                            PlayerError::OperationFailed(reason),
                        );
                        return;
                    }
                    self.network_media_options_hook_last_accepted_load_sequence = Some(
                        self.network_media_options_hook_last_accepted_load_sequence
                            .map_or(current_load_sequence, |accepted| {
                                accepted.max(current_load_sequence)
                            }),
                    );
                    self.network_media_options_hook_latest_started_load_sequence = Some(
                        self.network_media_options_hook_latest_started_load_sequence
                            .map_or(current_load_sequence, |started| {
                                started.max(current_load_sequence)
                            }),
                    );
                } else {
                    self.network_media_options_hook_instance_id = Some(hook_instance_id.to_owned());
                    self.network_media_options_hook_last_accepted_load_sequence =
                        Some(current_load_sequence);
                    self.network_media_options_hook_latest_started_load_sequence =
                        Some(current_load_sequence);
                    self.network_media_options_expected_transition = None;
                    self.pending_network_media_options_hook_active_result = None;
                    self.deferred_network_media_options_hook_transition_result = None;
                }
                self.network_media_options_hook_loaded = true;
                self.network_media_options_hook_ownership_possible = true;
                self.network_media_options_hook_configured_generation = Some(generation);
                self.network_media_options_hook_configuration_error = None;
                self.network_media_options_hook_last_heartbeat_at = Some(Instant::now());
                self.network_media_options_hook_pending_heartbeat = None;
                self.network_media_options_hook_pending_event_poll_command_id = None;
                self.queue_network_media_options_hook_recovered();
            }
            Some("stale") => {
                self.network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook rejected stale generation {generation}"
                ));
            }
            Some("owner-live") => {
                let active_owner = parsed
                    .get("activeOwnerId")
                    .and_then(Value::as_str)
                    .unwrap_or("another Sorotte process");
                self.network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook is owned by {active_owner}"
                ));
            }
            _ => {
                self.network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook returned an invalid status for generation {generation}"
                ));
            }
        }
    }

    fn handle_network_options_hook_ownership(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let status = parsed.get("status").and_then(Value::as_str);
        let reason = match status {
            Some("ownership-lost") => "Sorotte's mpv network-options hook ownership was replaced",
            Some("lease-expired") => "Sorotte's mpv network-options hook lease expired",
            Some("released") => {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
                self.network_media_options_hook_ownership_possible = false;
                self.network_media_options_hook_configured_generation = None;
                self.network_media_options_hook_last_heartbeat_at = None;
                self.network_media_options_hook_pending_heartbeat = None;
                self.network_media_options_hook_pending_event_poll_command_id = None;
                self.network_media_options_hook_instance_id = None;
                self.network_media_options_hook_last_accepted_load_sequence = None;
                self.network_media_options_hook_latest_started_load_sequence = None;
                self.network_media_options_expected_transition = None;
                self.pending_network_media_options_hook_active_result = None;
                return;
            }
            _ => return,
        };
        self.network_media_options_hook_configured_generation = None;
        self.network_media_options_hook_last_heartbeat_at = None;
        self.network_media_options_hook_pending_heartbeat = None;
        self.network_media_options_hook_pending_event_poll_command_id = None;
        self.network_media_options_hook_instance_id = None;
        self.network_media_options_hook_last_accepted_load_sequence = None;
        self.network_media_options_hook_latest_started_load_sequence = None;
        self.network_media_options_expected_transition = None;
        self.network_media_options_hook_ownership_possible = false;
        self.pending_network_media_options_hook_active_result = None;
        self.network_media_options_hook_configuration_error = Some(reason.to_owned());
        self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
            reason.to_owned(),
        ));
    }

    fn handle_network_options_hook_heartbeat(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        if Self::network_options_hook_generation(&parsed)
            != self.network_media_options_hook_configured_generation
            || parsed.get("status").and_then(Value::as_str) != Some("renewed")
        {
            return;
        }
        let Some(nonce) = parsed.get("heartbeatNonce").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        if self
            .network_media_options_hook_pending_heartbeat
            .is_some_and(|pending| pending.nonce == nonce)
        {
            self.network_media_options_hook_pending_heartbeat = None;
            self.network_media_options_hook_pending_event_poll_command_id = None;
            self.network_media_options_hook_last_heartbeat_at = Some(Instant::now());
            self.queue_network_media_options_hook_recovered();
        }
    }

    fn handle_network_options_hook_active_result(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            return;
        };
        let Some(attempt_id) = parsed.get("attempt").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        let Some(status) = Self::parse_network_options_hook_status(&parsed) else {
            return;
        };
        let Some(load_sequence) = Self::network_options_hook_load_sequence(&parsed) else {
            return;
        };
        let source_path = Self::network_options_hook_path(&parsed, "sourcePath");
        let stream_open_filename = Self::network_options_hook_path(&parsed, "streamOpenFilename");
        self.pending_network_media_options_hook_active_result =
            Some(NetworkOptionsHookActiveResult {
                attempt_id,
                generation,
                load_sequence,
                source_kind: NetworkOptionsMediaTargetKind::from_target(source_path.as_deref()),
                stream_target_kind: NetworkOptionsMediaTargetKind::from_target(
                    stream_open_filename.as_deref(),
                ),
                source_path: source_path.map(SecretValue::from),
                status,
                verification_complete: Self::network_options_hook_verification_complete(&parsed),
                option_results: Self::parse_network_options_hook_option_results(&parsed),
                effective_options: Self::parse_network_options_hook_effective_options(&parsed),
            });
    }

    fn handle_network_options_hook_transition_result(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            return;
        };
        if Some(generation) != self.network_media_options_hook_configured_generation {
            return;
        }
        let Some(status) = Self::parse_network_options_hook_status(&parsed) else {
            return;
        };
        let Some(load_sequence) = Self::network_options_hook_load_sequence(&parsed) else {
            return;
        };
        if self
            .network_media_options_expected_transition
            .is_some_and(|expected| {
                self.active_media_generation != Some(expected.media_generation)
                    || load_sequence != expected.load_sequence
            })
        {
            return;
        }
        if self
            .network_media_options_hook_last_accepted_load_sequence
            .is_some_and(|accepted| load_sequence <= accepted)
            || self
                .deferred_network_media_options_hook_transition_result
                .as_ref()
                .is_some_and(|pending| load_sequence <= pending.load_sequence)
        {
            return;
        }
        let source_path = Self::network_options_hook_path(&parsed, "sourcePath");
        let stream_open_filename = Self::network_options_hook_path(&parsed, "streamOpenFilename");
        self.deferred_network_media_options_hook_transition_result =
            Some(NetworkOptionsHookTransitionResult {
                generation,
                load_sequence,
                source_kind: NetworkOptionsMediaTargetKind::from_target(source_path.as_deref()),
                stream_target_kind: NetworkOptionsMediaTargetKind::from_target(
                    stream_open_filename.as_deref(),
                ),
                source_path: source_path.map(SecretValue::from),
                status,
                verification_complete: Self::network_options_hook_verification_complete(&parsed),
                option_results: Self::parse_network_options_hook_option_results(&parsed),
                effective_options: Self::parse_network_options_hook_effective_options(&parsed),
            });
    }

    fn apply_network_options_hook_transition_result(
        &mut self,
        result: NetworkOptionsHookTransitionResult,
        observed_path: Option<Option<String>>,
    ) {
        if result.generation != self.network_media_options_generation
            || self
                .network_media_options_hook_last_accepted_load_sequence
                .is_some_and(|accepted| result.load_sequence <= accepted)
        {
            return;
        }
        if let Some(expected) = self.network_media_options_expected_transition {
            if self.active_media_generation != Some(expected.media_generation)
                || result.load_sequence != expected.load_sequence
            {
                return;
            }
            if let Some(Some(observed_path)) = observed_path.as_ref()
                && result.source_path.as_ref().is_none_or(|source| {
                    !Self::media_target_matches(source.expose_secret(), observed_path)
                })
            {
                return;
            }
            self.network_media_options_expected_transition = None;
        }
        self.network_media_options_hook_last_accepted_load_sequence = Some(result.load_sequence);
        self.queue_network_media_options_hook_recovered();

        let completes_pending_policy = self.network_media_options_apply_identity.is_some()
            || matches!(
                self.network_media_options_policy_state,
                MpvNetworkMediaPolicyState::Failed(_)
                    | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
            );
        match result.status {
            NetworkOptionsHookApplyStatus::NoActiveMedia => {
                self.clear_network_media_options_path_identity();
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::NoActiveMedia,
                    Some(result.load_sequence),
                );
                if completes_pending_policy {
                    self.queue_network_media_policy_outcome(
                        MpvNetworkMediaPolicyOutcome::NoActiveMedia,
                    );
                }
            }
            NetworkOptionsHookApplyStatus::LocalMediaUnchanged => {
                if let Some(path) = result.source_path.as_ref().map(SecretValue::expose_secret) {
                    self.begin_network_media_options_apply_attempt(
                        self.active_media_generation,
                        path,
                    );
                }
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                    Some(result.load_sequence),
                );
                self.queue_network_media_policy_outcome(
                    MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged,
                );
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated
            | NetworkOptionsHookApplyStatus::PartiallyApplied
            | NetworkOptionsHookApplyStatus::Failed => {
                if let Some(path) = result.source_path.as_ref().map(SecretValue::expose_secret) {
                    self.begin_network_media_options_apply_attempt(
                        self.active_media_generation,
                        path,
                    );
                }
                let application_state = self.record_network_media_option_application(
                    result.load_sequence,
                    result.status,
                    result.verification_complete,
                    result.option_results,
                    result.effective_options,
                );
                if application_state == MpvNetworkMediaPolicyApplicationState::Applied {
                    self.record_network_media_options_policy_applied(
                        MpvNetworkMediaPolicyState::NetworkMediaUpdated,
                        Some(result.load_sequence),
                    );
                    self.queue_network_media_policy_outcome(
                        MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated,
                    );
                } else {
                    let error = self.network_media_option_application_error(
                        result.load_sequence,
                        result.source_kind,
                        result.stream_target_kind,
                        application_state,
                    );
                    self.queue_network_media_policy_outcome(MpvNetworkMediaPolicyOutcome::Failed(
                        error,
                    ));
                }
            }
        }
    }

    fn handle_legacy_syncplayintf_options_ack(&mut self, payload: Option<&str>) {
        let parsed = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok());
        let Some(parsed) = parsed else {
            if self
                .legacy_syncplayintf_pending_options_generation
                .is_some()
            {
                self.legacy_syncplayintf_options_ack_error = Some(
                    "Sorotte syncplayintf returned a malformed settings acknowledgement".to_owned(),
                );
            }
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(pending_generation) = self.legacy_syncplayintf_pending_options_generation else {
            return;
        };
        let Some(generation) = parsed.get("generation").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            self.legacy_syncplayintf_options_ack_error =
                Some("Sorotte syncplayintf acknowledgement omitted a valid generation".to_owned());
            return;
        };
        if generation < pending_generation {
            return;
        }
        if generation > pending_generation {
            self.legacy_syncplayintf_options_ack_error = Some(format!(
                "Sorotte syncplayintf acknowledged unexpected future generation {generation} while waiting for {pending_generation}"
            ));
            return;
        }
        match parsed.get("status").and_then(Value::as_str) {
            Some("applied") => {
                self.legacy_syncplayintf_options_applied = true;
                self.legacy_syncplayintf_pending_options_generation = None;
                self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
                self.legacy_syncplayintf_options_ack_error = None;
                self.legacy_syncplayintf_lease_reacquire_required = false;
                let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                    SorotteBridgeHealth::Ready
                } else {
                    SorotteBridgeHealth::Disabled
                };
                self.set_sorotte_bridge_health(health);
            }
            Some(status @ ("busy" | "rejected")) => {
                let detail = parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("the bridge rejected the settings update");
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf did not apply generation {generation}: {detail}"
                ));
                if status == "busy" {
                    self.legacy_syncplayintf_lease_reacquire_required = true;
                }
            }
            _ => {
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf returned an invalid status for generation {generation}"
                ));
            }
        }
    }

    fn handle_legacy_syncplayintf_pong(&mut self, payload: Option<&str>) {
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL) {
            return;
        }
        let Some(nonce) = parsed.get("nonce").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        if self.legacy_syncplayintf_pending_ping_nonce != Some(nonce) {
            return;
        }
        let Some(bridge_instance_id) = parsed
            .get("bridgeInstanceId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let Some(script_name) = parsed
            .get("scriptName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let bridge_instance_changed = self
            .legacy_syncplayintf_bridge_instance_id
            .as_deref()
            .is_some_and(|current| current != bridge_instance_id);
        if bridge_instance_changed {
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
        }
        self.legacy_syncplayintf_bridge_instance_id = Some(bridge_instance_id.to_owned());
        self.legacy_syncplayintf_script_name = script_name.to_owned();
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
        if bridge_instance_changed {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::Discovery,
                format!(
                    "Sorotte's mpv bridge instance changed to {bridge_instance_id}; reapplying runtime settings"
                ),
                false,
            );
        }
    }

    fn handle_legacy_syncplayintf_lease_expired(&mut self, payload: Option<&str>) {
        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = Some(
            "Sorotte syncplayintf input lease expired; reapplying the current settings".to_owned(),
        );
        self.begin_sorotte_bridge_runtime_recovery(
            SorotteBridgeFailureKind::AcknowledgementTimeout,
            "Sorotte syncplayintf input lease expired; reapplying the current settings",
            false,
        );
    }

    fn handle_legacy_syncplayintf_chat_request(&mut self, payload: Option<&str>) {
        if !self.chat_input_polling_enabled() {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(message) = parsed.get("text").and_then(Value::as_str) else {
            return;
        };
        self.pending_chat_requests.push_back(message.to_owned());
    }

    fn maybe_emit_local_file_update_from_observed_state(&mut self) {
        if self.pending_load_request().is_some() {
            return;
        }
        let Some(path) = self.observed_state.path.as_deref() else {
            return;
        };

        let mut update = Self::local_file_update_for_path(path);
        if let Some(duration_seconds) = self.observed_state.duration_seconds {
            update = update.with_duration_seconds(duration_seconds);
        }
        if let Some(size_bytes) = self.observed_state.size_bytes {
            update = update.with_size_bytes(size_bytes);
        }
        if Self::local_file_update_ready_for_sync(&update) {
            self.record_local_file_update_if_changed(update);
        }
    }

    fn send_ipc_command_if_attached(&mut self, command: Value) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached_without_draining_events(command)?;
        self.drain_ipc_events_if_attached();
        Ok(())
    }

    fn send_ipc_command_if_attached_without_draining_events(
        &mut self,
        command: Value,
    ) -> Result<(), PlayerError> {
        if let Some(ipc_client) = self.ipc_client.as_mut() {
            ipc_client
                .send_command_expect_success(command)
                .map_err(PlayerError::OperationFailed)?;
        } else if !self.simulation_mode {
            return Err(PlayerError::NotConnected);
        }
        Ok(())
    }

    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
        };

        LocalFileUpdate::new(name)
            .with_size_bytes(size_bytes)
            .with_path(path.to_owned())
    }

    fn local_file_update_ready_for_sync(update: &LocalFileUpdate) -> bool {
        match update.path.as_deref() {
            Some(path) if !path.contains("://") => update.duration_seconds.is_some(),
            _ => true,
        }
    }

    fn local_file_update_matches_request(update: &LocalFileUpdate, requested_target: &str) -> bool {
        if requested_target.trim().is_empty() {
            return false;
        }

        if let Some(path) = update.path.as_deref()
            && Self::media_target_matches(path, requested_target)
        {
            return true;
        }

        if Self::media_target_matches(&update.name, requested_target) {
            return true;
        }

        Path::new(requested_target)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|requested_name| Self::media_target_matches(&update.name, requested_name))
    }

    fn media_target_matches(left: &str, right: &str) -> bool {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }

    fn media_load_failure_kind_from_message(message: &str) -> PlayerMediaLoadFailureKind {
        let normalized = message.to_ascii_lowercase();
        if normalized.contains("failed to recognize file format")
            || normalized.contains("unsupported")
        {
            return PlayerMediaLoadFailureKind::FormatUnsupported;
        }
        if (normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno"))
            && (normalized.contains("not found")
                || normalized.contains("not enough permissions")
                || normalized.contains("no such file"))
        {
            return PlayerMediaLoadFailureKind::HelperMissing;
        }
        if normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno")
        {
            return PlayerMediaLoadFailureKind::HelperBroken;
        }
        if normalized.contains("connection")
            || normalized.contains("network")
            || normalized.contains("http")
            || normalized.contains("timed out")
        {
            return PlayerMediaLoadFailureKind::Network;
        }
        if normalized.contains("aborted") || normalized.contains("interrupt") {
            return PlayerMediaLoadFailureKind::LoadAborted;
        }
        PlayerMediaLoadFailureKind::Unknown
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport(transport: impl MpvJsonIpcTransport + 'static) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            network_media_options_hook_enabled: false,
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn with_network_options_hook_test_transport(
        transport: impl MpvJsonIpcTransport + 'static,
    ) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            network_media_options_hook_enabled: true,
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport_and_registered_observers(
        transport: impl MpvJsonIpcTransport + 'static,
    ) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            network_media_options_hook_enabled: false,
            observers_registered: true,
            transport_observers_registered: true,
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport_and_ipc_timeout(
        transport: impl MpvJsonIpcTransport + 'static,
        command_timeout: std::time::Duration,
    ) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new_with_command_timeout(
                Box::new(transport),
                command_timeout,
            )),
            network_media_options_hook_enabled: false,
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn enable_test_legacy_chat_input(&mut self) {
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_bridge_instance_id = Some("test-bridge".to_owned());
        self.legacy_syncplayintf_owner_id = "test-owner".to_owned();
        self.legacy_syncplayintf_attachment_id = "test-attachment".to_owned();
        self.legacy_syncplayintf_options_applied = true;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = Some(1);
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplay_ui_settings.chat_input_enabled = true;
    }

    #[cfg(test)]
    pub(crate) fn reset_test_legacy_syncplayintf_attachment(&mut self) {
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
    }

    #[cfg(test)]
    pub(crate) fn replace_test_ipc_transport(
        &mut self,
        transport: impl MpvJsonIpcTransport + 'static,
    ) {
        self.release_sorotte_bridge_best_effort();
        self.collect_ipc_connection_events();
        self.simulation_mode = false;
        self.ipc_client = Some(MpvJsonIpcClient::new(Box::new(transport)));
        self.ipc_endpoint = None;
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
        self.observers_registered = false;
        self.transport_observers_registered = false;
        self.reset_network_media_options_attachment_state();
        self.legacy_syncplay_osd_placement_restore = None;
    }

    #[cfg(test)]
    pub(crate) fn force_test_legacy_syncplayintf_heartbeat_due(&mut self) {
        self.legacy_syncplayintf_last_heartbeat_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL);
        self.maintain_legacy_syncplayintf_lease();
    }

    #[cfg(test)]
    pub(crate) fn force_test_network_media_options_hook_heartbeat_due(&mut self) {
        self.network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);
        self.maintain_network_media_options_hook_lease();
    }

    #[cfg(test)]
    pub(crate) fn force_test_network_media_options_hook_heartbeat_ack_timeout(&mut self) {
        if let Some(pending) = self.network_media_options_hook_pending_heartbeat.as_mut() {
            pending.sent_at = Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT);
        }
        self.maintain_network_media_options_hook_lease();
    }

    #[cfg(test)]
    pub(crate) fn test_network_media_options_hook_heartbeat_pending(&self) -> bool {
        self.network_media_options_hook_pending_heartbeat.is_some()
    }

    #[cfg(test)]
    pub(crate) fn test_network_media_options_hook_is_ready(&self) -> bool {
        self.network_media_options_hook_is_ready()
    }

    #[cfg(test)]
    pub(crate) fn prepare_test_network_options_hook_v3_reducer(&mut self) {
        self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        self.network_media_options_hook_loaded = true;
        self.network_media_options_hook_instance_id = Some("test-hook-instance".to_owned());
        self.network_media_options_hook_configured_generation =
            Some(self.network_media_options_generation);
        self.network_media_options_hook_last_heartbeat_at = Some(Instant::now());
        self.network_media_options_hook_latest_started_load_sequence = Some(0);
    }

    #[cfg(test)]
    pub(crate) fn defer_test_network_options_hook_v3_transition(
        &mut self,
        load_sequence: u64,
        source_path: &str,
        stream_open_filename: &str,
        status: &str,
        error: Option<&str>,
    ) {
        let mut payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "hookInstanceId": "test-hook-instance",
            "loadSequence": load_sequence,
            "sourcePath": source_path,
            "streamOpenFilename": stream_open_filename,
            "status": status,
        });
        if let Some(error) = error {
            payload["error"] = Value::String(error.to_owned());
        }
        self.handle_network_options_hook_transition_result(Some(&payload.to_string()));
    }

    #[cfg(test)]
    pub(crate) fn defer_test_network_options_hook_verified_transition(
        &mut self,
        load_sequence: u64,
        source_path: &str,
        status: &str,
        option_results: &[(&str, &str)],
        effective_options: &[(&str, &str)],
    ) {
        let (wire_status, application_state) = match status {
            "network-updated" => ("network-updated", Some("applied")),
            "partially-applied" => ("failed", Some("partially-applied")),
            "failed" => ("failed", Some("failed")),
            other => (other, None),
        };
        let mut payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "hookInstanceId": "test-hook-instance",
            "loadSequence": load_sequence,
            "sourcePath": source_path,
            "streamOpenFilename": source_path,
            "status": wire_status,
            "verification": "complete",
            "optionResults": option_results
                .iter()
                .map(|(name, status)| json!({ "name": name, "status": status }))
                .collect::<Vec<_>>(),
            "effectiveOptions": effective_options
                .iter()
                .map(|(name, value)| ((*name).to_owned(), Value::String((*value).to_owned())))
                .collect::<serde_json::Map<_, _>>(),
        });
        if let Some(application_state) = application_state {
            payload["applicationState"] = Value::String(application_state.to_owned());
        }
        self.handle_network_options_hook_transition_result(Some(&payload.to_string()));
    }

    #[cfg(test)]
    pub(crate) fn defer_test_network_options_hook_v3_transition_for_instance(
        &mut self,
        hook_instance_id: &str,
        load_sequence: u64,
        source_path: &str,
        status: &str,
    ) {
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "hookInstanceId": hook_instance_id,
            "loadSequence": load_sequence,
            "sourcePath": source_path,
            "streamOpenFilename": source_path,
            "status": status,
        });
        self.handle_network_options_hook_transition_result(Some(&payload.to_string()));
    }

    #[cfg(test)]
    pub(crate) fn configure_test_network_options_hook_instance(
        &mut self,
        hook_instance_id: &str,
        current_load_sequence: u64,
    ) {
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "hookInstanceId": hook_instance_id,
            "currentLoadSequence": current_load_sequence,
            "status": "configured",
        });
        self.handle_network_options_hook_configured(Some(&payload.to_string()));
    }

    #[cfg(test)]
    pub(crate) fn configure_test_network_options_hook_instance_fields(
        &mut self,
        hook_instance_id: Option<&str>,
        current_load_sequence: Option<u64>,
    ) {
        let mut payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "status": "configured",
        });
        if let Some(hook_instance_id) = hook_instance_id {
            payload["hookInstanceId"] = Value::String(hook_instance_id.to_owned());
        }
        if let Some(current_load_sequence) = current_load_sequence {
            payload["currentLoadSequence"] = Value::from(current_load_sequence);
        }
        self.handle_network_options_hook_configured(Some(&payload.to_string()));
    }

    #[cfg(test)]
    pub(crate) fn invalidate_test_network_options_hook_delivery(&mut self) {
        self.invalidate_network_media_options_hook_delivery();
    }

    #[cfg(test)]
    pub(crate) fn flush_test_network_options_hook_v3_transition(&mut self) {
        self.flush_deferred_network_media_options_observation();
    }

    #[cfg(test)]
    pub(crate) fn begin_test_network_options_event_batch(&mut self) {
        self.network_media_options_event_batch_depth += 1;
    }

    #[cfg(test)]
    pub(crate) fn observe_test_network_options_path(&mut self, path: &str) {
        self.observe_authoritative_path_for_network_options(
            Some(path),
            AuthoritativePathObservationOrigin::PathEvent,
        );
    }

    #[cfg(test)]
    pub(crate) fn observe_test_network_options_pending_start(&mut self) {
        self.observe_authoritative_path_for_network_options(
            None,
            AuthoritativePathObservationOrigin::StartFilePending,
        );
    }

    #[cfg(test)]
    pub(crate) fn observe_test_network_options_null_path(&mut self) {
        self.observe_authoritative_path_for_network_options(
            None,
            AuthoritativePathObservationOrigin::PathEvent,
        );
    }

    #[cfg(test)]
    pub(crate) fn observe_test_network_options_terminal_end(&mut self) {
        self.observe_authoritative_path_for_network_options(
            None,
            AuthoritativePathObservationOrigin::EndFileIdle,
        );
    }

    #[cfg(test)]
    pub(crate) fn handle_test_network_options_start_file(&mut self, playlist_entry_id: u64) {
        self.handle_start_file_event(&json!({ "playlist_entry_id": playlist_entry_id }));
    }

    #[cfg(test)]
    pub(crate) fn handle_test_network_options_end_file(&mut self, playlist_entry_id: u64) {
        self.handle_end_file_event(&json!({
            "playlist_entry_id": playlist_entry_id,
            "reason": "eof",
        }));
    }

    #[cfg(test)]
    pub(crate) fn end_test_network_options_event_batch(&mut self) {
        self.network_media_options_event_batch_depth = self
            .network_media_options_event_batch_depth
            .saturating_sub(1);
        self.flush_deferred_network_media_options_observation();
    }

    #[cfg(test)]
    pub(crate) fn test_network_options_policy_source_path(&self) -> Option<&str> {
        self.network_media_options_apply_identity
            .as_ref()
            .map(|identity| identity.path.as_str())
    }

    #[cfg(test)]
    pub(crate) fn test_network_options_last_accepted_load_sequence(&self) -> Option<u64> {
        self.network_media_options_hook_last_accepted_load_sequence
    }

    #[cfg(test)]
    pub(crate) fn set_test_network_options_awaiting_authoritative_transition(
        &mut self,
        awaiting: bool,
    ) {
        self.set_network_media_policy_state(if awaiting {
            MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
        } else {
            MpvNetworkMediaPolicyState::Unknown
        });
    }

    #[cfg(test)]
    pub(crate) fn test_network_options_awaiting_authoritative_transition(&self) -> bool {
        matches!(
            self.network_media_options_policy_state,
            MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
        )
    }

    #[cfg(test)]
    pub(crate) fn force_test_legacy_syncplayintf_discovery_due(&mut self) {
        self.legacy_syncplayintf_last_discovery_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL);
        self.maintain_legacy_syncplayintf_lease();
    }

    #[cfg(test)]
    pub(crate) fn configure_test_bundled_sorotte_bridge_without_retry(
        &mut self,
    ) -> SorotteBridgeHealth {
        self.configure_bundled_sorotte_bridge_inner(Duration::ZERO)
    }

    #[cfg(test)]
    pub(crate) fn set_test_sorotte_bridge_owner_id(&mut self, owner_id: impl Into<String>) {
        self.legacy_syncplayintf_owner_id = owner_id.into();
    }

    #[cfg(test)]
    pub(crate) fn queue_test_pending_chat_request(&mut self, message: impl Into<String>) {
        self.pending_chat_requests.push_back(message.into());
    }

    #[cfg(test)]
    pub(crate) fn load_lifecycle_reacquisition_required_for_test(&self) -> bool {
        self.lifecycle_reconciliation_due
            || self.player_lifecycle.reconciliation_required
            || self.player_lifecycle.requires_authoritative_snapshot()
    }

    #[cfg(test)]
    pub(crate) fn force_load_lifecycle_reacquisition_due_for_test(&mut self) {
        self.lifecycle_reconciliation_due = false;
        self.reconcile_lifecycle_from_authority();
    }

    #[cfg(test)]
    pub(crate) fn inject_authoritative_playlist_snapshot_for_test(
        &mut self,
        entries: impl IntoIterator<Item = (i64, Option<String>, bool)>,
        current_path: Option<String>,
    ) {
        let attachment_epoch = self.lifecycle_epoch();
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch,
            entries: entries
                .into_iter()
                .map(|(id, original_filename, current)| {
                    AuthoritativePlaylistEntry::new(id, original_filename, current)
                })
                .collect(),
            current_path,
        });
        self.lifecycle_reconciliation_due = self.player_lifecycle.requires_authoritative_snapshot();
    }

    #[cfg(test)]
    pub(crate) fn has_load_transition_for_test(&self, generation: PlayerMediaGeneration) -> bool {
        self.player_lifecycle
            .load_attempts
            .values()
            .any(|attempt| attempt.media_generation == generation)
    }

    #[cfg(test)]
    pub(crate) fn pending_load_transition_generations_for_test(
        &self,
    ) -> Vec<PlayerMediaGeneration> {
        self.player_lifecycle
            .load_attempts
            .values()
            .filter(|attempt| !attempt.state.is_terminal())
            .map(|attempt| attempt.media_generation)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn observe_current_load_ready_for_test(&mut self, playlist_entry_id: i64) {
        self.active_playlist_entry_id = u64::try_from(playlist_entry_id).ok();
        self.handle_file_loaded_event();
        self.handle_playback_restart_event();
    }
}

// Pre-reducer load-registry and recovery fixtures were removed after their unique
// ownership, provisional-EOF, and watchdog cases were ported to reducer-backed suites.

#[cfg(test)]
mod lifecycle_transcript_capture_tests {
    use super::*;

    #[test]
    fn opt_in_capture_records_manual_responses_and_automatic_event_ingress() {
        let mut adapter = MpvAdapter::default();
        adapter.enable_lifecycle_transcript_capture();

        adapter
            .record_lifecycle_transcript_input(
                Some(PlayerCommandId::new(7)),
                None,
                json!({
                    "request_id": 7,
                    "error": "success",
                    "data": "https://media.invalid/video?token=private-value",
                }),
            )
            .expect("manual command response capture");
        adapter.handle_ipc_event(&json!({
            "event": "client-message",
            "playlist_entry_id": 19,
            "args": ["synthetic"],
        }));

        let transcript = adapter
            .take_lifecycle_transcript()
            .expect("enabled capture should return a transcript");
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript.records()[0].ingress_sequence, 1);
        assert_eq!(transcript.records()[1].ingress_sequence, 2);
        assert_eq!(
            transcript.records()[0].command_id,
            Some(PlayerCommandId::new(7))
        );
        assert_eq!(transcript.records()[1].playlist_entry_id, Some(19));
        assert!(
            !transcript
                .to_json_lines()
                .expect("transcript JSON")
                .contains("private-value")
        );
        assert!(adapter.take_lifecycle_transcript().is_none());
    }
}

#[cfg(test)]
mod version_policy_tests {
    use super::*;
    use std::{collections::VecDeque, io};

    #[derive(Debug)]
    struct VersionResponseTransport {
        responses: VecDeque<String>,
    }

    impl VersionResponseTransport {
        fn new(response: &str) -> Self {
            Self {
                responses: VecDeque::from([format!("{response}\n")]),
            }
        }

        fn new_many(responses: &[&str]) -> Self {
            Self {
                responses: responses
                    .iter()
                    .map(|response| format!("{response}\n"))
                    .collect(),
            }
        }
    }

    impl MpvJsonIpcTransport for VersionResponseTransport {
        fn send_line_until(&mut self, _line: &str, _deadline: Instant) -> io::Result<()> {
            Ok(())
        }

        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            let response = self.responses.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test response queue was empty",
                )
            })?;
            line.clear();
            line.push_str(&response);
            Ok(line.len())
        }
    }

    fn initialize_with_version_response(response: &str) -> (MpvAdapter, Result<(), PlayerError>) {
        let client = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new(response)));
        let mut adapter = MpvAdapter::default();
        let result = adapter.initialize_json_ipc_attachment(PathBuf::from("test-mpv-ipc"), client);
        (adapter, result)
    }

    fn operation_failure_message(result: Result<(), PlayerError>) -> String {
        let error = result.expect_err("version policy should reject this attachment");
        assert!(crate::is_unsupported_mpv_version_error(&error));
        match error {
            PlayerError::OperationFailed(message) => message,
            other => panic!("unexpected version-policy error: {other:?}"),
        }
    }

    #[test]
    fn json_ipc_initialization_accepts_minimum_and_newer_mpv_versions() {
        for reported in ["0.41.0", "mpv 0.41.1-UNKNOWN", "1.0.0"] {
            let response = format!(r#"{{"request_id":1,"error":"success","data":"{reported}"}}"#);
            let (adapter, result) = initialize_with_version_response(&response);

            result.unwrap_or_else(|error| panic!("{reported} should be supported: {error}"));
            assert!(adapter.ipc_client.is_some());
            assert_eq!(adapter.ipc_endpoint, Some(PathBuf::from("test-mpv-ipc")));
        }
    }

    #[test]
    fn rejected_replacement_preserves_the_existing_supported_attachment() {
        let (mut adapter, result) = initialize_with_version_response(
            r#"{"request_id":1,"error":"success","data":"0.41.0"}"#,
        );
        result.expect("the initial supported attachment should succeed");
        let replacement = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new(
            r#"{"request_id":1,"error":"success","data":"0.40.0"}"#,
        )));

        let error = adapter
            .initialize_json_ipc_attachment(PathBuf::from("unsupported-replacement"), replacement)
            .expect_err("an unsupported replacement must be rejected");

        assert!(crate::is_unsupported_mpv_version_error(&error));
        assert!(adapter.ipc_client.is_some());
        assert_eq!(adapter.ipc_endpoint, Some(PathBuf::from("test-mpv-ipc")));
    }

    #[test]
    fn supported_replacement_fences_old_commands_and_reuses_no_playlist_ownership() {
        let (mut adapter, result) = initialize_with_version_response(
            r#"{"request_id":1,"error":"success","data":"0.41.0"}"#,
        );
        result.expect("the initial supported attachment should succeed");
        let old_attachment = adapter.lifecycle_epoch();
        let old_generation = adapter.allocate_media_generation();
        adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: old_attachment,
            media_generation: old_generation,
            playlist_entry_id: 1,
            observed_target: "C:/old-core.mkv".to_owned(),
            file_loaded: true,
        });
        let old_attempt = adapter
            .player_lifecycle
            .active_load_attempt
            .expect("old core should have active reducer ownership");
        adapter.active_playlist_entry_id = Some(1);
        adapter.active_media_generation = Some(old_generation);
        adapter.current_path = Some("C:/old-core.mkv".to_owned());
        adapter.observed_state.path = adapter.current_path.clone();
        let old_command = adapter.register_tracked_command(
            Some(old_generation),
            TrackedCommandKind::Seek {
                target_seconds: 30.0,
                seeking_finished: false,
                position_in_tolerance: false,
            },
        );
        adapter.accept_tracked_command(old_command);
        adapter.pending_command_progress_updates.clear();
        adapter.pending_ordered_player_events.clear();

        let replacement = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new_many(&[
            r#"{"request_id":1,"error":"success","data":"0.41.1"}"#,
            r#"{"request_id":2,"error":"success","data":[{"id":1,"filename":"C:/new-core.mkv","current":true,"playing":true}]}"#,
            r#"{"request_id":3,"error":"success","data":"C:/new-core.mkv"}"#,
        ])));
        adapter
            .initialize_json_ipc_attachment(PathBuf::from("supported-replacement"), replacement)
            .expect("supported replacement should attach");
        adapter.reconcile_lifecycle_from_authority();

        assert_ne!(adapter.lifecycle_epoch(), old_attachment);
        let new_attempt = adapter
            .player_lifecycle
            .playlist_entry_attempts
            .get(&1)
            .copied()
            .expect("new core entry should receive new attachment ownership");
        assert_ne!(new_attempt, old_attempt);
        assert_eq!(
            adapter.player_lifecycle.active_load_attempt,
            Some(new_attempt)
        );
        assert_ne!(adapter.active_media_generation, Some(old_generation));
        assert_eq!(adapter.current_path.as_deref(), Some("C:/new-core.mkv"));
        assert!(
            adapter
                .pending_command_progress_updates
                .iter()
                .any(|progress| {
                    progress.command_id == old_command
                        && progress.state
                            == sorotte_player_api::PlayerCommandProgressState::Finished(
                                PlayerCommandResult::Failed(
                                    PlayerCommandFailureKind::TransportDisconnected,
                                ),
                            )
                })
        );
        assert!(
            adapter
                .pending_media_load_outcomes
                .iter()
                .all(|outcome| outcome.outcome.loaded_target.as_deref() != Some("C:/old-core.mkv"))
        );
    }

    #[test]
    fn json_ipc_initialization_rejects_mpv_older_than_0_41_0() {
        for reported in ["0.34.1", "0.40.99"] {
            let response = format!(r#"{{"request_id":1,"error":"success","data":"{reported}"}}"#);
            let (adapter, result) = initialize_with_version_response(&response);
            let message = operation_failure_message(result);

            assert!(message.contains("requires mpv 0.41.0 or newer"));
            assert!(message.contains(&format!("reports mpv {reported}")));
            assert!(message.contains("upgrade mpv"));
            assert!(adapter.ipc_client.is_none());
            assert!(adapter.ipc_endpoint.is_none());
        }
    }

    #[test]
    fn json_ipc_initialization_rejects_missing_or_unrecognized_versions() {
        let cases = [
            (
                r#"{"request_id":1,"error":"property unavailable"}"#,
                "does not expose the mpv-version property",
            ),
            (
                r#"{"request_id":1,"error":"success","data":null}"#,
                "did not report an mpv-version",
            ),
            (
                r#"{"request_id":1,"error":"success","data":"custom-build"}"#,
                "reported an unrecognized mpv-version",
            ),
            (
                r#"{"request_id":1,"error":"success","data":"0.41"}"#,
                "reported an unrecognized mpv-version",
            ),
        ];

        for (response, expected_reason) in cases {
            let (adapter, result) = initialize_with_version_response(response);
            let message = operation_failure_message(result);

            assert!(message.contains("requires mpv 0.41.0 or newer"));
            assert!(
                message.contains(expected_reason),
                "unexpected error: {message}"
            );
            assert!(adapter.ipc_client.is_none());
            assert!(adapter.ipc_endpoint.is_none());
        }
    }

    #[test]
    fn unsupported_version_predicate_does_not_match_unrelated_operation_failures() {
        assert!(!crate::is_unsupported_mpv_version_error(
            &PlayerError::OperationFailed("mpv IPC connection timed out".to_owned())
        ));
        assert_eq!(crate::MINIMUM_SUPPORTED_MPV_VERSION, "0.41.0");
    }

    #[test]
    fn protocol_failures_are_not_misclassified_as_version_rejections() {
        let (_adapter, result) = initialize_with_version_response("not-json");
        let error = result.expect_err("invalid IPC JSON must fail initialization");

        assert!(!crate::is_unsupported_mpv_version_error(&error));
        assert!(
            matches!(error, PlayerError::OperationFailed(message) if message.contains("invalid mpv IPC JSON"))
        );
    }
}

#[cfg(test)]
mod timeline_kind_tests {
    use super::*;

    fn loaded_adapter(path: &str, duration_seconds: Option<f64>) -> MpvAdapter {
        let generation = PlayerMediaGeneration::new(41);
        let mut adapter = MpvAdapter {
            active_file_loaded: true,
            active_media_generation: Some(generation),
            next_media_generation: 42,
            current_path: Some(path.to_owned()),
            path_metadata_generation: Some(generation),
            duration_metadata_generation: Some(generation),
            observed_state: MpvObservedState {
                path: Some(path.to_owned()),
                duration_seconds,
                seekable: Some(true),
                ..MpvObservedState::default()
            },
            ..MpvAdapter::default()
        };
        let attachment_epoch = adapter.lifecycle_epoch();
        adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation: generation,
            playlist_entry_id: 41,
            observed_target: path.to_owned(),
            file_loaded: true,
        });
        adapter.active_playlist_entry_id = Some(41);
        adapter.refresh_timeline_kind_from_metadata();
        adapter
    }

    fn observe_ytdl_is_live(adapter: &mut MpvAdapter, data: Value) {
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_YTDL_IS_LIVE,
            "data": data,
        }));
    }

    fn observe_full_metadata(adapter: &mut MpvAdapter, data: Value) {
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_METADATA,
            "data": data,
        }));
    }

    #[test]
    fn youtube_live_metadata_is_positive_sliding_timeline_evidence() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

        observe_ytdl_is_live(&mut adapter, json!("true"));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert_eq!(
            adapter.ytdl_is_live_metadata_generation,
            adapter.active_media_generation
        );
    }

    #[test]
    fn full_metadata_event_detects_youtube_live_media() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

        observe_full_metadata(
            &mut adapter,
            json!({ "title": "Live channel", "ytdl_is_live": "true" }),
        );

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(adapter.ytdl_is_live);
    }

    #[test]
    fn absent_or_false_live_metadata_keeps_durationless_network_media_unknown() {
        for data in [Value::Null, json!(false), json!("false")] {
            let mut adapter = loaded_adapter("https://media.invalid/unknown.m3u8", None);

            observe_ytdl_is_live(&mut adapter, data);

            assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
            assert!(!adapter.ytdl_is_live);
        }
    }

    #[test]
    fn positive_live_metadata_is_sticky_for_the_active_generation() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_full_metadata(&mut adapter, json!({ "ytdl_is_live": "true" }));

        observe_ytdl_is_live(&mut adapter, Value::Null);
        observe_ytdl_is_live(&mut adapter, json!("false"));
        observe_full_metadata(&mut adapter, json!({ "title": "metadata refresh" }));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(adapter.ytdl_is_live);

        let mut reverse_order = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut reverse_order, json!("true"));
        observe_full_metadata(&mut reverse_order, json!({ "ytdl_is_live": "false" }));
        assert_eq!(reverse_order.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(reverse_order.ytdl_is_live);
    }

    #[test]
    fn youtube_cache_stall_recovery_preserves_the_active_generation_and_live_timeline() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=characterization", None);
        let generation = adapter
            .active_media_generation
            .expect("the characterization fixture should have active media");
        adapter.active_generation_has_restarted = true;
        adapter.transport_phase = PlayerTransportPhase::Playing;
        observe_ytdl_is_live(&mut adapter, json!("true"));
        adapter.pending_transport_telemetry_updates.clear();

        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": "paused-for-cache",
            "data": true,
        }));
        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": "core-idle",
            "data": true,
        }));
        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": "demuxer-cache-state",
            "data": {
                "cache-duration": 0.0,
                "raw-input-rate": 0,
                "eof": false,
                "underrun": true,
            },
        }));

        assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Rebuffering);
        assert_eq!(adapter.active_media_generation, Some(generation));
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert_eq!(adapter.take_media_load_outcome(), None);

        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": "paused-for-cache",
            "data": false,
        }));
        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": "core-idle",
            "data": false,
        }));
        adapter.handle_ipc_event(&json!({ "event": "playback-restart" }));

        assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Playing);
        assert_eq!(adapter.active_media_generation, Some(generation));
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert_eq!(adapter.take_media_load_outcome(), None);
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .all(|update| update.media_generation == Some(generation))
        );
    }

    #[test]
    fn finite_duration_network_media_is_vod_without_positive_live_metadata() {
        let mut adapter = loaded_adapter("https://media.invalid/movie.m3u8", Some(120.0));
        observe_ytdl_is_live(&mut adapter, json!("false"));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
    }

    #[test]
    fn local_paths_and_file_urls_are_always_vod() {
        for path in ["C:/media/movie.mkv", "file:///C:/media/movie.mkv"] {
            let mut adapter = loaded_adapter(path, None);
            observe_ytdl_is_live(&mut adapter, json!("true"));

            assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
        }
    }

    #[test]
    fn new_generation_clears_live_evidence_and_rejects_stale_metadata() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));
        let previous_generation = adapter.active_media_generation;
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

        adapter.handle_start_file_event(&json!({ "playlist_entry_id": 42 }));
        let current_generation = adapter.active_media_generation;
        assert_ne!(current_generation, previous_generation);
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, None);

        adapter.active_file_loaded = true;
        adapter.current_path = Some("https://media.invalid/next.m3u8".to_owned());
        adapter.observed_state.path = adapter.current_path.clone();
        adapter.observed_state.duration_seconds = None;
        adapter.path_metadata_generation = current_generation;
        adapter.duration_metadata_generation = current_generation;
        adapter.ytdl_is_live = true;
        adapter.ytdl_is_live_metadata_generation = previous_generation;
        adapter.refresh_timeline_kind_from_metadata();

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
    }

    #[test]
    fn ending_the_active_generation_clears_live_evidence() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

        adapter.handle_end_file_event(&json!({ "reason": "eof", "playlist_entry_id": 41 }));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, None);
    }

    #[test]
    fn empty_cache_range_snapshot_clears_the_conservative_live_window() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));

        let populated = adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [{ "start": 80.0, "end": 100.0 }],
        }));
        assert_eq!(
            populated.known_live_seekable_window,
            Some(PlayerSeekableRange::new(80.0, 100.0))
        );

        let cleared = adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [],
        }));
        assert_eq!(cleared.seekable_ranges, Some(Vec::new()));
        assert_eq!(cleared.known_live_seekable_window, None);
        assert_eq!(adapter.latest_cached_seekable_window, None);
    }

    #[test]
    fn newer_cache_state_snapshot_clears_metrics_that_mpv_omits() {
        let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
        adapter.cache_state_telemetry_update(&json!({
            "cache-duration": 30.0,
            "fw-bytes": 157_286_400,
            "raw-input-rate": 4_000_000,
            "reader-pts": 42.0,
            "cache-end": 72.0,
            "eof": false,
            "underrun": true,
        }));
        adapter.cache_state_telemetry_update(&json!({}));
        let cleared = adapter
            .pending_cache_telemetry_updates
            .pop_back()
            .expect("the newer cache-state observation should be queued");

        assert!(cleared.media_generation.is_some());
        assert!(cleared.observed_at.is_some());
        assert_eq!(cleared.buffered_ahead_seconds, None);
        assert_eq!(cleared.buffered_ahead_bytes, None);
        assert_eq!(cleared.input_rate_bytes_per_second, None);
        assert_eq!(cleared.reader_position_seconds, None);
        assert_eq!(cleared.cache_end_seconds, None);
        assert_eq!(cleared.eof, None);
        assert_eq!(cleared.underrun, None);
    }

    #[test]
    fn authoritative_seek_event_emits_an_explicit_same_generation_cache_clear() {
        let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
        let generation = adapter.active_media_generation;
        adapter.cache_state_telemetry_update(&json!({
            "cache-duration": 30.0,
            "fw-bytes": 157_286_400,
            "raw-input-rate": 4_000_000,
            "reader-pts": 42.0,
            "cache-end": 72.0,
            "eof": false,
            "underrun": true,
        }));
        adapter.pending_cache_telemetry_updates.clear();
        adapter.pending_transport_telemetry_updates.clear();

        adapter.handle_seek_event();

        let cleared = adapter
            .pending_cache_telemetry_updates
            .pop_front()
            .expect("the production seek handler should queue a cache clear");
        assert_eq!(cleared.media_generation, generation);
        assert!(cleared.observed_at.is_some());
        assert_eq!(cleared.buffered_ahead_seconds, None);
        assert_eq!(cleared.buffered_ahead_bytes, None);
        assert_eq!(cleared.input_rate_bytes_per_second, None);
        assert_eq!(cleared.reader_position_seconds, None);
        assert_eq!(cleared.cache_end_seconds, None);
        assert_eq!(cleared.eof, None);
        assert_eq!(cleared.underrun, None);
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .any(|update| {
                    update.media_generation == generation
                        && update.phase == Some(PlayerTransportPhase::Seeking)
                        && update.seeking == Some(true)
                })
        );
    }

    #[test]
    fn new_media_generation_clears_all_cache_cap_diagnostics() {
        let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
        let first_generation = adapter.active_media_generation;
        adapter.cache_state_telemetry_update(&json!({
            "cache-duration": 30.0,
            "fw-bytes": 157_286_400,
            "raw-input-rate": 4_000_000,
            "reader-pts": 42.0,
            "cache-end": 72.0,
            "eof": false,
            "underrun": true,
        }));
        adapter.observed_state.demuxer_cache_idle = Some(true);
        adapter.observed_state.paused_for_cache = Some(false);
        let populated = adapter.network_media_diagnostic_snapshot();
        assert_eq!(populated.forward_bytes, Some(157_286_400));
        assert_eq!(populated.cache_underrun, Some(true));

        adapter.handle_start_file_event(&json!({ "playlist_entry_id": 99 }));

        let reset = adapter.network_media_diagnostic_snapshot();
        assert_ne!(reset.media_generation, first_generation);
        assert_eq!(reset.cache_duration_seconds, None);
        assert_eq!(reset.forward_bytes, None);
        assert_eq!(reset.raw_input_rate_bytes_per_second, None);
        assert_eq!(reset.reader_position_seconds, None);
        assert_eq!(reset.cache_end_seconds, None);
        assert_eq!(reset.cache_eof, None);
        assert_eq!(reset.cache_underrun, None);
        assert_eq!(reset.demuxer_cache_idle, None);
        assert_eq!(reset.paused_for_cache, None);
        assert_eq!(reset.observed_at, None);
    }
}

#[cfg(test)]
mod transport_queue_pressure_tests {
    use super::*;

    #[test]
    fn legacy_snapshot_survives_eviction_of_rich_position_and_pause_fields() {
        let generation = PlayerMediaGeneration::new(1);
        let mut adapter = MpvAdapter {
            active_media_generation: Some(generation),
            ..MpvAdapter::default()
        };
        adapter.queue_playback_telemetry_update(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.0)
                .with_paused(true),
        );
        adapter.queue_transport_telemetry_update(
            PlayerTransportTelemetryUpdate::default()
                .with_position_seconds(12.0)
                .with_logical_pause(true),
        );

        for index in 0..=MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES {
            let phase = if index % 2 == 0 {
                PlayerTransportPhase::Playing
            } else {
                PlayerTransportPhase::ReadyPaused
            };
            adapter.queue_transport_telemetry_update(
                PlayerTransportTelemetryUpdate::default().with_phase(phase),
            );
        }

        let mut rich_position_seen = false;
        let mut rich_pause_seen = false;
        while let Some(update) = adapter.take_transport_telemetry_update() {
            rich_position_seen |= update.position_seconds.is_some();
            rich_pause_seen |= update.logical_pause.is_some();
        }
        assert!(!rich_position_seen);
        assert!(!rich_pause_seen);
        let legacy = adapter
            .take_playback_telemetry_update()
            .expect("coalesced legacy telemetry remains available after rich queue pressure");
        assert_eq!(legacy.position_seconds, Some(12.0));
        assert_eq!(legacy.paused, Some(true));
    }
}

#[cfg(test)]
mod interrupted_network_stream_recovery_tests {
    use super::*;
    use crate::lifecycle::LoadAttemptState;
    use std::io;

    const NETWORK_PATH: &str = "https://media.example.invalid/premature-eof";

    #[derive(Debug, Default)]
    struct RejectingRecoveryTransport {
        response: Option<String>,
    }

    impl MpvJsonIpcTransport for RejectingRecoveryTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
            let request_id = request["request_id"]
                .as_u64()
                .ok_or_else(|| io::Error::other("missing request id"))?;
            self.response = Some(
                json!({
                    "request_id": request_id,
                    "error": "recovery load rejected",
                })
                .to_string()
                    + "\n",
            );
            Ok(())
        }

        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            let response = self
                .response
                .take()
                .ok_or_else(|| io::Error::other("missing recovery response"))?;
            line.clear();
            line.push_str(&response);
            Ok(line.len())
        }
    }

    fn loaded_network_vod(position_seconds: f64, duration_seconds: f64) -> MpvAdapter {
        let generation = PlayerMediaGeneration::new(7);
        let mut adapter = MpvAdapter {
            simulation_mode: true,
            current_path: Some(NETWORK_PATH.to_owned()),
            active_media_generation: Some(generation),
            next_media_generation: 8,
            active_playlist_entry_id: Some(10),
            transport_phase: PlayerTransportPhase::Playing,
            active_file_loaded: true,
            active_generation_has_restarted: true,
            timeline_kind: PlayerTimelineKind::Vod,
            path_metadata_generation: Some(generation),
            duration_metadata_generation: Some(generation),
            observed_state: MpvObservedState {
                path: Some(NETWORK_PATH.to_owned()),
                duration_seconds: Some(duration_seconds),
                position_seconds: Some(position_seconds),
                seekable: Some(true),
                eof_reached: Some(false),
                paused_for_cache: Some(false),
                ..MpvObservedState::default()
            },
            ..MpvAdapter::default()
        };
        let attachment_epoch = adapter.lifecycle_epoch();
        adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation: generation,
            playlist_entry_id: 10,
            observed_target: NETWORK_PATH.to_owned(),
            file_loaded: true,
        });
        adapter
    }

    #[test]
    fn premature_network_eof_uses_a_new_attempt_in_the_same_generation() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        adapter
            .network_media_options
            .insert("cache-secs".to_owned(), "30".to_owned());
        let generation = adapter
            .active_media_generation
            .expect("fixture should have an active generation");
        let old_attempt = adapter
            .player_lifecycle
            .active_load_attempt
            .expect("fixture should have an active attempt");
        let command =
            adapter.interrupted_network_stream_recovery_load_command(NETWORK_PATH, 257.25);
        assert_eq!(command[0], MPV_COMMAND_LOADFILE);
        assert_eq!(command[1], NETWORK_PATH);
        assert_eq!(command[4]["start"], "257.25");
        assert_eq!(command[4]["cache-secs"], "30");

        adapter.handle_end_file_event(&json!({
            "reason": "eof",
            "playlist_entry_id": 10,
        }));

        let recovery = adapter
            .interrupted_network_stream_recovery
            .expect("early EOF should create a bounded recovery attempt");
        assert_eq!(recovery.media_generation, generation);
        assert_ne!(recovery.latest_attempt_id, old_attempt);
        assert_eq!(recovery.resume_position_seconds, 257.25);
        assert_eq!(adapter.player_lifecycle.logical_terminal, None);
        assert!(matches!(
            adapter.player_lifecycle.load_attempts[&old_attempt].state,
            LoadAttemptState::Terminal(PlayerPhysicalLoadOutcome::Ended)
        ));
        assert_eq!(
            adapter.player_lifecycle.load_attempts[&recovery.latest_attempt_id].media_generation,
            generation
        );
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Loading);
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .any(|update| {
                    update.media_generation == Some(generation)
                        && update.phase == Some(PlayerTransportPhase::Loading)
                        && update.position_seconds == Some(257.25)
                        && update.eof_reached == Some(false)
                })
        );
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .all(|update| {
                    update.phase != Some(PlayerTransportPhase::Ended)
                        && update.eof_reached != Some(true)
                })
        );
    }

    #[test]
    fn near_tail_and_local_eof_are_not_reloaded() {
        let mut near_tail = loaded_network_vod(1_910.0, 1_919.0);
        near_tail.handle_end_file_event(&json!({
            "reason": "eof",
            "playlist_entry_id": 10,
        }));
        assert_eq!(near_tail.transport_phase, PlayerTransportPhase::Ended);
        assert_eq!(near_tail.network_stream_recovery_attempt_count(), 0);

        let mut local = loaded_network_vod(257.25, 1_919.0);
        local.current_path = Some("C:/media/movie.mkv".to_owned());
        local.observed_state.path = local.current_path.clone();
        local.handle_end_file_event(&json!({
            "reason": "eof",
            "playlist_entry_id": 10,
        }));
        assert_eq!(local.transport_phase, PlayerTransportPhase::Ended);
        assert_eq!(local.network_stream_recovery_attempt_count(), 0);
    }

    #[test]
    fn sustained_cache_stall_uses_the_same_bounded_recovery_transaction() {
        let mut adapter = loaded_network_vod(400.0, 1_919.0);
        let generation = adapter.active_media_generation;
        adapter.handle_ipc_event(&json!({
            "event": "property-change",
            "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
            "data": true,
        }));
        let delay = adapter.network_cache_stall_recovery_delay();
        adapter
            .network_cache_stall
            .as_mut()
            .expect("cache pause should arm the watchdog")
            .last_progress_at = Instant::now() - delay - Duration::from_millis(1);

        adapter.maintain_network_cache_stall_recovery();

        assert_eq!(adapter.active_media_generation, generation);
        assert_eq!(adapter.network_stream_recovery_attempt_count(), 1);
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Loading);
        assert_eq!(adapter.network_cache_stall, None);
    }

    #[test]
    fn rejected_recovery_preserves_old_physical_owner_and_total_budget() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        adapter.simulation_mode = false;
        adapter.ipc_client = Some(MpvJsonIpcClient::new(Box::new(
            RejectingRecoveryTransport::default(),
        )));
        let generation = adapter
            .active_media_generation
            .expect("fixture should have an active generation");
        let active_attempt = adapter
            .player_lifecycle
            .active_load_attempt
            .expect("fixture should have an active attempt");
        adapter.observed_state.eof_reached = Some(true);
        adapter.queue_transport_telemetry_update(
            adapter
                .transport_update_for(generation)
                .with_phase(PlayerTransportPhase::Ended),
        );
        adapter.queue_cache_telemetry_update(PlayerCacheTelemetryUpdate {
            media_generation: Some(generation),
            eof: Some(true),
            ..PlayerCacheTelemetryUpdate::default()
        });

        for attempt in 1..=MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS {
            assert!(!adapter.try_recover_interrupted_network_stream(generation));
            assert_eq!(adapter.network_stream_recovery_attempt_count(), attempt);
            assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
            assert_eq!(adapter.observed_state.eof_reached, Some(true));
            assert_eq!(adapter.pending_transport_telemetry_updates.len(), 1);
            assert_eq!(adapter.pending_cache_telemetry_updates.len(), 1);
            assert_eq!(
                adapter.player_lifecycle.active_load_attempt,
                Some(active_attempt)
            );
            assert_eq!(
                adapter.player_lifecycle.load_attempts[&active_attempt].superseded_by,
                None
            );
            if attempt < MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS {
                let advanced = 257.25 + attempt as f64 * 3.0;
                adapter.observe_interrupted_network_stream_recovery_progress(advanced);
                adapter.observed_state.position_seconds = Some(advanced);
            }
        }
        assert!(!adapter.try_recover_interrupted_network_stream(generation));
        assert_eq!(
            adapter.network_stream_recovery_attempt_count(),
            MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
        );
    }

    #[test]
    fn split_pump_eof_property_is_provisional_until_end_file_classifies_recovery() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        let generation = adapter
            .active_media_generation
            .expect("fixture should have an active generation");
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_EOF_REACHED,
            "data": true,
        }));

        assert!(adapter.player_lifecycle.provisional_eof_attempt().is_some());
        let provisional = adapter
            .take_ordered_event_batch()
            .expect("the provisional observation remains pump-visible");
        assert!(provisional.ordered_events.iter().all(|event| {
            !matches!(
                &event.kind,
                PlayerOrderedEventKind::Transport(update)
                    if update.media_generation == Some(generation)
                        && (matches!(
                            update.phase,
                            Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed)
                        ) || update.eof_reached == Some(true))
            )
        }));
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
        assert_eq!(
            adapter.observed_state.eof_reached,
            Some(true),
            "physical EOF remains internal evidence while the published delta stays non-terminal"
        );

        adapter.handle_end_file_event(&json!({
            "reason": "eof",
            "playlist_entry_id": 10,
        }));
        let recovered = adapter
            .take_ordered_event_batch()
            .expect("causal end-file should publish the recovery transition");
        assert!(recovered.ordered_events.iter().any(|event| matches!(
            &event.kind,
            PlayerOrderedEventKind::Transport(update)
                if update.media_generation == Some(generation)
                    && update.phase == Some(PlayerTransportPhase::Loading)
                    && update.eof_reached == Some(false)
        )));
        assert!(recovered.ordered_events.iter().all(|event| {
            !matches!(
                &event.kind,
                PlayerOrderedEventKind::Transport(update)
                    if update.media_generation == Some(generation)
                        && (matches!(
                            update.phase,
                            Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed)
                        ) || update.eof_reached == Some(true))
            )
        }));
    }

    #[test]
    fn progress_seek_and_restart_cancel_provisional_eof_without_a_terminal() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        let eof_event = json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_EOF_REACHED,
            "data": true,
        });

        adapter.handle_ipc_event(&eof_event);
        assert!(adapter.player_lifecycle.provisional_eof_attempt().is_some());
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_TIME_POS,
            "data": 258.0,
        }));
        assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);

        adapter.handle_ipc_event(&eof_event);
        adapter.handle_seek_event();
        assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);

        adapter.handle_ipc_event(&eof_event);
        adapter.handle_playback_restart_event();
        assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);
        assert_ne!(adapter.transport_phase, PlayerTransportPhase::Ended);
        assert_eq!(adapter.player_lifecycle.logical_terminal, None);
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .all(|update| {
                    update.phase != Some(PlayerTransportPhase::Ended)
                        && update.phase != Some(PlayerTransportPhase::Failed)
                        && update.eof_reached != Some(true)
                })
        );
    }

    #[test]
    fn cache_progress_and_configured_wait_restart_the_watchdog() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        adapter
            .network_media_options
            .insert("cache-pause-wait".to_owned(), "45".to_owned());
        assert_eq!(
            adapter.network_cache_stall_recovery_delay(),
            Duration::from_secs(50)
        );
        adapter.observed_state.paused_for_cache = Some(true);
        adapter.observed_state.buffered_ahead_bytes = Some(1_000);
        adapter.observed_state.cache_end_seconds = Some(258.0);
        adapter.observe_network_cache_pause_for_recovery(true);
        let old_progress = Instant::now() - Duration::from_secs(60);
        adapter
            .network_cache_stall
            .as_mut()
            .expect("rebuffering network VOD should arm recovery")
            .last_progress_at = old_progress;

        adapter.observed_state.buffered_ahead_bytes = Some(2_000);
        adapter.observed_state.cache_end_seconds = Some(259.0);
        adapter.maintain_network_cache_stall_recovery();

        let stall = adapter
            .network_cache_stall
            .expect("forward cache growth should keep the watchdog armed");
        assert!(stall.last_progress_at > old_progress);
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
        assert_eq!(adapter.network_stream_recovery_attempt_count(), 0);
    }

    #[test]
    fn first_usable_cache_sample_restarts_watchdog_instead_of_reloading() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        adapter.observed_state.paused_for_cache = Some(true);
        adapter.observe_network_cache_pause_for_recovery(true);
        let old_progress = Instant::now() - adapter.network_cache_stall_recovery_delay();
        adapter
            .network_cache_stall
            .as_mut()
            .expect("rebuffering network VOD should arm recovery")
            .last_progress_at = old_progress;

        adapter.observed_state.buffered_ahead_bytes = Some(1_000);
        adapter.observed_state.cache_end_seconds = Some(258.0);
        adapter.maintain_network_cache_stall_recovery();

        let stall = adapter
            .network_cache_stall
            .expect("the first positive cache sample should refresh the watchdog");
        assert!(stall.last_progress_at > old_progress);
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
        assert_eq!(adapter.network_stream_recovery_attempt_count(), 0);
    }

    #[test]
    fn cache_watchdog_ignores_initial_buffering_and_active_seek_and_disarms_on_release() {
        let mut adapter = loaded_network_vod(257.25, 1_919.0);
        adapter.active_generation_has_restarted = false;
        adapter.observed_state.paused_for_cache = Some(true);
        adapter.observe_network_cache_pause_for_recovery(true);
        assert!(adapter.network_cache_stall.is_none());

        adapter.active_generation_has_restarted = true;
        adapter.observed_state.seeking = Some(true);
        adapter.observe_network_cache_pause_for_recovery(true);
        assert!(adapter.network_cache_stall.is_none());

        adapter.observed_state.seeking = Some(false);
        adapter.observe_network_cache_pause_for_recovery(true);
        assert!(adapter.network_cache_stall.is_some());
        adapter.observed_state.paused_for_cache = Some(false);
        adapter.observe_network_cache_pause_for_recovery(false);
        assert!(adapter.network_cache_stall.is_none());
    }

    #[test]
    fn cache_stall_near_tail_can_recover_but_only_acceptance_disarms_it() {
        let mut near_tail = loaded_network_vod(1_918.0, 1_919.0);
        near_tail.observed_state.paused_for_cache = Some(true);
        near_tail.observe_network_cache_pause_for_recovery(true);
        near_tail
            .network_cache_stall
            .as_mut()
            .expect("near-tail cache stall should arm recovery")
            .last_progress_at = Instant::now() - near_tail.network_cache_stall_recovery_delay();

        near_tail.maintain_network_cache_stall_recovery();

        assert_eq!(near_tail.transport_phase, PlayerTransportPhase::Loading);
        assert_eq!(near_tail.network_stream_recovery_attempt_count(), 1);
        assert!(near_tail.network_cache_stall.is_none());

        let mut exhausted = loaded_network_vod(257.25, 1_919.0);
        let generation = exhausted
            .active_media_generation
            .expect("fixture should have an active generation");
        let active_attempt = exhausted
            .player_lifecycle
            .active_load_attempt
            .expect("fixture should have an active attempt");
        exhausted.observed_state.paused_for_cache = Some(true);
        exhausted.interrupted_network_stream_recovery = Some(InterruptedNetworkStreamRecovery {
            media_generation: generation,
            latest_attempt_id: active_attempt,
            resume_position_seconds: 257.25,
            consecutive_attempts: MAX_CONSECUTIVE_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS,
            total_attempts: MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS,
        });
        exhausted.observe_network_cache_pause_for_recovery(true);
        exhausted
            .network_cache_stall
            .as_mut()
            .expect("exhausted cache stall should remain represented")
            .last_progress_at = Instant::now() - exhausted.network_cache_stall_recovery_delay();

        exhausted.maintain_network_cache_stall_recovery();

        assert!(exhausted.network_cache_stall.is_some());
        assert_eq!(exhausted.transport_phase, PlayerTransportPhase::Playing);
    }
}
