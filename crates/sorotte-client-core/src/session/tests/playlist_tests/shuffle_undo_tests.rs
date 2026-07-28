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
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "shuffle-remaining must not reload an unchanged selected row"
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
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "shuffle-entire must finalize its newly selected media before the echo"
    );
    let shuffled_files = runtime
        .session()
        .current_room_playlist()
        .expect("shuffle should project its playlist")
        .files
        .clone();
    let shuffled_files_json =
        serde_json::to_string(&shuffled_files).expect("test playlist should serialize");
    runtime
        .session_mut_for_test()
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistChange":{{"files":{shuffled_files_json},"user":"alice"}}}}}}"#
        ))
        .expect("shuffle playlist echo should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("shuffle index echo should apply");
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "matching shuffle echoes must not create a second media reset"
    );

    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert!(
        !outbound_messages.is_empty(),
        "shuffle entire should emit protocol messages"
    );

    let playlist_index = outbound_messages
        .iter()
        .find_map(|message| match message {
            ProtocolMessage::Set(set) => set.set.playlist_index.as_ref(),
            _ => None,
        })
        .expect("shuffle entire should emit Set.playlistIndex");
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
                r#"{"Set":{"playlistChange":{"files":["episode4.mkv","episode5.mkv"],"user":"alice"}}}"#,
            )
            .expect("updated playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("updated playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        Some(false),
        "the remote playlist update should queue its own reset before the local undo scenario"
    );

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("undo playlist should not fail"),
        "undo playlist should emit restore actions when a previous playlist exists"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "undo must finalize the restored media target before its echo"
    );

    {
        let outbound_messages = runtime.control().outbound_messages();
        assert_eq!(outbound_messages.len(), 3);
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
        assert_eq!(playlist_index.index, 0);
    }

    runtime
            .session_mut_for_test()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("restored playlist echo should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("restored playlist index echo should apply");
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "matching undo echoes must not create a second media reset"
    );

    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("second undo playlist should not fail"),
        "second undo should toggle back to the most recent playlist snapshot"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "toggling back to a different media target must also be finalized"
    );
    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert_eq!(
        outbound_messages.len(),
        6,
        "each undo batch must retain its own trailing State behind the playlist commands it \
         describes"
    );
    assert!(
        matches!(outbound_messages[0], ProtocolMessage::Set(_))
            && matches!(outbound_messages[1], ProtocolMessage::Set(_))
            && matches!(outbound_messages[2], ProtocolMessage::State(_))
            && matches!(outbound_messages[3], ProtocolMessage::Set(_))
            && matches!(outbound_messages[4], ProtocolMessage::Set(_))
            && matches!(outbound_messages[5], ProtocolMessage::State(_)),
        "a later undo batch must not move its playlist commands ahead of the preceding batch's \
         State"
    );
    let playlist_change = outbound_messages
        .iter()
        .filter_map(|message| match message {
            ProtocolMessage::Set(set) => set.set.playlist_change.as_ref(),
            _ => None,
        })
        .find(|playlist_change| playlist_change.files == ["episode4.mkv", "episode5.mkv"])
        .expect("second undo should include its playlistChange message");
    assert_eq!(playlist_change.files, vec!["episode4.mkv", "episode5.mkv"]);
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
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"different-local-file.mkv"}}}}}"#,
        )
        .expect("local file announcement should apply");
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
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "removing the last target must not leave an unconsumable media reset, even when the attached file fallback differs"
    );

    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("second undo playlist should not fail"),
        "second undo should toggle back to the restored playlist without waiting for an echo"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "restoring a target from an empty playlist must finalize that media"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 4);
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
