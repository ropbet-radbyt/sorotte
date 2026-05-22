use super::*;

#[test]
fn gui_persisted_config_runtime_owner_reorders_shared_playlist_without_switching_active_media() {
    let root = test_temp_root("shared-playlist-reorder-preserves-active-media");
    let episode_a_path = root.join("episodeA.mkv");
    let episode_b_path = root.join("episodeB.mkv");
    let episode_c_path = root.join("episodeC.mkv");
    let episode_d_path = root.join("episodeD.mkv");
    for path in [
        &episode_a_path,
        &episode_b_path,
        &episode_c_path,
        &episode_d_path,
    ] {
        std::fs::write(path, b"test").expect("shared-playlist media fixture should be written");
    }

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episodeB.mkv")
            .with_path(episode_b_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec![
            "episodeA.mkv".to_owned(),
            "episodeB.mkv".to_owned(),
            "episodeC.mkv".to_owned(),
            "episodeD.mkv".to_owned(),
        ],
        selected_index: Some(1),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episodeA.mkv",
                    "episodeB.mkv",
                    "episodeC.mkv",
                    "episodeD.mkv",
                ])
                && state.selection.selected_main_window_playlist == Some(1)
        },
        "shared-playlist reorder seed should populate the initial playlist",
    );

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 3,
        to_index: 0,
    }));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "episodeD.mkv",
            "episodeA.mkv",
            "episodeB.mkv",
            "episodeC.mkv"
        ]
    );
    assert_eq!(
        state.selection.selected_main_window_playlist,
        Some(2),
        "local playlist reorder should keep the currently active entry selected"
    );

    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: state.current_shared_playlist_entries(),
        selected_index: None,
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episodeD.mkv",
                    "episodeA.mkv",
                    "episodeB.mkv",
                    "episodeC.mkv",
                ])
                && state.selection.selected_main_window_playlist == Some(2)
        },
        "shared-playlist reorder should preserve the active entry after the runtime echo",
    );

    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(episode_b_path.to_string_lossy().as_ref()),
        "shared-playlist reorder should not switch the attached player away from the current item"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(42.0),
        "shared-playlist reorder should not rewind the currently playing media"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reorders_playlist_with_local_highlight_without_switching_active_media()
 {
    let root =
        test_temp_root("shared-playlist-reorder-preserves-active-media-with-local-highlight");
    let episode_a_path = root.join("episodeA.mkv");
    let episode_b_path = root.join("episodeB.mkv");
    let episode_c_path = root.join("episodeC.mkv");
    let episode_d_path = root.join("episodeD.mkv");
    for path in [
        &episode_a_path,
        &episode_b_path,
        &episode_c_path,
        &episode_d_path,
    ] {
        std::fs::write(path, b"test").expect("shared-playlist media fixture should be written");
    }

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episodeB.mkv")
            .with_path(episode_b_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec![
            "episodeA.mkv".to_owned(),
            "episodeB.mkv".to_owned(),
            "episodeC.mkv".to_owned(),
            "episodeD.mkv".to_owned(),
        ],
        selected_index: Some(1),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episodeA.mkv",
                    "episodeB.mkv",
                    "episodeC.mkv",
                    "episodeD.mkv",
                ])
                && state.selection.selected_main_window_playlist == Some(1)
        },
        "shared-playlist reorder seed should populate the initial playlist",
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(3)));
    assert_eq!(
        state.selection.selected_main_window_playlist,
        Some(3),
        "the UI should allow highlighting a non-active playlist row before reordering"
    );

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 3,
        to_index: 0,
    }));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "episodeD.mkv",
            "episodeA.mkv",
            "episodeB.mkv",
            "episodeC.mkv"
        ]
    );
    assert_eq!(
        state.selection.selected_main_window_playlist,
        Some(0),
        "local reorder should keep the highlighted row selected even when it is not the active room entry"
    );

    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: state.current_shared_playlist_entries(),
        selected_index: None,
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episodeD.mkv",
                    "episodeA.mkv",
                    "episodeB.mkv",
                    "episodeC.mkv",
                ])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist reorder should preserve the local highlight after the runtime echo",
    );

    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(episode_b_path.to_string_lossy().as_ref()),
        "playlist reorder should not switch the attached player to the highlighted row"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(42.0),
        "playlist reorder should not rewind the current media when a different row is only locally highlighted"
    );

    let _ = std::fs::remove_dir_all(&root);
}
