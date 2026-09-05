use std::fmt;

use sorotte_player_api::{
    PlayerPlayIntent, PlayerSeekableRange, PlayerTimelineKind, PlayerTransportPhase,
};
pub use sorotte_protocol::MediaLoadIntent;

const MIN_ADVANCEMENT_SECONDS: f64 = 0.01;
const NORMAL_PLAYBACK_RATE: f64 = 1.0;
const CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM: f64 = 1.03;
const HEALTHY_CATCHUP_BUFFER_SECONDS: f64 = 2.0;
const MAXIMUM_CATCHUP_EPISODE_SECONDS: f64 = 300.0;
const SEEK_PREPARATION_TIMEOUT_SECONDS: f64 = 60.0;
const SEEK_PREPARATION_MAX_STABILIZATION_SECONDS: f64 = 1.0;
const NEAREST_BUFFERED_TARGET_LIMIT_SECONDS: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTransportKind {
    LocalFile,
    NetworkVod,
    LiveSliding,
    NonSeekable,
}

impl MediaTransportKind {
    pub const fn default_seekable(self) -> bool {
        matches!(self, Self::LocalFile | Self::NetworkVod | Self::LiveSliding)
    }

    pub const fn allows_rate_correction(self) -> bool {
        !matches!(self, Self::NonSeekable)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LogicalMediaId(String);

impl LogicalMediaId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err("logical media ID must not be empty");
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LogicalMediaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogicalMediaId")
            .field("bytes", &self.0.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaLoadPlan {
    pub media_generation: u64,
    pub load_attempt: u64,
    pub logical_media_changed: bool,
    pub playback_episode_changed: bool,
    pub load_intent: MediaLoadIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPolicy {
    PreserveContent,
    Balanced,
    StayClosest,
    PauseRoom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackCoordinatorConfig {
    pub recovery_policy: RecoveryPolicy,
    pub negligible_lag_seconds: f64,
    pub hard_seek_threshold_seconds: f64,
    pub maximum_catchup_rate: f64,
    pub maximum_hard_seeks_per_episode: u32,
    pub stability_interval_seconds: f64,
    pub position_tolerance_seconds: f64,
    pub command_timeout_seconds: f64,
    pub command_retry_budget: u32,
    pub command_retry_cooldown_seconds: f64,
    pub seek_preparation_timeout_seconds: f64,
    pub seek_preparation_minimum_headroom_seconds: f64,
    pub nearest_buffered_target_limit_seconds: f64,
}

impl Default for PlaybackCoordinatorConfig {
    fn default() -> Self {
        Self {
            recovery_policy: RecoveryPolicy::Balanced,
            negligible_lag_seconds: 1.0,
            hard_seek_threshold_seconds: 8.0,
            maximum_catchup_rate: 1.05,
            maximum_hard_seeks_per_episode: 1,
            stability_interval_seconds: 4.0,
            position_tolerance_seconds: 0.35,
            command_timeout_seconds: 10.0,
            command_retry_budget: 1,
            command_retry_cooldown_seconds: 2.0,
            seek_preparation_timeout_seconds: SEEK_PREPARATION_TIMEOUT_SECONDS,
            seek_preparation_minimum_headroom_seconds: HEALTHY_CATCHUP_BUFFER_SECONDS,
            nearest_buffered_target_limit_seconds: NEAREST_BUFFERED_TARGET_LIMIT_SECONDS,
        }
    }
}

impl PlaybackCoordinatorConfig {
    pub fn normalized(mut self) -> Self {
        let defaults = Self::default();
        if !self.negligible_lag_seconds.is_finite() || self.negligible_lag_seconds < 0.0 {
            self.negligible_lag_seconds = defaults.negligible_lag_seconds;
        }
        if !self.hard_seek_threshold_seconds.is_finite()
            || self.hard_seek_threshold_seconds <= self.negligible_lag_seconds
        {
            self.hard_seek_threshold_seconds = defaults.hard_seek_threshold_seconds;
        }
        if !self.maximum_catchup_rate.is_finite()
            || !(NORMAL_PLAYBACK_RATE..=1.25).contains(&self.maximum_catchup_rate)
        {
            self.maximum_catchup_rate = defaults.maximum_catchup_rate;
        }
        if !self.stability_interval_seconds.is_finite() || self.stability_interval_seconds <= 0.0 {
            self.stability_interval_seconds = defaults.stability_interval_seconds;
        }
        if !self.position_tolerance_seconds.is_finite() || self.position_tolerance_seconds < 0.0 {
            self.position_tolerance_seconds = defaults.position_tolerance_seconds;
        }
        if !self.command_timeout_seconds.is_finite() || self.command_timeout_seconds <= 0.0 {
            self.command_timeout_seconds = defaults.command_timeout_seconds;
        }
        if !self.command_retry_cooldown_seconds.is_finite()
            || self.command_retry_cooldown_seconds < 0.0
        {
            self.command_retry_cooldown_seconds = defaults.command_retry_cooldown_seconds;
        }
        if !self.seek_preparation_timeout_seconds.is_finite()
            || self.seek_preparation_timeout_seconds <= 0.0
        {
            self.seek_preparation_timeout_seconds = defaults.seek_preparation_timeout_seconds;
        }
        let minimum_seek_preparation_timeout = self
            .command_timeout_seconds
            .min(MAXIMUM_CATCHUP_EPISODE_SECONDS);
        self.seek_preparation_timeout_seconds = self.seek_preparation_timeout_seconds.clamp(
            minimum_seek_preparation_timeout,
            MAXIMUM_CATCHUP_EPISODE_SECONDS,
        );
        if !self.seek_preparation_minimum_headroom_seconds.is_finite()
            || self.seek_preparation_minimum_headroom_seconds < 0.0
        {
            self.seek_preparation_minimum_headroom_seconds =
                defaults.seek_preparation_minimum_headroom_seconds;
        }
        self.seek_preparation_minimum_headroom_seconds = self
            .seek_preparation_minimum_headroom_seconds
            .clamp(0.0, 60.0);
        if !self.nearest_buffered_target_limit_seconds.is_finite()
            || self.nearest_buffered_target_limit_seconds <= 0.0
        {
            self.nearest_buffered_target_limit_seconds =
                defaults.nearest_buffered_target_limit_seconds;
        }
        self.nearest_buffered_target_limit_seconds =
            self.nearest_buffered_target_limit_seconds.clamp(1.0, 60.0);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesiredRoomPlayback {
    pub media_generation: u64,
    pub state_revision: u64,
    pub paused: bool,
    pub anchor_position_seconds: f64,
    pub anchor_observed_at_seconds: f64,
    pub force_seek: bool,
}

/// Describes why a desired room state is being updated.
///
/// Runtime integrations should use [`Self::ExplicitSeek`] only for a newly
/// observed authoritative `doSeek` operation that still needs a player
/// command, and [`Self::ExplicitSeekAlreadyDispatched`] for the canonical
/// echo of a local player seek. Ordinary timestamp aging, pause changes,
/// reconnect reconciliation, and barrier alignment must use [`Self::Ordinary`]
/// so they cannot replace an in-flight frozen seek target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesiredRoomPlaybackUpdateKind {
    #[default]
    Ordinary,
    ExplicitSeek,
    ExplicitSeekAlreadyDispatched,
    AuthoritativeSeekAfterSupersededDispatch,
}

impl DesiredRoomPlayback {
    pub fn position_at(self, now_seconds: f64) -> f64 {
        let elapsed = if self.paused {
            0.0
        } else {
            (now_seconds - self.anchor_observed_at_seconds).max(0.0)
        };
        (self.anchor_position_seconds + elapsed).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerTransportObservation {
    pub media_generation: u64,
    pub observed_at_seconds: f64,
    pub phase: Option<PlayerTransportPhase>,
    pub position_seconds: Option<f64>,
    pub playback_rate: Option<f64>,
    pub logical_pause: Option<bool>,
    pub paused_for_cache: Option<bool>,
    pub seeking: Option<bool>,
    pub seekable: Option<bool>,
    pub timeline_kind: Option<PlayerTimelineKind>,
    pub seekable_ranges: Option<Vec<PlayerSeekableRange>>,
    pub known_live_seekable_window: Option<PlayerSeekableRange>,
    pub core_idle: Option<bool>,
    pub playback_restart_sequence: Option<u64>,
    pub cache_buffering_percent: Option<f64>,
    pub buffered_ahead_seconds: Option<f64>,
    pub input_rate_bytes_per_second: Option<u64>,
}

impl PlayerTransportObservation {
    pub fn new(media_generation: u64, observed_at_seconds: f64) -> Self {
        Self {
            media_generation,
            observed_at_seconds,
            phase: None,
            position_seconds: None,
            playback_rate: None,
            logical_pause: None,
            paused_for_cache: None,
            seeking: None,
            seekable: None,
            timeline_kind: None,
            seekable_ranges: None,
            known_live_seekable_window: None,
            core_idle: None,
            playback_restart_sequence: None,
            cache_buffering_percent: None,
            buffered_ahead_seconds: None,
            input_rate_bytes_per_second: None,
        }
    }

    pub fn with_phase(mut self, phase: PlayerTransportPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_position(mut self, position_seconds: f64) -> Self {
        self.position_seconds = Some(position_seconds);
        self
    }

    pub fn with_playback_rate(mut self, playback_rate: f64) -> Self {
        self.playback_rate = Some(playback_rate);
        self
    }

    pub fn with_logical_pause(mut self, logical_pause: bool) -> Self {
        self.logical_pause = Some(logical_pause);
        self
    }

    pub fn with_cache_pause(mut self, paused_for_cache: bool) -> Self {
        self.paused_for_cache = Some(paused_for_cache);
        self
    }

    pub fn with_seeking(mut self, seeking: bool) -> Self {
        self.seeking = Some(seeking);
        self
    }

    pub fn with_seekable(mut self, seekable: bool) -> Self {
        self.seekable = Some(seekable);
        self
    }

    pub fn with_timeline_kind(mut self, timeline_kind: PlayerTimelineKind) -> Self {
        self.timeline_kind = Some(timeline_kind);
        self
    }

    pub fn with_seekable_ranges(mut self, seekable_ranges: Vec<PlayerSeekableRange>) -> Self {
        self.seekable_ranges = Some(seekable_ranges);
        self
    }

    pub fn with_known_live_seekable_window(
        mut self,
        known_live_seekable_window: PlayerSeekableRange,
    ) -> Self {
        self.known_live_seekable_window = Some(known_live_seekable_window);
        self
    }

    pub fn with_restart_sequence(mut self, sequence: u64) -> Self {
        self.playback_restart_sequence = Some(sequence);
        self
    }

    pub fn with_core_idle(mut self, core_idle: bool) -> Self {
        self.core_idle = Some(core_idle);
        self
    }

    pub fn with_cache_buffering_percent(mut self, percent: f64) -> Self {
        self.cache_buffering_percent = Some(percent);
        self
    }

    pub fn with_buffered_ahead_seconds(mut self, seconds: f64) -> Self {
        self.buffered_ahead_seconds = Some(seconds);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoordinatorCommandId(u64);

impl CoordinatorCommandId {
    pub const fn new(value: u64) -> Self {
        Self(if value == 0 { 1 } else { value })
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoordinatorPlayerCommand {
    SetPaused(bool),
    Play(PlayerPlayIntent),
    SetPosition(f64),
    SetPlaybackRate(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedPlaybackReason {
    NonSeekableLag,
    HardSeekBudgetExhausted,
    CatchupDidNotConverge,
    RecoveryCommandTimedOut,
    TimelineWindowUnavailable,
    TransportFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekTargetAvailability {
    Cached,
    FetchRequired,
    Unknown,
    OutsideLiveWindow,
    NonSeekable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekPreparationPhase {
    Seeking,
    Fetching,
    Refilling,
    ReadyToJoin,
    CatchingUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekPreparationDegradedReason {
    NonSeekable,
    OutsideLiveWindow,
    TimedOut,
    TimelineWindowUnavailable,
    TransportFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekPreparationTerminalOutcome {
    Ready,
    Superseded,
    Cancelled,
    Degraded(SeekPreparationDegradedReason),
}

/// Public, UI-safe view of a client-owned unbuffered-seek episode.
///
/// Cache percentage is mpv's refill target progress, not media download
/// progress. All target fields remain frozen for the life of an episode except
/// that a live/sliding target may be clamped once its current window becomes
/// known.
#[derive(Debug, Clone, PartialEq)]
pub struct SeekPreparationSnapshot {
    pub id: u64,
    pub media_generation: u64,
    pub load_attempt: u64,
    pub room_revision: u64,
    pub latest_room_revision: u64,
    pub requested_target_seconds: f64,
    pub frozen_target_seconds: f64,
    pub frozen_room_anchor_position_seconds: f64,
    pub frozen_room_anchor_observed_at_seconds: f64,
    pub latest_room_position_seconds: f64,
    pub availability: SeekTargetAvailability,
    pub phase: SeekPreparationPhase,
    pub cache_buffering_percent: Option<f64>,
    pub buffered_ahead_seconds: Option<f64>,
    pub nearest_safe_buffered_position_seconds: Option<f64>,
    pub started_at_seconds: f64,
    pub terminal_outcome: Option<SeekPreparationTerminalOutcome>,
    pub can_keep_waiting: bool,
    pub can_cancel_and_remain: bool,
    pub can_join_nearest_buffered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackCoordinatorAction {
    Execute {
        command_id: CoordinatorCommandId,
        command: CoordinatorPlayerCommand,
    },
    RequestRoomPause {
        recovery_episode_id: u64,
    },
    RevisionApplied {
        media_generation: u64,
        state_revision: u64,
    },
    Started {
        media_generation: u64,
        state_revision: u64,
        observed_position_seconds: f64,
    },
    Degraded {
        media_generation: u64,
        recovery_episode_id: Option<u64>,
        reason: DegradedPlaybackReason,
    },
    CommandTimedOut {
        command_id: CoordinatorCommandId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackDiagnostic {
    Empty,
    Loading,
    Prebuffering,
    ReadyWaitingForRoom,
    Starting,
    Playing,
    Rebuffering,
    RecoveringByCatchup,
    RecoveringBySeek,
    Degraded,
    Ended,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaybackCoordinatorMetrics {
    pub first_frame_latency_seconds: Option<f64>,
    pub started_ack_latency_seconds: Option<f64>,
    pub buffer_episode_count: u64,
    pub total_buffer_duration_seconds: f64,
    pub hard_seek_count: u64,
    pub gentle_catchup_count: u64,
    pub degraded_recovery_count: u64,
    pub stale_generation_observations: u64,
    pub stale_timestamp_observations: u64,
    pub command_timeouts: u64,
    pub applied_revision_count: u64,
    pub steady_state_skew_seconds: Option<f64>,
    pub last_buffered_ahead_seconds: Option<f64>,
    pub last_input_rate_bytes_per_second: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEpisodeSnapshot {
    pub id: u64,
    pub media_generation: u64,
    pub entered_at_seconds: f64,
    pub hard_seek_attempts: u32,
    pub stable_since_seconds: Option<f64>,
    pub catchup_deadline_seconds: Option<f64>,
    pub degraded: bool,
}

#[derive(Clone)]
struct MediaState {
    logical_id: LogicalMediaId,
    generation: u64,
    load_attempt: u64,
    kind: MediaTransportKind,
    prepared_at_seconds: f64,
    load_restart_baseline: u64,
}

impl fmt::Debug for MediaState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaState")
            .field("logical_id", &self.logical_id)
            .field("generation", &self.generation)
            .field("load_attempt", &self.load_attempt)
            .field("kind", &self.kind)
            .field("prepared_at_seconds", &self.prepared_at_seconds)
            .field("load_restart_baseline", &self.load_restart_baseline)
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
struct ObservedState {
    observed_at_seconds: f64,
    phase: PlayerTransportPhase,
    position_seconds: Option<f64>,
    playback_rate: Option<f64>,
    logical_pause: Option<bool>,
    paused_for_cache: bool,
    seeking: bool,
    seekable: Option<bool>,
    seekable_window: Option<(f64, f64)>,
    timeline_kind: Option<PlayerTimelineKind>,
    known_live_seekable_window: Option<PlayerSeekableRange>,
    core_idle: Option<bool>,
    playback_restart_sequence: u64,
    cache_buffering_percent: Option<f64>,
    buffered_ahead_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
enum PendingCommandKind {
    Pause,
    Play {
        baseline_position: Option<f64>,
        intent: PlayerPlayIntent,
    },
    Seek {
        target_position_seconds: f64,
        baseline_restart_sequence: u64,
    },
    Rate {
        target_rate: f64,
    },
}

#[derive(Debug, Clone, Copy)]
struct PendingCommand {
    id: CoordinatorCommandId,
    revision: u64,
    issued_at_seconds: f64,
    accepted: bool,
    kind: PendingCommandKind,
}

#[derive(Debug, Clone)]
struct RecoveryEpisode {
    id: u64,
    media_generation: u64,
    entered_at_seconds: f64,
    hard_seek_attempts: u32,
    post_cache_baseline_at_seconds: Option<f64>,
    stable_since_seconds: Option<f64>,
    catchup_deadline_seconds: Option<f64>,
    decision_made: bool,
    cache_metrics_frozen_until_decision: bool,
    catchup_active: bool,
    seek_active: bool,
    gentle_catchup_only: bool,
    degraded: bool,
}

#[derive(Debug, Clone)]
struct SeekPreparationEpisode {
    id: u64,
    media_generation: u64,
    load_attempt: u64,
    room_revision: u64,
    latest_room_revision: u64,
    latest_room_paused: bool,
    requested_target_seconds: f64,
    frozen_target_seconds: f64,
    frozen_room_anchor_position_seconds: f64,
    frozen_room_anchor_observed_at_seconds: f64,
    latest_room_position_seconds: f64,
    availability: SeekTargetAvailability,
    phase: SeekPreparationPhase,
    cache_buffering_percent: Option<f64>,
    buffered_ahead_seconds: Option<f64>,
    input_rate_bytes_per_second: Option<u64>,
    nearest_safe_buffered_position_seconds: Option<f64>,
    started_at_seconds: f64,
    deadline_seconds: f64,
    primary_seek_command_id: Option<CoordinatorCommandId>,
    primary_seek_issued: bool,
    primary_seek_observation_sequence: Option<u64>,
    refill_started_observation_sequence: Option<u64>,
    refill_released_after_seek: bool,
    stable_playable_since: Option<(u64, f64)>,
}

impl SeekPreparationEpisode {
    fn snapshot(
        &self,
        terminal_outcome: Option<SeekPreparationTerminalOutcome>,
    ) -> SeekPreparationSnapshot {
        let active = terminal_outcome.is_none();
        SeekPreparationSnapshot {
            id: self.id,
            media_generation: self.media_generation,
            load_attempt: self.load_attempt,
            room_revision: self.room_revision,
            latest_room_revision: self.latest_room_revision,
            requested_target_seconds: self.requested_target_seconds,
            frozen_target_seconds: self.frozen_target_seconds,
            frozen_room_anchor_position_seconds: self.frozen_room_anchor_position_seconds,
            frozen_room_anchor_observed_at_seconds: self.frozen_room_anchor_observed_at_seconds,
            latest_room_position_seconds: self.latest_room_position_seconds,
            availability: self.availability,
            phase: self.phase,
            cache_buffering_percent: self.cache_buffering_percent,
            buffered_ahead_seconds: self.buffered_ahead_seconds,
            nearest_safe_buffered_position_seconds: self.nearest_safe_buffered_position_seconds,
            started_at_seconds: self.started_at_seconds,
            terminal_outcome,
            can_keep_waiting: active,
            can_cancel_and_remain: active && !self.primary_seek_issued,
            can_join_nearest_buffered: active
                && !self.latest_room_paused
                && self.nearest_safe_buffered_position_seconds.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RateOverride {
    episode_id: u64,
    reset_requested: bool,
    reset_command_observation_sequence: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PlaybackCoordinator {
    config: PlaybackCoordinatorConfig,
    media: Option<MediaState>,
    desired: Option<DesiredRoomPlayback>,
    observed: Option<ObservedState>,
    recovery: Option<RecoveryEpisode>,
    seek_preparation: Option<SeekPreparationEpisode>,
    last_seek_preparation_terminal: Option<SeekPreparationSnapshot>,
    cached_seekable_ranges: Option<Vec<PlayerSeekableRange>>,
    pending_commands: Vec<PendingCommand>,
    next_media_generation: u64,
    next_recovery_episode_id: u64,
    next_seek_preparation_id: u64,
    next_command_id: u64,
    last_applied_revision: Option<u64>,
    last_started_revision: Option<u64>,
    desired_seek_satisfied_revision: Option<u64>,
    required_seek_dispatch_revision: Option<u64>,
    authoritative_alignment_guard_revision: Option<u64>,
    completed_seek_restart_baseline: Option<u64>,
    rate_override: Option<RateOverride>,
    observation_sequence: u64,
    last_playback_rate_observation_sequence: Option<u64>,
    retry_not_before_seconds: f64,
    failed_command_attempts: u32,
    command_budget_degraded: bool,
    pending_degraded_reason: Option<DegradedPlaybackReason>,
    diagnostic: PlaybackDiagnostic,
    metrics: PlaybackCoordinatorMetrics,
}

impl Default for PlaybackCoordinator {
    fn default() -> Self {
        Self::new(PlaybackCoordinatorConfig::default())
    }
}

impl PlaybackCoordinator {
    pub fn new(config: PlaybackCoordinatorConfig) -> Self {
        Self {
            config: config.normalized(),
            media: None,
            desired: None,
            observed: None,
            recovery: None,
            seek_preparation: None,
            last_seek_preparation_terminal: None,
            cached_seekable_ranges: None,
            pending_commands: Vec::new(),
            next_media_generation: 0,
            next_recovery_episode_id: 0,
            next_seek_preparation_id: 0,
            next_command_id: 0,
            last_applied_revision: None,
            last_started_revision: None,
            desired_seek_satisfied_revision: None,
            required_seek_dispatch_revision: None,
            authoritative_alignment_guard_revision: None,
            completed_seek_restart_baseline: None,
            rate_override: None,
            observation_sequence: 0,
            last_playback_rate_observation_sequence: None,
            retry_not_before_seconds: 0.0,
            failed_command_attempts: 0,
            command_budget_degraded: false,
            pending_degraded_reason: None,
            diagnostic: PlaybackDiagnostic::Empty,
            metrics: PlaybackCoordinatorMetrics::default(),
        }
    }

    pub fn set_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.config = config.normalized();
    }

    pub fn reset_transport_adapter_epoch(&mut self, now_seconds: f64) {
        self.observed = None;
        self.cached_seekable_ranges = None;
        self.metrics.steady_state_skew_seconds = None;
        self.metrics.last_buffered_ahead_seconds = None;
        self.metrics.last_input_rate_bytes_per_second = None;
        self.pending_commands.clear();
        if let Some(recovery) = self.recovery.as_mut() {
            recovery.post_cache_baseline_at_seconds = None;
            recovery.stable_since_seconds = None;
            recovery.catchup_deadline_seconds = None;
            recovery.decision_made = recovery.degraded;
            recovery.catchup_active = false;
            recovery.seek_active = false;
        }
        // The adapter boundary invalidates observations and commands, not the
        // logical recovery episode or its already-consumed hard-seek budget.
        self.request_rate_reset();
        self.completed_seek_restart_baseline = None;
        if let Some(episode) = self.seek_preparation.as_mut() {
            episode.primary_seek_command_id = None;
            episode.availability = SeekTargetAvailability::Unknown;
            episode.phase = SeekPreparationPhase::Seeking;
            episode.cache_buffering_percent = None;
            episode.buffered_ahead_seconds = None;
            episode.input_rate_bytes_per_second = None;
            episode.nearest_safe_buffered_position_seconds = None;
            episode.primary_seek_observation_sequence = Some(self.observation_sequence);
            episode.refill_started_observation_sequence = None;
            episode.refill_released_after_seek = false;
            episode.stable_playable_since = None;
        }
        if let Some(media) = self.media.as_mut() {
            media.load_restart_baseline = 0;
        }
        self.retry_not_before_seconds = 0.0;
        self.failed_command_attempts = 0;
        self.command_budget_degraded = false;
        self.pending_degraded_reason = None;
        if let Some(media) = self.media.as_mut() {
            if now_seconds.is_finite() {
                media.prepared_at_seconds = now_seconds;
            }
            self.diagnostic = PlaybackDiagnostic::Loading;
        } else {
            self.diagnostic = PlaybackDiagnostic::Empty;
        }
    }

    pub fn prepare_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let same_logical_media = self
            .media
            .as_ref()
            .is_some_and(|media| media.logical_id == logical_id);
        let inferred_intent = if same_logical_media {
            MediaLoadIntent::TransportRefresh
        } else {
            MediaLoadIntent::NewPlayback
        };
        self.prepare_media_with_intent(logical_id, kind, inferred_intent, now_seconds)
    }

    pub fn prepare_media_with_intent(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        requested_intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        self.prepare_media_with_intent_internal(
            logical_id,
            kind,
            requested_intent,
            now_seconds,
            false,
        )
    }

    pub fn prepare_media_for_room_participation(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        self.prepare_media_with_intent_internal(
            logical_id,
            kind,
            MediaLoadIntent::TransportRefresh,
            now_seconds,
            true,
        )
    }

    fn prepare_media_with_intent_internal(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        requested_intent: MediaLoadIntent,
        now_seconds: f64,
        preserve_changed_identity_transport_refresh: bool,
    ) -> MediaLoadPlan {
        let load_restart_baseline = self
            .observed
            .map_or(0, |observed| observed.playback_restart_sequence);
        let same_logical_media = self
            .media
            .as_ref()
            .is_some_and(|media| media.logical_id == logical_id);
        let load_intent = match (same_logical_media, requested_intent) {
            (true, MediaLoadIntent::TransportRefresh) => MediaLoadIntent::TransportRefresh,
            (true, MediaLoadIntent::NewPlayback | MediaLoadIntent::Replay) => {
                MediaLoadIntent::Replay
            }
            (false, MediaLoadIntent::TransportRefresh)
                if preserve_changed_identity_transport_refresh =>
            {
                MediaLoadIntent::TransportRefresh
            }
            (false, _) => MediaLoadIntent::NewPlayback,
        };
        if same_logical_media && load_intent == MediaLoadIntent::TransportRefresh {
            let ready_handoff_to_rearm = self
                .last_seek_preparation_terminal
                .as_ref()
                .filter(|terminal| {
                    terminal.terminal_outcome == Some(SeekPreparationTerminalOutcome::Ready)
                        && self.recovery.is_some()
                        && kind != MediaTransportKind::LocalFile
                })
                .cloned();
            if self
                .last_seek_preparation_terminal
                .as_ref()
                .is_some_and(|terminal| {
                    terminal.terminal_outcome != Some(SeekPreparationTerminalOutcome::Cancelled)
                })
            {
                self.last_seek_preparation_terminal = None;
            }
            if let Some(recovery) = self.recovery.as_mut() {
                recovery.post_cache_baseline_at_seconds = None;
                recovery.stable_since_seconds = None;
                recovery.catchup_deadline_seconds = None;
                recovery.decision_made = recovery.degraded;
                recovery.catchup_active = false;
                recovery.seek_active = false;
                self.request_rate_reset();
            } else {
                self.close_recovery_without_metrics();
            }
            let (media_generation, load_attempt) = {
                let media = self.media.as_mut().expect("media existence was checked");
                media.load_attempt = media.load_attempt.saturating_add(1);
                media.kind = kind;
                media.prepared_at_seconds = now_seconds;
                media.load_restart_baseline = load_restart_baseline;
                (media.generation, media.load_attempt)
            };
            self.observed = None;
            self.cached_seekable_ranges = None;
            self.metrics.steady_state_skew_seconds = None;
            self.metrics.last_buffered_ahead_seconds = None;
            self.metrics.last_input_rate_bytes_per_second = None;
            self.pending_commands.clear();
            if let Some(episode) = self.seek_preparation.as_mut() {
                episode.load_attempt = load_attempt;
                episode.primary_seek_command_id = None;
                episode.primary_seek_issued = false;
                episode.availability = SeekTargetAvailability::Unknown;
                episode.phase = SeekPreparationPhase::Seeking;
                episode.cache_buffering_percent = None;
                episode.buffered_ahead_seconds = None;
                episode.input_rate_bytes_per_second = None;
                episode.nearest_safe_buffered_position_seconds = None;
                episode.primary_seek_observation_sequence = None;
                episode.refill_started_observation_sequence = None;
                episode.refill_released_after_seek = false;
                episode.stable_playable_since = None;
                episode.deadline_seconds =
                    now_seconds + self.config.seek_preparation_timeout_seconds;
            } else if let Some(terminal) = ready_handoff_to_rearm {
                let latest_room_revision = self
                    .desired
                    .map_or(terminal.latest_room_revision, |desired| {
                        desired.state_revision
                    });
                let latest_room_position_seconds = self
                    .desired
                    .map_or(terminal.latest_room_position_seconds, |desired| {
                        desired.position_at(now_seconds)
                    });
                self.seek_preparation = Some(SeekPreparationEpisode {
                    id: terminal.id,
                    media_generation,
                    load_attempt,
                    room_revision: terminal.room_revision,
                    latest_room_revision,
                    latest_room_paused: self.desired.is_some_and(|desired| desired.paused),
                    requested_target_seconds: terminal.requested_target_seconds,
                    frozen_target_seconds: terminal.frozen_target_seconds,
                    frozen_room_anchor_position_seconds: terminal
                        .frozen_room_anchor_position_seconds,
                    frozen_room_anchor_observed_at_seconds: terminal
                        .frozen_room_anchor_observed_at_seconds,
                    latest_room_position_seconds,
                    availability: SeekTargetAvailability::Unknown,
                    phase: SeekPreparationPhase::Seeking,
                    cache_buffering_percent: None,
                    buffered_ahead_seconds: None,
                    input_rate_bytes_per_second: None,
                    nearest_safe_buffered_position_seconds: None,
                    started_at_seconds: terminal.started_at_seconds,
                    deadline_seconds: now_seconds + self.config.seek_preparation_timeout_seconds,
                    primary_seek_command_id: None,
                    primary_seek_issued: false,
                    primary_seek_observation_sequence: None,
                    refill_started_observation_sequence: None,
                    refill_released_after_seek: false,
                    stable_playable_since: None,
                });
            }
            // A refreshed URL (for example, an expiring Plex URL) represents
            // a new local transport attempt even though room identity stays
            // stable. The retained desired level must be re-observed against
            // that transport; an earlier seek/play acknowledgment is invalid.
            self.desired_seek_satisfied_revision = None;
            self.completed_seek_restart_baseline = None;
            self.last_applied_revision = None;
            self.last_started_revision = None;
            self.retry_not_before_seconds = 0.0;
            self.failed_command_attempts = 0;
            self.command_budget_degraded = false;
            self.pending_degraded_reason = None;
            if let Some(desired) = self.desired.as_mut() {
                desired.force_seek = true;
            }
            self.diagnostic = PlaybackDiagnostic::Loading;
            return MediaLoadPlan {
                media_generation,
                load_attempt,
                logical_media_changed: false,
                playback_episode_changed: false,
                load_intent,
            };
        }

        let logical_media_changed = !same_logical_media;
        self.finish_seek_preparation(SeekPreparationTerminalOutcome::Cancelled);
        self.last_seek_preparation_terminal = None;
        self.close_recovery_without_metrics();
        self.next_media_generation = self.next_media_generation.saturating_add(1).max(1);
        let generation = self.next_media_generation;
        self.media = Some(MediaState {
            logical_id,
            generation,
            load_attempt: 1,
            kind,
            prepared_at_seconds: now_seconds,
            load_restart_baseline,
        });
        self.desired = None;
        self.observed = None;
        self.cached_seekable_ranges = None;
        self.metrics.steady_state_skew_seconds = None;
        self.metrics.last_buffered_ahead_seconds = None;
        self.metrics.last_input_rate_bytes_per_second = None;
        self.pending_commands.clear();
        self.last_applied_revision = None;
        self.last_started_revision = None;
        self.desired_seek_satisfied_revision = None;
        self.required_seek_dispatch_revision = None;
        self.authoritative_alignment_guard_revision = None;
        self.completed_seek_restart_baseline = None;
        self.failed_command_attempts = 0;
        self.command_budget_degraded = false;
        self.pending_degraded_reason = None;
        self.diagnostic = PlaybackDiagnostic::Loading;
        MediaLoadPlan {
            media_generation: generation,
            load_attempt: 1,
            logical_media_changed,
            playback_episode_changed: true,
            load_intent,
        }
    }

    /// Interrupts transport recovery for a user/lifecycle action while
    /// retaining ownership of any catch-up rate until baseline telemetry is
    /// observed.
    pub fn interrupt_recovery(&mut self) -> Vec<PlaybackCoordinatorAction> {
        self.cancel_seek_preparation_for_lifecycle();
        if self.degraded_seek_preparation_holds_current_revision() {
            self.clear_seek_preparation_terminal();
            // Clearing a degraded hold is a lifecycle/user supersession, not
            // proof that the old canonical target was reached. Invalidate the
            // old desire so its synthetic "satisfied" disposition cannot
            // start playback at the wrong position before fresh room state is
            // installed.
            self.desired = None;
            self.desired_seek_satisfied_revision = None;
            self.required_seek_dispatch_revision = None;
        }
        self.close_recovery_without_metrics();
        let mut actions = Vec::new();
        if let Some(observed) = self.observed {
            self.reconcile_rate_override(observed, observed.observed_at_seconds, &mut actions);
        }
        actions
    }

    pub fn current_media_generation(&self) -> Option<u64> {
        self.media.as_ref().map(|media| media.generation)
    }

    pub fn current_load_attempt(&self) -> Option<u64> {
        self.media.as_ref().map(|media| media.load_attempt)
    }

    pub fn current_logical_media_id(&self) -> Option<&LogicalMediaId> {
        self.media.as_ref().map(|media| &media.logical_id)
    }

    pub fn current_media_transport_kind(&self) -> Option<MediaTransportKind> {
        self.media.as_ref().map(|media| media.kind)
    }

    /// Retires the current logical media generation without retiring the
    /// player attachment. Late observations and command completions from the
    /// old generation can no longer satisfy future room authority.
    pub fn retire_media(&mut self) -> Vec<PlaybackCoordinatorAction> {
        let actions = self.interrupt_recovery();
        self.finish_seek_preparation(SeekPreparationTerminalOutcome::Cancelled);
        self.last_seek_preparation_terminal = None;
        self.media = None;
        self.desired = None;
        self.observed = None;
        self.cached_seekable_ranges = None;
        self.pending_commands.clear();
        self.last_applied_revision = None;
        self.last_started_revision = None;
        self.desired_seek_satisfied_revision = None;
        self.required_seek_dispatch_revision = None;
        self.authoritative_alignment_guard_revision = None;
        self.completed_seek_restart_baseline = None;
        self.rate_override = None;
        self.last_playback_rate_observation_sequence = None;
        self.retry_not_before_seconds = 0.0;
        self.failed_command_attempts = 0;
        self.command_budget_degraded = false;
        self.pending_degraded_reason = None;
        self.clear_participant_status_transport_metrics();
        self.diagnostic = PlaybackDiagnostic::Empty;
        actions
    }

    pub fn update_desired_room_state(
        &mut self,
        desired: DesiredRoomPlayback,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.update_desired_room_state_with_kind(desired, DesiredRoomPlaybackUpdateKind::Ordinary)
    }

    pub fn update_desired_room_state_with_kind(
        &mut self,
        mut desired: DesiredRoomPlayback,
        update_kind: DesiredRoomPlaybackUpdateKind,
    ) -> Vec<PlaybackCoordinatorAction> {
        if self.current_media_generation() != Some(desired.media_generation)
            || !desired.anchor_position_seconds.is_finite()
            || !desired.anchor_observed_at_seconds.is_finite()
        {
            return Vec::new();
        }
        if self
            .desired
            .is_some_and(|current| desired.state_revision < current.state_revision)
        {
            return Vec::new();
        }

        let revision_changed = self
            .desired
            .is_none_or(|current| current.state_revision != desired.state_revision);
        let explicit_seek = revision_changed
            && matches!(
                update_kind,
                DesiredRoomPlaybackUpdateKind::ExplicitSeek
                    | DesiredRoomPlaybackUpdateKind::ExplicitSeekAlreadyDispatched
            );
        let replacement_seek = explicit_seek || desired.force_seek;
        let replacement_supersedes_dispatched_seek = replacement_seek
            && (self
                .seek_preparation
                .as_ref()
                .is_some_and(|episode| episode.primary_seek_issued)
                || self.pending_commands.iter().any(|command| {
                    command.revision < desired.state_revision
                        && matches!(command.kind, PendingCommandKind::Seek { .. })
                }));
        if revision_changed {
            // A room-position skew is meaningful only for the desired
            // revision against which it was measured. Cached observations may
            // be replayed while adopting a newer revision, so discard the old
            // value before any replay and require a fresh transport sample to
            // establish the replacement measurement.
            self.metrics.steady_state_skew_seconds = None;
            let guards_superseded_dispatch = update_kind
                == DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch
                || replacement_supersedes_dispatched_seek;
            let carries_supersession_guard = !guards_superseded_dispatch
                && update_kind == DesiredRoomPlaybackUpdateKind::Ordinary
                && self.authoritative_alignment_guard_revision.is_some();
            if guards_superseded_dispatch || carries_supersession_guard {
                // Preparing -> Committed and quorum/status revisions must not
                // drop the fence while an older async seek can still land.
                desired.force_seek = true;
                self.required_seek_dispatch_revision = Some(desired.state_revision);
                self.authoritative_alignment_guard_revision = Some(desired.state_revision);
            } else {
                self.required_seek_dispatch_revision = None;
                self.authoritative_alignment_guard_revision = None;
            }
        }
        if explicit_seek {
            if self.seek_preparation.is_none() {
                self.last_seek_preparation_terminal = None;
            }
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Superseded);
            self.begin_seek_preparation_if_needed(desired);
            if update_kind == DesiredRoomPlaybackUpdateKind::ExplicitSeekAlreadyDispatched
                && self.seek_preparation.is_some()
            {
                self.mark_seek_preparation_primary_issued(None);
            }
        } else if let Some(episode) = self.seek_preparation.as_mut() {
            episode.latest_room_revision = desired.state_revision;
            episode.latest_room_paused = desired.paused;
            episode.latest_room_position_seconds =
                desired.position_at(desired.anchor_observed_at_seconds);
        }
        let terminal_hold_rebound = revision_changed
            && update_kind == DesiredRoomPlaybackUpdateKind::Ordinary
            && self.seek_preparation.is_none()
            && self
                .last_seek_preparation_terminal
                .as_mut()
                .is_some_and(|terminal| {
                    let holds = terminal.media_generation == desired.media_generation
                        && matches!(
                            terminal.terminal_outcome,
                            Some(SeekPreparationTerminalOutcome::Cancelled)
                                | Some(SeekPreparationTerminalOutcome::Degraded(_))
                        );
                    if holds {
                        terminal.latest_room_revision = desired.state_revision;
                        terminal.latest_room_position_seconds =
                            desired.position_at(desired.anchor_observed_at_seconds);
                    }
                    holds
                });
        if revision_changed && self.seek_preparation.is_none() && !terminal_hold_rebound {
            let terminal_is_older =
                self.last_seek_preparation_terminal
                    .as_ref()
                    .is_some_and(|terminal| {
                        terminal.media_generation == desired.media_generation
                            && terminal.latest_room_revision < desired.state_revision
                    });
            if terminal_is_older {
                self.last_seek_preparation_terminal = None;
            }
        }
        if revision_changed {
            let preparation_seek_command = self
                .seek_preparation
                .as_ref()
                .and_then(|episode| episode.primary_seek_command_id);
            self.pending_commands.retain(|command| {
                command.revision >= desired.state_revision
                    || preparation_seek_command == Some(command.id)
            });
            let terminal_seek_held =
                self.last_seek_preparation_terminal
                    .as_ref()
                    .is_some_and(|terminal| {
                        terminal.media_generation == desired.media_generation
                            && terminal.latest_room_revision == desired.state_revision
                            && matches!(
                                terminal.terminal_outcome,
                                Some(SeekPreparationTerminalOutcome::Cancelled)
                                    | Some(SeekPreparationTerminalOutcome::Degraded(_))
                            )
                    });
            self.desired_seek_satisfied_revision = if self.seek_preparation.is_some() {
                None
            } else if terminal_seek_held {
                Some(desired.state_revision)
            } else {
                (!desired.force_seek).then_some(desired.state_revision)
            };
            if desired.force_seek {
                self.completed_seek_restart_baseline = None;
                self.close_recovery_without_metrics();
            }
            self.retry_not_before_seconds = 0.0;
            self.failed_command_attempts = 0;
            self.command_budget_degraded = false;
            self.pending_degraded_reason = None;
        }
        if desired.paused {
            self.close_recovery_without_metrics();
        }
        self.desired = Some(desired);
        let mut actions = Vec::new();
        if let Some(observed) = self.observed {
            self.reconcile_rate_override(observed, observed.observed_at_seconds, &mut actions);
        }
        actions
    }

    pub fn observe(
        &mut self,
        observation: PlayerTransportObservation,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.observe_with_seek_preparation_evidence(observation, true, false)
    }

    pub(crate) fn rebase_observation(
        &mut self,
        observation: PlayerTransportObservation,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.observe_with_seek_preparation_evidence(observation, true, true)
    }

    #[cfg(test)]
    pub(crate) fn observed_transport_for_test(&self) -> Option<PlayerTransportObservation> {
        let observed = self.observed?;
        Some(PlayerTransportObservation {
            media_generation: self.current_media_generation()?,
            observed_at_seconds: observed.observed_at_seconds,
            phase: Some(observed.phase),
            position_seconds: observed.position_seconds,
            playback_rate: observed.playback_rate,
            logical_pause: observed.logical_pause,
            paused_for_cache: Some(observed.paused_for_cache),
            seeking: Some(observed.seeking),
            seekable: observed.seekable,
            timeline_kind: observed.timeline_kind,
            seekable_ranges: self.cached_seekable_ranges.clone(),
            known_live_seekable_window: observed.known_live_seekable_window,
            core_idle: observed.core_idle,
            playback_restart_sequence: Some(observed.playback_restart_sequence),
            cache_buffering_percent: observed.cache_buffering_percent,
            buffered_ahead_seconds: observed.buffered_ahead_seconds,
            input_rate_bytes_per_second: None,
        })
    }

    pub(crate) fn replay_observation(
        &mut self,
        observation: PlayerTransportObservation,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.observe_with_seek_preparation_evidence(observation, false, false)
    }

    fn observe_with_seek_preparation_evidence(
        &mut self,
        observation: PlayerTransportObservation,
        seek_preparation_evidence_is_fresh: bool,
        replace_previous_state: bool,
    ) -> Vec<PlaybackCoordinatorAction> {
        let Some(media_generation) = self.media.as_ref().map(|media| media.generation) else {
            return Vec::new();
        };
        if observation.media_generation != media_generation {
            self.metrics.stale_generation_observations =
                self.metrics.stale_generation_observations.saturating_add(1);
            return Vec::new();
        }
        if !observation.observed_at_seconds.is_finite()
            || self.observed.is_some_and(|observed| {
                observation.observed_at_seconds < observed.observed_at_seconds
            })
        {
            self.metrics.stale_timestamp_observations =
                self.metrics.stale_timestamp_observations.saturating_add(1);
            return Vec::new();
        }
        if observation.timeline_kind == Some(PlayerTimelineKind::SlidingLive)
            && let Some(media) = self.media.as_mut()
            && media.kind == MediaTransportKind::NetworkVod
        {
            // Production opens remote sources conservatively as VOD. Promote
            // only on fresh positive player evidence; sparse/missing telemetry
            // must never demote a confirmed sliding source or invent one.
            media.kind = MediaTransportKind::LiveSliding;
        }
        self.observation_sequence = self.observation_sequence.saturating_add(1);

        let position_sampled = observation
            .position_seconds
            .is_some_and(|value| value.is_finite() && value >= 0.0);
        let previous = (!replace_previous_state).then_some(self.observed).flatten();
        if replace_previous_state {
            self.cached_seekable_ranges = None;
            self.last_playback_rate_observation_sequence = None;
            self.metrics.last_buffered_ahead_seconds = None;
            self.metrics.last_input_rate_bytes_per_second = None;
        }
        let mut observed = previous.unwrap_or(ObservedState {
            observed_at_seconds: observation.observed_at_seconds,
            phase: PlayerTransportPhase::Empty,
            position_seconds: None,
            playback_rate: None,
            logical_pause: None,
            paused_for_cache: false,
            seeking: false,
            seekable: None,
            seekable_window: None,
            timeline_kind: None,
            known_live_seekable_window: None,
            core_idle: None,
            playback_restart_sequence: 0,
            cache_buffering_percent: None,
            buffered_ahead_seconds: None,
        });
        observed.observed_at_seconds = observation.observed_at_seconds;
        if let Some(value) = observation.phase {
            observed.phase = value;
        }
        if let Some(value) = observation
            .position_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            observed.position_seconds = Some(value);
        }
        if let Some(value) = observation
            .playback_rate
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            observed.playback_rate = Some(value);
            self.last_playback_rate_observation_sequence = Some(self.observation_sequence);
        }
        if let Some(value) = observation.logical_pause {
            observed.logical_pause = Some(value);
        }
        if let Some(value) = observation.paused_for_cache {
            observed.paused_for_cache = value;
        }
        if let Some(value) = observation.seeking {
            observed.seeking = value;
        }
        if let Some(value) = observation.seekable {
            observed.seekable = Some(value);
        }
        if let Some(value) = observation.timeline_kind {
            observed.timeline_kind = Some(value);
        }
        if let Some(ranges) = observation.seekable_ranges.as_deref() {
            let normalized = normalize_seekable_ranges(ranges);
            observed.seekable_window = latest_valid_seekable_window(&normalized);
            self.cached_seekable_ranges = Some(normalized);
        }
        let known_live_window = observation.known_live_seekable_window.filter(|window| {
            window.start_seconds.is_finite()
                && window.end_seconds.is_finite()
                && window.start_seconds >= 0.0
                && window.end_seconds >= window.start_seconds
        });
        if observation.timeline_kind == Some(PlayerTimelineKind::SlidingLive)
            && observation.seekable_ranges.is_some()
        {
            // A cache-state snapshot is complete for the adapter's locally
            // usable interval. An explicit empty/invalid snapshot clears the
            // prior interval instead of retaining a stale live target.
            observed.known_live_seekable_window = known_live_window;
        } else if let Some(window) = known_live_window {
            observed.known_live_seekable_window = Some(window);
        }
        if let Some(value) = observation.core_idle {
            observed.core_idle = Some(value);
        }
        if let Some(value) = observation.playback_restart_sequence {
            observed.playback_restart_sequence = value;
        }
        if let Some(value) = observation
            .cache_buffering_percent
            .filter(|value| value.is_finite())
        {
            observed.cache_buffering_percent = Some(value.clamp(0.0, 100.0));
        }
        if let Some(value) = observation
            .buffered_ahead_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            observed.buffered_ahead_seconds = Some(value);
        }
        self.observed = Some(observed);

        let target_scoped_cache_evidence = self.seek_preparation_accepts_target_cache_evidence(
            observed,
            &observation,
            seek_preparation_evidence_is_fresh,
        );
        let recovery_cache_metrics_frozen = self.recovery.as_ref().is_some_and(|episode| {
            episode.cache_metrics_frozen_until_decision && !episode.decision_made
        });
        let cache_metrics_update_allowed = seek_preparation_evidence_is_fresh
            && !recovery_cache_metrics_frozen
            && self
                .seek_preparation
                .as_ref()
                .is_none_or(|episode| !episode.primary_seek_issued);
        if cache_metrics_update_allowed {
            self.metrics.last_buffered_ahead_seconds = observation
                .buffered_ahead_seconds
                .filter(|value| value.is_finite() && *value >= 0.0)
                .or(self.metrics.last_buffered_ahead_seconds);
            self.metrics.last_input_rate_bytes_per_second = observation
                .input_rate_bytes_per_second
                .or(self.metrics.last_input_rate_bytes_per_second);
        }

        let advancing = previous
            .and_then(|previous| previous.position_seconds)
            .zip(observed.position_seconds)
            .is_some_and(|(previous, current)| current - previous > MIN_ADVANCEMENT_SECONDS)
            && observed.logical_pause == Some(false)
            && !observed.paused_for_cache
            && !observed.seeking;

        self.update_diagnostic_for_observation(observed);
        let mut actions = self.take_pending_actions();
        self.complete_observed_commands(observed, advancing);
        self.update_seek_preparation(
            observed,
            &observation,
            seek_preparation_evidence_is_fresh,
            target_scoped_cache_evidence,
            &mut actions,
        );
        if self.seek_preparation.is_none() {
            let recovery_degraded =
                self.update_recovery_episode(observed, advancing, position_sampled);
            if let Some(reason) = recovery_degraded {
                self.enter_degraded_recovery(reason, &mut actions);
            }
        }
        self.coordinate_desired_state(
            observed,
            &observation,
            seek_preparation_evidence_is_fresh,
            advancing,
            &mut actions,
        );
        self.reconcile_rate_override(observed, observed.observed_at_seconds, &mut actions);
        actions
    }

    pub fn command_accepted(&mut self, command_id: CoordinatorCommandId) -> bool {
        let Some(command) = self
            .pending_commands
            .iter_mut()
            .find(|command| command.id == command_id)
        else {
            return false;
        };
        command.accepted = true;
        true
    }

    /// Retires a command that was produced by reconciliation but superseded
    /// before the owning runtime dispatched it. Accepted commands must remain
    /// tracked until their transport completion arrives.
    pub(crate) fn supersede_unaccepted_command(
        &mut self,
        command_id: CoordinatorCommandId,
    ) -> bool {
        let Some(command) = self
            .pending_commands
            .iter()
            .find(|command| command.id == command_id)
        else {
            return false;
        };
        if command.accepted {
            return false;
        }
        self.pending_commands
            .retain(|command| command.id != command_id);
        true
    }

    pub(crate) fn pending_command_pause_target(
        &self,
        command_id: CoordinatorCommandId,
    ) -> Option<bool> {
        let command = self
            .pending_commands
            .iter()
            .find(|command| command.id == command_id)?;
        match command.kind {
            PendingCommandKind::Pause => Some(true),
            PendingCommandKind::Play { .. } => Some(false),
            PendingCommandKind::Seek { .. } | PendingCommandKind::Rate { .. } => None,
        }
    }

    pub(crate) fn active_seek_preparation_lost_command_tracking(
        &self,
        command_id: CoordinatorCommandId,
    ) -> bool {
        self.seek_preparation
            .as_ref()
            .is_some_and(|episode| episode.primary_seek_command_id == Some(command_id))
            && self
                .pending_commands
                .iter()
                .any(|command| command.id == command_id)
    }

    pub fn command_failed(&mut self, command_id: CoordinatorCommandId, now_seconds: f64) -> bool {
        if !self
            .pending_commands
            .iter()
            .any(|command| command.id == command_id)
        {
            return false;
        }
        let preparation_failed = self
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| episode.primary_seek_command_id == Some(command_id));
        if preparation_failed {
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::TransportFailed,
            ));
            self.desired_seek_satisfied_revision = current_revision;
            self.diagnostic = PlaybackDiagnostic::Degraded;
            self.pending_degraded_reason = Some(DegradedPlaybackReason::TransportFailed);
        } else {
            self.pending_commands
                .retain(|command| command.id != command_id);
        }
        self.failed_command_attempts = self.failed_command_attempts.saturating_add(1);
        self.retry_not_before_seconds = now_seconds + self.config.command_retry_cooldown_seconds;
        true
    }

    pub(crate) fn take_pending_actions(&mut self) -> Vec<PlaybackCoordinatorAction> {
        self.pending_degraded_reason
            .take()
            .map(|reason| PlaybackCoordinatorAction::Degraded {
                media_generation: self.current_media_generation().unwrap_or_default(),
                recovery_episode_id: None,
                reason,
            })
            .into_iter()
            .collect()
    }

    pub fn tick(&mut self, now_seconds: f64) -> Vec<PlaybackCoordinatorAction> {
        if !now_seconds.is_finite() {
            return Vec::new();
        }
        let mut actions = self.take_pending_actions();
        if self
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| now_seconds >= episode.deadline_seconds)
        {
            let timeline_window_unavailable =
                self.seek_preparation.as_ref().is_some_and(|episode| {
                    !episode.primary_seek_issued
                        && self.observed.is_some_and(|observed| {
                            observed.timeline_kind == Some(PlayerTimelineKind::Unknown)
                                || (self.media.as_ref().is_some_and(|media| {
                                    media.kind == MediaTransportKind::LiveSliding
                                }) && observed.known_live_seekable_window.is_none())
                        })
                });
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                if timeline_window_unavailable {
                    SeekPreparationDegradedReason::TimelineWindowUnavailable
                } else {
                    SeekPreparationDegradedReason::TimedOut
                },
            ));
            self.desired_seek_satisfied_revision = current_revision;
            self.diagnostic = PlaybackDiagnostic::Degraded;
            actions.push(PlaybackCoordinatorAction::Degraded {
                media_generation: self.current_media_generation().unwrap_or_default(),
                recovery_episode_id: None,
                reason: if timeline_window_unavailable {
                    DegradedPlaybackReason::TimelineWindowUnavailable
                } else {
                    DegradedPlaybackReason::RecoveryCommandTimedOut
                },
            });
        }
        if self
            .observed
            .is_some_and(|observed| observed.paused_for_cache)
        {
            for command in &mut self.pending_commands {
                if matches!(command.kind, PendingCommandKind::Pause) {
                    // mpv intentionally masks logical pause while cache pause
                    // is active. Keep the command observation-backed without
                    // charging that unobservable interval to its timeout.
                    command.issued_at_seconds = now_seconds;
                }
            }
        }
        // An unbuffered seek is observation-backed like every other player
        // command, but its completion may legitimately take the full fetch
        // window. The preparation deadline (which Keep waiting can extend)
        // is its timeout boundary; the generic command timeout would turn a
        // normal slow fetch into an early terminal failure.
        let preparation_seek_command_id = self
            .seek_preparation
            .as_ref()
            .and_then(|episode| episode.primary_seek_command_id);
        let timed_out = self
            .pending_commands
            .iter()
            .filter(|command| {
                Some(command.id) != preparation_seek_command_id
                    && now_seconds - command.issued_at_seconds
                        >= self.config.command_timeout_seconds
            })
            .map(|command| command.id)
            .collect::<Vec<_>>();
        if !timed_out.is_empty() {
            self.pending_commands
                .retain(|command| !timed_out.contains(&command.id));
            self.metrics.command_timeouts = self
                .metrics
                .command_timeouts
                .saturating_add(timed_out.len() as u64);
            self.failed_command_attempts = self
                .failed_command_attempts
                .saturating_add(timed_out.len() as u32);
            self.retry_not_before_seconds =
                now_seconds + self.config.command_retry_cooldown_seconds;
        }
        actions.extend(
            timed_out
                .into_iter()
                .map(|command_id| PlaybackCoordinatorAction::CommandTimedOut { command_id }),
        );
        if self.failed_command_attempts > self.config.command_retry_budget
            && !self.command_budget_degraded
        {
            let generation = self.current_media_generation().unwrap_or_default();
            let episode = self.recovery.as_ref().map(|episode| episode.id);
            self.metrics.degraded_recovery_count =
                self.metrics.degraded_recovery_count.saturating_add(1);
            actions.push(PlaybackCoordinatorAction::Degraded {
                media_generation: generation,
                recovery_episode_id: episode,
                reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            });
            self.diagnostic = PlaybackDiagnostic::Degraded;
            self.command_budget_degraded = true;
            if let Some(recovery) = self.recovery.as_mut() {
                recovery.degraded = true;
            }
            // A recovery command budget is allowed to stop further recovery
            // attempts, but it must never strand a coordinator-owned speed
            // override on the player. Baseline restoration has a separate
            // safety obligation and is retried outside that budget.
            self.request_rate_reset();
        }
        if let Some(observed) = self.observed {
            self.reconcile_rate_override(observed, now_seconds, &mut actions);
        }
        actions
    }

    pub fn diagnostic(&self) -> PlaybackDiagnostic {
        self.diagnostic
    }

    pub fn metrics(&self) -> &PlaybackCoordinatorMetrics {
        &self.metrics
    }

    pub(crate) fn clear_participant_status_transport_metrics(&mut self) {
        self.metrics.steady_state_skew_seconds = None;
        self.metrics.last_buffered_ahead_seconds = None;
        self.metrics.last_input_rate_bytes_per_second = None;
    }

    pub(crate) fn expire_transport_observation(&mut self) {
        self.observed = None;
        self.cached_seekable_ranges = None;
        self.clear_participant_status_transport_metrics();
    }

    #[cfg(test)]
    pub(crate) fn set_steady_state_skew_seconds_for_test(&mut self, skew_seconds: f64) {
        self.metrics.steady_state_skew_seconds = Some(skew_seconds);
    }

    pub fn recovery_episode(&self) -> Option<RecoveryEpisodeSnapshot> {
        self.recovery
            .as_ref()
            .map(|episode| RecoveryEpisodeSnapshot {
                id: episode.id,
                media_generation: episode.media_generation,
                entered_at_seconds: episode.entered_at_seconds,
                hard_seek_attempts: episode.hard_seek_attempts,
                stable_since_seconds: episode.stable_since_seconds,
                catchup_deadline_seconds: episode.catchup_deadline_seconds,
                degraded: episode.degraded,
            })
    }

    pub fn desired_revision_pending(&self) -> Option<u64> {
        let desired = self.desired?;
        (self.last_applied_revision != Some(desired.state_revision))
            .then_some(desired.state_revision)
    }

    /// Whether the transport/recovery lifecycle currently owns correction.
    ///
    /// Callers may retain their legacy steady-state drift policy while this is
    /// false. Loading, buffering, seeking, and the full recovery stability
    /// interval must remain exclusively owned by this coordinator.
    pub fn ordinary_correction_blocked(&self) -> bool {
        self.seek_preparation.is_some()
            || self.terminal_seek_preparation_holds_current_revision()
            || self.recovery.is_some()
            || self.rate_override.is_some()
            || !self.pending_commands.is_empty()
            || self.desired_revision_pending().is_some()
            || self
                .observed
                .is_some_and(|observed| self.transport_blocks_correction(observed))
    }

    fn degraded_seek_preparation_holds_current_revision(&self) -> bool {
        let Some(desired) = self.desired else {
            return false;
        };
        self.last_seek_preparation_terminal
            .as_ref()
            .is_some_and(|terminal| {
                terminal.media_generation == desired.media_generation
                    && terminal.latest_room_revision == desired.state_revision
                    && matches!(
                        terminal.terminal_outcome,
                        Some(SeekPreparationTerminalOutcome::Degraded(_))
                    )
            })
    }

    fn terminal_seek_preparation_holds_current_revision(&self) -> bool {
        let Some(desired) = self.desired else {
            return false;
        };
        self.last_seek_preparation_terminal
            .as_ref()
            .is_some_and(|terminal| {
                terminal.media_generation == desired.media_generation
                    && terminal.latest_room_revision == desired.state_revision
                    && matches!(
                        terminal.terminal_outcome,
                        Some(SeekPreparationTerminalOutcome::Cancelled)
                            | Some(SeekPreparationTerminalOutcome::Degraded(_))
                    )
            })
    }

    fn authoritative_alignment_guards_active_preparation(&self) -> bool {
        self.seek_preparation.as_ref().is_some_and(|episode| {
            self.authoritative_alignment_guard_revision == Some(episode.latest_room_revision)
        })
    }

    pub fn seek_preparation_snapshot(&self) -> Option<SeekPreparationSnapshot> {
        if let Some(episode) = self.seek_preparation.as_ref() {
            let mut snapshot = episode.snapshot(None);
            if self.authoritative_alignment_guards_active_preparation() {
                // A guarded revision must reach its canonical target. Offering
                // a local alternative would only cause the guard to reissue
                // the original seek as soon as the alternative completed.
                snapshot.can_join_nearest_buffered = false;
            }
            return Some(snapshot);
        }
        let mut terminal = self.last_seek_preparation_terminal.clone()?;
        let recovery = self.recovery.as_ref();
        let visible_handoff = match terminal.terminal_outcome {
            Some(SeekPreparationTerminalOutcome::Ready) => {
                self.desired_revision_pending().is_some() || recovery.is_some()
            }
            Some(SeekPreparationTerminalOutcome::Superseded) => recovery.is_some(),
            _ => false,
        };
        let transport_can_join = self.observed.is_some_and(|observed| {
            matches!(
                observed.phase,
                PlayerTransportPhase::ReadyPaused | PlayerTransportPhase::Playing
            ) && !observed.paused_for_cache
                && !observed.seeking
        });
        if !visible_handoff
            || !transport_can_join
            || recovery.is_some_and(|episode| episode.degraded)
        {
            return None;
        }
        terminal.phase = if recovery.is_some_and(|episode| {
            episode.catchup_active || episode.seek_active || episode.gentle_catchup_only
        }) {
            SeekPreparationPhase::CatchingUp
        } else {
            SeekPreparationPhase::ReadyToJoin
        };
        Some(terminal)
    }

    pub fn last_seek_preparation_terminal_outcome(&self) -> Option<SeekPreparationTerminalOutcome> {
        self.last_seek_preparation_terminal
            .as_ref()
            .and_then(|snapshot| snapshot.terminal_outcome)
    }

    pub fn last_seek_preparation_terminal_snapshot(&self) -> Option<SeekPreparationSnapshot> {
        self.last_seek_preparation_terminal.clone()
    }

    pub fn keep_waiting_for_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if let Some(episode) = self.seek_preparation.as_mut()
            && now_seconds.is_finite()
        {
            episode.deadline_seconds = now_seconds + self.config.seek_preparation_timeout_seconds;
        }
        Vec::new()
    }

    pub fn cancel_seek_preparation(&mut self, _now_seconds: f64) -> Vec<PlaybackCoordinatorAction> {
        if self
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| !episode.primary_seek_issued)
        {
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Cancelled);
            self.desired_seek_satisfied_revision = current_revision;
        }
        Vec::new()
    }

    /// Cancels preparation for an authoritative lifecycle supersession.
    ///
    /// Unlike the user-facing "cancel and remain" action, room, media, and
    /// manual-control transitions may invalidate a frozen anchor after the
    /// low-level seek was already dispatched.
    pub fn cancel_seek_preparation_for_lifecycle(&mut self) -> bool {
        let Some(primary_seek_issued) = self
            .seek_preparation
            .as_ref()
            .map(|episode| episode.primary_seek_issued)
        else {
            return false;
        };
        let current_revision = self.desired.map(|desired| desired.state_revision);
        self.finish_seek_preparation(SeekPreparationTerminalOutcome::Cancelled);
        self.desired_seek_satisfied_revision = current_revision;
        primary_seek_issued
    }

    pub fn clear_seek_preparation_terminal(&mut self) {
        self.last_seek_preparation_terminal = None;
    }

    pub fn join_nearest_buffered_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if !now_seconds.is_finite()
            || self.desired.is_none_or(|desired| desired.paused)
            || self.authoritative_alignment_guards_active_preparation()
        {
            return Vec::new();
        }
        let Some((target, revision, restart_sequence)) = self
            .seek_preparation
            .as_ref()
            .and_then(|episode| {
                (!episode.latest_room_paused)
                    .then_some(episode.nearest_safe_buffered_position_seconds)
                    .flatten()
                    .map(|target| (target, episode.latest_room_revision))
            })
            .and_then(|(target, revision)| {
                self.observed
                    .map(|observed| (target, revision, observed.playback_restart_sequence))
            })
        else {
            return Vec::new();
        };
        self.finish_seek_preparation(SeekPreparationTerminalOutcome::Superseded);
        self.desired_seek_satisfied_revision = None;
        let mut actions = Vec::new();
        let command_id = self.issue_command(
            revision,
            now_seconds,
            PendingCommandKind::Seek {
                target_position_seconds: target,
                baseline_restart_sequence: restart_sequence,
            },
            CoordinatorPlayerCommand::SetPosition(target),
            &mut actions,
        );
        if command_id.is_some() {
            self.begin_recovery_handoff(now_seconds);
            if let Some(recovery) = self.recovery.as_mut() {
                // The user explicitly chose this nearby cached point to avoid
                // another unbuffered hard seek. Converge at the conservative
                // catch-up rate regardless of later cache telemetry; if that
                // cannot succeed, degrade instead of jumping straight back
                // into the same buffer loop.
                recovery.gentle_catchup_only = true;
            }
        } else {
            if let Some(terminal) = self.last_seek_preparation_terminal.as_mut() {
                terminal.terminal_outcome = Some(SeekPreparationTerminalOutcome::Degraded(
                    SeekPreparationDegradedReason::TransportFailed,
                ));
            }
            self.desired_seek_satisfied_revision = Some(revision);
            self.diagnostic = PlaybackDiagnostic::Degraded;
            actions.push(PlaybackCoordinatorAction::Degraded {
                media_generation: self.current_media_generation().unwrap_or_default(),
                recovery_episode_id: None,
                reason: DegradedPlaybackReason::TransportFailed,
            });
        }
        actions
    }

    fn begin_seek_preparation_if_needed(&mut self, desired: DesiredRoomPlayback) {
        let Some(media) = self.media.as_ref() else {
            return;
        };
        if media.kind == MediaTransportKind::LocalFile {
            return;
        }
        let requested_target = desired.position_at(desired.anchor_observed_at_seconds);
        let availability = classify_seek_target(
            media.kind,
            self.observed.and_then(|observed| observed.seekable),
            self.cached_seekable_ranges.as_deref(),
            self.observed
                .and_then(|observed| observed.known_live_seekable_window),
            requested_target,
        );
        if availability == SeekTargetAvailability::Cached {
            return;
        }
        self.next_seek_preparation_id = self.next_seek_preparation_id.saturating_add(1).max(1);
        let frozen_target = if media.kind == MediaTransportKind::LiveSliding {
            clamp_to_live_seekable_window(
                requested_target,
                self.observed
                    .and_then(|observed| observed.known_live_seekable_window),
            )
            .unwrap_or(requested_target)
        } else {
            requested_target
        };
        let nearest = matches!(
            availability,
            SeekTargetAvailability::FetchRequired | SeekTargetAvailability::Unknown
        )
        .then(|| {
            nearest_buffered_target(
                media.kind,
                requested_target,
                self.cached_seekable_ranges.as_deref(),
                self.config.nearest_buffered_target_limit_seconds,
            )
        })
        .flatten();
        self.seek_preparation = Some(SeekPreparationEpisode {
            id: self.next_seek_preparation_id,
            media_generation: media.generation,
            load_attempt: media.load_attempt,
            room_revision: desired.state_revision,
            latest_room_revision: desired.state_revision,
            latest_room_paused: desired.paused,
            requested_target_seconds: requested_target,
            frozen_target_seconds: frozen_target,
            frozen_room_anchor_position_seconds: desired.anchor_position_seconds,
            frozen_room_anchor_observed_at_seconds: desired.anchor_observed_at_seconds,
            latest_room_position_seconds: requested_target,
            availability,
            phase: SeekPreparationPhase::Seeking,
            // Refill/headroom samples are target-scoped. Values observed at
            // the pre-seek position must not be presented as progress for
            // this frozen target.
            cache_buffering_percent: None,
            buffered_ahead_seconds: None,
            input_rate_bytes_per_second: None,
            nearest_safe_buffered_position_seconds: nearest,
            started_at_seconds: desired.anchor_observed_at_seconds,
            deadline_seconds: desired.anchor_observed_at_seconds
                + self.config.seek_preparation_timeout_seconds,
            primary_seek_command_id: None,
            primary_seek_issued: false,
            primary_seek_observation_sequence: None,
            refill_started_observation_sequence: None,
            refill_released_after_seek: false,
            stable_playable_since: None,
        });
        self.close_recovery_without_metrics();
        if availability == SeekTargetAvailability::NonSeekable {
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::NonSeekable,
            ));
        } else if availability == SeekTargetAvailability::OutsideLiveWindow
            && self
                .cached_seekable_ranges
                .as_ref()
                .is_some_and(Vec::is_empty)
        {
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::OutsideLiveWindow,
            ));
        }
    }

    fn update_seek_preparation(
        &mut self,
        observed: ObservedState,
        observation: &PlayerTransportObservation,
        seek_preparation_evidence_is_fresh: bool,
        target_scoped_cache_evidence: bool,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        let Some(media_kind) = self.media.as_ref().map(|media| media.kind) else {
            return;
        };
        let Some(current) = self.seek_preparation.as_ref() else {
            return;
        };
        if matches!(
            observed.phase,
            PlayerTransportPhase::Ended | PlayerTransportPhase::Failed
        ) {
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::TransportFailed,
            ));
            self.desired_seek_satisfied_revision = current_revision;
            actions.push(PlaybackCoordinatorAction::Degraded {
                media_generation: self.current_media_generation().unwrap_or_default(),
                recovery_episode_id: None,
                reason: DegradedPlaybackReason::TransportFailed,
            });
            return;
        }
        let requested_target = current.requested_target_seconds;
        let latest_room_position = current.latest_room_position_seconds;
        let availability = classify_seek_target(
            media_kind,
            observed.seekable,
            self.cached_seekable_ranges.as_deref(),
            observed.known_live_seekable_window,
            requested_target,
        );
        if availability == SeekTargetAvailability::NonSeekable {
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::NonSeekable,
            ));
            self.desired_seek_satisfied_revision = current_revision;
            actions.push(PlaybackCoordinatorAction::Degraded {
                media_generation: self.current_media_generation().unwrap_or_default(),
                recovery_episode_id: None,
                reason: DegradedPlaybackReason::NonSeekableLag,
            });
            return;
        }
        if availability == SeekTargetAvailability::OutsideLiveWindow
            && self
                .cached_seekable_ranges
                .as_ref()
                .is_some_and(Vec::is_empty)
        {
            let current_revision = self.desired.map(|desired| desired.state_revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Degraded(
                SeekPreparationDegradedReason::OutsideLiveWindow,
            ));
            self.desired_seek_satisfied_revision = current_revision;
            return;
        }

        let nearest = matches!(
            availability,
            SeekTargetAvailability::FetchRequired | SeekTargetAvailability::Unknown
        )
        .then(|| {
            nearest_buffered_target(
                media_kind,
                requested_target,
                self.cached_seekable_ranges.as_deref(),
                self.config.nearest_buffered_target_limit_seconds,
            )
        })
        .flatten()
        .filter(|candidate| {
            *candidate <= latest_room_position + self.config.position_tolerance_seconds
                && latest_room_position - *candidate
                    <= self.config.nearest_buffered_target_limit_seconds
        });
        let Some(episode) = self.seek_preparation.as_mut() else {
            return;
        };
        episode.availability = availability;
        episode.nearest_safe_buffered_position_seconds = nearest;
        if media_kind == MediaTransportKind::LiveSliding
            && availability == SeekTargetAvailability::OutsideLiveWindow
            && !episode.primary_seek_issued
            && let Some(clamped) =
                clamp_to_live_seekable_window(requested_target, observed.known_live_seekable_window)
        {
            episode.frozen_target_seconds = clamped;
        }

        let at_target = observed.position_seconds.is_some_and(|position| {
            (position - episode.frozen_target_seconds).abs()
                <= self.config.position_tolerance_seconds
        });
        let playable_phase = matches!(
            observed.phase,
            PlayerTransportPhase::ReadyPaused | PlayerTransportPhase::Playing
        );
        let post_seek_observation = episode
            .primary_seek_observation_sequence
            .is_some_and(|sequence| self.observation_sequence > sequence)
            && seek_preparation_evidence_is_fresh;
        if target_scoped_cache_evidence {
            if let Some(percent) = observation
                .cache_buffering_percent
                .filter(|percent| percent.is_finite())
            {
                episode.cache_buffering_percent = Some(percent.clamp(0.0, 100.0));
            }
            if let Some(seconds) = observation
                .buffered_ahead_seconds
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            {
                episode.buffered_ahead_seconds = Some(seconds);
            }
            if let Some(bytes_per_second) = observation.input_rate_bytes_per_second {
                episode.input_rate_bytes_per_second = Some(bytes_per_second);
            }
        }

        let refill_started = post_seek_observation
            && (observation.paused_for_cache == Some(true)
                || matches!(
                    observation.phase,
                    Some(PlayerTransportPhase::Prebuffering | PlayerTransportPhase::Rebuffering)
                ));
        if refill_started {
            episode.refill_started_observation_sequence = Some(self.observation_sequence);
            episode.refill_released_after_seek = false;
            episode.stable_playable_since = None;
        }

        let transport_ready = episode.primary_seek_issued
            && post_seek_observation
            && playable_phase
            && !observed.seeking
            && at_target
            && !observed.paused_for_cache;
        let fresh_playable_or_cache_release = matches!(
            observation.phase,
            Some(PlayerTransportPhase::ReadyPaused | PlayerTransportPhase::Playing)
        ) || observation.paused_for_cache == Some(false);
        if transport_ready
            && fresh_playable_or_cache_release
            && episode
                .refill_started_observation_sequence
                .is_some_and(|sequence| self.observation_sequence > sequence)
        {
            episode.refill_released_after_seek = true;
        }

        let minimum_headroom_ready = episode.buffered_ahead_seconds.is_some_and(|seconds| {
            seconds >= self.config.seek_preparation_minimum_headroom_seconds
        });
        let refill_progress_ready = episode
            .cache_buffering_percent
            .is_some_and(|percent| percent >= 100.0);
        let quantitative_cache_telemetry_observed =
            episode.buffered_ahead_seconds.is_some() || episode.cache_buffering_percent.is_some();
        let fresh_aligned_playable_observation =
            observation.phase.is_some_and(|phase| {
                matches!(
                    phase,
                    PlayerTransportPhase::ReadyPaused | PlayerTransportPhase::Playing
                )
            }) || observation.position_seconds.is_some_and(|position| {
                (position - episode.frozen_target_seconds).abs()
                    <= self.config.position_tolerance_seconds
            });
        let telemetry_poor_stable = if transport_ready
            && !quantitative_cache_telemetry_observed
            && episode.refill_started_observation_sequence.is_none()
            && fresh_aligned_playable_observation
        {
            match episode.stable_playable_since {
                Some((sequence, since_seconds)) => {
                    self.observation_sequence > sequence
                        && observed.observed_at_seconds - since_seconds
                            >= self
                                .config
                                .stability_interval_seconds
                                .min(SEEK_PREPARATION_MAX_STABILIZATION_SECONDS)
                }
                None => {
                    episode.stable_playable_since =
                        Some((self.observation_sequence, observed.observed_at_seconds));
                    false
                }
            }
        } else {
            episode.stable_playable_since = None;
            false
        };
        let ready = transport_ready
            && (minimum_headroom_ready
                || refill_progress_ready
                || episode.refill_released_after_seek
                || telemetry_poor_stable);
        if ready {
            let revision = episode.latest_room_revision;
            let paused_alignment_needed = self.desired.is_some_and(|desired| {
                desired.paused
                    && observed.position_seconds.is_none_or(|position| {
                        (position - desired.position_at(observed.observed_at_seconds)).abs()
                            > self.config.position_tolerance_seconds
                    })
            });
            if observed.logical_pause == Some(true) {
                self.completed_seek_restart_baseline = Some(observed.playback_restart_sequence);
            }
            self.desired_seek_satisfied_revision = (!paused_alignment_needed).then_some(revision);
            self.finish_seek_preparation(SeekPreparationTerminalOutcome::Ready);
            if self.desired.is_some_and(|desired| !desired.paused) {
                self.begin_recovery_handoff(observed.observed_at_seconds);
            }
            return;
        }

        episode.phase = if observed.seeking || !episode.primary_seek_issued {
            SeekPreparationPhase::Seeking
        } else if observed.paused_for_cache
            || matches!(
                observed.phase,
                PlayerTransportPhase::Prebuffering | PlayerTransportPhase::Rebuffering
            )
        {
            if at_target {
                SeekPreparationPhase::Refilling
            } else {
                SeekPreparationPhase::Fetching
            }
        } else if matches!(
            availability,
            SeekTargetAvailability::FetchRequired | SeekTargetAvailability::Unknown
        ) && !at_target
        {
            SeekPreparationPhase::Fetching
        } else {
            SeekPreparationPhase::Refilling
        };
    }

    fn seek_preparation_accepts_target_cache_evidence(
        &self,
        observed: ObservedState,
        observation: &PlayerTransportObservation,
        evidence_is_fresh: bool,
    ) -> bool {
        let Some(episode) = self.seek_preparation.as_ref() else {
            return false;
        };
        let target = episode.frozen_target_seconds;
        let source_position_is_target = observation.position_seconds.is_some_and(|position| {
            position.is_finite()
                && (position - target).abs() <= self.config.position_tolerance_seconds
        });
        let source_ranges_cover_target =
            observation
                .seekable_ranges
                .as_deref()
                .is_some_and(|ranges| {
                    ranges.iter().any(|range| {
                        range.start_seconds.is_finite()
                            && range.end_seconds.is_finite()
                            && range.start_seconds <= target
                            && target <= range.end_seconds
                    })
                });
        episode.primary_seek_issued
            && episode
                .primary_seek_observation_sequence
                .is_some_and(|sequence| self.observation_sequence > sequence)
            && evidence_is_fresh
            && observed.position_seconds.is_some_and(|position| {
                (position - target).abs() <= self.config.position_tolerance_seconds
            })
            && (source_position_is_target || source_ranges_cover_target)
    }

    fn begin_recovery_handoff(&mut self, now_seconds: f64) {
        if self.recovery.is_some() {
            return;
        }
        self.next_recovery_episode_id = self.next_recovery_episode_id.saturating_add(1).max(1);
        self.recovery = Some(RecoveryEpisode {
            id: self.next_recovery_episode_id,
            media_generation: self.current_media_generation().unwrap_or_default(),
            entered_at_seconds: now_seconds,
            hard_seek_attempts: 0,
            post_cache_baseline_at_seconds: Some(now_seconds),
            stable_since_seconds: None,
            catchup_deadline_seconds: None,
            decision_made: false,
            cache_metrics_frozen_until_decision: true,
            catchup_active: false,
            seek_active: false,
            gentle_catchup_only: false,
            degraded: false,
        });
    }

    fn finish_seek_preparation(&mut self, outcome: SeekPreparationTerminalOutcome) {
        let Some(mut episode) = self.seek_preparation.take() else {
            return;
        };
        if outcome == SeekPreparationTerminalOutcome::Ready {
            // Recovery may only inherit quantitative headroom proven for the
            // frozen target. Assignment is intentional: telemetry-poor
            // completion clears any value retained from the pre-seek epoch.
            self.metrics.last_buffered_ahead_seconds = episode.buffered_ahead_seconds;
            self.metrics.last_input_rate_bytes_per_second = episode.input_rate_bytes_per_second;
        }
        if let Some(command_id) = episode.primary_seek_command_id {
            self.pending_commands
                .retain(|command| command.id != command_id);
        }
        episode.phase = if outcome == SeekPreparationTerminalOutcome::Ready {
            SeekPreparationPhase::ReadyToJoin
        } else {
            episode.phase
        };
        self.last_seek_preparation_terminal = Some(episode.snapshot(Some(outcome)));
    }

    fn update_diagnostic_for_observation(&mut self, observed: ObservedState) {
        if self.degraded_seek_preparation_holds_current_revision() {
            self.diagnostic = PlaybackDiagnostic::Degraded;
            return;
        }
        self.diagnostic = match observed.phase {
            PlayerTransportPhase::Empty => PlaybackDiagnostic::Empty,
            PlayerTransportPhase::Loading => PlaybackDiagnostic::Loading,
            PlayerTransportPhase::Prebuffering => PlaybackDiagnostic::Prebuffering,
            PlayerTransportPhase::ReadyPaused => PlaybackDiagnostic::ReadyWaitingForRoom,
            PlayerTransportPhase::Playing if observed.core_idle == Some(true) => {
                PlaybackDiagnostic::Starting
            }
            PlayerTransportPhase::Playing => PlaybackDiagnostic::Playing,
            PlayerTransportPhase::Rebuffering => PlaybackDiagnostic::Rebuffering,
            PlayerTransportPhase::Seeking => PlaybackDiagnostic::Starting,
            PlayerTransportPhase::Ended => PlaybackDiagnostic::Ended,
            PlayerTransportPhase::Failed => PlaybackDiagnostic::Failed,
        };
    }

    fn update_recovery_episode(
        &mut self,
        observed: ObservedState,
        advancing: bool,
        position_sampled: bool,
    ) -> Option<DegradedPlaybackReason> {
        let buffering = observed.paused_for_cache
            || matches!(observed.phase, PlayerTransportPhase::Rebuffering);
        if buffering {
            let hard_seek_rebuffered = self.recovery.as_ref().is_some_and(|episode| {
                episode.seek_active && episode.hard_seek_attempts > 0 && !episode.degraded
            });
            if self.recovery.is_none() {
                self.next_recovery_episode_id =
                    self.next_recovery_episode_id.saturating_add(1).max(1);
                self.metrics.buffer_episode_count =
                    self.metrics.buffer_episode_count.saturating_add(1);
                self.recovery = Some(RecoveryEpisode {
                    id: self.next_recovery_episode_id,
                    media_generation: self.current_media_generation().unwrap_or_default(),
                    entered_at_seconds: observed.observed_at_seconds,
                    hard_seek_attempts: 0,
                    post_cache_baseline_at_seconds: None,
                    stable_since_seconds: None,
                    catchup_deadline_seconds: None,
                    decision_made: false,
                    cache_metrics_frozen_until_decision: false,
                    catchup_active: false,
                    seek_active: false,
                    gentle_catchup_only: false,
                    degraded: false,
                });
            } else if let Some(episode) = self.recovery.as_mut() {
                episode.post_cache_baseline_at_seconds = None;
                episode.stable_since_seconds = None;
            }
            self.diagnostic = PlaybackDiagnostic::Rebuffering;
            return hard_seek_rebuffered.then_some(DegradedPlaybackReason::HardSeekBudgetExhausted);
        }

        if let Some(episode) = self.recovery.as_mut() {
            if observed.seeking {
                episode.post_cache_baseline_at_seconds = None;
                episode.stable_since_seconds = None;
            } else if episode.post_cache_baseline_at_seconds.is_none() && position_sampled {
                episode.post_cache_baseline_at_seconds = Some(observed.observed_at_seconds);
            } else if position_sampled {
                if advancing {
                    episode
                        .stable_since_seconds
                        .get_or_insert(observed.observed_at_seconds);
                } else {
                    // Stability means continuous observed advancement. Sparse
                    // non-position telemetry is neutral, but a fresh position
                    // sample that does not advance restarts the interval.
                    episode.stable_since_seconds = None;
                }
            }
            self.diagnostic = if episode.seek_active {
                PlaybackDiagnostic::RecoveringBySeek
            } else if episode.catchup_active {
                PlaybackDiagnostic::RecoveringByCatchup
            } else {
                PlaybackDiagnostic::Starting
            };
        }
        None
    }

    fn complete_observed_commands(&mut self, observed: ObservedState, advancing: bool) {
        let tolerance = self.config.position_tolerance_seconds;
        let mut recovery_seek_completed = false;
        let mut play_completed = false;
        self.pending_commands.retain(|command| match command.kind {
            PendingCommandKind::Pause => observed.logical_pause != Some(true),
            PendingCommandKind::Play {
                baseline_position,
                intent,
            } => {
                let restart_satisfied = match intent {
                    PlayerPlayIntent::Resume => true,
                    PlayerPlayIntent::StartAfterLoad {
                        baseline_restart_sequence,
                    }
                    | PlayerPlayIntent::StartAfterSeek {
                        baseline_restart_sequence,
                    } => observed.playback_restart_sequence > baseline_restart_sequence,
                };
                let advanced_from_baseline = baseline_position
                    .zip(observed.position_seconds)
                    .is_some_and(|(baseline, current)| {
                        current - baseline > MIN_ADVANCEMENT_SECONDS
                    });
                let completed = observed.logical_pause == Some(false)
                    && !observed.paused_for_cache
                    && !observed.seeking
                    && restart_satisfied
                    && (advancing || advanced_from_baseline);
                play_completed |= completed;
                !completed
            }
            PendingCommandKind::Seek {
                target_position_seconds,
                baseline_restart_sequence,
            } => {
                let completed = !observed.seeking
                    && observed.position_seconds.is_some_and(|position| {
                        (position - target_position_seconds).abs() <= tolerance
                    });
                if completed {
                    self.desired_seek_satisfied_revision = Some(command.revision);
                    if self.required_seek_dispatch_revision == Some(command.revision) {
                        self.required_seek_dispatch_revision = None;
                    }
                    if observed.logical_pause == Some(true) {
                        self.completed_seek_restart_baseline = Some(baseline_restart_sequence);
                    }
                    recovery_seek_completed = true;
                }
                !completed
            }
            PendingCommandKind::Rate { target_rate } => observed
                .playback_rate
                .is_none_or(|rate| (rate - target_rate).abs() > 0.001),
        });
        if play_completed {
            self.completed_seek_restart_baseline = None;
        }
        if recovery_seek_completed
            && let Some(episode) = self.recovery.as_mut()
            && episode.seek_active
        {
            // A hard seek is one phase of the same recovery episode. Re-open
            // the decision exactly once for residual lag; the episode's seek
            // budget is intentionally retained, so another large-lag decision
            // degrades instead of creating a seek loop.
            episode.seek_active = false;
            episode.decision_made = false;
            episode.stable_since_seconds = None;
        }
    }

    fn coordinate_desired_state(
        &mut self,
        observed: ObservedState,
        observation: &PlayerTransportObservation,
        seek_preparation_evidence_is_fresh: bool,
        advancing: bool,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        let Some(desired) = self.desired else {
            return;
        };
        if desired.media_generation != self.current_media_generation().unwrap_or_default() {
            return;
        }

        if self.authoritative_alignment_guard_revision == Some(desired.state_revision)
            && self.desired_seek_satisfied_revision == Some(desired.state_revision)
            && self.last_started_revision != Some(desired.state_revision)
            && !observed.seeking
        {
            let target =
                self.clamp_seek_target(desired.position_at(observed.observed_at_seconds), observed);
            let displaced = observed.position_seconds.is_none_or(|position| {
                (position - target).abs() > self.config.position_tolerance_seconds
            });
            if displaced {
                // A superseded async seek may complete after the canonical
                // replacement appeared satisfied. Re-arm the replacement and
                // discard any play command derived from that transient sample.
                self.desired_seek_satisfied_revision = None;
                self.required_seek_dispatch_revision = Some(desired.state_revision);
                self.pending_commands.retain(|command| {
                    command.revision != desired.state_revision
                        || !matches!(command.kind, PendingCommandKind::Play { .. })
                });
                if observed.logical_pause != Some(true) {
                    self.issue_command(
                        desired.state_revision,
                        observed.observed_at_seconds,
                        PendingCommandKind::Pause,
                        CoordinatorPlayerCommand::SetPaused(true),
                        actions,
                    );
                }
            }
        }

        if self.seek_preparation.is_some() {
            if observed.logical_pause != Some(true) {
                // Preparation owns a frozen target. Never let an unpaused
                // player continue rendering the old position while timeline
                // classification, seeking, or refill is unresolved.
                self.issue_command(
                    desired.state_revision,
                    observed.observed_at_seconds,
                    PendingCommandKind::Pause,
                    CoordinatorPlayerCommand::SetPaused(true),
                    actions,
                );
            }
            let target_was_already_satisfied =
                self.issue_seek_preparation_primary_seek(desired, observed, actions);
            if target_was_already_satisfied {
                // update_seek_preparation ran before command coordination for
                // this observation, when primary_seek_issued was still false.
                // Re-evaluate now so an already-satisfied ReadyPaused target
                // cannot wait forever for telemetry that may never change.
                let target_scoped_cache_evidence = self
                    .seek_preparation_accepts_target_cache_evidence(
                        observed,
                        observation,
                        seek_preparation_evidence_is_fresh,
                    );
                self.update_seek_preparation(
                    observed,
                    observation,
                    seek_preparation_evidence_is_fresh,
                    target_scoped_cache_evidence,
                    actions,
                );
            }
            if self.seek_preparation.is_some() {
                return;
            }
        }

        if self.terminal_seek_preparation_holds_current_revision() {
            if (desired.paused || self.degraded_seek_preparation_holds_current_revision())
                && observed.logical_pause != Some(true)
            {
                self.issue_command(
                    desired.state_revision,
                    observed.observed_at_seconds,
                    PendingCommandKind::Pause,
                    CoordinatorPlayerCommand::SetPaused(true),
                    actions,
                );
            }
            return;
        }

        if desired.paused {
            if self.transport_blocks_correction(observed) {
                if observed.logical_pause != Some(true) {
                    // Latch room pause immediately even while loading,
                    // buffering, or seeking. Position correction remains
                    // deferred until the transport is safe, and completion
                    // still requires an observation of logical pause.
                    self.issue_command(
                        desired.state_revision,
                        observed.observed_at_seconds,
                        PendingCommandKind::Pause,
                        CoordinatorPlayerCommand::SetPaused(true),
                        actions,
                    );
                }
                return;
            }
            self.issue_desired_seek_if_needed(desired, observed, actions);
            if !self.desired_seek_is_satisfied(desired) {
                return;
            }
            if observed.logical_pause != Some(true) {
                self.issue_command(
                    desired.state_revision,
                    observed.observed_at_seconds,
                    PendingCommandKind::Pause,
                    CoordinatorPlayerCommand::SetPaused(true),
                    actions,
                );
                return;
            }
            if observed.logical_pause == Some(true) {
                self.mark_revision_applied(desired, observed, false, actions);
            }
            return;
        }

        if self.transport_blocks_correction(observed) {
            return;
        }

        self.issue_desired_seek_if_needed(desired, observed, actions);
        if !self.desired_seek_is_satisfied(desired) {
            return;
        }

        if observed.logical_pause != Some(false) {
            let intent =
                if let Some(baseline_restart_sequence) = self.completed_seek_restart_baseline {
                    PlayerPlayIntent::StartAfterSeek {
                        baseline_restart_sequence,
                    }
                } else if self.last_started_revision.is_none() {
                    PlayerPlayIntent::StartAfterLoad {
                        baseline_restart_sequence: self
                            .media
                            .as_ref()
                            .map_or(0, |media| media.load_restart_baseline),
                    }
                } else {
                    PlayerPlayIntent::Resume
                };
            self.issue_command(
                desired.state_revision,
                observed.observed_at_seconds,
                PendingCommandKind::Play {
                    baseline_position: observed.position_seconds,
                    intent,
                },
                CoordinatorPlayerCommand::Play(intent),
                actions,
            );
            return;
        }

        self.coordinate_recovery(
            desired,
            observed,
            seek_preparation_evidence_is_fresh,
            advancing,
            actions,
        );
        let play_command_pending = self.pending_commands.iter().any(|command| {
            command.revision == desired.state_revision
                && matches!(command.kind, PendingCommandKind::Play { .. })
        });
        if advancing && !observed.paused_for_cache && !observed.seeking && !play_command_pending {
            self.mark_revision_applied(desired, observed, true, actions);
        }
    }

    fn issue_desired_seek_if_needed(
        &mut self,
        desired: DesiredRoomPlayback,
        observed: ObservedState,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        if self.desired_seek_is_satisfied(desired) || observed.seeking {
            return;
        }
        let target =
            self.clamp_seek_target(desired.position_at(observed.observed_at_seconds), observed);
        let dispatch_required =
            self.required_seek_dispatch_revision == Some(desired.state_revision);
        if !dispatch_required
            && observed.position_seconds.is_some_and(|position| {
                (position - target).abs() <= self.config.position_tolerance_seconds
            })
        {
            self.desired_seek_satisfied_revision = Some(desired.state_revision);
            return;
        }
        let target_is_cached = self.cached_seekable_ranges.as_ref().is_some_and(|ranges| {
            ranges
                .iter()
                .any(|range| (range.start_seconds..=range.end_seconds).contains(&target))
        });
        let media_kind = self.media.as_ref().map(|media| media.kind);
        if !self.observed_seekable(observed) {
            if media_kind.is_some_and(|kind| kind != MediaTransportKind::LocalFile) {
                // Remote/non-seekable forced alignment needs an explicit
                // terminal disposition. Going straight to "satisfied" would
                // let an authoritative barrier start at the wrong position.
                self.begin_seek_preparation_if_needed(desired);
                self.required_seek_dispatch_revision = None;
                if observed.logical_pause != Some(true) {
                    self.issue_command(
                        desired.state_revision,
                        observed.observed_at_seconds,
                        PendingCommandKind::Pause,
                        CoordinatorPlayerCommand::SetPaused(true),
                        actions,
                    );
                }
                if self.last_seek_preparation_terminal_outcome()
                    == Some(SeekPreparationTerminalOutcome::Degraded(
                        SeekPreparationDegradedReason::NonSeekable,
                    ))
                {
                    actions.push(PlaybackCoordinatorAction::Degraded {
                        media_generation: desired.media_generation,
                        recovery_episode_id: None,
                        reason: DegradedPlaybackReason::NonSeekableLag,
                    });
                }
            } else {
                // Local playback keeps the historical best-effort behavior.
                self.desired_seek_satisfied_revision = Some(desired.state_revision);
                self.required_seek_dispatch_revision = None;
            }
            return;
        }
        let unresolved_timeline = media_kind.is_some_and(|kind| match kind {
            MediaTransportKind::NetworkVod => {
                observed.timeline_kind == Some(PlayerTimelineKind::Unknown)
            }
            MediaTransportKind::LiveSliding => observed.known_live_seekable_window.is_none(),
            MediaTransportKind::LocalFile | MediaTransportKind::NonSeekable => false,
        });
        if !target_is_cached && unresolved_timeline {
            // An authoritative barrier/reconnect alignment normally bypasses
            // the client-only explicit-seek preparation path. Once the
            // adapter explicitly reports an unresolved timeline, however,
            // silently waiting here would leave the revision pending forever.
            // Reuse the bounded episode only as a classification/fetch wait;
            // it cannot issue a seek until the target is cached or the
            // timeline becomes known.
            if self.seek_preparation.is_none() {
                self.begin_seek_preparation_if_needed(desired);
            }
            if observed.logical_pause != Some(true) {
                self.issue_command(
                    desired.state_revision,
                    observed.observed_at_seconds,
                    PendingCommandKind::Pause,
                    CoordinatorPlayerCommand::SetPaused(true),
                    actions,
                );
            }
            return;
        }
        self.issue_command(
            desired.state_revision,
            observed.observed_at_seconds,
            PendingCommandKind::Seek {
                target_position_seconds: target,
                baseline_restart_sequence: observed.playback_restart_sequence,
            },
            CoordinatorPlayerCommand::SetPosition(target),
            actions,
        );
    }

    fn issue_seek_preparation_primary_seek(
        &mut self,
        desired: DesiredRoomPlayback,
        observed: ObservedState,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) -> bool {
        let Some(episode) = self.seek_preparation.as_ref() else {
            return false;
        };
        if episode.primary_seek_issued
            || self.transport_blocks_correction(observed)
            || !self.observed_seekable(observed)
        {
            return false;
        }
        let target = episode.frozen_target_seconds;
        let dispatch_required =
            self.required_seek_dispatch_revision == Some(desired.state_revision);
        if !dispatch_required
            && observed.position_seconds.is_some_and(|position| {
                (position - target).abs() <= self.config.position_tolerance_seconds
            })
        {
            self.mark_seek_preparation_primary_issued(None);
            return true;
        }
        if self.media.as_ref().is_some_and(|media| {
            media.kind == MediaTransportKind::NetworkVod
                && observed.timeline_kind == Some(PlayerTimelineKind::Unknown)
                && episode.availability != SeekTargetAvailability::Cached
        }) {
            // The current mpv adapter has explicitly said that it has not yet
            // distinguished finite VOD from a moving live timeline. Do not
            // let a playable-phase event race ahead of that classification.
            return false;
        }
        if self.media.as_ref().is_some_and(|media| {
            media.kind == MediaTransportKind::LiveSliding
                && episode.availability == SeekTargetAvailability::Unknown
        }) {
            // A sliding source must reveal its current window before Sorotte
            // can safely choose an effective frozen target.
            return false;
        }
        let command_id = self.issue_command(
            desired.state_revision,
            observed.observed_at_seconds,
            PendingCommandKind::Seek {
                target_position_seconds: target,
                baseline_restart_sequence: observed.playback_restart_sequence,
            },
            CoordinatorPlayerCommand::SetPosition(target),
            actions,
        );
        if let Some(command_id) = command_id {
            self.mark_seek_preparation_primary_issued(Some(command_id));
        }
        false
    }

    fn mark_seek_preparation_primary_issued(&mut self, command_id: Option<CoordinatorCommandId>) {
        if let Some(episode) = self.seek_preparation.as_mut() {
            episode.primary_seek_issued = true;
            episode.primary_seek_command_id = command_id;
            episode.primary_seek_observation_sequence = Some(self.observation_sequence);
            episode.cache_buffering_percent = None;
            episode.buffered_ahead_seconds = None;
            episode.input_rate_bytes_per_second = None;
            episode.refill_started_observation_sequence = None;
            episode.refill_released_after_seek = false;
            episode.stable_playable_since = None;
        }
        self.invalidate_cache_metrics_for_seek();
    }

    fn invalidate_cache_metrics_for_seek(&mut self) {
        self.metrics.last_buffered_ahead_seconds = None;
        self.metrics.last_input_rate_bytes_per_second = None;
        if let Some(observed) = self.observed.as_mut() {
            observed.cache_buffering_percent = None;
            observed.buffered_ahead_seconds = None;
        }
    }

    fn coordinate_recovery(
        &mut self,
        desired: DesiredRoomPlayback,
        observed: ObservedState,
        skew_evidence_is_fresh: bool,
        advancing: bool,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        let Some(episode_snapshot) = self.recovery.as_ref().cloned() else {
            return;
        };
        if observed.paused_for_cache
            || observed.seeking
            || !advancing
            || episode_snapshot
                .post_cache_baseline_at_seconds
                .is_none_or(|baseline| observed.observed_at_seconds <= baseline)
        {
            return;
        }

        let room_position = desired.position_at(observed.observed_at_seconds);
        let local_position = observed.position_seconds.unwrap_or(room_position);
        let lag = (room_position - local_position).max(0.0);
        if skew_evidence_is_fresh {
            self.metrics.steady_state_skew_seconds = Some(local_position - room_position);
        }

        if !episode_snapshot.decision_made {
            let mut decision_made = true;
            match self.config.recovery_policy {
                RecoveryPolicy::PreserveContent => {}
                RecoveryPolicy::PauseRoom => {
                    actions.push(PlaybackCoordinatorAction::RequestRoomPause {
                        recovery_episode_id: episode_snapshot.id,
                    });
                }
                RecoveryPolicy::Balanced if lag <= self.config.negligible_lag_seconds => {}
                policy
                    if lag > self.config.negligible_lag_seconds
                        && ((policy == RecoveryPolicy::Balanced
                            && lag < self.config.hard_seek_threshold_seconds)
                            || (episode_snapshot.gentle_catchup_only
                                && matches!(
                                    policy,
                                    RecoveryPolicy::Balanced | RecoveryPolicy::StayClosest
                                )
                                && lag <= self.config.nearest_buffered_target_limit_seconds)) =>
                {
                    if self
                        .media
                        .as_ref()
                        .is_some_and(|media| media.kind.allows_rate_correction())
                    {
                        self.metrics.gentle_catchup_count =
                            self.metrics.gentle_catchup_count.saturating_add(1);
                        if let Some(episode) = self.recovery.as_mut() {
                            episode.catchup_active = true;
                        }
                        let full_catchup_rate_allowed = !episode_snapshot.gentle_catchup_only
                            && observation_has_catchup_headroom(&self.metrics);
                        let target_rate = if full_catchup_rate_allowed {
                            self.config.maximum_catchup_rate
                        } else {
                            self.config
                                .maximum_catchup_rate
                                .min(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM)
                        };
                        let rate_gain = (target_rate - NORMAL_PLAYBACK_RATE).max(0.0);
                        let expected_convergence_seconds = if rate_gain > f64::EPSILON {
                            lag / rate_gain
                        } else {
                            self.config.stability_interval_seconds
                        };
                        let deadline_after = (expected_convergence_seconds * 1.5
                            + self.config.stability_interval_seconds)
                            .max(
                                self.config.command_timeout_seconds
                                    + self.config.stability_interval_seconds,
                            )
                            .min(MAXIMUM_CATCHUP_EPISODE_SECONDS);
                        if let Some(episode) = self.recovery.as_mut() {
                            episode.catchup_deadline_seconds =
                                Some(observed.observed_at_seconds + deadline_after);
                        }
                        self.rate_override = Some(RateOverride {
                            episode_id: episode_snapshot.id,
                            reset_requested: false,
                            reset_command_observation_sequence: None,
                        });
                        self.issue_command(
                            desired.state_revision,
                            observed.observed_at_seconds,
                            PendingCommandKind::Rate { target_rate },
                            CoordinatorPlayerCommand::SetPlaybackRate(target_rate),
                            actions,
                        );
                    } else {
                        self.enter_degraded_recovery(
                            DegradedPlaybackReason::NonSeekableLag,
                            actions,
                        );
                    }
                }
                RecoveryPolicy::Balanced | RecoveryPolicy::StayClosest => {
                    if lag <= self.config.negligible_lag_seconds {
                        // Already converged.
                    } else if episode_snapshot.gentle_catchup_only {
                        // Joining the nearest buffered point is an explicit
                        // request to avoid another discontinuity. If gentle
                        // convergence is no longer viable, remain there and
                        // degrade instead of seeking back to the uncached room
                        // target that prompted the alternative.
                        self.enter_degraded_recovery(
                            DegradedPlaybackReason::CatchupDidNotConverge,
                            actions,
                        );
                    } else if self.observed_seekable(observed)
                        && episode_snapshot.hard_seek_attempts
                            < self.config.maximum_hard_seeks_per_episode
                    {
                        let target = self.clamp_seek_target(room_position, observed);
                        self.metrics.hard_seek_count =
                            self.metrics.hard_seek_count.saturating_add(1);
                        if let Some(episode) = self.recovery.as_mut() {
                            episode.hard_seek_attempts =
                                episode.hard_seek_attempts.saturating_add(1);
                            episode.seek_active = true;
                            episode.post_cache_baseline_at_seconds = None;
                            episode.stable_since_seconds = None;
                        }
                        self.issue_command(
                            desired.state_revision,
                            observed.observed_at_seconds,
                            PendingCommandKind::Seek {
                                target_position_seconds: target,
                                baseline_restart_sequence: observed.playback_restart_sequence,
                            },
                            CoordinatorPlayerCommand::SetPosition(target),
                            actions,
                        );
                    } else {
                        decision_made = true;
                        self.enter_degraded_recovery(
                            if self.observed_seekable(observed) {
                                DegradedPlaybackReason::HardSeekBudgetExhausted
                            } else {
                                DegradedPlaybackReason::NonSeekableLag
                            },
                            actions,
                        );
                    }
                }
            }
            if let Some(episode) = self.recovery.as_mut() {
                episode.decision_made = decision_made;
            }
        }

        let stable = self.recovery.as_ref().and_then(|episode| {
            episode
                .stable_since_seconds
                .map(|since| observed.observed_at_seconds - since)
        });
        let stable_interval_elapsed =
            stable.is_some_and(|elapsed| elapsed >= self.config.stability_interval_seconds);
        // Recovery owns catch-up after buffering and only needs to prove that the player is no
        // longer behind the room. If it has crossed ahead, hand the stable transport back to the
        // ordinary bidirectional drift policy instead of retaining recovery ownership forever.
        let converged = lag <= self.config.negligible_lag_seconds;
        let catchup_timed_out = self.recovery.as_ref().is_some_and(|episode| {
            episode.catchup_active
                && episode
                    .catchup_deadline_seconds
                    .is_some_and(|deadline| observed.observed_at_seconds >= deadline)
        });
        if catchup_timed_out && !converged {
            self.enter_degraded_recovery(DegradedPlaybackReason::CatchupDidNotConverge, actions);
            return;
        }
        if stable_interval_elapsed && !converged {
            let seek_failed_to_converge = self
                .recovery
                .as_ref()
                .is_some_and(|episode| episode.seek_active);
            let degraded_reason = match self.config.recovery_policy {
                RecoveryPolicy::PreserveContent | RecoveryPolicy::PauseRoom => {
                    Some(DegradedPlaybackReason::NonSeekableLag)
                }
                RecoveryPolicy::Balanced | RecoveryPolicy::StayClosest
                    if seek_failed_to_converge =>
                {
                    Some(DegradedPlaybackReason::HardSeekBudgetExhausted)
                }
                RecoveryPolicy::Balanced | RecoveryPolicy::StayClosest => None,
            };
            if let Some(reason) = degraded_reason {
                self.enter_degraded_recovery(reason, actions);
            }
        }
        if converged && stable_interval_elapsed {
            self.close_recovery_with_metrics(observed.observed_at_seconds);
        }
    }

    fn issue_command(
        &mut self,
        revision: u64,
        now_seconds: f64,
        kind: PendingCommandKind,
        command: CoordinatorPlayerCommand,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) -> Option<CoordinatorCommandId> {
        let baseline_rate_reset = matches!(
            kind,
            PendingCommandKind::Rate { target_rate }
                if (target_rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
                    && self
                        .rate_override
                        .is_some_and(|rate_override| rate_override.reset_requested)
        );
        if (!baseline_rate_reset
            && (self.command_budget_degraded
                || self.failed_command_attempts > self.config.command_retry_budget))
            || now_seconds < self.retry_not_before_seconds
            || self.pending_commands.iter().any(|pending| {
                matches!(
                    (pending.kind, kind),
                    (PendingCommandKind::Pause, PendingCommandKind::Pause)
                        | (
                            PendingCommandKind::Play { .. },
                            PendingCommandKind::Play { .. }
                        )
                        | (
                            PendingCommandKind::Seek { .. },
                            PendingCommandKind::Seek { .. }
                        )
                        | (
                            PendingCommandKind::Rate { .. },
                            PendingCommandKind::Rate { .. }
                        )
                )
            })
        {
            return None;
        }
        if matches!(kind, PendingCommandKind::Seek { .. }) {
            // Buffered duration and input rate describe the current playback
            // position. Once a seek is actually admitted, they cannot safely
            // describe the destination, regardless of which lifecycle issued
            // the discontinuity.
            self.invalidate_cache_metrics_for_seek();
        }
        self.next_command_id = self.next_command_id.saturating_add(1).max(1);
        let command_id = CoordinatorCommandId(self.next_command_id);
        self.pending_commands.push(PendingCommand {
            id: command_id,
            revision,
            issued_at_seconds: now_seconds,
            accepted: false,
            kind,
        });
        if baseline_rate_reset && let Some(rate_override) = self.rate_override.as_mut() {
            rate_override.reset_command_observation_sequence = Some(self.observation_sequence);
        }
        actions.push(PlaybackCoordinatorAction::Execute {
            command_id,
            command,
        });
        Some(command_id)
    }

    fn mark_revision_applied(
        &mut self,
        desired: DesiredRoomPlayback,
        observed: ObservedState,
        started: bool,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        if self.last_applied_revision != Some(desired.state_revision) {
            self.last_applied_revision = Some(desired.state_revision);
            self.metrics.applied_revision_count =
                self.metrics.applied_revision_count.saturating_add(1);
            actions.push(PlaybackCoordinatorAction::RevisionApplied {
                media_generation: desired.media_generation,
                state_revision: desired.state_revision,
            });
        }
        if started && self.last_started_revision != Some(desired.state_revision) {
            self.last_started_revision = Some(desired.state_revision);
            if self.authoritative_alignment_guard_revision == Some(desired.state_revision) {
                self.authoritative_alignment_guard_revision = None;
                self.required_seek_dispatch_revision = None;
            }
            let position = observed.position_seconds.unwrap_or_default();
            if let Some(media) = self.media.as_ref() {
                let latency = observed.observed_at_seconds - media.prepared_at_seconds;
                if self.metrics.first_frame_latency_seconds.is_none() && latency >= 0.0 {
                    self.metrics.first_frame_latency_seconds = Some(latency);
                }
            }
            let start_latency = observed.observed_at_seconds - desired.anchor_observed_at_seconds;
            if start_latency >= 0.0 {
                self.metrics.started_ack_latency_seconds = Some(start_latency);
            }
            actions.push(PlaybackCoordinatorAction::Started {
                media_generation: desired.media_generation,
                state_revision: desired.state_revision,
                observed_position_seconds: position,
            });
        }
    }

    fn enter_degraded_recovery(
        &mut self,
        reason: DegradedPlaybackReason,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        if let Some(episode) = self.recovery.as_mut() {
            if episode.degraded {
                return;
            }
            episode.degraded = true;
            episode.decision_made = true;
            episode.catchup_active = false;
            episode.seek_active = false;
            episode.catchup_deadline_seconds = None;
        }
        self.metrics.degraded_recovery_count =
            self.metrics.degraded_recovery_count.saturating_add(1);
        self.request_rate_reset();
        self.diagnostic = PlaybackDiagnostic::Degraded;
        actions.push(PlaybackCoordinatorAction::Degraded {
            media_generation: self.current_media_generation().unwrap_or_default(),
            recovery_episode_id: self.recovery.as_ref().map(|episode| episode.id),
            reason,
        });
    }

    fn desired_seek_is_satisfied(&self, desired: DesiredRoomPlayback) -> bool {
        !desired.force_seek || self.desired_seek_satisfied_revision == Some(desired.state_revision)
    }

    fn observed_seekable(&self, observed: ObservedState) -> bool {
        observed.seekable.unwrap_or_else(|| {
            self.media
                .as_ref()
                .is_some_and(|media| media.kind.default_seekable())
        })
    }

    fn clamp_seek_target(&self, target: f64, observed: ObservedState) -> f64 {
        let target = target.max(0.0);
        if self.cached_seekable_ranges.as_ref().is_some_and(|ranges| {
            ranges
                .iter()
                .any(|range| (range.start_seconds..=range.end_seconds).contains(&target))
        }) {
            // A disjoint older live range can still be locally seekable. The
            // adapter's `known_live_seekable_window` is only its rightmost
            // conservative interval, not permission to rewrite a target that
            // is explicitly present in another cached interval.
            return target;
        }
        if self
            .media
            .as_ref()
            .is_none_or(|media| media.kind != MediaTransportKind::LiveSliding)
        {
            // mpv's VOD demuxer ranges describe cached data, not the full
            // source. A seek outside them performs a valid low-level demuxer
            // seek and must not be silently rewritten to the cache edge.
            return target;
        }
        let Some(window) = observed.known_live_seekable_window else {
            return target;
        };
        let (start, end) = (window.start_seconds, window.end_seconds);
        // Avoid targeting a point still being written at the live edge.
        let end = (end - 1.0_f64.min((end - start).max(0.0) / 4.0)).max(start);
        target.clamp(start, end)
    }

    fn transport_blocks_correction(&self, observed: ObservedState) -> bool {
        observed.paused_for_cache
            || observed.seeking
            || matches!(
                observed.phase,
                PlayerTransportPhase::Empty
                    | PlayerTransportPhase::Loading
                    | PlayerTransportPhase::Prebuffering
                    | PlayerTransportPhase::Rebuffering
                    | PlayerTransportPhase::Seeking
                    | PlayerTransportPhase::Ended
                    | PlayerTransportPhase::Failed
            )
    }

    fn close_recovery_with_metrics(&mut self, now_seconds: f64) {
        self.request_rate_reset();
        if let Some(episode) = self.recovery.take() {
            self.metrics.total_buffer_duration_seconds +=
                (now_seconds - episode.entered_at_seconds).max(0.0);
        }
        self.diagnostic = PlaybackDiagnostic::Playing;
    }

    fn close_recovery_without_metrics(&mut self) {
        self.request_rate_reset();
        self.recovery = None;
    }

    fn request_rate_reset(&mut self) {
        let Some(rate_override) = self.rate_override.as_mut() else {
            return;
        };
        if !rate_override.reset_requested {
            rate_override.reset_requested = true;
            rate_override.reset_command_observation_sequence = None;
            // A baseline reset supersedes an in-flight catch-up command. Once
            // that reset is pending, repeated pause/lifecycle reconciliation
            // must leave it tracked instead of replacing it every tick.
            self.pending_commands.retain(|command| {
                !matches!(
                    command.kind,
                    PendingCommandKind::Rate { target_rate }
                        if (target_rate - NORMAL_PLAYBACK_RATE).abs() > 0.001
                )
            });
        }
    }

    fn reconcile_rate_override(
        &mut self,
        observed: ObservedState,
        command_now_seconds: f64,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        let Some(rate_override) = self.rate_override else {
            return;
        };
        let owning_episode_active = self
            .recovery
            .as_ref()
            .is_some_and(|episode| episode.id == rate_override.episode_id);
        if !rate_override.reset_requested && owning_episode_active {
            // The override is still intentionally owned by this recovery
            // episode, whether mpv currently reports the old baseline or the
            // requested target. In particular, the observation that decides
            // to start catch-up commonly still carries speed=1.0.
            return;
        }
        if !rate_override.reset_requested {
            self.request_rate_reset();
        }
        let Some(rate_override) = self.rate_override else {
            return;
        };
        let baseline_confirmed_after_reset_command = rate_override
            .reset_command_observation_sequence
            .is_some_and(|issued_after_sequence| {
                self.last_playback_rate_observation_sequence
                    .is_some_and(|observed_sequence| observed_sequence > issued_after_sequence)
                    && observed
                        .playback_rate
                        .is_some_and(|rate| (rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001)
            });
        if baseline_confirmed_after_reset_command {
            self.rate_override = None;
            return;
        }
        let revision = self.desired.map_or(0, |desired| desired.state_revision);
        self.issue_command(
            revision,
            command_now_seconds,
            PendingCommandKind::Rate {
                target_rate: NORMAL_PLAYBACK_RATE,
            },
            CoordinatorPlayerCommand::SetPlaybackRate(NORMAL_PLAYBACK_RATE),
            actions,
        );
    }
}

fn latest_valid_seekable_window(ranges: &[PlayerSeekableRange]) -> Option<(f64, f64)> {
    ranges
        .iter()
        .filter(|range| {
            range.start_seconds.is_finite()
                && range.end_seconds.is_finite()
                && range.end_seconds >= range.start_seconds
                && range.end_seconds >= 0.0
        })
        .max_by(|left, right| left.end_seconds.total_cmp(&right.end_seconds))
        .map(|range| (range.start_seconds.max(0.0), range.end_seconds))
}

fn normalize_seekable_ranges(ranges: &[PlayerSeekableRange]) -> Vec<PlayerSeekableRange> {
    let mut normalized = ranges
        .iter()
        .filter_map(|range| {
            (range.start_seconds.is_finite()
                && range.end_seconds.is_finite()
                && range.end_seconds >= range.start_seconds
                && range.end_seconds >= 0.0)
                .then_some(PlayerSeekableRange::new(
                    range.start_seconds.max(0.0),
                    range.end_seconds,
                ))
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left.end_seconds.total_cmp(&right.end_seconds))
    });
    let mut merged: Vec<PlayerSeekableRange> = Vec::with_capacity(normalized.len());
    for range in normalized {
        if let Some(previous) = merged.last_mut()
            && range.start_seconds <= previous.end_seconds
        {
            previous.end_seconds = previous.end_seconds.max(range.end_seconds);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn classify_seek_target(
    kind: MediaTransportKind,
    observed_seekable: Option<bool>,
    cached_ranges: Option<&[PlayerSeekableRange]>,
    known_live_seekable_window: Option<PlayerSeekableRange>,
    target_seconds: f64,
) -> SeekTargetAvailability {
    if kind == MediaTransportKind::NonSeekable || observed_seekable == Some(false) {
        return SeekTargetAvailability::NonSeekable;
    }
    if kind == MediaTransportKind::LocalFile {
        return SeekTargetAvailability::Cached;
    }
    if cached_ranges.is_some_and(|ranges| {
        ranges
            .iter()
            .any(|range| (range.start_seconds..=range.end_seconds).contains(&target_seconds))
    }) {
        return SeekTargetAvailability::Cached;
    }
    if kind == MediaTransportKind::LiveSliding {
        let Some(window) = known_live_seekable_window else {
            return SeekTargetAvailability::Unknown;
        };
        if !(window.start_seconds..=window.end_seconds).contains(&target_seconds) {
            return SeekTargetAvailability::OutsideLiveWindow;
        }
    }
    if cached_ranges.is_some() {
        SeekTargetAvailability::FetchRequired
    } else {
        SeekTargetAvailability::Unknown
    }
}

fn clamp_to_live_seekable_window(
    target_seconds: f64,
    window: Option<PlayerSeekableRange>,
) -> Option<f64> {
    let window = window?;
    let (start, end) = (window.start_seconds, window.end_seconds);
    let safe_end = (end - 1.0_f64.min((end - start).max(0.0) / 4.0)).max(start);
    Some(target_seconds.clamp(start, safe_end))
}

fn nearest_buffered_target(
    kind: MediaTransportKind,
    target_seconds: f64,
    ranges: Option<&[PlayerSeekableRange]>,
    maximum_distance_seconds: f64,
) -> Option<f64> {
    let ranges = ranges?;
    if ranges
        .iter()
        .any(|range| (range.start_seconds..=range.end_seconds).contains(&target_seconds))
    {
        return None;
    }
    ranges
        .iter()
        .map(|range| {
            let safe_end = if kind == MediaTransportKind::LiveSliding {
                (range.end_seconds
                    - 1.0_f64.min((range.end_seconds - range.start_seconds).max(0.0) / 4.0))
                .max(range.start_seconds)
            } else {
                range.end_seconds
            };
            target_seconds.clamp(range.start_seconds, safe_end)
        })
        .filter(|candidate| (*candidate - target_seconds).abs() > f64::EPSILON)
        .min_by(|left, right| {
            (*left - target_seconds)
                .abs()
                .total_cmp(&(*right - target_seconds).abs())
        })
        .filter(|candidate| (*candidate - target_seconds).abs() <= maximum_distance_seconds)
}

fn observation_has_catchup_headroom(metrics: &PlaybackCoordinatorMetrics) -> bool {
    metrics
        .last_buffered_ahead_seconds
        .is_some_and(|seconds| seconds >= HEALTHY_CATCHUP_BUFFER_SECONDS)
}

#[cfg(test)]
mod tests;
