use super::*;
use crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome;
use sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible;

#[test]
fn gui_persisted_config_runtime_owner_auto_advances_shared_playlist_once_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
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
        pause_intent_stages: usize,
        pause_dispatches: Vec<bool>,
        eof_observations: usize,
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

        fn observe_external_player_end_of_file(&mut self, _now_seconds: f64) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .eof_observations += 1;
            Ok(())
        }

        fn stage_attached_player_pause_intent(
            &mut self,
            _paused: bool,
            _now_seconds: f64,
        ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pause_intent_stages += 1;
            Ok(Vec::new())
        }

        fn supports_playback_pause_changes(&self) -> bool {
            true
        }

        fn local_pause_state(&self) -> Option<bool> {
            Some(false)
        }

        fn set_playback_paused(&mut self, paused: bool) -> Result<bool, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pause_dispatches
                .push(paused);
            Ok(true)
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
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
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
    let active_settings = StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    owner.active_session_settings = Some(
        stored_client_settings_runtime_snapshot_legacy_compatible(&active_settings),
    );
    let mut state = SorotteGuiShellAppState::from_stored_settings(&active_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: false,
    }));
    assert!(
        !state.main_window.shared_playlist_enabled,
        "the test must exercise an unsaved draft value opposite to the active session"
    );

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }
    assert!(
        state.main_window.shared_playlist_enabled,
        "runtime projection must restore the active session's enabled playlist state"
    );
    assert!(
        state.main_window.playback.can_manage_playlist,
        "runtime command availability must follow the active session rather than the draft"
    );
    assert!(
        state.main_window.playlist.is_empty(),
        "an unsaved disable must not replace the session playlist with player-local media"
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push_back(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
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
    {
        let recorded = recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.eof_observations, 1);
        assert_eq!(
            recorded.pause_intent_stages, 0,
            "natural EOF must not be staged as a direct player gesture"
        );
        assert!(
            recorded.pause_dispatches.is_empty(),
            "natural EOF must not be sent through the user pause mutation seam"
        );
    }

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
fn gui_persisted_config_runtime_owner_pins_playlist_target_lookup_to_active_settings() {
    let enabled_settings = StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session_projects_to_shell = true;
    owner.active_session_settings = Some(
        stored_client_settings_runtime_snapshot_legacy_compatible(&enabled_settings),
    );
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&enabled_settings);
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: false,
    }));
    assert_eq!(
        owner.current_shared_playlist_target(&state).as_deref(),
        Some("episode.mkv"),
        "an unsaved disable must not hide the active session playlist target"
    );

    let disabled_settings = StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    owner.active_session_settings = Some(
        stored_client_settings_runtime_snapshot_legacy_compatible(&disabled_settings),
    );
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    assert_eq!(
        owner.current_shared_playlist_target(&state),
        None,
        "an unsaved enable must not activate playlist lookup for a disabled session"
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
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

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_applies_autoplay_unpause_to_attached_player_without_remote_playstate()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        applied_pauses: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct RecordingSessionState {
        telemetry_updates: Vec<(Option<bool>, Option<f64>)>,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
        local_actions: Vec<GuiAttachedPlayerRuntimeAction>,
    }

    impl GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn take_attached_player_local_runtime_actions(
            &mut self,
        ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
            Ok(std::mem::take(&mut self.local_actions))
        }

        fn sync_local_playback_telemetry(
            &mut self,
            paused: Option<bool>,
            position_seconds: Option<f64>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .telemetry_updates
                .push((paused, position_seconds));
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

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingSessionRuntimeAdapter {
            state: session_state.clone(),
            local_actions: vec![GuiAttachedPlayerRuntimeAction::Paused {
                paused: false,
                cause: sorotte_client_core::PlayerCommandCause::AutomaticReadinessStart,
            }],
        }),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.player_local_file_placeholder = false;
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(0.0);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .applied_pauses,
        vec![false],
        "client-core autoplay unpause must be applied to the attached player even before a remote room playstate exists"
    );
    assert_eq!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .telemetry_updates,
        vec![(Some(false), Some(0.0))],
        "the applied local autoplay unpause should be mirrored back into session telemetry"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_auto_loops_single_item_shared_playlist_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
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
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
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
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
        player_state.playback_updates.push_back(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
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
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
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
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
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
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
        player_state.playback_updates.push_back(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
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
