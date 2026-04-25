use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::SystemTime,
};

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

pub(super) fn optional_string_list_multiline_text(value: Option<&[String]>) -> String {
    value
        .filter(|entries| !entries.is_empty())
        .map(|entries| entries.join("\n"))
        .unwrap_or_default()
}

pub(super) fn optional_room_text(value: Option<&str>) -> &str {
    value.filter(|text| !text.is_empty()).unwrap_or("(unset)")
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

pub(super) fn parse_editable_string_list_text(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('[') != trimmed.ends_with(']') {
        return None;
    }
    parse_serialized_string_list_legacy_compatible(trimmed)
}

pub(super) fn parse_room_history_text(value: &str) -> Option<Vec<String>> {
    let rooms = value
        .lines()
        .filter_map(nonempty_room_name_text)
        .collect::<BTreeSet<_>>();
    (!rooms.is_empty()).then(|| rooms.into_iter().collect())
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

#[cfg(test)]
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

pub(super) const NO_ROOM_JOINED_LABEL: &str = "(no room joined)";

pub(super) fn nonempty_room_name_text(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn configured_room_name_text(value: &str) -> Option<String> {
    (!value.is_empty() && value != "(unset)").then(|| value.to_owned())
}

pub(super) fn joined_room_name_text(value: &str) -> Option<&str> {
    if value.is_empty() || value == NO_ROOM_JOINED_LABEL {
        None
    } else {
        Some(value)
    }
}

pub(super) fn shared_playlist_entry_for_media_path(path: &str) -> Option<String> {
    let trimmed = normalized_editable_text(path)?;
    if trimmed.contains("://") {
        return Some(trimmed);
    }
    Some(
        Path::new(&trimmed)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(trimmed.as_str())
            .to_owned(),
    )
}

pub(super) fn player_arguments_text_for_path(
    arguments: Option<&BTreeMap<String, Vec<String>>>,
    player_path: Option<&str>,
) -> String {
    let Some(player_path) = player_path.and_then(normalized_editable_text) else {
        return String::new();
    };
    arguments
        .and_then(|arguments| arguments.get(&player_path))
        .map(|arguments| arguments.join(" "))
        .unwrap_or_default()
}

pub(super) fn set_player_arguments_text_for_path(
    arguments: &mut Option<BTreeMap<String, Vec<String>>>,
    player_path: Option<&str>,
    value: &str,
) {
    let Some(player_path) = player_path.and_then(normalized_editable_text) else {
        return;
    };
    let parsed_arguments = parse_command_line_like_text_legacy_compatible(value);
    let map = arguments.get_or_insert_with(BTreeMap::new);
    if parsed_arguments.is_empty() {
        map.remove(&player_path);
    } else {
        map.insert(player_path, parsed_arguments);
    }
    if map.is_empty() {
        *arguments = None;
    }
}

fn parse_command_line_like_text_legacy_compatible(value: &str) -> Vec<String> {
    let mut characters = value.chars().peekable();
    let mut parsed = Vec::new();

    while characters.peek().is_some() {
        while characters
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            characters.next();
        }
        if characters.peek().is_none() {
            break;
        }

        let mut token = String::new();
        while let Some(&character) = characters.peek() {
            if character.is_whitespace() {
                break;
            }
            if character == '"' {
                token.push(character);
                characters.next();
                for next_character in characters.by_ref() {
                    token.push(next_character);
                    if next_character == '"' {
                        break;
                    }
                }
                continue;
            }
            token.push(character);
            characters.next();
        }

        if !token.is_empty() {
            parsed.push(token);
        }
    }

    parsed
}
