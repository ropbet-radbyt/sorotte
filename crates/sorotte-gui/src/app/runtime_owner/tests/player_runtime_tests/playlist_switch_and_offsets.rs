use super::*;

#[test]
fn gui_persisted_config_runtime_owner_resets_inbound_shared_playlist_switches_before_applying_fresh_room_playstate()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opened_paths.push(path.to_owned());
            state.local_file_updates.push(
                sorotte_player_api::LocalFileUpdate::new(
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path),
                )
                .with_path(path.to_owned()),
            );
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
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

    let root = test_temp_root("shared-playlist-background-search");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("background shared-playlist search fixture directory should be created");
    let current_media_path = root.join("episode1.mkv");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("background shared-playlist current media fixture should be written");
    std::fs::write(&selected_media_path, b"test")
        .expect("background shared-playlist search fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_paused_values
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let recorded_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let opened_selected_media = recorded_state
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref());
        let applied_reset_rewind = recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON);
        drop(recorded_state);
        if opened_selected_media && applied_reset_rewind {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
        "background shared-playlist search should eventually open the selected media"
    );
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON),
        "playlist index changes should rewind a newly opened item before any fresh room sync arrives; recorded_state={recorded_state:?}, pending_reset={}, placeholder={}, player_local_file={:?}",
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        owner.player_local_file_placeholder,
        owner.player_local_file,
    );
    assert!(
        !recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < f64::EPSILON),
        "stale room playstate from the previous file should not be replayed onto the newly opened item"
    );
    drop(recorded_state);

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":7.5,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .iter()
            .any(|position| *position > 7.4)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| *position > 7.4),
        "once the room playstate changes for the new file, the attached player should follow it"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_opens_local_queue_and_select_target_before_and_after_echo() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opened_paths.push(path.to_owned());
            state.local_file_updates.push(
                sorotte_player_api::LocalFileUpdate::new(
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path),
                )
                .with_path(path.to_owned()),
            );
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
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

        fn set_paused(&mut self, _paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }
    }

    let root = test_temp_root("local-queue-select-player-target");
    std::fs::create_dir_all(&root).expect("playlist fixture directory should be created");
    let current_media_path = root.join("episode1.mkv");
    let selected_media_path = root.join("episode2.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("current playlist media fixture should be written");
    std::fs::write(&selected_media_path, b"test")
        .expect("selected playlist media fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.session_transport_driver = Some(Box::new(ExternallyDrivenTestSessionTransport));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    handle.push_request(GuiRuntimeRequest::QueuePlaylistEntry {
        entry: "episode2.mkv".to_owned(),
        select_after_queue: true,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .is_empty(),
        "queue-and-select must remain fenced before its terminal write receipt"
    );
    let mut outbound_lines = session_transport.drain_outbound_protocol_lines();
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        outbound_lines.extend(session_transport.drain_outbound_protocol_lines());
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let opened_target = recorded
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref());
        let rewound_target = recorded
            .set_positions
            .iter()
            .any(|position| position.abs() < f64::EPSILON);
        drop(recorded);
        let emitted_reset_state = outbound_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"position\":0.0")
                && line.contains("\"paused\":true")
        });
        if opened_target && rewound_target && emitted_reset_state {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        outbound_lines
            .iter()
            .any(|line| { line.contains("\"playlistChange\"") && line.contains("episode2.mkv") })
    );
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":1"))
    );
    assert!(outbound_lines.iter().any(|line| {
        line.contains("\"State\"")
            && line.contains("\"position\":0.0")
            && line.contains("\"paused\":true")
    }));

    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            recorded
                .opened_paths
                .iter()
                .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
            "queue-and-select should open the selected media after its exact delivery receipt; recorded={recorded:?}"
        );
        assert!(
            recorded
                .set_positions
                .iter()
                .any(|position| position.abs() < f64::EPSILON)
        );
    }
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    for line in outbound_lines
        .iter()
        .filter(|line| line.contains("\"Set\""))
    {
        session_transport.push_inbound_protocol_line(line.clone());
    }
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should remain active")
            .has_pending_playlist_index_reset_intent(),
        "the reflected queue-and-select acknowledgment must not strand or recreate a reset"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_applies_user_offset_only_at_player_sync_boundary() {
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
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.user_offset_seconds = 5.0;
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .all(|position| *position > 14.9 && *position < 15.5),
        "attached-player room sync should add the active user offset when seeking the player"
    );
    assert_eq!(
        owner
            .player_position_seconds
            .map(|position| position.round()),
        Some(10.0),
        "stored runtime playback position should stay on the global timeline instead of the shifted player time"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_seeks_before_pausing_attached_player_for_room_pause() {
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
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_position_seconds = Some(5.0);
    owner.player_paused = Some(false);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < f64::EPSILON),
        "attached-player pause sync should seek to the room position before pausing"
    );
    assert!(
        recorded_state.set_paused_values.contains(&true),
        "attached-player pause sync should still pause once the position is corrected"
    );
    drop(recorded_state);
    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert_eq!(owner.player_paused, Some(true));
}
