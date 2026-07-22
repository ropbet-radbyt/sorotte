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
    playback_barrier: playback_barrier::ClientPlaybackBarrierState,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientSessionLocalActionSnapshot {
    user_views: BTreeMap<String, ClientUserView>,
    media_match_peer_tiers: BTreeMap<String, MediaMatchTier>,
    local_position: Option<f64>,
    local_paused: Option<bool>,
    local_playback_rate: Option<f64>,
    local_paused_for_cache: Option<bool>,
    local_cache_buffering_percent: Option<f64>,
    pending_cache_room_playstate_resync: bool,
    cache_recovery_observation_position: Option<f64>,
    cache_recovery_waiting_for_post_cache_position: bool,
    last_seek_position_before_manual_seek: Option<f64>,
    last_paused_on_leave_at_seconds: Option<f64>,
    last_rewound_at_seconds: Option<f64>,
    autoplay_timer_running: bool,
    autoplay_time_left_seconds: f64,
}

mod apply;
mod file_metadata;
mod helpers;
mod lifecycle;
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
