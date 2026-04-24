use super::*;

pub(crate) fn normalize_controlled_room_input(room: String) -> (String, Option<String>) {
    normalize_controlled_room_input_legacy_compatible(room)
}

pub(crate) fn build_client_loop_config_from_env() -> ClientLoopConfig {
    let room = env_trimmed("SYNCPLAY_CLIENT_ROOM").unwrap_or_else(|| "cli-demo".to_owned());
    let (room, controlled_room_password_override) = normalize_controlled_room_input(room);

    ClientLoopConfig {
        host: env_trimmed("SYNCPLAY_CLIENT_HOST").unwrap_or_else(|| "127.0.0.1".to_owned()),
        port: env_port("SYNCPLAY_CLIENT_PORT").unwrap_or(8999),
        server_password: env_trimmed("SYNCPLAY_CLIENT_SERVER_PASSWORD"),
        username: env_trimmed("SYNCPLAY_CLIENT_USERNAME")
            .or_else(|| env_trimmed("SYNCPLAY_CLIENT_NAME"))
            .unwrap_or_else(|| "cli-user".to_owned()),
        room,
        version: env_trimmed("SYNCPLAY_CLIENT_VERSION").unwrap_or_else(|| "1.2.255".to_owned()),
        max_retries: env_u32("SYNCPLAY_CLIENT_MAX_RETRIES").unwrap_or(3),
        max_connected_runtime_seconds: env_non_negative_f64(
            "SYNCPLAY_CLIENT_MAX_CONNECTED_RUNTIME_SECONDS",
        )
        .unwrap_or(10.0),
        readiness_supported_override: env_flag_override("SYNCPLAY_CLIENT_READINESS_SUPPORTED"),
        local_can_control_override: env_flag_override("SYNCPLAY_CLIENT_CAN_CONTROL"),
        is_playing_music_override: env_flag_override("SYNCPLAY_CLIENT_IS_PLAYING_MUSIC"),
        recently_advanced_override: env_flag_override("SYNCPLAY_CLIENT_RECENTLY_ADVANCED"),
        autoplay_enabled: env_flag_enabled("SYNCPLAY_CLIENT_AUTOPLAY"),
        autoplay_require_same_filenames: env_flag_enabled(
            "SYNCPLAY_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES",
        ),
        ready_at_start_override: env_flag_override("SYNCPLAY_CLIENT_READY_AT_START"),
        shared_playlists_enabled_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHARED_PLAYLIST_ENABLED",
        ),
        pause_on_leave_override: env_flag_override("SYNCPLAY_CLIENT_PAUSE_ON_LEAVE"),
        loop_at_end_of_playlist_override: env_flag_override(
            "SYNCPLAY_CLIENT_LOOP_AT_END_OF_PLAYLIST",
        ),
        loop_single_files_override: env_flag_override("SYNCPLAY_CLIENT_LOOP_SINGLE_FILES"),
        only_switch_to_trusted_domains_override: env_flag_override(
            "SYNCPLAY_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS",
        ),
        trusted_domains_override: env_string_list("SYNCPLAY_CLIENT_TRUSTED_DOMAINS"),
        rewind_on_desync_override: env_flag_override("SYNCPLAY_CLIENT_REWIND_ON_DESYNC"),
        fastforward_on_desync_override: env_flag_override("SYNCPLAY_CLIENT_FASTFORWARD_ON_DESYNC"),
        slow_on_desync_override: env_flag_override("SYNCPLAY_CLIENT_SLOW_ON_DESYNC"),
        dont_slow_down_with_me_override: env_flag_override(
            "SYNCPLAY_CLIENT_DONT_SLOW_DOWN_WITH_ME",
        ),
        rewind_threshold_seconds_override: env_non_negative_f64(
            "SYNCPLAY_CLIENT_REWIND_THRESHOLD_SECONDS",
        ),
        fastforward_threshold_seconds_override: env_non_negative_f64(
            "SYNCPLAY_CLIENT_FASTFORWARD_THRESHOLD_SECONDS",
        ),
        slowdown_threshold_seconds_override: env_non_negative_f64(
            "SYNCPLAY_CLIENT_SLOWDOWN_THRESHOLD_SECONDS",
        ),
        unpause_action_override: env_trimmed("SYNCPLAY_CLIENT_UNPAUSE_ACTION")
            .and_then(|value| parse_unpause_action_mode_legacy_compatible(&value)),
        auto_play_threshold_override: env_trimmed("SYNCPLAY_CLIENT_AUTOPLAY_MIN_USERS")
            .and_then(|value| parse_autoplay_min_users_override_legacy_compatible(&value)),
        filename_privacy_mode: env_privacy_mode("SYNCPLAY_CLIENT_FILENAME_PRIVACY_MODE")
            .unwrap_or(PrivacyMode::SendRaw),
        filesize_privacy_mode: env_privacy_mode("SYNCPLAY_CLIENT_FILESIZE_PRIVACY_MODE")
            .unwrap_or(PrivacyMode::SendRaw),
        show_duration_notification_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_DURATION_NOTIFICATION",
        ),
        different_duration_threshold_seconds_override: env_non_negative_f64(
            "SYNCPLAY_CLIENT_DIFFERENT_DURATION_THRESHOLD_SECONDS",
        ),
        show_same_room_osd_override: env_flag_override("SYNCPLAY_CLIENT_SHOW_SAME_ROOM_OSD"),
        show_osd_warnings_override: env_flag_override("SYNCPLAY_CLIENT_SHOW_OSD_WARNINGS"),
        show_noncontroller_osd_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_NONCONTROLLER_OSD",
        ),
        show_different_room_osd_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_DIFFERENT_ROOM_OSD",
        ),
        controlled_room_password_override,
    }
}
