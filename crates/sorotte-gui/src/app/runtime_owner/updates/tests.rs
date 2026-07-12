use std::sync::{Condvar, Mutex};

use super::*;
use crate::app::{
    feature_slices::updates,
    remote_services::{LegacyUpdateCheckStatus, UpdateCandidateSource, UpdateChannel},
    runtime_bridge::{GuiQueuedRuntimeOwner, GuiRuntimeRequest},
    runtime_owner::GuiPersistedConfigRuntimeOwner,
    shell_state::SorotteGuiShellAppState,
};
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

#[derive(Default)]
struct BlockGate {
    entered: Mutex<bool>,
    released: Mutex<bool>,
    entered_cv: Condvar,
    released_cv: Condvar,
}

impl BlockGate {
    fn block(&self) {
        *self.entered.lock().expect("gate should remain available") = true;
        self.entered_cv.notify_all();
        let mut released = self.released.lock().expect("gate should remain available");
        while !*released {
            released = self
                .released_cv
                .wait(released)
                .expect("gate should remain available");
        }
    }

    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut entered = self.entered.lock().expect("gate should remain available");
        while !*entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "worker should enter the blocking service"
            );
            let (next, timeout) = self
                .entered_cv
                .wait_timeout(entered, remaining)
                .expect("gate should remain available");
            entered = next;
            assert!(
                !timeout.timed_out() || *entered,
                "worker should enter the blocking service"
            );
        }
    }

    fn release(&self) {
        *self.released.lock().expect("gate should remain available") = true;
        self.released_cv.notify_all();
    }
}

#[derive(Clone)]
struct FakeUpdateService {
    calls: Arc<Mutex<Vec<String>>>,
    check_result: LegacyUpdateCheckResult,
    download_result: UpdateDownloadResult,
    launch_result: UpdateApplyLaunchResult,
    blocked_check_language: Option<String>,
    check_gate: Option<Arc<BlockGate>>,
    download_gate: Option<Arc<BlockGate>>,
    launch_gate: Option<Arc<BlockGate>>,
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
        if self.blocked_check_language.as_deref() == Some(language)
            && let Some(gate) = self.check_gate.as_ref()
        {
            gate.block();
        }
        let mut result = self.check_result.clone();
        result.message = format!("result:{language}");
        result.user_initiated = user_initiated;
        result
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
        if let Some(gate) = self.download_gate.as_ref() {
            gate.block();
        }
        self.download_result.clone()
    }

    fn launch_staged_update(&self, staged_update: &StagedUpdate) -> UpdateApplyLaunchResult {
        if let Some(gate) = self.launch_gate.as_ref() {
            gate.block();
        }
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
        checked_at_utc: "1970-01-01 00:00:00.000".to_owned(),
        user_initiated: true,
    }
}

fn fake_service(calls: Arc<Mutex<Vec<String>>>) -> FakeUpdateService {
    FakeUpdateService {
        calls,
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
        blocked_check_language: None,
        check_gate: None,
        download_gate: None,
        launch_gate: None,
    }
}

fn fake_runtime() -> (GuiUpdateRuntime, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        GuiUpdateRuntime::with_service(
            Some(PathBuf::from("C:/config")),
            Arc::new(fake_service(calls.clone())),
        ),
        calls,
    )
}

fn runtime_with_service(service: FakeUpdateService) -> GuiUpdateRuntime {
    GuiUpdateRuntime::with_service(Some(PathBuf::from("C:/config")), Arc::new(service))
}

fn pump_until_actions(
    runtime: &mut GuiUpdateRuntime,
    handle: &GuiQueuedRuntimeBridgeHandle,
) -> Vec<GuiShellAction> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        runtime.pump_background_check(handle);
        let actions = handle.drain_actions();
        if !actions.is_empty() {
            return actions;
        }
        assert!(Instant::now() < deadline, "update worker should complete");
        std::thread::yield_now();
    }
}

fn update_view(automatic: bool, language: &str, channel: Option<&str>) -> updates::RuntimeView {
    updates::RuntimeView {
        model: GuiUpdateCheckState::default(),
        policy: updates::RuntimePolicy {
            automatic,
            last_checked_for_updates: None,
            language: language.to_owned(),
            channel: channel.map(str::to_owned),
        },
    }
}

#[test]
fn typed_update_commands_run_asynchronously_and_emit_ordered_actions() {
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
    assert!(handle.drain_actions().is_empty());
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(result)] if result.message == "result:fr"
    ));

    runtime.handle_command(&handle, Command::Download(candidate()));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(_)]
    ));

    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [
            GuiShellAction::ApplyUpdateDownloadResult(_),
            GuiShellAction::BeginStagedUpdateApply
        ]
    ));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyStagedUpdateLaunchResult(_)]
    ));

    runtime.handle_command(&handle, Command::ApplyStaged(staged_update()));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
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
fn blocked_service_does_not_block_runtime_owner_command_or_pump() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls);
    service.blocked_check_language = Some("blocked".to_owned());
    service.check_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let watchdog_gate = gate.clone();
    let (watchdog_done_tx, watchdog_done_rx) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if watchdog_done_rx
            .recv_timeout(Duration::from_secs(2))
            .is_err()
        {
            watchdog_gate.release();
        }
    });

    let started = Instant::now();
    runtime.handle_command(
        &handle,
        Command::CheckForUpdates {
            language: "blocked".to_owned(),
            update_channel: None,
            user_initiated: true,
        },
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "starting an update must not block the runtime-owner loop"
    );
    gate.wait_until_entered();
    let pump_started = Instant::now();
    runtime.pump_background_check(&handle);
    assert!(
        pump_started.elapsed() < Duration::from_secs(1),
        "polling a blocked update must leave transport/session pumping live"
    );

    gate.release();
    let _ = watchdog_done_tx.send(());
    watchdog.join().expect("watchdog should finish");
    let _ = pump_until_actions(&mut runtime, &handle);
}

#[test]
fn blocked_update_job_leaves_session_transport_pumping_live() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls);
    service.blocked_check_language = Some("blocked".to_owned());
    service.check_gate = Some(gate.clone());
    let runtime = runtime_with_service(service);
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core session runtime should bootstrap");
    owner.update_runtime = runtime;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let watchdog_gate = gate.clone();
    let (watchdog_done_tx, watchdog_done_rx) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if watchdog_done_rx
            .recv_timeout(Duration::from_secs(2))
            .is_err()
        {
            watchdog_gate.release();
        }
    });

    handle.push_request(GuiRuntimeRequest::CheckForUpdates {
        language: "blocked".to_owned(),
        update_channel: None,
        user_initiated: true,
    });
    let started = Instant::now();
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the complete owner pump must not wait for the update service"
    );
    gate.wait_until_entered();
    assert!(
        session_transport
            .drain_outbound_protocol_lines()
            .iter()
            .any(|line| line.contains("\"Hello\"")),
        "the owner must flush the startup Hello after scheduling update work"
    );

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
    );
    let generation_before_inbound = owner.runtime_pump_generation;
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        owner.runtime_pump_generation,
        generation_before_inbound.wrapping_add(1)
    );
    assert!(
        owner
            .session
            .as_ref()
            .is_some_and(|session| session.server_handshake_completed()),
        "a server Hello must be processed while the update service remains blocked"
    );

    gate.release();
    let _ = watchdog_done_tx.send(());
    watchdog.join().expect("watchdog should finish");
    let _ = pump_until_actions(&mut owner.update_runtime, &handle);
}

#[test]
fn background_check_uses_narrow_policy_and_schedules_only_once() {
    let (mut runtime, calls) = fake_runtime();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(true, "fr", Some("dev")));

    runtime.pump_background_check(&handle);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::BeginUpdateCheck {
            user_initiated: false,
        }]
    );
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(result)] if result.message == "result:fr"
    ));

    runtime.pump_background_check(&handle);
    assert!(handle.drain_actions().is_empty());
    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec!["check:fr:false:dev".to_owned()]
    );
}

#[test]
fn runtime_owner_routes_startup_check_through_update_coordinator() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_service(fake_service(calls.clone()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.update_runtime = runtime;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        language: Some("fr".to_owned()),
        update_channel: Some("dev".to_owned()),
        last_checked_for_updates: None,
        ..StoredClientSettingsMvp::default()
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut actions = Vec::new();
    while !actions
        .iter()
        .any(|action| matches!(action, GuiShellAction::ApplyUpdateCheckResult(_)))
    {
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        actions.extend(handle.drain_actions());
        assert!(
            Instant::now() < deadline,
            "startup update coordinator should return a result"
        );
        std::thread::yield_now();
    }

    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::BeginUpdateCheck {
            user_initiated: false
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyUpdateCheckResult(result)
            if result.message == "result:fr" && !result.user_initiated
    )));
    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec!["check:fr:false:dev".to_owned()]
    );
}

#[test]
fn startup_due_update_and_empty_public_server_cache_complete_independently() {
    assert_startup_remote_jobs_complete_independently(true);
    assert_startup_remote_jobs_complete_independently(false);
}

fn assert_startup_remote_jobs_complete_independently(update_completes_first: bool) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let update_gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.blocked_check_language = Some("fr".to_owned());
    service.check_gate = Some(update_gate.clone());

    let public_server_gate = Arc::new(BlockGate::default());
    let public_server_calls = Arc::new(Mutex::new(Vec::new()));
    let fetch_gate = public_server_gate.clone();
    let fetch_calls = public_server_calls.clone();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.update_runtime = runtime_with_service(service);
    owner
        .update_runtime
        .reconcile(&update_view(true, "fr", Some("dev")));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        language: Some("fr".to_owned()),
        update_channel: Some("dev".to_owned()),
        last_checked_for_updates: None,
        public_servers: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    };
    let mut projected_state = SorotteGuiShellAppState::from_stored_settings(&settings);
    let mut observed_state = SorotteGuiShellAppState::from_stored_settings(&settings);
    let mut observed_actions = Vec::new();

    owner.run_deferred_startup_remote_actions_with_fetcher(
        &handle,
        &mut projected_state,
        move |language| {
            fetch_calls
                .lock()
                .expect("public-server fetch calls should remain available")
                .push(language.to_owned());
            fetch_gate.block();
            Ok(vec![(
                "Hydrated Primary".to_owned(),
                "hydrated.example:8999".to_owned(),
            )])
        },
    );

    update_gate.wait_until_entered();
    public_server_gate.wait_until_entered();
    apply_startup_remote_actions(&handle, &mut observed_state, &mut observed_actions);
    assert!(owner.update_runtime.active_job.is_some());
    assert!(owner.startup_remote_actions_rx.is_some());
    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec!["check:fr:false:dev".to_owned()]
    );
    assert_eq!(
        *public_server_calls
            .lock()
            .expect("public-server fetch calls should remain available"),
        vec!["fr".to_owned()]
    );

    if update_completes_first {
        update_gate.release();
        pump_startup_remote_jobs_until(
            &mut owner,
            &handle,
            &mut projected_state,
            &mut observed_state,
            &mut observed_actions,
            |action| matches!(action, GuiShellAction::ApplyUpdateCheckResult(_)),
        );
        assert!(observed_state.public_servers.servers.is_empty());

        public_server_gate.release();
        pump_startup_remote_jobs_until(
            &mut owner,
            &handle,
            &mut projected_state,
            &mut observed_state,
            &mut observed_actions,
            |action| matches!(action, GuiShellAction::ApplyStartupPublicServerCache(_)),
        );
    } else {
        public_server_gate.release();
        pump_startup_remote_jobs_until(
            &mut owner,
            &handle,
            &mut projected_state,
            &mut observed_state,
            &mut observed_actions,
            |action| matches!(action, GuiShellAction::ApplyStartupPublicServerCache(_)),
        );
        assert!(matches!(
            owner.update_runtime.active_job.as_ref(),
            Some(active) if matches!(
                active.kind,
                UpdateJobKind::Check {
                    origin: UpdateJobOrigin::Startup,
                    user_initiated: false,
                }
            )
        ));

        update_gate.release();
        pump_startup_remote_jobs_until(
            &mut owner,
            &handle,
            &mut projected_state,
            &mut observed_state,
            &mut observed_actions,
            |action| matches!(action, GuiShellAction::ApplyUpdateCheckResult(_)),
        );
    }

    assert_eq!(observed_state.public_servers.servers.len(), 1);
    assert_eq!(
        observed_state.public_servers.servers[0].address,
        "hydrated.example:8999"
    );
    assert_eq!(
        observed_state.update_check.message.as_deref(),
        Some("result:fr")
    );
    assert_eq!(
        observed_actions
            .iter()
            .filter(|action| matches!(action, GuiShellAction::ApplyUpdateCheckResult(_)))
            .count(),
        1
    );
    assert_eq!(
        observed_actions
            .iter()
            .filter(|action| matches!(action, GuiShellAction::ApplyStartupPublicServerCache(_)))
            .count(),
        1
    );
}

fn pump_startup_remote_jobs_until(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    projected_state: &mut SorotteGuiShellAppState,
    observed_state: &mut SorotteGuiShellAppState,
    observed_actions: &mut Vec<GuiShellAction>,
    completed: impl Fn(&GuiShellAction) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        owner.run_deferred_startup_remote_actions_with_fetcher(handle, projected_state, |_| {
            panic!("startup public-server hydration must only start once")
        });
        owner.update_runtime.pump_background_check(handle);
        let actions = apply_startup_remote_actions(handle, observed_state, observed_actions);
        if actions.iter().any(&completed) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "startup remote job should complete"
        );
        std::thread::yield_now();
    }
}

fn apply_startup_remote_actions(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    observed_actions: &mut Vec<GuiShellAction>,
) -> Vec<GuiShellAction> {
    let actions = handle.drain_actions();
    for action in &actions {
        let _ = state.apply(action.clone());
    }
    observed_actions.extend(actions.iter().cloned());
    actions
}

#[test]
fn stale_background_result_cannot_overwrite_newer_manual_result() {
    assert_stale_automatic_result_is_ignored(UpdateJobOrigin::Background);
}

#[test]
fn stale_startup_result_cannot_overwrite_newer_manual_result() {
    assert_stale_automatic_result_is_ignored(UpdateJobOrigin::Startup);
}

fn assert_stale_automatic_result_is_ignored(origin: UpdateJobOrigin) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.blocked_check_language = Some("automatic".to_owned());
    service.check_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(true, "automatic", Some("dev")));

    match origin {
        UpdateJobOrigin::Startup => runtime.start_startup_check(&handle),
        UpdateJobOrigin::Background => runtime.pump_background_check(&handle),
        UpdateJobOrigin::Interactive => unreachable!(),
    }
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::BeginUpdateCheck {
            user_initiated: false
        }]
    ));
    gate.wait_until_entered();

    runtime.handle_command(
        &handle,
        Command::CheckForUpdates {
            language: "manual".to_owned(),
            update_channel: Some("stable".to_owned()),
            user_initiated: true,
        },
    );
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(result)] if result.message == "result:manual"
    ));

    gate.release();
    std::thread::sleep(Duration::from_millis(20));
    for _ in 0..4 {
        runtime.pump_background_check(&handle);
    }
    let late_actions = handle.drain_actions();
    assert!(
        !late_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyUpdateCheckResult(result)
                if result.message == "result:automatic"
        )),
        "an automatic result from a superseded job must be ignored"
    );
    assert_eq!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .iter()
            .filter(|call| call.starts_with("check:"))
            .count(),
        2,
        "a fresh automatic retry must not start before the manual result is projected"
    );
}

#[test]
fn duplicate_download_and_install_is_rejected_while_first_job_is_active() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.download_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    gate.wait_until_entered();
    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::AnnounceSystemChatEvent(message)]
            if message.contains("another update operation is in progress")
    ));
    assert_eq!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .iter()
            .filter(|call| call.starts_with("download:"))
            .count(),
        1
    );

    gate.release();
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [
            GuiShellAction::ApplyUpdateDownloadResult(_),
            GuiShellAction::BeginStagedUpdateApply
        ]
    ));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyStagedUpdateLaunchResult(_)]
    ));
}

#[test]
fn cancelled_download_and_install_never_launches_staged_update() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.download_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(false, "en", Some("stable")));

    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    gate.wait_until_entered();

    runtime.reconcile(&update_view(false, "en", Some("dev")));
    runtime.pump_background_check(&handle);
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(result)]
            if result.state == UpdateDownloadState::Failed
                && result.message.contains("settings changed")
    ));

    runtime.handle_command(&handle, Command::DownloadAndInstall(candidate()));
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::AnnounceSystemChatEvent(message)]
            if message.contains("another update operation is in progress")
    ));
    assert_eq!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .iter()
            .filter(|call| call.starts_with("download:"))
            .count(),
        1,
        "a replacement must not enter the shared staging directory while cancellation drains"
    );
    assert!(matches!(
        runtime.active_job.as_ref(),
        Some(active)
            if active.kind == UpdateJobKind::DownloadAndInstall
                && active.cancelled_by_config
    ));

    gate.release();
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.active_job.is_some() {
        assert!(
            Instant::now() < deadline,
            "cancelled download worker should finish after its gate is released"
        );
        runtime.pump_background_check(&handle);
        std::thread::yield_now();
    }

    assert!(handle.drain_actions().is_empty());
    runtime.handle_command(&handle, Command::Download(candidate()));
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(result)]
            if result.state == UpdateDownloadState::Staged
    ));
    assert_eq!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .iter()
            .filter(|call| call.starts_with("download:"))
            .count(),
        2,
        "the replacement may start only after the cancelled staging worker exits"
    );
    assert!(
        calls
            .lock()
            .expect("fake update calls should remain available")
            .iter()
            .all(|call| !call.starts_with("launch:")),
        "a staged result from the cancelled generation must not launch the helper"
    );
}

#[test]
fn apply_staged_remains_owned_across_update_configuration_changes() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls.clone());
    service.launch_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(false, "en", Some("stable")));

    runtime.handle_command(&handle, Command::ApplyStaged(staged_update()));
    gate.wait_until_entered();

    runtime.reconcile(&update_view(false, "fr", Some("dev")));
    runtime.pump_background_check(&handle);
    assert!(handle.drain_actions().is_empty());
    assert!(matches!(
        runtime.active_job.as_ref(),
        Some(active) if active.kind == UpdateJobKind::ApplyStaged
    ));

    gate.release();
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyStagedUpdateLaunchResult(result)] if result.success
    ));
    assert_eq!(
        *calls
            .lock()
            .expect("fake update calls should remain available"),
        vec!["launch:9.8.7".to_owned()]
    );
}

#[test]
fn previous_channel_generation_is_cancelled_and_its_result_is_ignored() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls);
    service.blocked_check_language = Some("old".to_owned());
    service.check_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(true, "old", Some("dev")));
    runtime.pump_background_check(&handle);
    let _ = handle.drain_actions();
    gate.wait_until_entered();

    runtime.reconcile(&update_view(false, "new", Some("stable")));
    runtime.pump_background_check(&handle);
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(result)]
            if result.status == LegacyUpdateCheckStatus::Failed
                && result.message.contains("settings changed")
    ));

    runtime.handle_command(
        &handle,
        Command::CheckForUpdates {
            language: "new".to_owned(),
            update_channel: Some("stable".to_owned()),
            user_initiated: true,
        },
    );
    assert!(matches!(
        pump_until_actions(&mut runtime, &handle).as_slice(),
        [GuiShellAction::ApplyUpdateCheckResult(result)] if result.message == "result:new"
    ));
    gate.release();
    std::thread::sleep(Duration::from_millis(20));
    runtime.pump_background_check(&handle);
    assert!(!handle.drain_actions().iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyUpdateCheckResult(result) if result.message == "result:old"
    )));
}

#[test]
fn config_generation_change_terminates_active_download_state() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(BlockGate::default());
    let mut service = fake_service(calls);
    service.download_gate = Some(gate.clone());
    let mut runtime = runtime_with_service(service);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    runtime.reconcile(&update_view(false, "en", Some("stable")));
    runtime.observe_actions(&[GuiShellAction::BeginUpdateDownload]);
    runtime.handle_command(&handle, Command::Download(candidate()));
    gate.wait_until_entered();

    runtime.reconcile(&update_view(false, "en", Some("dev")));
    runtime.pump_background_check(&handle);
    assert!(matches!(
        handle.drain_actions().as_slice(),
        [GuiShellAction::ApplyUpdateDownloadResult(result)]
            if result.state == UpdateDownloadState::Failed
                && result.message.contains("settings changed")
    ));
    assert_eq!(runtime.model.download_state, UpdateDownloadState::Failed);

    gate.release();
    std::thread::sleep(Duration::from_millis(20));
    runtime.pump_background_check(&handle);
    assert!(handle.drain_actions().is_empty());
}
