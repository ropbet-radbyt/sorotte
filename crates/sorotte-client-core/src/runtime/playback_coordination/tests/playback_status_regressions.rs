use super::*;

#[test]
fn delayed_remote_pause_observation_preserves_participant_readiness() {
    let mut runtime = ClientRuntime::new(
        readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::Ready),
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("delayed-room-pause").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.observe_external_player_transport(
        transport(1, 0.0, PlayerTransportPhase::Playing, 1.0),
        0.0,
    );
    runtime.session_mut().apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        0.05,
    ).unwrap();
    runtime
        .record_external_system_player_pause_command_result(
            true,
            PlayerCommandCause::RemoteRoomSynchronization,
            true,
            0.05,
        )
        .unwrap();
    runtime.deliver_queued_protocol_messages();
    for now in [3.0, 3.2] {
        runtime.observe_external_player_transport(
            paused_transport(1, now, PlayerTransportPhase::ReadyPaused, 1.0),
            now,
        );
    }
    assert!(
        runtime.session().pending_readiness_intent().is_none(),
        "a delayed room pause must not become a participant's NotReady gesture"
    );
    assert!(
        runtime
            .playback_coordination
            .pending_remote_pause_observation
            .is_none()
    );
    // Once consumed, that remote command cannot swallow a later native gesture.
    for now in [3.5, 3.7] {
        runtime.observe_external_player_transport(
            transport(1, now, PlayerTransportPhase::Playing, now),
            now,
        );
    }
    for now in [5.0, 5.2] {
        runtime.observe_external_player_transport(
            paused_transport(1, now, PlayerTransportPhase::ReadyPaused, 5.0),
            now,
        );
    }
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .unwrap()
            .desired,
        UserReadinessIntent::NotReady
    );
}

#[test]
fn participant_status_adopts_scope_received_after_playback_is_already_applied() {
    let mut session = participant_status_session();
    session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":1}}}"#,
        0.0,
    ).unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("scope-arrives-later").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
    coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);
    assert!(coordination.last_applied_revision.is_some());

    session.apply_message_json_at(
        r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":19,"transportRevision":1}}}}"#,
        0.3,
    ).unwrap();
    assert!(
        coordination
            .update_desired_from_session(&session, 0.3)
            .is_empty()
    );
    assert!(
        coordination
            .take_participant_status_report(&session, true, 0.3)
            .unwrap()
            .playback_scope
            .is_none(),
        "cached playback cannot qualify a newly received scope"
    );
    assert!(
        coordination
            .observe_transport(transport(1, 0.4, PlayerTransportPhase::Playing, 0.4), 0.4)
            .is_empty(),
        "advisory scope adoption must not replay control or barrier acknowledgements"
    );
    let report = coordination
        .take_participant_status_report(&session, true, 0.4)
        .unwrap();
    assert_eq!(report.phase, ParticipantPlaybackPhase::Playing);
    assert_eq!(
        report.playback_scope,
        session.participant_status_authoritative_scope()
    );
    assert!(report.playback_scope.is_some());

    // A status scope ahead of canonical transport cannot qualify old playback.
    session.apply_message_json_at(
        r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":20,"transportRevision":2}}}}"#,
        0.5,
    ).unwrap();
    coordination.update_desired_from_session(&session, 0.5);
    coordination.observe_transport(transport(1, 0.6, PlayerTransportPhase::Playing, 0.6), 0.6);
    assert!(
        coordination
            .take_participant_status_report(&session, true, 0.6)
            .unwrap()
            .playback_scope
            .is_none()
    );

    session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":0.6,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":2}}}"#,
        0.6,
    ).unwrap();
    coordination.update_desired_from_session(&session, 0.7);
    coordination.observe_transport(transport(1, 0.8, PlayerTransportPhase::Playing, 0.8), 0.8);
    let report = coordination
        .take_participant_status_report(&session, true, 0.8)
        .unwrap();
    assert_eq!(
        report.playback_scope,
        session.participant_status_authoritative_scope()
    );
    assert!(report.playback_scope.is_some());
}
