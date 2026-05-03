use super::*;

#[test]
fn gui_shell_app_state_applies_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Room)));
    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: true,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: None,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.active_view, GuiShellView::Room);
    assert_eq!(state.selected_plugin, GuiPluginSelection::Plex);
    assert_eq!(state.main_window.room_name, "DraftRoom");
    assert!(state.commands.can_reset_configuration);
    assert!(!state.commands.can_toggle_pause);
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
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_main_window_surface_across_configuration_runtime_snapshots()
 {
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
                can_send_chat_message: false,
                chat_unavailable_reason: None,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
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
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(true);
    let mut saved = state.saved_configuration.clone();
    saved.chat_input_enabled = Some(false);

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}
