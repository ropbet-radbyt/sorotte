use super::*;

#[test]
fn queued_runtime_control_set_playlist_and_index_emit_protocol_messages() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetPlaylist(vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
        ]))
        .expect("playlist effect should be supported");
    control
        .emit(ClientEffect::SetPlaylistIndex(1))
        .expect("playlist index effect should be supported");

    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control playlist change to emit Set message");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("Set message should contain playlistChange payload");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("expected queued control playlist index to emit Set message");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("Set message should contain playlistIndex payload");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
}

#[test]
fn plex_playlist_sidecar_outbound_keeps_syncplay_files_baseline() {
    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetPlaylist(vec![plex_uri.to_owned()]))
        .expect("playlist effect should be supported");

    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control playlist change to emit Set message");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("Set message should contain playlistChange payload");

    assert_eq!(
        playlist_change.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        playlist_change.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
}

#[test]
fn plex_playlist_sidecar_inbound_restores_canonical_playlist_uri() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistChange":{{"files":["Episode 11 [1080p].mkv"],"user":"alice","sorottePlexPlaylistUris":["{plex_uri}"]}}}}}}"#
        ))
        .expect("playlist sidecar should apply");

    let playlist = session
        .current_room_playlist()
        .expect("playlist should be available");
    assert_eq!(playlist.files, vec![plex_uri.to_owned()]);
}

#[test]
fn playlist_reorder_index_echo_does_not_queue_reset_when_active_target_is_unchanged() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episodeA.mkv","episodeB.mkv","episodeC.mkv","episodeD.mkv"],"user":"bob"}}}"#,
            )
            .expect("initial playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .expect("initial playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "the first playlist index should not queue a reset"
    );

    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episodeD.mkv","episodeA.mkv","episodeB.mkv","episodeC.mkv"],"user":"bob"}}}"#,
            )
            .expect("reordered playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#)
        .expect("reordered playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "playlist reorders that preserve the active target should not queue a rewind/reset intent"
    );
}

#[test]
fn client_runtime_queue_playlist_item_preserves_existing_selection_without_select_after_queue() {
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
            .run_queue_playlist_item("episode3.mkv", false)
            .expect("queue playlist item should not fail"),
        "queueing a playlist item should emit protocol messages"
    );

    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("local queue should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(
        playlist.index,
        Some(0),
        "plain queue should preserve the existing room playlist selection"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_change = playlist_change_message
        .set
        .playlist_change
        .as_ref()
        .expect("playlist change payload should be present");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );

    let ProtocolMessage::Set(playlist_index_message) = &control.outbound_messages()[1] else {
        panic!("expected queued Set playlist index protocol message");
    };
    let playlist_index = playlist_index_message
        .set
        .playlist_index
        .as_ref()
        .expect("playlist index payload should be present");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_replace_playlist_preserves_existing_selection_without_redundant_index_update() {
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
            .run_replace_playlist(
                vec![
                    "episode1.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    "episode3.mkv".to_owned(),
                ],
                Some(0),
            )
            .expect("replace playlist should not fail"),
        "playlist replace should emit a protocol message when the playlist contents change"
    );
    assert_eq!(
        runtime
            .session_mut()
            .take_pending_playlist_index_reset_intent(),
        None,
        "preserving the existing selection during playlist replace should not queue a reset"
    );

    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("local playlist replace should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(
        playlist.index,
        Some(0),
        "playlist replace should preserve the existing room playlist selection"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(
        control.outbound_messages().len(),
        1,
        "playlist replace should omit a redundant playlistIndex update when the selected index is unchanged"
    );
    let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_change = playlist_change_message
        .set
        .playlist_change
        .as_ref()
        .expect("playlist change payload should be present");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert!(
        playlist_change_message.set.playlist_index.is_none(),
        "playlist replace should not send playlistIndex when the selected row is already current"
    );
}

#[test]
fn client_runtime_queue_playlist_item_dispatches_playlist_change_and_preserves_index() {
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
            .run_queue_playlist_item("episode3.mkv", false)
            .expect("queue command should not fail"),
        "queue command should emit playlist change/index updates"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("queue command should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(playlist.index, Some(0));

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound queue message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound queue message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_queue_playlist_item_preserves_whitespace_only_file_name() {
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
            .run_queue_playlist_item(" ", false)
            .expect("queue command should not fail"),
        "queue command should preserve whitespace-only file names"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound queue message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", " "]
    );
    assert!(playlist_change.user.is_none());
}

#[test]
fn client_runtime_queue_and_select_playlist_item_sets_new_item_index() {
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
        "queue-and-select command should emit playlist change/index updates"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound queue-and-select message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 2);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_queue_playlist_item_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_queue_playlist_item("episode1.mkv", false)
            .expect("queue command should not fail"),
        "queue command should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_queue_playlist_item_omits_duplicate_entries() {
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
        !runtime
            .run_queue_playlist_item("episode2.mkv", false)
            .expect("duplicate queue command should not fail"),
        "duplicate queue requests should be suppressed"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("duplicate queue should preserve the current room playlist");
    assert_eq!(playlist.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert_eq!(playlist.index, Some(0));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_delete_playlist_index_dispatches_playlist_change_and_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_delete_playlist_index(1)
            .expect("delete command should not fail"),
        "delete command should emit playlist change/index updates"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound delete message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode3.mkv"]);
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound delete message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_delete_playlist_index_last_item_emits_only_playlist_change() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
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
            .run_delete_playlist_index(0)
            .expect("delete command should not fail"),
        "delete command should emit playlist change for last item removal"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("outbound delete message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("outbound message should include playlistChange");
    assert!(playlist_change.files.is_empty());
    assert!(playlist_change.user.is_none());
}

#[test]
fn client_runtime_delete_playlist_index_is_omitted_for_invalid_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_delete_playlist_index(3)
            .expect("delete command should not fail"),
        "delete command should be suppressed for invalid index"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}
