use super::*;

#[test]
fn gui_shell_app_state_tracks_validation_issues_and_preserves_view_modal_across_resync() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::About)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "System",
        label: "Language",
        value: "zz".to_owned(),
    }));

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(state.open_modal, Some(GuiShellModal::About));
    assert_eq!(state.validation.issues.len(), 2);
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Connection" && issue.label == "Port")
    );
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "System" && issue.label == "Language")
    );

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Validation] status=2 issue(s), last_action_error=(none)"));
    assert!(rendered.contains("Connection / Port: must be a valid TCP port from 1 to 65535."));
    assert!(
        rendered.contains("System / Language: must be one of the supported legacy language tags.")
    );
    assert!(rendered.contains("active_view=public-servers"));
    assert!(rendered.contains("open_modal=about"));
}

#[test]
fn gui_shell_app_state_validates_trusted_domain_configuration_text() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Privacy",
        label: "Trusted Domains",
        value: "['trusted.example',".to_owned(),
    }));

    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Privacy" && issue.label == "Trusted Domains")
    );
    assert!(state.render_lines().join("\n").contains(
        "Privacy / Trusted Domains: must be a comma/semicolon-separated list or legacy bracketed list."
    ));
    assert_eq!(
        state.configuration.to_stored_settings().trusted_domains,
        None
    );
}

#[test]
fn gui_shell_app_state_tracks_action_errors_for_rejected_inputs() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::AddMediaSearchDirectory("   ".to_owned(),)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory cannot be empty.")
    );

    assert!(state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert_eq!(state.validation.last_action_error, None);

    assert!(!state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory is already present.")
    );
}

#[test]
fn gui_shell_app_state_edits_room_history_from_configuration_surface() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room_list: Some(vec!["beta".to_owned(), "alpha".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    let configuration_tree = state.configuration_widget_tree();
    let editor = configuration_tree
        .find("room-history:edit:entries")
        .expect("room-history text area should exist while editing");
    assert_eq!(editor.kind, GuiWidgetKind::TextArea);
    assert_eq!(editor.value.as_deref(), Some("beta\nalpha"));

    assert!(state.apply(GuiShellAction::UpdateRoomHistoryEdit(
        "zeta\n\nalpha\nbeta".to_owned()
    )));
    assert!(state.apply(GuiShellAction::CommitRoomHistoryEdit));

    assert_eq!(
        state.configuration.to_stored_settings().room_list,
        Some(vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "zeta".to_owned(),
        ])
    );
    assert_eq!(
        state
            .configuration
            .control_value("Connection", "Room History"),
        Some("alpha\nbeta\nzeta")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Connection", "Room History Count"),
        Some("3")
    );
    assert!(state.room_history_edit_session.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Room history updated: 3 entries.")
    );
}

#[test]
fn gui_shell_app_state_cancels_room_history_edit_without_changing_settings() {
    let original = vec!["beta".to_owned(), "alpha".to_owned()];
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room_list: Some(original.clone()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    assert!(state.apply(GuiShellAction::UpdateRoomHistoryEdit(
        "zeta\nalpha".to_owned()
    )));
    assert!(state.apply(GuiShellAction::CancelRoomHistoryEdit));

    assert_eq!(
        state.configuration.to_stored_settings().room_list,
        Some(original)
    );
    assert!(state.room_history_edit_session.is_none());
}

#[test]
fn gui_shell_app_state_applies_gui_interaction_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(true),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));

    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: Some(1),
                    selected_main_window_playlist: Some(1),
                    selected_menu_action: Some((1, 1)),
                    selected_media_search_directory: Some(0),
                },
                selected_public_server_index: Some(0),
                focused_configuration_control: Some(
                    GuiFocusedConfigurationControlRuntimeSnapshot {
                        section: "Connection".to_owned(),
                        label: "Host".to_owned(),
                        activation_count: 3,
                    }
                ),
                public_server_edit_session: Some(GuiPublicServerEditSessionRuntimeSnapshot {
                    editing_index: Some(0),
                    label_buffer: "Alpha Edited".to_owned(),
                    address_buffer: "alpha.example:9999".to_owned(),
                    is_dirty: true,
                }),
                main_window_user_edit_session: Some(GuiMainWindowUserEditSessionRuntimeSnapshot {
                    editing_index: 1,
                    username_buffer: "Bob Runtime".to_owned(),
                    is_dirty: true,
                }),
                text_edit_session: Some(GuiTextEditSessionRuntimeSnapshot {
                    section: "Connection".to_owned(),
                    label: "Host".to_owned(),
                    buffer: "runtime.example".to_owned(),
                    is_dirty: true,
                }),
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );

    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.main_window.playlist[1].is_selected);
    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(state.menus.sections[0].actions[1].is_selected);
    assert_eq!(state.selection.selected_media_search_directory, Some(0));
    assert!(state.media_search.directories[0].is_selected);
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.focused_configuration_control.as_ref().map(|focused| (
            focused.section,
            focused.label,
            focused.activation_count
        )),
        Some(("Connection", "Host", 3))
    );
    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(Some(0))
    );
    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(1)
    );
    assert_eq!(
        state
            .text_edit_session
            .as_ref()
            .map(|session| session.buffer.as_str()),
        Some("runtime.example")
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
}

#[test]
fn gui_shell_app_state_preserves_local_playlist_selection_across_stale_interaction_snapshots() {
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
    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.main_window_playlist_selection_is_local);

    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: Some(1),
                    selected_main_window_playlist: Some(0),
                    selected_menu_action: state.selection.selected_menu_action,
                    selected_media_search_directory: state.selection.selected_media_search_directory,
                },
                selected_public_server_index: None,
                focused_configuration_control: None,
                public_server_edit_session: None,
                main_window_user_edit_session: None,
                text_edit_session: None,
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );

    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.main_window_playlist_selection_is_local);
    assert!(state.main_window.playlist[1].is_selected);
    assert!(!state.main_window.playlist[0].is_selected);
}

#[test]
fn gui_shell_app_state_normalizes_disabled_menu_selection_in_gui_interaction_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Seek",
                enabled: false,
            }],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        },
    )));

    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: state.selection.selected_main_window_user,
                    selected_main_window_playlist: state.selection.selected_main_window_playlist,
                    selected_menu_action: Some((1, 1)),
                    selected_media_search_directory: state
                        .selection
                        .selected_media_search_directory,
                },
                selected_public_server_index: state.selected_public_server_index(),
                focused_configuration_control: None,
                public_server_edit_session: None,
                main_window_user_edit_session: None,
                text_edit_session: None,
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(
        state.menus.sections[0]
            .actions
            .iter()
            .find(|action| action.label == "Open Media Search")
            .is_some_and(|action| action.enabled && action.is_selected)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_interaction_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: Some(0),
                    selected_main_window_playlist: None,
                    selected_menu_action: None,
                    selected_media_search_directory: None,
                },
                selected_public_server_index: None,
                focused_configuration_control: Some(
                    GuiFocusedConfigurationControlRuntimeSnapshot {
                        section: "Connection".to_owned(),
                        label: "Missing".to_owned(),
                        activation_count: 0,
                    }
                ),
                public_server_edit_session: None,
                main_window_user_edit_session: None,
                text_edit_session: None,
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI interaction runtime snapshots cannot focus an unknown configuration control.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("  runtime draft  ".to_owned()),
        }
    )));
    assert_eq!(
        state.outgoing_chat_message.as_deref(),
        Some("runtime draft")
    );

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("updated runtime draft".to_owned()),
        }
    )));
    assert_eq!(
        state.outgoing_chat_message.as_deref(),
        Some("updated runtime draft")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SaveConfiguration,
    )));
    assert!(!state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("runtime draft".to_owned()),
        }
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI draft runtime snapshots cannot stage an outgoing chat message while a different pending operation is active."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
    assert_eq!(state.outgoing_chat_message, None);
}

#[test]
fn gui_shell_app_state_applies_gui_configuration_draft_runtime_snapshots() {
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));

    let replacement = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        public_servers: Some(vec![(
            "Primary".to_owned(),
            "syncplay.example:8999".to_owned(),
        )]),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(
            GuiConfigurationDraftRuntimeSnapshot {
                settings: replacement.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(state.main_window.room_name, "DraftRoom");
    assert_eq!(
        state
            .public_servers
            .servers
            .first()
            .map(|row| row.address.as_str()),
        Some("syncplay.example:8999")
    );
    assert!(state.commands.can_reset_configuration);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(
            GuiConfigurationDraftRuntimeSnapshot {
                settings: StoredClientSettingsMvp {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI configuration draft runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
}

#[test]
fn gui_shell_app_state_applies_gui_saved_configuration_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.commands.can_reset_configuration);

    let replacement = StoredClientSettingsMvp {
        host: Some("dirty.example".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            GuiSavedConfigurationRuntimeSnapshot {
                settings: replacement.clone(),
            }
        ))
    );

    assert_eq!(state.saved_configuration, replacement);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("dirty.example")
    );
    assert!(!state.commands.can_reset_configuration);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_saved_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            GuiSavedConfigurationRuntimeSnapshot {
                settings: StoredClientSettingsMvp {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI saved-configuration runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
}
