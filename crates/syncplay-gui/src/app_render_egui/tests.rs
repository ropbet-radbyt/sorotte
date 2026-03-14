use std::path::PathBuf;

use eframe::egui;
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

use super::GuiWidgetEguiRenderer;
use crate::app::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use crate::app::shell_state::{
    GuiDraftRuntimeSnapshot, GuiShellAction, GuiShellModal, GuiShellView,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    SyncplayGuiShellAppState,
};
use crate::app::testing::support::{TEST_USERNAME, browser_runtime_user};
use crate::app::widget_tree::GuiWidgetKind;

#[test]
fn gui_widget_egui_renderer_rebuilds_widget_tree_from_renderer_contract() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let expected_tree = state.shell_widget_tree();
    let mut renderer = GuiWidgetEguiRenderer::default();

    state.render_shell_widgets(&mut renderer);

    assert_eq!(renderer.root(), Some(&expected_tree));
}

#[test]
fn gui_widget_egui_renderer_exposes_modal_specific_titles_and_actions() {
    assert_eq!(
        GuiWidgetEguiRenderer::modal_window_title(GuiShellModal::TlsCertificatePrompt),
        "TLS Certificate Prompt"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::TlsCertificatePrompt),
        vec![
            (
                "shell:modal:tls:trust",
                "Trust Certificate",
                GuiShellAction::TrustTlsCertificatePrompt,
            ),
            (
                "shell:modal:tls:reject",
                "Reject Certificate",
                GuiShellAction::RejectTlsCertificatePrompt,
            ),
            (
                "shell:modal:tls:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::UpdateNotice),
        vec![
            (
                "shell:modal:update:dismiss",
                "Dismiss Notice",
                GuiShellAction::DismissUpdateNotice,
            ),
            (
                "shell:modal:update:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
            (
                "shell:modal:update:check-again",
                "Check Again",
                GuiShellAction::AnnounceUpdateNoticeAvailable,
            ),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::About),
        vec![
            (
                "shell:modal:about:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
            (
                "shell:modal:about:update",
                "Check for Updates",
                GuiShellAction::AnnounceUpdateNoticeAvailable,
            ),
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_selected_media_search_directory_for_native_browse_dialog() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("D:/AltMedia")
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_last_media_dialog_directory_for_native_browse_dialog() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.last_media_dialog_directory = Some("E:/Dialogs".to_owned());
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("E:/Dialogs")
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_search_browse_override_path_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_path_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH" => {
                Some("  C:/Smoke/Media Search  ".to_owned())
            }
            _ => None,
        }),
        Some("C:/Smoke/Media Search".to_owned())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_path_from_lookup(&|_name| None),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_file_pick_override_paths_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => {
                Some("  C:/Smoke/episode1.mkv | | D:/Alt/episode2.mp4  ".to_owned())
            }
            _ => None,
        }),
        Some(vec![
            "C:/Smoke/episode1.mkv".to_owned(),
            "D:/Alt/episode2.mp4".to_owned(),
        ])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|_name| None),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => Some("   |  ".to_owned()),
            _ => None,
        }),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_playlist_target_for_hovered_shared_playlist_drops() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode1.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request,
        GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec!["C:/Media/episode1.mkv".to_owned()],
        }
    );
}

#[test]
fn gui_widget_egui_renderer_falls_back_to_window_target_when_shared_playlist_drop_is_unavailable() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/movie.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request.target,
        GuiDroppedFilesTarget::Window,
        "non-shared-playlist drops should fall back to the generic window open path"
    );
}

#[test]
fn gui_widget_egui_renderer_maps_playlist_workflow_controls_to_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.main_window.playback.can_toggle_pause = true;

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    let shell_tree = state.shell_widget_tree();
    let add_url_button = shell_tree.find("main-window:playlist:add-url").unwrap();
    let open_url_button = shell_tree.find("main-window:playlist:open-url").unwrap();
    let open_selected_button = shell_tree
        .find("main-window:playlist:open-selected")
        .unwrap();
    let open_selected_folder_button = shell_tree
        .find("main-window:playlist:open-selected-folder")
        .unwrap();
    let trust_selected_button = shell_tree
        .find("main-window:playlist:trust-selected")
        .unwrap();
    let shuffle_remaining_button = shell_tree
        .find("main-window:playlist:shuffle-remaining")
        .unwrap();
    let shuffle_entire_button = shell_tree
        .find("main-window:playlist:shuffle-entire")
        .unwrap();
    let undo_button = shell_tree.find("main-window:playlist:undo").unwrap();
    let edit_button = shell_tree.find("main-window:playlist:edit").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, add_url_button),
        vec![GuiShellAction::BeginSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_url_button),
        vec![GuiShellAction::BeginMediaUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_selected_button),
        vec![GuiShellAction::RequestMainWindowUserMediaOpen(
            "https://example.com/live".to_owned()
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_selected_folder_button),
        vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
            "https://example.com/live".to_owned(),
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, trust_selected_button),
        vec![GuiShellAction::AddTrustedDomain("example.com".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, shuffle_remaining_button),
        vec![GuiShellAction::ShuffleRemainingSharedPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, shuffle_entire_button),
        vec![GuiShellAction::ShuffleEntireSharedPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, undo_button),
        vec![GuiShellAction::UndoSharedPlaylistChange]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, edit_button),
        vec![GuiShellAction::BeginSharedPlaylistTextEdit]
    );

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistTextEdit(
        "Episode 9.mkv\nhttps://example.com/live".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistUrlEdit(
        "https://example.com/extra".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));

    let shell_tree = state.shell_widget_tree();
    let playlist_text_node = shell_tree.find("main-window:playlist-edit:text").unwrap();
    let playlist_text_commit = shell_tree.find("main-window:playlist-edit:commit").unwrap();
    let playlist_text_cancel = shell_tree.find("main-window:playlist-edit:cancel").unwrap();
    let playlist_url_text_node = shell_tree
        .find("main-window:playlist-url-edit:text")
        .unwrap();
    let playlist_url_commit = shell_tree
        .find("main-window:playlist-url-edit:commit")
        .unwrap();
    let playlist_url_cancel = shell_tree
        .find("main-window:playlist-url-edit:cancel")
        .unwrap();
    let media_url_text_node = shell_tree.find("main-window:media-url-edit:text").unwrap();
    let media_url_cancel = shell_tree
        .find("main-window:media-url-edit:cancel")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_commit),
        vec![
            GuiShellAction::ReplaceSharedPlaylistEntries(vec![
                "Episode 9.mkv".to_owned(),
                "https://example.com/live".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistTextEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_cancel),
        vec![GuiShellAction::CancelSharedPlaylistTextEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_commit),
        vec![
            GuiShellAction::AppendSharedPlaylistEntries(vec![
                "https://example.com/extra".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistUrlEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_cancel),
        vec![GuiShellAction::CancelSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, media_url_cancel),
        vec![GuiShellAction::CancelMediaUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_text_node,
            "Episode 10.mkv",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
            "Episode 10.mkv".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_url_text_node,
            "https://example.com/final",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
            "https://example.com/final".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            media_url_text_node,
            "https://media.example/stream",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateMediaUrlEdit("https://media.example/stream".to_owned()),
            GuiShellAction::RequestMainWindowUserMediaOpen(
                "https://media.example/stream".to_owned(),
            ),
            GuiShellAction::CancelMediaUrlEdit,
        ])
    );
}

#[test]
fn gui_widget_egui_renderer_maps_surface_button_and_list_nodes_to_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Lounge".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: vec![
                MainWindowRuntimeRoomSnapshot {
                    room_name: "Lounge".to_owned(),
                    is_controlled: false,
                    has_named_users: true,
                },
                MainWindowRuntimeRoomSnapshot {
                    room_name: "Cinema".to_owned(),
                    is_controlled: false,
                    has_named_users: true,
                },
            ],
            users: vec![
                browser_runtime_user(TEST_USERNAME, "Lounge", true, false, false),
                MainWindowRuntimeUserSnapshot {
                    has_file: true,
                    file_name: Some("https://example.com/live".to_owned()),
                    file_is_url: true,
                    file_is_trusted: false,
                    ..browser_runtime_user("Bob", "Lounge", false, false, false)
                },
            ],
            playlist: vec!["Episode 1".to_owned()],
            can_toggle_pause: true,
            can_set_ready: true,
            can_set_others_ready: true,
            ..Default::default()
        }
    )));
    state.commands.can_disconnect_session = true;
    let shell_tree = state.shell_widget_tree();
    let public_servers_surface = shell_tree.find("public-servers-root").unwrap();
    let menu_action = shell_tree.find("menus:action:0:0").unwrap();
    let exit_menu_action = shell_tree.find("menus:action:0:3").unwrap();
    let seek_menu_action = shell_tree.find("menus:action:1:3").unwrap();
    let quick_open_media = shell_tree.find("shell:quick:open-media-file").unwrap();
    let playlist_row = shell_tree.find("main-window:playlist:0").unwrap();
    let browser_join_button = shell_tree.find("main-window:room-group:1:join").unwrap();
    let user_open_button = shell_tree.find("main-window:user:1:open").unwrap();
    let user_trust_button = shell_tree.find("main-window:user:1:trust").unwrap();
    let user_ready_button = shell_tree.find("main-window:user:1:ready").unwrap();
    let room_set_button = shell_tree.find("main-window:room:set").unwrap();
    let room_join_button = shell_tree.find("main-window:room:join").unwrap();
    let room_leave_button = shell_tree.find("main-window:room:leave").unwrap();
    let pause_button = shell_tree.find("main-window:control:toggle-pause").unwrap();
    let playlist_add_input = shell_tree.find("main-window:playlist:new").unwrap();
    let playlist_add_button = shell_tree.find("main-window:playlist:add").unwrap();
    let playlist_remove_button = shell_tree.find("main-window:playlist:remove").unwrap();
    let edit_button = shell_tree.find("public-servers:command:edit").unwrap();
    let directory_remove_button = shell_tree.find("media-search:directory:remove").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::action_for_surface_node(public_servers_surface),
        Some(GuiShellAction::SwitchView(GuiShellView::PublicServers))
    );
    assert!(GuiWidgetEguiRenderer::is_open_media_file_menu_action(
        &state,
        menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_exit_menu_action(
        &state,
        menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_open_media_file_menu_action(
        &state,
        exit_menu_action
    ));
    assert!(GuiWidgetEguiRenderer::is_exit_menu_action(
        &state,
        exit_menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_seek_menu_action(
        &state,
        menu_action
    ));
    assert!(GuiWidgetEguiRenderer::is_seek_menu_action(
        &state,
        seek_menu_action
    ));
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, menu_action),
        vec![
            GuiShellAction::SelectMenuAction {
                section_index: 0,
                action_index: 0,
            },
            GuiShellAction::TriggerSelectedMenuAction,
        ]
    );
    assert_eq!(quick_open_media.kind, GuiWidgetKind::Button);
    assert!(quick_open_media.enabled);
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_list_item_node(playlist_row),
        Some(GuiShellAction::SelectMainWindowPlaylist(0))
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, browser_join_button),
        vec![GuiShellAction::JoinMainWindowRoom("Cinema".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_open_button),
        vec![GuiShellAction::RequestMainWindowUserMediaOpen(
            "https://example.com/live".to_owned()
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_trust_button),
        vec![GuiShellAction::AddTrustedDomain("example.com".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_ready_button),
        vec![GuiShellAction::RequestMainWindowUserReady {
            username: "Bob".to_owned(),
            ready: true,
        }]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_set_button),
        vec![GuiShellAction::SetMainWindowRoom("Lounge".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_join_button),
        vec![GuiShellAction::JoinMainWindowRoom("Lounge".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_leave_button),
        vec![GuiShellAction::LeaveMainWindowRoom]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, pause_button),
        vec![GuiShellAction::BeginPlaybackPauseToggle]
    );
    assert_eq!(playlist_add_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_add_button),
        vec![GuiShellAction::CommitNewPlaylistEntry]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_remove_button),
        vec![GuiShellAction::RemoveSelectedMainWindowPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, edit_button),
        vec![GuiShellAction::BeginEditSelectedPublicServer]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("C:/Media")
    );
    assert!(GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "save-configuration",
        true
    ));
    assert!(!GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "save-configuration",
        false
    ));
    assert!(!GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "(none)", true
    ));
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, directory_remove_button),
        vec![GuiShellAction::RemoveSelectedMediaSearchDirectory]
    );

    let mut controlled_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controlled_room_state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    let controlled_room_tree = controlled_room_state.main_window_widget_tree();
    let create_commit_button = controlled_room_tree
        .find("main-window:controlled-room-create:commit")
        .unwrap();
    let create_cancel_button = controlled_room_tree
        .find("main-window:controlled-room-create:cancel")
        .unwrap();
    let create_actions = GuiWidgetEguiRenderer::actions_for_button_node(
        &controlled_room_state,
        create_commit_button,
    );
    assert_eq!(create_actions.len(), 2);
    assert!(matches!(
        &create_actions[0],
        GuiShellAction::RequestControllerAuth { room, password }
            if room == "Lounge"
                && password.len() == 10
                && password.chars().enumerate().all(|(index, c)| match index {
                    2 | 6 => c == '-',
                    0 | 1 => c.is_ascii_uppercase(),
                    _ => c.is_ascii_digit(),
                })
    ));
    assert_eq!(
        create_actions[1],
        GuiShellAction::CancelCreateControlledRoomEdit
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controlled_room_state,
            create_cancel_button
        ),
        vec![GuiShellAction::CancelCreateControlledRoomEdit]
    );

    let mut controller_auth_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert!(
        controller_auth_state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    let controller_auth_tree = controller_auth_state.main_window_widget_tree();
    let controller_auth_commit_button = controller_auth_tree
        .find("main-window:controller-auth:commit")
        .unwrap();
    let controller_auth_cancel_button = controller_auth_tree
        .find("main-window:controller-auth:cancel")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controller_auth_state,
            controller_auth_commit_button
        ),
        vec![
            GuiShellAction::RequestControllerAuth {
                room: "+Lounge:ABCDEF123456".to_owned(),
                password: "ab-123-456".to_owned(),
            },
            GuiShellAction::CancelControllerAuthEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controller_auth_state,
            controller_auth_cancel_button
        ),
        vec![GuiShellAction::CancelControllerAuthEdit]
    );
}

#[test]
fn gui_widget_egui_renderer_maps_text_and_checkbox_edits_to_actions() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let configuration_tree = state.configuration_widget_tree();
    let host = configuration_tree.find("config:Connection:Host").unwrap();
    let autoplay = configuration_tree
        .find("config:Readiness:Autoplay")
        .unwrap();
    let trusted_domains = configuration_tree
        .find("config:Privacy:Trusted Domains")
        .unwrap();
    let unpause_action = configuration_tree
        .find("config:Readiness:Unpause Action")
        .unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            host,
            "syncplay.example",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Host",
            value: "syncplay.example".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, autoplay, true),
        Some(GuiShellAction::EditConfigurationBool {
            section: "Readiness",
            label: "Autoplay",
            value: true,
        })
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            trusted_domains,
            "youtube.com; *.example.com/videos",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Privacy",
            label: "Trusted Domains",
            value: "youtube.com; *.example.com/videos".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::configuration_select_options_for_node(&state, unpause_action),
        Some(vec![
            "IfAlreadyReady".to_owned(),
            "IfOthersReady".to_owned(),
            "IfMinUsersReady".to_owned(),
            "Always".to_owned(),
        ])
    );

    let chat_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let chat_tree = chat_state.main_window_widget_tree();
    let chat_input = chat_tree.find("main-window:chat-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &chat_state,
            chat_input,
            "Hello world",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("Hello world".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("Hello world".to_owned()),
        ])
    );

    let room_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let room_tree = room_state.main_window_widget_tree();
    let room_input = room_tree.find("main-window:room-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            room_input,
            "TeamRoom",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: "TeamRoom".to_owned(),
            },
            GuiShellAction::JoinMainWindowRoom("TeamRoom".to_owned()),
        ])
    );

    assert!(room_tree.find("main-window:user:new").is_none());

    let add_playlist_input = room_tree.find("main-window:playlist:new").unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            add_playlist_input,
            "Episode 1.mkv",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateNewPlaylistEntryDraft("Episode 1.mkv".to_owned()),
            GuiShellAction::CommitNewPlaylistEntry,
        ])
    );

    let mut user_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(user_state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(user_state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(user_state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    let user_tree = user_state.main_window_widget_tree();
    assert!(user_tree.find("main-window:user-edit:username").is_none());

    let mut controlled_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controlled_room_state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    let controlled_room_tree = controlled_room_state.main_window_widget_tree();
    let controlled_room_input = controlled_room_tree
        .find("main-window:controlled-room-create:room")
        .unwrap();
    let controlled_room_actions = GuiWidgetEguiRenderer::actions_for_text_input_node(
        &controlled_room_state,
        controlled_room_input,
        "Studio",
        true,
        true,
    )
    .expect("controlled-room input should map edits");
    assert_eq!(controlled_room_actions.len(), 3);
    assert_eq!(
        controlled_room_actions[0],
        GuiShellAction::UpdateCreateControlledRoomEdit("Studio".to_owned())
    );
    assert!(matches!(
        &controlled_room_actions[1],
        GuiShellAction::RequestControllerAuth { room, password }
            if room == "Studio"
                && password.len() == 10
                && password.chars().enumerate().all(|(index, c)| match index {
                    2 | 6 => c == '-',
                    0 | 1 => c.is_ascii_uppercase(),
                    _ => c.is_ascii_digit(),
                })
    ));
    assert_eq!(
        controlled_room_actions[2],
        GuiShellAction::CancelCreateControlledRoomEdit
    );

    let mut controller_auth_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    let controller_auth_tree = controller_auth_state.main_window_widget_tree();
    let controller_auth_input = controller_auth_tree
        .find("main-window:controller-auth:password")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &controller_auth_state,
            controller_auth_input,
            "ab-123-456",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateControllerAuthPasswordEdit("ab-123-456".to_owned()),
            GuiShellAction::RequestControllerAuth {
                room: "+Lounge:ABCDEF123456".to_owned(),
                password: "ab-123-456".to_owned(),
            },
            GuiShellAction::CancelControllerAuthEdit,
        ])
    );
}
