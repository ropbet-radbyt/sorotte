use super::*;

#[test]
fn gui_persisted_config_runtime_owner_initially_syncs_live_room_position_to_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
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
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(0.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("live room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < 1.0),
        "the first live room playstate should seek the attached player onto the active timeline"
    );
    assert!(
        owner
            .player_position_seconds
            .is_some_and(|position| (position - 42.0).abs() < 1.0)
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_matching_local_file_before_applying_playlist_reset()
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
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

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

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
        .note_local_playlist_index_reset_intent(true);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.player_local_file_placeholder = true;
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert!(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should remain pending until the attached player reports a real local file update"
    );

    owner.player_local_file_placeholder = false;
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_positions, vec![0.0]);
    assert_eq!(recorded.set_paused_values, vec![true]);
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should clear after the rewind is applied"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retries_playlist_reset_after_transient_attached_player_rewind_failure()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
        successful_positions: Vec<f64>,
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_position_attempts += 1;
            if state.set_position_attempts == 1 {
                return Err(syncplay_player_api::PlayerError::OperationFailed(
                    "property unavailable".to_owned(),
                ));
            }
            state.successful_positions.push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
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

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

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
        .note_local_playlist_index_reset_intent(true);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.set_position_attempts, 1);
        assert!(recorded.successful_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert!(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "transient rewind failures should leave the playlist reset intent pending for a later retry"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_position_attempts, 2);
    assert_eq!(recorded.successful_positions, vec![0.0]);
    assert_eq!(recorded.set_paused_values, vec![true]);
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should clear after a later retry succeeds"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_applies_desync_seek_when_room_playstate_is_unchanged() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
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
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .expect("room playstate should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(10.0))
        .expect("initial local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    owner.player_position_seconds = Some(20.0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(20.0))
        .expect("desynced local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "steady-state attached-player sync should still rewind desynced playback even when the room playstate snapshot is unchanged"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retries_attached_player_seek_after_transient_failure() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
        successful_positions: Vec<f64>,
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_position_attempts += 1;
            if state.set_position_attempts == 1 {
                return Err(syncplay_player_api::PlayerError::OperationFailed(
                    "transient failure".to_owned(),
                ));
            }
            state.successful_positions.push(position_seconds);
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
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(0.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.set_position_attempts, 1);
        assert!(recorded.successful_positions.is_empty());
    }
    assert_eq!(owner.player_position_seconds, Some(0.0));

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_position_attempts, 2);
    assert!(
        recorded
            .successful_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "retrying the room playstate sync should seek close to the requested room position"
    );
    assert!(
        owner
            .player_position_seconds
            .is_some_and(|position| (position - 10.0).abs() < 1.0)
    );
}
