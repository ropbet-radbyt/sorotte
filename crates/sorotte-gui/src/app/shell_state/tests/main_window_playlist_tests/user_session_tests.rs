use super::*;

#[test]
fn gui_shell_app_state_announces_main_window_user_membership_events() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceMainWindowUserJoined(
        "alice".to_owned(),
    )));
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice joined the room.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User joined: alice.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceSelectedMainWindowUserRenamed(
            "alice-prime".to_owned(),
        ))
    );
    assert_eq!(state.main_window.users[1].username, "alice-prime");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice is now known as alice-prime.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User renamed: alice -> alice-prime.")
    );

    assert!(state.apply(GuiShellAction::AnnounceSelectedMainWindowUserLeft));
    assert_eq!(state.main_window.users.len(), 1);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice-prime left the room.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User removed: alice-prime.")
    );
}

#[test]
fn gui_shell_app_state_commits_native_add_drafts_and_playlist_appends_after_success() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::UpdateNewMainWindowUserDraft(
        "alice".to_owned(),
    )));
    assert_eq!(state.new_main_window_user_draft, "alice");
    assert!(state.apply(GuiShellAction::CommitNewMainWindowUser));
    assert_eq!(state.new_main_window_user_draft, "");
    assert_eq!(
        state
            .main_window
            .users
            .last()
            .map(|user| user.username.as_str()),
        Some("alice")
    );

    assert!(
        state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
            "Episode 1.mkv".to_owned(),
        ]))
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .last()
            .map(|row| row.label.as_str()),
        Some("Episode 1.mkv")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_user_announcement_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::AnnounceSelectedMainWindowUserRenamed(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Renamed main-window user names must be non-empty.")
    );

    assert!(!state.apply(GuiShellAction::AnnounceSelectedMainWindowUserLeft));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The local user row cannot be removed from the main-window shell.")
    );
}

#[test]
fn gui_shell_app_state_announces_playback_readiness_and_autoplay_events() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];

    assert!(state.apply(GuiShellAction::AnnouncePlaybackPaused));
    assert!(state.main_window.playback_paused);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Playback paused.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Playback paused.")
    );

    assert!(state.apply(GuiShellAction::AnnouncePlaybackResumed));
    assert!(!state.main_window.playback_paused);
    assert!(state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert!(state.main_window.users[0].is_ready);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("You are now marked ready.")
    );

    assert!(state.apply(GuiShellAction::AnnounceLocalUserNotReady));
    assert!(!state.main_window.users[0].is_ready);
    assert!(state.apply(GuiShellAction::AnnounceAutoplayState(true)));
    assert!(state.main_window.autoplay_active);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Autoplay enabled.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Autoplay enabled.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_playback_readiness_and_autoplay_events() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];

    assert!(!state.apply(GuiShellAction::AnnouncePlaybackPaused));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback pause state cannot change when pause controls are unavailable.")
    );

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];

    assert!(!state.apply(GuiShellAction::AnnouncePlaybackResumed));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback is already running.")
    );
    assert!(state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert!(!state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The local user is already marked ready.")
    );
    assert!(!state.apply(GuiShellAction::AnnounceAutoplayState(true)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Autoplay is already active.")
    );
}

#[test]
fn gui_shell_app_state_starts_controlled_room_and_controller_auth_edit_sessions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(
        state
            .controlled_room_create_session
            .as_ref()
            .is_some_and(|session| !session.is_dirty && session.room_buffer == "Lounge")
    );

    assert!(state.apply(GuiShellAction::UpdateCreateControlledRoomEdit(
        "Studio".to_owned(),
    )));
    assert!(
        state
            .controlled_room_create_session
            .as_ref()
            .is_some_and(|session| session.is_dirty && session.room_buffer == "Studio")
    );
    assert!(state.apply(GuiShellAction::CancelCreateControlledRoomEdit));
    assert!(state.controlled_room_create_session.is_none());

    assert!(state.apply(GuiShellAction::SetMainWindowRoom(
        "+Lounge:ABCDEF123456".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert!(
        state
            .controller_auth_edit_session
            .as_ref()
            .is_some_and(|session| {
                !session.is_dirty
                    && session.room_name == "+Lounge:ABCDEF123456"
                    && session.password_buffer.is_empty()
            })
    );

    assert!(
        state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    assert!(
        state
            .controller_auth_edit_session
            .as_ref()
            .is_some_and(|session| { session.is_dirty && session.password_buffer == "ab-123-456" })
    );
    assert!(state.apply(GuiShellAction::CancelControllerAuthEdit));
    assert!(state.controller_auth_edit_session.is_none());
}

#[test]
fn gui_shell_app_state_rejects_invalid_controlled_room_and_controller_auth_edit_sessions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("A joined room is required before creating a controlled room.")
    );

    assert!(!state.apply(GuiShellAction::UpdateCreateControlledRoomEdit(
        "Studio".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No controlled-room creation editor is currently active.")
    );

    assert!(!state.apply(GuiShellAction::CancelCreateControlledRoomEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No controlled-room creation editor is currently active.")
    );

    let mut joined_room_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(!joined_room_state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("Controller access can only be requested while a controlled room is active.")
    );

    assert!(
        !joined_room_state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("No controller-auth editor is currently active.")
    );

    assert!(!joined_room_state.apply(GuiShellAction::CancelControllerAuthEdit));
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("No controller-auth editor is currently active.")
    );
}

#[test]
fn gui_shell_app_state_renames_main_window_users_through_edit_sessions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("alice".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "alice-prime".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(state.main_window.users[1].username, "alice-prime");
    assert!(state.main_window_user_edit_session.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User renamed: alice -> alice-prime.")
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        TEST_USERNAME.to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(state.main_window.users[0].username, TEST_USERNAME);
    assert_eq!(
        state.configuration.to_stored_settings().username.as_deref(),
        Some(TEST_USERNAME)
    );
}

#[test]
fn gui_shell_app_state_remaps_main_window_user_edit_sessions_across_runtime_row_reorders() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("bob".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "bob-local".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
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

    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(0)
    );
    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.username_buffer.as_str()),
        Some("bob-local")
    );
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert!(state.main_window.users[0].is_selected);
}

#[test]
fn gui_shell_app_state_keeps_main_window_selection_on_the_active_user_edit_row() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("bob".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));

    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
    assert!(
        state
            .main_window_user_edit_session
            .as_ref()
            .is_some_and(|session| session.editing_index == 1)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_user_edit_sessions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "nobody".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No main-window user edit session is currently active.")
    );

    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit("   ".to_owned(),)));
    assert!(!state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Renamed main-window user names must be non-empty.")
    );

    assert!(state.apply(GuiShellAction::CancelMainWindowUserEdit));
    assert!(state.apply(GuiShellAction::AddMainWindowUser("alice".to_owned(),)));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit("You".to_owned(),)));
    assert!(!state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("A main-window user with that name already exists.")
    );
}

#[test]
fn gui_shell_app_state_tracks_cross_surface_selection_and_preserves_it_across_resync() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(0)));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 3,
        action_index: 1,
    }));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Username",
        value: TEST_USERNAME.to_owned(),
    }));

    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.selection.selected_menu_action, Some((3, 1)));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert!(state.main_window.users[0].is_selected);
    assert!(state.main_window.playlist[0].is_selected);
    assert!(state.menus.sections[3].actions[1].is_selected);
    assert!(!state.menus.sections[3].actions[0].is_selected);
    assert!(!state.media_search.directories[0].is_selected);
    assert!(state.media_search.directories[1].is_selected);

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Selection] user=0, playlist=0, menu=3:1, media_directory=1"));
}
