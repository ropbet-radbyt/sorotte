use super::*;
use crate::app::local_command_dispatch::GuiShellDispatchPlan;
use crate::app::runtime_queue::GuiQueuedRuntimeOwnerPump;

fn settle_until(
    pump: &mut dyn GuiNativeRuntimePump,
    bridge: &mut GuiQueuedRuntimeBridge,
    state: &mut SorotteGuiShellAppState,
    condition: impl Fn(&SorotteGuiShellAppState) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        pump.pump(state);
        for action in bridge.drain_runtime_actions() {
            // Native output draining permits harmless no-op projections. The
            // completion and persisted-state assertions below are the oracle.
            let _ = state.apply(action);
        }
        if condition(state) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "runtime output timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

enum SaveAction {
    Save,
    Move,
    SaveAndConnect,
}

fn save_after_plugin_toggle_case(
    consume_toggle_before_save: bool,
    threaded: bool,
    action: SaveAction,
) {
    let env = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    struct RestoreInstallRoot(Option<std::ffi::OsString>);
    impl Drop for RestoreInstallRoot {
        fn drop(&mut self) {
            // SAFETY: The shared config-root lock outlives this restore guard.
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, value),
                    None => std::env::remove_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV),
                }
            }
        }
    }
    let _restore = RestoreInstallRoot(std::env::var_os(SOROTTE_CLIENT_INSTALL_ROOT_ENV));
    let root = crate::app::testing::support::test_temp_dir("save-after-plugin-toggle");
    let install_root = root.path().join("install");
    std::fs::create_dir(&install_root).unwrap();
    env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, &install_root);
    let mut path = root.path().join("sorotte.ini");
    // An owned loopback endpoint keeps Save & connect independent of external
    // servers. The transport is shut down before the listener leaves scope.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let saved = StoredClientSettings {
        host: Some("127.0.0.1".to_owned()),
        port: Some(listener.local_addr().unwrap().port()),
        username: Some("before".to_owned()),
        player_path: Some("vlc".to_owned()),
        plex_plugin_enabled: Some(true),
        public_servers: Some(Vec::new()),
        ..Default::default()
    };
    upsert_sorotte_ini_stored_client_settings_at_path(&path, &saved).unwrap();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    owner.startup_saved_connect_attempted = true;
    owner.startup_remote_actions_attempted = true;
    owner.startup_public_server_hydration.completed = true;
    owner.startup_stream_helper_probe_completed = true;
    let (mut bridge, handle) = GuiQueuedRuntimeBridge::new();
    let mut pump: Box<dyn GuiNativeRuntimePump> = if threaded {
        Box::new(GuiThreadedRuntimeOwnerPump::new(handle.clone(), owner).unwrap())
    } else {
        Box::new(GuiQueuedRuntimeOwnerPump::new(handle.clone(), owner))
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionUsername,
        value: "after".to_owned().into(),
    }));
    // Use the same action-to-request planner as the native host. The toggle
    // has no eager shell effect and does not mark a global pending operation.
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::SetPluginEnabled {
            plugin: GuiPluginSelection::Plex,
            enabled: false,
        }],
    );
    assert!(plan.shell_actions.is_empty());
    assert_eq!(plan.pre_shell_runtime_requests.len(), 1);
    for request in plan.pre_shell_runtime_requests {
        assert!(bridge.dispatch_runtime_request(&state, request).is_empty());
    }
    assert!(state.pending_operation.is_none());
    if consume_toggle_before_save {
        settle_until(pump.as_mut(), &mut bridge, &mut state, |state| {
            state.saved_configuration.plex_plugin_enabled == Some(false)
        });
        assert_eq!(state.saved_configuration.plex_plugin_enabled, Some(false));
    }

    assert!(state.commands.can_save_configuration);
    if matches!(action, SaveAction::Move) {
        let new_root = root.path().join("relocated");
        assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
            new_root.display().to_string()
        )));
        path = new_root.join("sorotte.ini");
    }
    assert!(
        state.apply(if matches!(action, SaveAction::SaveAndConnect) {
            GuiShellAction::BeginSaveAndConnect
        } else {
            GuiShellAction::BeginConfigurationSave
        })
    );
    // Capture the submitted settings through the real pending-completion bridge.
    assert!(bridge.actions_for_pending_completion(&state).is_empty());
    settle_until(pump.as_mut(), &mut bridge, &mut state, |state| {
        state.pending_operation.is_none()
    });
    let persisted = load_sorotte_ini_stored_client_settings_from_path(&path)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.username.as_deref(), Some("after"));
    assert!(state.pending_operation.is_none());
    assert_eq!(state.saved_configuration, persisted);
    let observed_plugin_enabled = persisted.plex_plugin_enabled;
    eprintln!(
        "threaded={threaded}, consume_toggle_before_save={consume_toggle_before_save}: saved username={:?}, Plex plugin={observed_plugin_enabled:?}",
        persisted.username
    );

    // A subsequent ordinary toggle still works; this is a lost requested
    // change, not a permanently broken persistence fixture.
    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::Plex,
        enabled: false,
    });
    settle_until(pump.as_mut(), &mut bridge, &mut state, |state| {
        state.saved_configuration.plex_plugin_enabled == Some(false)
    });
    assert_eq!(
        load_sorotte_ini_stored_client_settings_from_path(&path)
            .unwrap()
            .unwrap()
            .plex_plugin_enabled,
        Some(false)
    );
    assert_eq!(state.saved_configuration.plex_plugin_enabled, Some(false));
    pump.shutdown();
    assert_eq!(
        observed_plugin_enabled,
        Some(false),
        "Saving an unrelated draft must retain the earlier queued explicit plugin disable"
    );
}

#[test]
fn queued_save_retains_prior_plugin_toggle() {
    save_after_plugin_toggle_case(false, false, SaveAction::Save);
}

#[test]
fn save_after_toggle_ack_control() {
    save_after_plugin_toggle_case(true, false, SaveAction::Save);
}

#[test]
fn threaded_queued_save_retains_prior_plugin_toggle() {
    save_after_plugin_toggle_case(false, true, SaveAction::Save);
}

#[test]
fn threaded_save_after_toggle_ack_control() {
    save_after_plugin_toggle_case(true, true, SaveAction::Save);
}

#[test]
fn storage_move_retains_prior_queued_plugin_toggle() {
    save_after_plugin_toggle_case(false, false, SaveAction::Move);
}

#[test]
fn threaded_storage_move_retains_prior_queued_plugin_toggle() {
    save_after_plugin_toggle_case(false, true, SaveAction::Move);
}

#[test]
fn save_and_connect_retains_prior_queued_plugin_toggle() {
    save_after_plugin_toggle_case(false, false, SaveAction::SaveAndConnect);
}

#[test]
fn threaded_save_and_connect_retains_prior_queued_plugin_toggle() {
    save_after_plugin_toggle_case(false, true, SaveAction::SaveAndConnect);
}
