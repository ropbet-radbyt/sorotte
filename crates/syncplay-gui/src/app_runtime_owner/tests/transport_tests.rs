use super::*;

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_transport_lines() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    assert_eq!(
        startup_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                room_control_status: "Pending: waiting for server room state.".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: false,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Chat",
                    enabled: false,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in startup_actions {
        assert!(state.apply(action));
    }
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(!state.commands.can_send_chat_message);

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    assert_eq!(
        hello_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                room_control_status: "Not required: current room is not controlled.".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: true,
                can_set_others_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Window",
                        action_label: "Show Chat",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Create Controlled Room",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: true,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(outbound_protocol_lines.is_empty());
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| action.enabled)
    );
    assert!(state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let outbound_actions = handle.drain_actions();
    assert!(
        outbound_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "queued owner should still complete the local chat send when the session runtime accepts it"
    );
    for action in outbound_actions {
        assert!(state.apply(action));
    }

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"Chat\""));
    assert!(outbound_protocol_lines[0].contains("hello room"));

    session_transport
        .push_inbound_protocol_line(r#"{"Chat":{"username":"alice","message":"hello room"}}"#);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let inbound_actions = handle.drain_actions();
    assert!(
        inbound_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "queued owner should turn inbound protocol chat into a GUI chat message action"
    );
    for action in inbound_actions {
        assert!(state.apply(action));
    }
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());
    assert_eq!(state.main_window.chat.len(), 1);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let (release_server_tx, release_server_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut chat_line = String::new();
        reader
            .read_line(&mut chat_line)
            .expect("test session transport server should read one outbound chat line");
        stream
            .write_all(br#"{"Chat":{"username":"alice","message":"hello room"}}"#)
            .expect("test session transport server should write one inbound line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound line");
        release_server_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("test session transport server should be releasable after the echo");
        (hello_line, chat_line)
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let mut combined_actions = handle.drain_actions();
    for action in combined_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_ready_rx,
        Duration::from_secs(1),
        "test session transport server hello readiness",
    );

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_sync_actions = handle.drain_actions();
    for action in hello_sync_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    combined_actions.extend(hello_sync_actions);

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let second_actions = handle.drain_actions();
    for action in second_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    combined_actions.extend(second_actions);

    let third_actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .chat
                .last()
                .is_some_and(|entry| entry.sender == "alice" && entry.message == "hello room")
        },
        "the echoed chat message over TCP transport",
    );
    combined_actions.extend(third_actions);

    assert!(
        combined_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "tcp transport should preserve the local send completion"
    );
    assert!(
        combined_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "tcp transport should feed the server response back through the client-core chat adapter"
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("alice".to_owned(), "hello room".to_owned()))
    );
    assert_eq!(state.main_window.chat.len(), 1);

    release_server_tx
        .send(())
        .expect("test session transport server should be releasable");

    let (hello_line, chat_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(chat_line.contains("\"Chat\""));
    assert!(chat_line.contains("hello room"));
}

#[test]
fn gui_persisted_config_runtime_owner_routes_local_readiness_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let _hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .expect("test session transport server should read one outbound ready line");
        stream
            .write_all(br#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("test session transport server should write one inbound ready line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound ready line");
        ready_line
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_ready_rx,
        Duration::from_secs(1),
        "test session transport startup hello",
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let ready_line = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(ready_line.contains("\"Set\""));
    assert!(ready_line.contains("\"ready\""));
    assert!(ready_line.contains("\"isReady\":true"));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .users
                .iter()
                .any(|user| user.username == "alice" && user.is_self && user.is_ready)
        },
        "local readiness update over TCP transport",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_marks_local_open_media_not_ready_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let _hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");

        let mut outbound_lines = Vec::new();
        for _ in 0..4 {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("test session transport server should read one outbound media-open line");
            outbound_lines.push(line);
        }
        outbound_lines
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_ready_rx,
        Duration::from_secs(1),
        "test session transport startup hello",
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/movie.mkv".to_owned()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_lines = server_thread
        .join()
        .expect("test session transport server thread should complete");
    let ready_line = outbound_lines
        .iter()
        .find(|line| line.contains("\"ready\""))
        .expect("media open should emit an outbound readiness update");
    let file_line = outbound_lines
        .iter()
        .find(|line| line.contains("\"file\""))
        .expect("media open should emit an outbound file update");
    assert!(ready_line.contains("\"Set\""));
    assert!(ready_line.contains("\"ready\""));
    assert!(ready_line.contains("\"isReady\":false"));
    assert!(file_line.contains("\"Set\""));
    assert!(file_line.contains("\"file\""));
    assert!(file_line.contains("\"movie.mkv\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .users
                .iter()
                .any(|user| user.username == "alice" && user.is_self && !user.is_ready)
        },
        "local open-media not-ready projection over TCP transport",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_startup_saved_connect_uses_hostname_transport() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("startup hostname transport test should bind");
    let address = listener
        .local_addr()
        .expect("startup hostname transport test should expose a local address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("startup hostname transport test should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("startup hostname transport test should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "startup hostname transport test",
        );
        hello_tx
            .send(hello_line)
            .expect("startup hostname transport test should report the hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("startup hostname transport test should write one inbound hello line");
        stream
            .write_all(b"\r\n")
            .expect("startup hostname transport test should terminate the inbound hello line");
        stream
            .flush()
            .expect("startup hostname transport test should flush the inbound hello line");
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("startup hostname transport test should release the server");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("localhost".to_owned()),
        port: Some(address.port()),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    for action in startup_actions {
        assert!(state.apply(action));
    }

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(1),
        "startup hostname transport detached hello",
    );
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"room\":{\"name\":\"room1\"}"));

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_sync_actions = handle.drain_actions();
    for action in hello_sync_actions {
        assert!(state.apply(action));
    }

    assert_eq!(state.main_window.room_name, "room1");
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "alice" && user.is_self),
        "startup hostname transport should project the connected local user",
    );

    release_tx
        .send(())
        .expect("startup hostname transport test should release the server");
    server_thread
        .join()
        .expect("startup hostname transport test server thread should exit cleanly");
}

#[test]
fn gui_persisted_config_runtime_owner_shared_playlist_open_publishes_local_file_over_transport() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    for action in startup_actions {
        assert!(state.apply(action));
    }
    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    for action in hello_actions {
        assert!(state.apply(action));
    }
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let open_actions = handle.drain_actions();
    for action in open_actions.iter().cloned() {
        assert!(state.apply(action));
    }

    assert!(
        open_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 2 selected media entries into the shared playlist."
        )),
        "shared-playlist open should report playlist-backed success",
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("episode1.mkv")
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line
                .contains(r#""playlistChange":{"files":["episode1.mkv","episode2.mkv"]"#)),
        "shared-playlist open should publish the room playlist over the detached transport",
    );
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains(r#""playlistIndex":{"index":0"#)),
        "shared-playlist open should publish the selected playlist index over the detached transport",
    );
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            let Some(file) = message.get("Set").and_then(|set| set.get("file")) else {
                return false;
            };
            file.get("name").and_then(serde_json::Value::as_str) == Some("episode1.mkv")
                && file.get("duration").and_then(serde_json::Value::as_f64) == Some(0.0)
                && file.get("size").and_then(serde_json::Value::as_i64) == Some(0)
        }),
        "shared-playlist open should publish the local file metadata over the detached transport",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_server_hello_before_publishing_local_file_over_transport()
 {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(1234)
            .with_path("C:/Media/episode1.mkv"),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    assert!(
        startup_protocol_lines
            .iter()
            .all(|line| !line.contains(r#""Set":{"file":"#)),
        "local file metadata should stay queued until the server hello completes",
    );
    assert!(owner.last_published_local_file.is_none());

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            let Some(file) = message.get("Set").and_then(|set| set.get("file")) else {
                return false;
            };
            file.get("name").and_then(serde_json::Value::as_str) == Some("episode1.mkv")
                && file.get("duration").and_then(serde_json::Value::as_f64) == Some(42.0)
                && file.get("size").and_then(serde_json::Value::as_i64) == Some(1234)
        }),
        "local file metadata should publish after the server hello completes",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_routes_room_changes_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let _hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut room_line = String::new();
        reader
            .read_line(&mut room_line)
            .expect("test session transport server should read one outbound room-change line");
        let mut list_line = String::new();
        reader
            .read_line(&mut list_line)
            .expect("test session transport server should read one outbound room-list line");
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");
        (room_line, list_line)
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_ready_rx,
        Duration::from_secs(1),
        "test session transport server hello readiness",
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let (room_line, list_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(room_line.contains("\"Set\""));
    assert!(room_line.contains("\"room\""));
    assert!(room_line.contains("\"room2\""));
    assert!(list_line.contains("\"List\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room2",
        "room change over TCP transport",
    );
    assert_eq!(state.main_window.room_name, "room2");
}

#[test]
fn gui_persisted_config_runtime_owner_disconnects_on_non_protocol_tcp_lines_before_server_hello() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (invalid_line_ready_tx, invalid_line_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let _hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(br#"{"status":"connected"}"#)
            .expect("test session transport server should write one non-protocol startup line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the first non-protocol line");
        invalid_line_ready_tx
            .send(())
            .expect("test session transport server should signal invalid-line readiness");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &invalid_line_ready_rx,
        Duration::from_secs(1),
        "test session transport invalid inbound line",
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while owner.session_active() {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the non-protocol TCP startup line to terminate the session",
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        state.notifications.iter().any(|notification| notification
            .message
            .contains("Session transport TCP received an invalid protocol line")),
        "non-protocol TCP startup lines should surface a terminal transport error"
    );
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| !notification.message.contains("Reconnect attempt")),
        "protocol violations should terminate the session instead of scheduling a reconnect"
    );
    assert!(!state.main_window.playback.can_set_ready);
    assert!(!state.commands.can_disconnect_session);
    assert!(owner.session_transport_reconnect_due_at.is_none());

    server_thread
        .join()
        .expect("test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_rejects_room_changes_before_server_hello_without_optimistic_room_updates()
 {
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(state.main_window.room_name, "room1");
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level: GuiTransientNotificationLevel::Error, message }
                if message.contains("server Hello completes")
        )),
        "pre-Hello room requests should surface the runtime error without changing the joined room",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_disconnects_immediately_on_terminal_server_error() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
    );
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playback.can_set_ready,
        "initial queued-transport server hello",
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        owner.pending_room_change_request.is_some(),
        "queued room changes should latch until the runtime confirms the room transition"
    );
    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains(r#""room2""#)),
        "room change requests should still be dispatched before the terminal server error arrives"
    );

    session_transport
        .push_inbound_protocol_line(r#"{"Error":{"message":"wrong-password-server-error"}}"#);
    let error_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        error_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message,
            } if message.contains("wrong-password-server-error")
        )),
        "terminal server errors should surface the server-provided message"
    );
    assert!(
        !owner.session_active(),
        "terminal server errors should tear down the GUI session immediately instead of waiting for transport EOF"
    );
    assert!(
        owner.pending_room_change_request.is_none(),
        "terminal server errors should clear any pending room-change confirmation latch"
    );
    assert_eq!(state.main_window.room_name, "room1");
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| notification.message != "Room joined: room2."),
        "a failed room change must not later surface a false room-joined notification"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_updates_default_room_fallback_after_detached_room_edit() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.session_projects_to_shell = false;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "room9".to_owned(),
    }));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());

    handle.push_request(GuiRuntimeRequest::ReturnToDefaultRoom);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(outbound_protocol_lines.len(), 2);
    assert!(
        outbound_protocol_lines[0].contains(r#""room":{"name":"room9"}"#),
        "return-to-default should target the updated detached room setting"
    );
    assert!(
        outbound_protocol_lines[1].contains(r#""List""#),
        "room fallback should also request a fresh user list"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_emits_periodic_state_heartbeat_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (heartbeat_tx, heartbeat_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .expect("test session transport server should set a read timeout");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        hello_tx
            .send(hello_line)
            .expect("test session transport server should report the startup hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");

        let mut heartbeat_line = String::new();
        reader
            .read_line(&mut heartbeat_line)
            .expect("test session transport server should read one outbound heartbeat line");
        heartbeat_tx
            .send(heartbeat_line)
            .expect("test session transport server should report the heartbeat line");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(2),
        "the startup hello",
    );
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(hello_line.contains("\"room1\""));

    let deadline = Instant::now() + Duration::from_secs(2);
    let heartbeat_line = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(line) = heartbeat_rx.try_recv() {
            break line;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a GUI heartbeat line over TCP transport"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(heartbeat_line.contains("\"State\""));
    assert!(heartbeat_line.contains("\"ping\""));
    assert!(
        heartbeat_line.contains("\"clientLatencyCalculation\""),
        "heartbeat should include client ping metrics"
    );

    server_thread
        .join()
        .expect("test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_returns_to_default_room_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let (release_leave_tx, release_leave_rx) = mpsc::channel();
    let (release_server_tx, release_server_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let _hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "test session transport server",
        );
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");

        let mut join_line = String::new();
        reader
            .read_line(&mut join_line)
            .expect("test session transport server should read one outbound room-change line");
        let mut join_list_line = String::new();
        reader
            .read_line(&mut join_list_line)
            .expect("test session transport server should read one outbound room-list line");
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");

        let mut leave_line = String::new();
        reader
            .read_line(&mut leave_line)
            .expect("test session transport server should read one outbound default-room line");
        let mut leave_list_line = String::new();
        reader.read_line(&mut leave_list_line).expect(
            "test session transport server should read one outbound default-room list line",
        );
        release_leave_rx
            .recv_timeout(Duration::from_secs(1))
            .expect(
                "test session transport server should be released for the default-room response",
            );
        stream
            .write_all(br#"{"Set":{"room":{"name":"room1"}}}"#)
            .expect("test session transport server should write one inbound default-room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound default-room line");
        release_server_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("test session transport server should be releasable after the default-room response");

        (join_line, join_list_line, leave_line, leave_list_line)
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_ready_rx,
        Duration::from_secs(1),
        "test session transport startup hello",
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room2",
        "room join before default-room return",
    );
    assert!(
        state
            .notifications
            .iter()
            .all(|item| item.message != "Room joined: room2.")
    );

    handle.push_request(GuiRuntimeRequest::ReturnToDefaultRoom);
    let leave_request_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.main_window.room_name, "room2");
    assert!(
        leave_request_actions.iter().all(|action| !matches!(
            action,
            GuiShellAction::PushTransientNotification { message, .. }
                if message == "Returned to default room: room1."
        )),
        "the room should not be reported as left before the runtime confirms the fallback room",
    );

    release_leave_tx
        .send(())
        .expect("test session transport server should be releasable for the default-room response");
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room1",
        "default-room return over TCP transport",
    );

    release_server_tx
        .send(())
        .expect("test session transport server should be releasable");

    let (join_line, join_list_line, leave_line, leave_list_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(join_line.contains("\"room2\""));
    assert!(join_list_line.contains("\"List\""));
    assert!(leave_line.contains("\"room1\""));
    assert!(leave_list_line.contains("\"List\""));
    assert_eq!(state.main_window.room_name, "room1");
    assert!(
        state
            .notifications
            .iter()
            .all(|item| item.message != "Returned to default room: room1.")
    );
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("room1")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_reconnects_after_clean_tcp_server_close() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("reconnect test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("reconnect test session transport listener should expose a local address");
    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (first_server_ready_tx, first_server_ready_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let (reconnect_hello_tx, reconnect_hello_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut first_stream, _) = listener
            .accept()
            .expect("reconnect test session transport server should accept the first client");
        let first_reader_stream = first_stream
            .try_clone()
            .expect("reconnect test session transport server should clone the first stream");
        let mut first_reader = BufReader::new(first_reader_stream);
        let first_hello = read_client_hello_after_optional_start_tls(
            &mut first_reader,
            &mut first_stream,
            "reconnect test session transport server",
        );
        first_hello_tx
            .send(first_hello)
            .expect("reconnect test session transport server should report the startup hello");
        first_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("reconnect test session transport server should write the first hello");
        first_stream
            .write_all(b"\n")
            .expect("reconnect test session transport server should terminate the first hello");
        first_server_ready_tx
            .send(())
            .expect("reconnect test session transport server should signal hello readiness");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reconnect test session transport server should be released for EOF");
        drop(first_reader);
        drop(first_stream);

        let (mut second_stream, _) = listener
            .accept()
            .expect("reconnect test session transport server should accept the reconnect");
        let second_reader_stream = second_stream
            .try_clone()
            .expect("reconnect test session transport server should clone the reconnect stream");
        let mut second_reader = BufReader::new(second_reader_stream);
        let reconnect_hello = read_client_hello_after_optional_start_tls(
            &mut second_reader,
            &mut second_stream,
            "reconnect test session transport server",
        );
        reconnect_hello_tx
            .send(reconnect_hello)
            .expect("reconnect test session transport server should report the reconnect hello");
        second_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("reconnect test session transport server should write the reconnect hello");
        second_stream
            .write_all(b"\n")
            .expect("reconnect test session transport server should terminate the reconnect hello");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let first_hello = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "reconnect test session transport startup hello",
    );
    assert!(first_hello.contains("\"Hello\""));
    assert!(first_hello.contains("\"alice\""));
    first_server_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("reconnect test session transport server should signal the first hello");

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.commands.can_send_chat_message,
        "initial TCP server hello",
    );

    release_first_tx
        .send(())
        .expect("reconnect test session transport server should be releasable");

    let deadline = Instant::now() + Duration::from_secs(2);
    let reconnect_hello = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(reconnect_hello) = reconnect_hello_rx.try_recv() {
            break reconnect_hello;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the reconnect hello after a clean server close"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(reconnect_hello.contains("\"Hello\""));
    assert!(reconnect_hello.contains("\"alice\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .notifications
                .iter()
                .any(|notification| notification.message == "Session reconnected.")
        },
        "TCP reconnect completion",
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message == "Reconnect attempt 1 in 0.1 seconds.")
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message == "Session reconnected.")
    );

    server_thread
        .join()
        .expect("reconnect test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_clears_pending_room_change_request_when_reconnecting() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("room-change reconnect test listener should bind");
    let address = listener
        .local_addr()
        .expect("room-change reconnect test listener should expose a local address");
    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (first_server_ready_tx, first_server_ready_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let (reconnect_hello_tx, reconnect_hello_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut first_stream, _) = listener
            .accept()
            .expect("room-change reconnect test server should accept the first client");
        let first_reader_stream = first_stream
            .try_clone()
            .expect("room-change reconnect test server should clone the first stream");
        let mut first_reader = BufReader::new(first_reader_stream);
        let first_hello = read_client_hello_after_optional_start_tls(
            &mut first_reader,
            &mut first_stream,
            "room-change reconnect test server",
        );
        first_hello_tx
            .send(first_hello)
            .expect("room-change reconnect test server should report the startup hello");
        first_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("room-change reconnect test server should write the first hello");
        first_stream
            .write_all(b"\n")
            .expect("room-change reconnect test server should terminate the first hello");
        first_server_ready_tx
            .send(())
            .expect("room-change reconnect test server should signal hello readiness");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("room-change reconnect test server should be released for EOF");
        drop(first_reader);
        drop(first_stream);

        let (mut second_stream, _) = listener
            .accept()
            .expect("room-change reconnect test server should accept the reconnect");
        let second_reader_stream = second_stream
            .try_clone()
            .expect("room-change reconnect test server should clone the reconnect stream");
        let mut second_reader = BufReader::new(second_reader_stream);
        let reconnect_hello = read_client_hello_after_optional_start_tls(
            &mut second_reader,
            &mut second_stream,
            "room-change reconnect test server",
        );
        reconnect_hello_tx
            .send(reconnect_hello)
            .expect("room-change reconnect test server should report the reconnect hello");
        second_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room9"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("room-change reconnect test server should write the reconnect hello");
        second_stream
            .write_all(b"\n")
            .expect("room-change reconnect test server should terminate the reconnect hello");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let first_hello = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "room-change reconnect test startup hello",
    );
    assert!(first_hello.contains("\"Hello\""));
    assert!(first_hello.contains("\"alice\""));
    first_server_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("room-change reconnect test server should signal the first hello");

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.commands.can_send_chat_message,
        "room-change reconnect initial TCP server hello",
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        owner.pending_room_change_request,
        Some(GuiPendingRoomChangeRequest::Join {
            requested_room: "room2".to_owned(),
        }),
        "room changes should stay pending until the runtime confirms the transition",
    );
    owner.suppressed_attached_room_playstate_after_playlist_reset = Some(GuiSessionRoomPlaystate {
        paused: Some(true),
        ..GuiSessionRoomPlaystate::default()
    });
    owner.pending_local_attached_pause_override = Some(false);

    release_first_tx
        .send(())
        .expect("room-change reconnect test server should be releasable");

    let deadline = Instant::now() + Duration::from_secs(2);
    let reconnect_hello = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(reconnect_hello) = reconnect_hello_rx.try_recv() {
            break reconnect_hello;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the reconnect hello after the room-change disconnect"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(reconnect_hello.contains("\"Hello\""));
    assert!(reconnect_hello.contains("\"alice\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room9",
        "room-change reconnect second TCP server hello",
    );

    assert!(
        owner.pending_room_change_request.is_none(),
        "reconnect scheduling should clear stale room-change confirmation state",
    );
    assert!(
        owner
            .suppressed_attached_room_playstate_after_playlist_reset
            .is_none(),
        "reconnect scheduling should clear stale playlist-reset suppression state",
    );
    assert!(
        owner.pending_local_attached_pause_override.is_none(),
        "reconnect scheduling should clear stale local pause override state",
    );
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| !notification.message.starts_with("Room joined:")),
        "a dropped room change must not later surface a false room-joined notification after reconnect",
    );
    assert!(
        state.notifications.iter().all(|notification| !notification
            .message
            .starts_with("Returned to default room:")),
        "a dropped room change must not later surface a false return-to-default notification after reconnect",
    );

    server_thread
        .join()
        .expect("room-change reconnect test server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_reconnects_after_tcp_inbound_idle_timeout() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("idle-timeout test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("idle-timeout test session transport listener should expose a local address");
    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (first_server_ready_tx, first_server_ready_rx) = mpsc::channel();
    let (reconnect_hello_tx, reconnect_hello_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut first_stream, _) = listener
            .accept()
            .expect("idle-timeout test session transport server should accept the first client");
        let first_reader_stream = first_stream
            .try_clone()
            .expect("idle-timeout test session transport server should clone the first stream");
        let mut first_reader = BufReader::new(first_reader_stream);
        let first_hello = read_client_hello_after_optional_start_tls(
            &mut first_reader,
            &mut first_stream,
            "idle-timeout test session transport server",
        );
        first_hello_tx
            .send(first_hello)
            .expect("idle-timeout test session transport server should report the startup hello");
        first_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("idle-timeout test session transport server should write the first hello");
        first_stream
            .write_all(b"\n")
            .expect("idle-timeout test session transport server should terminate the first hello");
        first_server_ready_tx
            .send(())
            .expect("idle-timeout test session transport server should signal hello readiness");

        let (mut second_stream, _) = listener
            .accept()
            .expect("idle-timeout test session transport server should accept the reconnect");
        let second_reader_stream = second_stream
            .try_clone()
            .expect("idle-timeout test session transport server should clone the reconnect stream");
        let mut second_reader = BufReader::new(second_reader_stream);
        let reconnect_hello = read_client_hello_after_optional_start_tls(
            &mut second_reader,
            &mut second_stream,
            "idle-timeout test session transport server",
        );
        reconnect_hello_tx
            .send(reconnect_hello)
            .expect("idle-timeout test session transport server should report the reconnect hello");
        second_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("idle-timeout test session transport server should write the reconnect hello");
        second_stream.write_all(b"\n").expect(
            "idle-timeout test session transport server should terminate the reconnect hello",
        );
    });

    let (owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let driver =
        crate::app::GuiTcpSessionTransportDriver::connect_from_host_arg(&address.to_string())
            .expect("idle-timeout test transport driver should connect")
            .with_inbound_idle_timeout(Duration::from_millis(100));
    let mut owner = owner.with_session_transport_driver(Box::new(driver));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let first_hello = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "idle-timeout test session transport startup hello",
    );
    assert!(first_hello.contains("\"Hello\""));
    assert!(first_hello.contains("\"alice\""));
    first_server_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("idle-timeout test session transport server should signal the first hello");

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.commands.can_send_chat_message,
        "initial TCP server hello before idle timeout",
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let reconnect_hello = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(reconnect_hello) = reconnect_hello_rx.try_recv() {
            break reconnect_hello;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the reconnect hello after the inbound idle timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(reconnect_hello.contains("\"Hello\""));
    assert!(reconnect_hello.contains("\"alice\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .notifications
                .iter()
                .any(|notification| notification.message == "Session reconnected.")
        },
        "TCP reconnect completion after idle timeout",
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message == "Reconnect attempt 1 in 0.1 seconds.")
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message == "Session reconnected.")
    );

    server_thread
        .join()
        .expect("idle-timeout test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_reconnects_client_core_tcp_session_for_public_server_connect()
{
    use std::{io::BufReader, net::TcpListener, sync::mpsc, time::Duration};

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("first test session transport listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("first test session transport listener should expose a local address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("second test session transport listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("second test session transport listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("first test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("first test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "first test session transport server",
        );
        first_hello_tx
            .send(hello_line)
            .expect("first test session transport server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first test session transport server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("second test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("second test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "second test session transport server",
        );
        second_hello_tx
            .send(hello_line)
            .expect("second test session transport server should report its hello");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", first_address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Secondary".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let first_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "first test session transport startup hello",
    );
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"alice\""));

    let mut stale_main_window = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_main_window.shared_playlist_enabled = true;
    stale_main_window.playlist = vec!["episode2.mkv".to_owned()];
    stale_main_window.can_set_ready = true;
    stale_main_window.playback_paused = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_main_window
    )));

    let mut stale_interaction = GuiInteractionRuntimeSnapshot::from_shell_state(&state);
    stale_interaction.selection.selected_main_window_playlist = Some(0);
    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            stale_interaction
        ))
    );

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
    assert!(state.main_window.shared_playlist_enabled);
    assert_eq!(state.main_window.playlist.len(), 1);
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| action.enabled)
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let reconnect_actions = handle.drain_actions();
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "public-server connect should complete through the client-core session runtime"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if !snapshot.shared_playlist_enabled
                    && snapshot.playlist.is_empty()
                    && !snapshot.can_set_ready
                    && !snapshot.playback_paused
        )),
        "public-server reconnect should clear stale session-owned main-window state before the new server replies"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot)
                if snapshot.selection.selected_main_window_playlist.is_none()
        )),
        "public-server reconnect should clear stale playlist selection before the new server replies"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot)
                if snapshot.action_overrides.contains(&MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Playlist",
                    enabled: false,
                })
        )),
        "public-server reconnect should clear stale playlist menu state before the new server replies"
    );
    for action in reconnect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert!(!state.main_window.shared_playlist_enabled);
    assert!(state.main_window.playlist.is_empty());
    assert!(!state.main_window.playback.can_set_ready);
    assert!(!state.main_window.playback_paused);
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| !action.enabled)
    );

    let second_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_hello_rx,
        Duration::from_secs(1),
        "second test session transport reconnect hello",
    );
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"alice\""));
    assert!(second_hello_line.contains("\"room1\""));

    release_first_tx
        .send(())
        .expect("first test session transport server should be releasable");
    first_server_thread
        .join()
        .expect("first test session transport server thread should complete");
    second_server_thread
        .join()
        .expect("second test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_clears_pending_room_change_request_for_public_server_connect()
{
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("pending-room public-server test first listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("pending-room public-server test first listener should expose a local address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("pending-room public-server test second listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("pending-room public-server test second listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("pending-room public-server test first server should accept one client");
        let reader_stream = stream.try_clone().expect(
            "pending-room public-server test first server should clone the accepted stream",
        );
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "pending-room public-server test first server",
        );
        first_hello_tx
            .send(hello_line)
            .expect("pending-room public-server test first server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect(
                "pending-room public-server test first server should be released after reconnect",
            );
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("pending-room public-server test second server should accept one client");
        let reader_stream = stream.try_clone().expect(
            "pending-room public-server test second server should clone the accepted stream",
        );
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "pending-room public-server test second server",
        );
        second_hello_tx
            .send(hello_line)
            .expect("pending-room public-server test second server should report its hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room9"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("pending-room public-server test second server should write the reconnect hello");
        stream.write_all(b"\n").expect(
            "pending-room public-server test second server should terminate the reconnect hello",
        );
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", first_address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Secondary".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let first_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "pending-room public-server test startup hello",
    );
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"alice\""));

    owner.pending_room_change_request = Some(GuiPendingRoomChangeRequest::Join {
        requested_room: "room2".to_owned(),
    });
    owner.suppressed_attached_room_playstate_after_playlist_reset = Some(GuiSessionRoomPlaystate {
        paused: Some(true),
        ..GuiSessionRoomPlaystate::default()
    });
    owner.pending_local_attached_pause_override = Some(false);

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    assert!(
        connect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "public-server connect should complete through the client-core session runtime",
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }

    let second_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_hello_rx,
        Duration::from_secs(1),
        "pending-room public-server test reconnect hello",
    );
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"alice\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room9",
        "pending-room public-server second server hello",
    );

    assert!(
        owner.pending_room_change_request.is_none(),
        "public-server connect should clear stale room-change confirmation state",
    );
    assert!(
        owner
            .suppressed_attached_room_playstate_after_playlist_reset
            .is_none(),
        "public-server connect should clear stale playlist-reset suppression state",
    );
    assert!(
        owner.pending_local_attached_pause_override.is_none(),
        "public-server connect should clear stale local pause override state",
    );
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| !notification.message.starts_with("Room joined:")),
        "public-server connect must not surface a false room-joined notification from the previous session",
    );
    assert!(
        state.notifications.iter().all(|notification| !notification
            .message
            .starts_with("Returned to default room:")),
        "public-server connect must not surface a false return-to-default notification from the previous session",
    );

    release_first_tx
        .send(())
        .expect("pending-room public-server test first server should be releasable");
    first_server_thread
        .join()
        .expect("pending-room public-server test first server thread should complete");
    second_server_thread
        .join()
        .expect("pending-room public-server test second server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_republishes_local_file_after_public_server_connect() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("first test session transport listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("first test session transport listener should expose a local address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("second test session transport listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("second test session transport listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("first test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("first test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "first test session transport server",
        );
        first_hello_tx
            .send(hello_line)
            .expect("first test session transport server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first test session transport server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let (second_file_tx, second_file_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("second test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("second test session transport server should clone the reconnect stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "second test session transport server",
        );
        second_hello_tx
            .send(hello_line)
            .expect("second test session transport server should report its hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("second test session transport server should write the reconnect hello");
        stream
            .write_all(b"\n")
            .expect("second test session transport server should terminate the reconnect hello");
        stream
            .flush()
            .expect("second test session transport server should flush the reconnect hello");
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("second test session transport server should set a read timeout");

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.contains("\"file\"") {
                        second_file_tx
                            .send(line)
                            .expect("second test session transport server should report the republished file");
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    break;
                }
                Err(error) => panic!(
                    "second test session transport server should keep reading protocol lines: {error}"
                ),
            }
        }
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", first_address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_duration_seconds(95.5)
            .with_size_bytes(123456789)
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.last_published_local_file = owner.player_local_file.clone();

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Secondary".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let first_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        Duration::from_secs(1),
        "first test session transport startup hello",
    );
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"alice\""));

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let second_hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_hello_rx,
        Duration::from_secs(1),
        "second test session transport reconnect hello",
    );
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"alice\""));
    assert!(second_hello_line.contains("\"room1\""));

    let deadline = Instant::now() + Duration::from_secs(2);
    let republished_file_line = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(file_line) = second_file_rx.try_recv() {
            break file_line;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the republished local file after reconnecting to the public server"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(republished_file_line.contains("\"file\""));
    assert!(republished_file_line.contains("\"episode1.mkv\""));
    assert!(republished_file_line.contains("\"duration\":95.5"));
    assert!(republished_file_line.contains("\"size\":123456789"));

    release_first_tx
        .send(())
        .expect("first test session transport server should be releasable");
    first_server_thread
        .join()
        .expect("first test session transport server thread should complete");
    second_server_thread
        .join()
        .expect("second test session transport server thread should complete");
}
