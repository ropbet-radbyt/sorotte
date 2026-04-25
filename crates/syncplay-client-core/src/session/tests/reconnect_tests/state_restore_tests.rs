use super::*;

#[test]
fn reconnect_state_restore_emits_ready_and_file_actions_after_hello() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file metadata should apply");

    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("reconnect hello should apply");

    let restore_actions = session.runtime_actions_for_reconnect_state_restore_if_needed();
    assert_eq!(
        restore_actions,
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringState,
            ),
            ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false,
            },
            ClientRuntimeAction::SetFile {
                file_payload: json!({
                    "name": "movie.mkv",
                    "size": 123456789,
                    "duration": 95.5
                })
            },
            ClientRuntimeAction::RequestUserList,
        ]
    );
    assert!(
        session
            .runtime_actions_for_reconnect_state_restore_if_needed()
            .is_empty(),
        "state restore actions should drain after first retrieval"
    );
}

#[test]
fn repeated_reconnect_resets_preserve_cached_restore_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file metadata should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");

    session.reset_sync_state_for_reconnect();
    session.reset_sync_state_for_reconnect();

    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("reconnect hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty server playlist snapshot should apply");

    let state_restore_actions = session.runtime_actions_for_reconnect_state_restore_if_needed();
    assert_eq!(
        state_restore_actions,
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringState,
            ),
            ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false,
            },
            ClientRuntimeAction::SetFile {
                file_payload: json!({
                    "name": "movie.mkv",
                    "size": 123456789,
                    "duration": 95.5
                }),
            },
            ClientRuntimeAction::RequestUserList,
        ]
    );

    let playlist_restore_actions =
        session.runtime_actions_for_reconnect_playlist_restore_if_needed();
    assert_eq!(
        playlist_restore_actions,
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringPlaylist,
            ),
            ClientRuntimeAction::SetPlaylist {
                files: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
            },
            ClientRuntimeAction::SetPlaylistIndex { index: 1 },
        ]
    );
}

#[test]
fn client_runtime_reconnect_state_restore_dispatches_ready_and_file_messages() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("reconnect hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_reconnect_state_restore_if_needed()
        .expect("reconnect state restore should dispatch");

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 3);
    assert_eq!(
        control.reconnect_notifications(),
        &[ReconnectTransitionNotification::RestoringState]
    );

    let ProtocolMessage::Set(ready_message) = &control.outbound_messages()[0] else {
        panic!("first reconnect restore message should be Set.ready");
    };
    let ready = ready_message
        .set
        .ready
        .as_ref()
        .expect("first reconnect restore message should include ready payload");
    assert!(ready.is_ready);
    assert_eq!(ready.manually_initiated, Some(false));

    let ProtocolMessage::Set(file_message) = &control.outbound_messages()[1] else {
        panic!("second reconnect restore message should be Set.file");
    };
    let file = file_message
        .set
        .file
        .as_ref()
        .expect("second reconnect restore message should include file payload");
    assert_eq!(file.name.as_deref(), Some("movie.mkv"));
    assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
    assert_eq!(file.duration, Some(95.5));
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[2] else {
        panic!("third reconnect restore message should be List");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}
