use super::*;

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
    let tabs = tree
        .find("configuration:tabs")
        .expect("configuration tabs should exist in widget tree");
    assert_eq!(
        tabs.layout_mode,
        Some(GuiLayoutMode::TabStrip {
            min_tab_width: 132.0,
        })
    );
    assert!(
        tree.find("configuration:tab:connection")
            .expect("connection tab should exist")
            .selected
    );
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
    assert!(
        tree.find("config:Privacy:Trusted Domains").is_none(),
        "privacy controls should be hidden while the connection tab is selected"
    );

    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PrivacyChat,
    )));
    let tree = state.configuration_widget_tree();
    let trusted_domains = tree
        .find("config:Privacy:Trusted Domains")
        .expect("trusted-domains control should exist once the privacy tab is selected");
    assert_eq!(trusted_domains.kind, GuiWidgetKind::TextArea);

    let save = tree
        .find("config-command:save")
        .expect("save command should exist in widget tree");
    assert_eq!(save.kind, GuiWidgetKind::Button);
    assert!(save.enabled);
}

#[test]
fn gui_shell_app_state_projects_player_setup_into_configuration_widgets() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::NotConfigured,
                    message: "Set playerPath to mpv before connecting.".to_owned(),
                }),
            },
        ))
    );

    let configuration = state.configuration_widget_tree();
    assert!(configuration.find("config-player-setup").is_some());
    assert_eq!(
        configuration
            .find("config-player-setup:blocking")
            .and_then(|node| node.value.as_deref()),
        Some(
            "Set up mpv before connecting. Use Auto-detect, Choose mpv.exe, or Retry mpv after updating Player Path."
        )
    );
    assert!(
        !configuration
            .find("config-command:connect")
            .expect("connect button should exist")
            .enabled
    );
    assert!(
        !configuration
            .find("config-player-setup:retry")
            .expect("retry button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref()),
        Some("player-setup")
    );
    assert_eq!(
        shell
            .find("shell:modal:kind")
            .and_then(|node| node.value.as_deref()),
        Some("player-setup")
    );
    assert!(
        !shell
            .find("shell:modal:close")
            .expect("first-run player setup modal close button should exist")
            .enabled
    );
}

#[test]
fn gui_shell_app_state_projects_actionable_setup_alerts_only_after_feedback() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        state
            .configuration_widget_tree()
            .find("configuration:action-alert")
            .is_none(),
        "setup alerts should be hidden by default"
    );

    assert!(state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Success,
        message: "Configuration saved.".to_owned(),
    }));
    let success_tree = state.configuration_widget_tree();
    assert_eq!(
        success_tree
            .find("configuration:alert:level")
            .and_then(|node| node.value.as_deref()),
        Some("success")
    );
    assert_eq!(
        success_tree
            .find("configuration:alert:message")
            .and_then(|node| node.value.as_deref()),
        Some("Configuration saved.")
    );
    let close = success_tree
        .find("configuration:alert:close")
        .expect("setup alert close button should exist");
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, close),
        vec![GuiShellAction::DismissSetupAlert]
    );
    assert!(state.apply(GuiShellAction::DismissSetupAlert));
    assert!(
        state
            .configuration_widget_tree()
            .find("configuration:action-alert")
            .is_none()
    );

    assert!(state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: Some("Player path is invalid.".to_owned()),
        },
    )));
    let error_tree = state.configuration_widget_tree();
    assert_eq!(
        error_tree
            .find("configuration:alert:level")
            .and_then(|node| node.value.as_deref()),
        Some("error")
    );
    let fix_player_path = error_tree
        .find("configuration:alert:fix-player-path")
        .expect("player-path alert action should exist");
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, fix_player_path),
        vec![
            GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
            GuiShellAction::FocusConfigurationControl {
                section: "Connection",
                label: "Player Path",
            },
            GuiShellAction::BeginConfigurationTextEdit {
                section: "Connection",
                label: "Player Path",
            },
        ]
    );
}

#[test]
fn gui_shell_app_state_projects_stream_support_into_plugins_widgets() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            GuiStreamHelperRuntimeSnapshot {
                health: GuiStreamHelperHealth::MissingDownloader,
                message: Some(
                    "Extractor-backed page URLs need yt-dlp before mpv can load them.".to_owned(),
                ),
                target: Some("https://www.youtube.com/watch?v=UyjIPZfygTk".to_owned()),
                install_supported: false,
                integration_supported: true,
                retry_available: true,
                install_location: Some(
                    "C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin".to_owned()
                ),
                downloader_status: Some(
                    "Missing from Syncplay's managed install and PATH for yt-dlp.".to_owned()
                ),
                js_runtime_status: Some("PATH: 2.1.0 (C:/Tools/deno.exe)".to_owned()),
                open_install_location_available: true,
            },
        ))
    );

    let configuration = state.configuration_widget_tree();
    assert_eq!(
        configuration.find("config-stream-support"),
        None,
        "stream support should not be projected inside setup"
    );

    let plugins = state.plugins_widget_tree();
    assert!(plugins.find("plugins:stream-support").is_some());
    assert_eq!(
        plugins
            .find("plugins:stream-support:summary")
            .and_then(|node| node.value.as_deref()),
        Some("Extractor-backed page URLs need yt-dlp before mpv can load them.")
    );
    assert_eq!(
        plugins
            .find("plugins:stream-support:alert:level")
            .and_then(|node| node.value.as_deref()),
        Some("warning")
    );
    assert!(
        plugins
            .find("plugins:stream-support:recheck")
            .expect("recheck button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("menus:dialog:stream-support")
            .and_then(|node| node.value.as_deref()),
        Some("missing-downloader")
    );
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::StreamSupport)));
    let modal = state.shell_modal_widget_tree();
    assert_eq!(
        modal
            .find("shell:modal:stream-support:install-location")
            .and_then(|node| node.value.as_deref()),
        Some("C:/Users/test/AppData/Roaming/Syncplay/tools/stream-helper/bin")
    );
    assert_eq!(
        modal
            .find("shell:modal:stream-support:downloader-status")
            .and_then(|node| node.value.as_deref()),
        Some("Missing from Syncplay's managed install and PATH for yt-dlp.")
    );
    assert_eq!(
        modal
            .find("shell:modal:stream-support:js-runtime-status")
            .and_then(|node| node.value.as_deref()),
        Some("PATH: 2.1.0 (C:/Tools/deno.exe)")
    );
    assert_eq!(
        modal
            .find("shell:modal:stream-support:target")
            .and_then(|node| node.value.as_deref()),
        Some("https://www.youtube.com/watch?v=UyjIPZfygTk")
    );
}

#[test]
fn gui_shell_app_state_projects_stream_helper_remediation_progress_into_widgets() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:stream-support:remediation")
            .and_then(|node| node.value.as_deref()),
        Some("Downloading yt-dlp")
    );
    assert_eq!(
        plugins
            .find("plugins:stream-support:remediation-progress")
            .and_then(|node| node.value.as_deref()),
        Some("25%")
    );
    assert!(
        !plugins
            .find("plugins:stream-support:install")
            .expect("install button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("shell:stream-helper-remediation-active")
            .and_then(|node| node.value.as_deref()),
        Some("yes")
    );
    assert_eq!(
        shell
            .find("shell:stream-helper-remediation-label")
            .and_then(|node| node.value.as_deref()),
        Some("Downloading yt-dlp")
    );
    assert_eq!(
        shell
            .find("shell:stream-helper-remediation-progress")
            .and_then(|node| node.value.as_deref()),
        Some("0.250")
    );
}
