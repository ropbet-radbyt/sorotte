use std::collections::BTreeMap;

use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoplayThresholdOverride {
    Disable,
    Set(usize),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredClientSettingsMvp {
    pub language: Option<String>,
    pub check_for_updates_automatically: Option<bool>,
    pub last_checked_for_updates: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub server_password: Option<String>,
    pub username: Option<String>,
    pub room: Option<String>,
    pub room_list: Option<Vec<String>>,
    pub player_path: Option<String>,
    pub per_player_arguments: Option<BTreeMap<String, Vec<String>>>,
    pub media_search_directories: Option<Vec<String>>,
    pub public_servers: Option<Vec<(String, String)>>,
    pub folder_search_first_file_timeout_seconds: Option<f64>,
    pub folder_search_timeout_seconds: Option<f64>,
    pub folder_search_double_check_interval_seconds: Option<f64>,
    pub folder_search_warning_threshold_seconds: Option<f64>,
    pub force_gui_prompt: Option<bool>,
    pub autoplay_initial_state: Option<bool>,
    pub autoplay_require_same_filenames: Option<bool>,
    pub ready_at_start: Option<bool>,
    pub shared_playlist_enabled: Option<bool>,
    pub pause_on_leave: Option<bool>,
    pub loop_at_end_of_playlist: Option<bool>,
    pub loop_single_files: Option<bool>,
    pub only_switch_to_trusted_domains: Option<bool>,
    pub trusted_domains: Option<Vec<String>>,
    pub rewind_on_desync: Option<bool>,
    pub fastforward_on_desync: Option<bool>,
    pub slow_on_desync: Option<bool>,
    pub dont_slow_down_with_me: Option<bool>,
    pub rewind_threshold_seconds: Option<f64>,
    pub fastforward_threshold_seconds: Option<f64>,
    pub slowdown_threshold_seconds: Option<f64>,
    pub unpause_action: Option<UnpauseActionMode>,
    pub autoplay_min_users: Option<AutoplayThresholdOverride>,
    pub filename_privacy_mode: Option<PrivacyMode>,
    pub filesize_privacy_mode: Option<PrivacyMode>,
    pub show_duration_notification: Option<bool>,
    pub autosave_joins_to_list: Option<bool>,
    pub show_osd: Option<bool>,
    pub chat_input_enabled: Option<bool>,
    pub chat_input_font_underline: Option<bool>,
    pub chat_input_font_family: Option<String>,
    pub chat_input_relative_font_size: Option<i64>,
    pub chat_input_font_weight: Option<i64>,
    pub chat_input_font_color: Option<String>,
    pub chat_input_position: Option<String>,
    pub chat_direct_input: Option<bool>,
    pub chat_output_enabled: Option<bool>,
    pub chat_output_font_underline: Option<bool>,
    pub chat_output_font_family: Option<String>,
    pub chat_output_relative_font_size: Option<i64>,
    pub chat_output_font_weight: Option<i64>,
    pub chat_output_mode: Option<String>,
    pub chat_move_osd: Option<bool>,
    pub chat_max_lines: Option<i64>,
    pub chat_top_margin: Option<i64>,
    pub chat_left_margin: Option<i64>,
    pub chat_bottom_margin: Option<i64>,
    pub chat_osd_margin: Option<i64>,
    pub notification_timeout_seconds: Option<i64>,
    pub alert_timeout_seconds: Option<i64>,
    pub chat_timeout_seconds: Option<i64>,
    pub show_same_room_osd: Option<bool>,
    pub show_osd_warnings: Option<bool>,
    pub show_slowdown_osd: Option<bool>,
    pub show_noncontroller_osd: Option<bool>,
    pub show_different_room_osd: Option<bool>,
    pub show_contact_info: Option<bool>,
}

pub fn privacy_mode_legacy_name_compatible(mode: PrivacyMode) -> &'static str {
    match mode {
        PrivacyMode::SendRaw => "SendRaw",
        PrivacyMode::SendHashed => "SendHashed",
        PrivacyMode::DoNotSend => "DoNotSend",
    }
}

pub fn unpause_action_mode_legacy_name_compatible(mode: UnpauseActionMode) -> &'static str {
    match mode {
        UnpauseActionMode::IfAlreadyReady => "IfAlreadyReady",
        UnpauseActionMode::IfOthersReady => "IfOthersReady",
        UnpauseActionMode::IfMinUsersReady => "IfMinUsersReady",
        UnpauseActionMode::Always => "Always",
    }
}

pub fn autoplay_threshold_override_legacy_value_compatible(
    value: &AutoplayThresholdOverride,
) -> String {
    match value {
        AutoplayThresholdOverride::Disable => "0".to_owned(),
        AutoplayThresholdOverride::Set(count) => count.to_string(),
    }
}

pub fn parse_unpause_action_mode_legacy_compatible(value: &str) -> Option<UnpauseActionMode> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "ifalreadyready" | "if_already_ready" | "if-already-ready" => {
            Some(UnpauseActionMode::IfAlreadyReady)
        }
        "ifothersready" | "if_others_ready" | "if-others-ready" => {
            Some(UnpauseActionMode::IfOthersReady)
        }
        "ifminusersready" | "if_min_users_ready" | "if-min-users-ready" => {
            Some(UnpauseActionMode::IfMinUsersReady)
        }
        "always" => Some(UnpauseActionMode::Always),
        _ => None,
    }
}

pub fn parse_autoplay_min_users_override_legacy_compatible(
    value: &str,
) -> Option<AutoplayThresholdOverride> {
    let parsed = value.trim().parse::<i64>().ok()?;
    if parsed <= 0 {
        return Some(AutoplayThresholdOverride::Disable);
    }
    usize::try_from(parsed)
        .ok()
        .map(AutoplayThresholdOverride::Set)
}

#[cfg(test)]
mod tests {
    use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

    use super::{
        AutoplayThresholdOverride, autoplay_threshold_override_legacy_value_compatible,
        parse_autoplay_min_users_override_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible, privacy_mode_legacy_name_compatible,
        unpause_action_mode_legacy_name_compatible,
    };

    #[test]
    fn legacy_name_helpers_match_expected_python_labels() {
        assert_eq!(
            privacy_mode_legacy_name_compatible(PrivacyMode::SendHashed),
            "SendHashed"
        );
        assert_eq!(
            unpause_action_mode_legacy_name_compatible(UnpauseActionMode::IfMinUsersReady),
            "IfMinUsersReady"
        );
        assert_eq!(
            autoplay_threshold_override_legacy_value_compatible(&AutoplayThresholdOverride::Set(3)),
            "3"
        );
    }

    #[test]
    fn autoplay_and_unpause_parsers_accept_known_values() {
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("if-min-users-ready"),
            Some(UnpauseActionMode::IfMinUsersReady)
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("0"),
            Some(AutoplayThresholdOverride::Disable)
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("3"),
            Some(AutoplayThresholdOverride::Set(3))
        );
    }
}
