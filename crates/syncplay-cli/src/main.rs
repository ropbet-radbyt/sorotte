use std::env;
use std::time::Duration;

use anyhow::anyhow;
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, AutoplayCountdownNotification, ChatNotification, ClientRuntime,
    ClientSession, ControllerAuthTransitionNotification, FileDifferenceSummary, PrivacyMode,
    QueuedRuntimeControl, ReconnectTransitionNotification, UserChangeNotification,
};
use syncplay_player_mpv::MpvAdapter;
use syncplay_protocol::{ProtocolMessage, encode_message_line};
use syncplay_server::ServerApp;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::Instant;

const ROUND_HALF_EPSILON: f64 = 1e-12;

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
    env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
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
    env_trimmed(name).map(|_| env_flag_enabled(name))
}

fn env_u16(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
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

fn env_f64(name: &str) -> Option<f64> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
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

fn parse_local_input_chat_message(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(message) = trimmed
        .strip_prefix("chat ")
        .or_else(|| trimmed.strip_prefix("ch "))
        .or_else(|| trimmed.strip_prefix("/chat "))
        .or_else(|| trimmed.strip_prefix("/msg "))
    {
        let message = message.trim();
        return (!message.is_empty()).then(|| message.to_owned());
    }

    if trimmed == "chat"
        || trimmed == "ch"
        || trimmed == "/chat"
        || trimmed == "/msg"
        || trimmed.starts_with('/')
    {
        return None;
    }

    Some(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq)]
enum LocalInputCommand {
    Chat(String),
    RequestUserList,
    UndoSeek,
    SeekAbsolute(f64),
    SeekRelative(f64),
    TogglePause,
    ToggleReady,
    SetRoomWithLegacyFallback,
    SetRoom(String),
}

fn parse_seek_time_seconds_legacy_like(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if !value.contains(':') {
        let seconds = value.parse::<f64>().ok()?;
        return seconds.is_finite().then_some(seconds);
    }

    let parts: Vec<&str> = value.split(':').collect();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => {
            minutes.parse::<u64>().ok()? as f64 * 60.0 + seconds.parse::<f64>().ok()?
        }
        [hours, minutes, seconds] => {
            hours.parse::<u64>().ok()? as f64 * 3600.0
                + minutes.parse::<u64>().ok()? as f64 * 60.0
                + seconds.parse::<f64>().ok()?
        }
        _ => return None,
    };
    seconds.is_finite().then_some(seconds)
}

fn parse_seek_parameter(parameter: &str) -> Option<LocalInputCommand> {
    let parameter = parameter.trim();
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

fn parse_local_input_command(input: &str) -> Option<LocalInputCommand> {
    let trimmed = input.trim();
    if matches!(
        trimmed,
        "undo" | "u" | "revert" | "/undo" | "/u" | "/revert"
    ) {
        return Some(LocalInputCommand::UndoSeek);
    }
    if matches!(trimmed, "list" | "l" | "users" | "/list" | "/l" | "/users") {
        return Some(LocalInputCommand::RequestUserList);
    }
    if let Some(parameter) = trimmed
        .strip_prefix("seek ")
        .or_else(|| trimmed.strip_prefix("s "))
        .or_else(|| trimmed.strip_prefix("/seek "))
        .or_else(|| trimmed.strip_prefix("/s "))
    {
        return parse_seek_parameter(parameter);
    }
    if matches!(trimmed, "seek" | "s" | "/seek" | "/s") {
        return None;
    }
    if matches!(trimmed, "p" | "pause" | "play" | "/p" | "/pause" | "/play") {
        return Some(LocalInputCommand::TogglePause);
    }
    if let Some(room) = trimmed
        .strip_prefix("room ")
        .or_else(|| trimmed.strip_prefix("r "))
        .or_else(|| trimmed.strip_prefix("/room "))
        .or_else(|| trimmed.strip_prefix("/r "))
    {
        let room = room.trim();
        return (!room.is_empty()).then(|| LocalInputCommand::SetRoom(room.to_owned()));
    }
    if matches!(trimmed, "room" | "r" | "/room" | "/r") {
        return Some(LocalInputCommand::SetRoomWithLegacyFallback);
    }
    if matches!(trimmed, "t" | "toggle" | "/t" | "/toggle") {
        return Some(LocalInputCommand::ToggleReady);
    }
    parse_local_input_chat_message(input).map(LocalInputCommand::Chat)
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
        port: env_u16("SYNCPLAY_CLIENT_PORT").unwrap_or(8999),
        username: env_trimmed("SYNCPLAY_CLIENT_USERNAME")
            .or_else(|| env_trimmed("SYNCPLAY_CLIENT_NAME"))
            .unwrap_or_else(|| "cli-user".to_owned()),
        room,
        version: env_trimmed("SYNCPLAY_CLIENT_VERSION").unwrap_or_else(|| "1.2.255".to_owned()),
        max_retries: env_u32("SYNCPLAY_CLIENT_MAX_RETRIES").unwrap_or(3),
        max_connected_runtime_seconds: env_f64("SYNCPLAY_CLIENT_MAX_CONNECTED_RUNTIME_SECONDS")
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
        different_duration_threshold_seconds_override: env_f64(
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
                    let emitted = match command {
                        LocalInputCommand::Chat(chat_message) => {
                            runtime.run_send_chat_message(chat_message)?
                        }
                        LocalInputCommand::RequestUserList => runtime.run_request_user_list()?,
                        LocalInputCommand::UndoSeek => runtime.run_undo_seek()?,
                        LocalInputCommand::SeekAbsolute(position_seconds) => {
                            runtime.run_seek_to_position(position_seconds)?
                        }
                        LocalInputCommand::SeekRelative(offset_seconds) => {
                            runtime.run_seek_by_offset(offset_seconds)?
                        }
                        LocalInputCommand::TogglePause => runtime.run_toggle_pause()?,
                        LocalInputCommand::ToggleReady => runtime.run_toggle_ready(true)?,
                        LocalInputCommand::SetRoomWithLegacyFallback => runtime
                            .run_set_room_with_legacy_fallback(config.room.clone())?,
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
    if let Some(max_chat_message_length) = env_usize("SYNCPLAY_CLIENT_CHAT_MAX_LENGTH") {
        let chat_config = runtime.session_mut().chat_config_mut();
        chat_config.max_chat_message_length = max_chat_message_length;
        chat_config.apply_server_max_chat_message_length = false;
    }
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
    let server_port = env_u16("SYNCPLAY_SERVER_PORT");
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
        ClientLoopConfig, ConnectedSessionExit, LocalInputCommand, chat_notification_message,
        controller_auth_notification_hidden_from_osd,
        controller_auth_transition_notification_message, create_client_runtime,
        flush_autoplay_notifications_to_sink, flush_chat_notifications_to_sink,
        flush_controller_auth_notifications_to_sink, flush_file_difference_notifications_to_sink,
        flush_reconnect_notifications_to_sink, flush_user_change_notifications_to_sink,
        format_duration_legacy, format_file_difference_summary, normalize_controlled_room_input,
        parse_local_input_chat_message, parse_local_input_command,
        reconnect_transition_notification_message, run_client_network_loop,
        run_connected_client_session, user_change_notification_hidden_from_osd,
        user_change_notification_message,
    };
    use std::time::Duration;
    use syncplay_client_core::{
        AutoplayCountdownNotification, ChatNotification, ControllerAuthTransitionNotification,
        FileDifferenceSummary, PrivacyMode, ReconnectTransitionNotification,
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
            parse_local_input_chat_message("/msg hello everyone"),
            Some("hello everyone".to_owned())
        );
    }

    #[test]
    fn parse_local_input_chat_message_ignores_empty_and_unknown_commands() {
        assert_eq!(parse_local_input_chat_message(""), None);
        assert_eq!(parse_local_input_chat_message("   "), None);
        assert_eq!(parse_local_input_chat_message("chat"), None);
        assert_eq!(parse_local_input_chat_message("ch"), None);
        assert_eq!(parse_local_input_chat_message("/chat"), None);
        assert_eq!(parse_local_input_chat_message("/msg"), None);
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
        assert_eq!(parse_local_input_command("seek"), None);
        assert_eq!(parse_local_input_command("s"), None);
    }

    #[test]
    fn parse_local_input_command_parses_room_aliases() {
        assert_eq!(
            parse_local_input_command("room room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("r room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/room room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("/r room2"),
            Some(LocalInputCommand::SetRoom("room2".to_owned()))
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
    fn parse_local_input_command_parses_chat_and_ignores_unknown_slash_commands() {
        assert_eq!(
            parse_local_input_command("hello everyone"),
            Some(LocalInputCommand::Chat("hello everyone".to_owned()))
        );
        assert_eq!(
            parse_local_input_command("chat hello everyone"),
            Some(LocalInputCommand::Chat("hello everyone".to_owned()))
        );
        assert_eq!(parse_local_input_command("/unknown hello"), None);
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
