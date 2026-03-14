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
                    && message
                        == "Opened the first selected media file through the attached test player: C:/Media/episode1.mkv. Ignored 1 additional selections."
        )),
        "shared-playlist open should still drive the attached player"
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
