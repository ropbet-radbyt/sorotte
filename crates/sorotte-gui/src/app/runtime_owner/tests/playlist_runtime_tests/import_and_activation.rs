use super::*;

#[test]
fn gui_persisted_config_runtime_owner_keeps_text_playlist_entries_literal() {
    let root = test_temp_root("shared-playlist-text-format");
    let playlist_path = root.join("room-playlist.txt");
    std::fs::write(
        &playlist_path,
        "\n# literal text entry\nmedia/episode1.mkv\nhttps://example.com/live\n",
    )
    .expect("text playlist fixture should be written");

    let dispatch = GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(vec![
        playlist_path.to_string_lossy().into_owned(),
    ])
    .expect("text playlist should be imported");

    assert!(dispatch.imported_from_file);
    assert_eq!(
        dispatch.playlist_entries(),
        vec![
            "# literal text entry".to_owned(),
            "media/episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert!(
        dispatch
            .items
            .iter()
            .all(|item| item.local_origin.is_none())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_parses_m3u_comments_relative_paths_and_urls() {
    let root = test_temp_root("shared-playlist-m3u-format");
    let playlist_root = root.join("lists");
    std::fs::create_dir_all(&playlist_root).expect("playlist fixture directory should be created");
    let playlist_path = playlist_root.join("room-playlist.m3u");
    std::fs::write(
        &playlist_path,
        "\u{feff}#EXTM3U\n#EXTINF:120,Episode 1\nmedia/episode1.mkv\n  # ignored comment\nhttps://example.com/live?id=1\n",
    )
    .expect("M3U playlist fixture should be written");

    let dispatch = GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(vec![
        playlist_path.to_string_lossy().into_owned(),
    ])
    .expect("M3U playlist should be imported");

    assert!(dispatch.imported_from_file);
    assert_eq!(
        dispatch.playlist_entries(),
        vec![
            playlist_root
                .join("media")
                .join("episode1.mkv")
                .to_string_lossy()
                .into_owned(),
            "https://example.com/live?id=1".to_owned(),
        ]
    );
    assert!(
        dispatch
            .items
            .iter()
            .all(|item| item.local_origin.is_none())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_treats_local_hls_m3u8_as_one_media_target() {
    let root = test_temp_root("shared-playlist-hls-format");
    let manifest_path = root.join("live.m3u8");
    std::fs::write(
        &manifest_path,
        "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nsegment-1.ts\n#EXT-X-ENDLIST\n",
    )
    .expect("HLS manifest fixture should be written");
    let manifest_path = manifest_path.to_string_lossy().into_owned();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(&state, vec![manifest_path.clone()])
        .expect("HLS manifest should be dispatched as media");

    assert!(!dispatch.imported_from_file);
    assert_eq!(dispatch.playlist_entries(), vec!["live.m3u8".to_owned()]);
    assert_eq!(dispatch.items.len(), 1);
    assert_eq!(dispatch.items[0].published_entry, "live.m3u8");
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(manifest_path.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
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
    assert_eq!(state.active_view, GuiShellView::Room);
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
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
    assert_eq!(state.active_view, GuiShellView::Room);
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
fn gui_persisted_config_runtime_owner_local_playlist_activation_switches_media_before_server_echo()
{
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
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

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
        |state| {
            state.main_window.playlist.len() == 2
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "initial playlist selection should land on the current item",
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path.to_string_lossy().as_ref()),
        "plain local playlist selection should not switch the attached player before activation",
    );

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
