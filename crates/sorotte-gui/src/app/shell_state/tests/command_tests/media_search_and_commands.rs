use super::*;

#[test]
fn gui_shell_app_state_moves_and_removes_media_search_rows() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![
            "C:/Media".to_owned(),
            "D:/Archive".to_owned(),
            "E:/Incoming".to_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(2)));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryUp));
    assert_eq!(
        state
            .media_search
            .directories
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec!["C:/Media", "E:/Incoming", "D:/Archive"]
    );
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryDown));
    assert_eq!(state.selection.selected_media_search_directory, Some(2));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryUp));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));

    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert_eq!(
        state
            .media_search
            .directories
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec!["C:/Media", "D:/Archive"]
    );
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );

    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert!(!state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No media-search directory is currently selected.")
    );
    assert!(!state.commands.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_handles_media_search_event_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AnnounceMediaSearchDirectorySelected(0)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Media search directory selected: C:/Media.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "D:/Archive".to_owned(),
        ))
    );
    assert_eq!(state.media_search.directories.len(), 2);
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Media search directory added: D:/Archive.")
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SearchMissingMedia)
    );
    assert!(state.apply(GuiShellAction::CompleteMissingMediaSearch(Some(
        "movie.mkv".to_owned(),
    ))));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Media search directory added: D:/Archive.")
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert!(state.apply(GuiShellAction::CompleteMissingMediaSearch(None)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Missing media search completed: no match found.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_media_search_event_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::AnnounceMediaSearchDirectorySelected(0)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No media-search directory exists at the requested index.")
    );

    assert!(
        !state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory cannot be empty.")
    );

    assert!(!state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Missing-media search is unavailable when search actions are disabled.")
    );

    assert!(
        !state.apply(GuiShellAction::CompleteMissingMediaSearch(Some(
            "movie.mkv".to_owned(),
        )))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No missing-media search is currently in progress.")
    );
}

#[test]
fn gui_shell_app_state_handles_save_and_playback_toggle_command_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];
    state.refresh_validation();

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
    assert!(!state.commands.can_save_configuration);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelConfigurationSave));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration save canceled.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(state.apply(GuiShellAction::CompleteConfigurationSave(
        state.configuration.to_stored_settings(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_chat_pane_ready(&state.main_window.chat);

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::TogglePlaybackPause)
    );
    assert!(!state.commands.can_toggle_pause);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration save canceled.")
    );

    assert!(state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert_eq!(state.pending_operation, None);
    assert!(state.main_window.playback_paused);
    assert_chat_pane_ready(&state.main_window.chat);

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert!(state.apply(GuiShellAction::CancelPlaybackPauseToggle));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Playback toggle canceled.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_save_and_playback_toggle_command_actions() {
    let mut invalid_configuration_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(
        invalid_configuration_state.apply(GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Port",
            value: "70000".to_owned(),
        })
    );
    assert!(!invalid_configuration_state.commands.can_save_configuration);
    assert!(!invalid_configuration_state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        invalid_configuration_state
            .validation
            .last_action_error
            .as_deref(),
        Some("Configuration cannot be saved while validation issues remain.")
    );

    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];

    assert!(!state.apply(GuiShellAction::CompleteConfigurationSave(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration save is currently in progress.")
    );

    assert!(!state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback pause toggling is unavailable when pause controls are disabled.")
    );

    assert!(!state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No playback toggle is currently in progress.")
    );
}
