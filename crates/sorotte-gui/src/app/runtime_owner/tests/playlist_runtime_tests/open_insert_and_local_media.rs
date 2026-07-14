use super::*;

use crate::app::{GuiMediaSourceProviderId, GuiPlaylistDefaultSourceId};
use sorotte_plex::{
    PlexCachedMatch, PlexClientConfig, PlexMatchCache, PlexMediaType, parse_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

fn seeded_loopback_shared_playlist_owner(
    active_index: usize,
) -> (
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SorotteGuiShellAppState,
) {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode3.mkv")
            .with_path("C:/Media/episode3.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

fn seed_cached_plex_match_for_local_path(
    root: &std::path::Path,
    media_path: &std::path::Path,
    rating_key: &str,
    title: &str,
) {
    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let metadata = std::fs::metadata(media_path)
        .expect("cached Plex local-media fixture metadata should be readable");
    let file_name = media_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("cached Plex local-media fixture should have a UTF-8 file name");
    let local_file = sorotte_player_api::LocalFileUpdate::new(file_name)
        .with_path(media_path.to_string_lossy().into_owned())
        .with_size_bytes(metadata.len());
    let cache_key = server_scoped_cache_key_for_file(&plex_config, &local_file)
        .expect("server-scoped cache key should be available");
    let mut cache = PlexMatchCache::default();
    cache.entries.insert(
        cache_key,
        PlexCachedMatch {
            rating_key: rating_key.to_owned(),
            title: title.to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
    );
    cache
        .save_to_path(&root.join("cache").join("plex-watch-cache.json"))
        .expect("Plex cache should be written");
}

#[test]
fn gui_persisted_config_runtime_owner_publishes_cached_plex_uri_for_shared_local_media_open() {
    let root = test_temp_root("shared-playlist-local-plex-uri");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Media");
    std::fs::create_dir_all(&media_dir)
        .expect("shared-playlist Plex fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("shared-playlist Plex fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let local_file = sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
        .with_path(media_path_text.clone())
        .with_size_bytes(4);
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

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 1,
        "shared-playlist local media open should publish Plex URI",
    );

    let entries = state.current_shared_playlist_entries();
    let uri =
        parse_plex_playlist_uri(&entries[0]).expect("shared playlist entry should be a Plex URI");
    assert_eq!(uri.machine_identifier, "machine-1");
    assert_eq!(uri.rating_key, "123");
    assert_eq!(uri.file_name.as_deref(), Some("episode1.mkv"));
    assert_eq!(uri.title.as_deref(), Some("Episode 1"));
    assert_eq!(uri.duration_millis, Some(90_000));
    assert_eq!(uri.size_bytes, Some(4));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_cached_plex_drop_prefers_exact_local_media() {
    let root = test_temp_root("shared-playlist-automatic-cached-plex-local");
    let config_path = root.join("sorotte.ini");
    let drop_dir = root.join("Dropped");
    std::fs::create_dir_all(&drop_dir)
        .expect("cached Plex local-drop fixture directory should be created");
    let media_path = drop_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("cached Plex local-drop fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        state.main_window.playlist_default_source.current_source_id,
        GuiPlaylistDefaultSourceId::automatic()
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 1,
        "Automatic cached-Plex local drop should project into the shared playlist",
    );

    let entries = state.current_shared_playlist_entries();
    let uri = parse_plex_playlist_uri(&entries[0])
        .expect("peer-facing shared playlist entry should remain a Plex URI");
    assert_eq!(uri.machine_identifier, "machine-1");
    assert_eq!(uri.rating_key, "123");
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::local(),
        "Automatic should project a known local drop as Local even when its peer-facing entry is a Plex URI"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str()),
        "the selected drop should open the exact local filesystem path"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "Automatic local precedence must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_cached_plex_duplicate_drop_retains_exact_local_media()
 {
    let root = test_temp_root("shared-playlist-automatic-cached-plex-duplicate-local");
    let config_path = root.join("sorotte.ini");
    let drop_dir = root.join("DroppedOutsideSearchRoots");
    std::fs::create_dir_all(&drop_dir)
        .expect("cached Plex duplicate-drop fixture directory should be created");
    let current_media_path = drop_dir.join("current.mkv");
    let media_path = drop_dir.join("episode1.mkv");
    std::fs::write(&current_media_path, b"current")
        .expect("current local-media fixture should be written");
    std::fs::write(&media_path, b"test")
        .expect("cached Plex duplicate-drop fixture should be written");
    let current_media_path_text = current_media_path.to_string_lossy().into_owned();
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("current.mkv")
            .with_path(current_media_path_text.clone()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![media_path_text.clone()],
        )
        .expect("cached local file should produce a shared-playlist dispatch");
    let plex_uri = dispatch
        .playlist_entries
        .first()
        .cloned()
        .expect("cached local file should produce a Plex playlist URI");
    parse_plex_playlist_uri(&plex_uri)
        .expect("cached local file should publish a peer-facing Plex URI");

    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["current.mkv".to_owned(), plex_uri.clone()],
        selected_index: Some(0),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.current_shared_playlist_entries() == ["current.mkv".to_owned(), plex_uri.clone()]
                && state.main_window.active_playlist_index == Some(0)
        },
        "Automatic Plex row should be seeded behind the active local row",
    );
    assert_eq!(
        state.main_window.playlist[1]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream()
    );
    assert!(
        !state.main_window.playlist[1]
            .source_state
            .provider_selection_is_explicit,
        "the seeded Plex source should be inferred by Automatic"
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(2),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.playlist[1]
                .source_state
                .current_provider_id
                == GuiMediaSourceProviderId::local()
        },
        "duplicate cached-Plex drop should update the existing Automatic row to Local",
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["current.mkv".to_owned(), plex_uri],
        "duplicate append should remain a playlist no-op"
    );
    assert!(
        !state.main_window.playlist[1]
            .source_state
            .provider_selection_is_explicit,
        "Automatic local precedence should remain inferred rather than becoming a manual override"
    );
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str()),
        "the duplicate append should not interrupt the active local row"
    );
    assert!(owner.plex_stream_resolve_rx.is_none());

    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            == Some(media_path_text.as_str())
            || owner.plex_stream_resolve_rx.is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str()),
        "activating the deduplicated row should use the exact dropped filesystem path"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "activating a deduplicated local drop must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_deduplicated_local_drop_retains_first_exact_path() {
    let root = test_temp_root("shared-playlist-deduplicated-local-path-order");
    let first_dir = root.join("First");
    let second_dir = root.join("Second");
    std::fs::create_dir_all(&first_dir).expect("first duplicate-path directory should be created");
    std::fs::create_dir_all(&second_dir)
        .expect("second duplicate-path directory should be created");
    let first_path = first_dir.join("episode.mkv");
    let second_path = second_dir.join("episode.mkv");
    std::fs::write(&first_path, b"first").expect("first duplicate-path file should be written");
    std::fs::write(&second_path, b"second").expect("second duplicate-path file should be written");
    let first_path_text = first_path.to_string_lossy().into_owned();
    let second_path_text = second_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path_text.clone(), second_path_text],
        Some(0),
    );

    assert_eq!(
        state
            .current_shared_playlist_entries()
            .iter()
            .filter(|entry| entry.as_str() == "episode.mkv")
            .count(),
        1,
        "duplicate local targets should collapse to one accepted playlist entry"
    );
    assert_eq!(owner.local_shared_playlist_media_paths_by_target.len(), 1);
    assert_eq!(
        owner
            .local_shared_playlist_media_paths_by_target
            .values()
            .next(),
        Some(&std::path::PathBuf::from(&first_path_text)),
        "deduplication must retain the exact path paired with the first accepted entry"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(first_path_text.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reactivates_cached_plex_append_from_exact_dropped_path() {
    let root = test_temp_root("shared-playlist-cached-plex-append-local-hint");
    let config_path = root.join("sorotte.ini");
    let search_dir = root.join("ConfiguredMedia");
    let drop_dir = root.join("DroppedOutsideSearchRoots");
    std::fs::create_dir_all(&search_dir)
        .expect("configured media-search fixture directory should be created");
    std::fs::create_dir_all(&drop_dir)
        .expect("outside-root local-drop fixture directory should be created");
    let current_media_path = search_dir.join("episode1.mkv");
    let dropped_media_path = drop_dir.join("episode2.mkv");
    std::fs::write(&current_media_path, b"first")
        .expect("active local-media fixture should be written");
    std::fs::write(&dropped_media_path, b"second")
        .expect("appended local-media fixture should be written");
    let current_media_path_text = current_media_path.to_string_lossy().into_owned();
    let dropped_media_path_text = dropped_media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &dropped_media_path, "456", "Episode 2");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![search_dir.to_string_lossy().into_owned()]),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![current_media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.current_shared_playlist_entries().len() == 1
                && state.main_window.active_playlist_index == Some(0)
        },
        "initial local row should remain active before the cached-Plex append",
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str())
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![dropped_media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 2,
        "cached-Plex local drop should append without switching the active row",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert!(
        parse_plex_playlist_uri(&state.current_shared_playlist_entries()[1]).is_ok(),
        "peer-facing appended entry should remain a Plex URI"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str()),
        "appending should not interrupt the currently active local row"
    );
    assert!(owner.plex_stream_resolve_rx.is_none());

    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            == Some(dropped_media_path_text.as_str())
            || owner.plex_stream_resolve_rx.is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(dropped_media_path_text.as_str()),
        "later activation should reopen the exact appended path even though it is outside configured search roots"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "later activation of a retained local drop must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_uncached_plex_local_add_on_fast_path() {
    let root = test_temp_root("shared-playlist-local-plex-cache-miss");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Media");
    std::fs::create_dir_all(&media_dir)
        .expect("shared-playlist Plex miss fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("shared-playlist Plex miss fixture should exist");
    let media_path_text = media_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("not-a-valid-url".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![media_path_text.clone()],
        )
        .expect("uncached Plex local add should still produce a playlist entry");

    assert_eq!(dispatch.playlist_entries, vec!["episode1.mkv".to_owned()]);
    assert_eq!(dispatch.player_paths, Some(vec![media_path_text]));
    assert!(
        owner.pending_stream_feedback.is_empty(),
        "playlist publication must not run uncached Plex stream resolution before the row is projected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_shared_playlist_open_through_client_core_session_and_player()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_flushes_shared_playlist_before_player_open() {
    struct OutboundObservingPlayer {
        transport: crate::app::runtime_stack::GuiQueuedSessionTransportHandle,
        observed_outbound: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl PlayerAdapter for OutboundObservingPlayer {
        fn name(&self) -> &'static str {
            "outbound-observing"
        }

        fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.observed_outbound
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(self.transport.drain_outbound_protocol_lines());
            Ok(())
        }
    }

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let transport = owner
        .session_transport
        .as_ref()
        .expect("loopback owner should expose a session transport")
        .clone();
    let observed_outbound = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OutboundObservingPlayer {
        transport: transport.clone(),
        observed_outbound: observed_outbound.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode1.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let observed = observed_outbound
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outbound_at_open = observed
        .first()
        .expect("player open should observe outbound transport state");
    assert!(
        outbound_at_open
            .iter()
            .any(|line| line.contains("episode1.mkv")),
        "shared playlist transport update must be flushed before player open; outbound_at_open={outbound_at_open:?}"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_prioritizes_player_setup_before_stream_helper_modal_for_playlist_urls()
 {
    let root = test_temp_root("shared-playlist-youtube-no-player");
    let config_path = root.join("sorotte.ini");
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_applies_playlist_default_source_to_local_media_insert() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
        source_id: GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::plex_stream()),
    }));

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec!["C:/Media/episode1.mkv".to_owned()],
        Some(0),
    );

    assert_eq!(
        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream(),
        "local drag/drop additions should use the selected playlist default source"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "a newly selected Plex-default row should queue Plex stream resolution instead of opening local"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the selected local path must not be opened directly when the row source is Plex Stream"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_explicit_plex_default_wins_for_cached_plex_local_media_insert()
 {
    let root = test_temp_root("shared-playlist-explicit-plex-cached-local");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Dropped");
    std::fs::create_dir_all(&media_dir)
        .expect("explicit Plex cached-local fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test")
        .expect("explicit Plex cached-local fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
        source_id: GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::plex_stream()),
    }));

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![media_path_text.clone()],
        None,
    );

    let entries = state.current_shared_playlist_entries();
    parse_plex_playlist_uri(
        entries
            .first()
            .expect("cached local drop should project a shared-playlist row"),
    )
    .expect("cached local drop should retain its peer-facing Plex URI");
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream(),
        "an explicit Plex default must override local precedence"
    );
    assert!(
        state.main_window.playlist[0]
            .source_state
            .provider_selection_is_explicit,
        "the Plex default should be recorded as an explicit source choice"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the exact local path must not open directly when Plex was explicitly selected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_local_for_media_match_default_local_media_insert() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
            source_id: GuiPlaylistDefaultSourceId::provider(
                GuiMediaSourceProviderId::media_matching()
            ),
        })
    );

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec!["C:/Media/episode1.mkv".to_owned()],
        Some(0),
    );

    assert_eq!(
        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::local(),
        "a local drag/drop should stay local even when the playlist default is Media Matching"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
    assert!(
        owner.media_match_remote_lookup_rx.is_none(),
        "Media Matching should not run when the local file path is already available"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_appends_shared_playlist_media_without_switching_selection() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
