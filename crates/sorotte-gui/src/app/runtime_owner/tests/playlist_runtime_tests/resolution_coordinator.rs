use super::*;
use crate::app::runtime_owner::{
    GuiPendingPlaylistSourceResolution, player::SelectedPlaylistMediaSyncOutcome,
};
use crate::app::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiMediaSourceProviderId, GuiPlaylistSourceState,
    GuiPlaylistSourceStatus,
};

use sorotte_plex::{
    PlexCachedMatch, PlexClientConfig, PlexMatchCache, PlexMediaType, parse_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

fn detached_playlist_owner_and_state(
    config_path: Option<std::path::PathBuf>,
    room: &str,
    plex_enabled: bool,
) -> (
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SorotteGuiShellAppState,
) {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(config_path);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(plex_enabled),
        plex_sync_enabled: Some(plex_enabled),
        plex_streaming_enabled: Some(plex_enabled),
        plex_user_token: plex_enabled.then(|| "user-token".into()),
        plex_selected_server_id: plex_enabled.then(|| "machine-1".to_owned()),
        plex_selected_server_url: plex_enabled.then(|| "http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: plex_enabled.then(|| "server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    (owner, handle, state)
}

fn active_client_core_playlist_adapter() -> GuiClientCoreChatSessionRuntimeAdapter {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core playlist adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup hello should encode");
    assert_eq!(startup_lines.len(), 1);
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true,"chat":true}}}"#,
        )
        .expect("server hello should activate playlist control");
    assert!(GuiSessionRuntimeAdapter::playlist_control_available(
        &adapter
    ));
    adapter
}

fn apply_session_runtime_actions(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    state: &mut SorotteGuiShellAppState,
) {
    let actions = owner
        .session
        .as_mut()
        .expect("test owner should have a session")
        .drain_gui_actions(state);
    for action in actions {
        assert!(
            state.apply(action),
            "session projection should apply cleanly"
        );
    }
}

fn same_basename_media_paths(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let first_dir = root.join("First");
    let second_dir = root.join("Second");
    std::fs::create_dir_all(&first_dir).expect("first media directory should be created");
    std::fs::create_dir_all(&second_dir).expect("second media directory should be created");
    let first_path = first_dir.join("episode.mkv");
    let second_path = second_dir.join("episode.mkv");
    std::fs::write(&first_path, b"first").expect("first media fixture should be written");
    std::fs::write(&second_path, b"second").expect("second media fixture should be written");
    (first_path, second_path)
}

fn seed_cached_plex_versions(root: &std::path::Path, media_paths: &[std::path::PathBuf]) {
    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let mut cache = PlexMatchCache::default();
    for path in media_paths {
        let metadata = std::fs::metadata(path).expect("Plex version metadata should be readable");
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("Plex version file name should be UTF-8");
        let local_file = sorotte_player_api::LocalFileUpdate::new(file_name)
            .with_path(path.to_string_lossy().into_owned())
            .with_size_bytes(metadata.len());
        let cache_key = server_scoped_cache_key_for_file(&plex_config, &local_file)
            .expect("Plex version should have a cache key");
        cache.entries.insert(
            cache_key,
            PlexCachedMatch {
                rating_key: "same-rating-key".to_owned(),
                title: "Same episode".to_owned(),
                media_type: PlexMediaType::Episode,
                duration_millis: Some(90_000),
            },
        );
    }
    cache
        .save_to_path(&root.join("cache").join("plex-watch-cache.json"))
        .expect("Plex version cache should be written");
}

fn round_trip_main_window_runtime_snapshot(
    state: &SorotteGuiShellAppState,
) -> SorotteGuiShellAppState {
    let snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    let mut round_tripped =
        SorotteGuiShellAppState::from_stored_settings(&state.configuration.to_stored_settings());
    assert!(round_tripped.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot,)));
    round_tripped
}

fn activate_playlist_row_and_assert_exact_local_origin(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    index: usize,
    expected_path: &std::path::Path,
) {
    let expected_path = expected_path.to_string_lossy().into_owned();
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(index));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(owner, handle, state);
        if owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            == Some(expected_path.as_str())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(state.main_window.active_playlist_index, Some(index));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(expected_path.as_str())
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none()
            && owner.plex_stream_resolve_result.is_none()
            && owner.plex_stream_resolve_trigger_key.is_none(),
        "an exact retained local origin must not start or retain Plex resolution"
    );
}

#[test]
fn full_replacement_binds_same_basename_local_paths_to_distinct_row_ids() {
    let root = test_temp_root("playlist-row-origin-full-replacement");
    let (first_path, second_path) = same_basename_media_paths(&root);
    let first_path_text = first_path.to_string_lossy().into_owned();
    let second_path_text = second_path.to_string_lossy().into_owned();
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path_text.clone(), second_path_text.clone()],
        None,
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "episode.mkv".to_owned()]
    );
    let first_id = state.main_window.playlist[0].entry_id;
    let second_id = state.main_window.playlist[1].entry_id;
    assert_ne!(first_id, second_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path)
    );

    state = round_trip_main_window_runtime_snapshot(&state);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert_eq!(state.main_window.playlist[0].entry_id, first_id);
    assert_eq!(state.main_window.playlist[1].entry_id, second_id);

    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    let _ = handle.drain_actions();
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        0,
        &first_path,
    );
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget,
        "an idle sync should leave the first duplicate's trigger cached"
    );
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        1,
        &second_path,
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn active_session_same_label_full_replacement_rebinds_without_a_wire_change() {
    let root = test_temp_root("playlist-row-origin-active-noop-replacement");
    let (first_path, second_path) = same_basename_media_paths(&root);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path.to_string_lossy().into_owned()],
        None,
    );
    let first_id = state.main_window.playlist[0].entry_id;
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );
    let first_lines = owner
        .session
        .as_mut()
        .expect("active session should remain attached")
        .flush_outbound_protocol_lines()
        .expect("first replacement should encode");
    assert!(
        first_lines
            .iter()
            .any(|line| line.contains("\"playlistChange\"")),
        "the first replacement should publish the playlist label"
    );

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![second_path.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned()]
    );
    let second_id = state.main_window.playlist[0].entry_id;
    assert_ne!(
        second_id, first_id,
        "a full replacement must allocate a fresh local provenance identity"
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path),
        "the fresh row must bind the newly dropped exact path"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("Added from the local filesystem.")
    );

    let second_lines = owner
        .session
        .as_mut()
        .expect("active session should remain attached")
        .flush_outbound_protocol_lines()
        .expect("no-op replacement delivery should remain healthy");
    assert!(
        second_lines.iter().all(|line| {
            !line.contains("\"playlistChange\"") && !line.contains("\"playlistIndex\"")
        }),
        "identical wire labels and index must not publish a redundant playlist mutation"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn same_label_full_replacement_undo_restores_immediately_previous_exact_origin() {
    let root = test_temp_root("playlist-row-origin-full-replacement-undo");
    let (first_path, second_path) = same_basename_media_paths(&root);
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path.to_string_lossy().into_owned()],
        None,
    );
    let first_id = state.main_window.playlist[0].entry_id;
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![second_path.to_string_lossy().into_owned()],
        None,
    );
    let second_id = state.main_window.playlist[0].entry_id;
    assert_ne!(second_id, first_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path)
    );
    assert_eq!(state.playlist_entry_id_undo_snapshot, Some(vec![first_id]));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert_eq!(state.main_window.playlist[0].entry_id, first_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path),
        "undo must restore the immediately previous A-row origin, not an older same-label snapshot"
    );
    assert_ne!(state.main_window.playlist[0].entry_id, second_id);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pure_duplicate_reorder_undo_restores_row_origins_and_pending_index() {
    let root = test_temp_root("playlist-row-origin-duplicate-reorder-undo");
    let (first_path, second_path) = same_basename_media_paths(&root);
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);
    state.main_window.playback.can_manage_playlist = true;

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![
            first_path.to_string_lossy().into_owned(),
            second_path.to_string_lossy().into_owned(),
        ],
        None,
    );
    owner.reconcile_local_shared_playlist_media_paths(&state);

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "episode.mkv".to_owned()]
    );
    let first_id = state.main_window.playlist[0].entry_id;
    let second_id = state.main_window.playlist[1].entry_id;
    state.main_window.active_playlist_index = Some(1);
    state.set_main_window_playlist_selection(Some(1), true);
    state.apply_selection_to_surfaces();
    let generation = owner.playlist_resolution.generation;
    owner.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
        index: 1,
        entry_id: second_id,
        generation,
        target: "episode.mkv".to_owned(),
        provider_id: GuiMediaSourceProviderId::plex_stream(),
    });

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 1,
        to_index: 0,
    }));
    owner.reconcile_local_shared_playlist_media_paths(&state);

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "episode.mkv".to_owned()],
        "the visible playlist is unchanged when duplicate labels exchange places"
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>(),
        vec![second_id, first_id]
    );
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        state.main_window.playlist[0].entry_id, second_id,
        "the active and selected duplicate must follow its identity during reorder"
    );
    assert_eq!(
        state.playlist_entry_id_undo_snapshot,
        Some(vec![first_id, second_id]),
        "the reorder must snapshot row identity even though its labels are unchanged"
    );
    assert_eq!(
        state.playlist_source_undo_snapshot.as_ref().map(|sources| {
            sources
                .iter()
                .map(|source| source.entry_id)
                .collect::<Vec<_>>()
        }),
        Some(vec![first_id, second_id])
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path)
    );
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.entry_id == second_id && pending.index == 0),
        "pending resolution must follow the duplicate row by identity during reorder"
    );

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    owner.reconcile_local_shared_playlist_media_paths(&state);

    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(
        state.main_window.playlist[1].entry_id, second_id,
        "undo must keep the active and selected duplicate attached to its restored exact origin"
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path)
    );
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.entry_id == second_id && pending.index == 1),
        "undo must return pending resolution to the restored duplicate row index"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn duplicate_plex_identity_versions_bind_their_distinct_local_origins() {
    let root = test_temp_root("playlist-row-origin-plex-versions");
    let first_dir = root.join("1080p");
    let second_dir = root.join("4k");
    std::fs::create_dir_all(&first_dir).expect("1080p version directory should be created");
    std::fs::create_dir_all(&second_dir).expect("4k version directory should be created");
    let first_path = first_dir.join("episode-1080p.mkv");
    let second_path = second_dir.join("episode-4k.mkv");
    std::fs::write(&first_path, b"small").expect("1080p version should be written");
    std::fs::write(&second_path, b"larger-version").expect("4k version should be written");
    seed_cached_plex_versions(&root, &[first_path.clone(), second_path.clone()]);
    let (mut owner, handle, mut state) =
        detached_playlist_owner_and_state(Some(root.join("sorotte.ini")), "room1", true);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![
            first_path.to_string_lossy().into_owned(),
            second_path.to_string_lossy().into_owned(),
        ],
        None,
    );

    let entries = state.current_shared_playlist_entries();
    assert_eq!(entries.len(), 2);
    assert_ne!(
        entries[0], entries[1],
        "version metadata should make the published rows distinguishable"
    );
    let first_uri = parse_plex_playlist_uri(&entries[0]).expect("first row should be a Plex URI");
    let second_uri = parse_plex_playlist_uri(&entries[1]).expect("second row should be a Plex URI");
    assert_eq!(first_uri.machine_identifier, second_uri.machine_identifier);
    assert_eq!(first_uri.rating_key, second_uri.rating_key);

    let first_id = state.main_window.playlist[0].entry_id;
    let second_id = state.main_window.playlist[1].entry_id;
    assert_ne!(first_id, second_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path)
    );
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&second_id),
        Some(&second_path)
    );

    state = round_trip_main_window_runtime_snapshot(&state);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert_eq!(state.main_window.playlist[0].entry_id, first_id);
    assert_eq!(state.main_window.playlist[1].entry_id, second_id);

    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    let _ = handle.drain_actions();
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        0,
        &first_path,
    );
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget,
        "an idle sync should leave the first Plex version's trigger cached"
    );
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        1,
        &second_path,
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn room_runtime_snapshot_change_clears_same_basename_local_origin() {
    let root = test_temp_root("playlist-row-origin-room-change");
    let (first_path, _) = same_basename_media_paths(&root);
    let first_path_text = first_path.to_string_lossy().into_owned();
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path_text],
        None,
    );
    let entry_id = state.main_window.playlist[0].entry_id;
    assert_eq!(state.main_window.playlist[0].label, "episode.mkv");
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&entry_id),
        Some(&first_path)
    );

    let mut room_two_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room2".to_owned()),
            player_path: Some("mpv".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    room_two_state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    room_two_state.main_window.active_playlist_index = Some(0);
    let room_two_snapshot =
        MainWindowRuntimeSnapshot::from_shell_state(&room_two_state.main_window);
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        room_two_snapshot,
    )));
    let room_two_entry_id = state.main_window.playlist[0].entry_id;
    assert_ne!(room_two_entry_id, entry_id);
    assert_eq!(state.main_window.room_name, "room2");
    assert_eq!(state.main_window.playlist[0].label, "episode.mkv");

    owner.reconcile_local_shared_playlist_media_paths(&state);

    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert_eq!(
        owner.playlist_resolution.room_name.as_deref(),
        Some("room2")
    );

    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    let _ = handle.drain_actions();
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(0));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        owner.player_local_file.is_none(),
        "a fresh same-basename row in another room must not consume Room A's origin"
    );
    assert!(
        !owner
            .playlist_resolution
            .local_origins_by_row
            .contains_key(&room_two_entry_id)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn undo_collision_does_not_bind_unrelated_same_label_row_to_retained_origin() {
    let root = test_temp_root("playlist-row-origin-undo");
    let (first_path, _) = same_basename_media_paths(&root);
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path.to_string_lossy().into_owned()],
        None,
    );
    let original_id = state.main_window.playlist[0].entry_id;

    assert!(
        state.apply(GuiShellAction::ReplaceSharedPlaylistEntries(vec![
            "replacement.mkv".to_owned(),
        ]))
    );
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .contains_key(&original_id),
        "the removed row's exact origin should remain retained only for undo"
    );

    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    let unrelated_id = state.main_window.playlist[0].entry_id;
    assert_ne!(unrelated_id, original_id);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&original_id),
        Some(&first_path)
    );
    assert!(
        !owner
            .playlist_resolution
            .local_origins_by_row
            .contains_key(&unrelated_id)
    );

    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    let _ = handle.drain_actions();
    owner.active_shared_playlist_index = Some(0);
    state.main_window.active_playlist_index = Some(0);
    let _ = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert!(
        owner.player_local_file.is_none(),
        "an unrelated fresh-ID row with the same label must not consume the retained undo origin"
    );

    assert!(
        state.apply(GuiShellAction::UndoSharedPlaylistChange),
        "undo should remain available after the runtime snapshot collision: error={:?}, snapshot={:?}, current={:?}",
        state.validation.last_action_error,
        state.playlist_undo_snapshot,
        state.current_shared_playlist_entries(),
    );
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert_eq!(state.main_window.playlist[0].entry_id, original_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&original_id),
        Some(&first_path)
    );
    assert!(
        !owner
            .playlist_resolution
            .local_origins_by_row
            .contains_key(&unrelated_id)
    );
    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        0,
        &first_path,
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn typed_origin_deleted_before_projection_is_not_bound_marked_or_opened() {
    let root = test_temp_root("playlist-row-origin-deleted-before-projection");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"test").expect("deleted-origin fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);
    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![media_path_text.clone()],
        )
        .expect("existing file should produce a typed dispatch");
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(media_path_text.as_str())
    );
    let expected_entries = dispatch.playlist_entries();

    std::fs::remove_file(&media_path).expect("origin should be deleted before projection");
    owner.open_shared_playlist_dispatch_runtime_impl(
        &handle,
        &mut state,
        vec![media_path_text],
        dispatch,
        None,
    );

    assert_eq!(state.current_shared_playlist_entries(), expected_entries);
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert_eq!(
        state.main_window.playlist[0].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::Inferred
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Missing
    );
    assert!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            != GuiMediaSourceProviderId::local()
            || state.main_window.playlist[0].source_state.status
                != GuiPlaylistSourceStatus::Available,
        "a deleted typed origin must never be projected as Local/Available"
    );
    assert!(owner.player_local_file.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn nonexistent_paths_and_directories_are_rejected_before_playlist_dispatch() {
    let root = test_temp_root("playlist-row-origin-invalid-filesystem-targets");
    let directory = root.join("Season 1");
    std::fs::create_dir_all(&directory).expect("invalid directory fixture should be created");
    let nonexistent = root.join("missing-episode.mkv");
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);
    let original_playlist = state.current_shared_playlist_entries();

    for invalid_path in [nonexistent, directory] {
        let invalid_path = invalid_path.to_string_lossy().into_owned();
        assert!(
            owner
                .shared_playlist_open_dispatch_for_selected_paths_impl(
                    &state,
                    vec![invalid_path.clone()],
                )
                .is_err(),
            "non-filesystem targets must be rejected before typed dispatch"
        );
        owner.open_media_files_through_shared_playlist_runtime_impl(
            &handle,
            &mut state,
            vec![invalid_path],
            None,
        );
        assert_eq!(state.current_shared_playlist_entries(), original_playlist);
        assert!(owner.player_local_file.is_none());
        assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn mixed_deduplicated_and_new_drop_binds_both_rows_by_identity() {
    let root = test_temp_root("playlist-origin-mixed-dedup-and-insert");
    let existing_path = root.join("existing.mkv");
    let new_path = root.join("new.mkv");
    std::fs::write(&existing_path, b"existing").expect("existing fixture should be written");
    std::fs::write(&new_path, b"new").expect("new fixture should be written");
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);
    state.apply_shared_playlist_entries(vec!["existing.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    owner.active_shared_playlist_index = Some(0);
    let existing_id = state.main_window.playlist[0].entry_id;

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![
            existing_path.to_string_lossy().into_owned(),
            new_path.to_string_lossy().into_owned(),
        ],
        Some(1),
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["existing.mkv".to_owned(), "new.mkv".to_owned()]
    );
    let new_id = state.main_window.playlist[1].entry_id;
    assert_ne!(existing_id, new_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&existing_id),
        Some(&existing_path),
        "the deduplicated item must still bind its accepted typed origin"
    );
    assert_eq!(
        owner.playlist_resolution.local_origins_by_row.get(&new_id),
        Some(&new_path),
        "the newly inserted item must bind its own typed origin"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejected_full_replacement_does_not_advance_scope_or_cancel_pending_row() {
    let root = test_temp_root("playlist-origin-rejected-replacement-scope");
    let replacement_path = root.join("replacement.mkv");
    std::fs::write(&replacement_path, b"replacement")
        .expect("replacement fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["current.mkv".to_owned()], Some(0), false);
    let entry_id = state.main_window.playlist[0].entry_id;
    owner.reconcile_local_shared_playlist_media_paths(&state);
    let generation = owner.playlist_resolution.generation;
    owner.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
        index: 0,
        entry_id,
        generation,
        target: "current.mkv".to_owned(),
        provider_id: GuiMediaSourceProviderId::plex_stream(),
    });

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![replacement_path.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(owner.playlist_resolution.generation, generation);
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.entry_id == entry_id && pending.generation == generation),
        "a rejected replacement must not cancel resolution for the still-visible row"
    );
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["current.mkv".to_owned()]
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn same_session_playlist_revision_invalidates_same_label_origin_scope() {
    struct RevisionSession {
        revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    impl GuiSessionRuntimeAdapter for RevisionSession {
        fn current_room_playlist_revision(&self) -> Option<u64> {
            Some(self.revision.load(std::sync::atomic::Ordering::Relaxed))
        }

        fn current_room_playlist_remote_revision(&self) -> u64 {
            self.revision.load(std::sync::atomic::Ordering::Relaxed)
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

    let root = test_temp_root("playlist-origin-session-revision");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"episode").expect("session revision fixture should be written");
    let revision = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RevisionSession {
            revision: revision.clone(),
        }),
    );
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    let entry_id = state.main_window.playlist[0].entry_id;
    owner.reconcile_local_shared_playlist_media_paths(&state);
    owner
        .playlist_resolution
        .local_origins_by_row
        .insert(entry_id, media_path);
    owner.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
        index: 0,
        entry_id,
        generation: owner.playlist_resolution.generation,
        target: "episode.mkv".to_owned(),
        provider_id: GuiMediaSourceProviderId::plex_stream(),
    });
    let prior_generation = owner.playlist_resolution.generation;

    revision.store(2, std::sync::atomic::Ordering::Relaxed);
    owner.reconcile_local_shared_playlist_media_paths(&state);

    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert!(owner.pending_playlist_source_resolution.is_none());
    assert!(owner.playlist_resolution.generation > prior_generation);
    assert_eq!(owner.playlist_resolution.playlist_revision, Some(2));
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    assert_ne!(state.main_window.playlist[0].entry_id, entry_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn client_core_self_echo_preserves_origins_but_partial_remote_replacement_freshens_scope() {
    let root = test_temp_root("playlist-origin-client-core-echo-and-remote");
    let episode_path = root.join("episode.mkv");
    let old_path = root.join("old.mkv");
    std::fs::write(&episode_path, b"episode").expect("episode fixture should be written");
    std::fs::write(&old_path, b"old").expect("old fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![
            episode_path.to_string_lossy().into_owned(),
            old_path.to_string_lossy().into_owned(),
        ],
        None,
    );
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "old.mkv".to_owned()]
    );
    let episode_id = state.main_window.playlist[0].entry_id;
    let old_id = state.main_window.playlist[1].entry_id;
    let local_revision = owner
        .session
        .as_ref()
        .and_then(|session| session.current_room_playlist_revision())
        .expect("local playlist revision should be projected");
    assert_eq!(
        owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .current_room_playlist_remote_revision(),
        0
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("Added from the local filesystem.")
    );

    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv","old.mkv"],"user":"alice"}}}"#,
        )
        .expect("matching self-echo should apply");
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(
        !owner.apply_pending_playlist_row_scope_reset(&mut state),
        "a matching self-echo must not create a new row scope"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_revision()),
        Some(local_revision)
    );
    assert_eq!(state.main_window.playlist[0].entry_id, episode_id);
    assert_eq!(state.main_window.playlist[1].entry_id, old_id);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&episode_id),
        Some(&episode_path)
    );
    assert_eq!(
        owner.playlist_resolution.local_origins_by_row.get(&old_id),
        Some(&old_path)
    );

    state.playlist_undo_snapshot = Some(vec!["stale.mkv".to_owned()]);
    state.playlist_source_undo_snapshot = Some(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.source_state.clone())
            .collect(),
    );
    state.playlist_entry_id_undo_snapshot = Some(vec![episode_id, old_id]);
    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv","new.mkv"],"user":"bob"}}}"#,
        )
        .expect("partial remote replacement should apply");
    apply_session_runtime_actions(&mut owner, &mut state);
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["episode.mkv".to_owned(), "new.mkv".to_owned()]
    );
    assert_eq!(
        state.main_window.playlist[0].entry_id, episode_id,
        "the metadata-free wire snapshot demonstrates the stale-ID migration risk before scope reconciliation"
    );

    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert_ne!(state.main_window.playlist[0].entry_id, episode_id);
    assert_ne!(state.main_window.playlist[1].entry_id, old_id);
    for row in &state.main_window.playlist {
        assert_eq!(row.source_state.policy, GuiPlaylistSourcePolicy::Automatic);
        assert_eq!(
            row.source_state.selection_origin,
            GuiPlaylistSourceSelectionOrigin::Inferred
        );
        assert_eq!(
            row.source_state.detail.as_deref(),
            Some("Waiting for playlist activation.")
        );
    }
    assert!(state.playlist_undo_snapshot.is_none());
    assert!(state.playlist_source_undo_snapshot.is_none());
    assert!(state.playlist_entry_id_undo_snapshot.is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn separated_session_remove_and_install_each_freshen_same_label_row_scope() {
    let root = test_temp_root("playlist-origin-session-generation-transitions");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"episode").expect("session generation fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(!owner.apply_pending_playlist_row_scope_reset(&mut state));

    let original_id = state.main_window.playlist[0].entry_id;
    assert!(state.set_playlist_source_state(
        0,
        GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::plex_stream()),
    ));
    owner
        .playlist_resolution
        .local_origins_by_row
        .insert(original_id, media_path.clone());
    let installed_generation = owner.session_generation;

    owner.remove_session_runtime();
    assert!(owner.session_generation > installed_generation);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    let disconnected_id = state.main_window.playlist[0].entry_id;
    assert_ne!(disconnected_id, original_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());

    assert!(state.set_playlist_source_state(
        0,
        GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::plex_stream()),
    ));
    owner.install_session_runtime(Box::new(active_client_core_playlist_adapter()));
    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    assert_ne!(state.main_window.playlist[0].entry_id, disconnected_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::Inferred
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn same_room_detached_to_connected_session_replacement_resets_row_scope() {
    let root = test_temp_root("playlist-origin-detached-to-connected-session");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"episode")
        .expect("detached session origin fixture should be written");
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("configured connect scope test should bind a listener");
    let address = listener
        .local_addr()
        .expect("configured connect scope listener should expose an address");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some(address.ip().to_string()),
        port: Some(address.port()),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .ensure_detached_client_core_chat_session(&state)
        .expect("detached session should bootstrap");
    assert!(!owner.session_projects_to_shell);

    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    assert!(state.set_playlist_source_state(
        0,
        GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::plex_stream()),
    ));
    owner.reconcile_local_shared_playlist_media_paths(&state);
    let detached_generation = owner.session_generation;
    let stale_entry_id = state.main_window.playlist[0].entry_id;
    owner
        .playlist_resolution
        .local_origins_by_row
        .insert(stale_entry_id, media_path);
    owner.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
        index: 0,
        entry_id: stale_entry_id,
        generation: owner.playlist_resolution.generation,
        target: "episode.mkv".to_owned(),
        provider_id: GuiMediaSourceProviderId::plex_stream(),
    });

    owner.complete_saved_server_connect_runtime(&handle, &mut state, false);
    assert!(owner.session_projects_to_shell);
    assert!(
        owner.session_generation > detached_generation,
        "replacing the detached session must advance the explicit session generation"
    );
    assert_eq!(state.main_window.room_name, "room1");

    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert!(owner.pending_playlist_source_resolution.is_none());
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    assert_ne!(state.main_window.playlist[0].entry_id, stale_entry_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );

    drop(owner);
    drop(listener);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn intervening_remote_revision_is_not_hidden_by_a_later_local_mutation() {
    let root = test_temp_root("playlist-origin-remote-then-local-before-reconcile");
    let episode_path = root.join("episode.mkv");
    let old_path = root.join("old.mkv");
    std::fs::write(&episode_path, b"episode").expect("episode fixture should be written");
    std::fs::write(&old_path, b"old").expect("old fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![
            episode_path.to_string_lossy().into_owned(),
            old_path.to_string_lossy().into_owned(),
        ],
        None,
    );
    let old_ids = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.entry_id)
        .collect::<Vec<_>>();
    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv","old.mkv"],"user":"alice"}}}"#,
        )
        .expect("matching self-echo should apply");

    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv","remote.mkv"],"user":"bob"}}}"#,
        )
        .expect("remote replacement should apply");
    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .replace_playlist(
            vec!["episode.mkv".to_owned(), "local.mkv".to_owned()],
            Some(0),
        )
        .expect("later local replacement should apply optimistically");
    apply_session_runtime_actions(&mut owner, &mut state);
    assert_eq!(
        owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .current_room_playlist_remote_revision(),
        1,
        "the remote generation must remain observable after the newer local mutation"
    );

    owner.reconcile_local_shared_playlist_media_paths(&state);
    assert!(owner.apply_pending_playlist_row_scope_reset(&mut state));
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert!(
        state
            .main_window
            .playlist
            .iter()
            .all(|row| !old_ids.contains(&row.entry_id))
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_scope_is_reset_before_a_following_local_drop_binds_its_fresh_row() {
    let root = test_temp_root("playlist-origin-remote-before-local-drop");
    let original_path = root.join("episode.mkv");
    let dropped_path = root.join("after-remote.mkv");
    std::fs::write(&original_path, b"original").expect("original fixture should be written");
    std::fs::write(&dropped_path, b"dropped").expect("drop fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("room1".to_owned()),
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
    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![original_path.to_string_lossy().into_owned()],
        None,
    );
    let original_id = state.main_window.playlist[0].entry_id;
    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv"],"user":"alice"}}}"#,
        )
        .expect("matching self-echo should apply");
    assert!(state.set_playlist_source_state(
        0,
        GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::plex_stream()),
    ));

    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode.mkv"],"user":"bob"}}}"#)
        .expect("same-label remote replacement should apply");
    apply_session_runtime_actions(&mut owner, &mut state);
    owner.reconcile_playlist_resolution_scope(&handle, &mut state);
    assert_ne!(state.main_window.playlist[0].entry_id, original_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic,
        "the remote scope must be reset before any following command can consume ForcePlex"
    );
    assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
    assert!(
        owner.plex_stream_resolve_rx.is_none()
            && owner.plex_stream_resolve_result.is_none()
            && owner.plex_stream_resolve_trigger_key.is_none(),
        "the stale ForcePlex row must not start Plex resolution before the reset"
    );

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![dropped_path.to_string_lossy().into_owned()],
        Some(1),
    );
    let dropped_row = state
        .main_window
        .playlist
        .iter()
        .find(|row| row.label == "after-remote.mkv")
        .expect("the following local drop should append a fresh row");
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&dropped_row.entry_id),
        Some(&dropped_path),
        "the following local drop must bind after, and survive, the remote scope reset"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_duplicate_reorder_and_insert_freshen_every_occurrence_identity() {
    let root = test_temp_root("playlist-origin-remote-duplicate-generations");
    for (scenario, remote_entries) in [
        (
            "reorder",
            vec!["episode.mkv".to_owned(), "episode.mkv".to_owned()],
        ),
        (
            "insert",
            vec![
                "episode.mkv".to_owned(),
                "episode.mkv".to_owned(),
                "episode.mkv".to_owned(),
            ],
        ),
    ] {
        let scenario_root = root.join(scenario);
        let (first_path, second_path) = same_basename_media_paths(&scenario_root);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_session_runtime(Box::new(active_client_core_playlist_adapter()));
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        owner.open_media_files_through_shared_playlist_runtime_impl(
            &handle,
            &mut state,
            vec![
                first_path.to_string_lossy().into_owned(),
                second_path.to_string_lossy().into_owned(),
            ],
            None,
        );
        let old_ids = state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>();
        owner
            .session
            .as_mut()
            .expect("session should remain attached")
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode.mkv","episode.mkv"],"user":"alice"}}}"#,
            )
            .expect("matching duplicate self-echo should apply");

        let remote_line = serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": remote_entries,
                    "user": "bob",
                }
            }
        })
        .to_string();
        owner
            .session
            .as_mut()
            .expect("session should remain attached")
            .apply_message_json(&remote_line)
            .expect("remote duplicate generation should apply");
        apply_session_runtime_actions(&mut owner, &mut state);
        owner.reconcile_local_shared_playlist_media_paths(&state);
        assert!(
            owner.apply_pending_playlist_row_scope_reset(&mut state),
            "remote duplicate {scenario} should advance the row scope"
        );
        assert!(owner.playlist_resolution.local_origins_by_row.is_empty());
        assert!(
            state
                .main_window
                .playlist
                .iter()
                .all(|row| !old_ids.contains(&row.entry_id)),
            "remote duplicate {scenario} must not migrate any occurrence identity"
        );
        assert!(state.main_window.playlist.iter().all(|row| {
            row.source_state.policy == GuiPlaylistSourcePolicy::Automatic
                && row.source_state.selection_origin == GuiPlaylistSourceSelectionOrigin::Inferred
        }));
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn append_preserves_prior_row_exact_origin_for_later_reactivation() {
    let root = test_temp_root("playlist-origin-append-preserves-prior");
    let first_path = root.join("outside-search-a").join("first.mkv");
    let second_path = root.join("outside-search-b").join("second.mkv");
    std::fs::create_dir_all(
        first_path
            .parent()
            .expect("first path should have a parent"),
    )
    .expect("first fixture directory should be created");
    std::fs::create_dir_all(
        second_path
            .parent()
            .expect("second path should have a parent"),
    )
    .expect("second fixture directory should be created");
    std::fs::write(&first_path, b"first").expect("first fixture should be written");
    std::fs::write(&second_path, b"second").expect("second fixture should be written");
    let (mut owner, handle, mut state) = detached_playlist_owner_and_state(None, "room1", false);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path.to_string_lossy().into_owned()],
        None,
    );
    let first_id = state.main_window.playlist[0].entry_id;
    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![second_path.to_string_lossy().into_owned()],
        Some(1),
    );

    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .get(&first_id),
        Some(&first_path),
        "an append must retain the accepted origin for every surviving row"
    );
    owner.player_local_file = None;
    owner.last_attached_media_resolution_trigger = None;
    let _ = handle.drain_actions();
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        1,
        &second_path,
    );
    activate_playlist_row_and_assert_exact_local_origin(
        &mut owner,
        &handle,
        &mut state,
        0,
        &first_path,
    );

    let _ = std::fs::remove_dir_all(root);
}
