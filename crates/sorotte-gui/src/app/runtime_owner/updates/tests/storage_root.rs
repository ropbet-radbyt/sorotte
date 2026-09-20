use super::*;
use crate::app::{
    GuiConfigStorageChangeTarget, GuiPendingCompletionRequest,
    testing::support::pump_and_apply_runtime_owner_actions,
};
use sorotte_client_app::app_boundary::{
    persistence::upsert_sorotte_ini_stored_client_settings_at_path,
    storage::{
        SOROTTE_CLIENT_INSTALL_ROOT_ENV, parse_sorotte_client_install_locator_config_root,
        sorotte_client_install_locator_path,
    },
};

use crate::app::runtime_owner::tests::CONFIG_ROOT_ENV_LOCK;

struct StorageEnvironment {
    _lock: std::sync::MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    root: tempfile::TempDir,
}

impl StorageEnvironment {
    fn new() -> Self {
        let lock = CONFIG_ROOT_ENV_LOCK
            .lock()
            .expect("storage environment lock");
        let root = tempfile::tempdir().expect("isolated fixture root");
        let keys = [
            "APPDATA",
            "HOME",
            "XDG_CONFIG_HOME",
            SOROTTE_CLIENT_INSTALL_ROOT_ENV,
        ];
        let saved = keys
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        let default_root = root.path().join("default");
        let install_root = root.path().join("install");
        std::fs::create_dir_all(&default_root).unwrap();
        std::fs::create_dir_all(&install_root).unwrap();
        // SAFETY: The shared runtime-owner lock serializes these variables with
        // the existing storage tests. Drop restores them before releasing it.
        unsafe {
            for key in ["APPDATA", "HOME", "XDG_CONFIG_HOME"] {
                std::env::set_var(key, &default_root);
            }
            std::env::set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, &install_root);
        }
        Self {
            _lock: lock,
            saved,
            root,
        }
    }
}

impl Drop for StorageEnvironment {
    fn drop(&mut self) {
        // SAFETY: The probe lock remains held until the original values are restored.
        unsafe {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

struct RootRecordingService {
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

impl GuiUpdateService for RootRecordingService {
    fn check_for_updates(&self, _: &str, _: bool, _: Option<&str>) -> UpdateCheckResult {
        check_result()
    }

    fn download_and_stage_update(
        &self,
        _: &UpdateCandidate,
        gui_config_root: Option<&Path>,
    ) -> UpdateDownloadResult {
        let root = gui_config_root.expect("runtime supplies a storage root");
        self.roots.lock().unwrap().push(root.to_path_buf());
        // Exercise the real destination handed to the service with a harmless
        // fixture file. Downloading, verification and installing are out of scope.
        let stage = root.join("updates").join("storage-regression");
        std::fs::create_dir_all(&stage).unwrap();
        let package = stage.join("fixture.txt");
        std::fs::write(&package, "fixture update bytes").unwrap();
        let mut staged = staged_update();
        staged.package_path = package.display().to_string();
        UpdateDownloadResult {
            state: UpdateDownloadState::Staged,
            message: "Fixture staged.".to_owned(),
            staged_update: Some(staged),
        }
    }

    fn launch_staged_update(&self, _: &StagedUpdate) -> UpdateApplyLaunchResult {
        UpdateApplyLaunchResult {
            success: true,
            message: "Simulated updater launch.".to_owned(),
        }
    }
}

fn stage_root_after_relocation(relocate: bool, restart: bool, install: bool) -> (PathBuf, PathBuf) {
    let env = StorageEnvironment::new();
    let old_root = env.root.path().join("old");
    let new_root = env.root.path().join("new");
    std::fs::create_dir_all(&old_root).unwrap();
    let old_path = old_root.join("sorotte.ini");
    let settings = StoredClientSettings {
        check_for_updates_automatically: Some(false),
        ..StoredClientSettings::default()
    };
    upsert_sorotte_ini_stored_client_settings_at_path(&old_path, &settings).unwrap();
    let roots = Arc::new(Mutex::new(Vec::new()));
    let service = Arc::new(RootRecordingService {
        roots: roots.clone(),
    });
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(old_path));
    // Replace only the external service; preserve the production constructor's root.
    owner.update_runtime.service = service.clone();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    if relocate {
        assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
            new_root.display().to_string()
        )));
        assert!(state.apply(GuiShellAction::BeginConfigurationSave));
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::ChangeConfigStorageRoot {
                target: GuiConfigStorageChangeTarget::CustomRoot(new_root.display().to_string()),
                baseline: Box::new(state.saved_configuration.clone()),
                settings: settings.clone(),
            },
        ));
        let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert!(actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigStorageRootChange { .. }
        )));
        assert_eq!(
            std::fs::canonicalize(owner.config_path.as_ref().unwrap()).unwrap(),
            std::fs::canonicalize(new_root.join("sorotte.ini")).unwrap()
        );
        let install_root = env.root.path().join("install");
        let locator =
            std::fs::read_to_string(sorotte_client_install_locator_path(&install_root)).unwrap();
        let loaded_root =
            parse_sorotte_client_install_locator_config_root(&locator, &install_root).unwrap();
        assert_eq!(
            std::fs::canonicalize(loaded_root).unwrap(),
            std::fs::canonicalize(&new_root).unwrap()
        );
        if restart {
            owner = GuiPersistedConfigRuntimeOwner::with_config_path(owner.config_path.clone());
            owner.update_runtime.service = service;
        }
    }
    // Use the same check-result reducer and Begin action as ordinary update UI.
    assert!(state.apply(GuiShellAction::ApplyUpdateCheckResult(check_result())));
    assert!(state.apply(GuiShellAction::BeginUpdateDownload));
    handle.push_request(if install {
        GuiRuntimeRequest::DownloadAndInstallUpdate(candidate())
    } else {
        GuiRuntimeRequest::DownloadUpdate(candidate())
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut completed = false;
    let mut launched = !install;
    while !completed || !launched {
        let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        completed |= actions.iter().any(|action| matches!(action,
            GuiShellAction::ApplyUpdateDownloadResult(result) if result.state == UpdateDownloadState::Staged
        ));
        launched |= actions.iter().any(|action| {
            matches!(action,
                GuiShellAction::ApplyStagedUpdateLaunchResult(result) if result.success
            )
        });
        assert!(Instant::now() < deadline, "fixture update job completes");
        std::thread::yield_now();
    }
    let observed = roots.lock().unwrap().clone();
    assert_eq!(observed.len(), 1);
    let actual = std::fs::canonicalize(&observed[0]).unwrap();
    let expected = std::fs::canonicalize(if relocate { &new_root } else { &old_root }).unwrap();
    assert!(
        actual
            .join("updates/storage-regression/fixture.txt")
            .is_file()
    );
    // A follow-on interactive check remains usable in this same owner.
    assert!(state.apply(GuiShellAction::BeginUpdateCheck {
        user_initiated: true
    }));
    handle.push_request(GuiRuntimeRequest::CheckForUpdates {
        language: "en".to_owned(),
        update_channel: None,
        user_initiated: true,
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::ApplyUpdateCheckResult(_)))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "next interactive check completes"
        );
        std::thread::yield_now();
    }
    println!(
        "relocate={relocate} restart={restart} install={install}; expected={}; actual={}; stage file exists; follow-on check completed",
        expected.display(),
        actual.display()
    );
    // Drop the owner before the temporary storage fixture is removed.
    drop(owner);
    (expected, actual)
}

#[test]
fn download_after_storage_move_uses_current_root() {
    let (expected, actual) = stage_root_after_relocation(true, false, false);
    assert_eq!(
        actual, expected,
        "download staging must follow the active storage root"
    );
}

#[test]
fn install_after_storage_move_uses_current_root() {
    let (expected, actual) = stage_root_after_relocation(true, false, true);
    assert_eq!(
        actual, expected,
        "install download must follow the active storage root"
    );
}

#[test]
fn update_without_storage_move_control() {
    let (expected, actual) = stage_root_after_relocation(false, false, false);
    assert_eq!(actual, expected);
}

#[test]
fn update_after_storage_move_and_restart_control() {
    let (expected, actual) = stage_root_after_relocation(true, true, false);
    assert_eq!(actual, expected);
}

#[test]
fn storage_move_preserves_owned_download_and_redirects_the_next_download() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.download_gate = Some(gate.clone());
    let mut runtime =
        GuiUpdateRuntime::with_service(Some(PathBuf::from("original")), Arc::new(service));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.handle_command(&handle, Command::Download(candidate()));
    gate.wait_until_entered();
    runtime.set_config_root(Some(PathBuf::from("relocated")));
    gate.release();
    assert!(
        matches!(pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(result)] if result.state == UpdateDownloadState::Staged)
    );
    runtime.handle_command(&handle, Command::Download(candidate()));
    assert!(
        matches!(pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(result)] if result.state == UpdateDownloadState::Staged)
    );
    assert_eq!(
        *calls.lock().unwrap(),
        ["download:9.8.7:original", "download:9.8.7:relocated"]
    );
}
