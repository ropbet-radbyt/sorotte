use super::*;

#[test]
fn gui_persisted_config_runtime_owner_reports_runtime_gaps_explicitly() {
    let media_root = test_temp_root("runtime-gap-open-media");
    let episode_path = media_root.join("episode1.mkv");
    let movie_path = media_root.join("movie.mkv");
    std::fs::write(&episode_path, b"episode").expect("episode fixture should be written");
    std::fs::write(&movie_path, b"movie").expect("movie fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![episode_path.to_string_lossy().into_owned()],
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
        paths: vec![movie_path.to_string_lossy().into_owned()],
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            chat_input_enabled: Some(true),
            shared_playlist_enabled: Some(false),
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    toggle_state.main_window.playback.can_toggle_pause = true;
    toggle_state.main_window.playlist =
        vec![MainWindowPlaylistRow::inferred("episode1.mkv", false)];
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
            && runtime_chat_pane_ready(chat)
            && rooms == &browser_runtime_rooms("(no room joined)", false, true)
    )));
    assert!(toggle_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: _,
            },
            pending_operation: None,
        })
    )));

    let mut chat_state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
                chat_unavailable_reason: Some(reason),
            },
            pending_operation: None,
        }) if reason == "Chat input is unavailable because no session runtime is connected."
    )));
    assert!(chat_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message
            == "Chat input is unavailable because no session runtime is connected. The message was not sent."
    )));
    for action in chat_actions {
        assert!(chat_state.apply(action));
    }
    assert_eq!(chat_state.outgoing_chat_message, None);
    assert!(chat_state.pending_operation.is_none());
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_connect_once_does_not_persist_unrelated_draft() {
    use std::net::TcpListener;

    let root = test_temp_root("config-connect-once-does-not-save");
    let config_path = root.join("sorotte.ini");
    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("connect-once test should reserve a local TCP port");
    let address = listener
        .local_addr()
        .expect("connect-once test listener should expose an address");
    drop(listener);

    let stored_settings = StoredClientSettingsMvp {
        host: Some(address.ip().to_string()),
        port: Some(address.port()),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        language: Some("en".to_owned()),
        player_path: Some("C:/Program Files/VideoLAN/VLC/vlc.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &stored_settings)
        .expect("initial connect-once config should be written");
    let config_before_connect =
        std::fs::read(&config_path).expect("initial connect-once config should remain readable");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&stored_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "pt-br".to_owned().into(),
    }));
    assert!(state.has_unsaved_configuration_changes());
    assert!(state.apply(GuiShellAction::BeginConnectOnce));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::ConnectOnce)
    );

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectSavedServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();

    assert!(
        connect_actions.iter().all(|action| !matches!(
            action,
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(_)
        )),
        "Connect once must never project the draft as saved configuration",
    );
    assert_eq!(
        std::fs::read(&config_path).expect("connect-once config should remain readable"),
        config_before_connect,
        "Connect once must not rewrite the persisted INI",
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    assert_eq!(
        state.configuration.to_stored_settings().language.as_deref(),
        Some("pt_BR")
    );
    assert!(state.has_unsaved_configuration_changes());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_saves_configuration_for_save_and_connect() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let root = test_temp_root("config-connect-saves-before-connect");
    let config_path = root.join("sorotte.ini");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some(connect_host.clone()),
        port: Some(connect_port),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("C:/Program Files/VideoLAN/VLC/vlc.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "room2".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginSaveAndConnect));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::SaveAndConnect)
    );

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
    assert_eq!(state.pending_saved_server_connect_intent, None);
    assert_eq!(state.saved_configuration.room.as_deref(), Some("room2"));
    assert_eq!(
        state.saved_configuration.host.as_deref(),
        Some(connect_host.as_str())
    );
    assert_eq!(state.saved_configuration.port, Some(connect_port));

    let persisted_settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("config connect should leave a readable sorotte.ini")
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let root = test_temp_root("detached-missing-media-search");
    let nested = root.join("nested");
    let found_path = nested.join("missing-target.mkv");
    std::fs::create_dir_all(&nested)
        .expect("detached missing-media search test should create a directory tree");
    std::fs::write(&found_path, b"detached-missing-media-target")
        .expect("detached missing-media search test should create the target file");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_clears_detached_missing_media_search_without_target() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();

    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            pending_operation: None,
            ..
        })
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message
            == "Missing-media search through the attached session runtime failed: Detached GUI missing-media search could not determine a target file from the current player or playlist state."
    )));
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
}
