use super::*;

fn seeded_loopback_shared_playlist_owner(
    active_index: usize,
) -> (
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SyncplayGuiShellAppState,
) {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode3.mkv")
            .with_path("C:/Media/episode3.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ],
        selected_index: Some(active_index),
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
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.main_window.active_playlist_index == Some(active_index)
        },
        "shared-playlist seed with active index",
    );

    (owner, handle, state)
}

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
        playlist_insert_slot: None,
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
    assert_eq!(state.active_view, GuiShellView::Room);
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
fn gui_persisted_config_runtime_owner_prioritizes_player_setup_before_stream_helper_modal_for_playlist_urls()
 {
    let root = test_temp_root("shared-playlist-youtube-no-player");
    let config_path = root.join("syncplay.ini");
    let helper_bin_dir = root.join("tools").join("stream-helper").join("bin");
    std::fs::create_dir_all(&helper_bin_dir)
        .expect("managed helper bin dir should be created for playlist-url regression");
    std::fs::write(
        helper_bin_dir.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        }),
        b"not an executable",
    )
    .expect("invalid yt-dlp fixture should be written");
    std::fs::write(
        helper_bin_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" }),
        b"not an executable",
    )
    .expect("invalid deno fixture should be written");

    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(Some(config_path));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["https://www.youtube.com/watch?v=qDVPFAuBSXw".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|message| message.contains("Set playerPath to mpv")),
        "playlist URL opens without a player should keep the player setup blocker visible"
    );
    assert_eq!(
        owner.stream_helper_runtime_snapshot.target, None,
        "stream-helper preflight should not run before player attachment is available"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::OpenModal(GuiShellModal::StreamSupport)
        )),
        "playlist URL opens without a player should not open the stream-helper modal first"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_inserts_shared_playlist_media_at_requested_slot() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["episode1.mkv".to_owned(), "episode3.mkv".to_owned()],
        selected_index: Some(0),
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
                .eq(["episode1.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist seed before slot insert",
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode2.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
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
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist insert at requested slot",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist insert should report a runtime-backed success"
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_appends_shared_playlist_media_without_switching_selection() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
        selected_index: Some(0),
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
                .eq(["episode1.mkv", "episode2.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist seed before append",
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode3.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(2),
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
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
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist append preserves selection",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist append should report a runtime-backed success"
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_session_playlist_index_when_local_selection_is_stale_on_append()
 {
    let (mut owner, handle, mut state) = seeded_loopback_shared_playlist_owner(2);

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.main_window_playlist_selection_is_local);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode4.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(3),
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
                    "episode1.mkv",
                    "episode2.mkv",
                    "episode3.mkv",
                    "episode4.mkv",
                ])
        },
        "shared-playlist append with stale local selection",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(2));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index()),
        Some(2)
    );
    assert_eq!(owner.active_shared_playlist_index, Some(2));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode3.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_remaps_active_playlist_index_when_inserting_before_active() {
    let (mut owner, handle, mut state) = seeded_loopback_shared_playlist_owner(2);

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.main_window_playlist_selection_is_local);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode1-5.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
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
                    "episode1.mkv",
                    "episode1-5.mkv",
                    "episode2.mkv",
                    "episode3.mkv",
                ])
        },
        "shared-playlist insert before active entry",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(3));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index()),
        Some(3)
    );
    assert_eq!(owner.active_shared_playlist_index, Some(3));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode3.mkv")
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
        playlist_insert_slot: None,
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
        playlist_insert_slot: None,
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
        playlist_insert_slot: None,
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
