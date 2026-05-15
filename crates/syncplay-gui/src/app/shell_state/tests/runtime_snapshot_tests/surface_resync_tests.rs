use super::*;

#[test]
fn gui_shell_app_state_only_enables_media_open_after_runtime_support_arrives() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let initial_tree = state.shell_widget_tree();
    assert!(
        initial_tree
            .find("menus:action:0:0")
            .is_some_and(|node| !node.enabled)
    );
    assert!(
        initial_tree
            .find("shell:quick:open-media-file")
            .is_some_and(|node| !node.enabled)
    );
    assert!(GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(&state));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: TEST_USERNAME.to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["Episode 1".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        }
    )));

    let runtime_tree = state.shell_widget_tree();
    assert!(
        runtime_tree
            .find("menus:action:0:0")
            .is_some_and(|node| node.enabled)
    );
    assert!(
        runtime_tree
            .find("shell:quick:open-media-file")
            .is_some_and(|node| node.enabled)
    );
    assert!(GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(&state));
}

#[test]
fn gui_shell_app_state_resyncs_surfaces_from_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 0,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: None,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Readiness",
        label: "Shared Playlists",
        value: true,
    }));

    assert!(state.main_window.shared_playlist_enabled);
    assert!(!state.main_window.playback.can_manage_playlist);
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|item| item.label == "Show Playlist")
            .is_some_and(|item| item.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
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
            .find(|item| item.label == "Open Media File")
            .is_some_and(|item| !item.enabled && !item.is_selected)
    );
    assert!(
        file.actions
            .iter()
            .find(|item| item.label == "Open Media Search")
            .is_some_and(|item| item.enabled && item.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(playback.actions.iter().all(|item| !item.enabled));
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|item| item.label == "About")
            .is_some_and(|item| !item.enabled)
    );
}

#[test]
fn gui_shell_app_state_keeps_local_ready_transition_pending_until_runtime_matches() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(TEST_USERNAME.to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        room: Some("Room".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let runtime_snapshot = |is_ready| MainWindowRuntimeSnapshot {
        room_name: "Room".to_owned(),
        shared_playlist_enabled: false,
        controlled_room_active: false,
        users: vec![MainWindowRuntimeUserSnapshot {
            username: TEST_USERNAME.to_owned(),
            is_self: true,
            is_ready,
            is_controller: false,
            ..Default::default()
        }],
        playlist: vec!["episode1.mkv".to_owned()],
        chat: Vec::new(),
        can_toggle_pause: false,
        can_seek: false,
        can_set_ready: true,
        can_manage_playlist: false,
        playback_paused: false,
        autoplay_active: false,
        hide_empty_rooms: false,
        rooms: Vec::new(),
        ..Default::default()
    };

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(false),
    )));
    assert!(state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert_eq!(state.pending_local_ready_target, Some(true));
    assert!(state.displayed_local_main_window_user_ready());
    assert!(state.main_window.users[0].is_ready);

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(false),
    )));
    assert_eq!(state.pending_local_ready_target, Some(true));
    assert!(state.displayed_local_main_window_user_ready());
    assert!(!state.main_window.users[0].is_ready);

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(true),
    )));
    assert_eq!(state.pending_local_ready_target, None);
    assert!(state.displayed_local_main_window_user_ready());
    assert!(state.main_window.users[0].is_ready);
}

#[test]
fn gui_shell_app_state_replaces_pending_local_ready_target_before_runtime_matches() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(TEST_USERNAME.to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        room: Some("Room".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let runtime_snapshot = |is_ready| MainWindowRuntimeSnapshot {
        room_name: "Room".to_owned(),
        shared_playlist_enabled: false,
        controlled_room_active: false,
        users: vec![MainWindowRuntimeUserSnapshot {
            username: TEST_USERNAME.to_owned(),
            is_self: true,
            is_ready,
            is_controller: false,
            ..Default::default()
        }],
        playlist: Vec::new(),
        chat: Vec::new(),
        can_toggle_pause: false,
        can_seek: false,
        can_set_ready: true,
        can_manage_playlist: false,
        playback_paused: false,
        autoplay_active: false,
        hide_empty_rooms: false,
        rooms: Vec::new(),
        ..Default::default()
    };

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(false),
    )));
    assert!(state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert_eq!(state.pending_local_ready_target, Some(true));
    assert!(state.displayed_local_main_window_user_ready());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(false),
    )));
    assert_eq!(state.pending_local_ready_target, Some(true));
    assert!(state.displayed_local_main_window_user_ready());
    assert!(!state.main_window.users[0].is_ready);

    assert!(state.apply(GuiShellAction::AnnounceLocalUserNotReady));
    assert_eq!(state.pending_local_ready_target, Some(false));
    assert!(!state.displayed_local_main_window_user_ready());
    assert!(!state.main_window.users[0].is_ready);

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(true),
    )));
    assert_eq!(state.pending_local_ready_target, Some(false));
    assert!(!state.displayed_local_main_window_user_ready());
    assert!(state.main_window.users[0].is_ready);

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        runtime_snapshot(false),
    )));
    assert_eq!(state.pending_local_ready_target, None);
    assert!(!state.displayed_local_main_window_user_ready());
    assert!(!state.main_window.users[0].is_ready);
}

#[test]
fn gui_shell_app_state_preserves_runtime_main_window_surface_across_configuration_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
            playlist: vec!["Episode 1".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "bob".to_owned(),
                message: "synced".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: true,
                can_send_chat_message: true,
                chat_unavailable_reason: None,
            },
            pending_operation: None,
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
    assert_eq!(state.main_window.playlist[0].label, "Episode 1");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("synced")
    );
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_merges_runtime_main_window_users_with_configuration_room_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
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
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "MergedRoom".to_owned(),
    }));

    assert_eq!(state.main_window.room_name, "MergedRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}

#[test]
fn gui_shell_app_state_preserves_connected_room_surface_across_configuration_room_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: true,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: None,
            },
            pending_operation: None,
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "DraftRoom".to_owned(),
    }));

    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("DraftRoom")
    );
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}

#[test]
fn gui_shell_app_state_merges_runtime_main_window_users_with_configuration_runtime_room_updates() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
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
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.room = Some("RuntimeMergedRoom".to_owned());
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert_eq!(state.main_window.room_name, "RuntimeMergedRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}
