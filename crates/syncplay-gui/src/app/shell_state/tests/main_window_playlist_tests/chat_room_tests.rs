use super::*;

#[test]
fn gui_shell_app_state_adds_media_directory_and_pushes_chat_messages() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::PushChatMessage {
        sender: "system".to_owned(),
        message: "Connected".to_owned(),
    }));

    assert_eq!(state.media_search.directories.len(), 1);
    assert_eq!(state.media_search.directories[0].path, "C:/Media");
    assert!(state.media_search.directories[0].is_selected);
    assert!(state.media_search.can_search_missing_media);
    assert_eq!(state.main_window.chat.len(), 2);
    assert_eq!(state.main_window.chat[1].message, "Connected");

    let saved = state.configuration.to_stored_settings();
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned()])
    );
}

#[test]
fn gui_shell_app_state_tracks_local_and_remote_chat_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello world".to_owned(),)));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert!(
        state
            .render_lines()
            .join("\n")
            .contains("[Chat Send] pending_message=(none)")
    );
    assert!(state.apply(GuiShellAction::BeginLocalChatSend("again".to_owned(),)));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert_chat_pane_ready(&state.main_window.chat);
    assert!(state.notifications.is_empty());

    assert!(state.apply(GuiShellAction::AnnounceRemoteChatMessage {
        sender: "alice".to_owned(),
        message: "hi there".to_owned(),
    }));
    assert_eq!(
        state.main_window.chat.last().map(|row| row.sender.as_str()),
        Some("alice")
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("hi there")
    );

    assert!(state.apply(GuiShellAction::AnnounceSystemChatEvent(
        "Connection stabilized.".to_owned(),
    )));
    assert_eq!(
        state.main_window.chat.last().map(|row| row.sender.as_str()),
        Some("system")
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Connection stabilized.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_chat_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Chat input is disabled in Chat settings. The message was not sent.")
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.apply(GuiShellAction::BeginLocalChatSend("   ".to_owned())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Local chat messages must be non-empty.")
    );

    assert!(state.apply(GuiShellAction::CompleteLocalChatSend));
    assert_eq!(state.validation.last_action_error, None);

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert!(state.apply(GuiShellAction::BeginLocalChatSend("again".to_owned())));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert!(!state.apply(GuiShellAction::CancelLocalChatSend));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No local chat send is currently in progress.")
    );
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert!(state.notifications.is_empty());

    assert!(!state.apply(GuiShellAction::AnnounceRemoteChatMessage {
        sender: " ".to_owned(),
        message: "hi".to_owned(),
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Remote chat sender and message must both be non-empty.")
    );
}

#[test]
fn gui_shell_app_state_handles_text_edits_and_room_switches() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Username",
        value: TEST_USERNAME.to_owned(),
    }));
    assert!(state.apply(GuiShellAction::SetMainWindowRoom(
        "+room:ABCDEF123456".to_owned(),
    )));

    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.username.as_deref(), Some(TEST_USERNAME));
    assert_eq!(saved.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
}

#[test]
fn gui_shell_app_state_preserves_whitespace_room_names_in_text_edits_and_room_joins() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "  TeamRoom  ".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::SetMainWindowRoom("  TeamRoom  ".to_owned(),)));
    assert!(state.apply(GuiShellAction::JoinMainWindowRoom("   ".to_owned(),)));

    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.room.as_deref(), Some("  TeamRoom  "));
    assert_eq!(state.main_window.room_name, "  TeamRoom  ");
}

#[test]
fn gui_shell_app_state_normalizes_bare_controlled_room_names_from_saved_settings() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Test:77F8DA30FB3E".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+Test:77F8DA30FB3E")
    );
    assert_eq!(
        state.saved_configuration.room.as_deref(),
        Some("+Test:77F8DA30FB3E")
    );
    assert_eq!(state.main_window.room_name, "+Test:77F8DA30FB3E");
    assert!(state.main_window.controlled_room_active);
}

#[test]
fn gui_shell_app_state_preserves_controlled_room_auth_for_saved_connect_target() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("+Test:77F8DA30FB3E:RH-273-303".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+Test:77F8DA30FB3E:RH-273-303")
    );
    assert_eq!(
        state.saved_configuration.room.as_deref(),
        Some("+Test:77F8DA30FB3E:RH-273-303")
    );
    assert_eq!(state.main_window.room_name, "+Test:77F8DA30FB3E");

    let target = state
        .saved_session_connect_target()
        .expect("startup state should produce a saved connect target");
    assert_eq!(target.room, "+Test:77F8DA30FB3E");
    assert_eq!(
        target.controlled_room_password_override.as_deref(),
        Some("RH-273-303")
    );
}

#[test]
fn gui_shell_app_state_defers_room_join_and_leave_to_runtime_confirmation() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("+room:ABCDEF123456".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let baseline_chat_len = state.main_window.chat.len();
    let baseline_notification_len = state.notifications.len();

    assert!(state.apply(GuiShellAction::JoinMainWindowRoom(
        "+room:NEEDS_RUNTIME".to_owned(),
    )));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+room:ABCDEF123456")
    );
    assert_eq!(state.main_window.chat.len(), baseline_chat_len);
    assert_eq!(state.notifications.len(), baseline_notification_len);

    assert!(state.apply(GuiShellAction::LeaveMainWindowRoom));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+room:ABCDEF123456")
    );
    assert_eq!(state.main_window.chat.len(), baseline_chat_len);
    assert_eq!(state.notifications.len(), baseline_notification_len);
}

#[test]
fn gui_shell_app_state_rejects_invalid_room_status_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::JoinMainWindowRoom(String::new())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Room name cannot be empty.")
    );
    assert!(state.apply(GuiShellAction::JoinMainWindowRoom("   ".to_owned())));

    assert!(!state.apply(GuiShellAction::LeaveMainWindowRoom));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No joined room is currently active.")
    );
}
