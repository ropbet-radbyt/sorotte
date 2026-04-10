use super::super::{
    GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner,
    GuiShellAction, MainWindowRuntimeRoomSnapshot, MainWindowRuntimeUserSnapshot,
    SyncplayGuiShellAppState,
};

pub(crate) const TEST_USERNAME: &str = "test-user";

pub(crate) fn browser_runtime_user(
    username: &str,
    room_name: &str,
    is_self: bool,
    is_ready: bool,
    is_controller: bool,
) -> MainWindowRuntimeUserSnapshot {
    MainWindowRuntimeUserSnapshot {
        username: username.to_owned(),
        room_name: room_name.to_owned(),
        is_self,
        is_ready,
        is_controller,
        file_is_trusted: true,
        ..Default::default()
    }
}

pub(crate) fn browser_runtime_rooms(
    room_name: &str,
    is_controlled: bool,
    has_named_users: bool,
) -> Vec<MainWindowRuntimeRoomSnapshot> {
    vec![MainWindowRuntimeRoomSnapshot {
        room_name: room_name.to_owned(),
        is_controlled,
        has_named_users,
    }]
}

pub(crate) fn test_default_syncplay_config_env_root() -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from("test-appdata-root")
    } else {
        std::path::PathBuf::from("test-home-root")
    }
}

pub(crate) fn test_default_syncplay_config_root() -> std::path::PathBuf {
    if cfg!(windows) {
        test_default_syncplay_config_env_root()
    } else {
        test_default_syncplay_config_env_root().join(".config")
    }
}

pub(crate) fn test_default_syncplay_config_target() -> std::path::PathBuf {
    test_default_syncplay_config_root().join("syncplay.ini")
}

pub(crate) fn test_temp_root(label: &str) -> std::path::PathBuf {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-{label}-{}-{unique_suffix}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("test temp root should be created");
    root
}

pub(crate) fn pump_and_apply_runtime_owner_actions(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Vec<GuiShellAction> {
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    let actions = handle.drain_actions();
    for action in &actions {
        if !state.apply(action.clone()) {
            panic!("state.apply({action:?}) failed with state {state:?}",);
        }
    }
    actions
}

pub(crate) fn pump_and_apply_runtime_owner_actions_until<P>(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    timeout: std::time::Duration,
    predicate: P,
    context: &str,
) -> Vec<GuiShellAction>
where
    P: Fn(&SyncplayGuiShellAppState) -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    let mut all_actions = Vec::new();
    loop {
        let actions = pump_and_apply_runtime_owner_actions(owner, handle, state);
        all_actions.extend(actions);
        if predicate(state) {
            return all_actions;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {context}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
