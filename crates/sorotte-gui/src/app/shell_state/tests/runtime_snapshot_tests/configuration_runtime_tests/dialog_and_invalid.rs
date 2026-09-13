use super::*;

#[test]
fn gui_shell_app_state_preserves_open_modal_across_configuration_runtime_snapshots() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());

    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::About)));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.open_modal, Some(GuiShellModal::About));
}

#[test]
fn gui_shell_app_state_tracks_media_trust_policy_without_opening_a_modal() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());

    assert_eq!(
        state
            .configuration
            .control_value(SettingId::PrivacyTrustedDomainsOnly),
        Some("yes")
    );

    for value in [false, true] {
        assert!(state.apply(GuiShellAction::EditConfigurationBool {
            id: SettingId::PrivacyTrustedDomainsOnly,
            value,
        }));
        assert_eq!(
            state
                .configuration
                .to_stored_settings()
                .only_switch_to_trusted_domains,
            Some(value)
        );
        assert_eq!(state.open_modal, None);
        assert!(
            state
                .menu_dialog_widget_tree()
                .find("menu.tls_certificates")
                .is_none()
        );
    }

    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .only_switch_to_trusted_domains,
        Some(true)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_runtime_snapshots() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: StoredClientSettings {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettings::default()
                },
                saved_settings: StoredClientSettings {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettings::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI configuration runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
}
