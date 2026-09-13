use super::*;

#[test]
fn gui_configuration_preserves_typed_menu_runtime_overrides() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                id: MenuActionId::CheckForUpdates,
                enabled: false,
            }],
            about_dialog_available: state.menus.about_dialog_available,
        },
    )));

    assert_eq!(
        state
            .menus
            .action(MenuActionId::CheckForUpdates)
            .map(|action| action.enabled),
        Some(false)
    );
    assert_eq!(
        state.runtime_menu_action_overrides,
        vec![MenuActionRuntimeOverride {
            id: MenuActionId::CheckForUpdates,
            enabled: false,
        }]
    );
}
