use anyhow::anyhow;
use sorotte_client_app::app_boundary::application::{ClientApplication, ClientCommand};
#[cfg(test)]
use sorotte_client_app::app_boundary::commands::{
    LocalInputCommand, LocalOffsetCommand, PlannedLocalRuntimeAction, controlled_room_base_name,
    generate_room_password, parse_local_input_chat_message, parse_local_input_command,
    playlist_index_in_bounds,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::diagnostics::{
    ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
    ReconnectCorrectionDiagnosticsState,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::diagnostics::{
    reconnect_correction_metrics_delta_alert_lines,
    reconnect_correction_metrics_delta_alert_lines_localized,
    reconnect_correction_metrics_delta_json_line, reconnect_correction_metrics_delta_message,
    reconnect_correction_metrics_delta_message_localized,
    reconnect_correction_state_snapshot_json_line, reconnect_correction_state_snapshot_message,
    reconnect_correction_state_snapshot_message_localized,
    reconnect_correction_state_threshold_alert_lines,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::language::normalized_runtime_language_tag;
#[cfg(test)]
use sorotte_client_app::app_boundary::language::runtime_language_selection_line;
#[cfg(test)]
use sorotte_client_app::app_boundary::notifications::FileDifferenceNotificationState;
#[cfg(test)]
use sorotte_client_app::app_boundary::session::ConnectedSessionOuterLoopExitKind as ConnectedSessionExit;
#[cfg(test)]
use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettings, parse_autoplay_min_users_override,
    parse_unpause_action_mode,
};

mod client_args;
mod client_config;
mod config_paths;
mod diagnostics_config;
mod env_support;
mod language_support;
mod local_runtime_actions;
mod mpv_startup;
mod notifications;
mod protocol_io;
mod session_runner;
mod startup_playlist;
mod stdin_input;
mod stored_settings;
mod update_check;

#[cfg(feature = "fuzz-support")]
#[doc(hidden)]
pub mod fuzz_support {
    pub use crate::protocol_io::{InboundProtocolLineReader, MAX_INBOUND_PROTOCOL_LINE_BYTES};
}

#[cfg(test)]
use self::client_args::{
    HostArgumentError, SyncplayClientArgumentIssue, localized_compatibility_input_label,
    localized_compatibility_note_label, localized_startup_compatibility_heading,
    localized_syncplay_ini_compatibility_heading, parse_host_and_optional_port_from_host_arg,
    syncplay_force_gui_prompt_compatibility_line,
};
use self::client_args::{
    SyncplayClientArgOverrides, apply_syncplay_client_arg_overrides,
    emit_syncplay_client_arg_compatibility_warnings, parse_syncplay_client_arg_overrides,
    print_syncplay_client_help, should_halt_for_stored_force_gui_prompt,
    stored_force_gui_prompt_compatibility_line, syncplay_unrecognized_arguments_diagnostic_line,
    validate_composed_client_endpoint,
};
use self::client_config::build_client_loop_config_from_env;
#[cfg(test)]
use self::client_config::{
    ChatPolicyOverrides, ClientBehaviorOverrides, ClientLoopConfig, ReadinessAutoplayOverrides,
    apply_chat_policy_overrides, apply_client_behavior_overrides,
    apply_readiness_autoplay_overrides, client_hello_features, create_client_runtime,
    create_client_session, normalize_controlled_room_input,
    parse_reconnect_state_restore_correction_policy_mode,
};
use self::config_paths::set_sorotte_cli_config_cli_overrides;
#[cfg(test)]
use self::diagnostics_config::{
    ClientLoopDiagnosticsConfig, apply_syncplay_client_arg_diagnostics_overrides,
    reconnect_correction_diagnostics_alert_thresholds_from_env,
    reconnect_correction_diagnostics_format_from_env,
};
use self::env_support::{env_flag_enabled, env_trimmed};
#[cfg(test)]
use self::env_support::{
    parse_env_bool, parse_env_non_negative_f64, parse_env_port, parse_env_string_list,
};
use self::language_support::{resolved_runtime_language_tag, set_runtime_language_for_process};
#[cfg(test)]
use self::local_runtime_actions::{
    publish_pending_local_file_updates, run_planned_local_runtime_action,
};
#[cfg(test)]
use self::mpv_startup::spawn_external_player_from_spec;
use self::mpv_startup::spawn_external_player_if_requested;
#[cfg(test)]
use self::mpv_startup::{
    ExplicitMpvIpcStartupPlayerArgDiagnostics, ExplicitMpvIpcStartupPlayerArgs,
    ExplicitMpvIpcStartupPlayerCommand, ExternalPlayerLaunchSpec, ManagedMpvLaunchEnvConfig,
    analyze_explicit_mpv_ipc_startup_player_args, apply_syncplay_client_arg_managed_mpv_overrides,
    explicit_mpv_ipc_startup_player_arg_diagnostic_lines,
    external_player_launch_spec_from_overrides, find_default_managed_mpv_bin,
    managed_mpv_launch_base_args, managed_mpv_launch_env_config_from_env,
    non_mpv_player_path_ignored_by_mpv_integration_warning_line,
    parse_explicit_mpv_ipc_startup_player_args, player_path_requests_managed_mpv,
    resolve_managed_mpv_launch_program,
    should_skip_external_player_launch_due_to_mpv_integration_env,
};
#[cfg(test)]
use self::mpv_startup::{
    apply_startup_file_to_attached_player_if_explicit_mpv_ipc,
    create_client_runtime_with_managed_mpv_support,
    create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test,
    create_client_runtime_with_prepared_mpv_and_startup_health_for_test,
    create_client_runtime_with_prepared_mpv_for_test, player_path_compatibility_warning_line,
};
#[cfg(all(test, windows))]
use self::mpv_startup::{
    connect_mpv_adapter_with_retry, retry_explicit_mpv_ipc_startup_player_command,
};
#[cfg(test)]
use self::notifications::{
    autoplay_countdown_notification_message_localized, chat_notification_message,
    controller_auth_notification_hidden_from_osd, controller_auth_transition_notification_message,
    controller_auth_transition_notification_message_localized,
    flush_autoplay_notifications_to_sink, flush_chat_notifications_to_sink,
    flush_controller_auth_notifications_to_sink, flush_file_difference_notifications_to_sink,
    flush_reconnect_correction_diagnostics_to_sink, flush_reconnect_notifications_to_sink,
    flush_user_change_notifications_to_sink, format_duration, format_file_difference_summary,
    localized_file_difference_summary, player_playback_drift_diagnostic_messages_localized,
    player_playback_telemetry_update_message, player_playback_telemetry_update_message_localized,
    reconnect_transition_notification_message, reconnect_transition_notification_message_localized,
    seek_preparation_diagnostic_messages, user_change_notification_hidden_from_osd,
    user_change_notification_message, user_change_notification_message_localized,
};
use self::session_runner::run_client_network_loop_with_startup_overrides_and_stored_settings;
#[cfg(test)]
use self::session_runner::{
    cli_plex_config_from_env_and_stored_settings, client_runtime_now_seconds,
    run_client_network_loop, run_client_network_loop_with_prepared_runtime_for_test,
    run_connected_client_session, run_connected_client_session_with_plex_config_for_test,
    run_connected_client_session_with_startup_overrides,
};
#[cfg(test)]
use self::startup_playlist::protocol_lines_for_startup_playlist_load_from_file;
use self::stored_settings::{
    apply_stored_client_settings, apply_stored_media_search_startup_file_fallback_if_missing,
    apply_stored_startup_player_defaults_if_arg_absent, clear_sorotte_cli_gui_state,
    clear_sorotte_cli_stored_settings, load_sorotte_cli_stored_settings,
    persist_sorotte_cli_language_setting, persist_sorotte_cli_per_player_arguments_setting,
    persist_sorotte_cli_player_path_setting, persist_sorotte_cli_stored_settings,
};
#[cfg(test)]
use self::stored_settings::{
    apply_syncplay_ui_settings_to_mpv_adapter, parse_sorotte_ini_stored_client_settings,
    resolve_startup_file_with_media_search_fallback, syncplay_ui_settings_from_stored_settings,
    upsert_sorotte_ini_stored_client_settings,
};
use self::update_check::apply_headless_automatic_update_check;
#[cfg(test)]
use self::update_check::{
    DEFAULT_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS, parse_utc_timestamp,
    persist_sorotte_cli_last_checked_for_updates_setting,
    should_run_headless_automatic_update_check, utc_timestamp_string,
};

fn persist_explicit_syncplay_client_arg_settings(overrides: &SyncplayClientArgOverrides) {
    if let Some(language) = overrides.language.as_deref()
        && !overrides.no_store
        && let Err(error) = persist_sorotte_cli_language_setting(language)
    {
        eprintln!("warning: failed to persist --language setting: {error}");
    }
    if let Some(player_path) = overrides.player_path.as_deref()
        && !overrides.no_store
        && let Err(error) = persist_sorotte_cli_player_path_setting(player_path)
    {
        eprintln!("warning: failed to persist --player-path setting: {error}");
    }
    if let Some(player_path) = overrides.player_path.as_deref()
        && !overrides.no_store
        && !overrides.player_args.is_empty()
        && let Err(error) =
            persist_sorotte_cli_per_player_arguments_setting(player_path, &overrides.player_args)
    {
        eprintln!("warning: failed to persist per-player arguments setting: {error}");
    }
}

pub async fn run_sorotte_cli_from_env() -> anyhow::Result<()> {
    let mut client_arg_overrides = parse_syncplay_client_arg_overrides(std::env::args().skip(1));
    if client_arg_overrides.show_version {
        println!("sorotte-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if client_arg_overrides.show_help {
        print_syncplay_client_help(client_arg_overrides.language.as_deref());
        return Ok(());
    }
    if !client_arg_overrides.unknown_options.is_empty() {
        eprintln!(
            "{}",
            syncplay_unrecognized_arguments_diagnostic_line(&client_arg_overrides.unknown_options)
        );
        return Err(anyhow!("unrecognized arguments"));
    }
    set_sorotte_cli_config_cli_overrides(
        client_arg_overrides
            .config_path
            .as_ref()
            .map(std::path::PathBuf::from),
        client_arg_overrides
            .config_root
            .as_ref()
            .map(std::path::PathBuf::from),
    );
    if client_arg_overrides.clear_gui_data_requested {
        match clear_sorotte_cli_stored_settings() {
            Ok(true) => {
                eprintln!("cleared stored Sorotte settings (sorotte.ini) for --clear-gui-data");
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!("warning: failed to clear stored Sorotte settings: {error}");
            }
        }
        match clear_sorotte_cli_gui_state() {
            Ok(true) => {
                eprintln!("cleared Sorotte GUI state for --clear-gui-data");
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!("warning: failed to clear Sorotte GUI state: {error}");
            }
        }
    }
    let should_connect =
        env_flag_enabled("SOROTTE_CLIENT_CONNECT") || client_arg_overrides.should_connect_client();
    if !should_connect {
        persist_explicit_syncplay_client_arg_settings(&client_arg_overrides);
    }
    emit_syncplay_client_arg_compatibility_warnings(&client_arg_overrides);
    if client_arg_overrides.should_halt_for_syncplay_force_gui_prompt_compatibility() {
        return Ok(());
    }
    let stored_settings = load_sorotte_cli_stored_settings()
        .map_err(|error| anyhow!("failed to load stored Sorotte settings: {error:#}"))?;
    if let Some(stored_settings) = stored_settings.as_ref() {
        if let Some(line) =
            stored_force_gui_prompt_compatibility_line(&client_arg_overrides, stored_settings)
        {
            eprintln!("{line}");
        }
        if should_halt_for_stored_force_gui_prompt(&client_arg_overrides, stored_settings) {
            return Ok(());
        }
    }
    set_runtime_language_for_process(resolved_runtime_language_tag(
        &client_arg_overrides,
        stored_settings.as_ref(),
    ));
    if should_connect {
        let mut config = build_client_loop_config_from_env();
        if let Some(stored_settings) = stored_settings.as_ref() {
            apply_stored_client_settings(
                &mut config,
                stored_settings,
                crate::env_support::env_trimmed,
            );
            apply_stored_startup_player_defaults_if_arg_absent(
                &mut client_arg_overrides,
                stored_settings,
            );
        }
        apply_stored_media_search_startup_file_fallback_if_missing(
            &mut client_arg_overrides,
            stored_settings.as_ref(),
        );
        apply_syncplay_client_arg_overrides(&mut config, &client_arg_overrides);
        validate_composed_client_endpoint(&config)
            .map_err(|error| anyhow!("invalid client endpoint: {error}"))?;
        persist_explicit_syncplay_client_arg_settings(&client_arg_overrides);
        apply_headless_automatic_update_check(&client_arg_overrides, stored_settings.as_ref());
        if !client_arg_overrides.no_store
            && let Err(error) = persist_sorotte_cli_stored_settings(&config)
        {
            eprintln!("warning: failed to persist stored Sorotte settings: {error}");
        }
        if let Err(error) = spawn_external_player_if_requested(&client_arg_overrides) {
            eprintln!("warning: failed to launch external player startup path: {error}");
        }
        run_client_network_loop_with_startup_overrides_and_stored_settings(
            &config,
            client_arg_overrides.load_playlist_from_file.as_deref(),
            Some(&client_arg_overrides),
            stored_settings.as_ref(),
        )
        .await?;
        return Ok(());
    }

    apply_headless_automatic_update_check(&client_arg_overrides, stored_settings.as_ref());

    let mut client =
        ClientApplication::with_default_session(sorotte_player_mpv::MpvAdapter::default());
    let events = client.dispatch(ClientCommand::ReceiveProtocolLine {
        line: r#"{"Hello":{"username":"cli-user","room":{"name":"cli-demo"},"version":"1.2.255"}}"#
            .to_owned(),
        received_at_seconds: 0.0,
    });
    if let Some(sorotte_client_app::app_boundary::application::ClientEvent::OperationFailed {
        message,
        ..
    }) = events.into_iter().find(|event| {
        matches!(
            event,
            sorotte_client_app::app_boundary::application::ClientEvent::OperationFailed { .. }
        )
    }) {
        return Err(anyhow!(message));
    }

    println!(
        "sorotte-cli bootstrap complete for user {} in room {}",
        client.session().username().unwrap_or("unknown"),
        client.session().room().unwrap_or("unknown")
    );
    Ok(())
}

#[cfg(test)]
mod tests;
