use super::*;

#[test]
fn gui_widget_egui_renderer_maps_configuration_tab_buttons_to_shell_actions() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let shell_tree = state.shell_widget_tree();
    assert!(shell_tree.find("main-window:tab:playlist").is_none());
    let configuration_privacy_tab = shell_tree.find("configuration:tab:privacy-chat").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, configuration_privacy_tab),
        vec![GuiShellAction::SelectConfigurationTab(
            GuiConfigurationTab::PrivacyChat,
        )]
    );
}

#[test]
fn gui_widget_egui_renderer_maps_plugin_enablement_checkboxes_to_shell_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let plugins = state.plugins_widget_tree();
    let stream_enabled = plugins
        .find("plugins:stream-support:enabled")
        .expect("stream enablement checkbox should exist");
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, stream_enabled, false),
        Some(GuiShellAction::SetPluginEnabled {
            plugin: GuiPluginSelection::StreamSupport,
            enabled: false,
        })
    );

    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    let plugins = state.plugins_widget_tree();
    let media_matching_enabled = plugins
        .find("plugins:media-matching:enabled")
        .expect("media matching enablement checkbox should exist");
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, media_matching_enabled, false),
        Some(GuiShellAction::SetPluginEnabled {
            plugin: GuiPluginSelection::MediaMatching,
            enabled: false,
        })
    );

    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    let plugins = state.plugins_widget_tree();
    let plex_enabled = plugins
        .find("plugins:plex:enabled")
        .expect("plex enablement checkbox should exist");
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, plex_enabled, false),
        Some(GuiShellAction::SetPluginEnabled {
            plugin: GuiPluginSelection::Plex,
            enabled: false,
        })
    );
}

#[test]
fn gui_widget_egui_renderer_maps_config_storage_buttons_and_external_override_state() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
            GuiConfigStorageRuntimeSnapshot {
                config_path: Some("C:/Sorotte/sorotte.ini".to_owned()),
                storage_root: Some("C:/Sorotte".to_owned()),
                default_storage_root: Some("C:/Users/test/AppData/Roaming/Sorotte".to_owned()),
                source_label: "custom config root".to_owned(),
                external_override_active: false,
            },
        ))
    );
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::InterfaceSystem,
    )));
    let tree = state.configuration_widget_tree();
    let default_button = tree.find("config-storage:root:default").unwrap();
    assert!(default_button.enabled);
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, default_button),
        vec![GuiShellAction::BeginConfigStorageDefaultReset]
    );
    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        "D:/PortableSorotte".to_owned(),
    )));
    assert_eq!(state.pending_operation, None);
    let tree = state.configuration_widget_tree();
    assert!(
        tree.find("config-command:save")
            .expect("Save button should exist")
            .enabled,
        "selecting a storage root should leave Save available"
    );
    assert_eq!(
        tree.find("config-storage:source")
            .expect("storage source should exist")
            .value
            .as_deref(),
        Some("selected custom root (save to apply)")
    );
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ChangeConfigStorageRoot)
    );
    assert!(state.apply(GuiShellAction::CancelConfigStorageRootChange));

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
            GuiConfigStorageRuntimeSnapshot {
                external_override_active: true,
                source_label: "SOROTTE_CLIENT_CONFIG_ROOT".to_owned(),
                ..state.config_storage.clone()
            },
        ))
    );
    let tree = state.configuration_widget_tree();
    assert!(
        !tree
            .find("config-storage:root:browse")
            .expect("storage Browse button should exist")
            .enabled
    );
    assert!(
        !tree
            .find("config-storage:root:default")
            .expect("storage Use Default button should exist")
            .enabled
    );
}

#[test]
fn gui_widget_egui_renderer_maps_surface_button_and_list_nodes_to_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            can_seek: true,
            can_undo_seek: true,
            can_set_offset: true,
            can_set_ready: true,
            can_set_others_ready: true,
            ..Default::default()
        }
    )));
    state.commands.can_disconnect_session = true;
    state.commands.can_send_chat_message = true;
    state.outgoing_chat_message = Some("hello ui".to_owned());
    assert!(state.apply(GuiShellAction::ToggleMainWindowRoomChange));
    state.commands.can_disconnect_session = true;
    state.commands.can_send_chat_message = true;
    let shell_tree = state.shell_widget_tree();
    let public_servers_surface = shell_tree.find("public-servers-root").unwrap();
    let plugins_surface = shell_tree.find("plugins-root").unwrap();
    let stream_plugin_row = shell_tree.find("plugins:list:stream-support").unwrap();
    let plex_plugin_row = shell_tree.find("plugins:list:plex").unwrap();
    let menu_action = shell_tree.find("menus:action:0:0").unwrap();
    let exit_menu_action = shell_tree.find("menus:action:0:3").unwrap();
    let seek_menu_action = shell_tree.find("menus:action:1:3").unwrap();
    let quick_open_media = shell_tree.find("shell:quick:open-media-file").unwrap();
    let playlist_row = shell_tree.find("main-window:playlist:0").unwrap();
    let room_toggle_button = shell_tree.find("main-window:room-actions:toggle").unwrap();
    let room_set_button = shell_tree.find("main-window:room:set").unwrap();
    let room_join_button = shell_tree.find("main-window:room:join").unwrap();
    let room_leave_button = shell_tree.find("main-window:room:leave").unwrap();
    let create_controlled_room_button = shell_tree
        .find("main-window:room-actions:create-controlled-room")
        .unwrap();
    let play_button = shell_tree.find("main-window:control:play").unwrap();
    let pause_button = shell_tree.find("main-window:control:pause").unwrap();
    let toggle_pause_button = shell_tree.find("main-window:control:toggle-pause").unwrap();
    let seek_button = shell_tree.find("main-window:control:seek").unwrap();
    let undo_seek_button = shell_tree.find("main-window:control:undo-seek").unwrap();
    let local_ready_button = shell_tree.find("main-window:control:set-ready").unwrap();
    let playlist_add_files_button = shell_tree.find("main-window:playlist:add-files").unwrap();
    let playlist_add_url_button = shell_tree.find("main-window:playlist:add-url").unwrap();
    let playlist_add_plex_button = shell_tree.find("main-window:playlist:add-plex").unwrap();
    let playlist_more_menu = shell_tree.find("main-window:playlist:more-menu").unwrap();
    let playlist_remove_button = shell_tree.find("main-window:playlist:0:remove").unwrap();
    let chat_send_button = shell_tree.find("main-window:chat:send").unwrap();
    assert!(
        shell_tree.find("main-window:control:open-url").is_none(),
        "Open URL should not be exposed from the Controls pane"
    );
    assert!(
        shell_tree.find("main-window:browser").is_none(),
        "the old separate room browser should not be projected"
    );
    assert!(
        shell_tree.find("main-window:room-group:1:join").is_none(),
        "other-room join actions should not be exposed from the combined current-room panel"
    );
    assert!(
        shell_tree.find("main-window:user:1:open").is_none(),
        "per-user open buttons should not be exposed from the combined room panel"
    );
    assert!(
        shell_tree.find("main-window:user:1:ready").is_none(),
        "per-user ready buttons should not be exposed from the combined room panel"
    );
    let edit_button = shell_tree.find("public-servers:command:edit").unwrap();
    let directory_remove_button = shell_tree.find("media-search:directory:remove").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::action_for_surface_node(public_servers_surface),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_surface_node(plugins_surface),
        Some(GuiShellAction::SwitchView(GuiShellView::Plugins))
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
        GuiWidgetEguiRenderer::action_for_list_item_node(stream_plugin_row),
        Some(GuiShellAction::SelectPlugin(
            GuiPluginSelection::StreamSupport,
        ))
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_list_item_node(plex_plugin_row),
        Some(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex))
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_toggle_button),
        vec![GuiShellAction::ToggleMainWindowRoomChange]
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
        GuiWidgetEguiRenderer::actions_for_button_node(&state, create_controlled_room_button),
        vec![GuiShellAction::BeginCreateControlledRoomEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, play_button),
        vec![GuiShellAction::BeginPlaybackResume]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, pause_button),
        vec![GuiShellAction::BeginPlaybackPause]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, toggle_pause_button),
        vec![GuiShellAction::BeginPlaybackPauseToggle]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, seek_button),
        vec![GuiShellAction::RequestSeekPrompt]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, undo_seek_button),
        vec![GuiShellAction::RequestPlaybackUndoSeek]
    );
    assert!(
        shell_tree.find("main-window:control:set-offset").is_none(),
        "Set Offset should not be exposed in the consolidated playlist controls"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, local_ready_button),
        vec![GuiShellAction::AnnounceLocalUserReady]
    );
    assert_eq!(playlist_add_files_button.kind, GuiWidgetKind::Button);
    assert_eq!(playlist_add_url_button.kind, GuiWidgetKind::Button);
    assert_eq!(playlist_add_plex_button.kind, GuiWidgetKind::Button);
    assert_eq!(playlist_more_menu.kind, GuiWidgetKind::Button);
    assert_eq!(
        playlist_more_menu
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:playlist:load", "main-window:playlist:save"]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_remove_button),
        vec![
            GuiShellAction::SelectMainWindowPlaylist(0),
            GuiShellAction::RemoveSelectedMainWindowPlaylist
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_add_plex_button),
        vec![
            GuiShellAction::BeginPlexPlaylistSearch,
            GuiShellAction::SubmitPlexPlaylistSearch {
                query: String::new()
            }
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, chat_send_button),
        vec![GuiShellAction::BeginLocalChatSend("hello ui".to_owned())]
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
                && password.expose_secret().len() == 10
                && password.expose_secret().chars().enumerate().all(|(index, c)| match index {
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert!(
        controller_auth_state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".into(),
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
                password: "ab-123-456".into(),
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
fn gui_widget_egui_renderer_keeps_local_ready_available_for_non_controller_empty_room() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.commands.can_disconnect_session = true;

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "+room1".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: true,
            users: vec![browser_runtime_user("alice", "+room1", true, false, false)],
            playlist: Vec::new(),
            can_set_ready: true,
            can_set_others_ready: false,
            can_manage_playlist: false,
            ..Default::default()
        }
    )));

    let tree = state.main_window_widget_tree();
    let ready_button = tree
        .find("main-window:control:set-ready")
        .expect("local Ready button should exist");
    assert!(
        ready_button.enabled,
        "local Ready should not depend on playlist or controller privileges"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, ready_button),
        vec![GuiShellAction::AnnounceLocalUserReady]
    );
}
