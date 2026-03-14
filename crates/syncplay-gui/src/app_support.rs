use std::time::SystemTime;

use syncplay_client_app::app_boundary::{
    commands::LocalOffsetCommand,
    persistence::parse_serialized_string_list_legacy_compatible,
    state::{AutoplayThresholdOverride, StoredClientSettingsMvp},
};

use super::DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD;

pub(super) fn optional_text(value: Option<&str>) -> &str {
    value
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("(unset)")
}

pub(super) fn optional_string_list_text(value: Option<&[String]>) -> String {
    value
        .filter(|entries| !entries.is_empty())
        .map(|entries| entries.join("; "))
        .unwrap_or_else(|| "(unset)".to_owned())
}

pub(super) fn parse_trusted_domains_text(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('[') != trimmed.ends_with(']') {
        return None;
    }
    parse_serialized_string_list_legacy_compatible(trimmed)
}

pub(super) fn optional_port_text(value: Option<u16>) -> String {
    value.map_or_else(|| "(unset)".to_owned(), |value| value.to_string())
}

pub(super) fn optional_seconds_text(value: Option<f64>) -> String {
    value.map_or_else(|| "(unset)".to_owned(), |value| format!("{value:.2}s"))
}

pub(super) fn optional_f64_text(value: Option<f64>) -> String {
    value.map_or_else(|| "(unset)".to_owned(), |value| value.to_string())
}

pub(super) fn optional_i64_text(value: Option<i64>) -> String {
    value.map_or_else(|| "(unset)".to_owned(), |value| value.to_string())
}

pub(super) fn optional_index_text(value: Option<usize>) -> String {
    value.map_or_else(|| "(none)".to_owned(), |value| value.to_string())
}

pub(super) fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(super) fn autoplay_threshold_from_settings(settings: &StoredClientSettingsMvp) -> usize {
    match settings.autoplay_min_users.as_ref() {
        Some(AutoplayThresholdOverride::Set(count)) => (*count).clamp(2, 99),
        Some(AutoplayThresholdOverride::Disable) | None => DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD,
    }
}

pub(super) fn format_offset_command(command: &LocalOffsetCommand) -> String {
    match command {
        LocalOffsetCommand::Absolute(seconds) => seconds.to_string(),
        LocalOffsetCommand::Relative(seconds) if *seconds >= 0.0 => format!("+{seconds}"),
        LocalOffsetCommand::Relative(seconds) => seconds.to_string(),
        LocalOffsetCommand::RelativeFromCurrentPositionMinus(seconds) => format!("/{seconds}"),
    }
}

pub(super) fn system_time_seconds() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(super) fn normalized_editable_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "(unset)" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
