use super::*;

#[test]
fn gui_shell_app_state_projects_menu_dialog_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .find("menu.play")
        .expect("playback toggle action should exist");
    assert_eq!(pause.kind, GuiWidgetKind::Button);
    assert!(!pause.enabled);
    assert!(!pause.selected);
    assert!(
        tree.find("menu.toggle_playback_buttons")
            .is_some_and(|action| action.selected),
        "checkable Window state must be independent from menu-row selection"
    );

    let about = tree
        .find("menus:dialog:about")
        .expect("about dialog status should exist");
    assert_eq!(about.kind, GuiWidgetKind::Status);
    assert!(about.enabled);
    assert!(about.selected);
    assert_eq!(about.value.as_deref(), Some("yes"));
    assert!(tree.find("menus:about:summary").is_some());
}

#[test]
fn gui_shell_app_state_projects_public_server_and_media_search_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    let connection_configuration = state.configuration_widget_tree();
    assert!(
        connection_configuration
            .find("configuration:connection-tools")
            .is_some()
    );
    assert!(
        connection_configuration
            .find("settings.section.connection")
            .is_some()
    );
    assert!(
        connection_configuration
            .find("settings.section.sync")
            .is_some(),
        "connection setup should keep playback/search tuning reachable"
    );

    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::Overview,
    )));
    let configuration = state.configuration_widget_tree();
    assert_eq!(configuration.kind, GuiWidgetKind::Layout);
    assert_eq!(configuration.layout_mode, Some(GuiLayoutMode::Stack));
    let configuration_tabs = configuration.find("configuration:tabs").unwrap();
    assert_eq!(
        configuration_tabs.layout_mode,
        Some(GuiLayoutMode::TabStrip {
            min_tab_width: 132.0,
        })
    );
    let section_grid = configuration.find("configuration:sections").unwrap();
    assert_eq!(section_grid.kind, GuiWidgetKind::Layout);
    assert_eq!(
        section_grid.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 420.0,
            max_columns: 3,
        })
    );
    let connection_section = configuration.find("settings.section.connection").unwrap();
    assert_eq!(connection_section.column_span, 2);
    assert!(
        configuration.find("public-servers-root").is_some(),
        "public server management should be embedded in setup"
    );
    assert!(
        configuration.find("media-search-root").is_some(),
        "media search management should be embedded in setup"
    );

    let main_window = state.main_window_widget_tree();
    assert_eq!(main_window.kind, GuiWidgetKind::Layout);
    assert_eq!(main_window.layout_mode, Some(GuiLayoutMode::Stack));
    assert!(main_window.find("main-window:tabs").is_none());
    let top_region = main_window.find("main-window:top-region").unwrap();
    assert_eq!(
        top_region.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 240.0,
            max_columns: 3,
        })
    );
    let summary_column = main_window.find("main-window:summary-column").unwrap();
    assert_eq!(summary_column.kind, GuiWidgetKind::Layout);
    let room_panel = main_window.find("main-window:connection").unwrap();
    assert_eq!(room_panel.label, "Room");
    assert_eq!(room_panel.min_content_height, Some(320.0));
    assert!(main_window.find("main-window:browser").is_none());
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
            "main-window:playlist-column",
            "main-window:chat-panel",
        ]
    );

    let public_servers = state.public_server_widget_tree();
    assert_eq!(public_servers.kind, GuiWidgetKind::Panel);
    assert_eq!(public_servers.label, "Saved / Public Servers");
    assert!(public_servers.find("public-servers:list").is_some());
    assert!(public_servers.find("public-servers:commands").is_some());

    let media_search = state.media_search_widget_tree();
    assert_eq!(media_search.kind, GuiWidgetKind::Panel);
    assert!(media_search.find("media-search:directories").is_some());
    assert!(media_search.find("media-search:utility").is_some());

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins.layout_mode,
        Some(GuiLayoutMode::ResponsiveColumns {
            min_column_width: 260.0,
            max_columns: 3,
        })
    );
    assert!(plugins.find("plugins:list:stream-support").is_some());
    assert_eq!(
        plugins
            .find("plugins:stream-support")
            .expect("stream support plugin detail should exist")
            .column_span,
        2
    );
}
