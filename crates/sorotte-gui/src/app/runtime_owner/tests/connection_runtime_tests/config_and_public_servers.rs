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
    assert!(chat_actions.contains(&GuiShellAction::CompleteLocalChatSend));
    for action in chat_actions {
        assert!(chat_state.apply(action));
    }
    assert_eq!(chat_state.outgoing_chat_message, None);
    assert!(chat_state.pending_operation.is_none());
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_connect_once_does_not_persist_unrelated_draft() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let root = test_temp_root("config-connect-once-does-not-save");
    let config_path = root.join("sorotte.ini");
    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("connect-once test should bind a local TCP listener");
    let address = listener
        .local_addr()
        .expect("connect-once test listener should expose an address");
    let connect_host = address.ip().to_string();
    let connect_port = address.port();
    let room_input = "+room1:CB39A19549E8:AB-123-456";
    let canonical_room = "+room1:CB39A19549E8";
    let (hello_tx, hello_rx) = mpsc::channel();
    let (controller_auth_tx, controller_auth_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("connect-once test should accept a GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("connect-once test stream should clone"),
        );
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "connect-once test",
        );
        hello_tx
            .send(hello_line)
            .expect("connect-once test should report the Hello");
        stream
            .write_all(
                format!(
                    r#"{{"Hello":{{"username":"draft-alice","room":{{"name":"{canonical_room}"}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
                )
                .as_bytes(),
            )
            .expect("connect-once test should write the server Hello");
        stream
            .write_all(b"\r\n")
            .expect("connect-once test should terminate the server Hello");
        stream
            .flush()
            .expect("connect-once test should flush the server Hello");
        controller_auth_tx
            .send(read_next_non_default_ready_line(
                &mut reader,
                "connect-once test controller auth",
            ))
            .expect("connect-once test should report controller auth");
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("connect-once test should release the server");
    });

    let stored_settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        port: Some(8999),
        username: Some("saved-alice".to_owned()),
        room: Some("saved-room".to_owned()),
        server_password: Some("saved-secret".into()),
        language: Some("en".to_owned()),
        shared_playlist_enabled: Some(false),
        chat_input_enabled: Some(false),
        rewind_on_desync: Some(false),
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
    for (id, value) in [
        (SettingId::ConnectionHost, connect_host.clone()),
        (SettingId::ConnectionPort, connect_port.to_string()),
        (SettingId::ConnectionUsername, "draft-alice".to_owned()),
        (SettingId::ConnectionRoom, room_input.to_owned()),
    ] {
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id,
            value: value.into(),
        }));
    }
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "pt-br".to_owned().into(),
    }));
    for id in [
        SettingId::PlaybackSharedPlaylists,
        SettingId::ChatInputEnabled,
        SettingId::SyncRewindOnDesync,
    ] {
        assert!(state.apply(GuiShellAction::EditConfigurationBool { id, value: true }));
    }
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "draft-secret".to_owned().into(),
    }));
    assert!(state.has_unsaved_configuration_changes());
    assert!(state.pending_apply_requirements.is_empty());
    assert!(state.apply(GuiShellAction::BeginConnectOnce));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::ConnectOnce)
    );

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::from_state(&state)
            .expect("staged Connect Once should capture submitted settings"),
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
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::Reconnect],
        "Connect Once must not claim that saved reconnect-required settings were applied",
    );
    assert_eq!(state.saved_configuration, stored_settings);
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    let draft_settings = state.configuration.to_stored_settings();
    assert_eq!(draft_settings.host.as_deref(), Some(connect_host.as_str()));
    assert_eq!(draft_settings.port, Some(connect_port));
    assert_eq!(draft_settings.username.as_deref(), Some("draft-alice"));
    assert_eq!(draft_settings.room.as_deref(), Some(room_input));
    assert_eq!(draft_settings.language.as_deref(), Some("pt_BR"));
    assert_eq!(draft_settings.shared_playlist_enabled, Some(true));
    assert_eq!(draft_settings.chat_input_enabled, Some(true));
    assert_eq!(draft_settings.rewind_on_desync, Some(true));
    assert_eq!(
        draft_settings
            .server_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("draft-secret")
    );
    assert!(matches!(
        &state.configuration.server_password,
        crate::app::SecretDraft::Replace(_)
    ));
    assert!(state.has_unsaved_configuration_changes());

    let active_settings = owner
        .active_session_settings
        .as_ref()
        .expect("successful Connect Once should retain effective active-session settings");
    assert_eq!(
        active_settings.config.connection.host.as_deref(),
        Some(connect_host.as_str())
    );
    assert_eq!(active_settings.config.connection.port.get(), connect_port);
    assert_eq!(
        active_settings
            .config
            .connection
            .username
            .as_ref()
            .map(|username| username.as_str()),
        Some("draft-alice")
    );
    assert_eq!(
        active_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str()),
        Some(canonical_room)
    );
    assert_eq!(
        active_settings
            .config
            .connection
            .server_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("draft-secret")
    );
    assert_eq!(
        active_settings
            .config
            .connection
            .controlled_room_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert_eq!(active_settings.settings.language.as_deref(), Some("en"));
    assert_eq!(
        active_settings.settings.shared_playlist_enabled,
        Some(false)
    );
    assert_eq!(active_settings.settings.chat_input_enabled, Some(false));
    assert_eq!(active_settings.settings.rewind_on_desync, Some(false));

    let hello_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &hello_rx,
        Duration::from_secs(5),
        "connect-once test GUI Hello",
    );
    assert!(hello_line.contains("\"username\":\"draft-alice\""));
    assert!(hello_line.contains(canonical_room));
    assert!(
        hello_line.contains(&sorotte_client_core::legacy_server_password_token(
            "draft-secret"
        ))
    );
    assert!(hello_line.contains("\"sharedPlaylists\":false"));
    assert!(
        !hello_line.contains("AB-123-456"),
        "the controlled-room password must not leak into Hello"
    );

    let controller_auth_line = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &controller_auth_rx,
        Duration::from_secs(5),
        "connect-once test controller auth",
    );
    assert!(controller_auth_line.contains("\"controllerAuth\""));
    assert!(controller_auth_line.contains(canonical_room));
    assert!(controller_auth_line.contains("\"AB-123-456\""));
    assert!(
        !state.commands.can_send_chat_message,
        "the unsaved chat-input draft must not enable chat in the active session"
    );
    assert_eq!(
        std::fs::read(&config_path).expect("connect-once config should remain readable"),
        config_before_connect,
        "active-session pumps must not rewrite the persisted INI",
    );

    let mut matching_active_settings = owner
        .active_session_settings
        .as_ref()
        .expect("Connect Once should still own its active-session settings")
        .settings
        .clone();
    matching_active_settings.room = Some(room_input.to_owned());
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(matching_active_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
    assert_eq!(state.saved_configuration, matching_active_settings);

    release_tx
        .send(())
        .expect("connect-once test should release the server");
    server_thread
        .join()
        .expect("connect-once test server thread should exit cleanly");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn threaded_connect_requests_use_submitted_settings_before_the_latest_input_arrives() {
    use std::{
        io::BufReader,
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    for (label, intent) in [
        ("connect-once", GuiSavedServerConnectIntent::ConnectOnce),
        (
            "save-and-connect",
            GuiSavedServerConnectIntent::SaveAndConnect,
        ),
    ] {
        let root = test_temp_root(&format!("threaded-submitted-{label}"));
        let config_path = root.join("sorotte.ini");
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("threaded submitted-settings test should bind its live server");
        let live_address = listener
            .local_addr()
            .expect("threaded submitted-settings listener should expose its address");
        let dead_listener = TcpListener::bind("127.0.0.1:0")
            .expect("threaded submitted-settings test should reserve a stale port");
        let dead_address = dead_listener
            .local_addr()
            .expect("stale listener should expose its address");
        drop(dead_listener);

        let original = StoredClientSettingsMvp {
            host: Some(dead_address.ip().to_string()),
            port: Some(dead_address.port()),
            username: Some("saved-user".to_owned()),
            room: Some("saved-room".to_owned()),
            shared_playlist_enabled: Some(false),
            player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
            ..StoredClientSettingsMvp::default()
        };
        upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &original)
            .expect("threaded submitted-settings fixture should persist initial settings");

        let (hello_tx, hello_rx) = mpsc::channel();
        let server_thread = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("submitted endpoint should receive the GUI connection");
            let mut reader = BufReader::new(
                stream
                    .try_clone()
                    .expect("submitted endpoint stream should clone"),
            );
            let hello = read_client_hello_after_optional_start_tls(
                &mut reader,
                &mut stream,
                "threaded submitted-settings connect",
            );
            hello_tx
                .send(hello)
                .expect("submitted endpoint should report the client Hello");
        });

        let owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
        let (mut runtime, handle) = GuiQueuedRuntimeBridge::new();
        let mut pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
            handle.clone(),
            owner,
            Duration::from_millis(5),
        )
        .expect("threaded submitted-settings runtime should spawn");
        let mut state = SorotteGuiShellAppState::from_stored_settings(&original);

        GuiNativeRuntimePump::pump(&mut pump, &state);
        let stale_deadline = Instant::now() + Duration::from_secs(2);
        let mut stale_projection_attempted = false;
        while !stale_projection_attempted {
            assert!(
                Instant::now() < stale_deadline,
                "threaded runtime did not consume the stale initial input"
            );
            stale_projection_attempted = handle.drain_actions().iter().any(|action| {
                matches!(
                    action,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message,
                    } if message.contains("Configured server connect through the detached session runtime failed")
                )
            });
            if !stale_projection_attempted {
                thread::sleep(Duration::from_millis(5));
            }
        }

        for (id, value) in [
            (SettingId::ConnectionHost, live_address.ip().to_string()),
            (SettingId::ConnectionPort, live_address.port().to_string()),
            (SettingId::ConnectionUsername, "submitted-user".to_owned()),
            (SettingId::ConnectionRoom, "submitted-room".to_owned()),
        ] {
            assert!(state.apply(GuiShellAction::EditConfigurationText {
                id,
                value: value.into(),
            }));
        }
        assert!(state.apply(GuiShellAction::EditConfigurationBool {
            id: SettingId::PlaybackSharedPlaylists,
            value: true,
        }));
        assert!(match intent {
            GuiSavedServerConnectIntent::ConnectOnce => {
                state.apply(GuiShellAction::BeginConnectOnce)
            }
            GuiSavedServerConnectIntent::SaveAndConnect => {
                state.apply(GuiShellAction::BeginSaveAndConnect)
            }
        });

        assert!(runtime.actions_for_pending_completion(&state).is_empty());
        // Deliberately do not submit `state` to the pump here. The request wakes the worker while
        // its compatibility projection still contains the dead endpoint above.
        let hello = hello_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("explicit request payload should connect to the submitted endpoint");
        assert!(hello.contains("\"username\":\"submitted-user\""));
        assert!(hello.contains("submitted-room"));
        assert_eq!(
            hello.contains("\"sharedPlaylists\":true"),
            intent == GuiSavedServerConnectIntent::SaveAndConnect,
            "Connect Once must keep unrelated settings pinned while Save and Connect submits them",
        );

        let completion_deadline = Instant::now() + Duration::from_secs(2);
        while state.pending_operation.is_some() {
            assert!(
                Instant::now() < completion_deadline,
                "threaded submitted-settings connect never cleared the shell pending operation; last error: {:?}",
                state.validation.last_action_error
            );
            for action in GuiNativeRuntimeBridge::drain_runtime_actions(&mut runtime) {
                let _ = state.apply(action);
            }
            GuiNativeRuntimePump::pump(&mut pump, &state);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.active_view, GuiShellView::Room);

        let persisted = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
            .expect("threaded connect should leave a readable config")
            .expect("threaded connect fixture should retain stored settings");
        if intent == GuiSavedServerConnectIntent::SaveAndConnect {
            assert_eq!(
                persisted.host.as_deref(),
                Some(live_address.ip().to_string().as_str())
            );
            assert_eq!(persisted.port, Some(live_address.port()));
            assert_eq!(persisted.room.as_deref(), Some("submitted-room"));
        } else {
            assert_eq!(persisted, original);
        }

        drop(pump);
        server_thread
            .join()
            .expect("submitted endpoint server thread should exit cleanly");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn threaded_public_connect_uses_submitted_server_and_identity_before_latest_input() {
    use std::{
        io::BufReader,
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    let live_listener = TcpListener::bind("127.0.0.1:0")
        .expect("threaded public-connect test should bind its submitted server");
    let live_address = live_listener
        .local_addr()
        .expect("submitted public server should expose its address");
    let dead_listener = TcpListener::bind("127.0.0.1:0")
        .expect("threaded public-connect test should reserve a stale server");
    let dead_address = dead_listener
        .local_addr()
        .expect("stale public server should expose its address");
    drop(dead_listener);

    let (hello_tx, hello_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = live_listener
            .accept()
            .expect("submitted public server should receive the GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("submitted public-server stream should clone"),
        );
        let hello = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "threaded submitted public-server connect",
        );
        hello_tx
            .send(hello)
            .expect("submitted public server should report the client Hello");
    });

    let original = StoredClientSettingsMvp {
        username: Some("saved-user".to_owned()),
        room: Some("saved-room".to_owned()),
        shared_playlist_enabled: Some(false),
        public_servers: Some(vec![("Stale".to_owned(), dead_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    };
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let (mut runtime, handle) = GuiQueuedRuntimeBridge::new();
    let mut pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        handle.clone(),
        owner,
        Duration::from_millis(5),
    )
    .expect("threaded public-connect runtime should spawn");
    let mut state = SorotteGuiShellAppState::from_stored_settings(&original);

    GuiNativeRuntimePump::pump(&mut pump, &state);
    let input_deadline = Instant::now() + Duration::from_secs(2);
    while handle.drain_actions().is_empty() {
        assert!(
            Instant::now() < input_deadline,
            "threaded runtime did not consume the stale public-server input"
        );
        thread::sleep(Duration::from_millis(5));
    }

    state.public_servers.servers[0].label = "Submitted".to_owned();
    state.public_servers.servers[0].address = live_address.to_string();
    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionUsername,
        value: "submitted-user".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "submitted-room".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());

    // As above, the explicit request must be sufficient while the worker still holds `original`.
    let hello = hello_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("public request payload should connect to the submitted server");
    assert!(hello.contains("\"username\":\"submitted-user\""));
    assert!(hello.contains("submitted-room"));
    assert!(hello.contains("\"sharedPlaylists\":false"));

    drop(pump);
    server_thread
        .join()
        .expect("submitted public-server thread should exit cleanly");
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
        host: Some("old.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(false),
        player_path: Some("C:/Program Files/VideoLAN/VLC/vlc.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: connect_host.clone().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionPort,
        value: connect_port.to_string().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "room2".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::Reconnect];
    assert!(state.apply(GuiShellAction::BeginSaveAndConnect));
    assert_eq!(
        state.pending_saved_server_connect_intent,
        Some(GuiSavedServerConnectIntent::SaveAndConnect)
    );

    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::from_state(&state)
            .expect("staged Save and Connect should capture submitted settings"),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();

    let active_settings = owner
        .active_session_settings
        .as_ref()
        .expect("Save and Connect should retain the persisted active-session settings");
    assert_eq!(active_settings.settings.room.as_deref(), Some("room2"));
    assert_eq!(active_settings.settings.shared_playlist_enabled, Some(true));

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
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
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
    assert_eq!(persisted_settings.shared_playlist_enabled, Some(true));
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
    assert!(hello_line.contains("\"sharedPlaylists\":true"));

    release_tx
        .send(())
        .expect("config connect test should release the server");
    server_thread
        .join()
        .expect("config connect test server thread should exit cleanly");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ordinary_save_promotes_media_library_fields_without_unpinning_session_settings() {
    let root = test_temp_root("ordinary-save-active-media-settings");
    let config_path = root.join("sorotte.ini");
    let original = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        port: Some(8999),
        language: Some("en".to_owned()),
        player_path: Some("C:/old/mpv.exe".to_owned()),
        streaming_quality_preset: Some("720p".to_owned()),
        shared_playlist_enabled: Some(false),
        media_search_directories: Some(vec!["C:/OldMedia".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(2.0),
        folder_search_timeout_seconds: Some(5.0),
        folder_search_double_check_interval_seconds: Some(1.0),
        folder_search_warning_threshold_seconds: Some(3.0),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &original)
        .expect("ordinary-save fixture should persist initial settings");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    owner.startup_saved_connect_attempted = true;
    owner.session_projects_to_shell = true;
    owner.active_session_settings = Some(
        sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
            &original,
        ),
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&original);

    for (id, value) in [
        (SettingId::ConnectionHost, "unsaved.example"),
        (SettingId::PlayerExecutable, "C:/new/mpv.exe"),
        (SettingId::StreamingQuality, "1080p"),
        (SettingId::GeneralLanguage, "fr"),
        (
            SettingId::MediaLibraryDirectories,
            "C:/NewMedia\nD:/Archive",
        ),
        (SettingId::MediaLibraryFirstFileTimeout, "4"),
        (SettingId::MediaLibrarySearchTimeout, "9"),
        (SettingId::MediaLibraryDoubleCheckInterval, "2"),
        (SettingId::MediaLibraryWarningThreshold, "7"),
    ] {
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id,
            value: value.to_owned().into(),
        }));
    }
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let submitted_settings = state.configuration.to_stored_settings();
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(submitted_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);

    let active = owner
        .active_session_settings
        .as_ref()
        .expect("ordinary save should preserve the active-session snapshot");
    assert_eq!(
        active.settings.media_search_directories,
        submitted_settings.media_search_directories
    );
    assert_eq!(
        active.settings.folder_search_first_file_timeout_seconds,
        Some(4.0)
    );
    assert_eq!(active.settings.folder_search_timeout_seconds, Some(9.0));
    assert_eq!(
        active.settings.folder_search_double_check_interval_seconds,
        Some(2.0)
    );
    assert_eq!(
        active.settings.folder_search_warning_threshold_seconds,
        Some(7.0)
    );
    assert_eq!(active.settings.host.as_deref(), Some("saved.example"));
    assert_eq!(
        active.settings.player_path.as_deref(),
        Some("C:/old/mpv.exe")
    );
    assert_eq!(
        active.settings.streaming_quality_preset.as_deref(),
        Some("720p")
    );
    assert_eq!(active.settings.shared_playlist_enabled, Some(false));
    assert_eq!(active.settings.language.as_deref(), Some("en"));
    assert_eq!(
        owner.runtime_operation_settings(&state),
        active.settings,
        "connected runtime operations must consume the promoted, still-pinned snapshot",
    );

    let persisted = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("ordinary save should leave a readable config")
        .expect("ordinary save should persist submitted settings");
    assert_eq!(persisted, submitted_settings);
    let _ = std::fs::remove_dir_all(root);
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
                br#"{"Hello":{"username":"draft-alice","room":{"name":"draft-room"},"version":"1.7.5","features":{"chat":true}}}"#,
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
        language: Some("en".to_owned()),
        shared_playlist_enabled: Some(false),
        chat_input_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionUsername,
        value: "draft-alice".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionRoom,
        value: "draft-room".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "pt-br".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::ChatInputEnabled,
        value: true,
    }));
    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::from_state(&state)
            .expect("staged public-server connect should capture submitted settings"),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    let active_settings = owner
        .active_session_settings
        .as_ref()
        .expect("public-server connect should pin an explicit active-session snapshot");
    assert_eq!(
        active_settings.config.connection.host.as_deref(),
        Some(address.ip().to_string().as_str())
    );
    assert_eq!(active_settings.config.connection.port.get(), address.port());
    assert_eq!(
        active_settings.settings.username.as_deref(),
        Some("draft-alice")
    );
    assert_eq!(active_settings.settings.room.as_deref(), Some("draft-room"));
    assert_eq!(active_settings.settings.language.as_deref(), Some("en"));
    assert_eq!(
        active_settings.settings.shared_playlist_enabled,
        Some(false)
    );
    assert_eq!(active_settings.settings.chat_input_enabled, Some(false));
    let projected_hello_in_connect_actions = connect_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "draft-room"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "draft-alice" && user.is_self)
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
    assert!(hello_line.contains("\"draft-alice\""));
    assert!(hello_line.contains("\"draft-room\""));
    assert!(hello_line.contains("\"sharedPlaylists\":false"));

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    let projected_hello_in_followup_actions = hello_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "draft-room"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "draft-alice" && user.is_self)
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

    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompletePendingOperation))
    );
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
