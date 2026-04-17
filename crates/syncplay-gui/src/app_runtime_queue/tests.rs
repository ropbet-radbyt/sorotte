use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::{GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiQueuedRuntimeBridge};

use crate::app::{
    GuiPendingCompletionRequest, GuiPendingOperationKind, GuiQueuedRuntimeOwner, GuiRuntimeRequest,
    GuiShellAction, GuiShellView, GuiTransientNotificationLevel, SyncplayGuiShellAppState,
    native_host::GuiEframeNativeHost,
    runtime_bridge::GuiPreviewRuntimeOwner,
    runtime_queue::{
        GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwnerPump, GuiThreadedRuntimeOwnerPump,
    },
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_queued_runtime_bridge_and_preview_owner_cover_runtime_requests() {
    let (_host, host_handle) = GuiEframeNativeHost::with_queued_runtime();
    assert!(host_handle.drain_requests().is_empty());

    let (mut runtime, handle) = GuiQueuedRuntimeBridge::new();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        GuiQueuedRuntimeOwnerPump::new(preview_handle.clone(), GuiPreviewRuntimeOwner);
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
            .actions_for_selected_media_files(&state, vec!["C:/Media/movie.mkv".to_owned()])
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::OpenMediaFiles {
            paths: vec!["C:/Media/movie.mkv".to_owned()],
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
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    assert_eq!(
        handle.drain_preview_response_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
            ]),
        ]
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
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
    handle.push_action(GuiShellAction::CompleteLocalChatSend);
    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![GuiShellAction::CompleteLocalChatSend]
    );
    assert!(state.apply(GuiShellAction::CancelPendingOperation));
    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SendChatMessage("hello".to_owned())
        )]
    );
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

#[test]
fn gui_threaded_runtime_owner_pump_wakes_immediately_for_requests_without_waiting_for_poll_timeout()
{
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        handle.clone(),
        GuiPreviewRuntimeOwner,
        Duration::from_secs(30),
    );

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
        fn pump(
            &mut self,
            _handle: &GuiQueuedRuntimeBridgeHandle,
            _state: &SyncplayGuiShellAppState,
        ) {
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        GuiQueuedRuntimeBridgeHandle::default(),
        DropAwareRuntimeOwner {
            dropped: dropped.clone(),
        },
        Duration::from_millis(10),
    );

    drop(threaded_pump);

    assert!(
        dropped.load(Ordering::SeqCst),
        "threaded runtime owner should be dropped after the worker shuts down",
    );
}

#[test]
fn gui_threaded_runtime_owner_pump_reuses_identical_state_snapshots() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut threaded_pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        GuiQueuedRuntimeBridgeHandle::default(),
        GuiPreviewRuntimeOwner,
        Duration::from_secs(30),
    );

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let first_snapshot = threaded_pump
        .last_submitted_state
        .clone()
        .expect("first pump should submit a state snapshot");

    GuiNativeRuntimePump::pump(&mut threaded_pump, &state);
    let second_snapshot = threaded_pump
        .last_submitted_state
        .clone()
        .expect("second pump should keep a state snapshot");

    assert!(
        Arc::ptr_eq(&first_snapshot, &second_snapshot),
        "identical UI state submissions should reuse the existing runtime snapshot",
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
