use std::path::Path;

use crate::constants::LEGACY_SYNCPLAYINTF_SCRIPT_NAME;

pub(crate) fn legacy_syncplayintf_script_name_for_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(LEGACY_SYNCPLAYINTF_SCRIPT_NAME)
        .to_owned()
}

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
        self.syncplayintf_options_payload() != other.syncplayintf_options_payload()
    }

    pub fn uses_syncplayintf_bridge(&self) -> bool {
        self.chat_output_enabled || self.chat_input_enabled
    }

    pub(crate) fn syncplayintf_options_payload(&self) -> String {
        let options = [
            (
                "chatInputEnabled",
                legacy_syncplay_bool_string_compatible(self.chat_input_enabled),
            ),
            (
                "chatInputFontFamily",
                self.chat_input_font_family.trim().to_owned(),
            ),
            (
                "chatInputRelativeFontSize",
                self.chat_input_relative_font_size.to_string(),
            ),
            (
                "chatInputFontWeight",
                self.chat_input_font_weight.to_string(),
            ),
            (
                "chatInputFontUnderline",
                legacy_syncplay_bool_string_compatible(self.chat_input_font_underline),
            ),
            (
                "chatInputFontColor",
                self.chat_input_font_color.trim().to_owned(),
            ),
            (
                "chatInputPosition",
                self.chat_input_position.trim().to_owned(),
            ),
            (
                "chatOutputFontFamily",
                self.chat_output_font_family.trim().to_owned(),
            ),
            (
                "chatOutputRelativeFontSize",
                self.chat_output_relative_font_size.to_string(),
            ),
            (
                "chatOutputFontWeight",
                self.chat_output_font_weight.to_string(),
            ),
            (
                "chatOutputFontUnderline",
                legacy_syncplay_bool_string_compatible(self.chat_output_font_underline),
            ),
            ("chatOutputMode", self.chat_output_mode.trim().to_owned()),
            ("chatMaxLines", self.chat_max_lines.to_string()),
            ("chatTopMargin", self.chat_top_margin.to_string()),
            ("chatLeftMargin", self.chat_left_margin.to_string()),
            ("chatBottomMargin", self.chat_bottom_margin.to_string()),
            (
                "chatDirectInput",
                legacy_syncplay_bool_string_compatible(self.chat_direct_input),
            ),
            (
                "notificationTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.notification_timeout_ms),
            ),
            (
                "alertTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.alert_timeout_ms),
            ),
            (
                "chatTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.chat_timeout_ms),
            ),
            (
                "chatOutputEnabled",
                legacy_syncplay_bool_string_compatible(self.chat_output_enabled),
            ),
        ];

        options
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
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

fn legacy_syncplay_bool_string_compatible(value: bool) -> String {
    if value {
        "True".to_owned()
    } else {
        "False".to_owned()
    }
}

fn legacy_syncplay_timeout_seconds_string_compatible(duration_ms: u64) -> String {
    if duration_ms.is_multiple_of(1_000) {
        return (duration_ms / 1_000).to_string();
    }

    let seconds = duration_ms as f64 / 1_000.0;
    let mut formatted = format!("{seconds:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
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
