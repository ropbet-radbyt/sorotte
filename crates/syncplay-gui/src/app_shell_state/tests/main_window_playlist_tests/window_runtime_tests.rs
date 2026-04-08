use super::*;

#[test]
fn gui_shell_app_state_applies_main_window_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("InitialRoom".to_owned()),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["One".to_owned(), "Two".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "alice".to_owned(),
                message: "hello".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.playlist.len(), 2);
    assert!(state.notifications.is_empty());
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "+RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: true,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: true,
                    is_controller: true,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "carol".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["Two".to_owned(), "Three".to_owned()],
            chat: vec![
                MainWindowRuntimeChatSnapshot {
                    sender: "system".to_owned(),
                    message: "room sync".to_owned(),
                },
                MainWindowRuntimeChatSnapshot {
                    sender: "bob".to_owned(),
                    message: "ready".to_owned(),
                },
            ],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(state.main_window.room_name, "+RuntimeRoom");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.playback_paused);
    assert!(!state.main_window.autoplay_active);
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.main_window.users[0].username.as_str(), "bob");
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.main_window.playlist[0].label.as_str(), "Two");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("ready")
    );
}

#[test]
fn gui_shell_app_state_syncs_playback_menu_actions_from_main_window_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["One".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: true,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Playlist Actions")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: String::new(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: Vec::new(),
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots must include a non-empty room name.")
    );

    assert!(!state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "Alice".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots cannot contain duplicate user names.")
    );
}

#[test]
fn gui_shell_app_state_preserves_whitespace_room_names_in_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "   ".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                room_name: "   ".to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(state.main_window.room_name, "   ");
    assert_eq!(
        state
            .main_window
            .users
            .first()
            .map(|user| user.room_name.as_str()),
        Some("   ")
    );
}

#[test]
fn gui_shell_app_state_applies_full_gui_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Keep".to_owned(), "keep.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Existing".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "SeedRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["A".to_owned(), "B".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "system".to_owned(),
                message: "seed".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(0)));

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::PublicServers,
            open_modal: Some(GuiShellModal::UpdateNotice),
            main_window: MainWindowRuntimeSnapshot {
                room_name: "+LiveRoom".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: true,
                users: vec![
                    MainWindowRuntimeUserSnapshot {
                        username: "bob".to_owned(),
                        is_self: false,
                        is_ready: true,
                        is_controller: true,
                        ..Default::default()
                    },
                    MainWindowRuntimeUserSnapshot {
                        username: "carol".to_owned(),
                        is_self: false,
                        is_ready: false,
                        is_controller: false,
                        ..Default::default()
                    },
                ],
                playlist: vec!["B".to_owned(), "C".to_owned()],
                chat: vec![MainWindowRuntimeChatSnapshot {
                    sender: "bob".to_owned(),
                    message: "synced".to_owned(),
                }],
                can_toggle_pause: true,
                can_seek: false,
                can_set_ready: true,
                can_manage_playlist: true,
                playback_paused: true,
                autoplay_active: true,
                hide_empty_rooms: false,
                rooms: Vec::new(),
                ..Default::default()
            },
            public_servers: PublicServerBrowserShellState {
                servers: vec![
                    PublicServerBrowserRow {
                        label: "Alpha".to_owned(),
                        address: "alpha.example:8999".to_owned(),
                        is_selected: false,
                    },
                    PublicServerBrowserRow {
                        label: "Beta".to_owned(),
                        address: "beta.example:8999".to_owned(),
                        is_selected: true,
                    },
                ],
                can_connect: true,
                can_refresh: true,
                can_add_custom_server: true,
            },
            media_search: MediaSearchWorkflowShellState {
                directories: vec![
                    MediaSearchDirectoryRow {
                        path: "D:/Media".to_owned(),
                        is_selected: false,
                    },
                    MediaSearchDirectoryRow {
                        path: "E:/Library".to_owned(),
                        is_selected: true,
                    },
                ],
                can_browse_directories: true,
                can_search_missing_media: true,
                first_file_timeout_seconds: Some(1.0),
                search_timeout_seconds: Some(15.0),
                double_check_interval_seconds: Some(2.0),
                warning_threshold_seconds: Some(5.0),
            },
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));
    assert_eq!(state.main_window.room_name, "+LiveRoom");
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.main_window.users[0].username.as_str(), "bob");
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.main_window.playlist[0].label.as_str(), "B");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Playlist Actions")
            .is_some_and(|action| action.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "About")
            .is_some_and(|action| !action.enabled)
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("GUI runtime snapshot applied.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_full_gui_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::MainWindow,
            open_modal: None,
            main_window: MainWindowRuntimeSnapshot {
                room_name: String::new(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: Vec::new(),
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: false,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: Vec::new(),
                ..Default::default()
            },
            public_servers: PublicServerBrowserShellState {
                servers: Vec::new(),
                can_connect: false,
                can_refresh: true,
                can_add_custom_server: true,
            },
            media_search: MediaSearchWorkflowShellState {
                directories: Vec::new(),
                can_browse_directories: true,
                can_search_missing_media: false,
                first_file_timeout_seconds: None,
                search_timeout_seconds: None,
                double_check_interval_seconds: None,
                warning_threshold_seconds: None,
            },
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots must include a non-empty room name.")
    );
}
