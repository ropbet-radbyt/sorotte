use super::*;
use sorotte_client_core::{CoordinatorPlayerCommand, PlaybackCoordinationSnapshot};

#[derive(Debug, Default)]
struct CoordinatorAuthorityPlayerState {
    paused: Vec<bool>,
    positions: Vec<f64>,
    playback_rates: Vec<f64>,
}

struct CoordinatorAuthorityPlayer {
    state: std::sync::Arc<std::sync::Mutex<CoordinatorAuthorityPlayerState>>,
}

impl PlayerAdapter for CoordinatorAuthorityPlayer {
    fn name(&self) -> &'static str {
        "coordinator-authority"
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .paused
            .push(paused);
        Ok(())
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .positions
            .push(position_seconds);
        Ok(())
    }

    fn set_playback_rate(
        &mut self,
        playback_rate: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .playback_rates
            .push(playback_rate);
        Ok(())
    }
}

struct CoordinatorAuthoritySession {
    actions: Vec<GuiAttachedPlayerRuntimeAction>,
    recovery_cleanup_actions: Vec<GuiAttachedPlayerRuntimeAction>,
}

impl GuiSessionRuntimeAdapter for CoordinatorAuthoritySession {
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

    fn playback_coordination_snapshot(&self) -> Option<PlaybackCoordinationSnapshot> {
        Some(PlaybackCoordinationSnapshot {
            media_generation: Some(1),
            diagnostic: sorotte_client_core::PlaybackDiagnostic::ReadyWaitingForRoom,
            recovery_episode: None,
            metrics: Default::default(),
            transport_telemetry_observed: true,
            ordinary_correction_blocked: false,
            last_applied_revision: None,
            last_started_revision: None,
            last_degraded_reason: None,
        })
    }

    fn attached_player_runtime_actions(
        &mut self,
        _now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(std::mem::take(&mut self.actions))
    }

    fn interrupt_attached_playback_recovery(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(std::mem::take(&mut self.recovery_cleanup_actions))
    }

    // Model the old GUI adapter's self-origin filter. Coordinator authority
    // must be checked before this legacy accessor is consulted.
    fn current_room_playstate_for_attached_player_sync(&self) -> Option<GuiSessionRoomPlaystate> {
        None
    }
}

fn run_self_attributed_coordinator_actions(
    actions: Vec<GuiAttachedPlayerRuntimeAction>,
) -> std::sync::Arc<std::sync::Mutex<CoordinatorAuthorityPlayerState>> {
    let state = std::sync::Arc::new(std::sync::Mutex::new(
        CoordinatorAuthorityPlayerState::default(),
    ));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session = Some(Box::new(CoordinatorAuthoritySession {
        actions,
        recovery_cleanup_actions: Vec::new(),
    }));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        CoordinatorAuthorityPlayer {
            state: state.clone(),
        },
    )));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    let shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    owner.sync_session_playstate_to_attached_player_impl(&shell, false);
    state
}

#[test]
fn gui_controller_barrier_reconciles_before_legacy_self_origin_filter() {
    let state = run_self_attributed_coordinator_actions(vec![
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(1),
            command: CoordinatorPlayerCommand::SetPosition(12.0),
        },
    ]);
    assert_eq!(
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .positions,
        vec![12.0]
    );
}

#[test]
fn gui_all_eligible_controller_participation_obeys_server_commit() {
    let state = run_self_attributed_coordinator_actions(vec![
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(2),
            command: CoordinatorPlayerCommand::Play(sorotte_player_api::PlayerPlayIntent::Resume),
        },
    ]);
    assert_eq!(
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .paused,
        vec![false]
    );
}

#[test]
fn gui_controller_obeys_server_owned_room_buffering_pause_and_resume() {
    let state = run_self_attributed_coordinator_actions(vec![
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(3),
            command: CoordinatorPlayerCommand::SetPaused(true),
        },
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(4),
            command: CoordinatorPlayerCommand::Play(sorotte_player_api::PlayerPlayIntent::Resume),
        },
    ]);
    assert_eq!(
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .paused,
        vec![true, false]
    );
}

#[test]
fn gui_recovery_interrupt_resets_rate_on_the_real_attached_player() {
    let state = std::sync::Arc::new(std::sync::Mutex::new(
        CoordinatorAuthorityPlayerState::default(),
    ));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session = Some(Box::new(CoordinatorAuthoritySession {
        actions: Vec::new(),
        recovery_cleanup_actions: vec![GuiAttachedPlayerRuntimeAction::Coordinator {
            command_id: sorotte_client_core::CoordinatorCommandId::new(5),
            command: CoordinatorPlayerCommand::SetPlaybackRate(1.0),
        }],
    }));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        CoordinatorAuthorityPlayer {
            state: state.clone(),
        },
    )));

    assert!(owner.interrupt_attached_playback_recovery_impl("test interruption"));
    assert_eq!(
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .playback_rates,
        vec![1.0],
        "recovery cleanup must cross the GUI's external-player seam instead of the no-op runtime player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_skips_self_origin_room_position_sync_for_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("self-origin room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "force-sync should not replay the local user's own room position back into the attached player"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "force-sync should not replay the local user's own room pause state back into the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_ignores_unattributed_room_playstate_when_no_remote_users_are_known()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);
    owner.player_paused = Some(false);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"State":{"playstate":{"position":0.0,"paused":true}}}"#)
        .expect("unattributed room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "room playstate without remote authority should not rewind the attached player while alone"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "room playstate without remote authority should not pause the attached player while alone"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_local_file_before_applying_room_playstate() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "room playstate should seek once the attached player reports a local file"
    );
    assert_eq!(recorded.set_paused_values, vec![true]);
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_advancement_without_seeking_on_cache_release() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(3.0);
    owner.player_paused = Some(false);
    owner.player_paused_for_cache = Some(true);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room seek should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !recorded.set_positions.is_empty(),
            "room seek should still be applied while cache pause defers unpause"
        );
        assert!(
            recorded.set_paused_values.is_empty(),
            "cache pause should defer room unpause"
        );
        recorded.set_positions.clear();
    }
    assert!(owner.pending_attached_room_unpause_observation.is_some());
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused_for_cache(false),
        );
    owner.refresh_player_state_impl();
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":34.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("post-cache room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            recorded.set_positions.is_empty(),
            "cache release alone must not seek to the room's newer moving position"
        );
        assert!(
            recorded.set_paused_values.is_empty(),
            "cache release alone must not replay the room unpause"
        );
    }
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "cache release is not evidence that playback has resumed"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(30.0)
                .with_paused(false),
        );
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "one stationary post-cache sample must keep desired play pending"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(30.25)
                .with_paused(false),
        );
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_none(),
        "fresh forward position advancement should acknowledge desired play"
    );
    assert!(
        owner.last_applied_attached_room_playstate.is_some(),
        "the room playstate may be marked applied only after advancement is observed"
    );
    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(recorded.set_positions.is_empty());
    assert!(recorded.set_paused_values.is_empty());
    assert_ne!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state()),
        Some(true),
        "observed post-cache recovery should not become a manual pause"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retains_room_play_until_advancement_after_ipc_acceptance() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(10.0);
    owner.player_paused = Some(true);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room play should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    let baseline_position_seconds = owner
        .player_position_seconds
        .expect("room sync should retain an observation baseline");
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false]
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "IPC acceptance must not overwrite the last observed pause property"
    );
    assert!(owner.pending_attached_room_unpause_observation.is_some());
    assert_eq!(
        owner.last_applied_attached_room_playstate, None,
        "IPC acceptance alone must not mark desired play as applied"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(baseline_position_seconds)
                .with_paused(false),
        );
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "a pause=false property without forward motion is not observed playback"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(baseline_position_seconds + 0.25)
                .with_paused(false),
        );
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    assert!(owner.pending_attached_room_unpause_observation.is_none());
    assert!(owner.last_applied_attached_room_playstate.is_some());
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "retained desired play should not busy-loop unpause commands while awaiting observations"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_force_room_sync_for_matched_playlist_target_without_reset_intent()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("matched-playlist-target-no-reset");
    let media_path = root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("playlist target fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(0.0);
    owner.player_paused = Some(false);

    let stored_settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        rewind_on_desync: Some(false),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&stored_settings);
    state.apply_shared_playlist_entries(vec!["episode1.mkv".to_owned()], Some(0), false);
    owner.active_shared_playlist_index = Some(0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_runtime_settings(&stored_client_settings_runtime_snapshot_legacy_compatible(
            &stored_settings,
        ))
        .expect("runtime settings should sync");

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":41.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.set_positions.clear();
        recorded.set_paused_values.clear();
    }
    owner.player_position_seconds = Some(42.0);

    let selected_media_sync =
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        selected_media_sync,
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
    );

    let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
    );
    assert!(
        !selection_handoff_ready,
        "matched playlist targets without a pending reset should not force a room playstate handoff"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(
        &state,
        selection_handoff_ready,
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, selection_handoff_ready);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "playlist updates that keep the current target selected should not rewind the attached player; recorded={recorded:?}"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "playlist updates that keep the current target selected should not toggle pause state; recorded={recorded:?}"
    );
    assert_eq!(owner.player_position_seconds, Some(42.0));

    let _ = std::fs::remove_dir_all(&root);
}
