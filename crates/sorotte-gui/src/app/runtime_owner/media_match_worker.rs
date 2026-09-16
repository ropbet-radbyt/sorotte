//! Index I/O belongs to the worker; the runtime grants activation for its current job.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use sorotte_media_match::{MediaIndexCommitError, MediaIndexCommitOutcome, MediaMatchSettings};

use crate::app::{
    media_match_support::{
        MediaMatchIndexRebuildResult, MediaMatchToolProgress,
        prepare_media_match_index_rebuild_backup,
    },
    runtime_queue::GuiQueuedRuntimeBridgeHandle,
};

#[derive(Clone, Copy)]
pub(in crate::app) enum IndexFinalization {
    Activate,
    Abort,
}

pub(in crate::app) struct IndexJobScope {
    pub(super) root: PathBuf,
    pub(super) settings: MediaMatchSettings,
    pub(super) player_path: Option<String>,
    pub(super) room_target: Option<String>,
}

pub(in crate::app) enum GuiMediaMatchBackgroundWorkerEvent {
    Progress(MediaMatchToolProgress),
    AwaitingActivation {
        extraction_succeeded: bool,
    },
    Finished {
        result: Result<MediaMatchIndexRebuildResult, String>,
        finalization: Result<Option<MediaIndexCommitOutcome>, MediaIndexCommitError>,
    },
}

/// Each invocation owns a fresh event/decision channel. Dropping the runtime's decision
/// sender aborts an unaccepted stage; a completion can never migrate to a successor job.
pub(super) fn spawn_index_worker<F>(
    name: &str,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    wake: GuiQueuedRuntimeBridgeHandle,
    rebuild: F,
) -> Result<
    (
        mpsc::Receiver<GuiMediaMatchBackgroundWorkerEvent>,
        mpsc::Sender<IndexFinalization>,
    ),
    std::io::Error,
>
where
    F: FnOnce(
            &Path,
            &mpsc::Sender<GuiMediaMatchBackgroundWorkerEvent>,
        ) -> Result<MediaMatchIndexRebuildResult, String>
        + Send
        + 'static,
{
    let (tx, rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    #[cfg(test)]
    let trace_client = crate::app::latency_review_probe::current_client();
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            #[cfg(test)]
            crate::app::latency_review_probe::client(&trace_client);
            let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Progress(
                MediaMatchToolProgress {
                    label: "Preparing the Media Match index".to_owned(),
                    detail: None,
                    progress_fraction: 0.0,
                },
            ));
            wake.notify_threaded_runtime_owner();
            let transaction = match prepare_media_match_index_rebuild_backup(&root) {
                Ok(transaction) => transaction,
                Err(error) => {
                    let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished {
                        result: Err(error.clone()),
                        finalization: Err(MediaIndexCommitError::NotActivated(error)),
                    });
                    wake.notify_threaded_runtime_owner();
                    return;
                }
            };
            let result = if cancel.load(Ordering::Acquire) {
                Err("Media Matching rebuild canceled before extraction".to_owned())
            } else {
                rebuild(transaction.staging_app_root(), &tx)
            };
            if tx
                .send(GuiMediaMatchBackgroundWorkerEvent::AwaitingActivation {
                    extraction_succeeded: result.is_ok(),
                })
                .is_err()
            {
                let _ = transaction.abort();
                return;
            }
            wake.notify_threaded_runtime_owner();
            // Disk copying, validation, fsync and abort cleanup must all stay here.
            // Once activation is granted it finishes atomically; a later cancellation
            // suppresses result effects but does not pretend to roll back a committed index.
            let finalization = match finish_rx.recv() {
                Ok(IndexFinalization::Activate) => {
                    let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Progress(
                        MediaMatchToolProgress {
                            label: "Saving the Media Match index".to_owned(),
                            detail: None,
                            progress_fraction: 1.0,
                        },
                    ));
                    wake.notify_threaded_runtime_owner();
                    transaction.commit().map(Some)
                }
                Ok(IndexFinalization::Abort) | Err(_) => transaction
                    .abort()
                    .map(|()| None)
                    .map_err(MediaIndexCommitError::NotActivated),
            };
            let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished {
                result,
                finalization,
            });
            wake.notify_threaded_runtime_owner();
        })?;
    Ok((rx, finish_tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_media_match::MediaIndexService;
    use std::time::Duration;

    #[test]
    fn runtime_rejects_activation_after_root_or_settings_change() {
        use crate::app::runtime_owner::GuiPersistedConfigRuntimeOwner;
        use crate::app::runtime_state::GuiRuntimeState;
        for root_changed in [true, false] {
            let original = tempfile::tempdir().unwrap();
            let successor = tempfile::tempdir().unwrap();
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(
                original.path().join("sorotte.ini"),
            ));
            let mut state = GuiRuntimeState::from_stored_settings(&Default::default());
            let (tx, rx) = mpsc::channel();
            let (decision_tx, decisions) = mpsc::channel();
            owner.media_match_background_worker_rx = Some(rx);
            owner.media_match_background_finish_tx = Some(decision_tx);
            owner.media_match_background_scope = Some(IndexJobScope {
                root: original.path().to_owned(),
                settings: state.media_match.model.settings.clone(),
                player_path: None,
                room_target: None,
            });
            if root_changed {
                owner.config_path = Some(successor.path().join("sorotte.ini"));
            } else {
                state.media_match.model.settings.wire_sharing_enabled =
                    !state.media_match.model.settings.wire_sharing_enabled;
            }
            tx.send(GuiMediaMatchBackgroundWorkerEvent::AwaitingActivation {
                extraction_succeeded: true,
            })
            .unwrap();
            owner.pump_media_match_background_worker(
                &GuiQueuedRuntimeBridgeHandle::default(),
                &mut state,
            );
            assert!(matches!(
                decisions.try_recv().unwrap(),
                IndexFinalization::Abort
            ));
        }
    }

    #[test]
    fn completed_old_selection_cannot_publish_its_match_and_late_cancel_reports_committed_index() {
        use crate::app::runtime_owner::{
            GuiMediaMatchBackgroundCancelDisposition, GuiPersistedConfigRuntimeOwner,
        };
        use crate::app::runtime_state::GuiRuntimeState;
        for canceled in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(
                root.path().join("sorotte.ini"),
            ));
            let mut state = GuiRuntimeState::from_stored_settings(&Default::default());
            owner.media_match_background_scope = Some(IndexJobScope {
                root: root.path().to_owned(),
                settings: state.media_match.model.settings.clone(),
                player_path: Some("previous.mkv".to_owned()),
                room_target: Some("previous.mkv".to_owned()),
            });
            if canceled {
                owner.media_match_background_cancel_disposition =
                    Some(GuiMediaMatchBackgroundCancelDisposition::RestorePrevious);
            }
            let (tx, rx) = mpsc::channel();
            owner.media_match_background_worker_rx = Some(rx);
            let mut old_result = result();
            old_result.nearest_match = Some("old selection only".to_owned());
            old_result.last_evidence = Some("old selection evidence".to_owned());
            tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished {
                result: Ok(old_result),
                finalization: Ok(Some(MediaIndexCommitOutcome::Activated {
                    cleanup_warning: None,
                })),
            })
            .unwrap();
            owner.pump_media_match_background_worker(
                &GuiQueuedRuntimeBridgeHandle::default(),
                &mut state,
            );
            assert_eq!(owner.media_match_runtime_snapshot.nearest_match, None);
            assert_ne!(
                owner.media_match_runtime_snapshot.last_evidence.as_deref(),
                Some("old selection evidence")
            );
            if canceled {
                assert_eq!(
                    owner
                        .media_match_runtime_snapshot
                        .background_status
                        .as_deref(),
                    Some("canceled after activation: completed index kept")
                );
            }
        }
    }

    fn result() -> MediaMatchIndexRebuildResult {
        MediaMatchIndexRebuildResult {
            message: "indexed".to_owned(),
            cache_status: "indexed".to_owned(),
            current_decision: None,
            nearest_match: None,
            last_evidence: None,
        }
    }

    #[test]
    fn preparation_failure_reports_completion_and_removes_outer_staging_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("cache")).unwrap();
        let previous = root.path().join("cache/media-match");
        std::fs::write(&previous, b"obstructed index root").unwrap();
        let (rx, _finish) = spawn_index_worker(
            "test-index-prepare-failure",
            root.path().to_owned(),
            Arc::new(AtomicBool::new(false)),
            GuiQueuedRuntimeBridgeHandle::default(),
            |_, _| panic!("extraction must not run after preparation fails"),
        )
        .unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                GuiMediaMatchBackgroundWorkerEvent::Progress(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::Finished {
                    result: Err(error),
                    finalization: Err(MediaIndexCommitError::NotActivated(reason)),
                } => {
                    assert!(!error.is_empty());
                    assert_eq!(error, reason);
                    break;
                }
                _ => panic!("preparation failure must finish without asking for activation"),
            }
        }
        assert_eq!(std::fs::read(previous).unwrap(), b"obstructed index root");
        assert_eq!(
            std::fs::read_dir(root.path().join("cache"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn canceled_before_extraction_waits_for_abort_and_preserves_active_generation() {
        let root = tempfile::tempdir().unwrap();
        let initial = prepare_media_match_index_rebuild_backup(root.path()).unwrap();
        MediaIndexService::new(initial.staging_app_root().join("cache/media-match"))
            .open()
            .unwrap();
        initial.commit().unwrap();
        let manifest = root.path().join("cache/media-match/current.json");
        let previous = std::fs::read(&manifest).unwrap();
        let (rx, finish) = spawn_index_worker(
            "test-canceled-index",
            root.path().to_owned(),
            Arc::new(AtomicBool::new(true)),
            GuiQueuedRuntimeBridgeHandle::default(),
            |_, _| panic!("canceled job must not start extraction"),
        )
        .unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                GuiMediaMatchBackgroundWorkerEvent::Progress(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::AwaitingActivation {
                    extraction_succeeded: false,
                } => break,
                _ => panic!("cancellation must await the runtime's abort decision"),
            }
        }
        assert_eq!(std::fs::read(&manifest).unwrap(), previous);
        finish.send(IndexFinalization::Abort).unwrap();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            GuiMediaMatchBackgroundWorkerEvent::Finished {
                result: Err(error),
                finalization: Ok(None),
            } => assert!(error.contains("canceled before extraction")),
            _ => panic!("canceled extraction must finish without activation"),
        }
        assert_eq!(std::fs::read(manifest).unwrap(), previous);
        assert_eq!(
            std::fs::read_dir(root.path().join("cache"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn aborted_extraction_reports_failure_or_cancellation_and_retires_job() {
        use crate::app::runtime_owner::{
            GuiMediaMatchBackgroundCancelDisposition, GuiPersistedConfigRuntimeOwner,
        };
        use crate::app::runtime_state::GuiRuntimeState;
        for canceled in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(
                root.path().join("sorotte.ini"),
            ));
            let mut state = GuiRuntimeState::from_stored_settings(&Default::default());
            owner.media_match_background_trigger_key = Some("failed-job".to_owned());
            owner.media_match_background_worker_cancel = Some(Arc::new(AtomicBool::new(canceled)));
            if canceled {
                owner.media_match_background_cancel_disposition =
                    Some(GuiMediaMatchBackgroundCancelDisposition::RestorePrevious);
            }
            let (tx, rx) = mpsc::channel();
            owner.media_match_background_worker_rx = Some(rx);
            tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished {
                result: Err("extraction failed".to_owned()),
                finalization: Ok(None),
            })
            .unwrap();
            owner.pump_media_match_background_worker(
                &GuiQueuedRuntimeBridgeHandle::default(),
                &mut state,
            );
            assert_eq!(
                owner
                    .media_match_runtime_snapshot
                    .background_status
                    .as_deref(),
                Some(if canceled {
                    "canceled: previous index restored"
                } else {
                    "failed: previous index remains active"
                })
            );
            assert!(owner.media_match_background_worker_rx.is_none());
            assert!(owner.media_match_background_trigger_key.is_none());
            assert!(owner.media_match_background_worker_cancel.is_none());
            assert!(owner.media_match_background_cancel_disposition.is_none());
        }
    }

    #[test]
    fn disconnected_index_worker_retires_pending_job_and_reports_failure() {
        use crate::app::runtime_owner::GuiPersistedConfigRuntimeOwner;
        use crate::app::runtime_state::GuiRuntimeState;
        let root = tempfile::tempdir().unwrap();
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.path().join("sorotte.ini")));
        let mut state = GuiRuntimeState::from_stored_settings(&Default::default());
        let (tx, rx) = mpsc::channel();
        let (finish, _decisions) = mpsc::channel();
        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_finish_tx = Some(finish);
        owner.media_match_background_worker_cancel = Some(Arc::new(AtomicBool::new(false)));
        owner.media_match_background_trigger_key = Some("disconnected-job".to_owned());
        owner.media_match_background_scope = Some(IndexJobScope {
            root: root.path().to_owned(),
            settings: state.media_match.model.settings.clone(),
            player_path: None,
            room_target: None,
        });
        drop(tx);
        owner.pump_media_match_background_worker(
            &GuiQueuedRuntimeBridgeHandle::default(),
            &mut state,
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("failed: index worker stopped before reporting completion")
        );
        assert!(owner.media_match_background_worker_rx.is_none());
        assert!(owner.media_match_background_finish_tx.is_none());
        assert!(owner.media_match_background_scope.is_none());
        assert!(owner.media_match_background_worker_cancel.is_none());
        assert!(owner.media_match_background_trigger_key.is_none());
    }

    fn wait_staged(rx: &mpsc::Receiver<GuiMediaMatchBackgroundWorkerEvent>) {
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                GuiMediaMatchBackgroundWorkerEvent::Progress(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::AwaitingActivation {
                    extraction_succeeded,
                } => {
                    assert!(extraction_succeeded);
                    break;
                }
                GuiMediaMatchBackgroundWorkerEvent::Finished { .. } => {
                    panic!("finished without activation decision")
                }
            }
        }
    }

    #[test]
    fn staged_index_waits_for_runtime_activation_and_abort_cleans_stage() {
        let root = tempfile::tempdir().unwrap();
        let caller_thread = thread::current().id();
        let (rx, finish) = spawn_index_worker(
            "test-index-owner",
            root.path().to_owned(),
            Arc::new(AtomicBool::new(false)),
            GuiQueuedRuntimeBridgeHandle::default(),
            move |stage, _| {
                assert_ne!(thread::current().id(), caller_thread);
                MediaIndexService::new(stage.join("cache/media-match"))
                    .open()
                    .unwrap();
                Ok(result())
            },
        )
        .unwrap();
        wait_staged(&rx);
        assert!(!root.path().join("cache/media-match/current.json").exists());
        finish.send(IndexFinalization::Abort).unwrap();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            GuiMediaMatchBackgroundWorkerEvent::Finished { finalization, .. } => {
                assert!(matches!(finalization, Ok(None)))
            }
            _ => panic!("abort should finish without activation"),
        }
        assert!(
            std::fs::read_dir(root.path().join("cache"))
                .unwrap()
                .all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".media-match-build-")
                })
        );
    }

    #[test]
    fn dropped_activation_sender_aborts_worker_owned_stage() {
        let root = tempfile::tempdir().unwrap();
        let (rx, finish) = spawn_index_worker(
            "test-abandoned-index",
            root.path().to_owned(),
            Arc::new(AtomicBool::new(false)),
            GuiQueuedRuntimeBridgeHandle::default(),
            |stage, _| {
                MediaIndexService::new(stage.join("cache/media-match"))
                    .open()
                    .unwrap();
                Ok(result())
            },
        )
        .unwrap();
        wait_staged(&rx);
        drop(finish);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            GuiMediaMatchBackgroundWorkerEvent::Finished {
                finalization: Ok(None),
                ..
            }
        ));
        assert!(!root.path().join("cache/media-match/current.json").exists());
    }

    #[test]
    fn index_activation_lock_does_not_block_runtime_pump() {
        use super::super::GuiPersistedConfigRuntimeOwner;
        use crate::app::runtime_state::GuiRuntimeState;
        let root = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (rx, finish) = spawn_index_worker(
            "test-locked-index",
            root.path().to_owned(),
            cancel.clone(),
            GuiQueuedRuntimeBridgeHandle::default(),
            move |stage, _| {
                MediaIndexService::new(stage.join("cache/media-match"))
                    .open()
                    .unwrap();
                ready_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(result())
            },
        )
        .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(
                root.path()
                    .join("cache/media-match/.media-index-activation.lock"),
            )
            .unwrap();
        lock.lock().unwrap();
        release_tx.send(()).unwrap();
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.path().join("sorotte.ini")));
        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_finish_tx = Some(finish);
        owner.media_match_background_worker_cancel = Some(cancel);
        let (responsive_tx, responsive_rx) = mpsc::channel();
        let task = thread::spawn(move || {
            let mut state = GuiRuntimeState::from_stored_settings(&Default::default());
            let handle = GuiQueuedRuntimeBridgeHandle::default();
            while owner.media_match_background_finish_tx.is_some() {
                owner.pump_media_match_background_worker(&handle, &mut state);
                thread::sleep(Duration::from_millis(1));
            }
            owner.pump_media_match_background_worker(&handle, &mut state);
            responsive_tx
                .send(owner)
                .unwrap_or_else(|_| panic!("runtime should remain available"));
        });
        let response = responsive_rx.recv_timeout(Duration::from_secs(2));
        drop(lock);
        let mut owner = response.expect("runtime must return while activation is blocked on disk");
        task.join().unwrap();
        let rx = owner.media_match_background_worker_rx.take().unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                GuiMediaMatchBackgroundWorkerEvent::Progress(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::Finished { finalization, .. } => {
                    assert!(matches!(
                        finalization,
                        Ok(Some(MediaIndexCommitOutcome::Activated { .. }))
                    ));
                    break;
                }
                _ => panic!("activation was already accepted"),
            }
        }
        assert!(root.path().join("cache/media-match/current.json").is_file());
    }
}
