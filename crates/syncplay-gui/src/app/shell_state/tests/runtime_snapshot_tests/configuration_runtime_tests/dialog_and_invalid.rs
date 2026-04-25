use super::*;

#[test]
fn gui_shell_app_state_updates_dialog_expectations_from_configuration_edits_without_runtime_overrides()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Privacy",
        label: "Trusted Domains Only",
        value: true,
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "System",
        label: "Auto Update",
        value: true,
    }));

    assert!(state.menus.tls_prompt_expected);
    assert!(!state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_preserves_runtime_dialog_expectations_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));

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
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: StoredClientSettingsMvp {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
                saved_settings: StoredClientSettingsMvp {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
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
