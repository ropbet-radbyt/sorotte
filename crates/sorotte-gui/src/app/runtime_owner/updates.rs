use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime},
};

use super::super::{
    feature_slices::updates::{Command, RuntimePolicy, RuntimeView},
    remote_services::{
        self, LegacyUpdateCheckResult, StagedUpdate, UpdateApplyLaunchResult, UpdateCandidate,
        UpdateDownloadResult, UpdateDownloadState,
    },
    runtime_queue::GuiQueuedRuntimeBridgeHandle,
    shell_state::GuiShellAction,
    ui_state::GuiUpdateCheckState,
};

const BACKGROUND_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(86_400);
const BACKGROUND_UPDATE_CHECK_RETRY_INTERVAL: Duration = Duration::from_secs(60);

pub(super) trait GuiUpdateService: Send + Sync {
    fn check_for_updates(
        &self,
        language: &str,
        user_initiated: bool,
        update_channel: Option<&str>,
    ) -> LegacyUpdateCheckResult;

    fn download_and_stage_update(
        &self,
        candidate: &UpdateCandidate,
        gui_config_root: Option<&Path>,
    ) -> UpdateDownloadResult;

    fn launch_staged_update(&self, staged_update: &StagedUpdate) -> UpdateApplyLaunchResult;
}

#[derive(Debug, Default)]
struct SystemGuiUpdateService;

impl GuiUpdateService for SystemGuiUpdateService {
    fn check_for_updates(
        &self,
        language: &str,
        user_initiated: bool,
        update_channel: Option<&str>,
    ) -> LegacyUpdateCheckResult {
        remote_services::check_for_updates(Some(language), user_initiated, update_channel)
    }

    fn download_and_stage_update(
        &self,
        candidate: &UpdateCandidate,
        gui_config_root: Option<&Path>,
    ) -> UpdateDownloadResult {
        remote_services::download_and_stage_update(candidate, gui_config_root)
    }

    fn launch_staged_update(&self, staged_update: &StagedUpdate) -> UpdateApplyLaunchResult {
        remote_services::launch_staged_update(staged_update)
    }
}

pub(in crate::app) struct GuiUpdateRuntime {
    service: Arc<dyn GuiUpdateService>,
    config_root: Option<PathBuf>,
    model: GuiUpdateCheckState,
    policy: RuntimePolicy,
    background_check_rx: Option<mpsc::Receiver<LegacyUpdateCheckResult>>,
    background_check_next_due_at: Option<Instant>,
}

impl GuiUpdateRuntime {
    pub(super) fn new(config_root: Option<PathBuf>) -> Self {
        Self::with_service(config_root, Arc::new(SystemGuiUpdateService))
    }

    fn with_service(config_root: Option<PathBuf>, service: Arc<dyn GuiUpdateService>) -> Self {
        Self {
            service,
            config_root,
            model: GuiUpdateCheckState::default(),
            policy: RuntimePolicy {
                automatic: false,
                last_checked_for_updates: None,
                language: "en".to_owned(),
                channel: None,
            },
            background_check_rx: None,
            background_check_next_due_at: None,
        }
    }

    pub(in crate::app) fn reconcile(&mut self, view: &RuntimeView) {
        self.model = view.model.clone();
        self.policy = view.policy.clone();
    }

    pub(super) fn handle_command(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        command: Command,
    ) {
        let actions = match command {
            Command::CheckForUpdates {
                language,
                update_channel,
                user_initiated,
            } => vec![GuiShellAction::ApplyUpdateCheckResult(
                self.service.check_for_updates(
                    &language,
                    user_initiated,
                    update_channel.as_deref(),
                ),
            )],
            Command::Download(candidate) => vec![GuiShellAction::ApplyUpdateDownloadResult(
                self.service
                    .download_and_stage_update(&candidate, self.config_root.as_deref()),
            )],
            Command::DownloadAndInstall(candidate) => {
                let result = self
                    .service
                    .download_and_stage_update(&candidate, self.config_root.as_deref());
                let staged_update = result.staged_update.clone();
                let mut actions = vec![GuiShellAction::ApplyUpdateDownloadResult(result)];
                if let Some(staged_update) = staged_update {
                    actions.push(GuiShellAction::BeginStagedUpdateApply);
                    actions.push(GuiShellAction::ApplyStagedUpdateLaunchResult(
                        self.service.launch_staged_update(&staged_update),
                    ));
                }
                actions
            }
            Command::ApplyStaged(staged_update) => {
                vec![GuiShellAction::ApplyStagedUpdateLaunchResult(
                    self.service.launch_staged_update(&staged_update),
                )]
            }
        };
        self.emit_actions(handle, actions);
    }

    pub(super) fn observe_actions(&mut self, actions: &[GuiShellAction]) {
        for action in actions {
            match action {
                GuiShellAction::BeginUpdateCheck { user_initiated } => {
                    self.model.status = Some(remote_services::LegacyUpdateCheckStatus::Checking);
                    self.model.message = Some("Checking for updates".to_owned());
                    self.model.user_initiated = *user_initiated;
                    self.model.download_state = UpdateDownloadState::Idle;
                    self.model.staged_update = None;
                }
                GuiShellAction::ApplyUpdateCheckResult(result) => {
                    self.model = GuiUpdateCheckState {
                        status: Some(result.status.clone()),
                        message: Some(result.message.clone()),
                        url: result.url.clone(),
                        candidate: result.candidate.clone(),
                        download_state: UpdateDownloadState::Idle,
                        staged_update: None,
                        self_update_supported: result.self_update_supported,
                        last_checked_for_updates: Some(result.checked_at_utc.clone()),
                        user_initiated: result.user_initiated,
                    };
                    self.policy.last_checked_for_updates = Some(result.checked_at_utc.clone());
                }
                GuiShellAction::BeginUpdateDownload => {
                    self.model.download_state = UpdateDownloadState::Downloading;
                    self.model.message = Some("Downloading and staging update...".to_owned());
                }
                GuiShellAction::ApplyUpdateDownloadResult(result) => {
                    self.model.download_state = result.state;
                    self.model.message = Some(result.message.clone());
                    self.model.staged_update = result.staged_update.clone();
                }
                GuiShellAction::BeginStagedUpdateApply => {
                    self.model.message = Some("Launching update helper...".to_owned());
                }
                GuiShellAction::ApplyStagedUpdateLaunchResult(result) => {
                    self.model.message = Some(result.message.clone());
                }
                _ => {}
            }
        }
    }

    pub(super) fn pump_background_check(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        startup_remote_check_pending: bool,
    ) {
        if let Some(rx) = self.background_check_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.background_check_next_due_at =
                        Some(Instant::now() + BACKGROUND_UPDATE_CHECK_INTERVAL);
                    self.emit_actions(handle, vec![GuiShellAction::ApplyUpdateCheckResult(result)]);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.background_check_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.background_check_next_due_at =
                        Some(Instant::now() + BACKGROUND_UPDATE_CHECK_RETRY_INTERVAL);
                    return;
                }
            }
        }

        if startup_remote_check_pending
            || self
                .background_check_next_due_at
                .is_some_and(|due_at| Instant::now() < due_at)
            || matches!(
                self.model.status,
                Some(remote_services::LegacyUpdateCheckStatus::Checking)
            )
            || matches!(self.model.download_state, UpdateDownloadState::Downloading)
            || !remote_services::automatic_update_check_due(
                self.policy.automatic,
                self.policy.last_checked_for_updates.as_deref(),
                SystemTime::now(),
            )
        {
            return;
        }

        let language = self.policy.language.clone();
        let update_channel = self.policy.channel.clone();
        self.emit_actions(
            handle,
            vec![GuiShellAction::BeginUpdateCheck {
                user_initiated: false,
            }],
        );

        let service = self.service.clone();
        let thread_language = language.clone();
        let thread_update_channel = update_channel.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-background-update".to_owned())
            .spawn(move || {
                let result = service.check_for_updates(
                    &thread_language,
                    false,
                    thread_update_channel.as_deref(),
                );
                let _ = tx.send(result);
            }) {
            Ok(_thread) => {
                self.background_check_rx = Some(rx);
            }
            Err(_error) => {
                let result =
                    self.service
                        .check_for_updates(&language, false, update_channel.as_deref());
                self.background_check_next_due_at =
                    Some(Instant::now() + BACKGROUND_UPDATE_CHECK_INTERVAL);
                self.emit_actions(handle, vec![GuiShellAction::ApplyUpdateCheckResult(result)]);
            }
        }
    }

    fn emit_actions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        actions: Vec<GuiShellAction>,
    ) {
        self.observe_actions(&actions);
        handle.push_actions(actions);
    }
}

#[cfg(test)]
mod tests;
