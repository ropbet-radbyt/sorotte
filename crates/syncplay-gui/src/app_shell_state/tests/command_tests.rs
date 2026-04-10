use super::*;

#[test]
fn gui_shell_app_state_moves_and_removes_media_search_rows() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
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
    assert!(state.main_window.chat.is_empty());

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
    assert!(state.main_window.chat.is_empty());

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
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
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
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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

#[test]
fn gui_shell_app_state_handles_configuration_reset_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.commands.can_reset_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "draft.example".to_owned(),
    }));
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ResetConfiguration)
    );
    assert!(!state.commands.can_reset_configuration);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelConfigurationReset));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReset));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReset(
        state.saved_configuration.clone(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("saved.example")
    );
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("SavedRoom")
    );
    assert!(!state.commands.can_reset_configuration);
    assert!(state.main_window.chat.is_empty());
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_reset_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Configuration reset is unavailable with no unsaved changes.")
    );

    assert!(!state.apply(GuiShellAction::CompleteConfigurationReset(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration reset is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a configuration reset.")
    );
}

#[test]
fn gui_shell_app_state_handles_configuration_reload_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("before.example".to_owned()),
        room: Some("BeforeRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.commands.can_reload_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
    assert!(!state.commands.can_reload_configuration);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelConfigurationReload));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("dirty.example")
    );
    assert!(state.commands.can_reload_configuration);

    let replacement = StoredClientSettingsMvp {
        host: Some("after.example".to_owned()),
        room: Some("AfterRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        public_servers: Some(vec![(
            "Primary".to_owned(),
            "syncplay.example:8999".to_owned(),
        )]),
        ..StoredClientSettingsMvp::default()
    };
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReload(
        replacement.clone(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert_eq!(state.saved_configuration, replacement);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.main_window.chat.is_empty());
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|item| item.label == "About")
            .is_some_and(|item| !item.enabled)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_reload_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteConfigurationReload(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration reload is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginConfigurationReload));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelConfigurationReload));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a configuration reload.")
    );
}

#[test]
fn gui_shell_app_state_handles_clear_gui_data_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.active_view = GuiShellView::PublicServers;
    state.last_media_dialog_directory = Some("D:/Dialogs".to_owned());

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ClearGuiData)
    );
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Clear GUI data canceled.")
    );

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert_eq!(
        state.saved_configuration,
        StoredClientSettingsMvp::default()
    );
    assert!(state.public_servers.servers.is_empty());
    assert!(state.media_search.directories.is_empty());
    assert_eq!(state.last_media_dialog_directory, None);
    assert!(state.main_window.chat.is_empty());
}

#[test]
fn gui_shell_app_state_rejects_invalid_clear_gui_data_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No clear-GUI-data operation is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a clear-GUI-data request.")
    );
}

#[test]
fn gui_shell_app_state_tracks_configuration_text_edit_session_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.example".to_owned(),
    )));
    let rendered = state.render_lines().join("\n");
    assert!(
        rendered
            .contains("[Text Edit] editing=Connection / Host, dirty=yes, buffer=syncplay.example")
    );

    assert!(state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
    assert!(
        state
            .render_lines()
            .join("\n")
            .contains("[Text Edit] editing=(none)")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.cancelled".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
}

#[test]
fn gui_shell_app_state_tracks_focused_configuration_controls_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .autoplay_initial_state,
        Some(true)
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .text_edit_session
            .as_ref()
            .map(|session| session.label),
        Some("Host")
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    let rendered = state.render_lines().join("\n");
    assert!(
        rendered.contains("[Control Focus] focused=Connection / Host, kind=text, activations=1")
    );
    assert!(rendered.contains("[Text Edit] editing=Connection / Host"));

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );

    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert!(state.focused_configuration_control.is_none());
    assert!(!state.apply(GuiShellAction::ClearConfigurationControlFocus));
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_focus_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration control is currently focused.")
    );

    assert!(!state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Privacy",
        label: "Trusted Domain Count",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control is not focusable.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_text_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "OSD",
        label: "Show OSD",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control does not support text-edit sessions.")
    );

    assert!(!state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "orphan".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );

    assert!(!state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );
}

#[test]
fn gui_shell_app_state_tracks_pending_operations_and_busy_command_availability() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();

    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::RefreshPublicServers,
    )));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(!state.commands.can_reload_configuration);
    assert!(!state.commands.can_connect_public_server);
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_search_missing_media);
    assert!(!state.commands.can_toggle_pause);
    assert!(!state.commands.can_send_chat_message);

    let busy_render = state.render_lines().join("\n");
    assert!(busy_render.contains("[Commands] busy=yes"));
    assert!(busy_render.contains("[Pending] operation=refresh-public-servers"));

    assert!(!state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );

    assert!(state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(state.pending_operation, None);
    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(!state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No GUI operation is currently in progress.")
    );
}
