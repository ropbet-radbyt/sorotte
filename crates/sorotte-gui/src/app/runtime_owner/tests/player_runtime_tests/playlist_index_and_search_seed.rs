use super::*;

#[test]
fn gui_persisted_config_runtime_owner_resets_same_file_playlist_index_switches() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
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
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
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
    }

    let root = test_temp_root("shared-playlist-same-file-reset");
    let current_media_path = root.join("episode1.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("same-file playlist reset fixture should be written");

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
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode1.mkv"],"user":"bob"}}}"#
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

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state.opened_paths.is_empty(),
        "same-file playlist index changes should not reopen the attached media"
    );
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON),
        "same-file playlist index changes should still consume the reset handoff and rewind"
    );
    assert!(
        !recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < f64::EPSILON),
        "same-file playlist index changes should not replay the stale room timeline"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_rewind_again_for_omitted_user_local_index_acknowledgement()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
        opened_paths: Vec<String>,
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

    let root = test_temp_root("shared-playlist-omitted-user-index-ack");
    let current_media_path = root.join("episode1.mkv");
    let selected_media_path = root.join("episode2.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("omitted-user playlist acknowledgment current fixture should be written");
    std::fs::write(&selected_media_path, b"test")
        .expect("omitted-user playlist acknowledgment selected fixture should be written");

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
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.selection.selected_main_window_playlist == Some(0),
        "initial shared-playlist selection should be active",
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let mut outbound_protocol_lines = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        outbound_protocol_lines.extend(session_transport.drain_outbound_protocol_lines());
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let reset_consumed = !owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .has_pending_playlist_index_reset_intent();
        let recorded_rewind = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .iter()
            .any(|position| position.abs() < f64::EPSILON);
        if state.selection.selected_main_window_playlist == Some(1)
            && owner
                .player_local_file
                .as_ref()
                .and_then(|file| file.path.as_deref())
                == Some(selected_media_path.to_string_lossy().as_ref())
            && reset_consumed
            && recorded_rewind
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.to_string_lossy().as_ref()),
        "the local index switch should open the newly selected media after its delivery receipt"
    );
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .has_pending_playlist_index_reset_intent(),
        "the intended local playlist reset should be consumed before its acknowledgment arrives"
    );
    {
        let mut recorded_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            recorded_state
                .set_positions
                .iter()
                .filter(|position| position.abs() < f64::EPSILON)
                .count(),
            1,
            "the local playlist switch should rewind the attached player exactly once"
        );
        recorded_state.set_positions.clear();
    }
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":1")),
        "the local playlist switch should be published before its delayed acknowledgment"
    );

    session_transport
        .push_inbound_protocol_line(r#"{"Set":{"playlistIndex":{"index":1}}}"#.to_owned());
    session_transport.push_inbound_protocol_line(
        r#"{"Chat":{"username":"bob","message":"index acknowledgment processed"}}"#.to_owned(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if state
            .main_window
            .chat
            .iter()
            .any(|entry| entry.sender == "bob" && entry.message == "index acknowledgment processed")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        state.main_window.chat.iter().any(|entry| {
            entry.sender == "bob" && entry.message == "index acknowledgment processed"
        }),
        "the transport should process the index acknowledgment before assertions are evaluated"
    );

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .is_empty(),
        "a delayed omitted-user acknowledgment must not seek the attached player a second time"
    );
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .has_pending_playlist_index_reset_intent(),
        "a delayed omitted-user acknowledgment must not queue another playlist reset"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reuses_media_search_index_for_later_playlist_selection() {
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

    let root = test_temp_root("shared-playlist-search-cache");
    let season_directory = root.join("season-1");
    std::fs::create_dir_all(&season_directory)
        .expect("shared-playlist cache fixture directory should be created");
    let episode_two_path = season_directory.join("episode2.mkv");
    let episode_three_path = season_directory.join("episode3.mkv");
    std::fs::write(&episode_two_path, b"test")
        .expect("shared-playlist cache fixture episode two should be written");
    std::fs::write(&episode_three_path, b"test")
        .expect("shared-playlist cache fixture episode three should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

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
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_two_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner.attached_media_search_index.is_some(),
        "first background search should populate the reusable media index"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#.to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_three_path.to_string_lossy().as_ref()),
        "later playlist selections should resolve immediately from the cached media index"
    );

    let _ = std::fs::remove_dir_all(&root);
}
