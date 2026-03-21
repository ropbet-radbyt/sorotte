use super::{GuiLayoutMode, GuiWidgetRenderer};

use crate::app::{
    GuiMediaIndexRuntimeSnapshot, GuiShellAction, GuiShellModal, GuiShellView,
    GuiTransientNotificationLevel, GuiWidgetKind, GuiWidgetNode, MainWindowRuntimeSnapshot,
    SyncplayGuiShellAppState,
};

use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_shell_app_state_projects_configuration_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "widget.example".to_owned(),
    )));

    let tree = state.configuration_widget_tree();
    let host = tree
        .find("config:Connection:Host")
        .expect("host control should exist in widget tree");
    assert_eq!(host.kind, GuiWidgetKind::TextInput);
    assert_eq!(host.value.as_deref(), Some("widget.example"));
    assert!(host.enabled);
    assert!(host.selected);
    let player_arguments = tree
        .find("config:Connection:Player Arguments")
        .expect("player-arguments control should exist in widget tree");
    assert_eq!(player_arguments.kind, GuiWidgetKind::TextInput);
    assert!(!player_arguments.enabled);
    let room_history = tree
        .find("config:Connection:Room History")
        .expect("room-history control should exist in widget tree");
    assert_eq!(room_history.kind, GuiWidgetKind::TextArea);
    let trusted_domains = tree
        .find("config:Privacy:Trusted Domains")
        .expect("trusted-domains control should exist in widget tree");
    assert_eq!(trusted_domains.kind, GuiWidgetKind::TextArea);

    let save = tree
        .find("config-command:save")
        .expect("save command should exist in widget tree");
    assert_eq!(save.kind, GuiWidgetKind::Button);
    assert!(save.enabled);
}

#[test]
fn gui_shell_app_state_projects_main_window_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "hello widget".to_owned(),
    )));

    let tree = state.main_window_widget_tree();
    let browser = tree
        .find("main-window:browser")
        .expect("room browser should exist in widget tree");
    assert_eq!(browser.kind, GuiWidgetKind::Panel);
    let room_group = tree
        .find("main-window:room-group:0")
        .expect("current room group should exist in widget tree");
    assert_eq!(room_group.kind, GuiWidgetKind::Panel);
    let room_group_state = tree
        .find("main-window:room-group:0:state")
        .expect("room-group state should exist in widget tree");
    assert_eq!(room_group_state.kind, GuiWidgetKind::Status);
    let user_state = tree
        .find("main-window:user:1:state")
        .expect("selected user state should exist in widget tree");
    assert_eq!(user_state.kind, GuiWidgetKind::Status);
    assert!(user_state.selected);
    assert!(tree.find("main-window:user:new").is_none());
    let room_input = tree
        .find("main-window:room-input")
        .expect("room input should exist in widget tree");
    assert_eq!(room_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(room_input.value.as_deref(), Some("Lounge"));
    assert!(!room_input.enabled);
    let room_control = tree
        .find("main-window:room-control")
        .expect("room-control status should exist in widget tree");
    assert_eq!(room_control.kind, GuiWidgetKind::Status);
    assert_eq!(
        room_control.value.as_deref(),
        Some("Unavailable: no active server session.")
    );

    let playlist = tree
        .find("main-window:playlist:1")
        .expect("selected playlist row should exist in widget tree");
    assert_eq!(playlist.kind, GuiWidgetKind::ListItem);
    assert!(playlist.selected);
    let new_playlist = tree
        .find("main-window:playlist:new")
        .expect("new playlist input should exist in widget tree");
    assert_eq!(new_playlist.kind, GuiWidgetKind::TextInput);
    assert_eq!(new_playlist.value.as_deref(), Some(""));
    let playlist_add = tree
        .find("main-window:playlist:add")
        .expect("playlist add button should exist in widget tree");
    assert_eq!(playlist_add.kind, GuiWidgetKind::Button);
    assert!(!playlist_add.enabled);
    assert!(tree.find("main-window:user:add").is_none());

    let chat_input = tree
        .find("main-window:chat-input")
        .expect("chat input should exist in widget tree");
    assert_eq!(chat_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(chat_input.value.as_deref(), Some("hello widget"));
    assert_eq!(chat_input.enabled, state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_projects_runtime_room_control_status_into_main_window_widget_tree() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.room_name = "+room1".to_owned();
    snapshot.controlled_room_active = true;
    snapshot.room_control_status = "Not granted by server: room controls are locked.".to_owned();

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:room-control")
            .and_then(|node| node.value.as_deref()),
        Some("Not granted by server: room controls are locked.")
    );
}

#[test]
fn gui_shell_app_state_projects_menu_dialog_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::AnnounceAboutDialogRequested));

    let tree = state.menu_dialog_widget_tree();
    let pause = tree
        .find("menus:action:1:0")
        .expect("playback toggle action should exist");
    assert_eq!(pause.kind, GuiWidgetKind::Button);
    assert!(!pause.enabled);
    assert!(pause.selected);

    let about = tree
        .find("menus:dialog:about")
        .expect("about dialog status should exist");
    assert_eq!(about.kind, GuiWidgetKind::Status);
    assert!(about.enabled);
    assert!(about.selected);
    assert_eq!(about.value.as_deref(), Some("yes"));
}

#[test]
fn gui_shell_app_state_projects_public_server_and_media_search_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(3.0),
        folder_search_timeout_seconds: Some(30.0),
        folder_search_double_check_interval_seconds: Some(2.5),
        folder_search_warning_threshold_seconds: Some(7.5),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Beta".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "beta.example:9000".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(0)));

    let server_tree = state.public_server_widget_tree();
    let row = server_tree
        .find("public-servers:row:0")
        .expect("public server row should exist");
    assert_eq!(row.kind, GuiWidgetKind::ListItem);
    assert!(row.selected);
    assert_eq!(row.value.as_deref(), Some("alpha.example:8999"));

    let edit_label = server_tree
        .find("public-servers:edit:label")
        .expect("public server edit label should exist");
    assert_eq!(edit_label.kind, GuiWidgetKind::TextInput);
    assert_eq!(edit_label.value.as_deref(), Some("Beta"));
    let edit_button = server_tree
        .find("public-servers:command:edit")
        .expect("public server edit command should exist");
    assert_eq!(edit_button.kind, GuiWidgetKind::Button);
    assert!(!edit_button.enabled);
    let commit_button = server_tree
        .find("public-servers:edit:commit")
        .expect("public server edit commit should exist");
    assert_eq!(commit_button.kind, GuiWidgetKind::Button);
    assert!(commit_button.enabled);
    let cancel_button = server_tree
        .find("public-servers:edit:cancel")
        .expect("public server edit cancel should exist");
    assert_eq!(cancel_button.kind, GuiWidgetKind::Button);
    assert!(cancel_button.enabled);

    let media_tree = state.media_search_widget_tree();
    let directory = media_tree
        .find("media-search:directory:0")
        .expect("media search directory should exist");
    assert_eq!(directory.kind, GuiWidgetKind::ListItem);
    assert!(directory.selected);

    let search = media_tree
        .find("media-search:command:search")
        .expect("media search command should exist");
    assert_eq!(search.kind, GuiWidgetKind::Button);
    assert!(search.enabled);
    let remove = media_tree
        .find("media-search:directory:remove")
        .expect("media-search remove command should exist");
    assert_eq!(remove.kind, GuiWidgetKind::Button);
    assert!(remove.enabled);
    let first_file_timing = media_tree
        .find("media-search:timing:first-file")
        .expect("media-search first-file timing status should exist");
    assert_eq!(first_file_timing.value.as_deref(), Some("3.00s"));
    let search_timing = media_tree
        .find("media-search:timing:search")
        .expect("media-search search timing status should exist");
    assert_eq!(search_timing.value.as_deref(), Some("30.00s"));
    let double_check_timing = media_tree
        .find("media-search:timing:double-check")
        .expect("media-search double-check timing status should exist");
    assert_eq!(double_check_timing.value.as_deref(), Some("2.50s"));
    let warning_timing = media_tree
        .find("media-search:timing:warning-threshold")
        .expect("media-search warning-threshold timing status should exist");
    assert_eq!(warning_timing.value.as_deref(), Some("7.50s"));
}

#[test]
fn gui_shell_app_state_projects_responsive_layout_metadata_for_major_surfaces() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        chat_input_enabled: Some(true),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );

    let configuration = state.configuration_widget_tree();
    assert_eq!(configuration.kind, GuiWidgetKind::Layout);
    assert_eq!(configuration.layout_mode, Some(GuiLayoutMode::Stack));
    let section_grid = configuration.find("configuration:sections").unwrap();
    assert_eq!(section_grid.kind, GuiWidgetKind::Layout);
    assert_eq!(
        section_grid.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 420.0,
            max_columns: 3,
        })
    );
    let connection_section = configuration.find("config-section:Connection").unwrap();
    assert_eq!(connection_section.column_span, 2);

    let main_window = state.main_window_widget_tree();
    assert_eq!(main_window.kind, GuiWidgetKind::Layout);
    assert_eq!(main_window.layout_mode, Some(GuiLayoutMode::Stack));
    let top_region = main_window.find("main-window:top-region").unwrap();
    assert_eq!(
        top_region.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 360.0,
            max_columns: 3,
        })
    );
    let summary_column = main_window.find("main-window:summary-column").unwrap();
    assert_eq!(summary_column.kind, GuiWidgetKind::Layout);
    let browser = main_window.find("main-window:browser").unwrap();
    assert_eq!(browser.min_content_height, Some(300.0));
    let playlist = main_window.find("main-window:playlist").unwrap();
    assert_eq!(playlist.min_content_height, Some(220.0));
    let chat = main_window.find("main-window:chat").unwrap();
    assert_eq!(chat.min_content_height, Some(180.0));
    let chat_panel = main_window.find("main-window:chat-panel").unwrap();
    assert_eq!(chat_panel.kind, GuiWidgetKind::Panel);

    let top_region_children: Vec<_> = top_region
        .children
        .iter()
        .map(|child| child.id.as_str())
        .collect();
    assert_eq!(
        top_region_children,
        vec![
            "main-window:summary-column",
            "main-window:browser",
            "main-window:playlist-column",
        ]
    );

    let public_servers = state.public_server_widget_tree();
    assert_eq!(public_servers.kind, GuiWidgetKind::Layout);
    let public_content = public_servers.find("public-servers:content").unwrap();
    assert_eq!(
        public_content.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 360.0,
            max_columns: 2,
        })
    );

    let media_search = state.media_search_widget_tree();
    assert_eq!(media_search.kind, GuiWidgetKind::Layout);
    let media_content = media_search.find("media-search:content").unwrap();
    assert_eq!(
        media_content.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 360.0,
            max_columns: 2,
        })
    );
}

#[test]
fn gui_shell_app_state_projects_single_main_window_editor_as_full_width_row() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));

    let tree = state.main_window_widget_tree();
    let editors = tree
        .find("main-window:editors")
        .expect("main window editors should exist when an editor is active");
    assert_eq!(editors.kind, GuiWidgetKind::Layout);
    assert_eq!(
        editors.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 420.0,
            max_columns: 2,
        })
    );
    assert_eq!(editors.children.len(), 1);
    assert_eq!(editors.children[0].id, "main-window:media-url-edit");
    assert_eq!(editors.children[0].column_span, 2);
}

#[test]
fn gui_shell_app_state_projects_shell_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::UpdateNotice)));
    assert!(state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Widget tree ready".to_owned(),
    }));

    let tree = state.shell_widget_tree();
    assert_eq!(tree.kind, GuiWidgetKind::Panel);

    let active_view = tree
        .find("shell:active-view")
        .expect("active view status should exist");
    assert_eq!(active_view.value.as_deref(), Some("public-servers"));

    let open_modal = tree
        .find("shell:open-modal")
        .expect("open modal status should exist");
    assert_eq!(open_modal.value.as_deref(), Some("update-notice"));

    let media_index_active = tree
        .find("shell:media-index-active")
        .expect("media-index active status should exist");
    assert_eq!(media_index_active.value.as_deref(), Some("no"));
    let media_index_status = tree
        .find("shell:media-index-status")
        .expect("media-index status should exist");
    assert_eq!(media_index_status.value.as_deref(), Some("(idle)"));

    let modal_kind = tree
        .find("shell:modal:kind")
        .expect("modal kind status should exist");
    assert_eq!(modal_kind.value.as_deref(), Some("update-notice"));
    let dismiss_notice = tree
        .find("shell:modal:update:dismiss")
        .expect("update notice dismiss button should exist");
    assert_eq!(dismiss_notice.kind, GuiWidgetKind::Button);

    let notification = tree
        .find("shell:notification:0")
        .expect("notification row should exist");
    assert_eq!(notification.kind, GuiWidgetKind::ListItem);
    assert_eq!(notification.value.as_deref(), Some("Widget tree ready"));

    let save_status = tree
        .find("shell:command:save")
        .expect("command status row should exist");
    assert_eq!(save_status.kind, GuiWidgetKind::Status);
    assert_eq!(save_status.value.as_deref(), Some("enabled"));

    let validation_status = tree
        .find("shell:validation:status")
        .expect("validation status row should exist");
    assert_eq!(validation_status.value.as_deref(), Some("clean"));

    let last_action_error = tree
        .find("shell:validation:last-action-error")
        .expect("last action error row should exist");
    assert_eq!(last_action_error.value.as_deref(), Some("(none)"));

    let public_servers = tree
        .find("public-servers-root")
        .expect("public server subtree should exist");
    assert!(public_servers.selected);
}

#[test]
fn gui_shell_app_state_projects_media_index_status_into_shell_widget_tree() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        state.apply(GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
            GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some("Indexing media 1/2: 14 folders, 2048 files (Anime)".to_owned()),
            },
        ))
    );

    let tree = state.shell_widget_tree();
    assert_eq!(
        tree.find("shell:media-index-active")
            .and_then(|node| node.value.as_deref()),
        Some("yes")
    );
    assert_eq!(
        tree.find("shell:media-index-status")
            .and_then(|node| node.value.as_deref()),
        Some("Indexing media 1/2: 14 folders, 2048 files (Anime)")
    );
}

#[test]
fn gui_shell_app_state_projects_validation_and_busy_command_status_into_widget_tree() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    let invalid_tree = state.shell_widget_tree();
    assert_eq!(
        invalid_tree
            .find("shell:validation:status")
            .and_then(|node| node.value.as_deref()),
        Some("1 issue(s)")
    );
    assert_eq!(
        invalid_tree
            .find("shell:validation:issue:0")
            .map(|node| (node.label.as_str(), node.value.as_deref())),
        Some((
            "Connection / Port",
            Some("must be a valid TCP port from 1 to 65535."),
        ))
    );
    assert_eq!(
        invalid_tree
            .find("shell:command:save")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "8999".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let busy_tree = state.shell_widget_tree();
    assert_eq!(
        busy_tree
            .find("shell:command:busy")
            .and_then(|node| node.value.as_deref()),
        Some("yes")
    );
    for widget_id in [
        "shell:command:save",
        "shell:command:reset",
        "shell:command:reload",
        "shell:command:connect-public-server",
        "shell:command:refresh-public-servers",
        "shell:command:search-missing-media",
        "shell:command:toggle-pause",
        "shell:command:send-chat-message",
    ] {
        assert_eq!(
            busy_tree
                .find(widget_id)
                .and_then(|node| node.value.as_deref()),
            Some("disabled"),
            "{widget_id} should surface as disabled while a pending operation is active",
        );
    }
}

#[test]
fn gui_shell_app_state_renders_shell_widget_trees_through_renderer() {
    #[derive(Default)]
    struct RecordingRenderer {
        events: Vec<String>,
    }

    impl GuiWidgetRenderer for RecordingRenderer {
        fn begin_node(&mut self, node: &GuiWidgetNode, depth: usize) {
            self.events.push(format!("begin:{depth}:{}", node.id));
        }

        fn end_node(&mut self, node: &GuiWidgetNode, depth: usize) {
            self.events.push(format!("end:{depth}:{}", node.id));
        }
    }

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Renderer adapter ready".to_owned(),
    }));

    let mut renderer = RecordingRenderer::default();
    state.render_shell_widgets(&mut renderer);

    assert_eq!(
        renderer.events.first().map(String::as_str),
        Some("begin:0:shell-root")
    );
    assert_eq!(
        renderer.events.last().map(String::as_str),
        Some("end:0:shell-root")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:notifications")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:notification:0")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:commands")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:command:save")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:validation")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:validation:status")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:public-servers-root")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "end:1:public-servers-root")
    );
}
