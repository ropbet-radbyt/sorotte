use super::*;

#[test]
fn gui_startup_actions_from_lookup_prefers_file_public_server_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS" => {
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
            "Startup loaded 1 public server from SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
                .to_owned(),
        )]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_bootstrap() {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SOROTTE_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SOROTTE_CLIENT_PORT" => Some("8995".to_owned()),
            "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_loopback_bootstrap() {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
            "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat loopback via SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_defaults() {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(
            "Startup enabled client-core chat TCP via SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for 127.0.0.1:8999 as gui-user in room gui-demo. Defaults: host=127.0.0.1, port=8999, user=gui-user, room=gui-demo."
                .to_owned(),
        )]
    );
}

#[test]
fn gui_startup_actions_from_lookup_keeps_remote_work_out_of_pre_window_actions() {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp {
            check_for_updates_automatically: Some(true),
            last_checked_for_updates: None,
            public_servers: None,
            ..StoredClientSettingsMvp::default()
        },
        None,
    );

    assert!(actions.iter().all(|action| {
        !matches!(
            action,
            GuiShellAction::ApplyUpdateCheckResult(_)
                | GuiShellAction::ApplyStartupPublicServerCache(_)
        )
    }));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_public_server_refresh_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview =
        super::super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(
        "Startup loaded 1 public server from SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
    ));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_tcp_bootstrap_and_public_server_sources() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SOROTTE_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SOROTTE_CLIENT_PORT" => Some("8995".to_owned()),
            "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    let preview =
        super::super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains(
        "Startup loaded 1 public server from SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
    ));
    assert!(!preview.contains("Startup summary:"));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn gui_startup_actions_from_lookup_reports_config_path_source() {
    let default_target = test_default_sorotte_config_target();
    let expected_message =
        super::super::GuiStartupConfigPathSource::DefaultConfigTarget(default_target.clone())
            .startup_message();
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::super::GuiStartupConfigPathSource::DefaultConfigTarget(default_target)),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(expected_message)]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_player_ipc_source_with_client_precedence() {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_CLIENT_MPV_IPC_PATH" => Some(r#"\\.\pipe\syncplay-mpv"#.to_owned()),
            "SOROTTE_MPV_IPC_PATH" => Some("/tmp/ignored-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::AnnounceSystemChatEvent(
            r"Startup will try mpv JSON IPC from SOROTTE_CLIENT_MPV_IPC_PATH (\\.\pipe\syncplay-mpv)."
                .to_owned(),
        )]
    );
}

#[test]
fn gui_startup_actions_from_lookup_and_config_path_source_keeps_multi_notice_details_in_system_chat()
 {
    let actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SOROTTE_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SOROTTE_CLIENT_PORT" => Some("8995".to_owned()),
            "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
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
    let actions = super::super::gui_startup_actions_from_lookup(
        |_name| None,
        &StoredClientSettingsMvp::default(),
    );

    assert!(actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Startup has no explicit mpv JSON IPC path. The GUI will use the saved playerPath when it points to mpv; otherwise set SOROTTE_CLIENT_MPV_IPC_PATH or SOROTTE_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
        )
    }));
}

#[test]
fn resolve_sorotte_gui_config_path_source_legacy_compatible_with_reports_default_target() {
    let env_root = test_default_sorotte_config_env_root();
    let env_root_string = env_root.display().to_string();
    let source = super::super::resolve_sorotte_gui_config_path_source_legacy_compatible_with(
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
        Some(
            super::super::GuiStartupConfigPathSource::DefaultConfigTarget(
                test_default_sorotte_config_target(),
            )
        )
    );
}

#[test]
fn resolve_sorotte_gui_config_path_source_legacy_compatible_with_reports_env_root() {
    let env_root = test_default_sorotte_config_env_root();
    let env_root_string = env_root.display().to_string();
    let source = super::super::resolve_sorotte_gui_config_path_source_legacy_compatible_with(
        &|name| match name {
            "APPDATA" if cfg!(windows) => Some(env_root_string.clone()),
            "HOME" if !cfg!(windows) => Some(env_root_string.clone()),
            "SOROTTE_CLIENT_CONFIG_ROOT" => Some("portable-config".to_owned()),
            _ => None,
        },
        || Some(std::path::PathBuf::from("/cwd")),
        |_path| false,
    );

    assert_eq!(
        source,
        Some(
            super::super::GuiStartupConfigPathSource::ConfigRootOverride(
                std::path::PathBuf::from("/cwd")
                    .join("portable-config")
                    .join("sorotte.ini"),
            )
        )
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_config_storage_snapshot() {
    let env_root = test_default_sorotte_config_env_root();
    let env_root_string = env_root.display().to_string();
    let actions = super::super::gui_startup_actions_from_lookup(
        |name| match name {
            "APPDATA" if cfg!(windows) => Some(env_root_string.clone()),
            "HOME" if !cfg!(windows) => Some(env_root_string.clone()),
            "SOROTTE_CLIENT_CONFIG_ROOT" => Some(env_root.join("portable").display().to_string()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
    );

    assert!(actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(snapshot)
                if snapshot.source_label == "SOROTTE_CLIENT_CONFIG_ROOT"
                    && snapshot.external_override_active
                    && snapshot
                        .config_path
                        .as_deref()
                        .is_some_and(|path| path.ends_with("sorotte.ini"))
        )
    }));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_config_path_source() {
    let override_path = std::path::PathBuf::from("custom-config-root").join("sorotte.ini");
    let expected_message =
        super::super::GuiStartupConfigPathSource::Override(override_path.clone()).startup_message();
    let startup_actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::super::GuiStartupConfigPathSource::Override(
            override_path,
        )),
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains("[Notifications] count=0"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_player_ipc_source() {
    let startup_actions = super::super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SOROTTE_MPV_IPC_PATH" => Some("/tmp/syncplay-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(
        "Startup will try mpv JSON IPC from SOROTTE_MPV_IPC_PATH (/tmp/syncplay-mpv.sock)."
    ));
    assert!(preview.contains("[Notifications] count=0"));
}
