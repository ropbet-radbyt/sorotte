pub mod application {
    pub use crate::application::{
        ClientApplication, ClientApplicationSettings, ClientCommand, ClientEvent, ConnectionPhase,
        PlexClientConfig, ProtocolLineApplyOutcome,
    };
}

pub mod commands {
    pub use crate::local_commands::{
        LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
        LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
        PlannedLocalRuntimeAction, PlannedLocalRuntimeDispatch, controlled_room_base_name,
        generate_room_password, local_input_error_output_line, localized_current_offset_message,
        localized_local_input_error_message, parse_local_input_chat_message,
        parse_local_input_command, parse_seek_time_seconds, plan_local_input_command,
        plan_local_input_dispatch, plan_local_offset_runtime_dispatch,
        plan_local_playlist_delete_runtime_dispatch, plan_local_playlist_select_runtime_dispatch,
        plan_local_runtime_dispatch, playlist_index_in_bounds, playlist_listing_message,
        playlist_listing_message_localized, render_local_input_display_lines,
        resolved_local_user_offset_seconds,
    };
}

pub mod compatibility {
    pub use crate::syncplay_config_contract::{
        SyncplayConfigurationGetterCompatibilityStatus, SyncplayConfigurationGetterIniCompatEntry,
        SyncplayConfigurationGetterStartupCompatEntry,
        syncplay_configuration_getter_ini_compat_entries,
        syncplay_configuration_getter_startup_compat_entries,
    };
}

pub mod diagnostics {
    pub use crate::reconnect_diagnostics::{
        ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
        ReconnectCorrectionDiagnosticsState, next_reconnect_correction_diagnostic_lines,
        reconnect_correction_metrics_delta_alert_lines,
        reconnect_correction_metrics_delta_alert_lines_localized,
        reconnect_correction_metrics_delta_json_line, reconnect_correction_metrics_delta_message,
        reconnect_correction_metrics_delta_message_localized,
        reconnect_correction_state_snapshot_json_line, reconnect_correction_state_snapshot_message,
        reconnect_correction_state_snapshot_message_localized,
        reconnect_correction_state_threshold_alert_lines,
    };
}

pub mod language {
    pub use crate::language::{
        SUPPORTED_RUNTIME_LANGUAGE_TAGS_DISPLAY, normalized_runtime_language_tag,
        resolve_runtime_language_tag, runtime_language_acknowledgement_line,
        runtime_language_selection_line,
    };
}

pub mod notifications {
    pub use crate::notifications::{
        FileDifferenceNotificationState, controller_auth_notification_hidden_from_osd,
        controller_auth_transition_notification_message,
        controller_auth_transition_notification_message_localized, format_duration,
        format_file_difference_summary, localized_file_difference_notification_line,
        localized_file_difference_summary, localized_file_differences_prefix,
        next_file_difference_notification_summary, reconnect_transition_notification_message,
        reconnect_transition_notification_message_localized,
        user_change_notification_hidden_from_osd, user_change_notification_message,
        user_change_notification_message_localized,
    };
}

pub mod persistence {
    pub use crate::sorotte_ini::{
        clear_sorotte_ini_stored_client_settings_at_path, create_private_directory,
        edit_sorotte_ini_stored_client_settings_at_path,
        load_sorotte_ini_stored_client_settings_from_path,
        merge_sorotte_ini_stored_client_settings_at_path, parse_sorotte_ini_stored_client_settings,
        relocate_sorotte_ini_stored_client_settings_at_path,
        update_sorotte_ini_stored_client_settings_at_path,
        upsert_sorotte_ini_stored_client_settings,
        upsert_sorotte_ini_stored_client_settings_at_path,
        upsert_sorotte_ini_stored_client_settings_clearing_plex_identity,
        upsert_sorotte_ini_stored_client_settings_clearing_plex_identity_at_path,
        write_sorotte_ini_contents_atomically_at_path,
    };
    pub use crate::syncplay_ini_values::{
        format_serialized_per_player_arguments_map, format_serialized_public_servers_list,
        format_serialized_string_list, parse_serialized_per_player_arguments_map,
        parse_serialized_public_servers_list, parse_serialized_string_list,
    };
}

pub mod participant_status {
    pub use crate::participant_status_presentation::{
        ParticipantStatusFreshness, ParticipantStatusPresentation,
        ParticipantStatusReportPresentation, format_participant_status_timestamp,
    };
}

pub mod readiness {
    pub use crate::readiness_presentation::{
        ParticipantReadinessPresentation, PendingReadinessIntentPresentation,
        ReadinessPresentationProtocol,
    };
}

pub mod storage {
    pub use crate::client_storage_paths::{
        SOROTTE_CLIENT_CONFIG_PATH_ENV, SOROTTE_CLIENT_CONFIG_ROOT_ENV,
        SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME, SOROTTE_CLIENT_INSTALL_ROOT_ENV,
        SOROTTE_CONFIG_FILE_NAME, SOROTTE_INSTALL_CONFIG_LOCATOR_FILE_NAME,
        SOROTTE_INSTALL_CONFIG_ROOT_KEY, SorotteClientStoragePaths, SorotteClientStorageSource,
        clear_sorotte_client_config_root_pointer, current_sorotte_client_install_root,
        default_sorotte_client_config_root, default_sorotte_client_config_root_from_lookup,
        ensure_sorotte_client_install_locator, ensure_sorotte_client_storage_root,
        load_sorotte_client_config_root_pointer_from_path, normalize_path,
        parse_sorotte_client_install_locator_config_root, paths_equivalent,
        persist_sorotte_client_config_root_pointer, persist_sorotte_client_install_locator,
        resolve_sorotte_client_storage_paths_from_lookup,
        resolve_sorotte_client_storage_paths_from_lookup_with_install_root,
        sorotte_client_config_root_pointer_path, sorotte_client_install_locator_contents,
        sorotte_client_install_locator_path, sorotte_client_install_root_from_lookup,
        try_resolve_sorotte_client_storage_paths,
        try_resolve_sorotte_client_storage_paths_from_lookup_with_install_root,
    };
}

pub mod session {
    pub use crate::session_loop::{
        ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
        ClientNetworkLoopAttemptPlan, ClientNetworkLoopEventPlan,
        ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
        ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
        ClientNetworkLoopStartupPlanInputs, ClientReconnectBackoffPlan, ConnectedSessionBranchPlan,
        ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction, ConnectedSessionDrainPlan,
        ConnectedSessionEventExecutionPlan, ConnectedSessionInboundApplyPlan,
        ConnectedSessionInboundPostApplyAction, ConnectedSessionInboundPostApplyPlan,
        ConnectedSessionOuterLoopExitKind, ConnectedSessionProtocolPlan,
        ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
        ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
        client_network_loop_attempt_disposition_for_execution_plan,
        client_network_loop_attempt_execution_plan_for_connect_failure,
        client_network_loop_attempt_execution_plan_for_connected_session_exit,
        client_network_loop_execution_outcome,
        client_network_loop_reconnect_exhausted_error_action, client_network_loop_startup_plan,
        client_reconnect_backoff_plan, connected_session_autoplay_tick_event_execution_plan,
        connected_session_drain_actions, connected_session_inbound_message_event_execution_plan,
        connected_session_inbound_post_apply_actions,
        connected_session_local_input_event_execution_plan,
        connected_session_player_coordination_tick_event_execution_plan,
        connected_session_runtime_step_actions,
    };
}

pub mod state {
    pub use crate::runtime_config::{
        ClientConfig, ClientConfigErrors, ClientConfigIssue, ClientConfigResolution,
        ConnectionConfig, EffectiveMpvStreamingOption, InterfaceConfig, MediaMatchConfig, Percent,
        PlaybackConfig, PlaybackRate, PlexConfig, PluginConfig, PublicServerConfig,
        ReadinessConfig, RoomBufferingConfig, RoomBufferingPolicy, RoomName, Seconds, ServerPort,
        StartSynchronizationConfig, StartSynchronizationPolicy, StartTimeoutAction,
        StreamingBufferConfig, StreamingPlaybackConfig, StreamingQualityDowngradeSuggestion,
        StreamingQualityPreset, StreamingQualitySuggestionReason, StreamingRecoveryConfig,
        StreamingRecoveryPolicy, SyncConfig, TlsPolicy, Username, resolve_client_config,
    };
    pub use crate::stored_config::{
        StoredClientSettingsConfigPlan, StoredClientSettingsEnvPresence,
        StoredClientSettingsRuntimeSnapshot, normalize_controlled_room_input,
        parse_host_and_optional_port_from_host_arg, stored_client_settings_config_plan,
        stored_client_settings_runtime_snapshot,
    };
    pub use crate::stored_settings::{
        AutoplayThresholdOverride, StoredClientSettings, autoplay_threshold_override_setting_value,
        parse_autoplay_min_users_override, parse_unpause_action_mode, privacy_mode_syncplay_name,
        unpause_action_mode_syncplay_name,
    };
}

#[cfg(test)]
mod tests {
    use super::{
        commands, compatibility, diagnostics, language, notifications, persistence, session, state,
        storage,
    };

    #[test]
    fn app_boundary_commands_compatibility_and_language_surface_remain_available() {
        assert!(!compatibility::syncplay_configuration_getter_startup_compat_entries().is_empty());
        assert!(!compatibility::syncplay_configuration_getter_ini_compat_entries().is_empty());
        assert!(language::SUPPORTED_RUNTIME_LANGUAGE_TAGS_DISPLAY.contains("de/en/es"));
        assert!(commands::parse_local_input_command("list").is_some());
        assert_eq!(storage::SOROTTE_CONFIG_FILE_NAME, "sorotte.ini");
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
        assert!(!notifications::format_duration(65.0).is_empty());
    }

    #[test]
    fn app_boundary_state_and_persistence_surface_round_trip_basic_values() {
        let settings = state::StoredClientSettings {
            host: Some("example.com:8998".to_string()),
            ..state::StoredClientSettings::default()
        };
        let runtime_config = state::ClientConfig::try_from_stored(&settings)
            .expect("valid stored settings should resolve");
        assert_eq!(
            runtime_config.connection.host.as_deref(),
            Some("example.com")
        );
        assert_eq!(runtime_config.connection.port.get(), 8998);
        let config_plan = state::stored_client_settings_config_plan(
            &settings,
            &state::StoredClientSettingsEnvPresence::default(),
        );
        assert_eq!(config_plan.host.as_deref(), Some("example.com"));
        assert_eq!(config_plan.port, Some(8998));

        let parsed = persistence::parse_sorotte_ini_stored_client_settings(
            "[server_data]\nhost = syncplay.test\n",
        );
        assert_eq!(parsed.host.as_deref(), Some("syncplay.test"));

        let serialized =
            persistence::format_serialized_string_list(&["alpha".to_string(), "beta".to_string()]);
        assert!(serialized.contains("alpha"));
    }
}
