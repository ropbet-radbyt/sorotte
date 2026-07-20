use super::*;

use crate::app::{GuiPlayerSetupIssueKind, GuiShellModal};

#[test]
fn gui_portable_smoke_regression_surfaces_first_run_player_setup_blocker() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        None,
        &|_name| None,
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "first-run.example".to_owned().into(),
    }));

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(_)
        )),
        "first-run smoke should project a player-setup runtime snapshot"
    );
    assert_eq!(
        state.player_setup_issue.as_ref().map(|issue| issue.kind),
        Some(GuiPlayerSetupIssueKind::NotConfigured)
    );
    assert_eq!(state.open_modal, Some(GuiShellModal::PlayerSetup));
    assert!(
        !state.commands.can_connect_saved_server,
        "first-run connect should stay blocked until mpv is configured"
    );

    let configuration = state.configuration_widget_tree();
    assert!(configuration.find("config-player-setup").is_some());
    assert_eq!(
        configuration
            .find("config-player-setup:blocking")
            .and_then(|node| node.value.as_deref()),
        Some(
            "Set up mpv before connecting. Use Auto-detect, Choose mpv.exe, or Retry mpv after updating Player Path."
        )
    );
    assert!(
        !configuration
            .find("config-command:connect-once")
            .expect("connect button should exist")
            .enabled
    );
    assert!(
        !configuration
            .find("config-player-setup:retry")
            .expect("retry button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref()),
        Some("player-setup")
    );
    assert!(
        !shell
            .find("shell:modal:close")
            .expect("close button should exist")
            .enabled
    );
}

#[test]
fn gui_portable_smoke_regression_surfaces_existing_config_player_recovery() {
    let settings = StoredClientSettingsMvp {
        host: Some("existing.example".to_owned()),
        room: Some("Cinema".to_owned()),
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.sync_player_from_lookup_and_settings(&|_name| None, Some(&settings), true);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(_)
        )),
        "existing-config smoke should project a player-setup runtime snapshot"
    );
    assert_eq!(
        state.player_setup_issue.as_ref().map(|issue| issue.kind),
        Some(GuiPlayerSetupIssueKind::MissingBinary)
    );
    assert_eq!(state.open_modal, Some(GuiShellModal::PlayerSetup));

    let main_window = state.main_window_widget_tree();
    assert!(main_window.find("main-window:player-setup").is_some());
    assert!(
        main_window
            .find("main-window:player-setup:retry")
            .expect("retry button should exist")
            .enabled
    );
    assert!(
        main_window
            .find("main-window:player-setup:open-settings")
            .expect("open-settings button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert!(
        shell
            .find("shell:modal:close")
            .expect("close button should exist")
            .enabled
    );
}
