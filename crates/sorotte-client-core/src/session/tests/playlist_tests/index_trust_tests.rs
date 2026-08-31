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
            .session_mut_for_test()
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
    assert_eq!(playstate.do_seek, Some(true));
}

#[test]
fn omitted_user_current_index_acknowledgement_does_not_queue_second_reset() {
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
        .expect("initial playlist index should apply");

    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    assert!(
        runtime
            .run_set_playlist_index(1)
            .expect("local playlist index change should succeed")
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "the local playlist switch should create its intended reset"
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .suppress_next_self_index_reset,
        "consuming the intended reset should still leave its echo suppression armed"
    );
    assert!(!runtime.session().has_pending_playlist_index_reset_intent());

    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("matching omitted-user index echo should acknowledge the local selection");

    assert!(
        !runtime.session().has_pending_playlist_index_reset_intent(),
        "the current local acknowledgement must not queue a second reset"
    );
    assert!(
        !runtime
            .session()
            .model
            .playlist
            .suppress_next_self_index_reset,
        "the current local acknowledgement should consume reset suppression"
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("playlist should remain projected")
            .index,
        Some(1)
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .pending_local_index_echoes
            .is_empty(),
        "the current acknowledgement should release index echo tracking"
    );
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
    assert_eq!(playstate.do_seek, Some(true));
}

#[test]
fn client_runtime_trusted_url_matching_supports_wildcard_and_path_prefix() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().trusted_domains = vec!["*.example.com/videos".to_owned()];

    assert!(session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos/a.mp4"));
    assert!(session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.example.com/clips/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos-evil/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.example.com//videos/a.mp4"));
    assert!(
        !session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos/../clips/a.mp4")
    );
    assert!(
        !session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos%2Fevil/a.mp4")
    );
    assert!(!session.uri_is_trusted_legacy_compatible("ftp://cdn.example.com/videos/a.mp4"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://a.b.example.com/videos/a.mp4"));
}

#[test]
fn client_runtime_trusted_url_matching_canonicalizes_host_port_and_ipv6() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().trusted_domains = vec![
        "BÜCHER.example/safe".to_owned(),
        "example.test:8443/media".to_owned(),
        "[2001:db8::1]/video".to_owned(),
    ];

    assert!(session.uri_is_trusted_legacy_compatible("https://xn--bcher-kva.example/safe/item"));
    assert!(session.uri_is_trusted_legacy_compatible("https://example.test:8443/media/item"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://example.test:9443/media/item"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://example.test/media/item"));
    assert!(session.uri_is_trusted_legacy_compatible("https://[2001:db8::1]/video/item"));
    assert!(!session.uri_is_trusted_legacy_compatible("https://[2001:db8::2]/video/item"));
}

#[test]
fn client_runtime_trusted_url_matching_preserves_scheme_less_default_ports() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().trusted_domains = vec!["example.com:443".to_owned()];

    assert!(session.uri_is_trusted_legacy_compatible("https://example.com/video"));
    assert!(session.uri_is_trusted_legacy_compatible("https://example.com:443/video"));
    assert!(!session.uri_is_trusted_legacy_compatible("http://example.com/video"));
    assert!(!session.uri_is_trusted_legacy_compatible("http://example.com:80/video"));
    assert!(!session.uri_is_trusted_legacy_compatible("http://example.com:443/video"));
}

#[test]
fn client_runtime_trusted_url_matching_preserves_literal_placeholder_labels() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().trusted_domains = vec![
        "sorotte-wildcard-placeholder.example".to_owned(),
        "*.sorotte-wildcard-placeholder.test".to_owned(),
    ];

    assert!(
        session
            .uri_is_trusted_legacy_compatible("https://sorotte-wildcard-placeholder.example/video")
    );
    assert!(!session.uri_is_trusted_legacy_compatible("https://attacker.example/video"));
    assert!(
        session.uri_is_trusted_legacy_compatible(
            "https://cdn.sorotte-wildcard-placeholder.test/video"
        )
    );
    assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.attacker.test/video"));
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
