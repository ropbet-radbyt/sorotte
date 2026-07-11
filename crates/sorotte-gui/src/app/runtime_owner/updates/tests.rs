use std::sync::Mutex;

use super::*;
use crate::app::{
    feature_slices::updates,
    remote_services::{LegacyUpdateCheckStatus, UpdateCandidateSource, UpdateChannel},
};

#[derive(Clone)]
struct FakeUpdateService {
    calls: Arc<Mutex<Vec<String>>>,
    check_result: LegacyUpdateCheckResult,
    download_result: UpdateDownloadResult,
    launch_result: UpdateApplyLaunchResult,
}

impl GuiUpdateService for FakeUpdateService {
    fn check_for_updates(
        &self,
        language: &str,
        user_initiated: bool,
        update_channel: Option<&str>,
    ) -> LegacyUpdateCheckResult {
        self.calls
            .lock()
            .expect("fake update calls should remain available")
            .push(format!(
                "check:{language}:{user_initiated}:{}",
                update_channel.unwrap_or_default()
            ));
        self.check_result.clone()
    }

    fn download_and_stage_update(
        &self,
        candidate: &UpdateCandidate,
        gui_config_root: Option<&Path>,
    ) -> UpdateDownloadResult {
        self.calls
            .lock()
            .expect("fake update calls should remain available")
            .push(format!(
                "download:{}:{}",
                candidate.version,
                gui_config_root
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ));
        self.download_result.clone()
    }

    fn launch_staged_update(&self, staged_update: &StagedUpdate) -> UpdateApplyLaunchResult {
        self.calls
            .lock()
            .expect("fake update calls should remain available")
            .push(format!("launch:{}", staged_update.candidate.version));
        self.launch_result.clone()
    }
}

fn candidate() -> UpdateCandidate {
    UpdateCandidate {
        channel: UpdateChannel::Stable,
        version: "9.8.7".to_owned(),
        git_sha: None,
        created_at_utc: "2026-07-11T00:00:00Z".to_owned(),
        target: "x86_64-pc-windows-msvc".to_owned(),
        package: "sorotte.zip".to_owned(),
        sha256: "abc123".to_owned(),
        download_url: "https://example.invalid/sorotte.zip".to_owned(),
        details_url: Some("https://example.invalid/release".to_owned()),
        source: UpdateCandidateSource::ReleaseAsset,
    }
}

fn staged_update() -> StagedUpdate {
    StagedUpdate {
        candidate: candidate(),
        package_path: "C:/updates/sorotte.zip".to_owned(),
        source_dir: "C:/updates/source".to_owned(),
        updater_path: "C:/updates/updater.exe".to_owned(),
        target_exe_path: "C:/Sorotte/sorotte-gui.exe".to_owned(),
        backup_dir: "C:/updates/backup".to_owned(),
        log_path: "C:/updates/update.log".to_owned(),
        restart: true,
    }
}

fn check_result() -> LegacyUpdateCheckResult {
    LegacyUpdateCheckResult {
        status: LegacyUpdateCheckStatus::UpdateAvailable,
        message: "An update is available.".to_owned(),
        url: Some("https://example.invalid/release".to_owned()),
        candidate: Some(candidate()),
        self_update_supported: true,
        public_servers: None,
        checked_at_utc: "2026-07-11T00:00:00Z".to_owned(),
        user_initiated: true,
    }
}

fn fake_runtime() -> (GuiUpdateRuntime, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = FakeUpdateService {
        calls: calls.clone(),
        check_result: check_result(),
        download_result: UpdateDownloadResult {
            state: UpdateDownloadState::Staged,
            message: "Update staged.".to_owned(),
            staged_update: Some(staged_update()),
        },
        launch_result: UpdateApplyLaunchResult {
            success: true,
            message: "Updater launched.".to_owned(),
        },
    };
    (
        GuiUpdateRuntime::with_service(Some(PathBuf::from("C:/config")), Arc::new(service)),
        calls,
    )
}

#[test]
fn typed_update_commands_emit_ordered_actions_without_shell_state() {
    let (mut runtime, calls) = fake_runtime();
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    runtime.handle_command(
        &handle,
        Command::CheckForUpdates {
            language: "fr".to_owned(),
            update_channel: Some("dev".to_owned()),
            user_initiated: true,
        },
    );
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::ApplyUpdateCheckResult(check_result())]
    );

    runtime.handle_command(&handle, Command::Download(candidate()));
    assert_eq!(handle.drain_actions().len(), 1);

    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    let install_actions = handle.drain_actions();
    assert!(matches!(
        install_actions.as_slice(),
        [
            GuiShellAction::ApplyUpdateDownloadResult(_),
            GuiShellAction::BeginStagedUpdateApply,
            GuiShellAction::ApplyStagedUpdateLaunchResult(_)
        ]
    ));

    runtime.handle_command(&handle, Command::ApplyStaged(staged_update()));
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::ApplyStagedUpdateLaunchResult(_)]
    ));

    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec![
            "check:fr:true:dev".to_owned(),
            "download:9.8.7:C:/config".to_owned(),
            "download:9.8.7:C:/config".to_owned(),
            "launch:9.8.7".to_owned(),
            "launch:9.8.7".to_owned(),
        ]
    );
}

#[test]
fn background_check_uses_narrow_policy_and_schedules_only_once() {
    let (mut runtime, calls) = fake_runtime();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&updates::RuntimeView {
        model: GuiUpdateCheckState::default(),
        policy: updates::RuntimePolicy {
            automatic: true,
            last_checked_for_updates: None,
            language: "fr".to_owned(),
            channel: Some("dev".to_owned()),
        },
    });

    runtime.pump_background_check(&handle, false);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::BeginUpdateCheck {
            user_initiated: false,
        }]
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let result_actions = loop {
        runtime.pump_background_check(&handle, false);
        let actions = handle.drain_actions();
        if !actions.is_empty() {
            break actions;
        }
        assert!(
            Instant::now() < deadline,
            "fake background update check should complete"
        );
        std::thread::yield_now();
    };
    assert!(matches!(
        result_actions.as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(_)]
    ));

    runtime.pump_background_check(&handle, false);
    assert!(handle.drain_actions().is_empty());
    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec!["check:fr:false:dev".to_owned()]
    );
}

#[test]
fn startup_remote_check_blocks_background_update_work() {
    let (mut runtime, calls) = fake_runtime();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&updates::RuntimeView {
        model: GuiUpdateCheckState::default(),
        policy: updates::RuntimePolicy {
            automatic: true,
            last_checked_for_updates: None,
            language: "en".to_owned(),
            channel: None,
        },
    });

    runtime.pump_background_check(&handle, true);

    assert!(handle.drain_actions().is_empty());
    assert!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .is_empty()
    );
}
