use super::*;

fn session_with_paused_room_state() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#,
        )
        .expect("initial room state should apply");
    session
}

#[test]
fn ping_only_reconcile_rejects_partial_playstate_updates() {
    let session = session_with_paused_room_state();
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new().with_playstate(PlaystatePayload::new().with_position(99.0)),
        100.0,
        0.25,
        false,
    );

    assert!(sent, "partial state should still receive a ping response");
    let room_state = runtime
        .session()
        .current_room_playstate()
        .expect("the complete room state should remain available");
    assert_eq!(
        (room_state.position, room_state.paused),
        (Some(10.0), Some(true)),
        "a position without a paused value is not a complete state update"
    );
}

#[test]
fn telemetry_reconcile_rejects_partial_playstate_updates() {
    let session = session_with_paused_room_state();
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(10.0)
                .with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new().with_playstate(PlaystatePayload::new().with_position(99.0)),
        100.0,
        0.25,
        false,
    );

    assert!(sent, "partial state should still receive a state response");
    let room_state = runtime
        .session()
        .current_room_playstate()
        .expect("the complete room state should remain available");
    assert_eq!(
        (room_state.position, room_state.paused),
        (Some(10.0), Some(true)),
        "telemetry availability must not make a partial remote playstate authoritative"
    );
}

#[test]
fn ping_only_reconcile_preserves_pending_local_state_until_client_ack() {
    let mut session = session_with_paused_room_state();
    let local_change =
        session.reconcile_state_and_build_response(StatePayload::new(), 10.0, false, 90.0, 0.2);
    let local_playstate = local_change
        .playstate
        .as_ref()
        .expect("a local pause change should be reported");
    assert_ne!(
        local_playstate.do_seek,
        Some(true),
        "the precondition must be a pause-only change"
    );
    assert_eq!(
        session.client_ignoring_on_the_fly(),
        1,
        "a pause-only local change must enter acknowledgement fencing"
    );

    // Model a player that has not produced fresh telemetry for this state
    // cycle while retaining the pending acknowledgement fence.
    session.model.playback.local_position = None;
    session.model.playback.local_paused = None;
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(99.0)
                .with_paused(false)
                .with_set_by("bob"),
        ),
        100.0,
        0.25,
        false,
    );

    assert!(
        sent,
        "the pending acknowledgement should still receive a ping-only response"
    );
    let room_state = runtime
        .session()
        .current_room_playstate()
        .expect("the pre-ack room state should remain available");
    assert_eq!(
        (room_state.position, room_state.paused),
        (Some(10.0), Some(true)),
        "a complete remote state must remain fenced until the server echoes the client counter"
    );
    let ProtocolMessage::State(response) = &runtime.control().outbound_messages()[0] else {
        panic!("ping-only reconcile should queue a State response");
    };
    assert_eq!(
        response
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client),
        Some(1),
        "the ping-only response must continue advertising the pending client counter"
    );
}
