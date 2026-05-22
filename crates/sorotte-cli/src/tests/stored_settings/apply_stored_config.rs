use super::*;

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_preserves_env_precedence() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_host = "SOROTTE_CLIENT_HOST";
    let key_name = "SOROTTE_CLIENT_NAME";
    let key_server_password = "SOROTTE_CLIENT_SERVER_PASSWORD";
    let key_ready_at_start = "SOROTTE_CLIENT_READY_AT_START";
    let key_shared_playlist_enabled = "SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED";
    let key_show_osd_warnings = "SOROTTE_CLIENT_SHOW_OSD_WARNINGS";
    let key_pause_on_leave = "SOROTTE_CLIENT_PAUSE_ON_LEAVE";
    let key_dont_slow_down_with_me = "SOROTTE_CLIENT_DONT_SLOW_DOWN_WITH_ME";
    let key_rewind_threshold = "SOROTTE_CLIENT_REWIND_THRESHOLD_SECONDS";
    let prior_host = std::env::var_os(key_host);
    let prior_name = std::env::var_os(key_name);
    let prior_server_password = std::env::var_os(key_server_password);
    let prior_ready_at_start = std::env::var_os(key_ready_at_start);
    let prior_shared_playlist_enabled = std::env::var_os(key_shared_playlist_enabled);
    let prior_show_osd_warnings = std::env::var_os(key_show_osd_warnings);
    let prior_pause_on_leave = std::env::var_os(key_pause_on_leave);
    let prior_dont_slow_down_with_me = std::env::var_os(key_dont_slow_down_with_me);
    let prior_rewind_threshold = std::env::var_os(key_rewind_threshold);
    env.set_var(key_host, "env.example");
    env.set_var(key_name, "env-user");
    env.set_var(key_server_password, "env-secret");
    env.set_var(key_ready_at_start, "false");
    env.set_var(key_shared_playlist_enabled, "true");
    env.set_var(key_show_osd_warnings, "true");
    env.set_var(key_pause_on_leave, "true");
    env.set_var(key_dont_slow_down_with_me, "true");
    env.set_var(key_rewind_threshold, "9.5");

    let mut config = test_client_loop_config();
    let original_host = config.host.clone();
    let original_username = config.username.clone();
    let original_server_password = config.server_password.clone();
    let original_shared_playlist_enabled = config.shared_playlists_enabled_override;
    let original_show_osd_warnings = config.show_osd_warnings_override;
    let original_dont_slow_down_with_me = config.dont_slow_down_with_me_override;
    let original_rewind_threshold = config.rewind_threshold_seconds_override;
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            language: Some("de".to_owned()),
            check_for_updates_automatically: Some(false),
            last_checked_for_updates: Some("2026-02-23 11:22:33.444".to_owned()),
            host: Some("stored.example".to_owned()),
            port: Some(4321),
            server_password: Some("stored-secret".to_owned()),
            username: Some("stored-user".to_owned()),
            room: Some("stored-room".to_owned()),
            room_list: None,
            player_path: Some("C:/players/stored-mpv.exe".to_owned()),
            per_player_arguments: None,
            media_search_directories: None,
            public_servers: None,
            folder_search_first_file_timeout_seconds: None,
            folder_search_timeout_seconds: None,
            folder_search_double_check_interval_seconds: None,
            folder_search_warning_threshold_seconds: None,
            force_gui_prompt: None,
            autoplay_initial_state: Some(true),
            autoplay_require_same_filenames: Some(true),
            ready_at_start: Some(true),
            shared_playlist_enabled: Some(false),
            pause_on_leave: Some(false),
            loop_at_end_of_playlist: Some(true),
            loop_single_files: Some(false),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(vec![
                "stored.example".to_owned(),
                "*.video.example/path".to_owned(),
            ]),
            rewind_on_desync: Some(false),
            fastforward_on_desync: Some(true),
            slow_on_desync: Some(false),
            dont_slow_down_with_me: Some(false),
            rewind_threshold_seconds: Some(1.25),
            fastforward_threshold_seconds: Some(3.5),
            slowdown_threshold_seconds: Some(2.25),
            unpause_action: Some(UnpauseActionMode::IfOthersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(4)),
            filename_privacy_mode: Some(PrivacyMode::SendHashed),
            filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
            autosave_joins_to_list: None,
            show_osd: None,
            chat_input_enabled: None,
            chat_input_font_underline: None,
            chat_input_font_family: None,
            chat_input_relative_font_size: None,
            chat_input_font_weight: None,
            chat_input_font_color: None,
            chat_input_position: None,
            chat_direct_input: None,
            chat_output_enabled: None,
            chat_output_font_underline: None,
            chat_output_font_family: None,
            chat_output_relative_font_size: None,
            chat_output_font_weight: None,
            chat_output_mode: None,
            chat_move_osd: None,
            chat_max_lines: None,
            chat_top_margin: None,
            chat_left_margin: None,
            chat_bottom_margin: None,
            chat_osd_margin: None,
            notification_timeout_seconds: None,
            alert_timeout_seconds: None,
            chat_timeout_seconds: None,
            show_duration_notification: Some(true),
            show_same_room_osd: Some(true),
            show_osd_warnings: Some(false),
            show_slowdown_osd: None,
            show_noncontroller_osd: Some(true),
            show_different_room_osd: Some(false),
            show_contact_info: None,
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.host, original_host);
    assert_eq!(config.username, original_username);
    assert_eq!(config.server_password, original_server_password);
    assert_eq!(config.port, 4321);
    assert_eq!(config.room, "stored-room");
    assert!(config.autoplay_enabled);
    assert!(config.autoplay_require_same_filenames);
    assert_eq!(config.ready_at_start_override, None);
    assert_eq!(
        config.shared_playlists_enabled_override,
        original_shared_playlist_enabled
    );
    assert_eq!(config.pause_on_leave_override, None);
    assert_eq!(config.loop_at_end_of_playlist_override, Some(true));
    assert_eq!(config.loop_single_files_override, Some(false));
    assert_eq!(config.only_switch_to_trusted_domains_override, Some(false));
    assert_eq!(
        config.trusted_domains_override,
        Some(vec![
            "stored.example".to_owned(),
            "*.video.example/path".to_owned(),
        ])
    );
    assert_eq!(config.rewind_on_desync_override, Some(false));
    assert_eq!(config.fastforward_on_desync_override, Some(true));
    assert_eq!(config.slow_on_desync_override, Some(false));
    assert_eq!(
        config.dont_slow_down_with_me_override,
        original_dont_slow_down_with_me
    );
    assert_eq!(
        config.rewind_threshold_seconds_override,
        original_rewind_threshold
    );
    assert_eq!(config.fastforward_threshold_seconds_override, Some(3.5));
    assert_eq!(config.slowdown_threshold_seconds_override, Some(2.25));
    assert_eq!(
        config.unpause_action_override,
        Some(UnpauseActionMode::IfOthersReady)
    );
    assert_eq!(
        config.auto_play_threshold_override,
        Some(AutoplayThresholdOverride::Set(4))
    );
    assert_eq!(config.filename_privacy_mode, PrivacyMode::SendHashed);
    assert_eq!(config.filesize_privacy_mode, PrivacyMode::DoNotSend);
    assert_eq!(config.show_duration_notification_override, Some(true));
    assert_eq!(config.show_same_room_osd_override, Some(true));
    assert_eq!(
        config.show_osd_warnings_override,
        original_show_osd_warnings
    );
    assert_eq!(config.show_noncontroller_osd_override, Some(true));
    assert_eq!(config.show_different_room_osd_override, Some(false));

    match prior_host {
        Some(value) => env.set_var(key_host, value),
        None => env.remove_var(key_host),
    }
    match prior_name {
        Some(value) => env.set_var(key_name, value),
        None => env.remove_var(key_name),
    }
    match prior_server_password {
        Some(value) => env.set_var(key_server_password, value),
        None => env.remove_var(key_server_password),
    }
    match prior_ready_at_start {
        Some(value) => env.set_var(key_ready_at_start, value),
        None => env.remove_var(key_ready_at_start),
    }
    match prior_shared_playlist_enabled {
        Some(value) => env.set_var(key_shared_playlist_enabled, value),
        None => env.remove_var(key_shared_playlist_enabled),
    }
    match prior_show_osd_warnings {
        Some(value) => env.set_var(key_show_osd_warnings, value),
        None => env.remove_var(key_show_osd_warnings),
    }
    match prior_pause_on_leave {
        Some(value) => env.set_var(key_pause_on_leave, value),
        None => env.remove_var(key_pause_on_leave),
    }
    match prior_dont_slow_down_with_me {
        Some(value) => env.set_var(key_dont_slow_down_with_me, value),
        None => env.remove_var(key_dont_slow_down_with_me),
    }
    match prior_rewind_threshold {
        Some(value) => env.set_var(key_rewind_threshold, value),
        None => env.remove_var(key_rewind_threshold),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_applies_server_password() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_server_password = "SOROTTE_CLIENT_SERVER_PASSWORD";
    let prior_server_password = std::env::var_os(key_server_password);
    env.remove_var(key_server_password);
    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            server_password: Some("stored-secret".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.server_password.as_deref(), Some("stored-secret"));

    match prior_server_password {
        Some(value) => env.set_var(key_server_password, value),
        None => env.remove_var(key_server_password),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_applies_ready_at_start() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_ready_at_start = "SOROTTE_CLIENT_READY_AT_START";
    let prior_ready_at_start = std::env::var_os(key_ready_at_start);
    env.remove_var(key_ready_at_start);
    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            ready_at_start: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.ready_at_start_override, Some(true));

    match prior_ready_at_start {
        Some(value) => env.set_var(key_ready_at_start, value),
        None => env.remove_var(key_ready_at_start),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_applies_shared_playlist_enabled() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_shared_playlist_enabled = "SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED";
    let prior_shared_playlist_enabled = std::env::var_os(key_shared_playlist_enabled);
    env.remove_var(key_shared_playlist_enabled);
    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            shared_playlist_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.shared_playlists_enabled_override, Some(false));

    match prior_shared_playlist_enabled {
        Some(value) => env.set_var(key_shared_playlist_enabled, value),
        None => env.remove_var(key_shared_playlist_enabled),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_uses_room_list_when_room_missing() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_room = "SOROTTE_CLIENT_ROOM";
    let prior_room = std::env::var_os(key_room);
    env.remove_var(key_room);
    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            room: None,
            room_list: Some(vec![
                "".to_owned(),
                "+room:ABCDEF123456:AB-123-456".to_owned(),
                "room-z".to_owned(),
            ]),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.room, "+room:ABCDEF123456");
    assert_eq!(
        config.controlled_room_password_override.as_deref(),
        Some("AB-123-456")
    );

    match prior_room {
        Some(value) => env.set_var(key_room, value),
        None => env.remove_var(key_room),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_prefers_room_over_room_list() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_room = "SOROTTE_CLIENT_ROOM";
    let prior_room = std::env::var_os(key_room);
    env.remove_var(key_room);
    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            room: Some("stored-room".to_owned()),
            room_list: Some(vec!["fallback-room".to_owned()]),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.room, "stored-room");

    match prior_room {
        Some(value) => env.set_var(key_room, value),
        None => env.remove_var(key_room),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_uses_public_servers_when_host_missing() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_host = "SOROTTE_CLIENT_HOST";
    let key_port = "SOROTTE_CLIENT_PORT";
    let prior_host = std::env::var_os(key_host);
    let prior_port = std::env::var_os(key_port);
    env.remove_var(key_host);
    env.remove_var(key_port);

    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            host: None,
            port: None,
            public_servers: Some(vec![
                ("Primary".to_owned(), "".to_owned()),
                ("Fallback".to_owned(), "public.example:7777".to_owned()),
            ]),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.host, "public.example");
    assert_eq!(config.port, 7777);

    match prior_host {
        Some(value) => env.set_var(key_host, value),
        None => env.remove_var(key_host),
    }
    match prior_port {
        Some(value) => env.set_var(key_port, value),
        None => env.remove_var(key_port),
    }
}

#[test]
fn apply_stored_client_settings_mvp_if_env_absent_prefers_stored_host_over_public_servers() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key_host = "SOROTTE_CLIENT_HOST";
    let key_port = "SOROTTE_CLIENT_PORT";
    let prior_host = std::env::var_os(key_host);
    let prior_port = std::env::var_os(key_port);
    env.remove_var(key_host);
    env.remove_var(key_port);

    let mut config = test_client_loop_config();
    apply_stored_client_settings_mvp_if_env_absent(
        &mut config,
        &StoredClientSettingsMvp {
            host: Some("stored.example".to_owned()),
            port: Some(4444),
            public_servers: Some(vec![(
                "Fallback".to_owned(),
                "public.example:7777".to_owned(),
            )]),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert_eq!(config.host, "stored.example");
    assert_eq!(config.port, 4444);

    match prior_host {
        Some(value) => env.set_var(key_host, value),
        None => env.remove_var(key_host),
    }
    match prior_port {
        Some(value) => env.set_var(key_port, value),
        None => env.remove_var(key_port),
    }
}
