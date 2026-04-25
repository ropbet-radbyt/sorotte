use super::*;

#[test]
fn persist_and_load_syncplay_cli_stored_settings_mvp_roundtrips_via_env_override_path() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("syncplay-cli-config-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");

    let seed_contents = "[general]\nlanguage = en\n";
    std::fs::write(&config_path, seed_contents).expect("seed config should write");
    env.set_var(key, &config_path);
    let config = ClientLoopConfig {
        host: "stored.example".to_owned(),
        port: 1234,
        server_password: None,
        username: "stored-user".to_owned(),
        room: "stored-room".to_owned(),
        autoplay_enabled: true,
        autoplay_require_same_filenames: true,
        ready_at_start_override: Some(true),
        shared_playlists_enabled_override: Some(false),
        pause_on_leave_override: Some(true),
        loop_at_end_of_playlist_override: Some(false),
        loop_single_files_override: Some(true),
        only_switch_to_trusted_domains_override: Some(false),
        trusted_domains_override: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        rewind_on_desync_override: Some(false),
        fastforward_on_desync_override: Some(true),
        slow_on_desync_override: Some(false),
        dont_slow_down_with_me_override: Some(true),
        rewind_threshold_seconds_override: Some(1.25),
        fastforward_threshold_seconds_override: Some(3.5),
        slowdown_threshold_seconds_override: Some(2.25),
        unpause_action_override: Some(UnpauseActionMode::IfMinUsersReady),
        auto_play_threshold_override: Some(AutoplayThresholdOverride::Set(5)),
        filename_privacy_mode: PrivacyMode::SendHashed,
        filesize_privacy_mode: PrivacyMode::DoNotSend,
        show_duration_notification_override: Some(false),
        show_same_room_osd_override: Some(true),
        show_osd_warnings_override: Some(false),
        show_noncontroller_osd_override: Some(true),
        show_different_room_osd_override: Some(false),
        ..test_client_loop_config()
    };
    persist_syncplay_cli_stored_settings_mvp_legacy_compatible(&config)
        .expect("persisted settings should succeed");

    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(
        loaded,
        StoredClientSettingsMvp {
            language: Some("en".to_owned()),
            check_for_updates_automatically: None,
            last_checked_for_updates: None,
            host: Some("stored.example".to_owned()),
            port: Some(1234),
            server_password: None,
            username: Some("stored-user".to_owned()),
            room: Some("stored-room".to_owned()),
            room_list: None,
            player_path: None,
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
            pause_on_leave: Some(true),
            loop_at_end_of_playlist: Some(false),
            loop_single_files: Some(true),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(vec![
                "youtube.com".to_owned(),
                "*.example.com/videos".to_owned(),
            ]),
            rewind_on_desync: Some(false),
            fastforward_on_desync: Some(true),
            slow_on_desync: Some(false),
            dont_slow_down_with_me: Some(true),
            rewind_threshold_seconds: Some(1.25),
            fastforward_threshold_seconds: Some(3.5),
            slowdown_threshold_seconds: Some(2.25),
            unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(5)),
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
            show_duration_notification: Some(false),
            show_same_room_osd: Some(true),
            show_osd_warnings: Some(false),
            show_slowdown_osd: None,
            show_noncontroller_osd: Some(true),
            show_different_room_osd: Some(false),
            show_contact_info: None,
        }
    );

    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("[general]\nlanguage = en\n"));
    assert!(written_contents.contains("[server_data]\nhost = stored.example\nport = 1234\n"));
    assert!(written_contents.contains("[client_settings]"));
    assert!(written_contents.contains("autoplayInitialState = True\n"));
    assert!(written_contents.contains("autoplayRequireSameFilenames = True\n"));
    assert!(written_contents.contains("readyAtStart = True\n"));
    assert!(written_contents.contains("sharedPlaylistEnabled = False\n"));
    assert!(written_contents.contains("pauseOnLeave = True\n"));
    assert!(written_contents.contains("loopAtEndOfPlaylist = False\n"));
    assert!(written_contents.contains("loopSingleFiles = True\n"));
    assert!(written_contents.contains("onlySwitchToTrustedDomains = False\n"));
    assert!(
        written_contents.contains("trustedDomains = ['youtube.com', '*.example.com/videos']\n")
    );
    assert!(written_contents.contains("rewindOnDesync = False\n"));
    assert!(written_contents.contains("fastforwardOnDesync = True\n"));
    assert!(written_contents.contains("slowOnDesync = False\n"));
    assert!(written_contents.contains("dontSlowDownWithMe = True\n"));
    assert!(written_contents.contains("rewindThreshold = 1.25\n"));
    assert!(written_contents.contains("fastforwardThreshold = 3.5\n"));
    assert!(written_contents.contains("slowdownThreshold = 2.25\n"));
    assert!(written_contents.contains("unpauseAction = IfMinUsersReady\n"));
    assert!(written_contents.contains("autoplayMinUsers = 5\n"));
    assert!(written_contents.contains("filenamePrivacyMode = SendHashed\n"));
    assert!(written_contents.contains("filesizePrivacyMode = DoNotSend\n"));
    assert!(written_contents.contains("[gui]"));
    assert!(written_contents.contains("showDurationNotification = False\n"));
    assert!(written_contents.contains("showSameRoomOSD = True\n"));
    assert!(written_contents.contains("showOSDWarnings = False\n"));
    assert!(written_contents.contains("showNonControllerOSD = True\n"));
    assert!(written_contents.contains("showDifferentRoomOSD = False\n"));

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_syncplay_cli_language_setting_legacy_compatible_updates_general_language() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("syncplay-cli-config-language-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(&config_path, "[general]\nlanguage = en\n").expect("seed config should write");
    env.set_var(key, &config_path);
    persist_syncplay_cli_language_setting_legacy_compatible("fr")
        .expect("language setting should persist");
    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(loaded.language.as_deref(), Some("fr"));
    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("[general]\nlanguage = fr\n"));

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_syncplay_cli_language_setting_legacy_compatible_normalizes_supported_aliases() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-config-language-normalize-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(&config_path, "[general]\nlanguage = en\n").expect("seed config should write");
    env.set_var(key, &config_path);
    persist_syncplay_cli_language_setting_legacy_compatible("PT-br")
        .expect("language alias should persist");
    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(loaded.language.as_deref(), Some("pt_BR"));
    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("[general]\nlanguage = pt_BR\n"));

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}
