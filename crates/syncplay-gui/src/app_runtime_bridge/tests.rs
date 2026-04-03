use super::{GuiNativeRuntimeBridge, GuiPreviewRuntimeBridge};

use crate::app::testing::support::test_temp_root;
use crate::app::{
    GuiRuntimeRequest, GuiSavedConfigurationRuntimeSnapshot, GuiShellAction, GuiShellView,
    GuiTransientNotificationLevel, SyncplayGuiShellAppState,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_preview_runtime_bridge_maps_selected_media_files_to_preview_actions() {
    let shared_playlist_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let fallback_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut runtime = GuiPreviewRuntimeBridge;

    assert_eq!(
        runtime.actions_for_selected_media_files(
            &shared_playlist_state,
            vec![
                "C:/Media/Episode 1.mkv".to_owned(),
                "C:/Media/Episode 2.mkv".to_owned()
            ],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "Episode 1.mkv".to_owned(),
                "Episode 2.mkv".to_owned(),
            ]),
        ]
    );
    assert_eq!(
        runtime.actions_for_selected_media_files(
            &fallback_state,
            vec!["C:/Media/movie.mkv".to_owned()],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec!["movie.mkv".to_owned()]),
        ]
    );
    assert_eq!(
        runtime.dispatch_runtime_request(
            &fallback_state,
            GuiRuntimeRequest::SendChatMessage("preview hello".to_owned()),
        ),
        vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            message: "Chat sent.".to_owned(),
        }]
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
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
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
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            vec!["C:/Media/episode2.mkv".to_owned()],
            true,
            Some(1),
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
                "episode3.mkv".to_owned(),
            ]),
        ]
    );
}

#[test]
fn gui_preview_runtime_bridge_maps_pending_operations_to_preview_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(runtime.shows_manual_pending_controls());

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

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteLocalChatSend]
    );
    for action in runtime.actions_for_pending_completion(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert!(state.main_window.chat.is_empty());
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_cancel(&state).is_empty());
}

#[test]
fn gui_preview_runtime_bridge_saves_configuration_before_config_view_connect_completion() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "room2".to_owned(),
    }));
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert!(state.apply(GuiShellAction::BeginSavedServerConnect));
    assert!(state.pending_saved_server_connect_saves_configuration);
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                GuiSavedConfigurationRuntimeSnapshot {
                    settings: state.configuration.to_stored_settings(),
                },
            ),
            GuiShellAction::CompleteSavedServerConnect,
        ]
    );
}

#[test]
fn gui_preview_runtime_bridge_keeps_main_window_connect_as_plain_connect_completion() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(state.apply(GuiShellAction::BeginSavedServerConnect));
    assert!(!state.pending_saved_server_connect_saves_configuration);
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteSavedServerConnect]
    );
}
