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
    let media_root = test_temp_root("transport-shared-playlist-local-publish");
    let episode1_path = media_root.join("episode1.mkv");
    let episode2_path = media_root.join("episode2.mkv");
    std::fs::write(&episode1_path, b"one").expect("first media fixture should be written");
    std::fs::write(&episode2_path, b"two").expect("second media fixture should be written");

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            episode1_path.to_string_lossy().into_owned(),
            episode2_path.to_string_lossy().into_owned(),
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
    let _ = std::fs::remove_dir_all(media_root);
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
fn gui_persisted_config_runtime_owner_does_not_publish_opened_local_path_before_player_completion()
{
    struct OpenOnlyPlayer;

    impl PlayerAdapter for OpenOnlyPlayer {
        fn name(&self) -> &'static str {
            "open-only"
        }

        fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }
    }

    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OpenOnlyPlayer)));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        session_transport
            .drain_outbound_protocol_lines()
            .iter()
            .any(|line| line.contains("\"Hello\""))
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    let media_root = test_temp_root("transport-open-path-before-metadata");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        owner.player_local_file_placeholder,
        "the playlist-resolution candidate should remain unconfirmed until correlated player completion"
    );
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.state),
        Some(crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Loading),
        "publishing the known local path must not promote the resolution attempt to Active",
    );
    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines.iter().all(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|message| message.get("Set").and_then(|set| set.get("file")).cloned())
                .is_none()
        }),
        "command acceptance must not publish the unresolved local file identity; outbound_protocol_lines={outbound_protocol_lines:?}"
    );
    assert!(owner.last_published_local_file.is_none());
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_publish_observed_then_rejected_tracked_candidate() {
    #[derive(Default)]
    struct ObservedThenRejectedState {
        progress: std::collections::VecDeque<sorotte_player_api::PlayerCommandProgress>,
        outcomes: std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
        local_files: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
    }

    struct ObservedThenRejectedPlayer {
        state: std::sync::Arc<std::sync::Mutex<ObservedThenRejectedState>>,
    }

    impl PlayerAdapter for ObservedThenRejectedPlayer {
        fn name(&self) -> &'static str {
            "observed-then-rejected"
        }

        fn execute_tracked(
            &mut self,
            command: sorotte_player_api::PlayerCommand,
        ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
            let sorotte_player_api::PlayerCommand::OpenFile(_) = command else {
                return Err(sorotte_player_api::PlayerError::Unsupported("test command"));
            };
            let command_id = sorotte_player_api::PlayerCommandId::new(41);
            let generation = sorotte_player_api::PlayerMediaGeneration::new(9);
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .progress
                .push_back(sorotte_player_api::PlayerCommandProgress::accepted(
                    command_id,
                    Some(generation),
                    None,
                ));
            Ok(command_id)
        }

        fn take_command_progress(&mut self) -> Option<sorotte_player_api::PlayerCommandProgress> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .progress
                .pop_front()
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .outcomes
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_files
                .pop_front()
        }
    }

    let player_state =
        std::sync::Arc::new(std::sync::Mutex::new(ObservedThenRejectedState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        ObservedThenRejectedPlayer {
            state: player_state.clone(),
        },
    )));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    let media_root = test_temp_root("transport-observed-then-rejected-tracked-media");
    let media_path = media_root.join("observed-then-rejected.mkv");
    std::fs::write(&media_path, b"rejected media fixture")
        .expect("rejected media fixture should be written");
    let requested_target = media_path.to_string_lossy().into_owned();
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![requested_target.clone()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    assert!(owner.player_local_file_placeholder);
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.state),
        Some(crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Loading)
    );

    {
        let mut queued = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queued
            .outcomes
            .push_back(sorotte_player_api::PlayerMediaLoadOutcome::success(
                requested_target.clone(),
                Some(requested_target.clone()),
            ));
        queued.local_files.push_back(
            sorotte_player_api::LocalFileUpdate::new("observed-then-rejected.mkv")
                .with_path(requested_target.clone()),
        );
    }
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let observed_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        observed_lines
            .iter()
            .all(|line| !line.contains("observed-then-rejected.mkv")),
        "file-loaded success/local observations must remain private until tracked completion: {observed_lines:?}"
    );
    assert!(owner.player_local_file_placeholder);
    assert!(!owner.player_local_file_ready_for_attached_sync());
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.state),
        Some(crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Loading)
    );

    {
        let mut queued = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queued
            .progress
            .push_back(sorotte_player_api::PlayerCommandProgress::finished(
                sorotte_player_api::PlayerCommandId::new(41),
                Some(sorotte_player_api::PlayerMediaGeneration::new(9)),
                None,
                None,
                sorotte_player_api::PlayerCommandResult::Failed(
                    sorotte_player_api::PlayerCommandFailureKind::Unknown,
                ),
            ));
    }
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let rejected_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        rejected_lines
            .iter()
            .all(|line| !line.contains("observed-then-rejected.mkv")),
        "a provisional local observation rejected by its tracked command must never publish: {rejected_lines:?}"
    );
    assert!(owner.player_local_file.is_none());
    assert!(!owner.player_local_file_placeholder);
    assert!(!owner.player_local_file_ready_for_attached_sync());
    assert!(owner.last_published_local_file.is_none());
    let attempt = owner
        .playlist_resolution_attempt
        .as_ref()
        .expect("the rejected attempt should remain available for fallback/retry");
    assert_eq!(
        attempt.state,
        crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Failed
    );
    assert_eq!(attempt.failed_candidates.len(), 1);

    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_never_publishes_accepted_then_rejected_local_media() {
    #[derive(Default)]
    struct RejectedLoadState {
        outcomes: std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
    }

    struct AcceptedThenRejectedPlayer {
        state: std::sync::Arc<std::sync::Mutex<RejectedLoadState>>,
    }

    impl PlayerAdapter for AcceptedThenRejectedPlayer {
        fn name(&self) -> &'static str {
            "accepted-then-rejected"
        }

        fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .outcomes
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RejectedLoadState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        AcceptedThenRejectedPlayer {
            state: player_state.clone(),
        },
    )));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    let previously_active_file = sorotte_player_api::LocalFileUpdate::new("previously-active.mkv")
        .with_path("C:/Media/previously-active.mkv");
    owner.player_local_file = Some(previously_active_file.clone());
    owner.player_local_file_placeholder = false;
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let previously_active_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        previously_active_lines
            .iter()
            .any(|line| line.contains("previously-active.mkv")),
        "test setup must publish the previously active identity: {previously_active_lines:?}",
    );
    assert_eq!(
        owner.last_published_local_file,
        Some(previously_active_file)
    );

    let media_root = test_temp_root("transport-accepted-then-rejected-media");
    let media_path = media_root.join("rejected.mkv");
    std::fs::write(&media_path, b"rejected media fixture")
        .expect("rejected media fixture should be written");
    let requested_target = media_path.to_string_lossy().into_owned();
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![requested_target.clone()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let accepted_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        accepted_lines
            .iter()
            .all(|line| !line.contains(r#""file""#)),
        "accepted-but-unconfirmed media must not publish a file identity: {accepted_lines:?}",
    );
    assert!(owner.player_local_file_placeholder);
    assert_eq!(
        owner
            .last_published_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("previously-active.mkv")
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .outcomes
        .push_back(sorotte_player_api::PlayerMediaLoadOutcome::failure(
            requested_target,
            None,
            sorotte_player_api::PlayerMediaLoadFailureKind::Unknown,
            "player rejected the accepted load",
        ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let rejected_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        rejected_lines
            .iter()
            .all(|line| !line.contains("rejected.mkv")),
        "rejected media must never leak into the room file identity: {rejected_lines:?}",
    );
    assert!(
        rejected_lines.iter().any(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|message| message.get("Set").and_then(|set| set.get("file")).cloned())
                .and_then(|file| file.as_object().cloned())
                .is_some_and(|file| file.is_empty())
        }),
        "failure after a prior active identity must publish a compensating file clear: {rejected_lines:?}",
    );
    assert!(owner.player_local_file.is_none());
    assert!(!owner.player_local_file_placeholder);
    assert!(owner.last_published_local_file.is_none());
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_publishes_cached_media_match_without_health_probe() {
    let root = test_temp_root("cached-media-match-publish-without-health");
    let config_path = root.join("settings.ini");
    let media_path = root.join("episode2.mkv");
    std::fs::write(&media_path, b"indexed media").expect("indexed media fixture should be written");
    let metadata = std::fs::metadata(&media_path).expect("indexed media metadata should load");
    let modified_unix_millis = metadata
        .modified()
        .expect("indexed media modified time should load")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("indexed media modified time should be after unix epoch")
        .as_millis() as u64;
    let extraction_settings =
        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
    let record = sorotte_media_match::MediaFingerprintRecord {
        identity: sorotte_media_match::MediaFileIdentity::new(
            &media_path,
            modified_unix_millis,
            metadata.len(),
        ),
        algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings,
        duration_seconds: Some(42.0),
        container_fingerprint: "cached-signature-fixture".to_owned(),
        audio_anchors: vec![sorotte_media_match::AudioAnchor {
            bucket: 700,
            t_ms: 10_000,
            weight: 4,
        }],
        audio_error: None,
    };
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(record);
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("cached media-match record should be saved");
    assert!(
        crate::app::media_match_support::media_match_wire_value_for_path(
            &root,
            &media_path.to_string_lossy()
        )
        .is_some(),
        "cached media-match record should be loadable for the indexed file"
    );

    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(metadata.len())
            .with_path(media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.media_match_runtime_snapshot.settings = state.media_match.settings.clone();

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true,"mediaMatch":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            message
                .get("Set")
                .and_then(|set| set.get("file"))
                .and_then(|file| file.get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY))
                .is_some()
        }),
        "cached media-match signatures should be shared even before the runtime health snapshot has been probed; outbound_protocol_lines={outbound_protocol_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_suppresses_cached_media_match_for_remote_exact_playlist_item()
{
    let root = test_temp_root("cached-media-match-suppressed-remote-exact-playlist");
    let config_path = root.join("settings.ini");
    let media_path = root.join("episode2.mkv");
    std::fs::write(&media_path, b"indexed media").expect("indexed media fixture should be written");
    let metadata = std::fs::metadata(&media_path).expect("indexed media metadata should load");
    let modified_unix_millis = metadata
        .modified()
        .expect("indexed media modified time should load")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("indexed media modified time should be after unix epoch")
        .as_millis() as u64;
    let record = sorotte_media_match::MediaFingerprintRecord {
        identity: sorotte_media_match::MediaFileIdentity::new(
            &media_path,
            modified_unix_millis,
            metadata.len(),
        ),
        algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings:
            sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        duration_seconds: Some(42.0),
        container_fingerprint: "cached-signature-fixture".to_owned(),
        audio_anchors: vec![sorotte_media_match::AudioAnchor {
            bucket: 700,
            t_ms: 10_000,
            weight: 4,
        }],
        audio_error: None,
    };
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(record);
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("cached media-match record should be saved");

    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(metadata.len())
            .with_path(media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.media_match_runtime_snapshot.settings = state.media_match.settings.clone();

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true,"mediaMatch":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            message
                .get("Set")
                .and_then(|set| set.get("file"))
                .is_some_and(|file| {
                    file.get("name").and_then(serde_json::Value::as_str) == Some("episode2.mkv")
                        && file
                            .get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY)
                            .is_none()
                })
        }),
        "exact shared-playlist receivers should publish normal file metadata without a media-match signature; outbound_protocol_lines={outbound_protocol_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_suppresses_media_match_without_server_capability() {
    let root = test_temp_root("cached-media-match-suppressed-without-capability");
    let config_path = root.join("settings.ini");
    let media_path = root.join("episode2.mkv");
    std::fs::write(&media_path, b"indexed media").expect("indexed media fixture should be written");
    let metadata = std::fs::metadata(&media_path).expect("indexed media metadata should load");
    let modified_unix_millis = metadata
        .modified()
        .expect("indexed media modified time should load")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("indexed media modified time should be after unix epoch")
        .as_millis() as u64;
    let extraction_settings =
        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
    let record = sorotte_media_match::MediaFingerprintRecord {
        identity: sorotte_media_match::MediaFileIdentity::new(
            &media_path,
            modified_unix_millis,
            metadata.len(),
        ),
        algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings,
        duration_seconds: Some(42.0),
        container_fingerprint: "cached-signature-fixture".to_owned(),
        audio_anchors: vec![sorotte_media_match::AudioAnchor {
            bucket: 700,
            t_ms: 10_000,
            weight: 4,
        }],
        audio_error: None,
    };
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(record);
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("cached media-match record should be saved");
    assert!(
        crate::app::media_match_support::media_match_wire_value_for_path(
            &root,
            &media_path.to_string_lossy()
        )
        .is_some(),
        "test setup should have a cached media-match signature"
    );

    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(metadata.len())
            .with_path(media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.media_match_runtime_snapshot.settings = state.media_match.settings.clone();

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true}}}"#,
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
            file.get("name").and_then(serde_json::Value::as_str) == Some("episode2.mkv")
                && file
                    .get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY)
                    .is_none()
        }),
        "normal file metadata should still publish without the media-match server capability; outbound_protocol_lines={outbound_protocol_lines:?}"
    );
    assert!(
        outbound_protocol_lines.iter().all(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|message| message.get("Set").and_then(|set| set.get("file")).cloned())
                .and_then(|file| {
                    file.get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY)
                        .cloned()
                })
                .is_none()
        }),
        "media-match signatures must not be sent unless the server explicitly advertises support; outbound_protocol_lines={outbound_protocol_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_republishes_media_match_when_signature_becomes_available() {
    let root = test_temp_root("cached-media-match-republish-after-file-publish");
    let config_path = root.join("settings.ini");
    let media_path = root.join("episode2.mkv");
    std::fs::write(&media_path, b"indexed media").expect("indexed media fixture should be written");
    let metadata = std::fs::metadata(&media_path).expect("indexed media metadata should load");

    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_duration_seconds(42.0)
            .with_size_bytes(metadata.len())
            .with_path(media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.media_match_runtime_snapshot.settings = state.media_match.settings.clone();

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true,"mediaMatch":true}}}"#,
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let first_publish_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        first_publish_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            message
                .get("Set")
                .and_then(|set| set.get("file"))
                .is_some_and(|file| {
                    file.get("name").and_then(serde_json::Value::as_str) == Some("episode2.mkv")
                        && file
                            .get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY)
                            .is_none()
                })
        }),
        "first file publish should occur before the test cache has a media-match signature; first_publish_lines={first_publish_lines:?}"
    );

    let modified_unix_millis = metadata
        .modified()
        .expect("indexed media modified time should load")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("indexed media modified time should be after unix epoch")
        .as_millis() as u64;
    let record = sorotte_media_match::MediaFingerprintRecord {
        identity: sorotte_media_match::MediaFileIdentity::new(
            &media_path,
            modified_unix_millis,
            metadata.len(),
        ),
        algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings:
            sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        duration_seconds: Some(42.0),
        container_fingerprint: "cached-signature-fixture".to_owned(),
        audio_anchors: vec![sorotte_media_match::AudioAnchor {
            bucket: 700,
            t_ms: 10_000,
            weight: 4,
        }],
        audio_error: None,
    };
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(record);
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("cached media-match record should be saved");
    assert!(
        crate::app::media_match_support::media_match_wire_value_for_path(
            &root,
            &media_path.to_string_lossy()
        )
        .is_some(),
        "cached media-match record should be loadable for the indexed file"
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let republish_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        republish_lines.iter().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            message
                .get("Set")
                .and_then(|set| set.get("file"))
                .and_then(|file| file.get(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY))
                .is_some()
        }),
        "same local file should republish once its media-match signature becomes available; republish_lines={republish_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
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
