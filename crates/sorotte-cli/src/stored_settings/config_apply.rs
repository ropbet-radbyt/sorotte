use sorotte_client_app::app_boundary::state::{
    ServerPort, parse_host_and_optional_port_from_host_arg, stored_client_settings_runtime_snapshot,
};

use crate::env_support::parse_env_port;

use super::*;

pub(crate) fn apply_stored_client_settings(
    config: &mut ClientLoopConfig,
    settings: &StoredClientSettings,
    env_value: impl Fn(&str) -> Option<String>,
) {
    let resolved = stored_client_settings_runtime_snapshot(settings);
    let runtime = resolved.config;

    if env_value("SOROTTE_CLIENT_HOST").is_none()
        && let Some(host) = runtime.connection.host
    {
        config.host = host;
    }
    // An invalid explicit/embedded stored port must not fall through to a
    // different stored port. An invalid environment port does not shadow it.
    let embedded_port = settings
        .host
        .as_deref()
        .and_then(|host| parse_host_and_optional_port_from_host_arg(host).1);
    if env_value("SOROTTE_CLIENT_PORT")
        .and_then(|value| parse_env_port(&value))
        .is_none()
        && let Some(port) = settings
            .port
            .or(embedded_port)
            .or(resolved.settings.port)
            .and_then(|port| ServerPort::new(port).ok())
    {
        config.port = port.get();
    }
    if env_value("SOROTTE_CLIENT_SERVER_PASSWORD").is_none()
        && let Some(password) = runtime.connection.server_password
    {
        config.server_password = Some(password);
    }
    if env_value("SOROTTE_CLIENT_USERNAME").is_none()
        && env_value("SOROTTE_CLIENT_NAME").is_none()
        && let Some(username) = runtime.connection.username
    {
        config.username = username.as_str().to_owned();
    }
    if env_value("SOROTTE_CLIENT_ROOM").is_none()
        && let Some(room) = runtime.connection.room
    {
        config.room = room.as_str().to_owned();
        if config.controlled_room_password_override.is_none() {
            config.controlled_room_password_override = runtime.connection.controlled_room_password;
        }
    }

    // Only explicit stored overrides apply; effective defaults must not erase
    // an existing CLI value. Nonempty environment input owns its field even
    // when that input was malformed (the port exception is handled above).
    if env_value("SOROTTE_CLIENT_AUTOPLAY").is_none() && settings.autoplay_initial_state.is_some() {
        config.autoplay_enabled = runtime.readiness.autoplay_initial_state;
    }
    if env_value("SOROTTE_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES").is_none()
        && settings.autoplay_require_same_filenames.is_some()
    {
        config.autoplay_require_same_filenames = runtime.readiness.autoplay_require_same_filenames;
    }
    if env_value("SOROTTE_CLIENT_READY_AT_START").is_none() && settings.ready_at_start.is_some() {
        config.ready_at_start_override = Some(runtime.readiness.ready_at_start);
    }
    if env_value("SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED").is_none()
        && settings.shared_playlist_enabled.is_some()
    {
        config.shared_playlists_enabled_override = Some(runtime.playback.shared_playlist_enabled);
    }
    if env_value("SOROTTE_CLIENT_PAUSE_ON_LEAVE").is_none() && settings.pause_on_leave.is_some() {
        config.pause_on_leave_override = Some(runtime.playback.pause_on_leave);
    }
    if env_value("SOROTTE_CLIENT_LOOP_AT_END_OF_PLAYLIST").is_none()
        && settings.loop_at_end_of_playlist.is_some()
    {
        config.loop_at_end_of_playlist_override = Some(runtime.playback.loop_at_end_of_playlist);
    }
    if env_value("SOROTTE_CLIENT_LOOP_SINGLE_FILES").is_none()
        && settings.loop_single_files.is_some()
    {
        config.loop_single_files_override = Some(runtime.playback.loop_single_files);
    }
    if env_value("SOROTTE_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS").is_none()
        && settings.only_switch_to_trusted_domains.is_some()
    {
        config.only_switch_to_trusted_domains_override =
            Some(runtime.playback.only_switch_to_trusted_domains);
    }
    if env_value("SOROTTE_CLIENT_TRUSTED_DOMAINS").is_none() && settings.trusted_domains.is_some() {
        config.trusted_domains_override = Some(runtime.playback.trusted_domains.clone());
    }
    if env_value("SOROTTE_CLIENT_REWIND_ON_DESYNC").is_none() && settings.rewind_on_desync.is_some()
    {
        config.rewind_on_desync_override = Some(runtime.synchronization.rewind_on_desync);
    }
    if env_value("SOROTTE_CLIENT_FASTFORWARD_ON_DESYNC").is_none()
        && settings.fastforward_on_desync.is_some()
    {
        config.fastforward_on_desync_override = Some(runtime.synchronization.fastforward_on_desync);
    }
    if env_value("SOROTTE_CLIENT_SLOW_ON_DESYNC").is_none() && settings.slow_on_desync.is_some() {
        config.slow_on_desync_override = Some(runtime.synchronization.slow_on_desync);
    }
    if env_value("SOROTTE_CLIENT_DONT_SLOW_DOWN_WITH_ME").is_none()
        && settings.dont_slow_down_with_me.is_some()
    {
        config.dont_slow_down_with_me_override =
            Some(runtime.synchronization.dont_slow_down_with_me);
    }
    if env_value("SOROTTE_CLIENT_REWIND_THRESHOLD_SECONDS").is_none()
        && settings.rewind_threshold_seconds.is_some()
    {
        config.rewind_threshold_seconds_override =
            Some(runtime.synchronization.rewind_threshold.get());
    }
    if env_value("SOROTTE_CLIENT_FASTFORWARD_THRESHOLD_SECONDS").is_none()
        && settings.fastforward_threshold_seconds.is_some()
    {
        config.fastforward_threshold_seconds_override =
            Some(runtime.synchronization.fastforward_threshold.get());
    }
    if env_value("SOROTTE_CLIENT_SLOWDOWN_THRESHOLD_SECONDS").is_none()
        && settings.slowdown_threshold_seconds.is_some()
    {
        config.slowdown_threshold_seconds_override =
            Some(runtime.synchronization.slowdown_threshold.get());
    }
    if env_value("SOROTTE_CLIENT_UNPAUSE_ACTION").is_none() && settings.unpause_action.is_some() {
        config.unpause_action_override = Some(runtime.readiness.unpause_action.clone());
    }
    if env_value("SOROTTE_CLIENT_AUTOPLAY_MIN_USERS").is_none()
        && settings.autoplay_min_users.is_some()
    {
        config.auto_play_threshold_override = Some(runtime.readiness.autoplay_min_users.clone());
    }
    if env_value("SOROTTE_CLIENT_FILENAME_PRIVACY_MODE").is_none()
        && settings.filename_privacy_mode.is_some()
    {
        config.filename_privacy_mode = runtime.playback.filename_privacy_mode;
    }
    if env_value("SOROTTE_CLIENT_FILESIZE_PRIVACY_MODE").is_none()
        && settings.filesize_privacy_mode.is_some()
    {
        config.filesize_privacy_mode = runtime.playback.filesize_privacy_mode;
    }
    if env_value("SOROTTE_CLIENT_SHOW_DURATION_NOTIFICATION").is_none()
        && settings.show_duration_notification.is_some()
    {
        config.show_duration_notification_override =
            Some(runtime.readiness.show_duration_notification);
    }
    if env_value("SOROTTE_CLIENT_SHOW_SAME_ROOM_OSD").is_none()
        && settings.show_same_room_osd.is_some()
    {
        config.show_same_room_osd_override = Some(runtime.interface.show_same_room_osd);
    }
    if env_value("SOROTTE_CLIENT_SHOW_OSD_WARNINGS").is_none()
        && settings.show_osd_warnings.is_some()
    {
        config.show_osd_warnings_override = Some(runtime.interface.show_osd_warnings);
    }
    if env_value("SOROTTE_CLIENT_SHOW_NONCONTROLLER_OSD").is_none()
        && settings.show_noncontroller_osd.is_some()
    {
        config.show_noncontroller_osd_override = Some(runtime.interface.show_noncontroller_osd);
    }
    if env_value("SOROTTE_CLIENT_SHOW_DIFFERENT_ROOM_OSD").is_none()
        && settings.show_different_room_osd.is_some()
    {
        config.show_different_room_osd_override = Some(runtime.interface.show_different_room_osd);
    }
}

#[cfg(test)]
mod configuration_composition_tests;
#[cfg(test)]
mod controlled_room_configuration_tests;
#[cfg(test)]
mod tests;
