use super::*;

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
