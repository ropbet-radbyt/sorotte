use super::*;
use crate::app::runtime_owner::{
    GuiAttachedSystemSeekOwnershipState, GuiAttachedSystemSeekSource,
    GuiPendingAttachedRoomUnpauseObservation,
};

#[derive(Debug, Default)]
struct PositionSessionState {
    local_position_seconds: Option<f64>,
    synchronized_positions: Vec<f64>,
    recorded_manual_seeks: Vec<f64>,
    manual_seek_attempts: Vec<f64>,
    manual_seek_failures_remaining: usize,
    eof_observations: usize,
    transport_updates: Vec<sorotte_player_api::PlayerTransportTelemetryUpdate>,
    playback_coordination_snapshot: Option<sorotte_client_core::PlaybackCoordinationSnapshot>,
    adapter_logical_generations: std::collections::BTreeMap<u64, u64>,
    keep_waiting_calls: usize,
}

struct PositionSession {
    state: std::sync::Arc<std::sync::Mutex<PositionSessionState>>,
}

impl GuiSessionRuntimeAdapter for PositionSession {
    fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
        Ok(())
    }

    fn connect_public_server(
        &mut self,
        _selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        Ok(current_servers)
    }

    fn search_missing_media(
        &mut self,
        _directories: Vec<String>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn sync_local_playback_telemetry(
        &mut self,
        _paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        if let Some(position_seconds) = position_seconds {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.local_position_seconds = Some(position_seconds);
            state.synchronized_positions.push(position_seconds);
        }
        Ok(())
    }

    fn record_manual_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.manual_seek_attempts.push(position_seconds);
        if state.manual_seek_failures_remaining > 0 {
            state.manual_seek_failures_remaining -= 1;
            return Ok(false);
        }
        state.local_position_seconds = Some(position_seconds);
        state.recorded_manual_seeks.push(position_seconds);
        Ok(true)
    }

    fn local_position_seconds(&self) -> Option<f64> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .local_position_seconds
    }

    fn current_room_name(&self) -> Option<&str> {
        Some("room1")
    }

    fn observe_external_player_end_of_file(&mut self, _now_seconds: f64) -> Result<(), String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .eof_observations += 1;
        Ok(())
    }

    fn sync_attached_player_transport_telemetry(
        &mut self,
        update: sorotte_player_api::PlayerTransportTelemetryUpdate,
        _now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .transport_updates
            .push(update);
        Ok(Vec::new())
    }

    fn playback_coordination_snapshot(
        &self,
    ) -> Option<sorotte_client_core::PlaybackCoordinationSnapshot> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .playback_coordination_snapshot
            .clone()
    }

    fn logical_generation_for_adapter_generation(
        &self,
        adapter_generation: sorotte_player_api::PlayerMediaGeneration,
    ) -> Option<u64> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .adapter_logical_generations
            .get(&adapter_generation.get())
            .copied()
    }

    fn keep_waiting_for_seek_preparation(
        &mut self,
        _now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keep_waiting_calls += 1;
        Ok(Vec::new())
    }
}

struct AlternateRoomPositionSession;

impl GuiSessionRuntimeAdapter for AlternateRoomPositionSession {
    fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
        Ok(())
    }

    fn connect_public_server(
        &mut self,
        _selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        Ok(current_servers)
    }

    fn search_missing_media(
        &mut self,
        _directories: Vec<String>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn current_room_name(&self) -> Option<&str> {
        Some("room2")
    }
}

#[derive(Default)]
struct PositionTelemetryPlayer {
    updates: std::collections::VecDeque<sorotte_player_api::PlayerTransportTelemetryUpdate>,
    playback_updates: std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
    command_progress_updates: std::collections::VecDeque<sorotte_player_api::PlayerCommandProgress>,
    local_file_observations:
        std::collections::VecDeque<sorotte_player_api::PlayerLocalFileObservation>,
    media_load_observations:
        std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadObservation>,
    set_position_calls: usize,
    fail_set_position_call: Option<usize>,
    next_command_id: u64,
}

impl PlayerAdapter for PositionTelemetryPlayer {
    fn name(&self) -> &'static str {
        "position-telemetry"
    }

    fn set_position(
        &mut self,
        _position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.set_position_calls += 1;
        if self.fail_set_position_call == Some(self.set_position_calls) {
            return Err(sorotte_player_api::PlayerError::OperationFailed(
                "test set-position failure".to_owned(),
            ));
        }
        Ok(())
    }

    fn execute_tracked(
        &mut self,
        command: sorotte_player_api::PlayerCommand,
    ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
        let sorotte_player_api::PlayerCommand::SetPosition(position_seconds) = command else {
            return Err(sorotte_player_api::PlayerError::Unsupported(
                "test execute_tracked command",
            ));
        };
        self.set_position(position_seconds)?;
        self.next_command_id += 1;
        Ok(sorotte_player_api::PlayerCommandId::new(
            self.next_command_id,
        ))
    }

    fn take_playback_telemetry_update(
        &mut self,
    ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
        self.playback_updates.pop_front()
    }

    fn take_transport_telemetry_update(
        &mut self,
    ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
        self.updates.pop_front()
    }

    fn take_command_progress(&mut self) -> Option<sorotte_player_api::PlayerCommandProgress> {
        self.command_progress_updates.pop_front()
    }

    fn take_local_file_observation(
        &mut self,
    ) -> Option<sorotte_player_api::PlayerLocalFileObservation> {
        self.local_file_observations.pop_front()
    }

    fn take_media_load_observation(
        &mut self,
    ) -> Option<sorotte_player_api::PlayerMediaLoadObservation> {
        self.media_load_observations.pop_front()
    }
}

fn position_update(
    observed_at_seconds: f64,
    position_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    position_update_at_rate(observed_at_seconds, position_seconds, 1.0)
}

fn position_update_at_rate(
    observed_at_seconds: f64,
    position_seconds: f64,
    playback_rate: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    let mut update = sorotte_player_api::PlayerTransportTelemetryUpdate::new(
        sorotte_player_api::PlayerMediaGeneration::new(1),
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
            std::time::Duration::from_secs_f64(observed_at_seconds),
        ),
    )
    .with_phase(sorotte_player_api::PlayerTransportPhase::Playing)
    .with_position_seconds(position_seconds)
    .with_logical_pause(false);
    update.playback_rate = Some(playback_rate);
    update.paused_for_cache = Some(false);
    update.seeking = Some(false);
    update.core_idle = Some(false);
    update
}

fn sparse_update(observed_at_seconds: f64) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    sorotte_player_api::PlayerTransportTelemetryUpdate::new(
        sorotte_player_api::PlayerMediaGeneration::new(1),
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
            std::time::Duration::from_secs_f64(observed_at_seconds),
        ),
    )
}

fn sparse_position_update(
    observed_at_seconds: f64,
    position_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    sparse_update(observed_at_seconds).with_position_seconds(position_seconds)
}

fn position_update_with_delivery(
    observed_at_seconds: f64,
    delivery_reference_seconds: f64,
    position_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    let mut update = position_update(observed_at_seconds, position_seconds);
    update.observed_at = Some(
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_observation(
            std::time::Duration::from_secs_f64(observed_at_seconds),
            std::time::Duration::from_secs_f64(delivery_reference_seconds),
        ),
    );
    update
}

fn seek_transition(
    seeking_at_seconds: f64,
    completed_at_seconds: f64,
    completed_position_seconds: f64,
) -> [sorotte_player_api::PlayerTransportTelemetryUpdate; 2] {
    let mut seeking = sparse_update(seeking_at_seconds);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.seeking = Some(true);
    let mut completed = position_update(completed_at_seconds, completed_position_seconds);
    completed.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    completed.seeking = Some(false);
    [seeking, completed]
}

fn active_seek_preparation_snapshot(
    media_generation: u64,
    target_seconds: f64,
) -> sorotte_client_core::PlaybackCoordinationSnapshot {
    sorotte_client_core::PlaybackCoordinationSnapshot {
        media_generation: Some(media_generation),
        pending_local_pause_intent: None,
        pending_local_pause_intent_dormant: false,
        last_local_pause_intent_stage_accepted: None,
        diagnostic: sorotte_client_core::PlaybackDiagnostic::ReadyWaitingForRoom,
        recovery_episode: None,
        seek_preparation: Some(sorotte_client_core::SeekPreparationSnapshot {
            id: 1,
            media_generation,
            load_attempt: 1,
            room_revision: 1,
            latest_room_revision: 1,
            requested_target_seconds: target_seconds,
            frozen_target_seconds: target_seconds,
            frozen_room_anchor_position_seconds: target_seconds,
            frozen_room_anchor_observed_at_seconds: 0.0,
            latest_room_position_seconds: target_seconds,
            availability: sorotte_client_core::SeekTargetAvailability::FetchRequired,
            phase: sorotte_client_core::SeekPreparationPhase::Fetching,
            cache_buffering_percent: None,
            buffered_ahead_seconds: None,
            nearest_safe_buffered_position_seconds: None,
            started_at_seconds: 0.0,
            terminal_outcome: None,
            can_keep_waiting: true,
            can_cancel_and_remain: false,
            can_join_nearest_buffered: false,
        }),
        last_seek_preparation_terminal_outcome: None,
        last_seek_preparation_terminal: None,
        metrics: Default::default(),
        transport_telemetry_observed: true,
        ordinary_correction_blocked: true,
        last_applied_revision: None,
        last_started_revision: None,
        last_degraded_reason: None,
    }
}

#[test]
fn attached_position_telemetry_grounds_normal_progress_and_publishes_native_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update(0.0, 10.0),
        position_update(0.1, 10.1),
        position_update(0.2, 15.1),
    ]);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_paused = Some(false);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.synchronized_positions, vec![10.0, 10.1, 15.1]);
    assert_eq!(state.recorded_manual_seeks, vec![15.1]);
    assert_eq!(state.local_position_seconds, Some(15.1));
}

#[test]
fn attached_position_telemetry_does_not_republish_sorotte_owned_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.push_back(position_update(0.0, 10.0));

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_paused = Some(false);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_position_seconds = Some(20.0);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(PositionTelemetryPlayer {
        updates: std::collections::VecDeque::from([position_update(0.1, 20.0)]),
        ..PositionTelemetryPlayer::default()
    })));
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.recorded_manual_seeks.is_empty());
    assert_eq!(state.local_position_seconds, Some(20.0));
}

#[test]
fn coordinator_seek_completion_is_never_republished_as_a_native_seek() {
    for completed_position_seconds in [39.5, 40.0, 40.5] {
        let session_state =
            std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
        let mut seeking = sparse_update(0.1);
        seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
        seeking.seeking = Some(true);
        let mut seek_complete = position_update_at_rate(0.2, completed_position_seconds, 1.0);
        seek_complete.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
        seek_complete.seeking = Some(false);
        let mut baseline_player = PositionTelemetryPlayer::default();
        baseline_player
            .updates
            .push_back(position_update_at_rate(0.0, 10.0, 1.0));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
        owner.session = Some(Box::new(PositionSession {
            state: session_state.clone(),
        }));
        owner.refresh_player_state_impl();

        let mut seek_player = PositionTelemetryPlayer::default();
        seek_player.updates.extend([seeking, seek_complete]);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(seek_player)));

        assert!(owner.apply_attached_player_runtime_actions_impl(
            vec![GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(1),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
            }],
            "coordinator seek ownership regression",
        ));
        owner.refresh_player_state_impl();

        let state = session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            state.manual_seek_attempts.is_empty(),
            "coordinator completion at {completed_position_seconds} must not enter the manual-seek path"
        );
        assert_eq!(
            state.local_position_seconds,
            Some(completed_position_seconds)
        );
    }
}

#[test]
fn split_mpv_seek_completion_reanchors_before_the_next_stable_position() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    let mut seeking = sparse_update(0.1);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.seeking = Some(true);
    let target_while_seeking = sparse_position_update(0.2, 40.0);
    let mut seeking_finished = sparse_update(0.3);
    seeking_finished.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    seeking_finished.seeking = Some(false);
    let adapter_command_id = sorotte_player_api::PlayerCommandId::new(1);
    let mut seek_player = PositionTelemetryPlayer::default();
    seek_player
        .updates
        .extend([seeking, target_while_seeking, seeking_finished]);
    seek_player.command_progress_updates.push_back(
        sorotte_player_api::PlayerCommandProgress::finished(
            adapter_command_id,
            Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
            Some(
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    std::time::Duration::from_secs_f64(0.3),
                ),
            ),
            Some(40.0),
            sorotte_player_api::PlayerCommandResult::Completed,
        ),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(seek_player)));
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
        }],
        "split mpv completion regression",
    );
    owner.refresh_player_state_impl();
    assert!(
        owner.attached_system_seek_ownership.is_empty(),
        "terminal observed position should safely re-anchor and retire ownership"
    );

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(PositionTelemetryPlayer {
        updates: std::collections::VecDeque::from([position_update(0.4, 40.1)]),
        ..PositionTelemetryPlayer::default()
    })));
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        state.manual_seek_attempts.is_empty(),
        "the first stable post-completion sample must not become a manual seek"
    );
    assert_eq!(state.synchronized_positions, vec![10.0, 40.1]);
}

#[test]
fn superseded_coordinator_seek_effects_remain_owned_after_replacement_dispatch() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    let mut seek_player = PositionTelemetryPlayer::default();
    seek_player.updates.extend(seek_transition(0.1, 0.2, 40.0));
    seek_player.updates.extend(seek_transition(0.3, 0.4, 20.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(seek_player)));

    owner.apply_attached_player_runtime_actions_impl(
        vec![
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(1),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
            },
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(2),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(20.0),
            },
        ],
        "superseded coordinator seek regression",
    );

    assert_eq!(owner.attached_system_seek_ownership.len(), 2);
    assert_eq!(
        owner.attached_system_seek_ownership[0].state,
        GuiAttachedSystemSeekOwnershipState::SupersededMayArrive
    );
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(state.synchronized_positions, vec![10.0, 40.0, 20.0]);
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn direct_runtime_position_preserves_older_coordinator_effect_ownership() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.apply_attached_player_runtime_actions_impl(
        vec![
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(1),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
            },
            GuiAttachedPlayerRuntimeAction::Position(20.0),
        ],
        "coordinator then direct position regression",
    );
    assert_eq!(owner.attached_system_seek_ownership.len(), 2);
    assert_eq!(
        owner.attached_system_seek_ownership[0].state,
        GuiAttachedSystemSeekOwnershipState::SupersededMayArrive
    );

    let mut effects_player = PositionTelemetryPlayer::default();
    effects_player
        .updates
        .extend(seek_transition(0.1, 0.2, 40.0));
    effects_player
        .updates
        .extend(seek_transition(0.3, 0.4, 20.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(effects_player)));
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        state.manual_seek_attempts.is_empty(),
        "late coordinator effect A must stay owned after direct system seek B"
    );
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn ownership_pressure_fails_closed_instead_of_reclassifying_a_system_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.apply_attached_player_runtime_actions_impl(
        (1..=9)
            .map(|index| GuiAttachedPlayerRuntimeAction::Position(index as f64 * 10.0))
            .collect(),
        "system seek ledger pressure regression",
    );
    assert_eq!(owner.attached_system_seek_ownership.len(), 8);
    assert!(owner.attached_system_seek_fail_closed.is_some());
    owner
        .attached_system_seek_fail_closed
        .as_mut()
        .expect("ledger pressure should install a fail-closed guard")
        .retire_after = std::time::Instant::now() + std::time::Duration::from_secs(1);
    owner.reconcile_attached_system_seek_command_progress(
        sorotte_player_api::PlayerCommandProgress::finished(
            sorotte_player_api::PlayerCommandId::new(9),
            Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
            None,
            None,
            sorotte_player_api::PlayerCommandResult::Failed(
                sorotte_player_api::PlayerCommandFailureKind::TimedOut,
            ),
        ),
    );
    assert!(
        owner
            .attached_system_seek_fail_closed
            .as_ref()
            .expect("timed-out unrecorded seek should preserve fail-closed classification")
            .retire_after
            > std::time::Instant::now() + std::time::Duration::from_secs(59)
    );
    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_position_seconds = Some(200.0);

    let mut late_unrecorded_effect = PositionTelemetryPlayer::default();
    late_unrecorded_effect
        .updates
        .extend(seek_transition(0.1, 0.2, 90.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(late_unrecorded_effect)));
    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty(),
        "capacity pressure must suppress ambiguous late effects instead of publishing a user seek"
    );
}

#[test]
fn failed_replacement_dispatch_preserves_the_accepted_seek_ownership() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    let mut seek_player = PositionTelemetryPlayer {
        fail_set_position_call: Some(2),
        ..PositionTelemetryPlayer::default()
    };
    seek_player.updates.extend(seek_transition(0.1, 0.2, 40.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(seek_player)));
    owner.apply_attached_player_runtime_actions_impl(
        vec![
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(1),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
            },
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id: sorotte_client_core::CoordinatorCommandId::new(2),
                command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(20.0),
            },
        ],
        "failed coordinator replacement regression",
    );

    assert_eq!(owner.attached_system_seek_ownership.len(), 1);
    assert_eq!(
        owner.attached_system_seek_ownership[0].source,
        GuiAttachedSystemSeekSource::Coordinator(sorotte_client_core::CoordinatorCommandId::new(1))
    );
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(state.local_position_seconds, Some(40.0));
}

#[test]
fn coordinator_seek_ownership_covers_coordinator_and_adapter_timeout_windows() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
        }],
        "long coordinator seek ownership regression",
    );
    assert!(
        owner.attached_system_seek_ownership[0].retire_after
            > std::time::Instant::now() + std::time::Duration::from_secs(15),
        "ownership must outlive both the ten-second coordinator and fifteen-second mpv windows"
    );

    let mut late_seek_player = PositionTelemetryPlayer::default();
    late_seek_player
        .updates
        .extend(seek_transition(12.0, 12.1, 40.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(late_seek_player)));
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn adapter_timeout_retains_seek_ownership_through_late_preparation_completion() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();
    let adapter_command_id = sorotte_player_api::PlayerCommandId::new(7);
    owner.note_attached_coordinator_seek_dispatched(
        sorotte_client_core::CoordinatorCommandId::new(1),
        Some(adapter_command_id),
        40.0,
        40.0,
    );

    owner.reconcile_attached_system_seek_command_progress(
        sorotte_player_api::PlayerCommandProgress::finished(
            adapter_command_id,
            Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
            Some(
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    std::time::Duration::from_secs_f64(15.0),
                ),
            ),
            Some(10.0),
            sorotte_player_api::PlayerCommandResult::Failed(
                sorotte_player_api::PlayerCommandFailureKind::TimedOut,
            ),
        ),
    );

    assert_eq!(owner.attached_system_seek_ownership.len(), 1);
    assert_eq!(
        owner.attached_system_seek_ownership[0].state,
        GuiAttachedSystemSeekOwnershipState::MayStillArrive
    );
    assert!(
        owner.attached_system_seek_ownership[0].retire_after
            > std::time::Instant::now() + std::time::Duration::from_secs(59)
    );

    let mut late_seek_player = PositionTelemetryPlayer::default();
    late_seek_player
        .updates
        .extend(seek_transition(20.0, 20.1, 40.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(late_seek_player)));
    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty()
    );
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn direct_position_joins_system_ownership_and_session_lifecycle_retires_it() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    let coordinator_seek = GuiAttachedPlayerRuntimeAction::Coordinator {
        command_id: sorotte_client_core::CoordinatorCommandId::new(1),
        command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
    };
    owner.apply_attached_player_runtime_actions_impl(
        vec![coordinator_seek.clone()],
        "direct position lifecycle regression",
    );
    assert_eq!(owner.attached_system_seek_ownership.len(), 1);
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Position(20.0)],
        "direct position lifecycle regression",
    );
    assert_eq!(owner.attached_system_seek_ownership.len(), 2);
    assert_eq!(
        owner.attached_system_seek_ownership[0].state,
        GuiAttachedSystemSeekOwnershipState::SupersededMayArrive
    );
    assert_eq!(
        owner.attached_system_seek_ownership[1].source,
        GuiAttachedSystemSeekSource::RuntimeAction
    );

    owner.apply_attached_player_runtime_actions_impl(
        vec![coordinator_seek.clone()],
        "session replacement lifecycle regression",
    );
    owner.install_session_runtime(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    assert!(owner.attached_system_seek_ownership.is_empty());

    owner.apply_attached_player_runtime_actions_impl(
        vec![coordinator_seek],
        "transport reconnect lifecycle regression",
    );
    owner.clear_session_attached_player_sync_state();
    assert!(owner.attached_system_seek_ownership.is_empty());
    assert_eq!(owner.attached_native_seek_tracker.media_generation, None);
    assert_eq!(
        owner.attached_transport_telemetry_authority,
        Default::default()
    );
}

#[test]
fn room_change_retires_seek_ownership_even_when_the_session_instance_survives() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.session = Some(Box::new(PositionSession {
        state: session_state,
    }));
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
        }],
        "room change lifecycle regression",
    );
    assert_eq!(owner.attached_system_seek_ownership.len(), 1);

    owner.session = Some(Box::new(AlternateRoomPositionSession));
    owner.refresh_player_state_impl();

    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn stale_delivery_timestamp_cannot_publish_or_ground_a_native_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_with_delivery(0.0, 0.0, 10.0),
        position_update_with_delivery(1.0, 5.0, 20.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(
        state.synchronized_positions,
        vec![10.0],
        "the four-second-old position must not be mirrored into the session before core rejects it"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(10.0),
        "stale rich telemetry must not overwrite the GUI's authoritative position"
    );
}

#[test]
fn stale_rich_update_drops_position_but_preserves_cache_lifecycle_fields() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(99.5)
            .with_paused(true),
    );
    let baseline = position_update_with_delivery(0.0, 0.0, 99.0);
    let mut stale = position_update_with_delivery(1.0, 5.0, 99.5);
    stale.logical_pause = Some(true);
    stale.paused_for_cache = Some(true);
    stale.cache_buffering_percent = Some(100.0);
    player.updates.extend([baseline, stale]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.player_local_file =
        Some(sorotte_player_api::LocalFileUpdate::new("episode.mkv").with_duration_seconds(100.0));
    owner.player_paused = Some(false);
    owner.player_paused_for_cache = Some(false);
    owner.player_cache_buffering_percent = Some(25.0);
    owner.pending_attached_room_unpause_observation = Some(
        GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
            baseline_position_seconds: Some(99.0),
        },
    );

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_position_seconds, Some(99.0));
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.player_paused_for_cache, Some(true));
    assert_eq!(owner.player_cache_buffering_percent, Some(100.0));
    assert_eq!(
        owner.pending_attached_room_unpause_observation,
        Some(
            GuiPendingAttachedRoomUnpauseObservation::AwaitingAdvancement {
                baseline_position_seconds: Some(99.0),
            }
        )
    );
    assert!(!owner.playlist_auto_advance_eof_latched);
    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.synchronized_positions, vec![99.0]);
    assert_eq!(state.eof_observations, 0);
}

#[test]
fn rejected_rich_update_does_not_disable_the_legacy_fallback_channel() {
    let mut player = PositionTelemetryPlayer::default();
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(12.0)
            .with_paused(false)
            .with_paused_for_cache(false)
            .with_cache_buffering_percent(37.5),
    );
    player
        .updates
        .push_back(position_update_with_delivery(1.0, 5.0, 99.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert!(!owner.attached_transport_telemetry_authority.position);
    assert_eq!(owner.player_position_seconds, Some(12.0));
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.player_paused_for_cache, Some(false));
    assert_eq!(owner.player_cache_buffering_percent, Some(37.5));
}

#[test]
fn delayed_old_media_generation_cannot_replace_the_authoritative_position() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut current = position_update(2.0, 10.0);
    current.media_generation = Some(sorotte_player_api::PlayerMediaGeneration::new(2));
    let old = position_update(3.0, 50.0);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([current, old]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .synchronized_positions,
        vec![10.0]
    );
    assert_eq!(owner.attached_native_seek_tracker.media_generation, Some(2));
}

#[test]
fn untimestamped_position_disarms_comparison_until_a_fresh_baseline() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut untimestamped = position_update(1.0, 20.0);
    untimestamped.observed_at = None;
    untimestamped.playback_rate = Some(4.0);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update(0.0, 10.0),
        untimestamped,
        sparse_position_update(2.0, 20.1),
        sparse_position_update(3.0, 21.1),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(
        state.synchronized_positions,
        vec![10.0, 20.1, 21.1],
        "the untimestamped position and its playback-rate field must be rejected as one observation"
    );
}

#[test]
fn regressing_position_timestamp_is_rejected_instead_of_becoming_the_anchor() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update(2.0, 10.0),
        position_update(1.0, 20.0),
        position_update(3.0, 20.1),
        position_update(4.0, 21.1),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(
        state.synchronized_positions,
        vec![10.0, 20.1, 21.1],
        "the regressed position must not be mirrored or installed as a comparison anchor"
    );
}

#[test]
fn attached_position_telemetry_uses_the_observed_playback_rate() {
    for playback_rate in [0.5, 2.0, 3.0, 4.0] {
        let session_state =
            std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
        let mut player = PositionTelemetryPlayer::default();
        player.updates.extend([
            position_update_at_rate(0.0, 10.0, playback_rate),
            position_update_at_rate(1.0, 10.0 + playback_rate, playback_rate),
        ]);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
        owner.session = Some(Box::new(PositionSession {
            state: session_state.clone(),
        }));

        owner.refresh_player_state_impl();

        assert!(
            session_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recorded_manual_seeks
                .is_empty(),
            "normal {playback_rate}x progress must not be classified as a native seek"
        );
    }
}

#[test]
fn low_rate_forward_seek_is_not_hidden_by_a_two_x_assumption() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 0.5),
        position_update_at_rate(1.0, 11.6, 0.5),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks,
        vec![11.6]
    );
}

#[test]
fn high_rate_backward_seek_is_measured_against_expected_progress() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 4.0),
        position_update_at_rate(1.0, 11.0, 4.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks,
        vec![11.0],
        "at 4x, advancing only one second over one wall-clock second is a three-second rewind"
    );
}

#[test]
fn rate_transition_reanchors_before_normal_four_x_progress() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut rate_transition = sorotte_player_api::PlayerTransportTelemetryUpdate::new(
        sorotte_player_api::PlayerMediaGeneration::new(1),
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
            std::time::Duration::from_secs_f64(0.5),
        ),
    );
    rate_transition.playback_rate = Some(4.0);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        rate_transition,
        position_update_at_rate(1.0, 12.0, 4.0),
        position_update_at_rate(2.0, 16.0, 4.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks
            .is_empty()
    );
}

#[test]
fn coalesced_rate_and_position_transition_is_a_new_anchor() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        position_update_at_rate(1.0, 20.0, 4.0),
        position_update_at_rate(2.0, 24.0, 4.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks
            .is_empty()
    );
}

#[test]
fn pause_transitions_do_not_turn_position_samples_into_seeks() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut paused = position_update_at_rate(1.0, 10.0, 1.0);
    paused.phase = Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused);
    paused.logical_pause = Some(true);
    let mut resumed = position_update_at_rate(2.0, 10.0, 1.0);
    resumed.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    resumed.logical_pause = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        paused,
        resumed,
        position_update_at_rate(3.0, 11.0, 1.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks
            .is_empty()
    );
}

#[test]
fn seeking_suppresses_intermediate_samples_but_publishes_the_completed_native_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut seeking = position_update_at_rate(0.2, 20.0, 1.0);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.seeking = Some(true);
    let mut seek_complete = position_update_at_rate(0.3, 20.0, 1.0);
    seek_complete.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    seek_complete.seeking = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        seeking,
        seek_complete,
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks,
        vec![20.0]
    );
}

#[test]
fn paused_core_idle_seek_publishes_after_seek_completion() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut paused = position_update_at_rate(0.0, 10.0, 1.0);
    paused.phase = Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused);
    paused.logical_pause = Some(true);
    paused.core_idle = Some(true);
    let mut seeking = sparse_update(0.1);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.logical_pause = Some(true);
    seeking.seeking = Some(true);
    seeking.core_idle = Some(true);
    let mut seek_complete = position_update_at_rate(0.2, 20.0, 1.0);
    seek_complete.phase = Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused);
    seek_complete.logical_pause = Some(true);
    seek_complete.seeking = Some(false);
    seek_complete.core_idle = Some(true);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([paused, seeking, seek_complete]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks,
        vec![20.0]
    );
}

#[test]
fn loading_and_unknown_rate_samples_only_reestablish_a_baseline() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut loading = position_update_at_rate(0.5, 30.0, 1.0);
    loading.phase = Some(sorotte_player_api::PlayerTransportPhase::Loading);
    let mut invalid_rate = position_update_at_rate(1.0, 30.0, 1.0);
    invalid_rate.playback_rate = Some(0.0);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        loading,
        invalid_rate,
        position_update_at_rate(2.0, 31.0, 1.0),
        position_update_at_rate(3.0, 32.0, 1.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks
            .is_empty()
    );
}

#[test]
fn rejected_native_seek_publication_retries_from_the_previous_anchor() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState {
        manual_seek_failures_remaining: 1,
        ..PositionSessionState::default()
    }));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        position_update_at_rate(0.1, 15.0, 1.0),
        position_update_at_rate(0.2, 15.1, 1.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.manual_seek_attempts, vec![15.0, 15.1]);
    assert_eq!(state.recorded_manual_seeks, vec![15.1]);
    assert_eq!(state.local_position_seconds, Some(15.1));
}

#[test]
fn rejected_coalesced_seek_completion_retries_with_current_transport_state() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState {
        manual_seek_failures_remaining: 1,
        ..PositionSessionState::default()
    }));
    let mut seeking = sparse_update(0.1);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.seeking = Some(true);
    let mut seek_complete = position_update_at_rate(0.2, 20.0, 1.0);
    seek_complete.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    seek_complete.seeking = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        seeking,
        seek_complete,
        sparse_position_update(0.3, 20.1),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.manual_seek_attempts, vec![20.0, 20.1]);
    assert_eq!(state.recorded_manual_seeks, vec![20.1]);
    assert_eq!(state.local_position_seconds, Some(20.1));
}

#[test]
fn sparse_pause_seek_and_loading_sequences_use_transport_ordered_state() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut pause = sparse_update(0.2);
    pause.phase = Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused);
    pause.logical_pause = Some(true);
    let mut play = sparse_update(0.4);
    play.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    play.logical_pause = Some(false);
    let mut seeking = sparse_update(1.6);
    seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    seeking.seeking = Some(true);
    let mut seek_complete = sparse_update(1.8);
    seek_complete.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    seek_complete.seeking = Some(false);
    let mut loading = sparse_update(2.0);
    loading.phase = Some(sorotte_player_api::PlayerTransportPhase::Loading);
    let mut loaded = sparse_update(2.2);
    loaded.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update_at_rate(0.0, 10.0, 1.0),
        pause,
        sparse_position_update(0.3, 10.0),
        play,
        sparse_position_update(0.5, 10.1),
        sparse_position_update(1.5, 11.1),
        seeking,
        sparse_position_update(1.7, 20.0),
        seek_complete,
        sparse_position_update(1.9, 20.0),
        loading,
        sparse_position_update(2.1, 30.0),
        loaded,
        sparse_position_update(2.3, 30.0),
        sparse_position_update(3.3, 31.0),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.recorded_manual_seeks, vec![20.0]);
    assert_eq!(
        state
            .synchronized_positions
            .iter()
            .filter(|position| **position == 20.0)
            .count(),
        1,
        "the intermediate Seeking position must not be grounded"
    );
    assert_eq!(
        state
            .synchronized_positions
            .iter()
            .filter(|position| **position == 30.0)
            .count(),
        1,
        "the intermediate Loading position must not be grounded"
    );
}

#[test]
fn never_known_playback_rate_does_not_infer_a_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut first = sparse_position_update(0.0, 10.0);
    first.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    first.logical_pause = Some(false);
    first.paused_for_cache = Some(false);
    first.seeking = Some(false);
    first.core_idle = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player
        .updates
        .extend([first, sparse_position_update(0.1, 20.0)]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recorded_manual_seeks
            .is_empty()
    );
}

#[test]
fn same_pump_completion_reanchors_before_the_following_stable_position() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PositionTelemetryPlayer::default(),
    )));
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
        }],
        "same-pump completion ordering regression",
    );
    let adapter_command_id = owner.attached_system_seek_ownership[0]
        .adapter_player_command_id
        .expect("tracked seek command");

    let mut while_seeking = sparse_position_update(0.2, 40.0);
    while_seeking.phase = Some(sorotte_player_api::PlayerTransportPhase::Seeking);
    while_seeking.seeking = Some(true);
    let mut seek_finished = sparse_update(0.3);
    seek_finished.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    seek_finished.seeking = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        while_seeking,
        seek_finished,
        sparse_position_update(0.4, 41.0),
    ]);
    player
        .command_progress_updates
        .push_back(sorotte_player_api::PlayerCommandProgress::finished(
            adapter_command_id,
            Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
            Some(
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    std::time::Duration::from_secs_f64(0.3),
                ),
            ),
            Some(40.0),
            sorotte_player_api::PlayerCommandResult::Completed,
        ));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(state.local_position_seconds, Some(41.0));
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn repeated_keep_waiting_renews_matching_seek_ownership_past_the_old_deadline() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState {
        playback_coordination_snapshot: Some(active_seek_preparation_snapshot(17, 40.0)),
        adapter_logical_generations: [(1, 17)].into(),
        ..PositionSessionState::default()
    }));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();
    owner.note_attached_coordinator_seek_dispatched(
        sorotte_client_core::CoordinatorCommandId::new(1),
        Some(sorotte_player_api::PlayerCommandId::new(7)),
        40.0,
        40.0,
    );
    assert_eq!(
        owner.attached_system_seek_ownership[0].media_generation,
        Some(1)
    );
    assert_eq!(
        owner.attached_system_seek_ownership[0].logical_media_generation,
        Some(17)
    );
    let old_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    owner.attached_system_seek_ownership[0].retire_after = old_deadline;

    for renewal_offset in [0, 30] {
        owner
            .session
            .as_mut()
            .expect("session")
            .keep_waiting_for_seek_preparation(55.0 + renewal_offset as f64)
            .expect("Keep waiting succeeds");
        owner.extend_attached_system_seek_ownership_after_keep_waiting(
            std::time::Instant::now() + std::time::Duration::from_secs(renewal_offset as u64),
        );
    }
    owner.prune_attached_system_seek_ownership(old_deadline + std::time::Duration::from_millis(1));
    assert_eq!(owner.attached_system_seek_ownership.len(), 1);
    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keep_waiting_calls,
        2
    );

    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend(seek_transition(90.0, 90.1, 40.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty()
    );
    assert!(owner.attached_system_seek_ownership.is_empty());
}

#[test]
fn delayed_lifecycle_fields_survive_queue_dwell_while_stale_position_is_dropped() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let delayed_timestamp = |observed_at_seconds| {
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_observation(
            std::time::Duration::from_secs_f64(observed_at_seconds),
            std::time::Duration::from_secs_f64(10.0),
        )
    };
    let mut paused = sparse_update(1.0).with_position_seconds(100.0);
    paused.observed_at = Some(delayed_timestamp(1.0));
    paused.phase = Some(sorotte_player_api::PlayerTransportPhase::ReadyPaused);
    paused.logical_pause = Some(true);
    paused.paused_for_cache = Some(true);
    paused.cache_buffering_percent = Some(12.5);
    paused.seeking = Some(true);
    let mut resumed = sparse_update(2.0);
    resumed.observed_at = Some(delayed_timestamp(2.0));
    resumed.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    resumed.logical_pause = Some(false);
    resumed.paused_for_cache = Some(false);
    resumed.seeking = Some(false);
    let mut ended = sparse_update(3.0);
    ended.observed_at = Some(delayed_timestamp(3.0));
    ended.phase = Some(sorotte_player_api::PlayerTransportPhase::Ended);
    ended.eof_reached = Some(true);
    ended.demuxer_cache_idle = Some(true);
    ended.buffered_ahead_seconds = Some(0.0);
    let mut failed = sparse_update(4.0);
    failed.observed_at = Some(delayed_timestamp(4.0));
    failed.phase = Some(sorotte_player_api::PlayerTransportPhase::Failed);
    failed.error_kind = Some(sorotte_player_api::PlayerMediaLoadFailureKind::Network);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([paused, resumed, ended, failed]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_position_seconds = Some(7.0);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(owner.player_position_seconds, Some(7.0));
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.player_paused_for_cache, Some(false));
    assert_eq!(state.transport_updates.len(), 4);
    assert_eq!(state.transport_updates[0].position_seconds, None);
    assert_eq!(
        state
            .transport_updates
            .iter()
            .filter_map(|update| update.phase)
            .collect::<Vec<_>>(),
        vec![
            sorotte_player_api::PlayerTransportPhase::ReadyPaused,
            sorotte_player_api::PlayerTransportPhase::Playing,
            sorotte_player_api::PlayerTransportPhase::Ended,
            sorotte_player_api::PlayerTransportPhase::Failed,
        ]
    );
    assert_eq!(state.transport_updates[1].seeking, Some(false));
    assert_eq!(state.transport_updates[2].eof_reached, Some(true));
    assert_eq!(
        state.transport_updates[3].error_kind,
        Some(sorotte_player_api::PlayerMediaLoadFailureKind::Network)
    );
}

#[test]
fn old_generation_transport_is_processed_before_the_new_local_file_boundary() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("old.mkv"));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();
    owner.note_attached_runtime_position_dispatched(None, 40.0, 40.0);

    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend(seek_transition(1.0, 1.1, 40.0));
    player
        .local_file_observations
        .push_back(sorotte_player_api::PlayerLocalFileObservation::new(
            sorotte_player_api::LocalFileUpdate::new("new.mkv"),
            Some(sorotte_player_api::PlayerMediaGeneration::new(2)),
            Some(
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    std::time::Duration::from_secs_f64(2.0),
                ),
            ),
        ));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty()
    );
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.attached_native_seek_tracker.media_generation, Some(2));
    assert!(owner.attached_system_seek_ownership.is_empty());
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("new.mkv")
    );
}

#[test]
fn seek_ownership_matches_raw_player_targets_across_offset_changes_and_zero_clamping() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();
    owner.note_attached_runtime_position_dispatched(None, 40.0, 40.0);
    owner.user_offset_seconds = 5.0;
    owner.note_attached_runtime_position_dispatched(None, 40.0, 45.0);
    assert_eq!(
        owner.attached_system_seek_ownership[0].dispatch_offset_seconds,
        0.0
    );
    assert_eq!(
        owner.attached_system_seek_ownership[1].dispatch_offset_seconds,
        5.0
    );

    let mut late_first_seek = PositionTelemetryPlayer::default();
    late_first_seek
        .updates
        .extend(seek_transition(1.0, 1.1, 40.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(late_first_seek)));
    owner.refresh_player_state_impl();
    assert_eq!(owner.attached_system_seek_ownership.len(), 1);
    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty()
    );

    owner.attached_system_seek_ownership.clear();
    owner.user_offset_seconds = -5.0;
    owner.note_attached_runtime_position_dispatched(None, 2.0, 0.0);
    let mut clamped_seek = PositionTelemetryPlayer::default();
    clamped_seek.updates.extend(seek_transition(2.0, 2.1, 0.0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(clamped_seek)));
    owner.refresh_player_state_impl();
    assert!(owner.attached_system_seek_ownership.is_empty());
    assert_eq!(owner.player_position_seconds, Some(5.0));
    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .manual_seek_attempts
            .is_empty()
    );
}

#[test]
fn sparse_rich_update_preserves_legacy_authority_for_missing_fields() {
    let mut rich = sparse_update(1.0);
    rich.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    rich.cache_buffering_percent = Some(20.0);
    let mut player = PositionTelemetryPlayer::default();
    player.updates.push_back(rich);
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(12.0)
            .with_paused(true)
            .with_paused_for_cache(false)
            .with_cache_buffering_percent(99.0),
    );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_position_seconds, Some(12.0));
    assert_eq!(owner.player_paused, Some(true));
    assert_eq!(owner.player_paused_for_cache, Some(false));
    assert_eq!(owner.player_cache_buffering_percent, Some(20.0));
    assert!(!owner.attached_transport_telemetry_authority.position);
    assert!(!owner.attached_transport_telemetry_authority.logical_pause);
    assert!(
        owner
            .attached_transport_telemetry_authority
            .cache_buffering_percent
    );
}

#[test]
fn rich_authority_is_recomputed_each_pump_after_queue_pressure() {
    let mut first_player = PositionTelemetryPlayer::default();
    first_player.updates.push_back(position_update(1.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(first_player)));

    owner.refresh_player_state_impl();

    assert!(owner.attached_transport_telemetry_authority.position);
    assert!(owner.attached_transport_telemetry_authority.logical_pause);

    let mut sparse_rich = sparse_update(2.0);
    sparse_rich.phase = Some(sorotte_player_api::PlayerTransportPhase::Playing);
    let mut second_player = PositionTelemetryPlayer::default();
    second_player.updates.push_back(sparse_rich);
    second_player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(12.0)
            .with_paused(true),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(second_player)));

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_position_seconds, Some(12.0));
    assert_eq!(owner.player_paused, Some(true));
    assert!(!owner.attached_transport_telemetry_authority.position);
    assert!(!owner.attached_transport_telemetry_authority.logical_pause);
}

#[test]
fn media_load_failure_is_ordered_after_earlier_transport_from_the_same_drain() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("episode.mkv"));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    let generation = sorotte_player_api::PlayerMediaGeneration::new(1);
    let timestamp = |seconds| {
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
            std::time::Duration::from_secs_f64(seconds),
        )
    };
    let mut player = PositionTelemetryPlayer::default();
    player.updates.push_back(position_update(1.0, 11.0));
    player
        .media_load_observations
        .push_back(sorotte_player_api::PlayerMediaLoadObservation::new(
            sorotte_player_api::PlayerMediaLoadOutcome::failure(
                "episode.mkv",
                Some("episode.mkv".to_owned()),
                sorotte_player_api::PlayerMediaLoadFailureKind::Network,
                "test failure",
            ),
            Some(generation),
            Some(timestamp(2.0)),
        ));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_local_file, None);
    assert_eq!(owner.player_position_seconds, None);
    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        state
            .transport_updates
            .last()
            .and_then(|update| update.position_seconds),
        Some(11.0)
    );
}

#[test]
fn unsequenced_local_file_boundary_drops_same_drain_transport_fail_closed() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player
        .updates
        .push_back(position_update(0.0, 10.0));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("old.mkv"));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();
    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .clear();
    owner.note_attached_runtime_position_dispatched(None, 40.0, 40.0);

    let mut player = PositionTelemetryPlayer::default();
    player.local_file_observations.push_back(
        sorotte_player_api::PlayerLocalFileObservation::unsequenced(
            sorotte_player_api::LocalFileUpdate::new("new.mkv"),
        ),
    );
    player.updates.push_back(position_update(1.0, 40.0));
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(40.0)
            .with_paused(true),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("new.mkv")
    );
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.attached_native_seek_tracker.media_generation, None);
    assert!(owner.attached_system_seek_ownership.is_empty());
    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.transport_updates.is_empty());
    assert!(state.manual_seek_attempts.is_empty());
}

#[test]
fn sequenced_media_boundary_discards_generationless_playback_fallback() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("old.mkv"));
    owner.player_position_seconds = Some(5.0);
    owner.player_paused = Some(false);
    let mut player = PositionTelemetryPlayer::default();
    player
        .local_file_observations
        .push_back(sorotte_player_api::PlayerLocalFileObservation::new(
            sorotte_player_api::LocalFileUpdate::new("new.mkv"),
            Some(sorotte_player_api::PlayerMediaGeneration::new(2)),
            Some(
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    std::time::Duration::from_secs(2),
                ),
            ),
        ));
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(88.0)
            .with_paused(true),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("new.mkv")
    );
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(false));
}

#[test]
fn lower_generation_local_file_and_media_load_observations_are_rejected() {
    let mut current = position_update(2.0, 10.0);
    current.media_generation = Some(sorotte_player_api::PlayerMediaGeneration::new(2));
    let mut baseline_player = PositionTelemetryPlayer::default();
    baseline_player.updates.push_back(current);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("current.mkv"));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(baseline_player)));
    owner.refresh_player_state_impl();

    let timestamp = |seconds| {
        Some(
            sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                std::time::Duration::from_secs(seconds),
            ),
        )
    };
    let stale_generation = Some(sorotte_player_api::PlayerMediaGeneration::new(1));
    let mut player = PositionTelemetryPlayer::default();
    player
        .local_file_observations
        .push_back(sorotte_player_api::PlayerLocalFileObservation::new(
            sorotte_player_api::LocalFileUpdate::new("stale.mkv"),
            stale_generation,
            timestamp(3),
        ));
    player
        .media_load_observations
        .push_back(sorotte_player_api::PlayerMediaLoadObservation::new(
            sorotte_player_api::PlayerMediaLoadOutcome::failure(
                "current.mkv",
                None,
                sorotte_player_api::PlayerMediaLoadFailureKind::Network,
                "stale failure",
            ),
            stale_generation,
            timestamp(4),
        ));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));

    owner.refresh_player_state_impl();

    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("current.mkv")
    );
    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert_eq!(owner.attached_native_seek_tracker.media_generation, Some(2));
}
