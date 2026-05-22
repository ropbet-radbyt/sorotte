use super::*;
use crate::app::runtime_stack::GuiTcpSessionTransportDriver;

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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    let deadline = Instant::now() + Duration::from_secs(5);
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    owner.handle_session_transport_failure(
        &handle,
        &mut state,
        "Session transport TCP connection closed by the server.".to_owned(),
    );
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    owner.session_transport_reconnect_due_at = Some(Instant::now());

    let deadline = Instant::now() + Duration::from_secs(5);
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
    let driver = GuiTcpSessionTransportDriver::connect_from_host_arg(&address.to_string())
        .expect("idle-timeout test transport driver should connect")
        .with_inbound_idle_timeout(Duration::from_millis(100));
    let mut owner = owner.with_session_transport_driver(Box::new(driver));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
