use super::*;
use crate::PlaybackBarrierRequestScope;
use sorotte_protocol::{
    MediaLoadIntent, PlaybackBarrierPolicy, PrepareMediaPayload, RoomBufferingPolicy,
    RoomBufferingPolicyPayload,
};

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
            password: "AB-123-456".into(),
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
                password: "AB-123-456".into(),
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
            password: "AB-123-456".into(),
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
fn reconnect_drops_playback_barrier_set_but_retains_chat_and_playlist_commands() {
    const PRIVATE_MEDIA_ID: &str = "private-youtube-logical-id";
    let extension = PlaybackBarrierSetExtension::new()
        .with_prepare(PrepareMediaPayload::request(
            7,
            PRIVATE_MEDIA_ID,
            12.0,
            PlaybackBarrierPolicy::Controller,
            MediaLoadIntent::NewPlayback,
        ))
        .with_buffering_policy(
            RoomBufferingPolicyPayload::new(0, RoomBufferingPolicy::Independent)
                .with_request_nonce(7),
        );
    let effect = ClientEffect::send_playback_barrier_set(
        extension.clone(),
        PlaybackBarrierRequestScope::new("room-one", 31, 7),
    );
    assert!(
        !format!("{effect:?}").contains(PRIVATE_MEDIA_ID),
        "effect diagnostics must preserve logical-media redaction"
    );

    let mut control = QueuedRuntimeControl::default();
    control.begin_protocol_connection_generation();
    control.activate_protocol_connection_generation();
    control
        .emit(ClientEffect::SendChat("durable chat".to_owned()))
        .expect("chat should queue");
    control
        .emit(ClientEffect::SetPlaylist(vec![
            "episode-one.mkv".to_owned(),
        ]))
        .expect("playlist change should queue");
    control.emit(effect).expect("barrier Set should queue");

    assert_eq!(control.outbound_messages().len(), 3);
    assert!(matches!(
        control.outbound_messages()[0],
        ProtocolMessage::Chat(_)
    ));
    let ProtocolMessage::Set(playlist) = &control.outbound_messages()[1] else {
        panic!("second durable message should be the playlist Set");
    };
    assert!(playlist.set.playlist_change.is_some());
    let ProtocolMessage::Set(barrier) = &control.outbound_messages()[2] else {
        panic!("third message should be the playback-barrier Set");
    };
    assert_eq!(
        barrier
            .set
            .playback_barrier_v1()
            .expect("queued extension should decode"),
        Some(extension.clone())
    );

    control.begin_protocol_connection_generation();
    assert_eq!(
        control.outbound_messages().len(),
        2,
        "connection replacement must retain only durable chat and playlist commands"
    );
    assert!(matches!(
        control.outbound_messages()[0],
        ProtocolMessage::Chat(_)
    ));
    let ProtocolMessage::Set(retained_playlist) = &control.outbound_messages()[1] else {
        panic!("playlist Set should remain queued after reconnect");
    };
    assert!(retained_playlist.set.playlist_change.is_some());
    assert!(
        retained_playlist
            .set
            .playback_barrier_v1()
            .expect("retained Set extension should decode")
            .is_none(),
        "the stale playback-barrier Set must not cross connection generations"
    );
}

#[test]
fn room_media_and_explicit_cancellation_drop_only_playback_barrier_requests() {
    fn barrier_effect(room: &str, local_media_generation: u64, request_nonce: u64) -> ClientEffect {
        ClientEffect::send_playback_barrier_set(
            PlaybackBarrierSetExtension::new().with_buffering_policy(
                RoomBufferingPolicyPayload::new(0, RoomBufferingPolicy::PauseController)
                    .with_request_nonce(request_nonce),
            ),
            PlaybackBarrierRequestScope::new(room, local_media_generation, request_nonce),
        )
    }

    let mut control = QueuedRuntimeControl::default();
    control.begin_protocol_connection_generation();
    control.activate_protocol_connection_generation();
    control
        .emit(ClientEffect::SendChat("durable".to_owned()))
        .expect("chat should queue");
    control
        .emit(barrier_effect("room-one", 31, 7))
        .expect("barrier should queue");

    control.retain_protocol_playback_barrier_scope("room-one", 31);
    assert_eq!(control.outbound_messages().len(), 2);
    control.retain_protocol_playback_barrier_scope("room-one", 32);
    assert_eq!(control.outbound_messages().len(), 1);
    assert!(matches!(
        control.outbound_messages()[0],
        ProtocolMessage::Chat(_)
    ));

    control
        .emit(barrier_effect("room-one", 32, 8))
        .expect("replacement barrier should queue");
    control
        .emit(ClientEffect::SetRoom("room-two".to_owned()))
        .expect("room change should queue");
    assert_eq!(control.outbound_messages().len(), 2);
    assert!(control.outbound_messages().iter().all(|message| {
        !matches!(message, ProtocolMessage::Set(set) if set
            .set
            .playback_barrier_v1()
            .expect("queued Set extension should decode")
            .is_some())
    }));

    control
        .emit(barrier_effect("room-two", 33, 9))
        .expect("new-room barrier should queue");
    assert_eq!(control.outbound_messages().len(), 3);
    control.cancel_protocol_playback_barrier_requests();
    assert_eq!(control.outbound_messages().len(), 2);
    assert!(matches!(
        control.outbound_messages()[0],
        ProtocolMessage::Chat(_)
    ));
}

#[test]
fn authoritative_inbound_room_change_cancels_queued_playback_barrier_request() {
    let mut session = ClientSession::default();
    session.initialize_local_identity("alice".to_owned(), "room-one".to_owned());
    let mut control = QueuedRuntimeControl::default();
    control.begin_protocol_connection_generation();
    control.activate_protocol_connection_generation();
    control
        .emit(ClientEffect::SendChat("durable".to_owned()))
        .expect("chat should queue");
    control
        .emit(ClientEffect::send_playback_barrier_set(
            PlaybackBarrierSetExtension::new().with_buffering_policy(
                RoomBufferingPolicyPayload::new(0, RoomBufferingPolicy::PauseController)
                    .with_request_nonce(7),
            ),
            PlaybackBarrierRequestScope::new("room-one", 31, 7),
        ))
        .expect("barrier should queue");
    let mut runtime = ClientRuntime::new(session, RecordingPlayer::default(), control);

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"user":{"alice":{"room":{"name":"room-two"}}}}}"#)
        .expect("authoritative room change should apply");

    assert_eq!(runtime.session().room(), Some("room-two"));
    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    assert!(matches!(
        control.outbound_messages()[0],
        ProtocolMessage::Chat(_)
    ));
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
