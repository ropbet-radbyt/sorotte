use super::*;

#[test]
fn gui_shell_app_state_projects_main_window_room_owned_editor_content() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));

    let tree = state.main_window_widget_tree();
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(
        tree.find("main-window:editors").is_some(),
        "room editor row should be mounted in the unified room dashboard"
    );
    assert_eq!(
        tree.find("main-window:editors")
            .expect("room editor row should exist")
            .label,
        "Room Editors"
    );
    assert!(
        tree.find("main-window:content").is_some(),
        "room content should not keep the removed overview tab naming"
    );
    assert!(tree.find("main-window:content:overview").is_none());
    let media_url_edit = tree
        .find("main-window:media-url-edit")
        .expect("media-url editor should exist in the room dashboard");
    assert_eq!(media_url_edit.kind, GuiWidgetKind::Panel);
}

#[test]
fn gui_shell_app_state_projects_playlist_editors_inside_playlist_column() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));

    let tree = state.main_window_widget_tree();
    let playlist_column = tree
        .find("main-window:playlist-column")
        .expect("playlist column should exist in the room dashboard");
    assert!(
        playlist_column.find("main-window:playlist-edit").is_some(),
        "playlist text editor should be mounted with the playlist controls"
    );
    assert!(
        playlist_column
            .find("main-window:playlist-url-edit")
            .is_some(),
        "playlist URL editor should be mounted with the playlist controls"
    );
    assert!(
        tree.find("main-window:editors").is_none(),
        "playlist-owned editors should not be appended as a separate bottom-of-screen editor row"
    );
}

#[test]
fn gui_shell_app_state_projects_unified_room_content_and_selected_configuration_tab_content() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let main_window = state.main_window_widget_tree();
    assert!(main_window.find("main-window:chat-input").is_some());
    assert!(main_window.find("main-window:playlist").is_some());
    assert!(main_window.find("main-window:connection").is_some());
    assert!(main_window.find("main-window:participants").is_some());
    assert!(main_window.find("main-window:browser").is_none());

    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::InterfaceSystem,
    )));
    let configuration = state.configuration_widget_tree();
    assert!(configuration.find("config:OSD:Show OSD").is_some());
    assert!(configuration.find("config:System:Language").is_some());
    assert!(configuration.find("config:Connection:Host").is_none());
    assert!(
        configuration
            .find("config:Privacy:Trusted Domains")
            .is_none()
    );
}

#[test]
fn gui_shell_app_state_projects_shell_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
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
    assert_eq!(active_view.value.as_deref(), Some("setup"));

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
    assert!(!public_servers.selected);

    let plugins = tree
        .find("plugins-root")
        .expect("plugins subtree should exist");
    assert_eq!(plugins.label, "Plugins");
    assert!(!plugins.selected);

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Plugins)));
    let plugins_tree = state.shell_widget_tree();
    assert_eq!(
        plugins_tree
            .find("shell:active-view")
            .and_then(|node| node.value.as_deref()),
        Some("plugins")
    );
    assert!(
        plugins_tree
            .find("plugins-root")
            .expect("plugins subtree should exist")
            .selected
    );
}

#[test]
fn gui_shell_app_state_projects_media_index_status_into_shell_widget_tree() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
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
            .any(|event| event.ends_with(":public-servers-root"))
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event.ends_with(":public-servers-root"))
    );
}
