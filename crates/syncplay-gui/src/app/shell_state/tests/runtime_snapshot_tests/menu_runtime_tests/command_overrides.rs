use super::*;

#[test]
fn gui_shell_app_state_applies_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert_eq!(
        state.commands,
        GuiCommandAvailabilityState {
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
        }
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Room)));
    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_keeps_unrelated_command_flags_live_when_runtime_overrides_chat_send() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert!(!state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "0".to_owned(),
    }));

    assert!(!state.commands.can_send_chat_message);
    assert!(!state.commands.can_save_configuration);
    assert!(state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_chat_command_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        Some(false)
    );

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(false);
    let saved = state.saved_configuration.clone();
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        None
    );

    draft.chat_input_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::SaveConfiguration),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active."
        )
    );
}

#[test]
fn gui_shell_app_state_syncs_playback_menu_actions_from_gui_command_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: true,
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
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
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
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
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
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media Search")
            .is_some_and(|action| action.enabled && action.is_selected)
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
            .find(|action| action.label == "Shared Playlist")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
}
