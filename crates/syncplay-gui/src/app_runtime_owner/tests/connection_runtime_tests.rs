use super::*;

#[test]
fn gui_persisted_config_runtime_owner_reports_runtime_gaps_explicitly() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode1.mkv".to_owned()],
        load_into_shared_playlist: true,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                    .to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/movie.mkv".to_owned()],
        load_into_shared_playlist: false,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: "Opening media requires a playback runtime connection; the selected file was not opened."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Opening media requires a playback runtime connection; the selected file was not opened."
                    .to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
                    .to_owned(),
            ),
        ]
    );

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
    assert_eq!(cancel_actions, vec![GuiShellAction::CancelPendingOperation]);
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
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::CancelPlaybackPauseToggle,
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: "Playback toggle requires a playback runtime connection; the pause request was not applied."
                    .to_owned(),
            },
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
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
            }),
        ]
    );

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
    assert_eq!(
        chat_actions,
        vec![
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
            }),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message:
                    "Chat sending requires a session runtime connection; the message was not sent."
                        .to_owned(),
            },
        ]
    );
    for action in chat_actions {
        assert!(chat_state.apply(action));
    }
    assert_eq!(chat_state.outgoing_chat_message.as_deref(), Some("hello"));
    assert!(chat_state.pending_operation.is_none());
}

#[test]
fn gui_persisted_config_runtime_owner_bootstraps_detached_public_server_connect() {
    use std::{
        io::{BufRead, BufReader, Write},
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
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("detached public-server connect test should read the GUI hello");
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
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message == "Connected to public server: Primary."),
        "detached public-server connect should report the selected server connection"
    );

    let hello_line = hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("detached public-server connect should emit a GUI hello");
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
    assert_eq!(
        state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Public servers refreshed: 2 entries.")
    );
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
    let actions = handle.drain_actions();
    let found_path_text = found_path.to_string_lossy().into_owned();
    let expected_message = format!("Missing media found: {found_path_text}.");
    assert_eq!(
        actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            found_path_text.clone(),
        ))]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some(expected_message.as_str())
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
    assert_eq!(
        startup_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
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
    assert_eq!(
        hello_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
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

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-owner-missing-media-search-{}-{unique_suffix}",
        std::process::id()
    ));
    let nested = root.join("nested");
    let found_path = nested.join("episode2.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("test missing-media search directory tree should be created");
    std::fs::write(&found_path, b"test").expect("test missing-media search file should be written");
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));

    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
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
}
