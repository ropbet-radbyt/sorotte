use std::collections::BTreeMap;

use sorotte_player_api::{PlayerCommandId, PlayerTransportPhase};

/// Causal owner of a pause/play command issued by Sorotte.
///
/// Only [`Self::LocalUserPlaybackControl`] represents user readiness intent.
/// Every other cause is system-owned and must not be projected into a user
/// readiness mutation when its player observation arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerCommandCause {
    LocalUserPlaybackControl,
    RemoteRoomSynchronization,
    AutomaticReadinessStart,
    ReadinessGateHold,
    RoomBufferingPolicy,
    SeekPreparation,
    DesyncCorrection,
    Recovery,
    MediaLoading,
    PlaylistTransition,
    TransportRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerCommandCompletion {
    Pending,
    Completed { at_seconds: f64 },
    TimedOut { at_seconds: f64 },
    Failed { at_seconds: f64 },
    Superseded { at_seconds: f64 },
}

impl PlayerCommandCompletion {
    fn terminal_at_seconds(self) -> Option<f64> {
        match self {
            Self::Pending => None,
            Self::Completed { at_seconds }
            | Self::TimedOut { at_seconds }
            | Self::Failed { at_seconds }
            | Self::Superseded { at_seconds } => Some(at_seconds),
        }
    }
}

/// Registration metadata retained for one Sorotte-issued pause/play command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerCommandRegistration {
    pub command_id: PlayerCommandId,
    pub media_generation: u64,
    pub adapter_epoch: u64,
    pub cause: PlayerCommandCause,
    pub desired_paused: bool,
    pub issued_at_seconds: f64,
    pub completion: PlayerCommandCompletion,
}

impl PlayerCommandRegistration {
    pub fn new(
        command_id: PlayerCommandId,
        media_generation: u64,
        adapter_epoch: u64,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
    ) -> Self {
        Self {
            command_id,
            media_generation,
            adapter_epoch,
            cause,
            desired_paused,
            issued_at_seconds,
            completion: PlayerCommandCompletion::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerTransitionClassifierConfig {
    /// Minimum time covered by two stable observations before an unowned edge
    /// may be accepted as a native player gesture.
    pub stable_confirmation_seconds: f64,
    /// How long a terminal command remains able to own a late observation.
    pub command_observation_grace_seconds: f64,
    /// Maximum time a command that never reports terminal completion may own
    /// transport observations. This deadline is measured from issue time so a
    /// silent adapter cannot retain causal ownership indefinitely.
    pub pending_command_timeout_seconds: f64,
    /// Maximum time an unowned, unstable edge remains pending before it is
    /// conservatively recorded as unknown-origin.
    pub unknown_origin_timeout_seconds: f64,
}

impl Default for PlayerTransitionClassifierConfig {
    fn default() -> Self {
        Self {
            stable_confirmation_seconds: 0.1,
            command_observation_grace_seconds: 1.0,
            pending_command_timeout_seconds: 5.0,
            unknown_origin_timeout_seconds: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerTransitionTechnicalReason {
    Empty,
    Loading,
    Prebuffering,
    Rebuffering,
    Seeking,
    Ended,
    Failed,
    CachePause,
    MediaTransition,
    Recovery,
    SeekPreparation,
    RoomBufferingPolicy,
    PlaybackBarrier,
    Synchronization,
}

/// Full, merged context used to decide whether a logical pause edge is stable.
/// Sparse adapter updates should be merged before constructing this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerTransitionContext {
    phase: Option<PlayerTransportPhase>,
    paused_for_cache: bool,
    seeking: bool,
    media_transition_active: bool,
    recovery_active: bool,
    seek_preparation_active: bool,
    room_buffering_policy_active: bool,
    playback_barrier_active: bool,
    synchronization_active: bool,
}

impl PlayerTransitionContext {
    pub const fn new(phase: Option<PlayerTransportPhase>) -> Self {
        Self {
            phase,
            paused_for_cache: false,
            seeking: false,
            media_transition_active: false,
            recovery_active: false,
            seek_preparation_active: false,
            room_buffering_policy_active: false,
            playback_barrier_active: false,
            synchronization_active: false,
        }
    }

    pub const fn with_paused_for_cache(mut self, active: bool) -> Self {
        self.paused_for_cache = active;
        self
    }

    pub const fn with_seeking(mut self, active: bool) -> Self {
        self.seeking = active;
        self
    }

    pub const fn with_media_transition(mut self, active: bool) -> Self {
        self.media_transition_active = active;
        self
    }

    pub const fn with_recovery(mut self, active: bool) -> Self {
        self.recovery_active = active;
        self
    }

    pub const fn with_seek_preparation(mut self, active: bool) -> Self {
        self.seek_preparation_active = active;
        self
    }

    pub const fn with_room_buffering_policy(mut self, active: bool) -> Self {
        self.room_buffering_policy_active = active;
        self
    }

    pub const fn with_playback_barrier(mut self, active: bool) -> Self {
        self.playback_barrier_active = active;
        self
    }

    pub const fn with_synchronization(mut self, active: bool) -> Self {
        self.synchronization_active = active;
        self
    }

    pub const fn phase(self) -> Option<PlayerTransportPhase> {
        self.phase
    }

    fn technical_reason(self) -> Option<PlayerTransitionTechnicalReason> {
        if self.paused_for_cache {
            return Some(PlayerTransitionTechnicalReason::CachePause);
        }
        if self.seeking {
            return Some(PlayerTransitionTechnicalReason::Seeking);
        }
        if self.media_transition_active {
            return Some(PlayerTransitionTechnicalReason::MediaTransition);
        }
        if self.recovery_active {
            return Some(PlayerTransitionTechnicalReason::Recovery);
        }
        if self.seek_preparation_active {
            return Some(PlayerTransitionTechnicalReason::SeekPreparation);
        }
        if self.room_buffering_policy_active {
            return Some(PlayerTransitionTechnicalReason::RoomBufferingPolicy);
        }
        if self.playback_barrier_active {
            return Some(PlayerTransitionTechnicalReason::PlaybackBarrier);
        }
        if self.synchronization_active {
            return Some(PlayerTransitionTechnicalReason::Synchronization);
        }
        match self.phase {
            Some(PlayerTransportPhase::Empty) => Some(PlayerTransitionTechnicalReason::Empty),
            Some(PlayerTransportPhase::Loading) => Some(PlayerTransitionTechnicalReason::Loading),
            Some(PlayerTransportPhase::Prebuffering) => {
                Some(PlayerTransitionTechnicalReason::Prebuffering)
            }
            Some(PlayerTransportPhase::Rebuffering) => {
                Some(PlayerTransitionTechnicalReason::Rebuffering)
            }
            Some(PlayerTransportPhase::Seeking) => Some(PlayerTransitionTechnicalReason::Seeking),
            Some(PlayerTransportPhase::Ended) => Some(PlayerTransitionTechnicalReason::Ended),
            Some(PlayerTransportPhase::Failed) => Some(PlayerTransitionTechnicalReason::Failed),
            Some(PlayerTransportPhase::ReadyPaused | PlayerTransportPhase::Playing) | None => None,
        }
    }

    fn stable_for(self, logical_paused: bool) -> bool {
        matches!(
            (self.phase, logical_paused),
            (Some(PlayerTransportPhase::ReadyPaused), true)
                | (Some(PlayerTransportPhase::Playing), false)
        ) && self.technical_reason().is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerLogicalPauseObservation {
    pub media_generation: u64,
    pub adapter_epoch: u64,
    pub observed_at_seconds: f64,
    pub logical_paused: bool,
    pub context: PlayerTransitionContext,
}

impl PlayerLogicalPauseObservation {
    pub const fn new(
        media_generation: u64,
        adapter_epoch: u64,
        observed_at_seconds: f64,
        logical_paused: bool,
        context: PlayerTransitionContext,
    ) -> Self {
        Self {
            media_generation,
            adapter_epoch,
            observed_at_seconds,
            logical_paused,
            context,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePlayerAction {
    Play,
    Pause,
}

impl NativePlayerAction {
    const fn from_paused(paused: bool) -> Self {
        if paused { Self::Pause } else { Self::Play }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerTransitionUnknownReason {
    InitialObservation,
    ActiveCommandDidNotMatch,
    AmbiguousCommandMatch,
    UnstableTransitionTimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerTransitionIgnoredReason {
    NoActiveScope,
    ScopeMismatch,
    InvalidTimestamp,
    NonMonotonicTimestamp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerTransitionClassification {
    OwnedCommand {
        command_id: PlayerCommandId,
        cause: PlayerCommandCause,
        completion: PlayerCommandCompletion,
    },
    Technical {
        action: NativePlayerAction,
        reason: PlayerTransitionTechnicalReason,
    },
    AwaitingStability {
        action: NativePlayerAction,
        first_observed_at_seconds: f64,
    },
    NativePlayerGesture {
        action: NativePlayerAction,
    },
    UnknownOrigin {
        action: NativePlayerAction,
        reason: PlayerTransitionUnknownReason,
    },
    Duplicate,
    Ignored {
        reason: PlayerTransitionIgnoredReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PlayerCommandKey {
    adapter_epoch: u64,
    command_id: PlayerCommandId,
}

#[derive(Debug, Clone, Copy)]
struct RegisteredCommand {
    registration: PlayerCommandRegistration,
    ownership_deadline_seconds: Option<f64>,
    observation_matched: bool,
}

impl RegisteredCommand {
    fn matchable_at(self, observed_at_seconds: f64) -> bool {
        !self.observation_matched
            && !matches!(
                self.registration.completion,
                PlayerCommandCompletion::Superseded { .. }
            )
            && observed_at_seconds >= self.registration.issued_at_seconds
            && self
                .ownership_deadline_seconds
                .is_none_or(|deadline| observed_at_seconds <= deadline)
    }

    fn active_at(self, observed_at_seconds: f64) -> bool {
        !self.observation_matched
            && observed_at_seconds >= self.registration.issued_at_seconds
            && self
                .ownership_deadline_seconds
                .is_none_or(|deadline| observed_at_seconds <= deadline)
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingStableEdge {
    logical_paused: bool,
    first_observed_at_seconds: f64,
}

enum CommandMatch {
    None,
    One(PlayerCommandKey),
    Ambiguous,
}

/// Pure reducer that classifies logical pause transitions without producing
/// readiness or player side effects.
#[derive(Debug)]
pub struct PlayerTransitionClassifier {
    config: PlayerTransitionClassifierConfig,
    active_media_generation: Option<u64>,
    active_adapter_epoch: Option<u64>,
    commands: BTreeMap<PlayerCommandKey, RegisteredCommand>,
    accepted_logical_paused: Option<bool>,
    last_observed_at_seconds: Option<f64>,
    last_observed_logical_paused: Option<bool>,
    pending_edge: Option<PendingStableEdge>,
}

impl Default for PlayerTransitionClassifier {
    fn default() -> Self {
        Self::new(PlayerTransitionClassifierConfig::default())
    }
}

impl PlayerTransitionClassifier {
    pub fn new(config: PlayerTransitionClassifierConfig) -> Self {
        let defaults = PlayerTransitionClassifierConfig::default();
        let stable_confirmation_seconds = normalized_nonnegative_seconds(
            config.stable_confirmation_seconds,
            defaults.stable_confirmation_seconds,
        );
        let command_observation_grace_seconds = normalized_nonnegative_seconds(
            config.command_observation_grace_seconds,
            defaults.command_observation_grace_seconds,
        );
        let pending_command_timeout_seconds = normalized_nonnegative_seconds(
            config.pending_command_timeout_seconds,
            defaults.pending_command_timeout_seconds,
        );
        let unknown_origin_timeout_seconds = normalized_nonnegative_seconds(
            config.unknown_origin_timeout_seconds,
            defaults.unknown_origin_timeout_seconds,
        )
        .max(stable_confirmation_seconds);
        Self {
            config: PlayerTransitionClassifierConfig {
                stable_confirmation_seconds,
                command_observation_grace_seconds,
                pending_command_timeout_seconds,
                unknown_origin_timeout_seconds,
            },
            active_media_generation: None,
            active_adapter_epoch: None,
            commands: BTreeMap::new(),
            accepted_logical_paused: None,
            last_observed_at_seconds: None,
            last_observed_logical_paused: None,
            pending_edge: None,
        }
    }

    /// Begins a media/adapter scope. The first unowned logical state in the
    /// new scope establishes a baseline and can never become a gesture.
    pub fn begin_scope(&mut self, media_generation: u64, adapter_epoch: u64) -> bool {
        if media_generation == 0 || adapter_epoch == 0 {
            return false;
        }
        self.active_media_generation = Some(media_generation);
        self.active_adapter_epoch = Some(adapter_epoch);
        self.commands.clear();
        self.accepted_logical_paused = None;
        self.last_observed_at_seconds = None;
        self.last_observed_logical_paused = None;
        self.pending_edge = None;
        true
    }

    pub fn register_command(&mut self, registration: PlayerCommandRegistration) -> bool {
        if Some(registration.media_generation) != self.active_media_generation
            || Some(registration.adapter_epoch) != self.active_adapter_epoch
            || registration.media_generation == 0
            || registration.adapter_epoch == 0
            || !registration.issued_at_seconds.is_finite()
            || registration.command_id.get() == 0
            || !valid_completion(registration.completion, registration.issued_at_seconds)
            || registration
                .completion
                .terminal_at_seconds()
                .is_some_and(|at| {
                    at > registration.issued_at_seconds
                        + self.config.pending_command_timeout_seconds
                })
        {
            return false;
        }
        let key = PlayerCommandKey {
            adapter_epoch: registration.adapter_epoch,
            command_id: registration.command_id,
        };
        if self.commands.contains_key(&key) {
            return false;
        }
        let ownership_deadline_seconds =
            Some(registration.completion.terminal_at_seconds().map_or(
                registration.issued_at_seconds + self.config.pending_command_timeout_seconds,
                |at| at + self.config.command_observation_grace_seconds,
            ));
        self.commands.insert(
            key,
            RegisteredCommand {
                registration,
                ownership_deadline_seconds,
                observation_matched: false,
            },
        );
        true
    }

    /// Updates command completion while retaining its observation-ownership
    /// tombstone. The first terminal result replaces the absolute pending
    /// deadline with its observation grace; later terminal results may extend,
    /// but never shorten, that grace window.
    pub fn update_command_completion(
        &mut self,
        adapter_epoch: u64,
        command_id: PlayerCommandId,
        completion: PlayerCommandCompletion,
    ) -> bool {
        let key = PlayerCommandKey {
            adapter_epoch,
            command_id,
        };
        let Some(command) = self.commands.get_mut(&key) else {
            return false;
        };
        let Some(at_seconds) = completion.terminal_at_seconds() else {
            return false;
        };
        if !at_seconds.is_finite()
            || at_seconds < command.registration.issued_at_seconds
            || (command
                .registration
                .completion
                .terminal_at_seconds()
                .is_none()
                && at_seconds
                    > command.registration.issued_at_seconds
                        + self.config.pending_command_timeout_seconds)
            || (command
                .registration
                .completion
                .terminal_at_seconds()
                .is_some()
                && command
                    .ownership_deadline_seconds
                    .is_some_and(|deadline| at_seconds > deadline))
            || command
                .registration
                .completion
                .terminal_at_seconds()
                .is_some_and(|current| at_seconds < current)
        {
            return false;
        }
        let previous_terminal_at = command.registration.completion.terminal_at_seconds();
        command.registration.completion = completion;
        let deadline = at_seconds + self.config.command_observation_grace_seconds;
        command.ownership_deadline_seconds = Some(if previous_terminal_at.is_some() {
            command
                .ownership_deadline_seconds
                .map_or(deadline, |current| current.max(deadline))
        } else {
            deadline
        });
        true
    }

    /// Explicitly retires unmatched commands owned by one causal source.
    /// Superseded commands remain as short-lived tombstones so a late opposite
    /// edge is unknown-origin, but they cannot compete with the replacement
    /// command for an exact match.
    pub fn supersede_unmatched_commands(
        &mut self,
        cause: PlayerCommandCause,
        at_seconds: f64,
    ) -> usize {
        if !at_seconds.is_finite() {
            return 0;
        }
        let keys = self
            .commands
            .iter()
            .filter_map(|(key, command)| {
                (!command.observation_matched
                    && command.registration.cause == cause
                    && at_seconds >= command.registration.issued_at_seconds)
                    .then_some(*key)
            })
            .collect::<Vec<_>>();
        let mut superseded = 0;
        for key in keys {
            if self.update_command_completion(
                key.adapter_epoch,
                key.command_id,
                PlayerCommandCompletion::Superseded { at_seconds },
            ) {
                superseded += 1;
            }
        }
        superseded
    }

    pub fn command_registration(
        &self,
        adapter_epoch: u64,
        command_id: PlayerCommandId,
    ) -> Option<PlayerCommandRegistration> {
        self.commands
            .get(&PlayerCommandKey {
                adapter_epoch,
                command_id,
            })
            .map(|command| command.registration)
    }

    pub fn classify(
        &mut self,
        observation: PlayerLogicalPauseObservation,
    ) -> PlayerTransitionClassification {
        if self.active_media_generation.is_none() || self.active_adapter_epoch.is_none() {
            return PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::NoActiveScope,
            };
        }
        if self.active_media_generation != Some(observation.media_generation)
            || self.active_adapter_epoch != Some(observation.adapter_epoch)
        {
            return PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::ScopeMismatch,
            };
        }
        if !observation.observed_at_seconds.is_finite() {
            return PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::InvalidTimestamp,
            };
        }
        if let Some(last_observed_at_seconds) = self.last_observed_at_seconds
            && observation.observed_at_seconds <= last_observed_at_seconds
        {
            if observation.observed_at_seconds == last_observed_at_seconds
                && self.last_observed_logical_paused == Some(observation.logical_paused)
            {
                return PlayerTransitionClassification::Duplicate;
            }
            return PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::NonMonotonicTimestamp,
            };
        }

        self.last_observed_at_seconds = Some(observation.observed_at_seconds);
        self.last_observed_logical_paused = Some(observation.logical_paused);
        match self.matching_command(&observation) {
            CommandMatch::One(key) => {
                let command = self
                    .commands
                    .get_mut(&key)
                    .expect("selected command must remain registered");
                command.observation_matched = true;
                let registration = command.registration;
                self.accept_transition(observation.logical_paused);
                return PlayerTransitionClassification::OwnedCommand {
                    command_id: registration.command_id,
                    cause: registration.cause,
                    completion: registration.completion,
                };
            }
            CommandMatch::Ambiguous => {
                self.accept_transition(observation.logical_paused);
                return PlayerTransitionClassification::UnknownOrigin {
                    action: NativePlayerAction::from_paused(observation.logical_paused),
                    reason: PlayerTransitionUnknownReason::AmbiguousCommandMatch,
                };
            }
            CommandMatch::None => {}
        }

        if let Some(reason) = observation.context.technical_reason()
            && (reason != PlayerTransitionTechnicalReason::PlaybackBarrier
                || observation.logical_paused)
        {
            if self.accepted_logical_paused == Some(observation.logical_paused)
                && self.pending_edge.is_none()
            {
                return PlayerTransitionClassification::Duplicate;
            }
            self.accept_transition(observation.logical_paused);
            return PlayerTransitionClassification::Technical {
                action: NativePlayerAction::from_paused(observation.logical_paused),
                reason,
            };
        }

        if self.has_active_unmatched_command(observation.observed_at_seconds) {
            self.accept_transition(observation.logical_paused);
            return PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::from_paused(observation.logical_paused),
                reason: PlayerTransitionUnknownReason::ActiveCommandDidNotMatch,
            };
        }

        let Some(accepted_logical_paused) = self.accepted_logical_paused else {
            self.accept_transition(observation.logical_paused);
            return PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::from_paused(observation.logical_paused),
                reason: PlayerTransitionUnknownReason::InitialObservation,
            };
        };

        if observation.logical_paused == accepted_logical_paused {
            self.pending_edge = None;
            return PlayerTransitionClassification::Duplicate;
        }

        let pending = match self.pending_edge {
            Some(pending) if pending.logical_paused == observation.logical_paused => pending,
            _ => PendingStableEdge {
                logical_paused: observation.logical_paused,
                first_observed_at_seconds: observation.observed_at_seconds,
            },
        };
        self.pending_edge = Some(pending);
        let stable_for = observation.context.stable_for(observation.logical_paused);
        let stable_elapsed = observation.observed_at_seconds - pending.first_observed_at_seconds;
        if stable_for && stable_elapsed >= self.config.stable_confirmation_seconds {
            self.accept_transition(observation.logical_paused);
            return PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::from_paused(observation.logical_paused),
            };
        }
        if stable_elapsed >= self.config.unknown_origin_timeout_seconds {
            self.accept_transition(observation.logical_paused);
            return PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::from_paused(observation.logical_paused),
                reason: PlayerTransitionUnknownReason::UnstableTransitionTimedOut,
            };
        }
        PlayerTransitionClassification::AwaitingStability {
            action: NativePlayerAction::from_paused(observation.logical_paused),
            first_observed_at_seconds: pending.first_observed_at_seconds,
        }
    }

    /// Confirms that an explicit native-player Play owns the classifier's
    /// currently pending same-scope unpause edge. Initial, pause, technical,
    /// command-owned, unknown, consumed, and scope-reset observations never
    /// leave an edge eligible for this promotion.
    pub fn confirm_pending_native_play(&mut self) -> Option<PlayerTransitionClassification> {
        let logical_paused = false;
        let pending = self.pending_edge?;
        if pending.logical_paused != logical_paused
            || self.last_observed_logical_paused != Some(logical_paused)
        {
            return None;
        }
        self.accept_transition(logical_paused);
        Some(PlayerTransitionClassification::NativePlayerGesture {
            action: NativePlayerAction::Play,
        })
    }

    /// Returns the first observation timestamp for a same-scope native Play
    /// candidate. Callers use this to bind explicit promotion to the room
    /// authority which existed when the edge first appeared.
    pub(crate) fn pending_native_play_first_observed_at_seconds(&self) -> Option<f64> {
        self.pending_edge
            .filter(|pending| !pending.logical_paused)
            .map(|pending| pending.first_observed_at_seconds)
    }

    /// Consumes a native Play candidate invalidated by newer room authority.
    /// Accepting the current physical state prevents another telemetry sample
    /// for the same uninterrupted Playing edge from being treated as a fresh
    /// gesture. A later authoritative pause observation re-establishes the
    /// paused baseline, after which a genuinely fresh Play remains eligible.
    pub(crate) fn invalidate_pending_native_play(&mut self) -> bool {
        let Some(pending) = self.pending_edge else {
            return false;
        };
        if pending.logical_paused {
            return false;
        }
        self.accept_transition(false);
        true
    }

    /// Expires a candidate even when the adapter produces no confirming
    /// observation. This records an unknown origin and consumes the edge, so
    /// later duplicate telemetry cannot resurrect it as a user gesture.
    pub fn tick(&mut self, now_seconds: f64) -> Option<PlayerTransitionClassification> {
        if !now_seconds.is_finite() {
            return None;
        }
        let pending = self.pending_edge?;
        if now_seconds - pending.first_observed_at_seconds
            < self.config.unknown_origin_timeout_seconds
        {
            return None;
        }
        self.accept_transition(pending.logical_paused);
        Some(PlayerTransitionClassification::UnknownOrigin {
            action: NativePlayerAction::from_paused(pending.logical_paused),
            reason: PlayerTransitionUnknownReason::UnstableTransitionTimedOut,
        })
    }

    pub fn prune_commands(&mut self, now_seconds: f64) {
        if !now_seconds.is_finite() {
            return;
        }
        self.commands.retain(|_, command| {
            command
                .ownership_deadline_seconds
                .is_none_or(|deadline| now_seconds <= deadline)
        });
    }

    fn matching_command(&self, observation: &PlayerLogicalPauseObservation) -> CommandMatch {
        let mut selected: Option<(PlayerCommandKey, f64)> = None;
        let mut ambiguous = false;
        for (key, command) in &self.commands {
            let registration = command.registration;
            if registration.media_generation != observation.media_generation
                || registration.adapter_epoch != observation.adapter_epoch
                || registration.desired_paused != observation.logical_paused
                || !command.matchable_at(observation.observed_at_seconds)
            {
                continue;
            }
            match selected {
                Some((_, issued_at)) if registration.issued_at_seconds < issued_at => {}
                Some((_, issued_at)) if registration.issued_at_seconds == issued_at => {
                    ambiguous = true;
                }
                _ => {
                    selected = Some((*key, registration.issued_at_seconds));
                    ambiguous = false;
                }
            }
        }
        if ambiguous {
            CommandMatch::Ambiguous
        } else {
            selected.map_or(CommandMatch::None, |(key, _)| CommandMatch::One(key))
        }
    }

    fn has_active_unmatched_command(&self, observed_at_seconds: f64) -> bool {
        self.commands
            .values()
            .copied()
            .any(|command| command.active_at(observed_at_seconds))
    }

    fn accept_transition(&mut self, logical_paused: bool) {
        self.accepted_logical_paused = Some(logical_paused);
        self.pending_edge = None;
    }
}

fn normalized_nonnegative_seconds(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

fn valid_completion(completion: PlayerCommandCompletion, issued_at_seconds: f64) -> bool {
    completion
        .terminal_at_seconds()
        .is_none_or(|at| at.is_finite() && at >= issued_at_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATION: u64 = 7;
    const ADAPTER_EPOCH: u64 = 3;

    fn classifier() -> PlayerTransitionClassifier {
        let mut classifier = PlayerTransitionClassifier::new(PlayerTransitionClassifierConfig {
            stable_confirmation_seconds: 0.1,
            command_observation_grace_seconds: 1.0,
            pending_command_timeout_seconds: 0.5,
            unknown_origin_timeout_seconds: 0.5,
        });
        assert!(classifier.begin_scope(GENERATION, ADAPTER_EPOCH));
        classifier
    }

    fn context(paused: bool) -> PlayerTransitionContext {
        PlayerTransitionContext::new(Some(if paused {
            PlayerTransportPhase::ReadyPaused
        } else {
            PlayerTransportPhase::Playing
        }))
    }

    fn observation(
        observed_at_seconds: f64,
        paused: bool,
        context: PlayerTransitionContext,
    ) -> PlayerLogicalPauseObservation {
        PlayerLogicalPauseObservation::new(
            GENERATION,
            ADAPTER_EPOCH,
            observed_at_seconds,
            paused,
            context,
        )
    }

    fn seed(classifier: &mut PlayerTransitionClassifier, paused: bool) {
        assert_eq!(
            classifier.classify(observation(1.0, paused, context(paused))),
            PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::from_paused(paused),
                reason: PlayerTransitionUnknownReason::InitialObservation,
            }
        );
    }

    #[test]
    fn every_system_command_cause_owns_its_matching_transition() {
        let system_causes = [
            PlayerCommandCause::RemoteRoomSynchronization,
            PlayerCommandCause::AutomaticReadinessStart,
            PlayerCommandCause::ReadinessGateHold,
            PlayerCommandCause::RoomBufferingPolicy,
            PlayerCommandCause::SeekPreparation,
            PlayerCommandCause::DesyncCorrection,
            PlayerCommandCause::Recovery,
            PlayerCommandCause::MediaLoading,
            PlayerCommandCause::PlaylistTransition,
            PlayerCommandCause::TransportRefresh,
        ];
        for (index, cause) in system_causes.into_iter().enumerate() {
            let mut classifier = classifier();
            seed(&mut classifier, false);
            let command_id = PlayerCommandId::new(index as u64 + 1);
            assert!(classifier.register_command(PlayerCommandRegistration::new(
                command_id,
                GENERATION,
                ADAPTER_EPOCH,
                cause,
                true,
                1.1,
            )));
            assert_eq!(
                classifier.classify(observation(1.2, true, context(true))),
                PlayerTransitionClassification::OwnedCommand {
                    command_id,
                    cause,
                    completion: PlayerCommandCompletion::Pending,
                }
            );
        }
    }

    #[test]
    fn local_user_command_is_owned_and_never_reclassified_as_native() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        let command_id = PlayerCommandId::new(9);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::LocalUserPlaybackControl,
            false,
            1.1,
        )));
        assert_eq!(
            classifier.classify(observation(1.2, false, context(false))),
            PlayerTransitionClassification::OwnedCommand {
                command_id,
                cause: PlayerCommandCause::LocalUserPlaybackControl,
                completion: PlayerCommandCompletion::Pending,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.3, false, context(false))),
            PlayerTransitionClassification::Duplicate
        );
    }

    #[test]
    fn barrier_play_confirmation_promotes_only_the_pending_unpause_edge_once() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        assert_eq!(
            classifier.classify(observation(
                1.1,
                false,
                context(false).with_playback_barrier(true),
            )),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 1.1,
            }
        );
        assert_eq!(
            classifier.confirm_pending_native_play(),
            Some(PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Play,
            })
        );
        assert_eq!(
            classifier.confirm_pending_native_play(),
            None,
            "a consumed edge can be confirmed at most once"
        );
    }

    #[test]
    fn initial_observation_cannot_be_explicitly_promoted() {
        let mut classifier = classifier();
        assert_eq!(
            classifier.classify(observation(1.0, false, context(false))),
            PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::Play,
                reason: PlayerTransitionUnknownReason::InitialObservation,
            }
        );
        assert_eq!(classifier.confirm_pending_native_play(), None);
    }

    #[test]
    fn pending_pause_cannot_be_promoted_as_explicit_native_play() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert!(matches!(
            classifier.classify(observation(1.1, true, context(true))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Pause,
                ..
            }
        ));
        assert_eq!(classifier.confirm_pending_native_play(), None);
    }

    #[test]
    fn technical_play_edges_cannot_be_explicitly_promoted() {
        let cases = [
            PlayerTransitionContext::new(Some(PlayerTransportPhase::Loading)),
            context(false).with_seeking(true),
            context(false).with_recovery(true),
        ];
        for technical_context in cases {
            let mut classifier = classifier();
            seed(&mut classifier, true);
            assert!(matches!(
                classifier.classify(observation(1.1, false, technical_context)),
                PlayerTransitionClassification::Technical {
                    action: NativePlayerAction::Play,
                    ..
                }
            ));
            assert_eq!(classifier.confirm_pending_native_play(), None);
        }
    }

    #[test]
    fn owned_unknown_and_scope_reset_edges_cannot_be_explicitly_promoted() {
        let mut owned = classifier();
        seed(&mut owned, true);
        let owned_command = PlayerCommandId::new(16);
        assert!(owned.register_command(PlayerCommandRegistration::new(
            owned_command,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::AutomaticReadinessStart,
            false,
            1.05,
        )));
        assert!(matches!(
            owned.classify(observation(1.1, false, context(false))),
            PlayerTransitionClassification::OwnedCommand { .. }
        ));
        assert_eq!(owned.confirm_pending_native_play(), None);

        let mut unknown = classifier();
        seed(&mut unknown, true);
        assert!(unknown.register_command(PlayerCommandRegistration::new(
            PlayerCommandId::new(17),
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::ReadinessGateHold,
            true,
            1.05,
        )));
        assert!(matches!(
            unknown.classify(observation(1.1, false, context(false))),
            PlayerTransitionClassification::UnknownOrigin { .. }
        ));
        assert_eq!(unknown.confirm_pending_native_play(), None);

        let mut reset = classifier();
        seed(&mut reset, true);
        assert!(matches!(
            reset.classify(observation(1.1, false, context(false))),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
        assert!(reset.begin_scope(GENERATION + 1, ADAPTER_EPOCH + 1));
        assert_eq!(reset.confirm_pending_native_play(), None);

        let mut unscoped = PlayerTransitionClassifier::default();
        assert_eq!(unscoped.confirm_pending_native_play(), None);
    }

    #[test]
    fn timed_out_command_owns_a_late_observation_during_grace() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        let command_id = PlayerCommandId::new(2);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::RoomBufferingPolicy,
            true,
            1.1,
        )));
        assert!(classifier.update_command_completion(
            ADAPTER_EPOCH,
            command_id,
            PlayerCommandCompletion::TimedOut { at_seconds: 1.5 },
        ));
        assert_eq!(
            classifier.classify(observation(2.4, true, context(true))),
            PlayerTransitionClassification::OwnedCommand {
                command_id,
                cause: PlayerCommandCause::RoomBufferingPolicy,
                completion: PlayerCommandCompletion::TimedOut { at_seconds: 1.5 },
            }
        );
    }

    #[test]
    fn late_terminal_completion_extends_the_command_tombstone() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        let command_id = PlayerCommandId::new(3);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::AutomaticReadinessStart,
            false,
            1.1,
        )));
        assert!(classifier.update_command_completion(
            ADAPTER_EPOCH,
            command_id,
            PlayerCommandCompletion::TimedOut { at_seconds: 1.5 },
        ));
        assert!(classifier.update_command_completion(
            ADAPTER_EPOCH,
            command_id,
            PlayerCommandCompletion::Completed { at_seconds: 2.4 },
        ));
        assert_eq!(
            classifier.classify(observation(3.3, false, context(false))),
            PlayerTransitionClassification::OwnedCommand {
                command_id,
                cause: PlayerCommandCause::AutomaticReadinessStart,
                completion: PlayerCommandCompletion::Completed { at_seconds: 2.4 },
            }
        );
    }

    #[test]
    fn terminal_completion_after_existing_tombstone_deadline_is_rejected() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        let command_id = PlayerCommandId::new(15);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::AutomaticReadinessStart,
            false,
            1.1,
        )));
        assert!(classifier.update_command_completion(
            ADAPTER_EPOCH,
            command_id,
            PlayerCommandCompletion::TimedOut { at_seconds: 1.5 },
        ));
        assert!(
            !classifier.update_command_completion(
                ADAPTER_EPOCH,
                command_id,
                PlayerCommandCompletion::Completed { at_seconds: 2.51 },
            ),
            "a later terminal callback must not revive ownership after the 2.50 tombstone deadline"
        );
        assert_eq!(
            classifier.classify(observation(2.52, false, context(false))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 2.52,
            }
        );
    }

    #[test]
    fn technical_contexts_consume_their_edge_but_do_not_own_a_later_stable_edge() {
        let cases = [
            (
                PlayerTransitionContext::new(Some(PlayerTransportPhase::Loading)),
                PlayerTransitionTechnicalReason::Loading,
            ),
            (
                PlayerTransitionContext::new(Some(PlayerTransportPhase::Prebuffering)),
                PlayerTransitionTechnicalReason::Prebuffering,
            ),
            (
                context(true).with_paused_for_cache(true),
                PlayerTransitionTechnicalReason::CachePause,
            ),
            (
                context(true).with_seeking(true),
                PlayerTransitionTechnicalReason::Seeking,
            ),
            (
                context(true).with_media_transition(true),
                PlayerTransitionTechnicalReason::MediaTransition,
            ),
            (
                context(true).with_recovery(true),
                PlayerTransitionTechnicalReason::Recovery,
            ),
            (
                context(true).with_seek_preparation(true),
                PlayerTransitionTechnicalReason::SeekPreparation,
            ),
            (
                context(true).with_room_buffering_policy(true),
                PlayerTransitionTechnicalReason::RoomBufferingPolicy,
            ),
            (
                context(true).with_playback_barrier(true),
                PlayerTransitionTechnicalReason::PlaybackBarrier,
            ),
            (
                context(true).with_synchronization(true),
                PlayerTransitionTechnicalReason::Synchronization,
            ),
            (
                PlayerTransitionContext::new(Some(PlayerTransportPhase::Ended)),
                PlayerTransitionTechnicalReason::Ended,
            ),
            (
                PlayerTransitionContext::new(Some(PlayerTransportPhase::Failed)),
                PlayerTransitionTechnicalReason::Failed,
            ),
        ];
        for (technical_context, reason) in cases {
            let mut classifier = classifier();
            seed(&mut classifier, false);
            assert_eq!(
                classifier.classify(observation(1.2, true, technical_context)),
                PlayerTransitionClassification::Technical {
                    action: NativePlayerAction::Pause,
                    reason,
                }
            );
            assert_eq!(
                classifier.classify(observation(1.4, true, context(true))),
                PlayerTransitionClassification::Duplicate,
                "{reason:?} must consume the edge rather than defer a gesture"
            );
            assert_eq!(
                classifier.classify(observation(1.5, false, context(false))),
                PlayerTransitionClassification::AwaitingStability {
                    action: NativePlayerAction::Play,
                    first_observed_at_seconds: 1.5,
                },
                "a cleared {reason:?} context must not swallow a later user Play"
            );
            assert_eq!(
                classifier.classify(observation(1.61, false, context(false))),
                PlayerTransitionClassification::NativePlayerGesture {
                    action: NativePlayerAction::Play,
                },
                "a stable Play after {reason:?} must remain a native gesture"
            );
        }
    }

    #[test]
    fn user_play_during_the_old_technical_recovery_window_is_not_swallowed() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert_eq!(
            classifier.classify(observation(
                1.1,
                true,
                PlayerTransitionContext::new(Some(PlayerTransportPhase::Rebuffering)),
            )),
            PlayerTransitionClassification::Technical {
                action: NativePlayerAction::Pause,
                reason: PlayerTransitionTechnicalReason::Rebuffering,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.2, false, context(false))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 1.2,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.31, false, context(false))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Play,
            }
        );
    }

    #[test]
    fn user_pause_immediately_after_an_automatic_resume_is_not_swallowed() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        let command_id = PlayerCommandId::new(11);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::AutomaticReadinessStart,
            false,
            1.1,
        )));
        assert_eq!(
            classifier.classify(observation(1.2, false, context(false))),
            PlayerTransitionClassification::OwnedCommand {
                command_id,
                cause: PlayerCommandCause::AutomaticReadinessStart,
                completion: PlayerCommandCompletion::Pending,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.21, true, context(true))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Pause,
                first_observed_at_seconds: 1.21,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.32, true, context(true))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Pause,
            }
        );
    }

    #[test]
    fn command_without_adapter_completion_loses_ownership_at_issue_time_deadline() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            PlayerCommandId::new(12),
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::RemoteRoomSynchronization,
            false,
            1.1,
        )));

        assert_eq!(
            classifier.classify(observation(1.61, false, context(false))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 1.61,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.72, false, context(false))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Play,
            }
        );
    }

    #[test]
    fn completion_after_pending_deadline_cannot_resurrect_system_ownership() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        let command_id = PlayerCommandId::new(14);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::RemoteRoomSynchronization,
            false,
            1.1,
        )));
        assert!(
            !classifier.update_command_completion(
                ADAPTER_EPOCH,
                command_id,
                PlayerCommandCompletion::Completed { at_seconds: 1.61 },
            ),
            "a late callback must not revive ownership after the 1.60 deadline"
        );
        assert_eq!(
            classifier.classify(observation(1.62, false, context(false))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 1.62,
            }
        );
    }

    #[test]
    fn delayed_system_command_observation_after_pending_timeout_is_not_system_owned() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        let command_id = PlayerCommandId::new(13);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            command_id,
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::ReadinessGateHold,
            true,
            1.1,
        )));

        assert_eq!(
            classifier.classify(observation(1.61, true, context(true))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Pause,
                first_observed_at_seconds: 1.61,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.72, true, context(true))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Pause,
            }
        );
    }

    #[test]
    fn stable_unowned_pause_and_play_edges_become_native_gestures() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert_eq!(
            classifier.classify(observation(1.1, true, context(true))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Pause,
                first_observed_at_seconds: 1.1,
            }
        );
        assert_eq!(
            classifier.classify(observation(1.21, true, context(true))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Pause,
            }
        );
        assert_eq!(
            classifier.classify(observation(2.0, false, context(false))),
            PlayerTransitionClassification::AwaitingStability {
                action: NativePlayerAction::Play,
                first_observed_at_seconds: 2.0,
            }
        );
        assert_eq!(
            classifier.classify(observation(2.11, false, context(false))),
            PlayerTransitionClassification::NativePlayerGesture {
                action: NativePlayerAction::Play,
            }
        );
    }

    #[test]
    fn an_unstable_unowned_edge_times_out_as_unknown_and_is_consumed() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        let missing_phase = PlayerTransitionContext::new(None);
        assert!(matches!(
            classifier.classify(observation(1.1, true, missing_phase)),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
        assert_eq!(
            classifier.tick(1.61),
            Some(PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::Pause,
                reason: PlayerTransitionUnknownReason::UnstableTransitionTimedOut,
            })
        );
        assert_eq!(
            classifier.classify(observation(1.7, true, context(true))),
            PlayerTransitionClassification::Duplicate
        );
    }

    #[test]
    fn duplicate_telemetry_produces_at_most_one_native_gesture() {
        let mut classifier = classifier();
        seed(&mut classifier, true);
        assert!(matches!(
            classifier.classify(observation(1.1, false, context(false))),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
        assert!(matches!(
            classifier.classify(observation(1.21, false, context(false))),
            PlayerTransitionClassification::NativePlayerGesture { .. }
        ));
        assert_eq!(
            classifier.classify(observation(1.3, false, context(false))),
            PlayerTransitionClassification::Duplicate
        );
        assert_eq!(
            classifier.classify(observation(1.3, false, context(false))),
            PlayerTransitionClassification::Duplicate
        );
    }

    #[test]
    fn stale_media_generation_and_adapter_epoch_are_ignored() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        let stale_generation = PlayerLogicalPauseObservation::new(
            GENERATION - 1,
            ADAPTER_EPOCH,
            1.1,
            true,
            context(true),
        );
        let stale_epoch = PlayerLogicalPauseObservation::new(
            GENERATION,
            ADAPTER_EPOCH - 1,
            1.2,
            true,
            context(true),
        );
        assert_eq!(
            classifier.classify(stale_generation),
            PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::ScopeMismatch,
            }
        );
        assert_eq!(
            classifier.classify(stale_epoch),
            PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::ScopeMismatch,
            }
        );
        assert!(matches!(
            classifier.classify(observation(1.3, true, context(true))),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
    }

    #[test]
    fn unmatched_active_command_makes_an_edge_unknown() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            PlayerCommandId::new(8),
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::RemoteRoomSynchronization,
            false,
            1.1,
        )));
        assert_eq!(
            classifier.classify(observation(1.2, true, context(true))),
            PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::Pause,
                reason: PlayerTransitionUnknownReason::ActiveCommandDidNotMatch,
            }
        );
    }

    #[test]
    fn scope_replacement_clears_baseline_commands_and_pending_edges() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert!(matches!(
            classifier.classify(observation(1.1, true, context(true))),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
        assert!(classifier.register_command(PlayerCommandRegistration::new(
            PlayerCommandId::new(10),
            GENERATION,
            ADAPTER_EPOCH,
            PlayerCommandCause::PlaylistTransition,
            true,
            1.1,
        )));
        assert!(classifier.begin_scope(GENERATION + 1, ADAPTER_EPOCH + 1));
        assert!(
            classifier
                .command_registration(ADAPTER_EPOCH, PlayerCommandId::new(10))
                .is_none()
        );
        assert_eq!(
            classifier.classify(PlayerLogicalPauseObservation::new(
                GENERATION + 1,
                ADAPTER_EPOCH + 1,
                2.0,
                true,
                context(true),
            )),
            PlayerTransitionClassification::UnknownOrigin {
                action: NativePlayerAction::Pause,
                reason: PlayerTransitionUnknownReason::InitialObservation,
            }
        );
    }

    #[test]
    fn non_monotonic_observation_cannot_create_or_confirm_an_edge() {
        let mut classifier = classifier();
        seed(&mut classifier, false);
        assert_eq!(
            classifier.classify(observation(0.9, true, context(true))),
            PlayerTransitionClassification::Ignored {
                reason: PlayerTransitionIgnoredReason::NonMonotonicTimestamp,
            }
        );
        assert!(matches!(
            classifier.classify(observation(1.1, true, context(true))),
            PlayerTransitionClassification::AwaitingStability { .. }
        ));
    }
}
