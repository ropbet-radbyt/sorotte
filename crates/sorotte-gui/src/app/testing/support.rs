use super::super::{
    GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner,
    GuiShellAction, MainWindowRuntimeRoomSnapshot, MainWindowRuntimeUserSnapshot,
    SorotteGuiShellAppState,
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

pub(crate) fn test_default_sorotte_config_env_root() -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from("test-appdata-root")
    } else {
        std::path::PathBuf::from("test-home-root")
    }
}

pub(crate) fn test_default_sorotte_config_root() -> std::path::PathBuf {
    if cfg!(windows) {
        test_default_sorotte_config_env_root().join("Sorotte")
    } else if cfg!(target_os = "macos") {
        test_default_sorotte_config_env_root()
            .join("Library")
            .join("Application Support")
            .join("Sorotte")
    } else {
        test_default_sorotte_config_env_root()
            .join(".config")
            .join("sorotte")
    }
}

pub(crate) fn test_default_sorotte_config_target() -> std::path::PathBuf {
    test_default_sorotte_config_root().join("sorotte.ini")
}

pub(crate) fn test_temp_root(label: &str) -> std::path::PathBuf {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sorotte-gui-{label}-{}-{unique_suffix}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("test temp root should be created");
    normalize_test_temp_root(root)
}

#[cfg(windows)]
fn normalize_test_temp_root(root: std::path::PathBuf) -> std::path::PathBuf {
    let Ok(canonical_root) = std::fs::canonicalize(&root) else {
        return root;
    };
    let normalized_root = {
        let mut components = canonical_root.components();
        match components.next() {
            Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
                std::path::Prefix::VerbatimDisk(disk) => {
                    let mut normalized_root =
                        std::path::PathBuf::from(format!("{}:\\", char::from(disk)));
                    normalized_root.extend(components);
                    Some(normalized_root)
                }
                std::path::Prefix::VerbatimUNC(server, share) => {
                    let mut normalized_root = std::path::PathBuf::from(format!(
                        "\\\\{}\\{}",
                        server.to_string_lossy(),
                        share.to_string_lossy()
                    ));
                    normalized_root.extend(components);
                    Some(normalized_root)
                }
                _ => None,
            },
            _ => None,
        }
    };
    normalized_root.unwrap_or(canonical_root)
}

#[cfg(not(windows))]
fn normalize_test_temp_root(root: std::path::PathBuf) -> std::path::PathBuf {
    root
}

pub(crate) fn pump_and_apply_runtime_owner_actions(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
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
    state: &mut SorotteGuiShellAppState,
    timeout: std::time::Duration,
    predicate: P,
    context: &str,
) -> Vec<GuiShellAction>
where
    P: Fn(&SorotteGuiShellAppState) -> bool,
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
