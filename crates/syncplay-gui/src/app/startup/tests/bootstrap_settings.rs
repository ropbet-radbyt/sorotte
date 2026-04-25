use super::*;

#[test]
fn gui_client_core_chat_tcp_bootstrap_overrides_settings_enable_chat_and_seed_connection() {
    let settings = super::super::gui_startup_settings_from_lookup_with(
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
    let settings = super::super::gui_startup_settings_from_lookup_with(
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
    let settings = super::super::gui_startup_settings_from_lookup(
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
    let settings = super::super::gui_startup_settings_from_lookup(
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
    let settings = super::super::gui_startup_settings_from_lookup_with(
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
    let settings = super::super::gui_startup_settings_from_lookup_with(
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
