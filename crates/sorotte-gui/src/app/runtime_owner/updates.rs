use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
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

    fn download_and_stage_update_cancellable(
        &self,
        candidate: &UpdateCandidate,
        gui_config_root: Option<&Path>,
        _cancelled: &AtomicBool,
    ) -> UpdateDownloadResult {
        self.download_and_stage_update(candidate, gui_config_root)
    }

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

    fn download_and_stage_update_cancellable(
        &self,
        candidate: &UpdateCandidate,
        gui_config_root: Option<&Path>,
        cancelled: &AtomicBool,
    ) -> UpdateDownloadResult {
        remote_services::download_and_stage_update_cancellable(
            candidate,
            gui_config_root,
            cancelled,
        )
    }

    fn launch_staged_update(&self, staged_update: &StagedUpdate) -> UpdateApplyLaunchResult {
        remote_services::launch_staged_update(staged_update)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateJobOrigin {
    Startup,
    Background,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateJobKind {
    Check {
        origin: UpdateJobOrigin,
        user_initiated: bool,
    },
    Download,
    DownloadAndInstall,
    ApplyStaged,
}

impl UpdateJobKind {
    fn is_automatic(self) -> bool {
        matches!(
            self,
            Self::Check {
                origin: UpdateJobOrigin::Startup | UpdateJobOrigin::Background,
                ..
            }
        )
    }

    fn thread_name(self) -> &'static str {
        match self {
            Self::Check {
                origin: UpdateJobOrigin::Startup,
                ..
            } => "sorotte-gui-startup-update",
            Self::Check {
                origin: UpdateJobOrigin::Background,
                ..
            } => "sorotte-gui-background-update",
            Self::Check {
                origin: UpdateJobOrigin::Interactive,
                ..
            } => "sorotte-gui-update-check",
            Self::Download => "sorotte-gui-update-download",
            Self::DownloadAndInstall => "sorotte-gui-update-install",
            Self::ApplyStaged => "sorotte-gui-update-apply",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Check { .. } => "update check",
            Self::Download => "update download",
            Self::DownloadAndInstall => "update installation",
            Self::ApplyStaged => "staged update launch",
        }
    }

    fn is_config_cancellable(self) -> bool {
        !matches!(self, Self::ApplyStaged)
    }

    fn owns_staging_side_effects(self) -> bool {
        matches!(self, Self::Download | Self::DownloadAndInstall)
    }
}

enum UpdateWorkerOutput {
    Actions(Vec<GuiShellAction>),
    DownloadAndInstall(Box<UpdateDownloadResult>),
}

struct UpdateWorkerResult {
    id: u64,
    config_generation: u64,
    output: UpdateWorkerOutput,
}

type UpdateJobWork = Box<dyn FnOnce(Arc<AtomicBool>) -> UpdateWorkerOutput + Send>;

struct ActiveUpdateJob {
    id: u64,
    kind: UpdateJobKind,
    config_generation: u64,
    cancelled_by_config: bool,
    cancelled: Arc<AtomicBool>,
    result_rx: mpsc::Receiver<UpdateWorkerResult>,
}

impl Drop for ActiveUpdateJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateJobConfiguration {
    automatic: bool,
    language: String,
    channel: Option<String>,
}

pub(in crate::app) struct GuiUpdateRuntime {
    service: Arc<dyn GuiUpdateService>,
    config_root: Option<PathBuf>,
    model: GuiUpdateCheckState,
    policy: RuntimePolicy,
    job_configuration: UpdateJobConfiguration,
    config_generation: u64,
    next_job_id: u64,
    active_job: Option<ActiveUpdateJob>,
    pending_actions: Vec<GuiShellAction>,
    background_check_next_due_at: Option<Instant>,
}

impl GuiUpdateRuntime {
    pub(super) fn new(config_root: Option<PathBuf>) -> Self {
        Self::with_service(config_root, Arc::new(SystemGuiUpdateService))
    }

    fn with_service(config_root: Option<PathBuf>, service: Arc<dyn GuiUpdateService>) -> Self {
        let policy = RuntimePolicy {
            automatic: false,
            last_checked_for_updates: None,
            language: "en".to_owned(),
            channel: None,
        };
        Self {
            service,
            config_root,
            model: GuiUpdateCheckState::default(),
            job_configuration: UpdateJobConfiguration::from(&policy),
            policy,
            config_generation: 0,
            next_job_id: 0,
            active_job: None,
            pending_actions: Vec::new(),
            background_check_next_due_at: None,
        }
    }

    pub(in crate::app) fn reconcile(&mut self, view: &RuntimeView) {
        let next_job_configuration = UpdateJobConfiguration::from(&view.policy);
        if self.job_configuration != next_job_configuration {
            if let Some(kind) = self
                .active_job
                .as_ref()
                .filter(|active| active.kind.is_config_cancellable())
                .map(|active| active.kind)
            {
                let first_cancellation = if kind.owns_staging_side_effects() {
                    let active = self
                        .active_job
                        .as_mut()
                        .expect("cancellable update job should still be active");
                    let first_cancellation = !active.cancelled_by_config;
                    active.cancelled_by_config = true;
                    active.cancelled.store(true, Ordering::Release);
                    first_cancellation
                } else {
                    self.active_job = None;
                    true
                };
                if first_cancellation {
                    let checked_at_utc = self
                        .policy
                        .last_checked_for_updates
                        .clone()
                        .unwrap_or_else(|| "1970-01-01 00:00:00.000".to_owned());
                    self.pending_actions.extend(self.failure_actions_at(
                        kind,
                        "Update operation cancelled because update settings changed.".to_owned(),
                        checked_at_utc,
                    ));
                }
            }
            self.job_configuration = next_job_configuration;
            self.config_generation = self.config_generation.wrapping_add(1);
            // Checks have no persistent side effects and can be detached. Downloads retain
            // their active slot until their staging worker finishes cleanup. ApplyStaged is also
            // retained: once helper launch has been accepted, settings changes cannot cancel
            // that irreversible operation or hide its result.
            self.background_check_next_due_at = None;
        }
        self.model = view.model.clone();
        self.policy = view.policy.clone();
    }

    pub(super) fn handle_command(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        command: Command,
    ) {
        let (kind, work): (UpdateJobKind, UpdateJobWork) = match command {
            Command::CheckForUpdates {
                language,
                update_channel,
                user_initiated,
            } => {
                let service = self.service.clone();
                let origin = if user_initiated {
                    UpdateJobOrigin::Interactive
                } else {
                    UpdateJobOrigin::Background
                };
                (
                    UpdateJobKind::Check {
                        origin,
                        user_initiated,
                    },
                    Box::new(move |_cancelled| {
                        UpdateWorkerOutput::Actions(vec![GuiShellAction::ApplyUpdateCheckResult(
                            service.check_for_updates(
                                &language,
                                user_initiated,
                                update_channel.as_deref(),
                            ),
                        )])
                    }),
                )
            }
            Command::Download(candidate) => {
                let service = self.service.clone();
                let config_root = self.config_root.clone();
                (
                    UpdateJobKind::Download,
                    Box::new(move |_cancelled| {
                        UpdateWorkerOutput::Actions(vec![
                            GuiShellAction::ApplyUpdateDownloadResult(
                                service.download_and_stage_update_cancellable(
                                    &candidate,
                                    config_root.as_deref(),
                                    &_cancelled,
                                ),
                            ),
                        ])
                    }),
                )
            }
            Command::DownloadAndInstall(candidate) => {
                let service = self.service.clone();
                let config_root = self.config_root.clone();
                (
                    UpdateJobKind::DownloadAndInstall,
                    Box::new(move |_cancelled| {
                        UpdateWorkerOutput::DownloadAndInstall(Box::new(
                            service.download_and_stage_update_cancellable(
                                &candidate,
                                config_root.as_deref(),
                                &_cancelled,
                            ),
                        ))
                    }),
                )
            }
            Command::ApplyStaged(staged_update) => {
                let service = self.service.clone();
                (
                    UpdateJobKind::ApplyStaged,
                    Box::new(move |_cancelled| {
                        UpdateWorkerOutput::Actions(vec![
                            GuiShellAction::ApplyStagedUpdateLaunchResult(
                                service.launch_staged_update(&staged_update),
                            ),
                        ])
                    }),
                )
            }
        };

        self.start_command_job(handle, kind, work);
    }

    pub(super) fn start_startup_check(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        if self.active_job.is_some() {
            return;
        }
        self.emit_actions(
            handle,
            vec![GuiShellAction::BeginUpdateCheck {
                user_initiated: false,
            }],
        );
        self.start_check_job(
            handle,
            UpdateJobOrigin::Startup,
            self.policy.language.clone(),
            self.policy.channel.clone(),
            false,
        );
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

    pub(super) fn pump_background_check(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        if !self.pending_actions.is_empty() {
            let actions = std::mem::take(&mut self.pending_actions);
            self.emit_actions(handle, actions);
        }
        self.pump_active_job(handle);
        if self.active_job.is_some()
            || self
                .background_check_next_due_at
                .is_some_and(|due_at| Instant::now() < due_at)
            || !remote_services::automatic_update_check_due(
                self.policy.automatic,
                self.policy.last_checked_for_updates.as_deref(),
                SystemTime::now(),
            )
        {
            return;
        }

        self.emit_actions(
            handle,
            vec![GuiShellAction::BeginUpdateCheck {
                user_initiated: false,
            }],
        );
        self.start_check_job(
            handle,
            UpdateJobOrigin::Background,
            self.policy.language.clone(),
            self.policy.channel.clone(),
            false,
        );
    }

    fn start_command_job(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        kind: UpdateJobKind,
        work: UpdateJobWork,
    ) {
        if let Some(active) = self.active_job.as_ref()
            && !active.kind.is_automatic()
        {
            handle.push_action(GuiShellAction::AnnounceSystemChatEvent(format!(
                "Cannot start {} while another update operation is in progress.",
                kind.label()
            )));
            return;
        }

        // A command-originated request supersedes startup/background work. Its old
        // receiver is dropped, so the automatic result can never overwrite an
        // interactive operation.
        if !kind.is_automatic() {
            self.background_check_next_due_at = Some(
                Instant::now()
                    + if matches!(kind, UpdateJobKind::Check { .. }) {
                        BACKGROUND_UPDATE_CHECK_INTERVAL
                    } else {
                        BACKGROUND_UPDATE_CHECK_RETRY_INTERVAL
                    },
            );
        }
        self.active_job = None;
        self.spawn_job(handle, kind, work);
    }

    fn start_check_job(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        origin: UpdateJobOrigin,
        language: String,
        update_channel: Option<String>,
        user_initiated: bool,
    ) {
        let service = self.service.clone();
        self.spawn_job(
            handle,
            UpdateJobKind::Check {
                origin,
                user_initiated,
            },
            Box::new(move |_cancelled| {
                UpdateWorkerOutput::Actions(vec![GuiShellAction::ApplyUpdateCheckResult(
                    service.check_for_updates(&language, user_initiated, update_channel.as_deref()),
                )])
            }),
        );
    }

    fn spawn_job(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        kind: UpdateJobKind,
        work: UpdateJobWork,
    ) {
        self.next_job_id = self.next_job_id.wrapping_add(1);
        let id = self.next_job_id;
        let config_generation = self.config_generation;
        let (tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let spawn_result = std::thread::Builder::new()
            .name(kind.thread_name().to_owned())
            .spawn(move || {
                let output = work(worker_cancelled);
                let _ = tx.send(UpdateWorkerResult {
                    id,
                    config_generation,
                    output,
                });
            });

        match spawn_result {
            Ok(_thread) => {
                self.active_job = Some(ActiveUpdateJob {
                    id,
                    kind,
                    config_generation,
                    cancelled_by_config: false,
                    cancelled,
                    result_rx,
                });
            }
            Err(error) => {
                self.active_job = None;
                if kind.is_automatic() {
                    self.background_check_next_due_at =
                        Some(Instant::now() + BACKGROUND_UPDATE_CHECK_RETRY_INTERVAL);
                }
                self.emit_actions(
                    handle,
                    self.failure_actions(kind, format!("Unable to start update worker: {error}")),
                );
            }
        }
    }

    fn pump_active_job(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        let Some(active) = self.active_job.take() else {
            return;
        };
        match active.result_rx.try_recv() {
            Ok(result) => {
                if active.kind.is_automatic() {
                    self.background_check_next_due_at =
                        Some(Instant::now() + BACKGROUND_UPDATE_CHECK_INTERVAL);
                }
                if result.id == active.id
                    && result.config_generation == active.config_generation
                    && (!active.kind.is_config_cancellable()
                        || result.config_generation == self.config_generation)
                {
                    self.accept_worker_output(handle, active.kind, result.output);
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.active_job = Some(active);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if active.cancelled_by_config {
                    return;
                }
                if active.kind.is_automatic() {
                    self.background_check_next_due_at =
                        Some(Instant::now() + BACKGROUND_UPDATE_CHECK_RETRY_INTERVAL);
                }
                self.emit_actions(
                    handle,
                    self.failure_actions(
                        active.kind,
                        "Update worker stopped before returning a result.".to_owned(),
                    ),
                );
            }
        }
    }

    fn accept_worker_output(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        kind: UpdateJobKind,
        output: UpdateWorkerOutput,
    ) {
        match output {
            UpdateWorkerOutput::Actions(actions) => self.emit_actions(handle, actions),
            UpdateWorkerOutput::DownloadAndInstall(result) => {
                debug_assert_eq!(kind, UpdateJobKind::DownloadAndInstall);
                let result = *result;
                let staged_update = result.staged_update.clone();
                let mut actions = vec![GuiShellAction::ApplyUpdateDownloadResult(result)];
                if staged_update.is_some() {
                    actions.push(GuiShellAction::BeginStagedUpdateApply);
                }
                self.emit_actions(handle, actions);

                if let Some(staged_update) = staged_update {
                    let service = self.service.clone();
                    self.spawn_job(
                        handle,
                        UpdateJobKind::ApplyStaged,
                        Box::new(move |_cancelled| {
                            UpdateWorkerOutput::Actions(vec![
                                GuiShellAction::ApplyStagedUpdateLaunchResult(
                                    service.launch_staged_update(&staged_update),
                                ),
                            ])
                        }),
                    );
                }
            }
        }
    }

    fn failure_actions(&self, kind: UpdateJobKind, message: String) -> Vec<GuiShellAction> {
        self.failure_actions_at(
            kind,
            message,
            remote_services::legacy_utc_timestamp_string_legacy_compatible(SystemTime::now()),
        )
    }

    fn failure_actions_at(
        &self,
        kind: UpdateJobKind,
        message: String,
        checked_at_utc: String,
    ) -> Vec<GuiShellAction> {
        match kind {
            UpdateJobKind::Check { user_initiated, .. } => {
                vec![GuiShellAction::ApplyUpdateCheckResult(
                    LegacyUpdateCheckResult {
                        status: remote_services::LegacyUpdateCheckStatus::Failed,
                        message,
                        url: None,
                        candidate: None,
                        self_update_supported: false,
                        public_servers: None,
                        checked_at_utc,
                        user_initiated,
                    },
                )]
            }
            UpdateJobKind::Download | UpdateJobKind::DownloadAndInstall => {
                vec![GuiShellAction::ApplyUpdateDownloadResult(
                    UpdateDownloadResult {
                        state: UpdateDownloadState::Failed,
                        message,
                        staged_update: None,
                    },
                )]
            }
            UpdateJobKind::ApplyStaged => {
                vec![GuiShellAction::ApplyStagedUpdateLaunchResult(
                    UpdateApplyLaunchResult {
                        success: false,
                        message,
                    },
                )]
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

impl From<&RuntimePolicy> for UpdateJobConfiguration {
    fn from(policy: &RuntimePolicy) -> Self {
        Self {
            automatic: policy.automatic,
            language: policy.language.clone(),
            channel: policy.channel.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
