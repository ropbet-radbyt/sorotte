use super::*;

#[test]
fn gui_shell_app_state_moves_and_removes_playlist_rows() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.main_window.playlist.push(MainWindowPlaylistRow {
        label: "Second".to_owned(),
        is_selected: false,
    });
    state.main_window.playlist.push(MainWindowPlaylistRow {
        label: "Third".to_owned(),
        is_selected: false,
    });

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(2)));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Playlist pane ready for shared entries", "Third", "Second"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistDown));
    assert_eq!(state.selection.selected_main_window_playlist, Some(2));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::RemoveSelectedMainWindowPlaylist));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Playlist pane ready for shared entries", "Second"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert!(!state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The selected playlist row cannot move further.")
    );
}

#[test]
fn gui_shell_app_state_moves_playlist_rows_to_arbitrary_targets() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
            "Episode 2".to_owned(),
            "Episode 3".to_owned(),
        ]))
    );

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 2,
        to_index: 0,
    }));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Episode 3", "Episode 1", "Episode 2"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Episode 1", "Episode 2", "Episode 3"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
}

#[test]
fn gui_shell_app_state_preserves_selected_playlist_entry_when_reordering_another_row() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

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
        vec!["D", "A", "B", "C"]
    );
    assert_eq!(
        state.selection.selected_main_window_playlist,
        Some(2),
        "reordering a different row should keep the originally selected entry active"
    );

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B", "C", "D"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
}

#[test]
fn gui_shell_app_state_announces_shared_playlist_events() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["One", "Two"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Shared playlist loaded (2 entries).")
    );

    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(1)));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.main_window.playlist[1].is_selected);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Shared playlist selected: Two.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistEntryAdded(
            "Three".to_owned(),
        ))
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(state.main_window.playlist[2].label, "Three");

    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(2)));
    assert!(state.apply(GuiShellAction::AnnounceSelectedSharedPlaylistEntryRemoved));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["One", "Two"]
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Shared playlist entry removed: Three.")
    );

    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(Vec::new())));
    assert!(state.main_window.playlist.is_empty());
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Shared playlist cleared.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_shared_playlist_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
        ]))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Shared playlist events are unavailable when shared playlists are disabled.")
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        !state.apply(GuiShellAction::AnnounceSharedPlaylistEntryAdded(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Shared playlist entries must be non-empty.")
    );
    assert!(!state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(1)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No shared playlist entry exists at the requested index.")
    );
}

#[test]
fn gui_shell_app_state_tracks_playlist_workflow_editors_undo_and_shuffle() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
            "Episode 3.mkv".to_owned(),
            "Episode 4.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert_eq!(
        state
            .playlist_text_edit_session
            .as_ref()
            .map(|session| session.buffer.as_str()),
        Some("Episode 1.mkv\nEpisode 2.mkv\nEpisode 3.mkv\nEpisode 4.mkv")
    );
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistTextEdit(
        "Episode 1.mkv\nhttps://example.com/live".to_owned(),
    )));
    let replacement_entries = super::playlist_entries_from_multiline_text(
        state
            .playlist_text_edit_session
            .as_ref()
            .expect("playlist text edit session should remain active")
            .buffer
            .as_str(),
    );
    assert_eq!(
        replacement_entries,
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert!(state.apply(GuiShellAction::ReplaceSharedPlaylistEntries(
        replacement_entries.clone(),
    )));
    assert_eq!(state.current_shared_playlist_entries(), replacement_entries);
    assert!(state.apply(GuiShellAction::CancelSharedPlaylistTextEdit));
    assert!(state.playlist_text_edit_session.is_none());

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));
    assert!(
        state.apply(GuiShellAction::UpdateSharedPlaylistUrlEdit(
            "https://example.com/next\nhttps://example.com/bonus\nhttps://example.com/finale"
                .to_owned(),
        ))
    );
    let appended_entries = super::playlist_entries_from_multiline_text(
        state
            .playlist_url_edit_session
            .as_ref()
            .expect("playlist URL edit session should remain active")
            .buffer
            .as_str(),
    );
    assert_eq!(
        appended_entries,
        vec![
            "https://example.com/next".to_owned(),
            "https://example.com/bonus".to_owned(),
            "https://example.com/finale".to_owned(),
        ]
    );
    assert!(state.apply(GuiShellAction::AppendSharedPlaylistEntries(
        appended_entries.clone(),
    )));
    assert!(state.apply(GuiShellAction::CancelSharedPlaylistUrlEdit));
    assert!(state.playlist_url_edit_session.is_none());
    let entries_before_shuffle = state.current_shared_playlist_entries();
    assert_eq!(
        entries_before_shuffle,
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
            "https://example.com/next".to_owned(),
            "https://example.com/bonus".to_owned(),
            "https://example.com/finale".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    let mut shuffled_remaining = false;
    for _ in 0..4 {
        if state.apply(GuiShellAction::ShuffleRemainingSharedPlaylist) {
            shuffled_remaining = true;
            break;
        }
    }
    assert!(
        shuffled_remaining,
        "remaining-playlist shuffle should eventually permute the tail"
    );
    let entries_after_remaining_shuffle = state.current_shared_playlist_entries();
    assert_eq!(
        &entries_after_remaining_shuffle[..2],
        &entries_before_shuffle[..2]
    );
    let mut expected_tail = entries_before_shuffle[2..].to_vec();
    let mut actual_tail = entries_after_remaining_shuffle[2..].to_vec();
    expected_tail.sort();
    actual_tail.sort();
    assert_eq!(actual_tail, expected_tail);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_before_shuffle
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_after_remaining_shuffle
    );

    let mut shuffled_entire = false;
    for _ in 0..4 {
        if state.apply(GuiShellAction::ShuffleEntireSharedPlaylist) {
            shuffled_entire = true;
            break;
        }
    }
    assert!(
        shuffled_entire,
        "entire-playlist shuffle should eventually permute the playlist"
    );
    let entries_after_entire_shuffle = state.current_shared_playlist_entries();
    let mut expected_entries = entries_after_remaining_shuffle.clone();
    let mut actual_entries = entries_after_entire_shuffle.clone();
    expected_entries.sort();
    actual_entries.sort();
    assert_eq!(actual_entries, expected_entries);
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_after_remaining_shuffle
    );

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));
    assert!(state.apply(GuiShellAction::UpdateMediaUrlEdit(
        "https://media.example/stream".to_owned(),
    )));
    assert_eq!(
        state
            .media_url_edit_session
            .as_ref()
            .map(|session| (session.buffer.as_str(), session.is_dirty)),
        Some(("https://media.example/stream", true))
    );
    assert!(state.apply(GuiShellAction::CancelMediaUrlEdit));
    assert!(state.media_url_edit_session.is_none());
}

#[test]
fn gui_playlist_file_helpers_roundtrip_and_track_file_actions() {
    let root = test_temp_root("playlist-file-helpers");
    let playlist_path = root.join("shared-playlist.m3u");
    let playlist_path_string = playlist_path.to_string_lossy().into_owned();

    super::save_playlist_entries_to_path(
        &playlist_path_string,
        &[
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ],
    )
    .expect("playlist entries should save to disk");
    assert_eq!(
        std::fs::read_to_string(&playlist_path).expect("saved playlist file should be readable"),
        "Episode 1.mkv\nhttps://example.com/live"
    );

    std::fs::write(
        &playlist_path,
        " Episode 1.mkv \n\n https://example.com/live \n",
    )
    .expect("playlist fixture should be updated");
    assert_eq!(
        super::load_playlist_entries_from_path(&playlist_path_string)
            .expect("playlist entries should load from disk"),
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );

    assert_eq!(
        GuiWidgetEguiRenderer::playlist_load_override_path_from_lookup(&|name| {
            (name == "SYNCPLAY_GUI_TEST_LOAD_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_save_override_path_from_lookup(&|name| {
            (name == "SYNCPLAY_GUI_TEST_SAVE_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::LoadSharedPlaylistFromFile {
        path: playlist_path_string.clone(),
        entries: vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ],
        shuffled: false,
    }));
    let expected_load_message =
        format!("Shared playlist loaded from file: {playlist_path_string}.");
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(
        state.last_media_dialog_directory.as_deref(),
        Some(root.to_string_lossy().as_ref())
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some(expected_load_message.as_str())
    );

    assert!(state.apply(GuiShellAction::SaveSharedPlaylistToFile(
        playlist_path_string.clone(),
    )));
    let expected_save_message = format!("Shared playlist saved to file: {playlist_path_string}.");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some(expected_save_message.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}
