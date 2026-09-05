use super::*;
use sorotte_client_app::app_boundary::persistence::{
    clear_sorotte_ini_stored_client_settings_mvp_at_path,
    edit_sorotte_ini_stored_client_settings_mvp_at_path,
};

#[test]
fn gui_stale_full_save_keeps_independent_changes_and_never_restores_cleared_credentials() {
    for clear_file in [false, true] {
        let root = test_temp_root("concurrent-settings-full-save");
        let config_path = root.join("sorotte.ini");
        let baseline = StoredClientSettingsMvp {
            host: Some("localhost".into()),
            username: Some("before".into()),
            room: Some("before-room".into()),
            server_password: Some("synthetic-password".into()),
            plex_user_token: Some("synthetic-token".into()),
            ..Default::default()
        };
        upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &baseline).unwrap();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&baseline);
        if clear_file {
            clear_sorotte_ini_stored_client_settings_mvp_at_path(&config_path).unwrap();
        } else {
            edit_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, |settings| {
                settings.room = Some("independent-room".into());
                settings.server_password = None;
                settings.plex_user_token = None;
            })
            .unwrap();
        }
        let mut desired = baseline;
        desired.username = Some("edited-name".into());
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::ConnectionUsername,
            value: "edited-name".to_owned().into(),
        }));
        assert!(state.apply(GuiShellAction::BeginConfigurationSave));
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SaveConfiguration(desired),
        ));
        let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, GuiShellAction::CompleteConfigurationSave(_)))
        );
        let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
            .unwrap()
            .unwrap();
        assert_eq!(disk.username.as_deref(), Some("edited-name"));
        assert_eq!(
            disk.room.as_deref(),
            (!clear_file).then_some("independent-room")
        );
        assert!(disk.server_password.is_none());
        assert!(disk.plex_user_token.is_none());
        assert_eq!(state.saved_configuration, disk);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn gui_feature_patch_uses_current_disk_state_after_an_independent_credential_clear() {
    let root = test_temp_root("concurrent-settings-feature-patch");
    let config_path = root.join("sorotte.ini");
    let baseline = StoredClientSettingsMvp {
        room: Some("old-room".into()),
        plex_user_token: Some("synthetic-token".into()),
        plex_streaming_enabled: Some(false),
        ..Default::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &baseline).unwrap();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&baseline);
    edit_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, |settings| {
        settings.room = Some("independent-room".into());
        settings.plex_user_token = None;
    })
    .unwrap();
    assert!(owner.handle_toggle_plex_streaming_request(&handle, &mut state, true));
    let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .unwrap()
        .unwrap();
    assert_eq!(disk.room.as_deref(), Some("independent-room"));
    assert_eq!(disk.plex_streaming_enabled, Some(true));
    assert!(disk.plex_user_token.is_none());
    let _ = std::fs::remove_dir_all(root);
}
