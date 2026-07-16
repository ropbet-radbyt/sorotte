use super::*;

#[test]
fn gui_persisted_config_runtime_owner_routes_public_server_refresh_through_client_core_session() {
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
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
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("owner-missing-media-search-cache");
    let config_path = root.join("sorotte.ini");
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
        &crate::app::media_search_cache::PersistedMediaSearchRootIndexV2 {
            version: 2,
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompletePendingOperation)),
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
    assert_eq!(state.active_view, GuiShellView::Room);
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    let outbound_protocol_lines =
        without_default_ready_publish_lines(session_transport.drain_outbound_protocol_lines());
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"controllerAuth\""));
    assert!(outbound_protocol_lines[0].contains(canonical_room));
    assert!(outbound_protocol_lines[0].contains("\"AB-123-456\""));
}

#[test]
fn gui_persisted_config_runtime_owner_startup_saved_connect_preserves_controlled_room_auth() {
    use std::{
        io::{BufReader, Write},
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

        let controller_auth_line =
            read_next_non_default_ready_line(&mut reader, "startup auth test controller auth");
        controller_auth_tx
            .send(controller_auth_line)
            .expect("startup auth test should report controller auth");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            .as_ref()
            .map(|secret| secret.expose_secret()),
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
