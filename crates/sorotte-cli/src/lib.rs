use anyhow::anyhow;
use sorotte_client_app::app_boundary::application::{ClientApplication, ClientCommand};
#[cfg(test)]
use sorotte_client_app::app_boundary::commands::{
    LocalInputCommand, LocalOffsetCommand, PlannedLocalRuntimeAction,
    controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
    parse_local_input_chat_message, parse_local_input_command,
    playlist_index_in_bounds_legacy_compatible,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::diagnostics::{
    ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
    ReconnectCorrectionDiagnosticsState,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::diagnostics::{
    reconnect_correction_metrics_delta_alert_lines,
    reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible,
    reconnect_correction_metrics_delta_json_line, reconnect_correction_metrics_delta_message,
    reconnect_correction_metrics_delta_message_localized_legacy_compatible,
    reconnect_correction_state_snapshot_json_line, reconnect_correction_state_snapshot_message,
    reconnect_correction_state_snapshot_message_localized_legacy_compatible,
    reconnect_correction_state_threshold_alert_lines,
};
#[cfg(test)]
use sorotte_client_app::app_boundary::language::legacy_runtime_language_selection_line_legacy_compatible;
#[cfg(test)]
use sorotte_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;
#[cfg(test)]
use sorotte_client_app::app_boundary::notifications::FileDifferenceNotificationState;
#[cfg(test)]
use sorotte_client_app::app_boundary::session::ConnectedSessionOuterLoopExitKind as ConnectedSessionExit;
#[cfg(test)]
use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
    parse_autoplay_min_users_override_legacy_compatible,
    parse_unpause_action_mode_legacy_compatible,
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
    HostArgumentError, LegacyClientArgumentIssue,
    legacy_force_gui_prompt_compatibility_line_legacy_compatible,
    localized_compatibility_input_label_legacy_compatible,
    localized_compatibility_note_label_legacy_compatible,
    localized_legacy_ini_compatibility_heading_legacy_compatible,
    localized_legacy_startup_compatibility_heading_legacy_compatible,
    parse_host_and_optional_port_from_host_arg_legacy_compatible,
};
use self::client_args::{
    LegacyClientArgOverrides, apply_legacy_client_arg_overrides,
    emit_legacy_client_arg_compatibility_warnings, legacy_unrecognized_arguments_diagnostic_line,
    parse_legacy_client_arg_overrides, print_legacy_client_help,
    should_halt_for_stored_force_gui_prompt_legacy_compatible,
    stored_force_gui_prompt_compatibility_line_legacy_compatible,
    validate_composed_client_endpoint,
};
use self::client_config::build_client_loop_config_from_env;
#[cfg(test)]
use self::client_config::{
    ChatPolicyOverrides, ClientBehaviorOverrides, ClientLoopConfig, ReadinessAutoplayOverrides,
    apply_chat_policy_overrides, apply_client_behavior_overrides,
    apply_readiness_autoplay_overrides, client_hello_features_legacy_compatible,
    create_client_runtime, create_client_session, normalize_controlled_room_input,
    parse_reconnect_state_restore_correction_policy_mode_legacy_compatible,
};
use self::config_paths::set_sorotte_cli_config_cli_overrides;
#[cfg(test)]
use self::diagnostics_config::{
    ClientLoopDiagnosticsConfig, apply_legacy_client_arg_diagnostics_overrides,
    reconnect_correction_diagnostics_alert_thresholds_from_env,
    reconnect_correction_diagnostics_format_from_env,
};
use self::env_support::{env_flag_enabled, env_trimmed};
#[cfg(test)]
use self::env_support::{
    parse_env_bool_legacy_compatible, parse_env_non_negative_f64_legacy_compatible,
    parse_env_port_legacy_compatible, parse_env_string_list_legacy_compatible,
};
use self::language_support::{
    resolved_legacy_runtime_language_tag_legacy_compatible,
    set_legacy_runtime_language_for_process_legacy_compatible,
};
#[cfg(test)]
use self::local_runtime_actions::{
    publish_pending_local_file_updates, run_planned_local_runtime_action_legacy_compatible,
};
#[cfg(test)]
use self::mpv_startup::spawn_legacy_external_player_from_spec_legacy_compatible;
use self::mpv_startup::spawn_legacy_external_player_if_requested_legacy_compatible;
#[cfg(test)]
use self::mpv_startup::{
    LegacyExplicitMpvIpcStartupPlayerArgDiagnostics, LegacyExplicitMpvIpcStartupPlayerArgs,
    LegacyExplicitMpvIpcStartupPlayerCommand, LegacyExternalPlayerLaunchSpec,
    ManagedMpvLaunchEnvConfig,
    analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible,
    apply_legacy_client_arg_managed_mpv_overrides, find_default_managed_mpv_bin,
    legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible,
    legacy_external_player_launch_spec_from_overrides_legacy_compatible,
    legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible,
    legacy_player_path_requests_managed_mpv_legacy_compatible,
    managed_mpv_launch_base_args_legacy_compatible, managed_mpv_launch_env_config_from_env,
    parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible,
    resolve_managed_mpv_launch_program_legacy_compatible,
    should_skip_legacy_external_player_launch_due_to_mpv_integration_env,
};
#[cfg(test)]
use self::mpv_startup::{
    apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible,
    create_client_runtime_with_managed_mpv_support,
    create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test,
    create_client_runtime_with_prepared_mpv_and_startup_health_for_test,
    create_client_runtime_with_prepared_mpv_for_test,
    legacy_player_path_compatibility_warning_line_legacy_compatible,
};
#[cfg(all(test, windows))]
use self::mpv_startup::{
    connect_mpv_adapter_with_retry, retry_explicit_mpv_ipc_startup_player_command_legacy_compatible,
};
#[cfg(test)]
use self::notifications::{
    autoplay_countdown_notification_message_localized_legacy_compatible, chat_notification_message,
    controller_auth_notification_hidden_from_osd, controller_auth_transition_notification_message,
    controller_auth_transition_notification_message_localized_legacy_compatible,
    flush_autoplay_notifications_to_sink, flush_chat_notifications_to_sink,
    flush_controller_auth_notifications_to_sink, flush_file_difference_notifications_to_sink,
    flush_reconnect_correction_diagnostics_to_sink, flush_reconnect_notifications_to_sink,
    flush_user_change_notifications_to_sink, format_duration_legacy,
    format_file_difference_summary, localized_file_difference_summary_legacy_compatible,
    player_playback_drift_diagnostic_messages_localized_legacy_compatible,
    player_playback_telemetry_update_message,
    player_playback_telemetry_update_message_localized_legacy_compatible,
    reconnect_transition_notification_message,
    reconnect_transition_notification_message_localized_legacy_compatible,
    seek_preparation_diagnostic_messages, user_change_notification_hidden_from_osd,
    user_change_notification_message, user_change_notification_message_localized_legacy_compatible,
};
use self::session_runner::run_client_network_loop_with_legacy_startup_overrides_and_stored_settings;
#[cfg(test)]
use self::session_runner::{
    cli_plex_config_from_env_and_stored_settings, run_client_network_loop,
    run_connected_client_session, run_connected_client_session_with_legacy_startup_overrides,
};
#[cfg(test)]
use self::startup_playlist::protocol_lines_for_startup_playlist_load_from_file_legacy_compatible;
#[cfg(test)]
use self::stored_settings::{
    apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible,
    legacy_syncplay_ui_settings_from_stored_settings, parse_sorotte_ini_stored_client_settings_mvp,
    resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible,
    upsert_sorotte_ini_stored_client_settings_mvp,
};
use self::stored_settings::{
    apply_stored_client_settings_mvp_if_env_absent,
    apply_stored_legacy_startup_player_defaults_if_arg_absent,
    apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible,
    clear_sorotte_cli_gui_state, clear_sorotte_cli_stored_settings_legacy_compatible,
    load_sorotte_cli_stored_settings_mvp_legacy_compatible,
    persist_sorotte_cli_language_setting_legacy_compatible,
    persist_sorotte_cli_per_player_arguments_setting_legacy_compatible,
    persist_sorotte_cli_player_path_setting_legacy_compatible,
    persist_sorotte_cli_stored_settings_mvp_legacy_compatible,
};
use self::update_check::apply_headless_automatic_update_check_legacy_compatible;
#[cfg(test)]
use self::update_check::{
    LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS, legacy_utc_timestamp_string_legacy_compatible,
    parse_legacy_utc_timestamp_legacy_compatible,
    persist_sorotte_cli_last_checked_for_updates_setting_legacy_compatible,
    should_run_headless_automatic_update_check_legacy_compatible,
};

fn persist_explicit_legacy_client_arg_settings(overrides: &LegacyClientArgOverrides) {
    if let Some(language) = overrides.language.as_deref()
        && !overrides.no_store
        && let Err(error) = persist_sorotte_cli_language_setting_legacy_compatible(language)
    {
        eprintln!("warning: failed to persist legacy --language setting: {error}");
    }
    if let Some(player_path) = overrides.player_path.as_deref()
        && !overrides.no_store
        && let Err(error) = persist_sorotte_cli_player_path_setting_legacy_compatible(player_path)
    {
        eprintln!("warning: failed to persist legacy --player-path setting: {error}");
    }
    if let Some(player_path) = overrides.player_path.as_deref()
        && !overrides.no_store
        && !overrides.player_args.is_empty()
        && let Err(error) = persist_sorotte_cli_per_player_arguments_setting_legacy_compatible(
            player_path,
            &overrides.player_args,
        )
    {
        eprintln!("warning: failed to persist legacy per-player arguments setting: {error}");
    }
}

pub async fn run_sorotte_cli_from_env() -> anyhow::Result<()> {
    let mut client_arg_overrides = parse_legacy_client_arg_overrides(std::env::args().skip(1));
    if client_arg_overrides.show_version {
        println!("sorotte-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if client_arg_overrides.show_help {
        print_legacy_client_help(client_arg_overrides.language.as_deref());
        return Ok(());
    }
    if !client_arg_overrides.unknown_options.is_empty() {
        eprintln!(
            "{}",
            legacy_unrecognized_arguments_diagnostic_line(&client_arg_overrides.unknown_options)
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
        match clear_sorotte_cli_stored_settings_legacy_compatible() {
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
        persist_explicit_legacy_client_arg_settings(&client_arg_overrides);
    }
    emit_legacy_client_arg_compatibility_warnings(&client_arg_overrides);
    if client_arg_overrides.should_halt_for_legacy_force_gui_prompt_compatibility() {
        return Ok(());
    }
    let stored_settings = match load_sorotte_cli_stored_settings_mvp_legacy_compatible() {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("warning: failed to load stored Sorotte settings: {error}");
            None
        }
    };
    if let Some(stored_settings) = stored_settings.as_ref() {
        if let Some(line) = stored_force_gui_prompt_compatibility_line_legacy_compatible(
            &client_arg_overrides,
            stored_settings,
        ) {
            eprintln!("{line}");
        }
        if should_halt_for_stored_force_gui_prompt_legacy_compatible(
            &client_arg_overrides,
            stored_settings,
        ) {
            return Ok(());
        }
    }
    set_legacy_runtime_language_for_process_legacy_compatible(
        resolved_legacy_runtime_language_tag_legacy_compatible(
            &client_arg_overrides,
            stored_settings.as_ref(),
        ),
    );
    if should_connect {
        let mut config = build_client_loop_config_from_env();
        if let Some(stored_settings) = stored_settings.as_ref() {
            apply_stored_client_settings_mvp_if_env_absent(&mut config, stored_settings);
            apply_stored_legacy_startup_player_defaults_if_arg_absent(
                &mut client_arg_overrides,
                stored_settings,
            );
        }
        apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible(
            &mut client_arg_overrides,
            stored_settings.as_ref(),
        );
        apply_legacy_client_arg_overrides(&mut config, &client_arg_overrides);
        validate_composed_client_endpoint(&config)
            .map_err(|error| anyhow!("invalid client endpoint: {error}"))?;
        persist_explicit_legacy_client_arg_settings(&client_arg_overrides);
        apply_headless_automatic_update_check_legacy_compatible(
            &client_arg_overrides,
            stored_settings.as_ref(),
        );
        if !client_arg_overrides.no_store
            && let Err(error) = persist_sorotte_cli_stored_settings_mvp_legacy_compatible(&config)
        {
            eprintln!("warning: failed to persist stored Sorotte settings: {error}");
        }
        if let Err(error) =
            spawn_legacy_external_player_if_requested_legacy_compatible(&client_arg_overrides)
        {
            eprintln!("warning: failed to launch legacy external player startup path: {error}");
        }
        run_client_network_loop_with_legacy_startup_overrides_and_stored_settings(
            &config,
            client_arg_overrides.load_playlist_from_file.as_deref(),
            Some(&client_arg_overrides),
            stored_settings.as_ref(),
        )
        .await?;
        return Ok(());
    }

    apply_headless_automatic_update_check_legacy_compatible(
        &client_arg_overrides,
        stored_settings.as_ref(),
    );

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
