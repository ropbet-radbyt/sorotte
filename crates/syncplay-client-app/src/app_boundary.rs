pub mod commands {
    pub use crate::legacy_local_commands::{
        LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
        LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
        PlannedLocalRuntimeAction, PlannedLocalRuntimeDispatch,
        controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
        local_input_error_output_line_legacy_compatible,
        localized_current_offset_message_legacy_compatible,
        localized_local_input_error_message_legacy_compatible, parse_local_input_chat_message,
        parse_local_input_command, parse_seek_time_seconds_legacy_like,
        plan_local_input_command_legacy_compatible, plan_local_input_dispatch_legacy_compatible,
        plan_local_offset_runtime_dispatch_legacy_compatible,
        plan_local_playlist_delete_runtime_dispatch_legacy_compatible,
        plan_local_playlist_select_runtime_dispatch_legacy_compatible,
        plan_local_runtime_dispatch_legacy_compatible, playlist_index_in_bounds_legacy_compatible,
        playlist_listing_message_legacy_compatible,
        playlist_listing_message_localized_legacy_compatible,
        render_local_input_display_lines_legacy_compatible,
        resolved_local_user_offset_seconds_legacy_compatible,
    };
}

pub mod compatibility {
    pub use crate::legacy_compat::{
        LegacyConfigurationGetterCompatibilityStatus, LegacyConfigurationGetterIniCompatEntry,
        LegacyConfigurationGetterStartupCompatEntry,
        legacy_configuration_getter_ini_compat_entries,
        legacy_configuration_getter_startup_compat_entries,
    };
}

pub mod diagnostics {
    pub use crate::legacy_reconnect_diagnostics::{
        ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
        ReconnectCorrectionDiagnosticsState,
        next_reconnect_correction_diagnostic_lines_legacy_compatible,
        reconnect_correction_metrics_delta_alert_lines,
        reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible,
        reconnect_correction_metrics_delta_json_line, reconnect_correction_metrics_delta_message,
        reconnect_correction_metrics_delta_message_localized_legacy_compatible,
        reconnect_correction_state_snapshot_json_line, reconnect_correction_state_snapshot_message,
        reconnect_correction_state_snapshot_message_localized_legacy_compatible,
        reconnect_correction_state_threshold_alert_lines,
    };
}

pub mod language {
    pub use crate::legacy_language::{
        SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
        legacy_runtime_language_acknowledgement_line_legacy_compatible,
        legacy_runtime_language_selection_line_legacy_compatible,
        normalized_legacy_runtime_language_tag_legacy_compatible,
        resolve_legacy_runtime_language_tag_legacy_compatible,
    };
}

pub mod notifications {
    pub use crate::legacy_notifications::{
        FileDifferenceNotificationState, controller_auth_notification_hidden_from_osd,
        controller_auth_transition_notification_message,
        controller_auth_transition_notification_message_localized_legacy_compatible,
        format_duration_legacy, format_file_difference_summary,
        localized_file_difference_notification_line_legacy_compatible,
        localized_file_difference_summary_legacy_compatible,
        localized_file_differences_prefix_legacy_compatible,
        next_file_difference_notification_summary_legacy_compatible,
        reconnect_transition_notification_message,
        reconnect_transition_notification_message_localized_legacy_compatible,
        user_change_notification_hidden_from_osd, user_change_notification_message,
        user_change_notification_message_localized_legacy_compatible,
    };
}

pub mod persistence {
    pub use crate::legacy_ini_serde::{
        format_serialized_per_player_arguments_map_legacy_compatible,
        format_serialized_public_servers_list_legacy_compatible,
        format_serialized_string_list_legacy_compatible,
        parse_serialized_per_player_arguments_map_legacy_compatible,
        parse_serialized_public_servers_list_legacy_compatible,
        parse_serialized_string_list_legacy_compatible,
    };
    pub use crate::legacy_syncplay_ini::{
        clear_syncplay_ini_stored_client_settings_mvp_at_path,
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        parse_syncplay_ini_stored_client_settings_mvp,
        update_syncplay_ini_stored_client_settings_mvp_at_path,
        upsert_syncplay_ini_stored_client_settings_mvp,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    };
}

pub mod session {
    pub use crate::legacy_session_loop::{
        ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
        ClientNetworkLoopAttemptPlan, ClientNetworkLoopEventPlan,
        ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
        ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
        ClientNetworkLoopStartupPlanInputs, ConnectedSessionBranchPlan,
        ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction, ConnectedSessionDrainPlan,
        ConnectedSessionEventExecutionPlan, ConnectedSessionInboundApplyPlan,
        ConnectedSessionInboundPostApplyAction, ConnectedSessionInboundPostApplyPlan,
        ConnectedSessionOuterLoopExitKind, ConnectedSessionProtocolPlan,
        ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
        ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
        client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible,
        client_network_loop_execution_outcome_legacy_compatible,
        client_network_loop_reconnect_exhausted_error_action_legacy_compatible,
        client_network_loop_startup_plan_legacy_compatible,
        client_reconnect_backoff_plan_legacy_compatible,
        connected_session_autoplay_tick_event_execution_plan_legacy_compatible,
        connected_session_drain_actions_legacy_compatible,
        connected_session_inbound_message_event_execution_plan_legacy_compatible,
        connected_session_inbound_post_apply_actions_legacy_compatible,
        connected_session_local_input_event_execution_plan_legacy_compatible,
        connected_session_runtime_step_actions_legacy_compatible,
    };
}

pub mod state {
    pub use crate::legacy_runtime_config::{
        StoredClientSettingsConfigPlan, StoredClientSettingsEnvPresence,
        StoredClientSettingsRuntimeSnapshot, normalize_controlled_room_input_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
        stored_client_settings_config_plan_legacy_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    };
    pub use crate::legacy_settings::{
        AutoplayThresholdOverride, StoredClientSettingsMvp,
        autoplay_threshold_override_legacy_value_compatible,
        parse_autoplay_min_users_override_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible, privacy_mode_legacy_name_compatible,
        unpause_action_mode_legacy_name_compatible,
    };
}

#[cfg(test)]
mod tests {
    use super::{
        commands, compatibility, diagnostics, language, notifications, persistence, session, state,
    };

    #[test]
    fn app_boundary_commands_compatibility_and_language_surface_remain_available() {
        assert!(!compatibility::legacy_configuration_getter_startup_compat_entries().is_empty());
        assert!(!compatibility::legacy_configuration_getter_ini_compat_entries().is_empty());
        assert!(language::SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.contains("de/en/es"));
        assert!(commands::parse_local_input_command("list").is_some());
    }

    #[test]
    fn app_boundary_notifications_diagnostics_and_session_surface_remain_available() {
        assert!(matches!(
            diagnostics::ReconnectCorrectionDiagnosticsFormat::Text,
            diagnostics::ReconnectCorrectionDiagnosticsFormat::Text
        ));
        assert!(matches!(
            session::ConnectedSessionRuntimeStepAction::RunRoomPauseSync,
            session::ConnectedSessionRuntimeStepAction::RunRoomPauseSync
        ));
        assert!(matches!(
            session::ConnectedSessionOuterLoopExitKind::TransportClosed,
            session::ConnectedSessionOuterLoopExitKind::TransportClosed
        ));
        assert!(!notifications::format_duration_legacy(65.0).is_empty());
    }

    #[test]
    fn app_boundary_state_and_persistence_surface_round_trip_basic_values() {
        let mut settings = state::StoredClientSettingsMvp::default();
        settings.host = Some("example.com".to_string());
        let config_plan = state::stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &state::StoredClientSettingsEnvPresence::default(),
        );
        assert_eq!(config_plan.host.as_deref(), Some("example.com"));

        let parsed = persistence::parse_syncplay_ini_stored_client_settings_mvp(
            "[server_data]\nhost = syncplay.test\n",
        );
        assert_eq!(parsed.host.as_deref(), Some("syncplay.test"));

        let serialized = persistence::format_serialized_string_list_legacy_compatible(&[
            "alpha".to_string(),
            "beta".to_string(),
        ]);
        assert!(serialized.contains("alpha"));
    }
}
