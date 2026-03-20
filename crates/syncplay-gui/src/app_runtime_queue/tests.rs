use super::{GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiQueuedRuntimeBridge};

use crate::app::{
    GuiPendingCompletionRequest, GuiPendingOperationKind, GuiRuntimeRequest, GuiShellAction,
    GuiShellView, GuiTransientNotificationLevel, SyncplayGuiShellAppState,
    native_host::GuiEframeNativeHost, runtime_bridge::GuiPreviewRuntimeOwner,
    runtime_queue::GuiQueuedRuntimeOwnerPump,
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
            .actions_for_playlist_selection_change(&state, 1)
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
}
