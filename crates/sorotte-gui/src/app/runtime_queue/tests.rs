use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiQueuedRuntimeBridge,
    GuiRuntimeThreadUnavailablePump,
};

use crate::app::{
    GuiMediaIndexRuntimeSnapshot, GuiPendingCompletionRequest, GuiPendingOperationKind,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiShellAction, GuiShellView,
    GuiTransientNotificationLevel, SorotteGuiShellAppState,
    native_host::GuiEframeNativeHost,
    runtime_bridge::GuiPreviewRuntimeOwner,
    runtime_queue::{
        GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwnerPump, GuiThreadedRuntimeOwnerPump,
    },
    testing::support::test_temp_root,
};
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_queued_runtime_bridge_and_preview_owner_cover_runtime_requests() {
    let media_root = test_temp_root("queued-runtime-preview-media");
    let movie_path = media_root.join("movie.mkv");
    let episode1_path = media_root.join("episode1.mkv");
    let episode2_path = media_root.join("episode2.mkv");
    for path in [&movie_path, &episode1_path, &episode2_path] {
        std::fs::write(path, b"test").expect("queued runtime media fixture should be written");
    }
    let movie_path_text = movie_path.to_string_lossy().into_owned();
    let episode1_path_text = episode1_path.to_string_lossy().into_owned();
    let episode2_path_text = episode2_path.to_string_lossy().into_owned();
    let (_host, host_handle) = GuiEframeNativeHost::with_queued_runtime();
    assert!(host_handle.drain_requests().is_empty());

    let (mut runtime, handle) = GuiQueuedRuntimeBridge::new();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!runtime.shows_manual_pending_controls());
    assert!(runtime.drain_runtime_actions().is_empty());
    assert!(handle.drain_requests().is_empty());

    let (preview_runtime, preview_handle) =
        GuiQueuedRuntimeBridge::new_with_manual_pending_controls(true);
    assert!(preview_runtime.shows_manual_pending_controls());
    let mut preview_pump =
        GuiQueuedRuntimeOwnerPump::new(preview_handle.clone(), GuiPreviewRuntimeOwner::default());
    GuiNativeRuntimePump::pump(&mut preview_pump, &state);
    preview_handle.push_request(GuiRuntimeRequest::SeekOffset(3.5));
    GuiNativeRuntimePump::pump(&mut preview_pump, &state);
    assert_eq!(
        preview_handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 3.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 3.5 seconds.".to_owned(),),
        ]
    );
    assert!(
        runtime
            .actions_for_room_join(&state, "joined-room".to_owned())
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SetRoom("joined-room".to_owned())]
    );
    assert!(
        runtime
            .actions_for_room_join(&state, "   ".to_owned())
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SetRoom("   ".to_owned())]
    );
    assert!(runtime.actions_for_room_leave(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ReturnToDefaultRoom]
    );

    handle.push_action(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Runtime callback queued.".to_owned(),
    });
    handle.push_action(GuiShellAction::AnnounceSystemChatEvent(
        "Runtime callback applied.".to_owned(),
    ));

    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Runtime callback queued.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Runtime callback applied.".to_owned(),),
        ]
    );
    assert!(runtime.drain_runtime_actions().is_empty());

    assert!(
        runtime
            .actions_for_selected_media_files(&state, Vec::new())
            .is_empty()
    );
    assert!(handle.drain_requests().is_empty());

    assert!(
        runtime
            .actions_for_selected_media_files(&state, vec![movie_path_text.clone()])
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![movie_path_text],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        }]
    );

    assert!(runtime.actions_for_seek_offset(12.5).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SeekOffset(12.5)]
    );
    assert!(
        runtime
            .actions_for_playlist_entry_commit(&state, "Episode 1.mkv".to_owned(), true)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::QueuePlaylistEntry {
            entry: "Episode 1.mkv".to_owned(),
            select_after_queue: true,
        }]
    );
    assert!(
        runtime
            .actions_for_playlist_activation(&state, 1)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SetPlaylistIndex(1)]
    );
    assert!(
        runtime
            .actions_for_playlist_entry_removal(&state, 0)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::DeletePlaylistIndex(0)]
    );
    assert!(
        runtime
            .actions_for_playlist_reorder(&state, vec!["One".to_owned(), "Two".to_owned()], Some(1))
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ReplacePlaylist {
            files: vec!["One".to_owned(), "Two".to_owned()],
            selected_index: Some(1),
        }]
    );
    assert!(runtime.actions_for_playlist_undo(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::UndoPlaylistChange]
    );
    assert!(
        runtime
            .actions_for_playlist_shuffle_remaining(&state)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ShuffleRemainingPlaylist]
    );
    assert!(
        runtime
            .actions_for_playlist_shuffle_entire(&state)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ShuffleEntirePlaylist]
    );
    assert!(
        runtime
            .dispatch_runtime_request(&state, GuiRuntimeRequest::TogglePlaybackPause)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::TogglePlaybackPause]
    );
    assert!(
        runtime
            .dispatch_runtime_request(&state, GuiRuntimeRequest::SeekToPosition(42.0))
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SeekToPosition(42.0)]
    );
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![episode1_path_text, episode2_path_text],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    assert_eq!(
        handle.drain_preview_response_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
            ]),
        ]
    );

    state.outgoing_chat_message = Some("hello".to_owned());
    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage
    )));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_cancel(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SendChatMessage("hello".to_owned())
            ),
            GuiRuntimeRequest::CancelPendingOperation(GuiPendingOperationKind::SendChatMessage),
        ]
    );
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    ));
    assert_eq!(
        handle.drain_preview_response_actions(),
        vec![
            GuiShellAction::CompleteLocalChatSend,
            GuiShellAction::CancelPendingOperation,
        ]
    );

    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(handle.drain_requests().is_empty());
    handle.push_action(GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
        GuiMediaIndexRuntimeSnapshot {
            active: true,
            message: Some("Indexing media 1/1: missing-target.mkv".to_owned()),
        },
    ));
    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
            GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some("Indexing media 1/1: missing-target.mkv".to_owned()),
            },
        )]
    );
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(handle.drain_requests().is_empty());
    handle.push_action(GuiShellAction::CompleteLocalChatSend);
    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![GuiShellAction::CompleteLocalChatSend]
    );
    assert!(state.apply(GuiShellAction::CompleteLocalChatSend));
    state.commands.can_search_missing_media = true;
    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia
        )]
    );
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia
        )]
    );
    handle.push_action(GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
        GuiMediaIndexRuntimeSnapshot {
            active: false,
            message: None,
        },
    ));
    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
            GuiMediaIndexRuntimeSnapshot {
                active: false,
                message: None,
            },
        )]
    );
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia
        )]
    );
    assert!(state.apply(GuiShellAction::CancelPendingOperation));
    state.outgoing_chat_message = Some("hello".to_owned());
    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage
    )));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SendChatMessage("hello".to_owned())
        )]
    );

    let _ = std::fs::remove_dir_all(media_root);
}

fn wait_for_runtime_actions(
    handle: &GuiQueuedRuntimeBridgeHandle,
    timeout: Duration,
) -> Vec<GuiShellAction> {
    let started_at = Instant::now();
    loop {
        let actions = handle.drain_actions();
        if !actions.is_empty() {
            return actions;
        }
        assert!(
            started_at.elapsed() < timeout,
            "timed out waiting for threaded runtime actions",
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until(timeout: Duration, description: &str, mut condition: impl FnMut() -> bool) {
    let started_at = Instant::now();
    while !condition() {
        assert!(
            started_at.elapsed() < timeout,
            "timed out waiting for {description}",
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn gui_threaded_runtime_owner_reconciles_changed_input_once_and_polls_repeatedly() {
    struct CountingRuntimeOwner {
        input_changes: Arc<AtomicUsize>,
        projection_rebuilds: Arc<AtomicUsize>,
        polls: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<GuiRuntimeRequest>>>,
    }

    impl GuiQueuedRuntimeOwner for CountingRuntimeOwner {
        fn input_changed(
            &mut self,
            _handle: &GuiQueuedRuntimeBridgeHandle,
            input: &crate::app::feature_slices::GuiRuntimeInput,
        ) {
            self.input_changes.fetch_add(1, Ordering::SeqCst);
            let _projection = input.to_compatibility_projection();
            self.projection_rebuilds.fetch_add(1, Ordering::SeqCst);
        }

        fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(handle.drain_requests());
        }
    }

    let input_changes = Arc::new(AtomicUsize::new(0));
    let projection_rebuilds = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        handle.clone(),
        CountingRuntimeOwner {
            input_changes: input_changes.clone(),
            projection_rebuilds: projection_rebuilds.clone(),
            polls: polls.clone(),
            requests: requests.clone(),
        },
        Duration::from_millis(5),
    )
    .expect("threaded runtime owner should spawn");
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    wait_until(Duration::from_secs(1), "initial input change", || {
        input_changes.load(Ordering::SeqCst) == 1
    });
    wait_until(Duration::from_secs(1), "repeated runtime polls", || {
        polls.load(Ordering::SeqCst) >= 3
    });

    let first_revision = threaded_pump
        .shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .latest_input_revision;
    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    state.active_view = GuiShellView::Room;
    state.new_main_window_user_draft = "UI-only draft".to_owned();
    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);

    let polls_before_wait = polls.load(Ordering::SeqCst);
    wait_until(
        Duration::from_secs(1),
        "polls after unchanged UI input",
        || polls.load(Ordering::SeqCst) >= polls_before_wait + 2,
    );
    let second_revision = threaded_pump
        .shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .latest_input_revision;
    assert_eq!(input_changes.load(Ordering::SeqCst), 1);
    assert_eq!(projection_rebuilds.load(Ordering::SeqCst), 1);
    assert_eq!(second_revision, first_revision);

    handle.push_request(GuiRuntimeRequest::SeekOffset(1.0));
    handle.push_request(GuiRuntimeRequest::UndoSeek);
    wait_until(Duration::from_secs(1), "ordered runtime commands", || {
        requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
            == 2
    });
    assert_eq!(
        *requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![
            GuiRuntimeRequest::SeekOffset(1.0),
            GuiRuntimeRequest::UndoSeek,
        ]
    );
    assert_eq!(input_changes.load(Ordering::SeqCst), 1);
    assert_eq!(projection_rebuilds.load(Ordering::SeqCst), 1);
}

#[test]
fn gui_threaded_runtime_owner_pump_wakes_immediately_for_requests_without_waiting_for_poll_timeout()
{
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        handle.clone(),
        GuiPreviewRuntimeOwner::default(),
        Duration::from_secs(30),
    )
    .expect("threaded runtime owner should spawn");

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    handle.push_request(GuiRuntimeRequest::SeekOffset(3.5));

    assert_eq!(
        wait_for_runtime_actions(&handle, Duration::from_millis(250)),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 3.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 3.5 seconds.".to_owned(),),
        ]
    );
}

#[test]
fn gui_threaded_runtime_owner_pump_joins_worker_on_drop() {
    struct DropAwareRuntimeOwner {
        dropped: Arc<AtomicBool>,
    }

    impl Drop for DropAwareRuntimeOwner {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl GuiQueuedRuntimeOwner for DropAwareRuntimeOwner {
        fn input_changed(
            &mut self,
            _handle: &GuiQueuedRuntimeBridgeHandle,
            _input: &crate::app::feature_slices::GuiRuntimeInput,
        ) {
        }

        fn poll(&mut self, _handle: &GuiQueuedRuntimeBridgeHandle) {}
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        GuiQueuedRuntimeBridgeHandle::default(),
        DropAwareRuntimeOwner {
            dropped: dropped.clone(),
        },
        Duration::from_millis(10),
    )
    .expect("threaded runtime owner should spawn");

    drop(threaded_pump);

    assert!(
        dropped.load(Ordering::SeqCst),
        "threaded runtime owner should be dropped after the worker shuts down",
    );
}

#[test]
fn gui_threaded_runtime_owner_pump_reuses_identical_runtime_inputs() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        GuiQueuedRuntimeBridgeHandle::default(),
        GuiPreviewRuntimeOwner::default(),
        Duration::from_secs(30),
    )
    .expect("threaded runtime owner should spawn");

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let first_snapshot = threaded_pump
        .last_submitted_input
        .clone()
        .expect("first pump should submit a state snapshot");

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let second_snapshot = threaded_pump
        .last_submitted_input
        .clone()
        .expect("second pump should keep a state snapshot");

    assert!(
        Arc::ptr_eq(&first_snapshot, &second_snapshot),
        "identical runtime inputs should reuse the existing worker snapshot",
    );
}

#[test]
fn gui_threaded_runtime_owner_pump_reuses_input_after_ui_only_changes() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        GuiQueuedRuntimeBridgeHandle::default(),
        GuiPreviewRuntimeOwner::default(),
        Duration::from_secs(30),
    )
    .expect("threaded runtime owner should spawn");

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let first_input = threaded_pump
        .last_submitted_input
        .clone()
        .expect("first pump should submit runtime input");
    let first_revision = threaded_pump
        .shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .latest_input_revision;

    state.active_view = GuiShellView::Room;
    state.new_main_window_user_draft = "UI-only draft".to_owned();
    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let second_input = threaded_pump
        .last_submitted_input
        .clone()
        .expect("second pump should retain runtime input");
    let second_revision = threaded_pump
        .shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .latest_input_revision;

    assert!(
        Arc::ptr_eq(&first_input, &second_input),
        "UI-only changes must not clone or resubmit runtime input",
    );
    assert_eq!(second_revision, first_revision);
}

#[test]
fn gui_runtime_thread_unavailable_pump_reports_startup_failure_and_drains_requests() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut pump = GuiRuntimeThreadUnavailablePump::new(handle.clone(), "spawn denied".to_owned());

    GuiNativeRuntimePump::pump(&mut pump, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: "Sorotte GUI runtime could not start: spawn denied. Runtime actions are disabled until Sorotte is restarted.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Sorotte GUI runtime could not start: spawn denied. Runtime actions are disabled until Sorotte is restarted.".to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(1.0));
    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    GuiNativeRuntimePump::pump(&mut pump, &state);

    assert!(handle.drain_requests().is_empty());
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message: "Ignored 2 runtime requests because the Sorotte GUI runtime is unavailable."
                .to_owned(),
        }]
    );
}

#[test]
fn gui_queued_runtime_bridge_handle_notifies_repaint_for_runtime_actions_only() {
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let repaint_notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    handle.set_repaint_notifier({
        let repaint_notifications = repaint_notifications.clone();
        move || {
            repaint_notifications.fetch_add(1, Ordering::SeqCst);
        }
    });

    handle.push_action(GuiShellAction::AnnounceSystemChatEvent(
        "Runtime callback applied.".to_owned(),
    ));
    handle.push_request(GuiRuntimeRequest::SeekOffset(1.0));
    handle.push_actions(Vec::<GuiShellAction>::new());

    assert_eq!(
        repaint_notifications.load(Ordering::SeqCst),
        1,
        "only runtime actions should trigger a repaint notification",
    );
}
