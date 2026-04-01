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
    assert!(
        handle.drain_actions().iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "queued owner should turn inbound protocol chat into a GUI chat message action"
    );
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());
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
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

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

    let (hello_line, chat_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(chat_line.contains("\"Chat\""));
    assert!(chat_line.contains("hello room"));

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let third_actions = handle.drain_actions();
    for action in third_actions.iter().cloned() {
        assert!(state.apply(action));
    }
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/movie.mkv".to_owned()],
        load_into_shared_playlist: false,
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
        io::{BufRead, BufReader, Write},
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("startup hostname transport test should read one startup hello line");
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

    let hello_line = hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("startup hostname transport test should observe the detached hello");
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");
        room_line
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let room_line = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(room_line.contains("\"Set\""));
    assert!(room_line.contains("\"room\""));
    assert!(room_line.contains("\"room2\""));

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
fn gui_persisted_config_runtime_owner_ignores_non_protocol_tcp_lines_before_server_hello() {
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        stream
            .write_all(br#"{"status":"connected"}"#)
            .expect("test session transport server should write one non-protocol startup line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the first non-protocol line");
        stream.write_all(br#"{"phase":"startup"}"#).expect(
            "test session transport server should write a second non-protocol startup line",
        );
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the second non-protocol line");
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.playback.can_set_ready,
        "room bootstrap after non-protocol TCP startup lines",
    );

    assert!(state.main_window.playback.can_set_ready);
    assert!(
        !state.notifications.iter().any(|notification| notification
            .message
            .contains("Inbound session transport message apply failed")),
        "non-protocol TCP startup lines should be dropped before session apply"
    );

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
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(
        outbound_protocol_lines[0].contains(r#""room":{"name":"room9"}"#),
        "return-to-default should target the updated detached room setting"
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let hello_line = hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should receive the startup hello");
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
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
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

        (join_line, leave_line)
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
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");
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
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Room joined: room2.")
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

    let (join_line, leave_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(join_line.contains("\"room2\""));
    assert!(leave_line.contains("\"room1\""));
    assert_eq!(state.main_window.room_name, "room1");
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Returned to default room: room1.")
    );
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("room1")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_reconnects_client_core_tcp_session_for_public_server_connect()
{
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
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
        let (stream, _) = first_listener
            .accept()
            .expect("first test session transport server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("first test session transport server should read one startup hello line");
        first_hello_tx
            .send(hello_line)
            .expect("first test session transport server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first test session transport server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (stream, _) = second_listener
            .accept()
            .expect("second test session transport server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("second test session transport server should read one reconnect hello line");
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

    let first_hello_line = first_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first test session transport server should receive the startup hello");
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

    let second_hello_line = second_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("second test session transport server should receive the reconnect hello");
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
