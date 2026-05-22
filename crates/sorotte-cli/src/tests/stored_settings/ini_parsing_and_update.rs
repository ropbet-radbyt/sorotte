use super::*;

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_reads_python_style_sections() {
    let contents = "\u{feff}[general]\nlanguage = de\ncheckForUpdatesAutomatically = False\nlastCheckedForUpdates = 2026-02-23 11:22:33.444\n[server_data]\nhost = example.org\nport = 12345\npassword = secret\n\n[client_settings]\nname = Alice%%20\nroom = room-1\nroomList = ['room-1', 'room-2']\nplayerPath = C:/players/mpv.exe\nperPlayerArguments = {'C:/players/mpv.exe': ['--fs', '--profile=fast']}\nmediaSearchDirectories = ['C:/Media', 'D:/TV Shows']\npublicServers = [['syncplay.pl:8995 (France)', 'syncplay.pl:8995'], ['Custom', 'custom.example:8999']]\nfolderSearchFirstFileTimeout = 25.0\nfolderSearchTimeout = 20.0\nfolderSearchDoubleCheckInterval = 30.0\nfolderSearchWarningThreshold = 2.0\nforceGuiPrompt = True\nautoplayInitialState = True\nautoplayRequireSameFilenames = True\nreadyAtStart = True\nsharedPlaylistEnabled = False\npauseOnLeave = False\nloopAtEndOfPlaylist = True\nloopSingleFiles = False\nonlySwitchToTrustedDomains = True\ntrustedDomains = ['youtube.com', '*.example.com/videos']\nrewindOnDesync = False\nfastforwardOnDesync = True\nslowOnDesync = False\ndontSlowDownWithMe = True\nrewindThreshold = 1.25\nfastforwardThreshold = 3.5\nslowdownThreshold = 2.25\nunpauseAction = IfMinUsersReady\nautoplayMinUsers = 3\nfilenamePrivacyMode = SendHashed\nfilesizePrivacyMode = DoNotSend\n\n[gui]\nautosaveJoinsToList = True\nshowOSD = False\nchatInputEnabled = True\nchatInputFontUnderline = False\nchatInputFontFamily = sans-serif\nchatInputRelativeFontSize = 12\nchatInputFontWeight = 50\nchatInputFontColor = #abcdef\nchatInputPosition = Top\nchatDirectInput = False\nchatOutputEnabled = True\nchatOutputFontUnderline = False\nchatOutputFontFamily = serif\nchatOutputRelativeFontSize = 13\nchatOutputFontWeight = 75\nchatOutputMode = Chatroom\nchatMoveOSD = True\nchatMaxLines = 7\nchatTopMargin = 25\nchatLeftMargin = 20\nchatBottomMargin = 30\nchatOSDMargin = 110\nnotificationTimeout = 3\nalertTimeout = 5\nchatTimeout = 7\nshowDurationNotification = False\nshowSameRoomOSD = True\nshowOSDWarnings = False\nshowSlowdownOSD = True\nshowNonControllerOSD = True\nshowDifferentRoomOSD = False\nshowContactInfo = True\n";
    let settings = parse_sorotte_ini_stored_client_settings_mvp(contents);
    assert_eq!(
        settings,
        StoredClientSettingsMvp {
            language: Some("de".to_owned()),
            check_for_updates_automatically: Some(false),
            last_checked_for_updates: Some("2026-02-23 11:22:33.444".to_owned()),
            host: Some("example.org".to_owned()),
            port: Some(12345),
            server_password: Some("secret".to_owned()),
            username: Some("Alice%20".to_owned()),
            room: Some("room-1".to_owned()),
            room_list: Some(vec!["room-1".to_owned(), "room-2".to_owned()]),
            player_path: Some("C:/players/mpv.exe".to_owned()),
            per_player_arguments: Some(std::collections::BTreeMap::from([(
                "C:/players/mpv.exe".to_owned(),
                vec!["--fs".to_owned(), "--profile=fast".to_owned()],
            )])),
            media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/TV Shows".to_owned(),]),
            public_servers: Some(vec![
                (
                    "syncplay.pl:8995 (France)".to_owned(),
                    "syncplay.pl:8995".to_owned(),
                ),
                ("Custom".to_owned(), "custom.example:8999".to_owned()),
            ]),
            folder_search_first_file_timeout_seconds: Some(25.0),
            folder_search_timeout_seconds: Some(20.0),
            folder_search_double_check_interval_seconds: Some(30.0),
            folder_search_warning_threshold_seconds: Some(2.0),
            force_gui_prompt: Some(true),
            autoplay_initial_state: Some(true),
            autoplay_require_same_filenames: Some(true),
            ready_at_start: Some(true),
            shared_playlist_enabled: Some(false),
            pause_on_leave: Some(false),
            loop_at_end_of_playlist: Some(true),
            loop_single_files: Some(false),
            only_switch_to_trusted_domains: Some(true),
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
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
            filename_privacy_mode: Some(PrivacyMode::SendHashed),
            filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
            autosave_joins_to_list: Some(true),
            show_osd: Some(false),
            chat_input_enabled: Some(true),
            chat_input_font_underline: Some(false),
            chat_input_font_family: Some("sans-serif".to_owned()),
            chat_input_relative_font_size: Some(12),
            chat_input_font_weight: Some(50),
            chat_input_font_color: Some("#abcdef".to_owned()),
            chat_input_position: Some("Top".to_owned()),
            chat_direct_input: Some(false),
            chat_output_enabled: Some(true),
            chat_output_font_underline: Some(false),
            chat_output_font_family: Some("serif".to_owned()),
            chat_output_relative_font_size: Some(13),
            chat_output_font_weight: Some(75),
            chat_output_mode: Some("Chatroom".to_owned()),
            chat_move_osd: Some(true),
            chat_max_lines: Some(7),
            chat_top_margin: Some(25),
            chat_left_margin: Some(20),
            chat_bottom_margin: Some(30),
            chat_osd_margin: Some(110),
            notification_timeout_seconds: Some(3),
            alert_timeout_seconds: Some(5),
            chat_timeout_seconds: Some(7),
            show_duration_notification: Some(false),
            show_same_room_osd: Some(true),
            show_osd_warnings: Some(false),
            show_slowdown_osd: Some(true),
            show_noncontroller_osd: Some(true),
            show_different_room_osd: Some(false),
            show_contact_info: Some(true),
            ..StoredClientSettingsMvp::default()
        }
    );
}

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_normalizes_supported_language_tags_and_drops_invalid_values()
 {
    let normalized = parse_sorotte_ini_stored_client_settings_mvp("[general]\nlanguage = PT-br\n");
    let invalid = parse_sorotte_ini_stored_client_settings_mvp("[general]\nlanguage = klingon\n");

    assert_eq!(normalized.language.as_deref(), Some("pt_BR"));
    assert_eq!(invalid.language, None);
}

#[test]
fn legacy_utc_timestamp_string_legacy_compatible_roundtrips_fixed_timestamp() {
    let timestamp = UNIX_EPOCH + Duration::from_secs(1_800_000_000) + Duration::from_millis(123);
    let formatted = legacy_utc_timestamp_string_legacy_compatible(timestamp);
    let parsed = parse_legacy_utc_timestamp_legacy_compatible(&formatted)
        .expect("formatted timestamp should parse");

    assert_eq!(
        parsed
            .duration_since(UNIX_EPOCH)
            .expect("parsed timestamp should be after epoch"),
        timestamp
            .duration_since(UNIX_EPOCH)
            .expect("seed timestamp should be after epoch")
    );
}

#[test]
fn should_run_headless_automatic_update_check_legacy_compatible_honors_frequency() {
    let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let recent_timestamp = legacy_utc_timestamp_string_legacy_compatible(
        now - Duration::from_secs(crate::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS - 1),
    );
    let stale_timestamp = legacy_utc_timestamp_string_legacy_compatible(
        now - Duration::from_secs(crate::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS + 1),
    );

    assert!(
        should_run_headless_automatic_update_check_legacy_compatible(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: None,
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ),
        "missing timestamp should be treated as due"
    );
    assert!(
        !should_run_headless_automatic_update_check_legacy_compatible(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(recent_timestamp),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ),
        "recent timestamp should suppress the headless update-check notice"
    );
    assert!(
        should_run_headless_automatic_update_check_legacy_compatible(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(stale_timestamp),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ),
        "stale timestamp should re-enable the headless update-check notice"
    );
    assert!(
        !should_run_headless_automatic_update_check_legacy_compatible(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(false),
                last_checked_for_updates: None,
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ),
        "disabled automatic update checks should remain inert"
    );
}

#[test]
fn persist_sorotte_cli_last_checked_for_updates_setting_legacy_compatible_updates_general_timestamp()
 {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SOROTTE_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("sorotte-cli-update-check-persist-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let config_path = temp_dir.join("sorotte.ini");
    std::fs::write(
            &config_path,
            "[general]\ncheckForUpdatesAutomatically = True\nlastCheckedForUpdates = 2020-01-01 00:00:00.000\n",
        )
        .expect("seed config should write");
    env.set_var(key, &config_path);
    persist_sorotte_cli_last_checked_for_updates_setting_legacy_compatible(
        "2026-03-02 12:34:56.789",
    )
    .expect("timestamp persistence should succeed");

    let loaded = load_sorotte_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(
        loaded.last_checked_for_updates.as_deref(),
        Some("2026-03-02 12:34:56.789")
    );
    assert_eq!(loaded.check_for_updates_automatically, Some(true));

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_preserves_unrelated_entries() {
    let existing =
        "[general]\nlanguage = en\n\n[client_settings]\nroom = old-room\npublicservers = []\n";
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        existing,
        &StoredClientSettingsMvp {
            language: None,
            check_for_updates_automatically: Some(false),
            last_checked_for_updates: Some("2026-02-23 11:22:33.444".to_owned()),
            host: Some("example.org".to_owned()),
            port: Some(8999),
            server_password: Some("secret".to_owned()),
            username: Some("alice".to_owned()),
            room: Some("new-room".to_owned()),
            room_list: Some(vec!["room-a".to_owned(), "room-b".to_owned()]),
            player_path: Some("C:/players/mpv.exe".to_owned()),
            per_player_arguments: Some(std::collections::BTreeMap::from([(
                "C:/players/mpv.exe".to_owned(),
                vec!["--fs".to_owned(), "--profile=fast".to_owned()],
            )])),
            media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/TV Shows".to_owned()]),
            public_servers: Some(vec![
                (
                    "syncplay.pl:8995 (France)".to_owned(),
                    "syncplay.pl:8995".to_owned(),
                ),
                ("Custom".to_owned(), "custom.example:8999".to_owned()),
            ]),
            folder_search_first_file_timeout_seconds: Some(25.0),
            folder_search_timeout_seconds: Some(20.0),
            folder_search_double_check_interval_seconds: Some(30.0),
            folder_search_warning_threshold_seconds: Some(2.0),
            force_gui_prompt: Some(true),
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
            unpause_action: Some(UnpauseActionMode::IfOthersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Disable),
            filename_privacy_mode: Some(PrivacyMode::SendHashed),
            filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
            autosave_joins_to_list: Some(true),
            show_osd: Some(false),
            chat_input_enabled: Some(true),
            chat_input_font_underline: Some(false),
            chat_input_font_family: Some("sans-serif".to_owned()),
            chat_input_relative_font_size: Some(12),
            chat_input_font_weight: Some(50),
            chat_input_font_color: Some("#abcdef".to_owned()),
            chat_input_position: Some("Top".to_owned()),
            chat_direct_input: Some(false),
            chat_output_enabled: Some(true),
            chat_output_font_underline: Some(false),
            chat_output_font_family: Some("serif".to_owned()),
            chat_output_relative_font_size: Some(13),
            chat_output_font_weight: Some(75),
            chat_output_mode: Some("Chatroom".to_owned()),
            chat_move_osd: Some(true),
            chat_max_lines: Some(7),
            chat_top_margin: Some(25),
            chat_left_margin: Some(20),
            chat_bottom_margin: Some(30),
            chat_osd_margin: Some(110),
            notification_timeout_seconds: Some(3),
            alert_timeout_seconds: Some(5),
            chat_timeout_seconds: Some(7),
            show_duration_notification: Some(true),
            show_same_room_osd: Some(true),
            show_osd_warnings: Some(false),
            show_slowdown_osd: Some(true),
            show_noncontroller_osd: Some(false),
            show_different_room_osd: Some(true),
            show_contact_info: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[general]\nlanguage = en\n"));
    assert!(updated.contains("checkForUpdatesAutomatically = False\n"));
    assert!(updated.contains("lastCheckedForUpdates = 2026-02-23 11:22:33.444\n"));
    assert!(updated.contains("[server_data]\nhost = example.org\nport = 8999\n"));
    assert!(updated.contains("password = secret\n"));
    assert!(updated.contains("[client_settings]"));
    assert!(updated.contains("room = new-room\n"));
    assert!(updated.contains("roomList = ['room-a', 'room-b']\n"));
    assert!(updated.contains("playerPath = C:/players/mpv.exe\n"));
    assert!(
        updated
            .contains("perPlayerArguments = {'C:/players/mpv.exe': ['--fs', '--profile=fast']}\n")
    );
    assert!(updated.contains("mediaSearchDirectories = ['C:/Media', 'D:/TV Shows']\n"));
    assert!(
            updated.contains(
                "publicServers = [['syncplay.pl:8995 (France)', 'syncplay.pl:8995'], ['Custom', 'custom.example:8999']]\n"
            )
        );
    assert!(updated.contains("folderSearchFirstFileTimeout = 25\n"));
    assert!(updated.contains("folderSearchTimeout = 20\n"));
    assert!(updated.contains("folderSearchDoubleCheckInterval = 30\n"));
    assert!(updated.contains("folderSearchWarningThreshold = 2\n"));
    assert!(updated.contains("forceGuiPrompt = True\n"));
    assert!(!updated.contains("publicservers = []\n"));
    assert!(updated.contains("name = alice\n"));
    assert!(updated.contains("autoplayInitialState = True\n"));
    assert!(updated.contains("autoplayRequireSameFilenames = True\n"));
    assert!(updated.contains("readyAtStart = True\n"));
    assert!(updated.contains("sharedPlaylistEnabled = False\n"));
    assert!(updated.contains("pauseOnLeave = True\n"));
    assert!(updated.contains("loopAtEndOfPlaylist = False\n"));
    assert!(updated.contains("loopSingleFiles = True\n"));
    assert!(updated.contains("onlySwitchToTrustedDomains = False\n"));
    assert!(updated.contains("trustedDomains = ['youtube.com', '*.example.com/videos']\n"));
    assert!(updated.contains("rewindOnDesync = False\n"));
    assert!(updated.contains("fastforwardOnDesync = True\n"));
    assert!(updated.contains("slowOnDesync = False\n"));
    assert!(updated.contains("dontSlowDownWithMe = True\n"));
    assert!(updated.contains("rewindThreshold = 1.25\n"));
    assert!(updated.contains("fastforwardThreshold = 3.5\n"));
    assert!(updated.contains("slowdownThreshold = 2.25\n"));
    assert!(updated.contains("unpauseAction = IfOthersReady\n"));
    assert!(updated.contains("autoplayMinUsers = 0\n"));
    assert!(updated.contains("filenamePrivacyMode = SendHashed\n"));
    assert!(updated.contains("filesizePrivacyMode = DoNotSend\n"));
    assert!(updated.contains("[gui]"));
    assert!(updated.contains("autosaveJoinsToList = True\n"));
    assert!(updated.contains("showOSD = False\n"));
    assert!(updated.contains("chatInputEnabled = True\n"));
    assert!(updated.contains("chatInputFontUnderline = False\n"));
    assert!(updated.contains("chatInputFontFamily = sans-serif\n"));
    assert!(updated.contains("chatInputRelativeFontSize = 12\n"));
    assert!(updated.contains("chatInputFontWeight = 50\n"));
    assert!(updated.contains("chatInputFontColor = #abcdef\n"));
    assert!(updated.contains("chatInputPosition = Top\n"));
    assert!(updated.contains("chatDirectInput = False\n"));
    assert!(updated.contains("chatOutputEnabled = True\n"));
    assert!(updated.contains("chatOutputFontUnderline = False\n"));
    assert!(updated.contains("chatOutputFontFamily = serif\n"));
    assert!(updated.contains("chatOutputRelativeFontSize = 13\n"));
    assert!(updated.contains("chatOutputFontWeight = 75\n"));
    assert!(updated.contains("chatOutputMode = Chatroom\n"));
    assert!(updated.contains("chatMoveOSD = True\n"));
    assert!(updated.contains("chatMaxLines = 7\n"));
    assert!(updated.contains("chatTopMargin = 25\n"));
    assert!(updated.contains("chatLeftMargin = 20\n"));
    assert!(updated.contains("chatBottomMargin = 30\n"));
    assert!(updated.contains("chatOSDMargin = 110\n"));
    assert!(updated.contains("notificationTimeout = 3\n"));
    assert!(updated.contains("alertTimeout = 5\n"));
    assert!(updated.contains("chatTimeout = 7\n"));
    assert!(updated.contains("showDurationNotification = True\n"));
    assert!(updated.contains("showOSDWarnings = False\n"));
    assert!(updated.contains("showSlowdownOSD = True\n"));
    assert!(updated.contains("showContactInfo = True\n"));
    assert!(!updated.contains("room = old-room"));
}

#[test]
fn per_player_arguments_serialized_python_dict_parser_and_formatter_roundtrip() {
    let raw = "{'C:/players/mpv.exe': ['--fs', '--profile=fast'], 'C:/players/vlc.exe': []}";
    let parsed = parse_serialized_per_player_arguments_map_legacy_compatible(raw)
        .expect("perPlayerArguments dict should parse");
    assert_eq!(
        parsed.get("C:/players/mpv.exe"),
        Some(&vec!["--fs".to_owned(), "--profile=fast".to_owned()])
    );
    assert_eq!(
        parsed.get("C:/players/vlc.exe"),
        Some(&Vec::<String>::new())
    );

    let rendered = format_serialized_per_player_arguments_map_legacy_compatible(&parsed);
    assert_eq!(rendered, raw);
}

#[test]
fn public_servers_serialized_python_list_parser_and_formatter_roundtrip() {
    let raw =
        "[['syncplay.pl:8995 (France)', 'syncplay.pl:8995'], ['Custom', 'custom.example:8999']]";
    let parsed = parse_serialized_public_servers_list_legacy_compatible(raw)
        .expect("publicServers list should parse");
    assert_eq!(
        parsed,
        vec![
            (
                "syncplay.pl:8995 (France)".to_owned(),
                "syncplay.pl:8995".to_owned()
            ),
            ("Custom".to_owned(), "custom.example:8999".to_owned()),
        ]
    );

    let rendered = format_serialized_public_servers_list_legacy_compatible(&parsed);
    assert_eq!(rendered, raw);
}
