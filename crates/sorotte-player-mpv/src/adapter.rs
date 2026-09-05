mod attachment;
mod bridge_settings;
use network_options::NetworkOptionsState;
use network_options::*;
use stream_recovery::StreamRecoveryState;
#[cfg(test)]
use stream_recovery::{InterruptedNetworkStreamRecovery, NetworkStreamRecoveryEvidence};
#[cfg(test)]
mod command_ack_tests;
mod network_options;
mod player_adapter;
mod reconnection;
mod state;
mod stream_recovery;
#[cfg(feature = "test-support")]
mod verification;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use verification::{
    LifecycleVerificationPlaylistEntry, LifecycleVerificationTrackedLoad,
    MpvLifecycleVerificationHarness,
};

#[cfg(feature = "test-support")]
use std::sync::{Arc, atomic::AtomicBool};
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
use sorotte_lifecycle_evidence::{
    Disposition, ProcessRole, TargetKind, TransitionObservation, Trigger, emit_global,
    global_enabled,
};
use sorotte_player_api::{
    LoadAttemptId, LocalFileUpdate, PlayerActiveLoadSnapshot, PlayerAdapter, PlayerAttachmentEpoch,
    PlayerAuthoritativeSnapshot, PlayerCacheTelemetryUpdate, PlayerCommandFailureKind,
    PlayerCommandId, PlayerCommandProgress, PlayerCommandResult, PlayerError, PlayerEventSequence,
    PlayerLoadAttemptResult, PlayerLocalFileObservation, PlayerMediaGeneration,
    PlayerMediaLoadFailureKind, PlayerMediaLoadObservation, PlayerMediaLoadOutcome,
    PlayerObservationBatch, PlayerObservationTimestamp, PlayerOrderedEvent, PlayerOrderedEventKind,
    PlayerPhysicalLoadOutcome, PlayerPlayIntent, PlayerPlaybackTelemetryUpdate,
    PlayerSeekableRange, PlayerSemanticOutcome, PlayerSequenceBoundary, PlayerTimelineKind,
    PlayerTransportDelta, PlayerTransportPhase, PlayerTransportSnapshot,
    PlayerTransportTelemetryUpdate, SnapshotField,
};
use sorotte_secret::SecretValue;

use self::state::MpvObservedState;
use crate::bridge::{SorotteBridgeFailure, SorotteBridgeFailureKind, SorotteBridgeHealth};
use crate::bridge_resource::{
    lease_bundled_sorotte_bridge, lease_bundled_sorotte_network_options_hook,
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
const PAUSED_POSITION_TELEMETRY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const IPC_EVENT_FENCE_ACTIVE_INTERVAL: Duration = Duration::from_millis(100);
const IPC_EVENT_FENCE_IDLE_INTERVAL: Duration = Duration::from_millis(500);
const IPC_RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingCachePauseReadback {
    ipc_command_id: Option<u64>,
    tracked_play_command_id: PlayerCommandId,
    attachment_epoch: PlayerAttachmentEpoch,
    attempt_id: LoadAttemptId,
    media_generation: PlayerMediaGeneration,
    dispatch_observation_sequence: u64,
    completed_value: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnacknowledgedMediaLoadOutcome {
    attempt_id: Option<LoadAttemptId>,
    observation: PlayerMediaLoadObservation,
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
            Self::Load { file_loaded, .. } => *file_loaded,
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

#[derive(Clone, Copy)]
struct DeferredStartFileObservation {
    attachment_epoch: PlayerAttachmentEpoch,
    playlist_entry_id: i64,
    playback_restart_sequence_at_observation: u64,
    playback_restart_observed_after_start: bool,
    retained_paused: Option<bool>,
    retained_logical_pause: Option<bool>,
    retained_playback_rate: Option<f64>,
    retained_core_idle: Option<bool>,
}

#[derive(Clone)]
struct DeferredFileLoadedObservation {
    attachment_epoch: PlayerAttachmentEpoch,
    playlist_entry_id: i64,
    loaded_target: Option<String>,
}

pub struct MpvAdapter {
    stream_recovery: StreamRecoveryState,
    network_options: NetworkOptionsState,
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
    pending_local_file_update: Option<LocalFileUpdate>,
    pending_local_file_generation: Option<PlayerMediaGeneration>,
    pending_local_file_observed_at: Option<PlayerObservationTimestamp>,
    pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
    pending_transport_telemetry_updates: VecDeque<PlayerTransportTelemetryUpdate>,
    pending_cache_telemetry_updates: VecDeque<PlayerCacheTelemetryUpdate>,
    pending_tracked_commands: VecDeque<PendingTrackedCommand>,
    last_finished_tracked_command_debug: Option<String>,
    pending_command_progress_updates: VecDeque<PlayerCommandProgress>,
    pending_media_load_outcomes: VecDeque<PlayerMediaLoadObservation>,
    next_ordered_player_event_sequence: u64,
    pending_ordered_player_events: VecDeque<PlayerOrderedEvent>,
    ordered_player_event_reacquisition_required: bool,
    ordered_player_event_reacquisition_requested_by_consumer: bool,
    last_delivered_ordered_command_progress: Vec<PlayerCommandProgress>,
    last_delivered_ordered_media_load_outcomes: Vec<PlayerMediaLoadObservation>,
    unacknowledged_terminal_command_progress: BTreeMap<PlayerCommandId, PlayerCommandProgress>,
    unacknowledged_media_load_outcomes: VecDeque<UnacknowledgedMediaLoadOutcome>,
    pending_chat_requests: VecDeque<String>,
    pending_load_request: Option<String>,
    pending_load_generation: Option<PlayerMediaGeneration>,
    last_polled_local_file_update: Option<LocalFileUpdate>,
    last_paused_position_poll_at: Option<Instant>,
    last_paused_position_telemetry_at: Option<Instant>,
    last_ipc_event_fence_at: Option<Instant>,
    pending_ipc_event_fence_command_id: Option<u64>,
    pending_cache_pause_readback: Option<PendingCachePauseReadback>,
    cache_pause_observation_sequence: u64,
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
    /// Physical attempt that owns the adapter identity projection.
    ///
    /// `active_media_generation`, `active_playlist_entry_id`,
    /// `current_path`, and `active_file_loaded` are updated atomically with
    /// this key at lifecycle ownership boundaries.
    active_load_attempt_id: Option<LoadAttemptId>,
    active_media_generation: Option<PlayerMediaGeneration>,
    active_playlist_entry_id: Option<u64>,
    latest_start_file_observation: Option<DeferredStartFileObservation>,
    deferred_start_file_observation: Option<DeferredStartFileObservation>,
    deferred_file_loaded_observation: Option<DeferredFileLoadedObservation>,
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
    ipc_reconnect_not_before: Option<Instant>,
    simulation_mode: bool,
    #[cfg(feature = "test-support")]
    test_simulated_natural_eof_trigger: Option<Arc<AtomicBool>>,
    ipc_client: Option<MpvJsonIpcClient>,
    pending_ipc_connection_events: VecDeque<MpvIpcConnectionEvent>,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedPlayerLifecycleTransition {
    transition: &'static str,
    machine: &'static str,
    trigger: Trigger,
    disposition: Disposition,
    identities: Vec<(&'static str, u64)>,
}

#[cfg(test)]
std::thread_local! {
    static CAPTURED_PLAYER_LIFECYCLE_TRANSITIONS:
        std::cell::RefCell<Option<Vec<CapturedPlayerLifecycleTransition>>> =
            const { std::cell::RefCell::new(None) };
}

fn emit_player_lifecycle_transition(
    transition: &'static str,
    machine: &'static str,
    trigger: Trigger,
    disposition: Disposition,
    identities: &[(&'static str, u64)],
) {
    #[cfg(test)]
    if CAPTURED_PLAYER_LIFECYCLE_TRANSITIONS.with(|capture| {
        let mut capture = capture.borrow_mut();
        let Some(transitions) = capture.as_mut() else {
            return false;
        };
        transitions.push(CapturedPlayerLifecycleTransition {
            transition,
            machine,
            trigger,
            disposition,
            identities: identities.to_vec(),
        });
        true
    }) {
        return;
    }
    let mut observation =
        TransitionObservation::new(ProcessRole::Player, "attached-player", machine, transition)
            .target(TargetKind::PlayerState)
            .triggered_by(trigger)
            .authority("player-pending", "player-observed")
            .effect("lifecycle-transition", "lifecycle-transition")
            .disposition(disposition);
    for (name, value) in identities.iter().copied().filter(|(_, value)| *value > 0) {
        observation = observation.identity(name, value);
    }
    let _ = emit_global(observation);
}

#[cfg(test)]
fn capture_player_lifecycle_transitions(
    action: impl FnOnce(),
) -> Vec<CapturedPlayerLifecycleTransition> {
    CAPTURED_PLAYER_LIFECYCLE_TRANSITIONS.with(|capture| {
        assert!(
            capture.replace(Some(Vec::new())).is_none(),
            "lifecycle evidence capture must not be nested"
        );
    });
    action();
    CAPTURED_PLAYER_LIFECYCLE_TRANSITIONS.with(|capture| {
        capture
            .borrow_mut()
            .take()
            .expect("lifecycle evidence capture should remain active")
    })
}

fn emit_player_lifecycle_input_evidence(
    input: &PlayerLifecycleInput,
    state_before: &PlayerLifecycleState,
    state_after: &PlayerLifecycleState,
) {
    if state_before == state_after {
        return;
    }
    let attachment_epoch = state_before.attachment_epoch.get();
    match input {
        PlayerLifecycleInput::LoadAttemptSubmitted {
            command_id,
            media_generation,
            ..
        } => {
            let mut identities = vec![
                ("attachment-epoch", attachment_epoch),
                ("media-generation", media_generation.get()),
            ];
            if let Some(command_id) = command_id {
                identities.push(("command-id", command_id.get()));
            }
            let predecessor = state_before.active_attempt();
            let recovery =
                predecessor.is_some_and(|attempt| attempt.media_generation == *media_generation);
            if let Some(attempt) = predecessor {
                identities.push(("predecessor-load-attempt-id", attempt.id.get()));
            }
            if let Some(successor) = state_after.load_attempts.values().find(|attempt| {
                !state_before.load_attempts.contains_key(&attempt.id)
                    && attempt.media_generation == *media_generation
                    && attempt.command_id == *command_id
            }) {
                identities.push(("load-attempt-id", successor.id.get()));
            }
            if let Some(attempt) = predecessor
                && !attempt.state.is_terminal()
                && !attempt.logical_ownership_revoked
            {
                emit_player_lifecycle_transition(
                    "LOAD-SUPERSEDE-001",
                    "load-attempt",
                    if recovery {
                        Trigger::Recovery
                    } else {
                        Trigger::LocalInput
                    },
                    Disposition::Superseded,
                    &identities,
                );
            }
            if recovery {
                if state_before.provisional_eof_attempt().is_some() {
                    emit_player_lifecycle_transition(
                        "TRANSPORT-EOF-CANCEL-001",
                        "local-transport",
                        Trigger::Recovery,
                        Disposition::Applied,
                        &identities,
                    );
                }
                emit_player_lifecycle_transition(
                    "TRANSPORT-FAIL-001",
                    "local-transport",
                    Trigger::Fault,
                    Disposition::Failed,
                    &identities,
                );
                emit_player_lifecycle_transition(
                    "LOAD-RECOVER-001",
                    "load-attempt",
                    Trigger::Recovery,
                    Disposition::Accepted,
                    &identities,
                );
                emit_player_lifecycle_transition(
                    "LOAD-RECOVERY-SUBMIT-001",
                    "load-attempt",
                    Trigger::Recovery,
                    Disposition::Submitted,
                    &identities,
                );
            } else {
                emit_player_lifecycle_transition(
                    "LOAD-SUBMIT-001",
                    "load-attempt",
                    Trigger::LocalInput,
                    Disposition::Submitted,
                    &identities,
                );
            }
            emit_player_lifecycle_transition(
                "TRANSPORT-LOAD-001",
                "local-transport",
                Trigger::LocalInput,
                Disposition::Submitted,
                &identities,
            );
        }
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation,
            playlist_entry_id,
            file_loaded,
            ..
        } => {
            let mut identities = vec![
                ("attachment-epoch", attachment_epoch.get()),
                ("media-generation", media_generation.get()),
            ];
            if let Ok(entry_id) = u64::try_from(*playlist_entry_id) {
                identities.push(("playlist-entry-id", entry_id));
            }
            emit_player_lifecycle_transition(
                if *file_loaded {
                    "LOAD-ACTIVE-001"
                } else {
                    "LOAD-START-001"
                },
                "load-attempt",
                Trigger::PlayerEvent,
                Disposition::Applied,
                &identities,
            );
            emit_player_lifecycle_transition(
                "TRANSPORT-LOAD-001",
                "local-transport",
                Trigger::PlayerEvent,
                Disposition::Observed,
                &identities,
            );
        }
        PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch,
            attempt_id,
        } => emit_player_lifecycle_transition(
            "LOAD-ACCEPT-001",
            "load-attempt",
            Trigger::PlayerEvent,
            Disposition::Accepted,
            &[
                ("attachment-epoch", attachment_epoch.get()),
                ("load-attempt-id", attempt_id.get()),
            ],
        ),
        PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch,
            attempt_id,
            ..
        } => emit_player_lifecycle_transition(
            "LOAD-TERMINAL-001",
            "load-attempt",
            Trigger::PlayerEvent,
            Disposition::Rejected,
            &[
                ("attachment-epoch", attachment_epoch.get()),
                ("load-attempt-id", attempt_id.get()),
            ],
        ),
        PlayerLifecycleInput::StartFile {
            attachment_epoch,
            playlist_entry_id,
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Ok(entry_id) = u64::try_from(*playlist_entry_id) {
                identities.push(("playlist-entry-id", entry_id));
            }
            emit_player_lifecycle_transition(
                "LOAD-BIND-001",
                "load-attempt",
                Trigger::PlayerEvent,
                Disposition::Applied,
                &identities,
            );
            emit_player_lifecycle_transition(
                "LOAD-START-001",
                "load-attempt",
                Trigger::PlayerEvent,
                Disposition::Observed,
                &identities,
            );
        }
        PlayerLifecycleInput::FileLoaded {
            attachment_epoch,
            playlist_entry_id,
            ..
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Some(entry_id) = playlist_entry_id.and_then(|id| u64::try_from(id).ok()) {
                identities.push(("playlist-entry-id", entry_id));
            }
            emit_player_lifecycle_transition(
                "LOAD-ACTIVE-001",
                "load-attempt",
                Trigger::PlayerEvent,
                Disposition::Applied,
                &identities,
            );
        }
        PlayerLifecycleInput::EndFile {
            attachment_epoch,
            playlist_entry_id,
            outcome,
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Ok(entry_id) = u64::try_from(*playlist_entry_id) {
                identities.push(("playlist-entry-id", entry_id));
            }
            emit_player_lifecycle_transition(
                "LOAD-TERMINAL-001",
                "load-attempt",
                Trigger::PlayerEvent,
                Disposition::Committed,
                &identities,
            );
            let (transition, disposition) = match outcome {
                PlayerPhysicalLoadOutcome::Ended => ("TRANSPORT-END-001", Disposition::Committed),
                PlayerPhysicalLoadOutcome::Failed(_)
                | PlayerPhysicalLoadOutcome::NeverStarted
                | PlayerPhysicalLoadOutcome::TransportDisconnected => {
                    ("TRANSPORT-FAIL-001", Disposition::Failed)
                }
            };
            emit_player_lifecycle_transition(
                transition,
                "local-transport",
                Trigger::PlayerEvent,
                disposition,
                &identities,
            );
        }
        PlayerLifecycleInput::EofObserved {
            attachment_epoch,
            playlist_entry_id,
            reached,
            ..
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Some(entry_id) = playlist_entry_id.and_then(|id| u64::try_from(id).ok()) {
                identities.push(("playlist-entry-id", entry_id));
            }
            emit_player_lifecycle_transition(
                if *reached {
                    "TRANSPORT-EOF-CANDIDATE-001"
                } else {
                    "TRANSPORT-EOF-CANCEL-001"
                },
                "local-transport",
                Trigger::PlayerEvent,
                Disposition::Observed,
                &identities,
            );
        }
        PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch,
            playlist_entry_id,
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Some(entry_id) = playlist_entry_id.and_then(|id| u64::try_from(id).ok()) {
                identities.push(("playlist-entry-id", entry_id));
            }
            if state_before.provisional_eof_attempt().is_some()
                && state_after.provisional_eof_attempt().is_none()
            {
                emit_player_lifecycle_transition(
                    "TRANSPORT-EOF-CANCEL-001",
                    "local-transport",
                    Trigger::PlayerEvent,
                    Disposition::Applied,
                    &identities,
                );
            }
            emit_player_lifecycle_transition(
                "TRANSPORT-PLAY-001",
                "local-transport",
                Trigger::PlayerEvent,
                Disposition::Observed,
                &identities,
            );
        }
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch,
            media_generation,
            ..
        } => {
            if state_before.provisional_eof_attempt().is_some()
                && state_after.provisional_eof_attempt().is_none()
            {
                emit_player_lifecycle_transition(
                    "TRANSPORT-EOF-CANCEL-001",
                    "local-transport",
                    Trigger::PlayerEvent,
                    Disposition::Applied,
                    &[
                        ("attachment-epoch", attachment_epoch.get()),
                        ("media-generation", media_generation.get()),
                    ],
                );
            }
        }
        PlayerLifecycleInput::SeekingObserved {
            attachment_epoch,
            media_generation,
            seeking: true,
            ..
        } => {
            let identities = [
                ("attachment-epoch", attachment_epoch.get()),
                ("media-generation", media_generation.get()),
            ];
            if state_before.provisional_eof_attempt().is_some()
                && state_after.provisional_eof_attempt().is_none()
            {
                emit_player_lifecycle_transition(
                    "TRANSPORT-EOF-CANCEL-001",
                    "local-transport",
                    Trigger::PlayerEvent,
                    Disposition::Applied,
                    &identities,
                );
            }
            emit_player_lifecycle_transition(
                "TRANSPORT-SEEK-001",
                "local-transport",
                Trigger::PlayerEvent,
                Disposition::Observed,
                &identities,
            );
        }
        PlayerLifecycleInput::PhaseObserved {
            attachment_epoch,
            phase,
        } => {
            let transition = match phase {
                PlayerTransportPhase::Empty => Some("TRANSPORT-DETACH-001"),
                PlayerTransportPhase::Loading | PlayerTransportPhase::Prebuffering => {
                    Some("TRANSPORT-LOAD-001")
                }
                PlayerTransportPhase::ReadyPaused => Some("TRANSPORT-PAUSE-001"),
                PlayerTransportPhase::Playing => Some("TRANSPORT-PLAY-001"),
                PlayerTransportPhase::Rebuffering => Some("TRANSPORT-CACHE-PAUSE-001"),
                PlayerTransportPhase::Seeking => Some("TRANSPORT-SEEK-001"),
                PlayerTransportPhase::Ended => Some("TRANSPORT-END-001"),
                PlayerTransportPhase::Failed => Some("TRANSPORT-FAIL-001"),
            };
            if let Some(transition) = transition {
                if state_before.provisional_eof_attempt().is_some()
                    && state_after.provisional_eof_attempt().is_none()
                {
                    emit_player_lifecycle_transition(
                        "TRANSPORT-EOF-CANCEL-001",
                        "local-transport",
                        Trigger::PlayerEvent,
                        Disposition::Applied,
                        &[("attachment-epoch", attachment_epoch.get())],
                    );
                }
                emit_player_lifecycle_transition(
                    transition,
                    "local-transport",
                    Trigger::PlayerEvent,
                    if *phase == PlayerTransportPhase::Failed {
                        Disposition::Failed
                    } else {
                        Disposition::Observed
                    },
                    &[("attachment-epoch", attachment_epoch.get())],
                );
            }
        }
        PlayerLifecycleInput::TransportDelta {
            attachment_epoch,
            delta,
        } => {
            let mut identities = vec![("attachment-epoch", attachment_epoch.get())];
            if let Some(load_attempt_id) = delta.load_attempt_id {
                identities.push(("load-attempt-id", load_attempt_id.get()));
            }
            if let Some(media_generation) = delta.media_generation {
                identities.push(("media-generation", media_generation.get()));
            }
            let transition = if delta.paused_for_cache == Some(true) {
                Some("TRANSPORT-CACHE-PAUSE-001")
            } else if delta.seeking == Some(true) {
                Some("TRANSPORT-SEEK-001")
            } else if delta.eof_reached == Some(true) {
                Some("TRANSPORT-EOF-CANDIDATE-001")
            } else if delta.logical_pause == Some(true) {
                Some("TRANSPORT-PAUSE-001")
            } else if delta.logical_pause == Some(false) {
                Some("TRANSPORT-PLAY-001")
            } else {
                None
            };
            if let Some(transition) = transition {
                emit_player_lifecycle_transition(
                    transition,
                    "local-transport",
                    Trigger::PlayerEvent,
                    Disposition::Observed,
                    &identities,
                );
            }
        }
        PlayerLifecycleInput::TransportDisconnected { attachment_epoch } => {
            emit_player_lifecycle_transition(
                "TRANSPORT-FAIL-001",
                "local-transport",
                Trigger::Fault,
                Disposition::Failed,
                &[("attachment-epoch", attachment_epoch.get())],
            );
            emit_player_lifecycle_transition(
                "PLAYER-LOSS-001",
                "player-attachment",
                Trigger::Fault,
                Disposition::Failed,
                &[("attachment-epoch", attachment_epoch.get())],
            );
        }
        PlayerLifecycleInput::AttachmentReplaced if attachment_epoch > 1 => {
            emit_player_lifecycle_transition(
                "TRANSPORT-DETACH-001",
                "local-transport",
                Trigger::Recovery,
                Disposition::Superseded,
                &[("attachment-epoch", attachment_epoch)],
            );
        }
        PlayerLifecycleInput::CommandSubmitted { .. }
        | PlayerLifecycleInput::CommandAccepted { .. }
        | PlayerLifecycleInput::CommandRejected { .. }
        | PlayerLifecycleInput::CommandSuperseded { .. }
        | PlayerLifecycleInput::CommandTransportDisconnected { .. }
        | PlayerLifecycleInput::CommandCompleted { .. }
        | PlayerLifecycleInput::CommandCompletionNotObserved { .. }
        | PlayerLifecycleInput::PlaylistSnapshot { .. }
        | PlayerLifecycleInput::LifecycleReconciliationFailed { .. }
        | PlayerLifecycleInput::SeekingObserved { seeking: false, .. }
        | PlayerLifecycleInput::LocalFileChanged { .. }
        | PlayerLifecycleInput::SeekCommandSubmitted { .. }
        | PlayerLifecycleInput::SeekCommandAccepted { .. }
        | PlayerLifecycleInput::SeekCommandRejected { .. }
        | PlayerLifecycleInput::SeekCommandCompletionNotObserved { .. }
        | PlayerLifecycleInput::EventGapDetected { .. }
        | PlayerLifecycleInput::AuthoritativeSnapshotApplied(_)
        | PlayerLifecycleInput::TimerAdvanced { .. }
        | PlayerLifecycleInput::AttachmentReplaced => {}
    }
}

impl MpvAdapter {
    /// Enables sanitized decoded mpv event capture for lifecycle debugging.
    ///
    /// This observes the adapter's event pump. Outgoing commands and
    /// synchronous command responses are intentionally outside this recorder.
    pub fn enable_lifecycle_transcript_capture(&mut self) {
        self.lifecycle_transcript_recorder = Some(MpvTranscriptRecorder::new());
        self.next_lifecycle_transcript_ingress_sequence = 1;
    }

    fn record_lifecycle_transcript_event(
        &mut self,
        raw_json: Value,
    ) -> Result<(), MpvTranscriptError> {
        let Some(recorder) = self.lifecycle_transcript_recorder.as_mut() else {
            return Ok(());
        };
        let command_id = raw_json
            .get("request_id")
            .and_then(Value::as_u64)
            .map(PlayerCommandId::new);
        let playlist_entry_id = raw_json.get("playlist_entry_id").and_then(Value::as_i64);
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

    /// Installs a deterministic external trigger for a natural EOF on the
    /// simulated adapter. The ordinary maintenance pump consumes the trigger,
    /// so higher-layer tests exercise the same ordered lifecycle delivery as
    /// real mpv without adding a test-only command to the client protocol.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn set_test_simulated_natural_eof_trigger(&mut self, trigger: Arc<AtomicBool>) {
        debug_assert!(self.simulation_mode);
        self.test_simulated_natural_eof_trigger = Some(trigger);
    }

    #[cfg(feature = "test-support")]
    fn maintain_test_simulated_natural_eof_trigger(&mut self) {
        let triggered = self
            .test_simulated_natural_eof_trigger
            .as_ref()
            .is_some_and(|trigger| trigger.swap(false, Ordering::SeqCst));
        if !triggered || !self.simulation_mode {
            return;
        }
        let Some(playlist_entry_id) = self.active_playlist_entry_id else {
            return;
        };
        self.handle_end_file_event(&json!({
            "reason": "eof",
            "playlist_entry_id": playlist_entry_id,
        }));
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
        self.network_options.network_media_options_application_state =
            Some(MpvNetworkMediaPolicyApplicationState::Applied);
        self.network_options
            .network_media_options_diagnostic_load_sequence = Some(load_sequence);
        self.network_options
            .network_media_options_verification_complete = true;
        self.network_options.network_media_options_option_results = self
            .network_options
            .network_media_options
            .keys()
            .map(|name| MpvNetworkOptionApplyResult {
                name: name.clone(),
                status: MpvNetworkOptionApplyStatus::Applied,
            })
            .collect();
        self.network_options
            .network_media_options_effective_cache_options =
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
        let active = self
            .player_lifecycle
            .active_attempt()
            .cloned()
            .expect("test external load should establish a physical owner");
        self.install_physical_projection(
            active.id,
            active.media_generation,
            Some(playlist_entry_id),
            Some(target.to_owned()),
            true,
        );
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

    /// Advances the bounded maintenance required by Sorotte-owned mpv integrations.
    ///
    /// Runtime owners should call this from their regular tick even when they are not currently
    /// consuming playback telemetry. Every adapter observation getter also invokes it so an
    /// active transport-only or command-only consumer keeps hook leases alive.
    pub fn maintain_runtime_integrations(&mut self) {
        #[cfg(feature = "test-support")]
        self.maintain_test_simulated_natural_eof_trigger();
        self.drain_ipc_events_if_attached();
        self.maintain_json_ipc_reconnection_using(Instant::now(), MpvJsonIpcClient::connect);
        // Every delivery mode shares this pump. Let already-buffered observations complete
        // commands before applying their semantic deadline, then expire anything still pending.
        self.expire_tracked_commands();
        self.recover_network_media_options_hook_ownership_if_needed();
        self.maintain_network_cache_stall_recovery();
        self.maintain_network_media_options_hook_lease();
        self.maintain_legacy_syncplayintf_lease();
        self.maintain_cache_pause_readback_nonblocking();
        self.maintain_ipc_event_fence_nonblocking();
        // Synchronous heartbeat commands can themselves harvest a bounded batch of events. Flush
        // their control faults and ordinary observations. The nonblocking event fence above also
        // gives the worker a bounded opportunity to harvest property/lifecycle events that mpv
        // emitted just after an earlier command response.
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
                        pending.observation.media_generation == observation.media_generation
                            && pending.observation.outcome == observation.outcome
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
        let attempt_id = media_generation.and_then(|media_generation| {
            self.player_lifecycle
                .load_attempts
                .values()
                .rev()
                .find(|attempt| {
                    attempt.media_generation == media_generation
                        && attempt.requested_target == outcome.requested_target
                        && attempt.semantic_load_result.is_some()
                })
                .map(|attempt| attempt.id)
        });
        let observation = PlayerMediaLoadObservation::new(
            outcome,
            media_generation,
            Some(self.observation_timestamp()),
        );
        self.queue_media_load_observation(attempt_id, observation);
    }

    fn queue_media_load_observation(
        &mut self,
        attempt_id: Option<LoadAttemptId>,
        observation: PlayerMediaLoadObservation,
    ) {
        self.queue_ordered_player_event(PlayerOrderedEventKind::MediaLoad(observation.clone()));
        if self.pending_media_load_outcomes.len() >= MAX_UNACKNOWLEDGED_MEDIA_LOAD_OUTCOMES {
            self.pending_media_load_outcomes.pop_front();
        }
        self.pending_media_load_outcomes
            .push_back(observation.clone());
        if !self
            .unacknowledged_media_load_outcomes
            .iter()
            .any(|pending| pending.attempt_id == attempt_id && pending.observation == observation)
        {
            if self.unacknowledged_media_load_outcomes.len()
                >= MAX_UNACKNOWLEDGED_MEDIA_LOAD_OUTCOMES
            {
                self.unacknowledged_media_load_outcomes.pop_front();
            }
            self.unacknowledged_media_load_outcomes
                .push_back(UnacknowledgedMediaLoadOutcome {
                    attempt_id,
                    observation,
                });
        }
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

    /// Services only nonblocking lease/event work and returns the oldest player-chat request.
    /// This is the async-owner counterpart to [`PlayerAdapter::take_pending_chat_request`].
    pub fn take_pending_chat_request_nonblocking(&mut self) -> Option<String> {
        PlayerAdapter::maintain_runtime_leases_nonblocking(self);
        self.pending_chat_requests.pop_front()
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
                if let Some(attempt_id) = self.active_load_attempt_id {
                    self.update_physical_projection_path(
                        attempt_id,
                        authoritative_update.path.clone(),
                    );
                }
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
        if let Some(attempt_id) = self.active_load_attempt_id {
            self.update_physical_projection_path(attempt_id, polled_update.path.clone());
        }
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
                self.network_options
                    .deferred_network_media_options_observation = None;
                return Err(error);
            }
        };
        if !observed_interleaved_events {
            self.network_options
                .deferred_network_media_options_observation = None;
            return Ok(initial_update);
        }

        let final_path = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                self.network_options
                    .deferred_network_media_options_observation = None;
                return Ok(None);
            };
            ipc_client.get_property_string(MPV_PROPERTY_PATH)
        };
        // Direct events collected before this response are older than it. A handler can,
        // however, issue its own nested poll while reducing those events; that nested Poll
        // observation completes afterward and must outrank this captured response.
        self.network_options
            .deferred_network_media_options_observation = None;
        self.drain_ipc_events_without_network_options_flush();
        let nested_poll_observation = self
            .network_options
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
        if !position_seconds.is_finite() {
            return;
        }

        let position_changed = self
            .observed_state
            .position_seconds
            .is_none_or(|observed| (observed - position_seconds).abs() >= 1e-6);
        let telemetry_heartbeat_due =
            self.last_paused_position_telemetry_at
                .is_none_or(|last_observed| {
                    now.saturating_duration_since(last_observed)
                        >= PAUSED_POSITION_TELEMETRY_HEARTBEAT_INTERVAL
                });
        if !position_changed && !telemetry_heartbeat_due {
            return;
        }
        self.last_paused_position_telemetry_at = Some(now);

        let update = self
            .transport_update()
            .with_position_seconds(position_seconds);
        self.queue_transport_telemetry_update(update);
        if !position_changed {
            // A successful read is fresh transport evidence even when a paused
            // position is numerically unchanged. Keep that liveness signal at
            // heartbeat cadence without flooding the ordered event stream at
            // the 100 ms out-of-band-seek polling cadence.
            return;
        }

        self.position_seconds = position_seconds;
        self.observed_state.position_seconds = Some(position_seconds);
        self.observe_interrupted_network_stream_recovery_progress(position_seconds);
        self.queue_playback_telemetry_update(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(position_seconds),
        );
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
        #[cfg(test)]
        if !self.player_lifecycle.load_attempts.values().any(|attempt| {
            attempt.media_generation == generation
                && !attempt.state.is_terminal()
                && attempt.playlist_entry_id.is_some()
        }) {
            let test_entry_id = self
                .latest_start_file_observation
                .filter(|observation| observation.attachment_epoch == self.lifecycle_epoch())
                .map(|observation| observation.playlist_entry_id)
                .or_else(|| {
                    self.player_lifecycle
                        .load_attempts
                        .values()
                        .find(|attempt| {
                            attempt.media_generation == generation
                                && !attempt.state.is_terminal()
                                && attempt.playlist_entry_id.is_none()
                                && Self::media_target_matches(
                                    &requested_target,
                                    &attempt.requested_target,
                                )
                        })
                        .and_then(|attempt| i64::try_from(attempt.id.get()).ok())
                });
            if let Some(test_entry_id) = test_entry_id {
                let _ = self.bind_single_pending_test_load(test_entry_id);
            }
        }
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
        let projection_attempt = self
            .player_lifecycle
            .load_attempts
            .values()
            .find(|attempt| {
                attempt.media_generation == generation
                    && !attempt.state.is_terminal()
                    && attempt.playlist_entry_id.is_some()
            })
            .cloned();
        let Some(projection_attempt) = projection_attempt else {
            self.pending_load_request = Some(requested_target);
            self.pending_load_generation = Some(generation);
            self.lifecycle_reconciliation_due = true;
            return false;
        };
        self.install_physical_projection(
            projection_attempt.id,
            generation,
            projection_attempt.playlist_entry_id,
            polled_update.path.clone(),
            true,
        );
        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.path_metadata_generation = Some(generation);
        self.duration_metadata_generation = Some(generation);
        self.refresh_timeline_kind_from_metadata();
        // A coherent metadata poll can win the race with mpv's queued
        // `file-loaded` event. This path already commits the same lifecycle
        // boundary above, so publish the corresponding tracker evidence now.
        // Otherwise the later event is correctly deduplicated by lifecycle
        // state but can never finish the still-pending tracked load.
        self.observe_tracked_commands(Some(generation), TrackedCommandObservation::FileLoaded);
        self.observe_tracked_commands(
            Some(generation),
            TrackedCommandObservation::Phase(self.inferred_transport_phase()),
        );
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

    fn install_physical_projection(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        playlist_entry_id: Option<i64>,
        current_path: Option<String>,
        file_loaded: bool,
    ) {
        self.active_load_attempt_id = Some(attempt_id);
        self.active_media_generation = Some(media_generation);
        self.active_playlist_entry_id =
            playlist_entry_id.and_then(|entry_id| u64::try_from(entry_id).ok());
        self.current_path = current_path;
        self.active_file_loaded = file_loaded;
        debug_assert!(self.physical_projection_is_coherent());
    }

    fn clear_physical_projection(&mut self) {
        self.active_load_attempt_id = None;
        self.active_media_generation = None;
        self.active_playlist_entry_id = None;
        self.current_path = None;
        self.active_file_loaded = false;
        debug_assert!(self.physical_projection_is_coherent());
    }

    fn update_physical_projection_path(
        &mut self,
        attempt_id: LoadAttemptId,
        current_path: Option<String>,
    ) -> bool {
        if self.active_load_attempt_id != Some(attempt_id) {
            return false;
        }
        self.current_path = current_path;
        true
    }

    fn physical_projection_is_coherent(&self) -> bool {
        match (
            self.active_load_attempt_id,
            self.active_media_generation,
            self.active_playlist_entry_id,
            self.active_file_loaded,
        ) {
            (None, None, None, false) => self.current_path.is_none(),
            (Some(attempt_id), Some(media_generation), playlist_entry_id, _) => self
                .player_lifecycle
                .load_attempts
                .get(&attempt_id)
                .is_some_and(|attempt| {
                    attempt.media_generation == media_generation
                        && attempt
                            .playlist_entry_id
                            .and_then(|entry_id| u64::try_from(entry_id).ok())
                            == playlist_entry_id
                }),
            _ => false,
        }
    }

    fn apply_lifecycle_input(&mut self, input: PlayerLifecycleInput) -> Vec<PlayerLifecycleEffect> {
        let evidence = global_enabled().then(|| (input.clone(), self.player_lifecycle.clone()));
        let mut effects = Vec::new();
        if matches!(&input, PlayerLifecycleInput::LoadAttemptAccepted { .. }) {
            let now_tick = u64::try_from(self.observation_clock_origin.elapsed().as_millis())
                .unwrap_or(u64::MAX);
            let current = std::mem::take(&mut self.player_lifecycle);
            let (next, timer_effects) =
                reduce_player_lifecycle(current, PlayerLifecycleInput::TimerAdvanced { now_tick });
            self.player_lifecycle = next;
            effects.extend(timer_effects);
        }
        let current = std::mem::take(&mut self.player_lifecycle);
        let (next, input_effects) = reduce_player_lifecycle(current, input);
        self.player_lifecycle = next;
        effects.extend(input_effects);
        let newly_quiescent_attempts =
            effects
                .iter()
                .filter_map(|effect| match effect {
                    PlayerLifecycleEffect::EmitSemanticOutcome(outcome) => match &outcome.outcome {
                        PlayerSemanticOutcome::LoadAttempt(load)
                            if load.result == PlayerLoadAttemptResult::Indeterminate
                                && self
                                    .player_lifecycle
                                    .load_attempts
                                    .get(&load.attempt_id)
                                    .is_some_and(|attempt| {
                                        matches!(
                                        attempt.state,
                                        crate::lifecycle::LoadAttemptState::MayStillEmitQuiescent {
                                            ..
                                        }
                                    )
                                    }) =>
                        {
                            Some((load.attempt_id, load.media_generation))
                        }
                        PlayerSemanticOutcome::Command(_)
                        | PlayerSemanticOutcome::LoadAttempt(_) => None,
                    },
                    _ => None,
                })
                .collect::<Vec<_>>();
        for (attempt_id, media_generation) in newly_quiescent_attempts {
            self.clear_adapter_pending_load_after_quiescence(attempt_id, media_generation);
        }
        if effects.iter().any(|effect| {
            matches!(
                effect,
                PlayerLifecycleEffect::RequestLifecycleReconciliation
            )
        }) {
            self.lifecycle_reconciliation_due = true;
        }
        if let Some((input, state_before)) = evidence {
            emit_player_lifecycle_input_evidence(&input, &state_before, &self.player_lifecycle);
        }
        effects
    }

    fn clear_adapter_pending_load_after_quiescence(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) {
        let pending_load_was_generation = self.pending_load_generation == Some(media_generation);
        if pending_load_was_generation {
            self.pending_load_request = None;
            self.pending_load_generation = None;
            if self
                .network_options
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| embedded.media_generation == media_generation)
            {
                self.network_options.network_media_options_embedded_load = None;
            }
        }

        if self
            .stream_recovery
            .interrupted_network_stream_recovery
            .is_some_and(|recovery| recovery.latest_attempt_id == attempt_id)
        {
            self.stream_recovery.interrupted_network_stream_recovery = None;
            self.stream_recovery.network_stream_recovery_evidence = None;
            self.stream_recovery.network_cache_stall = None;
        }

        if self.active_load_attempt_id == Some(attempt_id)
            || (pending_load_was_generation && self.active_load_attempt_id.is_none())
        {
            self.transport_phase = PlayerTransportPhase::Empty;
            self.clear_physical_projection();
            self.paused = false;
            self.logical_pause_explicit = false;
            self.position_seconds = 0.0;
            self.playback_rate = 0.0;
            self.paused_for_cache = false;
            self.cache_buffering_percent = None;
            self.observed_state.path = None;
            self.observed_state.duration_seconds = None;
            self.observed_state.size_bytes = None;
            self.observed_state.position_seconds = None;
            self.observed_state.paused_for_cache = None;
            self.observed_state.cache_buffering_percent = None;
            self.observed_state.seeking = None;
            self.observed_state.seekable = None;
            self.observed_state.seekable_ranges = None;
            self.observed_state.eof_reached = None;
            self.reset_timeline_metadata();
            self.queue_cache_telemetry_update(
                self.cleared_cache_telemetry_update(Some(media_generation)),
            );
            let update = self
                .transport_update_for(media_generation)
                .with_phase(PlayerTransportPhase::Empty);
            self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        }

        // A semantic deadline is also an authority boundary: the physical
        // player may still contain the predecessor, be empty, or have admitted
        // the late successor without delivering start-file.
        self.lifecycle_reconciliation_due = true;
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
        let media_generation = self
            .player_lifecycle
            .load_attempts
            .get(attempt_id)
            .map(|attempt| attempt.media_generation);
        if let Some(media_generation) = media_generation {
            self.install_physical_projection(
                *attempt_id,
                media_generation,
                Some(playlist_entry_id),
                None,
                false,
            );
        }
        true
    }

    #[cfg_attr(test, allow(dead_code))]
    fn read_authoritative_property_at_response_boundary(
        &mut self,
        attachment_epoch: PlayerAttachmentEpoch,
        property_name: &str,
    ) -> Result<Option<Value>, ()> {
        self.read_authoritative_property_at_response_boundary_with_network_options_flush(
            attachment_epoch,
            property_name,
            true,
        )
    }

    fn read_authoritative_identity_property_at_response_boundary(
        &mut self,
        attachment_epoch: PlayerAttachmentEpoch,
        property_name: &str,
    ) -> Result<Option<Value>, ()> {
        self.read_authoritative_property_at_response_boundary_with_network_options_flush(
            attachment_epoch,
            property_name,
            false,
        )
    }

    #[cfg_attr(test, allow(dead_code))]
    fn reconcile_lifecycle_from_authority(&mut self) {
        let epoch = self.lifecycle_epoch();
        if self
            .ipc_client
            .as_ref()
            .is_none_or(|client| !client.is_healthy())
        {
            self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                attachment_epoch: epoch,
            });
            self.observe_unhealthy_ipc_transport();
            return;
        }
        let playlist = match self
            .read_authoritative_identity_property_at_response_boundary(epoch, MPV_PROPERTY_PLAYLIST)
        {
            Ok(Some(playlist)) => playlist,
            Ok(None) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                    attachment_epoch: epoch,
                });
                return;
            }
            Err(()) => return,
        };
        let path = match self
            .read_authoritative_identity_property_at_response_boundary(epoch, MPV_PROPERTY_PATH)
        {
            Ok(path) => path.and_then(|value| value.as_str().map(ToOwned::to_owned)),
            Err(()) => return,
        };
        let entries = Self::authoritative_playlist_entries(&playlist);
        let authoritative_current_entry_id = entries
            .iter()
            .find(|entry| entry.current)
            .map(|entry| entry.id);
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch: epoch,
            entries: entries.clone(),
            current_path: path.clone(),
        });
        self.observe_external_current_from_authority(&entries, path.as_deref());
        // Reapply identity-dependent ingress only after the authoritative
        // playlist has bound the physical attempt. Network-policy results
        // observed before either identity response remain deferred until this
        // point so start-file initialization cannot erase their verification.
        self.replay_deferred_start_file_if_bound();
        self.replay_deferred_file_loaded_if_bound();
        self.observed_state.path = path.clone();
        self.flush_deferred_network_media_options_for_authoritative_path(path.as_deref());
        // Events harvested after the path response are newer than the
        // authoritative identity pair and may now supersede it normally.
        self.drain_ipc_events_if_attached();

        let paused = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_PAUSE)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.paused = paused;
        if paused == Some(false) {
            self.logical_pause_explicit = false;
        }

        let position_seconds = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_TIME_POS)
        {
            Ok(value) => value.as_ref().and_then(Value::as_f64),
            Err(()) => return,
        };
        self.observed_state.position_seconds = position_seconds;

        let playback_rate = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_SPEED)
        {
            Ok(value) => value.as_ref().and_then(Value::as_f64),
            Err(()) => return,
        };
        self.observed_state.playback_rate = playback_rate;

        let paused_for_cache = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_PAUSED_FOR_CACHE)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.paused_for_cache = paused_for_cache;
        self.observed_state.logical_pause = self.observed_state.paused.map(|paused| {
            paused && (paused_for_cache != Some(true) || self.logical_pause_explicit)
        });

        let cache_buffering_percent = match self.read_authoritative_property_at_response_boundary(
            epoch,
            MPV_PROPERTY_CACHE_BUFFERING_STATE,
        ) {
            Ok(value) => value.as_ref().and_then(Value::as_f64),
            Err(()) => return,
        };
        self.observed_state.cache_buffering_percent = cache_buffering_percent;

        let core_idle = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_CORE_IDLE)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.core_idle = core_idle;

        let seeking = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_SEEKING)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.seeking =
            seeking.map(|seeking| self.normalize_transport_seeking_observation(seeking));

        let seekable = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_SEEKABLE)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.seekable = seekable;

        let demuxer_cache_idle = match self.read_authoritative_property_at_response_boundary(
            epoch,
            MPV_PROPERTY_DEMUXER_CACHE_IDLE,
        ) {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.demuxer_cache_idle = demuxer_cache_idle;

        let eof_reached = match self
            .read_authoritative_property_at_response_boundary(epoch, MPV_PROPERTY_EOF_REACHED)
        {
            Ok(value) => value.as_ref().and_then(Value::as_bool),
            Err(()) => return,
        };
        self.observed_state.eof_reached = eof_reached;
        self.refresh_network_stream_recovery_evidence();
        // File-loaded is the semantic load boundary, while readiness is a
        // transport observation. Every property above was applied at its own
        // response boundary, so later-query events remain authoritative.
        self.publish_reconciled_transport_state(authoritative_current_entry_id);
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

    fn observe_external_current_from_authority(
        &mut self,
        entries: &[AuthoritativePlaylistEntry],
        current_path: Option<&str>,
    ) {
        let Some(current) = entries.iter().find(|entry| entry.current) else {
            return;
        };
        if self
            .player_lifecycle
            .attempt_for_playlist_entry(current.id)
            .is_some()
        {
            return;
        }
        let target = current
            .original_filename
            .clone()
            .or_else(|| current_path.map(ToOwned::to_owned))
            .unwrap_or_default();
        let could_be_pending = self.player_lifecycle.load_attempts.values().any(|attempt| {
            !attempt.state.is_terminal()
                && attempt.playlist_entry_id.is_none()
                && (attempt.requested_target == target
                    || current.original_filename.as_deref()
                        == Some(attempt.requested_target.as_str()))
        });
        if could_be_pending || target.is_empty() {
            return;
        }
        let generation = self.allocate_media_generation();
        self.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: self.lifecycle_epoch(),
            media_generation: generation,
            playlist_entry_id: current.id,
            observed_target: target,
            file_loaded: self.active_file_loaded,
        });
    }

    fn publish_reconciled_transport_state(&mut self, authoritative_current_entry_id: Option<i64>) {
        let Some(active_attempt) = self.player_lifecycle.active_attempt().cloned() else {
            self.clear_physical_projection();
            self.transport_phase = PlayerTransportPhase::Empty;
            return;
        };
        if active_attempt.playlist_entry_id != authoritative_current_entry_id {
            // A quiescent successor may appear in an authoritative playlist
            // before correlated file-loaded evidence lets it take transport.
            // Those observations belong to the successor and must not be
            // projected onto the reducer-retained predecessor.
            return;
        }
        let file_loaded = if self.active_load_attempt_id == Some(active_attempt.id) {
            self.active_file_loaded
        } else {
            matches!(
                active_attempt.state,
                crate::lifecycle::LoadAttemptState::Active
            )
        };
        self.install_physical_projection(
            active_attempt.id,
            active_attempt.media_generation,
            active_attempt.playlist_entry_id,
            self.observed_state.path.clone(),
            file_loaded,
        );
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;
        let update = self
            .transport_update_for(active_attempt.media_generation)
            .with_phase(phase);
        self.queue_transport_telemetry_update_for_attempt(update, Some(active_attempt.id));
        self.observe_tracked_commands_from_authoritative_state(
            active_attempt.media_generation,
            phase,
        );
    }

    fn observe_tracked_commands_from_authoritative_state(
        &mut self,
        media_generation: PlayerMediaGeneration,
        phase: PlayerTransportPhase,
    ) {
        let logical_pause = self.observed_state.logical_pause;
        let paused_for_cache = self.observed_state.paused_for_cache;
        let seeking = self.observed_state.seeking;
        let position_seconds = self.observed_state.position_seconds;
        let playback_restart_sequence = self
            .active_generation_has_restarted
            .then_some(self.playback_restart_sequence);

        if let Some(logical_pause) = logical_pause {
            self.observe_tracked_commands(
                Some(media_generation),
                TrackedCommandObservation::LogicalPause(logical_pause),
            );
        }
        if let Some(paused_for_cache) = paused_for_cache {
            self.observe_tracked_commands(
                Some(media_generation),
                TrackedCommandObservation::CachePause(paused_for_cache),
            );
        }
        if let Some(playback_restart_sequence) = playback_restart_sequence {
            self.observe_tracked_commands(
                Some(media_generation),
                TrackedCommandObservation::PlaybackRestart(playback_restart_sequence),
            );
        }
        if let Some(seeking) = seeking {
            self.observe_tracked_commands(
                Some(media_generation),
                TrackedCommandObservation::Seeking(seeking),
            );
        }
        if let Some(position_seconds) = position_seconds {
            self.observe_tracked_commands(
                Some(media_generation),
                TrackedCommandObservation::Position(position_seconds),
            );
        }
        self.observe_tracked_commands(
            Some(media_generation),
            TrackedCommandObservation::Phase(phase),
        );
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
            physical_file_loaded: attempt.physical_file_loaded(),
            semantic_load_result: attempt.semantic_load_result,
            logical_ownership_revoked: attempt.logical_ownership_revoked,
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
        self.last_finished_tracked_command_debug = Some(format!("{command:?} => {result:?}"));
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

    fn ordered_logical_pause(
        &self,
        logical_pause: Option<bool>,
        paused_for_cache: Option<bool>,
    ) -> Option<bool> {
        // Ordered consumers merge sparse fields. Classify an unowned physical
        // cache stop as logical playing so an earlier transient pause cannot
        // remain authoritative.
        logical_pause.or_else(|| {
            (paused_for_cache == Some(true) && !self.logical_pause_explicit).then_some(false)
        })
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
        ordered_delta.logical_pause =
            self.ordered_logical_pause(ordered_delta.logical_pause, ordered_delta.paused_for_cache);
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

    fn queue_simulated_authoritative_transport_state(&mut self) {
        if !self.simulation_mode {
            return;
        }
        let Some(generation) = self.observation_media_generation() else {
            return;
        };
        self.observed_state.position_seconds = Some(self.position_seconds);
        self.observed_state.paused = Some(self.paused);
        self.observed_state.logical_pause = Some(self.paused);
        self.observed_state.paused_for_cache = Some(self.paused_for_cache);
        self.observed_state.seeking = Some(false);
        self.observed_state.playback_rate = (self.playback_rate.is_finite()
            && self.playback_rate > 0.0)
            .then_some(self.playback_rate);

        let mut update = self.transport_update_for(generation);
        update.phase = Some(self.transport_phase);
        update.position_seconds = self.observed_state.position_seconds;
        update.playback_rate = self.observed_state.playback_rate;
        update.logical_pause = self.observed_state.logical_pause;
        update.paused_for_cache = self.observed_state.paused_for_cache;
        update.seeking = self.observed_state.seeking;
        self.queue_transport_telemetry_update(update);
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
        self.apply_completed_cache_pause_readback_if_current();
        self.flush_deferred_network_media_options_observation();
    }

    fn reduce_pending_ipc_events(&mut self, flush_network_options: bool) -> bool {
        let outermost_batch = self.network_options.network_media_options_event_batch_depth == 0;
        self.network_options.network_media_options_event_batch_depth += 1;
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
        self.network_options.network_media_options_event_batch_depth -= 1;

        if outermost_batch && flush_network_options {
            self.flush_deferred_network_media_options_observation();
        }
        processed_any
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
        self.latest_start_file_observation = None;
        self.deferred_start_file_observation = None;
        self.deferred_file_loaded_observation = None;
        self.transport_phase = PlayerTransportPhase::Failed;
        if let Some((attempt_id, generation)) = active_owner {
            let update = self
                .transport_update_for(generation)
                .with_phase(PlayerTransportPhase::Failed);
            self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        }
        self.clear_physical_projection();
    }

    fn apply_paused_for_cache_observation(&mut self, paused_for_cache: bool) {
        self.invalidate_cache_pause_readback_scope();
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
            PlayerPlaybackTelemetryUpdate::default().with_paused_for_cache(paused_for_cache),
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
    }

    fn handle_ipc_event(&mut self, event: &Value) {
        // Capture decoded event-pump input before classification so ignored or
        // malformed events remain available to deterministic replay.
        let _ = self.record_lifecycle_transcript_event(event.clone());

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
                            .latest_start_file_observation
                            .filter(|observation| {
                                observation.attachment_epoch == self.lifecycle_epoch()
                            })
                            .map(|observation| observation.playlist_entry_id)
                            .or_else(|| {
                                self.active_playlist_entry_id
                                    .and_then(|entry_id| i64::try_from(entry_id).ok())
                            })
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
                        if let Some(test_entry_id) = test_entry_id {
                            let _ = self.bind_single_pending_test_load(test_entry_id);
                        }
                    }
                }
                #[cfg(test)]
                if next_path.is_some() && self.active_load_attempt_id.is_none() {
                    // Legacy scripted transports omit the authoritative playlist
                    // query used in production. This compatibility projection is
                    // test-only; production ownership is established by the
                    // reducer from playlist-entry evidence before metadata is
                    // correlated.
                    if let Some(active) = self.player_lifecycle.active_attempt().cloned() {
                        self.install_physical_projection(
                            active.id,
                            active.media_generation,
                            active.playlist_entry_id,
                            next_path.clone(),
                            true,
                        );
                        self.active_generation_has_restarted = false;
                        self.transport_phase = PlayerTransportPhase::Prebuffering;
                        let update = self
                            .transport_update_for(active.media_generation)
                            .with_phase(PlayerTransportPhase::Prebuffering);
                        self.queue_transport_telemetry_update_for_attempt(update, Some(active.id));
                    }
                }
                let projection_can_accept_path =
                    self.active_load_attempt_id.is_some_and(|attempt_id| {
                        self.player_lifecycle
                            .load_attempts
                            .get(&attempt_id)
                            .is_some_and(|attempt| {
                                !attempt.logical_ownership_revoked
                                    || attempt.superseded_by.is_none()
                            })
                    });
                if projection_can_accept_path && let Some(attempt_id) = self.active_load_attempt_id
                {
                    self.update_physical_projection_path(attempt_id, next_path.clone());
                }
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
                    let logical_pause = if paused && self.logical_pause_explicit {
                        Some(true)
                    } else {
                        (!paused || self.observed_state.paused_for_cache != Some(true))
                            .then_some(paused)
                    };
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
                    self.apply_paused_for_cache_observation(paused_for_cache);
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
                    if seeking {
                        self.invalidate_network_stream_recovery_position_for_seek();
                    }
                    let transport_seeking = self.normalize_transport_seeking_observation(seeking);
                    self.observed_state.seeking = Some(transport_seeking);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.seeking = Some(transport_seeking);
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
                        TrackedCommandObservation::Seeking(transport_seeking),
                    );
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                    if !transport_seeking {
                        // mpv need not re-emit an unchanged paused-for-cache
                        // property after a seek. Treat seek completion as the
                        // fresh boundary from which an ongoing cache stall may
                        // safely arm recovery again.
                        self.observe_network_cache_pause_for_recovery(
                            self.observed_state.paused_for_cache == Some(true),
                        );
                    }
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
                    // keep-open=always retains the current file at EOF, so mpv can publish
                    // eof-reached=true without following it with end-file. Reuse the same
                    // generation-fenced, duration-bounded recovery transaction as end-file
                    // when the retained EOF is materially before the declared VOD tail.
                    if eof_reached
                        && self.player_lifecycle.provisional_eof_attempt()
                            == self.active_load_attempt_id
                        && let Some(generation) = self.active_media_generation
                    {
                        let _ = self.try_recover_interrupted_network_stream(generation);
                    }
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
        self.refresh_network_stream_recovery_evidence();
    }

    fn handle_start_file_event(&mut self, event: &Value) {
        self.stream_recovery.network_stream_recovery_evidence = None;
        let Some(playlist_entry_id) = event.get("playlist_entry_id").and_then(Value::as_u64) else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        self.handle_start_file_observation(playlist_entry_id);
    }

    fn handle_start_file_observation(&mut self, playlist_entry_id: u64) {
        self.stream_recovery.network_stream_recovery_evidence = None;
        // `pause`, `speed`, and `core-idle` are player/core properties rather
        // than file metadata. mpv does not necessarily emit another property
        // change when an already-paused player begins a new file, so retain
        // their last observations across the media-generation boundary.
        let Some(lifecycle_playlist_entry_id) = i64::try_from(playlist_entry_id).ok() else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        self.invalidate_cache_pause_readback_scope();
        let lifecycle_epoch = self.lifecycle_epoch();
        let observation = DeferredStartFileObservation {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: lifecycle_playlist_entry_id,
            playback_restart_sequence_at_observation: self.playback_restart_sequence,
            playback_restart_observed_after_start: false,
            retained_paused: self.observed_state.paused,
            retained_logical_pause: self.observed_state.logical_pause,
            retained_playback_rate: self.observed_state.playback_rate,
            retained_core_idle: self.observed_state.core_idle,
        };
        let replaces_previous_start = self.latest_start_file_observation.is_some_and(|previous| {
            previous.attachment_epoch != lifecycle_epoch
                || previous.playlist_entry_id != lifecycle_playlist_entry_id
        });
        if replaces_previous_start {
            self.deferred_start_file_observation = None;
            self.deferred_file_loaded_observation = None;
        }
        self.latest_start_file_observation = Some(observation);
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
                    // authoritative playlist. Matching path/file-loaded
                    // evidence performs the actual binding without mutating
                    // the physical projection before ownership is proven.
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
            self.deferred_start_file_observation = Some(observation);
            self.lifecycle_reconciliation_due = true;
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
        self.deferred_start_file_observation = None;
        if self.lifecycle_attempt_owns_loading_projection(lifecycle_attempt_id) {
            self.apply_bound_start_file_observation(observation, lifecycle_attempt_id, generation);
        }
    }

    fn lifecycle_attempt_owns_loading_projection(&self, attempt_id: LoadAttemptId) -> bool {
        self.player_lifecycle.active_load_attempt == Some(attempt_id)
            && self
                .player_lifecycle
                .load_attempts
                .get(&attempt_id)
                .is_some_and(|attempt| {
                    !attempt.logical_ownership_revoked
                        && !matches!(
                            attempt.state,
                            crate::lifecycle::LoadAttemptState::MayStillEmitQuiescent { .. }
                        )
                })
    }

    fn invalidate_cache_pause_readback_scope(&mut self) {
        self.cache_pause_observation_sequence =
            self.cache_pause_observation_sequence.wrapping_add(1).max(1);
        self.pending_cache_pause_readback = None;
    }

    fn apply_bound_start_file_observation(
        &mut self,
        observation: DeferredStartFileObservation,
        lifecycle_attempt_id: LoadAttemptId,
        generation: PlayerMediaGeneration,
    ) {
        let newer_restart_already_applied_to_attempt = self.playback_restart_sequence
            != observation.playback_restart_sequence_at_observation
            && self.active_generation_has_restarted
            && self.active_load_attempt_id == Some(lifecycle_attempt_id)
            && self.active_media_generation == Some(generation);
        let replay_playback_restart = observation.playback_restart_observed_after_start
            || newer_restart_already_applied_to_attempt;
        if self
            .network_options
            .network_media_options_hook_instance_id
            .is_some()
            && self
                .network_options
                .network_media_options_hook_configured_generation
                == Some(self.network_options.network_media_options_generation)
        {
            let base_sequence = self
                .network_options
                .network_media_options_hook_latest_started_load_sequence
                .or(self
                    .network_options
                    .network_media_options_hook_last_accepted_load_sequence)
                .unwrap_or(0);
            let load_sequence = base_sequence.wrapping_add(1).max(1);
            self.network_options
                .network_media_options_hook_latest_started_load_sequence = Some(load_sequence);
            self.network_options
                .network_media_options_expected_transition =
                Some(ExpectedNetworkOptionsTransition {
                    media_generation: generation,
                    load_sequence,
                });
        } else {
            self.network_options
                .network_media_options_expected_transition = None;
        }

        self.network_options.network_media_options_apply_identity = None;
        self.reset_network_media_policy_diagnostics();

        self.install_physical_projection(
            lifecycle_attempt_id,
            generation,
            Some(observation.playlist_entry_id),
            None,
            false,
        );
        self.active_generation_has_restarted = false;
        self.stream_recovery.network_cache_stall = None;
        self.reset_timeline_metadata();
        self.paused_for_cache = false;
        self.cache_buffering_percent = None;
        self.observed_state.paused = observation.retained_paused;
        self.observed_state.logical_pause = observation.retained_logical_pause;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.observed_state.position_seconds = None;
        self.observed_state.playback_rate = observation.retained_playback_rate;
        self.observed_state.paused_for_cache = None;
        self.observed_state.cache_buffering_percent = None;
        self.observed_state.seeking = None;
        self.observed_state.seekable = None;
        self.observed_state.seekable_ranges = None;
        self.observed_state.core_idle = observation.retained_core_idle;
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
        update.logical_pause = observation.retained_logical_pause;
        update.playback_rate = observation.retained_playback_rate;
        update.core_idle = observation.retained_core_idle;
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
        if replay_playback_restart {
            // The restart was reduced at the authoritative response boundary
            // before playlist identity could bind this start-file. Replay the
            // newer edge only after the older start has initialized the exact
            // attempt, preserving the causal event order.
            self.handle_playback_restart_event();
        }
    }

    fn replay_deferred_start_file_if_bound(&mut self) {
        let Some(observation) = self.deferred_start_file_observation else {
            return;
        };
        if observation.attachment_epoch != self.lifecycle_epoch() {
            self.deferred_start_file_observation = None;
            return;
        }
        let Some(attempt_id) = self
            .player_lifecycle
            .attempt_for_playlist_entry(observation.playlist_entry_id)
        else {
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
        self.deferred_start_file_observation = None;
        if self.lifecycle_attempt_owns_loading_projection(attempt_id) {
            self.apply_bound_start_file_observation(observation, attempt_id, generation);
        }
    }

    fn normalize_transport_seeking_observation(&self, seeking: bool) -> bool {
        if !seeking {
            return false;
        }
        let tracked_seek_is_pending = self
            .pending_tracked_commands
            .iter()
            .any(|command| matches!(command.kind, TrackedCommandKind::Seek { .. }));
        let settled_intentional_pause = self.active_file_loaded
            && self.active_generation_has_restarted
            && self.observed_state.logical_pause == Some(true)
            && self.observed_state.paused_for_cache == Some(false)
            && self.observed_state.core_idle == Some(true);
        // mpv documents that both the seek event and `seeking=true` can
        // represent an internal playback resync. Preserve the raw edge for
        // lifecycle/native-seek ownership, but do not let it indefinitely
        // replace a settled intentional pause when Sorotte has no seek command
        // awaiting completion.
        !(settled_intentional_pause && !tracked_seek_is_pending)
    }

    fn queue_transient_native_seek_edge_if_normalized(&mut self, transport_seeking: bool) {
        if transport_seeking {
            return;
        }
        // Keep the stable projection ReadyPaused, but preserve the raw mpv
        // seek edge long enough for attached consumers to classify a manual
        // paused seek. A following normalized delta closes the edge in the
        // same ordered batch so it cannot latch transport in Seeking.
        let mut update = self
            .transport_update()
            .with_phase(PlayerTransportPhase::Seeking);
        update.seeking = Some(true);
        self.queue_transport_telemetry_update(update);
    }

    fn handle_seek_event(&mut self) {
        self.stream_recovery.network_cache_stall = None;
        self.invalidate_network_stream_recovery_position_for_seek();
        self.begin_seek_cache_evidence_epoch();
        let transport_seeking = self.normalize_transport_seeking_observation(true);
        self.queue_transient_native_seek_edge_if_normalized(transport_seeking);
        self.observed_state.seeking = Some(transport_seeking);
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;
        let mut update = self.transport_update().with_phase(phase);
        update.seeking = Some(transport_seeking);
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
            TrackedCommandObservation::Seeking(transport_seeking),
        );
    }

    fn handle_playback_restart_event(&mut self) {
        self.stream_recovery.network_cache_stall = None;
        let lifecycle_epoch = self.lifecycle_epoch();
        let lifecycle_playlist_entry_id = self
            .active_playlist_entry_id
            .and_then(|entry_id| i64::try_from(entry_id).ok());
        let restart_follows_unbound_or_different_deferred_start = self
            .deferred_start_file_observation
            .is_some_and(|observation| {
                let matches_latest = observation.attachment_epoch == lifecycle_epoch
                    && self.latest_start_file_observation.is_some_and(|latest| {
                        latest.attachment_epoch == observation.attachment_epoch
                            && latest.playlist_entry_id == observation.playlist_entry_id
                    });
                let deferred_attempt = self
                    .player_lifecycle
                    .attempt_for_playlist_entry(observation.playlist_entry_id);
                let reducer_active_attempt = self
                    .player_lifecycle
                    .active_attempt()
                    .map(|attempt| attempt.id);
                matches_latest
                    && (deferred_attempt.is_none() || deferred_attempt != reducer_active_attempt)
            });
        if restart_follows_unbound_or_different_deferred_start {
            if let Some(observation) = self.deferred_start_file_observation.as_mut() {
                observation.playback_restart_observed_after_start = true;
            }
            // An accepted successor remains unbound until the authoritative
            // playlist response arrives, so the physical predecessor can
            // still be reducer-active here. Preserve this edge for the newer
            // deferred start instead of projecting it onto that predecessor.
            self.lifecycle_reconciliation_due = true;
            return;
        }
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
        self.install_physical_projection(
            active_attempt.id,
            active_attempt.media_generation,
            active_attempt.playlist_entry_id,
            self.current_path.clone(),
            true,
        );
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
        self.handle_file_loaded_observation(self.observed_state.path.clone());
    }

    fn handle_file_loaded_observation(&mut self, loaded_target: Option<String>) {
        let lifecycle_epoch = self.lifecycle_epoch();
        #[allow(unused_mut)]
        let mut playlist_entry_id = self
            .latest_start_file_observation
            .filter(|observation| observation.attachment_epoch == lifecycle_epoch)
            .map(|observation| observation.playlist_entry_id)
            .or_else(|| {
                self.active_playlist_entry_id
                    .and_then(|entry_id| i64::try_from(entry_id).ok())
            });
        #[cfg(test)]
        if playlist_entry_id.is_none_or(|entry_id| {
            self.player_lifecycle
                .attempt_for_playlist_entry(entry_id)
                .is_none()
        }) {
            let test_entry_id = playlist_entry_id.or_else(|| {
                self.active_playlist_entry_id
                    .and_then(|entry_id| i64::try_from(entry_id).ok())
                    .or_else(|| {
                        self.player_lifecycle
                            .load_attempts
                            .values()
                            .find(|attempt| {
                                !attempt.state.is_terminal() && attempt.playlist_entry_id.is_none()
                            })
                            .and_then(|attempt| i64::try_from(attempt.id.get()).ok())
                    })
            });
            if let Some(test_entry_id) = test_entry_id
                && self.bind_single_pending_test_load(test_entry_id)
            {
                playlist_entry_id = Some(test_entry_id);
            }
        }
        self.replay_deferred_start_file_if_bound();
        let Some(playlist_entry_id) = playlist_entry_id else {
            self.lifecycle_reconciliation_due = true;
            return;
        };
        let Some(attempt_id) = self
            .player_lifecycle
            .attempt_for_playlist_entry(playlist_entry_id)
        else {
            self.deferred_file_loaded_observation = Some(DeferredFileLoadedObservation {
                attachment_epoch: lifecycle_epoch,
                playlist_entry_id,
                loaded_target,
            });
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
        if self
            .deferred_file_loaded_observation
            .as_ref()
            .is_some_and(|observation| {
                observation.attachment_epoch == lifecycle_epoch
                    && observation.playlist_entry_id == playlist_entry_id
            })
        {
            self.deferred_file_loaded_observation = None;
        }
        let lifecycle_effects = self.apply_lifecycle_input(PlayerLifecycleInput::FileLoaded {
            attachment_epoch: lifecycle_epoch,
            playlist_entry_id: Some(playlist_entry_id),
            loaded_target: loaded_target.or_else(|| self.observed_state.path.clone()),
        });
        let owns_logical_playback = lifecycle_effects.iter().any(|effect| {
            matches!(
                effect,
                PlayerLifecycleEffect::PhysicalFileLoaded {
                    attempt_id: observed_attempt_id,
                    owns_logical_playback: true,
                } if *observed_attempt_id == attempt_id
            )
        });
        if !owns_logical_playback {
            return;
        }
        self.install_physical_projection(
            attempt_id,
            generation,
            Some(playlist_entry_id),
            self.observed_state
                .path
                .clone()
                .or_else(|| self.current_path.clone()),
            true,
        );
        // `start-file` clears generation-scoped transport observations, but mpv does not
        // necessarily emit a fresh property-change when a scalar value (notably
        // `paused-for-cache=false`) is unchanged across the boundary. Reacquire one coherent
        // post-file-loaded snapshot so tracked commands cannot wait forever for an event that
        // mpv is entitled to omit.
        self.lifecycle_reconciliation_due = true;
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;
        let update = self.transport_update().with_phase(phase);
        self.queue_transport_telemetry_update_for_attempt(update, Some(attempt_id));
        self.observe_tracked_commands(Some(generation), TrackedCommandObservation::FileLoaded);
        self.observe_tracked_commands(Some(generation), TrackedCommandObservation::Phase(phase));
        // Path can arrive between start-file and file-loaded. Transport
        // ownership begins at a normal Starting event, while file identity and
        // semantic load completion remain gated on this owned file-loaded
        // observation.
        self.maybe_emit_local_file_update_from_observed_state();

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
        let observed_metadata_is_current =
            self.path_metadata_generation == Some(generation) && self.observed_state.path.is_some();
        let metadata_is_current = polled_update.is_some() || observed_metadata_is_current;
        let loaded_update = polled_update.unwrap_or_else(|| {
            let path = if observed_metadata_is_current {
                self.observed_state
                    .path
                    .as_deref()
                    .unwrap_or(&requested_target)
            } else {
                &requested_target
            };
            let mut update = Self::local_file_update_for_path(path);
            if self.duration_metadata_generation == Some(generation)
                && let Some(duration_seconds) = self.observed_state.duration_seconds
            {
                update = update.with_duration_seconds(duration_seconds);
            }
            if observed_metadata_is_current && let Some(size_bytes) = self.observed_state.size_bytes
            {
                update = update.with_size_bytes(size_bytes);
            }
            update
        });
        self.update_physical_projection_path(attempt_id, loaded_update.path.clone());
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

    fn replay_deferred_file_loaded_if_bound(&mut self) {
        let Some(observation) = self.deferred_file_loaded_observation.as_ref() else {
            return;
        };
        if observation.attachment_epoch != self.lifecycle_epoch() {
            self.deferred_file_loaded_observation = None;
            return;
        }
        if self.latest_start_file_observation.is_none_or(|start| {
            start.attachment_epoch != observation.attachment_epoch
                || start.playlist_entry_id != observation.playlist_entry_id
        }) {
            self.deferred_file_loaded_observation = None;
            return;
        }
        if self
            .player_lifecycle
            .attempt_for_playlist_entry(observation.playlist_entry_id)
            .is_none()
        {
            return;
        }
        let observation = self
            .deferred_file_loaded_observation
            .take()
            .expect("checked deferred file-loaded observation");
        self.handle_file_loaded_observation(observation.loaded_target);
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
        if self
            .latest_start_file_observation
            .is_some_and(|observation| {
                observation.attachment_epoch == self.lifecycle_epoch()
                    && observation.playlist_entry_id == lifecycle_playlist_entry_id
            })
        {
            self.latest_start_file_observation = None;
            self.deferred_start_file_observation = None;
            self.deferred_file_loaded_observation = None;
        }
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
            }) || self
                .player_lifecycle
                .is_known_terminal_playlist_entry(lifecycle_playlist_entry_id);
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
        let recovery_started = reason == "eof"
            && self.player_lifecycle.active_load_attempt == Some(lifecycle_attempt_id)
            && generation
                .is_some_and(|generation| self.try_recover_interrupted_network_stream(generation));
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

        let affects_physical_projection = self.active_load_attempt_id == Some(lifecycle_attempt_id);
        if affects_physical_projection {
            if !recovery_started {
                self.stream_recovery.interrupted_network_stream_recovery = None;
            }
            self.stream_recovery.network_stream_recovery_evidence = None;
            self.stream_recovery.network_cache_stall = None;
            if self
                .network_options
                .network_media_options_expected_transition
                .is_some_and(|expected| Some(expected.media_generation) == generation)
            {
                self.network_options
                    .network_media_options_expected_transition = None;
            }
            self.transport_phase = if logical_terminal {
                phase
            } else {
                PlayerTransportPhase::Empty
            };
            self.clear_physical_projection();
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
            if self
                .network_options
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| Some(embedded.media_generation) == generation)
            {
                self.network_options.network_media_options_embedded_load = None;
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

    fn maybe_emit_local_file_update_from_observed_state(&mut self) {
        if !self.active_file_loaded || self.pending_load_request().is_some() {
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
        let result = self.send_ipc_command_if_attached_without_draining_events(command);
        // mpv may emit a causally earlier lifecycle event immediately before
        // rejecting a command. In particular, natural EOF can release
        // `time-pos` before a desync seek reaches the IPC worker. Always
        // harvest those events so callers can distinguish a terminal media
        // transition from an unhealthy player connection.
        self.drain_ipc_events_if_attached();
        result
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
            network_options: NetworkOptionsState {
                network_media_options_hook_enabled: false,
                ..NetworkOptionsState::default()
            },
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
            network_options: NetworkOptionsState {
                network_media_options_hook_enabled: true,
                ..NetworkOptionsState::default()
            },
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
            network_options: NetworkOptionsState {
                network_media_options_hook_enabled: false,
                ..NetworkOptionsState::default()
            },
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
            network_options: NetworkOptionsState {
                network_media_options_hook_enabled: false,
                ..NetworkOptionsState::default()
            },
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
        self.last_ipc_event_fence_at = None;
        self.pending_ipc_event_fence_command_id = None;
        self.invalidate_cache_pause_readback_scope();
    }

    #[cfg(test)]
    pub(crate) fn force_ipc_event_fence_due_for_test(&mut self) {
        self.last_ipc_event_fence_at = Some(Instant::now() - IPC_EVENT_FENCE_IDLE_INTERVAL);
    }

    #[cfg(test)]
    pub(crate) fn force_test_legacy_syncplayintf_heartbeat_due(&mut self) {
        self.legacy_syncplayintf_last_heartbeat_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL);
        self.maintain_legacy_syncplayintf_lease();
    }

    #[cfg(test)]
    pub(crate) fn force_test_network_media_options_hook_heartbeat_due(&mut self) {
        self.network_options
            .network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);
        self.maintain_network_media_options_hook_lease();
    }

    #[cfg(test)]
    pub(crate) fn force_test_network_media_options_hook_heartbeat_ack_timeout(&mut self) {
        if let Some(pending) = self
            .network_options
            .network_media_options_hook_pending_heartbeat
            .as_mut()
        {
            pending.sent_at = Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT);
        }
        self.maintain_network_media_options_hook_lease();
    }

    #[cfg(test)]
    pub(crate) fn test_network_media_options_hook_heartbeat_pending(&self) -> bool {
        self.network_options
            .network_media_options_hook_pending_heartbeat
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn test_network_media_options_hook_is_ready(&self) -> bool {
        self.network_media_options_hook_is_ready()
    }

    #[cfg(test)]
    pub(crate) fn prepare_test_network_options_hook_v3_reducer(&mut self) {
        self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        self.network_options.network_media_options_hook_loaded = true;
        self.network_options.network_media_options_hook_instance_id =
            Some("test-hook-instance".to_owned());
        self.network_options
            .network_media_options_hook_configured_generation =
            Some(self.network_options.network_media_options_generation);
        self.network_options
            .network_media_options_hook_last_heartbeat_at = Some(Instant::now());
        self.network_options
            .network_media_options_hook_latest_started_load_sequence = Some(0);
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
            "configurationGeneration": self.network_options.network_media_options_generation,
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
            "configurationGeneration": self.network_options.network_media_options_generation,
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
            "configurationGeneration": self.network_options.network_media_options_generation,
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
            "configurationGeneration": self.network_options.network_media_options_generation,
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
            "configurationGeneration": self.network_options.network_media_options_generation,
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
        self.network_options.network_media_options_event_batch_depth += 1;
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
        self.network_options.network_media_options_event_batch_depth = self
            .network_options
            .network_media_options_event_batch_depth
            .saturating_sub(1);
        self.flush_deferred_network_media_options_observation();
    }

    #[cfg(test)]
    pub(crate) fn test_network_options_policy_source_path(&self) -> Option<&str> {
        self.network_options
            .network_media_options_apply_identity
            .as_ref()
            .map(|identity| identity.path.as_str())
    }

    #[cfg(test)]
    pub(crate) fn test_network_options_last_accepted_load_sequence(&self) -> Option<u64> {
        self.network_options
            .network_media_options_hook_last_accepted_load_sequence
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
            self.network_options.network_media_options_policy_state,
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
        let paused = self.observed_state.paused;
        let logical_pause = self.observed_state.logical_pause;
        let playback_rate = self.observed_state.playback_rate;
        let paused_for_cache = self.observed_state.paused_for_cache;
        let cache_buffering_percent = self.observed_state.cache_buffering_percent;
        let seeking = self.observed_state.seeking;
        let seekable = self.observed_state.seekable;
        let core_idle = self.observed_state.core_idle;
        let demuxer_cache_idle = self.observed_state.demuxer_cache_idle;
        let eof_reached = self.observed_state.eof_reached;
        let entries = entries
            .into_iter()
            .map(|(id, original_filename, current)| {
                AuthoritativePlaylistEntry::new(id, original_filename, current)
            })
            .collect::<Vec<_>>();
        let authoritative_current_entry_id = entries
            .iter()
            .find(|entry| entry.current)
            .map(|entry| entry.id);
        self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
            attachment_epoch,
            entries: entries.clone(),
            current_path: current_path.clone(),
        });
        self.observe_external_current_from_authority(&entries, current_path.as_deref());
        self.replay_deferred_start_file_if_bound();
        self.replay_deferred_file_loaded_if_bound();
        self.observed_state.path = current_path.clone();
        self.observed_state.paused = paused;
        self.observed_state.logical_pause = logical_pause;
        self.observed_state.playback_rate = playback_rate;
        self.observed_state.paused_for_cache = paused_for_cache;
        self.observed_state.cache_buffering_percent = cache_buffering_percent;
        self.observed_state.seeking = seeking;
        self.observed_state.seekable = seekable;
        self.observed_state.core_idle = core_idle;
        self.observed_state.demuxer_cache_idle = demuxer_cache_idle;
        self.observed_state.eof_reached = eof_reached;
        self.publish_reconciled_transport_state(authoritative_current_entry_id);
        self.lifecycle_reconciliation_due = self.player_lifecycle.reconciliation_required
            || self.player_lifecycle.requires_authoritative_snapshot()
            || self.deferred_start_file_observation.is_some()
            || self.deferred_file_loaded_observation.is_some();
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
            .filter(|attempt| {
                !attempt.state.is_terminal()
                    && !matches!(
                        attempt.state,
                        crate::lifecycle::LoadAttemptState::MayStillEmitQuiescent { .. }
                    )
            })
            .map(|attempt| attempt.media_generation)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn observe_load_ready_before_binding_for_test(
        &mut self,
        playlist_entry_id: i64,
        loaded_target: impl Into<String>,
    ) {
        let playlist_entry_id =
            u64::try_from(playlist_entry_id).expect("test playlist entry ID should be nonnegative");
        self.handle_start_file_observation(playlist_entry_id);
        self.handle_ipc_event(&json!({
            "event": "property-change",
            "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
            "data": false,
        }));
        self.handle_ipc_event(&json!({
            "event": "property-change",
            "name": MPV_PROPERTY_PAUSE,
            "data": true,
        }));
        self.handle_ipc_event(&json!({
            "event": "property-change",
            "name": MPV_PROPERTY_CORE_IDLE,
            "data": true,
        }));
        self.handle_file_loaded_observation(Some(loaded_target.into()));
    }
}

// Pre-reducer load-registry and recovery fixtures were removed after their unique
// ownership, provisional-EOF, and watchdog cases were ported to reducer-backed suites.

#[cfg(test)]
mod lifecycle_evidence_projection_tests;

#[cfg(test)]
mod lifecycle_transcript_capture_tests;

#[cfg(test)]
mod version_policy_tests;

#[cfg(test)]
mod timeline_kind_tests;

#[cfg(test)]
mod transport_queue_pressure_tests;

#[cfg(test)]
mod interrupted_network_stream_recovery_tests;

#[cfg(test)]
mod authoritative_reconciliation_regression_tests;
