use std::fmt;

use sorotte_player_api::{PlayerPlayIntent, PlayerSeekableRange, PlayerTransportPhase};
pub use sorotte_protocol::MediaLoadIntent;

const MIN_ADVANCEMENT_SECONDS: f64 = 0.01;
const NORMAL_PLAYBACK_RATE: f64 = 1.0;
const CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM: f64 = 1.03;
const HEALTHY_CATCHUP_BUFFER_SECONDS: f64 = 2.0;
const MAXIMUM_CATCHUP_EPISODE_SECONDS: f64 = 300.0;

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
    pub seekable_ranges: Option<Vec<PlayerSeekableRange>>,
    pub core_idle: Option<bool>,
    pub playback_restart_sequence: Option<u64>,
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
            seekable_ranges: None,
            core_idle: None,
            playback_restart_sequence: None,
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

    pub fn with_seekable_ranges(mut self, seekable_ranges: Vec<PlayerSeekableRange>) -> Self {
        self.seekable_ranges = Some(seekable_ranges);
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
    TransportFailed,
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
    core_idle: Option<bool>,
    playback_restart_sequence: u64,
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
    catchup_active: bool,
    seek_active: bool,
    degraded: bool,
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
    pending_commands: Vec<PendingCommand>,
    next_media_generation: u64,
    next_recovery_episode_id: u64,
    next_command_id: u64,
    last_applied_revision: Option<u64>,
    last_started_revision: Option<u64>,
    desired_seek_satisfied_revision: Option<u64>,
    completed_seek_restart_baseline: Option<u64>,
    rate_override: Option<RateOverride>,
    observation_sequence: u64,
    last_playback_rate_observation_sequence: Option<u64>,
    retry_not_before_seconds: f64,
    failed_command_attempts: u32,
    command_budget_degraded: bool,
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
            pending_commands: Vec::new(),
            next_media_generation: 0,
            next_recovery_episode_id: 0,
            next_command_id: 0,
            last_applied_revision: None,
            last_started_revision: None,
            desired_seek_satisfied_revision: None,
            completed_seek_restart_baseline: None,
            rate_override: None,
            observation_sequence: 0,
            last_playback_rate_observation_sequence: None,
            retry_not_before_seconds: 0.0,
            failed_command_attempts: 0,
            command_budget_degraded: false,
            diagnostic: PlaybackDiagnostic::Empty,
            metrics: PlaybackCoordinatorMetrics::default(),
        }
    }

    pub fn set_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.config = config.normalized();
    }

    pub fn reset_transport_adapter_epoch(&mut self, now_seconds: f64) {
        self.observed = None;
        self.pending_commands.clear();
        self.close_recovery_without_metrics();
        self.completed_seek_restart_baseline = None;
        if let Some(media) = self.media.as_mut() {
            media.load_restart_baseline = 0;
        }
        self.retry_not_before_seconds = 0.0;
        self.failed_command_attempts = 0;
        self.command_budget_degraded = false;
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
            (false, _) => MediaLoadIntent::NewPlayback,
        };
        if load_intent == MediaLoadIntent::TransportRefresh {
            self.close_recovery_without_metrics();
            let media = self.media.as_mut().expect("media existence was checked");
            media.load_attempt = media.load_attempt.saturating_add(1);
            media.kind = kind;
            media.prepared_at_seconds = now_seconds;
            media.load_restart_baseline = load_restart_baseline;
            self.observed = None;
            self.pending_commands.clear();
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
            if let Some(desired) = self.desired.as_mut() {
                desired.force_seek = true;
            }
            self.diagnostic = PlaybackDiagnostic::Loading;
            return MediaLoadPlan {
                media_generation: media.generation,
                load_attempt: media.load_attempt,
                logical_media_changed: false,
                playback_episode_changed: false,
                load_intent,
            };
        }

        let logical_media_changed = !same_logical_media;
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
        self.pending_commands.clear();
        self.last_applied_revision = None;
        self.last_started_revision = None;
        self.desired_seek_satisfied_revision = None;
        self.completed_seek_restart_baseline = None;
        self.failed_command_attempts = 0;
        self.command_budget_degraded = false;
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

    pub fn update_desired_room_state(
        &mut self,
        desired: DesiredRoomPlayback,
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
        if revision_changed {
            self.pending_commands
                .retain(|command| command.revision >= desired.state_revision);
            self.desired_seek_satisfied_revision =
                (!desired.force_seek).then_some(desired.state_revision);
            if desired.force_seek {
                self.completed_seek_restart_baseline = None;
                self.close_recovery_without_metrics();
            }
            self.retry_not_before_seconds = 0.0;
            self.failed_command_attempts = 0;
            self.command_budget_degraded = false;
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
        let Some(media) = self.media.as_ref() else {
            return Vec::new();
        };
        if observation.media_generation != media.generation {
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
        self.observation_sequence = self.observation_sequence.saturating_add(1);

        self.metrics.last_buffered_ahead_seconds = observation
            .buffered_ahead_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
            .or(self.metrics.last_buffered_ahead_seconds);
        self.metrics.last_input_rate_bytes_per_second = observation
            .input_rate_bytes_per_second
            .or(self.metrics.last_input_rate_bytes_per_second);

        let position_sampled = observation
            .position_seconds
            .is_some_and(|value| value.is_finite() && value >= 0.0);
        let previous = self.observed;
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
            core_idle: None,
            playback_restart_sequence: 0,
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
        if let Some(ranges) = observation.seekable_ranges.as_deref() {
            observed.seekable_window = latest_valid_seekable_window(ranges);
        }
        if let Some(value) = observation.core_idle {
            observed.core_idle = Some(value);
        }
        if let Some(value) = observation.playback_restart_sequence {
            observed.playback_restart_sequence = value;
        }
        self.observed = Some(observed);

        let advancing = previous
            .and_then(|previous| previous.position_seconds)
            .zip(observed.position_seconds)
            .is_some_and(|(previous, current)| current - previous > MIN_ADVANCEMENT_SECONDS)
            && observed.logical_pause == Some(false)
            && !observed.paused_for_cache
            && !observed.seeking;

        self.update_diagnostic_for_observation(observed);
        let recovery_degraded = self.update_recovery_episode(observed, advancing, position_sampled);

        let mut actions = Vec::new();
        if let Some(reason) = recovery_degraded {
            self.enter_degraded_recovery(reason, &mut actions);
        }
        self.complete_observed_commands(observed, advancing);
        self.coordinate_desired_state(observed, advancing, &mut actions);
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

    pub fn command_failed(&mut self, command_id: CoordinatorCommandId, now_seconds: f64) -> bool {
        let original_len = self.pending_commands.len();
        self.pending_commands
            .retain(|command| command.id != command_id);
        if self.pending_commands.len() == original_len {
            return false;
        }
        self.failed_command_attempts = self.failed_command_attempts.saturating_add(1);
        self.retry_not_before_seconds = now_seconds + self.config.command_retry_cooldown_seconds;
        true
    }

    pub fn tick(&mut self, now_seconds: f64) -> Vec<PlaybackCoordinatorAction> {
        if !now_seconds.is_finite() {
            return Vec::new();
        }
        let timed_out = self
            .pending_commands
            .iter()
            .filter(|command| {
                now_seconds - command.issued_at_seconds >= self.config.command_timeout_seconds
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
        let mut actions = timed_out
            .into_iter()
            .map(|command_id| PlaybackCoordinatorAction::CommandTimedOut { command_id })
            .collect::<Vec<_>>();
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
        self.recovery.is_some()
            || self.rate_override.is_some()
            || !self.pending_commands.is_empty()
            || self.desired_revision_pending().is_some()
            || self
                .observed
                .is_some_and(|observed| self.transport_blocks_correction(observed))
    }

    fn update_diagnostic_for_observation(&mut self, observed: ObservedState) {
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
                    catchup_active: false,
                    seek_active: false,
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
        advancing: bool,
        actions: &mut Vec<PlaybackCoordinatorAction>,
    ) {
        let Some(desired) = self.desired else {
            return;
        };
        if desired.media_generation != self.current_media_generation().unwrap_or_default() {
            return;
        }

        if desired.paused {
            if self.transport_blocks_correction(observed) {
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

        self.coordinate_recovery(desired, observed, advancing, actions);
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
        if observed.position_seconds.is_some_and(|position| {
            (position - target).abs() <= self.config.position_tolerance_seconds
        }) {
            self.desired_seek_satisfied_revision = Some(desired.state_revision);
            return;
        }
        if !self.observed_seekable(observed) {
            // Position alignment is impossible, but pause/play intent must not
            // deadlock behind an unsatisfiable seek prerequisite.
            self.desired_seek_satisfied_revision = Some(desired.state_revision);
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

    fn coordinate_recovery(
        &mut self,
        desired: DesiredRoomPlayback,
        observed: ObservedState,
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
        self.metrics.steady_state_skew_seconds = Some(local_position - room_position);

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
                RecoveryPolicy::Balanced if lag < self.config.hard_seek_threshold_seconds => {
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
                        let target_rate = if observation_has_catchup_headroom(&self.metrics) {
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
        let converged =
            (local_position - room_position).abs() <= self.config.negligible_lag_seconds;
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
    ) {
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
            return;
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
        let Some((start, end)) = observed.seekable_window else {
            return target;
        };
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

fn observation_has_catchup_headroom(metrics: &PlaybackCoordinatorMetrics) -> bool {
    metrics
        .last_buffered_ahead_seconds
        .is_some_and(|seconds| seconds >= HEALTHY_CATCHUP_BUFFER_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinator(kind: MediaTransportKind) -> (PlaybackCoordinator, u64) {
        let mut coordinator = PlaybackCoordinator::default();
        let generation = coordinator
            .prepare_media(LogicalMediaId::new("episode-1").unwrap(), kind, 0.0)
            .media_generation;
        (coordinator, generation)
    }

    fn desired(generation: u64, revision: u64, paused: bool, position: f64) -> DesiredRoomPlayback {
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: revision,
            paused,
            anchor_position_seconds: position,
            anchor_observed_at_seconds: 0.0,
            force_seek: false,
        }
    }

    fn playing(generation: u64, at: f64, position: f64) -> PlayerTransportObservation {
        PlayerTransportObservation::new(generation, at)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(position)
            .with_logical_pause(false)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
    }

    fn begin_catchup_override(
        config: PlaybackCoordinatorConfig,
    ) -> (PlaybackCoordinator, u64, f64, CoordinatorCommandId) {
        begin_catchup_override_with_decision_rate(config, None)
    }

    fn begin_catchup_override_with_decision_rate(
        config: PlaybackCoordinatorConfig,
        decision_playback_rate: Option<f64>,
    ) -> (PlaybackCoordinator, u64, f64, CoordinatorCommandId) {
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("catchup-override").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));
        let mut decision = playing(generation, 12.0, 21.0);
        if let Some(playback_rate) = decision_playback_rate {
            decision = decision.with_playback_rate(playback_rate);
        }
        let actions = coordinator.observe(decision);
        let (command_id, target_rate) = actions
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                } if *rate > NORMAL_PLAYBACK_RATE => Some((*command_id, *rate)),
                _ => None,
            })
            .expect("moderate post-buffer lag should start rate catch-up");
        (coordinator, generation, target_rate, command_id)
    }

    fn observe_catchup_override(
        coordinator: &mut PlaybackCoordinator,
        generation: u64,
        target_rate: f64,
        command_id: CoordinatorCommandId,
    ) {
        assert!(coordinator.command_accepted(command_id));
        coordinator.observe(playing(generation, 13.0, 21.5).with_playback_rate(target_rate));
    }

    #[test]
    fn room_advancing_while_buffered_never_emits_a_correction() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 1, false, 10.0));

        for second in 10..20 {
            let actions = coordinator.observe(
                PlayerTransportObservation::new(generation, second as f64)
                    .with_phase(PlayerTransportPhase::Rebuffering)
                    .with_position(10.0)
                    .with_logical_pause(false)
                    .with_cache_pause(true),
            );
            assert!(actions.is_empty());
        }
        assert_eq!(coordinator.metrics().hard_seek_count, 0);
        assert_eq!(coordinator.metrics().buffer_episode_count, 1);
    }

    #[test]
    fn cache_release_with_small_lag_continues_without_seek() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 20.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.4));
        let actions = coordinator.observe(playing(generation, 12.0, 21.4));

        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
        assert_eq!(coordinator.metrics().hard_seek_count, 0);
    }

    #[test]
    fn moderate_lag_without_headroom_uses_conservative_catchup() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));
        let actions = coordinator.observe(playing(generation, 12.0, 21.0));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn buffered_headroom_allows_the_configured_catchup_rate() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        let mut buffering = PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true);
        buffering.buffered_ahead_seconds = Some(5.0);
        coordinator.observe(buffering);
        coordinator.observe(playing(generation, 11.0, 20.5));
        let actions = coordinator.observe(playing(generation, 12.0, 21.0));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - 1.05).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn balanced_catchup_does_not_close_before_position_converges() {
        let config = PlaybackCoordinatorConfig {
            stability_interval_seconds: 1.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("catchup-media").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));
        coordinator.observe(playing(generation, 12.0, 21.0));

        // More than a stability interval of healthy advancement is not enough
        // while the client remains several seconds behind.
        coordinator.observe(playing(generation, 13.5, 22.0));
        assert!(coordinator.recovery_episode().is_some());

        // Once catch-up reaches the moving room anchor, the already-stable
        // episode may close and ordinary steady-state correction can resume.
        coordinator.observe(playing(generation, 16.0, 31.0));
        assert!(coordinator.recovery_episode().is_none());
    }

    #[test]
    fn catchup_that_never_reduces_lag_degrades_at_a_bounded_deadline() {
        let config = PlaybackCoordinatorConfig {
            maximum_catchup_rate: 1.25,
            stability_interval_seconds: 1.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("bounded-catchup").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        let mut stalled = PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true);
        stalled.buffered_ahead_seconds = Some(5.0);
        coordinator.observe(stalled);
        coordinator.observe(playing(generation, 11.0, 20.5));
        coordinator.observe(playing(generation, 12.0, 21.0));
        let deadline = coordinator
            .recovery_episode()
            .and_then(|episode| episode.catchup_deadline_seconds)
            .expect("catchup decision should establish a deadline");

        let mut rate_applied = playing(generation, 13.0, 22.0);
        rate_applied.playback_rate = Some(1.25);
        coordinator.observe(rate_applied);
        let actions = coordinator.observe(playing(
            generation,
            deadline + 0.1,
            22.0 + (deadline + 0.1 - 13.0),
        ));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::CatchupDidNotConverge,
                ..
            }
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - 1.0).abs() < f64::EPSILON
        )));
        assert!(
            coordinator
                .recovery_episode()
                .is_some_and(|episode| episode.degraded)
        );
        coordinator.observe(
            playing(generation, deadline + 0.2, 22.1 + (deadline + 0.1 - 13.0))
                .with_playback_rate(NORMAL_PLAYBACK_RATE),
        );
        assert!(
            coordinator.rate_override.is_none(),
            "degraded recovery must release rate ownership only after baseline telemetry"
        );
    }

    #[test]
    fn repeated_cache_transitions_do_not_reset_hard_seek_budget() {
        let config = PlaybackCoordinatorConfig {
            maximum_hard_seeks_per_episode: 1,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("episode-1").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 40.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 10.2));
        let first = coordinator.observe(playing(generation, 12.0, 10.5));
        assert_eq!(
            first
                .iter()
                .filter(|action| matches!(
                    action,
                    PlaybackCoordinatorAction::Execute {
                        command: CoordinatorPlayerCommand::SetPosition(_),
                        ..
                    }
                ))
                .count(),
            1
        );

        let renewed_stall = coordinator.observe(
            PlayerTransportObservation::new(generation, 13.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.5)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        assert!(renewed_stall.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::HardSeekBudgetExhausted,
                ..
            }
        )));
        coordinator.observe(playing(generation, 14.0, 10.7));
        let second = coordinator.observe(playing(generation, 15.0, 11.0));
        assert!(!second.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
        assert_eq!(coordinator.metrics().hard_seek_count, 1);
        assert!(
            coordinator
                .recovery_episode()
                .is_some_and(|episode| episode.degraded)
        );
    }

    #[test]
    fn large_residual_lag_after_recovery_seek_degrades_without_second_seek() {
        let config = PlaybackCoordinatorConfig {
            maximum_hard_seeks_per_episode: 1,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("episode-1").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 40.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 10.2));
        let first = coordinator.observe(playing(generation, 12.0, 10.5));
        let target = first
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(target),
                    ..
                } => Some(*target),
                _ => None,
            })
            .expect("recovery should issue its one hard seek");

        coordinator.observe(
            PlayerTransportObservation::new(generation, 29.0)
                .with_phase(PlayerTransportPhase::Seeking)
                .with_position(target)
                .with_logical_pause(false)
                .with_seeking(true)
                .with_seekable(true),
        );
        coordinator.observe(playing(generation, 30.0, target));
        let residual = coordinator.observe(playing(generation, 31.0, target + 0.5));

        assert!(!residual.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
        assert!(residual.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::HardSeekBudgetExhausted,
                ..
            }
        )));
        assert_eq!(coordinator.metrics().hard_seek_count, 1);
    }

    #[test]
    fn play_received_during_loading_remains_pending_until_advancement() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 7, false, 5.0));
        assert!(
            coordinator
                .observe(
                    PlayerTransportObservation::new(generation, 1.0)
                        .with_phase(PlayerTransportPhase::Loading)
                        .with_position(0.0)
                        .with_logical_pause(true)
                )
                .is_empty()
        );
        assert_eq!(coordinator.desired_revision_pending(), Some(7));

        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 2.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(5.0)
                .with_logical_pause(true)
                .with_seekable(true),
        );
        let play_id = actions
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::Play(_),
                } => Some(*command_id),
                _ => None,
            })
            .expect("ready player should receive retained play");
        assert!(coordinator.command_accepted(play_id));
        assert_eq!(coordinator.desired_revision_pending(), Some(7));

        coordinator.observe(playing(generation, 3.0, 5.0).with_restart_sequence(1));
        let applied = coordinator.observe(playing(generation, 4.0, 5.5).with_restart_sequence(1));
        assert!(applied.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::RevisionApplied {
                state_revision: 7,
                ..
            }
        )));
        assert_eq!(coordinator.desired_revision_pending(), None);
    }

    #[test]
    fn ready_paused_core_idle_allows_prepare_seek_and_play() {
        let (mut paused_coordinator, paused_generation) =
            coordinator(MediaTransportKind::NetworkVod);
        paused_coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(paused_generation, 1, true, 12.0)
        });
        let seek_actions = paused_coordinator.observe(
            PlayerTransportObservation::new(paused_generation, 1.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(seek_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 12.0).abs() < f64::EPSILON
        )));

        let (mut playing_coordinator, playing_generation) =
            coordinator(MediaTransportKind::NetworkVod);
        playing_coordinator.update_desired_room_state(desired(playing_generation, 1, false, 0.0));
        let play_actions = playing_coordinator.observe(
            PlayerTransportObservation::new(playing_generation, 1.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(play_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterLoad { .. }),
                ..
            }
        )));
    }

    #[test]
    fn authoritative_pause_aligns_position_before_sending_pause() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 1, false, 10.0));
        coordinator.observe(playing(generation, 1.0, 10.0).with_restart_sequence(1));
        coordinator.observe(playing(generation, 1.1, 10.1).with_restart_sequence(1));

        coordinator.update_desired_room_state(DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: true,
            anchor_position_seconds: 12.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        });
        let seek_first = coordinator.observe(playing(generation, 2.0, 10.2));
        assert!(seek_first.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 12.0).abs() < f64::EPSILON
        )));
        assert!(!seek_first.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));

        let pause_second = coordinator.observe(
            playing(generation, 2.1, 12.0)
                .with_seeking(false)
                .with_restart_sequence(2),
        );
        assert!(pause_second.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));
    }

    #[test]
    fn ordinary_resume_completes_without_a_new_playback_restart() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
        let initial = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(initial.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterLoad { .. }),
                ..
            }
        )));
        coordinator.observe(playing(generation, 0.2, 0.0).with_restart_sequence(1));
        coordinator.observe(playing(generation, 0.3, 0.1).with_restart_sequence(1));

        coordinator.update_desired_room_state(desired(generation, 2, true, 0.1));
        coordinator.observe(playing(generation, 0.4, 0.2).with_restart_sequence(1));
        coordinator.observe(
            PlayerTransportObservation::new(generation, 0.5)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.2)
                .with_logical_pause(true)
                .with_cache_pause(false)
                .with_seekable(true)
                .with_core_idle(true)
                .with_restart_sequence(1),
        );

        coordinator.update_desired_room_state(desired(generation, 3, false, 0.2));
        let resume = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.6)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.2)
                .with_logical_pause(true)
                .with_cache_pause(false)
                .with_seekable(true)
                .with_core_idle(true)
                .with_restart_sequence(1),
        );
        assert!(resume.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::Resume),
                ..
            }
        )));
        let resumed = coordinator.observe(
            playing(generation, 0.7, 0.3)
                .with_restart_sequence(1)
                .with_core_idle(false),
        );
        assert!(resumed.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::RevisionApplied {
                state_revision: 3,
                ..
            } | PlaybackCoordinatorAction::Started {
                state_revision: 3,
                ..
            }
        )));
        assert_eq!(coordinator.desired_revision_pending(), None);
    }

    #[test]
    fn play_after_paused_seek_uses_the_seek_start_restart_baseline() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 10.0)
        });
        let seek = coordinator.observe(
            PlayerTransportObservation::new(generation, 1.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true)
                .with_restart_sequence(5),
        );
        assert!(seek.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 10.0).abs() < f64::EPSILON
        )));
        coordinator.observe(
            PlayerTransportObservation::new(generation, 1.1)
                .with_phase(PlayerTransportPhase::Seeking)
                .with_position(10.0)
                .with_logical_pause(true)
                .with_seeking(true)
                .with_seekable(true)
                .with_restart_sequence(5),
        );
        coordinator.observe(
            PlayerTransportObservation::new(generation, 1.2)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(10.0)
                .with_logical_pause(true)
                .with_cache_pause(false)
                .with_seeking(false)
                .with_seekable(true)
                .with_core_idle(true)
                .with_restart_sequence(6),
        );

        coordinator.update_desired_room_state(desired(generation, 2, false, 10.0));
        let play = coordinator.observe(
            PlayerTransportObservation::new(generation, 1.3)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(10.0)
                .with_logical_pause(true)
                .with_cache_pause(false)
                .with_seekable(true)
                .with_core_idle(true)
                .with_restart_sequence(6),
        );
        assert!(play.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterSeek {
                    baseline_restart_sequence: 5,
                }),
                ..
            }
        )));
        let started = coordinator.observe(
            playing(generation, 1.4, 10.1)
                .with_core_idle(false)
                .with_restart_sequence(6),
        );
        assert!(started.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Started {
                state_revision: 2,
                ..
            }
        )));
    }

    #[test]
    fn pause_received_during_recovery_supersedes_catchup_without_a_recovery_seek() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));

        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 12.0,
            ..desired(generation, 2, true, 26.0)
        });
        let actions = coordinator.observe(playing(generation, 12.0, 21.0));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));
        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if *rate > NORMAL_PLAYBACK_RATE
        )));
        assert!(coordinator.recovery_episode().is_none());
    }

    #[test]
    fn explicit_room_seek_supersedes_the_old_recovery_target() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 40.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );

        coordinator.update_desired_room_state(DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 8.0,
            anchor_observed_at_seconds: 11.0,
            force_seek: true,
        });
        let actions = coordinator.observe(playing(generation, 11.0, 10.0));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 8.0).abs() < f64::EPSILON
        )));
        assert!(coordinator.recovery_episode().is_none());
        assert_eq!(coordinator.metrics().hard_seek_count, 0);
    }

    #[test]
    fn pending_desired_revision_and_command_both_block_legacy_correction() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 1, true, 0.0));
        assert!(coordinator.ordinary_correction_blocked());

        coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(!coordinator.ordinary_correction_blocked());

        coordinator.update_desired_room_state(desired(generation, 2, false, 0.0));
        let play = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.2)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(play.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::Play(_),
                ..
            }
        )));
        assert!(coordinator.ordinary_correction_blocked());

        coordinator.observe(
            playing(generation, 0.3, 0.1)
                .with_restart_sequence(1)
                .with_core_idle(false),
        );
        assert!(!coordinator.ordinary_correction_blocked());
    }

    #[test]
    fn newer_forced_revision_supersedes_pending_barrier_seek() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 10.0)
        });
        let first = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(first.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 10.0).abs() < f64::EPSILON
        )));

        coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 20.0)
        });
        let superseding = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.2)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(superseding.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 20.0).abs() < f64::EPSILON
        )));
        assert!(!superseding.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 10.0).abs() < f64::EPSILON
        )));

        let stale_target = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.3)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(10.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(!stale_target.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::RevisionApplied {
                state_revision: 2,
                ..
            }
        )));
        assert_eq!(coordinator.desired_revision_pending(), Some(2));

        let applied = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.4)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(20.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_core_idle(true),
        );
        assert!(applied.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::RevisionApplied {
                state_revision: 2,
                ..
            }
        )));
        assert_eq!(coordinator.desired_revision_pending(), None);
    }

    #[test]
    fn accepted_command_without_matching_observation_times_out_and_stays_pending() {
        let config = PlaybackCoordinatorConfig {
            command_timeout_seconds: 1.0,
            command_retry_budget: 0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("timeout-media").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(desired(generation, 9, false, 0.0));
        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true),
        );
        let command_id = actions
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::Play(_),
                } => Some(*command_id),
                _ => None,
            })
            .expect("retained desired play should emit a tracked command");
        assert!(coordinator.command_accepted(command_id));

        let timed_out = coordinator.tick(1.2);

        assert!(timed_out.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::CommandTimedOut { command_id: id } if *id == command_id
        )));
        assert!(timed_out.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
                ..
            }
        )));
        assert_eq!(coordinator.desired_revision_pending(), Some(9));
        assert_eq!(coordinator.metrics().command_timeouts, 1);
    }

    #[test]
    fn rejected_command_exhausts_budget_once_and_waits_for_new_room_intent() {
        let config = PlaybackCoordinatorConfig {
            command_retry_budget: 0,
            command_retry_cooldown_seconds: 0.1,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("rejected-command").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
        let ready_paused = PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true);
        let first = coordinator.observe(ready_paused.clone());
        let command_id = first
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute { command_id, .. } => Some(*command_id),
                _ => None,
            })
            .expect("play command should be issued");
        assert!(coordinator.command_failed(command_id, 0.2));

        let degraded = coordinator.tick(0.2);
        assert_eq!(
            degraded
                .iter()
                .filter(|action| matches!(
                    action,
                    PlaybackCoordinatorAction::Degraded {
                        reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(coordinator.tick(1.0).is_empty());
        assert!(coordinator.observe(ready_paused).is_empty());

        coordinator.update_desired_room_state(desired(generation, 2, false, 0.0));
        assert!(
            coordinator
                .observe(
                    PlayerTransportObservation::new(generation, 1.1)
                        .with_phase(PlayerTransportPhase::ReadyPaused)
                        .with_position(0.0)
                        .with_logical_pause(true)
                        .with_seekable(true),
                )
                .iter()
                .any(|action| matches!(action, PlaybackCoordinatorAction::Execute { .. }))
        );
    }

    #[test]
    fn playback_restart_without_advancement_does_not_acknowledge_desired_play() {
        let config = PlaybackCoordinatorConfig {
            command_timeout_seconds: 1.0,
            command_retry_budget: 0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("restart-without-advance").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(desired(generation, 3, false, 4.0));
        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(4.0)
                .with_logical_pause(true)
                .with_seekable(true),
        );
        let play_id = actions
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::Play(_),
                } => Some(*command_id),
                _ => None,
            })
            .expect("ready transport should receive retained play");
        coordinator.command_accepted(play_id);

        let restarted = coordinator.observe(playing(generation, 0.2, 4.0).with_restart_sequence(1));
        assert!(!restarted.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::RevisionApplied { .. }
                | PlaybackCoordinatorAction::Started { .. }
        )));
        assert_eq!(coordinator.desired_revision_pending(), Some(3));

        let timed_out = coordinator.tick(1.2);
        assert!(timed_out.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::CommandTimedOut { command_id } if *command_id == play_id
        )));
        assert!(timed_out.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
                ..
            }
        )));
    }

    #[test]
    fn non_seekable_large_lag_degrades_instead_of_chasing_the_room() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NonSeekable);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 40.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true)
                .with_seekable(false),
        );
        coordinator.observe(playing(generation, 11.0, 10.2).with_seekable(false));
        let actions = coordinator.observe(playing(generation, 12.0, 10.5).with_seekable(false));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::NonSeekableLag,
                ..
            }
        )));
        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
        assert_eq!(coordinator.metrics().hard_seek_count, 0);
    }

    #[test]
    fn non_seekable_moderate_lag_terminates_recovery_explicitly() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NonSeekable);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true)
                .with_seekable(false),
        );
        coordinator.observe(playing(generation, 11.0, 20.5).with_seekable(false));
        let actions = coordinator.observe(playing(generation, 12.0, 21.0).with_seekable(false));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::NonSeekableLag,
                ..
            }
        )));
        assert!(
            coordinator
                .recovery_episode()
                .is_some_and(|episode| episode.degraded)
        );
        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_)
                    | CoordinatorPlayerCommand::SetPlaybackRate(_),
                ..
            }
        )));
    }

    #[test]
    fn preserve_content_reports_degraded_lag_and_keeps_seek_correction_blocked() {
        let config = PlaybackCoordinatorConfig {
            recovery_policy: RecoveryPolicy::PreserveContent,
            stability_interval_seconds: 1.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("preserve-content").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 40.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 10.2));
        coordinator.observe(playing(generation, 12.0, 10.5));
        let actions = coordinator.observe(playing(generation, 13.1, 11.0));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::NonSeekableLag,
                ..
            }
        )));
        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
        assert!(coordinator.ordinary_correction_blocked());
    }

    #[test]
    fn recovery_stability_requires_continuous_position_advancement() {
        let config = PlaybackCoordinatorConfig {
            recovery_policy: RecoveryPolicy::PreserveContent,
            stability_interval_seconds: 2.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let generation = coordinator
            .prepare_media(
                LogicalMediaId::new("stable-media").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
        coordinator.observe(
            PlayerTransportObservation::new(generation, 0.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(0.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 1.0, 1.0));
        coordinator.observe(playing(generation, 2.0, 2.0));
        assert_eq!(
            coordinator
                .recovery_episode()
                .and_then(|episode| episode.stable_since_seconds),
            Some(2.0)
        );

        // A sparse lifecycle observation is neutral, but a fresh position
        // sample without advancement restarts the stable interval.
        coordinator.observe(
            PlayerTransportObservation::new(generation, 2.5)
                .with_phase(PlayerTransportPhase::Playing),
        );
        assert_eq!(
            coordinator
                .recovery_episode()
                .and_then(|episode| episode.stable_since_seconds),
            Some(2.0)
        );
        coordinator.observe(playing(generation, 3.0, 2.0));
        assert_eq!(
            coordinator
                .recovery_episode()
                .and_then(|episode| episode.stable_since_seconds),
            None
        );

        coordinator.observe(playing(generation, 4.0, 4.0));
        coordinator.observe(playing(generation, 5.9, 5.9));
        assert!(coordinator.recovery_episode().is_some());
        coordinator.observe(playing(generation, 6.1, 6.1));
        assert!(coordinator.recovery_episode().is_none());
    }

    #[test]
    fn live_recovery_seek_is_clamped_behind_the_latest_seekable_edge() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::LiveSliding);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 200.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(85.0)
                .with_logical_pause(false)
                .with_cache_pause(true)
                .with_seekable(true)
                .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)]),
        );
        coordinator.observe(playing(generation, 11.0, 85.1));
        let actions = coordinator.observe(playing(generation, 12.0, 85.3));

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 99.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn offset_shifted_seekable_window_keeps_its_nonnegative_portion() {
        assert_eq!(
            latest_valid_seekable_window(&[PlayerSeekableRange::new(-5.0, 95.0)]),
            Some((0.0, 95.0))
        );
    }

    #[test]
    fn vod_forced_seek_is_not_clamped_to_the_current_cache_range() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 5.0)
        });

        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 1.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(1.0)
                .with_logical_pause(true)
                .with_seekable(true)
                .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 2.0)])
                .with_core_idle(true),
        );

        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 5.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn stale_generation_observation_cannot_mutate_current_state() {
        let (mut coordinator, old_generation) = coordinator(MediaTransportKind::NetworkVod);
        let current_generation = coordinator
            .prepare_media(
                LogicalMediaId::new("episode-2").unwrap(),
                MediaTransportKind::NetworkVod,
                1.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(desired(current_generation, 1, false, 0.0));

        let actions = coordinator.observe(playing(old_generation, 2.0, 100.0));
        assert!(actions.is_empty());
        assert_eq!(coordinator.metrics().stale_generation_observations, 1);
        assert_eq!(coordinator.desired_revision_pending(), Some(1));
    }

    #[test]
    fn url_refresh_preserves_logical_generation() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
        coordinator.observe(playing(generation, 0.1, 0.0));
        coordinator.observe(playing(generation, 0.2, 0.1));
        assert_eq!(coordinator.desired_revision_pending(), None);

        let refreshed = coordinator.prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            10.0,
        );

        assert_eq!(refreshed.media_generation, generation);
        assert_eq!(refreshed.load_attempt, 2);
        assert!(!refreshed.logical_media_changed);
        assert_eq!(coordinator.desired_revision_pending(), Some(1));

        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true),
        );
        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 10.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn replay_of_same_logical_media_allocates_a_new_playback_generation() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        let replay = coordinator.prepare_media_with_intent(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::Replay,
            10.0,
        );

        assert_ne!(replay.media_generation, generation);
        assert_eq!(replay.load_attempt, 1);
        assert!(!replay.logical_media_changed);
        assert!(replay.playback_episode_changed);
        assert_eq!(replay.load_intent, MediaLoadIntent::Replay);
    }

    #[test]
    fn adapter_epoch_reset_retains_rate_ownership_until_baseline_is_observed() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));
        coordinator.observe(playing(generation, 12.0, 21.0));
        coordinator.observe(
            playing(generation, 13.0, 21.5)
                .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
        );

        coordinator.reset_transport_adapter_epoch(14.0);
        let reset = coordinator.observe(
            PlayerTransportObservation::new(generation, 14.1)
                .with_phase(PlayerTransportPhase::Loading)
                .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM)
                .with_core_idle(true),
        );
        assert!(reset.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
        )));
        assert!(coordinator.ordinary_correction_blocked());

        coordinator.observe(
            PlayerTransportObservation::new(generation, 14.2)
                .with_phase(PlayerTransportPhase::Loading)
                .with_playback_rate(NORMAL_PLAYBACK_RATE)
                .with_core_idle(true),
        );
        assert!(coordinator.rate_override.is_none());
    }

    #[test]
    fn failed_rate_reset_retries_without_reenabling_legacy_correction() {
        let config = PlaybackCoordinatorConfig {
            command_retry_cooldown_seconds: 0.1,
            ..PlaybackCoordinatorConfig::default()
        };
        let (mut coordinator, generation, target_rate, catchup_command_id) =
            begin_catchup_override(config);
        observe_catchup_override(
            &mut coordinator,
            generation,
            target_rate,
            catchup_command_id,
        );

        let reset = coordinator.interrupt_recovery();
        let reset_command_id = reset
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
                _ => None,
            })
            .expect("interrupting catch-up should restore the baseline rate");
        assert!(coordinator.command_failed(reset_command_id, 13.01));
        assert!(
            coordinator.ordinary_correction_blocked(),
            "failed cleanup must retain exclusive coordinator ownership"
        );

        assert!(coordinator.tick(13.05).is_empty());
        let retry = coordinator.tick(13.12);
        assert!(retry.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
        )));
    }

    #[test]
    fn timed_out_rate_reset_retries_after_its_cooldown() {
        let config = PlaybackCoordinatorConfig {
            command_timeout_seconds: 0.2,
            command_retry_budget: 5,
            command_retry_cooldown_seconds: 0.1,
            ..PlaybackCoordinatorConfig::default()
        };
        let (mut coordinator, generation, target_rate, catchup_command_id) =
            begin_catchup_override(config);
        observe_catchup_override(
            &mut coordinator,
            generation,
            target_rate,
            catchup_command_id,
        );

        let reset = coordinator.interrupt_recovery();
        let reset_command_id = reset
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
                _ => None,
            })
            .expect("interrupting catch-up should restore the baseline rate");
        assert!(coordinator.command_accepted(reset_command_id));

        let timed_out = coordinator.tick(13.21);
        assert!(timed_out.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::CommandTimedOut { command_id }
                if *command_id == reset_command_id
        )));
        assert!(coordinator.ordinary_correction_blocked());
        let retry = coordinator.tick(13.32);
        assert!(retry.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
        )));
    }

    #[test]
    fn command_budget_degradation_cannot_strand_a_catchup_rate() {
        let config = PlaybackCoordinatorConfig {
            command_timeout_seconds: 0.2,
            command_retry_budget: 0,
            command_retry_cooldown_seconds: 0.1,
            ..PlaybackCoordinatorConfig::default()
        };
        let (mut coordinator, _generation, _target_rate, catchup_command_id) =
            begin_catchup_override(config);
        assert!(coordinator.command_accepted(catchup_command_id));

        let degraded = coordinator.tick(12.21);
        assert!(degraded.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Degraded {
                reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
                ..
            }
        )));
        assert!(coordinator.command_budget_degraded);
        assert!(
            coordinator
                .rate_override
                .is_some_and(|rate| rate.reset_requested)
        );

        let reset = coordinator.tick(12.32);
        assert!(
            reset.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                    ..
                } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
            )),
            "baseline reset must bypass the exhausted recovery-command budget"
        );
    }

    #[test]
    fn decision_time_baseline_does_not_release_catchup_or_its_later_cleanup() {
        let (mut coordinator, generation, target_rate, catchup_command_id) =
            begin_catchup_override_with_decision_rate(
                PlaybackCoordinatorConfig::default(),
                Some(NORMAL_PLAYBACK_RATE),
            );
        assert!(
            coordinator.rate_override.is_some(),
            "the pre-command baseline sample must not discard catch-up ownership"
        );
        assert!(coordinator.ordinary_correction_blocked());
        observe_catchup_override(
            &mut coordinator,
            generation,
            target_rate,
            catchup_command_id,
        );

        let reset = coordinator.interrupt_recovery();
        let reset_command_id = reset
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
                _ => None,
            })
            .expect("interrupting catch-up should restore the baseline rate");
        assert!(coordinator.command_accepted(reset_command_id));
        assert!(coordinator.ordinary_correction_blocked());

        coordinator.observe(playing(generation, 13.1, 21.6));
        assert!(
            coordinator.ordinary_correction_blocked(),
            "a sparse post-command sample must not reuse the pre-command baseline rate"
        );

        coordinator.observe(playing(generation, 13.2, 21.7).with_playback_rate(target_rate));
        assert!(
            coordinator.ordinary_correction_blocked(),
            "target-rate telemetry cannot release cleanup ownership"
        );

        coordinator
            .observe(playing(generation, 13.3, 21.8).with_playback_rate(NORMAL_PLAYBACK_RATE));
        assert!(coordinator.rate_override.is_none());
        assert!(
            !coordinator.ordinary_correction_blocked(),
            "only a fresh post-reset baseline observation releases ownership"
        );
    }

    #[test]
    fn repeated_recovery_interruption_keeps_the_existing_baseline_reset_tracked() {
        let (mut coordinator, generation, target_rate, catchup_command_id) =
            begin_catchup_override(PlaybackCoordinatorConfig::default());
        observe_catchup_override(
            &mut coordinator,
            generation,
            target_rate,
            catchup_command_id,
        );

        let first_reset = coordinator.interrupt_recovery();
        let reset_command_id = first_reset
            .iter()
            .find_map(|action| match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
                _ => None,
            })
            .expect("the first interruption should request baseline speed");

        assert!(
            coordinator.interrupt_recovery().is_empty(),
            "repeated lifecycle reconciliation must not replace a pending reset"
        );
        assert!(coordinator.pending_commands.iter().any(|command| {
            command.id == reset_command_id
                && matches!(
                    command.kind,
                    PendingCommandKind::Rate { target_rate }
                        if (target_rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
                )
        }));

        coordinator
            .observe(playing(generation, 13.1, 21.6).with_playback_rate(NORMAL_PLAYBACK_RATE));
        assert!(coordinator.rate_override.is_none());
    }

    #[test]
    fn barrier_supersession_resets_catchup_before_applying_its_forced_seek() {
        let (mut coordinator, generation, target_rate, catchup_command_id) =
            begin_catchup_override(PlaybackCoordinatorConfig::default());
        observe_catchup_override(
            &mut coordinator,
            generation,
            target_rate,
            catchup_command_id,
        );

        let cleanup = coordinator.update_desired_room_state(DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: true,
            anchor_position_seconds: 30.0,
            anchor_observed_at_seconds: 14.0,
            force_seek: true,
        });
        assert!(cleanup.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
        )));
        assert!(coordinator.recovery_episode().is_none());
        assert!(coordinator.rate_override.is_some());

        let forced_seek =
            coordinator.observe(playing(generation, 14.1, 21.8).with_playback_rate(target_rate));
        assert!(forced_seek.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 30.0).abs() <= f64::EPSILON
        )));
    }

    #[test]
    fn pause_and_transport_refresh_reset_owned_catchup_rate() {
        let (mut pause_coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        pause_coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        pause_coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        pause_coordinator.observe(playing(generation, 11.0, 20.5));
        pause_coordinator.observe(playing(generation, 12.0, 21.0));
        pause_coordinator.observe(
            playing(generation, 13.0, 21.5)
                .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
        );

        let pause_actions = pause_coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 13.0,
            ..desired(generation, 2, true, 26.5)
        });
        assert!(pause_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
        )));

        // Re-enter catch-up, then refresh the same transport identity. The
        // new attempt must still restore the baseline before it can resume.
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            anchor_observed_at_seconds: 10.0,
            ..desired(generation, 1, false, 25.0)
        });
        coordinator.observe(
            PlayerTransportObservation::new(generation, 10.0)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(20.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        coordinator.observe(playing(generation, 11.0, 20.5));
        coordinator.observe(playing(generation, 12.0, 21.0));
        coordinator.observe(
            playing(generation, 13.0, 21.5)
                .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
        );
        let refresh = coordinator.prepare_media_with_intent(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::TransportRefresh,
            14.0,
        );
        let refresh_actions = coordinator.observe(
            PlayerTransportObservation::new(refresh.media_generation, 14.1)
                .with_phase(PlayerTransportPhase::Loading)
                .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM)
                .with_core_idle(true),
        );
        assert!(refresh_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn local_file_reaches_started_without_recovery_actions() {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::LocalFile);
        coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
        coordinator.observe(
            PlayerTransportObservation::new(generation, 0.1)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(0.0)
                .with_logical_pause(true)
                .with_seekable(true),
        );
        coordinator.observe(playing(generation, 0.2, 0.0).with_restart_sequence(1));
        let actions = coordinator.observe(playing(generation, 0.3, 0.1).with_restart_sequence(1));

        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PlaybackCoordinatorAction::Started { .. }))
        );
        assert_eq!(coordinator.metrics().buffer_episode_count, 0);
    }
}
