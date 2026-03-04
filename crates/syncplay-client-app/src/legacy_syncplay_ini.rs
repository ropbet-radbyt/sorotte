use std::path::Path;

use anyhow::anyhow;
use syncplay_client_core::PrivacyMode;

use crate::legacy_ini_serde::{
    format_serialized_per_player_arguments_map_legacy_compatible,
    format_serialized_public_servers_list_legacy_compatible,
    format_serialized_string_list_legacy_compatible,
    parse_serialized_per_player_arguments_map_legacy_compatible,
    parse_serialized_public_servers_list_legacy_compatible,
    parse_serialized_string_list_legacy_compatible,
};
use crate::legacy_language::normalized_legacy_runtime_language_tag_legacy_compatible;
use crate::legacy_settings::{
    StoredClientSettingsMvp, autoplay_threshold_override_legacy_value_compatible,
    parse_autoplay_min_users_override_legacy_compatible,
    parse_unpause_action_mode_legacy_compatible, privacy_mode_legacy_name_compatible,
    unpause_action_mode_legacy_name_compatible,
};

pub fn parse_syncplay_ini_stored_client_settings_mvp(contents: &str) -> StoredClientSettingsMvp {
    let mut settings = StoredClientSettingsMvp::default();
    let mut current_section: Option<String> = None;
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(section_name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current_section = Some(section_name.trim().to_ascii_lowercase());
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let value = raw_value.trim().replace("%%", "%");
        match current_section.as_deref() {
            Some("general") => match key.as_str() {
                "language" if !value.is_empty() => {
                    settings.language =
                        normalized_legacy_runtime_language_tag_legacy_compatible(&value)
                            .map(ToOwned::to_owned)
                }
                "checkforupdatesautomatically" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.check_for_updates_automatically = Some(parsed);
                    }
                }
                "lastcheckedforupdates" if !value.is_empty() => {
                    settings.last_checked_for_updates = Some(value)
                }
                _ => {}
            },
            Some("server_data") => match key.as_str() {
                "host" if !value.is_empty() => settings.host = Some(value),
                "port" => {
                    if let Some(port) = parse_ini_port_legacy_compatible(&value) {
                        settings.port = Some(port);
                    }
                }
                "password" if !value.is_empty() => settings.server_password = Some(value),
                _ => {}
            },
            Some("client_settings") => match key.as_str() {
                "name" if !value.is_empty() => settings.username = Some(value),
                "room" if !value.is_empty() => settings.room = Some(value),
                "roomlist" => {
                    if let Some(parsed) = parse_serialized_string_list_legacy_compatible(&value) {
                        settings.room_list = Some(parsed);
                    }
                }
                "playerpath" if !value.is_empty() => settings.player_path = Some(value),
                "perplayerarguments" => {
                    if let Some(parsed) =
                        parse_serialized_per_player_arguments_map_legacy_compatible(&value)
                    {
                        settings.per_player_arguments = Some(parsed);
                    }
                }
                "mediasearchdirectories" => {
                    if let Some(parsed) = parse_serialized_string_list_legacy_compatible(&value) {
                        settings.media_search_directories = Some(parsed);
                    }
                }
                "publicservers" => {
                    if let Some(parsed) =
                        parse_serialized_public_servers_list_legacy_compatible(&value)
                    {
                        settings.public_servers = Some(parsed);
                    }
                }
                "foldersearchfirstfiletimeout" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.folder_search_first_file_timeout_seconds = Some(parsed);
                    }
                }
                "foldersearchtimeout" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.folder_search_timeout_seconds = Some(parsed);
                    }
                }
                "foldersearchdoublecheckinterval" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.folder_search_double_check_interval_seconds = Some(parsed);
                    }
                }
                "foldersearchwarningthreshold" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.folder_search_warning_threshold_seconds = Some(parsed);
                    }
                }
                "forceguiprompt" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.force_gui_prompt = Some(parsed);
                    }
                }
                "autoplayinitialstate" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.autoplay_initial_state = Some(parsed);
                    }
                }
                "autoplayrequiresamefilenames" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.autoplay_require_same_filenames = Some(parsed);
                    }
                }
                "readyatstart" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.ready_at_start = Some(parsed);
                    }
                }
                "sharedplaylistenabled" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.shared_playlist_enabled = Some(parsed);
                    }
                }
                "pauseonleave" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.pause_on_leave = Some(parsed);
                    }
                }
                "loopatendofplaylist" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.loop_at_end_of_playlist = Some(parsed);
                    }
                }
                "loopsinglefiles" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.loop_single_files = Some(parsed);
                    }
                }
                "onlyswitchtotrusteddomains" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.only_switch_to_trusted_domains = Some(parsed);
                    }
                }
                "trusteddomains" => {
                    if let Some(parsed) = parse_serialized_string_list_legacy_compatible(&value) {
                        settings.trusted_domains = Some(parsed);
                    }
                }
                "rewindondesync" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.rewind_on_desync = Some(parsed);
                    }
                }
                "fastforwardondesync" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.fastforward_on_desync = Some(parsed);
                    }
                }
                "slowondesync" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.slow_on_desync = Some(parsed);
                    }
                }
                "dontslowdownwithme" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.dont_slow_down_with_me = Some(parsed);
                    }
                }
                "rewindthreshold" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.rewind_threshold_seconds = Some(parsed);
                    }
                }
                "fastforwardthreshold" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.fastforward_threshold_seconds = Some(parsed);
                    }
                }
                "slowdownthreshold" => {
                    if let Some(parsed) = parse_ini_non_negative_f64_legacy_compatible(&value) {
                        settings.slowdown_threshold_seconds = Some(parsed);
                    }
                }
                "unpauseaction" => {
                    if let Some(parsed) = parse_unpause_action_mode_legacy_compatible(&value) {
                        settings.unpause_action = Some(parsed);
                    }
                }
                "autoplayminusers" => {
                    if let Some(parsed) =
                        parse_autoplay_min_users_override_legacy_compatible(&value)
                    {
                        settings.autoplay_min_users = Some(parsed);
                    }
                }
                "filenameprivacymode" => {
                    if let Some(mode) = PrivacyMode::from_legacy_name(&value) {
                        settings.filename_privacy_mode = Some(mode);
                    }
                }
                "filesizeprivacymode" => {
                    if let Some(mode) = PrivacyMode::from_legacy_name(&value) {
                        settings.filesize_privacy_mode = Some(mode);
                    }
                }
                _ => {}
            },
            Some("gui") => match key.as_str() {
                "autosavejoinstolist" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.autosave_joins_to_list = Some(parsed);
                    }
                }
                "showosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_osd = Some(parsed);
                    }
                }
                "chatinputenabled" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_input_enabled = Some(parsed);
                    }
                }
                "chatinputfontunderline" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_input_font_underline = Some(parsed);
                    }
                }
                "chatinputfontfamily" if !value.is_empty() => {
                    settings.chat_input_font_family = Some(value);
                }
                "chatinputrelativefontsize" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_input_relative_font_size = Some(parsed);
                    }
                }
                "chatinputfontweight" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_input_font_weight = Some(parsed);
                    }
                }
                "chatinputfontcolor" if !value.is_empty() => {
                    settings.chat_input_font_color = Some(value);
                }
                "chatinputposition" if !value.is_empty() => {
                    settings.chat_input_position = Some(value);
                }
                "chatdirectinput" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_direct_input = Some(parsed);
                    }
                }
                "chatoutputenabled" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_output_enabled = Some(parsed);
                    }
                }
                "chatoutputfontunderline" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_output_font_underline = Some(parsed);
                    }
                }
                "chatoutputfontfamily" if !value.is_empty() => {
                    settings.chat_output_font_family = Some(value);
                }
                "chatoutputrelativefontsize" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_output_relative_font_size = Some(parsed);
                    }
                }
                "chatoutputfontweight" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_output_font_weight = Some(parsed);
                    }
                }
                "chatoutputmode" if !value.is_empty() => {
                    settings.chat_output_mode = Some(value);
                }
                "chatmoveosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.chat_move_osd = Some(parsed);
                    }
                }
                "chatmaxlines" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_max_lines = Some(parsed);
                    }
                }
                "chattopmargin" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_top_margin = Some(parsed);
                    }
                }
                "chatleftmargin" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_left_margin = Some(parsed);
                    }
                }
                "chatbottommargin" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_bottom_margin = Some(parsed);
                    }
                }
                "chatosdmargin" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_osd_margin = Some(parsed);
                    }
                }
                "notificationtimeout" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.notification_timeout_seconds = Some(parsed);
                    }
                }
                "alerttimeout" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.alert_timeout_seconds = Some(parsed);
                    }
                }
                "chattimeout" => {
                    if let Some(parsed) = parse_ini_i64_legacy_compatible(&value) {
                        settings.chat_timeout_seconds = Some(parsed);
                    }
                }
                "showdurationnotification" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_duration_notification = Some(parsed);
                    }
                }
                "showsameroomosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_same_room_osd = Some(parsed);
                    }
                }
                "showosdwarnings" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_osd_warnings = Some(parsed);
                    }
                }
                "showslowdownosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_slowdown_osd = Some(parsed);
                    }
                }
                "shownoncontrollerosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_noncontroller_osd = Some(parsed);
                    }
                }
                "showdifferentroomosd" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_different_room_osd = Some(parsed);
                    }
                }
                "showcontactinfo" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.show_contact_info = Some(parsed);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    settings
}

pub fn load_syncplay_ini_stored_client_settings_mvp_from_path(
    path: &Path,
) -> anyhow::Result<Option<StoredClientSettingsMvp>> {
    if !path.is_file() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("failed reading stored settings {}: {error}", path.display()))?;
    Ok(Some(parse_syncplay_ini_stored_client_settings_mvp(
        &contents,
    )))
}

pub fn upsert_syncplay_ini_stored_client_settings_mvp(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
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

fn parse_ini_bool_legacy_compatible(value: &str) -> Option<bool> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized == "1"
        || normalized.eq_ignore_ascii_case("true")
        || normalized.eq_ignore_ascii_case("yes")
        || normalized.eq_ignore_ascii_case("on")
    {
        return Some(true);
    }
    if normalized == "0"
        || normalized.eq_ignore_ascii_case("false")
        || normalized.eq_ignore_ascii_case("no")
        || normalized.eq_ignore_ascii_case("off")
    {
        return Some(false);
    }
    None
}

fn parse_ini_port_legacy_compatible(value: &str) -> Option<u16> {
    let port = value.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

fn parse_ini_non_negative_f64_legacy_compatible(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn escape_syncplay_ini_value_legacy_compatible(value: &str) -> String {
    value.replace('%', "%%")
}

fn format_ini_bool_legacy_compatible(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn format_ini_non_negative_f64_legacy_compatible(value: f64) -> Option<String> {
    (value.is_finite() && value >= 0.0).then(|| value.to_string())
}

fn parse_ini_i64_legacy_compatible(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

fn upsert_ini_value_legacy_compatible(
    lines: &mut Vec<String>,
    section: &str,
    key: &str,
    value: &str,
) {
    let section_header = format!("[{section}]");
    let mut section_start = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().eq_ignore_ascii_case(&section_header) {
            section_start = Some(idx);
            break;
        }
    }

    let rendered = format!(
        "{key} = {}",
        escape_syncplay_ini_value_legacy_compatible(value)
    );

    if let Some(section_start_idx) = section_start {
        let mut insert_at = lines.len();
        let mut key_index = None;
        for (idx, line) in lines.iter().enumerate().skip(section_start_idx + 1) {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                insert_at = idx;
                break;
            }
            if let Some((candidate_key, _)) = trimmed.split_once('=')
                && candidate_key.trim().eq_ignore_ascii_case(key)
            {
                key_index = Some(idx);
                break;
            }
        }
        if let Some(idx) = key_index {
            lines[idx] = rendered;
        } else {
            lines.insert(insert_at, rendered);
        }
        return;
    }

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.push(String::new());
    }
    lines.push(section_header);
    lines.push(rendered);
}

pub fn upsert_syncplay_ini_stored_client_settings_mvp_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            anyhow!(
                "failed creating stored settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let existing_contents = if path.is_file() {
        std::fs::read_to_string(path).map_err(|error| {
            anyhow!("failed reading stored settings {}: {error}", path.display())
        })?
    } else {
        String::new()
    };
    let updated_contents =
        upsert_syncplay_ini_stored_client_settings_mvp(&existing_contents, settings);
    std::fs::write(path, updated_contents)
        .map_err(|error| anyhow!("failed writing stored settings {}: {error}", path.display()))
}

pub fn update_syncplay_ini_stored_client_settings_mvp_at_path<F>(
    path: &Path,
    update: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&mut StoredClientSettingsMvp),
{
    let mut settings =
        load_syncplay_ini_stored_client_settings_mvp_from_path(path)?.unwrap_or_default();
    update(&mut settings);
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(path, &settings)
}

pub fn clear_syncplay_ini_stored_client_settings_mvp_at_path(path: &Path) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(anyhow!(
            "stored settings path is not a file and cannot be cleared: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path).map_err(|error| {
        anyhow!(
            "failed clearing stored settings {}: {error}",
            path.display()
        )
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        clear_syncplay_ini_stored_client_settings_mvp_at_path,
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        parse_syncplay_ini_stored_client_settings_mvp,
        update_syncplay_ini_stored_client_settings_mvp_at_path,
        upsert_syncplay_ini_stored_client_settings_mvp,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    };
    use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};

    fn unique_temp_syncplay_ini_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("syncplay-client-app-{label}-{unique}"))
            .join("syncplay.ini")
    }

    #[test]
    fn parse_syncplay_ini_stored_client_settings_mvp_normalizes_and_reads_known_sections() {
        let settings = parse_syncplay_ini_stored_client_settings_mvp(
            "[general]\n\
             language = PT-br\n\
             [server_data]\n\
             port = 8999\n\
             [client_settings]\n\
             autoplayMinUsers = 3\n\
             [gui]\n\
             chatInputRelativeFontSize = 2\n",
        );

        assert_eq!(settings.language.as_deref(), Some("pt_BR"));
        assert_eq!(settings.port, Some(8999));
        assert_eq!(
            settings.autoplay_min_users,
            Some(AutoplayThresholdOverride::Set(3))
        );
        assert_eq!(settings.chat_input_relative_font_size, Some(2));
    }

    #[test]
    fn upsert_syncplay_ini_stored_client_settings_mvp_preserves_existing_entries() {
        let updated = upsert_syncplay_ini_stored_client_settings_mvp(
            "[misc]\nfoo = bar\n[client_settings]\nname = old\n",
            &StoredClientSettingsMvp {
                username: Some("alice".to_owned()),
                ..StoredClientSettingsMvp::default()
            },
        );

        assert!(updated.contains("[misc]\nfoo = bar\n"));
        assert!(updated.contains("[client_settings]\nname = alice\n"));
    }

    #[test]
    fn path_helpers_roundtrip_settings_file_contents() {
        let path = unique_temp_syncplay_ini_path("roundtrip");
        let settings = StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("lobby".to_owned()),
            ..StoredClientSettingsMvp::default()
        };

        upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &settings)
            .expect("settings should write");
        let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("settings should load")
            .expect("settings file should exist");

        assert_eq!(loaded.username.as_deref(), Some("alice"));
        assert_eq!(loaded.room.as_deref(), Some("lobby"));

        std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
            .expect("temp test directory should be removable");
    }

    #[test]
    fn load_syncplay_ini_stored_client_settings_mvp_from_path_returns_none_for_missing_file() {
        let path = unique_temp_syncplay_ini_path("missing");

        let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("missing path should not error");

        assert_eq!(loaded, None);
    }

    #[test]
    fn update_helper_loads_mutates_and_rewrites_existing_settings() {
        let path = unique_temp_syncplay_ini_path("update");
        upsert_syncplay_ini_stored_client_settings_mvp_at_path(
            &path,
            &StoredClientSettingsMvp {
                username: Some("alice".to_owned()),
                ..StoredClientSettingsMvp::default()
            },
        )
        .expect("initial settings should write");

        update_syncplay_ini_stored_client_settings_mvp_at_path(&path, |settings| {
            settings.room = Some("lobby".to_owned());
        })
        .expect("settings should update");
        let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("settings should load")
            .expect("settings file should exist");

        assert_eq!(loaded.username.as_deref(), Some("alice"));
        assert_eq!(loaded.room.as_deref(), Some("lobby"));

        std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
            .expect("temp test directory should be removable");
    }

    #[test]
    fn clear_helper_removes_existing_settings_file() {
        let path = unique_temp_syncplay_ini_path("clear");
        upsert_syncplay_ini_stored_client_settings_mvp_at_path(
            &path,
            &StoredClientSettingsMvp {
                username: Some("alice".to_owned()),
                ..StoredClientSettingsMvp::default()
            },
        )
        .expect("initial settings should write");

        let cleared = clear_syncplay_ini_stored_client_settings_mvp_at_path(&path)
            .expect("clear should succeed");

        assert!(cleared);
        assert!(!path.exists());

        std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
            .expect("temp test directory should be removable");
    }
}
