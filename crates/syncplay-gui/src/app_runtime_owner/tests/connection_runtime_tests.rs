use super::*;

#[test]
fn gui_persisted_config_runtime_owner_reports_runtime_gaps_explicitly() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode1.mkv".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let first_open_actions = handle.drain_actions();
    assert!(first_open_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message == "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
    )));
    assert!(first_open_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::AnnounceSystemChatEvent(message)
            if message == "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
    )));

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/movie.mkv".to_owned()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let second_open_actions = handle.drain_actions();
    assert!(second_open_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message == "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
    )));
    assert!(second_open_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::AnnounceSystemChatEvent(message)
            if message == "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
    )));

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let seek_actions = handle.drain_actions();
    assert!(seek_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message == "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
    )));
    assert!(seek_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::AnnounceSystemChatEvent(message)
            if message == "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
    )));

    let mut cancel_chat_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            chat_input_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    assert!(cancel_chat_state.apply(GuiShellAction::BeginLocalChatSend("cancel me".to_owned(),)));
    handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &cancel_chat_state);
    let cancel_actions = handle.drain_actions();
    assert!(cancel_actions.contains(&GuiShellAction::CancelPendingOperation));
    for action in cancel_actions {
        assert!(cancel_chat_state.apply(action));
    }
    assert!(cancel_chat_state.pending_operation.is_none());
    assert!(cancel_chat_state.outgoing_chat_message.is_none());

    let mut toggle_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    toggle_state.main_window.playback.can_toggle_pause = true;
    toggle_state.commands.can_toggle_pause = true;
    assert!(toggle_state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &toggle_state);
    let toggle_actions = handle.drain_actions();
    assert!(toggle_actions.contains(&GuiShellAction::CancelPlaybackPauseToggle));
    assert!(toggle_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message
            == "Playback toggle requires a playback runtime connection; the pause request was not applied."
    )));
    assert!(toggle_actions.iter().any(|action| matches!(
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
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms,
            ..
        }) if room_name == "(no room joined)"
            && room_control_status == "Unavailable: no active server session."
            && users
                == &vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )]
            && playlist.is_empty()
            && chat.is_empty()
            && rooms == &browser_runtime_rooms("(no room joined)", false, true)
    )));
    assert!(toggle_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: None,
        })
    )));

    let mut chat_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(chat_state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &chat_state);
    let chat_actions = handle.drain_actions();
    assert!(chat_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: true,
            },
            pending_operation: None,
        })
    )));
    assert!(chat_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message
            == "Chat sending requires a session runtime connection; the message was not sent."
    )));
    for action in chat_actions {
        assert!(chat_state.apply(action));
    }
    assert_eq!(chat_state.outgoing_chat_message.as_deref(), Some("hello"));
    assert!(chat_state.pending_operation.is_none());
}

#[test]
fn gui_persisted_config_runtime_owner_saves_configuration_before_config_connect() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let root = test_temp_root("config-connect-saves-before-connect");
    let config_path = root.join("syncplay.ini");
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            host: Some("old.example".to_owned()),
            port: Some(8999),
            username: Some("alice".to_owned()),
            room: Some("old-room".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("initial syncplay config should be written");

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("config connect test should bind a TCP listener");
    let address = listener
        .local_addr()
        .expect("config connect test listener should expose an address");
    let connect_host = address.ip().to_string();
    let connect_port = address.port();
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("config connect test should accept a GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("config connect test stream should clone"),
        );
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "config connect test",
        );
        hello_tx
            .send(hello_line)
            .expect("config connect test should report the hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room2"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("config connect test should write the server hello");
        stream
            .write_all(b"\r\n")
            .expect("config connect test should terminate the server hello");
        stream
            .flush()
            .expect("config connect test should flush the server hello");
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("config connect test should release the server");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some(connect_host.clone()),
        port: Some(connect_port),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "room2".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::BeginSavedServerConnect));
    assert!(state.pending_saved_server_connect_saves_configuration);

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectSavedServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();

    assert!(
        connect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(snapshot)
                if snapshot.settings.room.as_deref() == Some("room2")
                    && snapshot.settings.host.as_deref() == Some(connect_host.as_str())
                    && snapshot.settings.port == Some(connect_port)
        )),
        "config-view connect should project the saved configuration before connect completion",
    );
    assert!(
        connect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSavedServerConnect)),
        "config-view connect should still finish through the normal saved-server connect action",
    );

    for action in &connect_actions {
        assert!(state.apply(action.clone()));
    }
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert!(state.pending_operation.is_none());
    assert!(!state.pending_saved_server_connect_saves_configuration);
    assert_eq!(state.saved_configuration.room.as_deref(), Some("room2"));
    assert_eq!(
        state.saved_configuration.host.as_deref(),
        Some(connect_host.as_str())
    );
    assert_eq!(state.saved_configuration.port, Some(connect_port));

    let persisted_settings = load_syncplay_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("config connect should leave a readable syncplay.ini")
        .expect("config connect should persist settings");
    assert_eq!(persisted_settings.room.as_deref(), Some("room2"));
    assert_eq!(
        persisted_settings.host.as_deref(),
        Some(connect_host.as_str())
    );
    assert_eq!(persisted_settings.port, Some(connect_port));

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(1),
        "config connect test GUI hello",
    );
    assert!(
        hello_line.contains("\"room\":{\"name\":\"room2\"}"),
        "config connect should send the updated room in the detached hello: {hello_line}",
    );

    release_tx
        .send(())
        .expect("config connect test should release the server");
    server_thread
        .join()
        .expect("config connect test server thread should exit cleanly");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_bootstraps_detached_public_server_connect() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("detached public-server connect test should bind a TCP listener");
    let address = listener
        .local_addr()
        .expect("detached public-server connect test listener should expose an address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("detached public-server connect test should accept a GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("detached public-server connect test stream should clone"),
        );
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "detached public-server connect test",
        );
        hello_tx
            .send(hello_line)
            .expect("detached public-server connect test should report the hello");
        stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("detached public-server connect test should write the server hello");
        stream
            .write_all(b"\r\n")
            .expect("detached public-server connect test should terminate the server hello");
        stream
            .flush()
            .expect("detached public-server connect test should flush the server hello");
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("detached public-server connect test should release the server");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    let projected_hello_in_connect_actions = connect_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "room1"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "alice" && user.is_self)
        )
    });
    assert!(
        connect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "detached public-server connect should complete through a bootstrapped client-core session runtime"
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(1),
        "detached public-server connect GUI hello",
    );
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(hello_line.contains("\"room1\""));

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    let projected_hello_in_followup_actions = hello_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "room1"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "alice" && user.is_self)
        )
    });
    assert!(
        projected_hello_in_connect_actions || projected_hello_in_followup_actions,
        "detached public-server connect should leave an attached session runtime that projects server hello state"
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    release_tx
        .send(())
        .expect("detached public-server connect test should release the server");
    server_thread
        .join()
        .expect("detached public-server connect server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_refreshes_public_servers_without_session() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![]),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompletePublicServerRefresh(servers)
                if servers
                    == &vec![
                        ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
                        ("Backup".to_owned(), "backup.example:9000".to_owned()),
                    ]
        )),
        "detached public-server refresh should normalize and complete without a preexisting session runtime"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(state.public_servers.servers.len(), 2);
    assert_eq!(state.selected_public_server_index(), Some(0));
}

#[test]
fn gui_persisted_config_runtime_owner_searches_missing_media_without_session() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-detached-missing-media-search-{}-{unique_suffix}",
        std::process::id()
    ));
    let nested = root.join("nested");
    let found_path = nested.join("missing-target.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("detached missing-media search test should create a directory tree");
    std::fs::write(&found_path, b"detached-missing-media-target")
        .expect("detached missing-media search test should create the target file");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "missing-target.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let mut actions = handle.drain_actions();
    let found_path_text = found_path.to_string_lossy().into_owned();
    let expected_message = format!("Missing media found: {found_path_text}.");
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(snapshot)
                if snapshot.active
                    && snapshot
                        .message
                        .as_deref()
                        .is_some_and(|message| message.starts_with("Indexing media 1/1: "))
        )),
        "detached missing-media search should surface the background media-index refresh status"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_))),
        "detached missing-media search should remain pending until the background index completes"
    );
    for action in actions.drain(..) {
        assert!(state.apply(action));
    }
    assert!(state.media_index_status.active);
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SearchMissingMedia)
    );

    let search_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut completion_actions = Vec::new();
    while completion_actions.is_empty() {
        assert!(
            std::time::Instant::now() < search_deadline,
            "timed out waiting for detached missing-media search completion"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        actions = handle.drain_actions();
        completion_actions = actions
            .iter()
            .filter(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_)))
            .cloned()
            .collect();
        for action in actions {
            assert!(state.apply(action));
        }
    }

    assert_eq!(
        completion_actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            found_path_text.clone(),
        ))]
    );
    assert!(state.pending_operation.is_none());
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| notification.message != expected_message),
        "detached missing-media completion should not emit a success notification"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_public_server_refresh_through_client_core_session() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![]),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompletePublicServerRefresh(servers)
                if servers
                    == &vec![
                        ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
                        ("Backup".to_owned(), "backup.example:9000".to_owned()),
                    ]
        )),
        "queued owner should route public-server refresh through the client-core session runtime"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_chat_disabled_until_server_hello_reports_support() {
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
            && chat.is_empty()
            && rooms == &browser_runtime_rooms("room1", false, true)
    )));
    assert!(startup_actions.iter().any(|action| matches!(
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
                override_action.section_title == "Window"
                    && override_action.action_label == "Show Chat"
                    && !override_action.enabled)
    )));
    assert!(startup_actions.iter().any(|action| matches!(
        action,
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
        })
    )));
    for action in startup_actions {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);
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
            && chat.is_empty()
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
                override_action.section_title == "Window"
                    && override_action.action_label == "Show Chat"
                    && override_action.enabled)
            && action_overrides.iter().any(|override_action|
                override_action.section_title == "Advanced"
                    && override_action.action_label == "Create Controlled Room"
                    && override_action.enabled)
    )));
    assert!(hello_actions.iter().any(|action| matches!(
        action,
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
        })
    )));
    for action in hello_actions {
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
            .is_some_and(|action| action.enabled)
    );
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_missing_media_search_through_client_core_session() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("owner-missing-media-search-cache");
    let config_path = root.join("syncplay.ini");
    let nested = root.join("nested");
    let found_path = nested.join("episode2.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("test missing-media search directory tree should be created");
    std::fs::write(&found_path, b"test").expect("test missing-media search file should be written");
    let built_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64;
    crate::app::media_search_cache::persist_media_search_root_index_at_root(
        &root,
        &crate::app::media_search_cache::PersistedMediaSearchRootIndexV1 {
            version: 1,
            root_key: crate::app::media_search_cache::normalized_media_search_root_key(&root),
            root_path: root.to_string_lossy().into_owned(),
            built_at_unix_ms,
            candidates_by_name: std::collections::HashMap::from([(
                "episode2.mkv".to_owned(),
                vec!["nested\\episode2.mkv".to_owned()],
            )]),
        },
    )
    .expect("persisted missing-media cache fixture should be written");
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));

    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#
            .to_owned(),
        r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#.to_owned(),
    ]);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    let found_path_text = found_path.to_string_lossy().into_owned();
    let expected_message =
        format!("Opened media file through the attached recording player: {found_path_text}.");
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                pending_operation: None,
                ..
            })
        )),
        "queued owner should clear the pending search before continuing the session"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message
                        == &format!(
                            "Opened media file through the attached recording player: {found_path_text}."
                        )
        )),
        "queued owner should continue the session with the located file through the attached player"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_))),
        "queued owner should continue through the player path instead of stopping at a found-path completion"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some(expected_message.as_str())
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![found_path_text]
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| {
                index.root_indexes_by_key.contains_key(
                    &crate::app::media_search_cache::normalized_media_search_root_key(&root),
                )
            }),
        "explicit missing-media search should resolve through the preloaded persisted root index before any later root warming occurs"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_normalizes_controlled_room_input_and_remembers_password() {
    let room_input = "+room1:CB39A19549E8:ab-123-456";
    let canonical_room = "+room1:CB39A19549E8";
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", room_input)
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room_input.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    assert!(startup_protocol_lines[0].contains(canonical_room));
    assert!(
        !startup_protocol_lines[0].contains("AB-123-456"),
        "startup hello should not leak the controlled-room password"
    );

    session_transport.push_inbound_protocol_line(format!(
        r#"{{"Hello":{{"username":"alice","room":{{"name":"{canonical_room}"}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"controllerAuth\""));
    assert!(outbound_protocol_lines[0].contains(canonical_room));
    assert!(outbound_protocol_lines[0].contains("\"AB-123-456\""));
}

#[test]
fn gui_persisted_config_runtime_owner_startup_saved_connect_preserves_controlled_room_auth() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let room_input = "+Test:77F8DA30FB3E:RH-273-303";
    let canonical_room = "+Test:77F8DA30FB3E";
    let listener =
        TcpListener::bind("127.0.0.1:0").expect("startup auth test should bind a TCP listener");
    let address = listener
        .local_addr()
        .expect("startup auth test listener should expose an address");
    let connect_host = address.ip().to_string();
    let connect_port = address.port();
    let (hello_tx, hello_rx) = mpsc::channel();
    let (controller_auth_tx, controller_auth_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("startup auth test should accept a GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("startup auth test stream should clone"),
        );
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "startup auth test",
        );
        stream
            .write_all(
                format!(
                    r#"{{"Hello":{{"username":"alice","room":{{"name":"{canonical_room}"}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
                )
                .as_bytes(),
            )
            .expect("startup auth test should write the server hello");
        stream
            .write_all(b"\r\n")
            .expect("startup auth test should terminate the server hello");
        stream
            .flush()
            .expect("startup auth test should flush the server hello");
        hello_tx
            .send(hello_line)
            .expect("startup auth test should report the hello after the server hello is flushed");

        let mut controller_auth_line = String::new();
        reader
            .read_line(&mut controller_auth_line)
            .expect("startup auth test should read controller auth");
        controller_auth_tx
            .send(controller_auth_line)
            .expect("startup auth test should report controller auth");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some(connect_host.clone()),
        port: Some(connect_port),
        username: Some("alice".to_owned()),
        room: Some(room_input.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some(room_input)
    );
    assert_eq!(state.main_window.room_name, canonical_room);
    assert_eq!(
        state
            .saved_session_connect_target()
            .and_then(|target| target.controlled_room_password_override)
            .as_deref(),
        Some("RH-273-303")
    );

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(5),
        |state| state.commands.can_disconnect_session,
        "startup saved-server connect",
    );

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(5),
        "startup auth test GUI hello",
    );
    assert!(hello_line.contains(canonical_room));
    assert!(
        !hello_line.contains("RH-273-303"),
        "startup hello should not leak the controlled-room password"
    );

    let controller_auth_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &controller_auth_rx,
        Duration::from_secs(5),
        "startup auth test controller auth",
    );
    assert!(controller_auth_line.contains("\"controllerAuth\""));
    assert!(controller_auth_line.contains(canonical_room));
    assert!(controller_auth_line.contains("\"RH-273-303\""));

    server_thread
        .join()
        .expect("startup auth test server thread should exit cleanly");
}

#[test]
fn gui_persisted_config_runtime_owner_normalizes_bare_controlled_room_input_on_startup() {
    let room_input = "Test:77F8DA30FB3E";
    let canonical_room = "+Test:77F8DA30FB3E";
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", room_input)
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room_input.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    assert!(startup_protocol_lines[0].contains(canonical_room));
}

#[test]
fn gui_persisted_config_runtime_owner_loopback_transport_echoes_client_core_chat() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);

    let actions = handle.drain_actions();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "loopback transport should preserve the local send completion"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "loopback transport should feed the encoded chat line back through inbound handling"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("alice".to_owned(), "hello room".to_owned()))
    );
    assert_eq!(state.main_window.chat.len(), 1);
}

#[test]
fn gui_persisted_config_runtime_owner_manual_disconnect_applies_pause_on_leave() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.commands.can_disconnect_session);
    assert!(state.apply(GuiShellAction::BeginSessionDisconnect));

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::DisconnectSession,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&true),
        "explicit disconnect should still pause the attached player"
    );
    assert!(owner.session.is_none());
    assert!(state.pending_operation.is_none());
    assert!(
        state
            .notifications
            .iter()
            .all(|notification| notification.message != "Session disconnected."),
        "disconnect completion should no longer emit a success notification"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_discards_attached_player_chat_without_a_sendable_session() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        pending_chat_requests: std::collections::VecDeque<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_pending_chat_request(&mut self) -> Option<String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pending_chat_requests
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState {
        pending_chat_requests: std::collections::VecDeque::from(["hello from mpv".to_owned()]),
    }));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.notifications.iter().any(|notification| {
            notification.message
                == "Chat input from the attached player requires an active session with chat support."
        }),
        "player chat typed without an active session should be rejected immediately"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_chat_requests
            .is_empty(),
        "unsendable player chat should be drained instead of leaking into a later session"
    );

    let (next_owner, session_transport) = owner
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap after player-chat rejection");
    let mut owner = next_owner;

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(
        startup_protocol_lines
            .iter()
            .all(|line| !line.contains("\"Chat\"")),
        "only the startup hello should be queued after late session bootstrap"
    );

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        session_transport.drain_outbound_protocol_lines().is_empty(),
        "rejected player chat must not be sent after the later session handshake"
    );
}
