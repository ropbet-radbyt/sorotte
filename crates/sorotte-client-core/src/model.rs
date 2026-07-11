use super::*;

#[derive(Debug, Default)]
pub struct ClientModel {
    pub(crate) connection: ConnectionState,
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
    pub(crate) phase: ConnectionPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionPhase {
    #[default]
    Disconnected,
    Connecting,
    AwaitingHello,
    Active(ServerCapabilities),
    Reconnecting {
        attempt: u32,
    },
    Closing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCapabilities {
    pub chat: bool,
    pub readiness: bool,
    pub remote_readiness: bool,
    pub shared_playlists: bool,
    pub managed_rooms: bool,
    pub media_match: bool,
    pub plex_playlist_uris: bool,
    pub persistent_rooms: bool,
    pub max_username_length: usize,
    pub max_room_name_length: usize,
    pub max_filename_length: usize,
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
    pub(crate) local_playback_rate: Option<f64>,
    pub(crate) local_paused_for_cache: Option<bool>,
    pub(crate) local_cache_buffering_percent: Option<f64>,
    pub(crate) pending_cache_room_playstate_resync: bool,
    pub(crate) client_ignoring_on_the_fly: u32,
    pub(crate) server_ignoring_on_the_fly: u32,
    pending_room_pause_sync: Option<PendingRoomPauseSync>,
    pending_local_pause_change: Option<PendingLocalPauseChange>,
    pub(crate) local_pause_change_health: LocalPauseChangeHealth,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    LocalPauseChangeRequested {
        original_paused: Option<bool>,
        original_ready: Option<bool>,
        original_last_paused_on_leave_at_seconds: Option<f64>,
        planned_paused: Option<bool>,
        planned_ready: Option<bool>,
        planned_last_paused_on_leave_at_seconds: Option<f64>,
        effects: Vec<ClientEffect>,
    },
    RoomPauseSyncRequested {
        original_position: Option<f64>,
        target_position: Option<f64>,
        target_paused: Option<bool>,
        clear_cache_resync_on_success: bool,
    },
    EffectSucceeded(ClientEffect),
    EffectFailed(ClientEffect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalPauseChangeHealth {
    #[default]
    Healthy,
    ControlEffectFailedAfterPlayerChange,
}

#[derive(Debug)]
struct PendingLocalPauseChange {
    original: LocalPauseStateSnapshot,
    original_health: LocalPauseChangeHealth,
    effects: Vec<ClientEffect>,
    next_effect_index: usize,
    player_pause_succeeded: bool,
    stage: LocalPauseChangeStage,
}

#[derive(Debug, Clone, Copy)]
struct LocalPauseStateSnapshot {
    paused: Option<bool>,
    ready: Option<bool>,
    last_paused_on_leave_at_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalPauseChangeStage {
    PlayerPause,
    ControlEffects,
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
            ClientEvent::LocalPauseChangeRequested {
                original_paused,
                original_ready,
                original_last_paused_on_leave_at_seconds,
                planned_paused,
                planned_ready,
                planned_last_paused_on_leave_at_seconds,
                effects,
            } => self.begin_local_pause_change(
                LocalPauseStateSnapshot {
                    paused: original_paused,
                    ready: original_ready,
                    last_paused_on_leave_at_seconds: original_last_paused_on_leave_at_seconds,
                },
                LocalPauseStateSnapshot {
                    paused: planned_paused,
                    ready: planned_ready,
                    last_paused_on_leave_at_seconds: planned_last_paused_on_leave_at_seconds,
                },
                effects,
            ),
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
                if self.local_pause_change_expects(&effect) {
                    self.apply_local_pause_change_effect_succeeded(effect)
                } else {
                    self.apply_room_pause_sync_effect_succeeded(effect)
                }
            }
            ClientEvent::EffectFailed(effect) => {
                if self.local_pause_change_expects(&effect) {
                    self.apply_local_pause_change_effect_failed(effect)
                } else {
                    self.apply_room_pause_sync_effect_failed(effect)
                }
            }
        }
    }

    pub(crate) fn local_pause_change_in_flight(&self) -> bool {
        self.playback.pending_local_pause_change.is_some()
    }

    pub(crate) fn apply_local_pause_state(
        &mut self,
        paused: Option<bool>,
        ready: Option<bool>,
        last_paused_on_leave_at_seconds: Option<f64>,
    ) {
        self.playback.local_paused = paused;
        self.playback.last_paused_on_leave_at_seconds = last_paused_on_leave_at_seconds;
        self.restore_local_ready_state(ready);
    }

    fn begin_local_pause_change(
        &mut self,
        original: LocalPauseStateSnapshot,
        planned: LocalPauseStateSnapshot,
        effects: Vec<ClientEffect>,
    ) -> Vec<ClientEffect> {
        if self.playback.pending_local_pause_change.is_some()
            || !Self::valid_local_pause_effect_sequence(&effects)
        {
            return Vec::new();
        }
        let stage = if matches!(effects.first(), Some(ClientEffect::SetPlayerPaused(_))) {
            LocalPauseChangeStage::PlayerPause
        } else {
            LocalPauseChangeStage::ControlEffects
        };
        let first_effect = effects[0].clone();
        self.apply_local_pause_state(
            planned.paused,
            planned.ready,
            planned.last_paused_on_leave_at_seconds,
        );
        self.playback.pending_local_pause_change = Some(PendingLocalPauseChange {
            original,
            original_health: self.playback.local_pause_change_health,
            effects,
            next_effect_index: 0,
            player_pause_succeeded: false,
            stage,
        });
        vec![first_effect]
    }

    fn valid_local_pause_effect_sequence(effects: &[ClientEffect]) -> bool {
        matches!(
            effects,
            [ClientEffect::SetPlayerPaused(_)]
                | [
                    ClientEffect::SetPlayerPaused(_),
                    ClientEffect::SetReady { .. }
                ]
                | [ClientEffect::SetReady { .. }]
        )
    }

    fn local_pause_change_expects(&self, effect: &ClientEffect) -> bool {
        self.playback
            .pending_local_pause_change
            .as_ref()
            .and_then(|pending| pending.effects.get(pending.next_effect_index))
            == Some(effect)
    }

    fn apply_local_pause_change_effect_succeeded(
        &mut self,
        effect: ClientEffect,
    ) -> Vec<ClientEffect> {
        if !self.local_pause_change_expects(&effect) {
            return Vec::new();
        }
        let mut pending = self
            .playback
            .pending_local_pause_change
            .take()
            .expect("matching local pause effect requires a pending transaction");
        if let ClientEffect::SetPlayerPaused(paused) = effect {
            pending.player_pause_succeeded = true;
            self.playback.local_paused = Some(paused);
        }
        pending.next_effect_index += 1;
        let Some(next_effect) = pending.effects.get(pending.next_effect_index).cloned() else {
            self.playback.local_pause_change_health = LocalPauseChangeHealth::Healthy;
            return Vec::new();
        };
        pending.stage = LocalPauseChangeStage::ControlEffects;
        self.playback.pending_local_pause_change = Some(pending);
        vec![next_effect]
    }

    fn apply_local_pause_change_effect_failed(
        &mut self,
        effect: ClientEffect,
    ) -> Vec<ClientEffect> {
        if !self.local_pause_change_expects(&effect) {
            return Vec::new();
        }
        let pending = self
            .playback
            .pending_local_pause_change
            .take()
            .expect("matching local pause effect requires a pending transaction");
        self.restore_local_ready_state(pending.original.ready);
        match pending.stage {
            LocalPauseChangeStage::PlayerPause => {
                self.playback.local_paused = pending.original.paused;
                self.playback.last_paused_on_leave_at_seconds =
                    pending.original.last_paused_on_leave_at_seconds;
                self.playback.local_pause_change_health = pending.original_health;
            }
            LocalPauseChangeStage::ControlEffects if pending.player_pause_succeeded => {
                self.playback.local_pause_change_health =
                    LocalPauseChangeHealth::ControlEffectFailedAfterPlayerChange;
            }
            LocalPauseChangeStage::ControlEffects => {
                self.playback.local_paused = pending.original.paused;
                self.playback.last_paused_on_leave_at_seconds =
                    pending.original.last_paused_on_leave_at_seconds;
                self.playback.local_pause_change_health = pending.original_health;
            }
        }
        Vec::new()
    }

    fn restore_local_ready_state(&mut self, ready: Option<bool>) {
        let Some(username) = self.connection.username.clone() else {
            return;
        };
        let room_name = {
            let user = self.room.users.entry(username.clone()).or_default();
            user.ready = ready;
            user.room.clone()
        };
        if let Some(room_name) = room_name {
            self.room
                .domain
                .join_room_with_ready(&username, &room_name, ready);
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

#[derive(Default)]
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

impl std::fmt::Debug for PlaylistState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaylistState")
            .field("rooms", &self.rooms)
            .field("pending", &self.pending)
            .field(
                "active_targets_before_index_update_count",
                &self.active_targets_before_index_update.len(),
            )
            .field("undo_snapshot_count", &self.undo_snapshots.len())
            .field("shuffle_nonce", &self.shuffle_nonce)
            .field("received_first_index", &self.received_first_index)
            .field(
                "pending_index_reset_pause_before_sync",
                &self.pending_index_reset_pause_before_sync,
            )
            .field(
                "pending_index_reset_refresh_recently_advanced",
                &self.pending_index_reset_refresh_recently_advanced,
            )
            .field(
                "suppress_next_self_index_reset",
                &self.suppress_next_self_index_reset,
            )
            .field(
                "last_seek_position_before_manual_seek",
                &self.last_seek_position_before_manual_seek,
            )
            .finish()
    }
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
    pub(crate) file_restore_snapshot: Option<SharedFile>,
    pub(crate) file_restore_intent: Option<SharedFile>,
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
    pub(crate) reidentify_intent: Option<(String, SecretValue)>,
    pub(crate) last_auth_password_attempt: Option<SecretValue>,
    pub(crate) room_passwords: BTreeMap<String, SecretValue>,
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
    fn local_pause_reducer_keeps_player_truth_and_restores_readiness_after_control_failure() {
        let mut model = ClientModel::default();
        model.connection.username = Some("alice".to_owned());
        model.playback.local_paused = Some(false);
        model.playback.last_paused_on_leave_at_seconds = Some(12.0);
        model.room.users.insert(
            "alice".to_owned(),
            ClientUserView {
                room: Some("room1".to_owned()),
                ready: Some(true),
                ..ClientUserView::default()
            },
        );
        model
            .room
            .domain
            .join_room_with_ready("alice", "room1", Some(true));
        let effects = vec![
            ClientEffect::SetPlayerPaused(true),
            ClientEffect::SetReady {
                ready: false,
                manually_initiated: false,
            },
        ];

        assert_eq!(
            model.apply(ClientEvent::LocalPauseChangeRequested {
                original_paused: Some(false),
                original_ready: Some(true),
                original_last_paused_on_leave_at_seconds: Some(12.0),
                planned_paused: Some(true),
                planned_ready: Some(false),
                planned_last_paused_on_leave_at_seconds: None,
                effects: effects.clone(),
            }),
            vec![ClientEffect::SetPlayerPaused(true)]
        );
        assert_eq!(model.playback.local_paused, Some(true));
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, None);
        assert_eq!(
            model.room.users.get("alice").and_then(|user| user.ready),
            Some(false),
            "the request event, rather than the planner, must apply optimistic state"
        );
        assert!(
            model
                .apply(ClientEvent::LocalPauseChangeRequested {
                    original_paused: Some(false),
                    original_ready: Some(true),
                    original_last_paused_on_leave_at_seconds: Some(12.0),
                    planned_paused: Some(false),
                    planned_ready: Some(true),
                    planned_last_paused_on_leave_at_seconds: Some(99.0),
                    effects: vec![ClientEffect::SetPlayerPaused(false)],
                })
                .is_empty(),
            "a second pause transaction must be rejected while the first effect is in flight"
        );
        assert_eq!(model.playback.local_paused, Some(true));
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, None);
        assert_eq!(
            model.apply(ClientEvent::EffectSucceeded(effects[0].clone())),
            vec![effects[1].clone()]
        );
        assert!(
            model
                .apply(ClientEvent::EffectFailed(effects[1].clone()))
                .is_empty()
        );

        assert_eq!(model.playback.local_paused, Some(true));
        assert_eq!(
            model.room.users.get("alice").and_then(|user| user.ready),
            Some(true),
            "the failed control effect must roll back only optimistic readiness"
        );
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, None);
        assert_eq!(
            model.playback.local_pause_change_health,
            LocalPauseChangeHealth::ControlEffectFailedAfterPlayerChange
        );
        assert!(!model.local_pause_change_in_flight());
    }

    #[test]
    fn local_pause_reducer_restores_optimistic_state_when_player_effect_fails() {
        let mut model = ClientModel::default();
        model.connection.username = Some("alice".to_owned());
        model.playback.local_paused = Some(false);
        model.playback.last_paused_on_leave_at_seconds = Some(8.0);
        model.room.users.insert(
            "alice".to_owned(),
            ClientUserView {
                room: Some("room1".to_owned()),
                ready: Some(true),
                ..ClientUserView::default()
            },
        );
        let player_effect = ClientEffect::SetPlayerPaused(true);

        assert_eq!(
            model.apply(ClientEvent::LocalPauseChangeRequested {
                original_paused: Some(false),
                original_ready: Some(true),
                original_last_paused_on_leave_at_seconds: Some(8.0),
                planned_paused: Some(true),
                planned_ready: Some(false),
                planned_last_paused_on_leave_at_seconds: None,
                effects: vec![player_effect.clone()],
            }),
            vec![player_effect.clone()]
        );
        assert_eq!(model.playback.local_paused, Some(true));
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, None);
        assert_eq!(
            model.room.users.get("alice").and_then(|user| user.ready),
            Some(false)
        );
        assert!(
            model
                .apply(ClientEvent::EffectFailed(player_effect))
                .is_empty()
        );

        assert_eq!(model.playback.local_paused, Some(false));
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, Some(8.0));
        assert_eq!(
            model.room.users.get("alice").and_then(|user| user.ready),
            Some(true)
        );
        assert_eq!(
            model.playback.local_pause_change_health,
            LocalPauseChangeHealth::Healthy
        );
        assert!(!model.local_pause_change_in_flight());
    }

    #[test]
    fn local_pause_reducer_rejects_multiple_control_effects_without_applying_plan() {
        let mut model = ClientModel::default();
        model.connection.username = Some("alice".to_owned());
        model.playback.local_paused = Some(false);
        model.playback.last_paused_on_leave_at_seconds = Some(8.0);
        model.room.users.insert(
            "alice".to_owned(),
            ClientUserView {
                room: Some("room1".to_owned()),
                ready: Some(true),
                ..ClientUserView::default()
            },
        );

        assert!(
            model
                .apply(ClientEvent::LocalPauseChangeRequested {
                    original_paused: Some(false),
                    original_ready: Some(true),
                    original_last_paused_on_leave_at_seconds: Some(8.0),
                    planned_paused: Some(true),
                    planned_ready: Some(false),
                    planned_last_paused_on_leave_at_seconds: None,
                    effects: vec![
                        ClientEffect::SetReady {
                            ready: false,
                            manually_initiated: false,
                        },
                        ClientEffect::SetReady {
                            ready: true,
                            manually_initiated: false,
                        },
                    ],
                })
                .is_empty()
        );

        assert_eq!(model.playback.local_paused, Some(false));
        assert_eq!(model.playback.last_paused_on_leave_at_seconds, Some(8.0));
        assert_eq!(
            model.room.users.get("alice").and_then(|user| user.ready),
            Some(true)
        );
        assert!(!model.local_pause_change_in_flight());
    }

    #[test]
    fn controller_state_debug_redacts_stored_passwords() {
        let mut model = ClientModel::default();
        model.controller.reidentify_intent =
            Some(("+room:ABCDEF123456".to_owned(), "reidentify-secret".into()));
        model.controller.last_auth_password_attempt = Some("attempt-secret".into());
        model
            .controller
            .room_passwords
            .insert("+room:ABCDEF123456".to_owned(), "stored-secret".into());

        let debug = format!("{model:?}");
        assert!(!debug.contains("reidentify-secret"));
        assert!(!debug.contains("attempt-secret"));
        assert!(!debug.contains("stored-secret"));
    }

    #[test]
    fn playlist_state_debug_redacts_tokenized_media_history() {
        const MARKER: &str = "playlist-state-token-canary";
        let target = format!("https://media.example/video?X-Plex-Token={MARKER}");
        let mut model = ClientModel::default();
        model
            .playlist
            .active_targets_before_index_update
            .insert("room".to_owned(), target.clone());
        model
            .playlist
            .undo_snapshots
            .insert("room".to_owned(), vec![target]);

        let debug = format!("{model:?}");

        assert!(!debug.contains(MARKER));
    }
}
