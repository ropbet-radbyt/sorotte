use super::*;

#[derive(Debug, Default)]
pub struct ClientSession {
    pub(crate) model: ClientModel,
    behavior_config: SessionBehaviorConfig,
    chat_config: ChatConfig,
    pending_chat_notifications: Vec<ChatNotification>,
    pending_controlled_room_creation_notifications: Vec<ControlledRoomCreationNotification>,
    pending_controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    pending_user_change_notifications: Vec<UserChangeNotification>,
    pending_compatibility_fallbacks: Vec<ClientCompatibilityFallback>,
    pending_playstate_transport_evidence: Option<PendingPlaystateTransportEvidence>,
    playback_barrier: playback_barrier::ClientPlaybackBarrierState,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingPlaystateTransportEvidence {
    room: String,
    transport_revision: u64,
    paused: bool,
    seek_position_seconds: Option<f64>,
    authority_observed_at_seconds: f64,
}

const MAX_PENDING_COMPATIBILITY_FALLBACKS: usize = 128;

impl ClientSession {
    pub(crate) fn retain_compatibility_fallbacks(
        &mut self,
        fallbacks: impl IntoIterator<Item = ClientCompatibilityFallback>,
    ) {
        let remaining = MAX_PENDING_COMPATIBILITY_FALLBACKS
            .saturating_sub(self.pending_compatibility_fallbacks.len());
        self.pending_compatibility_fallbacks.extend(
            fallbacks
                .into_iter()
                .take(remaining)
                .map(ClientCompatibilityFallback::bounded),
        );
    }
}

/// Opaque rollback state for a playback-rate command emitted by desync correction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesyncCorrectionDispatchSnapshot {
    speed_changed: bool,
    speed_correction_rate: Option<f64>,
    local_playback_rate: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientSessionLocalActionSnapshot {
    user_views: BTreeMap<String, ClientUserView>,
    media_match_peer_tiers: BTreeMap<String, MediaMatchTier>,
    local_position: Option<f64>,
    local_paused: Option<bool>,
    local_playback_rate: Option<f64>,
    speed_changed: bool,
    speed_correction_rate: Option<f64>,
    local_paused_for_cache: Option<bool>,
    local_cache_buffering_percent: Option<f64>,
    pending_cache_room_playstate_resync: bool,
    cache_recovery_observation_position: Option<f64>,
    cache_recovery_waiting_for_post_cache_position: bool,
    client_ignoring_on_the_fly: u32,
    last_seek_position_before_manual_seek: Option<f64>,
    last_paused_on_leave_at_seconds: Option<f64>,
    last_rewound_at_seconds: Option<f64>,
    autoplay_timer_running: bool,
    autoplay_time_left_seconds: f64,
}

pub(crate) struct StateReconcileContext {
    pub(crate) local_state_change_global_playstate: Option<RoomPlaystateView>,
    pub(crate) local_pause_mutation_intent: Option<LocalPauseMutationIntent>,
    pub(crate) received_at_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalPauseMutationIntent {
    pub(crate) paused: bool,
    pub(crate) base_transport_revision: Option<u64>,
}

mod apply;
mod file_metadata;
mod helpers;
mod lifecycle;
mod participant_status;
mod playback;
mod playback_barrier;
mod playlist;
mod queries;
mod readiness_v2;
mod reconnect;

pub use playlist::playback_uri_is_trusted_legacy_compatible;

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests;
