use super::{
    GuiAppHost, GuiNativeApp, GuiNativeRuntimeBridge, GuiPreviewRuntimeBridge, GuiShellAction,
    GuiShellDispatchPlan, GuiTextPreviewHost, GuiTransientNotificationLevel, GuiWidgetEguiRenderer,
    SyncplayGuiShellAppState,
};

use crate::app::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use crate::app::{
    GuiConfigurationTab, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiRuntimeRequest, GuiShellModal, GuiShellView,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_text_preview_host_uses_summary_and_widget_tree_output() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Shell App State]"));
    assert!(rendered.contains("[Widget Tree]"));
    assert!(rendered.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_native_app_and_preview_runtime_map_seek_prompt_input_to_runtime_actions() {
    assert_eq!(
        GuiNativeApp::parse_seek_offset_seconds(" 12.5 "),
        Some(12.5)
    );
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds("NaN"), None);
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds(""), None);

    let mut runtime = GuiPreviewRuntimeBridge;
    assert_eq!(
        runtime.actions_for_seek_offset(12.5),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 12.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 12.5 seconds.".to_owned(),),
        ]
    );
}

#[test]
fn gui_native_app_reads_drag_and_drop_test_override_from_lookup() {
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_DROP_FILE_PATHS" => {
                Some("  C:/Drops/episode1.mkv | D:/Alt/episode2.mp4 ".to_owned())
            }
            "SYNCPLAY_GUI_TEST_DROP_TARGET" => Some(" playlist ".to_owned()),
            _ => None,
        })
        .expect("drop override should parse"),
        Some(GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec![
                "C:/Drops/episode1.mkv".to_owned(),
                "D:/Alt/episode2.mp4".to_owned(),
            ],
            playlist_insert_slot: None,
        })
    );
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|_name| None)
            .expect("missing drop override should not fail"),
        None
    );
}

#[test]
fn gui_text_preview_host_renders_player_setup_shell_state() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "player-setup.example".to_owned(),
    }));
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

    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Player Setup] status=not-configured"));
    assert!(rendered.contains("id=config-player-setup"));
    assert!(rendered.contains("id=shell:modal:player-setup:retry"));
}

#[test]
fn gui_native_app_routes_player_setup_modal_retry_through_runtime_dispatch() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::MissingBinary,
                    message: "GUI-owned mpv launch failed from saved player path.".to_owned(),
                }),
            },
        ))
    );
    assert_eq!(state.open_modal, Some(GuiShellModal::PlayerSetup));

    let retry_button = state
        .shell_widget_tree()
        .find("shell:modal:player-setup:retry")
        .cloned()
        .expect("player setup retry button should exist");
    let actions = GuiWidgetEguiRenderer::actions_for_clicked_button(&state, &retry_button);
    let dispatch_plan = GuiShellDispatchPlan::from_shell_actions(&state, actions);

    assert!(dispatch_plan.shell_actions.is_empty());
    assert_eq!(
        dispatch_plan.runtime_requests,
        vec![GuiRuntimeRequest::RetryPlayerLaunch]
    );

    let mut runtime = GuiPreviewRuntimeBridge;
    let preview_actions = GuiNativeRuntimeBridge::dispatch_runtime_request(
        &mut runtime,
        &state,
        GuiRuntimeRequest::RetryPlayerLaunch,
    );
    assert!(preview_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Info,
            message,
        } if message == "Retrying mpv launch with the current player settings."
    )));
}

#[test]
fn gui_native_app_routes_player_setup_modal_open_settings_to_connection_tab() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PrivacyChat,
    )));
    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::MissingBinary,
                    message: "GUI-owned mpv launch failed from saved player path.".to_owned(),
                }),
            },
        ))
    );
    assert_eq!(state.open_modal, Some(GuiShellModal::PlayerSetup));

    let open_settings = state
        .shell_widget_tree()
        .find("shell:modal:player-setup:open-settings")
        .cloned()
        .expect("player setup open-settings button should exist");
    let actions = GuiWidgetEguiRenderer::actions_for_clicked_button(&state, &open_settings);
    for action in actions {
        assert!(state.apply(action));
    }

    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Connection
    );
    assert_eq!(state.open_modal, None);
}
