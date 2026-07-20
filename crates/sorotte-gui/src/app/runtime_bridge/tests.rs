use super::{GuiNativeRuntimeBridge, GuiPreviewRuntimeBridge};

use crate::app::testing::support::test_temp_root;
use crate::app::{
    GuiPendingOperationKind, GuiPlexPlaylistJobCancellationReason, GuiRuntimeRequest,
    GuiSavedConfigurationRuntimeSnapshot, GuiSavedServerConnectIntent, GuiShellAction,
    GuiShellView, MainWindowPlaylistRow, SecretDraft, SettingId, SorotteGuiShellAppState,
};
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_runtime_request_debug_redacts_controller_password() {
    let secret = "gui-runtime-request-password-canary";
    let request = GuiRuntimeRequest::RequestControllerAuth {
        room: "room".to_owned(),
        password: secret.into(),
    };

    let debug = format!("{request:?}");
    assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
    assert!(!debug.contains(secret));
}

#[test]
fn gui_preview_runtime_bridge_maps_selected_media_files_to_preview_actions() {
    let root = test_temp_root("preview-selected-media-files");
    let episode1_path = root.join("Episode 1.mkv");
    let episode2_path = root.join("Episode 2.mkv");
    let movie_path = root.join("movie.mkv");
    for path in [&episode1_path, &episode2_path, &movie_path] {
        std::fs::write(path, b"test").expect("preview media fixture should be written");
    }
    let shared_playlist_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let fallback_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut runtime = GuiPreviewRuntimeBridge;

    assert_eq!(
        runtime.actions_for_selected_media_files(
            &shared_playlist_state,
            vec![
                episode1_path.to_string_lossy().into_owned(),
                episode2_path.to_string_lossy().into_owned(),
            ],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "Episode 1.mkv".to_owned(),
                "Episode 2.mkv".to_owned(),
            ]),
        ]
    );
    assert_eq!(
        runtime.actions_for_selected_media_files(
            &fallback_state,
            vec![movie_path.to_string_lossy().into_owned()],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec!["movie.mkv".to_owned()]),
        ]
    );
    assert_eq!(
        runtime.dispatch_runtime_request(
            &fallback_state,
            GuiRuntimeRequest::SendChatMessage("preview hello".to_owned()),
        ),
        Vec::new()
    );

    let _ = std::fs::remove_dir_all(root);
    assert_eq!(
        runtime.dispatch_runtime_request(
            &fallback_state,
            GuiRuntimeRequest::CancelPlexPlaylistJobs {
                reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
            },
        ),
        Vec::new()
    );
}

#[test]
fn gui_preview_runtime_bridge_imports_playlist_files_for_shared_playlist_ingest() {
    let root = test_temp_root("preview-shared-playlist-drop");
    let playlist_path = root.join("drop-list.m3u");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("preview playlist import fixture should be written");

    assert_eq!(
        GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            None,
            vec![playlist_path.to_string_lossy().into_owned()],
            true,
            None,
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "https://example.com/live".to_owned(),
            ]),
        ]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_preview_runtime_bridge_merges_shared_playlist_inserts_into_existing_rows() {
    let root = test_temp_root("preview-shared-playlist-insert");
    let media_path = root.join("episode2.mkv");
    std::fs::write(&media_path, b"test").expect("preview insert fixture should be written");
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "episode1.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ]))
    );

    assert_eq!(
        GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            Some(&state),
            vec![media_path.to_string_lossy().into_owned()],
            true,
            Some(1),
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
                "episode3.mkv".to_owned(),
            ]),
        ]
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gui_preview_runtime_bridge_maps_pending_operations_to_preview_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(runtime.shows_manual_pending_controls());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "draft.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteConfigurationSave(
            state.configuration.to_stored_settings(),
        )]
    );
    assert_eq!(
        runtime.actions_for_pending_cancel(&state),
        vec![GuiShellAction::CancelPendingOperation]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    state.main_window.playback.can_toggle_pause = true;
    state.main_window.playlist = vec![MainWindowPlaylistRow::inferred("episode1.mkv", false)];
    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompletePlaybackPauseToggle]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteSelectedPublicServerConnect]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    state.outgoing_chat_message = Some("hello".to_owned());
    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage
    )));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteLocalChatSend]
    );
    for action in runtime.actions_for_pending_completion(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(state.main_window.chat.len(), 1);
    assert_eq!(state.main_window.chat[0].message, "Chat pane ready");
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_cancel(&state).is_empty());
}

#[test]
fn gui_preview_runtime_bridge_saves_configuration_for_explicit_save_and_connect() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        server_password: Some("old-secret".into()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "room2".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "new-secret".to_owned().into(),
    }));
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert!(state.apply(GuiShellAction::BeginSaveAndConnect));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::SaveAndConnect)
    );
    let completion_actions = runtime.actions_for_pending_completion(&state);
    assert_eq!(
        completion_actions,
        vec![
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                GuiSavedConfigurationRuntimeSnapshot {
                    settings: state.configuration.to_stored_settings(),
                },
            ),
            GuiShellAction::CompleteSavedServerConnect,
        ]
    );
    for action in completion_actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.configuration.server_password, SecretDraft::Unchanged);
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::ConnectionServerPassword),
        Some("")
    );
    assert_eq!(state.saved_configuration.room.as_deref(), Some("room2"));
    assert_eq!(
        state
            .saved_configuration
            .server_password
            .as_ref()
            .map(|value| value.expose_secret()),
        Some("new-secret")
    );
    assert!(!state.has_unsaved_configuration_changes());
}

#[test]
fn gui_preview_runtime_bridge_connect_once_never_saves_the_draft() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "room2".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "new-secret".to_owned().into(),
    }));
    let saved_before_connect = state.saved_configuration.clone();
    assert!(state.apply(GuiShellAction::BeginConnectOnce));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::ConnectOnce)
    );
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteSavedServerConnect]
    );
    assert_eq!(state.saved_configuration, saved_before_connect);
}
