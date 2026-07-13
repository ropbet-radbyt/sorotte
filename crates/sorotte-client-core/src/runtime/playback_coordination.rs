use super::*;

use crate::control::client_effect_player_error;

use std::collections::BTreeMap;

use sorotte_player_api::{
    PlayerCommand, PlayerCommandId, PlayerCommandProgressState, PlayerCommandResult,
    PlayerMediaGeneration, PlayerTransportTelemetryUpdate,
};
pub use sorotte_protocol::PlaybackBarrierTimeoutAction;
use sorotte_protocol::{
    PlaybackBarrierParticipantPhase, PlaybackBarrierPhase, PlaybackBarrierPolicy,
    PlaybackBarrierRecoveryDisposition, PlaybackBarrierRecoveryPayload,
    PlaybackBarrierRequestResultStatus, PlaybackBarrierSetExtension, PrepareMediaPayload,
    RoomBufferingPolicy, RoomBufferingPolicyPayload,
};

const PLAYBACK_BARRIER_RETRY_MIN_SECONDS: f64 = 0.1;
const PLAYBACK_BARRIER_RETRY_MAX_SECONDS: f64 = 30.0;
const PLAYBACK_BARRIER_RETRY_MAX_BACKOFF_EXPONENT: u32 = 5;

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
    /// Compatibility view used by attached-player integrations. Dormant
    /// controlled-room intent is deliberately hidden until fresh authority
    /// for the current connection has been established.
    pub pending_local_pause_intent: Option<bool>,
    pub pending_local_pause_intent_dormant: bool,
    pub last_local_pause_intent_stage_accepted: Option<bool>,
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
    local_echo: bool,
    barrier_media_generation: Option<u64>,
    barrier_state_revision: Option<u64>,
    buffering_media_generation: Option<u64>,
    buffering_state_revision: Option<u64>,
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
    report_epoch: u64,
    media_generation: u64,
    state_revision: Option<u64>,
    buffering: bool,
    buffered_seconds: Option<f64>,
    observed_at: Option<f64>,
}

#[derive(Clone, PartialEq)]
struct PendingMediaCoordinationIntent {
    local_media_generation: u64,
    load_intent: MediaLoadIntent,
    include_start_barrier: bool,
    request_id: String,
    retry_request_nonce: Option<u64>,
    retry_not_before_seconds: Option<f64>,
    retry_attempts: u32,
    room: Option<String>,
}

impl std::fmt::Debug for PendingMediaCoordinationIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingMediaCoordinationIntent")
            .field("local_media_generation", &self.local_media_generation)
            .field("load_intent", &self.load_intent)
            .field("include_start_barrier", &self.include_start_barrier)
            .field("request_id", &"<redacted>")
            .field("retry_request_nonce", &self.retry_request_nonce)
            .field("retry_not_before_seconds", &self.retry_not_before_seconds)
            .field("retry_attempts", &self.retry_attempts)
            .field("room", &self.room)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct PlaybackBarrierOperation {
    local_media_generation: u64,
    load_intent: MediaLoadIntent,
    include_start_barrier: bool,
    request_id: String,
    request_nonce: u64,
    logical_media_id: String,
    room: String,
}

impl std::fmt::Debug for PlaybackBarrierOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackBarrierOperation")
            .field("local_media_generation", &self.local_media_generation)
            .field("load_intent", &self.load_intent)
            .field("include_start_barrier", &self.include_start_barrier)
            .field("request_id", &"<redacted>")
            .field("request_nonce", &self.request_nonce)
            .field("logical_media_id", &"<redacted>")
            .field("room", &self.room)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPlaybackBarrierRecovery {
    operation: PlaybackBarrierOperation,
    recovery_nonce: Option<u64>,
    room: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingPlaybackBarrierRequest {
    pub(crate) extension: PlaybackBarrierSetExtension,
    pub(crate) room: String,
    pub(crate) local_media_generation: u64,
    pub(crate) request_nonce: u64,
    operation: PlaybackBarrierOperation,
    recovery_request: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalTransportGeneration {
    logical_generation: u64,
    load_attempt: u64,
    adapter_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReconnectReconciliation {
    target_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalIntentAuthorization {
    Authorized,
    AwaitingControlledRoomReauthentication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalControlAuthorityFreshness {
    Awaiting,
    Authorized,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionLocalControlAuthority {
    room: String,
    username: Option<String>,
    connection_generation: u64,
    freshness: LocalControlAuthorityFreshness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingLocalPauseIntent {
    paused: bool,
    room: String,
    local_media_generation: u64,
    connection_generation: u64,
    authorization: LocalIntentAuthorization,
    replay_player_after_reauthorization: bool,
}

#[derive(Debug, Default)]
pub(crate) struct RuntimePlaybackCoordination {
    coordinator: PlaybackCoordinator,
    adapter_generation_bindings: BTreeMap<u64, LocalTransportGeneration>,
    pending_media_identity: Option<(u64, u64)>,
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
    pending_local_pause_intent: Option<PendingLocalPauseIntent>,
    last_local_pause_intent_stage_accepted: Option<bool>,
    connection_generation: u64,
    local_control_authority: Option<ConnectionLocalControlAuthority>,
    pending_forced_seek_revision: Option<u64>,
    transport_telemetry_observed: bool,
    transport_telemetry_available: bool,
    reconnect_reconciliation: Option<ReconnectReconciliation>,
    last_applied_revision: Option<u64>,
    last_started_revision: Option<u64>,
    last_degraded_reason: Option<DegradedPlaybackReason>,
    last_reported_barrier_ready: Option<BarrierReadySignature>,
    last_reported_barrier_started: Option<(u64, u64)>,
    barrier_start_config: PlaybackBarrierStartConfig,
    room_buffering_config: PlaybackBarrierRoomBufferingConfig,
    next_room_barrier_request_nonce: u64,
    initiated_barrier: Option<PlaybackBarrierOperation>,
    accepted_barrier: Option<PlaybackBarrierOperation>,
    pending_barrier_recovery: Option<PendingPlaybackBarrierRecovery>,
    accepted_barrier_terminal: bool,
    pending_media_coordination: Option<PendingMediaCoordinationIntent>,
    handled_barrier_timeout: Option<(u64, Option<u64>)>,
    pending_barrier_timeout_action: Option<PlaybackBarrierTimeoutAction>,
    last_reported_room_buffering: Option<(u64, u64, Option<u64>, bool)>,
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
        self.prepare_media_internal(logical_id, kind, None, now_seconds)
    }

    pub(crate) fn prepare_media_with_intent(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        self.prepare_media_internal(logical_id, kind, Some(intent), now_seconds)
    }

    fn prepare_media_internal(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: Option<MediaLoadIntent>,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let placeholder_adapter_generation = self
            .coordinator
            .current_logical_media_id()
            .filter(|logical_id| logical_id.as_str().starts_with("adapter-media-generation-"))
            .and(self.highest_bound_adapter_generation);
        let plan = match intent {
            Some(intent) => {
                self.coordinator
                    .prepare_media_with_intent(logical_id, kind, intent, now_seconds)
            }
            None => self
                .coordinator
                .prepare_media(logical_id, kind, now_seconds),
        };
        self.adapter_generation_bindings
            .retain(|_, binding| binding.logical_generation != plan.media_generation);
        self.pending_media_identity = Some((plan.media_generation, plan.load_attempt));
        self.latest_observation = None;
        self.transport_telemetry_observed = false;
        self.player_command_bindings.clear();
        self.last_reported_barrier_ready = None;
        self.last_reported_barrier_started = None;
        if plan.playback_episode_changed {
            self.desired_generation = None;
            self.desired_fingerprint = None;
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            self.pending_forced_seek_revision = None;
            self.last_applied_revision = None;
            self.last_started_revision = None;
            self.last_degraded_reason = None;
            self.initiated_barrier = None;
            self.accepted_barrier = None;
            self.pending_barrier_recovery = None;
            self.accepted_barrier_terminal = false;
            self.pending_media_coordination = Some(PendingMediaCoordinationIntent {
                local_media_generation: plan.media_generation,
                load_intent: plan.load_intent,
                include_start_barrier: true,
                request_id: new_playback_barrier_request_id(plan.media_generation),
                retry_request_nonce: None,
                retry_not_before_seconds: None,
                retry_attempts: 0,
                room: None,
            });
            self.handled_barrier_timeout = None;
            self.pending_barrier_timeout_action = None;
            self.last_reported_room_buffering = None;
        }
        if let Some(adapter_generation) = placeholder_adapter_generation {
            self.adapter_generation_bindings.insert(
                adapter_generation,
                LocalTransportGeneration {
                    logical_generation: plan.media_generation,
                    load_attempt: plan.load_attempt,
                    adapter_generation,
                },
            );
            self.pending_media_identity = None;
        }
        plan
    }

    pub(crate) fn reset_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        self.adapter_epoch = self.adapter_epoch.saturating_add(1);
        self.adapter_generation_bindings.clear();
        self.highest_bound_adapter_generation = None;
        self.pending_media_identity = self
            .coordinator
            .current_media_generation()
            .zip(self.coordinator.current_load_attempt());
        self.player_command_bindings.clear();
        self.latest_observation = None;
        self.adapter_clock_offset_seconds = None;
        self.last_external_now_seconds = None;
        self.last_coordinator_now_seconds = None;
        self.transport_telemetry_observed = false;
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.last_reported_barrier_ready = None;
        self.last_reported_barrier_started = None;
        self.last_reported_room_buffering = None;
        self.pending_barrier_timeout_action = None;
        if let Some(reconciliation) = self.reconnect_reconciliation.as_mut() {
            reconciliation.target_revision = None;
        }
        self.coordinator.reset_transport_adapter_epoch(now_seconds);
        self.adapter_epoch
    }

    pub(crate) fn mark_transport_telemetry_available(&mut self) {
        self.transport_telemetry_available = true;
    }

    pub(crate) fn reconnect_coordinator_available(&self) -> bool {
        self.transport_telemetry_available && self.coordinator.current_media_generation().is_some()
    }

    /// Starts an observation-backed reconnect correction episode.
    ///
    /// The first desired-state update after this call is always a new forced
    /// revision. This is deliberately separate from the legacy reconnect
    /// validator: accepting a player command cannot complete this episode.
    pub(crate) fn begin_reconnect_reconciliation(&mut self) -> bool {
        if self.reconnect_reconciliation.is_some() {
            return false;
        }
        self.reconnect_reconciliation = Some(ReconnectReconciliation::default());
        true
    }

    pub(crate) fn reconnect_reconciliation_complete(&self) -> bool {
        self.reconnect_reconciliation
            .and_then(|reconciliation| reconciliation.target_revision)
            .is_some_and(|target_revision| self.last_applied_revision == Some(target_revision))
    }

    pub(crate) fn finish_reconnect_reconciliation(&mut self) {
        self.reconnect_reconciliation = None;
    }

    pub(crate) fn interrupt_recovery(&mut self) -> Vec<PlaybackCoordinatorAction> {
        let actions = self.coordinator.interrupt_recovery();
        self.record_observation_outcomes(&actions);
        actions
    }

    pub(crate) fn snapshot(&self) -> PlaybackCoordinationSnapshot {
        let active_local_pause_intent = self
            .pending_local_pause_intent
            .as_ref()
            .filter(|intent| {
                intent.connection_generation == self.connection_generation
                    && intent.authorization == LocalIntentAuthorization::Authorized
            })
            .map(|intent| intent.paused);
        PlaybackCoordinationSnapshot {
            media_generation: self.coordinator.current_media_generation(),
            pending_local_pause_intent: active_local_pause_intent,
            pending_local_pause_intent_dormant: self.pending_local_pause_intent.is_some()
                && active_local_pause_intent.is_none(),
            last_local_pause_intent_stage_accepted: self.last_local_pause_intent_stage_accepted,
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

    fn next_room_barrier_request_nonce(&mut self) -> u64 {
        self.next_room_barrier_request_nonce = self
            .next_room_barrier_request_nonce
            .saturating_add(1)
            .max(1);
        self.next_room_barrier_request_nonce
    }

    fn bind_local_control_authority_context(
        &mut self,
        session: &ClientSession,
        controlled_freshness: LocalControlAuthorityFreshness,
    ) {
        let room = session.room().unwrap_or_default().to_owned();
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            freshness: if controlled_room_name(&room) {
                controlled_freshness
            } else {
                LocalControlAuthorityFreshness::Authorized
            },
            room,
            username: session.username().map(str::to_owned),
            connection_generation: self.connection_generation,
        });
    }

    fn local_control_authority_freshness(
        &self,
        session: &ClientSession,
    ) -> Option<LocalControlAuthorityFreshness> {
        let authority = self.local_control_authority.as_ref()?;
        (authority.connection_generation == self.connection_generation
            && Some(authority.room.as_str()) == session.room()
            && authority.username.as_deref() == session.username())
        .then_some(authority.freshness)
    }

    fn current_connection_local_control_is_authorized(&self, session: &ClientSession) -> bool {
        match self.local_control_authority_freshness(session) {
            Some(LocalControlAuthorityFreshness::Authorized) => true,
            Some(
                LocalControlAuthorityFreshness::Awaiting | LocalControlAuthorityFreshness::Denied,
            ) => false,
            // Coordination used directly in lower-level tests has no
            // connection lifecycle binding. Preserve that compatibility
            // path, but never fall back to a cached session projection once
            // connection-scoped authority tracking has begun.
            None if self.local_control_authority.is_none() => {
                session.local_can_control() == Some(true)
            }
            None => false,
        }
    }

    pub(crate) fn begin_protocol_connection_generation(&mut self, session: &ClientSession) {
        self.connection_generation = self.connection_generation.saturating_add(1).max(1);
        self.bind_local_control_authority_context(
            session,
            LocalControlAuthorityFreshness::Awaiting,
        );
        if let Some(intent) = self.pending_local_pause_intent.as_mut() {
            if controlled_room_name(&intent.room) {
                // A controller projection restored from the previous session
                // is not authority for the replacement transport. Keep the
                // user's command as dormant semantic intent until this
                // connection receives fresh controller evidence.
                intent.authorization =
                    LocalIntentAuthorization::AwaitingControlledRoomReauthentication;
                intent.replay_player_after_reauthorization = true;
            } else {
                intent.connection_generation = self.connection_generation;
                intent.authorization = LocalIntentAuthorization::Authorized;
            }
        }
        self.last_reported_barrier_ready = None;
        self.last_reported_barrier_started = None;
        self.last_reported_room_buffering = None;

        let recovering = self
            .pending_barrier_recovery
            .take()
            .map(|recovery| recovery.operation);
        let accepted = self.accepted_barrier.take();
        let initiated = self.initiated_barrier.take();
        let recoverable = recovering.or(accepted).or(initiated);

        if let Some(operation) = recoverable {
            // A socket write cannot distinguish an accepted request from
            // bytes lost before server parsing. Recover even a terminal
            // lifecycle first: the retained server operation also owns the
            // ongoing buffering policy and may need to be rebound from an
            // overlapping old transport identity.
            self.pending_barrier_recovery = Some(PendingPlaybackBarrierRecovery {
                operation,
                recovery_nonce: None,
                room: None,
            });
        } else if let Some(pending) = self.pending_media_coordination.as_mut() {
            // An unsent current-media intent remains valid, but it must bind
            // to the newly authenticated room and receive a fresh nonce.
            pending.room = None;
        }
    }

    pub(crate) fn playback_barrier_set_for_new_media(
        &mut self,
        plan: &MediaLoadPlan,
        session: &ClientSession,
        now_seconds: f64,
    ) -> Option<PendingPlaybackBarrierRequest> {
        if !plan.playback_episode_changed {
            return None;
        }

        self.playback_barrier_set_for_pending_media(session, now_seconds)
    }

    pub(crate) fn playback_barrier_set_for_pending_media(
        &mut self,
        session: &ClientSession,
        now_seconds: f64,
    ) -> Option<PendingPlaybackBarrierRequest> {
        if let Some(mut recovery) = self.pending_barrier_recovery.clone() {
            let room = session.room()?.to_owned();
            if recovery.room.as_deref() != Some(room.as_str()) {
                recovery.room = Some(room.clone());
                recovery.recovery_nonce = None;
                self.pending_barrier_recovery = Some(recovery.clone());
            }
            if recovery.recovery_nonce.is_some()
                || !session.playback_barrier_v1_negotiated()
                || session.local_can_control() != Some(true)
                || self.coordinator.current_media_generation()
                    != Some(recovery.operation.local_media_generation)
                || !self.current_logical_media_matches(&recovery.operation.logical_media_id)
            {
                return None;
            }

            let recovery_nonce = self.next_room_barrier_request_nonce();
            let extension = PlaybackBarrierSetExtension::new().with_recovery(
                PlaybackBarrierRecoveryPayload::query(
                    recovery.operation.request_id.clone(),
                    recovery.operation.request_nonce,
                    recovery_nonce,
                    recovery.operation.logical_media_id.clone(),
                ),
            );
            return Some(PendingPlaybackBarrierRequest {
                extension,
                room,
                local_media_generation: recovery.operation.local_media_generation,
                request_nonce: recovery_nonce,
                operation: recovery.operation,
                recovery_request: true,
            });
        }

        let mut intent = self.pending_media_coordination.clone()?;
        let room = session.room()?.to_owned();
        if intent.room.as_deref() != Some(room.as_str()) {
            // The old serialized scope is cancelled by the control outbox.
            // The current-media semantic intent remains useful in the new
            // room and is rebound only after its authorization succeeds.
            intent.room = Some(room.clone());
            self.pending_media_coordination = Some(intent.clone());
        }
        if self.initiated_barrier.as_ref().is_some_and(|operation| {
            operation.local_media_generation == intent.local_media_generation
        }) || !session.playback_barrier_v1_negotiated()
            || session.local_can_control() != Some(true)
            || intent
                .retry_not_before_seconds
                .is_some_and(|deadline| !now_seconds.is_finite() || now_seconds < deadline)
        {
            return None;
        }

        let (local_generation, logical_media_id) = self.current_logical_media()?;
        if local_generation != intent.local_media_generation {
            self.pending_media_coordination = None;
            return None;
        }
        if intent.include_start_barrier
            && intent.load_intent != MediaLoadIntent::Replay
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                logical_media_ids_match(&logical_media_id, &prepare.logical_media_id)
                    && session.playback_barrier_status().is_some_and(|status| {
                        status.media_generation == prepare.media_generation
                            && matches!(
                                status.phase,
                                PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::Committed
                            )
                    })
            })
        {
            // A peer has already established the room generation for this
            // logical source. Loading that source locally is participation,
            // not a competing start request.
            self.pending_media_coordination = None;
            return None;
        }
        let load_intent = if intent.include_start_barrier
            && intent.load_intent == MediaLoadIntent::NewPlayback
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                logical_media_ids_match(&logical_media_id, &prepare.logical_media_id)
                    && session.playback_barrier_status().is_some_and(|status| {
                        status.media_generation == prepare.media_generation
                            && matches!(
                                status.phase,
                                PlaybackBarrierPhase::AwaitingDecision
                                    | PlaybackBarrierPhase::Complete
                                    | PlaybackBarrierPhase::Degraded
                            )
                    })
            }) {
            // A fresh coordinator has no local load history from which to
            // infer replay intent. Retained terminal room history provides
            // the missing identity without weakening active-generation
            // exclusion above.
            MediaLoadIntent::Replay
        } else {
            intent.load_intent
        };
        let request_nonce = intent
            .retry_request_nonce
            .unwrap_or_else(|| self.next_room_barrier_request_nonce());
        let mut extension = PlaybackBarrierSetExtension::new();
        let mut start_requested = false;
        if intent.include_start_barrier
            && let Some(policy) = self.barrier_start_config.policy
        {
            let timeout_ms = (self.barrier_start_config.timeout_seconds * 1_000.0)
                .round()
                .clamp(1.0, u64::MAX as f64) as u64;
            let mut prepare = PrepareMediaPayload::request(
                request_nonce,
                logical_media_id.clone(),
                0.0,
                policy,
                load_intent,
            )
            .with_request_id(intent.request_id.clone())
            .with_timeout_ms(timeout_ms)
            .with_timeout_action(self.barrier_start_config.timeout_action);
            if policy == PlaybackBarrierPolicy::Quorum {
                prepare = prepare.with_quorum_percent(self.barrier_start_config.quorum_percent);
            }
            extension = extension.with_prepare(prepare);
            start_requested = true;
        }

        let room_config = self.room_buffering_config;
        let mut buffering = RoomBufferingPolicyPayload::new(0, room_config.policy)
            .with_request_nonce(request_nonce)
            .with_request_id(intent.request_id.clone())
            .with_load_intent(load_intent)
            .with_debounce_ms(seconds_to_milliseconds(room_config.debounce_seconds))
            .with_resume_hysteresis_ms(seconds_to_milliseconds(
                room_config.resume_hysteresis_seconds,
            ))
            .with_max_pause_ms(seconds_to_milliseconds(room_config.maximum_pause_seconds));
        if room_config.policy == RoomBufferingPolicy::Quorum {
            buffering = buffering.with_quorum_percent(room_config.quorum_percent);
        }
        let operation = PlaybackBarrierOperation {
            local_media_generation: local_generation,
            load_intent,
            include_start_barrier: start_requested,
            request_id: intent.request_id,
            request_nonce,
            logical_media_id,
            room: room.clone(),
        };
        Some(PendingPlaybackBarrierRequest {
            extension: extension.with_buffering_policy(buffering),
            room,
            local_media_generation: local_generation,
            request_nonce,
            operation,
            recovery_request: false,
        })
    }

    pub(crate) fn confirm_playback_barrier_request_queued(
        &mut self,
        request: &PendingPlaybackBarrierRequest,
    ) {
        if request.recovery_request {
            if let Some(recovery) = self.pending_barrier_recovery.as_mut()
                && recovery.operation == request.operation
            {
                recovery.recovery_nonce = Some(request.request_nonce);
                recovery.room = Some(request.room.clone());
            }
            return;
        }
        if self
            .pending_media_coordination
            .as_ref()
            .is_some_and(|intent| {
                intent.local_media_generation == request.local_media_generation
                    && intent.request_id == request.operation.request_id
            })
        {
            self.initiated_barrier = Some(request.operation.clone());
        }
    }

    pub(crate) fn discard_room_scoped_playback_barrier_intent(&mut self) {
        self.initiated_barrier = None;
        self.accepted_barrier = None;
        self.pending_barrier_recovery = None;
        self.pending_media_coordination = None;
        self.accepted_barrier_terminal = false;
    }

    pub(crate) fn handle_authoritative_playback_barrier_room_change(&mut self) {
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            room: String::new(),
            username: None,
            connection_generation: self.connection_generation,
            freshness: LocalControlAuthorityFreshness::Awaiting,
        });
        if self.initiated_barrier.is_some()
            || self.accepted_barrier.is_some()
            || self.pending_barrier_recovery.is_some()
        {
            self.discard_room_scoped_playback_barrier_intent();
        } else if let Some(pending) = self.pending_media_coordination.as_mut() {
            // A pre-authentication media intent has never been serialized and
            // may safely bind to the authoritative destination room.
            pending.room = None;
        }
    }

    pub(crate) fn bind_authoritative_room_control_context(&mut self, session: &ClientSession) {
        self.bind_local_control_authority_context(
            session,
            LocalControlAuthorityFreshness::Awaiting,
        );
    }

    pub(crate) fn observe_playback_barrier_server_extension(
        &mut self,
        extension: &PlaybackBarrierSetExtension,
        session: &ClientSession,
        now_seconds: f64,
    ) -> bool {
        let retry_scheduled = extension.request_result.as_ref().is_some_and(|result| {
            if result.status != PlaybackBarrierRequestResultStatus::RetryLater
                || !now_seconds.is_finite()
            {
                return false;
            }
            let Some(operation) = self.initiated_barrier.as_ref() else {
                return false;
            };
            if operation.request_id != result.request_id
                || operation.request_nonce != result.request_nonce
            {
                return false;
            }
            let Some(intent) = self.pending_media_coordination.as_mut() else {
                return false;
            };
            if intent.local_media_generation != operation.local_media_generation
                || intent.request_id != operation.request_id
            {
                return false;
            }

            intent.retry_attempts = intent.retry_attempts.saturating_add(1);
            let retry_delay_seconds =
                playback_barrier_retry_delay_seconds(result.retry_after_ms, intent.retry_attempts);
            intent.retry_request_nonce = Some(operation.request_nonce);
            intent.retry_not_before_seconds = Some(now_seconds + retry_delay_seconds);
            self.initiated_barrier = None;
            true
        });

        if let Some(response) = extension.recovery.as_ref()
            && let Some(recovery) = self.pending_barrier_recovery.clone()
            && recovery.recovery_nonce == Some(response.recovery_nonce)
            && recovery.operation.request_id == response.request_id
            && recovery.operation.request_nonce == response.original_request_nonce
            && logical_media_ids_match(
                &recovery.operation.logical_media_id,
                &response.logical_media_id,
            )
        {
            match response.disposition {
                Some(PlaybackBarrierRecoveryDisposition::Recovered) => {}
                Some(PlaybackBarrierRecoveryDisposition::Existing) => {
                    let existing_lifecycle_applied =
                        session.playback_barrier_prepare().is_some_and(|prepare| {
                            logical_media_ids_match(
                                &recovery.operation.logical_media_id,
                                &prepare.logical_media_id,
                            )
                        });
                    if existing_lifecycle_applied {
                        self.discard_room_scoped_playback_barrier_intent();
                    }
                }
                Some(PlaybackBarrierRecoveryDisposition::Absent) => {
                    let operation = recovery.operation;
                    let was_terminal = self.accepted_barrier_terminal;
                    self.initiated_barrier = None;
                    self.accepted_barrier = None;
                    self.pending_barrier_recovery = None;
                    self.accepted_barrier_terminal = false;
                    self.pending_media_coordination = Some(PendingMediaCoordinationIntent {
                        local_media_generation: operation.local_media_generation,
                        load_intent: if was_terminal {
                            MediaLoadIntent::TransportRefresh
                        } else {
                            operation.load_intent
                        },
                        include_start_barrier: !was_terminal && operation.include_start_barrier,
                        request_id: if was_terminal {
                            new_playback_barrier_request_id(operation.local_media_generation)
                        } else {
                            operation.request_id
                        },
                        retry_request_nonce: (!was_terminal).then_some(operation.request_nonce),
                        retry_not_before_seconds: None,
                        retry_attempts: 0,
                        room: session.room().map(str::to_owned),
                    });
                }
                Some(
                    PlaybackBarrierRecoveryDisposition::Superseded
                    | PlaybackBarrierRecoveryDisposition::Rejected,
                ) => self.discard_room_scoped_playback_barrier_intent(),
                None => {}
            }
        }

        let candidate = self
            .initiated_barrier
            .clone()
            .or_else(|| self.accepted_barrier.clone())
            .or_else(|| {
                self.pending_barrier_recovery
                    .as_ref()
                    .map(|recovery| recovery.operation.clone())
            });
        if let Some(operation) = candidate {
            let prepare_matches = extension.prepare.as_ref().is_some_and(|prepare| {
                prepare.request_id.as_deref() == Some(operation.request_id.as_str())
                    && prepare.request_nonce == operation.request_nonce
                    && prepare.media_generation > 0
                    && logical_media_ids_match(
                        &operation.logical_media_id,
                        &prepare.logical_media_id,
                    )
                    && session
                        .playback_barrier_prepare()
                        .is_some_and(|accepted| accepted == prepare)
            });
            let policy_matches = extension.buffering_policy.as_ref().is_some_and(|policy| {
                policy.request_id.as_deref() == Some(operation.request_id.as_str())
                    && policy.request_nonce == operation.request_nonce
                    && policy.media_generation > 0
                    && session
                        .playback_barrier_buffering_policy()
                        .is_some_and(|accepted| accepted == policy)
            });
            if prepare_matches || policy_matches {
                self.initiated_barrier = Some(operation.clone());
                self.accepted_barrier = Some(operation);
                self.pending_media_coordination = None;
                self.pending_barrier_recovery = None;
            }
        }

        if let Some(operation) = self.accepted_barrier.as_ref()
            && session.playback_barrier_prepare().is_some_and(|prepare| {
                prepare.request_id.as_deref() == Some(operation.request_id.as_str())
                    && prepare.request_nonce == operation.request_nonce
            })
            && session.playback_barrier_status().is_some_and(|status| {
                matches!(
                    status.phase,
                    PlaybackBarrierPhase::Complete | PlaybackBarrierPhase::Degraded
                )
            })
        {
            self.accepted_barrier_terminal = true;
        }

        retry_scheduled
    }

    pub(crate) fn pending_playback_barrier_retry_delay_at(&self, now_seconds: f64) -> Option<f64> {
        let intent = self.pending_media_coordination.as_ref()?;
        let deadline = intent.retry_not_before_seconds?;
        (self.initiated_barrier.is_none() && now_seconds.is_finite())
            .then_some((deadline - now_seconds).max(0.0))
    }

    fn bind_adapter_generation(
        &mut self,
        adapter_generation: PlayerMediaGeneration,
        external_now_seconds: f64,
    ) -> Option<u64> {
        let adapter_generation = adapter_generation.get();
        if let Some(binding) = self.adapter_generation_bindings.get(&adapter_generation) {
            let current_identity = self
                .coordinator
                .current_media_generation()
                .zip(self.coordinator.current_load_attempt());
            return (current_identity == Some((binding.logical_generation, binding.load_attempt))
                && binding.adapter_generation == adapter_generation)
                .then_some(binding.logical_generation);
        }

        let (logical_generation, load_attempt) = match self.pending_media_identity {
            Some(identity)
                if self
                    .highest_bound_adapter_generation
                    .is_none_or(|highest| adapter_generation > highest) =>
            {
                self.pending_media_identity = None;
                identity
            }
            Some(_) => return None,
            None if self
                .highest_bound_adapter_generation
                .is_none_or(|highest| adapter_generation > highest) =>
            {
                if let Some(identity) = self
                    .coordinator
                    .current_media_generation()
                    .zip(self.coordinator.current_load_attempt())
                {
                    identity
                } else {
                    let logical_id = LogicalMediaId::new(format!(
                        "adapter-media-generation-{adapter_generation}"
                    ))
                    .expect("generated logical media ID is non-empty");
                    let plan = self.prepare_media(
                        logical_id,
                        MediaTransportKind::NetworkVod,
                        external_now_seconds,
                    );
                    (plan.media_generation, plan.load_attempt)
                }
            }
            None => return None,
        };
        self.adapter_generation_bindings.insert(
            adapter_generation,
            LocalTransportGeneration {
                logical_generation,
                load_attempt,
                adapter_generation,
            },
        );
        self.highest_bound_adapter_generation = Some(
            self.highest_bound_adapter_generation
                .map_or(adapter_generation, |highest| {
                    highest.max(adapter_generation)
                }),
        );
        Some(logical_generation)
    }

    fn map_observation_time(
        &self,
        update: &PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> (f64, Option<f64>) {
        let raw_seconds = update
            .observed_at
            .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
        match raw_seconds {
            Some(raw_seconds) => {
                let offset = self
                    .adapter_clock_offset_seconds
                    .unwrap_or(external_now_seconds - raw_seconds);
                (raw_seconds + offset, Some(offset))
            }
            None => (self.coordinator_now(external_now_seconds), None),
        }
    }

    fn observation_timestamp_is_accepted(
        &self,
        media_generation: u64,
        observed_at_seconds: f64,
    ) -> bool {
        observed_at_seconds.is_finite()
            && self.latest_observation.as_ref().is_none_or(|current| {
                current.media_generation != media_generation
                    || observed_at_seconds >= current.observed_at_seconds
            })
    }

    fn commit_observation_clock(
        &mut self,
        external_now_seconds: f64,
        observed_at_seconds: f64,
        candidate_offset_seconds: Option<f64>,
    ) {
        if self.adapter_clock_offset_seconds.is_none() {
            self.adapter_clock_offset_seconds = candidate_offset_seconds;
        }
        self.last_external_now_seconds = Some(
            self.last_external_now_seconds
                .map_or(external_now_seconds, |current| {
                    current.max(external_now_seconds)
                }),
        );
        self.last_coordinator_now_seconds = Some(
            self.last_coordinator_now_seconds
                .map_or(observed_at_seconds, |current| {
                    current.max(observed_at_seconds)
                }),
        );
    }

    pub(crate) fn observe_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        // Receiving an update establishes adapter capability even when the
        // event itself is stale or cannot be bound to the active load. Once
        // known, reconnect validation must never fall back to direct player
        // correction merely because the current transport is between samples.
        self.transport_telemetry_available = true;
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
        let (observed_at_seconds, candidate_offset_seconds) =
            self.map_observation_time(&update, external_now_seconds);
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
        let timestamp_accepted =
            self.observation_timestamp_is_accepted(media_generation, observed_at_seconds);
        if owns_current_media && timestamp_accepted {
            self.commit_observation_clock(
                external_now_seconds,
                observed_at_seconds,
                candidate_offset_seconds,
            );
            self.transport_telemetry_observed = true;
            self.merge_latest_observation(observation.clone());
        }
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
        let Some(current) = self.latest_observation.as_mut() else {
            self.latest_observation = Some(newer);
            return;
        };
        if current.media_generation != newer.media_generation {
            if self.coordinator.current_media_generation() == Some(newer.media_generation) {
                self.latest_observation = Some(newer);
            }
            return;
        }
        if newer.observed_at_seconds < current.observed_at_seconds {
            return;
        }
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
        if session.playback_barrier_status()?.phase != PlaybackBarrierPhase::Preparing {
            return None;
        }
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
            && observation.seeking != Some(true);
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
        let commit = session.playback_barrier_active_commit()?;
        if prepare.media_generation != commit.media_generation
            || self.coordinator.current_media_generation() != Some(local_media_generation)
            || !self.current_logical_media_matches(&prepare.logical_media_id)
        {
            return None;
        }
        Some((commit.media_generation, commit.state_revision))
    }

    fn capture_barrier_timeout_action(&mut self, session: &ClientSession) {
        let Some(operation) = self.initiated_barrier.as_ref() else {
            return;
        };
        if self.coordinator.current_media_generation() != Some(operation.local_media_generation) {
            return;
        }
        let Some(prepare) = session.playback_barrier_prepare().filter(|prepare| {
            prepare.request_nonce == operation.request_nonce
                && prepare.request_id.as_deref() == Some(operation.request_id.as_str())
        }) else {
            return;
        };
        let Some(status) = session.playback_barrier_status().filter(|status| {
            status.media_generation == prepare.media_generation
                && status.participants.values().any(|participant| {
                    participant.phase == PlaybackBarrierParticipantPhase::PrepareTimedOut
                })
        }) else {
            return;
        };
        let identity = (status.media_generation, status.state_revision);
        if self.handled_barrier_timeout == Some(identity) {
            return;
        }
        self.handled_barrier_timeout = Some(identity);
        let action = prepare.timeout_action.unwrap_or_default();
        if action != PlaybackBarrierTimeoutAction::Continue {
            self.pending_barrier_timeout_action = Some(action);
        }
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
            report_epoch: session.playback_barrier_buffering_report_epoch(),
            media_generation: policy.media_generation,
            state_revision: policy.state_revision,
            buffering,
            buffered_seconds: observation.buffered_ahead_seconds,
            observed_at: Some(observation.observed_at_seconds),
        })
    }

    fn should_report_room_buffering(
        &self,
        report_epoch: u64,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) -> bool {
        self.last_reported_room_buffering
            != Some((report_epoch, media_generation, state_revision, buffering))
    }

    fn mark_room_buffering_reported(
        &mut self,
        report_epoch: u64,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
    ) {
        self.last_reported_room_buffering =
            Some((report_epoch, media_generation, state_revision, buffering));
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

    pub(crate) fn stage_local_pause_intent(&mut self, paused: bool, session: &ClientSession) {
        let Some(room) = session.room() else {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            return;
        };
        let Some(local_media_generation) = self.coordinator.current_media_generation() else {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            return;
        };
        let controlled_room = controlled_room_name(room);
        let authorization =
            if !controlled_room || self.current_connection_local_control_is_authorized(session) {
                LocalIntentAuthorization::Authorized
            } else {
                match self.local_control_authority_freshness(session) {
                    Some(LocalControlAuthorityFreshness::Denied) => {
                        // Fresh, correlated denial is conclusive for this
                        // connection. Do not retain a command that could replay
                        // after an unrelated authority transition.
                        self.pending_local_pause_intent = None;
                        self.last_local_pause_intent_stage_accepted = Some(false);
                        return;
                    }
                    Some(LocalControlAuthorityFreshness::Awaiting) => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None if self.local_control_authority.is_some() => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None if session.local_can_control().is_none() => {
                        LocalIntentAuthorization::AwaitingControlledRoomReauthentication
                    }
                    None => {
                        // A direct coordination caller with a known
                        // non-controller projection has no connection-scoped
                        // evidence channel, so reject immediately.
                        self.pending_local_pause_intent = None;
                        self.last_local_pause_intent_stage_accepted = Some(false);
                        return;
                    }
                    Some(LocalControlAuthorityFreshness::Authorized) => {
                        unreachable!("authorized authority was handled above")
                    }
                }
            };
        self.pending_local_pause_intent = Some(PendingLocalPauseIntent {
            paused,
            room: room.to_owned(),
            local_media_generation,
            connection_generation: self.connection_generation,
            authorization,
            replay_player_after_reauthorization: authorization
                == LocalIntentAuthorization::AwaitingControlledRoomReauthentication,
        });
        self.last_local_pause_intent_stage_accepted = Some(true);
    }

    pub(crate) fn rollback_local_pause_intent(&mut self, paused: bool) {
        if self
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.paused == paused)
        {
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
        }
    }

    pub(crate) fn observe_local_control_authority(
        &mut self,
        session: &ClientSession,
        room: Option<&str>,
        username: Option<&str>,
        authorized: bool,
    ) {
        let target_room = room.or_else(|| session.room());
        let target_username = username.or_else(|| session.username());
        if target_room != session.room() || target_username != session.username() {
            return;
        }

        let Some(target_room) = target_room else {
            return;
        };
        let authority_allows_control = !controlled_room_name(target_room) || authorized;
        self.local_control_authority = Some(ConnectionLocalControlAuthority {
            room: target_room.to_owned(),
            username: target_username.map(str::to_owned),
            connection_generation: self.connection_generation,
            freshness: if authority_allows_control {
                LocalControlAuthorityFreshness::Authorized
            } else {
                LocalControlAuthorityFreshness::Denied
            },
        });

        let pending_context_matches =
            self.pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    target_room == intent.room
                        && self.coordinator.current_media_generation()
                            == Some(intent.local_media_generation)
                });
        if !pending_context_matches {
            return;
        }
        if authority_allows_control {
            let intent = self
                .pending_local_pause_intent
                .as_mut()
                .expect("matching pause intent must still exist");
            intent.connection_generation = self.connection_generation;
            intent.authorization = LocalIntentAuthorization::Authorized;
        } else {
            self.pending_local_pause_intent = None;
        }
    }

    pub(crate) fn update_desired_from_session(
        &mut self,
        session: &ClientSession,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.update_desired_from_session_with_replay(session, external_now_seconds, true)
    }

    fn update_desired_from_session_with_replay(
        &mut self,
        session: &ClientSession,
        external_now_seconds: f64,
        allow_command_replay: bool,
    ) -> Vec<PlaybackCoordinatorAction> {
        let Some(media_generation) = self.coordinator.current_media_generation() else {
            return Vec::new();
        };
        let Some(raw) = session.current_room_playstate().cloned() else {
            return Vec::new();
        };
        let Some(authority) = session.current_room_playstate_authority() else {
            return Vec::new();
        };
        let canonical_local_echo = authority == RoomPlaystateAuthority::LegacyLocalEcho;
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
            .and_then(|prepare| {
                let status = session.playback_barrier_status()?;
                if status.media_generation != prepare.media_generation {
                    return None;
                }
                match status.phase {
                    PlaybackBarrierPhase::Preparing => {
                        Some((prepare.media_generation, prepare.target_position, None))
                    }
                    PlaybackBarrierPhase::Committed => {
                        let commit = session.playback_barrier_active_commit()?;
                        Some((
                            prepare.media_generation,
                            prepare.target_position,
                            Some((commit.state_revision, commit.anchor_position)),
                        ))
                    }
                    PlaybackBarrierPhase::AwaitingDecision
                    | PlaybackBarrierPhase::Complete
                    | PlaybackBarrierPhase::Degraded => None,
                }
            });
        self.capture_barrier_timeout_action(session);
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
        let (buffering_media_generation, buffering_state_revision) = match authority {
            RoomPlaystateAuthority::ServerBufferingPolicy { media_generation } => session
                .playback_barrier_buffering_status()
                .filter(|status| status.config.media_generation == media_generation)
                .map_or((Some(media_generation), None), |status| {
                    (Some(media_generation), status.config.state_revision)
                }),
            RoomPlaystateAuthority::LegacyRemoteUser
            | RoomPlaystateAuthority::LegacyLocalEcho
            | RoomPlaystateAuthority::ServerBarrier { .. } => (None, None),
        };
        let authority_may_accept_local_intent = match authority {
            RoomPlaystateAuthority::LegacyRemoteUser | RoomPlaystateAuthority::LegacyLocalEcho => {
                true
            }
            RoomPlaystateAuthority::ServerBarrier {
                media_generation, ..
            } => {
                session.playback_barrier_status().is_some_and(|status| {
                    status.media_generation == media_generation
                        && status.phase == PlaybackBarrierPhase::AwaitingDecision
                }) && session.local_can_control().unwrap_or(false)
            }
            RoomPlaystateAuthority::ServerBufferingPolicy { .. } => false,
        };
        let mut local_intent_active = false;
        let mut local_intent_requires_player_replay = false;
        let intent_context_matches =
            self.pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    session.room() == Some(intent.room.as_str())
                        && intent.local_media_generation == media_generation
                });
        if self.pending_local_pause_intent.is_some() && !intent_context_matches {
            self.pending_local_pause_intent = None;
        }
        if canonical_local_echo
            && self
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.paused == paused)
        {
            // A matching canonical self-echo retires the command even if its
            // originating connection is now dormant.
            self.pending_local_pause_intent = None;
        } else if authority_may_accept_local_intent {
            let controlled_room = session.room().is_some_and(controlled_room_name);
            let local_control_authority = session.local_can_control();
            let connection_authority_allows_overlay =
                !controlled_room || self.current_connection_local_control_is_authorized(session);
            let may_overlay = self
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    intent.connection_generation == self.connection_generation
                        && intent.authorization == LocalIntentAuthorization::Authorized
                        && connection_authority_allows_overlay
                });
            if controlled_room
                && local_control_authority == Some(false)
                && self
                    .pending_local_pause_intent
                    .as_ref()
                    .is_some_and(|intent| {
                        intent.authorization == LocalIntentAuthorization::Authorized
                    })
            {
                // A known non-controller cannot retain a command for a later
                // authority transition. A reconnect-dormant intent waits for
                // fresh correlated auth/user evidence instead of trusting a
                // provisional List projection.
                self.pending_local_pause_intent = None;
            } else if may_overlay
                && let Some((local_paused, replay_player)) = self
                    .pending_local_pause_intent
                    .as_ref()
                    .map(|intent| (intent.paused, intent.replay_player_after_reauthorization))
                && paused != local_paused
            {
                // Keep an authorized command authoritative across the
                // transport observation that can arrive before its canonical
                // server echo. Dormant reconnect intent never reaches here.
                paused = local_paused;
                local_intent_active = true;
                local_intent_requires_player_replay = replay_player;
            }
        } else {
            // Preparing/Committed start synchronization and room buffering
            // remain server-owned. AwaitingDecision is deliberately excluded:
            // a controller's ordinary play/pause command resolves it.
            self.pending_local_pause_intent = None;
        }
        let local_echo =
            canonical_local_echo || (local_intent_active && !local_intent_requires_player_replay);
        if !position_seconds.is_finite() {
            return Vec::new();
        }

        let fingerprint = RoomDesiredFingerprint {
            paused,
            position_seconds: raw.position.unwrap_or(position_seconds),
            do_seek: raw.do_seek == Some(true),
            local_echo,
            barrier_media_generation,
            barrier_state_revision,
            buffering_media_generation,
            buffering_state_revision,
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
        let authority_changed = self.desired_fingerprint.as_ref().is_none_or(|previous| {
            previous.local_echo != fingerprint.local_echo
                || previous.barrier_media_generation != fingerprint.barrier_media_generation
                || previous.barrier_state_revision != fingerprint.barrier_state_revision
                || previous.buffering_media_generation != fingerprint.buffering_media_generation
                || previous.buffering_state_revision != fingerprint.buffering_state_revision
        });
        let barrier_became_authoritative = authority_changed
            && (barrier_media_generation.is_some()
                || barrier_state_revision.is_some()
                || buffering_media_generation.is_some()
                || buffering_state_revision.is_some());
        let reconnect_forced_revision = self
            .reconnect_reconciliation
            .is_some_and(|reconciliation| reconciliation.target_revision.is_none());
        let desired_changed = first_for_generation
            || pause_changed
            || paused_position_changed
            || explicit_seek_changed
            || authority_changed
            || reconnect_forced_revision;
        if desired_changed {
            self.desired_revision = self.desired_revision.saturating_add(1).max(1);
            let authoritative_pause_alignment = !local_echo && pause_changed && paused;
            let forced_reconciliation = self.reconnect_reconciliation.is_some()
                || (!local_echo
                    && (first_for_generation
                        || explicit_seek_changed
                        || barrier_became_authoritative
                        || authoritative_pause_alignment));
            self.pending_forced_seek_revision =
                forced_reconciliation.then_some(self.desired_revision);
            if let Some(reconciliation) = self.reconnect_reconciliation.as_mut() {
                reconciliation.target_revision = Some(self.desired_revision);
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
        let replay_desired =
            allow_command_replay && (!local_echo || self.reconnect_reconciliation.is_some());
        if replay_desired
            && desired_changed
            && let Some(observation) = self.latest_observation.clone()
        {
            actions.extend(self.coordinator.observe(observation));
        }
        if replay_desired {
            actions.extend(self.coordinator.tick(coordinator_now));
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

fn controlled_room_name(room_name: &str) -> bool {
    if !room_name.starts_with('+') {
        return false;
    }
    let Some((_, hash)) = room_name.rsplit_once(':') else {
        return false;
    };
    hash.len() == 12 && hash.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn normalized_positive_seconds(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn new_playback_barrier_request_id(local_media_generation: u64) -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::getrandom(&mut bytes).is_ok() {
        return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    }

    let unix_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "fallback-{:x}-{unix_nanos:x}-{local_media_generation:x}",
        std::process::id()
    )
}

fn seconds_to_milliseconds(seconds: f64) -> u64 {
    (seconds * 1_000.0).round().clamp(1.0, u64::MAX as f64) as u64
}

fn playback_barrier_retry_delay_seconds(retry_after_ms: u64, retry_attempt: u32) -> f64 {
    let requested_seconds = (retry_after_ms as f64 / 1_000.0).clamp(
        PLAYBACK_BARRIER_RETRY_MIN_SECONDS,
        PLAYBACK_BARRIER_RETRY_MAX_SECONDS,
    );
    let exponent = retry_attempt
        .saturating_sub(1)
        .min(PLAYBACK_BARRIER_RETRY_MAX_BACKOFF_EXPONENT);
    (requested_seconds * 2_f64.powi(exponent as i32)).min(PLAYBACK_BARRIER_RETRY_MAX_SECONDS)
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub(crate) fn emit_pending_playback_barrier_request_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let Some(request) = self
            .playback_coordination
            .playback_barrier_set_for_pending_media(&self.session, now_seconds)
        else {
            return Ok(());
        };
        self.control.activate_protocol_connection_generation();
        let scope = PlaybackBarrierRequestScope::new(
            request.room.clone(),
            request.local_media_generation,
            request.request_nonce,
        );
        self.control
            .emit(ClientEffect::send_playback_barrier_set(
                request.extension.clone(),
                scope,
            ))
            .map_err(client_effect_player_error)?;
        self.playback_coordination
            .confirm_playback_barrier_request_queued(&request);
        Ok(())
    }

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
        self.prepare_playback_media_with_intent(
            logical_id,
            kind,
            MediaLoadIntent::NewPlayback,
            now_seconds,
        )
    }

    pub fn prepare_playback_media_with_intent(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let plan = self.playback_coordination.prepare_media_with_intent(
            logical_id,
            kind,
            intent,
            now_seconds,
        );
        if let Some(room) = self.session.room() {
            self.control
                .retain_protocol_playback_barrier_scope(room, plan.media_generation);
        } else {
            self.control.cancel_protocol_playback_barrier_requests();
        }
        if let Some(request) = self
            .playback_coordination
            .playback_barrier_set_for_new_media(&plan, &self.session, now_seconds)
        {
            self.control.activate_protocol_connection_generation();
            let scope = PlaybackBarrierRequestScope::new(
                request.room.clone(),
                request.local_media_generation,
                request.request_nonce,
            );
            if self
                .control
                .emit(ClientEffect::send_playback_barrier_set(
                    request.extension.clone(),
                    scope,
                ))
                .is_ok()
            {
                self.playback_coordination
                    .confirm_playback_barrier_request_queued(&request);
            }
        }
        plan
    }

    pub fn playback_coordination_snapshot(&self) -> PlaybackCoordinationSnapshot {
        self.playback_coordination.snapshot()
    }

    /// Interrupts a recovery episode for an externally owned player (the GUI
    /// attached-player path) and returns every cleanup command to that owner.
    /// The internal runtime player must not consume these commands because GUI
    /// sessions intentionally use a no-op adapter there.
    pub fn interrupt_external_playback_recovery(&mut self) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination.interrupt_recovery()
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

    /// Stages a user-owned pause/play command before the external player can
    /// synchronously publish its resulting transport observation. The staged
    /// intent overlays ordinary legacy room state until the matching server
    /// echo arrives, but server barrier and room-buffering authority still
    /// preempt it.
    pub fn stage_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .stage_local_pause_intent(paused, &self.session);
        let actions = self
            .playback_coordination
            .update_desired_from_session_with_replay(&self.session, now_seconds, false);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
    }

    /// Rolls back a staged pause/play intent when the external player rejects
    /// the command, restoring canonical room authority immediately.
    pub fn rollback_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .rollback_local_pause_intent(paused);
        let actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        actions
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

    pub(crate) fn interrupt_playback_recovery(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let actions = self.playback_coordination.interrupt_recovery();
        self.execute_playback_coordinator_actions(actions, now_seconds)
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
            CoordinatorPlayerCommand::Play(intent) => PlayerCommand::Play(intent),
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
                observation.report_epoch,
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
                observation.report_epoch,
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
        DisconnectedPlayer, PlayerAdapter, PlayerCapabilities, PlayerCapability, PlayerCommand,
        PlayerCommandId, PlayerError, PlayerMediaGeneration, PlayerObservationTimestamp,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
    };
    use sorotte_protocol::{
        CommitStartPayload, PlaybackBarrierParticipantStatus, PlaybackBarrierPhase,
        PlaybackBarrierStatusPayload, ProtocolMessage, RoomBufferingPhase,
        RoomBufferingStatusPayload, SetPayload,
    };
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

    fn controlled_session_with_authority(controller: bool) -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
            )
            .expect("controlled-room hello should apply");
        let controller_update = if controller {
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#
        } else {
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":false}}}}"#
        };
        session
            .apply_message_json(controller_update)
            .expect("local controller projection should apply");
        session
    }

    fn controlled_barrier_session() -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
            )
            .expect("controlled barrier-aware hello should apply");
        assert_eq!(session.local_can_control(), Some(false));
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

    fn paused_transport(
        adapter_generation: u64,
        observed_at_seconds: f64,
        phase: PlayerTransportPhase,
        position_seconds: f64,
    ) -> PlayerTransportTelemetryUpdate {
        let mut update = transport(
            adapter_generation,
            observed_at_seconds,
            phase,
            position_seconds,
        );
        update.logical_pause = Some(true);
        update.core_idle = Some(true);
        update
    }

    fn barrier_status(
        media_generation: u64,
        state_revision: Option<u64>,
        phase: PlaybackBarrierPhase,
    ) -> PlaybackBarrierStatusPayload {
        PlaybackBarrierStatusPayload {
            media_generation,
            state_revision,
            phase,
            policy: PlaybackBarrierPolicy::Controller,
            quorum: None,
            deadline: 100.0,
            participants: BTreeMap::new(),
            excluded_legacy_clients: BTreeSet::new(),
        }
    }

    fn has_pause_play_or_seek(actions: &[PlaybackCoordinatorAction]) -> bool {
        actions.iter().any(|action| {
            matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(_)
                        | CoordinatorPlayerCommand::Play(_)
                        | CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            )
        })
    }

    #[derive(Default)]
    struct CoordinatedTestPlayer {
        transport_updates: VecDeque<PlayerTransportTelemetryUpdate>,
        commands: Vec<PlayerCommand>,
        next_command_id: u64,
        advertises_telemetry: bool,
        reject_rate_commands: bool,
    }

    impl PlayerAdapter for CoordinatedTestPlayer {
        fn name(&self) -> &'static str {
            "coordinated-test-player"
        }

        fn capabilities(&self) -> PlayerCapabilities {
            if self.advertises_telemetry {
                PlayerCapabilities::from_capabilities([PlayerCapability::Telemetry])
            } else {
                PlayerCapabilities::NONE
            }
        }

        fn execute(&mut self, command: PlayerCommand) -> Result<(), PlayerError> {
            if self.reject_rate_commands && matches!(command, PlayerCommand::SetPlaybackRate(_)) {
                return Err(PlayerError::OperationFailed(
                    "test player rejected rate cleanup".to_owned(),
                ));
            }
            self.commands.push(command);
            Ok(())
        }

        fn execute_tracked(
            &mut self,
            command: PlayerCommand,
        ) -> Result<PlayerCommandId, PlayerError> {
            if self.reject_rate_commands && matches!(command, PlayerCommand::SetPlaybackRate(_)) {
                return Err(PlayerError::OperationFailed(
                    "test player rejected tracked rate cleanup".to_owned(),
                ));
            }
            self.commands.push(command);
            self.next_command_id = self.next_command_id.saturating_add(1).max(1);
            Ok(PlayerCommandId::new(self.next_command_id))
        }

        fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
            self.transport_updates.pop_front()
        }
    }

    fn reconnect_runtime(
        room_paused: bool,
        room_position: f64,
        local_paused: bool,
        local_position: f64,
    ) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session
            .apply_message_json_at(
                &format!(
                    r#"{{"State":{{"playstate":{{"position":{room_position},"paused":{room_paused},"doSeek":false,"setBy":"bob"}}}}}}"#
                ),
                10.0,
            )
            .unwrap();
        session.model.playback.local_paused = Some(local_paused);
        session.model.playback.local_position = Some(local_position);
        session.model.reconnect.state_restore_validation_pending = true;

        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.prepare_playback_media(
            LogicalMediaId::new("reconnect-media").unwrap(),
            MediaTransportKind::NetworkVod,
            9.0,
        );
        runtime
    }

    fn controlled_runtime_after_reconnect_without_fresh_authority(
        logical_media_id: &str,
    ) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
        let mut session = controlled_session_with_authority(true);
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination.prepare_media(
            LogicalMediaId::new(logical_media_id).unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        runtime.reconcile_external_player_playback(0.0);
        runtime.observe_external_player_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );

        runtime.begin_protocol_connection_generation();
        runtime.session_mut().reset_sync_state_for_reconnect();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
            )
            .unwrap();
        assert_eq!(
            runtime.session().local_can_control(),
            Some(true),
            "Hello restores the cached controller projection, which is deliberately not fresh authority"
        );
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                1.05,
            )
            .unwrap();
        let reconciliation = runtime.reconcile_external_player_playback(1.05);
        for action in reconciliation {
            if let PlaybackCoordinatorAction::Execute { command_id, .. } = action {
                runtime.report_external_coordinator_command_dispatch(command_id, Ok(()), 1.05);
            }
        }
        runtime.observe_external_player_transport(
            paused_transport(1, 1.075, PlayerTransportPhase::ReadyPaused, 10.0),
            1.075,
        );
        runtime
    }

    fn coordination_with_owned_catchup_rate() -> RuntimePlaybackCoordination {
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("catchup-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        let mut session = ClientSession::default();
        session.model.connection.username = Some("alice".to_owned());
        session.model.room.name = Some("room".to_owned());
        session.model.room.playstates.insert(
            "room".to_owned(),
            RoomPlaystateView {
                paused: Some(false),
                position: Some(0.0),
                set_by: Some("bob".to_owned()),
                ..RoomPlaystateView::default()
            },
        );
        session
            .model
            .room
            .playstate_updated_at_seconds
            .insert("room".to_owned(), 0.0);
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
        coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);
        coordination.observe_transport(
            transport(1, 10.0, PlayerTransportPhase::Rebuffering, 8.0),
            10.0,
        );
        coordination
            .observe_transport(transport(1, 11.0, PlayerTransportPhase::Playing, 9.0), 11.0);
        coordination.observe_transport(
            transport(1, 12.0, PlayerTransportPhase::Playing, 10.0),
            12.0,
        );
        let mut catchup = transport(1, 13.0, PlayerTransportPhase::Playing, 11.0);
        catchup.playback_rate = Some(1.03);
        coordination.observe_transport(catchup, 13.0);
        assert!(coordination.snapshot().recovery_episode.is_some());
        coordination
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
        let refreshed = runtime.prepare_playback_media_with_intent(
            logical_id,
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::TransportRefresh,
            101.0,
        );

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
        assert_eq!(prepare.media_generation, 0);
        assert!(prepare.request_nonce > 0);
        assert_eq!(prepare.load_intent, MediaLoadIntent::NewPlayback);
        assert_eq!(prepare.quorum_percent, Some(60));
        assert_eq!(prepare.timeout_ms, Some(12_000));
        let buffering = extension
            .buffering_policy
            .expect("ongoing buffering policy should be present");
        assert_eq!(buffering.media_generation, prepare.media_generation);
        assert_eq!(buffering.request_nonce, prepare.request_nonce);
        assert_eq!(buffering.load_intent, prepare.load_intent);
        assert_eq!(buffering.policy, RoomBufferingPolicy::Quorum);
        assert_eq!(buffering.quorum_percent, Some(70));
        assert_eq!(buffering.max_pause_ms, Some(20_000));
    }

    #[test]
    fn client_runtime_debug_redacts_playback_request_and_logical_media_identities() {
        const LOGICAL_MEDIA_MARKER: &str = "private-runtime-logical-media-canary";

        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new(LOGICAL_MEDIA_MARKER).unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request) = &runtime.control().outbound_messages()[0] else {
            panic!("coordination request should use Set");
        };
        let request_id = request
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .and_then(|extension| extension.prepare)
            .and_then(|prepare| prepare.request_id)
            .expect("current clients should attach an opaque request identity");

        let debug = format!("{runtime:?}");
        assert!(!debug.contains(LOGICAL_MEDIA_MARKER));
        assert!(!debug.contains(&request_id));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn startup_media_waits_for_controlled_room_auth_then_emits_once() {
        let mut runtime = ClientRuntime::new(
            controlled_barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
            policy: RoomBufferingPolicy::PauseController,
            ..PlaybackBarrierRoomBufferingConfig::default()
        });

        runtime.prepare_playback_media(
            LogicalMediaId::new("startup-controlled-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "media coordination must wait while controller authentication is pending"
        );

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller authentication should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("authenticated coordination should emit");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("repeated notification pump should be idempotent");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
            panic!("authenticated coordination should use a Set envelope");
        };
        let extension = set
            .set
            .playback_barrier_v1()
            .expect("barrier extension should decode")
            .expect("barrier extension should be present");
        assert!(extension.prepare.is_some());
        assert_eq!(
            extension
                .buffering_policy
                .as_ref()
                .map(|policy| policy.policy),
            Some(RoomBufferingPolicy::PauseController)
        );
    }

    #[test]
    fn failed_controlled_room_auth_does_not_emit_media_coordination() {
        let mut runtime = ClientRuntime::new(
            controlled_barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("startup-controlled-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller authentication failure should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("failed auth notification should dispatch");

        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn transport_write_does_not_acknowledge_start_and_reconnect_queries_server() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
            policy: RoomBufferingPolicy::PauseAnyEligible,
            ..PlaybackBarrierRoomBufferingConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("reconnect-current-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(initial_set) = &runtime.control().outbound_messages()[0] else {
            panic!("initial coordination should use a Set envelope");
        };
        let initial_prepare = initial_set
            .set
            .playback_barrier_v1()
            .expect("initial extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("initial request should start a barrier");
        let initial_nonce = initial_prepare.request_nonce;
        let request_id = initial_prepare
            .request_id
            .expect("new clients attach a stable operation id");
        runtime.flush_queued_protocol_messages();
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_some()
        );
        assert!(runtime.playback_coordination.accepted_barrier.is_none());

        runtime.begin_protocol_connection_generation();
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("uncertain start should query after reconnect");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("recovery query should be emitted exactly once");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
            panic!("recovery query should use a Set envelope");
        };
        let extension = recovery_set
            .set
            .playback_barrier_v1()
            .expect("recovery extension should decode")
            .expect("recovery extension should be present");
        assert!(extension.prepare.is_none());
        assert!(extension.buffering_policy.is_none());
        let recovery = extension
            .recovery
            .expect("recovery query should be present");
        assert_eq!(recovery.request_id, request_id);
        assert_eq!(recovery.original_request_nonce, initial_nonce);
        assert!(recovery.recovery_nonce > initial_nonce);
        assert_eq!(recovery.disposition, None);
    }

    #[test]
    fn absent_recovery_result_rebuilds_exactly_one_current_media_start() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("undelivered-current-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(initial_set) = &runtime.control().outbound_messages()[0] else {
            panic!("initial coordination should use a Set envelope");
        };
        let initial_prepare = initial_set
            .set
            .playback_barrier_v1()
            .expect("initial extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("initial request should start a barrier");
        let initial_nonce = initial_prepare.request_nonce;
        let request_id = initial_prepare
            .request_id
            .expect("request id should be present");

        runtime.begin_protocol_connection_generation();
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "the old serialized request must be dropped on reconnect"
        );
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("the current semantic intent should query the server");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
            panic!("recovery query should use a Set envelope");
        };
        let recovery = recovery_set
            .set
            .playback_barrier_v1()
            .expect("recovery extension should decode")
            .and_then(|extension| extension.recovery)
            .expect("recovery query should be present");
        let recovery_nonce = recovery.recovery_nonce;
        runtime.flush_queued_protocol_messages();

        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new().with_recovery(
                        PlaybackBarrierRecoveryPayload::result(
                            request_id.clone(),
                            initial_nonce,
                            recovery_nonce,
                            "undelivered-current-media",
                            PlaybackBarrierRecoveryDisposition::Absent,
                        ),
                    ),
                ),
            ))
            .expect("explicit absence should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("absence should authorize one fresh start");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("fresh start should not duplicate");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(rebuilt_set) = &runtime.control().outbound_messages()[0] else {
            panic!("rebuilt coordination should use a Set envelope");
        };
        let extension = rebuilt_set
            .set
            .playback_barrier_v1()
            .expect("rebuilt extension should decode")
            .expect("rebuilt extension should be present");
        let prepare = extension
            .prepare
            .expect("absence must rebuild the start intent");
        assert_eq!(prepare.load_intent, MediaLoadIntent::NewPlayback);
        assert_eq!(prepare.request_nonce, initial_nonce);
        assert_eq!(prepare.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(
            extension
                .buffering_policy
                .as_ref()
                .map(|policy| policy.request_nonce),
            Some(prepare.request_nonce)
        );
    }

    #[test]
    fn matching_canonical_response_is_the_application_ack_boundary() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("canonical-ack-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("request should use Set");
        };
        let request = request_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .expect("request extension should exist");
        runtime.flush_queued_protocol_messages();
        assert!(runtime.playback_coordination.accepted_barrier.is_none());
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_some()
        );

        let mut canonical_prepare = request.prepare.expect("start should be requested");
        canonical_prepare.media_generation = 7;
        let mut canonical_policy = request
            .buffering_policy
            .expect("buffering policy should be requested");
        canonical_policy.media_generation = 7;
        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new()
                        .with_prepare(canonical_prepare)
                        .with_buffering_policy(canonical_policy),
                ),
            ))
            .expect("canonical response should apply");

        assert!(runtime.playback_coordination.accepted_barrier.is_some());
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_none()
        );
    }

    #[test]
    fn matching_retry_later_retains_exact_operation_and_emits_once_when_due() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("retry-later-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(initial_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("initial request should use Set");
        };
        let initial = initial_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("initial request should contain prepare");
        let request_id = initial.request_id.expect("request id should be present");
        let request_nonce = initial.request_nonce;

        runtime
            .session_mut()
            .apply_protocol_message_at(
                ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new().with_request_result(
                        sorotte_protocol::PlaybackBarrierRequestResultPayload::retry_later(
                            request_id.clone(),
                            request_nonce,
                            1_000,
                        ),
                    ),
                )),
                10.0,
            )
            .expect("retry result should apply without terminating the session");

        assert!(runtime.session().is_active());
        assert!(runtime.control().outbound_messages().is_empty());
        assert!(runtime.playback_coordination.initiated_barrier.is_none());
        let pending = runtime
            .playback_coordination
            .pending_media_coordination
            .as_ref()
            .expect("semantic media intent must remain pending");
        assert_eq!(pending.request_id, request_id);
        assert_eq!(pending.retry_request_nonce, Some(request_nonce));
        assert_eq!(pending.retry_attempts, 1);
        assert_eq!(
            runtime.pending_playback_barrier_retry_delay_at(10.0),
            Some(1.0)
        );

        runtime
            .run_pending_playback_barrier_retry_at(10.999)
            .expect("early retry pump should be inert");
        assert!(runtime.control().outbound_messages().is_empty());
        runtime
            .run_pending_playback_barrier_retry_at(11.0)
            .expect("due retry should emit");
        runtime
            .run_pending_playback_barrier_retry_at(12.0)
            .expect("repeated retry pump should be idempotent");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(retry_set) = &runtime.control().outbound_messages()[0] else {
            panic!("retry should use Set");
        };
        let retry = retry_set
            .set
            .playback_barrier_v1()
            .expect("retry extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("retry should retain the start request");
        assert_eq!(retry.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(retry.request_nonce, request_nonce);
        assert_eq!(retry.logical_media_id, "retry-later-media");
    }

    #[test]
    fn mismatched_retry_later_result_cannot_rearm_current_operation() {
        for (request_id_matches, request_nonce_matches) in
            [(false, true), (true, false), (false, false)]
        {
            let mut runtime = ClientRuntime::new(
                barrier_session(),
                DisconnectedPlayer,
                QueuedRuntimeControl::default(),
            );
            runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
                policy: Some(PlaybackBarrierPolicy::Controller),
                ..PlaybackBarrierStartConfig::default()
            });
            runtime.prepare_playback_media(
                LogicalMediaId::new("mismatched-retry-media").unwrap(),
                MediaTransportKind::NetworkVod,
                1.0,
            );
            let ProtocolMessage::Set(initial_set) =
                runtime.control().outbound_messages()[0].clone()
            else {
                panic!("initial request should use Set");
            };
            let initial = initial_set
                .set
                .playback_barrier_v1()
                .expect("request extension should decode")
                .and_then(|extension| extension.prepare)
                .expect("initial request should contain prepare");
            let request_id = initial.request_id.expect("request id should be present");
            let request_nonce = initial.request_nonce;
            let response_id = if request_id_matches {
                request_id.clone()
            } else {
                "another-operation".to_owned()
            };
            let response_nonce = if request_nonce_matches {
                request_nonce
            } else {
                request_nonce.saturating_add(1)
            };

            runtime
                .session_mut()
                .apply_protocol_message_at(
                    ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
                        PlaybackBarrierSetExtension::new().with_request_result(
                            sorotte_protocol::PlaybackBarrierRequestResultPayload::retry_later(
                                response_id,
                                response_nonce,
                                1_000,
                            ),
                        ),
                    )),
                    10.0,
                )
                .expect("mismatched result is syntactically valid");

            assert!(runtime.session().is_active());
            assert_eq!(runtime.control().outbound_messages().len(), 1);
            assert!(runtime.playback_coordination.initiated_barrier.is_some());
            assert_eq!(runtime.pending_playback_barrier_retry_delay_at(10.0), None);
            let pending = runtime
                .playback_coordination
                .pending_media_coordination
                .as_ref()
                .expect("mismatched result must preserve the original pending intent");
            assert_eq!(pending.request_id, request_id);
            assert_eq!(pending.retry_request_nonce, None);
            assert_eq!(pending.retry_attempts, 0);
        }
    }

    #[test]
    fn repeated_retry_later_results_apply_capped_exponential_backoff() {
        assert_eq!(playback_barrier_retry_delay_seconds(1_000, 1), 1.0);
        assert_eq!(playback_barrier_retry_delay_seconds(1_000, 2), 2.0);
        assert_eq!(playback_barrier_retry_delay_seconds(1_000, 6), 30.0);
        assert_eq!(playback_barrier_retry_delay_seconds(u64::MAX, 1), 30.0);
        assert_eq!(playback_barrier_retry_delay_seconds(0, 1), 0.1);
    }

    #[test]
    fn terminal_accepted_reconnect_recovers_before_emitting_any_fresh_request() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("terminal-recovery-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("request should use Set");
        };
        let request = request_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .expect("request extension should exist");
        let mut canonical_prepare = request.prepare.expect("start should be requested");
        canonical_prepare.media_generation = 7;
        let mut canonical_policy = request
            .buffering_policy
            .expect("buffering policy should be requested");
        canonical_policy.media_generation = 7;
        let request_id = canonical_prepare
            .request_id
            .clone()
            .expect("request id should be present");
        let request_nonce = canonical_prepare.request_nonce;
        runtime.flush_queued_protocol_messages();
        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new()
                        .with_prepare(canonical_prepare.clone())
                        .with_status(barrier_status(7, None, PlaybackBarrierPhase::Degraded))
                        .with_buffering_policy(canonical_policy.clone()),
                ),
            ))
            .expect("terminal canonical lifecycle should apply");
        assert!(runtime.playback_coordination.accepted_barrier_terminal);

        runtime.begin_protocol_connection_generation();
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("terminal lifecycle should query recovery first");
        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
            panic!("recovery should use Set");
        };
        let recovery = recovery_set
            .set
            .playback_barrier_v1()
            .expect("recovery extension should decode")
            .expect("recovery extension should exist");
        assert!(recovery.prepare.is_none());
        assert!(recovery.buffering_policy.is_none());
        let recovery = recovery.recovery.expect("recovery query should be present");
        assert_eq!(recovery.request_id, request_id);
        assert_eq!(recovery.original_request_nonce, request_nonce);
        runtime.flush_queued_protocol_messages();

        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new()
                        .with_prepare(canonical_prepare)
                        .with_status(barrier_status(7, None, PlaybackBarrierPhase::Degraded))
                        .with_buffering_policy(canonical_policy)
                        .with_recovery(
                            PlaybackBarrierRecoveryPayload::result(
                                request_id,
                                request_nonce,
                                recovery.recovery_nonce,
                                "terminal-recovery-media",
                                PlaybackBarrierRecoveryDisposition::Recovered,
                            )
                            .with_media_generation(7),
                        ),
                ),
            ))
            .expect("recovered terminal lifecycle should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("recovered terminal lifecycle should not emit fresh intent");
        assert!(runtime.control().outbound_messages().is_empty());
        assert!(runtime.playback_coordination.accepted_barrier.is_some());
        assert!(
            runtime
                .playback_coordination
                .pending_barrier_recovery
                .is_none()
        );
        assert!(runtime.playback_coordination.accepted_barrier_terminal);
    }

    #[test]
    fn absent_terminal_recovery_emits_only_one_fresh_policy_refresh() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("absent-terminal-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("request should use Set");
        };
        let request = request_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .expect("request extension should exist");
        let mut canonical_prepare = request.prepare.expect("start should be requested");
        canonical_prepare.media_generation = 11;
        let mut canonical_policy = request
            .buffering_policy
            .expect("buffering policy should be requested");
        canonical_policy.media_generation = 11;
        let request_id = canonical_prepare
            .request_id
            .clone()
            .expect("request id should be present");
        let request_nonce = canonical_prepare.request_nonce;
        runtime.flush_queued_protocol_messages();
        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new()
                        .with_prepare(canonical_prepare)
                        .with_status(barrier_status(11, None, PlaybackBarrierPhase::Degraded))
                        .with_buffering_policy(canonical_policy),
                ),
            ))
            .expect("terminal canonical lifecycle should apply");
        assert!(runtime.playback_coordination.accepted_barrier_terminal);

        runtime.begin_protocol_connection_generation();
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("terminal lifecycle should query recovery");
        let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
            panic!("recovery should use Set");
        };
        let recovery = recovery_set
            .set
            .playback_barrier_v1()
            .expect("recovery extension should decode")
            .and_then(|extension| extension.recovery)
            .expect("recovery query should be present");
        runtime.flush_queued_protocol_messages();

        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new().with_recovery(
                        PlaybackBarrierRecoveryPayload::result(
                            request_id,
                            request_nonce,
                            recovery.recovery_nonce,
                            "absent-terminal-media",
                            PlaybackBarrierRecoveryDisposition::Absent,
                        ),
                    ),
                ),
            ))
            .expect("explicit terminal absence should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("terminal absence should emit a policy refresh");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("policy refresh should remain exactly once");

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(refresh_set) = &runtime.control().outbound_messages()[0] else {
            panic!("policy refresh should use Set");
        };
        let refresh = refresh_set
            .set
            .playback_barrier_v1()
            .expect("policy refresh extension should decode")
            .expect("policy refresh extension should exist");
        assert!(refresh.prepare.is_none());
        assert!(refresh.recovery.is_none());
        let policy = refresh
            .buffering_policy
            .expect("policy refresh should be present");
        assert_eq!(policy.media_generation, 0);
        assert_eq!(policy.load_intent, MediaLoadIntent::TransportRefresh);
        assert!(policy.request_nonce > recovery.recovery_nonce);
    }

    #[test]
    fn mismatched_operation_id_cannot_acknowledge_same_nonce() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("nonce-collision-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("request should use Set");
        };
        let mut prepare = request_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("prepare should exist");
        prepare.media_generation = 9;
        prepare.request_id = Some("another-controller-operation".to_owned());
        runtime.flush_queued_protocol_messages();
        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new().with_prepare(prepare),
                ),
            ))
            .expect("peer canonical response should still apply to the session");

        assert!(runtime.playback_coordination.accepted_barrier.is_none());
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_some()
        );
    }

    #[test]
    fn matching_identity_on_rejected_zero_generation_echo_is_not_an_application_ack() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("invalid-canonical-echo").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(request) = runtime.control().outbound_messages()[0].clone() else {
            panic!("request should use Set");
        };
        runtime.flush_queued_protocol_messages();
        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::Set(request))
            .expect("request-shaped echo is syntactically valid");

        assert!(runtime.session().playback_barrier_prepare().is_none());
        assert!(runtime.playback_coordination.accepted_barrier.is_none());
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_some()
        );
    }

    #[test]
    fn queued_room_a_start_is_discarded_atomically_on_room_switch() {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("room-switch-queued-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        assert!(runtime.playback_coordination.initiated_barrier.is_some());

        assert!(
            runtime
                .run_set_room("room2")
                .expect("room switch should queue")
        );
        assert!(runtime.control().outbound_messages().iter().all(|message| {
            let ProtocolMessage::Set(set) = message else {
                return true;
            };
            set.set
                .playback_barrier_v1()
                .expect("extension should decode")
                .is_none()
        }));
        assert!(runtime.playback_coordination.initiated_barrier.is_none());
        assert!(runtime.playback_coordination.accepted_barrier.is_none());
        assert!(
            runtime
                .playback_coordination
                .pending_barrier_recovery
                .is_none()
        );
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_none()
        );

        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("authoritative destination room should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("room B should not resurrect the cancelled intent");
        runtime.begin_protocol_connection_generation();
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("reconnect should not resurrect room A");
        assert!(runtime.control().outbound_messages().iter().all(|message| {
            let ProtocolMessage::Set(set) = message else {
                return true;
            };
            set.set
                .playback_barrier_v1()
                .expect("extension should decode")
                .is_none()
        }));
    }

    #[test]
    fn new_controlled_room_reidentify_preserves_unserialized_pre_auth_media() {
        let mut runtime = ClientRuntime::new(
            controlled_barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("new-controlled-room-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        assert!(runtime.control().outbound_messages().is_empty());

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+other:ZYXWVU654321","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room should apply");
        runtime
            .run_controller_reidentify_if_needed()
            .expect("reidentify actions should dispatch");
        assert!(
            runtime
                .playback_coordination
                .pending_media_coordination
                .is_some()
        );

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+other:ZYXWVU654321","success":true}}}"#,
            )
            .expect("destination authentication should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("authenticated destination should emit media coordination");

        let barrier_requests = runtime
            .control()
            .outbound_messages()
            .iter()
            .filter(|message| {
                let ProtocolMessage::Set(set) = message else {
                    return false;
                };
                set.set
                    .playback_barrier_v1()
                    .ok()
                    .flatten()
                    .is_some_and(|extension| extension.prepare.is_some())
            })
            .count();
        assert_eq!(barrier_requests, 1);
    }

    #[test]
    fn auth_delayed_media_intent_rebinds_to_destination_controlled_room() {
        let mut runtime = ClientRuntime::new(
            controlled_barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("room-switch-current-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        assert!(runtime.control().outbound_messages().is_empty());

        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"room":{"name":"+other:ZYXWVU654321"}}}"#)
            .expect("authoritative room switch should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("unauthenticated destination must not emit");
        assert!(runtime.control().outbound_messages().is_empty());

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+other:ZYXWVU654321","success":true}}}"#,
            )
            .expect("destination controller authentication should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("current media should coordinate in the authenticated destination");
        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
            panic!("destination coordination should use a Set envelope");
        };
        assert!(
            set.set
                .playback_barrier_v1()
                .expect("destination extension should decode")
                .and_then(|extension| extension.prepare)
                .is_some()
        );
    }

    #[test]
    fn explicit_replay_can_supersede_same_logical_active_barrier() {
        let logical_id = "active-replay-media";
        let mut session = barrier_session();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(7, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                        .with_request_nonce(3),
                )
                .with_status(PlaybackBarrierStatusPayload {
                    media_generation: 7,
                    state_revision: None,
                    phase: PlaybackBarrierPhase::Preparing,
                    policy: PlaybackBarrierPolicy::Controller,
                    quorum: None,
                    deadline: 20.0,
                    participants: BTreeMap::new(),
                    excluded_legacy_clients: BTreeSet::new(),
                }),
        );
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.set_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        coordination.prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        let plan = coordination.prepare_media_with_intent(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::Replay,
            10.0,
        );

        let request = coordination
            .playback_barrier_set_for_new_media(&plan, &session, 10.0)
            .expect("explicit replay should not be mistaken for peer participation");
        assert_eq!(
            request.extension.prepare.map(|prepare| prepare.load_intent),
            Some(MediaLoadIntent::Replay)
        );
    }

    #[test]
    fn fresh_controller_infers_replay_from_retained_terminal_logical_identity() {
        let logical_id = "media-sha256:opaque-id";
        let mut session = barrier_session();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(9, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                        .with_request_nonce(4),
                )
                .with_commit(CommitStartPayload::new(9, 3, 0.0, 50.0, 55.0))
                .with_status(PlaybackBarrierStatusPayload {
                    media_generation: 9,
                    state_revision: Some(3),
                    phase: PlaybackBarrierPhase::Complete,
                    policy: PlaybackBarrierPolicy::Controller,
                    quorum: None,
                    deadline: 55.0,
                    participants: BTreeMap::new(),
                    excluded_legacy_clients: BTreeSet::new(),
                }),
        );

        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.set_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        let plan = coordination.prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::NetworkVod,
            60.0,
        );
        assert_eq!(plan.load_intent, MediaLoadIntent::NewPlayback);

        let extension = coordination
            .playback_barrier_set_for_new_media(&plan, &session, 60.0)
            .expect("fresh controller should publish an explicit replay request");
        assert_eq!(
            extension
                .extension
                .prepare
                .map(|prepare| prepare.load_intent),
            Some(MediaLoadIntent::Replay)
        );
        assert_eq!(
            extension
                .extension
                .buffering_policy
                .map(|policy| policy.load_intent),
            Some(MediaLoadIntent::Replay)
        );
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
        let buffering_policy =
            RoomBufferingPolicyPayload::new(10, RoomBufferingPolicy::PauseAnyEligible)
                .with_debounce_ms(750)
                .with_resume_hysteresis_ms(1_500)
                .with_max_pause_ms(30_000);
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(
                        10,
                        logical_id,
                        0.0,
                        PlaybackBarrierPolicy::Controller,
                    )
                    .with_request_nonce(10),
                )
                .with_status(PlaybackBarrierStatusPayload {
                    media_generation: 10,
                    state_revision: None,
                    phase: PlaybackBarrierPhase::Preparing,
                    policy: PlaybackBarrierPolicy::Controller,
                    quorum: None,
                    deadline: 120.0,
                    participants: BTreeMap::new(),
                    excluded_legacy_clients: BTreeSet::new(),
                })
                .with_buffering_policy(buffering_policy.clone()),
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

        runtime
            .session_mut()
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new()
                        .with_buffering_policy(buffering_policy.clone())
                        .with_buffering_status(RoomBufferingStatusPayload {
                            config: buffering_policy,
                            phase: RoomBufferingPhase::Monitoring,
                            eligible_clients: 2,
                            required_buffering_clients: 1,
                            buffering_clients: BTreeSet::new(),
                            pause_deadline: None,
                        }),
                ),
            ))
            .expect("authoritative same-policy snapshot should apply");
        runtime.reconcile_external_player_playback(102.1);
        let rearmed = runtime
            .control
            .drain_outbound_message_lines()
            .expect("the snapshot should rearm one current-state report");
        assert!(
            rearmed
                .iter()
                .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":true"))
        );
        runtime.reconcile_external_player_playback(102.2);
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "the snapshot epoch must not cause unchanged per-pump report spam"
        );

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
    fn server_enforced_prepare_timeout_action_is_captured_once_for_controller_ui() {
        let mut session = barrier_session();
        let mut participants = BTreeMap::new();
        participants.insert(
            "alice-client".to_owned(),
            PlaybackBarrierParticipantStatus {
                phase: PlaybackBarrierParticipantPhase::PrepareTimedOut,
                readiness: None,
                observed_position: None,
                degraded_reason: None,
            },
        );
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(
                        22,
                        "media-sha256:opaque-id",
                        0.0,
                        PlaybackBarrierPolicy::AllEligible,
                    )
                    .with_request_nonce(22)
                    .with_request_id("timeout-request")
                    .with_timeout_action(PlaybackBarrierTimeoutAction::RemainPaused),
                )
                .with_status(PlaybackBarrierStatusPayload {
                    media_generation: 22,
                    state_revision: None,
                    phase: PlaybackBarrierPhase::AwaitingDecision,
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
        coordination.initiated_barrier = Some(PlaybackBarrierOperation {
            local_media_generation: plan.media_generation,
            load_intent: MediaLoadIntent::NewPlayback,
            include_start_barrier: true,
            request_id: "timeout-request".to_owned(),
            request_nonce: 22,
            logical_media_id: "media-sha256:opaque-id".to_owned(),
            room: "room1".to_owned(),
        });
        coordination.capture_barrier_timeout_action(&session);
        assert_eq!(
            coordination.pending_barrier_timeout_action.take(),
            Some(PlaybackBarrierTimeoutAction::RemainPaused)
        );
        coordination.capture_barrier_timeout_action(&session);
        assert_eq!(coordination.pending_barrier_timeout_action, None);
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
            runtime.adapter_generation_bindings.get(&11),
            Some(&LocalTransportGeneration {
                logical_generation: initial.media_generation,
                load_attempt: refreshed.load_attempt,
                adapter_generation: 11,
            })
        );
        assert!(
            !runtime.adapter_generation_bindings.contains_key(&10),
            "refreshing the same logical source must invalidate the prior load attempt"
        );
    }

    #[test]
    fn delayed_previous_load_attempt_cannot_replace_current_transport_observation() {
        let mut runtime = RuntimePlaybackCoordination::default();
        let logical_id = LogicalMediaId::new("plex://server/item/42").unwrap();
        let initial =
            runtime.prepare_media(logical_id.clone(), MediaTransportKind::NetworkVod, 0.0);
        runtime.observe_transport(transport(10, 1.0, PlayerTransportPhase::Playing, 8.0), 1.0);

        let refreshed = runtime.prepare_media(logical_id, MediaTransportKind::NetworkVod, 2.0);
        assert_eq!(refreshed.media_generation, initial.media_generation);
        assert_eq!(refreshed.load_attempt, initial.load_attempt + 1);

        let stale_before_new_binding =
            runtime.observe_transport(transport(10, 3.0, PlayerTransportPhase::Failed, 99.0), 3.0);
        assert!(stale_before_new_binding.is_empty());
        assert!(runtime.latest_observation.is_none());
        assert!(!runtime.snapshot().transport_telemetry_observed);

        runtime.observe_transport(
            transport(11, 4.0, PlayerTransportPhase::ReadyPaused, 12.0),
            4.0,
        );
        let accepted = runtime.latest_observation.clone().unwrap();
        assert_eq!(accepted.phase, Some(PlayerTransportPhase::ReadyPaused));
        assert_eq!(accepted.position_seconds, Some(12.0));

        let stale_after_new_binding =
            runtime.observe_transport(transport(10, 5.0, PlayerTransportPhase::Ended, 100.0), 5.0);
        assert!(stale_after_new_binding.is_empty());
        assert_eq!(runtime.latest_observation, Some(accepted));
    }

    #[test]
    fn older_same_generation_observation_is_ignored_in_runtime_latest_state() {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        runtime.observe_transport(
            transport(1, 10.0, PlayerTransportPhase::Playing, 20.0),
            10.0,
        );
        let latest = runtime.latest_observation.clone().unwrap();

        runtime.observe_transport(
            transport(1, 9.0, PlayerTransportPhase::Rebuffering, 5.0),
            11.0,
        );

        assert_eq!(runtime.latest_observation, Some(latest));
        assert_eq!(runtime.snapshot().metrics.stale_timestamp_observations, 1);
        assert_eq!(runtime.last_external_now_seconds, Some(10.0));
        assert_eq!(runtime.last_coordinator_now_seconds, Some(10.0));
        assert_eq!(
            runtime.coordinator_now(12.0),
            12.0,
            "a rejected stale sample must not move the external/coordinator clock anchor backwards"
        );
    }

    #[test]
    fn reconnect_correction_waits_through_loading_cache_pause_and_seek() {
        for phase in [
            PlayerTransportPhase::Loading,
            PlayerTransportPhase::Rebuffering,
            PlayerTransportPhase::Seeking,
        ] {
            let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
            runtime
                .player_mut_for_test()
                .transport_updates
                .push_back(paused_transport(1, 1.0, phase, 0.0));

            runtime
                .run_reconnect_state_restore_validation_if_needed_at(10.0)
                .unwrap();

            assert!(
                runtime.player().commands.is_empty(),
                "{phase:?} must suppress reconnect correction commands"
            );
            assert!(
                runtime
                    .session()
                    .model
                    .reconnect
                    .state_restore_validation_pending,
                "{phase:?} must remain pending until a safe observation"
            );
        }
    }

    #[test]
    fn extractor_backed_transport_refresh_keeps_reconnect_coordinator_owned_without_a_fresh_sample()
    {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime.observe_external_player_transport(
            paused_transport(1, 1.0, PlayerTransportPhase::ReadyPaused, 0.0),
            10.0,
        );
        // Extractor-backed and expiring Plex URLs are NetworkVod transport
        // refreshes: their playback URL changes while logical media identity
        // and the coordinator generation remain stable.
        runtime.prepare_playback_media_with_intent(
            LogicalMediaId::new("reconnect-media").unwrap(),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::TransportRefresh,
            10.5,
        );
        runtime.player_mut_for_test().commands.clear();

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.5)
            .unwrap();

        assert!(runtime.player().commands.is_empty());
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "a known telemetry-capable adapter must not fall back to direct correction during reload"
        );
    }

    #[test]
    fn advertised_telemetry_capability_prevents_direct_reconnect_fallback_before_first_sample() {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime.player_mut_for_test().advertises_telemetry = true;

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();

        assert!(runtime.player().commands.is_empty());
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
    }

    #[test]
    fn telemetry_capability_without_a_media_transaction_retains_legacy_reconnect_correction() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":100.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                10.0,
            )
            .unwrap();
        session.model.playback.local_paused = Some(true);
        session.model.playback.local_position = Some(0.0);
        session.model.reconnect.state_restore_validation_pending = true;
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer {
                advertises_telemetry: true,
                ..CoordinatedTestPlayer::default()
            },
            QueuedRuntimeControl::default(),
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();

        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
        )));
        assert!(
            !runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "capability alone cannot select a coordinator with no media identity"
        );
    }

    #[test]
    fn accepted_reconnect_command_needs_matching_transport_observation() {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                0.0,
            ));

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();
        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
        )));
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
        assert_eq!(
            runtime
                .reconnect_state_restore_correction_metrics()
                .correction_actions_succeeded,
            0
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.5)
            .unwrap();
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "dispatch acceptance without observation must not complete reconnect validation"
        );

        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                2.0,
                PlayerTransportPhase::ReadyPaused,
                100.0,
            ));
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(11.0)
            .unwrap();
        assert!(
            !runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
        assert_eq!(
            runtime
                .reconnect_state_restore_correction_metrics()
                .correction_actions_succeeded,
            1
        );
    }

    #[test]
    fn reconnect_reconciliation_corrects_self_attributed_room_state() {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime
            .session_mut_for_test()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":100.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                10.0,
            )
            .unwrap();
        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                0.0,
            ));

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();
        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
        )));
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "self attribution must suppress ordinary echo replay, not reconnect reconciliation"
        );

        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                2.0,
                PlayerTransportPhase::ReadyPaused,
                100.0,
            ));
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(11.0)
            .unwrap();
        assert!(
            !runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
    }

    #[test]
    fn newer_room_state_supersedes_stale_reconnect_seek() {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                0.0,
            ));
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();

        runtime
            .session_mut_for_test()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":200.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                10.5,
            )
            .unwrap();
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.5)
            .unwrap();
        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 200.0).abs() < f64::EPSILON
        )));

        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                2.0,
                PlayerTransportPhase::ReadyPaused,
                100.0,
            ));
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(11.0)
            .unwrap();
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "an observation matching only the superseded seek must not complete validation"
        );

        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(
                1,
                3.0,
                PlayerTransportPhase::ReadyPaused,
                200.0,
            ));
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(12.0)
            .unwrap();
        assert!(
            !runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
    }

    #[test]
    fn reconnect_without_transport_telemetry_retains_legacy_direct_correction() {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();

        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
        )));
        assert!(
            !runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending
        );
    }

    #[test]
    fn manual_seek_interrupts_recovery_and_resets_owned_rate_first() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        assert!(runtime.run_seek_to_position(20.0).unwrap());

        assert!(matches!(
            runtime.player().commands.as_slice(),
            [PlayerCommand::SetPlaybackRate(rate), PlayerCommand::SetPosition(position)]
                if (*rate - 1.0).abs() < f64::EPSILON
                    && (*position - 20.0).abs() < f64::EPSILON
        ));
        assert!(
            runtime
                .playback_coordination
                .snapshot()
                .recovery_episode
                .is_none()
        );
    }

    #[test]
    fn local_pause_interrupts_recovery_and_resets_owned_rate_first() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        assert!(runtime.run_set_paused(true).unwrap());
        assert!(matches!(
            runtime.player().commands.as_slice(),
            [PlayerCommand::SetPlaybackRate(rate), PlayerCommand::SetPaused(true)]
                if (*rate - 1.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn rejected_rate_cleanup_does_not_swallow_the_users_manual_seek() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer {
                reject_rate_commands: true,
                ..CoordinatedTestPlayer::default()
            },
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        assert!(runtime.run_seek_to_position(20.0).unwrap());
        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPosition(position) if (*position - 20.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn rejected_rate_cleanup_does_not_swallow_the_users_pause() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer {
                reject_rate_commands: true,
                ..CoordinatedTestPlayer::default()
            },
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        assert!(runtime.run_set_paused(true).unwrap());
        assert!(
            runtime
                .player()
                .commands
                .iter()
                .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
        );
    }

    #[test]
    fn disconnect_resets_owned_rate_and_preserves_pause_on_leave() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        runtime.run_disconnect(20.0).unwrap();

        assert!(runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPlaybackRate(rate) if (*rate - 1.0).abs() < f64::EPSILON
        )));
        assert!(
            runtime
                .player()
                .commands
                .iter()
                .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
        );
        assert!(
            runtime
                .playback_coordination
                .snapshot()
                .recovery_episode
                .is_none()
        );
    }

    #[test]
    fn rejected_rate_cleanup_does_not_abort_disconnect() {
        let coordination = coordination_with_owned_catchup_rate();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .unwrap();
        session.model.playback.local_paused = Some(false);
        session.model.playback.local_position = Some(11.0);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer {
                reject_rate_commands: true,
                ..CoordinatedTestPlayer::default()
            },
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination = coordination;

        runtime.run_disconnect(20.0).unwrap();
        assert!(
            runtime
                .player()
                .commands
                .iter()
                .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
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
        assert_eq!(runtime.snapshot().metrics.stale_generation_observations, 0);
        assert_eq!(runtime.last_external_now_seconds, Some(1.0));
        assert_eq!(runtime.last_coordinator_now_seconds, Some(1.0));
        assert_eq!(
            runtime.coordinator_now(4.0),
            4.0,
            "an observation from the superseded media generation must not replace clock anchors"
        );
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
    fn unattributed_room_state_without_peers_does_not_become_coordinator_desire() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":3.0,"paused":true,"doSeek":false}}}"#,
                3.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("unattributed-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );

        assert!(
            coordination
                .update_desired_from_session(&session, 3.0)
                .is_empty()
        );
        assert!(coordination.desired_fingerprint.is_none());
        assert_eq!(coordination.coordinator.desired_revision_pending(), None);
    }

    #[test]
    fn local_pause_and_unpause_echoes_become_desired_without_command_replay() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("local-echo-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
        coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);

        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                1.0,
            )
            .unwrap();
        let pause_echo = coordination.update_desired_from_session(&session, 1.0);
        assert!(!has_pause_play_or_seek(&pause_echo));
        assert!(
            coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| desired.paused)
        );
        assert!(
            coordination.snapshot().ordinary_correction_blocked,
            "the coordinator owns the local desired revision until telemetry confirms it"
        );
        let paused = coordination.observe_transport(
            paused_transport(1, 1.1, PlayerTransportPhase::ReadyPaused, 1.0),
            1.1,
        );
        assert!(!has_pause_play_or_seek(&paused));
        assert!(!coordination.snapshot().ordinary_correction_blocked);

        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                2.0,
            )
            .unwrap();
        let unpause_echo = coordination.update_desired_from_session(&session, 2.0);
        assert!(!has_pause_play_or_seek(&unpause_echo));
        assert!(
            coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| !desired.paused)
        );
        let playing = coordination
            .observe_transport(transport(1, 2.1, PlayerTransportPhase::Playing, 1.2), 2.1);
        assert!(!has_pause_play_or_seek(&playing));
        assert!(!coordination.snapshot().ordinary_correction_blocked);
    }

    #[test]
    fn staged_local_unpause_survives_transport_observation_before_canonical_echo() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("staged-local-unpause").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );

        coordination.stage_local_pause_intent(false, &session);
        let staged = coordination.update_desired_from_session_with_replay(&session, 0.1, false);
        assert!(!has_pause_play_or_seek(&staged));
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false)
        );
        assert!(
            coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| !desired.paused && desired.local_echo)
        );

        let observed_play = coordination
            .observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);
        assert!(
            !observed_play.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(true),
                    ..
                }
            )),
            "the pre-echo playing observation must not replay the stale canonical pause"
        );

        let extra_pre_echo_pump = coordination.update_desired_from_session(&session, 0.2);
        assert!(
            !extra_pre_echo_pump.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(true),
                    ..
                }
            )),
            "an extra reconciliation pump must retain the staged unpause"
        );
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false)
        );

        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                0.3,
            )
            .unwrap();
        let canonical_echo = coordination.update_desired_from_session(&session, 0.3);
        assert!(!has_pause_play_or_seek(&canonical_echo));
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            None,
            "the matching canonical echo must retire the staged intent"
        );

        let continued_play = coordination
            .observe_transport(transport(1, 0.4, PlayerTransportPhase::Playing, 10.25), 0.4);
        assert!(!has_pause_play_or_seek(&continued_play));
        assert!(
            !coordination.snapshot().ordinary_correction_blocked,
            "advancing playback after the echo must complete coordinator ownership"
        );
    }

    #[test]
    fn staged_controller_decision_survives_awaiting_decision_but_not_committed_start() {
        let logical_id = "phase-aware-local-intent";
        let mut awaiting_session = barrier_session();
        awaiting_session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":4.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                0.0,
            )
            .unwrap();
        apply_barrier_extension(
            &mut awaiting_session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(PrepareMediaPayload::new(
                    21,
                    logical_id,
                    4.0,
                    PlaybackBarrierPolicy::Controller,
                ))
                .with_status(barrier_status(
                    21,
                    None,
                    PlaybackBarrierPhase::AwaitingDecision,
                )),
        );
        let mut awaiting = RuntimePlaybackCoordination::default();
        awaiting.prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        awaiting.update_desired_from_session(&awaiting_session, 0.0);
        awaiting.stage_local_pause_intent(false, &awaiting_session);
        let controller_decision =
            awaiting.update_desired_from_session_with_replay(&awaiting_session, 0.1, false);
        assert!(!has_pause_play_or_seek(&controller_decision));
        assert_eq!(awaiting.snapshot().pending_local_pause_intent, Some(false));
        assert!(
            awaiting
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| !desired.paused && desired.local_echo),
            "a room controller must be able to resolve AwaitingDecision with ordinary play"
        );

        let mut committed_session = barrier_session();
        committed_session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                0.0,
            )
            .unwrap();
        apply_barrier_extension(
            &mut committed_session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(
                        22,
                        logical_id,
                        4.0,
                        PlaybackBarrierPolicy::Controller,
                    )
                    .with_request_nonce(5),
                )
                .with_status(barrier_status(22, None, PlaybackBarrierPhase::Preparing)),
        );
        apply_barrier_extension(
            &mut committed_session,
            PlaybackBarrierSetExtension::new()
                .with_commit(CommitStartPayload::new(22, 5, 4.0, 0.0, 10.0))
                .with_status(barrier_status(22, Some(5), PlaybackBarrierPhase::Committed)),
        );
        assert_eq!(
            committed_session
                .playback_barrier_status()
                .map(|status| status.phase),
            Some(PlaybackBarrierPhase::Committed)
        );
        assert!(matches!(
            committed_session.current_room_playstate_authority(),
            Some(RoomPlaystateAuthority::ServerBarrier {
                media_generation: 22,
                ..
            })
        ));
        let mut committed = RuntimePlaybackCoordination::default();
        committed.prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        committed.update_desired_from_session(&committed_session, 0.0);
        committed.stage_local_pause_intent(true, &committed_session);
        let committed_reconciliation =
            committed.update_desired_from_session_with_replay(&committed_session, 0.1, false);
        assert!(!has_pause_play_or_seek(&committed_reconciliation));
        assert_eq!(
            committed.snapshot().pending_local_pause_intent,
            None,
            "Committed remains server-owned until the client observes and acknowledges the synchronized start"
        );
        assert!(
            committed
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| !desired.paused),
            "a local pause must not suppress the canonical committed start before StartedAck"
        );
    }

    #[test]
    fn staged_current_media_unpause_survives_protocol_reconnect_until_matching_echo() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("reconnect-local-intent").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );
        coordination.stage_local_pause_intent(false, &session);
        coordination.update_desired_from_session_with_replay(&session, 0.1, false);

        // A protocol reconnect keeps the user's command for the same media;
        // adapter replacement and media preparation clear it separately.
        coordination.begin_protocol_connection_generation(&session);
        session.reset_sync_state_for_reconnect();
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false)
        );
        session
            .apply_message_json_at(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
            )
            .unwrap();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                1.1,
            )
            .unwrap();
        let stale_reconnect_state = coordination.update_desired_from_session(&session, 1.1);
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false)
        );
        assert!(!stale_reconnect_state.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));
        let pre_echo_play = coordination
            .observe_transport(transport(1, 1.2, PlayerTransportPhase::Playing, 10.1), 1.2);
        assert!(!pre_echo_play.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));

        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                1.3,
            )
            .unwrap();
        coordination.update_desired_from_session(&session, 1.3);
        assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    }

    #[test]
    fn controlled_room_reconnect_keeps_pause_intent_dormant_and_auth_failure_discards_it() {
        let mut session = controlled_session_with_authority(true);
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination.prepare_media(
            LogicalMediaId::new("controlled-auth-failure").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        runtime.reconcile_external_player_playback(0.0);
        runtime.observe_external_player_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );
        runtime.stage_external_player_pause_intent(false, 0.1);
        runtime.observe_external_player_transport(
            transport(1, 0.1, PlayerTransportPhase::Playing, 10.0),
            0.1,
        );

        runtime.begin_protocol_connection_generation();
        runtime.session_mut().reset_sync_state_for_reconnect();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
            )
            .unwrap();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"List":{"+room:ABCDEF123456":{"alice":{"controller":false}}}}"#,
                1.05,
            )
            .unwrap();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                1.1,
            )
            .unwrap();
        let dormant_reconciliation = runtime.reconcile_external_player_playback(1.1);
        assert!(dormant_reconciliation.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));
        assert_eq!(
            runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent,
            None,
            "dormant semantic intent must not be exposed as an active player override"
        );
        assert!(
            runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent_dormant,
            "the semantic command should remain dormant for correlated reauthentication"
        );
        assert_eq!(
            runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .map(|intent| intent.authorization),
            Some(LocalIntentAuthorization::AwaitingControlledRoomReauthentication)
        );

        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":false}}}"#,
                1.2,
            )
            .unwrap();
        assert_eq!(
            runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent,
            None,
            "a correlated authentication failure must permanently discard the stale command"
        );
        runtime.reconcile_external_player_playback(1.2);
        assert!(
            runtime
                .playback_coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| desired.paused && !desired.local_echo)
        );
    }

    #[test]
    fn controlled_room_reconnect_reauthorizes_same_media_intent_only_after_success() {
        let mut session = controlled_session_with_authority(true);
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.playback_coordination.prepare_media(
            LogicalMediaId::new("controlled-auth-success").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        runtime.reconcile_external_player_playback(0.0);
        runtime.observe_external_player_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );
        runtime.stage_external_player_pause_intent(false, 0.1);

        runtime.begin_protocol_connection_generation();
        runtime.session_mut().reset_sync_state_for_reconnect();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
            )
            .unwrap();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"List":{"+room:ABCDEF123456":{"alice":{"controller":false}}}}"#,
                1.05,
            )
            .unwrap();
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                1.1,
            )
            .unwrap();
        let dormant_reconciliation = runtime.reconcile_external_player_playback(1.1);
        assert!(
            runtime
                .playback_coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| desired.paused && !desired.local_echo)
        );
        for action in dormant_reconciliation {
            if let PlaybackCoordinatorAction::Execute { command_id, .. } = action {
                runtime.report_external_coordinator_command_dispatch(command_id, Ok(()), 1.1);
            }
        }
        runtime.observe_external_player_transport(
            paused_transport(1, 1.15, PlayerTransportPhase::ReadyPaused, 10.0),
            1.15,
        );

        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":true}}}"#,
                1.2,
            )
            .unwrap();
        let authorized_continuation = runtime.reconcile_external_player_playback(1.2);
        assert!(
            authorized_continuation.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(false)
                        | CoordinatorPlayerCommand::Play(_),
                    ..
                }
            )),
            "fresh authority should resume the dormant command: {authorized_continuation:?}"
        );
        let intent = runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .expect("successful reauthentication should safely continue the same intent");
        assert_eq!(intent.authorization, LocalIntentAuthorization::Authorized);
        assert_eq!(
            intent.connection_generation,
            runtime.playback_coordination.connection_generation
        );

        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                1.3,
            )
            .unwrap();
        runtime.reconcile_external_player_playback(1.3);
        assert_eq!(
            runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent,
            None
        );
    }

    #[test]
    fn controlled_room_new_toggle_after_reconnect_waits_for_fresh_authority() {
        let mut runtime =
            controlled_runtime_after_reconnect_without_fresh_authority("controlled-new-toggle");

        let dormant_actions = runtime.stage_external_player_pause_intent(false, 1.1);
        assert!(
            !dormant_actions.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(false)
                        | CoordinatorPlayerCommand::Play(_),
                    ..
                }
            )),
            "a cached controller projection must not authorize a new command on the replacement connection"
        );
        let dormant = runtime.playback_coordination_snapshot();
        assert_eq!(dormant.pending_local_pause_intent, None);
        assert!(dormant.pending_local_pause_intent_dormant);
        assert_eq!(dormant.last_local_pause_intent_stage_accepted, Some(true));
        assert!(
            runtime
                .playback_coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| desired.paused && !desired.local_echo)
        );

        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
                1.2,
            )
            .unwrap();
        let authorized_actions = runtime.reconcile_external_player_playback(1.2);
        assert_eq!(
            authorized_actions
                .iter()
                .filter(|action| matches!(
                    action,
                    PlaybackCoordinatorAction::Execute {
                        command: CoordinatorPlayerCommand::SetPaused(false)
                            | CoordinatorPlayerCommand::Play(_),
                        ..
                    }
                ))
                .count(),
            1,
            "fresh correlated Set.user authority should replay the dormant command exactly once: {authorized_actions:?}"
        );
        let authorized = runtime.playback_coordination_snapshot();
        assert_eq!(authorized.pending_local_pause_intent, Some(false));
        assert!(!authorized.pending_local_pause_intent_dormant);

        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                1.3,
            )
            .unwrap();
        runtime.reconcile_external_player_playback(1.3);
        let echoed = runtime.playback_coordination_snapshot();
        assert_eq!(echoed.pending_local_pause_intent, None);
        assert!(!echoed.pending_local_pause_intent_dormant);
    }

    #[test]
    fn controlled_room_fresh_denial_clears_and_rejects_new_reconnect_toggle() {
        let mut runtime = controlled_runtime_after_reconnect_without_fresh_authority(
            "controlled-new-toggle-denied",
        );

        runtime.stage_external_player_pause_intent(false, 1.1);
        assert!(
            runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent_dormant
        );
        runtime
            .session_mut()
            .apply_message_json_at(
                r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":false}}}"#,
                1.2,
            )
            .unwrap();
        let denied = runtime.playback_coordination_snapshot();
        assert_eq!(denied.pending_local_pause_intent, None);
        assert!(!denied.pending_local_pause_intent_dormant);

        let retry_actions = runtime.stage_external_player_pause_intent(false, 1.3);
        let retry = runtime.playback_coordination_snapshot();
        assert_eq!(retry.last_local_pause_intent_stage_accepted, Some(false));
        assert_eq!(retry.pending_local_pause_intent, None);
        assert!(!retry.pending_local_pause_intent_dormant);
        assert!(
            !retry_actions.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(false)
                        | CoordinatorPlayerCommand::Play(_),
                    ..
                }
            )),
            "fresh denial must remain conclusive for the current connection"
        );
    }

    #[test]
    fn uncontrolled_room_new_toggle_after_reconnect_remains_active() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("uncontrolled-new-toggle").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.begin_protocol_connection_generation(&session);
        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json_at(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
            )
            .unwrap();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                1.05,
            )
            .unwrap();

        coordination.stage_local_pause_intent(false, &session);
        coordination.update_desired_from_session_with_replay(&session, 1.1, false);
        let snapshot = coordination.snapshot();
        assert_eq!(snapshot.pending_local_pause_intent, Some(false));
        assert!(!snapshot.pending_local_pause_intent_dormant);
    }

    #[test]
    fn controlled_room_noncontroller_toggle_is_reconciled_to_canonical_pause() {
        let mut session = controlled_session_with_authority(false);
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("noncontroller-toggle").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
            0.0,
        );

        coordination.stage_local_pause_intent(false, &session);
        assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
        let correction = coordination
            .observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);
        assert!(correction.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));

        let mut playing_session = controlled_session_with_authority(false);
        playing_session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
                2.0,
            )
            .unwrap();
        let mut playing_coordination = RuntimePlaybackCoordination::default();
        playing_coordination.prepare_media(
            LogicalMediaId::new("noncontroller-pause-toggle").unwrap(),
            MediaTransportKind::LocalFile,
            2.0,
        );
        playing_coordination.update_desired_from_session(&playing_session, 2.0);
        playing_coordination
            .observe_transport(transport(1, 2.0, PlayerTransportPhase::Playing, 20.0), 2.0);
        playing_coordination.stage_local_pause_intent(true, &playing_session);
        assert_eq!(
            playing_coordination.snapshot().pending_local_pause_intent,
            None
        );
        let play_correction = playing_coordination.observe_transport(
            paused_transport(1, 2.1, PlayerTransportPhase::ReadyPaused, 20.1),
            2.1,
        );
        assert!(play_correction.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(false)
                    | CoordinatorPlayerCommand::Play(_),
                ..
            }
        )));
    }

    #[test]
    fn staged_pause_intent_is_scoped_to_room_and_local_media_generation() {
        let session = controlled_session_with_authority(true);
        let mut coordination = RuntimePlaybackCoordination::default();
        let first = coordination.prepare_media(
            LogicalMediaId::new("scoped-intent-one").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        coordination.stage_local_pause_intent(true, &session);
        let intent = coordination
            .pending_local_pause_intent
            .as_ref()
            .expect("controller command should be staged");
        assert_eq!(intent.room, "+room:ABCDEF123456");
        assert_eq!(intent.local_media_generation, first.media_generation);
        assert_eq!(
            intent.connection_generation,
            coordination.connection_generation
        );

        coordination.handle_authoritative_playback_barrier_room_change();
        assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
        coordination.stage_local_pause_intent(true, &session);
        coordination.prepare_media(
            LogicalMediaId::new("scoped-intent-two").unwrap(),
            MediaTransportKind::LocalFile,
            1.0,
        );
        assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    }

    #[test]
    fn complete_barrier_then_local_pause_retires_committed_play_desire() {
        let logical_id = "terminal-local-pause";
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
                0.0,
            )
            .unwrap();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(
                        12,
                        logical_id,
                        5.0,
                        PlaybackBarrierPolicy::Controller,
                    )
                    .with_request_nonce(8),
                )
                .with_commit(CommitStartPayload::new(12, 4, 5.0, 0.0, 10.0))
                .with_status(barrier_status(12, Some(4), PlaybackBarrierPhase::Committed)),
        );
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 5.0), 0.0);
        coordination.update_desired_from_session(&session, 0.0);
        assert_eq!(
            coordination.pending_forced_seek_revision,
            Some(coordination.desired_revision),
            "the committed barrier revision should still be awaiting advancement"
        );

        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new().with_status(barrier_status(
                12,
                Some(4),
                PlaybackBarrierPhase::Complete,
            )),
        );
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":5.3,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                1.0,
            )
            .unwrap();
        let terminal_pause = coordination.update_desired_from_session(&session, 1.0);

        assert!(!has_pause_play_or_seek(&terminal_pause));
        assert!(
            coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| {
                    desired.paused
                        && desired.barrier_media_generation.is_none()
                        && desired.barrier_state_revision.is_none()
                })
        );
        assert_eq!(coordination.pending_forced_seek_revision, None);
        let confirmed = coordination.observe_transport(
            paused_transport(1, 1.1, PlayerTransportPhase::ReadyPaused, 5.3),
            1.1,
        );
        assert!(!has_pause_play_or_seek(&confirmed));
        assert!(!coordination.snapshot().ordinary_correction_blocked);
    }

    #[test]
    fn buffering_status_supersedes_earlier_self_pause_echo_with_authoritative_alignment() {
        let mut session = barrier_session();
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":8.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
                0.0,
            )
            .unwrap();
        let mut coordination = RuntimePlaybackCoordination::default();
        coordination.prepare_media(
            LogicalMediaId::new("buffering-authority").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        coordination.update_desired_from_session(&session, 0.0);
        coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 8.0), 0.0);
        coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 8.2), 0.2);

        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
                1.0,
            )
            .unwrap();
        let self_echo = coordination.update_desired_from_session(&session, 1.0);
        assert!(!has_pause_play_or_seek(&self_echo));
        coordination.stage_local_pause_intent(false, &session);
        let staged_unpause =
            coordination.update_desired_from_session_with_replay(&session, 1.05, false);
        assert!(!has_pause_play_or_seek(&staged_unpause));
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false)
        );

        let config = RoomBufferingPolicyPayload::new(7, RoomBufferingPolicy::PauseAnyEligible)
            .with_state_revision(3)
            .with_debounce_ms(500)
            .with_resume_hysteresis_ms(1_000)
            .with_max_pause_ms(20_000);
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new().with_buffering_status(RoomBufferingStatusPayload {
                config,
                phase: RoomBufferingPhase::Paused,
                eligible_clients: 2,
                required_buffering_clients: 1,
                buffering_clients: BTreeSet::from(["bob".to_owned()]),
                pause_deadline: Some(21.0),
            }),
        );
        let authoritative = coordination.update_desired_from_session(&session, 1.1);
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            None,
            "server buffering authority must preempt a staged local unpause"
        );
        assert!(authoritative.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position),
                ..
            } if (*position - 10.0).abs() < f64::EPSILON
        )));
        assert!(
            coordination
                .desired_fingerprint
                .as_ref()
                .is_some_and(|desired| {
                    desired.buffering_media_generation == Some(7)
                        && desired.buffering_state_revision == Some(3)
                })
        );

        let aligned = coordination
            .observe_transport(transport(1, 1.2, PlayerTransportPhase::Playing, 10.0), 1.2);
        assert!(aligned.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )));
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
        session.model.connection.username = Some("alice".to_owned());
        session.model.room.name = Some("room".to_owned());
        session.model.room.playstates.insert(
            "room".to_owned(),
            RoomPlaystateView {
                paused: Some(false),
                position: Some(40.0),
                set_by: Some("bob".to_owned()),
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
