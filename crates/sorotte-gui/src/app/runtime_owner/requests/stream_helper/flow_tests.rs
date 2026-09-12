use super::*;
use crate::app::{
    helper_tools::tests::tool_fixture, shell_state::SorotteGuiShellAppState,
    testing::support::runtime_state_for_shell,
};
use sorotte_client_app::app_boundary::state::StoredClientSettings;
use std::time::{Duration, Instant};

fn state() -> GuiRuntimeState {
    runtime_state_for_shell(&SorotteGuiShellAppState::from_stored_settings(
        &StoredClientSettings::default(),
    ))
}

fn drain(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut GuiRuntimeState,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while owner.stream_helper_worker.is_some() {
        owner.pump_stream_helper_worker(handle, state);
        assert!(Instant::now() < deadline, "helper operation did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn managed_imports_reprobe_the_selected_url_and_failed_replacement_keeps_working_tools() {
    let root = tempfile::tempdir().unwrap();
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.path().join("Sorotte.ini")));
    let mut state = state();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let target = "https://www.youtube.com/watch?v=selected";
    owner.pending_stream_retry_target = Some(target.to_owned());
    let downloader = tool_fixture(root.path(), "yt-dlp");
    assert!(owner.handle_integrate_stream_helper_downloader_request(
        &handle,
        &mut state,
        downloader.display().to_string()
    ));
    assert!(owner.stream_helper_remediation_runtime_snapshot.active);
    drain(&mut owner, &handle, &mut state);
    let runtime = tool_fixture(root.path(), "deno");
    assert!(owner.handle_integrate_stream_helper_js_runtime_request(
        &handle,
        &mut state,
        runtime.display().to_string()
    ));
    drain(&mut owner, &handle, &mut state);
    assert_eq!(
        owner.stream_helper_runtime_snapshot.health,
        GuiStreamHelperHealth::Healthy
    );
    assert_eq!(
        owner.stream_helper_runtime_snapshot.target.as_deref(),
        Some(target)
    );
    assert!(!owner.stream_helper_remediation_runtime_snapshot.active);
    assert!(owner.pending_stream_retry_target.is_none());
    let before = owner.stream_helper_runtime_snapshot.clone();
    let wrong = tool_fixture(root.path(), "wrong");
    assert!(owner.handle_integrate_stream_helper_downloader_request(
        &handle,
        &mut state,
        wrong.display().to_string()
    ));
    drain(&mut owner, &handle, &mut state);
    assert_eq!(owner.stream_helper_runtime_snapshot, before);
    assert!(handle.drain_actions().iter().any(|action| matches!(action,
        GuiShellAction::PushTransientNotification { level: GuiTransientNotificationLevel::Error, message }
        if message.contains("version banner"))));
    owner.pending_stream_retry_target = Some(target.to_owned());
    assert!(owner.handle_retry_pending_stream_media_open_request(&handle, &mut state));
    assert_eq!(
        owner.stream_helper_runtime_snapshot.health,
        GuiStreamHelperHealth::Checking
    );
    assert!(!owner.stream_helper_runtime_snapshot.retry_available);
    drain(&mut owner, &handle, &mut state);
    assert_eq!(
        owner.stream_helper_runtime_snapshot.health,
        GuiStreamHelperHealth::Healthy
    );
    assert!(owner.handle_recheck_stream_helper_request(&handle, &mut state));
    drain(&mut owner, &handle, &mut state);
    assert_eq!(
        owner.stream_helper_runtime_snapshot.health,
        GuiStreamHelperHealth::Healthy
    );
}

#[test]
fn disabled_or_unconfigured_remediation_starts_no_worker_and_retry_requires_a_target() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let mut state = state();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    assert!(!owner.handle_install_stream_helper_request(&handle, &mut state));
    assert!(!owner.handle_retry_pending_stream_media_open_request(&handle, &mut state));
    assert!(!owner.handle_open_stream_helper_install_location_request(&handle, &mut state));
    state
        .settings
        .plugin_enablement
        .set_enabled_for(GuiPluginSelection::StreamSupport, false);
    assert!(owner.handle_install_stream_helper_request(&handle, &mut state));
    assert!(owner.handle_recheck_stream_helper_request(&handle, &mut state));
    assert!(owner.handle_retry_pending_stream_media_open_request(&handle, &mut state));
    assert!(owner.stream_helper_worker.is_none());
    assert!(!owner.stream_helper_remediation_runtime_snapshot.active);
}

#[test]
fn completed_install_cannot_retry_a_replaced_playlist_and_root_change_discards_worker_output() {
    for root_changed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.path().join("Sorotte.ini")));
        let mut state = state();
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let target = "https://www.youtube.com/watch?v=old";
        owner.pending_stream_retry_target = Some(target.to_owned());
        let scope = owner.stream_helper_scope(Some(target));
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = HelperWorker::spawn(
            "finished-install-fixture",
            scope.root.clone(),
            move |_, tx| {
                tx.send(StreamHelperEvent::Progress(
                    StreamHelperRemediationProgress {
                        label: "Installing".to_owned(),
                        detail: None,
                        progress_fraction: 0.5,
                    },
                ))
                .unwrap();
                tx.send(StreamHelperEvent::Finished(Ok("Installed".to_owned())))
                    .unwrap();
                ready_tx.send(()).unwrap();
            },
        )
        .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        owner.stream_helper_worker = Some(StreamHelperWorker {
            worker,
            scope,
            installing: true,
        });
        // A second request must not replace an installation that owns its root.
        assert!(owner.handle_install_stream_helper_request(&handle, &mut state));
        owner.playlist_resolution.generation += 1;
        if root_changed {
            owner.config_path = Some(root.path().join("other/Sorotte.ini"));
        }
        assert!(!owner.pump_stream_helper_worker(&handle, &mut state));
        assert!(owner.stream_helper_worker.is_none());
        assert!(!owner.stream_helper_remediation_runtime_snapshot.active);
        if root_changed {
            assert!(!handle.drain_actions().iter().any(|action| matches!(
                action,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    ..
                }
            )));
        } else {
            assert!(owner.pending_stream_retry_target.is_none());
        }
    }
}

#[test]
fn worker_without_terminal_result_reports_failure_and_clears_progress() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let mut state = state();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let target = "https://www.youtube.com/watch?v=retry";
    owner.stream_helper_runtime_snapshot.target = Some(target.to_owned());
    owner.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Checking;
    let scope = owner.stream_helper_scope(Some(target));
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = HelperWorker::spawn("disconnected-helper-fixture", None, move |_, tx| {
        tx.send(StreamHelperEvent::Progress(
            StreamHelperRemediationProgress {
                label: "Working".to_owned(),
                detail: None,
                progress_fraction: 0.3,
            },
        ))
        .unwrap();
        drop(tx);
        ready_tx.send(()).unwrap();
    })
    .unwrap();
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    owner.stream_helper_worker = Some(StreamHelperWorker {
        worker,
        scope,
        installing: false,
    });
    assert!(!owner.pump_stream_helper_worker(&handle, &mut state));
    assert!(owner.stream_helper_worker.is_none());
    assert!(!owner.stream_helper_remediation_runtime_snapshot.active);
    assert_eq!(
        owner.stream_helper_runtime_snapshot.health,
        GuiStreamHelperHealth::Broken
    );
    assert!(owner.stream_helper_runtime_snapshot.retry_available);
    assert!(handle.drain_actions().iter().any(|action| matches!(action,
        GuiShellAction::PushTransientNotification { level: GuiTransientNotificationLevel::Error, message }
        if message.contains("stopped before reporting"))));
}

#[test]
fn media_tool_imports_publish_readiness_after_both_validated_workers_finish() {
    use crate::app::media_match_support::MediaMatchTool;
    let root = tempfile::tempdir().unwrap();
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.path().join("Sorotte.ini")));
    let mut state = state();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    for (tool, mode) in [
        (MediaMatchTool::Ffmpeg, "ffmpeg"),
        (MediaMatchTool::Ffprobe, "ffprobe"),
    ] {
        let path = tool_fixture(root.path(), mode);
        assert!(owner.handle_import_media_match_tool_request(
            &handle,
            &mut state,
            tool,
            path.display().to_string()
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        while owner.media_match_tool_worker_rx.is_some() {
            owner.pump_media_match_tool_worker(&handle, &mut state);
            assert!(
                Instant::now() < deadline,
                "media tool operation did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let actions = handle.drain_actions();
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    ..
                }
            )),
            "{tool:?} import actions: {actions:#?}"
        );
        let status = match tool {
            MediaMatchTool::Ffmpeg => &owner.media_match_runtime_snapshot.ffmpeg_status,
            MediaMatchTool::Ffprobe => &owner.media_match_runtime_snapshot.ffprobe_status,
        };
        assert!(
            status
                .as_deref()
                .is_some_and(|status| status.contains(&format!("{mode} version 8.0"))),
            "{tool:?} import snapshot: {:#?}",
            owner.media_match_runtime_snapshot
        );
    }
    assert!(
        owner
            .media_match_runtime_snapshot
            .ffmpeg_status
            .as_deref()
            .unwrap()
            .contains("ffmpeg version 8.0"),
        "{:#?}",
        owner.media_match_runtime_snapshot
    );
    assert!(
        owner
            .media_match_runtime_snapshot
            .ffprobe_status
            .as_deref()
            .unwrap()
            .contains("ffprobe version 8.0"),
        "{:#?}",
        owner.media_match_runtime_snapshot
    );
    assert!(!owner.media_match_remediation_runtime_snapshot.active);
}
