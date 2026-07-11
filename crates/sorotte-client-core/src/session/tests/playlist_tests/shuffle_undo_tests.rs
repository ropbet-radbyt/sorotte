use super::*;

#[test]
fn client_runtime_shuffle_remaining_playlist_preserves_prefix_and_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv","episode4.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let mut sent = false;
    for _ in 0..4 {
        if runtime
            .run_shuffle_remaining_playlist()
            .expect("shuffle remaining should not fail")
        {
            sent = true;
            break;
        }
    }
    assert!(
        sent,
        "shuffle remaining should eventually emit playlist change/index updates"
    );

    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert_eq!(outbound_messages.len(), 2);

    let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
        panic!("first outbound shuffle-remaining message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        &playlist_change.files[..2],
        &["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    let mut expected_tail = vec!["episode3.mkv".to_owned(), "episode4.mkv".to_owned()];
    let mut actual_tail = playlist_change.files[2..].to_vec();
    expected_tail.sort();
    actual_tail.sort();
    assert_eq!(actual_tail, expected_tail);

    let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
        panic!("second outbound shuffle-remaining message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 1);
}

#[test]
fn client_runtime_shuffle_entire_playlist_resets_index_to_zero() {
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
            .run_shuffle_entire_playlist()
            .expect("shuffle entire should not fail"),
        "shuffle entire should emit at least a playlist index reset"
    );

    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert!(
        !outbound_messages.is_empty(),
        "shuffle entire should emit protocol messages"
    );

    let ProtocolMessage::Set(last_set) = outbound_messages
        .back()
        .expect("shuffle entire should emit at least one Set message")
    else {
        panic!("last outbound message should be Set.playlistIndex");
    };
    let playlist_index = last_set
        .set
        .playlist_index
        .as_ref()
        .expect("last outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 0);
}

#[test]
fn client_runtime_undo_playlist_change_toggles_between_previous_and_current_playlist() {
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
            .expect("initial playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("initial playlist index should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("updated playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("updated playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("undo playlist should not fail"),
        "undo playlist should emit restore actions when a previous playlist exists"
    );

    {
        let outbound_messages = runtime.control().outbound_messages();
        assert_eq!(outbound_messages.len(), 2);
        let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
            panic!("first outbound undo message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound undo message should include playlistChange");
        assert_eq!(
            playlist_change.files,
            vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
        );

        let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
            panic!("second outbound undo message should be Set.playlistIndex");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound undo message should include playlistIndex");
        assert_eq!(playlist_index.index, 2);
    }

    runtime
            .session_mut_for_test()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("restored playlist echo should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
        .expect("restored playlist index echo should apply");

    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("second undo playlist should not fail"),
        "second undo should toggle back to the most recent playlist snapshot"
    );
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 4);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[2] else {
        panic!("first outbound second-undo message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("second undo change message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode3.mkv"]);
}

#[test]
fn client_runtime_undo_playlist_change_toggles_initial_empty_snapshot_without_waiting_for_echo() {
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
        .expect("playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("undo playlist should not fail"),
        "first undo should restore the initial empty snapshot"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[0] else {
        panic!("undo playlist message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("undo playlist message should include playlistChange");
    assert!(playlist_change.files.is_empty());
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("undo should update the current room playlist immediately");
    assert!(playlist.files.is_empty());
    assert_eq!(playlist.index, None);

    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("second undo playlist should not fail"),
        "second undo should toggle back to the restored playlist without waiting for an echo"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 3);
    let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[1] else {
        panic!("second undo change message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("second undo change message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv"]);

    let ProtocolMessage::Set(index_message) = &runtime.control().outbound_messages()[2] else {
        panic!("second undo index message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second undo index message should include playlistIndex");
    assert_eq!(playlist_index.index, 0);
}
