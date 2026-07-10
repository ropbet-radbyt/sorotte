use super::*;

#[test]
fn queued_runtime_control_request_controller_auth_emits_protocol_message() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::RequestControllerAuth(
            ControllerAuthPayload::new()
                .with_room("+room:ABCDEF123456")
                .with_password("AB-123-456"),
        ))
        .expect("controller auth effect should be supported");

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued controller auth to emit Set message");
    };
    let controller_auth = set_message
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert!(controller_auth.user.is_none());
    assert!(controller_auth.success.is_none());
}

#[test]
fn client_runtime_controller_reidentify_dispatches_controller_auth_message() {
    let mut session = ClientSession::default();
    session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.3.0"}}"#,
            )
            .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controller_reidentify_if_needed()
        .expect("controller reidentify should dispatch");

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned(),
        }]
    );
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued controller auth Set message");
    };
    let controller_auth = set_message
        .set
        .controller_auth
        .as_ref()
        .expect("queued message should include controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert!(controller_auth.user.is_none());
    assert!(controller_auth.success.is_none());
}

#[test]
fn client_runtime_controller_reidentify_is_omitted_when_managed_rooms_are_unsupported() {
    let mut session = ClientSession::default();
    session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controller_reidentify_if_needed()
        .expect("controller reidentify should not fail when suppressed");

    let (_, _, control) = runtime.into_parts();
    assert!(control.outbound_messages().is_empty());
    assert!(control.controller_auth_notifications().is_empty());
}

#[test]
fn client_runtime_new_controlled_room_dispatches_room_then_controller_auth() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.0"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controller_reidentify_if_needed()
        .expect("controller reidentify should dispatch");

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 3);
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned(),
        }]
    );

    let ProtocolMessage::Set(room_set) = &control.outbound_messages()[0] else {
        panic!("first outbound message should be Set.room");
    };
    let room = room_set
        .set
        .room
        .as_ref()
        .expect("first outbound message should include room payload");
    assert_eq!(room.name, "+room:ABCDEF123456");

    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("second outbound message should be List");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));

    let ProtocolMessage::Set(auth_set) = &control.outbound_messages()[2] else {
        panic!("third outbound message should be Set.controllerAuth");
    };
    let controller_auth = auth_set
        .set
        .controller_auth
        .as_ref()
        .expect("third outbound message should include controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
}

#[test]
fn client_runtime_new_controlled_room_dispatches_creation_notification() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"ab 123 456"}}}"#,
            )
            .expect("new controlled room message should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controlled_room_creation_notifications_if_needed()
        .expect("controlled room creation notifications should dispatch");

    assert_eq!(
        runtime.control().controlled_room_creation_notifications(),
        &[ControlledRoomCreationNotification::Created {
            room: "+room:ABCDEF123456".to_owned(),
            password: "AB123456".to_owned(),
        }]
    );
}

#[test]
fn client_runtime_controller_auth_outcome_notifications_dispatch_from_inbound_set() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("controller auth notifications should dispatch");

    assert_eq!(
        runtime.control().controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Succeeded {
            username: "alice".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: false,
        }]
    );
}

#[test]
fn client_runtime_request_controller_auth_dispatches_protocol_message_with_normalized_password() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_request_controller_auth(" +room:ABCDEF123456 ", "ab_123-456!")
            .expect("controller auth request should not fail"),
        "manual controller auth request should emit outbound Set.controllerAuth after hello"
    );
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.controllerAuth protocol message");
    };
    let controller_auth = set_message
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(
        controller_auth.room.as_deref(),
        Some(" +room:ABCDEF123456 ")
    );
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB123-456")
    );
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: " +room:ABCDEF123456 ".to_owned()
        }]
    );
}

#[test]
fn client_runtime_request_controller_auth_is_omitted_when_managed_rooms_are_unsupported() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"managedRooms":false}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_request_controller_auth(" +room:ABCDEF123456 ", "ab_123-456!")
            .expect("controller auth request should not fail when suppressed"),
        "manual controller auth request should be suppressed when managedRooms is disabled"
    );
    let (_, _, control) = runtime.into_parts();
    assert!(control.outbound_messages().is_empty());
    assert!(control.controller_auth_notifications().is_empty());
}

#[test]
fn client_runtime_request_controller_auth_without_password_dispatches_empty_password_payload() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_request_controller_auth(" +room:ABCDEF123456 ", "   ")
            .expect("controller auth request should not fail"),
        "manual controller auth request should emit outbound Set.controllerAuth even with empty password after normalization"
    );
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.controllerAuth protocol message");
    };
    let controller_auth = set_message
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(
        controller_auth.room.as_deref(),
        Some(" +room:ABCDEF123456 ")
    );
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("")
    );
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: " +room:ABCDEF123456 ".to_owned()
        }]
    );
}

#[test]
fn client_runtime_request_controller_auth_dispatches_for_whitespace_only_room() {
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
            .run_request_controller_auth(" ", "AB-123-456")
            .expect("controller auth request should not fail"),
        "controller auth request should preserve whitespace-only room names"
    );
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.controllerAuth protocol message");
    };
    let controller_auth = set_message
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some(" "));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: " ".to_owned()
        }]
    );
}

#[test]
fn client_runtime_request_controller_auth_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_request_controller_auth("+room:ABCDEF123456", "AB-123-456")
            .expect("controller auth request should not fail"),
        "manual controller auth request should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
    assert!(runtime.control().controller_auth_notifications().is_empty());
}

#[test]
fn client_runtime_set_room_reidentifies_controlled_room_with_stored_password() {
    let mut session = ClientSession::default();
    session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"managedRooms":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_room("+room:ABCDEF123456")
            .expect("set room should not fail"),
        "room switch should emit outbound room/list/controller-auth dispatches"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 3);

    let ProtocolMessage::Set(room_set) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.room protocol message");
    };
    assert_eq!(
        room_set.set.room.as_ref().map(|room| room.name.as_str()),
        Some("+room:ABCDEF123456")
    );

    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));

    let ProtocolMessage::Set(auth_set) = &control.outbound_messages()[2] else {
        panic!("expected queued Set.controllerAuth protocol message");
    };
    let controller_auth = auth_set
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned()
        }]
    );
}

#[test]
fn client_runtime_set_room_with_inline_controlled_room_password_canonicalizes_and_reidentifies() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"managedRooms":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_room("+room:ABCDEF123456:ab-123-456")
            .expect("set room should not fail"),
        "inline controlled-room password should canonicalize the room and queue reidentify actions"
    );

    let (session, _, control) = runtime.into_parts();
    assert_eq!(
        session.controlled_room_passwords.get("+room:ABCDEF123456"),
        Some(&"AB-123-456".to_owned()),
        "inline controlled-room password should be cached for future reidentify flows"
    );
    assert_eq!(control.outbound_messages().len(), 3);

    let ProtocolMessage::Set(room_set) = &control.outbound_messages()[0] else {
        panic!("expected queued Set.room protocol message");
    };
    assert_eq!(
        room_set.set.room.as_ref().map(|room| room.name.as_str()),
        Some("+room:ABCDEF123456"),
        "outbound room switch should strip the inline password before sending Set.room"
    );

    let ProtocolMessage::List(list_message) = &control.outbound_messages()[1] else {
        panic!("expected queued List protocol message");
    };
    assert!(matches!(list_message.list, ListPayload::Request(_)));

    let ProtocolMessage::Set(auth_set) = &control.outbound_messages()[2] else {
        panic!("expected queued Set.controllerAuth protocol message");
    };
    let controller_auth = auth_set
        .set
        .controller_auth
        .as_ref()
        .expect("Set message should contain controllerAuth payload");
    assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(
        controller_auth
            .password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    assert_eq!(
        control.controller_auth_notifications(),
        &[ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned()
        }]
    );
}

#[test]
fn client_runtime_noncontroller_pause_toggle_suppresses_ready_flip_while_recently_rewound() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
    session.local_paused = Some(true);
    session.last_rewound_at_seconds = Some(unix_wall_clock_time_seconds_legacy_compatible());
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote unpaused room state should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_set_paused(false)
            .expect("non-controller unpause attempt should not fail"),
        "non-controller unpause attempt should still issue the player unpause"
    );

    assert_eq!(runtime.player().paused, Some(false));
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "a recent rewind should suppress the non-controller ready toggle"
    );
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(false),
        "recent-rewind suppression should leave the local ready state unchanged"
    );
}

#[test]
fn client_runtime_drain_controller_auth_notifications_to_sink_dispatches_callback() {
    let mut session = ClientSession::default();
    session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.3.0"}}"#,
            )
            .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controller_reidentify_if_needed()
        .expect("controller reidentify should dispatch");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("controller auth notifications should dispatch");

    let mut captured = Vec::new();
    runtime
        .drain_controller_auth_notifications_to_sink(|notification| {
            captured.push(notification.clone());
            Ok::<(), ()>(())
        })
        .expect("controller auth notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![
            ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            },
            ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
        ]
    );
    assert!(runtime.drain_controller_auth_notifications().is_empty());
}

#[test]
fn client_runtime_drain_controlled_room_creation_notifications_to_sink_dispatches_callback() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"ab 123 456"}}}"#,
            )
            .expect("new controlled room message should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_controlled_room_creation_notifications_if_needed()
        .expect("controlled room creation notifications should dispatch");

    let mut captured = Vec::new();
    runtime
        .drain_controlled_room_creation_notifications_to_sink(|notification| {
            captured.push(notification.clone());
            Ok::<(), ()>(())
        })
        .expect("controlled room creation notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![ControlledRoomCreationNotification::Created {
            room: "+room:ABCDEF123456".to_owned(),
            password: "AB123456".to_owned(),
        }]
    );
    assert!(
        runtime
            .drain_controlled_room_creation_notifications()
            .is_empty()
    );
}
