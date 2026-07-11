use super::*;

#[test]
fn handle_disconnect_clears_chat_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    assert!(session.server_chat_supported());

    let _ = session.handle_disconnect(200.0);
    assert_eq!(session.connection_phase(), &ConnectionPhase::Disconnected);
    assert!(!session.server_chat_supported());
}

#[test]
fn chat_config_defaults_include_legacy_max_message_length() {
    let config = ChatConfig::default();
    assert_eq!(
        config.max_chat_message_length,
        LEGACY_CHAT_MAX_MESSAGE_LENGTH
    );
    assert!(config.apply_server_max_chat_message_length);
}

#[test]
fn outbound_chat_message_truncates_to_configured_max_length() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session.chat_config_mut().max_chat_message_length = 5;
    assert_eq!(
        session.runtime_actions_for_outbound_chat_message("hello world".to_owned()),
        vec![ClientRuntimeAction::SendChat {
            message: "hello".to_owned(),
        }]
    );
}

#[test]
fn outbound_chat_message_preserves_empty_payload_legacy_compatible() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    assert_eq!(
        session.runtime_actions_for_outbound_chat_message("".to_owned()),
        vec![ClientRuntimeAction::SendChat {
            message: "".to_owned(),
        }]
    );
    assert_eq!(
        session.runtime_actions_for_outbound_chat_message("\n\r".to_owned()),
        vec![ClientRuntimeAction::SendChat {
            message: "".to_owned(),
        }]
    );
}

#[test]
fn outbound_chat_message_is_omitted_when_max_length_is_zero() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session.chat_config_mut().max_chat_message_length = 0;
    assert!(
        session
            .runtime_actions_for_outbound_chat_message("hello world".to_owned())
            .is_empty()
    );
}

#[test]
fn outbound_chat_message_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    assert!(
        session
            .runtime_actions_for_outbound_chat_message("hello world".to_owned())
            .is_empty()
    );
}

#[test]
fn outbound_chat_message_is_omitted_when_server_version_is_pre_chat_min_without_features() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    assert!(
        session
            .runtime_actions_for_outbound_chat_message("hello world".to_owned())
            .is_empty()
    );
}

#[test]
fn outbound_chat_message_strips_newlines_before_truncation() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session.chat_config_mut().max_chat_message_length = 4;
    assert_eq!(
        session.runtime_actions_for_outbound_chat_message("a\nb\rcd".to_owned()),
        vec![ClientRuntimeAction::SendChat {
            message: "abcd".to_owned(),
        }]
    );
}

#[test]
fn outbound_chat_message_is_omitted_when_server_chat_feature_is_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");
    assert!(
        session
            .runtime_actions_for_outbound_chat_message("hello world".to_owned())
            .is_empty()
    );
}

#[test]
fn client_runtime_chat_notifications_dispatch_from_inbound_chat() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
        .expect("chat should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch");

    assert_eq!(
        runtime.control().chat_notifications(),
        &[ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hello everyone".to_owned(),
        }]
    );
}

#[test]
fn client_runtime_chat_notifications_preserve_mixed_payload_order_across_batches() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(r#"{"Chat":"plain text first"}"#)
        .expect("text chat should apply");
    session
        .apply_message_json(
            r#"{"Chat":{"username":"bob","message":"object payload second","style":"notice"}}"#,
        )
        .expect("object chat with extra fields should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_chat_notifications_if_needed()
        .expect("first batch chat notifications should dispatch");

    assert_eq!(
        runtime.control().chat_notifications(),
        &[
            ChatNotification::Message {
                username: None,
                message: "plain text first".to_owned(),
            },
            ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "object payload second".to_owned(),
            },
        ]
    );

    assert_eq!(
        runtime.drain_chat_notifications(),
        vec![
            ChatNotification::Message {
                username: None,
                message: "plain text first".to_owned(),
            },
            ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "object payload second".to_owned(),
            },
        ]
    );
    assert!(runtime.drain_chat_notifications().is_empty());

    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Chat":{"username":"carol","message":"third batch message"}}"#)
        .expect("later object chat should apply");
    runtime
        .run_chat_notifications_if_needed()
        .expect("second batch chat notifications should dispatch");

    assert_eq!(
        runtime.drain_chat_notifications(),
        vec![ChatNotification::Message {
            username: Some("carol".to_owned()),
            message: "third batch message".to_owned(),
        }]
    );
    assert!(
        runtime
            .session_mut_for_test()
            .runtime_actions_for_chat_notifications_if_needed()
            .is_empty(),
        "chat notification actions should be fully drained after dispatch"
    );
}

#[test]
fn client_runtime_interleaved_user_change_and_chat_notifications_preserve_order_with_independent_drains()
 {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("initial bob join should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .session_mut_for_test()
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"controller":true}}}}"#,
        )
        .expect("bob room switch should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Chat":{"username":"bob","message":"moved to room2"}}"#)
        .expect("bob chat after room switch should apply");

    runtime
        .run_user_change_notifications_if_needed()
        .expect("user change notifications should dispatch");
    assert_eq!(
        runtime.drain_user_change_notifications(),
        vec![UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "room2".to_owned(),
            hide_from_osd: false,
        }],
        "room-switch notification should preserve user-change ordering before chat dispatch in first batch"
    );
    assert!(
        runtime.control().chat_notifications().is_empty(),
        "dispatching user-change notifications should not implicitly dispatch chat notifications"
    );

    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch");
    assert_eq!(
        runtime.drain_chat_notifications(),
        vec![ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "moved to room2".to_owned(),
        }],
        "chat notification should remain pending until chat dispatch runs"
    );
    assert!(runtime.drain_user_change_notifications().is_empty());
    assert!(runtime.drain_chat_notifications().is_empty());

    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Chat":{"username":"bob","message":"still in room2"}}"#)
        .expect("second bob chat should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("bob room switch back should apply");

    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch first in second batch");
    assert_eq!(
        runtime.drain_chat_notifications(),
        vec![ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "still in room2".to_owned(),
        }],
        "chat queue should preserve arrival order when dispatched before user-change notifications"
    );
    assert!(
        runtime.control().user_change_notifications().is_empty(),
        "dispatching chat notifications should not implicitly dispatch user-change notifications"
    );

    runtime
        .run_user_change_notifications_if_needed()
        .expect("user change notifications should dispatch after chat in second batch");
    assert_eq!(
        runtime.drain_user_change_notifications(),
        vec![UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            hide_from_osd: false,
        }],
        "user-change queue should preserve the room-switch notification independently of chat drain order"
    );

    runtime
        .run_chat_notifications_if_needed()
        .expect("repeated chat dispatch should be a no-op after drains");
    runtime
        .run_user_change_notifications_if_needed()
        .expect("repeated user-change dispatch should be a no-op after drains");
    assert!(runtime.drain_chat_notifications().is_empty());
    assert!(runtime.drain_user_change_notifications().is_empty());
}

#[test]
fn client_runtime_send_chat_message_dispatches_protocol_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_send_chat_message("hello room")
            .expect("send chat should dispatch"),
        "non-empty outbound chat should produce a queued send action"
    );

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued outbound message should be Chat");
    };
    assert_eq!(
        chat_message.chat,
        ChatPayload::Text("hello room".to_owned())
    );
}

#[test]
fn client_runtime_send_chat_message_dispatches_empty_protocol_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_send_chat_message("")
            .expect("send chat should dispatch"),
        "empty outbound chat should still produce a queued send action"
    );

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued outbound message should be Chat");
    };
    assert_eq!(chat_message.chat, ChatPayload::Text("".to_owned()));
}

#[test]
fn client_runtime_send_chat_message_does_not_emit_local_chat_notification_before_server_echo() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_send_chat_message("hello room")
            .expect("send chat should dispatch"),
        "outbound chat should be queued"
    );

    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notification dispatch should succeed with no pending notifications");
    assert!(
        runtime.control().chat_notifications().is_empty(),
        "sending local chat should not produce a local notification before server echo"
    );
    assert!(
        runtime.drain_chat_notifications().is_empty(),
        "runtime chat notification queue should stay empty before server echo"
    );

    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Chat":{"username":"alice","message":"hello room"}}"#)
        .expect("server echo chat should apply");
    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch after server echo");

    assert_eq!(
        runtime.drain_chat_notifications(),
        vec![ChatNotification::Message {
            username: Some("alice".to_owned()),
            message: "hello room".to_owned(),
        }],
        "chat notification should appear only after inbound server echo"
    );
}

#[test]
fn client_runtime_send_chat_message_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        !runtime
            .run_send_chat_message("hello room")
            .expect("chat send should not fail"),
        "chat send should be suppressed until server hello is applied"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_send_chat_message_is_omitted_when_server_chat_is_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_send_chat_message("hello room")
            .expect("chat send should not fail"),
        "disabled chat support should suppress outbound chat actions"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_send_chat_message_is_omitted_after_disconnect_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_disconnect(42.0)
        .expect("disconnect should apply pause-on-leave/runtime actions");
    assert!(
        !runtime
            .run_send_chat_message("hello while disconnected")
            .expect("chat send should not fail"),
        "chat send should be suppressed after disconnect until a new hello is applied"
    );
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "suppressed disconnected chat should not enqueue outbound chat payloads"
    );

    runtime
            .session_mut_for_test()
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("reconnect hello should apply");
    assert!(
        runtime
            .run_send_chat_message("hello after reconnect")
            .expect("chat send should not fail"),
        "chat send should resume after reconnect hello"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 1);
}

#[test]
fn client_runtime_player_chat_input_dispatches_protocol_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer {
        pending_chat_requests: std::collections::VecDeque::from([String::from("hello from mpv")]),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert_eq!(
        runtime
            .run_player_chat_input_if_needed()
            .expect("player chat input should dispatch"),
        1
    );

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued outbound message should be Chat");
    };
    assert_eq!(
        chat_message.chat,
        ChatPayload::Text("hello from mpv".to_owned())
    );
}

#[test]
fn client_runtime_player_chat_input_is_suppressed_when_server_chat_is_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer {
        pending_chat_requests: std::collections::VecDeque::from([String::from("hello from mpv")]),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert_eq!(
        runtime
            .run_player_chat_input_if_needed()
            .expect("suppressed player chat input should not fail"),
        0
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_drain_chat_notifications_to_sink_dispatches_callback() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
        .expect("chat should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch");

    let mut captured = Vec::new();
    runtime
        .drain_chat_notifications_to_sink(|notification| {
            captured.push(notification.clone());
            Ok::<(), ()>(())
        })
        .expect("chat notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hello everyone".to_owned(),
        }]
    );
    assert!(runtime.drain_chat_notifications().is_empty());
}
