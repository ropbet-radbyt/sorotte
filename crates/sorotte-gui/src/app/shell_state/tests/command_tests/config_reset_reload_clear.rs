use super::*;
use crate::app::{
    GuiConfigStorageRuntimeSnapshot, GuiSavedServerConnectIntent, GuiSettingApplyRequirement,
};

#[test]
fn gui_shell_app_state_handles_discard_configuration_changes_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.commands.can_reset_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "draft.example".to_owned().into(),
    }));
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::DiscardConfigurationChanges)
    );
    assert!(!state.commands.can_reset_configuration);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::CancelDiscardConfigurationChanges));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert!(
        state.apply(GuiShellAction::CompleteDiscardConfigurationChanges(
            state.saved_configuration.clone(),
        ))
    );
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
    assert_chat_pane_ready(&state.main_window.chat);
}

#[test]
fn gui_shell_app_state_rejects_invalid_discard_configuration_changes_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Discard changes is unavailable with no unsaved changes.")
    );

    assert!(
        !state.apply(GuiShellAction::CompleteDiscardConfigurationChanges(
            StoredClientSettingsMvp::default(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No discard-changes operation is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "dirty.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelDiscardConfigurationChanges));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not discard changes.")
    );
}

#[test]
fn gui_shell_app_state_rejects_clean_configuration_save_in_reducer_and_widget_availability() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.commands.can_save_configuration);
    assert!(
        state
            .configuration_widget_tree()
            .find("config-command:save")
            .is_none_or(|button| !button.enabled)
    );

    assert!(!state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Save changes is unavailable with no unsaved changes.")
    );
    assert_eq!(state.pending_operation, None);
    assert!(!state.commands.can_save_configuration);
    assert!(
        state
            .configuration_widget_tree()
            .find("config-command:save")
            .is_none_or(|button| !button.enabled)
    );
}

#[test]
fn gui_shell_app_state_handles_configuration_reload_command_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("before.example".to_owned()),
        room: Some("BeforeRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.commands.can_reload_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "dirty.example".to_owned().into(),
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
    assert_chat_pane_ready(&state.main_window.chat);
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteConfigurationReload(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration reload is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "dirty.example".to_owned().into(),
    }));
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.active_view = GuiShellView::Setup;
    state.last_media_dialog_directory = Some("D:/Dialogs".to_owned());

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.clear_gui_data_confirmation_visible);
    assert_eq!(state.pending_operation, None);
    let confirmation_tree = state.configuration_widget_tree();
    assert!(
        confirmation_tree
            .find("configuration:clear-gui-data-confirmation")
            .is_some()
    );
    assert!(
        confirmation_tree
            .find("config-command:confirm-clear-gui-data")
            .is_some_and(|node| node.enabled)
    );
    assert!(
        confirmation_tree
            .find("config-command:cancel-clear-gui-data")
            .is_some_and(|node| node.enabled)
    );
    assert!(state.apply(GuiShellAction::DismissClearGuiDataConfirmation));
    assert!(!state.clear_gui_data_confirmation_visible);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.apply(GuiShellAction::ConfirmClearGuiData));
    assert!(!state.clear_gui_data_confirmation_visible);
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
    assert!(state.apply(GuiShellAction::ConfirmClearGuiData));
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
    assert_chat_pane_ready(&state.main_window.chat);
}

#[test]
fn gui_shell_app_state_rejects_invalid_clear_gui_data_command_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ConfirmClearGuiData));
    assert!(!state.apply(GuiShellAction::DismissClearGuiDataConfirmation));
    assert!(!state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No clear-GUI-data operation is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "dirty.example".to_owned().into(),
    }));
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
fn gui_shell_app_state_settles_secret_draft_after_config_storage_save() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        server_password: Some("old-secret".into()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "new-secret".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        "C:/SorotteConfig".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));

    let persisted = state.configuration.to_stored_settings();
    assert!(
        state.apply(GuiShellAction::CompleteConfigStorageRootChange {
            snapshot: GuiConfigStorageRuntimeSnapshot {
                config_path: Some("C:/SorotteConfig/sorotte.ini".to_owned()),
                storage_root: Some("C:/SorotteConfig".to_owned()),
                default_storage_root: Some("C:/Default".to_owned()),
                source_label: "custom".to_owned(),
                external_override_active: false,
            },
            settings: persisted,
        })
    );

    assert_eq!(
        state.configuration.server_password,
        crate::app::SecretDraft::Unchanged
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::ConnectionServerPassword),
        Some("")
    );
    assert!(!state.has_unsaved_configuration_changes());
}

#[test]
fn gui_shell_app_state_requires_pending_config_location_to_be_saved_before_save_and_connect() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        "C:/SorotteConfig".to_owned(),
    )));
    assert_eq!(
        state
            .configuration_widget_tree()
            .find("config-command:save-and-connect")
            .map(|button| button.enabled),
        Some(false)
    );

    assert!(!state.apply(GuiShellAction::BeginSaveAndConnect));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Save the pending config-location change before using Save & connect.")
    );
    assert_eq!(
        state.pending_config_storage_target,
        Some(crate::app::GuiConfigStorageChangeTarget::CustomRoot(
            "C:/SorotteConfig".to_owned()
        ))
    );
    assert_eq!(state.pending_operation, None);
}

#[test]
fn gui_shell_app_state_connect_once_only_blocks_connection_identity_errors() {
    let saved = StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::StreamingQuality,
        value: "not-a-quality-preset".to_owned().into(),
    }));
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| { issue.setting_id == Some(SettingId::StreamingQuality) })
    );
    assert!(state.apply(GuiShellAction::BeginConnectOnce));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::ConnectOnce)
    );
    assert!(state.apply(GuiShellAction::CancelSavedServerConnect));

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionPort,
        value: "70000".to_owned().into(),
    }));
    assert!(!state.apply(GuiShellAction::BeginConnectOnce));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Configured server connect requires a saved host and a valid port.")
    );
    assert_eq!(state.pending_operation, None);
}

#[test]
fn gui_shell_app_state_discard_clears_active_configuration_edit_buffers() {
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("saved-room".to_owned()),
        room_list: Some(vec!["saved-room".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "draft.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    assert!(state.apply(GuiShellAction::UpdateRoomHistoryEdit(
        "stale-room".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::ConnectionRoom,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::ConnectionRoom,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "stale-room".to_owned().into(),
    )));

    assert!(state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert!(
        state.apply(GuiShellAction::CompleteDiscardConfigurationChanges(
            saved.clone(),
        ))
    );

    assert!(state.text_edit_session.is_none());
    assert!(state.room_history_edit_session.is_none());
    assert!(state.focused_configuration_control.is_none());
    assert_eq!(state.configuration.to_stored_settings(), saved);
    assert!(!state.has_unsaved_configuration_changes());
    assert!(!state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(state.configuration.to_stored_settings(), saved);
}

#[test]
fn gui_shell_app_state_reload_clears_active_secret_edit_and_staged_storage_target() {
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        server_password: Some("saved-secret".into()),
        ..StoredClientSettingsMvp::default()
    };
    let replacement = StoredClientSettingsMvp {
        host: Some("disk.example".to_owned()),
        server_password: Some("disk-secret".into()),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        "C:/StagedSorotteConfig".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::ConnectionServerPassword,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::ConnectionServerPassword,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "replacement-secret".to_owned().into(),
    )));

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReload(
        replacement.clone(),
    )));

    assert!(state.text_edit_session.is_none());
    assert!(state.focused_configuration_control.is_none());
    assert_eq!(state.pending_config_storage_target, None);
    assert_eq!(
        state.configuration.server_password,
        crate::app::SecretDraft::Unchanged
    );
    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert!(!state.has_unsaved_configuration_changes());
    assert!(!state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert!(!format!("{state:?}").contains("replacement-secret"));
}

#[test]
fn configuration_changes_expose_typed_apply_requirements_and_save_follow_up() {
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        player_path: Some("C:/mpv/mpv.exe".to_owned()),
        language: Some("en".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "next.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlayerExecutable,
        value: "C:/mpv-next/mpv.exe".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "pt_BR".to_owned().into(),
    }));

    let tree = state.configuration_widget_tree();
    assert_eq!(
        tree.find("settings.connection.host.apply-requirement")
            .and_then(|node| node.value.as_deref()),
        Some("Reconnect required")
    );
    assert_eq!(
        tree.find("settings.player.executable.apply-requirement")
            .and_then(|node| node.value.as_deref()),
        Some("Player restart required")
    );
    assert_eq!(
        tree.find("settings.general.language.apply-requirement")
            .and_then(|node| node.value.as_deref()),
        Some("Sorotte restart required")
    );

    let persisted = state.configuration.to_stored_settings();
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(state.apply(GuiShellAction::CompleteConfigurationSave(persisted)));
    assert_eq!(
        state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Configuration saved.")
    );
    assert_eq!(
        state.pending_apply_requirements,
        Vec::<GuiSettingApplyRequirement>::new(),
        "save completion stays generic, while the runtime owner supplies the exact active-state requirements",
    );
    assert!(
        state.apply(GuiShellAction::ApplyPendingApplyRequirementsSnapshot(vec![
            GuiSettingApplyRequirement::Reconnect,
            GuiSettingApplyRequirement::PlayerSettingsRetryAvailable,
            GuiSettingApplyRequirement::RestartPlayer,
            GuiSettingApplyRequirement::RestartApplication,
        ]))
    );
    assert_eq!(
        state.pending_apply_requirements,
        vec![
            GuiSettingApplyRequirement::Reconnect,
            GuiSettingApplyRequirement::PlayerSettingsRetryAvailable,
            GuiSettingApplyRequirement::RestartPlayer,
            GuiSettingApplyRequirement::RestartApplication,
        ]
    );
    state.notifications.clear();
    let clean_tree = state.configuration_widget_tree();
    assert!(clean_tree.find("configuration:changes").is_none());
    for requirement in [
        GuiSettingApplyRequirement::Reconnect,
        GuiSettingApplyRequirement::PlayerSettingsRetryAvailable,
        GuiSettingApplyRequirement::RestartPlayer,
        GuiSettingApplyRequirement::RestartApplication,
    ] {
        assert_eq!(
            clean_tree
                .find(&format!(
                    "configuration:pending-apply:{}",
                    requirement.automation_id()
                ))
                .and_then(|node| node.value.as_deref()),
            Some(requirement.label())
        );
    }

    state.pending_operation = Some(GuiPendingOperationState {
        kind: GuiPendingOperationKind::ConnectSavedServer,
    });
    state.pending_saved_server_connect_intent = Some(GuiSavedServerConnectIntent::ConnectOnce);
    assert!(state.apply(GuiShellAction::CompleteSavedServerConnect));
    assert_eq!(
        state.pending_apply_requirements,
        vec![
            GuiSettingApplyRequirement::Reconnect,
            GuiSettingApplyRequirement::PlayerSettingsRetryAvailable,
            GuiSettingApplyRequirement::RestartPlayer,
            GuiSettingApplyRequirement::RestartApplication,
        ]
    );
}
