use super::*;

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_edits() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned().into(),
    }));

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_runtime_snapshots()
{
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

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
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_catches_up() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Output",
        value: false,
    }));
    assert!(state.runtime_menu_action_overrides.is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_output_enabled = Some(false);
    let saved = state.saved_configuration.clone();
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );
    assert!(state.runtime_menu_action_overrides.is_empty());

    draft.chat_output_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved,
            }
        ))
    );

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
}
