#[cfg(windows)]
use super::spawn_legacy_external_player_from_spec_legacy_compatible;
use super::{
    AutoplayThresholdOverride, ChatPolicyOverrides, ClientBehaviorOverrides, ClientLoopConfig,
    ConnectedSessionExit, LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER, LegacyClientArgOverrides,
    LegacyExternalPlayerLaunchSpec, LocalInputCommand, LocalOffsetCommand,
    ManagedMpvLaunchEnvConfig, PlannedLocalRuntimeAction, ReadinessAutoplayOverrides,
    ReconnectCorrectionDiagnosticsFormat, ReconnectCorrectionDiagnosticsState,
    StoredClientSettingsMvp, apply_chat_policy_overrides, apply_client_behavior_overrides,
    apply_legacy_client_arg_managed_mpv_overrides, apply_legacy_client_arg_overrides,
    apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible,
    apply_readiness_autoplay_overrides, apply_stored_client_settings_mvp_if_env_absent,
    apply_stored_legacy_startup_player_defaults_if_arg_absent,
    apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible,
    chat_notification_message, clear_syncplay_cli_gui_qsettings_legacy_compatible,
    clear_syncplay_cli_stored_settings_legacy_compatible,
    controlled_room_base_name_legacy_compatible, controller_auth_notification_hidden_from_osd,
    controller_auth_transition_notification_message, create_client_runtime,
    create_client_runtime_with_managed_mpv_support, create_client_session,
    flush_autoplay_notifications_to_sink, flush_chat_notifications_to_sink,
    flush_controller_auth_notifications_to_sink, flush_file_difference_notifications_to_sink,
    flush_reconnect_correction_diagnostics_to_sink, flush_reconnect_notifications_to_sink,
    flush_user_change_notifications_to_sink, format_duration_legacy,
    format_file_difference_summary, generate_room_password_legacy_compatible,
    legacy_external_player_launch_spec_from_overrides_legacy_compatible,
    legacy_syncplay_ui_settings_from_stored_settings,
    legacy_syncplayintf_script_source_with_chat_input_bridge_legacy_compatible,
    legacy_utc_timestamp_string_legacy_compatible,
    load_syncplay_cli_stored_settings_mvp_legacy_compatible,
    managed_mpv_launch_base_args_legacy_compatible, managed_mpv_launch_env_config_from_env,
    normalize_controlled_room_input, parse_autoplay_min_users_override_legacy_compatible,
    parse_env_bool_legacy_compatible, parse_env_non_negative_f64_legacy_compatible,
    parse_env_port_legacy_compatible, parse_env_string_list_legacy_compatible,
    parse_host_and_optional_port_from_host_arg_legacy_compatible,
    parse_legacy_client_arg_overrides, parse_legacy_utc_timestamp_legacy_compatible,
    parse_local_input_chat_message, parse_local_input_command,
    parse_reconnect_state_restore_correction_policy_mode_legacy_compatible,
    parse_syncplay_ini_stored_client_settings_mvp, parse_unpause_action_mode_legacy_compatible,
    persist_syncplay_cli_language_setting_legacy_compatible,
    persist_syncplay_cli_last_checked_for_updates_setting_legacy_compatible,
    persist_syncplay_cli_per_player_arguments_setting_legacy_compatible,
    persist_syncplay_cli_player_path_setting_legacy_compatible,
    persist_syncplay_cli_stored_settings_mvp_legacy_compatible,
    player_playback_telemetry_update_message, playlist_index_in_bounds_legacy_compatible,
    protocol_lines_for_startup_playlist_load_from_file_legacy_compatible,
    reconnect_correction_diagnostics_alert_thresholds_from_env,
    reconnect_correction_diagnostics_format_from_env,
    reconnect_correction_metrics_delta_alert_lines, reconnect_correction_metrics_delta_json_line,
    reconnect_correction_metrics_delta_message, reconnect_correction_state_snapshot_json_line,
    reconnect_correction_state_snapshot_message, reconnect_correction_state_threshold_alert_lines,
    reconnect_transition_notification_message,
    resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible,
    run_client_network_loop, run_connected_client_session,
    run_connected_client_session_with_legacy_startup_overrides,
    run_planned_local_runtime_action_legacy_compatible,
    should_run_headless_automatic_update_check_legacy_compatible,
    should_skip_legacy_external_player_launch_due_to_mpv_integration_env,
    upsert_syncplay_ini_stored_client_settings_mvp, user_change_notification_hidden_from_osd,
    user_change_notification_message,
};
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use syncplay_client_app::app_boundary::compatibility::{
    LegacyConfigurationGetterCompatibilityStatus, LegacyConfigurationGetterIniCompatEntry,
    LegacyConfigurationGetterStartupCompatEntry, legacy_configuration_getter_ini_compat_entries,
    legacy_configuration_getter_startup_compat_entries,
};
use syncplay_client_app::app_boundary::persistence::{
    format_serialized_per_player_arguments_map_legacy_compatible,
    format_serialized_public_servers_list_legacy_compatible,
    parse_serialized_per_player_arguments_map_legacy_compatible,
    parse_serialized_public_servers_list_legacy_compatible,
};
use syncplay_client_core::{
    AutoplayCountdownNotification, ChatNotification, ClientRuntime, ClientSession,
    ControllerAuthTransitionNotification, FileDifferenceSummary, PrivacyMode, QueuedRuntimeControl,
    ReadinessAutoplayConfig, ReconnectStateRestoreCorrectionMetrics,
    ReconnectStateRestoreCorrectionPolicyMode, ReconnectStateRestoreCorrectionStateSnapshot,
    ReconnectTransitionNotification, UnpauseActionMode, UserChangeNotification,
};
use syncplay_player_api::{PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate};
use syncplay_player_mpv::{LegacySyncplayUiSettings, MpvAdapter};
#[cfg(windows)]
use syncplay_protocol::HelloPayload;
use syncplay_protocol::{
    IgnoringOnTheFlyPayload, ListPayload, PingPayload, PlaystatePayload, ProtocolMessage,
    StatePayload, decode_message_line, encode_message_line,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::unbounded_channel;

static STORED_SETTINGS_CONFIG_PATH_ENV_LOCK: Mutex<()> = Mutex::new(());
static LEGACY_GUI_QSETTINGS_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());
static LEGACY_EXTERNAL_PLAYER_ENV_LOCK: Mutex<()> = Mutex::new(());
static RECONNECT_DIAGNOSTICS_ENV_LOCK: Mutex<()> = Mutex::new(());

struct TestEnvGuard<'a> {
    _guard: MutexGuard<'a, ()>,
}

impl<'a> TestEnvGuard<'a> {
    fn lock(lock: &'a Mutex<()>) -> Self {
        Self {
            _guard: lock.lock().expect("lock poisoned"),
        }
    }

    fn set_var<K, V>(&self, key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        // SAFETY: Environment mutation is process-global in Rust 2024. CLI tests use
        // TestEnvGuard to hold the relevant domain mutex while mutating and restoring
        // env state, so test-owned env changes do not race each other.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_var<K>(&self, key: K)
    where
        K: AsRef<OsStr>,
    {
        // SAFETY: See set_var; the same guard serializes test-owned removals.
        unsafe {
            std::env::remove_var(key);
        }
    }
}

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

fn ignore_user_change_notification(_notification: &UserChangeNotification) -> anyhow::Result<()> {
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

fn test_client_loop_config() -> ClientLoopConfig {
    ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "cli-room".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 3,
        max_connected_runtime_seconds: 10.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    }
}

fn test_client_loop_config_with_addr(addr: std::net::SocketAddr) -> ClientLoopConfig {
    ClientLoopConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        ..test_client_loop_config()
    }
}

type TestServerLines = tokio::io::Lines<BufReader<OwnedReadHalf>>;

async fn expect_client_hello_and_send_standard_test_server_hello(
    lines: &mut TestServerLines,
    writer: &mut OwnedWriteHalf,
) {
    let first_line = lines
        .next_line()
        .await
        .expect("first client line read should succeed")
        .expect("first client line should be present");
    let hello_line =
        match decode_message_line(&first_line).expect("first client line should decode") {
            ProtocolMessage::Tls(tls_message) if tls_message.tls.start_tls == "send" => {
                writer
                    .write_all(b"{\"TLS\":{\"startTLS\":\"false\"}}\n")
                    .await
                    .expect("server TLS fallback write should succeed");
                writer
                    .flush()
                    .await
                    .expect("server TLS fallback flush should succeed");
                lines
                    .next_line()
                    .await
                    .expect("hello line read should succeed")
                    .expect("hello line should be present")
            }
            _ => first_line,
        };
    assert!(
        hello_line.contains("\"Hello\""),
        "client should send a Hello message after TLS negotiation"
    );
    writer
            .write_all(
                b"{\"Hello\":{\"username\":\"cli-user\",\"room\":{\"name\":\"cli-room\"},\"version\":\"1.2.255\",\"features\":{\"chat\":true,\"readiness\":false}}}\n",
            )
            .await
            .expect("server hello write should succeed");
    writer
        .flush()
        .await
        .expect("server hello flush should succeed");
}

fn seed_stub_player_pause_position_telemetry(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    paused: bool,
    position_seconds: f64,
) {
    runtime
        .player_mut()
        .set_paused(paused)
        .expect("stub player pause seed should succeed");
    runtime
        .player_mut()
        .set_position(position_seconds)
        .expect("stub player position seed should succeed");
    runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(paused)
                .with_position_seconds(position_seconds),
        );
}

fn seed_stub_player_playback_rate(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    rate: f64,
) {
    runtime
        .player_mut()
        .set_playback_rate(rate)
        .expect("stub player playback-rate seed should succeed");
}

async fn run_connected_client_session_expect_normal_exit(
    addr: std::net::SocketAddr,
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
) {
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;

    let exit = run_connected_client_session(
        stream,
        runtime,
        config,
        None,
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
}

fn cli_smoke_repo_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("current dir should be readable");
    for ancestor in cwd.ancestors().take(8) {
        if ancestor.join("mpv").exists() && ancestor.join("media").exists() {
            return ancestor.to_path_buf();
        }
    }
    panic!("expected repo root with ./mpv and ./media directories");
}

fn first_media_file(media_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(media_dir).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        let Some(ext) = ext else { continue };
        if matches!(ext.as_str(), "mkv" | "mp4" | "avi" | "webm" | "mov" | "m4v") {
            return Some(path);
        }
    }
    None
}

mod cli_runtime_overrides;
mod client_args_compat;
mod client_runtime;
mod connected_session_basics;
mod connected_session_desync;
mod connected_session_local_commands;
mod connected_session_reconnect_restore;
mod env_client_config;
mod local_input_commands;
mod mpv_smoke;
mod mpv_startup;
mod notification_messages;
mod output_notifications;
mod reconnect_diagnostics;
mod runtime_notifications;
mod startup_playlist;
mod stored_settings;
mod user_change_notifications;
