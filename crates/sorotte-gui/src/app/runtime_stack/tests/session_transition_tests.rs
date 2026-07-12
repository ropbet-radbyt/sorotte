use super::*;

use crate::app::support::system_time_seconds;
use sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible;

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_session_state_before_server_hello() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn rich_transport_keeps_steady_state_attached_drift_correction() {
    fn observed(
        seconds: f64,
        phase: PlayerTransportPhase,
        position: f64,
        paused: bool,
    ) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(std::time::Duration::from_secs_f64(
                seconds,
            )),
        )
        .with_phase(phase)
        .with_position_seconds(position)
        .with_logical_pause(paused);
        update.paused_for_cache = Some(false);
        update.seeking = Some(false);
        update.seekable = Some(true);
        update.core_idle = Some(false);
        update
    }

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .unwrap();
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#,
        )
        .unwrap();
    let now = system_time_seconds();
    adapter
        .prepare_attached_playback_media(
            LogicalMediaId::new("episode.mkv").unwrap(),
            MediaTransportKind::LocalFile,
            now,
        )
        .unwrap();
    adapter
        .sync_attached_player_transport_telemetry(
            observed(1.0, PlayerTransportPhase::ReadyPaused, 20.0, true),
            now,
        )
        .unwrap();
    adapter
        .sync_attached_player_transport_telemetry(
            observed(2.0, PlayerTransportPhase::Playing, 20.0, false),
            now + 1.0,
        )
        .unwrap();
    adapter
        .sync_attached_player_transport_telemetry(
            observed(3.0, PlayerTransportPhase::Playing, 20.2, false),
            now + 2.0,
        )
        .unwrap();

    adapter
        .sync_local_playback_telemetry(Some(false), Some(40.0))
        .unwrap();
    adapter
        .sync_attached_player_transport_telemetry(
            observed(4.0, PlayerTransportPhase::Playing, 40.0, false),
            now + 3.0,
        )
        .unwrap();
    let ahead_actions = adapter.attached_player_runtime_actions(now + 3.0).unwrap();
    assert!(ahead_actions.iter().any(|action| matches!(
        action,
        GuiAttachedPlayerRuntimeAction::Position(position) if *position < 30.0
    )));

    adapter
        .sync_local_playback_telemetry(Some(false), Some(0.0))
        .unwrap();
    adapter.dont_slow_down_with_me = true;
    adapter
        .sync_attached_player_transport_telemetry(
            observed(5.0, PlayerTransportPhase::Playing, 0.0, false),
            now + 4.0,
        )
        .unwrap();
    let _ = adapter.attached_player_runtime_actions(now + 4.0).unwrap();
    let behind_actions = adapter.attached_player_runtime_actions(now + 9.0).unwrap();
    assert!(behind_actions.iter().any(|action| matches!(
        action,
        GuiAttachedPlayerRuntimeAction::Position(position) if *position > 20.0
    )));
    assert!(!behind_actions.iter().any(|action| matches!(
        action,
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_dispatches_ready_at_start_after_server_hello() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ready_at_start: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(!state.main_window.users[0].is_ready);

    let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(
        &state.configuration.to_stored_settings(),
    );
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the session");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("inbound server hello should apply");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("ready-at-start lines should encode");
    assert_eq!(outbound_lines.len(), 1);
    assert!(outbound_lines[0].contains(r#""Set":{"ready":{"isReady":true"#));
    assert!(outbound_lines[0].contains(r#""manuallyInitiated":false"#));

    adapter
        .apply_message_json(&outbound_lines[0])
        .expect("ready-at-start echo should apply");
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("ready-at-start echo should surface a runtime snapshot");
    assert!(
        snapshot
            .users
            .iter()
            .any(|user| user.username == "alice" && user.is_self && user.is_ready)
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_applies_batched_top_level_commands() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}},"Chat":{"username":"bob","message":"hello room"}}"#,
        )
        .expect("batched server message should apply");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "bob" && message == "hello room"
        )),
        "batched Chat command should be applied after Hello; actions were {actions:?}"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_applies_valid_prefix_before_batched_unknown() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let result = adapter.apply_message_json(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}},"Bogus":{"x":1}}"#,
    );

    assert!(
        result.is_err(),
        "batched unknown command should still surface a protocol error"
    );
    assert_eq!(
        adapter.runtime.session().username(),
        Some("alice"),
        "valid Hello before the unknown command should be applied"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_requests_user_list_on_first_state_without_media() {
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
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#,
        )
        .expect("first inbound state should apply");

    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("first-state follow-up lines should encode");
    assert!(
        outbound_lines.iter().any(|line| line.contains(r#""List""#)),
        "connecting without media should request the user list on the first inbound state"
    );

    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":11.0,"paused":true,"doSeek":false,"setBy":"bob"},"ping":{"latencyCalculation":124.0}}}"#,
        )
        .expect("second inbound state should apply");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("second-state follow-up lines should encode");
    assert!(
        !outbound_lines.iter().any(|line| line.contains(r#""List""#)),
        "the automatic user-list request should only happen once"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_requests_user_list_on_first_state_with_local_media()
{
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
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
        )
        .expect("local file metadata should apply before the first state");
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#,
        )
        .expect("first inbound state should apply");

    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("first-state follow-up lines should encode");
    assert!(
        outbound_lines.iter().any(|line| line.contains(r#""List""#)),
        "connecting with media already loaded should still request the user list on the first inbound state"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_projects_remote_user_after_playlist_seed() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("smoke-user".to_owned()),
        room: Some("smoke-room".to_owned()),
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("smoke-user", "smoke-room")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    for line in [
        r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        r#"{"Set":{"playlistChange":{"files":["missing-source-a.mkv","missing-target.mkv"],"user":"smoke-user"}}}"#,
        r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
        r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
        r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"smoke-user"},"ping":{"latencyCalculation":123.0}}}"#,
    ] {
        adapter
            .apply_message_json(line)
            .expect("inbound missing-media seed line should apply");
    }
    for action in GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "bob" && user.is_ready && user.is_controller),
        "remote participant from the missing-media seed should be projected"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_stops_reconnect_on_server_error() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let error = adapter
        .apply_message_json(r#"{"Error":{"message":"wrong-password-server-error"}}"#)
        .expect_err("server error frames should fail the session adapter");

    assert!(
        error.contains("wrong-password-server-error"),
        "server errors should surface the server-provided message"
    );
    assert!(
        adapter.runtime.take_stop_reconnect_requested(),
        "server error frames should stop the reconnect loop before the transport closes"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_persists_reconnect_transitions_to_system_chat() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_client_core_chat_session_runtime_adapter_dispatches_reconnect_playlist_restore_messages() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("initial server hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("local playlist should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");

    adapter
        .runtime
        .session_mut()
        .reset_sync_state_for_reconnect();
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("reconnect hello should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty reconnect playlist snapshot should apply");

    let _ = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect playlist restore lines should encode");
    assert!(
        outbound_lines.iter().any(|line| {
            line.contains("\"playlistChange\"")
                && line.contains("\"episode1.mkv\"")
                && line.contains("\"episode2.mkv\"")
        }),
        "reconnect playlist restore should republish the local playlist"
    );
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":1")),
        "reconnect playlist restore should republish the selected playlist index"
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
fn gui_client_core_chat_session_runtime_adapter_rejects_controller_auth_when_managed_rooms_are_unsupported()
 {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"managedRooms":false}}}"#,
        )
        .expect("inbound server hello should apply");

    let error = GuiSessionRuntimeAdapter::request_controller_auth(
        &mut adapter,
        "+room:ABCDEF123456".to_owned(),
        "AB-123-456".to_owned(),
    )
    .expect_err("servers without managedRooms support should reject controller auth");
    assert!(
        error.contains("controlled-room support"),
        "error should identify the missing managedRooms capability"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_restores_readiness_controls_after_server_hello() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(false),
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
    expected_snapshot.room_control_status =
        "Not required: current room is not controlled.".to_owned();
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
fn gui_client_core_chat_session_runtime_adapter_disables_remote_readiness_without_control() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room:ABCDEF123456".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "+room:ABCDEF123456")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true,"readiness":true,"setOthersReadiness":true,"managedRooms":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("server hello should produce a main-window runtime snapshot");
    assert!(snapshot.can_set_ready);
    assert!(!snapshot.can_set_others_ready);
    assert_eq!(
        snapshot.room_control_status,
        "Not granted by server: room controls are locked."
    );
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
    assert_eq!(outbound_lines.len(), 2);
    let state_line = outbound_lines
        .iter()
        .find(|line| line.contains("\"State\""))
        .expect("state reconciliation should emit an outbound state line");
    assert!(state_line.contains("\"position\":12.0"));
    assert!(state_line.contains("\"paused\":false"));
    assert!(state_line.contains("\"latencyCalculation\":123.0"));
    assert!(
        outbound_lines.iter().any(|line| line.contains("\"List\"")),
        "first inbound state without local media should also request the user list",
    );
}
