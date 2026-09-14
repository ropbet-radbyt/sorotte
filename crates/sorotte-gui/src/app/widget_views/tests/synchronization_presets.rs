use super::*;
use crate::app::shell_state::{SecretDraft, SynchronizationPreset};

#[test]
fn preset_buttons_edit_the_draft_and_save_reload_and_discard_use_the_saved_baseline() {
    let initial = StoredClientSettings::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&initial);
    for tab in [
        GuiConfigurationTab::Overview,
        GuiConfigurationTab::PlaybackSearch,
    ] {
        assert!(state.apply(GuiShellAction::SelectConfigurationTab(tab)));
        let tree = state.configuration_widget_tree();
        assert_eq!(
            tree.find("settings-preset:current")
                .unwrap()
                .value
                .as_deref(),
            Some("Standard")
        );
        assert!(!tree.find("settings-preset:standard:apply").unwrap().enabled);
        let button = tree.find("settings-preset:watch-together:apply").unwrap();
        assert!(button.enabled);
        assert_eq!(
            GuiWidgetEguiRenderer::actions_for_button_node(&state, button),
            vec![GuiShellAction::ApplySynchronizationPreset(
                SynchronizationPreset::WatchTogether
            )]
        );
    }
    assert!(state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether
    )));
    assert_eq!(state.saved_configuration, initial);
    assert_eq!(
        state.draft_synchronization_preset(),
        Some(SynchronizationPreset::WatchTogether)
    );
    for (id, value) in [
        (SettingId::StreamingStartSynchronization, "wait-all"),
        (SettingId::StreamingStartQuorumPercent, "100"),
        (SettingId::StreamingStartTimeoutSeconds, "30"),
        (SettingId::StreamingStartTimeoutAction, "ask-controller"),
        (SettingId::StreamingRoomBufferingPolicy, "pause-eligible"),
        (SettingId::StreamingRoomQuorumPercent, "100"),
        (SettingId::StreamingRoomMaximumPauseSeconds, "30"),
    ] {
        assert_eq!(state.configuration.control_value(id), Some(value), "{id:?}");
    }
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let committed = state.configuration.to_stored_settings();
    assert!(state.apply(GuiShellAction::CompleteConfigurationSave(committed.clone())));
    assert_eq!(state.saved_configuration, committed);
    assert!(state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::Standard
    )));
    assert!(state.apply(GuiShellAction::BeginDiscardConfigurationChanges));
    assert!(
        state.apply(GuiShellAction::CompleteDiscardConfigurationChanges(
            committed.clone()
        ))
    );
    assert_eq!(
        state.draft_synchronization_preset(),
        Some(SynchronizationPreset::WatchTogether)
    );
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReload(committed)));
    assert_eq!(
        state.draft_synchronization_preset(),
        Some(SynchronizationPreset::WatchTogether)
    );
    assert!(!state.commands.can_save_configuration);
    assert!(!state.has_unsaved_configuration_changes());
}

#[test]
fn preset_repairs_invalid_owned_text_and_preserves_unrelated_invalid_text_and_secret_intent() {
    for clear_secret in [false, true] {
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
            server_password: Some("original".into()),
            streaming_read_ahead_seconds: Some(45.0),
            ..StoredClientSettings::default()
        });
        if clear_secret {
            state.configuration.remove_server_password();
        } else {
            state.configuration.begin_server_password_change();
            state
                .configuration
                .apply_text_value(SettingId::ConnectionServerPassword, "replacement");
        }
        let secret = state.configuration.server_password.clone();
        assert!(matches!(
            secret,
            SecretDraft::Clear | SecretDraft::Replace(_)
        ));
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::StreamingReadAheadSeconds,
            value: "invalid cache edit".to_owned().into(),
        }));
        assert_eq!(
            state.draft_synchronization_preset(),
            Some(SynchronizationPreset::Standard)
        );
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::StreamingStartTimeoutSeconds,
            value: "invalid timeout".to_owned().into(),
        }));
        assert_eq!(state.draft_synchronization_preset(), None);
        assert!(state.apply(GuiShellAction::ApplySynchronizationPreset(
            SynchronizationPreset::Standard
        )));
        assert_eq!(
            state.draft_synchronization_preset(),
            Some(SynchronizationPreset::Standard)
        );
        assert_eq!(
            state
                .configuration
                .control_value(SettingId::StreamingStartTimeoutSeconds),
            Some("15")
        );
        assert_eq!(
            state
                .configuration
                .control_value(SettingId::StreamingReadAheadSeconds),
            Some("invalid cache edit")
        );
        assert_eq!(state.configuration.server_password, secret);
        assert!(
            state
                .validation
                .issues
                .iter()
                .any(|issue| issue.setting_id == Some(SettingId::StreamingReadAheadSeconds))
        );
        assert!(
            !state.apply(GuiShellAction::BeginConfigurationSave),
            "unrelated invalid input must still block saving"
        );
    }
}

#[test]
fn preset_actions_and_buttons_are_blocked_during_an_edit_or_pending_save() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PlaybackSearch
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::StreamingStartTimeoutSeconds
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "22".to_owned().into()
    )));
    assert!(
        !state
            .configuration_widget_tree()
            .find("settings-preset:watch-together:apply")
            .unwrap()
            .enabled
    );
    assert!(!state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether
    )));
    assert!(state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::StreamingStartTimeoutSeconds),
        Some("22")
    );
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let draft = state.configuration.clone();
    assert!(
        !state
            .configuration_widget_tree()
            .find("settings-preset:watch-together:apply")
            .unwrap()
            .enabled
    );
    assert!(!state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether
    )));
    assert_eq!(state.configuration, draft);
    assert!(state.apply(GuiShellAction::CancelPendingOperation));
    assert!(state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether
    )));
    assert!(!state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether
    )));
}
