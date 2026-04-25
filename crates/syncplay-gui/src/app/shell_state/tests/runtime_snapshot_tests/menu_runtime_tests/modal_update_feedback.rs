use super::*;

#[test]
fn gui_shell_app_state_switches_views_and_tracks_modal_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Room)));
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
    assert_eq!(state.active_view, GuiShellView::Setup);

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
    assert_chat_pane_ready(&state.main_window.chat);

    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));
    assert!(state.menus.update_notice_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::AnnounceAboutDialogRequested));
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::AnnounceHelpRequested));
    assert_eq!(state.active_view, GuiShellView::Setup);
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
    assert_chat_pane_ready(&state.main_window.chat);
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

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Room)));
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
