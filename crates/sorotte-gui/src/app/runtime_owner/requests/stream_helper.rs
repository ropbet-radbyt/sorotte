use super::*;
use crate::app::{
    helper_tools::HelperWorker,
    runtime_state::GuiRuntimeState,
    stream_support::{StreamHelperAttachMode, probe_stream_helper_runtime_snapshot_with_cancel},
};
use crate::app::{
    shell_state::GuiStreamHelperRuntimeSnapshot,
    stream_support::probe_stream_helper_startup_snapshot,
};
use std::sync::mpsc;

#[cfg(test)]
#[path = "stream_helper/flow_tests.rs"]
mod flow_tests;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app::runtime_owner) struct StreamHelperProbeScope {
    root: Option<PathBuf>,
    attach_mode: StreamHelperAttachMode,
    target: Option<String>,
    player_epoch: u64,
    session_generation: u64,
    playlist_generation: u64,
}

enum StreamHelperOperation {
    Probe,
    Install,
    Downloader(String),
    JsRuntime(String),
}
enum StreamHelperEvent {
    Progress(StreamHelperRemediationProgress),
    Probed(GuiStreamHelperRuntimeSnapshot),
    Finished(Result<String, String>),
}
pub(in crate::app::runtime_owner) struct StreamHelperWorker {
    worker: HelperWorker<StreamHelperEvent>,
    scope: StreamHelperProbeScope,
    installing: bool,
}

impl GuiPersistedConfigRuntimeOwner {
    fn stream_helper_scope(&self, target: Option<&str>) -> StreamHelperProbeScope {
        StreamHelperProbeScope {
            root: self.syncplay_qsettings_root(),
            attach_mode: self.player_stream_helper_attach_mode(),
            target: target.map(str::to_owned),
            player_epoch: self.player_attachment_epoch,
            session_generation: self.session_generation,
            playlist_generation: self.playlist_resolution.generation,
        }
    }

    fn start_stream_helper_worker(
        &mut self,
        scope: StreamHelperProbeScope,
        operation: StreamHelperOperation,
    ) -> Result<(), String> {
        let installing = !matches!(operation, StreamHelperOperation::Probe);
        let probe_scope = scope.clone();
        let worker = HelperWorker::spawn(
            "sorotte-stream-helper",
            scope.root.clone(),
            move |cancel, tx| {
                let progress = |progress| {
                    let _ = tx.send(StreamHelperEvent::Progress(progress));
                };
                let result = match operation {
                    StreamHelperOperation::Probe => {
                        let snapshot = probe_stream_helper_runtime_snapshot_with_cancel(
                            probe_scope.root.as_deref(),
                            probe_scope.attach_mode,
                            probe_scope.target.as_deref(),
                            Some(cancel),
                        );
                        let _ = tx.send(StreamHelperEvent::Probed(snapshot));
                        return;
                    }
                    StreamHelperOperation::Install => {
                        install_or_update_managed_stream_helper_with_progress(
                            probe_scope.root.as_deref().expect("installation root"),
                            Some(cancel),
                            progress,
                        )
                    }
                    StreamHelperOperation::Downloader(source) => {
                        import_managed_stream_helper_downloader_with_progress(
                            probe_scope.root.as_deref().expect("installation root"),
                            Path::new(&source),
                            Some(cancel),
                            progress,
                        )
                    }
                    StreamHelperOperation::JsRuntime(source) => {
                        import_managed_stream_helper_js_runtime_with_progress(
                            probe_scope.root.as_deref().expect("installation root"),
                            Path::new(&source),
                            Some(cancel),
                            progress,
                        )
                    }
                };
                let _ = tx.send(StreamHelperEvent::Finished(result));
            },
        )?;
        self.stream_helper_worker = Some(StreamHelperWorker {
            worker,
            scope,
            installing,
        });
        Ok(())
    }

    pub(in crate::app::runtime_owner) fn queue_stream_helper_probe(
        &mut self,
        target: Option<&str>,
    ) -> GuiStreamHelperRuntimeSnapshot {
        let scope = self.stream_helper_scope(target);
        if self.stream_helper_probe_scope.as_ref() == Some(&scope) {
            return self.stream_helper_runtime_snapshot.clone();
        }
        if target.is_none_or(|target| {
            browser_stream_target_kind(target, None) != GuiStreamTargetKind::ExtractorPageUrl
        }) {
            if self
                .stream_helper_worker
                .as_ref()
                .is_some_and(|worker| !worker.installing)
            {
                self.stream_helper_worker = None;
            }
            self.stream_helper_runtime_snapshot =
                probe_stream_helper_startup_snapshot(scope.root.as_deref(), scope.attach_mode);
            self.stream_helper_probe_scope = Some(scope);
            return self.stream_helper_runtime_snapshot.clone();
        }
        if !self
            .stream_helper_worker
            .as_ref()
            .is_some_and(|worker| worker.installing || worker.scope == scope)
        {
            self.stream_helper_worker = None;
            if let Err(error) =
                self.start_stream_helper_worker(scope.clone(), StreamHelperOperation::Probe)
            {
                self.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Broken;
                self.stream_helper_runtime_snapshot.message = Some(error);
                return self.stream_helper_runtime_snapshot.clone();
            }
        }
        let mut snapshot =
            probe_stream_helper_startup_snapshot(scope.root.as_deref(), scope.attach_mode);
        snapshot.health = GuiStreamHelperHealth::Checking;
        snapshot.target = scope.target;
        snapshot.message = Some("Checking the tools required to open this URL.".to_owned());
        snapshot.retry_available = false;
        self.stream_helper_runtime_snapshot = snapshot.clone();
        snapshot
    }

    pub(in crate::app::runtime_owner) fn pump_stream_helper_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
    ) -> bool {
        let Some(active) = self.stream_helper_worker.take() else {
            return false;
        };
        if active.worker.root != self.syncplay_qsettings_root()
            || !state
                .settings
                .plugin_enablement
                .enabled_for(GuiPluginSelection::StreamSupport)
        {
            self.stream_helper_probe_scope = None;
            self.clear_stream_helper_remediation_progress(handle, state);
            return false;
        }
        if !active.installing
            && active.scope
                != self.stream_helper_scope(self.stream_helper_runtime_snapshot.target.as_deref())
        {
            self.stream_helper_probe_scope = None;
            if self.pending_stream_retry_target == active.scope.target {
                self.pending_stream_retry_target = None;
            }
            self.stream_helper_runtime_snapshot = probe_stream_helper_startup_snapshot(
                self.syncplay_qsettings_root().as_deref(),
                self.player_stream_helper_attach_mode(),
            );
            Self::push_actions_and_project(
                handle,
                state,
                vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                    self.stream_helper_runtime_snapshot.clone(),
                )],
            );
            return false;
        }
        loop {
            match active.worker.rx.try_recv() {
                Ok(StreamHelperEvent::Progress(progress)) => self
                    .report_stream_helper_remediation_progress(
                        handle,
                        state,
                        progress.label,
                        progress.detail,
                        progress.progress_fraction,
                    ),
                Ok(StreamHelperEvent::Finished(result)) => {
                    self.clear_stream_helper_remediation_progress(handle, state);
                    match result {
                        Ok(message) => {
                            if active.scope
                                != self.stream_helper_scope(active.scope.target.as_deref())
                                && self.pending_stream_retry_target == active.scope.target
                            {
                                self.pending_stream_retry_target = None;
                            }
                            self.mark_managed_player_stream_helper_refresh_required();
                            self.stream_helper_probe_scope = None;
                            let snapshot = self.recheck_stream_helper_runtime_snapshot(state);
                            Self::push_actions_and_project(
                                handle,
                                state,
                                vec![
                                    GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(snapshot),
                                    GuiShellAction::PushTransientNotification {
                                        level: GuiTransientNotificationLevel::Success,
                                        message,
                                    },
                                ],
                            );
                        }
                        Err(error) => Self::push_runtime_error_notification(handle, state, error),
                    }
                    return false;
                }
                Ok(StreamHelperEvent::Probed(snapshot)) => {
                    let ready = snapshot.health == GuiStreamHelperHealth::Healthy;
                    self.stream_helper_probe_scope = Some(active.scope);
                    self.stream_helper_runtime_snapshot = snapshot.clone();
                    Self::push_actions_and_project(
                        handle,
                        state,
                        vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                            snapshot.clone(),
                        )],
                    );
                    if ready
                        && self.pending_stream_retry_target.as_deref() == snapshot.target.as_deref()
                    {
                        self.pending_stream_retry_target = None;
                    }
                    // Resume the selected row through its existing resolution and delivery fence.
                    // Reissuing Open Media here would create a second playlist mutation.
                    return ready;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.stream_helper_worker = Some(active);
                    return false;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.clear_stream_helper_remediation_progress(handle, state);
                    let error =
                        "Stream helper worker stopped before reporting a result.".to_owned();
                    self.stream_helper_probe_scope = None;
                    self.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Broken;
                    self.stream_helper_runtime_snapshot.message = Some(error.clone());
                    self.stream_helper_runtime_snapshot.retry_available =
                        self.stream_helper_runtime_snapshot.target.is_some();
                    Self::push_actions_and_project(
                        handle,
                        state,
                        vec![
                            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                                self.stream_helper_runtime_snapshot.clone(),
                            ),
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: error,
                            },
                        ],
                    );
                    return false;
                }
            }
        }
    }

    fn begin_stream_helper_remediation(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
        operation: StreamHelperOperation,
    ) -> bool {
        if !state
            .settings
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        if self
            .stream_helper_worker
            .as_ref()
            .is_some_and(|worker| worker.installing)
        {
            return true;
        }
        let target = self.stream_helper_target_candidate(state);
        let scope = self.stream_helper_scope(target.as_deref());
        if scope.root.is_none() {
            Self::push_runtime_error_notification(
                handle,
                state,
                "Stream helper installation requires a writable GUI config root.".to_owned(),
            );
            return false;
        }
        self.stream_helper_worker = None;
        self.stream_helper_probe_scope = None;
        match self.start_stream_helper_worker(scope, operation) {
            Ok(()) => self.report_stream_helper_remediation_progress(
                handle,
                state,
                "Preparing stream helpers",
                None,
                0.02,
            ),
            Err(error) => Self::push_runtime_error_notification(handle, state, error),
        }
        true
    }

    pub(super) fn handle_install_stream_helper_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
    ) -> bool {
        self.begin_stream_helper_remediation(handle, state, StreamHelperOperation::Install)
    }
    pub(super) fn handle_integrate_stream_helper_downloader_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
        source: String,
    ) -> bool {
        self.begin_stream_helper_remediation(
            handle,
            state,
            StreamHelperOperation::Downloader(source),
        )
    }
    pub(super) fn handle_integrate_stream_helper_js_runtime_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
        source: String,
    ) -> bool {
        self.begin_stream_helper_remediation(
            handle,
            state,
            StreamHelperOperation::JsRuntime(source),
        )
    }
    pub(super) fn handle_recheck_stream_helper_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut GuiRuntimeState,
    ) -> bool {
        if !state
            .settings
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        self.stream_helper_probe_scope = None;
        if self
            .stream_helper_worker
            .as_ref()
            .is_some_and(|worker| !worker.installing)
        {
            self.stream_helper_worker = None;
        }
        let snapshot = self.recheck_stream_helper_runtime_snapshot(state);
        Self::push_actions_and_project(
            handle,
            state,
            vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                snapshot,
            )],
        );
        true
    }

    pub(super) fn handle_open_stream_helper_install_location_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut GuiRuntimeState,
    ) -> bool {
        let install_location = projected_state
            .player
            .stream_helper
            .install_location
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                self.syncplay_qsettings_root()
                    .map(|root| managed_stream_helper_bin_dir(&root))
            });
        let Some(install_location) = install_location else {
            Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        "Opening the managed stream-helper install location requires a writable GUI config root."
                            .to_owned(),
                    );
            return false;
        };
        if let Err(error) = std::fs::create_dir_all(&install_location) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not create the managed stream-helper install location '{}': {error}",
                    install_location.display()
                ),
            );
            return false;
        }
        self.open_stream_helper_install_location_runtime(handle, projected_state, install_location);
        true
    }

    pub(super) fn handle_retry_pending_stream_media_open_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut GuiRuntimeState,
    ) -> bool {
        if !projected_state
            .settings
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let Some(target) = self
            .pending_stream_retry_target
            .clone()
            .or_else(|| self.current_shared_playlist_target(projected_state))
        else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "No pending media URL is available to retry.".to_owned(),
            );
            return false;
        };
        Self::push_actions_and_project(handle, projected_state, vec![GuiShellAction::CloseModal]);
        self.stream_helper_probe_scope = None;
        let snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(Some(&target));
        let ready = snapshot.health == GuiStreamHelperHealth::Healthy;
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                snapshot,
            )],
        );
        if ready {
            self.sync_active_shared_playlist_media_and_playstate_impl(projected_state);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        shell_state::SorotteGuiShellAppState, testing::support::runtime_state_for_shell,
    };
    use sorotte_client_app::app_boundary::state::StoredClientSettings;

    #[test]
    fn successful_probe_resumes_resolution_without_reissuing_open_media() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = runtime_state_for_shell(&SorotteGuiShellAppState::from_stored_settings(
            &StoredClientSettings::default(),
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let target = "https://www.youtube.com/watch?v=selected";
        let scope = owner.stream_helper_scope(Some(target));
        owner.stream_helper_runtime_snapshot.target = Some(target.to_owned());
        owner.pending_stream_retry_target = Some(target.to_owned());
        let (sent_tx, sent_rx) = mpsc::channel();
        let worker = HelperWorker::spawn("ready-probe-fixture", None, move |_, tx| {
            tx.send(StreamHelperEvent::Probed(GuiStreamHelperRuntimeSnapshot {
                target: Some(target.to_owned()),
                ..GuiStreamHelperRuntimeSnapshot::default()
            }))
            .unwrap();
            sent_tx.send(()).unwrap();
        })
        .unwrap();
        sent_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        owner.stream_helper_worker = Some(StreamHelperWorker {
            worker,
            scope,
            installing: false,
        });
        assert!(owner.pump_stream_helper_worker(&handle, &mut state));
        assert!(owner.pending_stream_retry_target.is_none());
        let actions = handle.drain_actions();
        assert_eq!(
            actions.len(),
            1,
            "completion must not issue another Open Media request or mutate the playlist"
        );
        assert!(matches!(
            actions[0],
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(_)
        ));
    }

    #[test]
    fn probe_completion_cannot_restore_an_old_player_or_playlist_target() {
        for change_player in [true, false] {
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
            let mut state = runtime_state_for_shell(
                &SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default()),
            );
            let handle = GuiQueuedRuntimeBridgeHandle::default();
            let target = "https://www.youtube.com/watch?v=old";
            let scope = owner.stream_helper_scope(Some(target));
            owner.stream_helper_runtime_snapshot.target = Some(target.to_owned());
            owner.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Checking;
            owner.pending_stream_retry_target = Some(target.to_owned());
            let (ready_tx, ready_rx) = mpsc::channel();
            let worker = HelperWorker::spawn("stale-probe-fixture", None, move |_, tx| {
                let snapshot = GuiStreamHelperRuntimeSnapshot {
                    target: Some(target.to_owned()),
                    ..GuiStreamHelperRuntimeSnapshot::default()
                };
                tx.send(StreamHelperEvent::Probed(snapshot)).unwrap();
                ready_tx.send(()).unwrap();
            })
            .unwrap();
            ready_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            owner.stream_helper_worker = Some(StreamHelperWorker {
                worker,
                scope,
                installing: false,
            });
            if change_player {
                owner.player_attachment_epoch += 1;
            } else {
                owner.playlist_resolution.generation += 1;
            }
            assert!(!owner.pump_stream_helper_worker(&handle, &mut state));
            assert!(owner.pending_stream_retry_target.is_none());
            assert!(owner.stream_helper_worker.is_none());
            assert!(owner.stream_helper_runtime_snapshot.target.is_none());
        }
    }

    #[test]
    fn pending_probe_pump_returns_without_waiting_and_disable_cancels_it() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = runtime_state_for_shell(&SorotteGuiShellAppState::from_stored_settings(
            &StoredClientSettings::default(),
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let scope = owner.stream_helper_scope(None);
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let worker = HelperWorker::spawn("pending-probe-fixture", None, move |cancel, _| {
            while !cancel.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            cancelled_tx.send(()).unwrap();
        })
        .unwrap();
        owner.stream_helper_worker = Some(StreamHelperWorker {
            worker,
            scope,
            installing: false,
        });
        assert!(!owner.pump_stream_helper_worker(&handle, &mut state));
        assert!(owner.stream_helper_worker.is_some());
        owner.stop_disabled_plugin_runtime_work(
            &handle,
            &mut state,
            GuiPluginSelection::StreamSupport,
        );
        cancelled_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(owner.stream_helper_worker.is_none());
    }
}
