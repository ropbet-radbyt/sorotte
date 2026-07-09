use std::{collections::BTreeMap, fmt};

use sorotte_client_core::{PrivacyMode, UnpauseActionMode};
use sorotte_secret::SecretValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoplayThresholdOverride {
    Disable,
    Set(usize),
}

#[derive(Clone, Default, PartialEq)]
pub struct StoredClientSettingsMvp {
    pub language: Option<String>,
    pub check_for_updates_automatically: Option<bool>,
    pub update_channel: Option<String>,
    pub last_checked_for_updates: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub server_password: Option<SecretValue>,
    pub username: Option<String>,
    pub room: Option<String>,
    pub room_list: Option<Vec<String>>,
    pub player_path: Option<String>,
    pub per_player_arguments: Option<BTreeMap<String, Vec<String>>>,
    pub media_search_directories: Option<Vec<String>>,
    pub public_servers: Option<Vec<(String, String)>>,
    pub stream_support_plugin_enabled: Option<bool>,
    pub media_matching_plugin_enabled: Option<bool>,
    pub plex_plugin_enabled: Option<bool>,
    pub plex_sync_enabled: Option<bool>,
    pub plex_streaming_enabled: Option<bool>,
    pub plex_user_token: Option<SecretValue>,
    pub plex_selected_server_id: Option<String>,
    pub plex_selected_server_url: Option<String>,
    pub plex_selected_server_token: Option<SecretValue>,
    pub media_match_fingerprinting_enabled: Option<bool>,
    pub media_match_background_warmup_enabled: Option<bool>,
    pub media_match_wire_sharing_enabled: Option<bool>,
    pub media_match_runtime_tolerance_enabled: Option<bool>,
    pub media_match_autoplay_policy: Option<String>,
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

impl fmt::Debug for StoredClientSettingsMvp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredClientSettingsMvp")
            .field("language", &self.language)
            .field(
                "check_for_updates_automatically",
                &self.check_for_updates_automatically,
            )
            .field("update_channel", &self.update_channel)
            .field("last_checked_for_updates", &self.last_checked_for_updates)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("server_password", &self.server_password)
            .field("username", &self.username)
            .field("room", &self.room)
            .field("room_list", &self.room_list)
            .field("player_path", &self.player_path)
            .field("per_player_arguments", &self.per_player_arguments)
            .field("media_search_directories", &self.media_search_directories)
            .field("public_servers", &self.public_servers)
            .field(
                "stream_support_plugin_enabled",
                &self.stream_support_plugin_enabled,
            )
            .field(
                "media_matching_plugin_enabled",
                &self.media_matching_plugin_enabled,
            )
            .field("plex_plugin_enabled", &self.plex_plugin_enabled)
            .field("plex_sync_enabled", &self.plex_sync_enabled)
            .field("plex_streaming_enabled", &self.plex_streaming_enabled)
            .field("plex_user_token", &self.plex_user_token)
            .field("plex_selected_server_id", &self.plex_selected_server_id)
            .field("plex_selected_server_url", &self.plex_selected_server_url)
            .field(
                "plex_selected_server_token",
                &self.plex_selected_server_token,
            )
            .field(
                "media_match_fingerprinting_enabled",
                &self.media_match_fingerprinting_enabled,
            )
            .field(
                "media_match_background_warmup_enabled",
                &self.media_match_background_warmup_enabled,
            )
            .field(
                "media_match_wire_sharing_enabled",
                &self.media_match_wire_sharing_enabled,
            )
            .field(
                "media_match_runtime_tolerance_enabled",
                &self.media_match_runtime_tolerance_enabled,
            )
            .field(
                "media_match_autoplay_policy",
                &self.media_match_autoplay_policy,
            )
            .field(
                "folder_search_first_file_timeout_seconds",
                &self.folder_search_first_file_timeout_seconds,
            )
            .field(
                "folder_search_timeout_seconds",
                &self.folder_search_timeout_seconds,
            )
            .field(
                "folder_search_double_check_interval_seconds",
                &self.folder_search_double_check_interval_seconds,
            )
            .field(
                "folder_search_warning_threshold_seconds",
                &self.folder_search_warning_threshold_seconds,
            )
            .field("force_gui_prompt", &self.force_gui_prompt)
            .field("autoplay_initial_state", &self.autoplay_initial_state)
            .field(
                "autoplay_require_same_filenames",
                &self.autoplay_require_same_filenames,
            )
            .field("ready_at_start", &self.ready_at_start)
            .field("shared_playlist_enabled", &self.shared_playlist_enabled)
            .field("pause_on_leave", &self.pause_on_leave)
            .field("loop_at_end_of_playlist", &self.loop_at_end_of_playlist)
            .field("loop_single_files", &self.loop_single_files)
            .field(
                "only_switch_to_trusted_domains",
                &self.only_switch_to_trusted_domains,
            )
            .field("trusted_domains", &self.trusted_domains)
            .field("rewind_on_desync", &self.rewind_on_desync)
            .field("fastforward_on_desync", &self.fastforward_on_desync)
            .field("slow_on_desync", &self.slow_on_desync)
            .field("dont_slow_down_with_me", &self.dont_slow_down_with_me)
            .field("rewind_threshold_seconds", &self.rewind_threshold_seconds)
            .field(
                "fastforward_threshold_seconds",
                &self.fastforward_threshold_seconds,
            )
            .field(
                "slowdown_threshold_seconds",
                &self.slowdown_threshold_seconds,
            )
            .field("unpause_action", &self.unpause_action)
            .field("autoplay_min_users", &self.autoplay_min_users)
            .field("filename_privacy_mode", &self.filename_privacy_mode)
            .field("filesize_privacy_mode", &self.filesize_privacy_mode)
            .field(
                "show_duration_notification",
                &self.show_duration_notification,
            )
            .field("autosave_joins_to_list", &self.autosave_joins_to_list)
            .field("show_osd", &self.show_osd)
            .field("chat_input_enabled", &self.chat_input_enabled)
            .field("chat_input_font_underline", &self.chat_input_font_underline)
            .field("chat_input_font_family", &self.chat_input_font_family)
            .field(
                "chat_input_relative_font_size",
                &self.chat_input_relative_font_size,
            )
            .field("chat_input_font_weight", &self.chat_input_font_weight)
            .field("chat_input_font_color", &self.chat_input_font_color)
            .field("chat_input_position", &self.chat_input_position)
            .field("chat_direct_input", &self.chat_direct_input)
            .field("chat_output_enabled", &self.chat_output_enabled)
            .field(
                "chat_output_font_underline",
                &self.chat_output_font_underline,
            )
            .field("chat_output_font_family", &self.chat_output_font_family)
            .field(
                "chat_output_relative_font_size",
                &self.chat_output_relative_font_size,
            )
            .field("chat_output_font_weight", &self.chat_output_font_weight)
            .field("chat_output_mode", &self.chat_output_mode)
            .field("chat_move_osd", &self.chat_move_osd)
            .field("chat_max_lines", &self.chat_max_lines)
            .field("chat_top_margin", &self.chat_top_margin)
            .field("chat_left_margin", &self.chat_left_margin)
            .field("chat_bottom_margin", &self.chat_bottom_margin)
            .field("chat_osd_margin", &self.chat_osd_margin)
            .field(
                "notification_timeout_seconds",
                &self.notification_timeout_seconds,
            )
            .field("alert_timeout_seconds", &self.alert_timeout_seconds)
            .field("chat_timeout_seconds", &self.chat_timeout_seconds)
            .field("show_same_room_osd", &self.show_same_room_osd)
            .field("show_osd_warnings", &self.show_osd_warnings)
            .field("show_slowdown_osd", &self.show_slowdown_osd)
            .field("show_noncontroller_osd", &self.show_noncontroller_osd)
            .field("show_different_room_osd", &self.show_different_room_osd)
            .field("show_contact_info", &self.show_contact_info)
            .finish()
    }
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
    use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

    use super::{
        AutoplayThresholdOverride, StoredClientSettingsMvp,
        autoplay_threshold_override_legacy_value_compatible,
        parse_autoplay_min_users_override_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible, privacy_mode_legacy_name_compatible,
        unpause_action_mode_legacy_name_compatible,
    };

    #[test]
    fn stored_client_settings_debug_redacts_all_credentials() {
        let settings = StoredClientSettingsMvp {
            server_password: Some("server-password-secret".into()),
            plex_user_token: Some("plex-user-token-secret".into()),
            plex_selected_server_token: Some("plex-server-token-secret".into()),
            ..StoredClientSettingsMvp::default()
        };

        let debug = format!("{settings:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("server-password-secret"));
        assert!(!debug.contains("plex-user-token-secret"));
        assert!(!debug.contains("plex-server-token-secret"));
    }

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
