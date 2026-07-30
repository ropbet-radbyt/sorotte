use super::*;
use crate::FileSize;

#[test]
fn connection_phase_transitions_are_explicit_before_hello() {
    let mut session = ClientSession::default();
    assert_eq!(session.connection_phase(), &ConnectionPhase::Disconnected);

    session.mark_connecting();
    assert_eq!(session.connection_phase(), &ConnectionPhase::Connecting);
    session.mark_awaiting_hello();
    assert_eq!(session.connection_phase(), &ConnectionPhase::AwaitingHello);
    session.mark_reconnecting(3);
    assert_eq!(
        session.connection_phase(),
        &ConnectionPhase::Reconnecting { attempt: 3 }
    );
    session.mark_closing();
    assert_eq!(session.connection_phase(), &ConnectionPhase::Closing);
    session.mark_disconnected();
    assert_eq!(session.connection_phase(), &ConnectionPhase::Disconnected);
}

#[test]
fn hello_populates_session_state() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("valid hello should parse");

    assert_eq!(session.model.connection.username.as_deref(), Some("alice"));
    assert_eq!(session.model.room.name.as_deref(), Some("room1"));
    assert!(matches!(
        session.connection_phase(),
        ConnectionPhase::Active(_)
    ));
}

#[test]
fn active_connection_owns_concrete_server_capabilities() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":false,"readiness":true,"setOthersReadiness":false,"sharedPlaylists":true,"managedRooms":false,"mediaMatch":true,"sorottePlexPlaylistUris":true,"persistentRooms":true,"maxUsernameLength":12,"maxRoomNameLength":40,"maxFilenameLength":180}}}"#,
        )
        .expect("hello should apply");

    assert_eq!(
        session.connection_phase(),
        &ConnectionPhase::Active(ServerCapabilities {
            chat: false,
            readiness: true,
            remote_readiness: false,
            shared_playlists: true,
            managed_rooms: false,
            media_match: true,
            plex_playlist_uris: true,
            playback_barrier_v1: false,
            readiness_v2: false,
            persistent_rooms: true,
            max_username_length: 12,
            max_room_name_length: 40,
            max_filename_length: 180,
        })
    );
}

#[test]
fn hello_records_server_readiness_support_flag() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");

    assert!(session.server_readiness_supported());
}

#[test]
fn hello_records_server_chat_support_flag() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");

    assert!(!session.server_chat_supported());
}

#[test]
fn hello_records_persistent_rooms_and_server_limits() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"persistentRooms":true,"maxUsernameLength":12,"maxRoomNameLength":40,"maxFilenameLength":180}}}"#,
            )
            .expect("hello should apply");

    assert!(session.server_persistent_rooms_supported());
    assert_eq!(session.server_max_username_length(), Some(12));
    assert_eq!(session.server_max_room_name_length(), Some(40));
    assert_eq!(session.server_max_filename_length(), Some(180));
}

#[test]
fn hello_without_limit_features_uses_python_compatible_fallbacks() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    assert!(!session.server_persistent_rooms_supported());
    assert_eq!(
        session.server_max_username_length(),
        Some(LEGACY_FALLBACK_MAX_USERNAME_LENGTH)
    );
    assert_eq!(
        session.server_max_room_name_length(),
        Some(LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH)
    );
    assert_eq!(
        session.server_max_filename_length(),
        Some(LEGACY_FALLBACK_MAX_FILENAME_LENGTH)
    );
}

#[test]
fn hello_records_server_shared_playlist_support_flag() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":false}}}"#,
            )
            .expect("hello should apply");

    assert!(!session.server_shared_playlists_supported());
}

#[test]
fn hello_records_server_media_match_support_flag() {
    let mut supported_session = ClientSession::default();
    supported_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"mediaMatch":true}}}"#,
        )
        .expect("hello should apply");
    assert!(supported_session.server_media_match_supported());

    let mut legacy_session = ClientSession::default();
    legacy_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    assert!(!legacy_session.server_media_match_supported());
}

#[test]
fn hello_records_server_managed_rooms_support_flag() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"managedRooms":false}}}"#,
            )
            .expect("hello should apply");

    assert!(!session.server_managed_rooms_supported());
}

#[test]
fn hello_without_features_uses_legacy_version_gate_for_shared_playlist_support() {
    let mut old_server_session = ClientSession::default();
    old_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.255"}}"#,
        )
        .expect("hello should apply");
    assert!(!old_server_session.server_shared_playlists_supported());

    let mut new_server_session = ClientSession::default();
    new_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.4.0"}}"#,
        )
        .expect("hello should apply");
    assert!(new_server_session.server_shared_playlists_supported());
}

#[test]
fn hello_without_features_uses_legacy_version_gate_for_managed_rooms_support() {
    let mut old_server_session = ClientSession::default();
    old_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert!(!old_server_session.server_managed_rooms_supported());

    let mut new_server_session = ClientSession::default();
    new_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.0"}}"#,
        )
        .expect("hello should apply");
    assert!(new_server_session.server_managed_rooms_supported());
}

#[test]
fn hello_without_features_uses_legacy_version_gate_for_readiness_support() {
    let mut old_server_session = ClientSession::default();
    old_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert!(!old_server_session.server_readiness_supported());

    let mut new_server_session = ClientSession::default();
    new_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.0"}}"#,
        )
        .expect("hello should apply");
    assert!(new_server_session.server_readiness_supported());
}

#[test]
fn hello_without_features_uses_legacy_version_gate_for_chat_support() {
    let mut old_server_session = ClientSession::default();
    old_server_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert!(!old_server_session.server_chat_supported());

    let mut feature_list_session = ClientSession::default();
    feature_list_session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    assert!(feature_list_session.server_chat_supported());
}

#[test]
fn hello_without_features_uses_legacy_version_gate_for_set_others_readiness_support() {
    let mut old_server_session = ClientSession::default();
    old_server_session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.1","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    assert!(!old_server_session.server_set_others_readiness_supported());

    let mut new_server_session = ClientSession::default();
    new_server_session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    assert!(new_server_session.server_set_others_readiness_supported());
}

#[test]
fn hello_applies_server_chat_max_message_length_when_enabled() {
    let mut session = ClientSession::default();
    session.chat_config_mut().max_chat_message_length = 150;
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"maxChatMessageLength":12}}}"#,
            )
            .expect("hello should apply");

    assert_eq!(session.chat_config().max_chat_message_length, 12);
}

#[test]
fn hello_without_features_uses_legacy_fallback_chat_max_message_length() {
    let mut session = ClientSession::default();
    session.chat_config_mut().max_chat_message_length = 150;
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    assert_eq!(
        session.chat_config().max_chat_message_length,
        LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH
    );
}

#[test]
fn hello_does_not_override_chat_max_message_length_when_server_sync_disabled() {
    let mut session = ClientSession::default();
    session.chat_config_mut().max_chat_message_length = 23;
    session
        .chat_config_mut()
        .apply_server_max_chat_message_length = false;
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"maxChatMessageLength":12}}}"#,
            )
            .expect("hello should apply");

    assert_eq!(session.chat_config().max_chat_message_length, 23);
}

#[test]
fn client_session_apply_message_json_returns_server_error_payload() {
    let mut session = ClientSession::default();

    let error = session
        .apply_message_json(r#"{"Error":{"message":"wrong-password-server-error"}}"#)
        .expect_err("server error frames should surface to the caller");

    assert!(matches!(
        error,
        ProtocolError::ServerError { message } if message == "wrong-password-server-error"
    ));
}

#[test]
fn client_session_applies_valid_batched_commands_before_unknown_command_error() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let error = session
        .apply_message_json(
            r#"{"Set":{"ready":{"isReady":true,"username":"alice"}},"Bogus":{"x":1}}"#,
        )
        .expect_err("unknown batched command should still surface a protocol error");

    assert!(matches!(error, ProtocolError::InvalidJson(_)));
    assert_eq!(
        session.user_ready("alice"),
        Some(true),
        "valid commands before an unknown batched command should be applied"
    );
}

#[test]
fn client_session_apply_message_json_rejects_unexpected_tls_frames() {
    let mut session = ClientSession::default();

    let error = session
        .apply_message_json(r#"{"TLS":{"startTLS":"false"}}"#)
        .expect_err("unexpected TLS frames should not be ignored by the session");

    assert!(matches!(
        error,
        ProtocolError::UnexpectedTlsMessage { start_tls } if start_tls == "false"
    ));
}

#[test]
fn non_hello_message_is_ignored() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(r#"{"Chat":"hello"}"#)
        .expect_err("chat should not be accepted by hello-only parser");
}

#[test]
fn apply_protocol_message_applies_chat_without_mutating_identity_state() {
    let mut session = ClientSession::default();
    let message = ProtocolMessage::chat_text("hello");
    session
        .apply_protocol_message(message)
        .expect("chat protocol message should apply");
    assert!(session.model.connection.username.is_none());
    assert!(session.model.room.name.is_none());
    assert_eq!(
        session.runtime_actions_for_chat_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyChat(ChatNotification::Message {
            username: None,
            message: "hello".to_owned(),
        })]
    );
    assert!(
        session
            .runtime_actions_for_chat_notifications_if_needed()
            .is_empty(),
        "chat notifications should drain after first retrieval"
    );
}

#[test]
fn list_set_and_state_messages_reconcile_client_view() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello message should apply");

    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"bob.mp4","size":"15e2b0d3c338","duration":95.5,"mediaMatch":{"schema":"sorotte.mediaMatch.v3","profiles":[{"profile":"audio-constellation-v3"}]}},"isReady":true,"features":{"uiMode":"GUI"},"controller":true}}}}"#,
            )
            .expect("set user message should apply");
    assert_eq!(session.user_room("bob"), Some("room2"));
    assert_eq!(session.user_ready("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
    assert_eq!(
        session.user_file_size("bob").map(FileSize::to_json_value),
        Some(json!("15e2b0d3c338"))
    );
    assert_eq!(session.user_file_duration("bob"), Some(95.5));
    assert_eq!(session.user_media_match_signature("bob"), None);
    assert_eq!(
        session
            .user_capabilities("bob")
            .and_then(|capabilities| capabilities.ui_mode.as_deref()),
        Some("GUI")
    );
    assert!(
        session
            .drain_compatibility_fallbacks()
            .iter()
            .any(|fallback| matches!(
                fallback,
                crate::ClientCompatibilityFallback::IgnoredInvalidMediaMatch { .. }
            ))
    );
    assert_eq!(session.user_controller("bob"), Some(true));

    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("set ready message should apply");
    assert_eq!(session.user_ready("alice"), Some(true));

    session
        .apply_message_json(
            r#"{"Set":{"features":{"username":"bob","features":{"chat":true,"readiness":true}}}}"#,
        )
        .expect("set features update should apply");
    let bob_capabilities = session
        .user_capabilities("bob")
        .expect("typed peer capabilities should apply");
    assert!(bob_capabilities.chat);
    assert!(bob_capabilities.readiness);

    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"controller":false}},"room2":{"bob":{"isReady":false,"features":{"uiMode":"desktop"},"controller":true}}}}"#,
            )
            .expect("list snapshot should apply");
    assert_eq!(session.user_room("alice"), Some("room1"));
    assert_eq!(session.user_room("bob"), Some("room2"));
    assert_eq!(session.user_ready("bob"), Some(false));
    assert_eq!(session.user_file_name("bob"), None);
    assert_eq!(session.user_file_size("bob"), None);
    assert_eq!(session.user_file_duration("bob"), None);
    assert_eq!(session.user_media_match_signature("bob"), None);
    assert_eq!(
        session
            .user_capabilities("bob")
            .and_then(|capabilities| capabilities.ui_mode.as_deref()),
        Some("desktop")
    );
    assert_eq!(session.user_controller("bob"), Some(true));

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("state message should apply");
    let playstate = session
        .current_room_playstate()
        .expect("current room playstate should exist");
    assert_eq!(playstate.position, Some(42.0));
    assert_eq!(playstate.paused, Some(false));
    assert_eq!(playstate.do_seek, Some(true));
    assert_eq!(playstate.set_by.as_deref(), Some("alice"));
}

#[test]
fn set_commands_apply_in_wire_order_after_normalization() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"initial"},"version":"1.7.5","features":{}}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"from-user"}}},"room":{"name":"from-room"}}}"#,
        )
        .expect("ordered Set should apply");

    assert_eq!(session.room(), Some("from-room"));
    assert_eq!(session.user_room("alice"), Some("from-room"));
}

#[test]
fn set_command_order_completion_appends_only_missing_canonical_commands() {
    let set = sorotte_protocol::SetPayload::new().with_command_order(vec![
        "ready".to_owned(),
        "vendorExtension".to_owned(),
        "room".to_owned(),
    ]);
    let expected_snapshot = set.clone();

    let ordered = crate::inbound_order::ordered_set_commands(set);
    let command_names = ordered
        .iter()
        .map(|(command, _)| command.as_str())
        .collect::<Vec<_>>();

    // This explicit list is deliberately independent from the production
    // canonical-order table. It proves that wire order is retained first,
    // unknown commands keep their position, present canonical commands are
    // not duplicated, and every missing canonical command is appended.
    assert_eq!(
        command_names,
        [
            "ready",
            "vendorExtension",
            "room",
            "file",
            "user",
            "controllerAuth",
            "newControlledRoom",
            "playlistChange",
            "playlistIndex",
            "features",
            "sorottePlaybackBarrierV1",
            "sorotteReadinessV2",
        ]
    );
    for (_, snapshot) in ordered {
        assert_eq!(snapshot, expected_snapshot);
        assert_eq!(snapshot.command_order, ["ready", "vendorExtension", "room"]);
    }
}

#[test]
fn set_command_order_completion_uses_canonical_order_without_wire_metadata() {
    let ordered = crate::inbound_order::ordered_set_commands(sorotte_protocol::SetPayload::new());

    assert_eq!(
        ordered
            .iter()
            .map(|(command, _)| command.as_str())
            .collect::<Vec<_>>(),
        [
            "room",
            "file",
            "user",
            "controllerAuth",
            "newControlledRoom",
            "ready",
            "playlistChange",
            "playlistIndex",
            "features",
            "sorottePlaybackBarrierV1",
            "sorotteReadinessV2",
        ]
    );
}

#[test]
fn set_command_order_completion_preserves_complete_wire_permutation_exactly() {
    let complete_wire_order = [
        "vendorExtension",
        "sorotteReadinessV2",
        "features",
        "playlistIndex",
        "playlistChange",
        "ready",
        "newControlledRoom",
        "controllerAuth",
        "user",
        "file",
        "room",
        "sorottePlaybackBarrierV1",
    ];
    let set = sorotte_protocol::SetPayload::new().with_command_order(
        complete_wire_order
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
    );

    assert_eq!(
        crate::inbound_order::ordered_set_commands(set)
            .into_iter()
            .map(|(command, _)| command)
            .collect::<Vec<_>>(),
        complete_wire_order
    );
}

#[test]
fn invalid_file_extensions_become_typed_compatibility_fallbacks() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"file":{"name":"movie.mkv","size":{"nested":true},"mediaMatch":{"schema":"unsupported","profiles":[]}}}}}}"#,
        )
        .expect("compatible fields should still apply");

    assert_eq!(session.user_file_name("bob"), Some("movie.mkv"));
    assert_eq!(session.user_file_size("bob"), None);
    assert_eq!(session.user_media_match_signature("bob"), None);
    let fallbacks = session.drain_compatibility_fallbacks();
    assert!(fallbacks.iter().any(|fallback| matches!(
        fallback,
        crate::ClientCompatibilityFallback::IgnoredInvalidFileSize { .. }
    )));
    assert!(fallbacks.iter().any(|fallback| matches!(
        fallback,
        crate::ClientCompatibilityFallback::IgnoredInvalidMediaMatch { .. }
    )));
}

#[test]
fn set_user_left_event_removes_user_from_view() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello message should apply");

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"}}}}}"#)
        .expect("joined user should be tracked");
    assert_eq!(session.user_room("bob"), Some("room1"));

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#)
        .expect("left event should be accepted");
    assert_eq!(session.user_room("bob"), None);
}

#[test]
fn set_user_falsy_file_payload_does_not_clear_existing_file() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","size":123456789,"duration":95.5},"isReady":true}}}}"#,
            )
            .expect("initial user file should apply");
    assert_eq!(session.user_has_file("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
    assert_eq!(
        session.user_file_size("bob").map(FileSize::to_json_value),
        Some(json!(123456789))
    );
    assert_eq!(session.user_file_duration("bob"), Some(95.5));

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{},"isReady":true}}}}"#,
        )
        .expect("falsy file payload should be accepted");
    assert_eq!(session.user_has_file("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
    assert_eq!(
        session.user_file_size("bob").map(FileSize::to_json_value),
        Some(json!(123456789))
    );
    assert_eq!(session.user_file_duration("bob"), Some(95.5));
}

#[test]
fn list_snapshot_file_payload_can_clear_existing_file_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","size":123456789,"duration":95.5},"isReady":true}}}}"#,
            )
            .expect("initial user file should apply");
    assert_eq!(session.user_has_file("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
    assert_eq!(
        session.user_file_size("bob").map(FileSize::to_json_value),
        Some(json!(123456789))
    );
    assert_eq!(session.user_file_duration("bob"), Some(95.5));

    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":true,"file":{}}}}}"#,
            )
            .expect("list snapshot should apply");
    assert_eq!(session.user_has_file("bob"), Some(false));
    assert_eq!(session.user_file_name("bob"), None);
    assert_eq!(session.user_file_size("bob"), None);
    assert_eq!(session.user_file_duration("bob"), None);
}

#[test]
fn list_snapshot_file_payload_tracks_mixed_raw_and_hashed_metadata() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95}},"bob":{"isReady":true,"file":{"name":"movie.mkv","size":123456789,"duration":95.5,"mediaMatch":{"schema":"sorotte.mediaMatch.v3","profiles":[{"profile":"audio-constellation-v3","algorithmVersion":3}]}}}}}}"#,
            )
            .expect("list snapshot with mixed file metadata should apply");

    assert_eq!(session.user_has_file("alice"), Some(true));
    assert_eq!(session.user_file_name("alice"), Some("**Hidden filename**"));
    assert_eq!(
        session.user_file_size("alice").map(FileSize::to_json_value),
        Some(json!("15e2b0d3c338"))
    );
    assert_eq!(session.user_file_duration("alice"), Some(95.0));

    assert_eq!(session.user_has_file("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("movie.mkv"));
    assert_eq!(
        session.user_file_size("bob").map(FileSize::to_json_value),
        Some(json!(123456789))
    );
    assert_eq!(session.user_file_duration("bob"), Some(95.5));
    let signature = session
        .user_media_match_signature("bob")
        .expect("valid media signature should be normalized");
    assert_eq!(
        serde_json::to_value(signature).expect("signature should serialize"),
        json!({
            "schema": "sorotte.mediaMatch.v3",
            "profiles": [{"profile": "audio-constellation-v3", "algorithmVersion": 3, "durationMs": null, "audio": null}]
        })
    );
    assert_eq!(session.current_room_media_match_signatures().len(), 1);
}

#[test]
fn top_level_set_features_defaults_to_local_user_when_username_is_omitted() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(r#"{"Set":{"features":{"chat":true,"managedRooms":true}}}"#)
        .expect("top-level local feature update should apply");

    let capabilities = session
        .user_capabilities("alice")
        .expect("typed local capabilities should apply");
    assert!(capabilities.chat);
    assert!(capabilities.managed_rooms);
}

#[test]
fn top_level_set_file_is_ignored_for_local_user_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert_eq!(session.user_has_file("alice"), Some(false));
    assert_eq!(session.user_file_name("alice"), None);
    assert_eq!(session.user_file_size("alice"), None);
    assert_eq!(session.user_file_duration("alice"), None);

    session
        .apply_message_json(
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789}}}"#,
        )
        .expect("set file should apply");
    assert_eq!(session.user_has_file("alice"), Some(false));
    assert_eq!(session.user_file_name("alice"), None);
    assert_eq!(session.user_file_size("alice"), None);
    assert_eq!(session.user_file_duration("alice"), None);

    session
        .apply_message_json(r#"{"Set":{"file":{}}}"#)
        .expect("empty set file should apply");
    assert_eq!(session.user_has_file("alice"), Some(false));
    assert_eq!(session.user_file_name("alice"), None);
    assert_eq!(session.user_file_size("alice"), None);
    assert_eq!(session.user_file_duration("alice"), None);
}

#[test]
fn inbound_room_playstate_without_do_seek_clears_stale_seek_flag() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("seek state should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":43.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("ordinary state should apply");

    let playstate = session
        .current_room_playstate()
        .expect("room playstate should exist");
    assert_eq!(playstate.position, Some(43.0));
    assert_eq!(playstate.paused, Some(false));
    assert_eq!(
        playstate.do_seek,
        Some(false),
        "ordinary state updates should clear prior doSeek markers instead of leaving them sticky"
    );
}

#[test]
fn list_snapshot_empty_file_payload_does_not_block_readiness_checks() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":false,"file":{}}}}}"#,
            )
            .expect("list snapshot should apply");
    assert_eq!(session.user_has_file("bob"), Some(false));
    assert_eq!(session.user_file_name("bob"), None);
    assert!(
        session.all_other_users_in_current_room_ready(),
        "empty-object file payload should match legacy no-file behavior"
    );

    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":false,"file":{"name":"bob.mp4"}}}}}"#,
            )
            .expect("list snapshot should apply");
    assert_eq!(session.user_has_file("bob"), Some(true));
    assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
    assert!(
        !session.all_other_users_in_current_room_ready(),
        "non-ready users with file metadata should block readiness checks"
    );
}

#[test]
fn forward_compatible_file_presence_is_distinct_from_known_metadata() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/compatibility/file_presence.json"
    ))
    .expect("file-presence compatibility fixture should decode");

    for case in cases
        .as_array()
        .expect("file-presence fixture should be an array")
    {
        let label = case["label"]
            .as_str()
            .expect("file-presence case should have a label");
        let payload = case["payload"].clone();
        let expected_has_file = case["hasFile"]
            .as_bool()
            .expect("file-presence case should have hasFile");
        let expected_name = case["name"].as_str();
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");

        let message = json!({
            "Set": {
                "user": {
                    "bob": {
                        "room": {"name": "room1"},
                        "file": payload
                    }
                }
            }
        });
        session
            .apply_message_json(&message.to_string())
            .unwrap_or_else(|error| panic!("{label} should apply: {error}"));

        assert_eq!(
            session.user_has_file("bob"),
            Some(expected_has_file),
            "wrong presence for {label}"
        );
        assert_eq!(
            session.user_file_name("bob"),
            expected_name,
            "wrong known metadata for {label}"
        );
        if expected_name.is_none() {
            assert_eq!(
                session.user_file_size("bob"),
                None,
                "{label} should not synthesize a known size"
            );
            assert_eq!(
                session.user_file_duration("bob"),
                None,
                "{label} should not synthesize a known duration"
            );
        }
    }
}
