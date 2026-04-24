use super::{
    GuiAppHost, GuiPersistedUiState, GuiShellAction, GuiTextPreviewHost, SyncplayGuiShellAppState,
    run_gui_host, shell_widget_preview, startup_notice, startup_preview,
};

use crate::app::GuiShellView;
use crate::app::testing::support::{
    TEST_USERNAME, test_default_syncplay_config_env_root, test_default_syncplay_config_target,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn startup_notice_mentions_configuration_surface_and_grouped_sections() {
    let notice = startup_notice(&StoredClientSettingsMvp::default());

    assert!(notice.contains("[Shell App State]"));
    assert!(notice.contains("active_view=setup"));
    assert!(notice.contains("open_modal=(none)"));
    assert!(
        notice.contains("[Selection] user=0, playlist=(none), menu=0:0, media_directory=(none)")
    );
    assert!(notice.contains(
        "[Commands] busy=no, save_configuration=yes, reset_configuration=no, reload_configuration=yes, connect_saved_server=no, disconnect_session=no, connect_public_server=no, refresh_public_servers=yes, search_missing_media=no, toggle_pause=no, send_chat_message=no"
    ));
    assert!(notice.contains("[Pending] operation=(none)"));
    assert!(notice.contains("[Control Focus] focused=(none)"));
    assert!(notice.contains("[Public Server Edit] editing=(none)"));
    assert!(notice.contains("[Text Edit] editing=(none)"));
    assert!(notice.contains("[Notifications] count=0"));
    assert!(notice.contains("[Validation] status=clean, last_action_error=(none)"));
    assert!(notice.contains("setup surface initialized"));
    assert!(notice.contains("[Connection]"));
    assert!(notice.contains("[Readiness]"));
    assert!(notice.contains("[Privacy]"));
    assert!(notice.contains("[Media Search]"));
    assert!(notice.contains("[System]"));
    assert!(notice.contains("[Room]"));
    assert!(notice.contains("[Menus & Dialogs]"));
    assert!(notice.contains("[Public Server Browser]"));
    assert!(notice.contains("[Media Search Workflow]"));
    assert!(notice.contains("Playback Controls:"));
    assert!(notice.contains("Dialog Prompts:"));
    assert!(notice.contains("Servers (0):"));
    assert!(notice.contains("Directories (0):"));
    assert!(notice.contains("unified shell app state and action reducer"));
    assert!(notice.contains("Users (1):"));
    assert!(notice.contains("- Host [text]:"));
    assert!(notice.contains("- Server Password [password]:"));
    assert!(notice.contains("room-first shell"));
    assert!(!notice.contains("bootstrap placeholder"));
    assert!(notice.contains("de/en/es"));
}

#[test]
fn shell_widget_preview_renders_tree_through_text_preview_renderer() {
    let preview = shell_widget_preview(&StoredClientSettingsMvp::default());

    assert!(!preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
    assert!(preview.contains(
        "  - Setup [layout] id=configuration-root, enabled=yes, selected=yes, value=(none)"
    ));
    assert!(preview.contains(
        "    - Host [text-input] id=config:Connection:Host, enabled=yes, selected=no, value=(unset)"
    ));
    assert!(
        preview.contains(
            "  - Room [layout] id=main-window-root, enabled=yes, selected=no, value=(none)"
        )
    );
}

#[test]
fn startup_preview_includes_shell_summary_and_widget_tree_preview() {
    let preview = startup_preview(&StoredClientSettingsMvp::default());

    assert!(preview.contains("[Shell App State]"));
    assert!(preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_startup_remote_actions_run_due_automatic_update_checks() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: None,
        ..StoredClientSettingsMvp::default()
    };
    let expected = super::remote_services::LegacyUpdateCheckResult {
        status: super::remote_services::LegacyUpdateCheckStatus::UpdateAvailable,
        message: "Remote startup update available.".to_owned(),
        url: Some("https://syncplay.pl/download/".to_owned()),
        public_servers: None,
        checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
        user_initiated: false,
    };

    let actions = super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_| expected.clone(),
        |_| Ok(Vec::new()),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyUpdateCheckResult(expected)]
    );
}

#[test]
fn gui_startup_remote_actions_seed_public_servers_when_cache_is_empty() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: Some("2027-01-14 09:10:11.123".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_| panic!("update check should not run when the timestamp is still fresh"),
        |_| Ok(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyStartupPublicServerCache(vec![(
            "Primary".to_owned(),
            "syncplay.pl:8999".to_owned(),
        )])]
    );
}

#[test]
fn gui_startup_actions_from_lookup_prefers_file_public_server_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => {
                Some(r#"[["Ignored", "inline.example:8999"]]"#.to_owned())
            }
            _ => None,
        },
        &settings,
        None,
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(
            "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
                .to_owned(),
        )]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_bootstrap() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_loopback_bootstrap() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat loopback via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_defaults() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(
            "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for 127.0.0.1:8999 as gui-user in room gui-demo. Defaults: host=127.0.0.1, port=8999, user=gui-user, room=gui-demo."
                .to_owned(),
        )]
    );
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_public_server_refresh_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(
        "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
    ));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_tcp_bootstrap_and_public_server_sources() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    let preview = super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains(
        "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
    ));
    assert!(!preview.contains("Startup summary:"));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn gui_startup_actions_from_lookup_reports_config_path_source() {
    let default_target = test_default_syncplay_config_target();
    let expected_message =
        super::GuiStartupConfigPathSource::DefaultConfigTarget(default_target.clone())
            .startup_message();
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::GuiStartupConfigPathSource::DefaultConfigTarget(
            default_target,
        )),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_player_ipc_source_with_client_precedence() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_CLIENT_MPV_IPC_PATH" => Some(r#"\\.\pipe\syncplay-mpv"#.to_owned()),
            "SYNCPLAY_MPV_IPC_PATH" => Some("/tmp/ignored-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(
            r"Startup will try mpv JSON IPC from SYNCPLAY_CLIENT_MPV_IPC_PATH (\\.\pipe\syncplay-mpv)."
                .to_owned(),
        )]
    );
}

#[test]
fn gui_startup_actions_from_lookup_and_config_path_source_keeps_multi_notice_details_in_system_chat()
 {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp {
            public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
            ..StoredClientSettingsMvp::default()
        },
        None,
    );

    let notification_messages = actions
        .iter()
        .filter_map(|action| match action {
            GuiShellAction::PushTransientNotification { message, .. } => Some(message.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(notification_messages.is_empty());
}

#[test]
fn gui_startup_actions_from_lookup_reports_missing_player_ipc_source() {
    let actions =
        super::gui_startup_actions_from_lookup(|_name| None, &StoredClientSettingsMvp::default());

    assert!(actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Startup has no explicit mpv JSON IPC path. The GUI will use the saved playerPath when it points to mpv; otherwise set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
        )
    }));
}

#[test]
fn resolve_syncplay_gui_config_path_source_legacy_compatible_with_reports_default_target() {
    let env_root = test_default_syncplay_config_env_root();
    let env_root_string = env_root.display().to_string();
    let source = super::resolve_syncplay_gui_config_path_source_legacy_compatible_with(
        &|name| match name {
            "APPDATA" if cfg!(windows) => Some(env_root_string.clone()),
            "HOME" if !cfg!(windows) => Some(env_root_string.clone()),
            _ => None,
        },
        || None,
        |_path| false,
    );

    assert_eq!(
        source,
        Some(super::GuiStartupConfigPathSource::DefaultConfigTarget(
            test_default_syncplay_config_target(),
        ))
    );
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_config_path_source() {
    let override_path = std::path::PathBuf::from("custom-config-root").join("syncplay.ini");
    let expected_message =
        super::GuiStartupConfigPathSource::Override(override_path.clone()).startup_message();
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::GuiStartupConfigPathSource::Override(override_path)),
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_player_ipc_source() {
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_MPV_IPC_PATH" => Some("/tmp/syncplay-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(
        "Startup will try mpv JSON IPC from SYNCPLAY_MPV_IPC_PATH (/tmp/syncplay-mpv.sock)."
    ));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn run_gui_host_passes_shell_state_through_host_boundary() {
    #[derive(Default)]
    struct RecordingHost {
        saw_configuration_view: bool,
    }

    impl GuiAppHost for RecordingHost {
        type Output = String;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            self.saw_configuration_view = state.active_view == GuiShellView::Setup;
            format!("host:{}", state.active_view.label())
        }
    }

    let mut host = RecordingHost::default();
    let rendered = run_gui_host(&StoredClientSettingsMvp::default(), &mut host);

    assert_eq!(rendered, "host:setup");
    assert!(host.saw_configuration_view);
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_restores_non_ini_state() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = GuiPersistedUiState {
        active_view: Some(GuiShellView::Setup),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: Some("C:/Media".to_owned()),
        last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.last_media_dialog_directory.as_deref(),
        Some("D:/Dialogs")
    );
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        persisted_ui_state.public_servers
    );
    assert_eq!(state.selected_public_server_index(), Some(0));
    assert_eq!(state.selection.selected_media_search_directory, Some(0));
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_prefers_gui_public_servers_over_ini_rows() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        port: Some(8999),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = GuiPersistedUiState {
        active_view: Some(GuiShellView::Setup),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: None,
        last_media_dialog_directory: None,
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        vec![("Custom".to_owned(), "custom.example:9001".to_owned())]
    );
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}

#[test]
fn gui_client_core_chat_tcp_bootstrap_overrides_settings_enable_chat_and_seed_connection() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("2001:db8::1".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("9000".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("gui-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("gui-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| Ok(None),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("2001:db8::1"));
    assert_eq!(settings.port, Some(9000));
    assert_eq!(settings.username.as_deref(), Some("gui-user"));
    assert_eq!(settings.room.as_deref(), Some("gui-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
}

#[test]
fn gui_client_core_chat_loopback_bootstrap_overlays_settings_enable_chat() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("gui-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("gui-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| Ok(None),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host, None);
    assert_eq!(settings.port, None);
    assert_eq!(settings.username.as_deref(), Some("gui-user"));
    assert_eq!(settings.room.as_deref(), Some("gui-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
}

#[test]
fn gui_startup_settings_from_lookup_seeds_public_servers_without_tcp_bootstrap() {
    let settings = super::gui_startup_settings_from_lookup(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => Some(
                r#"[[" Primary ", " syncplay.pl:8999 "], ["Duplicate", "SYNCPLAY.PL:8999"]]"#
                    .to_owned(),
            ),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(
        settings.public_servers,
        Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
    );
    assert_eq!(settings.host, None);
    assert_eq!(settings.username, None);
}

#[test]
fn gui_startup_settings_from_lookup_merges_tcp_bootstrap_and_file_public_servers() {
    let settings = super::gui_startup_settings_from_lookup(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        |path| {
            if path == "public-servers.txt" {
                Ok(r#"[[" File Primary ", " file.example:8999 "]]"#.to_owned())
            } else {
                Err("unexpected file read".to_owned())
            }
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("syncplay.example"));
    assert_eq!(settings.port, Some(8995));
    assert_eq!(settings.username.as_deref(), Some(TEST_USERNAME));
    assert_eq!(settings.room.as_deref(), Some("room-a"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
    assert_eq!(
        settings.public_servers,
        Some(vec![(
            "File Primary".to_owned(),
            "file.example:8999".to_owned()
        )])
    );
}

#[test]
fn gui_startup_settings_from_lookup_loads_stored_config_before_rendering() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_CLIENT_CONFIG_PATH" => Some("stored-syncplay.ini".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |path| {
            assert_eq!(path, std::path::Path::new("stored-syncplay.ini"));
            Ok(Some(StoredClientSettingsMvp {
                host: Some("persisted.example".to_owned()),
                port: Some(8999),
                username: Some("persisted-user".to_owned()),
                room: Some("persisted-room".to_owned()),
                player_path: Some("C:/Players/mpv.exe".to_owned()),
                ..StoredClientSettingsMvp::default()
            }))
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("persisted.example"));
    assert_eq!(settings.port, Some(8999));
    assert_eq!(settings.username.as_deref(), Some("persisted-user"));
    assert_eq!(settings.room.as_deref(), Some("persisted-room"));
    assert_eq!(settings.player_path.as_deref(), Some("C:/Players/mpv.exe"));

    let state = SyncplayGuiShellAppState::from_stored_settings(&settings);
    assert_eq!(
        state.configuration.settings.host.as_deref(),
        Some("persisted.example")
    );
    assert_eq!(state.configuration.settings.port, Some(8999));
    assert_eq!(
        state.configuration.settings.username.as_deref(),
        Some("persisted-user")
    );
    assert_eq!(
        state.configuration.settings.room.as_deref(),
        Some("persisted-room")
    );
    assert_eq!(
        state.configuration.settings.player_path.as_deref(),
        Some("C:/Players/mpv.exe")
    );
}

#[test]
fn gui_startup_settings_from_lookup_overlays_bootstrap_on_loaded_config() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_CLIENT_CONFIG_PATH" => Some("stored-syncplay.ini".to_owned()),
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("runtime.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("runtime-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("runtime-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| {
            Ok(Some(StoredClientSettingsMvp {
                host: Some("persisted.example".to_owned()),
                port: Some(7777),
                username: Some("persisted-user".to_owned()),
                room: Some("persisted-room".to_owned()),
                player_path: Some("C:/Players/mpv.exe".to_owned()),
                ready_at_start: Some(true),
                ..StoredClientSettingsMvp::default()
            }))
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("runtime.example"));
    assert_eq!(settings.port, Some(8995));
    assert_eq!(settings.username.as_deref(), Some("runtime-user"));
    assert_eq!(settings.room.as_deref(), Some("runtime-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
    assert_eq!(settings.player_path.as_deref(), Some("C:/Players/mpv.exe"));
    assert_eq!(settings.ready_at_start, Some(true));
}
