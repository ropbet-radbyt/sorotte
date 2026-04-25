use super::*;

#[test]
fn gui_shell_app_state_tracks_configuration_text_edit_session_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.example".to_owned(),
    )));
    let rendered = state.render_lines().join("\n");
    assert!(
        rendered
            .contains("[Text Edit] editing=Connection / Host, dirty=yes, buffer=syncplay.example")
    );

    assert!(state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
    assert!(
        state
            .render_lines()
            .join("\n")
            .contains("[Text Edit] editing=(none)")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.cancelled".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
}

#[test]
fn gui_shell_app_state_tracks_focused_configuration_controls_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .autoplay_initial_state,
        Some(true)
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .text_edit_session
            .as_ref()
            .map(|session| session.label),
        Some("Host")
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    let rendered = state.render_lines().join("\n");
    assert!(
        rendered.contains("[Control Focus] focused=Connection / Host, kind=text, activations=1")
    );
    assert!(rendered.contains("[Text Edit] editing=Connection / Host"));

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );

    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert!(state.focused_configuration_control.is_none());
    assert!(!state.apply(GuiShellAction::ClearConfigurationControlFocus));
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_focus_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration control is currently focused.")
    );

    assert!(!state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Privacy",
        label: "Trusted Domain Count",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control is not focusable.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_text_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "OSD",
        label: "Show OSD",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control does not support text-edit sessions.")
    );

    assert!(!state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "orphan".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );

    assert!(!state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );
}

#[test]
fn gui_shell_app_state_tracks_pending_operations_and_busy_command_availability() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();

    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::RefreshPublicServers,
    )));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(!state.commands.can_reload_configuration);
    assert!(!state.commands.can_connect_public_server);
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_search_missing_media);
    assert!(!state.commands.can_toggle_pause);
    assert!(!state.commands.can_send_chat_message);

    let busy_render = state.render_lines().join("\n");
    assert!(busy_render.contains("[Commands] busy=yes"));
    assert!(busy_render.contains("[Pending] operation=refresh-public-servers"));

    assert!(!state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );

    assert!(state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(state.pending_operation, None);
    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(!state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No GUI operation is currently in progress.")
    );
}
