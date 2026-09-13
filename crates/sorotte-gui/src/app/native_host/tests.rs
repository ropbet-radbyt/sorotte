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
use sorotte_client_app::app_boundary::state::StoredClientSettings;

fn queued_native_app() -> (GuiNativeApp, super::GuiQueuedRuntimeBridgeHandle) {
    let (runtime, handle) = super::GuiQueuedRuntimeBridge::new();
    let app = GuiNativeApp {
        state: SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default()),
        runtime: Box::new(runtime),
        runtime_pump: Box::new(super::GuiNoopRuntimePump),
        runtime_repaint_handle: None,
        gui_state_root: None,
        test_drop_request: None,
        playback_prompt: None,
        playback_prompt_buffer: String::new(),
        playback_prompt_error: None,
    };
    (app, handle)
}

#[test]
fn minimized_native_app_drains_runtime_actions_without_rendering() {
    let (mut app, handle) = queued_native_app();
    let context = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let initial_messages = app.state.main_window.chat.len();
    for i in 0..32 {
        handle.push_action(GuiShellAction::AnnounceSystemChatEvent(format!(
            "event {i}"
        )));
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .minimized = Some(true);
        let _ = context.run_logic(&input, |ctx| eframe::App::logic(&mut app, ctx, &mut frame));
    }
    assert!(
        handle.drain_actions().is_empty(),
        "minimized windows must consume runtime output instead of accumulating a restore backlog"
    );
    assert_eq!(app.state.main_window.chat.len(), initial_messages + 32);
}

#[test]
fn minimized_native_app_dispatches_pending_work_once_and_accepts_completion() {
    let (mut app, handle) = queued_native_app();
    app.state.pending_operation = Some(crate::app::GuiPendingOperationState {
        kind: crate::app::GuiPendingOperationKind::SetPlaybackPause(true),
    });
    let context = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut input = egui::RawInput::default();
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    for _ in 0..4 {
        let _ = context.run_logic(&input, |ctx| eframe::App::logic(&mut app, ctx, &mut frame));
    }
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::CompletePendingOperation(
            crate::app::GuiPendingCompletionRequest::SetPlaybackPause(true),
        )]
    );
    handle.push_action(GuiShellAction::CompletePlaybackPauseState(true));
    let _ = context.run_logic(&input, |ctx| eframe::App::logic(&mut app, ctx, &mut frame));
    assert!(app.state.pending_operation.is_none());
    assert!(handle.drain_requests().is_empty());
}

#[test]
fn minimized_native_app_closes_only_after_successful_update_launch() {
    for success in [false, true] {
        let (mut app, handle) = queued_native_app();
        handle.push_action(GuiShellAction::ApplyStagedUpdateLaunchResult(
            UpdateApplyLaunchResult {
                success,
                message: "update launch result".to_owned(),
            },
        ));
        let context = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .minimized = Some(true);
        let output = context.run_logic(&input, |ctx| eframe::App::logic(&mut app, ctx, &mut frame));
        assert_eq!(
            output
                .viewport_commands
                .values()
                .flatten()
                .any(|command| { matches!(command, egui::ViewportCommand::Close) }),
            success
        );
        assert!(handle.drain_actions().is_empty());
    }
}

#[test]
fn display_fixture_theme_selects_the_matching_global_palette() {
    let context = egui::Context::default();
    for (requested, expected) in [("dark", egui::Theme::Dark), ("light", egui::Theme::Light)] {
        GuiNativeApp::apply_test_theme_override_from_lookup(&context, &|key| {
            (key == "SOROTTE_GUI_TEST_THEME").then(|| requested.to_owned())
        });
        assert_eq!(context.theme(), expected);
        assert_eq!(
            context.global_style().visuals.dark_mode,
            expected == egui::Theme::Dark
        );
    }
}

#[test]
fn gui_text_preview_host_uses_summary_and_widget_tree_output() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettings::default()
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());

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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        player_path: Some("C:/totally-missing/mpv.exe".to_owned()),
        ..StoredClientSettings::default()
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
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

#[test]
fn visible_native_app_keeps_dropped_media_through_runtime_round_trips() {
    assert_visible_dropped_media(false);
}

#[test]
fn threaded_visible_native_app_keeps_dropped_media_through_runtime_round_trips() {
    assert_visible_dropped_media(true);
}

fn assert_visible_dropped_media(threaded: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("drag-window-target.mkv");
    std::fs::write(&path, b"drag-window-target").unwrap();
    let (mut app, handle) = queued_native_app();
    let mut owner =
        super::GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
            Some(directory.path().join("sorotte.ini")),
            &|name| (name == "SOROTTE_GUI_ENABLE_TEST_PLAYER").then(|| "true".to_owned()),
        );
    owner.startup_saved_connect_attempted = true;
    app.runtime_pump = if threaded {
        Box::new(
            crate::app::runtime_queue::GuiThreadedRuntimeOwnerPump::new(handle.clone(), owner)
                .unwrap(),
        )
    } else {
        Box::new(crate::app::runtime_queue::GuiQueuedRuntimeOwnerPump::new(
            handle.clone(),
            owner,
        ))
    };
    app.test_drop_request = Some(GuiDroppedFilesRequest {
        paths: vec![path.to_string_lossy().into_owned()],
        target: GuiDroppedFilesTarget::Window,
        playlist_insert_slot: None,
    });
    let context = egui::Context::default();
    context.enable_accesskit();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let repaint_context = context.clone();
    handle.set_repaint_notifier(move || repaint_context.request_repaint());
    app.runtime_repaint_handle = Some(handle);
    let mut labels = std::collections::BTreeSet::new();
    let mut frame = eframe::Frame::_new_kittest();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut frames_after_visible = 0;
    for frame_index in 0.. {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| {
            eframe::App::logic(&mut app, ui.ctx(), &mut frame);
            eframe::App::ui(&mut app, ui, &mut frame);
        });
        output.textures_delta.clear();
        if !threaded && frame_index == 0 {
            assert_eq!(
                app.state.current_shared_playlist_entries(),
                vec!["drag-window-target.mkv".to_owned()],
                "visible input must consume output produced by its runtime pump before yielding"
            );
        }
        if let Some(update) = output.platform_output.accesskit_update {
            for (_, node) in update.nodes {
                if let Some(label) = node.label() {
                    labels.insert(label.to_owned());
                }
            }
        }
        if labels.contains("drag-window-target.mkv") {
            frames_after_visible += 1;
        }
        if frames_after_visible >= 16 || std::time::Instant::now() >= deadline {
            break;
        }
        if threaded {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert!(
        labels.contains("drag-window-target.mkv"),
        "dropped media must be accessible: {labels:?}"
    );
    assert_eq!(app.state.active_view, GuiShellView::Room);
    assert_eq!(
        app.state.current_shared_playlist_entries(),
        vec!["drag-window-target.mkv".to_owned()]
    );
}
