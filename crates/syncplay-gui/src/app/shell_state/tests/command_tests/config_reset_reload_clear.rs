use super::*;

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
    assert_eq!(state.active_view, GuiShellView::Setup);
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
    state.active_view = GuiShellView::Setup;
    state.last_media_dialog_directory = Some("D:/Dialogs".to_owned());

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ClearGuiData)
    );
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Clear GUI data canceled.")
    );

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Setup);
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
