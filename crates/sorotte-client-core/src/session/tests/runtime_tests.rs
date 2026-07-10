use super::*;

#[test]
fn client_runtime_local_media_open_dispatches_not_ready_protocol_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_local_media_opened_not_ready()
            .expect("local media open should dispatch readiness")
    );
    assert_eq!(runtime.session().user_ready("alice"), Some(false));

    let (_, _player, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued local-media-open action to emit Set message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should include ready payload");
    assert_eq!(ready.is_ready, Some(false));
    assert_eq!(ready.manually_initiated, Some(false));
}

#[test]
fn client_runtime_user_change_notifications_dispatch_from_inbound_set() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#)
        .expect("user join should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_user_change_notifications_if_needed()
        .expect("user change notifications should dispatch");

    assert_eq!(
        runtime.control().user_change_notifications(),
        &[UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: true,
        }]
    );
}

#[test]
fn client_runtime_toggle_ready_dispatches_protocol_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_toggle_ready(true)
            .expect("toggle ready should not fail"),
        "toggle ready should emit outbound Set.ready after hello"
    );
    let (_, _, control) = runtime.into_parts();

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(true));
}

#[test]
fn client_runtime_toggle_ready_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_toggle_ready(true)
            .expect("toggle ready should not fail"),
        "toggle ready should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_ready_for_user_dispatches_protocol_message_with_username() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_ready_for_user("bob", true, true)
            .expect("set ready for user should not fail"),
        "set ready for user should emit outbound Set.ready after hello"
    );
    let (_, _, control) = runtime.into_parts();

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(true));
    assert_eq!(ready.username.as_deref(), Some("bob"));
}

#[test]
fn client_runtime_set_ready_for_user_without_username_dispatches_local_ready_message() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_ready_for_user("", true, true)
            .expect("set ready without username should not fail"),
        "set ready without username should emit local Set.ready after hello"
    );
    let (_, _, control) = runtime.into_parts();

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(true));
    assert!(
        ready.username.is_none(),
        "local ready set should omit username payload field"
    );
}

#[test]
fn client_runtime_set_ready_for_explicit_local_username_dispatches_username_payload() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_ready_for_user("alice", false, true)
            .expect("set ready for explicit local username should not fail"),
        "set ready for explicit local username should emit outbound Set.ready with username"
    );
    let (_, _, control) = runtime.into_parts();

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(false));
    assert_eq!(ready.manually_initiated, Some(true));
    assert_eq!(ready.username.as_deref(), Some("alice"));
}

#[test]
fn client_runtime_set_ready_for_whitespace_username_preserves_payload() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_ready_for_user(" ", true, true)
            .expect("set ready for whitespace username should not fail"),
        "set ready for whitespace username should emit outbound Set.ready with username"
    );
    let (_, _, control) = runtime.into_parts();

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(true));
    assert_eq!(ready.username.as_deref(), Some(" "));
}

#[test]
fn client_runtime_set_ready_for_user_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_ready_for_user("bob", true, true)
            .expect("set ready for user should not fail"),
        "set ready for user should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_ready_for_user_is_omitted_without_local_room_control() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"readiness":true,"setOthersReadiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_ready_for_user("bob", true, true)
            .expect("set ready for other user should not fail"),
        "set ready for other user should be suppressed when the local user cannot control the current room"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_room_dispatches_protocol_message() {
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
            .run_set_room("  room2  ")
            .expect("set room should not fail"),
        "set room should emit outbound Set.room after hello"
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
    assert_eq!(room.name, "  room2  ");
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message after room switch");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_set_room_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_room("room2")
            .expect("set room should not fail"),
        "set room should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_room_is_omitted_when_target_is_empty() {
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
        !runtime.run_set_room("").expect("set room should not fail"),
        "empty room switch should be ignored"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_room_dispatches_when_target_is_whitespace_only() {
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
            .run_set_room("   ")
            .expect("set room should not fail"),
        "whitespace-only room switch should still emit outbound Set.room"
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
    assert_eq!(room.name, "   ");
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message after whitespace-only room switch");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_set_room_dispatches_even_when_target_is_unchanged() {
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
            .run_set_room("room1")
            .expect("set room should not fail"),
        "unchanged room switch should still emit outbound Set.room"
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
    assert_eq!(room.name, "room1");
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message after unchanged room switch");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_set_room_with_legacy_fallback_uses_default_when_no_file() {
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
            .run_set_room_with_legacy_fallback("fallback-room")
            .expect("set room fallback should not fail"),
        "room fallback should emit outbound Set.room from default room"
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
    assert_eq!(room.name, "fallback-room");
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message after room fallback");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_request_user_list_dispatches_protocol_message() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_request_user_list()
            .expect("list request should not fail"),
        "list request should emit outbound List request after hello"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::List(list_message) = &control.outbound_messages()[0] else {
        panic!("expected queued List protocol message");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));
}

#[test]
fn client_runtime_request_user_list_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_request_user_list()
            .expect("list request should not fail"),
        "list request should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_request_user_list_is_omitted_after_disconnect_until_next_hello() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session.behavior_config_mut().pause_on_leave = false;

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_disconnect(42.0)
        .expect("disconnect should not fail");

    assert!(
        !runtime
            .run_request_user_list()
            .expect("list request should not fail"),
        "list request should be suppressed after disconnect until the next hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_run_disconnect_applies_pause_on_leave_action() {
    let session = ClientSession {
        local_paused: Some(false),
        ..ClientSession::default()
    };
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_disconnect(42.0)
        .expect("disconnect handling should dispatch pause action");

    let (session, player, control) = runtime.into_parts();
    assert_eq!(session.last_paused_on_leave_at_seconds(), Some(42.0));
    assert_eq!(player.paused, Some(true));
    assert!(
        control.outbound_messages().is_empty(),
        "disconnect handling should not queue outbound protocol messages"
    );
}

#[test]
fn client_runtime_flush_queued_protocol_lines_to_transport_uses_sender_callback() {
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
        .run_readiness_unpause_attempt(10.0, true, true, false)
        .expect("readiness attempt should dispatch");

    let mut sent_lines = Vec::new();
    runtime
        .flush_queued_protocol_lines_to_transport(|line| {
            sent_lines.push(line.to_owned());
            Ok(())
        })
        .expect("transport sender callback should be invoked per line");

    assert_eq!(sent_lines.len(), 1);
    assert!(sent_lines[0].contains("\"Set\""));
    assert!(sent_lines[0].contains("\"isReady\":true"));
    assert!(runtime.flush_queued_protocol_messages().is_empty());
}

#[test]
fn client_runtime_protocol_transport_failure_preserves_failed_message_and_tail() {
    let mut control = QueuedRuntimeControl::default();
    control.send_chat("first".to_owned());
    control.send_chat("second".to_owned());
    control.send_chat("third".to_owned());
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        RecordingPlayer::default(),
        control,
    );
    let mut first_attempt = Vec::new();

    let error = runtime
        .flush_queued_protocol_lines_to_transport(|line| {
            first_attempt.push(line.to_owned());
            if first_attempt.len() == 2 {
                Err(ProtocolError::ServerError {
                    message: "transport failed".to_owned(),
                })
            } else {
                Ok(())
            }
        })
        .expect_err("second transport send should fail");

    assert!(matches!(
        error,
        ProtocolError::ServerError { message } if message == "transport failed"
    ));
    assert_eq!(first_attempt.len(), 2);
    assert_eq!(runtime.control().outbound_messages().len(), 2);

    let mut retry = Vec::new();
    runtime
        .flush_queued_protocol_lines_to_transport(|line| {
            retry.push(line.to_owned());
            Ok(())
        })
        .expect("failed message and tail should be retryable");

    assert_eq!(retry.len(), 2);
    assert!(retry[0].contains("second"));
    assert!(retry[1].contains("third"));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_drain_user_change_notifications_to_sink_dispatches_callback() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#)
        .expect("user join should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_user_change_notifications_if_needed()
        .expect("user change notifications should dispatch");

    let mut captured = Vec::new();
    runtime
        .drain_user_change_notifications_to_sink(|notification| {
            captured.push(notification.clone());
            Ok::<(), ()>(())
        })
        .expect("user change notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: true,
        }]
    );
    assert!(runtime.drain_user_change_notifications().is_empty());
}

#[test]
fn client_runtime_notification_sink_failure_preserves_failed_notification_and_tail() {
    let notifications = ["first", "second", "third"].map(|message| ChatNotification::Message {
        username: Some("alice".to_owned()),
        message: message.to_owned(),
    });
    let mut control = QueuedRuntimeControl::default();
    for notification in notifications.clone() {
        control.notify_chat(notification);
    }
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        RecordingPlayer::default(),
        control,
    );
    let mut attempted = Vec::new();

    let result = runtime.drain_chat_notifications_to_sink(|notification| {
        attempted.push(notification.clone());
        if attempted.len() == 2 {
            Err("notification sink failed")
        } else {
            Ok(())
        }
    });

    assert_eq!(result, Err("notification sink failed"));
    assert_eq!(attempted, notifications[..2]);
    assert_eq!(
        runtime.drain_chat_notifications(),
        notifications[1..].to_vec(),
        "the failed notification and unattempted tail must remain queued"
    );
}

#[test]
fn client_runtime_drain_player_playback_telemetry_updates_to_sink_dispatches_callback() {
    let session = ClientSession::default();
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(12.5)
                .with_playback_rate(0.95),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let mut captured = Vec::new();
    runtime
        .drain_player_playback_telemetry_updates_to_sink(|update| {
            captured.push(update.clone());
            Ok::<(), ()>(())
        })
        .expect("playback telemetry sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![PlayerPlaybackTelemetryUpdate {
            paused: Some(true),
            position_seconds: Some(12.5),
            playback_rate: Some(0.95),
            paused_for_cache: None,
            cache_buffering_percent: None,
        }]
    );
    assert!(runtime.drain_player_playback_telemetry_updates().is_empty());
}

#[test]
fn client_runtime_coalesces_pending_playback_telemetry_to_latest_values() {
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(10.0),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        player,
        QueuedRuntimeControl::default(),
    );
    runtime.sync_player_playback_telemetry_into_session_and_buffer();
    runtime.player_mut().pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(20.0)
            .with_playback_rate(1.25),
    );
    runtime.sync_player_playback_telemetry_into_session_and_buffer();

    assert_eq!(
        runtime.drain_player_playback_telemetry_updates(),
        vec![PlayerPlaybackTelemetryUpdate {
            paused: Some(true),
            position_seconds: Some(20.0),
            playback_rate: Some(1.25),
            paused_for_cache: None,
            cache_buffering_percent: None,
        }]
    );
}

#[test]
fn client_runtime_playback_telemetry_sink_failure_preserves_latest_update() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5);
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(update.clone()),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        player,
        QueuedRuntimeControl::default(),
    );

    let result = runtime
        .drain_player_playback_telemetry_updates_to_sink(|_| Err::<(), _>("telemetry sink failed"));

    assert_eq!(result, Err("telemetry sink failed"));
    assert_eq!(
        runtime.drain_player_playback_telemetry_updates(),
        vec![update]
    );
}

#[test]
fn client_runtime_drain_player_playback_telemetry_updates_refreshes_local_state() {
    let mut session = ClientSession::default();
    session.local_paused = Some(true);
    session.local_position = Some(1.0);
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(12.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let updates = runtime.drain_player_playback_telemetry_updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(runtime.session().local_paused, Some(false));
    assert_eq!(runtime.session().local_position, Some(12.5));

    assert!(
        runtime
            .run_toggle_pause()
            .expect("toggle pause should use telemetry-refreshed local paused state"),
        "toggle pause should emit a local SetPaused action"
    );
    assert_eq!(
        runtime.player().paused,
        Some(true),
        "toggle should invert the telemetry-confirmed paused=false state"
    );
}
