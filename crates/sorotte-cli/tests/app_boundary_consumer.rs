use sorotte_client_app::app_boundary::{
    commands, compatibility, diagnostics, language, notifications, persistence, session, state,
};

#[test]
fn sorotte_cli_package_consumes_app_boundary_runtime_surface() {
    assert!(!compatibility::syncplay_configuration_getter_startup_compat_entries().is_empty());
    assert!(language::SUPPORTED_RUNTIME_LANGUAGE_TAGS_DISPLAY.contains("de/en/es"));
    assert!(commands::parse_local_input_command("list").is_some());
    assert!(matches!(
        diagnostics::ReconnectCorrectionDiagnosticsFormat::Text,
        diagnostics::ReconnectCorrectionDiagnosticsFormat::Text
    ));
    assert!(matches!(
        session::ConnectedSessionOuterLoopExitKind::TransportClosed,
        session::ConnectedSessionOuterLoopExitKind::TransportClosed
    ));
    assert!(!notifications::format_duration(5.0).is_empty());
}

#[test]
fn sorotte_cli_package_consumes_app_boundary_state_and_persistence_surface() {
    let parsed = persistence::parse_sorotte_ini_stored_client_settings(
        "[server_data]\nhost = syncplay.test\n",
    );
    assert_eq!(parsed.host.as_deref(), Some("syncplay.test"));

    let serialized =
        persistence::format_serialized_string_list(&["alpha".to_string(), "beta".to_string()]);
    assert!(serialized.contains("alpha"));

    let settings = state::StoredClientSettings {
        host: Some("example.com".to_string()),
        ..state::StoredClientSettings::default()
    };
    let snapshot = state::stored_client_settings_runtime_snapshot(&settings);
    assert_eq!(
        snapshot.config.connection.host.as_deref(),
        Some("example.com")
    );
}
