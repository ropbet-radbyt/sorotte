use super::*;

#[test]
fn playlist_received_before_hello_promotes_its_remote_and_selection_revisions() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode.mkv"],"user":"bob"}}}"#)
        .expect("pre-Hello playlist should be retained");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#)
        .expect("pre-Hello playlist selection should be retained");
    assert_eq!(session.current_room_playlist_remote_revision(), 0);
    assert_eq!(session.current_room_playlist_selection_revision(), None);

    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should promote the pending playlist");
    assert_eq!(
        session.current_room_playlist_remote_revision(),
        1,
        "the promoted playlist must retain its remote generation"
    );
    assert_eq!(
        session.current_room_playlist_selection_revision(),
        Some(1),
        "the promoted index must retain its selection generation"
    );
}

#[test]
fn recently_advanced_tracks_local_playlist_index_updates() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json_at(
            r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#,
            10.0,
        )
        .expect("local playlist index should apply");
    assert!(session.recently_advanced(17.9));
    assert!(!session.recently_advanced(18.1));

    session
        .apply_message_json_at(
            r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#,
            20.0,
        )
        .expect("remote playlist index should apply");
    assert!(!session.recently_advanced(20.1));
}

#[test]
fn recently_advanced_refreshes_when_local_playlist_reset_is_applied() {
    let mut session = ClientSession::default();

    session.begin_local_playlist_index_reset_intent(true, 10.0);
    assert!(session.recently_advanced(17.9));
    assert!(!session.recently_advanced(18.1));

    assert_eq!(
        session.take_pending_playlist_index_reset_intent_at(70.0),
        Some(true)
    );
    assert!(session.recently_advanced(77.9));
    assert!(!session.recently_advanced(78.1));
}

#[test]
fn recently_advanced_does_not_refresh_for_remote_playlist_reset() {
    let mut session = ClientSession::default();

    session.queue_playlist_index_reset_intent(false);
    assert_eq!(
        session.take_pending_playlist_index_reset_intent_at(70.0),
        Some(false)
    );
    assert!(!session.recently_advanced(70.1));
}

#[test]
fn local_playlist_actions_are_suppressed_when_server_shared_playlists_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let _ = session.runtime_actions_for_local_playlist_queue("episode4.mkv".to_owned(), false);
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":false}}}"#,
            )
            .expect("hello should apply");

    assert!(
        session
            .runtime_actions_for_local_playlist_index_set(1)
            .is_empty()
    );
    assert!(session.runtime_actions_for_local_playlist_next().is_empty());
    assert!(
        session
            .runtime_actions_for_local_playlist_queue("episode5.mkv".to_owned(), true)
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_delete(1)
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_replace(
                vec![
                    "episode3.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    "episode1.mkv".to_owned(),
                ],
                Some(2),
            )
            .is_empty()
    );
    assert!(session.runtime_actions_for_local_playlist_undo().is_empty());
    assert!(
        session
            .runtime_actions_for_local_playlist_shuffle_remaining()
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_shuffle_entire()
            .is_empty()
    );
}

#[test]
fn room_switch_ignores_old_room_playlist_index_until_destination_snapshot_arrives() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"}}}}}"#)
        .expect("bob should join room1");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["room1-episode1.mkv","room1-episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("room1 playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("initial room1 playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "the first playlist index in the room should not queue a reset intent"
    );

    let actions = session.runtime_actions_for_local_room_switch("room2".to_owned());
    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::SetRoom {
                room: "room2".to_owned(),
            },
            ClientRuntimeAction::RequestUserList,
        ]
    );
    assert_eq!(
        session
            .model
            .controller
            .pending_local_room_switch_target
            .as_deref(),
        Some("room2"),
        "room switches should mark the destination room while waiting for the server echo"
    );
    assert!(
        !session.model.playlist.received_first_index,
        "room switches should reset playlist-index transition tracking immediately"
    );

    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .expect("late old-room playlist traffic should still apply to bob's room state");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "late old-room playlist traffic should not queue a reset intent"
    );
    assert!(
        !session.model.playlist.received_first_index,
        "late old-room playlist traffic should not consume the first destination playlist index"
    );
    assert_eq!(
        session
            .room_playlist("room1")
            .and_then(|playlist| playlist.index),
        Some(1),
        "old-room playlist state should still update in the background"
    );

    session
        .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room2 echo should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["room2-episode1.mkv","room2-episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("room2 playlist snapshot should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("first room2 playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "the first destination playlist index after a room switch should not queue a reset"
    );

    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("subsequent room2 playlist indexes should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        Some(false),
        "subsequent destination playlist indexes should restore normal reset behavior"
    );
}

#[test]
fn shared_playlist_runtime_actions_are_omitted_after_disconnect() {
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
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session.model.playlist.undo_snapshots.insert(
        "room1".to_owned(),
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
    );

    let _ = session.handle_disconnect(42.0);

    assert!(
        session
            .runtime_actions_for_local_playlist_index_set(2)
            .is_empty(),
        "playlist index changes should be suppressed after disconnect"
    );
    assert!(
        session.runtime_actions_for_local_playlist_next().is_empty(),
        "playlist next should be suppressed after disconnect"
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_queue("episode4.mkv".to_owned(), false)
            .is_empty(),
        "playlist queue should be suppressed after disconnect"
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_delete(1)
            .is_empty(),
        "playlist delete should be suppressed after disconnect"
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_replace(
                vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                Some(0),
            )
            .is_empty(),
        "playlist replace should be suppressed after disconnect"
    );
    assert!(
        session.runtime_actions_for_local_playlist_undo().is_empty(),
        "playlist undo should be suppressed after disconnect"
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_shuffle_remaining()
            .is_empty(),
        "playlist shuffle-remaining should be suppressed after disconnect"
    );
    assert!(
        session
            .runtime_actions_for_local_playlist_shuffle_entire()
            .is_empty(),
        "playlist shuffle-entire should be suppressed after disconnect"
    );
}
