use super::*;

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_session_state_before_server_hello() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_main_window = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_main_window.room_name = "live-room".to_owned();
    stale_main_window.shared_playlist_enabled = true;
    stale_main_window.controlled_room_active = true;
    stale_main_window.users = vec![
        MainWindowRuntimeUserSnapshot {
            username: "alice".to_owned(),
            is_self: true,
            is_ready: true,
            is_controller: true,
            ..Default::default()
        },
        MainWindowRuntimeUserSnapshot {
            username: "bob".to_owned(),
            is_self: false,
            is_ready: false,
            is_controller: false,
            ..Default::default()
        },
    ];
    stale_main_window.playlist = vec!["episode2.mkv".to_owned()];
    stale_main_window.can_set_ready = false;
    stale_main_window.can_manage_playlist = true;
    stale_main_window.playback_paused = true;
    stale_main_window.autoplay_active = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_main_window
    )));
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: true,
            }],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("pre-Hello session state should clear stale main-window runtime state");
    assert_eq!(snapshot.room_name, "room1");
    assert!(!snapshot.shared_playlist_enabled);
    assert!(!snapshot.controlled_room_active);
    assert_eq!(
        snapshot.users,
        vec![browser_runtime_user("alice", "room1", true, false, false)]
    );
    assert!(snapshot.playlist.is_empty());
    assert!(!snapshot.can_set_ready);
    assert!(!snapshot.can_manage_playlist);
    assert!(!snapshot.playback_paused);
    assert!(!snapshot.autoplay_active);

    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("pre-Hello session state should clear stale menu runtime state");
    assert!(
        snapshot
            .action_overrides
            .contains(&MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            })
    );
    assert!(
        snapshot
            .action_overrides
            .contains(&MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            })
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_persists_reconnect_transitions_to_system_chat() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry should queue notifications");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Reconnect attempt 1 in 0.1 seconds."
        )),
        "reconnect retry should surface a warning notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Reconnect attempt 1 in 0.1 seconds."
        )),
        "reconnect retry should persist a system chat entry"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Session reconnected."
        )),
        "reconnect success should surface a success notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Session reconnected."
        )),
        "reconnect success should persist a system chat entry"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_persists_reconnect_state_restore_details_to_system_chat()
 {
    assert_eq!(
        GuiClientCoreChatSessionRuntimeAdapter::reconnect_transition_actions(
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: false,
                room_paused: true,
                local_position: 5.0,
                room_position: 7.5,
                position_diff_seconds: 2.5,
            },
            Some("en"),
        ),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: "Session state restore mismatch detected (2.500 seconds).".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Session state restore mismatch detected (2.500 seconds).".to_owned(),
            ),
        ]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_dispatches_remote_ready_changes_when_supported() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("inbound server hello should apply");

    assert!(
        GuiSessionRuntimeAdapter::set_user_ready(&mut adapter, "bob".to_owned(), true).is_ok(),
        "newer readiness-capable servers should allow remote readiness changes"
    );

    let outbound_protocol_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("remote readiness lines should encode");
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"ready\""));
    assert!(outbound_protocol_lines[0].contains("\"username\":\"bob\""));
    assert!(outbound_protocol_lines[0].contains("\"isReady\":true"));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_rejects_remote_ready_changes_when_unsupported() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.1","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let error = GuiSessionRuntimeAdapter::set_user_ready(&mut adapter, "bob".to_owned(), true)
        .expect_err("older readiness-capable servers should reject remote readiness changes");
    assert!(
        error.contains("remote readiness changes"),
        "error should identify the missing remote readiness capability"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_restores_readiness_controls_after_server_hello() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.can_set_ready = false;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.can_set_ready = true;
    expected_snapshot.can_set_others_ready = true;
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback.can_set_ready);
    assert!(GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconciles_inbound_state_through_runtime() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    GuiSessionRuntimeAdapter::sync_local_playback_telemetry(&mut adapter, Some(false), Some(12.0))
        .expect("local playback telemetry should sync");

    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#,
        )
        .expect("inbound state should reconcile through client runtime");

    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconciled state response should encode");
    assert_eq!(outbound_lines.len(), 1);
    assert!(outbound_lines[0].contains("\"State\""));
    assert!(outbound_lines[0].contains("\"position\":12.0"));
    assert!(outbound_lines[0].contains("\"paused\":false"));
    assert!(outbound_lines[0].contains("\"latencyCalculation\":123.0"));
}
