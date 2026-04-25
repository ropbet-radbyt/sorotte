use super::*;

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
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
        value: "syncplay.example".to_owned(),
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
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
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
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
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
        value: "syncplay.example".to_owned(),
    }));

    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
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
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}
