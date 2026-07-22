use super::*;

#[test]
fn gui_portable_smoke_regression_covers_tcp_state_churn_and_reconnect() {
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    // The workspace suite runs this end-to-end loopback smoke alongside hundreds of tests.
    // Keep scheduling delays from masquerading as transport failures while preserving a
    // finite deadline for every cross-thread and runtime-pump synchronization point.
    const SMOKE_DEADLINE: Duration = Duration::from_secs(10);

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("portable tcp churn smoke first listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("portable tcp churn smoke first listener should expose an address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("portable tcp churn smoke second listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("portable tcp churn smoke second listener should expose an address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (first_chat_tx, first_chat_rx) = mpsc::channel();
    let (first_state_tx, first_state_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("portable tcp churn smoke first server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable tcp churn smoke first server should clone stream");
        let mut reader = BufReader::new(reader_stream);

        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "portable tcp churn smoke first server",
        );
        first_hello_tx
            .send(hello_line)
            .expect("portable tcp churn smoke first server should report startup hello");

        for line in [
            r#"{"Hello":{"username":"portable-user","room":{"name":"portable-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"portable-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write initial line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate initial line");
        }
        first_state_tx
            .send("initial".to_owned())
            .expect("portable tcp churn smoke first server should signal initial state");

        let first_chat_line = read_protocol_line_matching(
            &mut reader,
            |line| line.contains("\"Chat\""),
            "portable tcp churn smoke first server",
        );
        first_chat_tx
            .send(first_chat_line)
            .expect("portable tcp churn smoke first server should report first chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"hellotcp"}}"#,
            r#"{"Set":{"playlistChange":{"files":["postchat1.mkv","postchat2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"portable-room"},"file":{"name":"bob-post.mp4"},"isReady":false,"controller":false}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write post-chat line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate post-chat line");
        }
        first_state_tx
            .send("postchat".to_owned())
            .expect("portable tcp churn smoke first server should signal post-chat state");

        let second_chat_line = read_protocol_line_matching(
            &mut reader,
            |line| line.contains("\"Chat\""),
            "portable tcp churn smoke first server",
        );
        first_chat_tx
            .send(second_chat_line)
            .expect("portable tcp churn smoke first server should report second chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"goodbyeprimary"}}"#,
            r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write user-left line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate user-left line");
        }
        first_state_tx
            .send("user-left".to_owned())
            .expect("portable tcp churn smoke first server should signal user-left state");

        release_first_rx
            // The release is intentionally sent only after the complete replacement-session
            // churn has been asserted. Parallel CI can legitimately take longer than one second
            // to exercise that second connection even though transport switching is healthy.
            .recv_timeout(SMOKE_DEADLINE)
            .expect("portable tcp churn smoke first server should be releasable");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let (second_chat_tx, second_chat_rx) = mpsc::channel();
    let (second_state_tx, second_state_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("portable tcp churn smoke second server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable tcp churn smoke second server should clone stream");
        let mut reader = BufReader::new(reader_stream);

        let hello_line = read_client_hello_after_optional_start_tls(
            &mut reader,
            &mut stream,
            "portable tcp churn smoke second server",
        );
        second_hello_tx
            .send(hello_line)
            .expect("portable tcp churn smoke second server should report reconnect hello");

        for line in [
            r#"{"Hello":{"username":"portable-user","room":{"name":"portable-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect1.mkv","reconnect2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"portable-room"},"file":{"name":"carol.mp4"},"isReady":false,"controller":false}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write initial line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate initial line");
        }
        second_state_tx
            .send("initial".to_owned())
            .expect("portable tcp churn smoke second server should signal initial state");

        let first_chat_line = read_protocol_line_matching(
            &mut reader,
            |line| line.contains("\"Chat\""),
            "portable tcp churn smoke second server",
        );
        second_chat_tx
            .send(first_chat_line)
            .expect("portable tcp churn smoke second server should report first chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"helloreconnect"}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect-post1.mkv","reconnect-post2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":40.0,"paused":true,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"portable-room"},"file":{"name":"carol-post.mp4"},"isReady":true,"controller":true}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write post-chat line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate post-chat line");
        }
        second_state_tx
            .send("postchat".to_owned())
            .expect("portable tcp churn smoke second server should signal post-chat state");

        let second_chat_line = read_protocol_line_matching(
            &mut reader,
            |line| line.contains("\"Chat\""),
            "portable tcp churn smoke second server",
        );
        second_chat_tx
            .send(second_chat_line)
            .expect("portable tcp churn smoke second server should report second chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"goodbyereconnect"}}"#,
            r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write user-left line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate user-left line");
        }
        second_state_tx
            .send("user-left".to_owned())
            .expect("portable tcp churn smoke second server should signal user-left state");
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "portable-user",
            "portable-room",
            first_address.to_string(),
            TlsPolicy::PreferTls,
        )
        .expect("portable tcp churn smoke owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("portable-user".to_owned()),
        room: Some("portable-room".to_owned()),
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Reconnect".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let first_hello = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_hello_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke first startup hello",
    );
    assert!(first_hello.contains("\"Hello\""));
    assert!(first_hello.contains("\"portable-user\""));
    assert_eq!(
        first_state_rx
            .recv_timeout(SMOKE_DEADLINE)
            .expect("portable tcp churn smoke first server should publish initial state"),
        "initial"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv"])
                && state.main_window.playback_paused
                && state.selection.selected_main_window_playlist == Some(1)
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "bob" && user.is_ready && user.is_controller)
        },
        "portable primary initial state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv", "episode2.mkv"]
    );
    assert!(state.main_window.playback_paused);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "bob" && user.is_ready && user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hellotcp".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hellotcp".to_owned()),
    ));
    let first_chat_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        first_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    assert!(state.pending_operation.is_none());
    assert!(state.outgoing_chat_message.is_none());
    let first_chat = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_chat_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke first server should receive first chat",
    );
    assert!(first_chat.contains("\"Chat\""));
    assert!(first_chat.contains("hellotcp"));
    assert_eq!(
        first_state_rx
            .recv_timeout(SMOKE_DEADLINE)
            .expect("portable tcp churn smoke first server should publish post-chat state"),
        "postchat"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["postchat1.mkv", "postchat2.mkv"])
                && !state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "bob" && !user.is_ready && !user.is_controller)
        },
        "portable primary post-chat state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["postchat1.mkv", "postchat2.mkv"]
    );
    assert!(!state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "bob" && !user.is_ready && !user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "goodbyeprimary".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("goodbyeprimary".to_owned()),
    ));
    let second_primary_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        second_primary_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let second_primary_chat = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &first_chat_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke first server should receive second chat",
    );
    assert!(second_primary_chat.contains("\"Chat\""));
    assert!(second_primary_chat.contains("goodbyeprimary"));
    assert_eq!(
        first_state_rx
            .recv_timeout(SMOKE_DEADLINE)
            .expect("portable tcp churn smoke first server should publish user-left state"),
        "user-left"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .users
                .iter()
                .all(|user| user.username != "bob")
        },
        "portable primary user-left state",
    );
    assert!(
        state
            .main_window
            .users
            .iter()
            .all(|user| user.username != "bob")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::from_state(&state)
            .expect("staged reconnect should capture its submitted public server"),
    ));
    let reconnect_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect))
    );

    let second_hello = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_hello_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke second reconnect hello",
    );
    assert!(second_hello.contains("\"Hello\""));
    assert!(second_hello.contains("\"portable-user\""));
    assert!(second_hello.contains("\"portable-room\""));
    assert_eq!(
        second_state_rx.recv_timeout(SMOKE_DEADLINE).expect(
            "portable tcp churn smoke second server should publish initial reconnect state"
        ),
        "initial"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["reconnect1.mkv", "reconnect2.mkv"])
                && !state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "carol" && !user.is_ready && !user.is_controller)
        },
        "portable reconnect initial state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["reconnect1.mkv", "reconnect2.mkv"]
    );
    assert!(!state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "carol" && !user.is_ready && !user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "helloreconnect".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("helloreconnect".to_owned()),
    ));
    let first_reconnect_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        first_reconnect_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let first_reconnect_chat = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_chat_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke second server should receive first reconnect chat",
    );
    assert!(first_reconnect_chat.contains("\"Chat\""));
    assert!(first_reconnect_chat.contains("helloreconnect"));
    assert_eq!(
        second_state_rx.recv_timeout(SMOKE_DEADLINE).expect(
            "portable tcp churn smoke second server should publish reconnect post-chat state"
        ),
        "postchat"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["reconnect-post1.mkv", "reconnect-post2.mkv"])
                && state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "carol" && user.is_ready && user.is_controller)
        },
        "portable reconnect post-chat state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["reconnect-post1.mkv", "reconnect-post2.mkv"]
    );
    assert!(state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "carol" && user.is_ready && user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "goodbyereconnect".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("goodbyereconnect".to_owned()),
    ));
    let second_reconnect_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        second_reconnect_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let second_reconnect_chat = recv_from_channel_while_pumping_runtime(
        &mut owner,
        &handle,
        &mut state,
        &second_chat_rx,
        SMOKE_DEADLINE,
        "portable tcp churn smoke second server should receive second reconnect chat",
    );
    assert!(second_reconnect_chat.contains("\"Chat\""));
    assert!(second_reconnect_chat.contains("goodbyereconnect"));
    assert_eq!(
        second_state_rx.recv_timeout(SMOKE_DEADLINE).expect(
            "portable tcp churn smoke second server should publish reconnect user-left state"
        ),
        "user-left"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        SMOKE_DEADLINE,
        |state| {
            state
                .main_window
                .users
                .iter()
                .all(|user| user.username != "carol")
        },
        "portable reconnect user-left state",
    );
    assert!(
        state
            .main_window
            .users
            .iter()
            .all(|user| user.username != "carol")
    );

    release_first_tx
        .send(())
        .expect("portable tcp churn smoke first server should be releasable");
    first_server_thread
        .join()
        .expect("portable tcp churn smoke first server thread should complete");
    second_server_thread
        .join()
        .expect("portable tcp churn smoke second server thread should complete");
}
