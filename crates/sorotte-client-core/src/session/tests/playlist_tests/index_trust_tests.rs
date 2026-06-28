use super::*;

#[test]
fn client_runtime_set_playlist_index_dispatches_protocol_message() {
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
            .run_set_playlist_index(1)
            .expect("set playlist index should not fail"),
        "set playlist index should emit outbound Set.playlistIndex"
    );
    assert!(
        runtime
            .session()
            .recently_advanced(unix_wall_clock_time_seconds_legacy_compatible()),
        "local playlist changes should immediately enter the recently-advanced grace window"
    );
    assert_eq!(
        runtime
            .session_mut()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "local playlist changes should queue a pause-and-rewind reset intent before the server echo"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("local playlist selection should update the current room playlist immediately");
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
        panic!("expected queued State protocol message after playlist index change");
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
fn client_runtime_set_playlist_index_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_playlist_index(0)
            .expect("set playlist index should not fail"),
        "set playlist index should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_playlist_index_is_omitted_for_invalid_index() {
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
            .run_set_playlist_index(3)
            .expect("set playlist index should not fail"),
        "set playlist index should be suppressed when index is out of bounds"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_playlist_index_is_omitted_for_untrusted_url_target() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["https://example.com/video.mp4"],"user":"alice"}}}"#,
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
            .run_set_playlist_index(0)
            .expect("set playlist index should not fail"),
        "set playlist index should be suppressed for untrusted URL targets"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn playlist_target_switch_allows_plex_uri_without_trusted_domain() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["plex://abc123machine/metadata/456?title=Example&file=Example.mkv&duration=7200000&type=movie"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session.behavior_config_mut().only_switch_to_trusted_domains = true;
    session.behavior_config_mut().trusted_domains.clear();

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_set_playlist_index(0)
            .expect("set playlist index should not fail"),
        "plex:// playlist targets should not be blocked by web trusted-domain policy"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 2);
}

#[test]
fn client_runtime_set_playlist_index_allows_default_trusted_youtube_domain() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["https://youtube.com/watch?v=abc"],"user":"alice"}}}"#,
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
            .run_set_playlist_index(0)
            .expect("set playlist index should not fail"),
        "set playlist index should allow default trusted domains"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &runtime.control().outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_index = set_message
        .set
        .playlist_index
        .as_ref()
        .expect("Set message should contain playlistIndex payload");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[1] else {
        panic!("expected queued State protocol message after trusted playlist switch");
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
fn client_runtime_trusted_url_matching_supports_wildcard_and_path_prefix() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().trusted_domains = vec!["*.example.com/videos".to_owned()];

    assert!(session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.example.com/clips/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("ftp://cdn.example.com/videos/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://a.b.example.com/videos/a.mp4"));
}

#[test]
fn client_runtime_trusted_url_matching_respects_only_switch_toggle() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().only_switch_to_trusted_domains = false;
    session.behavior_config_mut().trusted_domains.clear();

    assert!(session.uri_is_trusted_legacy_compatible("http://example.com/video.mp4"));
    assert!(session.uri_is_trusted_legacy_compatible("https://example.com/video.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("ftp://example.com/video.mp4"));
}
