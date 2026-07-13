use super::types::{LocalInputCommand, LocalOffsetCommand};

pub fn parse_local_input_chat_message(input: &str) -> Option<String> {
    if input.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    for alias in ["chat", "ch"] {
        if input == alias {
            return Some(String::new());
        }

        let Some(message) = input
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };

        if message.is_empty() {
            return Some(String::new());
        }

        return Some(message.to_owned());
    }

    None
}

fn parse_create_command_legacy_compatible(input: &str) -> Option<Option<String>> {
    for alias in ["create", "c"] {
        if input == alias {
            return Some(None);
        }

        let Some(parameter) = input
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };

        if parameter.is_empty() {
            return Some(None);
        }

        return Some(Some(parameter.to_owned()));
    }

    None
}

fn parse_user_ready_command_legacy_compatible(
    input: &str,
    aliases: &[&str],
    ready: bool,
) -> Option<LocalInputCommand> {
    for alias in aliases {
        if input == *alias {
            return Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready,
            });
        }

        let Some(parameter) = input
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };

        if parameter.is_empty() {
            return Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready,
            });
        }

        return Some(LocalInputCommand::SetUserReady {
            username: parameter.to_owned(),
            ready,
        });
    }

    None
}

fn parse_room_command_legacy_compatible(input: &str) -> Option<Option<LocalInputCommand>> {
    for alias in ["room", "r"] {
        if input == alias {
            return Some(Some(LocalInputCommand::SetRoomWithLegacyFallback));
        }

        let Some(parameter) = input
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };

        if parameter.is_empty() {
            return Some(Some(LocalInputCommand::SetRoomWithLegacyFallback));
        }

        return Some(Some(LocalInputCommand::SetRoom(parameter.to_owned())));
    }

    None
}

fn parse_time_seconds_with_component_limits_legacy(
    value: &str,
    max_first_digits: usize,
    max_other_digits: usize,
) -> Option<f64> {
    if value.is_empty() {
        return None;
    }

    let mut parts: Vec<&str> = Vec::with_capacity(3);
    let mut start = 0usize;
    for (idx, ch) in value.char_indices() {
        if ch.is_ascii_digit() || ch == '.' {
            continue;
        }
        if idx == start {
            return None;
        }
        parts.push(&value[start..idx]);
        start = idx + ch.len_utf8();
    }
    if start >= value.len() {
        return None;
    }
    parts.push(&value[start..]);

    if parts.len() > 3 {
        return None;
    }

    for (index, part) in parts.iter().enumerate() {
        let is_last = index == parts.len() - 1;
        let (whole, fractional) = if is_last {
            let mut split = part.split('.');
            let whole = split.next().unwrap_or_default();
            let fractional = split.next();
            if split.next().is_some() {
                return None;
            }
            (whole, fractional)
        } else {
            (*part, None)
        };

        if whole.is_empty() || !whole.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let max_digits = if index == 0 {
            max_first_digits
        } else {
            max_other_digits
        };
        if whole.len() > max_digits {
            return None;
        }

        if let Some(fractional) = fractional
            && (fractional.is_empty()
                || fractional.len() > 3
                || !fractional.chars().all(|ch| ch.is_ascii_digit()))
        {
            return None;
        }
    }

    let seconds = match parts.as_slice() {
        [seconds] => seconds.parse::<f64>().ok()?,
        [minutes, seconds] => {
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds.parse::<f64>().ok()?;
            minutes as f64 * 60.0 + seconds
        }
        [hours, minutes, seconds] => {
            let hours = hours.parse::<u64>().ok()?;
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds.parse::<f64>().ok()?;
            hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds
        }
        _ => return None,
    };
    seconds.is_finite().then_some(seconds)
}

pub fn parse_seek_time_seconds_legacy_like(value: &str) -> Option<f64> {
    parse_time_seconds_with_component_limits_legacy(value, 4, 6)
}

fn parse_offset_time_seconds_legacy_like(value: &str) -> Option<f64> {
    parse_time_seconds_with_component_limits_legacy(value, 9, 9)
}

fn parse_seek_parameter(parameter: &str) -> Option<LocalInputCommand> {
    if parameter.is_empty() {
        return None;
    }

    if let Some(value) = parameter.strip_prefix('+') {
        let seconds = parse_seek_time_seconds_legacy_like(value)?;
        return Some(LocalInputCommand::SeekRelative(seconds));
    }
    if let Some(value) = parameter.strip_prefix('-') {
        let seconds = parse_seek_time_seconds_legacy_like(value)?;
        return Some(LocalInputCommand::SeekRelative(-seconds));
    }

    let seconds = parse_seek_time_seconds_legacy_like(parameter)?;
    Some(LocalInputCommand::SeekAbsolute(seconds))
}

fn parse_offset_parameter_legacy_compatible(parameter: &str) -> Option<LocalOffsetCommand> {
    if parameter.is_empty() {
        return None;
    }

    if let Some(value) = parameter.strip_prefix('+') {
        let seconds = parse_offset_time_seconds_legacy_like(value)?;
        return Some(LocalOffsetCommand::Relative(seconds));
    }
    if let Some(value) = parameter.strip_prefix('-') {
        let seconds = parse_offset_time_seconds_legacy_like(value)?;
        return Some(LocalOffsetCommand::Relative(-seconds));
    }
    if let Some(value) = parameter.strip_prefix('/') {
        let seconds = parse_offset_time_seconds_legacy_like(value)?;
        return Some(LocalOffsetCommand::RelativeFromCurrentPositionMinus(
            seconds,
        ));
    }

    let seconds = parse_offset_time_seconds_legacy_like(parameter)?;
    Some(LocalOffsetCommand::Absolute(seconds))
}

fn parse_offset_input_legacy_compatible(input: &str) -> Option<LocalInputCommand> {
    let remainder = if let Some(remainder) = input.strip_prefix("offset") {
        remainder
    } else {
        input.strip_prefix('o')?
    };

    let parameter = if let Some(parameter) = remainder.strip_prefix(' ') {
        if parameter.starts_with(' ') {
            return None;
        }
        parameter
    } else {
        remainder
    };
    if parameter.is_empty() {
        return None;
    }

    let offset_command = parse_offset_parameter_legacy_compatible(parameter)?;
    Some(LocalInputCommand::SetUserOffset(offset_command))
}

fn parse_seek_input_legacy_compatible(input: &str) -> Option<LocalInputCommand> {
    if input.is_empty() {
        return None;
    }

    let (parameter, had_seek_prefix) = if let Some(value) = input.strip_prefix("seek") {
        (value, true)
    } else if let Some(value) = input.strip_prefix('s') {
        (value, true)
    } else {
        (input, false)
    };

    if had_seek_prefix {
        let parameter = if let Some(parameter) = parameter.strip_prefix(' ') {
            if parameter.starts_with(' ') {
                return None;
            }
            parameter
        } else {
            parameter
        };
        if parameter.is_empty() {
            return None;
        }
        return parse_seek_parameter(parameter);
    } else {
        let starts_like_seek_value = parameter
            .chars()
            .next()
            .is_some_and(|ch| ch == '+' || ch == '-' || ch.is_ascii_digit());
        if !starts_like_seek_value {
            return None;
        }
    }

    parse_seek_parameter(parameter)
}

fn parse_playlist_index_parameter_legacy_compatible(parameter: &str) -> Option<i64> {
    let one_based_index = parameter.trim().parse::<i64>().ok()?;
    if one_based_index <= 0 {
        return None;
    }
    one_based_index.checked_sub(1)
}

fn parse_queue_command_legacy_compatible(
    input: &str,
    aliases: &[&str],
    select_after_queue: bool,
) -> Option<LocalInputCommand> {
    for alias in aliases {
        if input == *alias {
            return Some(LocalInputCommand::ShowQueueMissingFileError);
        }

        let Some(file_name) = input
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };

        if file_name.is_empty() {
            return Some(LocalInputCommand::ShowQueueMissingFileError);
        }

        return Some(LocalInputCommand::QueuePlaylistItem {
            file_name: file_name.to_owned(),
            select_after_queue,
        });
    }

    None
}

fn matches_local_command_alias_legacy_compatible(input: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| {
        if input == *alias {
            return true;
        }
        input
            .strip_prefix(alias)
            .is_some_and(|rest| rest.starts_with(' '))
    })
}

fn is_known_local_command_token_legacy_compatible(token: &str) -> bool {
    matches!(
        token,
        "help"
            | "h"
            | "?"
            | "\\?"
            | "undoplaylist"
            | "shuffleremainingplaylist"
            | "shuffleentireplaylist"
            | "undo"
            | "u"
            | "revert"
            | "keep-waiting"
            | "join-nearest-buffered-position"
            | "cancel-and-remain"
            | "list"
            | "l"
            | "users"
            | "playlist"
            | "ql"
            | "pl"
            | "select"
            | "qs"
            | "next"
            | "qn"
            | "queue"
            | "qa"
            | "add"
            | "queueandselect"
            | "qas"
            | "delete"
            | "d"
            | "qd"
            | "setready"
            | "sr"
            | "setnotready"
            | "sn"
            | "snr"
            | "create"
            | "c"
            | "auth"
            | "a"
            | "seek"
            | "s"
            | "pause"
            | "play"
            | "p"
            | "room"
            | "r"
            | "toggle"
            | "t"
            | "offset"
            | "o"
            | "chat"
            | "ch"
    )
}

pub fn parse_local_input_command(input: &str) -> Option<LocalInputCommand> {
    if input.starts_with(' ') {
        return None;
    }
    if input.chars().next().is_some_and(char::is_whitespace) {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }

    let trimmed = input.trim_end_matches(' ');
    if matches_local_command_alias_legacy_compatible(trimmed, &["help", "h", "?", "/?", "\\?"]) {
        return Some(LocalInputCommand::ShowHelp);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["keep-waiting"]) {
        return Some(LocalInputCommand::KeepWaitingForSeekPreparation);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["join-nearest-buffered-position"]) {
        return Some(LocalInputCommand::JoinNearestBufferedSeekPreparation);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["cancel-and-remain"]) {
        return Some(LocalInputCommand::CancelSeekPreparation);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["undoplaylist"]) {
        return Some(LocalInputCommand::UndoPlaylistChange);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["shuffleremainingplaylist"]) {
        return Some(LocalInputCommand::ShuffleRemainingPlaylist);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["shuffleentireplaylist"]) {
        return Some(LocalInputCommand::ShuffleEntirePlaylist);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["undo", "u", "revert"]) {
        return Some(LocalInputCommand::UndoSeek);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["list", "l", "users"]) {
        return Some(LocalInputCommand::RequestUserList);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["playlist", "ql", "pl"]) {
        return Some(LocalInputCommand::ShowPlaylist);
    }
    if let Some(index) = trimmed
        .strip_prefix("select ")
        .or_else(|| trimmed.strip_prefix("qs "))
    {
        return parse_playlist_index_parameter_legacy_compatible(index)
            .map(LocalInputCommand::SelectPlaylistIndex)
            .or(Some(LocalInputCommand::ShowPlaylistInvalidIndexError));
    }
    if matches!(trimmed, "select" | "qs") {
        return Some(LocalInputCommand::ShowPlaylistInvalidIndexError);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["next", "qn"]) {
        return Some(LocalInputCommand::NextPlaylistItem);
    }
    if let Some(command) =
        parse_queue_command_legacy_compatible(input, &["queueandselect", "qas"], true)
    {
        return Some(command);
    }
    if let Some(command) =
        parse_queue_command_legacy_compatible(input, &["queue", "qa", "add"], false)
    {
        return Some(command);
    }
    if let Some(index) = trimmed
        .strip_prefix("delete ")
        .or_else(|| trimmed.strip_prefix("d "))
        .or_else(|| trimmed.strip_prefix("qd "))
    {
        return parse_playlist_index_parameter_legacy_compatible(index)
            .map(LocalInputCommand::DeletePlaylistIndex)
            .or(Some(LocalInputCommand::ShowPlaylistInvalidIndexError));
    }
    if matches!(trimmed, "delete" | "d" | "qd") {
        return Some(LocalInputCommand::ShowPlaylistInvalidIndexError);
    }
    if let Some(command) =
        parse_user_ready_command_legacy_compatible(input, &["setready", "sr"], true)
    {
        return Some(command);
    }
    if let Some(command) =
        parse_user_ready_command_legacy_compatible(input, &["setnotready", "sn", "snr"], false)
    {
        return Some(command);
    }
    if let Some(room_name) = parse_create_command_legacy_compatible(input) {
        return Some(LocalInputCommand::CreateControlledRoom(room_name));
    }
    if let Some(password) = trimmed
        .strip_prefix("auth ")
        .or_else(|| trimmed.strip_prefix("a "))
    {
        let password = password.trim();
        return Some(LocalInputCommand::AuthController(password.into()));
    }
    if matches!(trimmed, "auth" | "a") {
        return Some(LocalInputCommand::AuthController(String::new().into()));
    }
    if let Some(parameter) = input
        .strip_prefix("seek ")
        .or_else(|| input.strip_prefix("s "))
    {
        return parse_seek_parameter(parameter).or(Some(LocalInputCommand::ShowUnknownCommandHelp));
    }
    if matches!(trimmed, "seek" | "s") {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["p", "pause", "play"]) {
        return Some(LocalInputCommand::TogglePause);
    }
    if let Some(room_command) = parse_room_command_legacy_compatible(input) {
        return room_command;
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["t", "toggle"]) {
        return Some(LocalInputCommand::ToggleReady);
    }
    if let Some(command) = parse_offset_input_legacy_compatible(input) {
        return Some(command);
    }
    let command_token = trimmed.split_whitespace().next().unwrap_or_default();
    if matches!(command_token, "o" | "offset" | "/o" | "/offset")
        || trimmed.starts_with("o+")
        || trimmed.starts_with("o-")
        || trimmed.starts_with("o/")
        || trimmed.starts_with("offset+")
        || trimmed.starts_with("offset-")
        || trimmed.starts_with("offset/")
    {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if let Some(command) = parse_seek_input_legacy_compatible(input) {
        return Some(command);
    }
    if trimmed.starts_with("s+")
        || trimmed.starts_with("s-")
        || trimmed.starts_with("seek+")
        || trimmed.starts_with("seek-")
    {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if let Some(chat_message) = parse_local_input_chat_message(input) {
        return Some(LocalInputCommand::Chat(chat_message));
    }
    if is_known_local_command_token_legacy_compatible(command_token) {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if trimmed.starts_with('/') {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if input.chars().any(|ch| ch.is_whitespace() && ch != ' ') {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if trimmed.is_empty() {
        return None;
    }
    Some(LocalInputCommand::ShowUnknownCommandHelp)
}
