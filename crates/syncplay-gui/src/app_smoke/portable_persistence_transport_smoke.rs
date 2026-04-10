use super::*;

#[test]
fn gui_portable_smoke_regression_sequences_persistence_and_transport_flows() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    // Persistence save + reload (portable equivalent of the isolated config checks).
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "syncplay-gui-portable-smoke-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut persisted_owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let persisted_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut persisted_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("portable-save.example".to_owned()),
        room: Some("portable-room-a".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationSave));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let save_actions = persisted_handle.drain_actions();
    assert!(
        save_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationSave(settings) if settings == &saved_settings
        )),
        "portable persistence smoke save should emit completion with persisted settings"
    );
    for action in save_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("portable smoke save should leave a readable config"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("portable-reload.example".to_owned()),
        room: Some("portable-room-b".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("portable smoke reload seed should write config");
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationReload));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let reload_actions = persisted_handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings)
                if settings == &reloaded_settings
        )),
        "portable nontransport smoke reload should emit completion with reloaded settings"
    );
    for action in reload_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        persisted_state.saved_configuration.host.as_deref(),
        Some("portable-reload.example")
    );
    let _ = std::fs::remove_file(&path);

    // Loopback transport chat echo.
    let mut loopback_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("portable-user", "portable-room")
        .expect("portable smoke loopback runtime owner should bootstrap");
    let loopback_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut loopback_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            chat_input_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    assert!(loopback_state.apply(GuiShellAction::BeginLocalChatSend(
        "portable-loopback".to_owned()
    )));
    loopback_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("portable-loopback".to_owned()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut loopback_owner, &loopback_handle, &loopback_state);
    let loopback_actions = loopback_handle.drain_actions();
    assert!(
        loopback_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "portable smoke loopback segment should complete local chat sends"
    );
    assert!(
        loopback_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "portable-user" && message == "portable-loopback"
        )),
        "portable smoke loopback segment should echo chat through inbound handling"
    );
    for action in loopback_actions {
        assert!(loopback_state.apply(action));
    }
    assert_eq!(
        loopback_state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("portable-user".to_owned(), "portable-loopback".to_owned()))
    );
    assert_eq!(loopback_state.main_window.chat.len(), 1);

    // TCP startup + reconnect swap.
    let first_listener =
        TcpListener::bind("127.0.0.1:0").expect("portable smoke first tcp listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("portable smoke first tcp listener should expose a local address");
    let second_listener =
        TcpListener::bind("127.0.0.1:0").expect("portable smoke second tcp listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("portable smoke second tcp listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("portable smoke first tcp server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable smoke first tcp server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "portable smoke first tcp server",
        );
        first_hello_tx
            .send(hello_line)
            .expect("portable smoke first tcp server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable smoke first tcp server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("portable smoke second tcp server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable smoke second tcp server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "portable smoke second tcp server",
        );
        second_hello_tx
            .send(hello_line)
            .expect("portable smoke second tcp server should report its hello");
    });

    let mut tcp_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "portable-user",
            "portable-room",
            first_address.to_string(),
        )
        .expect("portable smoke tcp runtime owner should bootstrap");
    let tcp_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut tcp_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("portable-user".to_owned()),
        room: Some("portable-room".to_owned()),
        public_servers: Some(vec![("Reconnect".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut tcp_owner, &tcp_handle, &tcp_state);
    for action in tcp_handle.drain_actions() {
        assert!(tcp_state.apply(action));
    }
    let first_hello_line = recv_from_channel_while_pumping_runtime(
        &mut tcp_owner,
        &tcp_handle,
        &mut tcp_state,
        &first_hello_rx,
        Duration::from_secs(1),
        "portable smoke first tcp startup hello",
    );
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"portable-user\""));

    assert!(tcp_state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    tcp_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut tcp_owner, &tcp_handle, &tcp_state);
    let reconnect_actions = tcp_handle.drain_actions();
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "portable smoke reconnect segment should complete selected public-server connect"
    );
    for action in reconnect_actions {
        assert!(tcp_state.apply(action));
    }

    let second_hello_line = recv_from_channel_while_pumping_runtime(
        &mut tcp_owner,
        &tcp_handle,
        &mut tcp_state,
        &second_hello_rx,
        Duration::from_secs(1),
        "portable smoke second tcp reconnect hello",
    );
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"portable-user\""));
    assert!(second_hello_line.contains("\"portable-room\""));

    release_first_tx
        .send(())
        .expect("portable smoke first tcp server should be releasable");
    first_server_thread
        .join()
        .expect("portable smoke first tcp server thread should complete");
    second_server_thread
        .join()
        .expect("portable smoke second tcp server thread should complete");
}
