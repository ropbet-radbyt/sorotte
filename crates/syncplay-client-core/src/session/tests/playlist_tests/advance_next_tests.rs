use super::*;

#[test]
fn client_runtime_advance_playlist_index_dispatches_protocol_message() {
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
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should emit outbound Set.playlistIndex"
    );
    assert!(
        runtime
            .session()
            .recently_advanced(unix_wall_clock_time_seconds_legacy_compatible()),
        "playlist advance should immediately enter the recently-advanced grace window"
    );
    assert_eq!(
        runtime
            .session_mut()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "playlist advance should queue a pause-and-rewind reset intent before the server echo"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("playlist advance should update the current room playlist immediately");
    assert_eq!(playlist.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert_eq!(playlist.index, Some(1));

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_index = set_message
        .set
        .playlist_index
        .as_ref()
        .expect("Set message should contain playlistIndex payload");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
    let ProtocolMessage::State(state_message) = &control.outbound_messages()[1] else {
        panic!("expected queued State protocol message after playlist advance");
    };
    let playstate = state_message
        .state
        .playstate
        .as_ref()
        .expect("reset state should include a playstate payload");
    assert_eq!(playstate.position, Some(0.0));
    assert_eq!(playstate.paused, Some(true));
    assert_eq!(playstate.do_seek, None);
}

#[test]
fn client_runtime_advance_playlist_index_is_omitted_for_untrusted_url_target() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","https://example.com/video.mp4"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should be suppressed for untrusted URL targets"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_advance_playlist_index_is_omitted_without_next_item() {
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
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should be suppressed when no next item exists"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_advance_playlist_index_loops_to_start_when_loop_at_end_enabled() {
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
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode2.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");
    session.behavior_config_mut().loop_at_end_of_playlist = true;

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should loop back to first item when loop-at-end is enabled"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_index = set_message
        .set
        .playlist_index
        .as_ref()
        .expect("Set message should contain playlistIndex payload");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
    let ProtocolMessage::State(state_message) = &control.outbound_messages()[1] else {
        panic!("expected queued State protocol message after playlist loop");
    };
    let playstate = state_message
        .state
        .playstate
        .as_ref()
        .expect("reset state should include a playstate payload");
    assert_eq!(playstate.position, Some(0.0));
    assert_eq!(playstate.paused, Some(true));
    assert_eq!(playstate.do_seek, None);
}

#[test]
fn client_runtime_advance_playlist_index_rewinds_single_music_file_legacy_style() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["song.flac"],"user":"alice"}}}"#)
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"song.flac","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should rewind/unpause for single music playlist entries"
    );
    assert_eq!(runtime.player().position, Some(0.0));
    assert_eq!(runtime.player().paused, Some(false));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_advance_playlist_index_rewinds_single_file_when_loop_single_enabled() {
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
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");
    session.behavior_config_mut().loop_single_files = true;

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_advance_playlist_index()
            .expect("next playlist command should not fail"),
        "next playlist command should rewind/unpause when loop-single-files is enabled"
    );
    assert_eq!(runtime.player().position, Some(0.0));
    assert_eq!(runtime.player().paused, Some(false));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_playlist_next_is_omitted_when_local_file_mismatches_current_index() {
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
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode2.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file update should apply");

    assert!(
        session.runtime_actions_for_local_playlist_next().is_empty(),
        "playlist next should be suppressed until the local player reports the current shared-playlist item"
    );
}
