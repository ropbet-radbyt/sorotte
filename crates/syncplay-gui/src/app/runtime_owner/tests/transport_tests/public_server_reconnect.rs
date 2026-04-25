use super::*;

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
