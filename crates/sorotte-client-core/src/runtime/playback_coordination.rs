use super::*;

use std::collections::BTreeMap;

use sorotte_player_api::{
    PlayerCommand, PlayerCommandId, PlayerCommandProgressState, PlayerCommandResult,
    PlayerMediaGeneration, PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    PlaybackBarrierParticipantPhase, PlaybackBarrierPolicy, PlaybackBarrierSetExtension,
    PrepareMediaPayload, RoomBufferingPolicy, RoomBufferingPolicyPayload,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackBarrierTimeoutAction {
    #[default]
    Continue,
    RemainPaused,
    AskController,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackBarrierStartConfig {
    pub policy: Option<PlaybackBarrierPolicy>,
    pub quorum_percent: u32,
    pub timeout_seconds: f64,
    pub timeout_action: PlaybackBarrierTimeoutAction,
}

impl Default for PlaybackBarrierStartConfig {
    fn default() -> Self {
        Self {
            policy: None,
            quorum_percent: 75,
            timeout_seconds: 15.0,
            timeout_action: PlaybackBarrierTimeoutAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackBarrierRoomBufferingConfig {
    pub policy: RoomBufferingPolicy,
    pub quorum_percent: u32,
    pub debounce_seconds: f64,
    pub resume_hysteresis_seconds: f64,
    pub maximum_pause_seconds: f64,
}

impl Default for PlaybackBarrierRoomBufferingConfig {
    fn default() -> Self {
        Self {
            policy: RoomBufferingPolicy::Independent,
            quorum_percent: 75,
            debounce_seconds: 0.75,
            resume_hysteresis_seconds: 1.5,
            maximum_pause_seconds: 30.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackCoordinationSnapshot {
    pub media_generation: Option<u64>,
    pub diagnostic: PlaybackDiagnostic,
    pub recovery_episode: Option<RecoveryEpisodeSnapshot>,
    pub metrics: PlaybackCoordinatorMetrics,
    pub transport_telemetry_observed: bool,
    pub ordinary_correction_blocked: bool,
    pub last_applied_revision: Option<u64>,
    pub last_started_revision: Option<u64>,
    pub last_degraded_reason: Option<DegradedPlaybackReason>,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomDesiredFingerprint {
    paused: bool,
    position_seconds: f64,
    do_seek: bool,
    barrier_media_generation: Option<u64>,
    barrier_state_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarrierReadySignature {
    room_media_generation: u64,
    local_media_generation: u64,
    loaded: bool,
    seekable: Option<bool>,
    buffer_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RoomBufferingObservation {
    media_generation: u64,
    state_revision: Option<u64>,
    buffering: bool,
    buffered_seconds: Option<f64>,
    observed_at: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct RuntimePlaybackCoordination {
    coordinator: PlaybackCoordinator,
    adapter_generation_bindings: BTreeMap<u64, u64>,
    pending_media_generation: Option<u64>,
    highest_bound_adapter_generation: Option<u64>,
    adapter_epoch: u64,
    player_command_bindings: BTreeMap<PlayerCommandId, CoordinatorCommandId>,
    latest_observation: Option<PlayerTransportObservation>,
    adapter_clock_offset_seconds: Option<f64>,
    last_external_now_seconds: Option<f64>,
    last_coordinator_now_seconds: Option<f64>,
    desired_generation: Option<u64>,
    desired_revision: u64,
    desired_fingerprint: Option<RoomDesiredFingerprint>,
    pending_forced_seek_revision: Option<u64>,
    transport_telemetry_observed: bool,
    last_applied_revision: Option<u64>,
    last_started_revision: Option<u64>,
    last_degraded_reason: Option<DegradedPlaybackReason>,
    last_reported_barrier_ready: Option<BarrierReadySignature>,
    last_reported_barrier_started: Option<(u64, u64)>,
    barrier_start_config: PlaybackBarrierStartConfig,
    room_buffering_config: PlaybackBarrierRoomBufferingConfig,
    next_room_barrier_generation: u64,
    initiated_barrier: Option<(u64, u64)>,
    handled_barrier_timeout: Option<(u64, Option<u64>)>,
    pending_barrier_timeout_action: Option<PlaybackBarrierTimeoutAction>,
    last_reported_room_buffering: Option<(u64, Option<u64>, bool)>,
}

impl RuntimePlaybackCoordination {
    pub(crate) fn set_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.coordinator.set_config(config);
    }

    pub(crate) fn set_barrier_start_config(&mut self, config: PlaybackBarrierStartConfig) {
        self.barrier_start_config = PlaybackBarrierStartConfig {
            policy: config.policy,
            quorum_percent: config.quorum_percent.clamp(1, 100),
            timeout_seconds: if config.timeout_seconds.is_finite() && config.timeout_seconds > 0.0 {
                config.timeout_seconds
            } else {
                PlaybackBarrierStartConfig::default().timeout_seconds
            },
            timeout_action: config.timeout_action,
        };
    }

    pub(crate) fn set_room_buffering_config(&mut self, config: PlaybackBarrierRoomBufferingConfig) {
        let defaults = PlaybackBarrierRoomBufferingConfig::default();
        self.room_buffering_config = PlaybackBarrierRoomBufferingConfig {
            policy: config.policy,
            quorum_percent: config.quorum_percent.clamp(1, 100),
            debounce_seconds: normalized_positive_seconds(
                config.debounce_seconds,
                defaults.debounce_seconds,
            ),
            resume_hysteresis_seconds: normalized_positive_seconds(
                config.resume_hysteresis_seconds,
                defaults.resume_hysteresis_seconds,
            ),
            maximum_pause_seconds: normalized_positive_seconds(
                config.maximum_pause_seconds,
                defaults.maximum_pause_seconds,
            ),
        };
    }

    pub(crate) fn prepare_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let placeholder_adapter_generation = self
            .coordinator
            .current_logical_media_id()
            .filter(|logical_id| logical_id.as_str().starts_with("adapter-media-generation-"))
            .and(self.highest_bound_adapter_generation);
        let plan = self
            .coordinator
            .prepare_media(logical_id, kind, now_seconds);
        self.pending_media_generation = Some(plan.media_generation);
        self.latest_observation = None;
        self.transport_telemetry_observed = false;
        self.player_command_bindings.clear();
        self.last_reported_barrier_ready = None;
        self.last_reported_barrier_started = None;
        if plan.logical_media_changed {
            self.desired_generation = None;
            self.desired_fingerprint = None;
            self.pending_forced_seek_revision = None;
            self.last_applied_revision = None;
            self.last_started_revision = None;
            self.last_degraded_reason = None;
            self.initiated_barrier = None;
            self.handled_barrier_timeout = None;
            self.pending_barrier_timeout_action = None;
            self.last_reported_room_buffering = None;
        }
        if let Some(adapter_generation) = placeholder_adapter_generation {
            self.adapter_generation_bindings
                .insert(adapter_generation, plan.media_generation);
            self.pending_media_generation = None;
        }
        plan
    }

    pub(crate) fn reset_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        self.adapter_epoch = self.adapter_epoch.saturating_add(1);
        self.adapter_generation_bindings.clear();
        self.highest_bound_adapter_generation = None;
        self.pending_media_generation = self.coordinator.current_media_generation();
        self.player_command_bindings.clear();
        self.latest_observation = None;
        self.adapter_clock_offset_seconds = None;
        self.last_external_now_seconds = None;
        self.last_coordinator_now_seconds = None;
        self.transport_telemetry_observed = false;
        self.last_reported_barrier_ready = None;
        self.last_reported_barrier_started = None;
        self.last_reported_room_buffering = None;
        self.pending_barrier_timeout_action = None;
        self.coordinator.reset_transport_adapter_epoch(now_seconds);
        self.adapter_epoch
    }

    pub(crate) fn snapshot(&self) -> PlaybackCoordinationSnapshot {
        PlaybackCoordinationSnapshot {
            media_generation: self.coordinator.current_media_generation(),
            diagnostic: self.coordinator.diagnostic(),
            recovery_episode: self.coordinator.recovery_episode(),
            metrics: self.coordinator.metrics().clone(),
            transport_telemetry_observed: self.transport_telemetry_observed,
            ordinary_correction_blocked: self.coordinator.ordinary_correction_blocked(),
            last_applied_revision: self.last_applied_revision,
            last_started_revision: self.last_started_revision,
            last_degraded_reason: self.last_degraded_reason,
        }
    }

    pub(crate) fn coordinator_now(&self, external_now_seconds: f64) -> f64 {
        match (
            self.last_external_now_seconds,
            self.last_coordinator_now_seconds,
        ) {
            (Some(last_external), Some(last_coordinator)) => {
                last_coordinator + (external_now_seconds - last_external).max(0.0)
            }
            _ => external_now_seconds,
        }
    }

    fn current_logical_media_matches(&self, room_logical_media_id: &str) -> bool {
        let Some(local) = self.coordinator.current_logical_media_id() else {
            return false;
        };
        logical_media_ids_match(local.as_str(), room_logical_media_id)
    }

    fn current_logical_media(&self) -> Option<(u64, String)> {
        Some((
            self.coordinator.current_media_generation()?,
            self.coordinator
                .current_logical_media_id()?
                .as_str()
                .to_owned(),
        ))
    }

    fn next_room_barrier_generation(&mut self, now_seconds: f64) -> u64 {
        let wall_clock_candidate = if now_seconds.is_finite() && now_seconds > 0.0 {
            (now_seconds * 1_000.0).floor().min(u64::MAX as f64) as u64
        } else {
            1
        };
        self.next_room_barrier_generation = self
            .next_room_barrier_generation
            .saturating_add(1)
            .max(wall_clock_candidate)
            .max(1);
        self.next_room_barrier_generation
    }

    pub(crate) fn playback_barrier_set_for_new_media(
        &mut self,
        plan: &MediaLoadPlan,
        session: &ClientSession,
        now_seconds: f64,
    ) -> Option<PlaybackBarrierSetExtension> {
        if !plan.logical_media_changed
            || self
                .initiated_barrier
                .is_some_and(|(local_generation, _)| local_generation == plan.media_generation)
            || !session.playback_barrier_v1_negotiated()
            || session.local_can_control() != Some(true)
        {
            return None;
        }

        let (local_generation, logical_media_id) = self.current_logical_media()?;
        if local_generation != plan.media_generation {
            return None;
        }
        if session.playback_barrier_prepare().is_some_and(|prepare| {
            logical_media_ids_match(&logical_media_id, &prepare.logical_media_id)
        }) {
            // A peer has already established the room generation for this
            // logical source. Loading that source locally is participation,
            // not a competing start request.
            return None;
        }
        let room_generation = self.next_room_barrier_generation(now_seconds);
        let mut extension = PlaybackBarrierSetExtension::new();
        if let Some(policy) = self.barrier_start_config.policy {
            let timeout_ms = (self.barrier_start_config.timeout_seconds * 1_000.0)
                .round()
                .clamp(1.0, u64::MAX as f64) as u64;
            let mut prepare =
                PrepareMediaPayload::new(room_generation, logical_media_id, 0.0, policy)
                    .with_timeout_ms(timeout_ms);
            if policy == PlaybackBarrierPolicy::Quorum {
                prepare = prepare.with_quorum_percent(self.barrier_start_config.quorum_percent);
            }
            extension = extension.with_prepare(prepare);
        }

        let room_config = self.room_buffering_config;
        let mut buffering = RoomBufferingPolicyPayload::new(room_generation, room_config.policy)
            .with_debounce_ms(seconds_to_milliseconds(room_config.debounce_seconds))
            .with_resume_hysteresis_ms(seconds_to_milliseconds(
                room_config.resume_hysteresis_seconds,
            ))
            .with_max_pause_ms(seconds_to_milliseconds(room_config.maximum_pause_seconds));
        if room_config.policy == RoomBufferingPolicy::Quorum {
            buffering = buffering.with_quorum_percent(room_config.quorum_percent);
        }
        self.initiated_barrier = Some((local_generation, room_generation));
        Some(extension.with_buffering_policy(buffering))
    }

    fn bind_adapter_generation(
        &mut self,
        adapter_generation: PlayerMediaGeneration,
        external_now_seconds: f64,
    ) -> Option<u64> {
        let adapter_generation = adapter_generation.get();
        if let Some(generation) = self.adapter_generation_bindings.get(&adapter_generation) {
            return Some(*generation);
        }

        let logical_generation = match self.pending_media_generation {
            Some(generation)
                if self
                    .highest_bound_adapter_generation
                    .is_none_or(|highest| adapter_generation > highest) =>
            {
                self.pending_media_generation = None;
                generation
            }
            Some(_) => return None,
            None if self
                .highest_bound_adapter_generation
                .is_none_or(|highest| adapter_generation > highest) =>
            {
                if let Some(generation) = self.coordinator.current_media_generation() {
                    generation
                } else {
                    let logical_id = LogicalMediaId::new(format!(
                        "adapter-media-generation-{adapter_generation}"
                    ))
                    .expect("generated logical media ID is non-empty");
                    self.prepare_media(
                        logical_id,
                        MediaTransportKind::NetworkVod,
                        external_now_seconds,
                    )
                    .media_generation
                }
            }
            None => return None,
        };
        self.adapter_generation_bindings
            .insert(adapter_generation, logical_generation);
        self.highest_bound_adapter_generation = Some(
            self.highest_bound_adapter_generation
                .map_or(adapter_generation, |highest| {
                    highest.max(adapter_generation)
                }),
        );
        Some(logical_generation)
    }

    fn map_observation_time(
        &mut self,
        update: &PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> f64 {
        let raw_seconds = update
            .observed_at
            .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
        let observed_at_seconds = match raw_seconds {
            Some(raw_seconds) => {
                let offset = *self
                    .adapter_clock_offset_seconds
                    .get_or_insert(external_now_seconds - raw_seconds);
                raw_seconds + offset
            }
            None => self.coordinator_now(external_now_seconds),
        };
        self.last_external_now_seconds = Some(external_now_seconds);
        self.last_coordinator_now_seconds = Some(observed_at_seconds);
        observed_at_seconds
    }

    pub(crate) fn observe_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let Some(adapter_generation) = update.media_generation else {
            return Vec::new();
        };
        let Some(media_generation) =
            self.bind_adapter_generation(adapter_generation, external_now_seconds)
        else {
            return Vec::new();
        };
        let owns_current_media =
            self.coordinator.current_media_generation() == Some(media_generation);
        let observed_at_seconds = self.map_observation_time(&update, external_now_seconds);
        let observation = PlayerTransportObservation {
            media_generation,
            observed_at_seconds,
            phase: update.phase,
            position_seconds: update.position_seconds,
            playback_rate: update.playback_rate,
            logical_pause: update.logical_pause,
            paused_for_cache: update.paused_for_cache,
            seeking: update.seeking,
            seekable: update.seekable,
            seekable_ranges: update.seekable_ranges,
            core_idle: update.core_idle,
            playback_restart_sequence: update.playback_restart_sequence,
            buffered_ahead_seconds: update.buffered_ahead_seconds,
            input_rate_bytes_per_second: update.input_rate_bytes_per_second,
        };
        if owns_current_media {
            self.transport_telemetry_observed = true;
        }
        self.merge_latest_observation(observation.clone());
        let actions = self.coordinator.observe(observation);
        self.record_observation_outcomes(&actions);
        actions
    }

    pub(crate) fn observe_transport_at_epoch(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
        adapter_epoch: u64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if adapter_epoch != self.adapter_epoch {
            return Vec::new();
        }
        self.observe_transport(update, external_now_seconds)
    }

    fn merge_latest_observation(&mut self, newer: PlayerTransportObservation) {
        let Some(current) = self.latest_observation.as_mut().filter(|current| {
            current.media_generation == newer.media_generation
                && current.observed_at_seconds <= newer.observed_at_seconds
        }) else {
            self.latest_observation = Some(newer);
            return;
        };
        current.observed_at_seconds = newer.observed_at_seconds;
        current.phase = newer.phase.or(current.phase);
        current.position_seconds = newer.position_seconds.or(current.position_seconds);
        current.playback_rate = newer.playback_rate.or(current.playback_rate);
        current.logical_pause = newer.logical_pause.or(current.logical_pause);
        current.paused_for_cache = newer.paused_for_cache.or(current.paused_for_cache);
        current.seeking = newer.seeking.or(current.seeking);
        current.seekable = newer.seekable.or(current.seekable);
        current.seekable_ranges = newer
            .seekable_ranges
            .or_else(|| current.seekable_ranges.take());
        current.core_idle = newer.core_idle.or(current.core_idle);
        current.playback_restart_sequence = newer
            .playback_restart_sequence
            .or(current.playback_restart_sequence);
        current.buffered_ahead_seconds = newer
            .buffered_ahead_seconds
            .or(current.buffered_ahead_seconds);
        current.input_rate_bytes_per_second = newer
            .input_rate_bytes_per_second
            .or(current.input_rate_bytes_per_second);
    }

    fn barrier_ready_signature(&self, session: &ClientSession) -> Option<BarrierReadySignature> {
        let prepare = session.playback_barrier_prepare()?;
        if !self.current_logical_media_matches(&prepare.logical_media_id) {
            return None;
        }
        let observation = self.latest_observation.as_ref()?;
        if self.coordinator.current_media_generation() != Some(observation.media_generation) {
            return None;
        }
        let phase = observation.phase?;
        let loaded = !matches!(
            phase,
            sorotte_player_api::PlayerTransportPhase::Empty
                | sorotte_player_api::PlayerTransportPhase::Loading
                | sorotte_player_api::PlayerTransportPhase::Failed
        );
        let target_applied = observation
            .position_seconds
            .is_some_and(|position| (position - prepare.target_position).abs() <= 0.5);
        let prepare_revision_applied = self.desired_fingerprint.as_ref().is_some_and(|desired| {
            desired.barrier_media_generation == Some(prepare.media_generation)
                && desired.barrier_state_revision.is_none()
                && self.last_applied_revision == Some(self.desired_revision)
        });
        let buffer_ready = phase == sorotte_player_api::PlayerTransportPhase::ReadyPaused
            && observation.logical_pause == Some(true)
            && target_applied
            && prepare_revision_applied
            && observation.paused_for_cache != Some(true)
            && observation.seeking != Some(true)
            && observation.core_idle != Some(true);
        Some(BarrierReadySignature {
            room_media_generation: prepare.media_generation,
            local_media_generation: observation.media_generation,
            loaded,
            seekable: observation.seekable,
            buffer_ready,
        })
    }

    fn barrier_started_target(
        &self,
        session: &ClientSession,
        local_media_generation: u64,
    ) -> Option<(u64, u64)> {
        let prepare = session.playback_barrier_prepare()?;
        let commit = session.playback_barrier_commit()?;
        if prepare.media_generation != commit.media_generation
            || self.coordinator.current_media_generation() != Some(local_media_generation)
            || !self.current_logical_media_matches(&prepare.logical_media_id)
        {
            return None;
        }
        Some((commit.media_generation, commit.state_revision))
    }

    fn barrier_timeout_requires_room_pause(&mut self, session: &ClientSession) -> bool {
        if self.barrier_start_config.timeout_action == PlaybackBarrierTimeoutAction::Continue {
            return false;
        }
        let Some((local_generation, room_generation)) = self.initiated_barrier else {
            return false;
        };
        if self.coordinator.current_media_generation() != Some(local_generation) {
            return false;
        }
        let Some(status) = session.playback_barrier_status().filter(|status| {
            status.media_generation == room_generation
                && status.participants.values().any(|participant| {
                    participant.phase == PlaybackBarrierParticipantPhase::TimedOut
                })
        }) else {
            return false;
        };
        let identity = (status.media_generation, status.state_revision);
        if self.handled_barrier_timeout == Some(identity) {
            return false;
        }
        self.handled_barrier_timeout = Some(identity);
        self.pending_barrier_timeout_action = Some(self.barrier_start_config.timeout_action);
        true
    }

    fn room_buffering_observation(
        &self,
        session: &ClientSession,
    ) -> Option<RoomBufferingObservation> {
        let policy = session.playback_barrier_buffering_policy()?;
        if policy.policy == RoomBufferingPolicy::Independent {
            return None;
        }
        if let Some(prepare) = session.playback_barrier_prepare()
            && prepare.media_generation == policy.media_generation
            && !self.current_logical_media_matches(&prepare.logical_media_id)
        {
            return None;
        }
        let observation = self.latest_observation.as_ref()?;
        if self.coordinator.current_media_generation() != Some(observation.media_generation) {
            return None;
        }
        let buffering = observation.paused_for_cache == Some(true)
            || observation.phase == Some(sorotte_player_api::PlayerTransportPhase::Rebuffering);
        Some(RoomBufferingObservation {
            media_generation: policy.media_generation,
            state_revision: policy.state_revision,
            buffering,
            buffered_seconds: observation.buffered_ahead_seconds,
            observed_at: Some(observation.observed_at_seconds),
        })
    }

    fn should_report_room_buffering(
        &self,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) -> bool {
        self.last_reported_room_buffering != Some((media_generation, state_revision, buffering))
    }

    fn mark_room_buffering_reported(
        &mut self,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) {
        self.last_reported_room_buffering = Some((media_generation, state_revision, buffering));
    }

    fn mark_barrier_ready_reported(&mut self, signature: BarrierReadySignature) {
        self.last_reported_barrier_ready = Some(signature);
    }

    fn mark_barrier_started_reported(&mut self, media_generation: u64, state_revision: u64) {
        self.last_reported_barrier_started = Some((media_generation, state_revision));
    }

    fn latest_observed_at_seconds(&self) -> Option<f64> {
        self.latest_observation
            .as_ref()
            .map(|observation| observation.observed_at_seconds)
    }

    pub(crate) fn update_desired_from_session(
        &mut self,
        session: &ClientSession,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let Some(media_generation) = self.coordinator.current_media_generation() else {
            return Vec::new();
        };
        let Some(raw) = session.current_room_playstate().cloned() else {
            return Vec::new();
        };
        let Some(projected) = session.current_room_playstate_at(external_now_seconds) else {
            return Vec::new();
        };
        let (Some(mut paused), Some(mut position_seconds)) = (projected.paused, projected.position)
        else {
            return Vec::new();
        };
        let barrier_state = session
            .playback_barrier_prepare()
            .filter(|prepare| self.current_logical_media_matches(&prepare.logical_media_id))
            .map(|prepare| {
                let commit = session
                    .playback_barrier_commit()
                    .filter(|commit| commit.media_generation == prepare.media_generation);
                (
                    prepare.media_generation,
                    prepare.target_position,
                    commit.map(|commit| (commit.state_revision, commit.anchor_position)),
                )
            });
        let timeout_requires_room_pause = self.barrier_timeout_requires_room_pause(session);
        let (barrier_media_generation, barrier_state_revision) = match barrier_state {
            Some((media_generation, _target_position, Some((state_revision, anchor_position)))) => {
                paused = false;
                if projected.paused != Some(false) {
                    position_seconds = anchor_position;
                }
                (Some(media_generation), Some(state_revision))
            }
            Some((media_generation, target_position, None)) => {
                paused = true;
                position_seconds = target_position;
                (Some(media_generation), None)
            }
            None => (None, None),
        };
        if timeout_requires_room_pause {
            paused = true;
        }
        if !position_seconds.is_finite() {
            return Vec::new();
        }

        let fingerprint = RoomDesiredFingerprint {
            paused,
            position_seconds: raw.position.unwrap_or(position_seconds),
            do_seek: raw.do_seek == Some(true),
            barrier_media_generation,
            barrier_state_revision,
        };
        let first_for_generation = self.desired_generation != Some(media_generation);
        let pause_changed = self
            .desired_fingerprint
            .as_ref()
            .is_some_and(|previous| previous.paused != paused);
        let paused_position_changed = paused
            && self.desired_fingerprint.as_ref().is_some_and(|previous| {
                (previous.position_seconds - fingerprint.position_seconds).abs() > f64::EPSILON
            });
        let explicit_seek_changed = fingerprint.do_seek
            && self
                .desired_fingerprint
                .as_ref()
                .is_none_or(|previous| previous != &fingerprint);
        let barrier_changed = self.desired_fingerprint.as_ref().is_none_or(|previous| {
            previous.barrier_media_generation != fingerprint.barrier_media_generation
                || previous.barrier_state_revision != fingerprint.barrier_state_revision
        });
        let desired_changed = first_for_generation
            || pause_changed
            || paused_position_changed
            || explicit_seek_changed
            || barrier_changed;
        if desired_changed {
            self.desired_revision = self.desired_revision.saturating_add(1).max(1);
            if first_for_generation || explicit_seek_changed || barrier_changed {
                self.pending_forced_seek_revision = Some(self.desired_revision);
            }
        }
        self.desired_generation = Some(media_generation);
        self.desired_fingerprint = Some(fingerprint);

        let coordinator_now = self.coordinator_now(external_now_seconds);
        let mut actions = self
            .coordinator
            .update_desired_room_state(DesiredRoomPlayback {
                media_generation,
                state_revision: self.desired_revision,
                paused,
                anchor_position_seconds: position_seconds,
                anchor_observed_at_seconds: coordinator_now,
                force_seek: self.pending_forced_seek_revision == Some(self.desired_revision),
            });
        if desired_changed && let Some(observation) = self.latest_observation.clone() {
            actions.extend(self.coordinator.observe(observation));
        }
        actions.extend(self.coordinator.tick(coordinator_now));
        if timeout_requires_room_pause {
            actions.push(PlaybackCoordinatorAction::RequestRoomPause {
                recovery_episode_id: 0,
            });
        }
        self.record_observation_outcomes(&actions);
        actions
    }

    fn record_observation_outcomes(&mut self, actions: &[PlaybackCoordinatorAction]) {
        for action in actions {
            match action {
                PlaybackCoordinatorAction::RevisionApplied { state_revision, .. } => {
                    self.last_applied_revision = Some(*state_revision);
                    if self.pending_forced_seek_revision == Some(*state_revision) {
                        self.pending_forced_seek_revision = None;
                    }
                }
                PlaybackCoordinatorAction::Started { state_revision, .. } => {
                    self.last_started_revision = Some(*state_revision);
                }
                PlaybackCoordinatorAction::Degraded { reason, .. } => {
                    self.last_degraded_reason = Some(*reason);
                }
                PlaybackCoordinatorAction::Execute { .. }
                | PlaybackCoordinatorAction::RequestRoomPause { .. }
                | PlaybackCoordinatorAction::CommandTimedOut { .. } => {}
            }
        }
    }

    pub(crate) fn bind_player_command(
        &mut self,
        player_command_id: PlayerCommandId,
        coordinator_command_id: CoordinatorCommandId,
    ) {
        self.player_command_bindings
            .insert(player_command_id, coordinator_command_id);
    }

    pub(crate) fn command_dispatch_succeeded(
        &mut self,
        coordinator_command_id: CoordinatorCommandId,
    ) {
        let _ = self.coordinator.command_accepted(coordinator_command_id);
    }

    pub(crate) fn command_dispatch_failed(
        &mut self,
        coordinator_command_id: CoordinatorCommandId,
        now_seconds: f64,
    ) {
        let _ = self
            .coordinator
            .command_failed(coordinator_command_id, now_seconds);
    }

    pub(crate) fn apply_player_command_progress(
        &mut self,
        progress: sorotte_player_api::PlayerCommandProgress,
        external_now_seconds: f64,
    ) {
        let Some(coordinator_command_id) = self
            .player_command_bindings
            .get(&progress.command_id)
            .copied()
        else {
            return;
        };
        match progress.state {
            PlayerCommandProgressState::Accepted => {
                let _ = self.coordinator.command_accepted(coordinator_command_id);
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                // Completion is observation-backed in the adapter, but the
                // coordinator still owns RevisionApplied/Started based on its
                // full transport observation stream.
                let _ = self.coordinator.command_accepted(coordinator_command_id);
                self.player_command_bindings.remove(&progress.command_id);
            }
            PlayerCommandProgressState::Finished(
                PlayerCommandResult::Superseded | PlayerCommandResult::Failed(_),
            ) => {
                let now_seconds = self.coordinator_now(external_now_seconds);
                let _ = self
                    .coordinator
                    .command_failed(coordinator_command_id, now_seconds);
                self.player_command_bindings.remove(&progress.command_id);
            }
        }
    }
}

fn logical_media_ids_match(local: &str, room: &str) -> bool {
    // Logical media IDs are opaque protocol identities. Paths, URL query
    // strings, and basenames are not safe equivalence relations: two distinct
    // YouTube videos or two private files can otherwise collapse together.
    local == room
}

fn normalized_positive_seconds(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn seconds_to_milliseconds(seconds: f64) -> u64 {
    (seconds * 1_000.0).round().clamp(1.0, u64::MAX as f64) as u64
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn set_playback_coordinator_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.playback_coordination.set_config(config);
    }

    pub fn set_playback_barrier_start_config(&mut self, config: PlaybackBarrierStartConfig) {
        self.playback_coordination.set_barrier_start_config(config);
    }

    pub fn set_playback_barrier_room_buffering_config(
        &mut self,
        config: PlaybackBarrierRoomBufferingConfig,
    ) {
        self.playback_coordination.set_room_buffering_config(config);
    }

    pub fn reset_playback_transport_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        self.playback_coordination.reset_adapter_epoch(now_seconds)
    }

    pub fn playback_transport_adapter_epoch(&self) -> u64 {
        self.playback_coordination.adapter_epoch
    }

    pub fn observe_external_player_transport_at_epoch(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
        adapter_epoch: u64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let actions = self.playback_coordination.observe_transport_at_epoch(
            update,
            now_seconds,
            adapter_epoch,
        );
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    pub fn prepare_playback_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let plan = self
            .playback_coordination
            .prepare_media(logical_id, kind, now_seconds);
        if let Some(extension) = self
            .playback_coordination
            .playback_barrier_set_for_new_media(&plan, &self.session, now_seconds)
        {
            self.control.activate_protocol_connection_generation();
            let _ = self
                .control
                .emit(ClientEffect::send_playback_barrier_set(extension));
        }
        plan
    }

    pub fn playback_coordination_snapshot(&self) -> PlaybackCoordinationSnapshot {
        self.playback_coordination.snapshot()
    }

    pub fn take_playback_barrier_timeout_action(&mut self) -> Option<PlaybackBarrierTimeoutAction> {
        self.playback_coordination
            .pending_barrier_timeout_action
            .take()
    }

    pub fn report_playback_barrier_media_ready(
        &mut self,
        media_generation: u64,
        loaded: bool,
        seekable: Option<bool>,
        buffer_ready: bool,
    ) -> Result<bool, ClientEffectError> {
        let Some(state) = self.session.playback_barrier_media_ready_observation(
            media_generation,
            loaded,
            seekable,
            buffer_ready,
        ) else {
            return Ok(false);
        };
        self.control.activate_protocol_connection_generation();
        self.control.emit(ClientEffect::SendState(state))?;
        Ok(true)
    }

    pub fn report_playback_barrier_started(
        &mut self,
        media_generation: u64,
        state_revision: u64,
        observed_position: f64,
        position_advancing: bool,
        observed_at: Option<f64>,
    ) -> Result<bool, ClientEffectError> {
        let Some(state) = self.session.playback_barrier_started_observation(
            media_generation,
            state_revision,
            observed_position,
            position_advancing,
            observed_at,
        ) else {
            return Ok(false);
        };
        self.control.activate_protocol_connection_generation();
        self.control.emit(ClientEffect::SendState(state))?;
        Ok(true)
    }

    pub fn observe_external_player_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let actions = self
            .playback_coordination
            .observe_transport(update, now_seconds);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    pub fn reconcile_external_player_playback(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    pub fn report_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        result: Result<(), PlayerError>,
        now_seconds: f64,
    ) {
        match result {
            Ok(()) => self
                .playback_coordination
                .command_dispatch_succeeded(command_id),
            Err(_) => self
                .playback_coordination
                .command_dispatch_failed(command_id, now_seconds),
        }
    }

    pub(crate) fn drain_player_transport_coordination(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let mut first_error = None;
        while let Some(progress) = self.player.take_command_progress() {
            self.playback_coordination
                .apply_player_command_progress(progress, now_seconds);
        }
        while let Some(update) = self.player.take_transport_telemetry_update() {
            let actions = self
                .playback_coordination
                .observe_transport(update, now_seconds);
            if let Err(error) = self.report_playback_barrier_observations(&actions)
                && first_error.is_none()
            {
                first_error = Some(crate::control::client_effect_player_error(error));
            }
            if let Err(error) = self.execute_playback_coordinator_actions(actions, now_seconds)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        let actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        if let Err(error) = self.report_playback_barrier_observations(&actions)
            && first_error.is_none()
        {
            first_error = Some(crate::control::client_effect_player_error(error));
        }
        if let Err(error) = self.execute_playback_coordinator_actions(actions, now_seconds)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }

    fn execute_playback_coordinator_actions(
        &mut self,
        actions: Vec<PlaybackCoordinatorAction>,
        external_now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let mut first_error = None;
        for action in actions {
            match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command,
                } => {
                    if let Err(error) = self.execute_playback_coordinator_command(
                        command_id,
                        command,
                        external_now_seconds,
                    ) && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                PlaybackCoordinatorAction::RequestRoomPause { .. } => {
                    if let Err(error) = self.run_set_paused(true)
                        && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                PlaybackCoordinatorAction::RevisionApplied { .. }
                | PlaybackCoordinatorAction::Started { .. }
                | PlaybackCoordinatorAction::Degraded { .. }
                | PlaybackCoordinatorAction::CommandTimedOut { .. } => {}
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The single command-execution seam for coordinator policy. This keeps
    /// tracked adapter IDs isolated from logical coordinator IDs.
    fn execute_playback_coordinator_command(
        &mut self,
        command_id: CoordinatorCommandId,
        command: CoordinatorPlayerCommand,
        external_now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let player_command = match command {
            CoordinatorPlayerCommand::SetPaused(paused) => PlayerCommand::SetPaused(paused),
            CoordinatorPlayerCommand::SetPosition(position_seconds) => {
                PlayerCommand::SetPosition(position_seconds)
            }
            CoordinatorPlayerCommand::SetPlaybackRate(rate) => PlayerCommand::SetPlaybackRate(rate),
        };
        match self.player.execute_tracked(player_command.clone()) {
            Ok(player_command_id) => {
                self.playback_coordination
                    .bind_player_command(player_command_id, command_id);
                Ok(())
            }
            Err(PlayerError::Unsupported("execute_tracked")) => {
                match self.player.execute(player_command) {
                    Ok(()) => {
                        self.playback_coordination
                            .command_dispatch_succeeded(command_id);
                        Ok(())
                    }
                    Err(error) => {
                        let now_seconds = self
                            .playback_coordination
                            .coordinator_now(external_now_seconds);
                        self.playback_coordination
                            .command_dispatch_failed(command_id, now_seconds);
                        Err(error)
                    }
                }
            }
            Err(error) => {
                let now_seconds = self
                    .playback_coordination
                    .coordinator_now(external_now_seconds);
                self.playback_coordination
                    .command_dispatch_failed(command_id, now_seconds);
                Err(error)
            }
        }
    }

    fn report_playback_barrier_observations(
        &mut self,
        actions: &[PlaybackCoordinatorAction],
    ) -> Result<(), ClientEffectError> {
        if let Some(signature) = self
            .playback_coordination
            .barrier_ready_signature(&self.session)
            && self.playback_coordination.last_reported_barrier_ready != Some(signature)
            && self.report_playback_barrier_media_ready(
                signature.room_media_generation,
                signature.loaded,
                signature.seekable,
                signature.buffer_ready,
            )?
        {
            self.playback_coordination
                .mark_barrier_ready_reported(signature);
        }

        if let Some(observation) = self
            .playback_coordination
            .room_buffering_observation(&self.session)
            && self.playback_coordination.should_report_room_buffering(
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
            )
            && let Some(state) = self.session.playback_barrier_transport_observation(
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
                observation.buffered_seconds,
                observation.observed_at,
            )
        {
            self.control.activate_protocol_connection_generation();
            self.control.emit(ClientEffect::SendState(state))?;
            self.playback_coordination.mark_room_buffering_reported(
                observation.media_generation,
                observation.state_revision,
                observation.buffering,
            );
        }

        for action in actions {
            let PlaybackCoordinatorAction::Started {
                media_generation: local_media_generation,
                observed_position_seconds,
                ..
            } = action
            else {
                continue;
            };
            let Some((room_media_generation, room_state_revision)) = self
                .playback_coordination
                .barrier_started_target(&self.session, *local_media_generation)
            else {
                continue;
            };
            if self.playback_coordination.last_reported_barrier_started
                == Some((room_media_generation, room_state_revision))
            {
                continue;
            }
            let observed_at = self.playback_coordination.latest_observed_at_seconds();
            if self.report_playback_barrier_started(
                room_media_generation,
                room_state_revision,
                *observed_position_seconds,
                true,
                observed_at,
            )? {
                self.playback_coordination
                    .mark_barrier_started_reported(room_media_generation, room_state_revision);
            }
        }
        Ok(())
    }

    fn apply_external_coordinator_control_actions(
        &mut self,
        actions: &[PlaybackCoordinatorAction],
    ) {
        if actions
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::RequestRoomPause { .. }))
        {
            let _ = self.run_set_paused(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_player_api::{
        DisconnectedPlayer, PlayerMediaGeneration, PlayerObservationTimestamp,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
    };
    use sorotte_protocol::{
        PlaybackBarrierParticipantStatus, PlaybackBarrierPhase, PlaybackBarrierStatusPayload,
        ProtocolMessage, SetPayload,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    fn barrier_session() -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
            )
            .expect("barrier-aware hello should apply");
        session
    }

    fn apply_barrier_extension(
        session: &mut ClientSession,
        extension: PlaybackBarrierSetExtension,
    ) {
        session
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(extension),
            ))
            .expect("barrier extension should apply");
    }

    fn transport(
        adapter_generation: u64,
        observed_at_seconds: f64,
        phase: PlayerTransportPhase,
        position_seconds: f64,
    ) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(adapter_generation),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(
                observed_at_seconds,
            )),
        )
        .with_phase(phase)
        .with_position_seconds(position_seconds)
        .with_logical_pause(false);
        update.paused_for_cache = Some(matches!(phase, PlayerTransportPhase::Rebuffering));
        update.seeking = Some(matches!(phase, PlayerTransportPhase::Seeking));
        update.seekable = Some(true);
        update.core_idle = Some(false);
        update
    }

    #[test]
    fn logical_media_identity_does_not_collapse_url_queries_or_basenames() {
        assert!(!logical_media_ids_match(
            "https://youtube.com/watch?v=video-a",
            "https://youtube.com/watch?v=video-b",
        ));
        assert!(!logical_media_ids_match(
            "C:/Alice/Movies/episode.mkv",
            "D:/Bob/Downloads/episode.mkv",
        ));
        assert!(logical_media_ids_match(
            "sha256:stable-logical-id",
            "sha256:stable-logical-id",
        ));
    }

    #[test]
    fn published_logical_identity_is_private_and_stable_across_peer_paths_and_youtube_forms() {
        let alice = LocalFileUpdate::new("episode.mkv")
            .with_size_bytes(42_000)
            .with_path("C:/Alice/episode.mkv");
        let bob = LocalFileUpdate::new("episode.mkv")
            .with_size_bytes(42_000)
            .with_duration_seconds(1_800.0)
            .with_path("D:/Bob/Videos/episode.mkv");
        assert_eq!(
            logical_media_id_for_local_file_update(&alice),
            logical_media_id_for_local_file_update(&bob)
        );

        let watch =
            LocalFileUpdate::new("https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=share");
        let short = LocalFileUpdate::new("https://youtu.be/dQw4w9WgXcQ?t=12");
        let watch_id = logical_media_id_for_local_file_update(&watch);
        assert_eq!(watch_id, logical_media_id_for_local_file_update(&short));
        assert!(!watch_id.as_str().contains("dQw4w9WgXcQ"));
        assert!(!watch_id.as_str().contains("youtube"));
    }

    #[test]
    fn controller_media_prepare_emits_start_and_room_buffering_policy_once() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Quorum),
            quorum_percent: 60,
            timeout_seconds: 12.0,
            timeout_action: PlaybackBarrierTimeoutAction::Continue,
        });
        runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
            policy: RoomBufferingPolicy::Quorum,
            quorum_percent: 70,
            maximum_pause_seconds: 20.0,
            ..PlaybackBarrierRoomBufferingConfig::default()
        });

        let logical_id = LogicalMediaId::new("media-sha256:opaque-id").unwrap();
        let initial = runtime.prepare_playback_media(
            logical_id.clone(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        let refreshed =
            runtime.prepare_playback_media(logical_id, MediaTransportKind::NetworkVod, 101.0);

        assert!(initial.logical_media_changed);
        assert!(!refreshed.logical_media_changed);
        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
            panic!("media preparation should emit a reliable Set");
        };
        let extension = set
            .set
            .playback_barrier_v1()
            .expect("extension should decode")
            .expect("extension should be present");
        let prepare = extension.prepare.expect("start prepare should be present");
        assert_eq!(prepare.logical_media_id, "media-sha256:opaque-id");
        assert_eq!(prepare.policy, PlaybackBarrierPolicy::Quorum);
        assert_eq!(prepare.quorum_percent, Some(60));
        assert_eq!(prepare.timeout_ms, Some(12_000));
        let buffering = extension
            .buffering_policy
            .expect("ongoing buffering policy should be present");
        assert_eq!(buffering.media_generation, prepare.media_generation);
        assert_eq!(buffering.policy, RoomBufferingPolicy::Quorum);
        assert_eq!(buffering.quorum_percent, Some(70));
        assert_eq!(buffering.max_pause_ms, Some(20_000));
    }

    #[test]
    fn legacy_server_never_receives_sorotte_barrier_control() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{}}}"#,
            )
            .unwrap();
        let mut runtime =
            ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::AllEligible),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("media-sha256:opaque-id").unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn ongoing_buffering_reports_only_transport_state_transitions() {
        let logical_id = "media-sha256:opaque-id";
        let mut session = barrier_session();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(PrepareMediaPayload::new(
                    10,
                    logical_id,
                    0.0,
                    PlaybackBarrierPolicy::Controller,
                ))
                .with_buffering_policy(
                    RoomBufferingPolicyPayload::new(10, RoomBufferingPolicy::PauseAnyEligible)
                        .with_debounce_ms(750)
                        .with_resume_hysteresis_ms(1_500)
                        .with_max_pause_ms(30_000),
                ),
        );
        let mut runtime =
            ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
        runtime.prepare_playback_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        runtime.control.drain_outbound_messages();

        runtime.observe_external_player_transport(
            transport(1, 1.0, PlayerTransportPhase::Rebuffering, 4.0),
            101.0,
        );
        let first = runtime
            .control
            .drain_outbound_message_lines()
            .expect("buffering report should encode");
        assert!(
            first
                .iter()
                .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":true"))
        );

        runtime.observe_external_player_transport(
            transport(1, 2.0, PlayerTransportPhase::Rebuffering, 4.0),
            102.0,
        );
        assert!(runtime.control().outbound_messages().is_empty());

        runtime.observe_external_player_transport(
            transport(1, 3.0, PlayerTransportPhase::Playing, 4.5),
            103.0,
        );
        let recovered = runtime
            .control
            .drain_outbound_message_lines()
            .expect("recovery report should encode");
        assert!(
            recovered
                .iter()
                .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":false"))
        );
    }

    #[test]
    fn configured_timeout_pause_is_one_shot_for_initiating_controller() {
        let mut session = barrier_session();
        let mut participants = BTreeMap::new();
        participants.insert(
            "alice-client".to_owned(),
            PlaybackBarrierParticipantStatus {
                phase: PlaybackBarrierParticipantPhase::TimedOut,
                readiness: None,
                observed_position: None,
                degraded_reason: None,
            },
        );
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(PrepareMediaPayload::new(
                    22,
                    "media-sha256:opaque-id",
                    0.0,
                    PlaybackBarrierPolicy::AllEligible,
                ))
                .with_status(PlaybackBarrierStatusPayload {
                    media_generation: 22,
                    state_revision: None,
                    phase: PlaybackBarrierPhase::Preparing,
                    policy: PlaybackBarrierPolicy::AllEligible,
                    quorum: None,
                    deadline: 100.0,
                    participants,
                    excluded_legacy_clients: BTreeSet::new(),
                }),
        );
        let mut coordination = RuntimePlaybackCoordination::default();
        let plan = coordination.prepare_media(
            LogicalMediaId::new("media-sha256:opaque-id").unwrap(),
            MediaTransportKind::NetworkVod,
            90.0,
        );
        coordination.initiated_barrier = Some((plan.media_generation, 22));
        coordination.set_barrier_start_config(PlaybackBarrierStartConfig {
            timeout_action: PlaybackBarrierTimeoutAction::RemainPaused,
            ..PlaybackBarrierStartConfig::default()
        });

        assert!(coordination.barrier_timeout_requires_room_pause(&session));
        assert_eq!(
            coordination.pending_barrier_timeout_action.take(),
            Some(PlaybackBarrierTimeoutAction::RemainPaused)
        );
        assert!(!coordination.barrier_timeout_requires_room_pause(&session));
    }

    #[test]
    fn refreshed_adapter_generation_stays_bound_to_stable_logical_generation() {
        let mut runtime = RuntimePlaybackCoordination::default();
        let logical_id = LogicalMediaId::new("plex://server/item/42").unwrap();
        let initial =
            runtime.prepare_media(logical_id.clone(), MediaTransportKind::NetworkVod, 0.0);
        runtime.observe_transport(transport(10, 1.0, PlayerTransportPhase::Playing, 1.0), 1.0);

        let refreshed = runtime.prepare_media(logical_id, MediaTransportKind::NetworkVod, 2.0);
        runtime.observe_transport(
            transport(11, 3.0, PlayerTransportPhase::ReadyPaused, 1.0),
            3.0,
        );

        assert_eq!(refreshed.media_generation, initial.media_generation);
        assert_eq!(refreshed.load_attempt, 2);
        assert!(runtime.snapshot().transport_telemetry_observed);
        assert_eq!(
            runtime.snapshot().media_generation,
            Some(initial.media_generation)
        );
        assert_eq!(
            runtime.adapter_generation_bindings.get(&10),
            Some(&initial.media_generation)
        );
        assert_eq!(
            runtime.adapter_generation_bindings.get(&11),
            Some(&initial.media_generation)
        );
    }

    #[test]
    fn stale_adapter_generation_cannot_seize_current_transport_ownership() {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        runtime.observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 1.0), 1.0);
        runtime.prepare_media(
            LogicalMediaId::new("episode-2").unwrap(),
            MediaTransportKind::NetworkVod,
            2.0,
        );

        let actions =
            runtime.observe_transport(transport(1, 3.0, PlayerTransportPhase::Playing, 100.0), 3.0);

        assert!(actions.is_empty());
        assert!(!runtime.snapshot().transport_telemetry_observed);
        assert_eq!(runtime.snapshot().metrics.stale_generation_observations, 1);
    }

    #[test]
    fn adapter_epoch_reset_accepts_restarted_generation_and_rejects_old_epoch() {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            90.0,
        );
        runtime.observe_transport_at_epoch(
            transport(5, 50.0, PlayerTransportPhase::Playing, 5.0),
            95.0,
            0,
        );
        let next_epoch = runtime.reset_adapter_epoch(100.0);
        assert_eq!(next_epoch, 1);

        let stale = runtime.observe_transport_at_epoch(
            transport(6, 51.0, PlayerTransportPhase::Playing, 99.0),
            101.0,
            0,
        );
        assert!(stale.is_empty());
        assert!(!runtime.snapshot().transport_telemetry_observed);

        runtime.observe_transport_at_epoch(
            transport(1, 1.0, PlayerTransportPhase::ReadyPaused, 5.0),
            102.0,
            next_epoch,
        );
        assert!(runtime.snapshot().transport_telemetry_observed);
        assert_eq!(
            runtime
                .latest_observation
                .as_ref()
                .map(|observation| observation.observed_at_seconds),
            Some(102.0)
        );
    }

    #[test]
    fn periodic_desired_reconciliation_does_not_advance_recovery_stability() {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        let mut session = ClientSession::default();
        session.model.room.name = Some("room".to_owned());
        session.model.room.playstates.insert(
            "room".to_owned(),
            RoomPlaystateView {
                paused: Some(false),
                position: Some(40.0),
                ..RoomPlaystateView::default()
            },
        );
        session
            .model
            .room
            .playstate_updated_at_seconds
            .insert("room".to_owned(), 10.0);
        runtime.update_desired_from_session(&session, 10.0);
        // Treat the initial media-position handoff as already satisfied; this
        // test isolates the recovery episode rather than startup seeking.
        runtime.pending_forced_seek_revision = None;
        runtime.update_desired_from_session(&session, 10.0);
        runtime.observe_transport(
            transport(1, 10.0, PlayerTransportPhase::Rebuffering, 10.0),
            10.0,
        );
        runtime.observe_transport(
            transport(1, 11.0, PlayerTransportPhase::Playing, 10.2),
            11.0,
        );
        runtime.observe_transport(
            transport(1, 12.0, PlayerTransportPhase::Playing, 10.5),
            12.0,
        );
        let episode = runtime
            .snapshot()
            .recovery_episode
            .expect("buffer recovery should remain active");
        assert_eq!(episode.hard_seek_attempts, 1);

        for now_seconds in [12.5, 13.0, 14.0, 15.0] {
            runtime.update_desired_from_session(&session, now_seconds);
        }

        let after = runtime
            .snapshot()
            .recovery_episode
            .expect("reconciliation without a fresh observation cannot close recovery");
        assert_eq!(after.id, episode.id);
        assert_eq!(after.hard_seek_attempts, 1);
        assert_eq!(runtime.snapshot().metrics.hard_seek_count, 1);
    }
}
