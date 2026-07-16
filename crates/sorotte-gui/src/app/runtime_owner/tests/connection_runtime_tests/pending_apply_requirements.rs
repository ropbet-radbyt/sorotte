use super::*;
use crate::app::GuiClientCoreChatSessionRuntimeAdapter;

fn save_current_draft(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) {
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let submitted = state.configuration.to_stored_settings();
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(submitted),
    ));
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    let actions = handle.drain_actions();
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyPendingApplyRequirementsSnapshot(_)
    )));
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
}

fn persisted_owner_and_state(
    label: &str,
    settings: &StoredClientSettingsMvp,
) -> (
    PathBuf,
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SorotteGuiShellAppState,
) {
    let root = test_temp_root(label);
    let config_path = root.join("sorotte.ini");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, settings)
        .expect("pending-apply fixture should persist its initial settings");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.startup_saved_connect_attempted = true;
    (
        root,
        owner,
        GuiQueuedRuntimeBridgeHandle::default(),
        SorotteGuiShellAppState::from_stored_settings(settings),
    )
}

#[test]
fn detached_ordinary_save_does_not_report_reconnect() {
    let initial = StoredClientSettingsMvp {
        host: Some("active-a.example".to_owned()),
        port: Some(8999),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-detached-save", &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "saved-b.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.saved_configuration.host.as_deref(),
        Some("saved-b.example")
    );
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn connected_host_save_and_revert_toggle_reconnect_symmetrically() {
    let active = StoredClientSettingsMvp {
        host: Some("active-a.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room-a".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-connected-host", &active);
    owner.session_projects_to_shell = true;
    owner.active_session_settings = Some(
        sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
            &active,
        ),
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "saved-b.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "active-a.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn application_language_and_force_prompt_save_reverts_are_symmetric() {
    let initial = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        force_gui_prompt: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-application-reverts", &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartApplication]
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "en".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(state.pending_apply_requirements.is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralForceGuiPrompt,
        value: true,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartApplication]
    );

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralForceGuiPrompt,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(state.pending_apply_requirements.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_player_save_and_revert_toggle_restart_player_symmetrically() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc-a.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-player-revert", &initial);
    let applied =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&initial),
        )
        .expect("initial player settings should resolve to a launch state");
    owner.player_launch_state = applied.clone();
    owner.applied_player_launch_state = Some(applied);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlayerExecutable,
        value: "C:/Players/vlc-b.exe".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlayerExecutable,
        value: "C:/Players/vlc-a.exe".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn successful_player_retry_reconciles_restart_player_requirement() {
    let initial = StoredClientSettingsMvp::default();
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-player-retry", &initial);
    owner.player_launch_state = GuiPlayerLaunchRuntimeState::TestPlayer;
    owner.applied_player_launch_state = Some(GuiPlayerLaunchRuntimeState::TestPlayer);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];

    handle.push_request(GuiRuntimeRequest::RetryPlayerLaunch);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    assert!(owner.current_player_launch_state_is_applied());
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_autoplay_overrides_do_not_create_reconnect_guidance_on_unrelated_save() {
    let initial = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room-a".to_owned()),
        autoplay_initial_state: Some(false),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(2),
        ),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-live-autoplay", &initial);
    let session = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room-a")
        .expect("autoplay requirement fixture should create a session");
    owner.install_active_session_runtime(
        Box::new(session),
        sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
            &initial,
        ),
    );

    handle.push_request(GuiRuntimeRequest::SetAutoplayEnabled(true));
    handle.push_request(GuiRuntimeRequest::SetAutoplayThreshold(5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }
    let live = owner
        .active_session_settings
        .as_ref()
        .expect("live session settings should remain available");
    let configured = owner
        .active_session_configured_settings
        .as_ref()
        .expect("configured session baseline should remain available");
    assert_eq!(live.settings.autoplay_initial_state, Some(true));
    assert_eq!(
        live.settings.autoplay_min_users,
        Some(sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(5))
    );
    assert_eq!(configured.settings.autoplay_initial_state, Some(false));
    assert_eq!(
        configured.settings.autoplay_min_users,
        Some(sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(2))
    );

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralCheckForUpdatesAutomatically,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect),
        "temporary live autoplay controls must not be compared as configured reconnect drift"
    );
    assert_eq!(
        owner
            .active_session_settings
            .as_ref()
            .and_then(|settings| settings.settings.autoplay_initial_state),
        Some(true),
        "ordinary saves must retain the live autoplay override"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reload_to_intentionally_unconfigured_player_clears_restart_requirement() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-reload-no-player", &initial);
    owner.player = None;
    owner.player_launch_state = GuiPlayerLaunchRuntimeState::None;
    owner.applied_player_launch_state = Some(GuiPlayerLaunchRuntimeState::None);
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];

    let reloaded = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    std::fs::remove_file(root.join("sorotte.ini"))
        .expect("no-player reload fixture should replace the original config");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&root.join("sorotte.ini"), &reloaded)
        .expect("no-player reload fixture should update the config");
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(initial),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }

    assert!(
        owner.current_player_launch_state_is_applied(),
        "reload launch state should be applied: launch={:?}, applied={:?}",
        owner.player_launch_state,
        owner.applied_player_launch_state,
    );
    assert_eq!(state.saved_configuration.player_path, None);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}
