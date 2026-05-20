use syncplay_client_core::PrivacyMode;

use crate::legacy_ini_serde::{
    parse_serialized_per_player_arguments_map_legacy_compatible,
    parse_serialized_public_servers_list_legacy_compatible,
    parse_serialized_string_list_legacy_compatible,
};
use crate::legacy_language::normalized_legacy_runtime_language_tag_legacy_compatible;
use crate::legacy_settings::{
    StoredClientSettingsMvp, parse_autoplay_min_users_override_legacy_compatible,
    parse_unpause_action_mode_legacy_compatible,
};

use super::helpers::{
    parse_ini_bool_legacy_compatible, parse_ini_i64_legacy_compatible,
    parse_ini_non_negative_f64_legacy_compatible, parse_ini_port_legacy_compatible,
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
                        settings.media_search_directories =
                            Some(normalize_media_search_directories(parsed));
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
            Some("plex") => match key.as_str() {
                "syncenabled" => {
                    if let Some(parsed) = parse_ini_bool_legacy_compatible(&value) {
                        settings.plex_sync_enabled = Some(parsed);
                    }
                }
                "usertoken" if !value.is_empty() => {
                    settings.plex_user_token = Some(value);
                }
                "selectedserverid" if !value.is_empty() => {
                    settings.plex_selected_server_id = Some(value);
                }
                "selectedserverurl" if !value.is_empty() => {
                    settings.plex_selected_server_url = Some(value);
                }
                "selectedservertoken" if !value.is_empty() => {
                    settings.plex_selected_server_token = Some(value);
                }
                _ => {}
            },
            _ => {}
        }
    }
    settings
}

fn normalize_media_search_directories(directories: Vec<String>) -> Vec<String> {
    directories
        .into_iter()
        .filter_map(|directory| {
            let directory = directory.trim();
            (!directory.is_empty()).then(|| directory.to_owned())
        })
        .collect()
}
