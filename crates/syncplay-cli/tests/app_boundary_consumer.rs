use syncplay_client_app::app_boundary::{
    commands, compatibility, diagnostics, language, notifications, persistence, session, state,
};

#[test]
fn syncplay_cli_package_consumes_app_boundary_runtime_surface() {
    assert!(!compatibility::legacy_configuration_getter_startup_compat_entries().is_empty());
    assert!(language::SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.contains("de/en/es"));
    assert!(commands::parse_local_input_command("list").is_some());
    assert!(matches!(
        diagnostics::ReconnectCorrectionDiagnosticsFormat::Text,
        diagnostics::ReconnectCorrectionDiagnosticsFormat::Text
    ));
    assert!(matches!(
        session::ConnectedSessionOuterLoopExitKind::TransportClosed,
        session::ConnectedSessionOuterLoopExitKind::TransportClosed
    ));
    assert!(!notifications::format_duration_legacy(5.0).is_empty());
}

#[test]
fn syncplay_cli_package_consumes_app_boundary_state_and_persistence_surface() {
    let parsed = persistence::parse_syncplay_ini_stored_client_settings_mvp(
        "[server_data]\nhost = syncplay.test\n",
    );
    assert_eq!(parsed.host.as_deref(), Some("syncplay.test"));

    let serialized = persistence::format_serialized_string_list_legacy_compatible(&[
        "alpha".to_string(),
        "beta".to_string(),
    ]);
    assert!(serialized.contains("alpha"));

    let mut settings = state::StoredClientSettingsMvp::default();
    settings.host = Some("example.com".to_string());
    let config_plan = state::stored_client_settings_config_plan_legacy_compatible(
        &settings,
        &state::StoredClientSettingsEnvPresence::default(),
    );
    assert_eq!(config_plan.host.as_deref(), Some("example.com"));
}
