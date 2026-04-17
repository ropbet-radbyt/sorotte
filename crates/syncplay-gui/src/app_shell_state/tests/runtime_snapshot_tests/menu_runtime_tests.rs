use super::*;

#[test]
fn gui_shell_app_state_switches_views_and_tracks_modal_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MenusAndDialogs)));
    assert_eq!(state.active_view, GuiShellView::MenusAndDialogs);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MediaSearch)));
    assert_eq!(state.active_view, GuiShellView::MediaSearch);

    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::About)));
    assert_eq!(state.open_modal, Some(GuiShellModal::About));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::UpdateNotice)));
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));
    assert!(state.apply(GuiShellAction::OpenModal(
        GuiShellModal::TlsCertificatePrompt
    )));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::CloseModal));
    assert_eq!(state.open_modal, None);
    assert!(!state.apply(GuiShellAction::CloseModal));
}

#[test]
fn gui_shell_app_state_announces_menu_and_dialog_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));
    assert!(state.main_window.chat.is_empty());

    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));
    assert!(state.menus.update_notice_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::AnnounceAboutDialogRequested));
    assert_eq!(state.active_view, GuiShellView::MenusAndDialogs);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::AnnounceHelpRequested));
    assert_eq!(state.active_view, GuiShellView::MenusAndDialogs);
    assert!(state.notifications.is_empty());
}

#[test]
fn gui_shell_app_state_dismisses_update_notice_and_completes_tls_prompt() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));
    assert!(state.apply(GuiShellAction::DismissUpdateNotice));
    assert!(!state.menus.update_notice_expected);
    assert_eq!(state.open_modal, None);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.apply(GuiShellAction::CloseModal));
    assert!(state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, None);

    assert!(state.apply(GuiShellAction::TrustTlsCertificatePrompt));
    assert!(!state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, None);
    assert!(state.main_window.chat.is_empty());
}

#[test]
fn gui_shell_app_state_applies_user_initiated_update_check_results() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyUpdateCheckResult(
        LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpdateAvailable,
            message: "A new version of Syncplay is available.".to_owned(),
            url: Some("https://syncplay.pl/download/".to_owned()),
            public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
            checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
            user_initiated: true,
        }
    )));

    assert!(state.menus.update_notice_expected);
    assert_eq!(state.open_modal, None);
    assert_eq!(
        state.update_check.message.as_deref(),
        Some("A new version of Syncplay is available.")
    );
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .last_checked_for_updates
            .as_deref(),
        Some("2026-03-08 09:10:11.123")
    );
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(
        state
            .public_servers
            .servers
            .first()
            .map(|row| (row.label.as_str(), row.address.as_str())),
        Some(("Primary", "syncplay.pl:8999"))
    );
    assert!(state.notifications.is_empty());
}

#[test]
fn gui_shell_app_state_applies_automatic_update_check_results_without_modal_when_up_to_date() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyUpdateCheckResult(
        LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: "Syncplay is up to date".to_owned(),
            url: None,
            public_servers: None,
            checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
            user_initiated: false,
        }
    )));

    assert!(!state.menus.update_notice_expected);
    assert_eq!(state.open_modal, None);
    assert_eq!(
        state.update_check.message.as_deref(),
        Some("Syncplay is up to date")
    );
    assert!(state.notifications.is_empty());
}

#[test]
fn gui_shell_app_state_auto_opens_new_runtime_prompt_flags() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: false,
            update_notice_expected: true,
            about_dialog_available: true,
        },
    )));
    assert_eq!(state.open_modal, None);

    assert!(state.apply(GuiShellAction::DismissUpdateNotice));
    assert_eq!(state.open_modal, None);

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));
}

#[test]
fn gui_shell_app_state_applies_menu_dialog_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![
                MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label: "Toggle Pause",
                    enabled: false,
                },
                MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Chat",
                    enabled: true,
                },
                MenuActionRuntimeOverride {
                    section_title: "Help",
                    action_label: "Check for Updates",
                    enabled: false,
                },
            ],
            tls_prompt_expected: true,
            update_notice_expected: false,
            about_dialog_available: false,
        },
    )));

    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(!state.menus.update_notice_expected);
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
            .find(|action| action.label == "About")
            .is_some_and(|action| !action.enabled)
    );
    assert!(state.notifications.is_empty());
}

#[test]
fn gui_shell_app_state_rejects_invalid_menu_dialog_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Invalid",
                action_label: "Missing",
                enabled: true,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No menu action exists for 'Invalid / Missing' in the runtime snapshot.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_feedback_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
        GuiFeedbackRuntimeSnapshot {
            validation_issues: vec![GuiValidationIssue {
                scope: "Runtime".to_owned(),
                label: "Sync".to_owned(),
                message: "Server health degraded.".to_owned(),
            }],
            notifications: vec![
                GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: "Server warning broadcast.".to_owned(),
                },
                GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Server status feed refreshed.".to_owned(),
                },
            ],
        },
    )));

    assert_eq!(state.validation.last_action_error, None);
    assert_eq!(state.validation.issues.len(), 2);
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Connection" && issue.label == "Port")
    );
    assert!(state.validation.issues.iter().any(|issue| {
        issue.scope == "Runtime"
            && issue.label == "Sync"
            && issue.message == "Server health degraded."
    }));
    assert_eq!(state.notifications.len(), 2);
    assert_eq!(
        state.notifications[0].message.as_str(),
        "Server warning broadcast."
    );
    assert_eq!(
        state.notifications[1].message.as_str(),
        "Server status feed refreshed."
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Runtime" && issue.label == "Sync")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_feedback_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
            GuiFeedbackRuntimeSnapshot {
                validation_issues: vec![GuiValidationIssue {
                    scope: "   ".to_owned(),
                    label: "Sync".to_owned(),
                    message: "Degraded.".to_owned(),
                }],
                notifications: Vec::new(),
            },
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI feedback runtime snapshots cannot contain empty validation scopes.")
    );

    assert!(
        !state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
            GuiFeedbackRuntimeSnapshot {
                validation_issues: Vec::new(),
                notifications: vec![GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: "   ".to_owned(),
                }],
            },
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI feedback runtime snapshots cannot contain empty notification messages.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_error_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: Some("  runtime error  ".to_owned()),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("runtime error")
    );

    assert!(state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: None,
        },
    )));
    assert_eq!(state.validation.last_action_error, None);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_error_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: Some("   ".to_owned()),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI error runtime snapshots cannot contain an empty action error message.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert_eq!(
        state.commands,
        GuiCommandAvailabilityState {
            can_save_configuration: false,
            can_reset_configuration: false,
            can_reload_configuration: false,
            can_connect_public_server: false,
            can_connect_saved_server: false,
            can_refresh_public_servers: false,
            can_disconnect_session: false,
            can_search_missing_media: false,
            can_toggle_pause: false,
            can_send_chat_message: false,
        }
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_keeps_unrelated_command_flags_live_when_runtime_overrides_chat_send() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert!(!state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "0".to_owned(),
    }));

    assert!(!state.commands.can_send_chat_message);
    assert!(!state.commands.can_save_configuration);
    assert!(state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_chat_command_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        Some(false)
    );

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(false);
    let saved = state.saved_configuration.clone();
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        None
    );

    draft.chat_input_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::SaveConfiguration),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active."
        )
    );
}

#[test]
fn gui_shell_app_state_syncs_playback_menu_actions_from_gui_command_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["One".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media Search")
            .is_some_and(|action| action.enabled && action.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Shared Playlist")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
}

#[test]
fn gui_pending_operation_kind_labels_are_stable() {
    let labels = [
        GuiPendingOperationKind::SaveConfiguration.label(),
        GuiPendingOperationKind::ResetConfiguration.label(),
        GuiPendingOperationKind::ReloadConfiguration.label(),
        GuiPendingOperationKind::ConnectPublicServer.label(),
        GuiPendingOperationKind::RefreshPublicServers.label(),
        GuiPendingOperationKind::SearchMissingMedia.label(),
        GuiPendingOperationKind::TogglePlaybackPause.label(),
        GuiPendingOperationKind::SendChatMessage.label(),
    ];

    assert_eq!(
        labels,
        [
            "save-configuration",
            "reset-configuration",
            "reload-configuration",
            "connect-public-server",
            "refresh-public-servers",
            "search-missing-media",
            "toggle-playback-pause",
            "send-chat-message",
        ]
    );
}
