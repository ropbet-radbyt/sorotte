use super::*;

#[test]
fn reconnect_playlist_restore_emits_actions_on_empty_server_playlist_snapshot() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");

    session.reset_sync_state_for_reconnect();
    assert!(
        session.current_room_playlist().is_none(),
        "reconnect reset should clear stale playlist state until server snapshot arrives"
    );

    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty server playlist snapshot should apply");

    let restore_actions = session.runtime_actions_for_reconnect_playlist_restore_if_needed();
    assert_eq!(
        restore_actions,
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
    assert!(
        session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "playlist restore actions should drain after first retrieval"
    );
}

#[test]
fn reconnect_playlist_restore_ignores_non_matching_playlist_updates() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("local playlist should apply");

    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["server_episode.mkv"],"user":"bob"}}}"#,
        )
        .expect("non-empty server playlist update should apply");
    assert!(
        session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "non-empty server playlist snapshots should suppress reconnect restore"
    );
}

#[test]
fn reconnect_playlist_restore_is_suppressed_when_server_shared_playlists_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");

    session.reset_sync_state_for_reconnect();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":false}}}"#,
            )
            .expect("reconnect hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty server playlist snapshot should apply");

    assert!(
        session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "reconnect playlist restore should be suppressed when the server disables shared playlists"
    );
    assert!(
        session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "suppressed reconnect restore should still drain the pending restore intent"
    );
}

#[test]
fn client_runtime_reconnect_playlist_restore_dispatches_protocol_messages() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");
    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty reconnect playlist snapshot should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_reconnect_playlist_restore_if_needed()
        .expect("reconnect playlist restore should dispatch");

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    assert_eq!(
        control.reconnect_notifications(),
        &[ReconnectTransitionNotification::RestoringPlaylist]
    );
    let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound reconnect restore message should be Set.playlistChange");
    };
    let playlist_change = playlist_change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(playlist_index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound reconnect restore message should be Set.playlistIndex");
    };
    let playlist_index = playlist_index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_reconnect_state_and_playlist_restore_precede_validation_mismatch_notification() {
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
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("local playlist index should apply");

    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("reconnect hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":120.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("reconnect room playstate should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty reconnect playlist snapshot should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_if_needed()
        .expect("reconnect state restore should dispatch");
    assert!(
        runtime.session().reconnect_state_restore_validation_pending,
        "state restore dispatch should enable reconnect validation"
    );

    runtime
        .run_reconnect_playlist_restore_if_needed()
        .expect("reconnect playlist restore should dispatch");
    assert!(
        runtime.session().reconnect_state_restore_validation_pending,
        "playlist restore should not clear reconnect validation pending state"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("reconnect validation should run after state+playlist restore dispatch");

    let reconnect_notifications = runtime.control().reconnect_notifications();
    assert_eq!(
        reconnect_notifications.len(),
        3,
        "reconnect notifications should preserve restore-state, restore-playlist, then validation-mismatch ordering"
    );
    assert_eq!(
        reconnect_notifications[0],
        ReconnectTransitionNotification::RestoringState
    );
    assert_eq!(
        reconnect_notifications[1],
        ReconnectTransitionNotification::RestoringPlaylist
    );
    let ReconnectTransitionNotification::StateRestoreValidationMismatch {
        local_paused,
        room_paused,
        local_position,
        room_position,
        position_diff_seconds,
    } = &reconnect_notifications[2]
    else {
        panic!("third reconnect notification should be a validation mismatch");
    };
    assert!(*local_paused);
    assert!(!room_paused);
    assert_eq!(*local_position, 117.5);
    assert!(
        (120.0..120.1).contains(room_position),
        "validation mismatch should use the aged room position recorded at validation time"
    );
    assert!(
        (*position_diff_seconds - (*room_position - 117.5)).abs() < 0.001,
        "position diff should be derived from the same aged room position"
    );
    assert_eq!(
        runtime.control().outbound_messages().len(),
        5,
        "state restore + playlist restore should enqueue ready/file/list/playlist/index protocol messages"
    );

    let ProtocolMessage::Set(first_outbound) = &runtime.control().outbound_messages()[0] else {
        panic!("first reconnect outbound message should be Set.ready");
    };
    assert!(first_outbound.set.ready.is_some());
    let ProtocolMessage::Set(second_outbound) = &runtime.control().outbound_messages()[1] else {
        panic!("second reconnect outbound message should be Set.file");
    };
    assert!(second_outbound.set.file.is_some());
    let ProtocolMessage::List(third_outbound) = &runtime.control().outbound_messages()[2] else {
        panic!("third reconnect outbound message should be List");
    };
    assert!(matches!(third_outbound.list, ListPayload::Request(_)));
    let ProtocolMessage::Set(fourth_outbound) = &runtime.control().outbound_messages()[3] else {
        panic!("fourth reconnect outbound message should be Set.playlistChange");
    };
    assert!(fourth_outbound.set.playlist_change.is_some());
    let ProtocolMessage::Set(fifth_outbound) = &runtime.control().outbound_messages()[4] else {
        panic!("fifth reconnect outbound message should be Set.playlistIndex");
    };
    assert!(fifth_outbound.set.playlist_index.is_some());

    assert_eq!(
        runtime.player().paused,
        Some(false),
        "validation mismatch should still issue corrective pause after playlist restore dispatch"
    );
    assert!(
        runtime
            .player()
            .position
            .is_some_and(|position| (120.0..120.1).contains(&position)),
        "validation mismatch should still issue corrective seek after playlist restore dispatch"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "validation pending should clear after post-restore correction"
    );
    assert_eq!(
        runtime.drain_player_playback_telemetry_updates(),
        vec![
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5)
        ],
        "telemetry should remain available for diagnostics drains after the ordered restore/playlist/validation sequence"
    );
}

#[test]
fn client_runtime_reconnect_playlist_restore_uses_latest_local_playlist_before_echo() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_queue_playlist_item("episode3.mkv", true)
            .expect("queue-and-select command should not fail"),
        "queue-and-select should enqueue playlist updates before the server echo"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("queue-and-select should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(playlist.index, Some(2));

    let (mut session, player, _control) = runtime.into_parts();
    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty reconnect playlist snapshot should apply");

    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_reconnect_playlist_restore_if_needed()
        .expect("reconnect playlist restore should dispatch");

    assert_eq!(runtime.control().outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[0] else {
        panic!("first reconnect restore message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("reconnect restore should include playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &runtime.control().outbound_messages()[1] else {
        panic!("second reconnect restore message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("reconnect restore should include playlistIndex");
    assert_eq!(playlist_index.index, 2);
    assert!(playlist_index.user.is_none());
}
