use super::*;

#[test]
fn normalize_controlled_room_input_extracts_canonical_room_and_password() {
    let (room, password) =
        normalize_controlled_room_input("+room:ABCDEF123456:ab-123-456".to_owned());
    assert_eq!(room, "+room:ABCDEF123456");
    assert_eq!(password.as_deref(), Some("AB-123-456"));

    let (room, password) = normalize_controlled_room_input("room1".to_owned());
    assert_eq!(room, "room1");
    assert!(password.is_none());
}

#[test]
fn controlled_room_base_name_legacy_compatible_strips_managed_suffix() {
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+base-room:ABCDEF123456"),
        "base-room"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+room_name:ABCDEF12345_"),
        "room_name"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("room1"),
        "room1"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible(" room1 "),
        " room1 "
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+room:SHORT"),
        "+room:SHORT"
    );
}

#[test]
fn generate_room_password_legacy_compatible_matches_expected_shape() {
    let password = generate_room_password_legacy_compatible();
    assert!(
        is_legacy_generated_room_password_shape(&password),
        "generated password should match legacy shape AA-999-999"
    );
}

#[test]
fn legacy_syncplay_ui_settings_from_stored_settings_uses_python_defaults_and_supported_overrides() {
    assert_eq!(
        legacy_syncplay_ui_settings_from_stored_settings(None),
        LegacySyncplayUiSettings::default()
    );

    let resolved =
        legacy_syncplay_ui_settings_from_stored_settings(Some(&StoredClientSettingsMvp {
            show_osd: Some(false),
            chat_input_enabled: Some(false),
            chat_input_font_family: Some("serif".to_owned()),
            chat_input_relative_font_size: Some(18),
            chat_input_font_weight: Some(50),
            chat_input_font_color: Some("#abcdef".to_owned()),
            chat_input_position: Some("Bottom".to_owned()),
            chat_output_enabled: Some(false),
            chat_output_font_family: Some("monospace".to_owned()),
            chat_output_relative_font_size: Some(30),
            chat_output_font_weight: Some(75),
            chat_output_mode: Some("Scrolling".to_owned()),
            chat_move_osd: Some(false),
            chat_max_lines: Some(9),
            chat_top_margin: Some(40),
            chat_left_margin: Some(35),
            chat_bottom_margin: Some(45),
            chat_osd_margin: Some(220),
            notification_timeout_seconds: Some(4),
            alert_timeout_seconds: Some(6),
            chat_timeout_seconds: Some(9),
            ..StoredClientSettingsMvp::default()
        }));

    assert_eq!(
        resolved,
        LegacySyncplayUiSettings {
            show_osd: false,
            chat_output_enabled: false,
            chat_input_enabled: false,
            chat_input_font_family: "serif".to_owned(),
            chat_input_relative_font_size: 18,
            chat_input_font_weight: 50,
            chat_input_font_color: "#abcdef".to_owned(),
            chat_input_position: "Bottom".to_owned(),
            chat_output_font_family: "monospace".to_owned(),
            chat_output_relative_font_size: 30,
            chat_output_font_weight: 75,
            chat_output_mode: "Scrolling".to_owned(),
            chat_move_osd: false,
            chat_max_lines: 9,
            chat_top_margin: 40,
            chat_left_margin: 35,
            chat_bottom_margin: 45,
            chat_osd_margin: 220,
            notification_timeout_ms: 4_000,
            alert_timeout_ms: 6_000,
            chat_timeout_ms: 9_000,
            ..LegacySyncplayUiSettings::default()
        }
    );
}

#[test]
fn create_client_runtime_with_managed_mpv_support_applies_legacy_syncplay_ui_settings() {
    let config = test_client_loop_config();
    let settings = StoredClientSettingsMvp {
        show_osd: Some(false),
        chat_output_enabled: Some(false),
        chat_input_enabled: Some(false),
        chat_input_position: Some("Bottom".to_owned()),
        chat_output_mode: Some("Scrolling".to_owned()),
        chat_move_osd: Some(false),
        chat_osd_margin: Some(180),
        notification_timeout_seconds: Some(2),
        alert_timeout_seconds: Some(4),
        chat_timeout_seconds: Some(8),
        ..StoredClientSettingsMvp::default()
    };

    let (runtime, _managed_guard) =
        create_client_runtime_with_managed_mpv_support(&config, None, Some(&settings))
            .expect("runtime creation should succeed");

    assert_eq!(
        runtime.player().legacy_syncplay_ui_settings(),
        &LegacySyncplayUiSettings {
            show_osd: false,
            chat_output_enabled: false,
            chat_input_enabled: false,
            chat_input_position: "Bottom".to_owned(),
            chat_output_mode: "Scrolling".to_owned(),
            chat_move_osd: false,
            chat_osd_margin: 180,
            notification_timeout_ms: 2_000,
            alert_timeout_ms: 4_000,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        }
    );
}

#[test]
fn create_client_runtime_applies_autoplay_require_same_filenames_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: true,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(
        runtime
            .session()
            .readiness_autoplay_config()
            .autoplay_require_same_filenames
    );
}

#[test]
fn create_client_runtime_applies_duration_comparison_override_flags() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: Some(false),
        different_duration_threshold_seconds_override: Some(1.0),
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    let readiness_config = runtime.session().readiness_autoplay_config();
    assert!(!readiness_config.show_duration_notification);
    assert_eq!(readiness_config.different_duration_threshold_seconds, 1.0);
}

#[test]
fn create_client_runtime_applies_show_same_room_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: Some(false),
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(!runtime.session().behavior_config().show_same_room_osd);
}

#[test]
fn create_client_runtime_applies_show_noncontroller_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: Some(true),
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(runtime.session().behavior_config().show_noncontroller_osd);
}

#[test]
fn create_client_runtime_applies_show_osd_warnings_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: Some(false),
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(!runtime.session().behavior_config().show_osd_warnings);
}

#[test]
fn create_client_runtime_applies_show_different_room_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: Some(true),
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(runtime.session().behavior_config().show_different_room_osd);
}
