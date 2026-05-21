use crate::legacy_ini_serde::{
    format_serialized_per_player_arguments_map_legacy_compatible,
    format_serialized_public_servers_list_legacy_compatible,
    format_serialized_string_list_legacy_compatible,
};
use crate::legacy_settings::{
    StoredClientSettingsMvp, autoplay_threshold_override_legacy_value_compatible,
    privacy_mode_legacy_name_compatible, unpause_action_mode_legacy_name_compatible,
};

use super::helpers::{
    format_ini_bool_legacy_compatible, format_ini_non_negative_f64_legacy_compatible,
    remove_ini_value_legacy_compatible, upsert_ini_value_legacy_compatible,
};

pub fn upsert_syncplay_ini_stored_client_settings_mvp(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
) -> String {
    upsert_syncplay_ini_stored_client_settings_mvp_with_plex_identity_clear(
        existing_contents,
        settings,
        false,
    )
}

pub fn upsert_syncplay_ini_stored_client_settings_mvp_clearing_plex_identity(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
) -> String {
    upsert_syncplay_ini_stored_client_settings_mvp_with_plex_identity_clear(
        existing_contents,
        settings,
        true,
    )
}

fn upsert_syncplay_ini_stored_client_settings_mvp_with_plex_identity_clear(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
    clear_plex_identity: bool,
) -> String {
    let had_bom = existing_contents.starts_with('\u{feff}');
    let mut lines = existing_contents
        .strip_prefix('\u{feff}')
        .unwrap_or(existing_contents)
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if let Some(host) = settings.host.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "server_data", "host", host);
    }
    if let Some(port) = settings.port {
        upsert_ini_value_legacy_compatible(&mut lines, "server_data", "port", &port.to_string());
    }
    if let Some(server_password) = settings.server_password.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "server_data", "password", server_password);
    }
    if let Some(username) = settings.username.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "client_settings", "name", username);
    }
    if let Some(room) = settings.room.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "client_settings", "room", room);
    }
    if let Some(room_list) = settings.room_list.as_ref() {
        let serialized = format_serialized_string_list_legacy_compatible(room_list);
        upsert_ini_value_legacy_compatible(&mut lines, "client_settings", "roomList", &serialized);
    }
    if let Some(player_path) = settings.player_path.as_deref() {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "playerPath",
            player_path,
        );
    }
    if let Some(per_player_arguments) = settings.per_player_arguments.as_ref() {
        let serialized =
            format_serialized_per_player_arguments_map_legacy_compatible(per_player_arguments);
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "perPlayerArguments",
            &serialized,
        );
    }
    if let Some(media_search_directories) = settings.media_search_directories.as_ref() {
        let serialized = format_serialized_string_list_legacy_compatible(media_search_directories);
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "mediaSearchDirectories",
            &serialized,
        );
    }
    if let Some(public_servers) = settings.public_servers.as_ref() {
        let serialized = format_serialized_public_servers_list_legacy_compatible(public_servers);
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "publicServers",
            &serialized,
        );
    }
    if let Some(value) = settings.plex_sync_enabled {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "plex",
            "syncEnabled",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if clear_plex_identity {
        remove_ini_value_legacy_compatible(&mut lines, "plex", "userToken");
        remove_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerId");
        remove_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerUrl");
        remove_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerToken");
    }
    if let Some(value) = settings.plex_user_token.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "plex", "userToken", value);
    }
    if let Some(value) = settings.plex_selected_server_id.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerId", value);
    }
    if let Some(value) = settings.plex_selected_server_url.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerUrl", value);
    }
    if let Some(value) = settings.plex_selected_server_token.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "plex", "selectedServerToken", value);
    }
    if let Some(value) = settings.folder_search_first_file_timeout_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "folderSearchFirstFileTimeout",
            &formatted,
        );
    }
    if let Some(value) = settings.folder_search_timeout_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "folderSearchTimeout",
            &formatted,
        );
    }
    if let Some(value) = settings.folder_search_double_check_interval_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "folderSearchDoubleCheckInterval",
            &formatted,
        );
    }
    if let Some(value) = settings.folder_search_warning_threshold_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "folderSearchWarningThreshold",
            &formatted,
        );
    }
    if let Some(value) = settings.force_gui_prompt {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "forceGuiPrompt",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.autoplay_initial_state {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "autoplayInitialState",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.autoplay_require_same_filenames {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "autoplayRequireSameFilenames",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.ready_at_start {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "readyAtStart",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.shared_playlist_enabled {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "sharedPlaylistEnabled",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.pause_on_leave {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "pauseOnLeave",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.loop_at_end_of_playlist {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "loopAtEndOfPlaylist",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.loop_single_files {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "loopSingleFiles",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.only_switch_to_trusted_domains {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "onlySwitchToTrustedDomains",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(trusted_domains) = settings.trusted_domains.as_ref() {
        let serialized = format_serialized_string_list_legacy_compatible(trusted_domains);
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "trustedDomains",
            &serialized,
        );
    }
    if let Some(value) = settings.rewind_on_desync {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "rewindOnDesync",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.fastforward_on_desync {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "fastforwardOnDesync",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.slow_on_desync {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "slowOnDesync",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.dont_slow_down_with_me {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "dontSlowDownWithMe",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.rewind_threshold_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "rewindThreshold",
            &formatted,
        );
    }
    if let Some(value) = settings.fastforward_threshold_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "fastforwardThreshold",
            &formatted,
        );
    }
    if let Some(value) = settings.slowdown_threshold_seconds
        && let Some(formatted) = format_ini_non_negative_f64_legacy_compatible(value)
    {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "slowdownThreshold",
            &formatted,
        );
    }
    if let Some(unpause_action) = settings.unpause_action.as_ref() {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "unpauseAction",
            unpause_action_mode_legacy_name_compatible(unpause_action.clone()),
        );
    }
    if let Some(autoplay_min_users) = settings.autoplay_min_users.as_ref() {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "autoplayMinUsers",
            &autoplay_threshold_override_legacy_value_compatible(autoplay_min_users),
        );
    }
    if let Some(mode) = settings.filename_privacy_mode {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "filenamePrivacyMode",
            privacy_mode_legacy_name_compatible(mode),
        );
    }
    if let Some(mode) = settings.filesize_privacy_mode {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "client_settings",
            "filesizePrivacyMode",
            privacy_mode_legacy_name_compatible(mode),
        );
    }
    if let Some(language) = settings.language.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "general", "language", language);
    }
    if let Some(value) = settings.check_for_updates_automatically {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "general",
            "checkForUpdatesAutomatically",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.update_channel.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "general", "updateChannel", value);
    }
    if let Some(value) = settings.last_checked_for_updates.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "general", "lastCheckedForUpdates", value);
    }
    if let Some(value) = settings.autosave_joins_to_list {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "autosaveJoinsToList",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_input_enabled {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatInputEnabled",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_input_font_underline {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatInputFontUnderline",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_input_font_family.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatInputFontFamily", value);
    }
    if let Some(value) = settings.chat_input_relative_font_size {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatInputRelativeFontSize",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.chat_input_font_weight {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatInputFontWeight",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.chat_input_font_color.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatInputFontColor", value);
    }
    if let Some(value) = settings.chat_input_position.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatInputPosition", value);
    }
    if let Some(value) = settings.chat_direct_input {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatDirectInput",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_output_enabled {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatOutputEnabled",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_output_font_underline {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatOutputFontUnderline",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_output_font_family.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatOutputFontFamily", value);
    }
    if let Some(value) = settings.chat_output_relative_font_size {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatOutputRelativeFontSize",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.chat_output_font_weight {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatOutputFontWeight",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.chat_output_mode.as_deref() {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatOutputMode", value);
    }
    if let Some(value) = settings.chat_move_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatMoveOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.chat_max_lines {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatMaxLines", &value.to_string());
    }
    if let Some(value) = settings.chat_top_margin {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatTopMargin", &value.to_string());
    }
    if let Some(value) = settings.chat_left_margin {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatLeftMargin", &value.to_string());
    }
    if let Some(value) = settings.chat_bottom_margin {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "chatBottomMargin",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.chat_osd_margin {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatOSDMargin", &value.to_string());
    }
    if let Some(value) = settings.notification_timeout_seconds {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "notificationTimeout",
            &value.to_string(),
        );
    }
    if let Some(value) = settings.alert_timeout_seconds {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "alertTimeout", &value.to_string());
    }
    if let Some(value) = settings.chat_timeout_seconds {
        upsert_ini_value_legacy_compatible(&mut lines, "gui", "chatTimeout", &value.to_string());
    }
    if let Some(value) = settings.show_duration_notification {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showDurationNotification",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_same_room_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showSameRoomOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_osd_warnings {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showOSDWarnings",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_slowdown_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showSlowdownOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_noncontroller_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showNonControllerOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_different_room_osd {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showDifferentRoomOSD",
            format_ini_bool_legacy_compatible(value),
        );
    }
    if let Some(value) = settings.show_contact_info {
        upsert_ini_value_legacy_compatible(
            &mut lines,
            "gui",
            "showContactInfo",
            format_ini_bool_legacy_compatible(value),
        );
    }

    let mut output = lines.join("\n");
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    if had_bom {
        format!("\u{feff}{output}")
    } else {
        output
    }
}
