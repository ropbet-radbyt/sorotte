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
    cancel_chat_state.outgoing_chat_message = Some("cancel me".to_owned());
    assert!(
        cancel_chat_state.apply(GuiShellAction::BeginPendingOperation(
            GuiPendingOperationKind::SendChatMessage
        ))
    );
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
    toggle_state.main_window.playlist = vec![MainWindowPlaylistRow {
        label: "episode1.mkv".to_owned(),
        is_selected: false,
    }];
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
    assert_eq!(chat_state.outgoing_chat_message, None);
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
        player_path: Some("C:/Program Files/VideoLAN/VLC/vlc.exe".to_owned()),
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
    assert_eq!(state.active_view, GuiShellView::Room);
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
