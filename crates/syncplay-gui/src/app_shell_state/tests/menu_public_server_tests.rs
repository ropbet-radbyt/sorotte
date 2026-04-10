use super::*;

#[test]
fn gui_shell_app_state_triggers_selected_menu_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();
    state.sync_playback_menu_actions_from_runtime_state(state.commands.can_toggle_pause);

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 0,
        action_index: 2,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(state.active_view, GuiShellView::PublicServers);

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 2,
        action_index: 4,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 1,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::TogglePlaybackPause)
    );
    assert!(state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert!(state.main_window.playback_paused);

    let mut disabled_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(disabled_state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 1,
    }));
    assert!(!disabled_state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(
        disabled_state.validation.last_action_error.as_deref(),
        Some("The selected menu action is currently disabled.")
    );
}

#[test]
fn gui_shell_app_state_selects_public_server_and_updates_config_host_port() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("Backup".to_owned(), "syncplay.example:8995".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);

    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
}

#[test]
fn gui_shell_app_state_handles_public_server_browser_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AnnouncePublicServerSelectionChanged(0)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Public server selected: Primary.")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ConnectPublicServer)
    );
    assert!(state.apply(GuiShellAction::CompleteSelectedPublicServerConnect));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.active_view, GuiShellView::Configuration);

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(
        state.apply(GuiShellAction::CompletePublicServerRefresh(vec![
            ("Refreshed".to_owned(), "syncplay.example:8995".to_owned()),
            ("Backup".to_owned(), "backup.example:8998".to_owned()),
        ]))
    );
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.public_servers.servers.len(), 2);
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceCustomPublicServerAdded {
            label: "Custom".to_owned(),
            address: "custom.example:9000".to_owned(),
        })
    );
    assert_eq!(state.public_servers.servers.len(), 3);
    assert!(state.public_servers.servers[2].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_public_server_browser_event_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Public server connect is unavailable when browser connect actions are disabled.")
    );

    assert!(!state.apply(GuiShellAction::CompletePublicServerRefresh(vec![])));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public server refresh is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert!(!state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );

    assert!(
        !state.apply(GuiShellAction::AnnounceCustomPublicServerAdded {
            label: "Broken".to_owned(),
            address: ":8999".to_owned(),
        })
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Custom public-server address is not valid.")
    );
}

#[test]
fn gui_shell_app_state_adds_edits_and_removes_public_server_rows() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Primary".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "syncplay.pl:8999".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "Primary");
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().public_servers,
        Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
    );
    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.pl"));
    assert_eq!(saved.port, Some(8999));

    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Primary EU".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "syncplay.example:8995".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(state.public_servers.servers[0].label, "Primary EU");
    assert_eq!(
        state.public_servers.servers[0].address,
        "syncplay.example:8995"
    );
    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Secondary".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "backup.example:8998".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CancelPublicServerEdit));
    assert!(state.public_server_edit_session.is_none());
    assert_eq!(state.public_servers.servers.len(), 1);

    assert!(state.apply(GuiShellAction::RemoveSelectedPublicServer));
    assert!(state.public_servers.servers.is_empty());
    assert_eq!(
        state.configuration.to_stored_settings().public_servers,
        None
    );
}

#[test]
fn gui_shell_app_state_remaps_public_server_edit_sessions_by_row_identity_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Beta Edited".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));

    let mut draft = state.configuration.to_stored_settings();
    draft.public_servers = Some(vec![
        ("Inserted".to_owned(), "inserted.example:8999".to_owned()),
        ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
        ("Beta".to_owned(), "beta.example:8999".to_owned()),
    ]);
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(Some(2))
    );
    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.label_buffer.as_str()),
        Some("Beta Edited")
    );
    assert_eq!(state.selected_public_server_index(), Some(2));
    assert!(state.public_servers.servers[2].is_selected);
}

#[test]
fn gui_shell_app_state_clears_public_server_edit_sessions_when_the_edited_row_disappears() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));

    let mut draft = state.configuration.to_stored_settings();
    draft.public_servers = Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]);
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert!(state.public_server_edit_session.is_none());
}

#[test]
fn gui_shell_app_state_keeps_public_server_selection_on_the_active_edit_row() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));

    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(state.public_servers.servers[1].is_selected);
    assert!(
        state
            .public_server_edit_session
            .as_ref()
            .is_some_and(|session| session.editing_index == Some(1))
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_public_server_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public server is currently selected.")
    );

    assert!(!state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public-server edit session is currently active.")
    );

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Broken".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        ":8999".to_owned(),
    )));
    assert!(!state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Public-server address is not valid.")
    );
}

#[test]
fn gui_shell_app_state_tracks_transient_notification_queue() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    for (level, message) in [
        (GuiTransientNotificationLevel::Info, "one"),
        (GuiTransientNotificationLevel::Success, "two"),
        (GuiTransientNotificationLevel::Warning, "three"),
        (GuiTransientNotificationLevel::Error, "four"),
        (GuiTransientNotificationLevel::Info, "five"),
        (GuiTransientNotificationLevel::Success, "six"),
    ] {
        assert!(state.apply(GuiShellAction::PushTransientNotification {
            level,
            message: message.to_owned(),
        }));
    }

    assert_eq!(state.notifications.len(), 5);
    assert_eq!(state.notifications[0].message, "two");
    assert_eq!(state.notifications[4].message, "six");

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Notifications] count=5"));
    assert!(rendered.contains("- success: two"));
    assert!(rendered.contains("- success: six"));

    assert!(state.apply(GuiShellAction::DismissTransientNotification(1)));
    assert_eq!(state.notifications.len(), 4);
    assert!(state.apply(GuiShellAction::ClearTransientNotifications));
    assert!(state.notifications.is_empty());
    assert!(!state.apply(GuiShellAction::ClearTransientNotifications));
}

#[test]
fn gui_shell_app_state_rejects_invalid_transient_notification_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "   ".to_owned(),
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Transient notification messages must be non-empty.")
    );

    assert!(!state.apply(GuiShellAction::DismissTransientNotification(0)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No transient notification exists at the requested index.")
    );
}
