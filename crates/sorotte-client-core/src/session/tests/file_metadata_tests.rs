use super::*;

#[test]
fn is_playing_music_uses_current_user_file_extension() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["video.mp4","song.FLAC"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    assert!(
        !session.is_playing_music(),
        "playlist selection alone should not enable music-mode behavior before the local file is loaded"
    );

    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"song.FLAC","duration":123.0}}}}}"#,
        )
        .expect("local file update should apply");
    assert!(session.is_playing_music());

    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"video.mp4","duration":95.0}}}}}"#,
        )
        .expect("updated local file should apply");
    assert!(!session.is_playing_music());
}

#[test]
fn same_filename_legacy_like_treats_hidden_filename_as_match() {
    assert!(ClientSession::same_filename_legacy_like(
        PRIVACY_HIDDEN_FILENAME,
        "anything.mkv",
    ));
    assert!(ClientSession::same_filename_legacy_like(
        "anything.mkv",
        PRIVACY_HIDDEN_FILENAME,
    ));
}

#[test]
fn same_filename_legacy_like_matches_url_encoded_and_plain_names() {
    assert!(ClientSession::same_filename_legacy_like(
        "https://example.invalid/media/Movie%20Name.mkv",
        "Movie Name.mkv",
    ));
}

#[test]
fn same_filename_legacy_like_matches_raw_filename_and_hash_form() {
    let raw_name = "Movie Name.mkv";
    let stripped = ClientSession::strip_filename_for_compare(raw_name, false);
    let hashed = ClientSession::hash_filename_for_compare(&stripped);
    assert!(ClientSession::same_filename_legacy_like(raw_name, &hashed));
}

#[test]
fn same_filesize_legacy_like_treats_numeric_zero_as_wildcard() {
    assert!(ClientSession::same_filesize_legacy_like(
        &Value::from(0),
        &Value::from(123_456_789),
    ));
    assert!(ClientSession::same_filesize_legacy_like(
        &Value::from(123_456_789),
        &Value::from(0),
    ));
    assert!(
        !ClientSession::same_filesize_legacy_like(&Value::from("0"), &Value::from(123_456_789),),
        "legacy behavior only treats numeric 0 as wildcard, not string \"0\""
    );
}

#[test]
fn same_filesize_legacy_like_matches_raw_and_hash_forms() {
    let raw_size = Value::from(123_456_789);
    let hashed = Value::from(ClientSession::hash_filesize_for_compare("123456789"));
    assert!(ClientSession::same_filesize_legacy_like(&raw_size, &hashed));
}

#[test]
fn same_fileduration_legacy_like_respects_default_threshold() {
    assert!(
        ClientSession::same_fileduration_legacy_compatible(10.49, 12.49),
        "rounded duration diff of 2 should match with legacy 2.5 threshold"
    );
    assert!(
        !ClientSession::same_fileduration_legacy_compatible(10.49, 13.49),
        "rounded duration diff of 3 should fail with legacy 2.5 threshold"
    );
}

#[test]
fn same_fileduration_legacy_like_uses_python_ties_to_even_rounding() {
    assert!(
        ClientSession::same_fileduration_legacy_compatible(1.5, 4.5),
        "Python round() ties-to-even should yield 2 vs 4 (diff 2), not away-from-zero"
    );
}

#[test]
fn same_fileduration_legacy_like_short_circuits_when_duration_notifications_disabled() {
    assert!(ClientSession::same_fileduration_legacy_like(
        1.0, 999.0, false, 2.5
    ));
}

#[test]
fn same_fileduration_legacy_compatible_with_overrides_respects_toggle_and_threshold() {
    assert!(
        ClientSession::same_fileduration_legacy_compatible_with_overrides(10.49, 13.49, false, 0.1)
    );
    assert!(
        !ClientSession::same_fileduration_legacy_compatible_with_overrides(10.49, 12.49, true, 1.0)
    );
    assert!(
        ClientSession::same_fileduration_legacy_compatible_with_overrides(10.49, 12.49, true, 3.0)
    );
}

#[test]
fn file_differences_for_current_room_detects_all_mismatch_types() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":987654321,"duration":100.0}}}}}"#,
            )
            .expect("peer file should apply");

    let summary = session
        .file_differences_for_current_room()
        .expect("current room file differences should be available");
    assert_eq!(
        summary,
        FileDifferenceSummary {
            filename: true,
            filesize: true,
            fileduration: true,
        }
    );
    assert!(summary.has_differences());
}

#[test]
fn file_differences_for_current_room_respects_duration_override_toggle() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.0}}}}}"#,
            )
            .expect("local file should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":99.0}}}}}"#,
            )
            .expect("peer file should apply");

    let default_summary = session
        .file_differences_for_current_room()
        .expect("file differences should be available with duration mismatch");
    assert_eq!(
        default_summary,
        FileDifferenceSummary {
            filename: false,
            filesize: false,
            fileduration: true,
        }
    );
    assert!(default_summary.has_differences());

    session
        .readiness_autoplay_config_mut()
        .show_duration_notification = false;
    let override_summary = session
        .file_differences_for_current_room()
        .expect("file differences should still be computable");
    assert_eq!(
        override_summary,
        FileDifferenceSummary {
            filename: false,
            filesize: false,
            fileduration: false,
        }
    );
    assert!(!override_summary.has_differences());
}

#[test]
fn file_differences_for_user_skips_out_of_room_and_missing_file_states() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert_eq!(session.file_differences_for_user("bob"), None);

    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"other.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("out-of-room peer should apply");
    assert_eq!(session.file_differences_for_user("bob"), None);

    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("in-room peer should apply");
    assert_eq!(
        session.file_differences_for_user("bob"),
        Some(FileDifferenceSummary {
            filename: true,
            filesize: false,
            fileduration: false,
        })
    );
}

#[test]
fn sanitize_outbound_file_payload_legacy_like_applies_privacy_modes_and_removes_path() {
    let payload = json!({
        "name": "https://example.invalid/media/Movie Name.mkv",
        "size": 123456789,
        "duration": 95.5,
        "path": "C:/media/movie.mkv",
        "extra": "keep-me"
    });

    let raw = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::SendRaw,
        PrivacyMode::SendRaw,
    )
    .expect("raw mode should return sanitized payload");
    assert_eq!(
        raw,
        json!({
            "name": "https://example.invalid/media/Movie Name.mkv",
            "size": 123456789,
            "duration": 95.5,
            "extra": "keep-me"
        })
    );

    let hashed = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::SendHashed,
        PrivacyMode::SendHashed,
    )
    .expect("hashed mode should return sanitized payload");
    assert_eq!(
        hashed,
        json!({
            "name": "a9858cb4803c",
            "size": "15e2b0d3c338",
            "duration": 95.5,
            "extra": "keep-me"
        })
    );

    let hidden = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::DoNotSend,
        PrivacyMode::DoNotSend,
    )
    .expect("hidden mode should return sanitized payload");
    assert_eq!(
        hidden,
        json!({
            "name": PRIVACY_HIDDEN_FILENAME,
            "size": 0,
            "duration": 95.5,
            "extra": "keep-me"
        })
    );
}

#[test]
fn sanitize_outbound_file_payload_legacy_like_supplies_legacy_defaults_for_missing_metadata() {
    let payload = json!({
        "name": "movie.mkv",
        "path": "C:/media/movie.mkv",
        "extra": "keep-me"
    });

    let raw = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::SendRaw,
        PrivacyMode::SendRaw,
    )
    .expect("raw mode should return sanitized payload");
    assert_eq!(
        raw,
        json!({
            "name": "movie.mkv",
            "size": 0,
            "duration": 0.0,
            "extra": "keep-me"
        })
    );

    let hashed = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::SendHashed,
        PrivacyMode::SendHashed,
    )
    .expect("hashed mode should return sanitized payload");
    let hashed_name = ClientSession::filename_with_privacy_mode_legacy_like(
        &json!("movie.mkv"),
        PrivacyMode::SendHashed,
    )
    .expect("hashed filename should be available");
    let hashed_zero_size = ClientSession::hash_filesize_for_compare("0");
    assert_eq!(
        hashed,
        json!({
            "name": hashed_name,
            "size": hashed_zero_size,
            "duration": 0.0,
            "extra": "keep-me"
        })
    );

    let hidden = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
        &payload,
        PrivacyMode::DoNotSend,
        PrivacyMode::DoNotSend,
    )
    .expect("hidden mode should return sanitized payload");
    assert_eq!(
        hidden,
        json!({
            "name": PRIVACY_HIDDEN_FILENAME,
            "size": 0,
            "duration": 0.0,
            "extra": "keep-me"
        })
    );
}

#[test]
fn privacy_mode_from_legacy_name_maps_expected_modes() {
    assert_eq!(
        PrivacyMode::from_legacy_name("SendRaw"),
        Some(PrivacyMode::SendRaw)
    );
    assert_eq!(
        PrivacyMode::from_legacy_name("SendHashed"),
        Some(PrivacyMode::SendHashed)
    );
    assert_eq!(
        PrivacyMode::from_legacy_name("DoNotSend"),
        Some(PrivacyMode::DoNotSend)
    );
    assert_eq!(PrivacyMode::from_legacy_name("unknown"), None);
}

#[test]
fn local_file_publish_runtime_actions_apply_privacy_and_update_local_user_file_view() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let file_payload = json!({
        "name": "https://example.invalid/media/Movie Name.mkv",
        "size": 123456789,
        "duration": 95.5,
        "path": "C:/media/movie.mkv",
        "extra": "keep-me"
    });

    let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
        &file_payload,
        PrivacyMode::SendHashed,
        PrivacyMode::SendHashed,
    );

    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::SetFile {
                file_payload: json!({
                    "name": "a9858cb4803c",
                    "size": "15e2b0d3c338",
                    "duration": 95.5,
                    "extra": "keep-me"
                }),
            },
            ClientRuntimeAction::RequestUserList,
        ]
    );
    assert_eq!(session.user_has_file("alice"), Some(true));
    assert_eq!(session.user_file_name("alice"), Some("a9858cb4803c"));
    assert_eq!(
        session.user_file_size("alice"),
        Some(&json!("15e2b0d3c338"))
    );
    assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));
}

#[test]
fn local_file_publish_empty_payload_clears_local_user_file_view() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("existing local file view should apply");
    assert_eq!(session.user_has_file("alice"), Some(true));

    let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
        &json!({}),
        PrivacyMode::SendRaw,
        PrivacyMode::SendRaw,
    );

    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::SetFile {
                file_payload: json!({}),
            },
            ClientRuntimeAction::RequestUserList,
        ]
    );
    assert_eq!(session.user_has_file("alice"), Some(false));
    assert_eq!(session.user_file_name("alice"), None);
    assert_eq!(session.user_file_size("alice"), None);
    assert_eq!(session.user_file_duration("alice"), None);
}

#[test]
fn client_runtime_publish_local_file_dispatches_sanitized_set_file_message() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .publish_local_file_legacy_compatible(
            &json!({
                "name": "movie.mkv",
                "size": 123456789,
                "duration": 95.5,
                "path": "C:/media/movie.mkv"
            }),
            PrivacyMode::DoNotSend,
            PrivacyMode::DoNotSend,
        )
        .expect("file publish should dispatch");

    let (session, player, control) = runtime.into_parts();
    assert_eq!(player.paused, None);
    assert_eq!(session.user_has_file("alice"), Some(true));
    assert_eq!(
        session.user_file_name("alice"),
        Some(PRIVACY_HIDDEN_FILENAME)
    );
    assert_eq!(session.user_file_size("alice"), Some(&json!(0)));
    assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));

    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.file protocol message");
    };
    let file = set_message
        .set
        .file
        .as_ref()
        .expect("queued message should include file payload");
    assert_eq!(file.name.as_deref(), Some(PRIVACY_HIDDEN_FILENAME));
    assert_eq!(file.duration, Some(95.5));
    assert_eq!(file.size.as_ref(), Some(&json!(0)));
    assert!(file.path.is_none());
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected trailing List request after Set.file");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_publish_pending_local_file_update_dispatches_sanitized_set_file_message() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_local_file_update: Some(
            LocalFileUpdate::new("https://example.invalid/media/Movie Name.mkv")
                .with_duration_seconds(95.5)
                .with_size_bytes(123_456_789)
                .with_path("C:/media/movie.mkv"),
        ),
        ..Default::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let published = runtime
        .publish_pending_local_file_update_legacy_compatible(
            PrivacyMode::SendHashed,
            PrivacyMode::DoNotSend,
        )
        .expect("pending local file update should publish");
    assert!(published);
    let published_again = runtime
        .publish_pending_local_file_update_legacy_compatible(
            PrivacyMode::SendHashed,
            PrivacyMode::DoNotSend,
        )
        .expect("second pending local file update poll should not fail");
    assert!(!published_again);

    let (session, player, control) = runtime.into_parts();
    assert_eq!(player.paused, None);
    assert_eq!(session.user_has_file("alice"), Some(true));
    assert_eq!(session.user_file_name("alice"), Some("a9858cb4803c"));
    assert_eq!(session.user_file_size("alice"), Some(&json!(0)));
    assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));

    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.file protocol message");
    };
    let file = set_message
        .set
        .file
        .as_ref()
        .expect("queued message should include file payload");
    assert_eq!(file.name.as_deref(), Some("a9858cb4803c"));
    assert_eq!(file.duration, Some(95.5));
    assert_eq!(file.size.as_ref(), Some(&json!(0)));
    assert!(file.path.is_none());
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected trailing List request after Set.file");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_publish_pending_local_file_update_without_metadata_uses_legacy_safe_defaults() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_local_file_update: Some(
            LocalFileUpdate::new("movie.mkv").with_path("C:/media/movie.mkv"),
        ),
        ..Default::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let published = runtime
        .publish_pending_local_file_update_legacy_compatible(
            PrivacyMode::SendRaw,
            PrivacyMode::SendHashed,
        )
        .expect("pending local file update should publish");
    assert!(published);

    let (session, player, control) = runtime.into_parts();
    assert_eq!(player.paused, None);
    assert_eq!(session.user_has_file("alice"), Some(true));
    assert_eq!(session.user_file_name("alice"), Some("movie.mkv"));
    assert_eq!(
        session.user_file_size("alice"),
        Some(&json!(ClientSession::hash_filesize_for_compare("0")))
    );
    assert_eq!(session.user_file_duration("alice"), Some(&json!(0.0)));

    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.file protocol message");
    };
    let file = set_message
        .set
        .file
        .as_ref()
        .expect("queued message should include file payload");
    assert_eq!(file.name.as_deref(), Some("movie.mkv"));
    assert_eq!(file.duration, Some(0.0));
    assert_eq!(
        file.size.as_ref(),
        Some(&json!(ClientSession::hash_filesize_for_compare("0")))
    );
    assert!(file.path.is_none());
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected trailing List request after Set.file");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_set_room_with_legacy_fallback_prefers_local_file_name() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv"}}}}}"#,
        )
        .expect("local file metadata should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_room_with_legacy_fallback("fallback-room")
            .expect("set room fallback should not fail"),
        "room fallback should emit outbound Set.room from local file name"
    );
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.room protocol message");
    };
    let room = set_message
        .set
        .room
        .as_ref()
        .expect("Set message should contain room payload");
    assert_eq!(room.name, "movie.mkv");
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message after room fallback");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}
