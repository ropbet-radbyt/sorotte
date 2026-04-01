use super::*;

#[test]
fn gui_persisted_config_runtime_owner_routes_shared_playlist_open_through_client_core_session_and_player()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist open through loopback session and player",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 2 selected media entries into the shared playlist."
        )),
        "shared-playlist open should report playlist-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_coerces_local_media_open_into_playlist_control_when_shared_playlist_is_enabled()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/local-only.mkv".to_owned()],
        load_into_shared_playlist: false,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.playlist.len() == 1
                && state.selection.selected_main_window_playlist == Some(0)
                && state.main_window.playlist[0].label == "local-only.mkv"
        },
        "shared-playlist-enabled local media opens route through playlist control",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist-enabled media opens should still report playlist success",
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
        )),
        "detached shared-playlist media opens should report that room sync is unavailable",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec!["local-only.mkv".to_owned()]
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/local-only.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_coerces_local_media_open_into_playlist_control_even_when_legacy_toggle_is_disabled()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/local-drop.mkv".to_owned()],
        load_into_shared_playlist: false,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.shared_playlist_enabled
                && state.main_window.playlist.len() == 1
                && state.selection.selected_main_window_playlist == Some(0)
                && state.main_window.playlist[0].label == "local-drop.mkv"
        },
        "playlist-backed local media opens remain active with the legacy toggle disabled",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "media opens should still report playlist success when the legacy toggle is off",
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
        )),
        "detached playlist-backed media opens should still warn about missing room sync",
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/local-drop.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_blocks_local_media_open_when_room_playlist_control_is_unavailable()
 {
    #[derive(Debug, Default)]
    struct NoControlSessionState {
        replace_playlist_calls: usize,
    }

    struct NoControlSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<NoControlSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for NoControlSessionRuntimeAdapter {
        fn playlist_control_available(&self) -> bool {
            false
        }

        fn replace_playlist(
            &mut self,
            _files: Vec<String>,
            _selected_index: Option<usize>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .replace_playlist_calls += 1;
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

    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(NoControlSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(NoControlSessionRuntimeAdapter {
            state: session_state.clone(),
        }),
    );
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let initial_playlist = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.label.clone())
        .collect::<Vec<_>>();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/blocked-drop.mkv".to_owned()],
        load_into_shared_playlist: false,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message == "Shared playlist control is unavailable for the active room; the selected media was not added to the room playlist or opened in the attached player."
        )),
        "non-controller media drops should fail instead of opening directly in the attached player",
    );
    assert!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>()
            == initial_playlist,
        "blocked non-controller media drops must not change the shared playlist locally",
    );
    assert!(
        owner.player_local_file.is_none(),
        "blocked non-controller media drops must not open a local file in the attached player",
    );
    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace_playlist_calls
            == 0,
        "blocked non-controller media drops must not attempt a session playlist mutation",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_imports_playlist_files_through_client_core_session() {
    let root = test_temp_root("shared-playlist-import");
    let playlist_path = root.join("room-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("shared playlist import fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist import through loopback session",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Imported 2 entries into the shared playlist."
        )),
        "shared-playlist imports should report a runtime-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert!(owner.player_local_file.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_imports_playlist_files_queued_before_startup_pump() {
    let root = test_temp_root("shared-playlist-import-startup-queue");
    let playlist_path = root.join("startup-room-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("startup shared playlist import fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist import queued before startup runtime pump",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Imported 2 entries into the shared playlist."
        )),
        "startup-queued shared-playlist imports should still report a runtime-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_opens_inbound_selected_shared_playlist_media() {
    let root = test_temp_root("shared-playlist-inbound-open");
    let selected_media_path = root.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("inbound shared-playlist media fixture should be written");

    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.selection.selected_main_window_playlist == Some(1),
        "inbound shared-playlist selection opens through attached player",
    );

    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_local_playlist_selection_switches_media_before_server_echo() {
    let root = test_temp_root("shared-playlist-local-select-before-echo");
    let current_media_path = root.join("episode1.mkv");
    let selected_media_path = root.join("episode2.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("current shared-playlist media fixture should be written");
    std::fs::write(&selected_media_path, b"test")
        .expect("selected shared-playlist media fixture should be written");

    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        |state| {
            state.main_window.playlist.len() == 2
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "initial playlist selection should land on the current item",
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if state.selection.selected_main_window_playlist == Some(1)
            && owner
                .player_local_file
                .as_ref()
                .and_then(|file| file.path.as_deref())
                == Some(selected_media_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(&root);
}
