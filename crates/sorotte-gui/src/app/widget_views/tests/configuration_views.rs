use super::*;

#[test]
fn gui_shell_app_state_projects_configuration_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "widget.example".to_owned().into(),
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
        .find("settings.connection.host")
        .expect("host control should exist in widget tree");
    assert_eq!(host.kind, GuiWidgetKind::TextInput);
    assert_eq!(host.value.as_deref(), Some("widget.example"));
    assert!(host.enabled);
    assert!(host.selected);
    let player_arguments = tree
        .find("settings.player.arguments")
        .expect("player-arguments control should exist in widget tree");
    assert_eq!(player_arguments.kind, GuiWidgetKind::TextInput);
    assert!(!player_arguments.enabled);
    let room_history = tree
        .find("settings.connection.room_history")
        .expect("room-history control should exist in widget tree");
    assert_eq!(room_history.kind, GuiWidgetKind::TextArea);
    assert!(
        tree.find("settings.privacy.trusted_domains").is_none(),
        "privacy controls should be hidden while the connection tab is selected"
    );

    assert!(state.apply(GuiShellAction::CommitConfigurationTextEdit));

    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PrivacyChat,
    )));
    let tree = state.configuration_widget_tree();
    let trusted_domains = tree
        .find("settings.privacy.trusted_domains")
        .expect("trusted-domains control should exist once the privacy tab is selected");
    assert_eq!(trusted_domains.kind, GuiWidgetKind::TextArea);

    let save = tree
        .find("config-command:save")
        .expect("save command should exist in widget tree");
    assert_eq!(save.kind, GuiWidgetKind::Button);
    assert!(save.enabled);
}

#[test]
fn gui_shell_app_state_distinguishes_default_draft_and_stored_setting_origins() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PlaybackSearch,
    )));

    let tree = state.configuration_widget_tree();
    assert_eq!(
        tree.find("settings.playback.unpause_action.origin")
            .and_then(|node| node.value.as_deref()),
        Some("Using application default")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlaybackUnpauseAction,
        value: "Always".to_owned().into(),
    }));
    let tree = state.configuration_widget_tree();
    assert_eq!(
        tree.find("settings.playback.unpause_action.origin")
            .and_then(|node| node.value.as_deref()),
        Some("Unsaved change")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let persisted = state.configuration.to_stored_settings();
    assert!(state.apply(GuiShellAction::CompleteConfigurationSave(persisted)));
    let tree = state.configuration_widget_tree();
    assert_eq!(
        tree.find("settings.playback.unpause_action.origin")
            .and_then(|node| node.value.as_deref()),
        Some("Stored override")
    );
}

#[test]
fn gui_shell_app_state_projects_player_setup_into_configuration_widgets() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::NotConfigured,
                    message: "Set playerPath to mpv before connecting.".to_owned(),
                    retry_available: false,
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
            .find("config-command:connect-once")
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
            GuiShellAction::FocusConfigurationControl(SettingId::PlayerExecutable),
            GuiShellAction::BeginConfigurationTextEdit(SettingId::PlayerExecutable),
        ]
    );
}

#[test]
fn gui_shell_app_state_projects_stream_support_into_plugins_widgets() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
                    "C:/Users/test/AppData/Roaming/Sorotte/tools/stream-helper/bin".to_owned()
                ),
                downloader_status: Some(
                    "Missing from Sorotte's managed install and PATH for yt-dlp.".to_owned()
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
    assert!(plugins.find("plugins:plex").is_none());
    assert!(
        plugins
            .find("plugins:list:stream-support")
            .expect("stream support list row should exist")
            .selected
    );
    assert!(
        !plugins
            .find("plugins:list:plex")
            .expect("plex list row should exist")
            .selected
    );
    let details = plugins
        .find("plugins:details")
        .expect("plugin details should exist");
    assert_eq!(details.children.len(), 1);
    assert_eq!(details.children[0].id, "plugins:stream-support");
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
        Some("C:/Users/test/AppData/Roaming/Sorotte/tools/stream-helper/bin")
    );
    assert_eq!(
        modal
            .find("shell:modal:stream-support:downloader-status")
            .and_then(|node| node.value.as_deref()),
        Some("Missing from Sorotte's managed install and PATH for yt-dlp.")
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
fn gui_shell_app_state_projects_media_match_plugin_widgets_and_actions() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    assert!(
        state.apply(GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
            GuiMediaMatchRuntimeSnapshot {
                settings: MediaMatchSettings {
                    fingerprinting_enabled: true,
                    runtime_tolerance_enabled: true,
                    autoplay_policy: MediaMatchAutoplayPolicy::DiagnosticsOnly,
                    ..MediaMatchSettings::default()
                },
                health: GuiMediaMatchToolHealth::Healthy,
                message: None,
                install_supported: true,
                integration_supported: true,
                install_location: Some(
                    "C:/Users/test/AppData/Roaming/Sorotte/tools/media-match/bin".to_owned(),
                ),
                ffmpeg_status: Some("ffmpeg 7.1 (C:/Tools/ffmpeg.exe)".to_owned()),
                ffprobe_status: Some("ffprobe 7.1 (C:/Tools/ffprobe.exe)".to_owned()),
                cache_status: Some("2 fingerprint records".to_owned()),
                current_decision: Some("probable: sampled-fast audio match".to_owned()),
                nearest_match: Some(
                    "episode-b.mkv (probable: sampled-fast audio match)".to_owned()
                ),
                last_evidence: Some("audio=0.94 offset=20s".to_owned()),
                remote_status: Some("bob: strong".to_owned()),
                background_status: Some("idle".to_owned()),
                open_install_location_available: true,
            },
        ))
    );

    let plugins = state.plugins_widget_tree();
    assert!(
        plugins
            .find("plugins:list:media-matching")
            .expect("media matching list row should exist")
            .selected
    );
    assert!(
        !plugins
            .find("plugins:list:stream-support")
            .unwrap()
            .selected
    );
    let details = plugins
        .find("plugins:details")
        .expect("plugin details should exist");
    assert_eq!(details.children.len(), 1);
    assert_eq!(details.children[0].id, "plugins:media-matching");
    assert_eq!(
        plugins
            .find("plugins:list:media-matching")
            .and_then(|node| node.value.as_deref()),
        Some("healthy")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:health")
            .and_then(|node| node.value.as_deref()),
        Some("healthy")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:health")
            .map(|node| node.label.as_str()),
        Some("State")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:tool-health")
            .and_then(|node| node.value.as_deref()),
        Some("healthy")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:cache-status")
            .and_then(|node| node.value.as_deref()),
        Some("2 fingerprint records")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:current-decision")
            .and_then(|node| node.value.as_deref()),
        Some("probable: sampled-fast audio match")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:nearest-match")
            .and_then(|node| node.value.as_deref()),
        Some("episode-b.mkv (probable: sampled-fast audio match)")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:background-status")
            .and_then(|node| node.value.as_deref()),
        Some("idle")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:remote-status")
            .and_then(|node| node.value.as_deref()),
        Some("bob: strong")
    );
    let last_evidence = plugins
        .find("plugins:media-matching:last-evidence")
        .expect("last evidence should exist");
    assert_eq!(last_evidence.kind, GuiWidgetKind::TextArea);
    assert_eq!(
        last_evidence.value.as_deref(),
        Some("audio=0.94 offset=20s")
    );

    let fingerprinting = plugins
        .find("plugins:media-matching:setting:fingerprinting")
        .expect("fingerprinting checkbox should exist");
    assert_eq!(fingerprinting.kind, GuiWidgetKind::Checkbox);
    assert_eq!(fingerprinting.label.as_str(), "Enable Media Matching");
    assert_eq!(fingerprinting.value.as_deref(), Some("yes"));
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, fingerprinting, false),
        Some(GuiShellAction::SetMediaMatchFingerprintingEnabled(false))
    );
    let background_warmup = plugins
        .find("plugins:media-matching:setting:background-warmup")
        .expect("background warmup checkbox should exist");
    assert_eq!(
        background_warmup.label.as_str(),
        "Background Library Indexing"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, background_warmup, false),
        Some(GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(false))
    );
    let wire_sharing = plugins
        .find("plugins:media-matching:setting:wire-sharing")
        .expect("wire sharing checkbox should exist");
    assert_eq!(wire_sharing.label.as_str(), "Share Room Match Signatures");
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, wire_sharing, false),
        Some(GuiShellAction::SetMediaMatchWireSharingEnabled(false))
    );
    let runtime_tolerance = plugins
        .find("plugins:media-matching:setting:runtime-tolerance")
        .expect("runtime tolerance checkbox should exist");
    assert_eq!(
        runtime_tolerance.label.as_str(),
        "Allow Small Duration Tolerance"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, runtime_tolerance, false),
        Some(GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(false))
    );

    let rebuild = plugins
        .find("plugins:media-matching:rebuild-index")
        .expect("rebuild-index button should exist");
    assert_eq!(rebuild.label.as_str(), "Rebuild Library Index");
    assert!(rebuild.enabled);
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, rebuild),
        vec![GuiShellAction::RebuildMediaMatchIndex]
    );
    let cancel = plugins
        .find("plugins:media-matching:cancel-rebuild")
        .expect("cancel-rebuild button should exist");
    assert!(!cancel.enabled);
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, cancel),
        vec![GuiShellAction::CancelMediaMatchRebuild]
    );
    let clear = plugins
        .find("plugins:media-matching:clear-cache")
        .expect("clear-cache button should exist");
    assert_eq!(clear.label.as_str(), "Clear Match Cache");
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, clear),
        vec![GuiShellAction::ClearMediaMatchCache]
    );
    let strong_policy = plugins
        .find("plugins:media-matching:policy:strong")
        .expect("strong policy button should exist");
    assert_eq!(
        strong_policy.label.as_str(),
        "Allow Verified Strong Matches"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, strong_policy),
        vec![GuiShellAction::SetMediaMatchAutoplayPolicy(
            MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        )]
    );
}

#[test]
fn gui_shell_app_state_projects_disabled_media_match_as_disabled_not_healthy() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    assert!(
        state.apply(GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
            GuiMediaMatchRuntimeSnapshot {
                settings: MediaMatchSettings {
                    fingerprinting_enabled: false,
                    ..MediaMatchSettings::default()
                },
                health: GuiMediaMatchToolHealth::Healthy,
                message: Some("tools ready".to_owned()),
                install_supported: true,
                integration_supported: true,
                install_location: Some(
                    "C:/Users/test/AppData/Roaming/Sorotte/tools/media-match/bin".to_owned(),
                ),
                ffmpeg_status: Some("ffmpeg 8.1".to_owned()),
                ffprobe_status: Some("ffprobe 8.1".to_owned()),
                cache_status: Some("4013 sampled-fast records".to_owned()),
                current_decision: None,
                nearest_match: None,
                last_evidence: None,
                remote_status: None,
                background_status: Some("idle".to_owned()),
                open_install_location_available: true,
            },
        ))
    );

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:list:media-matching")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:title")
            .and_then(|node| node.value.as_deref()),
        Some("Media matching disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:summary")
            .and_then(|node| node.value.as_deref()),
        Some(
            "Media Matching is off. Existing cache data is kept; enable it to index local files and match room media."
        )
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:health")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:tool-health")
            .and_then(|node| node.value.as_deref()),
        Some("healthy")
    );
    assert!(
        !plugins
            .find("plugins:media-matching:rebuild-index")
            .expect("rebuild-index button should exist")
            .enabled
    );
    assert!(
        plugins
            .find("plugins:media-matching:clear-cache")
            .expect("clear-cache button should exist")
            .enabled
    );
}

#[test]
fn gui_shell_app_state_projects_media_match_remediation_progress_into_widgets() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    assert!(state.apply(
        GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(
            GuiMediaMatchRemediationRuntimeSnapshot {
                active: true,
                label: Some("Downloading ffmpeg".to_owned()),
                detail: Some("Saving ffmpeg and ffprobe into Sorotte's tool directory.".to_owned()),
                progress_fraction: 0.5,
            },
        )
    ));

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:media-matching:remediation:label")
            .and_then(|node| node.value.as_deref()),
        Some("Downloading ffmpeg")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:remediation:progress")
            .and_then(|node| node.value.as_deref()),
        Some("50%")
    );
    assert!(
        !plugins
            .find("plugins:media-matching:install")
            .expect("install button should exist")
            .enabled
    );
}

#[test]
fn gui_shell_app_state_disables_media_match_rebuild_when_tools_missing() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    assert!(
        state.apply(GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
            GuiMediaMatchRuntimeSnapshot {
                settings: MediaMatchSettings {
                    fingerprinting_enabled: true,
                    ..MediaMatchSettings::default()
                },
                health: crate::app::shell_state::GuiMediaMatchToolHealth::MissingFfmpeg,
                integration_supported: true,
                ..GuiMediaMatchRuntimeSnapshot::default()
            },
        ))
    );

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:list:media-matching")
            .and_then(|node| node.value.as_deref()),
        Some("missing-ffmpeg")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:title")
            .and_then(|node| node.value.as_deref()),
        Some("ffmpeg required")
    );
    let rebuild = plugins
        .find("plugins:media-matching:rebuild-index")
        .expect("rebuild-index button should exist");

    assert!(!rebuild.enabled);
    let cancel = plugins
        .find("plugins:media-matching:cancel-rebuild")
        .expect("cancel-rebuild button should exist");
    assert!(!cancel.enabled);
}

#[test]
fn gui_shell_app_state_projects_only_selected_plugin_detail() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let plugins = state.plugins_widget_tree();
    assert!(plugins.find("plugins:stream-support").is_some());
    assert!(plugins.find("plugins:plex").is_none());

    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    let plugins = state.plugins_widget_tree();
    assert!(
        !plugins
            .find("plugins:list:stream-support")
            .expect("stream support list row should exist")
            .selected
    );
    assert!(
        plugins
            .find("plugins:list:plex")
            .expect("plex list row should exist")
            .selected
    );
    let details = plugins
        .find("plugins:details")
        .expect("plugin details should exist");
    assert_eq!(details.children.len(), 1);
    assert_eq!(details.children[0].id, "plugins:plex");
    assert!(plugins.find("plugins:plex:status").is_some());
    assert!(plugins.find("plugins:stream-support").is_none());
}

#[test]
fn gui_shell_app_state_projects_plugin_enablement_gates_without_losing_subsettings() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        stream_support_plugin_enabled: Some(false),
        media_matching_plugin_enabled: Some(false),
        plex_plugin_enabled: Some(false),
        media_match_fingerprinting_enabled: Some(true),
        media_match_background_warmup_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_url: Some("https://plex.example".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:list:stream-support")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:stream-support:enabled")
            .and_then(|node| node.value.as_deref()),
        Some("no")
    );
    assert!(
        !plugins
            .find("plugins:stream-support:install")
            .expect("stream install button should exist")
            .enabled
    );

    assert!(state.apply(GuiShellAction::SelectPlugin(
        GuiPluginSelection::MediaMatching,
    )));
    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:list:media-matching")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:enabled")
            .and_then(|node| node.value.as_deref()),
        Some("no")
    );
    assert_eq!(
        plugins
            .find("plugins:media-matching:setting:fingerprinting")
            .and_then(|node| node.value.as_deref()),
        Some("yes")
    );
    assert!(
        !plugins
            .find("plugins:media-matching:setting:fingerprinting")
            .expect("fingerprinting setting should exist")
            .enabled
    );
    assert!(
        !plugins
            .find("plugins:media-matching:rebuild-index")
            .expect("rebuild button should exist")
            .enabled
    );

    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:list:plex")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert_eq!(
        plugins
            .find("plugins:plex:enabled")
            .and_then(|node| node.value.as_deref()),
        Some("no")
    );
    assert_eq!(
        plugins
            .find("plugins:plex:health")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );
    assert!(
        !plugins
            .find("plugins:plex:disable-sync")
            .expect("sync button should exist")
            .enabled
    );

    assert!(state.apply(GuiShellAction::SetPluginEnabled {
        plugin: GuiPluginSelection::Plex,
        enabled: true,
    }));
    assert_eq!(
        state.configuration.to_stored_settings().plex_plugin_enabled,
        Some(true)
    );
    assert!(state.plex.enabled);
    assert!(state.plex.streaming_enabled);
}

#[test]
fn gui_shell_app_state_projects_empty_plex_server_discovery_status() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            authenticated: true,
            status: "ready".to_owned(),
            ..GuiPlexRuntimeSnapshot::default()
        }
    )));

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:plex:status:servers")
            .and_then(|node| node.value.as_deref()),
        Some("none found")
    );
    assert!(
        plugins
            .find("plugins:plex:enable-streaming")
            .is_some_and(|node| node.enabled),
        "Plex streaming can resolve plex:// machine URIs through accessible servers without a preselected server"
    );
}

#[test]
fn gui_shell_app_state_projects_plex_as_connection_and_server_cards() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            authenticated: true,
            selected_server_id: Some("raptor-machine".to_owned()),
            selected_server_url: Some("https://raptor.example:32400".to_owned()),
            servers: vec![
                GuiPlexServerRow {
                    name: "Raptor".to_owned(),
                    machine_identifier: "raptor-machine".to_owned(),
                    uri: "https://raptor.example:32400".to_owned(),
                    reachability: GuiPlexServerReachability::Reachable,
                    connection_kind: PlexServerConnectionKind::Remote,
                    has_local_connection: true,
                    owned: true,
                    selected: true,
                },
                GuiPlexServerRow {
                    name: "Tower".to_owned(),
                    machine_identifier: "tower-machine".to_owned(),
                    uri: "https://tower.example:32400".to_owned(),
                    reachability: GuiPlexServerReachability::Unreachable,
                    connection_kind: PlexServerConnectionKind::Remote,
                    has_local_connection: false,
                    owned: false,
                    selected: false,
                },
            ],
            status: "ready".to_owned(),
            ..GuiPlexRuntimeSnapshot::default()
        }
    )));

    let plugins = state.plugins_widget_tree();
    assert!(plugins.find("plugins:plex:status:sync").is_none());
    assert!(plugins.find("plugins:plex:status:auth").is_none());
    assert!(plugins.find("plugins:plex:status:state").is_none());
    assert_eq!(
        plugins
            .find("plugins:plex:connect")
            .map(|node| node.label.as_str()),
        None
    );
    assert_eq!(
        plugins
            .find("plugins:plex:disconnect")
            .map(|node| node.label.as_str()),
        Some("Disconnect Plex")
    );
    let selected = plugins
        .find("plugins:plex:server:0")
        .expect("selected Plex server row should exist");
    assert!(selected.selected);
    assert_eq!(selected.label, "Raptor");
    assert_eq!(
        selected.value.as_deref(),
        Some("local server · route: remote · https://raptor.example:32400")
    );
}

#[test]
fn gui_shell_app_state_treats_plex_media_miss_as_sync_issue_not_plugin_error() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex)));
    let miss = "No unambiguous Plex match for [EG]Gurren_Lagann_03_BD(720p_10bit)[BB5590A5].mkv"
        .to_owned();
    let server = GuiPlexServerRow {
        name: "Raptor".to_owned(),
        machine_identifier: "raptor-machine".to_owned(),
        uri: "https://raptor.example:32400".to_owned(),
        reachability: GuiPlexServerReachability::Reachable,
        connection_kind: PlexServerConnectionKind::Remote,
        has_local_connection: true,
        owned: true,
        selected: true,
    };

    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            enabled: true,
            streaming_enabled: true,
            authenticated: true,
            selected_server_id: Some("raptor-machine".to_owned()),
            selected_server_url: Some("https://raptor.example:32400".to_owned()),
            servers: vec![server.clone()],
            status: "ready".to_owned(),
            last_error: Some(miss.clone()),
            ..GuiPlexRuntimeSnapshot::default()
        }
    )));

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:plex:title")
            .and_then(|node| node.value.as_deref()),
        Some("Plex sync active")
    );
    assert_eq!(
        plugins
            .find("plugins:plex:health")
            .and_then(|node| node.value.as_deref()),
        Some("enabled")
    );
    assert_ne!(
        plugins
            .find("plugins:plex:summary")
            .and_then(|node| node.value.as_deref()),
        Some(miss.as_str())
    );
    let issue = plugins
        .find("plugins:plex:status:last-issue")
        .expect("per-media Plex miss should be shown as a neutral issue");
    assert_eq!(issue.label, "Last Sync Issue");
    assert_eq!(issue.value.as_deref(), Some(miss.as_str()));
    assert!(plugins.find("plugins:plex:status:error").is_none());

    assert!(state.apply(GuiShellAction::ApplyGuiPlexRuntimeSnapshot(
        GuiPlexRuntimeSnapshot {
            enabled: true,
            streaming_enabled: true,
            authenticated: true,
            selected_server_id: Some("raptor-machine".to_owned()),
            selected_server_url: Some("https://raptor.example:32400".to_owned()),
            servers: vec![server],
            status: "error".to_owned(),
            last_error: Some("Plex server rejected the timeline update.".to_owned()),
            ..GuiPlexRuntimeSnapshot::default()
        }
    )));

    let plugins = state.plugins_widget_tree();
    assert_eq!(
        plugins
            .find("plugins:plex:health")
            .and_then(|node| node.value.as_deref()),
        Some("error")
    );
    assert_eq!(
        plugins
            .find("plugins:plex:status:error")
            .map(|node| node.label.as_str()),
        Some("Last Error")
    );
}

#[test]
fn gui_shell_app_state_projects_stream_helper_remediation_progress_into_widgets() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        state.apply(GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            GuiStreamHelperRuntimeSnapshot {
                health: GuiStreamHelperHealth::MissingJsRuntime,
                message: Some("Import Deno or install the managed runtime.".to_owned()),
                target: Some("https://www.youtube.com/watch?v=UyjIPZfygTk".to_owned()),
                install_supported: true,
                integration_supported: true,
                retry_available: true,
                install_location: Some("C:/Users/test/AppData/Roaming/Sorotte/tools/stream-helper/bin".to_owned()),
                downloader_status: Some("Managed install: 2025.01.01 (C:/Users/test/AppData/Roaming/Sorotte/tools/stream-helper/bin/yt-dlp.exe)".to_owned()),
                js_runtime_status: Some("Missing from Sorotte's managed install and PATH for Deno.".to_owned()),
                open_install_location_available: true,
            },
        ))
    );
    assert!(state.apply(
        GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(
            GuiStreamHelperRemediationRuntimeSnapshot {
                active: true,
                label: Some("Downloading yt-dlp".to_owned()),
                detail: Some("Saving yt-dlp into Sorotte's helper directory.".to_owned()),
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
