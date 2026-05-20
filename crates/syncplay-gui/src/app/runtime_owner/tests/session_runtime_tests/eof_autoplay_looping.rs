use super::*;
use crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome;

#[test]
fn gui_persisted_config_runtime_owner_auto_advances_shared_playlist_once_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<syncplay_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    #[derive(Debug, Default)]
    struct RecordingSessionState {
        advance_calls: usize,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn playlist_control_available(&self) -> bool {
            true
        }

        fn can_auto_advance_to_next_playlist_item(&self) -> bool {
            true
        }

        fn advance_playlist_index(&mut self) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .advance_calls += 1;
            Ok(())
        }

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
            _current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(Vec::new())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state.local_file_updates.push_back(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
    }

    let recorded = std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingSessionRuntimeAdapter {
            state: recorded.clone(),
        }),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1511.0),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    assert_eq!(
        recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance_calls,
        1,
        "EOF should trigger one playlist advance when the player pauses at the file end"
    );

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    assert_eq!(
        recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance_calls,
        1,
        "the EOF auto-advance should stay latched until the player leaves the end-of-file state"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_ready_when_opening_auto_advanced_playlist_item() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    fn ready_state_from_protocol_line(line: &str) -> Option<bool> {
        let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
        message.get("Set")?.get("ready")?.get("isReady")?.as_bool()
    }

    let mut session = crate::app::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let _ = session
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready state should apply");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let root = test_temp_root("auto-advanced-playlist-ready-preserve");
    let episode_two_path = root.join("episode2.mkv");
    std::fs::write(&episode_two_path, b"test")
        .expect("auto-advance ready preservation fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.active_shared_playlist_index = Some(0);
    owner.playlist_auto_advance_eof_latched = true;
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .note_local_playlist_index_reset_intent(true);

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec![episode_two_path.to_string_lossy().into_owned()],
        Some(0),
        false,
    );
    let opened = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(opened, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![episode_two_path.to_string_lossy().into_owned()]
    );
    let open_lines = owner
        .session
        .as_mut()
        .expect("session should exist")
        .flush_outbound_protocol_lines()
        .expect("open protocol lines should encode");
    assert!(
        open_lines
            .iter()
            .all(|line| ready_state_from_protocol_line(line) != Some(false)),
        "opening the auto-advanced playlist item should not cancel readiness/autoplay; lines={open_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_auto_loops_single_item_shared_playlist_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<syncplay_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
        applied_pauses: Vec<bool>,
        applied_positions: Vec<f64>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state.local_file_updates.push_back(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
        player_state.playback_updates.push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(0.0),
        );
    }

    let mut session = crate::app::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = session
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":1510.0}}}}}"#,
        )
        .expect("local file update should apply");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        loop_single_files: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1511.0),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.applied_positions,
        vec![0.0],
        "single-item loop EOF should rewind the attached player"
    );
    assert_eq!(
        recorded.applied_pauses,
        vec![false],
        "single-item loop EOF should resume the attached player after rewinding"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_auto_loops_single_item_shared_playlist_at_eof_with_offset() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<syncplay_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
        applied_pauses: Vec<bool>,
        applied_positions: Vec<f64>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state.local_file_updates.push_back(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
        player_state.playback_updates.push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(0.0),
        );
    }

    let mut session = crate::app::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = session
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":1510.0}}}}}"#,
        )
        .expect("local file update should apply");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.user_offset_seconds = 5.0;

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        loop_single_files: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1515.0),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.applied_positions,
        vec![5.0],
        "single-item loop EOF should rewind the attached player on the offset-adjusted timeline"
    );
    assert_eq!(
        recorded.applied_pauses,
        vec![false],
        "single-item loop EOF should still resume the attached player after rewinding"
    );
}
