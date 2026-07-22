use super::*;

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_transport_lines() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    assert!(startup_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
            room_name,
            room_control_status,
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users,
            playlist,
            chat,
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms,
            ..
        }) if room_name == "room1"
            && room_control_status == "Pending: waiting for server room state."
            && users == &vec![browser_runtime_user("alice", "room1", true, false, false)]
            && playlist.is_empty()
            && runtime_chat_pane_ready(chat)
            && rooms == &browser_runtime_rooms("room1", false, true)
    )));
    assert!(startup_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: true,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: _,
            },
            pending_operation: None,
        })
    )));
    for action in startup_actions {
        assert!(state.apply(action));
    }
    assert!(!state.commands.can_send_chat_message);

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    assert!(hello_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
            room_name,
            room_control_status,
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users,
            playlist,
            chat,
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_set_others_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms,
            ..
        }) if room_name == "room1"
            && room_control_status == "Not required: current room is not controlled."
            && users == &vec![browser_runtime_user("alice", "room1", true, false, false)]
            && playlist.is_empty()
            && runtime_chat_pane_ready(chat)
            && rooms == &browser_runtime_rooms("room1", false, true)
    )));
    assert!(hello_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
            action_overrides,
            tls_prompt_expected,
            update_notice_expected,
            about_dialog_available,
        }) if *tls_prompt_expected == state.menus.tls_prompt_expected
            && *update_notice_expected == state.menus.update_notice_expected
            && *about_dialog_available == state.menus.about_dialog_available
            && action_overrides.iter().any(|override_action|
                override_action.id == MenuActionId::CreateControlledRoom
                    && override_action.enabled)
    )));
    assert!(hello_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: true,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: true,
                chat_unavailable_reason: _,
            },
            pending_operation: None,
        })
    )));
    for action in hello_actions {
        assert!(state.apply(action));
    }

    let outbound_protocol_lines =
        without_default_ready_publish_lines(session_transport.drain_outbound_protocol_lines());
    assert!(outbound_protocol_lines.is_empty());
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
    assert_eq!(state.main_window.chat.len(), 2);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_over_tcp_transport() {
    use std::{
        io::{BufReader, Write},
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
        let chat_line =
            read_next_non_default_ready_line(&mut reader, "test session transport chat line");
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
        .with_client_core_chat_tcp_session_runtime(
            "alice",
            "room1",
            address.to_string(),
            TlsPolicy::PreferTls,
        )
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    let hello_sync_actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.commands.can_send_chat_message,
        "TCP chat capability after the server hello",
    );
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
    assert_eq!(state.main_window.chat.len(), 3);
    assert!(state.main_window.chat.iter().any(|entry| {
        entry
            .message
            .contains("connection is continuing without encryption")
    }));

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
        io::{BufReader, Write},
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
    let (ready_line_tx, ready_line_rx) = mpsc::channel();
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
        let ready_line =
            read_next_non_default_ready_line(&mut reader, "test session transport ready line");
        ready_line_tx
            .send(ready_line)
            .expect("test session transport server should report the outbound ready line");
        stream
            .write_all(br#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("test session transport server should write one inbound ready line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound ready line");
        stream
            .flush()
            .expect("test session transport server should flush the inbound ready line");
        release_server_rx
            .recv_timeout(Duration::from_secs(2))
            .expect(
                "test session transport server should stay connected until readiness is projected",
            );
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "alice",
            "room1",
            address.to_string(),
            TlsPolicy::PreferTls,
        )
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.playback.can_set_ready,
        "local readiness capability after the server hello",
    );

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let ready_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &ready_line_rx,
        Duration::from_secs(1),
        "the outbound readiness line over TCP transport",
    );
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

    release_server_tx
        .send(())
        .expect("test session transport server should be releasable after readiness is projected");

    server_thread
        .join()
        .expect("test session transport server thread should complete");
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
    let (outbound_lines_tx, outbound_lines_rx) = mpsc::channel();
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

        let mut outbound_lines = Vec::new();
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("test session transport server should set a read timeout");
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    break;
                }
                Err(error) => panic!(
                    "test session transport server should read outbound media-open lines: {error}"
                ),
            }
            outbound_lines.push(line);
            if outbound_lines.iter().any(|line| line.contains("\"ready\""))
                && outbound_lines.iter().any(|line| line.contains("\"file\""))
            {
                break;
            }
        }
        stream
            .write_all(br#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#)
            .expect("test session transport server should echo the inbound not-ready state");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound not-ready line");
        stream
            .flush()
            .expect("test session transport server should flush the inbound not-ready line");
        outbound_lines_tx
            .send(outbound_lines)
            .expect("test session transport server should report outbound media-open lines");
        release_server_rx
            .recv_timeout(Duration::from_secs(5))
            .expect(
                "test session transport server should stay connected until the not-ready state is projected",
            );
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "alice",
            "room1",
            address.to_string(),
            TlsPolicy::PreferTls,
        )
        .expect("client-core tcp chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.playback.can_set_ready,
        "open-media transport capability after the server hello",
    );
    let media_root = test_temp_root("tcp-open-media-readiness");
    let media_path = media_root.join("movie.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_lines = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &outbound_lines_rx,
        Duration::from_secs(2),
        "the outbound media-open protocol lines over TCP transport",
    );
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
        Duration::from_secs(3),
        |state| {
            state
                .main_window
                .users
                .iter()
                .any(|user| user.username == "alice" && user.is_self && !user.is_ready)
        },
        "local open-media not-ready projection over TCP transport",
    );

    release_server_tx
        .send(())
        .expect("test session transport server should be releasable after not-ready is projected");

    server_thread
        .join()
        .expect("test session transport server thread should complete");
    let _ = std::fs::remove_dir_all(media_root);
}
