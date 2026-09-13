use super::*;

fn pump_session(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) {
    for _ in 0..8 {
        GuiQueuedRuntimeOwner::pump(owner, handle, state);
        for action in handle.drain_actions() {
            state.apply(action);
        }
    }
}

#[test]
fn attached_session_requests_reach_the_wire_and_server_events_reach_the_shell() {
    let (mut owner, transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .unwrap();
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    transport.push_inbound_protocol_line(r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true,"setOthersReadiness":true,"managedRooms":true}}}"#);
    transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":false}}}}"#,
    );
    transport.push_inbound_protocol_line(r#"{"Chat":{"username":"bob","message":"Welcome."}}"#);
    pump_session(&mut owner, &handle, &mut state);
    assert!(
        state
            .main_window
            .chat
            .iter()
            .any(|row| row.sender == "bob" && row.message == "Welcome.")
    );
    assert!(state.commands.can_send_chat_message);
    transport.take_written_lines();

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    handle.push_request(GuiRuntimeRequest::SendChatMessage("slash hello".to_owned()));
    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    handle.push_request(GuiRuntimeRequest::SetReadyForUser {
        username: "bob".to_owned(),
        ready: true,
    });
    pump_session(&mut owner, &handle, &mut state);
    assert!(state.pending_operation.is_none());
    let lines = transport
        .take_written_lines()
        .into_iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        lines.iter().any(|line| line["Chat"] == "hello"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line["Chat"] == "slash hello"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["Set"]["ready"]["isReady"] == true),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["Set"]["ready"]["username"] == "bob"),
        "{lines:?}"
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("runtime-room".to_owned()));
    handle.push_request(GuiRuntimeRequest::RequestControllerAuth {
        room: "+room:ABCDEF123456".to_owned(),
        password: "ab-123-456".into(),
    });
    pump_session(&mut owner, &handle, &mut state);
    let lines = transport
        .take_written_lines()
        .into_iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        lines
            .iter()
            .any(|line| line["Set"]["room"]["name"] == "runtime-room"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["Set"]["controllerAuth"]["room"] == "+room:ABCDEF123456"),
        "{lines:?}"
    );
}

#[test]
fn attached_session_missing_media_search_uses_the_room_file_and_completes() {
    let root = test_temp_root("session-runtime-missing-media");
    let media = root.join("nested").join("found.mkv");
    std::fs::create_dir_all(media.parent().unwrap()).unwrap();
    std::fs::write(&media, b"test").unwrap();
    let session =
        crate::app::runtime_stack::test_support::session_with_media_target("found.mkv".to_owned());
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        media_match_fingerprinting_enabled: Some(false),
        ..StoredClientSettings::default()
    });
    pump_session(&mut owner, &handle, &mut state);
    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut found = false;
    while std::time::Instant::now() < deadline && !found {
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        for action in handle.drain_actions() {
            if let GuiShellAction::CompleteMissingMediaSearch(path) = &action {
                assert_eq!(path.as_deref(), Some(media.to_string_lossy().as_ref()));
                found = true;
            }
            state.apply(action);
        }
        if !found {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(
        found,
        "room file should resolve through the background index"
    );
    assert!(state.pending_operation.is_none());
    assert!(
        !state
            .notifications
            .iter()
            .any(|item| item.message.starts_with("Missing media found:"))
    );
    std::fs::remove_dir_all(root).unwrap();
}
