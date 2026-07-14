use super::*;
use sorotte_plex::{
    PlexCachedMatch, PlexClientConfig, PlexMatchCache, PlexMediaType, parse_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

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
    let relative_media_root = playlist_root.join("media");
    std::fs::create_dir_all(&relative_media_root)
        .expect("playlist fixture directory should be created");
    let relative_media_path = relative_media_root.join("episode1.mkv");
    std::fs::write(&relative_media_path, b"relative media")
        .expect("relative M3U media fixture should be written");
    let absolute_media_path = root.join("episode2.mkv");
    std::fs::write(&absolute_media_path, b"absolute media")
        .expect("absolute M3U media fixture should be written");
    let absolute_media_file_url = reqwest::Url::from_file_path(&absolute_media_path)
        .expect("absolute M3U media fixture should convert to a file URL");
    let missing_absolute_path = root.join("private").join("missing-absolute.mkv");
    let missing_slash_absolute_path = missing_absolute_path
        .to_string_lossy()
        .replace('\\', "/")
        .replacen(":/", "://", 1);
    let playlist_path = playlist_root.join("room-playlist.m3u");
    std::fs::write(
        &playlist_path,
        format!(
            "\u{feff}#EXTM3U\n#EXTINF:120,Episode 1\nmedia/episode1.mkv\n{}\n{}\n  # ignored comment\nhttps://example.com/live?id=1\nmissing/episode3.mkv\n{}\n{}\n/home/alice/rooted-unix.mkv\n\\Users\\alice\\rooted-windows.mkv\nfile:/C:/Users/alice/file-single-slash.mkv\nFiLe:C:/Users/alice/file-opaque.mkv\n",
            absolute_media_path.to_string_lossy(),
            absolute_media_file_url,
            missing_absolute_path.to_string_lossy(),
            missing_slash_absolute_path,
        ),
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
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "https://example.com/live?id=1".to_owned(),
            "missing/episode3.mkv".to_owned(),
            "missing-absolute.mkv".to_owned(),
            "missing-absolute.mkv".to_owned(),
            "rooted-unix.mkv".to_owned(),
            "rooted-windows.mkv".to_owned(),
            "file-single-slash.mkv".to_owned(),
            "file-opaque.mkv".to_owned(),
        ]
    );
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(relative_media_path.to_string_lossy().as_ref())
    );
    assert_eq!(
        dispatch.items[1].local_origin.as_deref(),
        Some(absolute_media_path.to_string_lossy().as_ref())
    );
    assert_eq!(
        dispatch.items[2].local_origin.as_deref(),
        Some(absolute_media_path.to_string_lossy().as_ref())
    );
    assert!(
        dispatch.items[3..]
            .iter()
            .all(|item| item.local_origin.is_none())
    );
    assert!(dispatch.playlist_entries().iter().all(|entry| {
        !entry.contains(root.to_string_lossy().as_ref())
            && !entry.contains("/home/alice")
            && !entry.contains("\\Users\\alice")
            && !entry.contains("C:/Users/alice")
            && !entry.to_ascii_lowercase().starts_with("file:")
    }));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_treats_non_hls_m3u8_as_a_typed_playlist_import() {
    let root = test_temp_root("shared-playlist-non-hls-m3u8-format");
    let media_root = root.join("media");
    std::fs::create_dir_all(&media_root).expect("M3U8 media directory should be created");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"M3U8 media").expect("M3U8 media fixture should be written");
    let playlist_path = root.join("room-playlist.m3u8");
    std::fs::write(
        &playlist_path,
        "#EXTM3U\n#EXTINF:120,Episode 1\nmedia/episode1.mkv\nhttps://example.com/live\n",
    )
    .expect("non-HLS M3U8 fixture should be written");

    let dispatch = GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(vec![
        playlist_path.to_string_lossy().into_owned(),
    ])
    .expect("non-HLS M3U8 should be imported as a playlist");

    assert!(dispatch.imported_from_file);
    assert_eq!(
        dispatch.playlist_entries(),
        vec![
            "episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(media_path.to_string_lossy().as_ref())
    );
    assert!(dispatch.items[1].local_origin.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_publishes_cached_plex_uri_for_existing_m3u_local_file() {
    let root = test_temp_root("shared-playlist-m3u-cached-plex");
    let config_path = root.join("sorotte.ini");
    let playlist_root = root.join("lists");
    let media_root = playlist_root.join("media");
    std::fs::create_dir_all(&media_root).expect("M3U media directory should be created");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"cached plex media")
        .expect("cached Plex M3U media fixture should be written");
    let playlist_path = playlist_root.join("room-playlist.m3u");
    std::fs::write(&playlist_path, "#EXTM3U\nmedia/episode1.mkv\n")
        .expect("cached Plex M3U fixture should be written");

    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let metadata = std::fs::metadata(&media_path).expect("M3U media metadata should be readable");
    let local_file = sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
        .with_path(media_path.to_string_lossy().into_owned())
        .with_size_bytes(metadata.len());
    let cache_key = server_scoped_cache_key_for_file(&plex_config, &local_file)
        .expect("server-scoped cache key should be available");
    let mut cache = PlexMatchCache::default();
    cache.entries.insert(
        cache_key,
        PlexCachedMatch {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
    );
    cache
        .save_to_path(&root.join("cache").join("plex-watch-cache.json"))
        .expect("Plex cache should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![playlist_path.to_string_lossy().into_owned()],
        )
        .expect("cached Plex M3U should be imported");

    let published = dispatch.playlist_entries();
    let uri = parse_plex_playlist_uri(&published[0])
        .expect("existing M3U local media should publish its cached Plex URI");
    assert_eq!(uri.machine_identifier, "machine-1");
    assert_eq!(uri.rating_key, "123");
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(media_path.to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_toolbar_shuffle_keeps_duplicate_m3u_origins_typed() {
    let root = test_temp_root("shared-playlist-m3u-toolbar-shuffle-origins");
    let first_root = root.join("first");
    let second_root = root.join("second");
    std::fs::create_dir_all(&first_root).expect("first M3U media directory should be created");
    std::fs::create_dir_all(&second_root).expect("second M3U media directory should be created");
    let first_path = first_root.join("episode.mkv");
    let second_path = second_root.join("episode.mkv");
    std::fs::write(&first_path, b"first").expect("first duplicate media fixture should be written");
    std::fs::write(&second_path, b"second")
        .expect("second duplicate media fixture should be written");
    let playlist_path = root.join("duplicates.m3u");
    std::fs::write(
        &playlist_path,
        "#EXTM3U\nfirst/episode.mkv\nsecond/episode.mkv\n",
    )
    .expect("duplicate M3U fixture should be written");

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

    handle.push_request(GuiRuntimeRequest::ImportSharedPlaylistFile {
        path: playlist_path.to_string_lossy().into_owned(),
        shuffled: true,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "toolbar M3U shuffle should import duplicate local entries",
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "episode.mkv".to_owned()]
    );
    assert!(
        state
            .current_shared_playlist_entries()
            .iter()
            .all(|entry| !entry.contains(root.to_string_lossy().as_ref()))
    );
    let bound_origins = owner
        .playlist_resolution
        .local_origins_by_row
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        bound_origins,
        [first_path, second_path]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );
    assert_eq!(owner.playlist_resolution.local_origins_by_row.len(), 2);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_toolbar_import_reports_unreadable_playlist() {
    let root = test_temp_root("shared-playlist-toolbar-import-error");
    let missing_playlist = root.join("missing.m3u");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let playlist_before_import = state.current_shared_playlist_entries();

    handle.push_request(GuiRuntimeRequest::ImportSharedPlaylistFile {
        path: missing_playlist.to_string_lossy().into_owned(),
        shuffled: false,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification { level, message }
            if *level == GuiTransientNotificationLevel::Error
                && message.contains("Shared playlist import failed reading")
    )));
    assert_eq!(
        state.current_shared_playlist_entries(),
        playlist_before_import
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
