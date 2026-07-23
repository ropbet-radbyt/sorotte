use super::*;
use crate::app::runtime_owner::GuiAttachedCoordinatorSeekOwnershipState;

#[derive(Debug, Default)]
struct PositionSessionState {
    local_position_seconds: Option<f64>,
    synchronized_positions: Vec<f64>,
    recorded_manual_seeks: Vec<f64>,
    manual_seek_attempts: Vec<f64>,
    manual_seek_failures_remaining: usize,
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
    set_position_calls: usize,
    fail_set_position_call: Option<usize>,
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

    assert_eq!(owner.attached_coordinator_seek_ownership.len(), 2);
    assert_eq!(
        owner.attached_coordinator_seek_ownership[0].state,
        GuiAttachedCoordinatorSeekOwnershipState::SupersededMayArrive
    );
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(state.synchronized_positions, vec![10.0, 40.0, 20.0]);
    assert!(owner.attached_coordinator_seek_ownership.is_empty());
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

    assert_eq!(owner.attached_coordinator_seek_ownership.len(), 1);
    assert_eq!(
        owner.attached_coordinator_seek_ownership[0].coordinator_command_id,
        sorotte_client_core::CoordinatorCommandId::new(1)
    );
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.manual_seek_attempts.is_empty());
    assert_eq!(state.local_position_seconds, Some(40.0));
}

#[test]
fn expired_coordinator_seek_does_not_consume_a_later_native_seek_to_the_same_target() {
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
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(seek_player)));
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: sorotte_client_core::CoordinatorPlayerCommand::SetPosition(40.0),
        }],
        "expired coordinator seek regression",
    );
    owner.attached_coordinator_seek_ownership[0].expires_at =
        std::time::Instant::now() - std::time::Duration::from_secs(1);

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.manual_seek_attempts, vec![40.0]);
    assert!(owner.attached_coordinator_seek_ownership.is_empty());
}

#[test]
fn terminal_adapter_failure_retires_command_scoped_seek_ownership() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session = Some(Box::new(PositionSession {
        state: session_state,
    }));
    let adapter_command_id = sorotte_player_api::PlayerCommandId::new(7);
    owner.note_attached_coordinator_seek_dispatched(
        sorotte_client_core::CoordinatorCommandId::new(1),
        Some(adapter_command_id),
        40.0,
    );

    owner.reconcile_attached_coordinator_seek_command_progress(
        sorotte_player_api::PlayerCommandProgress::finished(
            adapter_command_id,
            Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
            None,
            None,
            sorotte_player_api::PlayerCommandResult::Failed(
                sorotte_player_api::PlayerCommandFailureKind::TimedOut,
            ),
        ),
    );

    assert!(owner.attached_coordinator_seek_ownership.is_empty());
}

#[test]
fn direct_position_and_session_lifecycle_changes_retire_seek_ownership() {
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
    assert_eq!(owner.attached_coordinator_seek_ownership.len(), 1);
    owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Position(20.0)],
        "direct position lifecycle regression",
    );
    assert!(owner.attached_coordinator_seek_ownership.is_empty());

    owner.apply_attached_player_runtime_actions_impl(
        vec![coordinator_seek.clone()],
        "session replacement lifecycle regression",
    );
    owner.install_session_runtime(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    assert!(owner.attached_coordinator_seek_ownership.is_empty());

    owner.apply_attached_player_runtime_actions_impl(
        vec![coordinator_seek],
        "transport reconnect lifecycle regression",
    );
    owner.clear_session_attached_player_sync_state();
    assert!(owner.attached_coordinator_seek_ownership.is_empty());
    assert_eq!(owner.attached_native_seek_tracker.media_generation, None);
    assert!(!owner.attached_transport_telemetry_available);
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
    assert_eq!(owner.attached_coordinator_seek_ownership.len(), 1);

    owner.session = Some(Box::new(AlternateRoomPositionSession));
    owner.refresh_player_state_impl();

    assert!(owner.attached_coordinator_seek_ownership.is_empty());
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
fn stale_rich_position_also_blocks_the_untimestamped_legacy_position_channel() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.playback_updates.push_back(
        sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(99.5)
            .with_paused(true),
    );
    player.updates.extend([
        position_update_with_delivery(0.0, 0.0, 10.0),
        position_update_with_delivery(1.0, 5.0, 99.5),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert!(!owner.playlist_auto_advance_eof_latched);
    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .synchronized_positions,
        vec![10.0]
    );
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
        sparse_position_update(3.0, 24.1),
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
        vec![10.0, 20.1, 24.1],
        "the untimestamped position must not be grounded or bridged into a later comparison, while its transport state remains effective"
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
