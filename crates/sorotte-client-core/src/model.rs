use super::*;

#[derive(Debug, Default)]
pub struct ClientModel {
    pub(crate) connection: ConnectionState,
    pub(crate) capabilities: ServerCapabilities,
    pub(crate) room: RoomState,
    pub(crate) playback: PlaybackSyncState,
    pub(crate) playlist: PlaylistState,
    pub(crate) readiness: ReadinessState,
    pub(crate) reconnect: ReconnectState,
    pub(crate) controller: ControllerState,
}

#[derive(Debug, Default)]
pub struct ConnectionState {
    pub(crate) username: Option<String>,
}

#[derive(Debug, Default)]
pub struct ServerCapabilities {
    pub(crate) readiness: Option<bool>,
    pub(crate) set_others_readiness: Option<bool>,
    pub(crate) managed_rooms: Option<bool>,
    pub(crate) shared_playlists: Option<bool>,
    pub(crate) media_match: Option<bool>,
    pub(crate) chat: Option<bool>,
    pub(crate) persistent_rooms: Option<bool>,
    pub(crate) max_username_length: Option<usize>,
    pub(crate) max_room_name_length: Option<usize>,
    pub(crate) max_filename_length: Option<usize>,
}

#[derive(Debug, Default)]
pub struct RoomState {
    pub(crate) name: Option<String>,
    pub(crate) domain: SyncDomain,
    pub(crate) users: BTreeMap<String, ClientUserView>,
    pub(crate) media_match_peer_tiers: BTreeMap<String, MediaMatchTier>,
    pub(crate) known_rooms: BTreeSet<String>,
    pub(crate) playstates: BTreeMap<String, RoomPlaystateView>,
    pub(crate) playstate_updated_at_seconds: BTreeMap<String, f64>,
}

#[derive(Debug, Default)]
pub struct PlaybackSyncState {
    pub(crate) desync_config: DesyncCorrectionConfig,
    pub(crate) speed_changed: bool,
    pub(crate) behind_first_detected_at_seconds: Option<f64>,
    pub(crate) last_paused_on_leave_at_seconds: Option<f64>,
    pub(crate) last_advanced_at_seconds: Option<f64>,
    pub(crate) last_rewound_at_seconds: Option<f64>,
    pub(crate) local_position: Option<f64>,
    pub(crate) local_paused: Option<bool>,
    pub(crate) local_paused_for_cache: Option<bool>,
    pub(crate) local_cache_buffering_percent: Option<f64>,
    pub(crate) pending_cache_room_playstate_resync: bool,
    pub(crate) client_ignoring_on_the_fly: u32,
    pub(crate) server_ignoring_on_the_fly: u32,
    pending_room_pause_sync: Option<PendingRoomPauseSync>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    RoomPauseSyncRequested {
        original_position: Option<f64>,
        target_position: Option<f64>,
        target_paused: Option<bool>,
        clear_cache_resync_on_success: bool,
    },
    EffectSucceeded(ClientEffect),
    EffectFailed(ClientEffect),
}

#[derive(Debug)]
struct PendingRoomPauseSync {
    original_position: Option<f64>,
    target_position: Option<f64>,
    target_paused: Option<bool>,
    clear_cache_resync_on_success: bool,
    stage: RoomPauseSyncStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomPauseSyncStage {
    Position,
    Pause,
    RollbackPosition,
}

impl ClientModel {
    pub fn apply(&mut self, event: ClientEvent) -> Vec<ClientEffect> {
        match event {
            ClientEvent::RoomPauseSyncRequested {
                original_position,
                target_position,
                target_paused,
                clear_cache_resync_on_success,
            } => self.begin_room_pause_sync(
                original_position,
                target_position,
                target_paused,
                clear_cache_resync_on_success,
            ),
            ClientEvent::EffectSucceeded(effect) => {
                self.apply_room_pause_sync_effect_succeeded(effect)
            }
            ClientEvent::EffectFailed(effect) => self.apply_room_pause_sync_effect_failed(effect),
        }
    }

    fn begin_room_pause_sync(
        &mut self,
        original_position: Option<f64>,
        target_position: Option<f64>,
        target_paused: Option<bool>,
        clear_cache_resync_on_success: bool,
    ) -> Vec<ClientEffect> {
        if self.playback.pending_room_pause_sync.is_some() {
            return Vec::new();
        }
        let Some(first_effect) = target_position
            .map(ClientEffect::SetPlayerPosition)
            .or_else(|| target_paused.map(ClientEffect::SetPlayerPaused))
        else {
            return Vec::new();
        };
        let stage = if target_position.is_some() {
            RoomPauseSyncStage::Position
        } else {
            RoomPauseSyncStage::Pause
        };
        self.playback.pending_room_pause_sync = Some(PendingRoomPauseSync {
            original_position,
            target_position,
            target_paused,
            clear_cache_resync_on_success,
            stage,
        });
        vec![first_effect]
    }

    fn apply_room_pause_sync_effect_succeeded(
        &mut self,
        effect: ClientEffect,
    ) -> Vec<ClientEffect> {
        let Some(pending) = self.playback.pending_room_pause_sync.as_mut() else {
            return Vec::new();
        };
        match (pending.stage, effect) {
            (RoomPauseSyncStage::Position, ClientEffect::SetPlayerPosition(position))
                if pending.target_position == Some(position) =>
            {
                self.playback.local_position = Some(position);
                if let Some(paused) = pending.target_paused {
                    pending.stage = RoomPauseSyncStage::Pause;
                    vec![ClientEffect::SetPlayerPaused(paused)]
                } else {
                    self.finish_room_pause_sync(true);
                    Vec::new()
                }
            }
            (RoomPauseSyncStage::Pause, ClientEffect::SetPlayerPaused(paused))
                if pending.target_paused == Some(paused) =>
            {
                self.playback.local_paused = Some(paused);
                self.finish_room_pause_sync(true);
                Vec::new()
            }
            (RoomPauseSyncStage::RollbackPosition, ClientEffect::SetPlayerPosition(position))
                if pending.original_position == Some(position) =>
            {
                self.playback.local_position = Some(position);
                self.finish_room_pause_sync(false);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn apply_room_pause_sync_effect_failed(&mut self, effect: ClientEffect) -> Vec<ClientEffect> {
        let Some(pending) = self.playback.pending_room_pause_sync.as_mut() else {
            return Vec::new();
        };
        match (pending.stage, effect) {
            (RoomPauseSyncStage::Position, ClientEffect::SetPlayerPosition(position))
                if pending.target_position == Some(position) =>
            {
                self.finish_room_pause_sync(false);
                Vec::new()
            }
            (RoomPauseSyncStage::Pause, ClientEffect::SetPlayerPaused(paused))
                if pending.target_paused == Some(paused) =>
            {
                if let Some(original_position) = pending
                    .original_position
                    .filter(|value| value.is_finite() && pending.target_position != Some(*value))
                {
                    pending.stage = RoomPauseSyncStage::RollbackPosition;
                    vec![ClientEffect::SetPlayerPosition(original_position)]
                } else {
                    self.finish_room_pause_sync(false);
                    Vec::new()
                }
            }
            (RoomPauseSyncStage::RollbackPosition, ClientEffect::SetPlayerPosition(position))
                if pending.original_position == Some(position) =>
            {
                self.finish_room_pause_sync(false);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn finish_room_pause_sync(&mut self, succeeded: bool) {
        let Some(pending) = self.playback.pending_room_pause_sync.take() else {
            return;
        };
        if succeeded && pending.clear_cache_resync_on_success {
            self.playback.pending_cache_room_playstate_resync = false;
        }
    }
}

#[derive(Debug, Default)]
pub struct PlaylistState {
    pub(crate) rooms: BTreeMap<String, RoomPlaylistView>,
    pub(crate) pending: Option<RoomPlaylistView>,
    pub(crate) active_targets_before_index_update: BTreeMap<String, String>,
    pub(crate) undo_snapshots: BTreeMap<String, Vec<String>>,
    pub(crate) shuffle_nonce: u64,
    pub(crate) received_first_index: bool,
    pub(crate) pending_index_reset_pause_before_sync: Option<bool>,
    pub(crate) pending_index_reset_refresh_recently_advanced: bool,
    pub(crate) suppress_next_self_index_reset: bool,
    pub(crate) last_seek_position_before_manual_seek: Option<f64>,
}

#[derive(Debug)]
pub struct ReadinessState {
    pub(crate) config: ReadinessAutoplayConfig,
    pub(crate) autoplay_enabled: bool,
    pub(crate) autoplay_timer_running: bool,
    pub(crate) autoplay_time_left_seconds: f64,
}

impl Default for ReadinessState {
    fn default() -> Self {
        let config = ReadinessAutoplayConfig::default();
        let autoplay_time_left_seconds = config.autoplay_delay_seconds;
        Self {
            config,
            autoplay_enabled: false,
            autoplay_timer_running: false,
            autoplay_time_left_seconds,
        }
    }
}

#[derive(Debug, Default)]
pub struct ReconnectState {
    pub(crate) policy: ReconnectPolicyConfig,
    pub(crate) ready_restore_snapshot: Option<bool>,
    pub(crate) ready_restore_intent: Option<bool>,
    pub(crate) file_restore_snapshot: Option<FilePayload>,
    pub(crate) file_restore_intent: Option<FilePayload>,
    pub(crate) controller_restore_snapshot: Option<bool>,
    pub(crate) playlist_restore_snapshot: Option<ReconnectPlaylistRestoreIntent>,
    pub(crate) playlist_restore_intent: Option<ReconnectPlaylistRestoreIntent>,
    pub(crate) state_restore_validation_pending: bool,
    pub(crate) state_restore_validation_retry_attempts: u32,
    pub(crate) state_restore_validation_retry_cooldown_ticks: u32,
    pub(crate) state_restore_validation_mismatch_notified: bool,
    pub(crate) state_restore_validation_mismatch_seen_in_cycle: bool,
    pub(crate) state_restore_correction_consecutive_mismatch_cycles: u32,
    pub(crate) state_restore_correction_consecutive_retry_exhaustions: u32,
    pub(crate) state_restore_correction_recovery_cooldown_reconnect_cycles_remaining: u32,
    pub(crate) state_restore_correction_recovery_suppressed_this_cycle: bool,
    pub(crate) state_restore_correction_recovery_reenable_notification_pending: bool,
    pub(crate) state_restore_correction_recovery_reenabled_this_cycle: bool,
    pub(crate) state_restore_correction_metrics: ReconnectStateRestoreCorrectionMetrics,
    pub(crate) in_progress: bool,
    pub(crate) connected_intent: bool,
}

#[derive(Default)]
pub struct ControllerState {
    pub(crate) controlled_room_switch_intent: Option<String>,
    pub(crate) pending_local_room_switch_target: Option<String>,
    pub(crate) reidentify_intent: Option<(String, String)>,
    pub(crate) last_auth_password_attempt: Option<String>,
    pub(crate) room_passwords: BTreeMap<String, String>,
}

impl std::fmt::Debug for ControllerState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reidentify_room = self
            .reidentify_intent
            .as_ref()
            .map(|(room, _password)| room);
        let password_rooms = self.room_passwords.keys().collect::<Vec<_>>();
        formatter
            .debug_struct("ControllerState")
            .field(
                "controlled_room_switch_intent",
                &self.controlled_room_switch_intent,
            )
            .field(
                "pending_local_room_switch_target",
                &self.pending_local_room_switch_target,
            )
            .field("reidentify_room", &reidentify_room)
            .field(
                "has_last_auth_password_attempt",
                &self.last_auth_password_attempt.is_some(),
            )
            .field("password_rooms", &password_rooms)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_pause_sync_reducer_sequences_failure_compensation() {
        let mut model = ClientModel::default();
        model.playback.local_position = Some(3.0);

        assert_eq!(
            model.apply(ClientEvent::RoomPauseSyncRequested {
                original_position: Some(3.0),
                target_position: Some(12.5),
                target_paused: Some(true),
                clear_cache_resync_on_success: false,
            }),
            vec![ClientEffect::SetPlayerPosition(12.5)]
        );
        assert!(
            model
                .apply(ClientEvent::RoomPauseSyncRequested {
                    original_position: Some(3.0),
                    target_position: Some(20.0),
                    target_paused: None,
                    clear_cache_resync_on_success: false,
                })
                .is_empty(),
            "a reducer transaction must not be replaced while an effect is in flight"
        );
        assert_eq!(
            model.apply(ClientEvent::EffectSucceeded(
                ClientEffect::SetPlayerPosition(12.5)
            )),
            vec![ClientEffect::SetPlayerPaused(true)]
        );
        assert_eq!(model.playback.local_position, Some(12.5));
        assert_eq!(
            model.apply(ClientEvent::EffectFailed(ClientEffect::SetPlayerPaused(
                true
            ))),
            vec![ClientEffect::SetPlayerPosition(3.0)]
        );
        assert!(
            model
                .apply(ClientEvent::EffectSucceeded(
                    ClientEffect::SetPlayerPosition(3.0)
                ))
                .is_empty()
        );
        assert_eq!(model.playback.local_position, Some(3.0));
    }

    #[test]
    fn controller_state_debug_redacts_stored_passwords() {
        let mut model = ClientModel::default();
        model.controller.reidentify_intent = Some((
            "+room:ABCDEF123456".to_owned(),
            "reidentify-secret".to_owned(),
        ));
        model.controller.last_auth_password_attempt = Some("attempt-secret".to_owned());
        model
            .controller
            .room_passwords
            .insert("+room:ABCDEF123456".to_owned(), "stored-secret".to_owned());

        let debug = format!("{model:?}");
        assert!(!debug.contains("reidentify-secret"));
        assert!(!debug.contains("attempt-secret"));
        assert!(!debug.contains("stored-secret"));
    }
}
