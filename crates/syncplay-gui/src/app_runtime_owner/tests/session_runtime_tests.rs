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
        search_requests: Vec<Vec<String>>,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn drain_gui_actions(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
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

        fn search_missing_media(
            &mut self,
            directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .search_requests
                .push(directories);
            Ok(Some("C:/Media/found.mkv".to_owned()))
        }
    }

    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingSessionRuntimeAdapter {
            state: session_state.clone(),
        }),
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
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
        GuiShellAction::ApplyGuiRuntimeSnapshot(SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::PublicServers,
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
    assert_eq!(inbound_actions.len(), 3);
    assert!(matches!(
        &inbound_actions[0],
        GuiShellAction::PushChatMessage { sender, message }
            if sender == "Server" && message == "Welcome."
    ));
    assert!(matches!(
        &inbound_actions[1],
        GuiShellAction::ApplyGuiRuntimeSnapshot(snapshot)
            if snapshot.active_view == GuiShellView::PublicServers
    ));
    assert_eq!(
        inbound_actions[2],
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
            },
            pending_operation: None,
        })
    );
    for action in inbound_actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.active_view, GuiShellView::PublicServers);
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
        Some("hello")
    );

    handle.push_request(GuiRuntimeRequest::SendChatMessage("slash hello".to_owned()));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let direct_chat_actions = handle.drain_actions();
    assert_eq!(
        direct_chat_actions,
        vec![
            GuiShellAction::PushChatMessage {
                sender: "You".to_owned(),
                message: "slash hello".to_owned(),
            },
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Chat sent.".to_owned(),
            },
        ]
    );
    for action in direct_chat_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("slash hello")
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
    assert_eq!(
        refresh_actions,
        vec![GuiShellAction::CompletePublicServerRefresh(vec![(
            "Runtime".to_owned(),
            "runtime.example:9000".to_owned(),
        )])]
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
    assert_eq!(
        search_actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            "C:/Media/found.mkv".to_owned(),
        ))]
    );
    for action in search_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Missing media found: C:/Media/found.mkv.")
    );

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

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
    assert_eq!(
        session_state.search_requests,
        vec![vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]]
    );
}
