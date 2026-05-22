use super::*;

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_session_runtime_for_session_requests() {
    #[derive(Debug, Default)]
    struct RecordingSessionState {
        queued_gui_actions: Vec<GuiShellAction>,
        room_requests: Vec<String>,
        local_ready_requests: Vec<bool>,
        user_ready_requests: Vec<(String, bool)>,
        controller_auth_requests: Vec<(String, String)>,
        sent_chat_messages: Vec<String>,
        connect_requests: Vec<Option<(String, String)>>,
        refresh_requests: Vec<Vec<(String, String)>>,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn drain_gui_actions(&mut self, _state: &SorotteGuiShellAppState) -> Vec<GuiShellAction> {
            std::mem::take(
                &mut self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .queued_gui_actions,
            )
        }

        fn set_room(&mut self, room: String) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .room_requests
                .push(room);
            Ok(())
        }

        fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_ready_requests
                .push(ready);
            Ok(())
        }

        fn set_user_ready(&mut self, username: String, ready: bool) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .user_ready_requests
                .push((username, ready));
            Ok(())
        }

        fn request_controller_auth(
            &mut self,
            room: String,
            password: String,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .controller_auth_requests
                .push((room, password));
            Ok(())
        }

        fn send_chat_message(&mut self, message: String) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .sent_chat_messages
                .push(message);
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .connect_requests
                .push(selected_server);
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .refresh_requests
                .push(current_servers);
            Ok(vec![(
                "Runtime".to_owned(),
                "runtime.example:9000".to_owned(),
            )])
        }

        fn missing_media_search_target_file_name(&self) -> Result<String, String> {
            Ok("found.mkv".to_owned())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Err("owner-side missing-media resolution should be used instead".to_owned())
        }
    }

    let media_root = test_temp_root("session-runtime-missing-media");
    let nested_media_root = media_root.join("nested");
    std::fs::create_dir_all(&nested_media_root)
        .expect("session-runtime missing-media fixture directory should be created");
    let found_media_path = nested_media_root.join("found.mkv");
    std::fs::write(&found_media_path, b"test")
        .expect("session-runtime missing-media fixture should be written");

    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingSessionRuntimeAdapter {
            state: session_state.clone(),
        }),
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut inbound_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    inbound_snapshot.chat.push(MainWindowRuntimeChatSnapshot {
        sender: "Server".to_owned(),
        message: "Welcome.".to_owned(),
    });

    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .queued_gui_actions = vec![
        GuiShellAction::PushChatMessage {
            sender: "Server".to_owned(),
            message: "Welcome.".to_owned(),
        },
        GuiShellAction::ApplyGuiRuntimeSnapshot(SorotteGuiRuntimeSnapshot {
            active_view: GuiShellView::Setup,
            open_modal: None,
            main_window: inbound_snapshot,
            public_servers: state.public_servers.clone(),
            media_search: state.media_search.clone(),
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }),
    ];
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let inbound_actions = handle.drain_actions();
    assert!(matches!(
        inbound_actions.first(),
        Some(GuiShellAction::PushChatMessage { sender, message })
            if sender == "Server" && message == "Welcome."
    ));
    assert!(matches!(
        inbound_actions.get(1),
        Some(GuiShellAction::ApplyGuiRuntimeSnapshot(snapshot))
            if snapshot.active_view == GuiShellView::Setup
    ));
    assert!(
        inbound_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: true,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: true,
                    can_toggle_pause: false,
                    can_send_chat_message: true,
                    chat_unavailable_reason: _,
                },
                pending_operation: None,
            })
        )),
        "inbound runtime snapshots should still refresh command availability after replaying attached-session actions"
    );
    for action in inbound_actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Welcome.")
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("runtime-room".to_owned()));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let chat_actions = handle.drain_actions();
    assert_eq!(chat_actions, vec![GuiShellAction::CompleteLocalChatSend]);
    for action in chat_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Welcome.")
    );

    handle.push_request(GuiRuntimeRequest::SendChatMessage("slash hello".to_owned()));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let direct_chat_actions = handle.drain_actions();
    assert!(direct_chat_actions.is_empty());
    for action in direct_chat_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Welcome.")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    assert_eq!(
        connect_actions,
        vec![GuiShellAction::CompleteSelectedPublicServerConnect]
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![(
            "Ignored".to_owned(),
            "ignored.example:8999".to_owned(),
        )]),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refresh_actions = handle.drain_actions();
    assert!(
        refresh_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompletePublicServerRefresh(servers)
                if servers == &vec![("Runtime".to_owned(), "runtime.example:9000".to_owned())]
        )),
        "public server refresh should still complete through the attached session runtime"
    );
    for action in refresh_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.as_str(), row.address.as_str()))
            .collect::<Vec<_>>(),
        vec![("Runtime", "runtime.example:9000")]
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let search_actions = handle.drain_actions();
    let mut search_completion_actions = search_actions
        .iter()
        .filter(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_)))
        .cloned()
        .collect::<Vec<_>>();
    let _ = search_actions;
    assert!(
        search_completion_actions.is_empty(),
        "missing-media search should stay pending until the background index completes"
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SearchMissingMedia)
    );
    let search_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while search_completion_actions.is_empty() {
        assert!(
            std::time::Instant::now() < search_deadline,
            "timed out waiting for background missing-media search completion"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        let actions = handle.drain_actions();
        search_completion_actions = actions
            .iter()
            .filter(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_)))
            .cloned()
            .collect();
    }
    assert_eq!(
        search_completion_actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            found_media_path.to_string_lossy().into_owned(),
        ))]
    );
    for action in search_completion_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    let expected_missing_media_message = format!(
        "Missing media found: {}.",
        found_media_path.to_string_lossy()
    );
    assert!(
        state
            .notifications
            .iter()
            .all(|item| item.message != expected_missing_media_message),
        "attached-session missing-media completion should not emit a success notification"
    );

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    let _ = std::fs::remove_dir_all(&media_root);

    handle.push_request(GuiRuntimeRequest::SetReadyForUser {
        username: "bob".to_owned(),
        ready: true,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    handle.push_request(GuiRuntimeRequest::RequestControllerAuth {
        room: "+room:ABCDEF123456".to_owned(),
        password: "ab-123-456".to_owned(),
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    let session_state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(session_state.room_requests, vec!["runtime-room".to_owned()]);
    assert_eq!(session_state.local_ready_requests, vec![true]);
    assert_eq!(
        session_state.user_ready_requests,
        vec![("bob".to_owned(), true)]
    );
    assert_eq!(
        session_state.controller_auth_requests,
        vec![("+room:ABCDEF123456".to_owned(), "ab-123-456".to_owned())]
    );
    assert_eq!(
        session_state.sent_chat_messages,
        vec!["hello".to_owned(), "slash hello".to_owned()]
    );
    assert_eq!(
        session_state.connect_requests,
        vec![Some(("Primary".to_owned(), "syncplay.pl:8999".to_owned()))]
    );
    assert_eq!(
        session_state.refresh_requests,
        vec![vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]]
    );

    let _ = std::fs::remove_dir_all(&media_root);
}
