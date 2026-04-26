use super::*;

#[test]
fn gui_widget_egui_renderer_maps_playlist_workflow_controls_to_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.main_window.playback.can_toggle_pause = true;

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    let shell_tree = state.shell_widget_tree();
    let add_files_button = shell_tree.find("main-window:playlist:add-files").unwrap();
    let more_menu_button = shell_tree.find("main-window:playlist:more-menu").unwrap();
    let add_url_button = shell_tree.find("main-window:playlist:add-url").unwrap();
    assert!(
        shell_tree.find("main-window:control:open-url").is_none(),
        "Open URL should not be exposed from the Controls pane"
    );
    let row_remove_button = shell_tree.find("main-window:playlist:1:remove").unwrap();

    assert_eq!(add_files_button.kind, GuiWidgetKind::Button);
    assert_eq!(add_url_button.kind, GuiWidgetKind::Button);
    assert_eq!(more_menu_button.kind, GuiWidgetKind::Button);
    let add_files_size = GuiWidgetEguiRenderer::compact_action_button_size(add_files_button);
    let add_url_size = GuiWidgetEguiRenderer::compact_action_button_size(add_url_button);
    let more_menu_size = GuiWidgetEguiRenderer::compact_action_button_size(more_menu_button);
    assert_eq!(add_files_size, egui::vec2(40.0, 40.0));
    assert_eq!(add_url_size, egui::vec2(40.0, 40.0));
    assert_eq!(more_menu_size, egui::vec2(52.0, 40.0));
    assert_eq!(
        more_menu_button
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:playlist:load", "main-window:playlist:save"]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, add_url_button),
        vec![GuiShellAction::BeginSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, row_remove_button),
        vec![
            GuiShellAction::SelectMainWindowPlaylist(1),
            GuiShellAction::RemoveSelectedMainWindowPlaylist
        ]
    );
    for removed_id in [
        "main-window:playlist:open-selected",
        "main-window:playlist:open-selected-folder",
        "main-window:playlist:trust-selected",
        "main-window:playlist:load-shuffle",
        "main-window:playlist:undo",
        "main-window:playlist:shuffle-remaining",
        "main-window:playlist:shuffle-entire",
        "main-window:playlist:edit",
    ] {
        assert!(
            shell_tree.find(removed_id).is_none(),
            "{removed_id} should not be exposed in the consolidated playlist toolbar"
        );
    }

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistTextEdit(
        "Episode 9.mkv\nhttps://example.com/live".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistUrlEdit(
        "https://example.com/extra".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));

    let shell_tree = state.shell_widget_tree();
    let media_url_text_node = shell_tree.find("main-window:media-url-edit:text").unwrap();
    let media_url_cancel = shell_tree
        .find("main-window:media-url-edit:cancel")
        .unwrap();
    assert!(
        shell_tree.find("main-window:playlist-edit:text").is_some(),
        "playlist editors should stay visible in the unified room dashboard"
    );
    let playlist_column = shell_tree.find("main-window:playlist-column").unwrap();
    assert!(
        playlist_column
            .find("main-window:playlist-edit:text")
            .is_some(),
        "playlist text editor should render inside the playlist column"
    );
    assert!(
        playlist_column
            .find("main-window:playlist-url-edit:text")
            .is_some(),
        "playlist URL editor should render inside the playlist column"
    );
    let shell_tree = state.shell_widget_tree();
    let playlist_text_node = shell_tree.find("main-window:playlist-edit:text").unwrap();
    let playlist_text_commit = shell_tree.find("main-window:playlist-edit:commit").unwrap();
    let playlist_text_cancel = shell_tree.find("main-window:playlist-edit:close").unwrap();
    let playlist_url_text_node = shell_tree
        .find("main-window:playlist-url-edit:text")
        .unwrap();
    let playlist_url_helper = shell_tree
        .find("main-window:playlist-url-edit:helper")
        .unwrap();
    let playlist_url_commit = shell_tree
        .find("main-window:playlist-url-edit:commit")
        .unwrap();
    let playlist_url_cancel = shell_tree
        .find("main-window:playlist-url-edit:close")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_commit),
        vec![
            GuiShellAction::ReplaceSharedPlaylistEntries(vec![
                "Episode 9.mkv".to_owned(),
                "https://example.com/live".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistTextEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_cancel),
        vec![GuiShellAction::CancelSharedPlaylistTextEdit]
    );
    assert_eq!(
        playlist_url_helper.value.as_deref(),
        Some("1 URL detected.")
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_commit),
        vec![
            GuiShellAction::AppendSharedPlaylistEntries(vec![
                "https://example.com/extra".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistUrlEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_cancel),
        vec![GuiShellAction::CancelSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, media_url_cancel),
        vec![GuiShellAction::CancelMediaUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_text_node,
            "Episode 10.mkv",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
            "Episode 10.mkv".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_url_text_node,
            "https://example.com/final",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
            "https://example.com/final".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            media_url_text_node,
            "https://media.example/stream",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateMediaUrlEdit("https://media.example/stream".to_owned()),
            GuiShellAction::RequestMainWindowUserMediaOpen(
                "https://media.example/stream".to_owned(),
            ),
            GuiShellAction::CancelMediaUrlEdit,
        ])
    );
}

#[test]
fn gui_widget_egui_renderer_maps_stream_support_buttons_to_import_and_retry_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            GuiStreamHelperRuntimeSnapshot {
                health: GuiStreamHelperHealth::MissingJsRuntime,
                message: Some("Import Deno or install the managed runtime.".to_owned()),
                target: Some("https://www.youtube.com/watch?v=UyjIPZfygTk".to_owned()),
                install_supported: true,
                integration_supported: true,
                retry_available: true,
                install_location: Some("C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin".to_owned()),
                downloader_status: Some("Managed install: 2025.01.01 (C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin/yt-dlp.exe)".to_owned()),
                js_runtime_status: Some("Missing from Syncplay's managed install and PATH for Deno.".to_owned()),
                open_install_location_available: true,
            },
        ))
    );
    let configuration_tree = state.configuration_widget_tree();
    assert!(
        configuration_tree.find("config-stream-support").is_none(),
        "stream support should be relocated out of setup"
    );
    let plugins_tree = state.plugins_widget_tree();
    let install_plugin_button = plugins_tree
        .find("plugins:stream-support:install")
        .expect("stream-support plugin install button should exist");
    let import_downloader_button = plugins_tree
        .find("plugins:stream-support:import-downloader")
        .expect("stream-support plugin import downloader button should exist");
    let recheck_plugin_button = plugins_tree
        .find("plugins:stream-support:recheck")
        .expect("stream-support plugin recheck button should exist");
    let open_location_plugin_button = plugins_tree
        .find("plugins:stream-support:open-location")
        .expect("stream-support plugin open-location button should exist");
    let retry_plugin_button = plugins_tree
        .find("plugins:stream-support:retry")
        .expect("stream-support plugin retry button should exist");

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, install_plugin_button),
        vec![GuiShellAction::InstallStreamHelper]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, recheck_plugin_button),
        vec![GuiShellAction::RecheckStreamHelper]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_location_plugin_button),
        vec![GuiShellAction::OpenStreamHelperInstallLocation]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, retry_plugin_button),
        vec![GuiShellAction::RetryPendingStreamMediaOpen]
    );
    assert_eq!(
        import_downloader_button.enabled,
        state.stream_helper.integration_supported
    );
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::StreamSupport)));
    let modal_tree = state.shell_modal_widget_tree();
    let install_button = modal_tree
        .find("shell:modal:stream-support:install")
        .expect("stream-support modal install button should exist");
    let recheck_button = modal_tree
        .find("shell:modal:stream-support:recheck")
        .expect("stream-support modal recheck button should exist");
    let open_location_button = modal_tree
        .find("shell:modal:stream-support:open-location")
        .expect("stream-support modal open-location button should exist");
    let retry_button = modal_tree
        .find("shell:modal:stream-support:retry")
        .expect("stream-support modal retry button should exist");

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, install_button),
        vec![GuiShellAction::InstallStreamHelper]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, recheck_button),
        vec![GuiShellAction::RecheckStreamHelper]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_location_button),
        vec![GuiShellAction::OpenStreamHelperInstallLocation]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, retry_button),
        vec![GuiShellAction::RetryPendingStreamMediaOpen]
    );
}

#[test]
fn gui_widget_egui_renderer_disables_stream_support_modal_actions_during_remediation() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            GuiStreamHelperRuntimeSnapshot {
                health: GuiStreamHelperHealth::MissingJsRuntime,
                message: Some("Import Deno or install the managed runtime.".to_owned()),
                target: Some("https://www.youtube.com/watch?v=UyjIPZfygTk".to_owned()),
                install_supported: true,
                integration_supported: true,
                retry_available: true,
                install_location: Some("C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin".to_owned()),
                downloader_status: Some("Managed install: 2025.01.01 (C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin/yt-dlp.exe)".to_owned()),
                js_runtime_status: Some("Missing from Syncplay's managed install and PATH for Deno.".to_owned()),
                open_install_location_available: true,
            },
        ))
    );
    assert!(state.apply(
        GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(
            GuiStreamHelperRemediationRuntimeSnapshot {
                active: true,
                label: Some("Downloading yt-dlp".to_owned()),
                detail: Some("Saving yt-dlp into Syncplay's helper directory.".to_owned()),
                progress_fraction: 0.25,
            },
        )
    ));

    assert!(!GuiWidgetEguiRenderer::modal_action_enabled(
        &state,
        "shell:modal:stream-support:install"
    ));
    assert!(!GuiWidgetEguiRenderer::modal_action_enabled(
        &state,
        "shell:modal:stream-support:retry"
    ));
    assert!(GuiWidgetEguiRenderer::modal_action_enabled(
        &state,
        "shell:modal:stream-support:open-location"
    ));
}
