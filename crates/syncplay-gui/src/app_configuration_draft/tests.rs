use super::FirstRunConfigurationDialogDraft;

use syncplay_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
};
use syncplay_client_core::UnpauseActionMode;

#[test]
fn configuration_draft_applies_edits_and_round_trips_to_stored_settings() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(draft.apply_text_value("Connection", "Host", "syncplay.example"));
    assert!(draft.apply_text_value("Connection", "Port", "8995"));
    assert!(draft.apply_text_value("Connection", "Server Password", "secret"));
    assert!(draft.apply_bool_value("Readiness", "Autoplay", true));
    assert!(draft.apply_text_value("Readiness", "Unpause Action", "Always"));
    assert!(draft.apply_text_value("Readiness", "Autoplay Min Users", "3"));
    assert!(draft.apply_text_value(
        "Privacy",
        "Trusted Domains",
        "youtube.com; *.example.com/videos"
    ));
    assert!(draft.apply_text_value("System", "Language", "pt-br"));

    let saved = draft.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
    assert_eq!(saved.server_password.as_deref(), Some("secret"));
    assert_eq!(saved.autoplay_initial_state, Some(true));
    assert_eq!(saved.unpause_action, Some(UnpauseActionMode::Always));
    assert_eq!(
        saved.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(
        saved.trusted_domains,
        Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned()
        ])
    );
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(
        draft.control_value("Privacy", "Trusted Domain Count"),
        Some("2")
    );
}

#[test]
fn configuration_draft_rejects_readonly_control_edits() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!draft.apply_text_value("Connection", "Public Servers", "5"));
    assert_eq!(draft.to_stored_settings().public_servers, None);
}
