use super::*;

#[test]
fn gui_persisted_config_runtime_owner_loopback_transport_echoes_client_core_chat() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);

    let actions = handle.drain_actions();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "loopback transport should preserve the local send completion"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "loopback transport should feed the encoded chat line back through inbound handling"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("alice".to_owned(), "hello room".to_owned()))
    );
    assert_eq!(state.main_window.chat.len(), 1);
}

#[test]
fn gui_persisted_config_runtime_owner_manual_disconnect_applies_pause_on_leave() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.commands.can_disconnect_session);
    assert!(state.apply(GuiShellAction::BeginSessionDisconnect));

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::DisconnectSession,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&true),
        "explicit disconnect should still pause the attached player"
    );
    assert!(owner.session.is_none());
    assert!(state.pending_operation.is_none());
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| notification.message != "Session disconnected."),
        "disconnect completion should no longer emit a success notification"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_discards_attached_player_chat_without_a_sendable_session() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        pending_chat_requests: std::collections::VecDeque<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_pending_chat_request(&mut self) -> Option<String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pending_chat_requests
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState {
        pending_chat_requests: std::collections::VecDeque::from(["hello from mpv".to_owned()]),
    }));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.notifications.iter().any(|notification| {
            notification.message
                == "Chat input from the attached player requires an active session with chat support."
        }),
        "player chat typed without an active session should be rejected immediately"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_chat_requests
            .is_empty(),
        "unsendable player chat should be drained instead of leaking into a later session"
    );

    let (next_owner, session_transport) = owner
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap after player-chat rejection");
    let mut owner = next_owner;

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(
        startup_protocol_lines
            .iter()
            .all(|line| !line.contains("\"Chat\"")),
        "only the startup hello should be queued after late session bootstrap"
    );

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        session_transport.drain_outbound_protocol_lines().is_empty(),
        "rejected player chat must not be sent after the later session handshake"
    );
}
