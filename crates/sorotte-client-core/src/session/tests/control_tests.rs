use super::*;

#[test]
fn dispatch_runtime_actions_applies_player_and_control_operations() {
    let actions = vec![
        ClientRuntimeAction::SetPaused(true),
        ClientRuntimeAction::SetRoom {
            room: "room2".to_owned(),
        },
        ClientRuntimeAction::SetPosition(42.0),
        ClientRuntimeAction::SetPlaybackRate(0.95),
        ClientRuntimeAction::SetReady {
            ready: true,
            manually_initiated: false,
        },
        ClientRuntimeAction::SetReadyForUser {
            ready: false,
            manually_initiated: true,
            username: "bob".to_owned(),
        },
        ClientRuntimeAction::SetFile {
            file: protocol_file_payload(json!({"name":"movie.mkv","size":123456789})),
        },
        ClientRuntimeAction::SetPlaylist {
            files: vec!["ep1.mkv".to_owned(), "ep2.mkv".to_owned()],
        },
        ClientRuntimeAction::SetPlaylistIndex { index: 1 },
        ClientRuntimeAction::RequestControllerAuth {
            room: "+room:ABCDEF123456".to_owned(),
            password: "AB-123-456".to_owned(),
        },
        ClientRuntimeAction::SendChat {
            message: "hello room".to_owned(),
        },
        ClientRuntimeAction::NotifyChat(ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hi".to_owned(),
        }),
        ClientRuntimeAction::NotifyControlledRoomCreation(
            ControlledRoomCreationNotification::Created {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB-123-456".to_owned(),
            },
        ),
        ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            },
        ),
        ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Attempting {
                retries: 3,
                delay_seconds: 0.8,
            },
        ),
        ClientRuntimeAction::ScheduleReconnect { delay_seconds: 0.4 },
        ClientRuntimeAction::StopReconnect,
    ];

    let mut player = RecordingPlayer::default();
    let mut control = RecordingRuntimeControl::default();
    ClientSession::dispatch_runtime_actions(&actions, &mut player, &mut control)
        .expect("runtime actions should dispatch cleanly");

    assert_eq!(player.paused, Some(true));
    assert_eq!(player.position, Some(42.0));
    assert_eq!(player.playback_rate, Some(0.95));
    assert_eq!(control.room_updates, vec!["room2".to_owned()]);
    assert_eq!(control.ready_updates, vec![(true, false)]);
    assert_eq!(
        control.ready_for_user_updates,
        vec![(false, true, "bob".to_owned())]
    );
    assert_eq!(
        control.file_updates,
        vec![protocol_file_payload(
            json!({"name":"movie.mkv","size":123456789})
        )]
    );
    assert_eq!(
        control.playlist_updates,
        vec![vec!["ep1.mkv".to_owned(), "ep2.mkv".to_owned()]]
    );
    assert_eq!(control.playlist_index_updates, vec![1]);
    assert_eq!(
        control.controller_auth_requests,
        vec![
            ControllerAuthPayload::new()
                .with_room("+room:ABCDEF123456")
                .with_password("AB-123-456")
        ]
    );
    assert_eq!(control.chat_messages, vec!["hello room".to_owned()]);
    assert_eq!(
        control.chat_notifications,
        vec![ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hi".to_owned(),
        }]
    );
    assert_eq!(
        control.controlled_room_creation_notifications,
        vec![ControlledRoomCreationNotification::Created {
            room: "+room:ABCDEF123456".to_owned(),
            password: "AB-123-456".to_owned(),
        }]
    );
    assert_eq!(
        control.controller_auth_notifications,
        vec![ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned(),
        }]
    );
    assert_eq!(
        control.reconnect_notifications,
        vec![ReconnectTransitionNotification::Attempting {
            retries: 3,
            delay_seconds: 0.8,
        }]
    );
    assert_eq!(control.reconnect_schedules, vec![0.4]);
    assert_eq!(control.stop_reconnect_calls, 1);
}

#[test]
fn dispatch_runtime_actions_stops_on_player_error() {
    let actions = vec![
        ClientRuntimeAction::SetPaused(true),
        ClientRuntimeAction::SetPosition(5.0),
        ClientRuntimeAction::SetReady {
            ready: true,
            manually_initiated: true,
        },
    ];

    let mut player = RecordingPlayer {
        fail_set_position: true,
        ..RecordingPlayer::default()
    };
    let mut control = RecordingRuntimeControl::default();
    let err = ClientSession::dispatch_runtime_actions(&actions, &mut player, &mut control)
        .expect_err("dispatch should bubble player failures");

    assert_eq!(err, PlayerError::Unsupported("set_position_failed"));
    assert_eq!(player.paused, Some(true));
    assert!(control.ready_updates.is_empty());
}

#[test]
fn queued_runtime_control_set_ready_emits_protocol_set_ready_message() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetReady {
            ready: true,
            manually_initiated: false,
        })
        .expect("ready effect should be supported");

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control to emit Set message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(false));
}

#[test]
fn queued_runtime_control_set_room_emits_protocol_set_room_message() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetRoom("room2".to_owned()))
        .expect("room effect should be supported");

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control room change to emit Set message");
    };
    let room = set_message
        .set
        .room
        .as_ref()
        .expect("Set message should contain room payload");
    assert_eq!(room.name, "room2");
}

#[test]
fn queued_runtime_control_set_file_emits_protocol_set_file_message() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetFile(protocol_file_payload(json!({
            "name": "movie.mkv",
            "duration": 95.5,
            "size": 123456789,
            "path": "C:/media/movie.mkv",
            "extra": "keep-me"
        }))))
        .expect("file effect should be supported");

    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control to emit Set message");
    };
    let file = set_message
        .set
        .file
        .as_ref()
        .expect("Set message should contain file payload");
    assert_eq!(file.name.as_deref(), Some("movie.mkv"));
    assert_eq!(file.duration, Some(95.5));
    assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
    assert_eq!(file.path.as_deref(), Some("C:/media/movie.mkv"));
    assert_eq!(file.extra.get("extra"), Some(&json!("keep-me")));
}

#[test]
fn queued_runtime_control_can_drain_encoded_protocol_lines() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetReady {
            ready: true,
            manually_initiated: false,
        })
        .expect("ready effect should be supported");

    let lines = control
        .drain_outbound_message_lines()
        .expect("queued messages should encode");
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("\"Set\""),
        "encoded line should contain Set envelope"
    );
    assert!(
        lines[0].contains("\"isReady\":true"),
        "encoded line should contain ready=true"
    );
    assert!(
        lines[0].contains("\"manuallyInitiated\":false"),
        "encoded line should preserve manuallyInitiated"
    );
    assert!(control.outbound_messages().is_empty());
}

#[test]
fn client_file_effect_rejects_non_object_payload() {
    let error = ClientEffect::set_file_from_value(json!("not-an-object"))
        .expect_err("non-object file payload should be rejected");

    assert!(matches!(error, ClientEffectError::InvalidFilePayload(_)));
}

#[derive(Default)]
struct UnsupportedEffectSink;

impl ClientEffectSink for UnsupportedEffectSink {
    fn emit(&mut self, _effect: ClientEffect) -> Result<(), ClientEffectError> {
        Err(ClientEffectError::Unsupported("set_ready_for_user"))
    }
}

#[test]
fn dispatch_runtime_actions_surfaces_effect_sink_failure() {
    let actions = vec![ClientRuntimeAction::SetReadyForUser {
        ready: true,
        manually_initiated: true,
        username: "bob".to_owned(),
    }];
    let mut player = RecordingPlayer::default();
    let mut control = UnsupportedEffectSink;

    let error = ClientSession::dispatch_runtime_actions(&actions, &mut player, &mut control)
        .expect_err("unsupported client effect should be returned");

    assert_eq!(
        error,
        PlayerError::OperationFailed(
            "client effect is not supported: set_ready_for_user".to_owned()
        )
    );
}
