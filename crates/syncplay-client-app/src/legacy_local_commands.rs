use std::sync::atomic::{AtomicU64, Ordering};

use syncplay_client_core::ClientSession;

const CONTROL_ROOM_HASH_LEN: usize = 12;
static ROOM_PASSWORD_NONCE: AtomicU64 = AtomicU64::new(0);
const PLAYLIST_EMPTY_MESSAGE_LEGACY: &str = "Playlist is currently empty.";
const PLAYLIST_INVALID_INDEX_ERROR_LEGACY: &str = "Invalid playlist index";
const QUEUE_MISSING_FILE_ERROR_LEGACY: &str = "No file/url given";
const UNKNOWN_COMMAND_MESSAGE_LEGACY: &str = "Unrecognized command";
const PROJECT_URL_LEGACY: &str = "https://syncplay.pl/";

#[derive(Debug, Clone, PartialEq)]
pub enum LocalOffsetCommand {
    Absolute(f64),
    Relative(f64),
    RelativeFromCurrentPositionMinus(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalInputCommand {
    Chat(String),
    RequestUserList,
    ShowUnknownCommandHelp,
    ShowHelp,
    ShowPlaylistInvalidIndexError,
    ShowQueueMissingFileError,
    ShowPlaylist,
    SelectPlaylistIndex(i64),
    NextPlaylistItem,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    SetUserOffset(LocalOffsetCommand),
    SeekAbsolute(f64),
    SeekRelative(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    CreateControlledRoom(Option<String>),
    AuthController(String),
    SetRoomWithLegacyFallback,
    SetRoom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInputCommandErrorKind {
    PlaylistInvalidIndex,
    QueueMissingFile,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalRuntimeAction {
    SendChat(String),
    RequestUserList,
    SetPlaylistIndex(i64),
    AdvancePlaylistIndex,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    SetUserOffset(LocalOffsetCommand),
    SeekToPosition(f64),
    SeekByOffset(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    SetRoomWithLegacyFallback(String),
    SetRoom(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedLocalRuntimeDispatch {
    pub line_to_emit: Option<String>,
    pub action: Option<PlannedLocalRuntimeAction>,
    pub updated_user_offset_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalInputDispatch {
    Suppressed,
    EmitUnknownCommandHelp,
    EmitHelp,
    EmitError(LocalInputCommandErrorKind),
    EmitPlaylist,
    Run(PlannedLocalRuntimeAction),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalInputCommand {
    SendChat(String),
    RequestUserList,
    ShowUnknownCommandHelp,
    ShowHelp,
    ShowError(LocalInputCommandErrorKind),
    ShowPlaylist,
    SelectPlaylistIndex(i64),
    NextPlaylistItem,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    SetUserOffset(LocalOffsetCommand),
    SeekAbsolute(f64),
    SeekRelative(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    SetRoomWithLegacyFallback(String),
    SetRoom(String),
}

pub struct LocalInputCommandPlanningContext<'a> {
    pub current_room: Option<&'a str>,
    pub configured_room: &'a str,
}

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

        if let Some(fractional) = fractional {
            if fractional.is_empty()
                || fractional.len() > 3
                || !fractional.chars().all(|ch| ch.is_ascii_digit())
            {
                return None;
            }
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
    } else if let Some(remainder) = input.strip_prefix('o') {
        remainder
    } else {
        return None;
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
        return Some(LocalInputCommand::AuthController(password.to_owned()));
    }
    if matches!(trimmed, "auth" | "a") {
        return Some(LocalInputCommand::AuthController(String::new()));
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

pub fn controlled_room_base_name_legacy_compatible(room: &str) -> String {
    if !room.starts_with('+') {
        return room.to_owned();
    }

    let Some(room_without_prefix) = room.strip_prefix('+') else {
        return room.to_owned();
    };
    let Some((room_base, hash_suffix)) = room_without_prefix.rsplit_once(':') else {
        return room.to_owned();
    };
    if room_base.is_empty()
        || hash_suffix.len() != CONTROL_ROOM_HASH_LEN
        || !hash_suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return room.to_owned();
    }
    room_base.to_owned()
}

pub fn generate_room_password_legacy_compatible() -> String {
    fn next_seed() -> u64 {
        let nanos_since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let nonce = ROOM_PASSWORD_NONCE.fetch_add(1, Ordering::Relaxed);
        nanos_since_epoch
            ^ nonce.rotate_left(17)
            ^ ((std::process::id() as u64) << 32)
            ^ 0x9E37_79B9_7F4A_7C15
    }

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed
    }

    fn next_letter(seed: &mut u64) -> char {
        let value = (lcg(seed) % 26) as u8;
        (b'A' + value) as char
    }

    fn next_digit(seed: &mut u64) -> char {
        let value = (lcg(seed) % 10) as u8;
        (b'0' + value) as char
    }

    let mut seed = next_seed();
    format!(
        "{}{}-{}{}{}-{}{}{}",
        next_letter(&mut seed),
        next_letter(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed)
    )
}

impl PlannedLocalInputCommand {
    pub fn uses_shared_playlists(&self) -> bool {
        matches!(
            self,
            Self::ShowPlaylist
                | Self::SelectPlaylistIndex(_)
                | Self::NextPlaylistItem
                | Self::QueuePlaylistItem { .. }
                | Self::DeletePlaylistIndex(_)
                | Self::UndoPlaylistChange
                | Self::ShuffleRemainingPlaylist
                | Self::ShuffleEntirePlaylist
        )
    }
}

pub fn plan_local_input_dispatch_legacy_compatible(
    command: PlannedLocalInputCommand,
    shared_playlists_enabled: bool,
) -> PlannedLocalInputDispatch {
    if !shared_playlists_enabled && command.uses_shared_playlists() {
        return PlannedLocalInputDispatch::Suppressed;
    }

    match command {
        PlannedLocalInputCommand::SendChat(chat_message) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SendChat(chat_message))
        }
        PlannedLocalInputCommand::RequestUserList => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestUserList)
        }
        PlannedLocalInputCommand::ShowUnknownCommandHelp => {
            PlannedLocalInputDispatch::EmitUnknownCommandHelp
        }
        PlannedLocalInputCommand::ShowHelp => PlannedLocalInputDispatch::EmitHelp,
        PlannedLocalInputCommand::ShowError(error_kind) => {
            PlannedLocalInputDispatch::EmitError(error_kind)
        }
        PlannedLocalInputCommand::ShowPlaylist => PlannedLocalInputDispatch::EmitPlaylist,
        PlannedLocalInputCommand::SelectPlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetPlaylistIndex(index))
        }
        PlannedLocalInputCommand::NextPlaylistItem => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::AdvancePlaylistIndex)
        }
        PlannedLocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }),
        PlannedLocalInputCommand::DeletePlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::DeletePlaylistIndex(index))
        }
        PlannedLocalInputCommand::UndoPlaylistChange => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoPlaylistChange)
        }
        PlannedLocalInputCommand::ShuffleRemainingPlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist)
        }
        PlannedLocalInputCommand::ShuffleEntirePlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleEntirePlaylist)
        }
        PlannedLocalInputCommand::UndoSeek => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoSeek)
        }
        PlannedLocalInputCommand::SetUserOffset(command) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserOffset(command))
        }
        PlannedLocalInputCommand::SeekAbsolute(position_seconds) => PlannedLocalInputDispatch::Run(
            PlannedLocalRuntimeAction::SeekToPosition(position_seconds),
        ),
        PlannedLocalInputCommand::SeekRelative(offset_seconds) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds))
        }
        PlannedLocalInputCommand::TogglePause => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::TogglePause)
        }
        PlannedLocalInputCommand::ToggleReady => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ToggleReady)
        }
        PlannedLocalInputCommand::SetUserReady { username, ready } => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserReady {
                username,
                ready,
            })
        }
        PlannedLocalInputCommand::RequestControllerAuth { room, password } => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestControllerAuth {
                room,
                password,
            })
        }
        PlannedLocalInputCommand::SetRoomWithLegacyFallback(room) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(
                room,
            ))
        }
        PlannedLocalInputCommand::SetRoom(room) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoom(room))
        }
    }
}

pub fn plan_local_input_command_legacy_compatible(
    command: LocalInputCommand,
    context: &LocalInputCommandPlanningContext<'_>,
) -> PlannedLocalInputCommand {
    match command {
        LocalInputCommand::Chat(chat_message) => PlannedLocalInputCommand::SendChat(chat_message),
        LocalInputCommand::RequestUserList => PlannedLocalInputCommand::RequestUserList,
        LocalInputCommand::ShowUnknownCommandHelp => {
            PlannedLocalInputCommand::ShowUnknownCommandHelp
        }
        LocalInputCommand::ShowHelp => PlannedLocalInputCommand::ShowHelp,
        LocalInputCommand::ShowPlaylistInvalidIndexError => {
            PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::PlaylistInvalidIndex)
        }
        LocalInputCommand::ShowQueueMissingFileError => {
            PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::QueueMissingFile)
        }
        LocalInputCommand::ShowPlaylist => PlannedLocalInputCommand::ShowPlaylist,
        LocalInputCommand::SelectPlaylistIndex(index) => {
            PlannedLocalInputCommand::SelectPlaylistIndex(index)
        }
        LocalInputCommand::NextPlaylistItem => PlannedLocalInputCommand::NextPlaylistItem,
        LocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => PlannedLocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        },
        LocalInputCommand::DeletePlaylistIndex(index) => {
            PlannedLocalInputCommand::DeletePlaylistIndex(index)
        }
        LocalInputCommand::UndoPlaylistChange => PlannedLocalInputCommand::UndoPlaylistChange,
        LocalInputCommand::ShuffleRemainingPlaylist => {
            PlannedLocalInputCommand::ShuffleRemainingPlaylist
        }
        LocalInputCommand::ShuffleEntirePlaylist => PlannedLocalInputCommand::ShuffleEntirePlaylist,
        LocalInputCommand::UndoSeek => PlannedLocalInputCommand::UndoSeek,
        LocalInputCommand::SetUserOffset(command) => {
            PlannedLocalInputCommand::SetUserOffset(command)
        }
        LocalInputCommand::SeekAbsolute(position_seconds) => {
            PlannedLocalInputCommand::SeekAbsolute(position_seconds)
        }
        LocalInputCommand::SeekRelative(offset_seconds) => {
            PlannedLocalInputCommand::SeekRelative(offset_seconds)
        }
        LocalInputCommand::TogglePause => PlannedLocalInputCommand::TogglePause,
        LocalInputCommand::ToggleReady => PlannedLocalInputCommand::ToggleReady,
        LocalInputCommand::SetUserReady { username, ready } => {
            PlannedLocalInputCommand::SetUserReady { username, ready }
        }
        LocalInputCommand::CreateControlledRoom(room_name) => {
            let room = room_name.unwrap_or_else(|| {
                context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned()
            });
            PlannedLocalInputCommand::RequestControllerAuth {
                room: controlled_room_base_name_legacy_compatible(&room),
                password: generate_room_password_legacy_compatible(),
            }
        }
        LocalInputCommand::AuthController(password) => {
            PlannedLocalInputCommand::RequestControllerAuth {
                room: context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned(),
                password,
            }
        }
        LocalInputCommand::SetRoomWithLegacyFallback => {
            PlannedLocalInputCommand::SetRoomWithLegacyFallback(context.configured_room.to_owned())
        }
        LocalInputCommand::SetRoom(room) => PlannedLocalInputCommand::SetRoom(room),
    }
}

pub fn resolved_local_user_offset_seconds_legacy_compatible(
    current_user_offset_seconds: f64,
    global_position_seconds: f64,
    command: &LocalOffsetCommand,
) -> f64 {
    let current_local_position = global_position_seconds + current_user_offset_seconds;
    match command {
        LocalOffsetCommand::Absolute(offset_seconds) => *offset_seconds,
        LocalOffsetCommand::Relative(offset_delta_seconds) => {
            current_user_offset_seconds + offset_delta_seconds
        }
        LocalOffsetCommand::RelativeFromCurrentPositionMinus(offset_seconds) => {
            current_local_position - offset_seconds
        }
    }
}

pub fn playlist_index_in_bounds_legacy_compatible(session: &ClientSession, index: i64) -> bool {
    if index < 0 {
        return false;
    }
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    session
        .current_room_playlist()
        .is_some_and(|playlist| index < playlist.files.len())
}

pub fn localized_local_input_error_message_legacy_compatible(
    error_kind: LocalInputCommandErrorKind,
    language: Option<&str>,
) -> &'static str {
    match (error_kind, language) {
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("de")) => {
            "Ungueltiger Playlist-Index"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("es")) => {
            "Indice de lista de reproduccion no valido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("eo")) => {
            "Nevalida ludlista indekso"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("fi")) => {
            "Virheellinen soittolistaindeksi"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("fr")) => {
            "Indice de playlist non valide"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("it")) => {
            "Indice della playlist non valido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("pt_PT" | "pt_BR")) => {
            "Indice de playlist invalido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("tr")) => {
            "Gecersiz oynatma listesi indeksi"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("ru")) => {
            "Nedopustimyi indeks spiska vosproizvedeniia"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("zh_CN")) => {
            "Wuxiao de bofang liebiao suoyin"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("ko")) => {
            "Yuhyo haji an-eun jaesaeng moglog indeks"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, _) => {
            PLAYLIST_INVALID_INDEX_ERROR_LEGACY
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("de")) => "Keine Datei/URL angegeben",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("es")) => {
            "No se proporciono archivo/url"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("eo")) => "Neniu dosiero/url donita",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("fi")) => {
            "Tiedostoa/url-osoitetta ei annettu"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("fr")) => "Aucun fichier/url fourni",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("it")) => "Nessun file/url fornito",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("pt_PT" | "pt_BR")) => {
            "Nenhum arquivo/url informado"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("tr")) => "Dosya/url verilmedi",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("ru")) => "Fail/url ne ukazan",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("zh_CN")) => "Wei tigong wenjian/url",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("ko")) => {
            "Pail/url-i jegongdoeji anassseumnida"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, _) => QUEUE_MISSING_FILE_ERROR_LEGACY,
    }
}

fn localized_error_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "FEHLER",
        Some("es") => "ERROR",
        Some("eo") => "ERARO",
        Some("fi") => "VIRHE",
        Some("fr") => "ERREUR",
        Some("it") => "ERRORE",
        Some("pt_PT" | "pt_BR") => "ERRO",
        Some("tr") => "HATA",
        Some("ru") => "OSHIBKA",
        Some("zh_CN") => "CUOWU",
        Some("ko") => "OREU",
        _ => "ERROR",
    }
}

pub fn local_input_error_output_line_legacy_compatible(
    error_kind: LocalInputCommandErrorKind,
    language: Option<&str>,
) -> String {
    format!(
        "{}:\t{}",
        localized_error_prefix_legacy_compatible(language),
        localized_local_input_error_message_legacy_compatible(error_kind, language)
    )
}

fn localized_unknown_command_message_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Unbekannter Befehl",
        Some("es") => "Comando no reconocido",
        Some("eo") => "Nekonata komando",
        Some("fi") => "Tuntematon komento",
        Some("fr") => "Commande non reconnue",
        Some("it") => "Comando non riconosciuto",
        Some("pt_PT" | "pt_BR") => "Comando nao reconhecido",
        Some("tr") => "Taninmayan komut",
        Some("ru") => "Neopoznannaia komanda",
        Some("zh_CN") => "Wei shibie de mingling",
        Some("ko") => "Insikhal su eomneun myeongryeong",
        _ => UNKNOWN_COMMAND_MESSAGE_LEGACY,
    }
}

fn localized_local_command_help_heading_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Verfuegbare Befehle:",
        Some("es") => "Comandos disponibles:",
        Some("eo") => "Doneblaj ordonoj:",
        Some("fi") => "Kaytettavissa olevat komennot:",
        Some("fr") => "Commandes disponibles:",
        Some("it") => "Comandi disponibili:",
        Some("pt_PT" | "pt_BR") => "Comandos disponiveis:",
        Some("tr") => "Kullanilabilir komutlar:",
        Some("ru") => "Dostupnye komandy:",
        Some("zh_CN") => "Ke yong mingling:",
        Some("ko") => "Sayong ganeunghan myeongryeong:",
        _ => "Available commands:",
    }
}

fn local_command_help_command_lines_legacy_compatible() -> &'static [&'static str] {
    &[
        "\tr [name] - change room",
        "\tl - show user list",
        "\tu - undo last seek",
        "\tp - toggle pause",
        "\t[s][+-]time - seek to the given value of time, if + or - is not specified it's absolute time in seconds or min:sec",
        "\to[+-]duration - offset local playback by the given duration (in seconds or min:sec) from the server seek position - this is a deprecated feature",
        "\th - this help",
        "\tt - toggles whether you are ready to watch or not",
        "\tsr [name] - sets user as ready",
        "\tsn [name] - sets user as not ready",
        "\tc [name] - create managed room using name of current room",
        "\ta [password] - authenticate as room operator with operator password",
        "\tch [message] - send a chat message in a room",
        "\tqa [file/url] - add file or url to bottom of playlist",
        "\tqas [file/url] - add file or url to bottom of playlist and select it",
        "\tql - show the current playlist",
        "\tqs [index] - select given entry in the playlist",
        "\tqn - select next entry in the playlist",
        "\tqd [index] - delete the given entry from the playlist",
        "\tshuffleremainingplaylist - shuffle remaining playlist entries",
        "\tshuffleentireplaylist - shuffle entire playlist and reset index to 1",
        "\tundoplaylist - undo last playlist change",
    ]
}

fn localized_local_command_help_command_lines_legacy_compatible(
    language: Option<&str>,
) -> &'static [&'static str] {
    match language {
        Some("de") => &[
            "\tr [name] - Raum wechseln",
            "\tl - Benutzerliste anzeigen",
            "\tu - letzten Suchsprung rueckgaengig machen",
            "\tp - Pause umschalten",
            "\t[s][+-]time - zur angegebenen Zeit springen; ohne + oder - ist dies eine absolute Zeit in Sekunden oder min:sec",
            "\to[+-]duration - lokale Wiedergabe relativ zur Server-Position um die angegebene Dauer verschieben (in Sekunden oder min:sec) - dies ist eine veraltete Funktion",
            "\th - diese Hilfe",
            "\tt - Bereitschaft zum Zuschauen umschalten",
            "\tsr [name] - Benutzer auf bereit setzen",
            "\tsn [name] - Benutzer auf nicht bereit setzen",
            "\tc [name] - verwalteten Raum aus dem Namen des aktuellen Raums erstellen",
            "\ta [password] - als Raumoperator mit Operator-Passwort authentifizieren",
            "\tch [message] - Chat-Nachricht im Raum senden",
            "\tqa [file/url] - Datei oder URL ans Ende der Playlist anhaengen",
            "\tqas [file/url] - Datei oder URL ans Ende der Playlist anhaengen und auswaehlen",
            "\tql - aktuelle Playlist anzeigen",
            "\tqs [index] - angegebenen Eintrag in der Playlist auswaehlen",
            "\tqn - naechsten Eintrag in der Playlist auswaehlen",
            "\tqd [index] - angegebenen Eintrag aus der Playlist loeschen",
            "\tshuffleremainingplaylist - verbleibende Playlist-Eintraege mischen",
            "\tshuffleentireplaylist - gesamte Playlist mischen und Index auf 1 zuruecksetzen",
            "\tundoplaylist - letzte Playlist-Aenderung rueckgaengig machen",
        ],
        Some("es") => &[
            "\tr [name] - cambiar de sala",
            "\tl - mostrar lista de usuarios",
            "\tu - deshacer la ultima busqueda",
            "\tp - alternar pausa",
            "\t[s][+-]time - buscar al valor de tiempo indicado; si no se especifica + o -, es tiempo absoluto en segundos o min:sec",
            "\to[+-]duration - desplazar la reproduccion local segun la duracion indicada (en segundos o min:sec) respecto a la posicion del servidor - esta es una funcion obsoleta",
            "\th - esta ayuda",
            "\tt - alterna si estas listo para ver o no",
            "\tsr [name] - marcar usuario como listo",
            "\tsn [name] - marcar usuario como no listo",
            "\tc [name] - crear sala gestionada usando el nombre de la sala actual",
            "\ta [password] - autenticarse como operador de la sala con la contrasena de operador",
            "\tch [message] - enviar un mensaje de chat en una sala",
            "\tqa [file/url] - agregar archivo o url al final de la lista de reproduccion",
            "\tqas [file/url] - agregar archivo o url al final de la lista y seleccionarlo",
            "\tql - mostrar la lista de reproduccion actual",
            "\tqs [index] - seleccionar la entrada indicada en la lista de reproduccion",
            "\tqn - seleccionar la siguiente entrada de la lista de reproduccion",
            "\tqd [index] - eliminar la entrada indicada de la lista de reproduccion",
            "\tshuffleremainingplaylist - mezclar las entradas restantes de la lista de reproduccion",
            "\tshuffleentireplaylist - mezclar toda la lista de reproduccion y restablecer el indice a 1",
            "\tundoplaylist - deshacer el ultimo cambio de la lista de reproduccion",
        ],
        Some("eo") => &[
            "\tr [name] - sxangxi cxambron",
            "\tl - montri uzantoliston",
            "\tu - malfari lastan sercxon",
            "\tp - sxalti pauzon",
            "\t[s][+-]time - salti al la donita tempo; sen + au - gxi estas absoluta tempo en sekundoj au min:sec",
            "\to[+-]duration - sxovi lokan reprodukton per la donita dauro (en sekundoj au min:sec) disde la servila pozicio - tio estas malrekomendita trajto",
            "\th - tiu helpo",
            "\tt - sxaltas cxu vi pretas spekti au ne",
            "\tsr [name] - marki uzanton preta",
            "\tsn [name] - marki uzanton ne preta",
            "\tc [name] - krei administratan cxambron uzante la nomon de la nuna cxambro",
            "\ta [password] - autentikigi kiel cxambro-operatoro per operatora pasvorto",
            "\tch [message] - sendi babilejan mesagxon en cxambro",
            "\tqa [file/url] - aldoni dosieron au url-on al la fino de la ludlisto",
            "\tqas [file/url] - aldoni dosieron au url-on al la fino de la ludlisto kaj elekti gxin",
            "\tql - montri la nunan ludliston",
            "\tqs [index] - elekti la donitan eron en la ludlisto",
            "\tqn - elekti la sekvan eron en la ludlisto",
            "\tqd [index] - forigi la donitan eron el la ludlisto",
            "\tshuffleremainingplaylist - miksi la restantajn ludlistajn erojn",
            "\tshuffleentireplaylist - miksi la tutan ludliston kaj reagordi la indekson al 1",
            "\tundoplaylist - malfari la lastan ludlistan sxangxon",
        ],
        Some("fi") => &[
            "\tr [name] - vaihda huonetta",
            "\tl - nayta kayttajalista",
            "\tu - kumoa viimeisin haku",
            "\tp - vaihda tauko",
            "\t[s][+-]time - siirry annettuun aikaan; ilman + tai - kyseessa on absoluuttinen aika sekunteina tai min:sec",
            "\to[+-]duration - siirra paikallista toistoa annetulla kestolla (sekunteina tai min:sec) palvelimen hakusijaintiin nahden - tama on vanhentunut ominaisuus",
            "\th - tama ohje",
            "\tt - vaihtaa oletko valmis katsomaan vai et",
            "\tsr [name] - merkitse kayttaja valmiiksi",
            "\tsn [name] - merkitse kayttaja ei-valmiiksi",
            "\tc [name] - luo hallittu huone nykyisen huoneen nimen perusteella",
            "\ta [password] - tunnistaudu huoneen operaattoriksi operaattorin salasanalla",
            "\tch [message] - laheta chat-viesti huoneessa",
            "\tqa [file/url] - lisaa tiedosto tai url soittolistan loppuun",
            "\tqas [file/url] - lisaa tiedosto tai url soittolistan loppuun ja valitse se",
            "\tql - nayta nykyinen soittolista",
            "\tqs [index] - valitse annettu merkinta soittolistasta",
            "\tqn - valitse seuraava merkinta soittolistasta",
            "\tqd [index] - poista annettu merkinta soittolistasta",
            "\tshuffleremainingplaylist - sekoita jaljella olevat soittolistan merkinnat",
            "\tshuffleentireplaylist - sekoita koko soittolista ja nollaa indeksi arvoon 1",
            "\tundoplaylist - kumoa viimeisin soittolistan muutos",
        ],
        Some("fr") => &[
            "\tr [name] - changer de salle",
            "\tl - afficher la liste des utilisateurs",
            "\tu - annuler le dernier seek",
            "\tp - basculer la pause",
            "\t[s][+-]time - aller a la valeur de temps indiquee ; sans + ou -, c'est un temps absolu en secondes ou min:sec",
            "\to[+-]duration - decaler la lecture locale de la duree indiquee (en secondes ou min:sec) par rapport a la position du serveur - c'est une fonctionnalite obsolete",
            "\th - cette aide",
            "\tt - basculer votre etat pret/pas pret",
            "\tsr [name] - definir l'utilisateur comme pret",
            "\tsn [name] - definir l'utilisateur comme non pret",
            "\tc [name] - creer une salle geree a partir du nom de la salle actuelle",
            "\ta [password] - s'authentifier comme operateur de salle avec le mot de passe operateur",
            "\tch [message] - envoyer un message de chat dans une salle",
            "\tqa [file/url] - ajouter un fichier ou une url en bas de la playlist",
            "\tqas [file/url] - ajouter un fichier ou une url en bas de la playlist et le selectionner",
            "\tql - afficher la playlist actuelle",
            "\tqs [index] - selectionner l'entree indiquee dans la playlist",
            "\tqn - selectionner l'entree suivante dans la playlist",
            "\tqd [index] - supprimer l'entree indiquee de la playlist",
            "\tshuffleremainingplaylist - melanger les entrees restantes de la playlist",
            "\tshuffleentireplaylist - melanger toute la playlist et reinitialiser l'index a 1",
            "\tundoplaylist - annuler la derniere modification de la playlist",
        ],
        Some("it") => &[
            "\tr [name] - cambia stanza",
            "\tl - mostra elenco utenti",
            "\tu - annulla l'ultimo seek",
            "\tp - attiva/disattiva pausa",
            "\t[s][+-]time - vai al valore di tempo indicato; se + o - non e specificato, e tempo assoluto in secondi o min:sec",
            "\to[+-]duration - sposta la riproduzione locale della durata indicata (in secondi o min:sec) rispetto alla posizione del server - questa e una funzione deprecata",
            "\th - questo aiuto",
            "\tt - alterna se sei pronto a guardare oppure no",
            "\tsr [name] - imposta utente come pronto",
            "\tsn [name] - imposta utente come non pronto",
            "\tc [name] - crea una stanza gestita usando il nome della stanza corrente",
            "\ta [password] - autenticati come operatore della stanza con la password operatore",
            "\tch [message] - invia un messaggio di chat in una stanza",
            "\tqa [file/url] - aggiungi file o url in fondo alla playlist",
            "\tqas [file/url] - aggiungi file o url in fondo alla playlist e selezionalo",
            "\tql - mostra la playlist corrente",
            "\tqs [index] - seleziona la voce indicata nella playlist",
            "\tqn - seleziona la voce successiva nella playlist",
            "\tqd [index] - elimina la voce indicata dalla playlist",
            "\tshuffleremainingplaylist - mescola le voci rimanenti della playlist",
            "\tshuffleentireplaylist - mescola l'intera playlist e reimposta l'indice a 1",
            "\tundoplaylist - annulla l'ultima modifica della playlist",
        ],
        Some("pt_PT" | "pt_BR") => &[
            "\tr [name] - mudar de sala",
            "\tl - mostrar lista de usuarios",
            "\tu - desfazer a ultima busca",
            "\tp - alternar pausa",
            "\t[s][+-]time - buscar para o valor de tempo indicado; sem + ou -, e tempo absoluto em segundos ou min:sec",
            "\to[+-]duration - deslocar a reproducao local pela duracao indicada (em segundos ou min:sec) a partir da posicao do servidor - este e um recurso obsoleto",
            "\th - esta ajuda",
            "\tt - alterna se voce esta pronto para assistir ou nao",
            "\tsr [name] - marcar usuario como pronto",
            "\tsn [name] - marcar usuario como nao pronto",
            "\tc [name] - criar sala gerenciada usando o nome da sala atual",
            "\ta [password] - autenticar como operador da sala com a senha de operador",
            "\tch [message] - enviar uma mensagem de chat na sala",
            "\tqa [file/url] - adicionar arquivo ou url ao fim da playlist",
            "\tqas [file/url] - adicionar arquivo ou url ao fim da playlist e seleciona-lo",
            "\tql - mostrar a playlist atual",
            "\tqs [index] - selecionar a entrada indicada na playlist",
            "\tqn - selecionar a proxima entrada da playlist",
            "\tqd [index] - excluir a entrada indicada da playlist",
            "\tshuffleremainingplaylist - embaralhar as entradas restantes da playlist",
            "\tshuffleentireplaylist - embaralhar toda a playlist e redefinir o indice para 1",
            "\tundoplaylist - desfazer a ultima alteracao da playlist",
        ],
        Some("tr") => &[
            "\tr [name] - oda degistir",
            "\tl - kullanici listesini goster",
            "\tu - son aramayi geri al",
            "\tp - duraklatmayi degistir",
            "\t[s][+-]time - verilen zaman degerine git; + veya - yoksa bu saniye veya min:sec cinsinden mutlak zamandir",
            "\to[+-]duration - yerel oynatimi sunucu arama konumundan verilen sure kadar kaydir (saniye veya min:sec) - bu kullanimdan kalkan bir ozelliktir",
            "\th - bu yardim",
            "\tt - izlemeye hazir olup olmadiginizi degistirir",
            "\tsr [name] - kullaniciyi hazir olarak ayarla",
            "\tsn [name] - kullaniciyi hazir degil olarak ayarla",
            "\tc [name] - guncel oda adini kullanarak yonetilen oda olustur",
            "\ta [password] - operator parolasi ile oda operatoru olarak kimlik dogrula",
            "\tch [message] - odada sohbet mesaji gonder",
            "\tqa [file/url] - dosya veya url'yi oynatma listesinin sonuna ekle",
            "\tqas [file/url] - dosya veya url'yi oynatma listesinin sonuna ekle ve sec",
            "\tql - guncel oynatma listesini goster",
            "\tqs [index] - oynatma listesindeki belirtilen girdiyi sec",
            "\tqn - oynatma listesindeki sonraki girdiyi sec",
            "\tqd [index] - oynatma listesindeki belirtilen girdiyi sil",
            "\tshuffleremainingplaylist - kalan oynatma listesi girdilerini karistir",
            "\tshuffleentireplaylist - tum oynatma listesini karistir ve indeksi 1'e sifirla",
            "\tundoplaylist - son oynatma listesi degisikligini geri al",
        ],
        Some("ru") => &[
            "\tr [name] - smenit komnatu",
            "\tl - pokazat spisok polzovatelei",
            "\tu - otmenit poslednii peremot",
            "\tp - perekliuchit pausu",
            "\t[s][+-]time - pereiti k ukazannomu vremeni; bez + ili - eto absoliutnoe vremia v sekundakh ili min:sec",
            "\to[+-]duration - smestit lokalnoe vosproizvedenie na ukazannuiu dlinu (v sekundakh ili min:sec) otnositelno pozitsii servera - eto ustarevshaia funktsiia",
            "\th - eta spravka",
            "\tt - perekliuchaet vash status gotovnosti k prosmotru",
            "\tsr [name] - ustanovit polzovatelia gotovym",
            "\tsn [name] - ustanovit polzovatelia negotovym",
            "\tc [name] - sozdat upravliaemuiu komnatu s ispolzovaniem imeni tekushchei komnaty",
            "\ta [password] - avtorizovatsia kak operator komnaty s parolem operatora",
            "\tch [message] - otpravit soobshchenie chata v komnate",
            "\tqa [file/url] - dobavit fail ili url v konets spiska vosproizvedeniia",
            "\tqas [file/url] - dobavit fail ili url v konets spiska vosproizvedeniia i vybrat ego",
            "\tql - pokazat tekushchii spisok vosproizvedeniia",
            "\tqs [index] - vybrat ukazannyi element v spiske vosproizvedeniia",
            "\tqn - vybrat sleduiushchii element v spiske vosproizvedeniia",
            "\tqd [index] - udalit ukazannyi element iz spiska vosproizvedeniia",
            "\tshuffleremainingplaylist - peremeshat ostavshiesia elementy spiska vosproizvedeniia",
            "\tshuffleentireplaylist - peremeshat ves spisok vosproizvedeniia i sbrosit indeks na 1",
            "\tundoplaylist - otmenit poslednee izmenenie spiska vosproizvedeniia",
        ],
        Some("zh_CN") => &[
            "\tr [name] - qiehuan fangjian",
            "\tl - xianshi yonghu liebiao",
            "\tu - chexiao shangci tuidong",
            "\tp - qiehuan zan ting",
            "\t[s][+-]time - tiaozhuan dao geiding shijian; ru guo meiyou + huo -, ze wei miaoshu huo min:sec de juedui shijian",
            "\to[+-]duration - xiangdui fuwuqi weizhi an geiding shichang pianyi bendi bofang (danwei wei miao huo min:sec) - zhe shi yi ge yi feiqi de gongneng",
            "\th - ci bangzhu",
            "\tt - qiehuan ni shifou zhunbei hao guankan",
            "\tsr [name] - jiang yonghu she wei yi zhunbei",
            "\tsn [name] - jiang yonghu she wei wei zhunbei",
            "\tc [name] - yong dangqian fangjian mingcheng chuangjian guanli fangjian",
            "\ta [password] - shiyong fangjian guanliyuan mima jinxing shenfen yanzheng",
            "\tch [message] - zai fangjian zhong fasong liaotian xiaoxi",
            "\tqa [file/url] - jiang wenjian huo url tianjia dao bofang liebiao diduan",
            "\tqas [file/url] - jiang wenjian huo url tianjia dao bofang liebiao diduan bing xuanze ta",
            "\tql - xianshi dangqian bofang liebiao",
            "\tqs [index] - xuanze bofang liebiao zhong de zhiding tiao",
            "\tqn - xuanze bofang liebiao zhong de xia yi tiao",
            "\tqd [index] - shanchu bofang liebiao zhong de zhiding tiao",
            "\tshuffleremainingplaylist - suiji dapaisheng yu bofang liebiao tiao mu",
            "\tshuffleentireplaylist - suiji dapaisheng zhengge bofang liebiao bing jiang suoyin chongzhi wei 1",
            "\tundoplaylist - chexiao shangci bofang liebiao genggai",
        ],
        Some("ko") => &[
            "\tr [name] - bang byeongyeong",
            "\tl - sayongja moglog pyosi",
            "\tu - majimak sigeul chwiso",
            "\tp - ilsi jeongji jeonhwan",
            "\t[s][+-]time - jijeonghan sigan-euro idong; + na - ga eopseumyeon cho ttoneun min:sec-ui jeoldae sigan-ibnida",
            "\to[+-]duration - seobeo jompeu wichi-eseo jijeonghan siganmankeum lokal jaesaeng-eul omgim (cho ttoneun min:sec) - ibeoseun deo isang gwonjangdoeji anhneun gineung-imnida",
            "\th - i doum mal",
            "\tt - sicheong junbi sangtae jeonhwan",
            "\tsr [name] - sayongjareul junbi wanlyo-ro seoljeong",
            "\tsn [name] - sayongjareul junbi an doem-euro seoljeong",
            "\tc [name] - hyeonjae bang ireumeuro gwanli bang saengseong",
            "\ta [password] - unyeongja bimillo bang unyeongja-ro inyong",
            "\tch [message] - bang-eseo chaet mesiji bonaegi",
            "\tqa [file/url] - pail ttoneun url-eul jaesaeng moglog kkeut-e chuga",
            "\tqas [file/url] - pail ttoneun url-eul jaesaeng moglog kkeut-e chuga hago seontaek",
            "\tql - hyeonjae jaesaeng moglog pyosi",
            "\tqs [index] - jaesaeng moglog-eseo jijeonghan hangmog seontaek",
            "\tqn - jaesaeng moglog-ui daeum hangmog seontaek",
            "\tqd [index] - jaesaeng moglog-eseo jijeonghan hangmog sakje",
            "\tshuffleremainingplaylist - nam-eun jaesaeng moglog hangmog seokgi",
            "\tshuffleentireplaylist - jeonche jaesaeng moglog-eul seokgo indeks-reul 1lo chogiha",
            "\tundoplaylist - majimak jaesaeng moglog byeongyeong chwiso",
        ],
        _ => local_command_help_command_lines_legacy_compatible(),
    }
}

fn local_command_help_lines_legacy_compatible(language: Option<&str>) -> Vec<String> {
    let command_lines = localized_local_command_help_command_lines_legacy_compatible(language);
    let mut lines = Vec::with_capacity(command_lines.len() + 1);
    lines.push(localized_local_command_help_heading_legacy_compatible(language).to_owned());
    lines.extend(command_lines.iter().copied().map(str::to_owned));
    lines
}

fn localized_syncplay_version_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Syncplay-Version",
        Some("es") => "Version de Syncplay",
        Some("eo") => "Versio de Syncplay",
        Some("fi") => "Syncplay-versio",
        Some("fr") => "Version de Syncplay",
        Some("it") => "Versione di Syncplay",
        Some("pt_PT" | "pt_BR") => "Versao do Syncplay",
        Some("tr") => "Syncplay surumu",
        Some("ru") => "Versiia Syncplay",
        Some("zh_CN") => "Syncplay banben",
        Some("ko") => "Syncplay beojeon",
        _ => "Syncplay version",
    }
}

fn localized_more_info_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Mehr Informationen unter",
        Some("es") => "Mas informacion en",
        Some("eo") => "Pli da informoj che",
        Some("fi") => "Lisatietoja osoitteessa",
        Some("fr") => "Plus d'informations sur",
        Some("it") => "Maggiori informazioni su",
        Some("pt_PT" | "pt_BR") => "Mais informacoes em",
        Some("tr") => "Daha fazla bilgi",
        Some("ru") => "Bolshe informatsii na",
        Some("zh_CN") => "Geng duo xinxi qing fangwen",
        Some("ko") => "Deo maneun jeongbo",
        _ => "More info available at",
    }
}

fn local_command_help_footer_lines_legacy_compatible(
    language: Option<&str>,
    version: &str,
) -> [String; 2] {
    [
        format!(
            "{}: {version}",
            localized_syncplay_version_prefix_legacy_compatible(language)
        ),
        format!(
            "{}: {PROJECT_URL_LEGACY}",
            localized_more_info_prefix_legacy_compatible(language)
        ),
    ]
}

pub fn render_local_input_display_lines_legacy_compatible(
    dispatch: &PlannedLocalInputDispatch,
    session: &ClientSession,
    language: Option<&str>,
    version: &str,
) -> Option<Vec<String>> {
    match dispatch {
        PlannedLocalInputDispatch::Suppressed | PlannedLocalInputDispatch::Run(_) => None,
        PlannedLocalInputDispatch::EmitUnknownCommandHelp => {
            let mut lines = Vec::with_capacity(1 + 1 + 22 + 2);
            lines.push(localized_unknown_command_message_legacy_compatible(language).to_owned());
            lines.extend(local_command_help_lines_legacy_compatible(language));
            lines.extend(local_command_help_footer_lines_legacy_compatible(
                language, version,
            ));
            Some(lines)
        }
        PlannedLocalInputDispatch::EmitHelp => {
            let mut lines = local_command_help_lines_legacy_compatible(language);
            lines.extend(local_command_help_footer_lines_legacy_compatible(
                language, version,
            ));
            Some(lines)
        }
        PlannedLocalInputDispatch::EmitError(error_kind) => {
            Some(vec![local_input_error_output_line_legacy_compatible(
                *error_kind,
                language,
            )])
        }
        PlannedLocalInputDispatch::EmitPlaylist => {
            Some(vec![playlist_listing_message_localized_legacy_compatible(
                session, language,
            )])
        }
    }
}

pub fn localized_current_offset_message_legacy_compatible(
    offset_seconds: f64,
    language: Option<&str>,
) -> String {
    match language {
        Some("de") => format!("Aktueller Versatz: {offset_seconds} Sekunden"),
        Some("es") => format!("Desfase actual: {offset_seconds} segundos"),
        Some("eo") => format!("Nuna kompenso: {offset_seconds} sekundoj"),
        Some("fi") => format!("Nykyinen siirtyma: {offset_seconds} sekuntia"),
        Some("fr") => format!("Decalage actuel : {offset_seconds} secondes"),
        Some("it") => format!("Offset attuale: {offset_seconds} secondi"),
        Some("pt_PT" | "pt_BR") => format!("Deslocamento atual: {offset_seconds} segundos"),
        Some("tr") => format!("Guncel kaydirma: {offset_seconds} saniye"),
        Some("ru") => format!("Tekushchee smeshchenie: {offset_seconds} sekund"),
        Some("zh_CN") => format!("Dangqian pianyi: {offset_seconds} miao"),
        Some("ko") => format!("Hyeonjae opeuset: {offset_seconds} cho"),
        _ => format!("Current offset: {offset_seconds} seconds"),
    }
}

fn localized_playlist_empty_message_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Playlist ist derzeit leer.",
        Some("es") => "La lista de reproduccion esta vacia.",
        Some("eo") => "La ludlisto estas malplena.",
        Some("fi") => "Soittolista on tyhja.",
        Some("fr") => "La playlist est actuellement vide.",
        Some("it") => "La playlist e attualmente vuota.",
        Some("pt_PT" | "pt_BR") => "A playlist esta vazia no momento.",
        Some("tr") => "Oynatma listesi su anda bos.",
        Some("ru") => "Spisok vosproizvedeniia seichas pust.",
        Some("zh_CN") => "Bofang liebiao muqian weikong.",
        Some("ko") => "Jaesaeng moglog-i hyeonjae bi-eo issseumnida.",
        _ => PLAYLIST_EMPTY_MESSAGE_LEGACY,
    }
}

pub fn playlist_listing_message_legacy_compatible(session: &ClientSession) -> String {
    let Some(playlist) = session.current_room_playlist() else {
        return PLAYLIST_EMPTY_MESSAGE_LEGACY.to_owned();
    };
    if playlist.files.is_empty() {
        return PLAYLIST_EMPTY_MESSAGE_LEGACY.to_owned();
    }

    let mut playlist_elements: Vec<String> = playlist
        .files
        .iter()
        .enumerate()
        .map(|(index, file_name)| format!("\t{}: {}", index + 1, file_name))
        .collect();
    if let Some(selected_index) = playlist.index.and_then(|index| usize::try_from(index).ok()) {
        if selected_index < playlist_elements.len() {
            playlist_elements[selected_index] = format!(" *{}", playlist_elements[selected_index]);
        }
    }
    playlist_elements.join("\n")
}

pub fn playlist_listing_message_localized_legacy_compatible(
    session: &ClientSession,
    language: Option<&str>,
) -> String {
    let Some(playlist) = session.current_room_playlist() else {
        return localized_playlist_empty_message_legacy_compatible(language).to_owned();
    };
    if playlist.files.is_empty() {
        return localized_playlist_empty_message_legacy_compatible(language).to_owned();
    }
    playlist_listing_message_legacy_compatible(session)
}

pub fn plan_local_offset_runtime_dispatch_legacy_compatible(
    current_user_offset_seconds: f64,
    global_position_seconds: f64,
    command: &LocalOffsetCommand,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    let updated_user_offset_seconds = resolved_local_user_offset_seconds_legacy_compatible(
        current_user_offset_seconds,
        global_position_seconds,
        command,
    );
    PlannedLocalRuntimeDispatch {
        line_to_emit: Some(localized_current_offset_message_legacy_compatible(
            updated_user_offset_seconds,
            language,
        )),
        action: Some(PlannedLocalRuntimeAction::SeekToPosition(
            global_position_seconds + updated_user_offset_seconds,
        )),
        updated_user_offset_seconds: Some(updated_user_offset_seconds),
    }
}

fn plan_local_playlist_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
    action: PlannedLocalRuntimeAction,
) -> PlannedLocalRuntimeDispatch {
    if !playlist_index_in_bounds_legacy_compatible(session, index) {
        return PlannedLocalRuntimeDispatch {
            line_to_emit: Some(local_input_error_output_line_legacy_compatible(
                LocalInputCommandErrorKind::PlaylistInvalidIndex,
                language,
            )),
            action: None,
            updated_user_offset_seconds: None,
        };
    }

    PlannedLocalRuntimeDispatch {
        line_to_emit: None,
        action: Some(action),
        updated_user_offset_seconds: None,
    }
}

pub fn plan_local_playlist_select_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch_legacy_compatible(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::SetPlaylistIndex(index),
    )
}

pub fn plan_local_playlist_delete_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch_legacy_compatible(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index),
    )
}

pub fn plan_local_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    current_user_offset_seconds: f64,
    action: PlannedLocalRuntimeAction,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    match action {
        PlannedLocalRuntimeAction::SetUserOffset(command) => {
            let global_position_seconds = session
                .current_room_playstate()
                .and_then(|playstate| playstate.position)
                .unwrap_or(0.0);
            plan_local_offset_runtime_dispatch_legacy_compatible(
                current_user_offset_seconds,
                global_position_seconds,
                &command,
                language,
            )
        }
        PlannedLocalRuntimeAction::SetPlaylistIndex(index) => {
            plan_local_playlist_select_runtime_dispatch_legacy_compatible(session, index, language)
        }
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index) => {
            plan_local_playlist_delete_runtime_dispatch_legacy_compatible(session, index, language)
        }
        action => PlannedLocalRuntimeDispatch {
            line_to_emit: None,
            action: Some(action),
            updated_user_offset_seconds: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
        LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
        PlannedLocalRuntimeAction, controlled_room_base_name_legacy_compatible,
        local_command_help_footer_lines_legacy_compatible,
        local_command_help_lines_legacy_compatible,
        local_input_error_output_line_legacy_compatible,
        localized_current_offset_message_legacy_compatible,
        localized_local_input_error_message_legacy_compatible,
        localized_unknown_command_message_legacy_compatible, parse_local_input_chat_message,
        parse_local_input_command, plan_local_input_command_legacy_compatible,
        plan_local_input_dispatch_legacy_compatible,
        plan_local_offset_runtime_dispatch_legacy_compatible,
        plan_local_playlist_delete_runtime_dispatch_legacy_compatible,
        plan_local_playlist_select_runtime_dispatch_legacy_compatible,
        plan_local_runtime_dispatch_legacy_compatible, playlist_index_in_bounds_legacy_compatible,
        playlist_listing_message_legacy_compatible,
        playlist_listing_message_localized_legacy_compatible,
        render_local_input_display_lines_legacy_compatible,
        resolved_local_user_offset_seconds_legacy_compatible,
    };

    #[test]
    fn parse_local_input_chat_message_recognizes_legacy_aliases() {
        assert_eq!(
            parse_local_input_chat_message("chat hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(parse_local_input_chat_message("ch"), Some(String::new()));
        assert_eq!(parse_local_input_chat_message(" hello everyone"), None);
        assert_eq!(parse_local_input_chat_message("/chat hello everyone"), None);
    }

    #[test]
    fn parse_local_input_command_parses_common_toggle_and_room_commands() {
        assert_eq!(
            parse_local_input_command("toggle"),
            Some(LocalInputCommand::ToggleReady)
        );
        assert_eq!(
            parse_local_input_command("room room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("room "),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
    }

    #[test]
    fn parse_local_input_command_parses_seek_and_offset_variants() {
        assert_eq!(
            parse_local_input_command("s+0:10"),
            Some(LocalInputCommand::SeekRelative(10.0))
        );
        assert_eq!(
            parse_local_input_command("offset /0:30"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::RelativeFromCurrentPositionMinus(30.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
    }

    #[test]
    fn parse_local_input_command_rejects_slash_and_tab_variants() {
        assert_eq!(
            parse_local_input_command("/queue episode1.mkv"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("queue\tepisode1.mkv"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(parse_local_input_command(" hello everyone"), None);
    }

    #[test]
    fn controlled_room_base_name_legacy_compatible_strips_managed_suffix() {
        assert_eq!(
            controlled_room_base_name_legacy_compatible("+base-room:ABCDEF123456"),
            "base-room".to_owned()
        );
        assert_eq!(
            controlled_room_base_name_legacy_compatible("+room:SHORT"),
            "+room:SHORT".to_owned()
        );
    }

    #[test]
    fn plan_local_input_command_legacy_compatible_resolves_special_room_flows() {
        let context = LocalInputCommandPlanningContext {
            current_room: Some("+watch-party:ABCDEF123456"),
            configured_room: "fallback-room",
        };

        let created = plan_local_input_command_legacy_compatible(
            LocalInputCommand::CreateControlledRoom(None),
            &context,
        );
        let PlannedLocalInputCommand::RequestControllerAuth { room, password } = created else {
            panic!("expected controller auth request");
        };
        assert_eq!(room, "watch-party");
        assert_eq!(password.len(), 10);

        let auth = plan_local_input_command_legacy_compatible(
            LocalInputCommand::AuthController("pw".to_owned()),
            &context,
        );
        assert_eq!(
            auth,
            PlannedLocalInputCommand::RequestControllerAuth {
                room: "+watch-party:ABCDEF123456".to_owned(),
                password: "pw".to_owned(),
            }
        );

        assert_eq!(
            plan_local_input_command_legacy_compatible(
                LocalInputCommand::SetRoomWithLegacyFallback,
                &LocalInputCommandPlanningContext {
                    current_room: None,
                    configured_room: "fallback-room",
                },
            ),
            PlannedLocalInputCommand::SetRoomWithLegacyFallback("fallback-room".to_owned())
        );
    }

    #[test]
    fn planned_local_input_command_uses_shared_playlists_matches_playlist_commands() {
        assert!(PlannedLocalInputCommand::ShowPlaylist.uses_shared_playlists());
        assert!(PlannedLocalInputCommand::SelectPlaylistIndex(1).uses_shared_playlists());
        assert!(!PlannedLocalInputCommand::ToggleReady.uses_shared_playlists());
        assert_eq!(
            plan_local_input_command_legacy_compatible(
                LocalInputCommand::ShowQueueMissingFileError,
                &LocalInputCommandPlanningContext {
                    current_room: None,
                    configured_room: "fallback-room",
                },
            ),
            PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::QueueMissingFile)
        );
    }

    #[test]
    fn resolved_local_user_offset_seconds_legacy_compatible_applies_all_modes() {
        assert_eq!(
            resolved_local_user_offset_seconds_legacy_compatible(
                5.0,
                100.0,
                &LocalOffsetCommand::Absolute(12.0),
            ),
            12.0
        );
        assert_eq!(
            resolved_local_user_offset_seconds_legacy_compatible(
                5.0,
                100.0,
                &LocalOffsetCommand::Relative(3.0),
            ),
            8.0
        );
        assert_eq!(
            resolved_local_user_offset_seconds_legacy_compatible(
                5.0,
                100.0,
                &LocalOffsetCommand::RelativeFromCurrentPositionMinus(90.0),
            ),
            15.0
        );
    }

    #[test]
    fn playlist_index_in_bounds_legacy_compatible_matches_current_room_playlist() {
        let mut session = syncplay_client_core::ClientSession::default();
        assert!(!playlist_index_in_bounds_legacy_compatible(&session, 0));

        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should set the current room");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");

        assert!(playlist_index_in_bounds_legacy_compatible(&session, 0));
        assert!(playlist_index_in_bounds_legacy_compatible(&session, 1));
        assert!(!playlist_index_in_bounds_legacy_compatible(&session, 2));
        assert!(!playlist_index_in_bounds_legacy_compatible(&session, -1));
    }

    #[test]
    fn playlist_listing_message_legacy_compatible_formats_entries_and_empty_states() {
        let mut session = syncplay_client_core::ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(
            playlist_listing_message_legacy_compatible(&session),
            "Playlist is currently empty."
        );

        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        assert_eq!(
            playlist_listing_message_legacy_compatible(&session),
            "\t1: episode1.mkv\n *\t2: episode2.mkv"
        );
    }

    #[test]
    fn playlist_listing_message_localized_legacy_compatible_uses_localized_empty_message() {
        let mut session = syncplay_client_core::ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(
            playlist_listing_message_localized_legacy_compatible(&session, Some("fr")),
            "La playlist est actuellement vide."
        );
        assert_eq!(
            playlist_listing_message_localized_legacy_compatible(&session, Some("de")),
            "Playlist ist derzeit leer."
        );
    }

    #[test]
    fn localized_local_input_error_message_legacy_compatible_localizes_known_messages() {
        assert_eq!(
            localized_local_input_error_message_legacy_compatible(
                LocalInputCommandErrorKind::PlaylistInvalidIndex,
                Some("es"),
            ),
            "Indice de lista de reproduccion no valido"
        );
        assert_eq!(
            localized_local_input_error_message_legacy_compatible(
                LocalInputCommandErrorKind::QueueMissingFile,
                Some("de"),
            ),
            "Keine Datei/URL angegeben"
        );
        assert_eq!(
            localized_local_input_error_message_legacy_compatible(
                LocalInputCommandErrorKind::QueueMissingFile,
                None,
            ),
            "No file/url given"
        );
    }

    #[test]
    fn local_input_error_output_line_legacy_compatible_formats_prefix_and_message() {
        assert_eq!(
            local_input_error_output_line_legacy_compatible(
                LocalInputCommandErrorKind::PlaylistInvalidIndex,
                Some("de"),
            ),
            "FEHLER:\tUngueltiger Playlist-Index"
        );
        assert_eq!(
            local_input_error_output_line_legacy_compatible(
                LocalInputCommandErrorKind::QueueMissingFile,
                None,
            ),
            "ERROR:\tNo file/url given"
        );
    }

    #[test]
    fn local_command_help_lines_legacy_compatible_include_expected_entries() {
        let lines = local_command_help_lines_legacy_compatible(None);
        assert_eq!(
            lines.first().map(String::as_str),
            Some("Available commands:")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\tql - show the current playlist"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\tqd [index] - delete the given entry"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\tundoplaylist - undo last playlist change"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\to[+-]duration - offset local playback"))
        );
    }

    #[test]
    fn local_command_help_lines_legacy_compatible_localize_heading_and_body() {
        let lines = local_command_help_lines_legacy_compatible(Some("fr"));
        assert_eq!(
            lines.first().map(String::as_str),
            Some("Commandes disponibles:")
        );
        assert!(lines.iter().any(|line| line == "\th - cette aide"));
        assert!(
            lines
                .iter()
                .any(|line| line == "\tql - afficher la playlist actuelle")
        );
    }

    #[test]
    fn local_command_help_footer_lines_legacy_compatible_include_expected_entries() {
        let lines = local_command_help_footer_lines_legacy_compatible(Some("de"), "1.7.5");
        assert_eq!(lines[0], "Syncplay-Version: 1.7.5");
        assert_eq!(lines[1], "Mehr Informationen unter: https://syncplay.pl/");
    }

    #[test]
    fn localized_unknown_command_message_legacy_compatible_uses_selected_language() {
        assert_eq!(
            localized_unknown_command_message_legacy_compatible(Some("es")),
            "Comando no reconocido"
        );
        assert_eq!(
            localized_unknown_command_message_legacy_compatible(None),
            "Unrecognized command"
        );
    }

    #[test]
    fn localized_current_offset_message_legacy_compatible_localizes_user_visible_message() {
        assert_eq!(
            localized_current_offset_message_legacy_compatible(2.5, Some("pt_BR")),
            "Deslocamento atual: 2.5 segundos"
        );
        assert_eq!(
            localized_current_offset_message_legacy_compatible(-1.0, None),
            "Current offset: -1 seconds"
        );
    }

    #[test]
    fn plan_local_offset_runtime_dispatch_legacy_compatible_emits_seek_and_status_line() {
        let dispatch = plan_local_offset_runtime_dispatch_legacy_compatible(
            5.0,
            100.0,
            &LocalOffsetCommand::Relative(3.0),
            Some("es"),
        );
        assert_eq!(dispatch.updated_user_offset_seconds, Some(8.0));
        assert_eq!(
            dispatch.line_to_emit.as_deref(),
            Some("Desfase actual: 8 segundos")
        );
        assert_eq!(
            dispatch.action,
            Some(PlannedLocalRuntimeAction::SeekToPosition(108.0))
        );
    }

    #[test]
    fn plan_local_input_dispatch_legacy_compatible_maps_and_suppresses_commands() {
        assert_eq!(
            plan_local_input_dispatch_legacy_compatible(PlannedLocalInputCommand::ShowHelp, true,),
            PlannedLocalInputDispatch::EmitHelp
        );
        assert_eq!(
            plan_local_input_dispatch_legacy_compatible(
                PlannedLocalInputCommand::ShowPlaylist,
                false,
            ),
            PlannedLocalInputDispatch::Suppressed
        );
        assert_eq!(
            plan_local_input_dispatch_legacy_compatible(
                PlannedLocalInputCommand::SendChat("hello".to_owned()),
                true,
            ),
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SendChat("hello".to_owned()))
        );
        assert_eq!(
            plan_local_input_dispatch_legacy_compatible(
                PlannedLocalInputCommand::SetRoomWithLegacyFallback("room2".to_owned()),
                true,
            ),
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(
                "room2".to_owned()
            ))
        );
    }

    #[test]
    fn render_local_input_display_lines_legacy_compatible_renders_unknown_help_and_playlist() {
        let mut session = syncplay_client_core::ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");

        let unknown_lines = render_local_input_display_lines_legacy_compatible(
            &PlannedLocalInputDispatch::EmitUnknownCommandHelp,
            &session,
            Some("es"),
            "1.7.5",
        )
        .expect("unknown command should render lines");
        assert_eq!(
            unknown_lines.first().map(String::as_str),
            Some("Comando no reconocido")
        );
        assert!(
            unknown_lines
                .iter()
                .any(|line| line == "Comandos disponibles:")
        );
        assert!(
            unknown_lines
                .iter()
                .any(|line| line == "Version de Syncplay: 1.7.5")
        );

        let playlist_lines = render_local_input_display_lines_legacy_compatible(
            &PlannedLocalInputDispatch::EmitPlaylist,
            &session,
            Some("de"),
            "1.7.5",
        )
        .expect("playlist should render lines");
        assert_eq!(
            playlist_lines,
            vec!["Playlist ist derzeit leer.".to_owned()]
        );
    }

    #[test]
    fn plan_local_playlist_runtime_dispatch_legacy_compatible_handles_valid_and_invalid_indices() {
        let mut session = syncplay_client_core::ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should set the current room");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");

        let invalid_dispatch =
            plan_local_playlist_select_runtime_dispatch_legacy_compatible(&session, 5, Some("fr"));
        assert_eq!(
            invalid_dispatch.line_to_emit.as_deref(),
            Some("ERREUR:\tIndice de playlist non valide")
        );
        assert_eq!(invalid_dispatch.action, None);

        let valid_dispatch =
            plan_local_playlist_delete_runtime_dispatch_legacy_compatible(&session, 1, Some("fr"));
        assert_eq!(valid_dispatch.line_to_emit, None);
        assert_eq!(
            valid_dispatch.action,
            Some(PlannedLocalRuntimeAction::DeletePlaylistIndex(1))
        );
    }

    #[test]
    fn plan_local_runtime_dispatch_legacy_compatible_promotes_special_cases() {
        let offset_dispatch = plan_local_runtime_dispatch_legacy_compatible(
            &syncplay_client_core::ClientSession::default(),
            5.0,
            PlannedLocalRuntimeAction::SetUserOffset(LocalOffsetCommand::Relative(3.0)),
            Some("en"),
        );
        assert_eq!(offset_dispatch.updated_user_offset_seconds, Some(8.0));
        assert_eq!(
            offset_dispatch.action,
            Some(PlannedLocalRuntimeAction::SeekToPosition(8.0))
        );

        let mut session = syncplay_client_core::ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        let playlist_dispatch = plan_local_runtime_dispatch_legacy_compatible(
            &session,
            0.0,
            PlannedLocalRuntimeAction::SetPlaylistIndex(3),
            Some("fr"),
        );
        assert_eq!(playlist_dispatch.action, None);
        assert_eq!(
            playlist_dispatch.line_to_emit.as_deref(),
            Some("ERREUR:\tIndice de playlist non valide")
        );
    }

    #[test]
    fn plan_local_runtime_dispatch_legacy_compatible_passthroughs_simple_actions() {
        let dispatch = plan_local_runtime_dispatch_legacy_compatible(
            &syncplay_client_core::ClientSession::default(),
            0.0,
            PlannedLocalRuntimeAction::TogglePause,
            Some("de"),
        );
        assert_eq!(dispatch.line_to_emit, None);
        assert_eq!(dispatch.updated_user_offset_seconds, None);
        assert_eq!(
            dispatch.action,
            Some(PlannedLocalRuntimeAction::TogglePause)
        );
    }
}
