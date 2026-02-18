use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, AutoplayCountdownNotification, ChatNotification, ClientRuntime,
    ClientSession, ControllerAuthTransitionNotification, FileDifferenceSummary, PrivacyMode,
    QueuedRuntimeControl, ReadinessAutoplayConfig, ReconnectTransitionNotification,
    UnpauseActionMode, UserChangeNotification,
};
use syncplay_player_mpv::MpvAdapter;
use syncplay_protocol::{ProtocolMessage, encode_message_line};
use syncplay_server::ServerApp;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::Instant;

const ROUND_HALF_EPSILON: f64 = 1e-12;
const CONTROL_ROOM_HASH_LEN: usize = 12;
const PLAYLIST_EMPTY_MESSAGE_LEGACY: &str = "Playlist is currently empty.";
const UNKNOWN_COMMAND_MESSAGE_LEGACY: &str = "Unrecognized command";
const PLAYLIST_INVALID_INDEX_ERROR_LEGACY: &str = "Invalid playlist index";
const QUEUE_MISSING_FILE_ERROR_LEGACY: &str = "No file/url given";
const PROJECT_URL_LEGACY: &str = "https://syncplay.pl/";
static ROOM_PASSWORD_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct ClientLoopConfig {
    host: String,
    port: u16,
    username: String,
    room: String,
    version: String,
    max_retries: u32,
    max_connected_runtime_seconds: f64,
    readiness_supported_override: Option<bool>,
    local_can_control_override: Option<bool>,
    is_playing_music_override: Option<bool>,
    recently_advanced_override: Option<bool>,
    autoplay_enabled: bool,
    autoplay_require_same_filenames: bool,
    filename_privacy_mode: PrivacyMode,
    filesize_privacy_mode: PrivacyMode,
    show_duration_notification_override: Option<bool>,
    different_duration_threshold_seconds_override: Option<f64>,
    show_same_room_osd_override: Option<bool>,
    show_osd_warnings_override: Option<bool>,
    show_noncontroller_osd_override: Option<bool>,
    show_different_room_osd_override: Option<bool>,
    controlled_room_password_override: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeLoopInputs {
    readiness_supported: bool,
    local_can_control: bool,
    is_playing_music: bool,
    recently_advanced: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ClientBehaviorOverrides {
    pause_on_leave: Option<bool>,
    loop_at_end_of_playlist: Option<bool>,
    loop_single_files: Option<bool>,
    only_switch_to_trusted_domains: Option<bool>,
    trusted_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoplayThresholdOverride {
    Disable,
    Set(usize),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ReadinessAutoplayOverrides {
    unpause_action: Option<UnpauseActionMode>,
    auto_play_threshold: Option<AutoplayThresholdOverride>,
    autoplay_delay_seconds: Option<f64>,
    last_paused_diff_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChatPolicyOverrides {
    max_chat_message_length: Option<usize>,
    apply_server_max_chat_message_length: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectedSessionExit {
    RuntimeWindowElapsed,
    TransportClosed,
}

#[derive(Debug, Default)]
struct FileDifferenceNotificationState {
    last_summary: Option<String>,
}

fn env_flag_enabled(name: &str) -> bool {
    env_trimmed(name)
        .and_then(|value| parse_env_bool_legacy_compatible(&value))
        .unwrap_or(false)
}

fn env_trimmed(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn env_flag_override(name: &str) -> Option<bool> {
    env_trimmed(name).and_then(|value| parse_env_bool_legacy_compatible(&value))
}

fn parse_env_bool_legacy_compatible(value: &str) -> Option<bool> {
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

fn parse_env_port_legacy_compatible(value: &str) -> Option<u16> {
    let port = value.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

fn env_port(name: &str) -> Option<u16> {
    env_trimmed(name).and_then(|value| parse_env_port_legacy_compatible(&value))
}

fn parse_env_string_list_legacy_compatible(value: &str) -> Option<Vec<String>> {
    let values: Vec<String> = value
        .split([',', ';', '\n', '\r'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn env_string_list(name: &str) -> Option<Vec<String>> {
    env_trimmed(name).and_then(|value| parse_env_string_list_legacy_compatible(&value))
}

fn behavior_overrides_from_env() -> ClientBehaviorOverrides {
    ClientBehaviorOverrides {
        pause_on_leave: env_flag_override("SYNCPLAY_CLIENT_PAUSE_ON_LEAVE"),
        loop_at_end_of_playlist: env_flag_override("SYNCPLAY_CLIENT_LOOP_AT_END_OF_PLAYLIST"),
        loop_single_files: env_flag_override("SYNCPLAY_CLIENT_LOOP_SINGLE_FILES"),
        only_switch_to_trusted_domains: env_flag_override(
            "SYNCPLAY_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS",
        ),
        trusted_domains: env_string_list("SYNCPLAY_CLIENT_TRUSTED_DOMAINS"),
    }
}

fn parse_unpause_action_mode_legacy_compatible(value: &str) -> Option<UnpauseActionMode> {
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

fn parse_autoplay_min_users_override_legacy_compatible(
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

fn readiness_overrides_from_env() -> ReadinessAutoplayOverrides {
    ReadinessAutoplayOverrides {
        unpause_action: env_trimmed("SYNCPLAY_CLIENT_UNPAUSE_ACTION")
            .and_then(|value| parse_unpause_action_mode_legacy_compatible(&value)),
        auto_play_threshold: env_trimmed("SYNCPLAY_CLIENT_AUTOPLAY_MIN_USERS")
            .and_then(|value| parse_autoplay_min_users_override_legacy_compatible(&value)),
        autoplay_delay_seconds: env_non_negative_f64("SYNCPLAY_CLIENT_AUTOPLAY_DELAY_SECONDS"),
        last_paused_diff_threshold_seconds: env_non_negative_f64(
            "SYNCPLAY_CLIENT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS",
        ),
    }
}

fn apply_readiness_autoplay_overrides(
    readiness_config: &mut ReadinessAutoplayConfig,
    overrides: &ReadinessAutoplayOverrides,
) {
    if let Some(unpause_action) = overrides.unpause_action.clone() {
        readiness_config.unpause_action = unpause_action;
    }
    if let Some(auto_play_threshold) = overrides.auto_play_threshold.as_ref() {
        readiness_config.auto_play_threshold = match auto_play_threshold {
            AutoplayThresholdOverride::Disable => None,
            AutoplayThresholdOverride::Set(value) => Some(*value),
        };
    }
    if let Some(autoplay_delay_seconds) = overrides.autoplay_delay_seconds {
        readiness_config.autoplay_delay_seconds = autoplay_delay_seconds;
    }
    if let Some(last_paused_diff_threshold_seconds) = overrides.last_paused_diff_threshold_seconds {
        readiness_config.last_paused_diff_threshold_seconds = last_paused_diff_threshold_seconds;
    }
}

fn chat_policy_overrides_from_env() -> ChatPolicyOverrides {
    ChatPolicyOverrides {
        max_chat_message_length: env_usize("SYNCPLAY_CLIENT_CHAT_MAX_LENGTH"),
        apply_server_max_chat_message_length: env_flag_override(
            "SYNCPLAY_CLIENT_APPLY_SERVER_CHAT_MAX_LENGTH",
        ),
    }
}

fn apply_chat_policy_overrides(session: &mut ClientSession, overrides: &ChatPolicyOverrides) {
    let chat_config = session.chat_config_mut();
    if let Some(max_chat_message_length) = overrides.max_chat_message_length {
        chat_config.max_chat_message_length = max_chat_message_length;
        if overrides.apply_server_max_chat_message_length.is_none() {
            chat_config.apply_server_max_chat_message_length = false;
        }
    }
    if let Some(apply_server_max_chat_message_length) =
        overrides.apply_server_max_chat_message_length
    {
        chat_config.apply_server_max_chat_message_length = apply_server_max_chat_message_length;
    }
}

fn apply_client_behavior_overrides(
    session: &mut ClientSession,
    overrides: &ClientBehaviorOverrides,
) {
    let behavior = session.behavior_config_mut();
    if let Some(pause_on_leave) = overrides.pause_on_leave {
        behavior.pause_on_leave = pause_on_leave;
    }
    if let Some(loop_at_end_of_playlist) = overrides.loop_at_end_of_playlist {
        behavior.loop_at_end_of_playlist = loop_at_end_of_playlist;
    }
    if let Some(loop_single_files) = overrides.loop_single_files {
        behavior.loop_single_files = loop_single_files;
    }
    if let Some(only_switch_to_trusted_domains) = overrides.only_switch_to_trusted_domains {
        behavior.only_switch_to_trusted_domains = only_switch_to_trusted_domains;
    }
    if let Some(trusted_domains) = overrides.trusted_domains.clone() {
        behavior.trusted_domains = trusted_domains;
    }
}

fn env_u32(name: &str) -> Option<u32> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn parse_env_non_negative_f64_legacy_compatible(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn env_non_negative_f64(name: &str) -> Option<f64> {
    env_trimmed(name).and_then(|value| parse_env_non_negative_f64_legacy_compatible(&value))
}

fn env_privacy_mode(name: &str) -> Option<PrivacyMode> {
    env_trimmed(name).and_then(|value| PrivacyMode::from_legacy_name(value.as_str()))
}

fn normalize_controlled_room_input(room: String) -> (String, Option<String>) {
    if !room.starts_with('+') {
        return (room, None);
    }

    let mut parts = room.split(':');
    let Some(base_name) = parts.next() else {
        return (room, None);
    };
    let Some(hash_suffix) = parts.next() else {
        return (room, None);
    };
    let Some(password) = parts.next() else {
        return (room, None);
    };

    let normalized_password = password
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let canonical_room = format!("{base_name}:{hash_suffix}");
    if normalized_password.is_empty() {
        return (canonical_room, None);
    }
    (canonical_room, Some(normalized_password))
}

fn controlled_room_base_name_legacy_compatible(room: &str) -> String {
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

fn generate_room_password_legacy_compatible() -> String {
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

fn parse_local_input_chat_message(input: &str) -> Option<String> {
    if input.starts_with(' ') {
        return None;
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    for alias in ["chat", "ch", "/chat", "/ch", "/msg"] {
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

    if trimmed.starts_with('/') {
        return None;
    }

    let command_token = trimmed.split_whitespace().next().unwrap_or_default();
    if is_known_local_command_token_legacy_compatible(command_token) {
        return None;
    }

    Some(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq)]
enum LocalOffsetCommand {
    Absolute(f64),
    Relative(f64),
    RelativeFromCurrentPositionMinus(f64),
}

#[derive(Debug, Clone, PartialEq)]
enum LocalInputCommand {
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

fn parse_create_command_legacy_compatible(input: &str) -> Option<Option<String>> {
    for alias in ["create", "c", "/create", "/c"] {
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
    for alias in ["room", "r", "/room", "/r"] {
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

fn parse_seek_time_seconds_legacy_like(value: &str) -> Option<f64> {
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

fn parse_local_input_command(input: &str) -> Option<LocalInputCommand> {
    if input.starts_with(' ') {
        return None;
    }

    let trimmed = input.trim();
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["help", "h", "?", "/help", "/h", "/?", "\\?"],
    ) {
        return Some(LocalInputCommand::ShowHelp);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["undoplaylist", "/undoplaylist"]) {
        return Some(LocalInputCommand::UndoPlaylistChange);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["shuffleremainingplaylist", "/shuffleremainingplaylist"],
    ) {
        return Some(LocalInputCommand::ShuffleRemainingPlaylist);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["shuffleentireplaylist", "/shuffleentireplaylist"],
    ) {
        return Some(LocalInputCommand::ShuffleEntirePlaylist);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["undo", "u", "revert", "/undo", "/u", "/revert"],
    ) {
        return Some(LocalInputCommand::UndoSeek);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["list", "l", "users", "/list", "/l", "/users"],
    ) {
        return Some(LocalInputCommand::RequestUserList);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["playlist", "ql", "pl", "/playlist", "/ql", "/pl"],
    ) {
        return Some(LocalInputCommand::ShowPlaylist);
    }
    if let Some(index) = trimmed
        .strip_prefix("select ")
        .or_else(|| trimmed.strip_prefix("qs "))
        .or_else(|| trimmed.strip_prefix("/select "))
        .or_else(|| trimmed.strip_prefix("/qs "))
    {
        return parse_playlist_index_parameter_legacy_compatible(index)
            .map(LocalInputCommand::SelectPlaylistIndex)
            .or(Some(LocalInputCommand::ShowPlaylistInvalidIndexError));
    }
    if matches!(trimmed, "select" | "qs" | "/select" | "/qs") {
        return Some(LocalInputCommand::ShowPlaylistInvalidIndexError);
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["next", "qn", "/next", "/qn"]) {
        return Some(LocalInputCommand::NextPlaylistItem);
    }
    if let Some(command) = parse_queue_command_legacy_compatible(
        input,
        &["queueandselect", "qas", "/queueandselect", "/qas"],
        true,
    ) {
        return Some(command);
    }
    if let Some(command) = parse_queue_command_legacy_compatible(
        input,
        &["queue", "qa", "add", "/queue", "/qa", "/add"],
        false,
    ) {
        return Some(command);
    }
    if let Some(index) = trimmed
        .strip_prefix("delete ")
        .or_else(|| trimmed.strip_prefix("d "))
        .or_else(|| trimmed.strip_prefix("qd "))
        .or_else(|| trimmed.strip_prefix("/delete "))
        .or_else(|| trimmed.strip_prefix("/d "))
        .or_else(|| trimmed.strip_prefix("/qd "))
    {
        return parse_playlist_index_parameter_legacy_compatible(index)
            .map(LocalInputCommand::DeletePlaylistIndex)
            .or(Some(LocalInputCommand::ShowPlaylistInvalidIndexError));
    }
    if matches!(trimmed, "delete" | "d" | "qd" | "/delete" | "/d" | "/qd") {
        return Some(LocalInputCommand::ShowPlaylistInvalidIndexError);
    }
    if let Some(command) = parse_user_ready_command_legacy_compatible(
        input,
        &["setready", "sr", "/setready", "/sr"],
        true,
    ) {
        return Some(command);
    }
    if let Some(command) = parse_user_ready_command_legacy_compatible(
        input,
        &["setnotready", "sn", "snr", "/setnotready", "/sn", "/snr"],
        false,
    ) {
        return Some(command);
    }
    if let Some(room_name) = parse_create_command_legacy_compatible(input) {
        return Some(LocalInputCommand::CreateControlledRoom(room_name));
    }
    if let Some(password) = trimmed
        .strip_prefix("auth ")
        .or_else(|| trimmed.strip_prefix("a "))
        .or_else(|| trimmed.strip_prefix("/auth "))
        .or_else(|| trimmed.strip_prefix("/a "))
    {
        let password = password.trim();
        return Some(LocalInputCommand::AuthController(password.to_owned()));
    }
    if matches!(trimmed, "auth" | "a" | "/auth" | "/a") {
        return Some(LocalInputCommand::AuthController(String::new()));
    }
    if let Some(parameter) = input
        .strip_prefix("seek ")
        .or_else(|| input.strip_prefix("s "))
        .or_else(|| input.strip_prefix("/seek "))
        .or_else(|| input.strip_prefix("/s "))
    {
        return parse_seek_parameter(parameter).or(Some(LocalInputCommand::ShowUnknownCommandHelp));
    }
    if matches!(trimmed, "seek" | "s" | "/seek" | "/s") {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    if matches_local_command_alias_legacy_compatible(
        trimmed,
        &["p", "pause", "play", "/p", "/pause", "/play"],
    ) {
        return Some(LocalInputCommand::TogglePause);
    }
    if let Some(room_command) = parse_room_command_legacy_compatible(input) {
        return room_command;
    }
    if matches_local_command_alias_legacy_compatible(trimmed, &["t", "toggle", "/t", "/toggle"]) {
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
    if matches!(trimmed, "/chat" | "/ch" | "/msg") {
        return None;
    }
    if trimmed.starts_with('/') {
        return Some(LocalInputCommand::ShowUnknownCommandHelp);
    }
    None
}

fn spawn_local_input_receiver_if_enabled() -> Option<UnboundedReceiver<String>> {
    if !env_flag_enabled("SYNCPLAY_CLIENT_STDIN") {
        return None;
    }

    let (sender, receiver) = unbounded_channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Some(receiver)
}

async fn recv_local_input_line(
    local_input_rx: &mut Option<&mut UnboundedReceiver<String>>,
) -> Option<String> {
    match local_input_rx {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending::<Option<String>>().await,
    }
}

fn build_client_loop_config_from_env() -> ClientLoopConfig {
    let room = env_trimmed("SYNCPLAY_CLIENT_ROOM").unwrap_or_else(|| "cli-demo".to_owned());
    let (room, controlled_room_password_override) = normalize_controlled_room_input(room);

    ClientLoopConfig {
        host: env_trimmed("SYNCPLAY_CLIENT_HOST").unwrap_or_else(|| "127.0.0.1".to_owned()),
        port: env_port("SYNCPLAY_CLIENT_PORT").unwrap_or(8999),
        username: env_trimmed("SYNCPLAY_CLIENT_USERNAME")
            .or_else(|| env_trimmed("SYNCPLAY_CLIENT_NAME"))
            .unwrap_or_else(|| "cli-user".to_owned()),
        room,
        version: env_trimmed("SYNCPLAY_CLIENT_VERSION").unwrap_or_else(|| "1.2.255".to_owned()),
        max_retries: env_u32("SYNCPLAY_CLIENT_MAX_RETRIES").unwrap_or(3),
        max_connected_runtime_seconds: env_non_negative_f64(
            "SYNCPLAY_CLIENT_MAX_CONNECTED_RUNTIME_SECONDS",
        )
        .unwrap_or(10.0),
        readiness_supported_override: env_flag_override("SYNCPLAY_CLIENT_READINESS_SUPPORTED"),
        local_can_control_override: env_flag_override("SYNCPLAY_CLIENT_CAN_CONTROL"),
        is_playing_music_override: env_flag_override("SYNCPLAY_CLIENT_IS_PLAYING_MUSIC"),
        recently_advanced_override: env_flag_override("SYNCPLAY_CLIENT_RECENTLY_ADVANCED"),
        autoplay_enabled: env_flag_enabled("SYNCPLAY_CLIENT_AUTOPLAY"),
        autoplay_require_same_filenames: env_flag_enabled(
            "SYNCPLAY_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES",
        ),
        filename_privacy_mode: env_privacy_mode("SYNCPLAY_CLIENT_FILENAME_PRIVACY_MODE")
            .unwrap_or(PrivacyMode::SendRaw),
        filesize_privacy_mode: env_privacy_mode("SYNCPLAY_CLIENT_FILESIZE_PRIVACY_MODE")
            .unwrap_or(PrivacyMode::SendRaw),
        show_duration_notification_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_DURATION_NOTIFICATION",
        ),
        different_duration_threshold_seconds_override: env_non_negative_f64(
            "SYNCPLAY_CLIENT_DIFFERENT_DURATION_THRESHOLD_SECONDS",
        ),
        show_same_room_osd_override: env_flag_override("SYNCPLAY_CLIENT_SHOW_SAME_ROOM_OSD"),
        show_osd_warnings_override: env_flag_override("SYNCPLAY_CLIENT_SHOW_OSD_WARNINGS"),
        show_noncontroller_osd_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_NONCONTROLLER_OSD",
        ),
        show_different_room_osd_override: env_flag_override(
            "SYNCPLAY_CLIENT_SHOW_DIFFERENT_ROOM_OSD",
        ),
        controlled_room_password_override,
    }
}

fn create_client_runtime(
    config: &ClientLoopConfig,
) -> ClientRuntime<MpvAdapter, QueuedRuntimeControl> {
    let mut session = ClientSession::default();
    session.set_autoplay_enabled(config.autoplay_enabled);
    if let Some(control_password) = config.controlled_room_password_override.as_deref() {
        session.remember_control_password_for_room(&config.room, control_password);
    }
    if let Some(show_same_room_osd) = config.show_same_room_osd_override {
        session.behavior_config_mut().show_same_room_osd = show_same_room_osd;
    }
    if let Some(show_osd_warnings) = config.show_osd_warnings_override {
        session.behavior_config_mut().show_osd_warnings = show_osd_warnings;
    }
    if let Some(show_noncontroller_osd) = config.show_noncontroller_osd_override {
        session.behavior_config_mut().show_noncontroller_osd = show_noncontroller_osd;
    }
    if let Some(show_different_room_osd) = config.show_different_room_osd_override {
        session.behavior_config_mut().show_different_room_osd = show_different_room_osd;
    }
    apply_client_behavior_overrides(&mut session, &behavior_overrides_from_env());
    {
        let readiness_config = session.readiness_autoplay_config_mut();
        readiness_config.autoplay_require_same_filenames = config.autoplay_require_same_filenames;
        if let Some(show_duration_notification) = config.show_duration_notification_override {
            readiness_config.show_duration_notification = show_duration_notification;
        }
        if let Some(different_duration_threshold_seconds) =
            config.different_duration_threshold_seconds_override
        {
            readiness_config.different_duration_threshold_seconds =
                different_duration_threshold_seconds;
        }
        apply_readiness_autoplay_overrides(readiness_config, &readiness_overrides_from_env());
    }
    apply_chat_policy_overrides(&mut session, &chat_policy_overrides_from_env());
    session.reconnect_policy_mut().max_retries = config.max_retries;
    ClientRuntime::new(
        session,
        MpvAdapter::default(),
        QueuedRuntimeControl::default(),
    )
}

async fn write_protocol_line(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    line: &str,
) -> anyhow::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn flush_runtime_protocol_lines(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
) -> anyhow::Result<()> {
    let mut lines = Vec::new();
    runtime.flush_queued_protocol_lines_to_transport(|line| {
        lines.push(line.to_owned());
        Ok(())
    })?;
    for line in &lines {
        write_protocol_line(writer, line).await?;
    }
    Ok(())
}

fn publish_pending_local_file_updates(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    loop {
        let published = runtime.publish_pending_local_file_update_legacy_compatible(
            config.filename_privacy_mode,
            config.filesize_privacy_mode,
        )?;
        if !published {
            break;
        }
    }
    Ok(())
}

fn format_file_difference_summary(summary: FileDifferenceSummary) -> Option<String> {
    let mut differences = Vec::new();
    if summary.filename {
        differences.push("filename");
    }
    if summary.filesize {
        differences.push("filesize");
    }
    if summary.fileduration {
        differences.push("duration");
    }

    if differences.is_empty() {
        None
    } else {
        Some(differences.join(", "))
    }
}

fn emit_file_difference_notification(summary: &str) -> anyhow::Result<()> {
    println!("file differences: {summary}");
    Ok(())
}

fn flush_file_difference_notifications_to_sink<F>(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    state: &mut FileDifferenceNotificationState,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> anyhow::Result<()>,
{
    let summary = runtime
        .session()
        .file_differences_for_current_room()
        .and_then(format_file_difference_summary);

    match summary {
        Some(summary) => {
            if state.last_summary.as_deref() != Some(summary.as_str()) {
                notify(summary.as_str())?;
            }
            state.last_summary = Some(summary);
        }
        None => {
            state.last_summary = None;
        }
    }

    Ok(())
}

fn emit_autoplay_countdown_notification(
    notification: &AutoplayCountdownNotification,
) -> anyhow::Result<()> {
    println!(
        "autoplay countdown: ready_users={} seconds_left={}",
        notification.ready_user_count, notification.seconds_left
    );
    Ok(())
}

fn flush_autoplay_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
{
    runtime.drain_autoplay_notifications_to_sink(|notification| notify(notification))
}

fn reconnect_transition_notification_message(
    notification: &ReconnectTransitionNotification,
) -> String {
    match notification {
        ReconnectTransitionNotification::Attempting {
            retries,
            delay_seconds,
        } => format!(
            "Connection with server lost, attempting to reconnect (retry={retries}, delay_seconds={delay_seconds:.3})"
        ),
        ReconnectTransitionNotification::Connected => "Reconnected to server".to_owned(),
        ReconnectTransitionNotification::Disconnected => {
            "Connection with server lost, reconnect attempts exhausted".to_owned()
        }
        ReconnectTransitionNotification::RestoringState => {
            "Restoring local state after reconnect...".to_owned()
        }
        ReconnectTransitionNotification::RestoringPlaylist => {
            "Restoring playlist on reconnect...".to_owned()
        }
    }
}

fn emit_reconnect_transition_notification(
    notification: &ReconnectTransitionNotification,
) -> anyhow::Result<()> {
    println!(
        "{}",
        reconnect_transition_notification_message(notification)
    );
    Ok(())
}

fn flush_reconnect_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ReconnectTransitionNotification) -> anyhow::Result<()>,
{
    runtime.drain_reconnect_notifications_to_sink(|notification| notify(notification))
}

fn controller_auth_transition_notification_message(
    notification: &ControllerAuthTransitionNotification,
) -> String {
    match notification {
        ControllerAuthTransitionNotification::Attempting { room } => {
            format!("Identifying as room operator in room {room}...")
        }
        ControllerAuthTransitionNotification::Succeeded { username, room, .. } => {
            format!("{username} authenticated as a room operator in room {room}")
        }
        ControllerAuthTransitionNotification::Failed { username, room, .. } => {
            format!("{username} failed to identify as a room operator in room {room}")
        }
    }
}

fn controller_auth_notification_hidden_from_osd(
    notification: &ControllerAuthTransitionNotification,
) -> bool {
    match notification {
        ControllerAuthTransitionNotification::Attempting { .. } => false,
        ControllerAuthTransitionNotification::Succeeded { hide_from_osd, .. }
        | ControllerAuthTransitionNotification::Failed { hide_from_osd, .. } => *hide_from_osd,
    }
}

fn emit_controller_auth_transition_notification(
    notification: &ControllerAuthTransitionNotification,
) -> anyhow::Result<()> {
    if controller_auth_notification_hidden_from_osd(notification) {
        return Ok(());
    }
    println!(
        "{}",
        controller_auth_transition_notification_message(notification)
    );
    Ok(())
}

fn flush_controller_auth_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ControllerAuthTransitionNotification) -> anyhow::Result<()>,
{
    runtime.drain_controller_auth_notifications_to_sink(|notification| notify(notification))
}

fn chat_notification_message(notification: &ChatNotification) -> String {
    match notification {
        ChatNotification::Message { username, message } => match username.as_deref() {
            Some(username) => format!("<{username}> {message}"),
            None => message.clone(),
        },
    }
}

fn emit_chat_notification(notification: &ChatNotification) -> anyhow::Result<()> {
    println!("{}", chat_notification_message(notification));
    Ok(())
}

fn local_command_help_lines_legacy_compatible() -> &'static [&'static str] {
    &[
        "Available commands:",
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

fn local_command_help_footer_lines_legacy_compatible(version: &str) -> [String; 2] {
    [
        format!("Syncplay version: {version}"),
        format!("More info available at: {PROJECT_URL_LEGACY}"),
    ]
}

fn emit_local_command_help_legacy_compatible(version: &str) -> anyhow::Result<()> {
    for line in local_command_help_lines_legacy_compatible() {
        println!("{line}");
    }
    for line in local_command_help_footer_lines_legacy_compatible(version) {
        println!("{line}");
    }
    Ok(())
}

fn emit_unknown_command_help_legacy_compatible(version: &str) -> anyhow::Result<()> {
    println!("{UNKNOWN_COMMAND_MESSAGE_LEGACY}");
    emit_local_command_help_legacy_compatible(version)
}

fn emit_local_error_message_legacy_compatible(message: &str) -> anyhow::Result<()> {
    println!("ERROR:\t{message}");
    Ok(())
}

fn apply_local_offset_command_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    user_offset_seconds: &mut f64,
    command: LocalOffsetCommand,
) -> anyhow::Result<bool> {
    let global_position = runtime
        .session()
        .current_room_playstate()
        .and_then(|playstate| playstate.position)
        .unwrap_or(0.0);
    let current_local_position = global_position + *user_offset_seconds;
    *user_offset_seconds = match command {
        LocalOffsetCommand::Absolute(offset_seconds) => offset_seconds,
        LocalOffsetCommand::Relative(offset_delta_seconds) => {
            *user_offset_seconds + offset_delta_seconds
        }
        LocalOffsetCommand::RelativeFromCurrentPositionMinus(offset_seconds) => {
            current_local_position - offset_seconds
        }
    };
    println!("Current offset: {} seconds", *user_offset_seconds);
    Ok(runtime.run_seek_to_position(global_position + *user_offset_seconds)?)
}

fn playlist_listing_message_legacy_compatible(session: &ClientSession) -> String {
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

fn emit_playlist_listing_for_current_room(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<()> {
    println!(
        "{}",
        playlist_listing_message_legacy_compatible(runtime.session())
    );
    Ok(())
}

fn flush_chat_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ChatNotification) -> anyhow::Result<()>,
{
    runtime.drain_chat_notifications_to_sink(|notification| notify(notification))
}

fn round_half_to_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;

    if fraction + ROUND_HALF_EPSILON < 0.5 {
        return floor;
    }
    if fraction - ROUND_HALF_EPSILON > 0.5 {
        return floor + 1.0;
    }

    if floor.rem_euclid(2.0) == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

fn format_duration_legacy(time_seconds: f64) -> String {
    let sign = if time_seconds < 0.0 { "-" } else { "" };
    let rounded_seconds = round_half_to_even(time_seconds.abs()) as u64;

    let mut weeks = rounded_seconds / 604_800;
    let title = if weeks > 0 {
        let title = weeks;
        weeks = 0;
        title
    } else {
        0
    };
    let days = (rounded_seconds % 604_800) / 86_400;
    let hours = (rounded_seconds % 86_400) / 3_600;
    let minutes = (rounded_seconds % 3_600) / 60;
    let seconds = rounded_seconds % 60;

    let mut formatted = if weeks > 0 {
        format!("{sign}{weeks}w, {days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if days > 0 {
        format!("{sign}{days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{minutes:02}:{seconds:02}")
    };

    if title > 0 {
        formatted.push_str(&format!(" (Title {title})"));
    }

    formatted
}

fn user_change_notification_message(notification: &UserChangeNotification) -> String {
    match notification {
        UserChangeNotification::Joined { username, room, .. } => {
            format!("{username} has joined the room: '{room}'")
        }
        UserChangeNotification::Playing {
            username,
            room,
            file_name,
            file_duration,
            include_room_addendum,
            ..
        } => match file_name.as_deref() {
            Some(file_name) => {
                let mut message = if let Some(duration_seconds) = file_duration
                    .as_ref()
                    .and_then(|duration| duration.as_f64())
                {
                    format!(
                        "{username} is playing '{file_name}' ({})",
                        format_duration_legacy(duration_seconds)
                    )
                } else {
                    format!("{username} is playing '{file_name}'")
                };
                if *include_room_addendum {
                    message.push_str(&format!(" in room: '{room}'"));
                }
                message
            }
            None if *include_room_addendum => {
                format!("{username} is playing a file in room: '{room}'")
            }
            None => format!("{username} is playing a file"),
        },
        UserChangeNotification::Left { username, .. } => format!("{username} has left"),
    }
}

fn user_change_notification_hidden_from_osd(notification: &UserChangeNotification) -> bool {
    match notification {
        UserChangeNotification::Joined { hide_from_osd, .. }
        | UserChangeNotification::Playing { hide_from_osd, .. }
        | UserChangeNotification::Left { hide_from_osd, .. } => *hide_from_osd,
    }
}

fn emit_user_change_notification(notification: &UserChangeNotification) -> anyhow::Result<()> {
    if user_change_notification_hidden_from_osd(notification) {
        return Ok(());
    }
    println!("{}", user_change_notification_message(notification));
    Ok(())
}

fn flush_user_change_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&UserChangeNotification) -> anyhow::Result<()>,
{
    runtime.drain_user_change_notifications_to_sink(|notification| notify(notification))
}

fn derive_runtime_loop_inputs(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
    now_seconds: f64,
) -> RuntimeLoopInputs {
    let session = runtime.session();
    let readiness_supported = config
        .readiness_supported_override
        .or_else(|| session.server_readiness_supported())
        .unwrap_or(true);
    let local_can_control = config
        .local_can_control_override
        .or_else(|| session.local_can_control())
        .unwrap_or(true);
    let is_playing_music = config
        .is_playing_music_override
        .unwrap_or_else(|| session.is_playing_music());
    let recently_advanced = config
        .recently_advanced_override
        .unwrap_or_else(|| session.recently_advanced(now_seconds));

    RuntimeLoopInputs {
        readiness_supported,
        local_can_control,
        is_playing_music,
        recently_advanced,
    }
}

async fn run_connected_client_session<F, G>(
    stream: TcpStream,
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
    chat_message_on_connect: Option<&str>,
    local_input_rx: Option<&mut UnboundedReceiver<String>>,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
) -> anyhow::Result<ConnectedSessionExit>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let mut local_input_rx = local_input_rx;
    let hello_message = ProtocolMessage::hello_basic(
        config.username.clone(),
        config.room.clone(),
        config.version.clone(),
    );
    runtime
        .session_mut()
        .apply_protocol_message(hello_message.clone())?;

    let hello_line = encode_message_line(&hello_message)?;
    let (reader, mut writer) = stream.into_split();
    write_protocol_line(&mut writer, &hello_line).await?;
    let mut pending_chat_message_on_connect = chat_message_on_connect.map(str::to_owned);
    publish_pending_local_file_updates(runtime, config)?;
    flush_runtime_protocol_lines(runtime, &mut writer).await?;

    let mut reader = BufReader::new(reader).lines();
    let connected_start = Instant::now();
    let mut autoplay_tick =
        tokio::time::interval(Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS));
    let mut file_difference_state = FileDifferenceNotificationState::default();
    let mut local_user_offset_seconds = 0.0f64;

    loop {
        if connected_start.elapsed().as_secs_f64() >= config.max_connected_runtime_seconds {
            return Ok(ConnectedSessionExit::RuntimeWindowElapsed);
        }

        tokio::select! {
            line = reader.next_line() => {
                match line? {
                    Some(line) => {
                        let now_seconds = connected_start.elapsed().as_secs_f64();
                        runtime.session_mut().apply_message_json_at(&line, now_seconds)?;
                        if let Some(message) = pending_chat_message_on_connect.take() {
                            let _ = runtime.run_send_chat_message(message)?;
                        }
                        runtime.run_reconnect_transition_if_needed()?;
                        runtime.run_controller_reidentify_if_needed()?;
                        runtime.run_controller_auth_notifications_if_needed()?;
                        runtime.run_chat_notifications_if_needed()?;
                        runtime.run_user_change_notifications_if_needed()?;
                        runtime.run_reconnect_state_restore_if_needed()?;
                        runtime.run_reconnect_playlist_restore_if_needed()?;
                        let inputs = derive_runtime_loop_inputs(runtime, config, now_seconds);
                        runtime.run_readiness_unpause_attempt(
                            now_seconds,
                            inputs.readiness_supported,
                            inputs.local_can_control,
                            inputs.is_playing_music,
                        )?;
                        publish_pending_local_file_updates(runtime, config)?;
                        flush_runtime_protocol_lines(runtime, &mut writer).await?;
                        flush_reconnect_notifications_to_sink(
                            runtime,
                            &mut emit_reconnect_transition_notification,
                        )?;
                        flush_controller_auth_notifications_to_sink(
                            runtime,
                            &mut emit_controller_auth_transition_notification,
                        )?;
                        flush_chat_notifications_to_sink(runtime, &mut emit_chat_notification)?;
                        flush_user_change_notifications_to_sink(
                            runtime,
                            &mut emit_user_change_notification,
                        )?;
                        flush_autoplay_notifications_to_sink(runtime, notification_sink)?;
                        flush_file_difference_notifications_to_sink(
                            runtime,
                            &mut file_difference_state,
                            file_difference_sink,
                        )?;
                    }
                    None => return Ok(ConnectedSessionExit::TransportClosed),
                }
            }
            _ = autoplay_tick.tick() => {
                let now_seconds = connected_start.elapsed().as_secs_f64();
                let inputs = derive_runtime_loop_inputs(runtime, config, now_seconds);
                runtime.update_autoplay_check(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                );
                runtime.tick_autoplay(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                )?;
                publish_pending_local_file_updates(runtime, config)?;
                flush_runtime_protocol_lines(runtime, &mut writer).await?;
                flush_autoplay_notifications_to_sink(runtime, notification_sink)?;
                flush_file_difference_notifications_to_sink(
                    runtime,
                    &mut file_difference_state,
                    file_difference_sink,
                )?;
            }
            local_line = recv_local_input_line(&mut local_input_rx) => {
                let Some(local_line) = local_line else {
                    local_input_rx = None;
                    continue;
                };

                if let Some(command) = parse_local_input_command(&local_line) {
                    let help_version = config.version.as_str();
                    let emitted = match command {
                        LocalInputCommand::Chat(chat_message) => {
                            runtime.run_send_chat_message(chat_message)?
                        }
                        LocalInputCommand::RequestUserList => runtime.run_request_user_list()?,
                        LocalInputCommand::ShowUnknownCommandHelp => {
                            emit_unknown_command_help_legacy_compatible(help_version)?;
                            false
                        }
                        LocalInputCommand::ShowHelp => {
                            emit_local_command_help_legacy_compatible(help_version)?;
                            false
                        }
                        LocalInputCommand::ShowPlaylistInvalidIndexError => {
                            emit_local_error_message_legacy_compatible(
                                PLAYLIST_INVALID_INDEX_ERROR_LEGACY,
                            )?;
                            false
                        }
                        LocalInputCommand::ShowQueueMissingFileError => {
                            emit_local_error_message_legacy_compatible(
                                QUEUE_MISSING_FILE_ERROR_LEGACY,
                            )?;
                            false
                        }
                        LocalInputCommand::ShowPlaylist => {
                            emit_playlist_listing_for_current_room(runtime)?;
                            false
                        }
                        LocalInputCommand::SelectPlaylistIndex(index) => {
                            runtime.run_set_playlist_index(index)?
                        }
                        LocalInputCommand::NextPlaylistItem => runtime.run_advance_playlist_index()?,
                        LocalInputCommand::QueuePlaylistItem {
                            file_name,
                            select_after_queue,
                        } => runtime.run_queue_playlist_item(file_name, select_after_queue)?,
                        LocalInputCommand::DeletePlaylistIndex(index) => {
                            runtime.run_delete_playlist_index(index)?
                        }
                        LocalInputCommand::UndoPlaylistChange => {
                            runtime.run_undo_playlist_change()?
                        }
                        LocalInputCommand::ShuffleRemainingPlaylist => {
                            runtime.run_shuffle_remaining_playlist()?
                        }
                        LocalInputCommand::ShuffleEntirePlaylist => {
                            runtime.run_shuffle_entire_playlist()?
                        }
                        LocalInputCommand::UndoSeek => runtime.run_undo_seek()?,
                        LocalInputCommand::SetUserOffset(offset_command) => {
                            apply_local_offset_command_legacy_compatible(
                                runtime,
                                &mut local_user_offset_seconds,
                                offset_command,
                            )?
                        }
                        LocalInputCommand::SeekAbsolute(position_seconds) => {
                            runtime.run_seek_to_position(position_seconds)?
                        }
                        LocalInputCommand::SeekRelative(offset_seconds) => {
                            runtime.run_seek_by_offset(offset_seconds)?
                        }
                        LocalInputCommand::TogglePause => runtime.run_toggle_pause()?,
                        LocalInputCommand::ToggleReady => runtime.run_toggle_ready(true)?,
                        LocalInputCommand::SetUserReady { username, ready } => {
                            runtime.run_set_ready_for_user(username, ready, true)?
                        }
                        LocalInputCommand::CreateControlledRoom(room_name) => {
                            let room = room_name.unwrap_or_else(|| {
                                runtime
                                    .session()
                                    .room
                                    .clone()
                                    .unwrap_or_else(|| config.room.clone())
                            });
                            let room = controlled_room_base_name_legacy_compatible(&room);
                            let password = generate_room_password_legacy_compatible();
                            runtime.run_request_controller_auth(room, password)?
                        }
                        LocalInputCommand::AuthController(password) => {
                            let room = runtime
                                .session()
                                .room
                                .clone()
                                .unwrap_or_else(|| config.room.clone());
                            runtime.run_request_controller_auth(room, password)?
                        }
                        LocalInputCommand::SetRoomWithLegacyFallback => {
                            runtime.run_set_room_with_legacy_fallback(config.room.clone())?
                        }
                        LocalInputCommand::SetRoom(room) => runtime.run_set_room(room)?,
                    };
                    if emitted {
                        flush_runtime_protocol_lines(runtime, &mut writer).await?;
                    }
                }
            }
        }
    }
}

async fn run_reconnect_backoff(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    retries: &mut u32,
) -> anyhow::Result<bool> {
    runtime.run_reconnect_retry(*retries)?;
    flush_reconnect_notifications_to_sink(runtime, &mut emit_reconnect_transition_notification)?;
    let mut reconnect_delay = None;
    let mut stop_requested = false;
    runtime.drain_reconnect_intents(
        |delay_seconds| reconnect_delay = Some(delay_seconds),
        || stop_requested = true,
    );

    if stop_requested {
        return Ok(true);
    }

    let delay_seconds = reconnect_delay.unwrap_or(0.1);
    tokio::time::sleep(Duration::from_secs_f64(delay_seconds)).await;
    *retries = retries.saturating_add(1);
    Ok(false)
}

async fn run_client_network_loop(config: &ClientLoopConfig) -> anyhow::Result<()> {
    let mut runtime = create_client_runtime(config);
    let mut local_input_rx = spawn_local_input_receiver_if_enabled();
    let chat_message_on_connect = env_trimmed("SYNCPLAY_CLIENT_CHAT_MESSAGE");
    let mut notification_sink = emit_autoplay_countdown_notification;
    let mut file_difference_sink = emit_file_difference_notification;
    let endpoint = format!("{}:{}", config.host, config.port);
    let network_start = Instant::now();
    let mut retries = 0_u32;

    loop {
        match TcpStream::connect(&endpoint).await {
            Ok(stream) => {
                retries = 0;
                match run_connected_client_session(
                    stream,
                    &mut runtime,
                    config,
                    chat_message_on_connect.as_deref(),
                    local_input_rx.as_mut(),
                    &mut notification_sink,
                    &mut file_difference_sink,
                )
                .await?
                {
                    ConnectedSessionExit::RuntimeWindowElapsed => return Ok(()),
                    ConnectedSessionExit::TransportClosed => {
                        let now_seconds = network_start.elapsed().as_secs_f64();
                        runtime.run_disconnect(now_seconds)?;
                        if run_reconnect_backoff(&mut runtime, &mut retries).await? {
                            return Err(anyhow!(
                                "server connection closed and reconnect retries were exhausted"
                            ));
                        }
                    }
                }
            }
            Err(connect_err) => {
                if run_reconnect_backoff(&mut runtime, &mut retries).await? {
                    return Err(connect_err.into());
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let motd_template = env_trimmed("SYNCPLAY_SERVER_MOTD_TEMPLATE");
    let rooms_db_file = env_trimmed("SYNCPLAY_SERVER_ROOMS_DB_FILE");
    let permanent_rooms_file = env_trimmed("SYNCPLAY_SERVER_PERMANENT_ROOMS_FILE");
    let tls_cert_path = env_trimmed("SYNCPLAY_SERVER_TLS_CERT_PATH");
    let stats_db_file = env_trimmed("SYNCPLAY_SERVER_STATS_DB_FILE");
    let server_port = env_port("SYNCPLAY_SERVER_PORT");
    let persistent_rooms_enabled = env_flag_enabled("SYNCPLAY_SERVER_PERSISTENT_ROOMS");
    let mut server = ServerApp::new();
    if let Some(template) = motd_template {
        server.runtime_mut().set_motd_template(Some(template));
    }
    server
        .runtime_mut()
        .set_persistent_rooms_db_path(rooms_db_file.as_deref().map(std::path::PathBuf::from))?;
    server.runtime_mut().set_permanent_rooms_file_path(
        permanent_rooms_file
            .as_deref()
            .map(std::path::PathBuf::from),
    )?;
    server
        .runtime_mut()
        .set_persistent_rooms_enabled(persistent_rooms_enabled || rooms_db_file.is_some());
    if let Some(port) = server_port {
        server
            .runtime_mut()
            .set_stats_snapshot_start_delay_for_port(port);
    }
    server
        .runtime_mut()
        .set_tls_cert_path(tls_cert_path.as_deref().map(std::path::PathBuf::from));
    server
        .runtime_mut()
        .set_stats_db_path(stats_db_file.as_deref().map(std::path::PathBuf::from))?;
    server.bootstrap_room("cli-demo");

    if env_flag_enabled("SYNCPLAY_CLIENT_CONNECT") {
        let config = build_client_loop_config_from_env();
        run_client_network_loop(&config).await?;
        return Ok(());
    }

    let mut client = ClientSession::default();
    client.apply_hello_json(
        r#"{"Hello":{"username":"cli-user","room":{"name":"cli-demo"},"version":"1.2.255"}}"#,
    )?;

    println!(
        "syncplay-cli bootstrap complete for user {} in room {}",
        client.username.as_deref().unwrap_or("unknown"),
        client.room.as_deref().unwrap_or("unknown")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AutoplayThresholdOverride, ChatPolicyOverrides, ClientBehaviorOverrides, ClientLoopConfig,
        ConnectedSessionExit, LocalInputCommand, LocalOffsetCommand, ReadinessAutoplayOverrides,
        apply_chat_policy_overrides, apply_client_behavior_overrides,
        apply_readiness_autoplay_overrides, chat_notification_message,
        controlled_room_base_name_legacy_compatible, controller_auth_notification_hidden_from_osd,
        controller_auth_transition_notification_message, create_client_runtime,
        flush_autoplay_notifications_to_sink, flush_chat_notifications_to_sink,
        flush_controller_auth_notifications_to_sink, flush_file_difference_notifications_to_sink,
        flush_reconnect_notifications_to_sink, flush_user_change_notifications_to_sink,
        format_duration_legacy, format_file_difference_summary,
        generate_room_password_legacy_compatible,
        local_command_help_footer_lines_legacy_compatible,
        local_command_help_lines_legacy_compatible, normalize_controlled_room_input,
        parse_autoplay_min_users_override_legacy_compatible, parse_env_bool_legacy_compatible,
        parse_env_non_negative_f64_legacy_compatible, parse_env_port_legacy_compatible,
        parse_env_string_list_legacy_compatible, parse_local_input_chat_message,
        parse_local_input_command, parse_unpause_action_mode_legacy_compatible,
        playlist_listing_message_legacy_compatible, reconnect_transition_notification_message,
        run_client_network_loop, run_connected_client_session,
        user_change_notification_hidden_from_osd, user_change_notification_message,
    };
    use std::time::Duration;
    use syncplay_client_core::{
        AutoplayCountdownNotification, ChatNotification, ClientSession,
        ControllerAuthTransitionNotification, FileDifferenceSummary, PrivacyMode,
        ReadinessAutoplayConfig, ReconnectTransitionNotification, UnpauseActionMode,
        UserChangeNotification,
    };
    use syncplay_player_api::PlayerAdapter;
    use syncplay_protocol::{ListPayload, ProtocolMessage, decode_message_line};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc::unbounded_channel;

    fn ignore_autoplay_notification(
        _notification: &AutoplayCountdownNotification,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn ignore_file_difference_notification(_summary: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn ignore_reconnect_notification(
        _notification: &ReconnectTransitionNotification,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn ignore_controller_auth_notification(
        _notification: &ControllerAuthTransitionNotification,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn ignore_chat_notification(_notification: &ChatNotification) -> anyhow::Result<()> {
        Ok(())
    }

    fn ignore_user_change_notification(
        _notification: &UserChangeNotification,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_legacy_generated_room_password_shape(password: &str) -> bool {
        let chars: Vec<char> = password.chars().collect();
        if chars.len() != 10 {
            return false;
        }
        chars[0].is_ascii_uppercase()
            && chars[1].is_ascii_uppercase()
            && chars[2] == '-'
            && chars[3].is_ascii_digit()
            && chars[4].is_ascii_digit()
            && chars[5].is_ascii_digit()
            && chars[6] == '-'
            && chars[7].is_ascii_digit()
            && chars[8].is_ascii_digit()
            && chars[9].is_ascii_digit()
    }

    #[test]
    fn reconnect_transition_notification_message_uses_legacy_style_wording() {
        assert_eq!(
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::Attempting {
                    retries: 2,
                    delay_seconds: 0.4,
                }
            ),
            "Connection with server lost, attempting to reconnect (retry=2, delay_seconds=0.400)"
        );
        assert_eq!(
            reconnect_transition_notification_message(&ReconnectTransitionNotification::Connected),
            "Reconnected to server"
        );
        assert_eq!(
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::Disconnected
            ),
            "Connection with server lost, reconnect attempts exhausted"
        );
    }

    #[test]
    fn controller_auth_transition_notification_message_uses_legacy_style_wording() {
        assert_eq!(
            controller_auth_transition_notification_message(
                &ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                }
            ),
            "Identifying as room operator in room +room:ABCDEF123456..."
        );
        assert_eq!(
            controller_auth_transition_notification_message(
                &ControllerAuthTransitionNotification::Succeeded {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                }
            ),
            "alice authenticated as a room operator in room +room:ABCDEF123456"
        );
        assert_eq!(
            controller_auth_transition_notification_message(
                &ControllerAuthTransitionNotification::Failed {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: true,
                }
            ),
            "alice failed to identify as a room operator in room +room:ABCDEF123456"
        );
    }

    #[test]
    fn controller_auth_notification_hidden_from_osd_uses_visibility_metadata() {
        assert!(
            !controller_auth_notification_hidden_from_osd(
                &ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                }
            ),
            "attempt notification should never be hidden by OSD visibility metadata"
        );
        assert!(controller_auth_notification_hidden_from_osd(
            &ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            }
        ));
        assert!(!controller_auth_notification_hidden_from_osd(
            &ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            }
        ));
    }

    #[test]
    fn chat_notification_message_formats_username_and_plain_text_payloads() {
        assert_eq!(
            chat_notification_message(&ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hello everyone".to_owned(),
            }),
            "<bob> hello everyone"
        );
        assert_eq!(
            chat_notification_message(&ChatNotification::Message {
                username: None,
                message: "server broadcast".to_owned(),
            }),
            "server broadcast"
        );
    }

    #[test]
    fn playlist_listing_message_legacy_compatible_formats_entries_and_selected_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
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
    fn playlist_listing_message_legacy_compatible_uses_empty_message_when_no_playlist() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(
            playlist_listing_message_legacy_compatible(&session),
            "Playlist is currently empty."
        );
    }

    #[test]
    fn parse_env_bool_legacy_compatible_parses_expected_tokens() {
        assert_eq!(parse_env_bool_legacy_compatible("1"), Some(true));
        assert_eq!(parse_env_bool_legacy_compatible("true"), Some(true));
        assert_eq!(parse_env_bool_legacy_compatible("YES"), Some(true));
        assert_eq!(parse_env_bool_legacy_compatible("on"), Some(true));
        assert_eq!(parse_env_bool_legacy_compatible("0"), Some(false));
        assert_eq!(parse_env_bool_legacy_compatible("false"), Some(false));
        assert_eq!(parse_env_bool_legacy_compatible("No"), Some(false));
        assert_eq!(parse_env_bool_legacy_compatible("off"), Some(false));
    }

    #[test]
    fn parse_env_bool_legacy_compatible_rejects_invalid_values() {
        assert_eq!(parse_env_bool_legacy_compatible(""), None);
        assert_eq!(parse_env_bool_legacy_compatible("  "), None);
        assert_eq!(parse_env_bool_legacy_compatible("maybe"), None);
        assert_eq!(parse_env_bool_legacy_compatible("2"), None);
    }

    #[test]
    fn parse_env_port_legacy_compatible_requires_port_range_one_to_65535() {
        assert_eq!(parse_env_port_legacy_compatible("1"), Some(1));
        assert_eq!(parse_env_port_legacy_compatible("65535"), Some(65535));
        assert_eq!(parse_env_port_legacy_compatible("0"), None);
        assert_eq!(parse_env_port_legacy_compatible("65536"), None);
        assert_eq!(parse_env_port_legacy_compatible("abc"), None);
    }

    #[test]
    fn parse_env_non_negative_f64_legacy_compatible_requires_finite_non_negative_values() {
        assert_eq!(parse_env_non_negative_f64_legacy_compatible("0"), Some(0.0));
        assert_eq!(
            parse_env_non_negative_f64_legacy_compatible("1.25"),
            Some(1.25)
        );
        assert_eq!(parse_env_non_negative_f64_legacy_compatible("-0.01"), None);
        assert_eq!(parse_env_non_negative_f64_legacy_compatible("NaN"), None);
        assert_eq!(parse_env_non_negative_f64_legacy_compatible("inf"), None);
        assert_eq!(parse_env_non_negative_f64_legacy_compatible("abc"), None);
    }

    #[test]
    fn parse_env_string_list_legacy_compatible_splits_and_trims_entries() {
        assert_eq!(
            parse_env_string_list_legacy_compatible(
                " youtube.com , *.example.com/videos ; youtu.be"
            ),
            Some(vec![
                "youtube.com".to_owned(),
                "*.example.com/videos".to_owned(),
                "youtu.be".to_owned()
            ])
        );
        assert_eq!(
            parse_env_string_list_legacy_compatible("alpha\nbeta\r\ngamma"),
            Some(vec![
                "alpha".to_owned(),
                "beta".to_owned(),
                "gamma".to_owned()
            ])
        );
    }

    #[test]
    fn parse_env_string_list_legacy_compatible_rejects_empty_values() {
        assert_eq!(parse_env_string_list_legacy_compatible(""), None);
        assert_eq!(parse_env_string_list_legacy_compatible(" , ; \n "), None);
    }

    #[test]
    fn parse_unpause_action_mode_legacy_compatible_accepts_known_values() {
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("IfAlreadyReady"),
            Some(UnpauseActionMode::IfAlreadyReady)
        );
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("if_others_ready"),
            Some(UnpauseActionMode::IfOthersReady)
        );
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("if-min-users-ready"),
            Some(UnpauseActionMode::IfMinUsersReady)
        );
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("always"),
            Some(UnpauseActionMode::Always)
        );
    }

    #[test]
    fn parse_unpause_action_mode_legacy_compatible_rejects_unknown_values() {
        assert_eq!(parse_unpause_action_mode_legacy_compatible(""), None);
        assert_eq!(
            parse_unpause_action_mode_legacy_compatible("sometimes"),
            None
        );
    }

    #[test]
    fn parse_autoplay_min_users_override_legacy_compatible_maps_legacy_ranges() {
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("-1"),
            Some(AutoplayThresholdOverride::Disable)
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("0"),
            Some(AutoplayThresholdOverride::Disable)
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("1"),
            Some(AutoplayThresholdOverride::Set(1))
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("3"),
            Some(AutoplayThresholdOverride::Set(3))
        );
        assert_eq!(
            parse_autoplay_min_users_override_legacy_compatible("abc"),
            None
        );
    }

    #[test]
    fn apply_client_behavior_overrides_updates_playlist_behavior_fields() {
        let mut session = ClientSession::default();
        let overrides = ClientBehaviorOverrides {
            pause_on_leave: Some(false),
            loop_at_end_of_playlist: Some(true),
            loop_single_files: Some(true),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(vec![
                "youtube.com".to_owned(),
                "*.example.com/videos".to_owned(),
            ]),
        };
        apply_client_behavior_overrides(&mut session, &overrides);

        let behavior = session.behavior_config();
        assert!(!behavior.pause_on_leave);
        assert!(behavior.loop_at_end_of_playlist);
        assert!(behavior.loop_single_files);
        assert!(!behavior.only_switch_to_trusted_domains);
        assert_eq!(
            behavior.trusted_domains,
            vec!["youtube.com".to_owned(), "*.example.com/videos".to_owned()]
        );
    }

    #[test]
    fn apply_readiness_autoplay_overrides_updates_fields() {
        let mut readiness = ReadinessAutoplayConfig::default();
        let overrides = ReadinessAutoplayOverrides {
            unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
            auto_play_threshold: Some(AutoplayThresholdOverride::Set(3)),
            autoplay_delay_seconds: Some(4.5),
            last_paused_diff_threshold_seconds: Some(2.25),
        };
        apply_readiness_autoplay_overrides(&mut readiness, &overrides);

        assert_eq!(readiness.unpause_action, UnpauseActionMode::IfMinUsersReady);
        assert_eq!(readiness.auto_play_threshold, Some(3));
        assert_eq!(readiness.autoplay_delay_seconds, 4.5);
        assert_eq!(readiness.last_paused_diff_threshold_seconds, 2.25);

        let disable_threshold_overrides = ReadinessAutoplayOverrides {
            auto_play_threshold: Some(AutoplayThresholdOverride::Disable),
            ..ReadinessAutoplayOverrides::default()
        };
        apply_readiness_autoplay_overrides(&mut readiness, &disable_threshold_overrides);
        assert_eq!(readiness.auto_play_threshold, None);
    }

    #[test]
    fn apply_chat_policy_overrides_sets_max_and_disables_server_sync_by_default() {
        let mut session = ClientSession::default();
        let overrides = ChatPolicyOverrides {
            max_chat_message_length: Some(12),
            apply_server_max_chat_message_length: None,
        };
        apply_chat_policy_overrides(&mut session, &overrides);

        let chat_config = session.chat_config();
        assert_eq!(chat_config.max_chat_message_length, 12);
        assert!(!chat_config.apply_server_max_chat_message_length);
    }

    #[test]
    fn apply_chat_policy_overrides_allows_explicit_server_sync_override() {
        let mut session = ClientSession::default();
        let overrides = ChatPolicyOverrides {
            max_chat_message_length: Some(12),
            apply_server_max_chat_message_length: Some(true),
        };
        apply_chat_policy_overrides(&mut session, &overrides);

        let chat_config = session.chat_config();
        assert_eq!(chat_config.max_chat_message_length, 12);
        assert!(chat_config.apply_server_max_chat_message_length);

        let overrides = ChatPolicyOverrides {
            max_chat_message_length: None,
            apply_server_max_chat_message_length: Some(false),
        };
        apply_chat_policy_overrides(&mut session, &overrides);
        assert!(!session.chat_config().apply_server_max_chat_message_length);
    }

    #[test]
    fn parse_local_input_chat_message_handles_plain_and_prefixed_inputs() {
        assert_eq!(
            parse_local_input_chat_message("hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("chat hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("ch hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("/chat hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("/ch hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("/msg hello everyone"),
            Some("hello everyone".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("chat   hello everyone  "),
            Some("  hello everyone  ".to_owned())
        );
        assert_eq!(
            parse_local_input_chat_message("/msg   hello everyone  "),
            Some("  hello everyone  ".to_owned())
        );
    }

    #[test]
    fn parse_local_input_chat_message_handles_empty_chat_aliases_and_ignores_unknown_commands() {
        assert_eq!(parse_local_input_chat_message(""), None);
        assert_eq!(parse_local_input_chat_message("   "), None);
        assert_eq!(parse_local_input_chat_message(" hello everyone"), None);
        assert_eq!(
            parse_local_input_chat_message(" /chat hello everyone"),
            None
        );
        assert_eq!(parse_local_input_chat_message("chat"), Some("".to_owned()));
        assert_eq!(parse_local_input_chat_message("ch"), Some("".to_owned()));
        assert_eq!(parse_local_input_chat_message("/chat"), Some("".to_owned()));
        assert_eq!(parse_local_input_chat_message("/ch"), Some("".to_owned()));
        assert_eq!(parse_local_input_chat_message("/msg"), Some("".to_owned()));
        assert_eq!(
            parse_local_input_chat_message("chat  "),
            Some(" ".to_owned())
        );
        assert_eq!(parse_local_input_chat_message("/msg "), Some("".to_owned()));
        assert_eq!(
            parse_local_input_chat_message("/msg   "),
            Some("  ".to_owned())
        );
        assert_eq!(parse_local_input_chat_message("chat\thello"), None);
        assert_eq!(parse_local_input_chat_message("help\tplease"), None);
        assert_eq!(parse_local_input_chat_message("/unknown hello"), None);
    }

    #[test]
    fn parse_local_input_command_parses_toggle_aliases() {
        assert_eq!(
            parse_local_input_command("toggle"),
            Some(LocalInputCommand::ToggleReady)
        );
        assert_eq!(
            parse_local_input_command("t"),
            Some(LocalInputCommand::ToggleReady)
        );
        assert_eq!(
            parse_local_input_command("/toggle"),
            Some(LocalInputCommand::ToggleReady)
        );
        assert_eq!(
            parse_local_input_command("/t"),
            Some(LocalInputCommand::ToggleReady)
        );
    }

    #[test]
    fn parse_local_input_command_parses_setready_aliases() {
        assert_eq!(
            parse_local_input_command("setready bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("sr bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("/setready bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("/sr bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("setready"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("setready "),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("sr"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("/setready"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("/sr"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("setready  "),
            Some(LocalInputCommand::SetUserReady {
                username: " ".to_owned(),
                ready: true
            })
        );
        assert_eq!(
            parse_local_input_command("setready   bob  "),
            Some(LocalInputCommand::SetUserReady {
                username: "  bob  ".to_owned(),
                ready: true
            })
        );
    }

    #[test]
    fn parse_local_input_command_parses_setnotready_aliases() {
        assert_eq!(
            parse_local_input_command("setnotready bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("sn bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("snr bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/setnotready bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/sn bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/snr bob"),
            Some(LocalInputCommand::SetUserReady {
                username: "bob".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("setnotready"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("setnotready "),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("sn"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("snr"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/setnotready"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/sn"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("/snr"),
            Some(LocalInputCommand::SetUserReady {
                username: String::new(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("setnotready  "),
            Some(LocalInputCommand::SetUserReady {
                username: " ".to_owned(),
                ready: false
            })
        );
        assert_eq!(
            parse_local_input_command("setnotready   bob  "),
            Some(LocalInputCommand::SetUserReady {
                username: "  bob  ".to_owned(),
                ready: false
            })
        );
    }

    #[test]
    fn parse_local_input_command_parses_create_aliases() {
        assert_eq!(
            parse_local_input_command("create"),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("create "),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("c"),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("c "),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("/create"),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("/create "),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("/c"),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("/c "),
            Some(LocalInputCommand::CreateControlledRoom(None))
        );
        assert_eq!(
            parse_local_input_command("create  "),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                " ".to_owned()
            )))
        );
        assert_eq!(
            parse_local_input_command("create   base-room"),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                "  base-room".to_owned()
            )))
        );
        assert_eq!(
            parse_local_input_command("create base-room"),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                "base-room".to_owned()
            )))
        );
        assert_eq!(
            parse_local_input_command("c base-room"),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                "base-room".to_owned()
            )))
        );
        assert_eq!(
            parse_local_input_command("/create base-room"),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                "base-room".to_owned()
            )))
        );
        assert_eq!(
            parse_local_input_command("/c base-room"),
            Some(LocalInputCommand::CreateControlledRoom(Some(
                "base-room".to_owned()
            )))
        );
    }

    #[test]
    fn parse_local_input_command_parses_auth_aliases() {
        assert_eq!(
            parse_local_input_command("auth ab-123-456"),
            Some(LocalInputCommand::AuthController("ab-123-456".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("a ab-123-456"),
            Some(LocalInputCommand::AuthController("ab-123-456".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/auth ab-123-456"),
            Some(LocalInputCommand::AuthController("ab-123-456".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/a ab-123-456"),
            Some(LocalInputCommand::AuthController("ab-123-456".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("auth"),
            Some(LocalInputCommand::AuthController(String::new()))
        );
        assert_eq!(
            parse_local_input_command("a"),
            Some(LocalInputCommand::AuthController(String::new()))
        );
        assert_eq!(
            parse_local_input_command("/auth"),
            Some(LocalInputCommand::AuthController(String::new()))
        );
        assert_eq!(
            parse_local_input_command("/a"),
            Some(LocalInputCommand::AuthController(String::new()))
        );
        assert_eq!(
            parse_local_input_command("auth   "),
            Some(LocalInputCommand::AuthController(String::new()))
        );
    }

    #[test]
    fn parse_local_input_command_parses_list_aliases() {
        assert_eq!(
            parse_local_input_command("list"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("l"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("users"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("/list"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("/l"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("/users"),
            Some(LocalInputCommand::RequestUserList)
        );
    }

    #[test]
    fn parse_local_input_command_parses_help_aliases() {
        assert_eq!(
            parse_local_input_command("help"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("h"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("?"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("/help"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("/h"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("/?"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("\\?"),
            Some(LocalInputCommand::ShowHelp)
        );
    }

    #[test]
    fn local_command_help_lines_legacy_compatible_includes_expected_entries() {
        let lines = local_command_help_lines_legacy_compatible();
        assert_eq!(lines.first(), Some(&"Available commands:"));
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
    fn local_command_help_footer_lines_legacy_compatible_includes_expected_entries() {
        let lines = local_command_help_footer_lines_legacy_compatible("1.7.5");
        assert_eq!(lines[0], "Syncplay version: 1.7.5");
        assert_eq!(lines[1], "More info available at: https://syncplay.pl/");
    }

    #[test]
    fn parse_local_input_command_parses_playlist_aliases() {
        assert_eq!(
            parse_local_input_command("playlist"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("ql"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("pl"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("/playlist"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("/ql"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("/pl"),
            Some(LocalInputCommand::ShowPlaylist)
        );
    }

    #[test]
    fn parse_local_input_command_parses_select_aliases() {
        assert_eq!(
            parse_local_input_command("select 1"),
            Some(LocalInputCommand::SelectPlaylistIndex(0))
        );
        assert_eq!(
            parse_local_input_command("qs 2"),
            Some(LocalInputCommand::SelectPlaylistIndex(1))
        );
        assert_eq!(
            parse_local_input_command("/select 3"),
            Some(LocalInputCommand::SelectPlaylistIndex(2))
        );
        assert_eq!(
            parse_local_input_command("/qs 4"),
            Some(LocalInputCommand::SelectPlaylistIndex(3))
        );
        assert_eq!(
            parse_local_input_command("select"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("qs"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("/select"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("/qs"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("select 0"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
    }

    #[test]
    fn parse_local_input_command_parses_next_aliases() {
        assert_eq!(
            parse_local_input_command("next"),
            Some(LocalInputCommand::NextPlaylistItem)
        );
        assert_eq!(
            parse_local_input_command("qn"),
            Some(LocalInputCommand::NextPlaylistItem)
        );
        assert_eq!(
            parse_local_input_command("/next"),
            Some(LocalInputCommand::NextPlaylistItem)
        );
        assert_eq!(
            parse_local_input_command("/qn"),
            Some(LocalInputCommand::NextPlaylistItem)
        );
    }

    #[test]
    fn parse_local_input_command_parses_queue_aliases() {
        assert_eq!(
            parse_local_input_command("queue episode1.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode1.mkv".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("queue  "),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: " ".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("queue   episode1.mkv  "),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "  episode1.mkv  ".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("qa episode2.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode2.mkv".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("add episode3.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode3.mkv".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("/queue episode4.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode4.mkv".to_owned(),
                select_after_queue: false
            })
        );
        assert_eq!(
            parse_local_input_command("queue"),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
        assert_eq!(
            parse_local_input_command("queue "),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
        assert_eq!(
            parse_local_input_command("qa"),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
        assert_eq!(
            parse_local_input_command("add"),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
    }

    #[test]
    fn parse_local_input_command_parses_queueandselect_aliases() {
        assert_eq!(
            parse_local_input_command("queueandselect episode1.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode1.mkv".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("queueandselect  "),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: " ".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("queueandselect   episode1.mkv  "),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "  episode1.mkv  ".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("qas episode2.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode2.mkv".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("/queueandselect episode3.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode3.mkv".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("/qas episode4.mkv"),
            Some(LocalInputCommand::QueuePlaylistItem {
                file_name: "episode4.mkv".to_owned(),
                select_after_queue: true
            })
        );
        assert_eq!(
            parse_local_input_command("queueandselect"),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
        assert_eq!(
            parse_local_input_command("queueandselect "),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
        assert_eq!(
            parse_local_input_command("qas"),
            Some(LocalInputCommand::ShowQueueMissingFileError)
        );
    }

    #[test]
    fn parse_local_input_command_parses_delete_aliases() {
        assert_eq!(
            parse_local_input_command("delete 1"),
            Some(LocalInputCommand::DeletePlaylistIndex(0))
        );
        assert_eq!(
            parse_local_input_command("d 2"),
            Some(LocalInputCommand::DeletePlaylistIndex(1))
        );
        assert_eq!(
            parse_local_input_command("qd 3"),
            Some(LocalInputCommand::DeletePlaylistIndex(2))
        );
        assert_eq!(
            parse_local_input_command("/delete 4"),
            Some(LocalInputCommand::DeletePlaylistIndex(3))
        );
        assert_eq!(
            parse_local_input_command("/d 5"),
            Some(LocalInputCommand::DeletePlaylistIndex(4))
        );
        assert_eq!(
            parse_local_input_command("/qd 6"),
            Some(LocalInputCommand::DeletePlaylistIndex(5))
        );
        assert_eq!(
            parse_local_input_command("delete"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("d"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("qd"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
        assert_eq!(
            parse_local_input_command("delete 0"),
            Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
        );
    }

    #[test]
    fn parse_local_input_command_parses_playlist_undo_aliases() {
        assert_eq!(
            parse_local_input_command("undoplaylist"),
            Some(LocalInputCommand::UndoPlaylistChange)
        );
        assert_eq!(
            parse_local_input_command("/undoplaylist"),
            Some(LocalInputCommand::UndoPlaylistChange)
        );
    }

    #[test]
    fn parse_local_input_command_parses_shuffle_playlist_aliases() {
        assert_eq!(
            parse_local_input_command("shuffleremainingplaylist"),
            Some(LocalInputCommand::ShuffleRemainingPlaylist)
        );
        assert_eq!(
            parse_local_input_command("/shuffleremainingplaylist"),
            Some(LocalInputCommand::ShuffleRemainingPlaylist)
        );
        assert_eq!(
            parse_local_input_command("shuffleentireplaylist"),
            Some(LocalInputCommand::ShuffleEntirePlaylist)
        );
        assert_eq!(
            parse_local_input_command("/shuffleentireplaylist"),
            Some(LocalInputCommand::ShuffleEntirePlaylist)
        );
    }

    #[test]
    fn parse_local_input_command_parses_undo_aliases() {
        assert_eq!(
            parse_local_input_command("undo"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("u"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("revert"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("/undo"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("/u"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("/revert"),
            Some(LocalInputCommand::UndoSeek)
        );
    }

    #[test]
    fn parse_local_input_command_parses_pause_aliases() {
        assert_eq!(
            parse_local_input_command("pause"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("play"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("p"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("/pause"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("/play"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("/p"),
            Some(LocalInputCommand::TogglePause)
        );
    }

    #[test]
    fn parse_local_input_command_parses_seek_aliases() {
        assert_eq!(
            parse_local_input_command("seek 90"),
            Some(LocalInputCommand::SeekAbsolute(90.0))
        );
        assert_eq!(
            parse_local_input_command("s 1:30"),
            Some(LocalInputCommand::SeekAbsolute(90.0))
        );
        assert_eq!(
            parse_local_input_command("/seek +0:10"),
            Some(LocalInputCommand::SeekRelative(10.0))
        );
        assert_eq!(
            parse_local_input_command("/s -2:00"),
            Some(LocalInputCommand::SeekRelative(-120.0))
        );
        assert_eq!(
            parse_local_input_command("s+0:10"),
            Some(LocalInputCommand::SeekRelative(10.0))
        );
        assert_eq!(
            parse_local_input_command("seek-2:00"),
            Some(LocalInputCommand::SeekRelative(-120.0))
        );
        assert_eq!(
            parse_local_input_command("+0:05"),
            Some(LocalInputCommand::SeekRelative(5.0))
        );
        assert_eq!(
            parse_local_input_command("1:30"),
            Some(LocalInputCommand::SeekAbsolute(90.0))
        );
        assert_eq!(
            parse_local_input_command("s 1 30"),
            Some(LocalInputCommand::SeekAbsolute(90.0))
        );
        assert_eq!(
            parse_local_input_command("seek 1h02m03"),
            Some(LocalInputCommand::SeekAbsolute(3723.0))
        );
        assert_eq!(
            parse_local_input_command("seek 1234"),
            Some(LocalInputCommand::SeekAbsolute(1234.0))
        );
        assert_eq!(
            parse_local_input_command("seek 1.123"),
            Some(LocalInputCommand::SeekAbsolute(1.123))
        );
        assert_eq!(
            parse_local_input_command("seek 12:123456"),
            Some(LocalInputCommand::SeekAbsolute(124176.0))
        );
        assert_eq!(
            parse_local_input_command("+1-30"),
            Some(LocalInputCommand::SeekRelative(90.0))
        );
        assert_eq!(
            parse_local_input_command("seek"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("s"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek nope"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek  +0:10"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek 90 "),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek 1::30"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek 12345"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek 1.1234"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek 12:1234567"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("s+oops"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
    }

    #[test]
    fn parse_local_input_command_parses_offset_aliases() {
        assert_eq!(
            parse_local_input_command("offset 1:30"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Absolute(90.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("o +0:10"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Relative(10.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("o-2:00"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Relative(-120.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset /0:30"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::RelativeFromCurrentPositionMinus(30.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("o 1 30"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Absolute(90.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset +1-30"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Relative(90.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset /1h2m3"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::RelativeFromCurrentPositionMinus(3723.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset 123456789"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Absolute(123456789.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset 1.123"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Absolute(1.123)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset 12:123456789"),
            Some(LocalInputCommand::SetUserOffset(
                LocalOffsetCommand::Absolute(123457509.0)
            ))
        );
        assert_eq!(
            parse_local_input_command("offset"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("o"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset nope"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset  +0:10"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset 1:30 "),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset 1234567890"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset 1.1234"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("offset 12:1234567890"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("o+oops"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
    }

    #[test]
    fn parse_local_input_command_parses_room_aliases() {
        assert_eq!(
            parse_local_input_command("room room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("room  room2  "),
            Some(LocalInputCommand::SetRoom(" room2  ".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("room   "),
            Some(LocalInputCommand::SetRoom("  ".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("room "),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("r room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("r "),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("/room room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/room "),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("/r room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/r "),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("room"),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("r"),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("/room"),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
        assert_eq!(
            parse_local_input_command("/r"),
            Some(LocalInputCommand::SetRoomWithLegacyFallback)
        );
    }

    #[test]
    fn parse_local_input_command_parses_chat_and_unknown_slash_command_help() {
        assert_eq!(
            parse_local_input_command("hello everyone"),
            Some(LocalInputCommand::Chat("hello everyone".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("chat hello everyone"),
            Some(LocalInputCommand::Chat("hello everyone".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/ch hello"),
            Some(LocalInputCommand::Chat("hello".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("chat"),
            Some(LocalInputCommand::Chat("".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("ch"),
            Some(LocalInputCommand::Chat("".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/chat"),
            Some(LocalInputCommand::Chat("".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/msg"),
            Some(LocalInputCommand::Chat("".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("chat  "),
            Some(LocalInputCommand::Chat(" ".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/msg   hello  "),
            Some(LocalInputCommand::Chat("  hello  ".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/unknown hello"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(parse_local_input_command(" hello everyone"), None);
        assert_eq!(parse_local_input_command(" /chat hello"), None);
        assert_eq!(parse_local_input_command(" /unknown hello"), None);
    }

    #[test]
    fn parse_local_input_command_noarg_aliases_ignore_extra_parameters_legacy_style() {
        assert_eq!(
            parse_local_input_command("help now"),
            Some(LocalInputCommand::ShowHelp)
        );
        assert_eq!(
            parse_local_input_command("list now"),
            Some(LocalInputCommand::RequestUserList)
        );
        assert_eq!(
            parse_local_input_command("playlist now"),
            Some(LocalInputCommand::ShowPlaylist)
        );
        assert_eq!(
            parse_local_input_command("next now"),
            Some(LocalInputCommand::NextPlaylistItem)
        );
        assert_eq!(
            parse_local_input_command("toggle now"),
            Some(LocalInputCommand::ToggleReady)
        );
        assert_eq!(
            parse_local_input_command("p now"),
            Some(LocalInputCommand::TogglePause)
        );
        assert_eq!(
            parse_local_input_command("undo now"),
            Some(LocalInputCommand::UndoSeek)
        );
        assert_eq!(
            parse_local_input_command("undoplaylist now"),
            Some(LocalInputCommand::UndoPlaylistChange)
        );
        assert_eq!(
            parse_local_input_command("shuffleremainingplaylist now"),
            Some(LocalInputCommand::ShuffleRemainingPlaylist)
        );
        assert_eq!(
            parse_local_input_command("shuffleentireplaylist now"),
            Some(LocalInputCommand::ShuffleEntirePlaylist)
        );
    }

    #[test]
    fn parse_local_input_command_noarg_aliases_require_literal_space_delimiter() {
        assert_eq!(
            parse_local_input_command("/help\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/list\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/playlist\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/next\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/pause\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/toggle\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/undo\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("/undoplaylist\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
    }

    #[test]
    fn parse_local_input_command_known_tokens_with_tab_delimiter_show_unknown_help() {
        assert_eq!(
            parse_local_input_command("help\tplease"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("chat\thello"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("queue\tmovie.mkv"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("room\troom2"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("setready\tbob"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
        assert_eq!(
            parse_local_input_command("seek\t1:30"),
            Some(LocalInputCommand::ShowUnknownCommandHelp)
        );
    }

    #[test]
    fn user_change_notification_message_uses_legacy_style_wording() {
        assert_eq!(
            user_change_notification_message(&UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: true,
            }),
            "bob has joined the room: 'room1'"
        );
        assert_eq!(
            user_change_notification_message(&UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: None,
                include_room_addendum: true,
                hide_from_osd: false,
            }),
            "bob is playing 'movie.mkv' in room: 'room1'"
        );
        assert_eq!(
            user_change_notification_message(&UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: None,
                include_room_addendum: false,
                hide_from_osd: false,
            }),
            "bob is playing 'movie.mkv'"
        );
        assert_eq!(
            user_change_notification_message(&UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: None,
                file_duration: None,
                include_room_addendum: false,
                hide_from_osd: false,
            }),
            "bob is playing a file"
        );
        assert_eq!(
            user_change_notification_message(&UserChangeNotification::Left {
                username: "bob".to_owned(),
                hide_from_osd: true,
            }),
            "bob has left"
        );
    }

    #[test]
    fn user_change_notification_hidden_from_osd_uses_visibility_metadata() {
        assert!(user_change_notification_hidden_from_osd(
            &UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: true,
            }
        ));
        assert!(!user_change_notification_hidden_from_osd(
            &UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: None,
                include_room_addendum: false,
                hide_from_osd: false,
            }
        ));
        assert!(user_change_notification_hidden_from_osd(
            &UserChangeNotification::Left {
                username: "bob".to_owned(),
                hide_from_osd: true,
            }
        ));
    }

    #[test]
    fn format_duration_legacy_matches_python_shape() {
        assert_eq!(format_duration_legacy(95.5), "01:36");
        assert_eq!(format_duration_legacy(3600.0), "01:00:00");
        assert_eq!(format_duration_legacy(604800.0), "00:00 (Title 1)");
        assert_eq!(format_duration_legacy(-1.5), "-00:02");
    }

    #[test]
    fn user_change_playing_message_includes_formatted_duration_when_available() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"movie.mkv","duration":95.5}}}}}"#,
            )
            .expect("playing update should apply");
        runtime
            .run_user_change_notifications_if_needed()
            .expect("user-change notification dispatch should succeed");

        let mut captured = Vec::new();
        flush_user_change_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(user_change_notification_message(notification));
            Ok(())
        })
        .expect("notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec!["bob is playing 'movie.mkv' (01:36) in room: 'room2'"]
        );
    }

    #[tokio::test]
    async fn connected_client_session_sends_hello_and_applies_inbound_set_ready() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            writer
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write ready update");
            writer.flush().await.expect("server flush should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 2.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");

        assert_eq!(runtime.session().user_ready("cli-user"), Some(true));
    }

    #[tokio::test]
    async fn connected_client_session_sends_chat_message_when_configured() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut chat_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("chat line read should not timeout")
                    .expect("chat line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                if let ProtocolMessage::Chat(payload) = message {
                    chat_payload = Some(payload.chat);
                    break;
                }
            }
            let Some(chat_payload) = chat_payload else {
                panic!("client should emit a chat message line after server hello");
            };
            assert_eq!(
                chat_payload,
                syncplay_protocol::ChatPayload::Text("hello room".to_owned())
            );
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            Some("hello room"),
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_omits_connect_chat_when_server_disables_chat() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":false}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            for _ in 0..4 {
                let line = match tokio::time::timeout(Duration::from_millis(200), lines.next_line())
                    .await
                {
                    Ok(Ok(Some(line))) => line,
                    Ok(Ok(None)) => break,
                    Ok(Err(error)) => panic!("line read should succeed: {error}"),
                    Err(_) => break,
                };
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "chat-disabled server should not receive connect-time chat line"
                );
            }
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            Some("hello room"),
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sends_chat_message_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut chat_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("chat line read should not timeout")
                    .expect("chat line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                if let ProtocolMessage::Chat(payload) = message {
                    chat_payload = Some(payload.chat);
                    break;
                }
            }
            let Some(chat_payload) = chat_payload else {
                panic!("client should emit chat from local input channel");
            };
            assert_eq!(
                chat_payload,
                syncplay_protocol::ChatPayload::Text("hello room".to_owned())
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("/chat hello room".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sends_empty_chat_message_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut chat_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("chat line read should not timeout")
                    .expect("chat line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                if let ProtocolMessage::Chat(payload) = message {
                    chat_payload = Some(payload.chat);
                    break;
                }
            }
            let Some(chat_payload) = chat_payload else {
                panic!("client should emit empty chat from local input channel");
            };
            assert_eq!(
                chat_payload,
                syncplay_protocol::ChatPayload::Text("".to_owned())
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("chat".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_preserves_whitespace_chat_message_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut chat_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("chat line read should not timeout")
                    .expect("chat line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                if let ProtocolMessage::Chat(payload) = message {
                    chat_payload = Some(payload.chat);
                    break;
                }
            }
            let Some(chat_payload) = chat_payload else {
                panic!("client should emit whitespace chat from local input channel");
            };
            assert_eq!(
                chat_payload,
                syncplay_protocol::ChatPayload::Text(" ".to_owned())
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("chat  ".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_ignores_leading_space_local_input_lines() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let maybe_outbound =
                tokio::time::timeout(Duration::from_millis(350), lines.next_line()).await;
            if let Ok(Ok(Some(line))) = maybe_outbound {
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "leading-space local line should not emit outbound chat"
                );
            }

            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send(" /chat hello room".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_does_not_chat_fallback_for_tab_delimited_chat_command() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let maybe_outbound =
                tokio::time::timeout(Duration::from_millis(350), lines.next_line()).await;
            if let Ok(Ok(Some(line))) = maybe_outbound {
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "tab-delimited known-token input should not emit outbound chat"
                );
            }

            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("chat\thello room".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_tab_delimited_known_tokens_do_not_emit_outbound_actions() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(400);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };

                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "tab-delimited known-token input should not emit outbound chat"
                );
                assert!(
                    !matches!(message, ProtocolMessage::State(_)),
                    "tab-delimited known-token input should not emit outbound state"
                );
                if let ProtocolMessage::Set(ref payload) = message {
                    assert!(
                        payload.set.room.is_none()
                            && payload.set.ready.is_none()
                            && payload.set.playlist_change.is_none()
                            && payload.set.playlist_index.is_none()
                            && payload.set.controller_auth.is_none(),
                        "tab-delimited known-token input should not emit local command set messages: {payload:?}"
                    );
                }
            }

            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.6,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            sender
                .send("chat\thello room".to_owned())
                .expect("chat command should queue");
            sender
                .send("queue\tmovie.mkv".to_owned())
                .expect("queue command should queue");
            sender
                .send("room\tother-room".to_owned())
                .expect("room command should queue");
            sender
                .send("auth\tAB-123-456".to_owned())
                .expect("auth command should queue");
            sender
                .send("create\tlocked-room".to_owned())
                .expect("create command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_toggles_ready_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut ready_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("ready line read should not timeout")
                    .expect("ready line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(ready) = payload.set.ready {
                    ready_payload = Some(ready);
                    break;
                }
            }
            let Some(ready_payload) = ready_payload else {
                panic!("client should emit Set.ready from local toggle command");
            };
            assert!(ready_payload.is_ready);
            assert_eq!(ready_payload.manually_initiated, Some(true));
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("toggle".to_owned())
                .expect("toggle command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_requests_user_list_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut saw_list_request = false;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("list line read should not timeout")
                    .expect("list line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::List(payload) = message else {
                    continue;
                };
                if matches!(payload.list, ListPayload::Request(_)) {
                    saw_list_request = true;
                    break;
                }
            }
            assert!(
                saw_list_request,
                "client should emit List request from local list/users command"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("list".to_owned())
                .expect("list command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_advances_playlist_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":0,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut saw_next_index = false;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist next line read should not timeout")
                    .expect("playlist next line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                let Some(playlist_index) = payload.set.playlist_index else {
                    continue;
                };
                if playlist_index.index == 1 {
                    saw_next_index = true;
                    break;
                }
            }
            assert!(
                saw_next_index,
                "client should emit Set.playlistIndex with next index from local next command"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("next".to_owned())
                .expect("next command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_queues_and_selects_playlist_item_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":0,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut queued_files = None;
            let mut queued_index = None;
            for _ in 0..8 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist queue line read should not timeout")
                    .expect("playlist queue line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if queued_files.is_none() {
                    if let Some(change) = payload.set.playlist_change.as_ref() {
                        queued_files = Some(change.files.clone());
                        continue;
                    }
                }
                if queued_index.is_none() {
                    if let Some(index) = payload.set.playlist_index.as_ref() {
                        queued_index = Some(index.index);
                    }
                }
                if queued_files.is_some() && queued_index.is_some() {
                    break;
                }
            }
            assert_eq!(
                queued_files,
                Some(vec![
                    "episode1.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    "episode3.mkv".to_owned()
                ]),
                "queue-and-select should emit playlistChange with appended file"
            );
            assert_eq!(
                queued_index,
                Some(2),
                "queue-and-select should emit playlistIndex targeting appended file"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("qas episode3.mkv".to_owned())
                .expect("queue-and-select command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_queues_whitespace_file_name_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":0,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut queued_files = None;
            for _ in 0..8 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist queue line read should not timeout")
                    .expect("playlist queue line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(change) = payload.set.playlist_change.as_ref() {
                    queued_files = Some(change.files.clone());
                    break;
                }
            }
            assert_eq!(
                queued_files,
                Some(vec![
                    "episode1.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    " ".to_owned()
                ]),
                "queue command should preserve whitespace-only file names"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("queue  ".to_owned())
                .expect("queue should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_deletes_playlist_item_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":2,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut deleted_files = None;
            let mut deleted_index = None;
            for _ in 0..8 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist delete line read should not timeout")
                    .expect("playlist delete line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if deleted_files.is_none() {
                    if let Some(change) = payload.set.playlist_change.as_ref() {
                        deleted_files = Some(change.files.clone());
                        continue;
                    }
                }
                if deleted_index.is_none() {
                    if let Some(index) = payload.set.playlist_index.as_ref() {
                        deleted_index = Some(index.index);
                    }
                }
                if deleted_files.is_some() && deleted_index.is_some() {
                    break;
                }
            }
            assert_eq!(
                deleted_files,
                Some(vec!["episode1.mkv".to_owned(), "episode3.mkv".to_owned()]),
                "delete command should emit playlistChange without removed file"
            );
            assert_eq!(
                deleted_index,
                Some(1),
                "delete command should emit playlistIndex adjusted to remaining file"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("delete 2".to_owned())
                .expect("delete command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_undoes_playlist_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut undone_files = None;
            for _ in 0..8 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist undo line read should not timeout")
                    .expect("playlist undo line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(change) = payload.set.playlist_change.as_ref() {
                    undone_files = Some(change.files.clone());
                    break;
                }
            }
            assert_eq!(
                undone_files,
                Some(Vec::<String>::new()),
                "playlist undo command should emit playlistChange with previous snapshot"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("undoplaylist".to_owned())
                .expect("undo playlist command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_shuffles_remaining_playlist_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv","episode4.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut shuffled_files = None;
            let mut shuffled_index = None;
            for _ in 0..24 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist shuffle-remaining line read should not timeout")
                    .expect("playlist shuffle-remaining line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if shuffled_files.is_none() {
                    if let Some(change) = payload.set.playlist_change.as_ref() {
                        shuffled_files = Some(change.files.clone());
                        continue;
                    }
                }
                if shuffled_index.is_none() {
                    if let Some(index) = payload.set.playlist_index.as_ref() {
                        shuffled_index = Some(index.index);
                    }
                }
                if shuffled_files.is_some() && shuffled_index.is_some() {
                    break;
                }
            }
            let Some(shuffled_files) = shuffled_files else {
                panic!("shuffle remaining command should emit Set.playlistChange");
            };
            assert_eq!(
                &shuffled_files[..2],
                &["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                "shuffle remaining command should keep entries up to current index unchanged"
            );
            let mut expected_tail = vec!["episode3.mkv".to_owned(), "episode4.mkv".to_owned()];
            let mut actual_tail = shuffled_files[2..].to_vec();
            expected_tail.sort();
            actual_tail.sort();
            assert_eq!(
                actual_tail, expected_tail,
                "shuffle remaining command should only permute remaining entries"
            );
            assert_eq!(
                shuffled_index,
                Some(1),
                "shuffle remaining command should preserve current playlist index"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.7,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            for _ in 0..8 {
                tokio::time::sleep(Duration::from_millis(60)).await;
                sender
                    .send("shuffleremainingplaylist".to_owned())
                    .expect("shuffle remaining command should queue");
            }
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_shuffles_entire_playlist_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":2,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let mut shuffled_files = None;
            let mut saw_index_reset = false;
            for _ in 0..12 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("playlist shuffle-entire line read should not timeout")
                    .expect("playlist shuffle-entire line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if shuffled_files.is_none() {
                    if let Some(change) = payload.set.playlist_change.as_ref() {
                        shuffled_files = Some(change.files.clone());
                        continue;
                    }
                }
                if let Some(index) = payload.set.playlist_index.as_ref() {
                    if index.index == 0 {
                        saw_index_reset = true;
                    }
                }
                if saw_index_reset {
                    break;
                }
            }
            assert!(
                saw_index_reset,
                "shuffle entire command should emit Set.playlistIndex resetting index to zero"
            );
            if let Some(shuffled_files) = shuffled_files {
                let mut expected = vec![
                    "episode1.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    "episode3.mkv".to_owned(),
                ];
                let mut actual = shuffled_files;
                expected.sort();
                actual.sort();
                assert_eq!(
                    actual, expected,
                    "shuffle entire command should keep playlist membership unchanged"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("shuffleentireplaylist".to_owned())
                .expect("shuffle entire command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_other_user_ready_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut target_ready_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("ready line read should not timeout")
                    .expect("ready line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(ready) = payload.set.ready {
                    if ready.username.as_deref() == Some("other-user") {
                        target_ready_payload = Some(ready);
                        break;
                    }
                }
            }
            let Some(ready_payload) = target_ready_payload else {
                panic!("client should emit Set.ready with username from local setready command");
            };
            assert!(ready_payload.is_ready);
            assert_eq!(ready_payload.manually_initiated, Some(true));
            assert_eq!(ready_payload.username.as_deref(), Some("other-user"));
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("sr other-user".to_owned())
                .expect("setready command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_explicit_local_username_ready_from_local_input_channel()
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut target_ready_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("ready line read should not timeout")
                    .expect("ready line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(ready) = payload.set.ready {
                    if ready.username.as_deref() == Some("cli-user") {
                        target_ready_payload = Some(ready);
                        break;
                    }
                }
            }
            let Some(ready_payload) = target_ready_payload else {
                panic!(
                    "client should emit Set.ready with username from explicit local username command"
                );
            };
            assert!(ready_payload.is_ready);
            assert_eq!(ready_payload.manually_initiated, Some(true));
            assert_eq!(ready_payload.username.as_deref(), Some("cli-user"));
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("sr cli-user".to_owned())
                .expect("setready command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_whitespace_username_ready_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut target_ready_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("ready line read should not timeout")
                    .expect("ready line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(ready) = payload.set.ready {
                    if ready.username.as_deref() == Some(" ") {
                        target_ready_payload = Some(ready);
                        break;
                    }
                }
            }
            let Some(ready_payload) = target_ready_payload else {
                panic!(
                    "client should emit Set.ready with whitespace username from local setready command"
                );
            };
            assert!(ready_payload.is_ready);
            assert_eq!(ready_payload.manually_initiated, Some(true));
            assert_eq!(ready_payload.username.as_deref(), Some(" "));
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("setready  ".to_owned())
                .expect("setready command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_local_ready_without_username_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"readiness\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut local_ready_updates = Vec::new();
            for _ in 0..12 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("ready line read should not timeout")
                    .expect("ready line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(ready) = payload.set.ready {
                    if ready.username.is_none() {
                        local_ready_updates.push((ready.is_ready, ready.manually_initiated));
                    }
                }
            }

            assert!(
                local_ready_updates.contains(&(true, Some(true))),
                "local setready command should emit a manually-initiated local ready update"
            );
            assert!(
                local_ready_updates.contains(&(false, Some(true))),
                "local setnotready command should emit a manually-initiated local not-ready update"
            );

            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("setready".to_owned())
                .expect("setready command should queue");
            sender
                .send("setnotready".to_owned())
                .expect("setnotready command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_creates_controlled_room_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+managed-room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let mut controller_auth_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("controller auth line read should not timeout")
                    .expect("controller auth line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(controller_auth) = payload.set.controller_auth {
                    controller_auth_payload = Some(controller_auth);
                    break;
                }
            }
            let Some(controller_auth_payload) = controller_auth_payload else {
                panic!("client should emit Set.controllerAuth from local create command");
            };
            assert_eq!(
                controller_auth_payload.room.as_deref(),
                Some("managed-room")
            );
            let Some(password) = controller_auth_payload.password.as_deref() else {
                panic!("controller auth payload should include password");
            };
            assert!(
                is_legacy_generated_room_password_shape(password),
                "create command password should match legacy generated password shape"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "fallback-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("create".to_owned())
                .expect("create command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_creates_controlled_room_with_whitespace_parameter_from_local_input_channel()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+managed-room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let mut controller_auth_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("controller auth line read should not timeout")
                    .expect("controller auth line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(controller_auth) = payload.set.controller_auth {
                    controller_auth_payload = Some(controller_auth);
                    break;
                }
            }
            let Some(controller_auth_payload) = controller_auth_payload else {
                panic!("client should emit Set.controllerAuth from local create command");
            };
            assert_eq!(controller_auth_payload.room.as_deref(), Some(" "));
            let Some(password) = controller_auth_payload.password.as_deref() else {
                panic!("controller auth payload should include password");
            };
            assert!(
                is_legacy_generated_room_password_shape(password),
                "create command password should match legacy generated password shape"
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "fallback-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("create  ".to_owned())
                .expect("create command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_authenticates_controller_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let mut controller_auth_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("controller auth line read should not timeout")
                    .expect("controller auth line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(controller_auth) = payload.set.controller_auth {
                    controller_auth_payload = Some(controller_auth);
                    break;
                }
            }
            let Some(controller_auth_payload) = controller_auth_payload else {
                panic!("client should emit Set.controllerAuth from local auth command");
            };
            assert_eq!(
                controller_auth_payload.room.as_deref(),
                Some("+room:ABCDEF123456")
            );
            assert_eq!(
                controller_auth_payload.password.as_deref(),
                Some("AB123-456")
            );
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("auth ab_123-456!".to_owned())
                .expect("auth command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_authenticates_controller_without_password_from_local_input_channel()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let mut controller_auth_payload = None;
            for _ in 0..6 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("controller auth line read should not timeout")
                    .expect("controller auth line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(controller_auth) = payload.set.controller_auth {
                    controller_auth_payload = Some(controller_auth);
                    break;
                }
            }
            let Some(controller_auth_payload) = controller_auth_payload else {
                panic!("client should emit Set.controllerAuth from local bare auth command");
            };
            assert_eq!(
                controller_auth_payload.room.as_deref(),
                Some("+room:ABCDEF123456")
            );
            assert_eq!(controller_auth_payload.password.as_deref(), Some(""));
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("auth".to_owned())
                .expect("auth command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_room_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut set_room_name = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("set room line read should not timeout")
                    .expect("set room line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(room) = payload.set.room {
                    set_room_name = Some(room.name);
                    break;
                }
            }
            let Some(set_room_name) = set_room_name else {
                panic!("client should emit Set.room from local room command");
            };
            assert_eq!(set_room_name, "room2");
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("room room2".to_owned())
                .expect("room command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_room_with_whitespace_preserved_from_local_input_channel()
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut set_room_name = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("set room line read should not timeout")
                    .expect("set room line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(room) = payload.set.room {
                    set_room_name = Some(room.name);
                    break;
                }
            }
            let Some(set_room_name) = set_room_name else {
                panic!("client should emit Set.room from local room command");
            };
            assert_eq!(set_room_name, " room2  ");
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("room  room2  ".to_owned())
                .expect("room command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_applies_legacy_fallback_for_room_command_with_single_trailing_space()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut set_room_name = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("set room line read should not timeout")
                    .expect("set room line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(room) = payload.set.room {
                    set_room_name = Some(room.name);
                    break;
                }
            }
            let Some(set_room_name) = set_room_name else {
                panic!("client should emit Set.room from legacy fallback room command");
            };
            assert_eq!(set_room_name, "cli-room");
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("room ".to_owned())
                .expect("room command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_room_from_local_input_channel_even_when_unchanged() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut set_room_name = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("set room line read should not timeout")
                    .expect("set room line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(room) = payload.set.room {
                    set_room_name = Some(room.name);
                    break;
                }
            }
            let Some(set_room_name) = set_room_name else {
                panic!("client should emit Set.room for unchanged local room command");
            };
            assert_eq!(set_room_name, "cli-room");
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("room cli-room".to_owned())
                .expect("room command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_sets_room_with_legacy_fallback_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut set_room_name = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("set room line read should not timeout")
                    .expect("set room line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                if let Some(room) = payload.set.room {
                    set_room_name = Some(room.name);
                    break;
                }
            }
            let Some(set_room_name) = set_room_name else {
                panic!("client should emit Set.room from local room fallback command");
            };
            assert_eq!(set_room_name, "fallback-room");
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "fallback-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("room".to_owned())
                .expect("room fallback command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_shows_playlist_from_local_input_channel_without_outbound_messages()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}
{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}
"#,
                )
                .await
                .expect("server hello and playlist snapshot writes should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                assert!(
                    payload.set.playlist_change.is_none() && payload.set.playlist_index.is_none(),
                    "playlist display command should not emit outbound playlist set messages"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("playlist".to_owned())
                .expect("playlist command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_shows_help_from_local_input_channel_without_outbound_messages()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                assert!(
                    payload.set.playlist_change.is_none() && payload.set.playlist_index.is_none(),
                    "help command should not emit outbound playlist set messages"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("help please".to_owned())
                .expect("help command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_shows_unknown_command_help_without_outbound_messages() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                let ProtocolMessage::Set(payload) = message else {
                    continue;
                };
                assert!(
                    payload.set.playlist_change.is_none() && payload.set.playlist_index.is_none(),
                    "unknown slash command help should not emit outbound playlist set messages"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("/unknown hello".to_owned())
                .expect("unknown command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_invalid_playlist_commands_do_not_fall_back_to_chat_or_emit_playlist_updates()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "invalid local playlist commands should not fall back to chat messages"
                );
                if let ProtocolMessage::Set(ref payload) = message {
                    assert!(
                        payload.set.playlist_change.is_none()
                            && payload.set.playlist_index.is_none(),
                        "invalid local playlist commands should not emit outbound playlist set messages"
                    );
                }
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender
                .send("queue".to_owned())
                .expect("queue command should queue");
            sender
                .send("select".to_owned())
                .expect("select command should queue");
            sender
                .send("delete".to_owned())
                .expect("delete command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_invalid_seek_offset_commands_show_help_without_falling_back_to_chat()
     {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

            let scan_deadline = tokio::time::Instant::now() + Duration::from_millis(350);
            loop {
                let remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }

                let next_line = tokio::time::timeout(remaining, lines.next_line()).await;
                let Ok(Ok(Some(line))) = next_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "invalid seek/offset commands should not fall back to chat messages"
                );
                if let ProtocolMessage::Set(ref payload) = message {
                    assert!(
                        payload.set.room.is_none()
                            && payload.set.ready.is_none()
                            && payload.set.playlist_change.is_none()
                            && payload.set.playlist_index.is_none()
                            && payload.set.controller_auth.is_none(),
                        "invalid seek/offset commands should not emit local-command set messages: {payload:?}"
                    );
                }
                assert!(
                    !matches!(message, ProtocolMessage::State(_)),
                    "invalid seek/offset commands should not emit outbound state messages"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            sender.send("seek".to_owned()).expect("seek should queue");
            sender
                .send("seek nope".to_owned())
                .expect("seek with invalid parameter should queue");
            sender
                .send("offset".to_owned())
                .expect("offset should queue");
            sender
                .send("o+oops".to_owned())
                .expect("offset shorthand with invalid parameter should queue");
            sender
                .send("s+oops".to_owned())
                .expect("seek shorthand with invalid parameter should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_toggles_pause_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
            let _ = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender
                .send("pause".to_owned())
                .expect("pause command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        assert!(
            runtime.player().paused(),
            "local pause command should toggle player paused state"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_seeks_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
            let _ = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        sender
            .send("seek 42".to_owned())
            .expect("seek command should queue");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        assert_eq!(
            runtime.player().position_seconds(),
            42.0,
            "local seek command should update player position"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_applies_offset_commands_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
            let _ = tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        sender
            .send("offset 5".to_owned())
            .expect("absolute offset command should queue");
        sender
            .send("o +2".to_owned())
            .expect("relative offset command should queue");
        sender
            .send("o /3".to_owned())
            .expect("slash-relative offset command should queue");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        assert_eq!(
            runtime.player().position_seconds(),
            4.0,
            "offset command sequence should adjust local player position with legacy-like math"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_undoes_seek_from_local_input_channel() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.7.5\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");
            let _ = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        sender
            .send("seek 12".to_owned())
            .expect("seek command should queue");
        sender
            .send("undo".to_owned())
            .expect("undo command should queue");
        sender
            .send("undo".to_owned())
            .expect("second undo command should queue");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        assert_eq!(
            runtime.player().position_seconds(),
            12.0,
            "seek + undo + undo sequence should restore the seek target"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_drops_local_chat_before_server_hello() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            let early_line =
                tokio::time::timeout(Duration::from_millis(150), lines.next_line()).await;
            assert!(
                early_line.is_err(),
                "pre-hello local chat should not produce outbound protocol lines"
            );

            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            for _ in 0..3 {
                let maybe_line =
                    tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
                let Ok(Ok(Some(line))) = maybe_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "pre-hello local chat should not be queued and sent after server hello"
                );
            }
            writer
                .shutdown()
                .await
                .expect("server shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let (sender, mut receiver) = unbounded_channel::<String>();
        tokio::spawn(async move {
            sender
                .send("/chat hello too soon".to_owned())
                .expect("chat command should queue");
        });
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert!(
            matches!(
                exit,
                ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
            ),
            "connected session should either observe peer close or exit on runtime window"
        );
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_drops_local_chat_queued_between_disconnect_and_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            {
                let (socket_1, _) = listener
                    .accept()
                    .await
                    .expect("first accept should succeed");
                let (reader_1, mut writer_1) = socket_1.into_split();
                let mut first_lines = BufReader::new(reader_1).lines();
                let first_hello = first_lines
                    .next_line()
                    .await
                    .expect("first hello read should succeed")
                    .expect("first hello should be present");
                assert!(
                    first_hello.contains("\"Hello\""),
                    "first connection should receive hello"
                );
                writer_1
                    .shutdown()
                    .await
                    .expect("first writer shutdown should succeed");
            }

            let (socket_2, _) = listener
                .accept()
                .await
                .expect("second accept should succeed");
            let (reader_2, mut writer_2) = socket_2.into_split();
            let mut second_lines = BufReader::new(reader_2).lines();
            let second_hello = second_lines
                .next_line()
                .await
                .expect("second hello read should succeed")
                .expect("second hello should be present");
            assert!(
                second_hello.contains("\"Hello\""),
                "second connection should receive hello"
            );

            tokio::time::sleep(Duration::from_millis(200)).await;
            writer_2
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("second server hello write should succeed");
            writer_2
                .flush()
                .await
                .expect("second server hello flush should succeed");

            for _ in 0..3 {
                let maybe_line =
                    tokio::time::timeout(Duration::from_millis(200), second_lines.next_line())
                        .await;
                let Ok(Ok(Some(line))) = maybe_line else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                assert!(
                    !matches!(message, ProtocolMessage::Chat(_)),
                    "chat queued during disconnect should not be replayed on reconnect"
                );
            }

            writer_2
                .shutdown()
                .await
                .expect("second writer shutdown should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let (sender, mut receiver) = unbounded_channel::<String>();
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let stream_1 = TcpStream::connect(addr)
            .await
            .expect("client should connect for first session");
        let first_exit = run_connected_client_session(
            stream_1,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("first connected session should run");
        assert_eq!(first_exit, ConnectedSessionExit::TransportClosed);

        runtime
            .run_disconnect(0.1)
            .expect("disconnect transition should be applied between sessions");
        sender
            .send("/chat reconnect gap message".to_owned())
            .expect("chat command should queue");

        let stream_2 = TcpStream::connect(addr)
            .await
            .expect("client should connect for second session");
        let second_exit = run_connected_client_session(
            stream_2,
            &mut runtime,
            &config,
            None,
            Some(&mut receiver),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("second connected session should run");
        assert_eq!(second_exit, ConnectedSessionExit::TransportClosed);

        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_truncates_chat_message_to_session_max_length() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );
            writer
                .write_all(
                    b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true}}}\n",
                )
                .await
                .expect("server hello write should succeed");

            let mut chat_payload = None;
            for _ in 0..4 {
                let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("chat line read should not timeout")
                    .expect("chat line read should succeed")
                else {
                    break;
                };
                let message = decode_message_line(&line).expect("line should decode");
                if let ProtocolMessage::Chat(payload) = message {
                    chat_payload = Some(payload.chat);
                    break;
                }
            }
            let Some(chat_payload) = chat_payload else {
                panic!("client should emit chat line after server hello");
            };
            assert_eq!(
                chat_payload,
                syncplay_protocol::ChatPayload::Text("hello".to_owned())
            );
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let chat_config = runtime.session_mut().chat_config_mut();
        chat_config.max_chat_message_length = 5;
        chat_config.apply_server_max_chat_message_length = false;
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            Some("hello room"),
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_publishes_pending_local_file_update() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, _writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            let set_file_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("set file line read should not timeout")
                .expect("set file line read should succeed")
                .expect("set file line should be present");
            let set_file_message =
                decode_message_line(&set_file_line).expect("set file line should decode");
            let ProtocolMessage::Set(set_message) = set_file_message else {
                panic!("second client line should be Set.file");
            };
            let file = set_message
                .set
                .file
                .expect("second client line should include file payload");
            assert_eq!(file.name.as_deref(), Some("movie.mkv"));
            assert_eq!(file.size.as_ref().and_then(|value| value.as_u64()), Some(0));
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .player_mut()
            .open_file("movie.mkv")
            .expect("mpv adapter should accept local file open");
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
        assert_eq!(runtime.session().user_has_file("cli-user"), Some(true));
    }

    #[tokio::test]
    async fn connected_client_session_restores_playlist_after_reconnect_empty_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            writer
                .write_all(
                    br#"{"Set":{"playlistChange":{"files":[]}}}
"#,
                )
                .await
                .expect("server should write empty playlist snapshot");
            writer.flush().await.expect("server flush should succeed");

            let mut outbound_messages = Vec::new();
            for _ in 0..4 {
                let maybe_line =
                    tokio::time::timeout(Duration::from_millis(300), lines.next_line()).await;
                let line = match maybe_line {
                    Ok(Ok(Some(line))) => line,
                    Ok(Ok(None)) => break,
                    Ok(Err(read_err)) => panic!("outbound line read should succeed: {read_err}"),
                    Err(_) => break,
                };
                outbound_messages
                    .push(decode_message_line(&line).expect("outbound line should decode"));
            }

            assert!(
                outbound_messages.iter().any(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(set_message)
                            if set_message
                                .set
                                .playlist_change
                                .as_ref()
                                .is_some_and(|payload| payload.files == vec!["episode1.mkv", "episode2.mkv"])
                    )
                }),
                "reconnect restore should emit playlistChange with cached files"
            );
            assert!(
                outbound_messages.iter().any(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(set_message)
                            if set_message
                                .set
                                .playlist_index
                                .as_ref()
                                .is_some_and(|payload| payload.index == 1)
                    )
                }),
                "reconnect restore should emit playlistIndex with cached index"
            );
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255"}}"#,
            )
            .expect("precondition hello should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"cli-user"}}}"#,
            )
            .expect("precondition playlist change should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}"#)
            .expect("precondition playlist index should apply");
        runtime.session_mut().reset_sync_state_for_reconnect();

        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_restores_ready_and_file_after_reconnect_hello() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255"}}
"#,
                )
                .await
                .expect("server should write reconnect hello");
            writer.flush().await.expect("server flush should succeed");

            let mut outbound_messages = Vec::new();
            for _ in 0..2 {
                let line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("outbound line read should not timeout")
                    .expect("outbound line read should succeed")
                    .expect("outbound line should be present");
                outbound_messages
                    .push(decode_message_line(&line).expect("outbound line should decode"));
            }

            assert!(
                outbound_messages.iter().any(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(set_message)
                            if set_message
                                .set
                                .ready
                                .as_ref()
                                .is_some_and(|ready| ready.is_ready && ready.manually_initiated == Some(false))
                    )
                }),
                "reconnect restore should emit Set.ready with restored value"
            );
            assert!(
                outbound_messages.iter().any(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(set_message)
                            if set_message
                                .set
                                .file
                                .as_ref()
                                .is_some_and(|file| file.name.as_deref() == Some("movie.mkv"))
                    )
                }),
                "reconnect restore should emit Set.file with restored metadata"
            );
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: Some(false),
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255"}}"#,
            )
            .expect("precondition hello should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"cli-user"}}}"#)
            .expect("precondition ready should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"cli-user":{"room":{"name":"cli-room"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("precondition file metadata should apply");
        runtime.session_mut().reset_sync_state_for_reconnect();

        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_reidentifies_controller_when_password_is_configured() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}
"#,
                )
                .await
                .expect("server should write hello response");
            writer.flush().await.expect("server flush should succeed");

            let controller_auth_line =
                tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .expect("controller auth read should not timeout")
                    .expect("controller auth read should succeed")
                    .expect("controller auth line should be present");
            let controller_auth_message = decode_message_line(&controller_auth_line)
                .expect("controller auth line should decode");
            let ProtocolMessage::Set(set_message) = controller_auth_message else {
                panic!("second client line should be Set.controllerAuth");
            };
            let controller_auth = set_message
                .set
                .controller_auth
                .expect("controller auth message should include controllerAuth payload");
            assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
            assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: Some("ab-123-456".to_owned()),
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn connected_client_session_switches_and_identifies_on_new_controlled_room() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("server should accept");
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();

            let hello_line = lines
                .next_line()
                .await
                .expect("hello line read should succeed")
                .expect("hello line should be present");
            assert!(
                hello_line.contains("\"Hello\""),
                "first client line should be a Hello message"
            );

            writer
                .write_all(
                    br#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}
"#,
                )
                .await
                .expect("server should write new controlled room payload");
            writer.flush().await.expect("server flush should succeed");

            let room_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("room update read should not timeout")
                .expect("room update read should succeed")
                .expect("room update line should be present");
            let room_message = decode_message_line(&room_line).expect("room update should decode");
            let ProtocolMessage::Set(room_set) = room_message else {
                panic!("second client line should be Set.room");
            };
            let room_payload = room_set
                .set
                .room
                .expect("second client line should include room payload");
            assert_eq!(room_payload.name, "+room:ABCDEF123456");

            let auth_line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("controller auth read should not timeout")
                .expect("controller auth read should succeed")
                .expect("controller auth line should be present");
            let auth_message =
                decode_message_line(&auth_line).expect("controller auth should decode");
            let ProtocolMessage::Set(auth_set) = auth_message else {
                panic!("third client line should be Set.controllerAuth");
            };
            let controller_auth = auth_set
                .set
                .controller_auth
                .expect("third client line should include controllerAuth payload");
            assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
            assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 0.5,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        let stream = TcpStream::connect(addr)
            .await
            .expect("client should connect to test listener");
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;

        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            None,
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await
        .expect("connected session should run");
        assert_eq!(exit, ConnectedSessionExit::TransportClosed);
        server_task.await.expect("server task join should succeed");
    }

    #[tokio::test]
    async fn client_network_loop_reconnects_after_transport_close() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        let server_task = tokio::spawn(async move {
            {
                let (socket_1, _) = listener
                    .accept()
                    .await
                    .expect("first accept should succeed");
                let (reader_1, _writer_1) = socket_1.into_split();
                let mut first_lines = BufReader::new(reader_1).lines();
                let first_hello = first_lines
                    .next_line()
                    .await
                    .expect("first hello read should succeed")
                    .expect("first hello should be present");
                assert!(
                    first_hello.contains("\"Hello\""),
                    "first connection should receive hello"
                );
            }

            let (socket_2, _) = listener
                .accept()
                .await
                .expect("second accept should succeed");
            let (reader_2, mut writer_2) = socket_2.into_split();
            let mut second_lines = BufReader::new(reader_2).lines();
            let second_hello = second_lines
                .next_line()
                .await
                .expect("second hello read should succeed")
                .expect("second hello should be present");
            assert!(
                second_hello.contains("\"Hello\""),
                "second connection should receive hello"
            );

            writer_2
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write ready update");
            writer_2.flush().await.expect("server flush should succeed");
            tokio::time::sleep(Duration::from_millis(250)).await;
            writer_2
                .write_all(
                    br#"{"Set":{"ready":{"isReady":true,"username":"cli-user","manuallyInitiated":false}}}
"#,
                )
                .await
                .expect("server should write second ready update");
            writer_2
                .flush()
                .await
                .expect("server second flush should succeed");
        });

        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: addr.port(),
            username: "cli-user".to_owned(),
            room: "cli-room".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 3,
            max_connected_runtime_seconds: 0.2,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        run_client_network_loop(&config)
            .await
            .expect("network loop should reconnect and finish");
        server_task.await.expect("server task join should succeed");
    }

    #[test]
    fn normalize_controlled_room_input_extracts_canonical_room_and_password() {
        let (room, password) =
            normalize_controlled_room_input("+room:ABCDEF123456:ab-123-456".to_owned());
        assert_eq!(room, "+room:ABCDEF123456");
        assert_eq!(password.as_deref(), Some("AB-123-456"));

        let (room, password) = normalize_controlled_room_input("room1".to_owned());
        assert_eq!(room, "room1");
        assert!(password.is_none());
    }

    #[test]
    fn controlled_room_base_name_legacy_compatible_strips_managed_suffix() {
        assert_eq!(
            controlled_room_base_name_legacy_compatible("+base-room:ABCDEF123456"),
            "base-room"
        );
        assert_eq!(
            controlled_room_base_name_legacy_compatible("+room_name:ABCDEF12345_"),
            "room_name"
        );
        assert_eq!(
            controlled_room_base_name_legacy_compatible("room1"),
            "room1"
        );
        assert_eq!(
            controlled_room_base_name_legacy_compatible(" room1 "),
            " room1 "
        );
        assert_eq!(
            controlled_room_base_name_legacy_compatible("+room:SHORT"),
            "+room:SHORT"
        );
    }

    #[test]
    fn generate_room_password_legacy_compatible_matches_expected_shape() {
        let password = generate_room_password_legacy_compatible();
        assert!(
            is_legacy_generated_room_password_shape(&password),
            "generated password should match legacy shape AA-999-999"
        );
    }

    #[test]
    fn create_client_runtime_applies_autoplay_require_same_filenames_flag() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: true,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        assert!(
            runtime
                .session()
                .readiness_autoplay_config()
                .autoplay_require_same_filenames
        );
    }

    #[test]
    fn create_client_runtime_applies_duration_comparison_override_flags() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: Some(false),
            different_duration_threshold_seconds_override: Some(1.0),
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        let readiness_config = runtime.session().readiness_autoplay_config();
        assert!(!readiness_config.show_duration_notification);
        assert_eq!(readiness_config.different_duration_threshold_seconds, 1.0);
    }

    #[test]
    fn create_client_runtime_applies_show_same_room_osd_override_flag() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: Some(false),
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        assert!(!runtime.session().behavior_config().show_same_room_osd);
    }

    #[test]
    fn create_client_runtime_applies_show_noncontroller_osd_override_flag() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: Some(true),
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        assert!(runtime.session().behavior_config().show_noncontroller_osd);
    }

    #[test]
    fn create_client_runtime_applies_show_osd_warnings_override_flag() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: Some(false),
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        assert!(!runtime.session().behavior_config().show_osd_warnings);
    }

    #[test]
    fn create_client_runtime_applies_show_different_room_osd_override_flag() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: Some(true),
            controlled_room_password_override: None,
        };

        let runtime = create_client_runtime(&config);
        assert!(runtime.session().behavior_config().show_different_room_osd);
    }

    #[test]
    fn flush_autoplay_notifications_to_sink_dispatches_notifications() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: true,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready should apply");
        runtime
            .session_mut()
            .readiness_autoplay_config_mut()
            .auto_play_threshold = Some(2);

        runtime
            .run_disconnect(0.0)
            .expect("disconnect should pause local player");
        runtime.update_autoplay_check(true, true, false, false);
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("autoplay tick should emit countdown notification");

        let mut captured = Vec::new();
        flush_autoplay_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("notification sink dispatch should succeed");

        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].ready_user_count, 2);
        assert_eq!(captured[0].seconds_left, 3);
    }

    #[test]
    fn flush_reconnect_notifications_to_sink_dispatches_notifications() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .run_reconnect_retry(0)
            .expect("reconnect retry should queue reconnect notification");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .run_reconnect_transition_if_needed()
            .expect("reconnect completion should queue reconnect notification");

        let mut captured = Vec::new();
        flush_reconnect_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("reconnect notifications should dispatch");
        flush_reconnect_notifications_to_sink(&mut runtime, &mut ignore_reconnect_notification)
            .expect("drained reconnect notification queue should be empty");

        assert_eq!(
            captured,
            vec![
                ReconnectTransitionNotification::Attempting {
                    retries: 0,
                    delay_seconds: 0.1,
                },
                ReconnectTransitionNotification::Connected,
            ]
        );
    }

    #[test]
    fn flush_reconnect_notifications_to_sink_dispatches_disconnected_notification() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime.session_mut().reconnect_policy_mut().max_retries = 0;
        runtime
            .run_reconnect_retry(1)
            .expect("terminal reconnect retry should queue disconnected notification");

        let mut captured = Vec::new();
        flush_reconnect_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("reconnect notifications should dispatch");

        assert_eq!(
            captured,
            vec![ReconnectTransitionNotification::Disconnected]
        );
    }

    #[test]
    fn flush_reconnect_notifications_to_sink_dispatches_state_restore_notification() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file metadata should apply");
        runtime.session_mut().reset_sync_state_for_reconnect();
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");

        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("reconnect state restore should dispatch");

        let mut captured = Vec::new();
        flush_reconnect_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("reconnect notifications should dispatch");

        assert_eq!(
            captured,
            vec![ReconnectTransitionNotification::RestoringState]
        );
    }

    #[test]
    fn flush_reconnect_notifications_to_sink_dispatches_playlist_restore_notification() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");
        runtime.session_mut().reset_sync_state_for_reconnect();
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty reconnect playlist snapshot should apply");

        runtime
            .run_reconnect_playlist_restore_if_needed()
            .expect("reconnect playlist restore should dispatch");

        let mut captured = Vec::new();
        flush_reconnect_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("reconnect notifications should dispatch");

        assert_eq!(
            captured,
            vec![ReconnectTransitionNotification::RestoringPlaylist]
        );
    }

    #[test]
    fn flush_controller_auth_notifications_to_sink_dispatches_attempt_notification() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: Some("AB-123-456".to_owned()),
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .run_controller_reidentify_if_needed()
            .expect("controller reidentify should dispatch");

        let mut captured = Vec::new();
        flush_controller_auth_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("controller auth notifications should dispatch");
        flush_controller_auth_notifications_to_sink(
            &mut runtime,
            &mut ignore_controller_auth_notification,
        )
        .expect("drained controller auth notification queue should be empty");

        assert_eq!(
            captured,
            vec![ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }]
        );
    }

    #[test]
    fn flush_controller_auth_notifications_to_sink_dispatches_outcome_notifications() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"cli-user","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"cli-user","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("controller auth notifications should dispatch");

        let mut captured = Vec::new();
        flush_controller_auth_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("controller auth notifications should dispatch");

        assert_eq!(
            captured,
            vec![
                ControllerAuthTransitionNotification::Succeeded {
                    username: "cli-user".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                },
                ControllerAuthTransitionNotification::Failed {
                    username: "cli-user".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                },
            ]
        );
    }

    #[test]
    fn flush_chat_notifications_to_sink_dispatches_chat_messages() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
            .expect("chat should apply");
        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch");

        let mut captured = Vec::new();
        flush_chat_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("chat notifications should dispatch");
        flush_chat_notifications_to_sink(&mut runtime, &mut ignore_chat_notification)
            .expect("drained chat notification queue should be empty");

        assert_eq!(
            captured,
            vec![ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hello everyone".to_owned(),
            }]
        );
    }

    #[test]
    fn flush_user_change_notifications_to_sink_dispatches_visibility_metadata() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#,
            )
            .expect("user join should apply");
        runtime
            .run_user_change_notifications_if_needed()
            .expect("user change notifications should dispatch");

        let mut captured = Vec::new();
        flush_user_change_notifications_to_sink(&mut runtime, &mut |notification| {
            captured.push(notification.clone());
            Ok(())
        })
        .expect("user change notifications should dispatch");
        flush_user_change_notifications_to_sink(&mut runtime, &mut ignore_user_change_notification)
            .expect("drained user change notification queue should be empty");

        assert_eq!(
            captured,
            vec![UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            }]
        );
    }

    #[test]
    fn format_file_difference_summary_uses_legacy_difference_order() {
        assert_eq!(
            format_file_difference_summary(FileDifferenceSummary {
                filename: true,
                filesize: true,
                fileduration: true,
            }),
            Some("filename, filesize, duration".to_owned())
        );
        assert_eq!(
            format_file_difference_summary(FileDifferenceSummary {
                filename: false,
                filesize: false,
                fileduration: false,
            }),
            None
        );
    }

    #[test]
    fn flush_file_difference_notifications_to_sink_dedupes_and_honors_duration_overrides() {
        let config = ClientLoopConfig {
            host: "127.0.0.1".to_owned(),
            port: 8999,
            username: "cli-user".to_owned(),
            room: "room1".to_owned(),
            version: "1.2.255".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: 1.0,
            readiness_supported_override: None,
            local_can_control_override: None,
            is_playing_music_override: None,
            recently_advanced_override: None,
            autoplay_enabled: false,
            autoplay_require_same_filenames: false,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
            show_duration_notification_override: None,
            different_duration_threshold_seconds_override: None,
            show_same_room_osd_override: None,
            show_osd_warnings_override: None,
            show_noncontroller_osd_override: None,
            show_different_room_osd_override: None,
            controlled_room_password_override: None,
        };
        let mut runtime = create_client_runtime(&config);
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.0}}}}}"#,
            )
            .expect("local user file should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":100.0}}}}}"#,
            )
            .expect("peer duration mismatch should apply");

        let mut state = super::FileDifferenceNotificationState::default();
        let mut captured = Vec::new();
        flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
            captured.push(summary.to_owned());
            Ok(())
        })
        .expect("duration mismatch should emit one notification");
        flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
            captured.push(summary.to_owned());
            Ok(())
        })
        .expect("identical summary should not emit duplicate notification");
        assert_eq!(captured, vec!["duration"]);

        runtime
            .session_mut()
            .readiness_autoplay_config_mut()
            .show_duration_notification = false;
        flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
            captured.push(summary.to_owned());
            Ok(())
        })
        .expect("disabling duration notifications should clear difference summary");
        assert_eq!(captured, vec!["duration"]);

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":123456789,"duration":100.0}}}}}"#,
            )
            .expect("peer filename mismatch should apply");
        flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
            captured.push(summary.to_owned());
            Ok(())
        })
        .expect("new filename mismatch should emit notification");
        assert_eq!(captured, vec!["duration", "filename"]);
    }
}
