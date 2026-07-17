use serde_json::{Value, json};

use crate::constants::LEGACY_SYNCPLAYINTF_PROTOCOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySyncplayOsdKind {
    Notification,
    Alert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySyncplayUiSettings {
    pub show_osd: bool,
    pub chat_output_enabled: bool,
    pub chat_input_enabled: bool,
    pub chat_input_font_underline: bool,
    pub chat_input_font_family: String,
    pub chat_input_relative_font_size: i64,
    pub chat_input_font_weight: i64,
    pub chat_input_font_color: String,
    pub chat_input_position: String,
    pub chat_direct_input: bool,
    pub chat_output_font_underline: bool,
    pub chat_output_font_family: String,
    pub chat_output_relative_font_size: i64,
    pub chat_output_font_weight: i64,
    pub chat_output_mode: String,
    pub chat_max_lines: i64,
    pub chat_top_margin: i64,
    pub chat_left_margin: i64,
    pub chat_bottom_margin: i64,
    pub chat_move_osd: bool,
    pub chat_osd_margin: i64,
    pub notification_timeout_ms: u64,
    pub alert_timeout_ms: u64,
    pub chat_timeout_ms: u64,
}

impl LegacySyncplayUiSettings {
    fn chat_input_position_top(&self) -> bool {
        self.chat_input_position.trim().eq_ignore_ascii_case("Top")
    }

    pub fn should_move_osd(&self) -> bool {
        self.chat_move_osd
            && (self.chat_output_enabled
                || (self.chat_input_enabled && self.chat_input_position_top()))
    }

    pub fn syncplayintf_options_differ(&self, other: &Self) -> bool {
        self.syncplayintf_settings_value() != other.syncplayintf_settings_value()
    }

    pub fn uses_syncplayintf_bridge(&self) -> bool {
        self.chat_output_enabled || self.chat_input_enabled
    }

    fn syncplayintf_settings_value(&self) -> Value {
        json!({
            "chatInputEnabled": self.chat_input_enabled,
            "chatInputFontFamily": self.chat_input_font_family.trim(),
            "chatInputRelativeFontSize": self.chat_input_relative_font_size,
            "chatInputFontWeight": self.chat_input_font_weight,
            "chatInputFontUnderline": self.chat_input_font_underline,
            "chatInputFontColor": self.chat_input_font_color.trim(),
            "chatInputPosition": self.chat_input_position.trim(),
            "chatOutputFontFamily": self.chat_output_font_family.trim(),
            "chatOutputRelativeFontSize": self.chat_output_relative_font_size,
            "chatOutputFontWeight": self.chat_output_font_weight,
            "chatOutputFontUnderline": self.chat_output_font_underline,
            "chatOutputMode": self.chat_output_mode.trim(),
            "chatMaxLines": self.chat_max_lines,
            "chatTopMargin": self.chat_top_margin,
            "chatLeftMargin": self.chat_left_margin,
            "chatBottomMargin": self.chat_bottom_margin,
            "chatDirectInput": self.chat_direct_input,
            "notificationTimeout": self.notification_timeout_ms as f64 / 1_000.0,
            "alertTimeout": self.alert_timeout_ms as f64 / 1_000.0,
            "chatTimeout": self.chat_timeout_ms as f64 / 1_000.0,
            "chatOutputEnabled": self.chat_output_enabled,
        })
    }

    pub(crate) fn syncplayintf_options_payload(
        &self,
        bridge_instance_id: &str,
        owner_id: &str,
        attachment_id: &str,
        generation: u64,
        lease_ms: u64,
    ) -> String {
        json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "bridgeInstanceId": bridge_instance_id,
            "ownerId": owner_id,
            "attachmentId": attachment_id,
            "generation": generation,
            "leaseMs": lease_ms,
            "settings": self.syncplayintf_settings_value(),
        })
        .to_string()
    }
}

impl Default for LegacySyncplayUiSettings {
    fn default() -> Self {
        Self {
            show_osd: true,
            chat_output_enabled: true,
            chat_input_enabled: true,
            chat_input_font_underline: false,
            chat_input_font_family: "sans-serif".to_owned(),
            chat_input_relative_font_size: 24,
            chat_input_font_weight: 1,
            chat_input_font_color: "#FFFF00".to_owned(),
            chat_input_position: "Top".to_owned(),
            chat_direct_input: false,
            chat_output_font_underline: false,
            chat_output_font_family: "sans-serif".to_owned(),
            chat_output_relative_font_size: 24,
            chat_output_font_weight: 1,
            chat_output_mode: "Chatroom".to_owned(),
            chat_max_lines: 7,
            chat_top_margin: 25,
            chat_left_margin: 20,
            chat_bottom_margin: 30,
            chat_move_osd: true,
            chat_osd_margin: 110,
            notification_timeout_ms: 3_000,
            alert_timeout_ms: 5_000,
            chat_timeout_ms: 7_000,
        }
    }
}

pub(crate) fn sanitize_legacy_syncplay_script_message_text(message: &str) -> String {
    message
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace('%', "%%")
        .replace('{', "\\{")
        .replace('}', "\\}")
}
