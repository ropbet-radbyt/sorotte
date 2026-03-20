use super::*;

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_controller_auth_transitions_as_notifications()
 {
    let room = "+room:ABCDEF123456";
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", room)
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .runtime
        .session_mut()
        .remember_control_password_for_room(room, "ab-123-456");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    let hello_actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "controller reidentify should surface an attempt notification"
    );
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "controller reidentify should persist the attempt message in system chat"
    );
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_control_status
                    == "Not granted by server: room controls are locked."
        )),
        "controlled-room hello should surface that server control has not been granted yet"
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
        )
        .expect("controller auth success should apply");
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "alice received controller access for +room:ABCDEF123456."
        )),
        "controller auth success should surface a success notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "alice received controller access for +room:ABCDEF123456."
        )),
        "controller auth success should persist the success message in system chat"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.controlled_room_active
                    && snapshot.room_control_status
                        == "Granted by server: you control this room."
                    && snapshot.users.iter().any(|user| {
                        user.username == "alice" && user.is_self && user.is_controller
                    })
        )),
        "controller auth success should refresh the main-window runtime snapshot"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_controlled_room_creation_before_reidentify()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    adapter.runtime.session_mut().set_autoplay_enabled(true);
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    for action in GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"ab 123 456"}}}"#,
        )
        .expect("new controlled room message should apply");
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let created_notice_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Success
                        && message == "Controlled room created: +room:ABCDEF123456."
            )
        })
        .expect("new controlled room should surface a success notification");
    let created_chat_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if message == "Created controlled room +room:ABCDEF123456 with password AB123456 (+room:ABCDEF123456:AB123456)."
            )
        })
        .expect("new controlled room should surface a system chat entry");
    let reidentify_notice_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Info
                        && message == "Requesting controller access for +room:ABCDEF123456."
            )
        })
        .expect("new controlled room should trigger controller reidentify");
    let reidentify_chat_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if message == "Requesting controller access for +room:ABCDEF123456."
            )
        })
        .expect("controller reidentify should be persisted in system chat");
    assert!(
        created_notice_index < reidentify_notice_index,
        "created-room notification should appear before the controller reidentify attempt"
    );
    assert!(
        created_chat_index < reidentify_chat_index,
        "created-room system chat should appear before the controller reidentify entry"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "+room:ABCDEF123456"
                    && snapshot.controlled_room_active
                    && !snapshot.autoplay_active
        )),
        "new controlled room should still refresh the main-window snapshot"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_auto_reidentifies_controlled_room_when_password_is_stored()
 {
    let room = "+room:ABCDEF123456";
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
        "alice",
        room,
        Some("ab-123-456".to_owned()),
    )
    .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    let hello_actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "draining GUI actions should auto-dispatch the controller reidentify attempt"
    );

    let outbound_protocol_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("controller auth lines should encode");
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"controllerAuth\""));
    assert!(outbound_protocol_lines[0].contains("\"+room:ABCDEF123456\""));
    assert!(outbound_protocol_lines[0].contains("\"AB-123-456\""));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_autoplay_countdown_notifications() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .expect("inbound server hello should apply");
    let hello_actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    for action in hello_actions {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        )
        .expect("remote ready user should apply");
    adapter.runtime.session_mut().set_autoplay_enabled(true);
    adapter
        .runtime
        .session_mut()
        .readiness_autoplay_config_mut()
        .auto_play_threshold = Some(2);
    adapter
        .runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true),
        );
    adapter
        .runtime
        .update_autoplay_check(true, true, false, false);
    adapter
        .runtime
        .tick_autoplay(true, true, false, false)
        .expect("first autoplay tick should emit notification");
    adapter
        .runtime
        .tick_autoplay(true, true, false, false)
        .expect("second autoplay tick should emit notification");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Autoplay in 3 seconds with 2 ready users."
        )),
        "first autoplay tick should surface a countdown notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Autoplay in 3 seconds with 2 ready users."
        )),
        "first autoplay tick should persist a countdown entry in system chat"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Autoplay in 2 seconds with 2 ready users."
        )),
        "second autoplay tick should surface a countdown notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Autoplay in 2 seconds with 2 ready users."
        )),
        "second autoplay tick should persist a countdown entry in system chat"
    );
}
