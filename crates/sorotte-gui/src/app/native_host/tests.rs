use super::{
    GuiAppHost, GuiNativeApp, GuiNativeRuntimeBridge, GuiNativeShellEffect, GuiPlaybackPromptKind,
    GuiPreviewRuntimeBridge, GuiShellAction, GuiShellDispatchPlan, GuiTextPreviewHost,
    GuiTransientNotificationLevel, GuiWidgetEguiRenderer, SorotteGuiShellAppState,
};

use crate::app::remote_services::UpdateApplyLaunchResult;
use crate::app::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use crate::app::{
    GuiConfigurationTab, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiRuntimeRequest, GuiShellModal, GuiShellView, MenuActionId,
    SettingId,
};
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_text_preview_host_uses_summary_and_widget_tree_output() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Shell App State]"));
    assert!(rendered.contains("[Widget Tree]"));
    assert!(rendered.contains("- Sorotte GUI [panel] id=shell-root"));
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
fn gui_native_menu_effects_are_typed_and_only_run_after_reducer_acceptance() {
    for (action, expected_effect) in [
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::OpenMedia),
            GuiNativeShellEffect::PickMediaFiles,
        ),
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::Exit),
            GuiNativeShellEffect::CloseWindow,
        ),
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::Seek),
            GuiNativeShellEffect::OpenPlaybackPrompt(GuiPlaybackPromptKind::Seek),
        ),
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::UndoSeek),
            GuiNativeShellEffect::RequestUndoSeek,
        ),
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::SetOffset),
            GuiNativeShellEffect::OpenPlaybackPrompt(GuiPlaybackPromptKind::Offset),
        ),
        (
            GuiShellAction::InvokeMenuAction(MenuActionId::Help),
            GuiNativeShellEffect::OpenHelp,
        ),
    ] {
        assert_eq!(
            GuiNativeApp::native_effect_for_applied_action(&action, true),
            Some(expected_effect),
        );
        assert_eq!(
            GuiNativeApp::native_effect_for_applied_action(&action, false),
            None,
            "a rejected command must not open a picker, prompt, URL, or dispatch undo",
        );
    }

    let mut disabled_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    for action_id in [
        MenuActionId::OpenMedia,
        MenuActionId::Seek,
        MenuActionId::UndoSeek,
        MenuActionId::SetOffset,
    ] {
        let action = GuiShellAction::InvokeMenuAction(action_id);
        let action_applied = disabled_state.apply(action.clone());
        assert!(!action_applied);
        assert_eq!(
            GuiNativeApp::native_effect_for_applied_action(&action, action_applied),
            None,
        );
    }
}

#[test]
fn gui_native_app_closes_after_successful_update_helper_launch() {
    assert!(GuiNativeApp::action_requests_app_close(
        &GuiShellAction::ApplyStagedUpdateLaunchResult(UpdateApplyLaunchResult {
            success: true,
            message: "Update helper started.".to_owned(),
        })
    ));
    assert!(!GuiNativeApp::action_requests_app_close(
        &GuiShellAction::ApplyStagedUpdateLaunchResult(UpdateApplyLaunchResult {
            success: false,
            message: "failed to launch update helper".to_owned(),
        })
    ));
}

#[test]
fn gui_native_app_reads_drag_and_drop_test_override_from_lookup() {
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|name| match name {
            "SOROTTE_GUI_TEST_DROP_FILE_PATHS" => {
                Some("  C:/Drops/episode1.mkv | D:/Alt/episode2.mp4 ".to_owned())
            }
            "SOROTTE_GUI_TEST_DROP_TARGET" => Some(" playlist ".to_owned()),
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "player-setup.example".to_owned().into(),
    }));
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

    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Player Setup] status=not-configured"));
    assert!(rendered.contains("id=config-player-setup"));
    assert!(rendered.contains("id=shell:modal:player-setup:retry"));
}

#[test]
fn gui_native_app_routes_player_setup_modal_retry_through_runtime_dispatch() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::MissingBinary,
                    message: "GUI-owned mpv launch failed from saved player path.".to_owned(),
                    retry_available: true,
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
fn repro_retryable_streaming_hook_warning_does_not_interrupt_playback_with_setup_modal() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::PlayerSettingsDegraded,
                    message: "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry: operation failed: hook lease expired".to_owned(),
                    retry_available: true,
                }),
            },
        ))
    );

    assert!(
        state.player_setup_issue.is_some(),
        "the retryable warning should remain available from the non-modal setup status"
    );
    assert_eq!(
        state.open_modal, None,
        "a retryable hook-health warning that explicitly leaves playback available must not seize focus with the setup-required modal"
    );
}

#[test]
fn gui_native_app_routes_player_setup_modal_open_settings_to_connection_tab() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Room)));
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::PrivacyChat,
    )));
    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::MissingBinary,
                    message: "GUI-owned mpv launch failed from saved player path.".to_owned(),
                    retry_available: true,
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

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Connection
    );
    assert_eq!(state.open_modal, None);
}

#[test]
fn gui_native_app_preserves_active_playlist_index_for_replace_requests_when_selection_is_local() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
            "Three".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(1)));
    assert_eq!(
        GuiNativeApp::preserve_active_playlist_request_index(&state),
        Some(1)
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(2)));
    assert!(state.main_window_playlist_selection_is_local);
    assert_eq!(
        GuiNativeApp::preserve_active_playlist_request_index(&state),
        None,
        "playlist replace/reorder requests should preserve the synced room index when the UI row highlight is local-only"
    );
}
