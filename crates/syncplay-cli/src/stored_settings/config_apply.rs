use super::*;

pub(crate) fn apply_stored_client_settings_mvp_if_env_absent(
    config: &mut ClientLoopConfig,
    settings: &StoredClientSettingsMvp,
) {
    let config_plan = stored_client_settings_config_plan_legacy_compatible(
        settings,
        &StoredClientSettingsEnvPresence {
            host: env_trimmed("SYNCPLAY_CLIENT_HOST").is_some(),
            port: env_port("SYNCPLAY_CLIENT_PORT").is_some(),
            server_password: env_trimmed("SYNCPLAY_CLIENT_SERVER_PASSWORD").is_some(),
            username: env_trimmed("SYNCPLAY_CLIENT_USERNAME").is_some()
                || env_trimmed("SYNCPLAY_CLIENT_NAME").is_some(),
            room: env_trimmed("SYNCPLAY_CLIENT_ROOM").is_some(),
            autoplay: env_trimmed("SYNCPLAY_CLIENT_AUTOPLAY").is_some(),
            autoplay_require_same_filenames: env_trimmed(
                "SYNCPLAY_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES",
            )
            .is_some(),
            ready_at_start: env_trimmed("SYNCPLAY_CLIENT_READY_AT_START").is_some(),
            shared_playlist_enabled: env_trimmed("SYNCPLAY_CLIENT_SHARED_PLAYLIST_ENABLED")
                .is_some(),
            pause_on_leave: env_trimmed("SYNCPLAY_CLIENT_PAUSE_ON_LEAVE").is_some(),
            loop_at_end_of_playlist: env_trimmed("SYNCPLAY_CLIENT_LOOP_AT_END_OF_PLAYLIST")
                .is_some(),
            loop_single_files: env_trimmed("SYNCPLAY_CLIENT_LOOP_SINGLE_FILES").is_some(),
            only_switch_to_trusted_domains: env_trimmed(
                "SYNCPLAY_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS",
            )
            .is_some(),
            trusted_domains: env_trimmed("SYNCPLAY_CLIENT_TRUSTED_DOMAINS").is_some(),
            rewind_on_desync: env_trimmed("SYNCPLAY_CLIENT_REWIND_ON_DESYNC").is_some(),
            fastforward_on_desync: env_trimmed("SYNCPLAY_CLIENT_FASTFORWARD_ON_DESYNC").is_some(),
            slow_on_desync: env_trimmed("SYNCPLAY_CLIENT_SLOW_ON_DESYNC").is_some(),
            dont_slow_down_with_me: env_trimmed("SYNCPLAY_CLIENT_DONT_SLOW_DOWN_WITH_ME").is_some(),
            rewind_threshold_seconds: env_trimmed("SYNCPLAY_CLIENT_REWIND_THRESHOLD_SECONDS")
                .is_some(),
            fastforward_threshold_seconds: env_trimmed(
                "SYNCPLAY_CLIENT_FASTFORWARD_THRESHOLD_SECONDS",
            )
            .is_some(),
            slowdown_threshold_seconds: env_trimmed("SYNCPLAY_CLIENT_SLOWDOWN_THRESHOLD_SECONDS")
                .is_some(),
            unpause_action: env_trimmed("SYNCPLAY_CLIENT_UNPAUSE_ACTION").is_some(),
            autoplay_min_users: env_trimmed("SYNCPLAY_CLIENT_AUTOPLAY_MIN_USERS").is_some(),
            filename_privacy_mode: env_trimmed("SYNCPLAY_CLIENT_FILENAME_PRIVACY_MODE").is_some(),
            filesize_privacy_mode: env_trimmed("SYNCPLAY_CLIENT_FILESIZE_PRIVACY_MODE").is_some(),
            show_duration_notification: env_trimmed("SYNCPLAY_CLIENT_SHOW_DURATION_NOTIFICATION")
                .is_some(),
            show_same_room_osd: env_trimmed("SYNCPLAY_CLIENT_SHOW_SAME_ROOM_OSD").is_some(),
            show_osd_warnings: env_trimmed("SYNCPLAY_CLIENT_SHOW_OSD_WARNINGS").is_some(),
            show_noncontroller_osd: env_trimmed("SYNCPLAY_CLIENT_SHOW_NONCONTROLLER_OSD").is_some(),
            show_different_room_osd: env_trimmed("SYNCPLAY_CLIENT_SHOW_DIFFERENT_ROOM_OSD")
                .is_some(),
        },
    );

    if let Some(host) = config_plan.host {
        config.host = host;
    }
    if let Some(port) = config_plan.port {
        config.port = port;
    }
    if let Some(password) = config_plan.server_password {
        config.server_password = Some(password);
    }
    if let Some(username) = config_plan.username {
        config.username = username;
    }
    if let Some(room) = config_plan.room {
        config.room = room;
        if config.controlled_room_password_override.is_none() {
            config.controlled_room_password_override =
                config_plan.controlled_room_password_override;
        }
    }
    if let Some(value) = config_plan.autoplay_enabled {
        config.autoplay_enabled = value;
    }
    if let Some(value) = config_plan.autoplay_require_same_filenames {
        config.autoplay_require_same_filenames = value;
    }
    if let Some(value) = config_plan.ready_at_start_override {
        config.ready_at_start_override = Some(value);
    }
    if let Some(value) = config_plan.shared_playlists_enabled_override {
        config.shared_playlists_enabled_override = Some(value);
    }
    if let Some(value) = config_plan.pause_on_leave_override {
        config.pause_on_leave_override = Some(value);
    }
    if let Some(value) = config_plan.loop_at_end_of_playlist_override {
        config.loop_at_end_of_playlist_override = Some(value);
    }
    if let Some(value) = config_plan.loop_single_files_override {
        config.loop_single_files_override = Some(value);
    }
    if let Some(value) = config_plan.only_switch_to_trusted_domains_override {
        config.only_switch_to_trusted_domains_override = Some(value);
    }
    if let Some(values) = config_plan.trusted_domains_override {
        config.trusted_domains_override = Some(values);
    }
    if let Some(value) = config_plan.rewind_on_desync_override {
        config.rewind_on_desync_override = Some(value);
    }
    if let Some(value) = config_plan.fastforward_on_desync_override {
        config.fastforward_on_desync_override = Some(value);
    }
    if let Some(value) = config_plan.slow_on_desync_override {
        config.slow_on_desync_override = Some(value);
    }
    if let Some(value) = config_plan.dont_slow_down_with_me_override {
        config.dont_slow_down_with_me_override = Some(value);
    }
    if let Some(value) = config_plan.rewind_threshold_seconds_override {
        config.rewind_threshold_seconds_override = Some(value);
    }
    if let Some(value) = config_plan.fastforward_threshold_seconds_override {
        config.fastforward_threshold_seconds_override = Some(value);
    }
    if let Some(value) = config_plan.slowdown_threshold_seconds_override {
        config.slowdown_threshold_seconds_override = Some(value);
    }
    if let Some(value) = config_plan.unpause_action_override {
        config.unpause_action_override = Some(value);
    }
    if let Some(value) = config_plan.auto_play_threshold_override {
        config.auto_play_threshold_override = Some(value);
    }
    if let Some(mode) = config_plan.filename_privacy_mode {
        config.filename_privacy_mode = mode;
    }
    if let Some(mode) = config_plan.filesize_privacy_mode {
        config.filesize_privacy_mode = mode;
    }
    if let Some(value) = config_plan.show_duration_notification_override {
        config.show_duration_notification_override = Some(value);
    }
    if let Some(value) = config_plan.show_same_room_osd_override {
        config.show_same_room_osd_override = Some(value);
    }
    if let Some(value) = config_plan.show_osd_warnings_override {
        config.show_osd_warnings_override = Some(value);
    }
    if let Some(value) = config_plan.show_noncontroller_osd_override {
        config.show_noncontroller_osd_override = Some(value);
    }
    if let Some(value) = config_plan.show_different_room_osd_override {
        config.show_different_room_osd_override = Some(value);
    }
}
