use super::*;

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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    assert!(
        without_default_ready_publish_lines(session_transport.drain_outbound_protocol_lines())
            .is_empty()
    );

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
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(1234)
            .with_path("C:/Media/episode1.mkv"),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_does_not_publish_placeholder_local_file_over_transport() {
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
    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        without_default_ready_publish_lines(session_transport.drain_outbound_protocol_lines())
            .is_empty()
    );

    owner.player_local_file = Some(
        GuiPersistedConfigRuntimeOwner::placeholder_local_file_for_path("C:/Media/episode1.mkv"),
    );
    owner.player_local_file_placeholder = true;
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let placeholder_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        placeholder_protocol_lines.iter().all(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|message| message.get("Set").and_then(|set| set.get("file")).cloned())
                .is_none()
        }),
        "placeholder local file metadata should not be published before real player metadata arrives",
    );
    assert!(owner.last_published_local_file.is_none());

    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(1234)
            .with_path("C:/Media/episode1.mkv"),
    );
    owner.player_local_file_placeholder = false;
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
        "real local file metadata should publish after the placeholder is replaced",
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
    let (room_lines_tx, room_lines_rx) = mpsc::channel();
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
        let room_line = read_next_non_default_ready_line(
            &mut reader,
            "test session transport room-change line",
        );
        let mut list_line = String::new();
        reader
            .read_line(&mut list_line)
            .expect("test session transport server should read one outbound room-list line");
        room_lines_tx
            .send((room_line, list_line))
            .expect("test session transport server should report outbound room-change lines");
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");
        stream
            .flush()
            .expect("test session transport server should flush the inbound room line");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
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
        "test session transport server hello readiness",
    );

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.playback.can_set_ready,
        "room-change transport capability after the server hello",
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let (room_line, list_line) = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &room_lines_rx,
        Duration::from_secs(1),
        "the outbound room-change protocol lines over TCP transport",
    );
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

    server_thread
        .join()
        .expect("test session transport server thread should complete");
}
