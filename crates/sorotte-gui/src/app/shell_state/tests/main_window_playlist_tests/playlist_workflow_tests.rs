use super::*;

#[test]
fn gui_shell_app_state_moves_and_removes_playlist_rows() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state
        .main_window
        .playlist
        .push(MainWindowPlaylistRow::inferred("Second", false));
    state
        .main_window
        .playlist
        .push(MainWindowPlaylistRow::inferred("Third", false));

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
fn gui_shell_app_state_tracks_plex_playlist_picker_lifecycle() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_url: Some("https://plex.example".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;

    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: String::new(),
    }));
    assert!(
        state
            .plex_playlist_search
            .as_ref()
            .is_some_and(|search| search.searching)
    );
    assert!(state.apply(GuiShellAction::CompletePlexPlaylistSearch {
        query: String::new(),
        results: vec![GuiPlexPlaylistSearchResult {
            rating_key: "14452".to_owned(),
            title: "Episode 11".to_owned(),
            parent_title: Some("Season 4".to_owned()),
            grandparent_title: Some("Re:Zero".to_owned()),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(1_470_058),
            file_name: Some("Episode 11.mkv".to_owned()),
        }],
        error: None,
    }));
    let search = state
        .plex_playlist_search
        .as_ref()
        .expect("picker should remain open");
    assert!(!search.searching);
    assert_eq!(search.selected_index, Some(0));
    assert_eq!(search.results[0].rating_key, "14452");

    assert!(state.apply(GuiShellAction::AddSelectedPlexPlaylistSearchResult));
    assert_eq!(
        state
            .plex_playlist_search
            .as_ref()
            .and_then(|search| search.adding_rating_key.as_deref()),
        Some("14452")
    );
    assert!(
        state.apply(GuiShellAction::CompletePlexPlaylistItemResolve {
            rating_key: "14452".to_owned(),
            error: None,
        })
    );
    assert!(
        state
            .plex_playlist_search
            .as_ref()
            .is_some_and(|search| search.adding_rating_key.is_none())
    );

    assert!(state.apply(GuiShellAction::AddSelectedPlexPlaylistSearchResult));
    assert!(
        !state.apply(GuiShellAction::CompletePlexPlaylistItemResolve {
            rating_key: "stale-worker-result".to_owned(),
            error: None,
        }),
        "a stale successful resolve must not clear a newer pending add"
    );
    assert_eq!(
        state
            .plex_playlist_search
            .as_ref()
            .and_then(|search| search.adding_rating_key.as_deref()),
        Some("14452")
    );

    assert!(
        state.apply(GuiShellAction::CompletePlexPlaylistItemResolve {
            rating_key: "14452".to_owned(),
            error: Some("Plex metadata 14452 did not include a playable part".to_owned()),
        })
    );
    assert_eq!(
        state
            .plex_playlist_search
            .as_ref()
            .and_then(|search| search.error.as_deref()),
        Some("Plex metadata 14452 did not include a playable part")
    );

    assert!(state.apply(GuiShellAction::CancelPlexPlaylistSearch));
    assert!(state.plex_playlist_search.is_none());
}

#[test]
fn gui_shell_app_state_rejects_stale_plex_search_completion_after_picker_reopens() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_url: Some("https://plex.example".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;

    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "first".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::CancelPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "second".to_owned(),
    }));

    assert!(
        !state.apply(GuiShellAction::CompletePlexPlaylistSearch {
            query: "first".to_owned(),
            results: vec![GuiPlexPlaylistSearchResult {
                rating_key: "stale".to_owned(),
                title: "Stale result".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: None,
                file_name: None,
            }],
            error: None,
        }),
        "a stale completion must not overwrite a reopened picker"
    );
    let search = state
        .plex_playlist_search
        .as_ref()
        .expect("reopened picker should remain active");
    assert!(search.searching);
    assert_eq!(search.query, "second");
    assert!(search.results.is_empty());

    assert!(state.apply(GuiShellAction::CompletePlexPlaylistSearch {
        query: "second".to_owned(),
        results: vec![GuiPlexPlaylistSearchResult {
            rating_key: "current".to_owned(),
            title: "Current result".to_owned(),
            parent_title: None,
            grandparent_title: None,
            media_type: PlexMediaType::Movie,
            duration_millis: None,
            file_name: None,
        }],
        error: None,
    }));
    let search = state
        .plex_playlist_search
        .as_ref()
        .expect("picker should remain open after current completion");
    assert!(!search.searching);
    assert_eq!(search.query, "second");
    assert_eq!(search.results[0].rating_key, "current");
}

#[test]
fn gui_shell_app_state_moves_playlist_rows_to_arbitrary_targets() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_shell_app_state_keeps_active_duplicate_attached_to_its_row_identity() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.apply_shared_playlist_entries(
        vec![
            "episode.mkv".to_owned(),
            "episode.mkv".to_owned(),
            "episode.mkv".to_owned(),
        ],
        Some(0),
        false,
    );
    let active_entry_id = state.main_window.playlist[1].entry_id;
    state.main_window.active_playlist_index = Some(1);

    assert!(state.move_main_window_playlist_row(0, 2));
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(state.main_window.playlist[0].entry_id, active_entry_id);

    state.set_main_window_playlist_selection(Some(1), true);
    assert!(state.move_selected_main_window_playlist(-1));
    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(state.main_window.playlist[1].entry_id, active_entry_id);

    state.set_main_window_playlist_selection(Some(0), true);
    assert!(state.remove_selected_main_window_playlist());
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(state.main_window.playlist[0].entry_id, active_entry_id);

    state.set_main_window_playlist_selection(Some(0), true);
    assert!(state.remove_selected_main_window_playlist());
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_ne!(state.main_window.playlist[0].entry_id, active_entry_id);
}

#[test]
fn gui_shell_app_state_preserves_distinct_duplicate_entry_ids_across_reorder_and_undo() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let original_entries = vec![
        "Duplicate.mkv".to_owned(),
        "Middle.mkv".to_owned(),
        "Duplicate.mkv".to_owned(),
        "Tail.mkv".to_owned(),
    ];
    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(
        original_entries.clone(),
    )));

    let original_ids = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.entry_id)
        .collect::<Vec<_>>();
    assert_ne!(original_ids[0], original_ids[2]);
    assert_eq!(
        original_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        original_ids.len(),
        "each duplicate occurrence must receive its own row identity"
    );

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 0,
        to_index: 2,
    }));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>(),
        vec![
            original_ids[1],
            original_ids[2],
            original_ids[0],
            original_ids[3],
        ],
        "moving a duplicate must move that occurrence's identity"
    );

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(state.current_shared_playlist_entries(), original_entries);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>(),
        original_ids,
        "undo must restore the original identity at every occurrence"
    );
}

#[test]
fn gui_shell_app_state_reuses_duplicate_entry_ids_through_shuffle_and_undo() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let original_entries = vec![
        "Duplicate.mkv".to_owned(),
        "Alpha.mkv".to_owned(),
        "Duplicate.mkv".to_owned(),
        "Beta.mkv".to_owned(),
        "Duplicate.mkv".to_owned(),
        "Gamma.mkv".to_owned(),
    ];
    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(
        original_entries.clone(),
    )));

    let original_ids = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.entry_id)
        .collect::<Vec<_>>();
    let original_id_set = original_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let original_duplicate_ids = state
        .main_window
        .playlist
        .iter()
        .filter(|row| row.label == "Duplicate.mkv")
        .map(|row| row.entry_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(original_id_set.len(), original_entries.len());
    assert_eq!(original_duplicate_ids.len(), 3);

    let mut shuffled = false;
    for _ in 0..16 {
        assert!(state.apply(GuiShellAction::ShuffleEntireSharedPlaylist));
        if state.current_shared_playlist_entries() != original_entries {
            shuffled = true;
            break;
        }
    }
    assert!(
        shuffled,
        "shuffle should eventually change the row ordering"
    );

    let shuffled_ids = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.entry_id)
        .collect::<std::collections::HashSet<_>>();
    let shuffled_duplicate_ids = state
        .main_window
        .playlist
        .iter()
        .filter(|row| row.label == "Duplicate.mkv")
        .map(|row| row.entry_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(shuffled_ids, original_id_set);
    assert_eq!(shuffled_duplicate_ids, original_duplicate_ids);

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(state.current_shared_playlist_entries(), original_entries);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>(),
        original_ids,
        "undo must restore the original order without reusing or colliding IDs"
    );
}

#[test]
fn gui_shell_app_state_preserves_selected_playlist_entry_when_reordering_another_row() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_shell_app_state_projects_playlist_source_defaults_and_disabled_options() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
        ]))
    );

    let row = state.main_window.playlist.first().unwrap();
    assert_eq!(row.source_state.current_label, "Local");
    assert_eq!(row.source_state.current_provider_id.as_str(), "local");
    assert!(
        row.source_state
            .options
            .iter()
            .any(|option| option.label == "Plex Stream"),
        "all registered source providers should be visible"
    );

    assert!(state.apply(GuiShellAction::SetPluginEnabled {
        plugin: GuiPluginSelection::MediaMatching,
        enabled: false,
    }));
    let media_matching = state.main_window.playlist[0]
        .source_state
        .options
        .iter()
        .find(|option| option.label == "Media Matching")
        .expect("Media Matching option should remain visible");
    assert!(!media_matching.enabled);
    assert_eq!(media_matching.status.label(), "disabled");
    assert!(
        media_matching
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("disabled"))
    );
}

#[test]
fn gui_shell_app_state_playlist_default_source_applies_only_to_new_rows() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
        ]))
    );

    assert!(
        state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
            source_id: GuiPlaylistDefaultSourceId::provider(
                GuiMediaSourceProviderId::media_matching()
            ),
        })
    );

    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            .as_str(),
        "local",
        "changing the playlist default must not rewrite existing row source selections"
    );
    assert!(
        state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
            "Episode 2".to_owned(),
        ]))
    );
    assert_eq!(
        state.main_window.playlist[1]
            .source_state
            .current_provider_id
            .as_str(),
        "media-matching",
        "new rows should prioritize the selected playlist default when it is available"
    );

    assert!(state.apply(GuiShellAction::SetMediaMatchFingerprintingEnabled(false)));
    assert!(
        state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
            "Episode 3".to_owned(),
        ]))
    );
    assert_eq!(
        state.main_window.playlist[2]
            .source_state
            .current_provider_id
            .as_str(),
        "local",
        "new rows should fall back to automatic inference when the selected default is unavailable"
    );
    assert_eq!(
        state
            .main_window
            .playlist_default_source
            .current_source_id
            .provider_id()
            .map(GuiMediaSourceProviderId::as_str),
        Some("media-matching"),
        "unavailable defaults stay selected globally so future settings can make them available again"
    );

    assert!(state.apply(GuiShellAction::SetMediaMatchFingerprintingEnabled(true)));
    assert!(
        state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
            "Episode 4".to_owned(),
        ]))
    );
    assert_eq!(
        state.main_window.playlist[3]
            .source_state
            .current_provider_id
            .as_str(),
        "media-matching",
        "future rows should use the selected playlist default again once it becomes available"
    );
}

#[test]
fn gui_shell_app_state_playlist_source_override_recovers_after_plugin_reenabled() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistSource {
        index: 0,
        provider_id: GuiMediaSourceProviderId::media_matching(),
    }));
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Resolving
    );

    assert!(state.apply(GuiShellAction::SetPluginEnabled {
        plugin: GuiPluginSelection::MediaMatching,
        enabled: false,
    }));
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            .as_str(),
        "media-matching"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Disabled
    );
    assert!(
        state.main_window.playlist[0]
            .source_state
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("disabled"))
    );

    assert!(state.apply(GuiShellAction::SetPluginEnabled {
        plugin: GuiPluginSelection::MediaMatching,
        enabled: true,
    }));
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            .as_str(),
        "media-matching",
        "plugin availability changes must not discard the row override"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Available,
        "re-enabled providers should recover from a transient disabled row state"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("Waiting for playlist activation.")
    );
}

#[test]
fn gui_shell_app_state_playlist_source_override_recovers_after_plex_runtime_unavailable() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistSource {
        index: 0,
        provider_id: GuiMediaSourceProviderId::plex_stream(),
    }));

    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            enabled: true,
            streaming_enabled: true,
            authenticated: false,
            selected_server_id: Some("machine-1".to_owned()),
            selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
            status: "Plex authentication expired.".to_owned(),
            ..GuiPlexRuntimeSnapshot::default()
        },
    )));
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            .as_str(),
        "plex-stream"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Disabled
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("Plex is not authenticated.")
    );

    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            enabled: true,
            streaming_enabled: true,
            authenticated: true,
            selected_server_id: Some("machine-1".to_owned()),
            selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
            status: "Plex connected.".to_owned(),
            ..GuiPlexRuntimeSnapshot::default()
        },
    )));
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id
            .as_str(),
        "plex-stream",
        "runtime availability changes must not discard the row override"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Available
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("Waiting for playlist activation.")
    );
}

#[test]
fn gui_shell_app_state_preserves_playlist_source_metadata_across_edits_and_undo() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        ]))
    );
    state.main_window.playlist[1].source_state.detail =
        Some("preserve this source detail".to_owned());

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 1,
        to_index: 0,
    }));
    let moved_row = state
        .main_window
        .playlist
        .iter()
        .find(|row| row.label == "B")
        .expect("moved row should still exist");
    assert_eq!(
        moved_row.source_state.detail.as_deref(),
        Some("preserve this source detail")
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(0)));
    assert!(state.apply(GuiShellAction::AnnounceSelectedSharedPlaylistEntryRemoved));
    assert!(
        state
            .main_window
            .playlist
            .iter()
            .all(|row| row.label != "B")
    );

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    let restored_row = state
        .main_window
        .playlist
        .iter()
        .find(|row| row.label == "B")
        .expect("undo should restore removed row");
    assert_eq!(
        restored_row.source_state.detail.as_deref(),
        Some("preserve this source detail")
    );
}

#[test]
fn gui_shell_app_state_ignores_duplicate_shared_playlist_additions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    let notification_count = state.notifications.len();
    let chat_count = state.main_window.chat.len();

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistEntryAdded(
            "Two".to_owned(),
        ))
    );
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["One".to_owned(), "Two".to_owned()]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(state.notifications.len(), notification_count);
    assert_eq!(state.main_window.chat.len(), chat_count);

    assert!(
        state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
            "Two".to_owned(),
            "Three".to_owned(),
            "Three".to_owned(),
        ]))
    );
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["One".to_owned(), "Two".to_owned(), "Three".to_owned()]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Shared playlist entry added: Three.")
    );
}

#[test]
fn gui_shell_app_state_filters_duplicate_playlist_insertions_from_media_open() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    let (duplicate_entries, duplicate_selected_index) = state
        .shared_playlist_entries_after_media_open_from_state(
            vec!["Episode 2.mkv".to_owned()],
            Some(2),
        );
    assert_eq!(
        duplicate_entries,
        vec!["Episode 1.mkv".to_owned(), "Episode 2.mkv".to_owned()]
    );
    assert_eq!(duplicate_selected_index, Some(1));

    let (entries, selected_index) = state.shared_playlist_entries_after_media_open_from_state(
        vec![
            "Episode 2.mkv".to_owned(),
            "Episode 3.mkv".to_owned(),
            "Episode 3.mkv".to_owned(),
        ],
        Some(2),
    );
    assert_eq!(
        entries,
        vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
            "Episode 3.mkv".to_owned(),
        ]
    );
    assert_eq!(selected_index, Some(1));
}

#[test]
fn gui_shell_app_state_announces_shared_playlist_events() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        !state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
        ]))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Shared playlist events are unavailable when shared playlists are disabled.")
    );

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    assert_eq!(
        GuiWidgetEguiRenderer::playlist_load_override_path_from_lookup(&|name| {
            (name == "SOROTTE_GUI_TEST_LOAD_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_save_override_path_from_lookup(&|name| {
            (name == "SOROTTE_GUI_TEST_SAVE_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
