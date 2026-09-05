use barrier::RoomBarrierState;
use barrier::*;
use local_intent::*;
use participant_status::ParticipantStatusReportingState;
use participant_status::*;
mod barrier;
mod local_intent;
mod participant_status;
use super::*;

use crate::control::client_effect_player_error;

use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "test-support")]
use sorotte_player_api::{LifecycleVerificationAttemptProjection, LifecycleVerificationProjection};
use sorotte_player_api::{
    LoadAttemptId, PlayerActiveLoadSnapshot, PlayerAttachmentEpoch, PlayerCommand,
    PlayerCommandFailureKind, PlayerCommandId, PlayerCommandProgressState, PlayerCommandResult,
    PlayerCommandSemanticResult, PlayerEvent, PlayerEventAcknowledgementToken, PlayerEventBatch,
    PlayerEventDeliveryMode, PlayerEventOrder, PlayerLoadAttemptResult, PlayerMediaGeneration,
    PlayerObservationTimestamp, PlayerPlaybackTelemetryUpdate, PlayerSemanticOutcome,
    PlayerSequenceBoundary, PlayerTransportDelta, PlayerTransportSnapshot,
    PlayerTransportTelemetryUpdate, SequencedPlayerEvent, SequencedPlayerSemanticOutcome,
    SnapshotField,
};
pub use sorotte_protocol::PlaybackBarrierTimeoutAction;
use sorotte_protocol::{
    PlaybackBarrierParticipantPhase, PlaybackBarrierPhase, PlaybackBarrierPolicy,
    PlaybackBarrierRecoveryDisposition, PlaybackBarrierRecoveryPayload,
    PlaybackBarrierRequestResultStatus, PlaybackBarrierSetExtension, PrepareMediaPayload,
    RecoveryStage, RoomBufferingPolicy, RoomBufferingPolicyPayload, RoomPauseOwner,
    TechnicalBlockCause, TechnicalPlayabilityPhase, TechnicalReadinessReport,
};

use crate::player_transition::{
    NativePlayerAction, PlayerCommandCause, PlayerCommandCompletion, PlayerCommandRegistration,
    PlayerLogicalPauseObservation, PlayerTransitionClassification, PlayerTransitionClassifier,
    PlayerTransitionContext,
};

const PLAYBACK_BARRIER_RETRY_MIN_SECONDS: f64 = 0.1;
const PLAYBACK_BARRIER_RETRY_MAX_SECONDS: f64 = 30.0;
const PLAYBACK_BARRIER_RETRY_MAX_BACKOFF_EXPONENT: u32 = 5;
const MAX_MISMATCHING_LOCAL_PAUSE_INTENT_UPDATES: u8 = 3;
const LOCAL_PAUSE_INTENT_MISMATCH_WINDOW_SECONDS: f64 = 10.0;
const MAX_DESYNC_POSITION_SAMPLE_AGE_SECONDS: f64 = 2.0;

mod ordered_events;
pub(crate) use ordered_events::OrderedPlayerEventConsumer;

const PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS: f64 = 5.0;
const PARTICIPANT_STATUS_HEARTBEAT_SECONDS: f64 = 1.0;

/// Lifecycle truth supplied by an externally owned player integration.
///
/// `Connected` is intentionally absent: only a current-generation transport
/// observation can establish that state. An integration uses `Connecting`
/// when it has attached or restarted a telemetry-capable player,
/// `TelemetryUnavailable` when a player is usable without transport
/// telemetry, and `Unavailable`, `Disconnected`, or `Failed` when it has
/// direct lifecycle evidence for those terminal states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPlayerAvailability {
    Unavailable,
    Connecting,
    TelemetryUnavailable,
    Disconnected,
    Failed,
}

fn participant_status_legacy_position_fallback(
    external_player_availability: Option<ExternalPlayerAvailability>,
    transport_telemetry_ever_observed: bool,
) -> bool {
    !transport_telemetry_ever_observed
        && external_player_availability
            .is_none_or(|availability| availability == ExternalPlayerAvailability::Connecting)
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
    /// Compatibility view used by attached-player integrations. Dormant
    /// controlled-room intent is deliberately hidden until fresh authority
    /// for the current connection has been established.
    pub pending_local_pause_intent: Option<bool>,
    pub pending_local_pause_intent_dormant: bool,
    pub last_local_pause_intent_stage_accepted: Option<bool>,
    pub diagnostic: PlaybackDiagnostic,
    pub recovery_episode: Option<RecoveryEpisodeSnapshot>,
    pub seek_preparation: Option<SeekPreparationSnapshot>,
    pub last_seek_preparation_terminal_outcome: Option<SeekPreparationTerminalOutcome>,
    pub last_seek_preparation_terminal: Option<SeekPreparationSnapshot>,
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
struct PlayerCommandBinding {
    coordinator_command_id: CoordinatorCommandId,
    desired_paused: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TechnicalReadinessFingerprint {
    connection_generation: u64,
    membership_epoch: u64,
    media_generation: u64,
    authoritative_playback_revision: Option<u64>,
    phase: TechnicalPlayabilityPhase,
    reason: Option<TechnicalBlockCause>,
    recovery: Option<RecoveryStage>,
}

#[derive(Debug, Clone)]
struct CurrentTransportEvidence {
    adapter_generation: u64,
    observation: PlayerTransportObservation,
    position_observation: Option<LocalPositionObservation>,
    participant_status_evidence_times: ParticipantStatusEvidenceTimes,
    last_received_at_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum MappedTransportCommitOutcome {
    Committed,
    LifecycleFenced,
    #[default]
    Rejected,
}

#[derive(Debug, Default)]
pub(crate) struct RuntimePlaybackCoordination {
    barrier: RoomBarrierState,
    participant_status: ParticipantStatusReportingState,
    coordinator: PlaybackCoordinator,
    adapter_generation_bindings: BTreeMap<u64, LocalTransportGeneration>,
    pending_media_identity: Option<(u64, u64)>,
    highest_bound_adapter_generation: Option<u64>,
    adapter_epoch: u64,
    player_command_bindings: BTreeMap<PlayerCommandId, PlayerCommandBinding>,
    pending_coordinator_command_completion_replay: bool,
    next_synthetic_player_command_id: u64,
    player_transition_classifier: PlayerTransitionClassifier,
    last_player_transition_classification: Option<PlayerTransitionClassification>,
    pending_native_play_authority_fence: Option<PendingNativePlayAuthorityFence>,
    last_technical_readiness_fingerprint: Option<TechnicalReadinessFingerprint>,
    next_technical_readiness_report_sequence: u64,
    latest_observation: Option<PlayerTransportObservation>,
    latest_position_observation: Option<LocalPositionObservation>,
    participant_status_evidence_times: ParticipantStatusEvidenceTimes,
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
    transport_telemetry_ever_observed: bool,
    awaiting_ordered_snapshot: bool,
    external_player_availability: Option<ExternalPlayerAvailability>,
    external_player_lifecycle_observed: bool,
    last_transport_telemetry_received_at_seconds: Option<f64>,
    transport_telemetry_wait_started_at_seconds: Option<f64>,
    transport_telemetry_lifecycle_fence_at_seconds: Option<f64>,
    participant_status_owner_clock_invalidated: bool,
    reconnect_reconciliation: Option<ReconnectReconciliation>,
    last_applied_revision: Option<u64>,
    last_started_revision: Option<u64>,
    last_degraded_reason: Option<DegradedPlaybackReason>,
}

impl RuntimePlaybackCoordination {
    fn clear_timeout_player_failure_for_recovered_load(
        &mut self,
        adapter_generation: PlayerMediaGeneration,
    ) {
        let logical_generation = self
            .adapter_generation_bindings
            .get(&adapter_generation.get())
            .map(|binding| binding.logical_generation)
            .or_else(|| self.coordinator.current_media_generation());
        if self
            .last_technical_readiness_fingerprint
            .is_some_and(|fingerprint| {
                fingerprint.reason == Some(TechnicalBlockCause::PlayerFailure)
                    && logical_generation == Some(fingerprint.media_generation)
            })
        {
            self.last_technical_readiness_fingerprint = None;
        }
    }

    pub(crate) fn set_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.coordinator.set_config(config);
    }

    pub(crate) fn prepare_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let (plan, actions) =
            self.prepare_media_internal(logical_id, kind, None, now_seconds, false, false);
        debug_assert!(actions.is_empty());
        plan
    }

    pub(crate) fn prepare_media_with_intent(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let (plan, actions) =
            self.prepare_media_internal(logical_id, kind, Some(intent), now_seconds, false, false);
        debug_assert!(actions.is_empty());
        plan
    }

    pub(crate) fn prepare_media_for_current_file_publication(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> (MediaLoadPlan, Vec<PlaybackCoordinatorAction>) {
        self.prepare_media_internal(
            logical_id,
            kind,
            Some(MediaLoadIntent::TransportRefresh),
            now_seconds,
            false,
            true,
        )
    }

    pub(crate) fn prepare_media_for_room_participation(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let (plan, actions) = self.prepare_media_internal(
            logical_id,
            kind,
            Some(MediaLoadIntent::TransportRefresh),
            now_seconds,
            true,
            false,
        );
        debug_assert!(actions.is_empty());
        plan
    }

    pub(crate) fn retire_media(&mut self) -> Vec<PlaybackCoordinatorAction> {
        let actions = self.coordinator.retire_media();
        self.adapter_generation_bindings.clear();
        self.pending_media_identity = None;
        self.highest_bound_adapter_generation = None;
        self.player_command_bindings.clear();
        self.pending_coordinator_command_completion_replay = false;
        self.last_player_transition_classification = None;
        self.pending_native_play_authority_fence = None;
        self.last_technical_readiness_fingerprint = None;
        self.latest_observation = None;
        self.latest_position_observation = None;
        self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        self.desired_generation = None;
        self.desired_fingerprint = None;
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.pending_forced_seek_revision = None;
        self.transport_telemetry_observed = false;
        self.last_transport_telemetry_received_at_seconds = None;
        self.transport_telemetry_wait_started_at_seconds = None;
        self.transport_telemetry_lifecycle_fence_at_seconds = None;
        self.participant_status_owner_clock_invalidated = false;
        self.reconnect_reconciliation = None;
        self.last_applied_revision = None;
        self.last_started_revision = None;
        self.last_degraded_reason = None;
        self.barrier.last_reported_barrier_ready = None;
        self.barrier.last_reported_barrier_started = None;
        self.barrier.initiated_barrier = None;
        self.barrier.accepted_barrier = None;
        self.barrier.pending_barrier_recovery = None;
        self.barrier.accepted_barrier_terminal = false;
        self.barrier.pending_media_coordination = None;
        self.barrier.handled_barrier_timeout = None;
        self.barrier.pending_barrier_timeout_action = None;
        self.barrier.last_reported_room_buffering = None;
        self.participant_status.last_participant_status_fingerprint = None;
        self.participant_status
            .last_participant_status_sent_at_seconds = None;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        actions
    }

    fn prepare_media_internal(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: Option<MediaLoadIntent>,
        now_seconds: f64,
        room_participation: bool,
        current_file_publication: bool,
    ) -> (MediaLoadPlan, Vec<PlaybackCoordinatorAction>) {
        let placeholder_adapter_generation = self
            .coordinator
            .current_logical_media_id()
            .filter(|logical_id| logical_id.as_str().starts_with("adapter-media-generation-"))
            .and(self.highest_bound_adapter_generation);
        let current_transport_evidence =
            self.highest_bound_adapter_generation
                .and_then(|adapter_generation| {
                    let binding = self.adapter_generation_bindings.get(&adapter_generation)?;
                    let current_generation = self.coordinator.current_media_generation()?;
                    let observation = self.latest_observation.clone()?;
                    (self.transport_telemetry_observed
                        && binding.logical_generation == current_generation
                        && observation.media_generation == current_generation)
                        .then_some(CurrentTransportEvidence {
                            adapter_generation,
                            observation,
                            position_observation: self.latest_position_observation,
                            participant_status_evidence_times: self
                                .participant_status_evidence_times,
                            last_received_at_seconds: self
                                .last_transport_telemetry_received_at_seconds,
                        })
                });
        let plan = if room_participation {
            self.coordinator
                .prepare_media_for_room_participation(logical_id, kind, now_seconds)
        } else {
            match intent {
                Some(intent) => self.coordinator.prepare_media_with_intent(
                    logical_id,
                    kind,
                    intent,
                    now_seconds,
                ),
                None => self
                    .coordinator
                    .prepare_media(logical_id, kind, now_seconds),
            }
        };
        let carries_current_transport_evidence =
            current_transport_evidence.as_ref().is_some_and(|evidence| {
                placeholder_adapter_generation == Some(evidence.adapter_generation)
                    || (current_file_publication
                        && plan.load_intent == MediaLoadIntent::TransportRefresh
                        && !plan.logical_media_changed
                        && !plan.playback_episode_changed)
            });
        self.adapter_generation_bindings
            .retain(|_, binding| binding.logical_generation != plan.media_generation);
        self.pending_media_identity = Some((plan.media_generation, plan.load_attempt));
        self.latest_observation = None;
        self.latest_position_observation = None;
        self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        self.transport_telemetry_observed = false;
        self.last_transport_telemetry_received_at_seconds = None;
        self.transport_telemetry_wait_started_at_seconds =
            self.transport_telemetry_available.then_some(now_seconds);
        // Startup media preparation can occur before the GUI/CLI owner has
        // established its runtime clock. In that phase `now_seconds` may be a
        // wall clock while ordered player delivery later uses a monotonic
        // owner clock; comparing them would reject every current-generation
        // load observation. Adapter/media generation binding is the complete
        // pre-owner fence. Add the timestamp fence only after lifecycle
        // ownership has supplied a comparable clock domain.
        self.transport_telemetry_lifecycle_fence_at_seconds =
            (self.external_player_availability.is_some() && now_seconds.is_finite())
                .then_some(now_seconds);
        self.participant_status_owner_clock_invalidated = false;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        self.player_command_bindings.clear();
        self.pending_coordinator_command_completion_replay = false;
        let classifier_adapter_epoch = self.classifier_adapter_epoch();
        self.player_transition_classifier
            .begin_scope(plan.media_generation, classifier_adapter_epoch);
        self.last_player_transition_classification = None;
        self.pending_native_play_authority_fence = None;
        self.last_technical_readiness_fingerprint = None;
        self.barrier.last_reported_barrier_ready = None;
        self.barrier.last_reported_barrier_started = None;
        if plan.playback_episode_changed {
            self.desired_generation = None;
            self.desired_fingerprint = None;
            self.pending_local_pause_intent = None;
            self.last_local_pause_intent_stage_accepted = None;
            self.pending_forced_seek_revision = None;
            self.last_applied_revision = None;
            self.last_started_revision = None;
            self.last_degraded_reason = None;
            self.barrier.initiated_barrier = None;
            self.barrier.accepted_barrier = None;
            self.barrier.pending_barrier_recovery = None;
            self.barrier.accepted_barrier_terminal = false;
            self.barrier.pending_media_coordination = (plan.load_intent
                != MediaLoadIntent::TransportRefresh)
                .then(|| PendingMediaCoordinationIntent {
                    local_media_generation: plan.media_generation,
                    load_intent: plan.load_intent,
                    include_start_barrier: true,
                    request_id: new_playback_barrier_request_id(plan.media_generation),
                    retry_request_nonce: None,
                    retry_not_before_seconds: None,
                    retry_attempts: 0,
                    room: None,
                });
            self.barrier.handled_barrier_timeout = None;
            self.barrier.pending_barrier_timeout_action = None;
            self.barrier.last_reported_room_buffering = None;
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
        let mut carried_actions = Vec::new();
        if carries_current_transport_evidence && let Some(mut evidence) = current_transport_evidence
        {
            // Local-file publication follows the adapter observation that
            // established the physical load. It can either refine the
            // temporary adapter-generation identity or repeat the already
            // prepared logical identity. Neither is a new physical lifecycle:
            // carry the accepted sample across the new logical load-attempt
            // token and rebind that exact adapter generation. A caller that is
            // actually replacing transport uses the ordinary preparation API
            // and therefore invalidates this evidence.
            evidence.observation.media_generation = plan.media_generation;
            if let Some(position) = evidence.position_observation.as_mut() {
                position.media_generation = plan.media_generation;
            }
            self.latest_observation = Some(evidence.observation.clone());
            self.latest_position_observation = evidence.position_observation;
            self.participant_status_evidence_times = evidence.participant_status_evidence_times;
            self.transport_telemetry_observed = true;
            self.last_transport_telemetry_received_at_seconds = evidence.last_received_at_seconds;
            self.transport_telemetry_wait_started_at_seconds = None;
            self.transport_telemetry_lifecycle_fence_at_seconds = None;
            self.external_player_availability = None;
            self.adapter_generation_bindings.insert(
                evidence.adapter_generation,
                LocalTransportGeneration {
                    logical_generation: plan.media_generation,
                    load_attempt: plan.load_attempt,
                    adapter_generation: evidence.adapter_generation,
                },
            );
            self.pending_media_identity = None;
            let actions = self.coordinator.rebase_observation(evidence.observation);
            self.record_observation_outcomes(&actions);
            carried_actions = actions;
        }
        (plan, carried_actions)
    }

    pub(crate) fn reset_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        self.adapter_epoch = self.adapter_epoch.saturating_add(1);
        // A new adapter cannot complete commands, recovery decisions, or a
        // degraded seek hold that belonged to the player it replaced. Treat
        // the boundary as a lifecycle supersession and make the next
        // canonical room state establish a fresh, still-bounded alignment
        // attempt against the replacement transport.
        let _ = self.coordinator.interrupt_recovery();
        self.coordinator.clear_seek_preparation_terminal();
        self.desired_generation = None;
        self.desired_fingerprint = None;
        self.pending_forced_seek_revision = None;
        self.adapter_generation_bindings.clear();
        self.highest_bound_adapter_generation = None;
        self.pending_media_identity = self
            .coordinator
            .current_media_generation()
            .zip(self.coordinator.current_load_attempt());
        self.player_command_bindings.clear();
        self.pending_coordinator_command_completion_replay = false;
        if let Some(media_generation) = self.coordinator.current_media_generation() {
            let classifier_adapter_epoch = self.classifier_adapter_epoch();
            self.player_transition_classifier
                .begin_scope(media_generation, classifier_adapter_epoch);
        }
        self.last_player_transition_classification = None;
        self.pending_native_play_authority_fence = None;
        self.last_technical_readiness_fingerprint = None;
        self.latest_observation = None;
        self.latest_position_observation = None;
        self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
        self.adapter_clock_offset_seconds = None;
        self.last_external_now_seconds = None;
        self.last_coordinator_now_seconds = None;
        self.transport_telemetry_observed = false;
        self.last_transport_telemetry_received_at_seconds = None;
        self.transport_telemetry_wait_started_at_seconds =
            self.transport_telemetry_available.then_some(now_seconds);
        self.transport_telemetry_lifecycle_fence_at_seconds =
            now_seconds.is_finite().then_some(now_seconds);
        self.participant_status_owner_clock_invalidated = false;
        self.pending_local_pause_intent = None;
        self.last_local_pause_intent_stage_accepted = None;
        self.barrier.last_reported_barrier_ready = None;
        self.barrier.last_reported_barrier_started = None;
        self.barrier.last_reported_room_buffering = None;
        self.participant_status.last_participant_status_fingerprint = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        self.barrier.pending_barrier_timeout_action = None;
        if let Some(reconciliation) = self.reconnect_reconciliation.as_mut() {
            reconciliation.target_revision = None;
        }
        self.coordinator.reset_transport_adapter_epoch(now_seconds);
        self.adapter_epoch
    }

    pub(crate) fn mark_transport_telemetry_available(&mut self) {
        self.transport_telemetry_available = true;
    }

    pub(crate) fn set_external_player_availability(
        &mut self,
        availability: ExternalPlayerAvailability,
        now_seconds: f64,
    ) -> bool {
        if self.external_player_availability == Some(availability) {
            return false;
        }
        let initial_lifecycle_observation = !self.external_player_lifecycle_observed;
        let lifecycle_fence = self.coordinator_now(now_seconds);
        self.external_player_lifecycle_observed = true;
        self.external_player_availability = Some(availability);
        self.transport_telemetry_wait_started_at_seconds = (availability
            == ExternalPlayerAvailability::Connecting
            && !(initial_lifecycle_observation && self.transport_telemetry_observed))
            .then_some(now_seconds);
        self.transport_telemetry_lifecycle_fence_at_seconds = (!initial_lifecycle_observation
            && lifecycle_fence.is_finite())
        .then_some(lifecycle_fence);
        // Every replacement lifecycle supersedes transport evidence from the
        // preceding lifecycle. Connecting can become Connected only after a
        // later accepted sample; TelemetryUnavailable must take effect
        // immediately instead of inheriting a five-second cache. The first
        // owner-clock observation has no preceding lifecycle to clear or
        // fence: CLI startup can drain current-epoch load/transport events
        // while sending Hello, before the connected loop owns its first clock.
        if !initial_lifecycle_observation {
            self.latest_observation = None;
            self.latest_position_observation = None;
            self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
            self.transport_telemetry_observed = false;
            self.last_transport_telemetry_received_at_seconds = None;
            self.coordinator
                .clear_participant_status_transport_metrics();
        }
        self.participant_status_owner_clock_invalidated = false;
        self.participant_status.last_participant_status_fingerprint = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        true
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

    pub(crate) fn keep_waiting_for_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.coordinator
            .keep_waiting_for_seek_preparation(self.coordinator_now(now_seconds))
    }

    pub(crate) fn cancel_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.coordinator
            .cancel_seek_preparation(self.coordinator_now(now_seconds))
    }

    pub(crate) fn join_nearest_buffered_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.coordinator
            .join_nearest_buffered_seek_preparation(self.coordinator_now(now_seconds))
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
            seek_preparation: self.coordinator.seek_preparation_snapshot(),
            last_seek_preparation_terminal_outcome: self
                .coordinator
                .last_seek_preparation_terminal_outcome(),
            last_seek_preparation_terminal: self
                .coordinator
                .last_seek_preparation_terminal_snapshot(),
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

    pub(crate) fn current_media_for_replay(&self) -> Option<(LogicalMediaId, MediaTransportKind)> {
        Some((
            self.coordinator.current_logical_media_id()?.clone(),
            self.coordinator.current_media_transport_kind()?,
        ))
    }

    pub(crate) fn begin_protocol_connection_generation(&mut self, session: &ClientSession) {
        self.connection_generation = self.connection_generation.saturating_add(1).max(1);
        self.participant_status.next_participant_status_sequence = 0;
        self.participant_status.last_participant_status_fingerprint = None;
        self.participant_status
            .last_participant_status_sent_at_seconds = None;
        self.participant_status.participant_status_room_scope = None;
        self.participant_status
            .participant_status_applied_room_scope = None;
        self.participant_status
            .participant_status_desired_scope_bindings
            .clear();
        // Reliable SetRoom intent survives connection replacement. Preserve
        // its semantic suppression fence until authoritative membership
        // reaches the target (or a later explicit room intent supersedes it),
        // otherwise old-room evidence can be emitted behind the retained
        // SetRoom and authenticated into the destination room.
        self.last_technical_readiness_fingerprint = None;
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
        self.barrier.last_reported_barrier_ready = None;
        self.barrier.last_reported_barrier_started = None;
        self.barrier.last_reported_room_buffering = None;

        let recovering = self
            .barrier
            .pending_barrier_recovery
            .take()
            .map(|recovery| recovery.operation);
        let accepted = self.barrier.accepted_barrier.take();
        let initiated = self.barrier.initiated_barrier.take();
        let recoverable = recovering.or(accepted).or(initiated);

        if let Some(operation) = recoverable {
            // A socket write cannot distinguish an accepted request from
            // bytes lost before server parsing. Recover even a terminal
            // lifecycle first: the retained server operation also owns the
            // ongoing buffering policy and may need to be rebound from an
            // overlapping old transport identity.
            self.barrier.pending_barrier_recovery = Some(PendingPlaybackBarrierRecovery {
                operation,
                recovery_nonce: None,
                room: None,
            });
        } else if let Some(pending) = self.barrier.pending_media_coordination.as_mut() {
            // An unsent current-media intent remains valid, but it must bind
            // to the newly authenticated room and receive a fresh nonce.
            pending.room = None;
        }
    }

    pub(crate) fn bind_authoritative_room_control_context(&mut self, session: &ClientSession) {
        self.bind_local_control_authority_context(
            session,
            LocalControlAuthorityFreshness::Awaiting,
        );
    }

    fn bind_adapter_generation(
        &mut self,
        adapter_generation: PlayerMediaGeneration,
        external_now_seconds: f64,
    ) -> Option<u64> {
        let adapter_generation = adapter_generation.get();
        if let Some(logical_generation) =
            self.logical_generation_for_adapter_generation(adapter_generation)
        {
            return Some(logical_generation);
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

    fn logical_generation_for_adapter_generation(&self, adapter_generation: u64) -> Option<u64> {
        let binding = self.adapter_generation_bindings.get(&adapter_generation)?;
        let current_identity = self
            .coordinator
            .current_media_generation()
            .zip(self.coordinator.current_load_attempt());
        (current_identity == Some((binding.logical_generation, binding.load_attempt))
            && binding.adapter_generation == adapter_generation)
            .then_some(binding.logical_generation)
    }

    fn map_observation_time(
        &self,
        observed_at: Option<PlayerObservationTimestamp>,
        external_now_seconds: f64,
    ) -> (f64, Option<f64>, f64) {
        match observed_at {
            Some(timestamp) => {
                let raw_seconds = timestamp.elapsed_since_adapter_start().as_secs_f64();
                let delivery_reference_seconds = timestamp
                    .delivery_reference_since_adapter_start()
                    .as_secs_f64();
                let offset = self
                    .adapter_clock_offset_seconds
                    .unwrap_or(external_now_seconds - delivery_reference_seconds);
                (
                    raw_seconds + offset,
                    Some(offset),
                    delivery_reference_seconds + offset,
                )
            }
            None => {
                let coordinator_now = self.coordinator_now(external_now_seconds);
                (coordinator_now, None, coordinator_now)
            }
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
        delivery_reference_seconds: f64,
        candidate_offset_seconds: Option<f64>,
    ) {
        if self.adapter_clock_offset_seconds.is_none() {
            self.adapter_clock_offset_seconds = candidate_offset_seconds;
        }
        self.last_external_now_seconds = Some(
            self.last_external_now_seconds
                .filter(|current| external_now_seconds >= *current)
                .map_or(external_now_seconds, |current| {
                    current.max(external_now_seconds)
                }),
        );
        self.last_coordinator_now_seconds = Some(
            self.last_coordinator_now_seconds
                .map_or(delivery_reference_seconds, |current| {
                    current.max(delivery_reference_seconds)
                }),
        );
    }

    fn update_latest_position_observation(
        &mut self,
        update: &PlayerTransportObservation,
        replace_previous_state: bool,
    ) {
        let reported_or_retained_playback_rate = update.playback_rate.or_else(|| {
            (!replace_previous_state)
                .then(|| {
                    self.latest_observation
                        .as_ref()
                        .and_then(|current| current.playback_rate)
                })
                .flatten()
        });
        let motion_regime_transition = !replace_previous_state
            && self.latest_observation.as_ref().is_some_and(|current| {
                update
                    .phase
                    .is_some_and(|phase| current.phase != Some(phase))
                    || update
                        .playback_rate
                        .is_some_and(|rate| current.playback_rate != Some(rate))
                    || update
                        .logical_pause
                        .is_some_and(|paused| current.logical_pause != Some(paused))
                    || update
                        .paused_for_cache
                        .is_some_and(|paused| current.paused_for_cache != Some(paused))
                    || update
                        .seeking
                        .is_some_and(|seeking| current.seeking != Some(seeking))
                    || update
                        .core_idle
                        .is_some_and(|core_idle| current.core_idle != Some(core_idle))
            });
        let invalid_playback_rate =
            reported_or_retained_playback_rate.is_some_and(|rate| !rate.is_finite() || rate <= 0.0);
        let effective_playback_rate = reported_or_retained_playback_rate
            .filter(|rate| rate.is_finite() && *rate > 0.0)
            .unwrap_or(NORMAL_PLAYBACK_RATE);
        if invalid_playback_rate {
            self.latest_position_observation = None;
        } else if update.position_seconds.is_some() {
            self.latest_position_observation = update
                .position_seconds
                .filter(|position| position.is_finite())
                .map(|position_seconds| LocalPositionObservation {
                    media_generation: update.media_generation,
                    observed_at_seconds: update.observed_at_seconds,
                    last_actual_position_observed_at_seconds: update.observed_at_seconds,
                    position_seconds,
                    playback_rate: effective_playback_rate,
                });
        } else if replace_previous_state {
            self.latest_position_observation = None;
        } else if motion_regime_transition {
            self.latest_position_observation = self
                .project_position_observation_to(update.observed_at_seconds)
                .map(|mut position| {
                    if update.playback_rate.is_some() {
                        position.playback_rate = effective_playback_rate;
                    }
                    position
                });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_mapped_transport_observation(
        &mut self,
        observation: PlayerTransportObservation,
        position_update: &PlayerTransportObservation,
        external_now_seconds: f64,
        delivery_reference_seconds: f64,
        candidate_offset_seconds: Option<f64>,
        mut replace_latest_observation: bool,
        mut replace_position_state: bool,
    ) -> MappedTransportCommitOutcome {
        if !external_now_seconds.is_finite()
            || !delivery_reference_seconds.is_finite()
            || candidate_offset_seconds.is_some_and(|offset| !offset.is_finite())
        {
            return MappedTransportCommitOutcome::Rejected;
        }
        if self
            .transport_telemetry_lifecycle_fence_at_seconds
            .is_some_and(|fence| observation.observed_at_seconds < fence)
        {
            return MappedTransportCommitOutcome::LifecycleFenced;
        }
        if self.coordinator.current_media_generation() != Some(observation.media_generation) {
            return MappedTransportCommitOutcome::Rejected;
        }
        if !self.observation_timestamp_is_accepted(
            observation.media_generation,
            observation.observed_at_seconds,
        ) {
            return MappedTransportCommitOutcome::Rejected;
        }
        let owner_clock_rolled_back = self.last_external_now_seconds.is_some_and(|last_observed| {
            !last_observed.is_finite() || external_now_seconds < last_observed
        });
        let replace_participant_status_clock =
            owner_clock_rolled_back || self.participant_status_owner_clock_invalidated;
        if replace_participant_status_clock {
            replace_latest_observation = true;
            replace_position_state = true;
            self.latest_observation = None;
            self.latest_position_observation = None;
            self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
            self.participant_status.last_participant_status_fingerprint = None;
        }
        let telemetry_gap_expired = self
            .last_transport_telemetry_received_at_seconds
            .is_some_and(|received_at| {
                external_now_seconds.is_finite()
                    && received_at.is_finite()
                    && external_now_seconds >= received_at
                    && external_now_seconds - received_at
                        > PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS
            });
        if telemetry_gap_expired {
            // A first sample after a long transport gap is a new observation
            // base. Sparse deltas must not inherit old position, rate, cache,
            // buffer, or skew evidence and republish it as fresh.
            self.latest_observation = None;
            self.latest_position_observation = None;
            self.participant_status_evidence_times = ParticipantStatusEvidenceTimes::default();
            self.coordinator.expire_transport_observation();
            self.participant_status.last_participant_status_fingerprint = None;
        }
        self.commit_observation_clock(
            external_now_seconds,
            delivery_reference_seconds,
            candidate_offset_seconds,
        );
        self.transport_telemetry_ever_observed = true;
        self.transport_telemetry_observed = true;
        self.last_transport_telemetry_received_at_seconds =
            Some(if replace_participant_status_clock {
                external_now_seconds
            } else {
                self.last_transport_telemetry_received_at_seconds
                    .map_or(external_now_seconds, |received_at| {
                        received_at.max(external_now_seconds)
                    })
            });
        self.transport_telemetry_wait_started_at_seconds = None;
        self.external_player_availability = None;
        self.participant_status_owner_clock_invalidated = false;
        self.update_participant_status_evidence_times(position_update, replace_position_state);
        self.update_latest_position_observation(position_update, replace_position_state);
        if replace_latest_observation {
            self.latest_observation = Some(observation);
        } else {
            self.merge_latest_observation(observation);
        }
        MappedTransportCommitOutcome::Committed
    }

    pub(crate) fn observe_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.observe_transport_with_semantics(update, external_now_seconds, false)
    }

    pub(crate) fn rebase_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.observe_transport_with_semantics(update, external_now_seconds, true)
    }

    fn observe_transport_with_semantics(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
        replace_previous_state: bool,
    ) -> Vec<PlaybackCoordinatorAction> {
        // Receiving an update establishes adapter capability even when the
        // event itself is stale or cannot be bound to the active load. Once
        // known, reconnect validation must never fall back to direct player
        // correction merely because the current transport is between samples.
        self.transport_telemetry_available = true;
        if matches!(
            self.external_player_availability,
            Some(
                ExternalPlayerAvailability::Unavailable
                    | ExternalPlayerAvailability::TelemetryUnavailable
                    | ExternalPlayerAvailability::Disconnected
                    | ExternalPlayerAvailability::Failed,
            )
        ) {
            // Detach/failure is a hard lifecycle fence. A sample already in
            // flight from the retired player must not bind a new adapter
            // generation, advance clocks, mutate the merged observation, or
            // drive coordinator/barrier actions. Only an explicit Connecting
            // transition reopens telemetry ingestion.
            return Vec::new();
        }
        let Some(adapter_generation) = update.media_generation else {
            return Vec::new();
        };
        let Some(media_generation) =
            self.bind_adapter_generation(adapter_generation, external_now_seconds)
        else {
            return Vec::new();
        };
        let (observed_at_seconds, candidate_offset_seconds, delivery_reference_seconds) =
            self.map_observation_time(update.observed_at, external_now_seconds);
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
            timeline_kind: update.timeline_kind,
            seekable_ranges: update.seekable_ranges,
            known_live_seekable_window: update.known_live_seekable_window,
            core_idle: update.core_idle,
            playback_restart_sequence: update.playback_restart_sequence,
            cache_buffering_percent: update.cache_buffering_percent,
            buffered_ahead_seconds: update.buffered_ahead_seconds,
            input_rate_bytes_per_second: update.input_rate_bytes_per_second,
        };
        let position_update = observation.clone();
        let commit_outcome = self.commit_mapped_transport_observation(
            observation.clone(),
            &position_update,
            external_now_seconds,
            delivery_reference_seconds,
            candidate_offset_seconds,
            replace_previous_state,
            replace_previous_state,
        );
        if commit_outcome == MappedTransportCommitOutcome::LifecycleFenced {
            return Vec::new();
        }
        let actions = if replace_previous_state {
            self.coordinator.rebase_observation(observation)
        } else {
            self.coordinator.observe(observation)
        };
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

    pub(crate) fn rebase_transport_at_epoch(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        external_now_seconds: f64,
        adapter_epoch: u64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if adapter_epoch != self.adapter_epoch {
            return Vec::new();
        }
        self.rebase_transport(update, external_now_seconds)
    }

    /// Records an EOF discovered by a legacy attached-player surface that
    /// cannot provide generation-aware transport telemetry. Keeping the
    /// observation in the merged transport snapshot makes the ordinary
    /// player-transition classifier treat the pause edge as technical rather
    /// than manufacturing a native user gesture.
    pub(crate) fn observe_external_end_of_file(&mut self, external_now_seconds: f64) {
        let Some(media_generation) = self.coordinator.current_media_generation() else {
            return;
        };
        let observed_at_seconds = self.coordinator_now(external_now_seconds);
        if !self.observation_timestamp_is_accepted(media_generation, observed_at_seconds) {
            return;
        }
        self.commit_observation_clock(external_now_seconds, observed_at_seconds, None);
        self.merge_latest_observation(
            PlayerTransportObservation::new(media_generation, observed_at_seconds)
                .with_phase(sorotte_player_api::PlayerTransportPhase::Ended)
                .with_logical_pause(true),
        );
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
        current.timeline_kind = newer.timeline_kind.or(current.timeline_kind);
        if (newer.timeline_kind == Some(sorotte_player_api::PlayerTimelineKind::SlidingLive)
            && newer.seekable_ranges.is_some())
            || newer.known_live_seekable_window.is_some()
        {
            current.known_live_seekable_window = newer.known_live_seekable_window;
        }
        current.seekable_ranges = newer
            .seekable_ranges
            .or_else(|| current.seekable_ranges.take());
        current.core_idle = newer.core_idle.or(current.core_idle);
        current.playback_restart_sequence = newer
            .playback_restart_sequence
            .or(current.playback_restart_sequence);
        current.cache_buffering_percent = newer
            .cache_buffering_percent
            .or(current.cache_buffering_percent);
        current.buffered_ahead_seconds = newer
            .buffered_ahead_seconds
            .or(current.buffered_ahead_seconds);
        current.input_rate_bytes_per_second = newer
            .input_rate_bytes_per_second
            .or(current.input_rate_bytes_per_second);
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
        let playlist_selection_transport_hold = session.has_pending_playlist_index_reset_intent();
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
        let authority_may_accept_local_intent = self
            .pending_local_pause_intent
            .as_ref()
            .is_none_or(|intent| {
                self.room_authority_may_accept_local_pause_intent(session, authority, intent.paused)
            });
        let mut local_intent_active = false;
        let mut local_intent_requires_player_replay = false;
        let intent_context_matches =
            self.pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    session.room() == Some(intent.room.as_str())
                        && intent.local_media_generation == media_generation
                        && session
                            .current_room_transport_revision()
                            .is_none_or(|revision| intent.base_transport_revision == Some(revision))
                });
        if self.pending_local_pause_intent.is_some() && !intent_context_matches {
            self.pending_local_pause_intent = None;
        }
        let canonical_playstate_updated_at_seconds = session.room().and_then(|room| {
            session
                .model
                .room
                .playstate_updated_at_seconds
                .get(room)
                .copied()
        });
        let player_confirms_local_intent =
            self.pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| {
                    self.latest_observation.as_ref().is_some_and(|observation| {
                        observation.media_generation == intent.local_media_generation
                            && observation.logical_pause == Some(intent.paused)
                    })
                });
        let mut retire_confirmed_or_stale_local_intent = false;
        if let Some(intent) = self.pending_local_pause_intent.as_mut() {
            let canonical_playstate_changed = match (
                canonical_playstate_updated_at_seconds,
                intent.last_canonical_playstate_updated_at_seconds,
            ) {
                // State messages are applied in transport order. Treat any
                // changed receipt timestamp as a new canonical observation so
                // a wall-clock rollback cannot strand an overlay indefinitely;
                // equal timestamps still coalesce batched/replayed updates.
                (Some(current), Some(previous)) => current != previous,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if canonical_playstate_changed
                && intent.paused == paused
                && player_confirms_local_intent
            {
                // The server may attribute a matching state to another user
                // after selecting its room anchor. Matching canonical truth
                // acknowledges the command only after the same media generation
                // has physically reached that pause state. Otherwise an
                // echo-before-player race can republish the preceding edge.
                retire_confirmed_or_stale_local_intent = true;
            } else if authority_may_accept_local_intent
                && intent.paused != paused
                && intent.connection_generation == self.connection_generation
                && intent.authorization == LocalIntentAuthorization::Authorized
            {
                if canonical_playstate_changed {
                    intent.last_canonical_playstate_updated_at_seconds =
                        canonical_playstate_updated_at_seconds;
                    intent.mismatching_canonical_playstate_updates = intent
                        .mismatching_canonical_playstate_updates
                        .saturating_add(1);
                    intent
                        .first_mismatching_canonical_playstate_at_seconds
                        .get_or_insert(external_now_seconds);
                }
                let mismatch_window_elapsed = intent
                    .first_mismatching_canonical_playstate_at_seconds
                    .is_some_and(|first_mismatch_at| {
                        let elapsed_seconds = external_now_seconds - first_mismatch_at;
                        elapsed_seconds.is_finite()
                            && elapsed_seconds >= LOCAL_PAUSE_INTENT_MISMATCH_WINDOW_SECONDS
                    });
                if intent
                    .first_mismatching_canonical_playstate_at_seconds
                    .is_some_and(|first_mismatch_at| external_now_seconds < first_mismatch_at)
                {
                    // GUI session clocks are wall-clock based. If the system
                    // clock moves backwards, restart the bounded wait on the
                    // adjusted timeline rather than waiting for the old wall
                    // time to catch up.
                    intent.first_mismatching_canonical_playstate_at_seconds =
                        Some(external_now_seconds);
                }
                retire_confirmed_or_stale_local_intent = intent
                    .mismatching_canonical_playstate_updates
                    >= MAX_MISMATCHING_LOCAL_PAUSE_INTENT_UPDATES
                    && mismatch_window_elapsed;
            }
        }
        if retire_confirmed_or_stale_local_intent {
            // A local overlay only bridges the command/echo race. Matching
            // canonical truth acknowledges it; repeated, sufficiently spaced
            // disagreement means it was rejected, lost, or superseded.
            self.pending_local_pause_intent = None;
        }
        if self.pending_local_pause_intent.is_some()
            && canonical_local_echo
            && player_confirms_local_intent
            && self
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.paused == paused)
        {
            // A matching canonical self-echo plus a matching physical player
            // observation retires the command even if its originating
            // connection is now dormant.
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
            // The exact Preparing gate remains server-owned. Room buffering
            // also retains ownership of Play, while Pause was admitted above
            // as a monotonic safety transition. Every other readiness V2
            // barrier phase permits an authorized controller's ordinary
            // play/pause command while its server echo is in flight.
            self.pending_local_pause_intent = None;
        }
        let mut local_echo =
            canonical_local_echo || (local_intent_active && !local_intent_requires_player_replay);
        if playlist_selection_transport_hold {
            // Set.playlistIndex and the State that authorizes its transport are
            // distinct frames. Hold the physical successor at its origin until
            // the application has loaded/reset it and accepted a post-selection
            // playstate; otherwise predecessor Play authority can be replayed
            // during the gap and land after the successor was briefly paused.
            // Apply this after readiness/barrier and local-intent overlays so no
            // predecessor authority class can escape the selection fence.
            paused = true;
            position_seconds = 0.0;
            local_echo = false;
        }
        if !position_seconds.is_finite() {
            return Vec::new();
        }

        // The fingerprint must describe the effective target admitted through
        // the playlist-selection fence. Recording the raw canonical position
        // here would make a Seek received during an asynchronous load appear
        // already applied: after the physical reset consumed the fence, the
        // same State revision would no longer look changed and its position
        // could remain stranded at zero. Keep the held origin in the
        // fingerprint so releasing the fence deterministically replays the
        // newest canonical position onto the now-observed successor media.
        let fingerprint_position_seconds = if playlist_selection_transport_hold {
            position_seconds
        } else {
            raw.position.unwrap_or(position_seconds)
        };
        let fingerprint = RoomDesiredFingerprint {
            paused,
            position_seconds: fingerprint_position_seconds,
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
        self.refresh_participant_status_room_scope(session);
        if desired_changed
            && let Some(scope) = self
                .participant_status
                .participant_status_room_scope
                .as_ref()
                .filter(|scope| scope.local_media_generation == media_generation)
                .cloned()
        {
            self.participant_status
                .participant_status_desired_scope_bindings
                .insert((media_generation, self.desired_revision), scope);
            while self
                .participant_status
                .participant_status_desired_scope_bindings
                .len()
                > 32
            {
                self.participant_status
                    .participant_status_desired_scope_bindings
                    .pop_first();
            }
        }

        let coordinator_now = self.coordinator_now(external_now_seconds);
        let superseded_dispatched_seek = if barrier_became_authoritative
            && matches!(authority, RoomPlaystateAuthority::ServerBarrier { .. })
        {
            // A startup barrier owns its canonical target and readiness
            // lifecycle. An earlier client-only unbuffered-seek episode must
            // not retain a frozen target or suppress the barrier alignment.
            let primary_seek_issued = self.coordinator.cancel_seek_preparation_for_lifecycle();
            self.coordinator.clear_seek_preparation_terminal();
            primary_seek_issued
        } else {
            false
        };
        let mut actions = self.coordinator.update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation,
                state_revision: self.desired_revision,
                paused,
                anchor_position_seconds: position_seconds,
                anchor_observed_at_seconds: coordinator_now,
                force_seek: self.pending_forced_seek_revision == Some(self.desired_revision),
            },
            if superseded_dispatched_seek {
                DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch
            } else if explicit_seek_changed {
                match authority {
                    RoomPlaystateAuthority::LegacyLocalEcho => {
                        DesiredRoomPlaybackUpdateKind::ExplicitSeekAlreadyDispatched
                    }
                    RoomPlaystateAuthority::LegacyRemoteUser => {
                        DesiredRoomPlaybackUpdateKind::ExplicitSeek
                    }
                    RoomPlaystateAuthority::ServerBarrier { .. }
                    | RoomPlaystateAuthority::ServerBufferingPolicy { .. } => {
                        DesiredRoomPlaybackUpdateKind::Ordinary
                    }
                }
            } else {
                DesiredRoomPlaybackUpdateKind::Ordinary
            },
        );
        let replay_desired_change =
            allow_command_replay && (!local_echo || self.reconnect_reconciliation.is_some());
        let replay_completed_command =
            allow_command_replay && self.pending_coordinator_command_completion_replay;
        if (replay_completed_command || (replay_desired_change && desired_changed))
            && let Some(observation) = self.latest_observation.clone()
        {
            // This is a merged cache of previously accepted adapter fields,
            // not a new transport sample. It may drive reconciliation, but it
            // must never manufacture target-scoped refill/headroom evidence.
            actions.extend(self.coordinator.replay_observation(observation));
            self.pending_coordinator_command_completion_replay = false;
        }
        if allow_command_replay {
            // Timer ownership is independent of whether this room state may
            // replay a cached player observation. In particular, a canonical
            // local echo suppresses command replay but must still advance
            // seek-preparation and command deadlines on the normal pump.
            actions.extend(self.coordinator.tick(coordinator_now));
        }
        self.record_observation_outcomes(&actions);
        actions
    }

    fn record_observation_outcomes(&mut self, actions: &[PlaybackCoordinatorAction]) {
        for action in actions {
            match action {
                PlaybackCoordinatorAction::RevisionApplied {
                    media_generation,
                    state_revision,
                } => {
                    self.last_applied_revision = Some(*state_revision);
                    self.participant_status
                        .participant_status_applied_room_scope = self
                        .participant_status
                        .participant_status_desired_scope_bindings
                        .get(&(*media_generation, *state_revision))
                        .cloned();
                    self.participant_status
                        .participant_status_desired_scope_bindings
                        .retain(|(generation, revision), _| {
                            *generation != *media_generation || *revision > *state_revision
                        });
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
        cause: PlayerCommandCause,
        desired_paused: Option<bool>,
        issued_at_seconds: f64,
    ) {
        self.player_command_bindings.insert(
            player_command_id,
            PlayerCommandBinding {
                coordinator_command_id,
                desired_paused,
            },
        );
        if let Some(desired_paused) = desired_paused {
            self.register_player_pause_command(
                player_command_id,
                cause,
                desired_paused,
                issued_at_seconds,
            );
        }
    }

    pub(crate) fn bind_standalone_player_pause_command(
        &mut self,
        player_command_id: PlayerCommandId,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
    ) {
        self.register_player_pause_command(
            player_command_id,
            cause,
            desired_paused,
            issued_at_seconds,
        );
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
        let failed = self
            .coordinator
            .command_failed(coordinator_command_id, now_seconds);
        if failed {
            let actions = self.coordinator.take_pending_actions();
            self.record_observation_outcomes(&actions);
        }
    }

    pub(crate) fn apply_player_command_progress(
        &mut self,
        progress: sorotte_player_api::PlayerCommandProgress,
        external_now_seconds: f64,
    ) -> bool {
        let registered_pause_command = self
            .player_transition_classifier
            .command_registration(self.classifier_adapter_epoch(), progress.command_id)
            .is_some();
        let pause_command_failed = registered_pause_command
            && matches!(
                progress.state,
                PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(_))
            );
        self.update_player_command_completion(&progress, external_now_seconds);
        let Some(binding) = self
            .player_command_bindings
            .get(&progress.command_id)
            .copied()
        else {
            return pause_command_failed;
        };
        let coordinator_command_id = binding.coordinator_command_id;
        match progress.state {
            PlayerCommandProgressState::Accepted => {
                let _ = self.coordinator.command_accepted(coordinator_command_id);
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                // Completion is observation-backed in the adapter, but the
                // coordinator still owns RevisionApplied/Started based on its
                // full transport observation stream. Replay the last accepted
                // projection once because an idempotent command may complete
                // without producing a distinct property delta (for example,
                // seeking to the position already reported by mpv).
                let _ = self.coordinator.command_accepted(coordinator_command_id);
                self.pending_coordinator_command_completion_replay = true;
                self.player_command_bindings.remove(&progress.command_id);
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                PlayerCommandFailureKind::TimedOut,
            )) if self
                .coordinator
                .active_seek_preparation_lost_command_tracking(coordinator_command_id) =>
            {
                // mpv's generic tracked-command window is deliberately
                // shorter than an unbuffered seek-preparation episode. Losing
                // that adapter tracker is not transport failure: retain the
                // semantic seek and let target telemetry plus the extendable
                // preparation deadline decide its outcome.
                self.player_command_bindings.remove(&progress.command_id);
            }
            PlayerCommandProgressState::Finished(
                PlayerCommandResult::Superseded | PlayerCommandResult::Failed(_),
            ) => {
                let now_seconds = self.coordinator_now(external_now_seconds);
                let failed = self
                    .coordinator
                    .command_failed(coordinator_command_id, now_seconds);
                if failed {
                    let actions = self.coordinator.take_pending_actions();
                    self.record_observation_outcomes(&actions);
                }
                self.player_command_bindings.remove(&progress.command_id);
            }
        }
        pause_command_failed
    }

    pub(crate) fn apply_player_command_outcome(
        &mut self,
        outcome: sorotte_player_api::PlayerCommandOutcome,
        external_now_seconds: f64,
    ) -> bool {
        let result = match outcome.result {
            PlayerCommandSemanticResult::Completed => PlayerCommandResult::Completed,
            PlayerCommandSemanticResult::Superseded => PlayerCommandResult::Superseded,
            PlayerCommandSemanticResult::Failed(failure) => PlayerCommandResult::Failed(failure),
            PlayerCommandSemanticResult::CompletionNotObserved => {
                PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut)
            }
            PlayerCommandSemanticResult::TransportDisconnected => {
                PlayerCommandResult::Failed(PlayerCommandFailureKind::TransportDisconnected)
            }
        };
        self.apply_player_command_progress(
            sorotte_player_api::PlayerCommandProgress::finished(
                outcome.command_id,
                outcome.media_generation,
                None,
                None,
                result,
            ),
            external_now_seconds,
        )
    }
}

impl RuntimePlaybackCoordination {
    fn classifier_adapter_epoch(&self) -> u64 {
        // Runtime epoch zero is the initial adapter instance. The classifier
        // reserves zero as an invalid/unscoped value, so expose that first
        // instance as epoch one without changing the public runtime epoch.
        self.adapter_epoch.max(1)
    }

    fn register_player_pause_command(
        &mut self,
        command_id: PlayerCommandId,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
    ) -> bool {
        let Some(media_generation) = self.coordinator.current_media_generation() else {
            return false;
        };
        let adapter_epoch = self.classifier_adapter_epoch();
        self.player_transition_classifier
            .register_command(PlayerCommandRegistration::new(
                command_id,
                media_generation,
                adapter_epoch,
                cause,
                desired_paused,
                issued_at_seconds,
            ))
    }

    pub(crate) fn register_completed_synthetic_pause_command(
        &mut self,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
    ) {
        let _ = self.register_synthetic_pause_command_completion(
            cause,
            desired_paused,
            issued_at_seconds,
            PlayerCommandCompletion::Completed {
                at_seconds: issued_at_seconds,
            },
        );
    }

    pub(crate) fn register_failed_synthetic_pause_command(
        &mut self,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
    ) {
        let _ = self.register_synthetic_pause_command_completion(
            cause,
            desired_paused,
            issued_at_seconds,
            PlayerCommandCompletion::Failed {
                at_seconds: issued_at_seconds,
            },
        );
    }

    fn register_synthetic_pause_command_completion(
        &mut self,
        cause: PlayerCommandCause,
        desired_paused: bool,
        issued_at_seconds: f64,
        completion: PlayerCommandCompletion,
    ) -> Option<PlayerCommandId> {
        self.next_synthetic_player_command_id =
            self.next_synthetic_player_command_id.wrapping_add(1).max(1);
        let command_id =
            PlayerCommandId::new((1_u64 << 63) | self.next_synthetic_player_command_id);
        let media_generation = self.coordinator.current_media_generation()?;
        let adapter_epoch = self.classifier_adapter_epoch();
        let mut registration = PlayerCommandRegistration::new(
            command_id,
            media_generation,
            adapter_epoch,
            cause,
            desired_paused,
            issued_at_seconds,
        );
        registration.completion = completion;
        self.player_transition_classifier
            .register_command(registration)
            .then_some(command_id)
    }

    pub(crate) fn standalone_command_issued_at_seconds(&self, external_now_seconds: f64) -> f64 {
        match self.last_external_now_seconds {
            Some(last_external)
                if external_now_seconds.is_finite()
                    && (external_now_seconds - last_external).abs() <= 3_600.0 =>
            {
                self.coordinator_now(external_now_seconds)
            }
            _ => self.last_coordinator_now_seconds.unwrap_or(0.0),
        }
    }

    fn command_progress_observed_at_seconds(
        &self,
        progress: &sorotte_player_api::PlayerCommandProgress,
        external_now_seconds: f64,
    ) -> f64 {
        progress
            .observed_at
            .zip(self.adapter_clock_offset_seconds)
            .map_or_else(
                || self.coordinator_now(external_now_seconds),
                |(observed_at, offset)| {
                    observed_at.elapsed_since_adapter_start().as_secs_f64() + offset
                },
            )
    }

    fn update_player_command_completion(
        &mut self,
        progress: &sorotte_player_api::PlayerCommandProgress,
        external_now_seconds: f64,
    ) {
        let at_seconds = self.command_progress_observed_at_seconds(progress, external_now_seconds);
        let completion = match progress.state {
            PlayerCommandProgressState::Accepted => return,
            PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                PlayerCommandCompletion::Completed { at_seconds }
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                PlayerCommandFailureKind::TimedOut,
            )) => PlayerCommandCompletion::TimedOut { at_seconds },
            PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(_)) => {
                PlayerCommandCompletion::Failed { at_seconds }
            }
            PlayerCommandProgressState::Finished(PlayerCommandResult::Superseded) => {
                PlayerCommandCompletion::Superseded { at_seconds }
            }
        };
        let adapter_epoch = self.classifier_adapter_epoch();
        self.player_transition_classifier.update_command_completion(
            adapter_epoch,
            progress.command_id,
            completion,
        );
    }

    fn player_transition_context(&self, session: &ClientSession) -> PlayerTransitionContext {
        let Some(observation) = self.latest_observation.as_ref() else {
            return PlayerTransitionContext::default();
        };
        let authority = session.current_room_playstate_authority();
        PlayerTransitionContext::new(observation.phase)
            .with_paused_for_cache(observation.paused_for_cache == Some(true))
            .with_seeking(observation.seeking == Some(true))
            .with_media_transition(matches!(
                observation.phase,
                Some(
                    sorotte_player_api::PlayerTransportPhase::Empty
                        | sorotte_player_api::PlayerTransportPhase::Loading
                        | sorotte_player_api::PlayerTransportPhase::Prebuffering
                )
            ))
            .with_recovery(self.coordinator.recovery_episode().is_some())
            .with_seek_preparation(self.coordinator.seek_preparation_snapshot().is_some())
            .with_room_buffering_policy(matches!(
                authority,
                Some(RoomPlaystateAuthority::ServerBufferingPolicy { .. })
            ))
            .with_playback_barrier(session.playback_barrier_status().is_some_and(|status| {
                matches!(
                    status.phase,
                    PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::AwaitingDecision
                )
            }))
            .with_synchronization(
                self.reconnect_reconciliation.is_some()
                    || (self
                        .player_command_bindings
                        .values()
                        .any(|binding| binding.desired_paused.is_some())
                        && self.coordinator.ordinary_correction_blocked()),
            )
    }

    fn classify_latest_player_transition(
        &mut self,
        session: &ClientSession,
    ) -> Option<PlayerTransitionClassification> {
        self.invalidate_pending_native_play_if_authority_changed(session);
        let observation = self.latest_observation.as_ref()?;
        let logical_paused = observation.logical_pause?;
        let player_observation = PlayerLogicalPauseObservation::new(
            observation.media_generation,
            self.classifier_adapter_epoch(),
            observation.observed_at_seconds,
            logical_paused,
            self.player_transition_context(session),
        );
        let classification = self
            .player_transition_classifier
            .classify(player_observation);
        self.last_player_transition_classification = Some(classification);
        self.sync_pending_native_play_authority_fence(session);
        Some(classification)
    }

    fn next_technical_readiness_report(
        &mut self,
        session: &ClientSession,
    ) -> Option<TechnicalReadinessReport> {
        let observation = self.latest_observation.as_ref()?;
        let phase = observation.phase?;
        let local_media_generation = observation.media_generation;
        let local_username = session.username()?;
        let canonical = session.canonical_participant_readiness(local_username)?;
        let media_generation = session.readiness_snapshot()?.media_generation?;
        if let Some(prepare) = session
            .playback_barrier_prepare()
            .filter(|prepare| prepare.media_generation == media_generation)
            && !self.current_logical_media_matches(&prepare.logical_media_id)
        {
            return None;
        }
        if self.coordinator.current_media_generation() != Some(local_media_generation) {
            return None;
        }
        let membership_epoch = canonical.membership_epoch;
        if membership_epoch == 0 {
            return None;
        }
        let recovery = self.coordinator.recovery_episode();
        let (technical_phase, reason, recovery_stage) =
            if recovery.as_ref().is_some_and(|episode| episode.degraded) {
                (
                    TechnicalPlayabilityPhase::TerminallyBlocked,
                    Some(TechnicalBlockCause::RecoveryExhausted),
                    None,
                )
            } else if recovery.is_some() {
                (
                    TechnicalPlayabilityPhase::TemporarilyBlocked,
                    Some(TechnicalBlockCause::Recovery),
                    Some(RecoveryStage::Retrying),
                )
            } else if phase == sorotte_player_api::PlayerTransportPhase::Failed {
                (
                    TechnicalPlayabilityPhase::TerminallyBlocked,
                    Some(TechnicalBlockCause::PlayerFailure),
                    None,
                )
            } else if observation.paused_for_cache == Some(true) {
                (
                    TechnicalPlayabilityPhase::TemporarilyBlocked,
                    Some(TechnicalBlockCause::CachePause),
                    Some(RecoveryStage::Waiting),
                )
            } else if observation.seeking == Some(true)
                || phase == sorotte_player_api::PlayerTransportPhase::Seeking
                || self.coordinator.seek_preparation_snapshot().is_some()
            {
                (
                    TechnicalPlayabilityPhase::TemporarilyBlocked,
                    Some(TechnicalBlockCause::Seeking),
                    Some(RecoveryStage::Waiting),
                )
            } else {
                match phase {
                    sorotte_player_api::PlayerTransportPhase::Empty
                    | sorotte_player_api::PlayerTransportPhase::Loading => (
                        TechnicalPlayabilityPhase::Preparing,
                        Some(TechnicalBlockCause::Loading),
                        Some(RecoveryStage::NotStarted),
                    ),
                    sorotte_player_api::PlayerTransportPhase::Prebuffering => (
                        TechnicalPlayabilityPhase::Preparing,
                        Some(TechnicalBlockCause::Prebuffering),
                        Some(RecoveryStage::Waiting),
                    ),
                    sorotte_player_api::PlayerTransportPhase::Rebuffering => (
                        TechnicalPlayabilityPhase::TemporarilyBlocked,
                        Some(TechnicalBlockCause::Rebuffering),
                        Some(RecoveryStage::Waiting),
                    ),
                    sorotte_player_api::PlayerTransportPhase::Ended => (
                        // Ordinary EOF is a transport/media transition, not a
                        // terminal participant failure. It keeps room-facing
                        // Ready intact while preventing start eligibility until
                        // the fresh playlist/replay generation is playable.
                        TechnicalPlayabilityPhase::Preparing,
                        Some(TechnicalBlockCause::EndOfFile),
                        Some(RecoveryStage::NotStarted),
                    ),
                    sorotte_player_api::PlayerTransportPhase::ReadyPaused
                    | sorotte_player_api::PlayerTransportPhase::Playing => {
                        (TechnicalPlayabilityPhase::Playable, None, None)
                    }
                    sorotte_player_api::PlayerTransportPhase::Seeking
                    | sorotte_player_api::PlayerTransportPhase::Failed => unreachable!(
                        "seeking and failed transport phases are handled before the phase match"
                    ),
                }
            };
        let authoritative_playback_revision =
            authoritative_technical_playback_revision(session, media_generation, reason);
        let fingerprint = TechnicalReadinessFingerprint {
            connection_generation: self.connection_generation,
            membership_epoch,
            media_generation,
            authoritative_playback_revision,
            phase: technical_phase,
            reason,
            recovery: recovery_stage,
        };
        if self.last_technical_readiness_fingerprint == Some(fingerprint) {
            return None;
        }
        self.next_technical_readiness_report_sequence = self
            .next_technical_readiness_report_sequence
            .max(canonical.last_technical_report_sequence)
            .saturating_add(1);
        if self.next_technical_readiness_report_sequence == 0 {
            return None;
        }
        let mut report = TechnicalReadinessReport::new(
            media_generation,
            membership_epoch,
            self.next_technical_readiness_report_sequence,
            technical_phase,
        )
        .with_observed_at(observation.observed_at_seconds);
        if let Some(state_revision) = authoritative_playback_revision {
            report = report.with_authoritative_playback_revision(state_revision);
        }
        if let Some(reason) = reason {
            report = report.with_reason(reason);
        }
        if let Some(recovery_stage) = recovery_stage {
            report = report.with_recovery(recovery_stage);
        }
        Some(report)
    }

    fn next_player_command_failure_readiness_report(
        &mut self,
        session: &ClientSession,
        external_now_seconds: f64,
    ) -> Option<TechnicalReadinessReport> {
        self.coordinator.current_media_generation()?;
        let local_username = session.username()?;
        let canonical = session.canonical_participant_readiness(local_username)?;
        let media_generation = session.readiness_snapshot()?.media_generation?;
        let membership_epoch = canonical.membership_epoch;
        if membership_epoch == 0 {
            return None;
        }
        let authoritative_playback_revision = authoritative_technical_playback_revision(
            session,
            media_generation,
            Some(TechnicalBlockCause::PlayerFailure),
        );
        let fingerprint = TechnicalReadinessFingerprint {
            connection_generation: self.connection_generation,
            membership_epoch,
            media_generation,
            authoritative_playback_revision,
            phase: TechnicalPlayabilityPhase::TemporarilyBlocked,
            reason: Some(TechnicalBlockCause::PlayerFailure),
            recovery: Some(RecoveryStage::NotStarted),
        };
        if self.last_technical_readiness_fingerprint == Some(fingerprint) {
            return None;
        }
        let observed_at = self.standalone_command_issued_at_seconds(external_now_seconds);
        self.next_technical_readiness_report_sequence = self
            .next_technical_readiness_report_sequence
            .max(canonical.last_technical_report_sequence)
            .saturating_add(1);
        if self.next_technical_readiness_report_sequence == 0 {
            return None;
        }
        let mut report = TechnicalReadinessReport::new(
            media_generation,
            membership_epoch,
            self.next_technical_readiness_report_sequence,
            TechnicalPlayabilityPhase::TemporarilyBlocked,
        )
        .with_reason(TechnicalBlockCause::PlayerFailure)
        .with_recovery(RecoveryStage::NotStarted);
        if observed_at.is_finite() {
            report = report.with_observed_at(observed_at);
        }
        if let Some(state_revision) = authoritative_playback_revision {
            report = report.with_authoritative_playback_revision(state_revision);
        }
        Some(report)
    }

    fn mark_technical_readiness_report_delivered(&mut self, report: &TechnicalReadinessReport) {
        self.last_technical_readiness_fingerprint = Some(TechnicalReadinessFingerprint {
            connection_generation: self.connection_generation,
            membership_epoch: report.membership_epoch,
            media_generation: report.media_generation,
            authoritative_playback_revision: report.authoritative_playback_revision,
            phase: report.phase,
            reason: report.reason,
            recovery: report.recovery,
        });
    }

    fn tick_player_transition_classifier(&mut self, external_now_seconds: f64) {
        let now_seconds = self.coordinator_now(external_now_seconds);
        if let Some(classification) = self.player_transition_classifier.tick(now_seconds) {
            self.last_player_transition_classification = Some(classification);
        }
        self.player_transition_classifier
            .prune_commands(now_seconds);
    }

    fn cause_for_coordinator_command(
        &self,
        command: CoordinatorPlayerCommand,
    ) -> PlayerCommandCause {
        if self.coordinator.recovery_episode().is_some() {
            return PlayerCommandCause::Recovery;
        }
        if self.coordinator.seek_preparation_snapshot().is_some() {
            return PlayerCommandCause::SeekPreparation;
        }
        if self
            .desired_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| fingerprint.buffering_media_generation.is_some())
        {
            return PlayerCommandCause::RoomBufferingPolicy;
        }
        if self
            .desired_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| fingerprint.barrier_media_generation.is_some())
        {
            return match command {
                CoordinatorPlayerCommand::SetPaused(true) => PlayerCommandCause::ReadinessGateHold,
                CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_) => {
                    PlayerCommandCause::AutomaticReadinessStart
                }
                CoordinatorPlayerCommand::SetPosition(_)
                | CoordinatorPlayerCommand::SetPlaybackRate(_) => {
                    PlayerCommandCause::RemoteRoomSynchronization
                }
            };
        }
        if matches!(
            command,
            CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::SetPlaybackRate(_)
        ) {
            PlayerCommandCause::DesyncCorrection
        } else {
            PlayerCommandCause::RemoteRoomSynchronization
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
    if getrandom::fill(&mut bytes).is_ok() {
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
    pub fn set_playback_coordinator_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.playback_coordination.set_config(config);
    }

    pub fn reset_playback_transport_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        let epoch = self.playback_coordination.reset_adapter_epoch(now_seconds);
        let _ = self.emit_participant_status_transition(now_seconds);
        epoch
    }

    /// Records lifecycle evidence for a player owned outside this runtime.
    ///
    /// A later accepted current-epoch transport observation promotes
    /// `Connecting` to the wire-level `Connected` state. `Unavailable` and
    /// `Failed` fence late observations until the owner explicitly starts a
    /// new `Connecting` lifecycle.
    pub fn set_external_player_availability(
        &mut self,
        availability: ExternalPlayerAvailability,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        if !self
            .playback_coordination
            .set_external_player_availability(availability, now_seconds)
        {
            return Ok(false);
        }
        self.emit_participant_status_transition(now_seconds)
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
        let mut actions = self.playback_coordination.observe_transport_at_epoch(
            update,
            now_seconds,
            adapter_epoch,
        );
        let _ = self.handle_latest_player_readiness_observation();
        let _ = self.promote_pending_native_play_before_pause_correction(&mut actions);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        let _ = self.emit_participant_status_transition(now_seconds);
        actions
    }

    pub fn rebase_external_player_transport_at_epoch(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
        adapter_epoch: u64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let mut actions = self.playback_coordination.rebase_transport_at_epoch(
            update,
            now_seconds,
            adapter_epoch,
        );
        let _ = self.handle_latest_player_readiness_observation();
        let _ = self.promote_pending_native_play_before_pause_correction(&mut actions);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        let _ = self.emit_participant_status_transition(now_seconds);
        actions
    }

    /// Feeds a legacy attached-player EOF signal through the same technical
    /// readiness and causal-classification path as an adapter-reported
    /// `PlayerTransportPhase::Ended` observation.
    pub fn observe_external_player_end_of_file(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.playback_coordination
            .observe_external_end_of_file(now_seconds);
        self.record_player_playback_projection(
            PlayerPlaybackTelemetryUpdate::default().with_paused(true),
        );
        if self
            .playback_coordination
            .coordinator
            .current_media_generation()
            .is_some()
        {
            let (playlist_revision, playlist_index) = self
                .session
                .current_room_playlist()
                .map_or((None, None), |playlist| {
                    (Some(playlist.revision), playlist.index)
                });
            let playlist_selection_revision =
                self.session.current_room_playlist_selection_revision();
            let canonical_playlist_epoch = self.session.current_room_playlist_canonical_epoch();
            self.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
                attempt_id: None,
                media_generation: None,
                playlist_revision,
                playlist_selection_revision,
                canonical_playlist_epoch,
                playlist_index,
                completed_file: self.last_local_file_update.clone(),
            });
        }
        self.handle_latest_player_readiness_observation()?;
        let _ = self.emit_participant_status_transition(now_seconds)?;
        Ok(())
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
        self.finish_prepared_playback_media(plan, now_seconds)
    }

    pub(crate) fn prepare_playback_media_for_current_file_publication(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> Result<MediaLoadPlan, PlayerError> {
        let (plan, actions) = self
            .playback_coordination
            .prepare_media_for_current_file_publication(logical_id, kind, now_seconds);
        let plan = self.finish_prepared_playback_media(plan, now_seconds);
        self.execute_playback_coordinator_actions(actions, now_seconds)?;
        Ok(plan)
    }

    pub fn prepare_playback_media_for_room_participation(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        let plan = self
            .playback_coordination
            .prepare_media_for_room_participation(logical_id, kind, now_seconds);
        self.finish_prepared_playback_media(plan, now_seconds)
    }

    /// Retires the active logical media after canonical playlist authority no
    /// longer selects an entry. The player attachment remains usable, but no
    /// cached file, EOF, telemetry, or barrier evidence from this generation
    /// may bind a later selection.
    pub fn retire_playback_media(&mut self) -> Result<bool, PlayerError> {
        let had_media = self
            .playback_coordination
            .coordinator
            .current_media_generation()
            .is_some()
            || self.last_local_file_update.is_some()
            || self.pending_natural_playback_completion.is_some();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let actions = self.playback_coordination.retire_media();
        self.last_local_file_update = None;
        self.pending_natural_playback_completion = None;
        self.pending_player_playback_telemetry_updates = EffectOutbox::default();
        self.pending_ordered_local_file_updates = EffectOutbox::default();
        self.pending_state_sync_player_error = None;
        self.pending_reconnect_rate_reset = false;
        self.control.cancel_protocol_playback_barrier_requests();
        self.execute_playback_coordinator_actions(actions, now_seconds)?;
        let status_changed = self.emit_participant_status_transition(now_seconds)?;
        Ok(had_media || status_changed)
    }

    fn finish_prepared_playback_media(
        &mut self,
        plan: MediaLoadPlan,
        now_seconds: f64,
    ) -> MediaLoadPlan {
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

    pub fn logical_generation_for_adapter_generation(
        &self,
        adapter_generation: PlayerMediaGeneration,
    ) -> Option<u64> {
        self.playback_coordination
            .logical_generation_for_adapter_generation(adapter_generation.get())
    }

    pub fn keep_waiting_for_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .keep_waiting_for_seek_preparation(now_seconds)
    }

    pub fn cancel_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .cancel_seek_preparation(now_seconds)
    }

    pub fn join_nearest_buffered_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination
            .join_nearest_buffered_seek_preparation(now_seconds)
    }

    /// Interrupts a recovery episode for an externally owned player (the GUI
    /// attached-player path) and returns every cleanup command to that owner.
    /// The internal runtime player must not consume these commands because GUI
    /// sessions intentionally use a no-op adapter there.
    pub fn interrupt_external_playback_recovery(&mut self) -> Vec<PlaybackCoordinatorAction> {
        self.playback_coordination.interrupt_recovery()
    }

    /// Tags a pause/play command issued through an attached-player system
    /// seam. The explicit cause prevents remote synchronization, gate holds,
    /// playlist transitions, and lifecycle corrections from being mistaken
    /// for a native user gesture when their telemetry arrives later.
    pub fn record_external_system_player_pause_command_result(
        &mut self,
        paused: bool,
        cause: PlayerCommandCause,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        debug_assert_ne!(cause, PlayerCommandCause::LocalUserPlaybackControl);
        let command_id = self.begin_external_player_pause_command(paused, cause, now_seconds);
        self.finish_external_player_pause_command(command_id, succeeded, now_seconds)
    }

    pub fn observe_external_player_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        if let Some(playback_rate) = update.playback_rate {
            self.session.apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default().with_playback_rate(playback_rate),
            );
        }
        let mut actions = self
            .playback_coordination
            .observe_transport(update, now_seconds);
        let _ = self.handle_latest_player_readiness_observation();
        let _ = self.promote_pending_native_play_before_pause_correction(&mut actions);
        let _ = self.report_playback_barrier_observations(&actions);
        self.apply_external_coordinator_control_actions(&actions);
        let _ = self.emit_participant_status_transition(now_seconds);
        actions
    }

    pub fn reconcile_external_player_playback(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        let mut actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        let _ = self.promote_pending_native_play_before_pause_correction(&mut actions);
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
        self.finish_external_coordinator_command_dispatch(command_id, None, result, now_seconds);
    }

    pub fn begin_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        let issued_at_seconds = self.playback_coordination.coordinator_now(now_seconds);
        let (cause, desired_paused, _) = self
            .playback_coordination
            .external_pause_command_registration(command_id, issued_at_seconds)?;
        self.playback_coordination
            .begin_external_pause_command(cause, desired_paused, now_seconds)
    }

    pub fn finish_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        player_command_id: Option<PlayerCommandId>,
        result: Result<(), PlayerError>,
        now_seconds: f64,
    ) {
        let issued_at_seconds = self.playback_coordination.coordinator_now(now_seconds);
        let pause_registration = player_command_id.is_none().then(|| {
            self.playback_coordination
                .external_pause_command_registration(command_id, issued_at_seconds)
        });
        if let Some(player_command_id) = player_command_id {
            let _ = self.playback_coordination.finish_external_pause_command(
                player_command_id,
                result.is_ok(),
                now_seconds,
            );
        }
        match result {
            Ok(()) => {
                if let Some(Some((cause, desired_paused, issued_at_seconds))) = pause_registration {
                    self.playback_coordination
                        .register_completed_synthetic_pause_command(
                            cause,
                            desired_paused,
                            issued_at_seconds,
                        );
                }
                self.playback_coordination
                    .command_dispatch_succeeded(command_id)
            }
            Err(_) => {
                if let Some(Some((cause, desired_paused, issued_at_seconds))) = pause_registration {
                    self.playback_coordination
                        .register_failed_synthetic_pause_command(
                            cause,
                            desired_paused,
                            issued_at_seconds,
                        );
                }
                if player_command_id.is_some() || pause_registration.flatten().is_some() {
                    let _ = self.report_player_command_failure_readiness(now_seconds);
                }
                self.playback_coordination
                    .command_dispatch_failed(command_id, now_seconds);
            }
        }
    }

    fn record_player_playback_projection(&mut self, update: PlayerPlaybackTelemetryUpdate) {
        if update == PlayerPlaybackTelemetryUpdate::default() {
            return;
        }
        self.session
            .apply_ordered_player_playback_telemetry_update(&update);
        if let Some(pending) = self.pending_player_playback_telemetry_updates.back_mut() {
            pending.paused = update.paused.or(pending.paused);
            pending.position_seconds = update.position_seconds.or(pending.position_seconds);
            pending.playback_rate = update.playback_rate.or(pending.playback_rate);
            pending.paused_for_cache = update.paused_for_cache.or(pending.paused_for_cache);
            pending.cache_buffering_percent = update
                .cache_buffering_percent
                .or(pending.cache_buffering_percent);
        } else {
            self.pending_player_playback_telemetry_updates
                .push_back(update);
        }
    }

    /// Compacts consumer state after the harness has acknowledged the batch externally.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn compact_acknowledged_player_event_batch_for_verification(
        &mut self,
        acknowledgement_token: PlayerEventAcknowledgementToken,
        sequence_boundary: PlayerSequenceBoundary,
    ) {
        self.ordered_player_events
            .compact_acknowledged_delivery(acknowledgement_token, sequence_boundary);
    }

    /// Returns the ordered consumer's comparable lifecycle projection.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn lifecycle_verification_projection(&self) -> LifecycleVerificationProjection {
        self.ordered_player_events
            .lifecycle_verification_projection()
    }

    pub(crate) fn drain_player_transport_coordination(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        if self.player.player_event_delivery_mode()
            == PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        {
            let mut first_error = self.pending_state_sync_player_error.take();
            if let Err(error) = self.drain_ordered_player_events(now_seconds)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            if first_error.is_none() {
                self.reconcile_player_transport_from_session(now_seconds, &mut first_error);
            }
            if let Err(error) = self.emit_participant_status_transition(now_seconds)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            return first_error.map_or(Ok(()), Err);
        }
        let mut first_error = None;
        while let Some(progress) = self.player.take_command_progress() {
            if self
                .playback_coordination
                .apply_player_command_progress(progress, now_seconds)
                && let Err(error) = self.report_player_command_failure_readiness(now_seconds)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        while let Some(update) = self.player.take_transport_telemetry_update() {
            let mut actions = self
                .playback_coordination
                .observe_transport(update, now_seconds);
            if let Err(error) = self.handle_latest_player_readiness_observation()
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            if let Err(error) =
                self.promote_pending_native_play_before_pause_correction(&mut actions)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
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
        self.reconcile_player_transport_from_session(now_seconds, &mut first_error);
        if let Err(error) = self.emit_participant_status_transition(now_seconds)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }

    fn reconcile_player_transport_from_session(
        &mut self,
        now_seconds: f64,
        first_error: &mut Option<PlayerError>,
    ) {
        let mut actions = self
            .playback_coordination
            .update_desired_from_session(&self.session, now_seconds);
        if let Err(error) = self.promote_pending_native_play_before_pause_correction(&mut actions)
            && first_error.is_none()
        {
            *first_error = Some(error);
        }
        if let Err(error) = self.report_playback_barrier_observations(&actions)
            && first_error.is_none()
        {
            *first_error = Some(crate::control::client_effect_player_error(error));
        }
        if let Err(error) = self.execute_playback_coordinator_actions(actions, now_seconds)
            && first_error.is_none()
        {
            *first_error = Some(error);
        }
        self.playback_coordination
            .tick_player_transition_classifier(now_seconds);
    }

    fn handle_latest_player_readiness_observation(&mut self) -> Result<(), PlayerError> {
        let classification = self
            .playback_coordination
            .classify_latest_player_transition(&self.session);
        if let Some(PlayerTransitionClassification::NativePlayerGesture { action }) = classification
        {
            self.dispatch_native_player_readiness_action(
                action,
                PlayerInteractionSurface::NativePlayerControl,
            )?;
        }

        if let Some(report) = self
            .playback_coordination
            .next_technical_readiness_report(&self.session)
            && let Some(action) = self
                .session
                .runtime_action_for_technical_readiness(report.clone())
        {
            self.dispatch_runtime_actions_with_causal_tracking(&[action])?;
            self.playback_coordination
                .mark_technical_readiness_report_delivered(&report);
        }
        Ok(())
    }

    pub(crate) fn interrupt_playback_recovery(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let actions = self.playback_coordination.interrupt_recovery();
        self.execute_playback_coordinator_actions(actions, now_seconds)
    }

    pub fn run_keep_waiting_for_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        let available = self
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some_and(|preparation| preparation.can_keep_waiting);
        let actions = self
            .playback_coordination
            .keep_waiting_for_seek_preparation(now_seconds);
        self.execute_playback_coordinator_actions(actions, now_seconds)?;
        Ok(available)
    }

    pub fn run_cancel_seek_preparation(&mut self, now_seconds: f64) -> Result<bool, PlayerError> {
        let available = self
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some_and(|preparation| preparation.can_cancel_and_remain);
        let actions = self
            .playback_coordination
            .cancel_seek_preparation(now_seconds);
        self.execute_playback_coordinator_actions(actions, now_seconds)?;
        Ok(available)
    }

    pub fn run_join_nearest_buffered_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        let available = self
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some_and(|preparation| preparation.can_join_nearest_buffered);
        let actions = self
            .playback_coordination
            .join_nearest_buffered_seek_preparation(now_seconds);
        self.execute_playback_coordinator_actions(actions, now_seconds)?;
        Ok(available)
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
                    if let Err(error) = self.run_system_owned_pause(true)
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
        let cause = self
            .playback_coordination
            .cause_for_coordinator_command(command);
        let desired_paused = match command {
            CoordinatorPlayerCommand::SetPaused(paused) => Some(paused),
            CoordinatorPlayerCommand::Play(_) => Some(false),
            CoordinatorPlayerCommand::SetPosition(_)
            | CoordinatorPlayerCommand::SetPlaybackRate(_) => None,
        };
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
                let issued_at_seconds = self
                    .playback_coordination
                    .coordinator_now(external_now_seconds);
                self.playback_coordination.bind_player_command(
                    player_command_id,
                    command_id,
                    cause,
                    desired_paused,
                    issued_at_seconds,
                );
                Ok(())
            }
            Err(PlayerError::Unsupported("execute_tracked")) => {
                match self.player.execute(player_command) {
                    Ok(()) => {
                        if let Some(desired_paused) = desired_paused {
                            let issued_at_seconds = self
                                .playback_coordination
                                .coordinator_now(external_now_seconds);
                            self.playback_coordination
                                .register_completed_synthetic_pause_command(
                                    cause,
                                    desired_paused,
                                    issued_at_seconds,
                                );
                        }
                        self.playback_coordination
                            .command_dispatch_succeeded(command_id);
                        Ok(())
                    }
                    Err(error) => {
                        let now_seconds = self
                            .playback_coordination
                            .coordinator_now(external_now_seconds);
                        if let Some(desired_paused) = desired_paused {
                            self.playback_coordination
                                .register_failed_synthetic_pause_command(
                                    cause,
                                    desired_paused,
                                    now_seconds,
                                );
                            let _ =
                                self.report_player_command_failure_readiness(external_now_seconds);
                        }
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
                if let Some(desired_paused) = desired_paused {
                    self.playback_coordination
                        .register_failed_synthetic_pause_command(
                            cause,
                            desired_paused,
                            now_seconds,
                        );
                    let _ = self.report_player_command_failure_readiness(external_now_seconds);
                }
                self.playback_coordination
                    .command_dispatch_failed(command_id, now_seconds);
                Err(error)
            }
        }
    }

    fn apply_external_coordinator_control_actions(
        &mut self,
        actions: &[PlaybackCoordinatorAction],
    ) {
        if actions
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::RequestRoomPause { .. }))
        {
            let _ = self.run_system_owned_pause(true);
        }
    }

    pub(crate) fn execute_causal_pause_command(
        &mut self,
        paused: bool,
        cause: PlayerCommandCause,
        external_now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let command = PlayerCommand::SetPaused(paused);
        let result = match self.player.execute_tracked(command.clone()) {
            Ok(player_command_id) => {
                let issued_at_seconds = self
                    .playback_coordination
                    .standalone_command_issued_at_seconds(external_now_seconds);
                self.playback_coordination
                    .bind_standalone_player_pause_command(
                        player_command_id,
                        cause,
                        paused,
                        issued_at_seconds,
                    );
                Ok(())
            }
            Err(PlayerError::Unsupported("execute_tracked")) => {
                let issued_at_seconds = self
                    .playback_coordination
                    .standalone_command_issued_at_seconds(external_now_seconds);
                match self.player.execute(command) {
                    Ok(()) => {
                        self.playback_coordination
                            .register_completed_synthetic_pause_command(
                                cause,
                                paused,
                                issued_at_seconds,
                            );
                        Ok(())
                    }
                    Err(error) => {
                        self.playback_coordination
                            .register_failed_synthetic_pause_command(
                                cause,
                                paused,
                                issued_at_seconds,
                            );
                        Err(error)
                    }
                }
            }
            Err(error) => {
                let issued_at_seconds = self
                    .playback_coordination
                    .standalone_command_issued_at_seconds(external_now_seconds);
                self.playback_coordination
                    .register_failed_synthetic_pause_command(cause, paused, issued_at_seconds);
                Err(error)
            }
        };
        if result.is_err() {
            // The gesture's semantic Ready/NotReady intent is independent of
            // physical player success. Report the failed transport command as
            // a technical blocker so a stale Playable report cannot let the
            // server commit an automatic start.
            let _ = self.report_player_command_failure_readiness(external_now_seconds);
        }
        result
    }

    pub(crate) fn report_player_command_failure_readiness(
        &mut self,
        external_now_seconds: f64,
    ) -> Result<(), PlayerError> {
        let Some(report) = self
            .playback_coordination
            .next_player_command_failure_readiness_report(&self.session, external_now_seconds)
        else {
            return Ok(());
        };
        let Some(action) = self
            .session
            .runtime_action_for_technical_readiness(report.clone())
        else {
            return Ok(());
        };
        ClientSession::dispatch_runtime_actions(&[action], &mut self.player, &mut self.control)?;
        self.playback_coordination
            .mark_technical_readiness_report_delivered(&report);
        Ok(())
    }

    pub(crate) fn run_system_owned_pause(&mut self, paused: bool) -> Result<(), PlayerError> {
        let previous = self.session.model.playback.local_paused;
        match self.execute_causal_pause_command(
            paused,
            PlayerCommandCause::Recovery,
            unix_wall_clock_time_seconds_legacy_compatible(),
        ) {
            Ok(()) => {
                self.session.model.playback.local_paused = Some(paused);
                Ok(())
            }
            Err(error) => {
                self.session.model.playback.local_paused = previous;
                Err(error)
            }
        }
    }
}

fn authoritative_technical_playback_revision(
    session: &ClientSession,
    media_generation: u64,
    reason: Option<TechnicalBlockCause>,
) -> Option<u64> {
    if reason == Some(TechnicalBlockCause::RoomBufferingPolicy)
        && let Some(state_revision) = session
            .playback_barrier_buffering_policy()
            .filter(|policy| policy.media_generation == media_generation)
            .and_then(|policy| policy.state_revision)
    {
        return Some(state_revision);
    }
    session
        .playback_barrier_commit()
        .filter(|commit| commit.media_generation == media_generation)
        .map(|commit| commit.state_revision)
}

#[cfg(test)]
mod tests;
