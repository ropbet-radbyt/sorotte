use super::*;

#[test]
fn gui_shell_app_state_applies_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: true,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(state.main_window.room_name, "DraftRoom");
    assert!(state.commands.can_reset_configuration);
    assert!(!state.commands.can_toggle_pause);
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
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_main_window_surface_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["Episode 1".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "bob".to_owned(),
                message: "synced".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: true,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
    assert_eq!(state.main_window.playlist[0].label, "Episode 1");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("synced")
    );
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(true);
    let mut saved = state.saved_configuration.clone();
    saved.chat_input_enabled = Some(false);

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

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
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_runtime_snapshots()
{
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
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
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_catches_up() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: false,
    }));
    assert!(state.runtime_menu_action_overrides.is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

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
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

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
    assert!(state.runtime_menu_action_overrides.is_empty());

    draft.chat_input_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved,
            }
        ))
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
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

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
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
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
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_flags_across_configuration_edits()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime_public_servers = state.public_servers.clone();
    runtime_public_servers.can_connect = false;
    runtime_public_servers.can_refresh = false;
    runtime_public_servers.can_add_custom_server = false;
    let mut runtime_media_search = state.media_search.clone();
    runtime_media_search.can_browse_directories = false;
    runtime_media_search.can_search_missing_media = false;

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    assert!(!state.public_servers.can_connect);
    assert!(!state.public_servers.can_refresh);
    assert!(!state.public_servers.can_add_custom_server);
    assert!(!state.media_search.can_browse_directories);
    assert!(!state.media_search.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_flags_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime_public_servers = state.public_servers.clone();
    runtime_public_servers.can_connect = false;
    runtime_public_servers.can_refresh = false;
    runtime_public_servers.can_add_custom_server = false;
    let mut runtime_media_search = state.media_search.clone();
    runtime_media_search.can_browse_directories = false;
    runtime_media_search.can_search_missing_media = false;

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert!(!state.public_servers.can_connect);
    assert!(!state.public_servers.can_refresh);
    assert!(!state.public_servers.can_add_custom_server);
    assert!(!state.media_search.can_browse_directories);
    assert!(!state.media_search.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_rows_across_configuration_edits()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let runtime_public_servers = PublicServerBrowserShellState {
        servers: vec![
            PublicServerBrowserRow {
                label: "Runtime Primary".to_owned(),
                address: "runtime.example:9000".to_owned(),
                is_selected: false,
            },
            PublicServerBrowserRow {
                label: "Runtime Backup".to_owned(),
                address: "backup.example:9001".to_owned(),
                is_selected: true,
            },
        ],
        can_connect: true,
        can_refresh: true,
        can_add_custom_server: true,
    };
    let runtime_media_search = MediaSearchWorkflowShellState {
        directories: vec![
            MediaSearchDirectoryRow {
                path: "D:/Runtime".to_owned(),
                is_selected: false,
            },
            MediaSearchDirectoryRow {
                path: "E:/Runtime".to_owned(),
                is_selected: true,
            },
        ],
        can_browse_directories: true,
        can_search_missing_media: true,
        first_file_timeout_seconds: state.media_search.first_file_timeout_seconds,
        search_timeout_seconds: state.media_search.search_timeout_seconds,
        double_check_interval_seconds: state.media_search.double_check_interval_seconds,
        warning_threshold_seconds: state.media_search.warning_threshold_seconds,
    };

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    assert_eq!(state.public_servers.servers[0].label, "Runtime Primary");
    assert_eq!(state.public_servers.servers[1].label, "Runtime Backup");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.media_search.directories[0].path, "D:/Runtime");
    assert_eq!(state.media_search.directories[1].path, "E:/Runtime");
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_rows_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let runtime_public_servers = PublicServerBrowserShellState {
        servers: vec![
            PublicServerBrowserRow {
                label: "Runtime Primary".to_owned(),
                address: "runtime.example:9000".to_owned(),
                is_selected: false,
            },
            PublicServerBrowserRow {
                label: "Runtime Backup".to_owned(),
                address: "backup.example:9001".to_owned(),
                is_selected: true,
            },
        ],
        can_connect: true,
        can_refresh: true,
        can_add_custom_server: true,
    };
    let runtime_media_search = MediaSearchWorkflowShellState {
        directories: vec![
            MediaSearchDirectoryRow {
                path: "D:/Runtime".to_owned(),
                is_selected: false,
            },
            MediaSearchDirectoryRow {
                path: "E:/Runtime".to_owned(),
                is_selected: true,
            },
        ],
        can_browse_directories: true,
        can_search_missing_media: true,
        first_file_timeout_seconds: state.media_search.first_file_timeout_seconds,
        search_timeout_seconds: state.media_search.search_timeout_seconds,
        double_check_interval_seconds: state.media_search.double_check_interval_seconds,
        warning_threshold_seconds: state.media_search.warning_threshold_seconds,
    };

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.public_servers.servers[0].label, "Runtime Primary");
    assert_eq!(state.public_servers.servers[1].label, "Runtime Backup");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.media_search.directories[0].path, "D:/Runtime");
    assert_eq!(state.media_search.directories[1].path, "E:/Runtime");
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
}

#[test]
fn gui_shell_app_state_updates_dialog_expectations_from_configuration_edits_without_runtime_overrides()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Privacy",
        label: "Trusted Domains Only",
        value: true,
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "System",
        label: "Auto Update",
        value: true,
    }));

    assert!(state.menus.tls_prompt_expected);
    assert!(!state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_preserves_runtime_dialog_expectations_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: StoredClientSettingsMvp {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
                saved_settings: StoredClientSettingsMvp {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI configuration runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
}
