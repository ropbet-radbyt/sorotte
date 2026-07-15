use super::*;

#[test]
fn controller_auth_transition_notification_message_uses_legacy_style_wording() {
    assert_eq!(
        controller_auth_transition_notification_message(
            &ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }
        ),
        "Identifying as room operator in room +room:ABCDEF123456..."
    );
    assert_eq!(
        controller_auth_transition_notification_message(
            &ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            }
        ),
        "alice authenticated as a room operator in room +room:ABCDEF123456"
    );
    assert_eq!(
        controller_auth_transition_notification_message(
            &ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            }
        ),
        "alice failed to identify as a room operator in room +room:ABCDEF123456"
    );
}

#[test]
fn controller_auth_transition_notification_message_localized_legacy_compatible_localizes_common_runtime_notifications()
 {
    assert_eq!(
        crate::controller_auth_transition_notification_message_localized_legacy_compatible(
            &ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            },
            Some("fr"),
        ),
        "Identification comme operateur de salle dans la salle +room:ABCDEF123456..."
    );
    assert_eq!(
        crate::controller_auth_transition_notification_message_localized_legacy_compatible(
            &ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
            Some("de"),
        ),
        "alice wurde als Raumoperator in Raum +room:ABCDEF123456 authentifiziert"
    );
    assert_eq!(
        crate::controller_auth_transition_notification_message_localized_legacy_compatible(
            &ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            },
            None,
        ),
        "alice failed to identify as a room operator in room +room:ABCDEF123456"
    );
}

#[test]
fn controller_auth_notification_hidden_from_osd_uses_visibility_metadata() {
    assert!(
        !controller_auth_notification_hidden_from_osd(
            &ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }
        ),
        "attempt notification should never be hidden by OSD visibility metadata"
    );
    assert!(controller_auth_notification_hidden_from_osd(
        &ControllerAuthTransitionNotification::Succeeded {
            username: "alice".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: true,
        }
    ));
    assert!(!controller_auth_notification_hidden_from_osd(
        &ControllerAuthTransitionNotification::Failed {
            username: "alice".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: false,
        }
    ));
}

#[test]
fn chat_notification_message_formats_username_and_plain_text_payloads() {
    assert_eq!(
        chat_notification_message(&ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hello everyone".to_owned(),
        }),
        "<bob> hello everyone"
    );
    assert_eq!(
        chat_notification_message(&ChatNotification::Message {
            username: None,
            message: "server broadcast".to_owned(),
        }),
        "server broadcast"
    );
}

#[test]
fn chat_notification_message_preserves_whitespace_and_senderless_legacy_formatting() {
    assert_eq!(
        chat_notification_message(&ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "  padded message  ".to_owned(),
        }),
        "<bob>   padded message  "
    );
    assert_eq!(
        chat_notification_message(&ChatNotification::Message {
            username: None,
            message: "  system notice  ".to_owned(),
        }),
        "  system notice  "
    );
}

#[test]
fn playlist_index_in_bounds_legacy_compatible_checks_current_room_bounds() {
    let mut session = ClientSession::default();
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, 0));
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, -1));

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

    assert!(playlist_index_in_bounds_legacy_compatible(&session, 0));
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, 1));
}

#[test]
fn run_planned_local_runtime_action_legacy_compatible_suppresses_out_of_range_playlist_and_dispatches_in_range()
 {
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

    let player = MpvAdapter::default();
    let control = QueuedRuntimeControl::default();
    let runtime = ClientRuntime::new(session, player, control);
    let mut runtime = ClientApplication::from_runtime(runtime);
    let mut user_offset_seconds = 0.0;

    assert!(
        !run_planned_local_runtime_action_legacy_compatible(
            &mut runtime,
            &mut user_offset_seconds,
            1.0,
            PlannedLocalRuntimeAction::SetPlaylistIndex(3),
        )
        .expect("out-of-range select should not fail"),
        "out-of-range select should be suppressed with legacy local error"
    );
    assert!(
        !run_planned_local_runtime_action_legacy_compatible(
            &mut runtime,
            &mut user_offset_seconds,
            1.0,
            PlannedLocalRuntimeAction::DeletePlaylistIndex(3),
        )
        .expect("out-of-range delete should not fail"),
        "out-of-range delete should be suppressed with legacy local error"
    );
    assert_eq!(
        runtime.pending_protocol_messages().len(),
        0,
        "out-of-range playlist commands should not emit outbound protocol messages"
    );

    assert!(
        run_planned_local_runtime_action_legacy_compatible(
            &mut runtime,
            &mut user_offset_seconds,
            1.0,
            PlannedLocalRuntimeAction::SetPlaylistIndex(0),
        )
        .expect("in-range select should not fail"),
        "in-range select should dispatch protocol updates"
    );
    assert_eq!(
        runtime.pending_protocol_messages().len(),
        2,
        "in-range select should emit the paused-at-zero reset state and the playlist index update"
    );
    assert!(
        runtime.pending_protocol_messages().iter().any(|message| {
            matches!(
                message,
                ProtocolMessage::Set(payload)
                    if payload
                        .set
                        .playlist_index
                        .as_ref()
                        .is_some_and(|index| index.index == 0)
            )
        }),
        "in-range select should still emit a playlist index update"
    );
}

#[test]
fn explicit_play_and_pause_actions_set_state_while_p_remains_a_toggle() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":false}}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"setBy":"alice"}}}"#,
        )
        .expect("playing room state should apply");
    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default().with_paused(false),
    );
    let player = sorotte_player_mpv::SimulatedPlayer::new().into_inner();
    let control = QueuedRuntimeControl::default();
    let runtime = ClientRuntime::new(session, player, control);
    let mut application = ClientApplication::from_runtime(runtime);
    let mut user_offset_seconds = 0.0;

    run_planned_local_runtime_action_legacy_compatible(
        &mut application,
        &mut user_offset_seconds,
        1.0,
        PlannedLocalRuntimeAction::Pause,
    )
    .expect("pause should dispatch");
    assert!(application.player().paused(), "pause must pause playback");

    run_planned_local_runtime_action_legacy_compatible(
        &mut application,
        &mut user_offset_seconds,
        2.0,
        PlannedLocalRuntimeAction::Pause,
    )
    .expect("repeated pause should remain valid");
    assert!(
        application.player().paused(),
        "a repeated pause must not resume playback"
    );

    run_planned_local_runtime_action_legacy_compatible(
        &mut application,
        &mut user_offset_seconds,
        3.0,
        PlannedLocalRuntimeAction::Play,
    )
    .expect("play should dispatch");
    assert!(!application.player().paused(), "play must resume playback");

    run_planned_local_runtime_action_legacy_compatible(
        &mut application,
        &mut user_offset_seconds,
        4.0,
        PlannedLocalRuntimeAction::Play,
    )
    .expect("repeated play should remain valid");
    assert!(
        !application.player().paused(),
        "a repeated play must not pause playback"
    );

    run_planned_local_runtime_action_legacy_compatible(
        &mut application,
        &mut user_offset_seconds,
        5.0,
        PlannedLocalRuntimeAction::TogglePause,
    )
    .expect("p compatibility toggle should dispatch");
    assert!(
        application.player().paused(),
        "the p compatibility action must still toggle playback"
    );
}

fn cli_v2_readiness_application() -> ClientApplication<MpvAdapter> {
    let player = sorotte_player_mpv::SimulatedPlayer::new().into_inner();
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        player,
        QueuedRuntimeControl::default(),
    );
    runtime.begin_protocol_connection_generation();
    let mut application = ClientApplication::from_runtime(runtime);
    application
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("V2 Hello should apply");
    application
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"sorotteReadinessV2":{"snapshot":{"roomReadinessRevision":1,"mediaGeneration":7,"startGatePhase":{"phase":"waitingForIntent","mediaGeneration":7},"pauseOwner":{"owner":"readinessStartGate","mediaGeneration":7},"participants":{"alice":{"roomReadinessRevision":1,"membershipEpoch":41,"username":"alice","userIntent":"notReady","userIntentRevision":1,"userIntentSource":{"type":"initialization"},"technicalState":{"phase":"playable","mediaGeneration":7},"participationRole":"required","roomReady":false,"startEligible":false}}}}}}"#,
        )
        .expect("V2 readiness snapshot should apply");
    assert!(application.session().server_readiness_v2_supported());
    assert_eq!(
        application
            .session()
            .canonical_participant_readiness("alice")
            .map(|participant| participant.membership_epoch),
        Some(41),
    );
    application
        .session_mut()
        .apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default().with_paused(false),
        );
    application
}

fn pending_readiness_intent(
    application: &mut ClientApplication<MpvAdapter>,
) -> sorotte_protocol::ReadinessIntentRequest {
    let pending = application
        .pending_protocol_line()
        .expect("CLI readiness line should serialize")
        .expect("CLI action should queue a readiness intent");
    let message = decode_message_line(pending.line()).expect("CLI readiness line should decode");
    let lease = pending.lease();
    application.acknowledge_protocol_line(lease);
    let ProtocolMessage::Set(payload) = message else {
        panic!("CLI readiness intent should use a Set envelope");
    };
    payload
        .set
        .readiness_v2()
        .expect("CLI readiness extension should decode")
        .and_then(|extension| extension.intent)
        .expect("CLI action should queue a readiness intent")
}

#[test]
fn cli_readiness_commands_preserve_direct_and_indirect_v2_sources() {
    use sorotte_protocol::{
        DirectReadinessSurface, PlayerInteractionSurface, PlayerReadinessAction,
        UserReadinessIntent, UserReadinessMutationSource,
    };

    let mut application = cli_v2_readiness_application();
    let mut user_offset_seconds = 0.0;

    for (action, desired) in [
        (
            PlannedLocalRuntimeAction::SetUserReady {
                username: String::new(),
                ready: true,
            },
            UserReadinessIntent::Ready,
        ),
        (
            PlannedLocalRuntimeAction::SetUserReady {
                username: String::new(),
                ready: false,
            },
            UserReadinessIntent::NotReady,
        ),
    ] {
        run_planned_local_runtime_action_legacy_compatible(
            &mut application,
            &mut user_offset_seconds,
            1.0,
            action,
        )
        .expect("direct CLI readiness command should dispatch");
        let intent = pending_readiness_intent(&mut application);
        assert_eq!(intent.desired, desired);
        assert_eq!(
            intent.source,
            UserReadinessMutationSource::DirectUser {
                surface: DirectReadinessSurface::CliCommand,
            }
        );
    }

    for (action, desired, player_action) in [
        (
            PlannedLocalRuntimeAction::Pause,
            UserReadinessIntent::NotReady,
            PlayerReadinessAction::Pause,
        ),
        (
            PlannedLocalRuntimeAction::Play,
            UserReadinessIntent::Ready,
            PlayerReadinessAction::Play,
        ),
    ] {
        run_planned_local_runtime_action_legacy_compatible(
            &mut application,
            &mut user_offset_seconds,
            2.0,
            action,
        )
        .expect("CLI playback command should dispatch");
        let intent = pending_readiness_intent(&mut application);
        assert_eq!(intent.desired, desired);
        assert_eq!(
            intent.source,
            UserReadinessMutationSource::IndirectPlayer {
                action: player_action,
                surface: PlayerInteractionSurface::SorottePlaybackControl,
            }
        );
    }
}
