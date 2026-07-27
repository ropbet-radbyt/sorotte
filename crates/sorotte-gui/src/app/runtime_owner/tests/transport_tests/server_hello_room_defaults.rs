use super::*;

#[test]
fn gui_persisted_config_runtime_owner_applies_hello_bundled_after_prefer_tls_refusal() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("bundled refusal test server should bind");
    let address = listener
        .local_addr()
        .expect("bundled refusal test listener should expose its address");
    let (stop_tx, stop_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept");
        let reader_stream = stream.try_clone().expect("accepted socket should clone");
        let mut reader = BufReader::new(reader_stream);
        let mut tls_request = String::new();
        reader
            .read_line(&mut tls_request)
            .expect("server should read STARTTLS request");
        assert!(tls_request.contains(r#""startTLS":"send""#));
        stream
            .write_all(
                br#"{"TLS":{"startTLS":"false"},"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("server should write bundled refusal and Hello");
        stream
            .write_all(b"\n")
            .expect("server should terminate bundled response");
        stream
            .flush()
            .expect("server should flush bundled response");
        let _ = stop_rx.recv_timeout(Duration::from_secs(2));
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "alice",
            "room1",
            address.to_string(),
            TlsPolicy::PreferTls,
        )
        .expect("PreferTls runtime owner should connect");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let deadline = Instant::now() + Duration::from_secs(1);
    while !owner
        .session
        .as_ref()
        .is_some_and(|session| session.server_handshake_completed())
    {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert!(
            Instant::now() < deadline,
            "bundled Hello should complete the application handshake"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let _ = stop_tx.send(());
    server_thread.join().expect("server thread should join");
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| !notification.message.contains("Unexpected TLS")),
        "the consumed STARTTLS refusal must not enter normal session decoding"
    );
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_ignores_unsaved_default_room_edit() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.session_projects_to_shell = false;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    assert!(
        without_default_ready_publish_lines(session_transport.drain_outbound_protocol_lines())
            .is_empty()
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "room9".to_owned().into(),
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
        outbound_protocol_lines[0].contains(r#""room":{"name":"room1"}"#),
        "return-to-default must use the saved room and ignore the unsaved detached draft"
    );
    assert!(
        outbound_protocol_lines[1].contains(r#""List""#),
        "room fallback should also request a fresh user list"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_emits_periodic_state_heartbeat_over_tcp_transport() {
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

        let heartbeat_line =
            read_next_non_default_ready_line(&mut reader, "test session transport heartbeat line");
        heartbeat_tx
            .send(heartbeat_line)
            .expect("test session transport server should report the heartbeat line");
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

    let deadline = Instant::now() + Duration::from_secs(5);
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

        let join_line = read_next_non_default_ready_line(
            &mut reader,
            "test session transport room-change line",
        );
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
        "default-room transport capability after the server hello",
    );

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
