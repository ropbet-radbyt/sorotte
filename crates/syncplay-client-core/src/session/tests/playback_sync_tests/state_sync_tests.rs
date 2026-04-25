use super::*;

#[test]
fn client_runtime_state_sync_reconcile_queues_outbound_state_after_inbound_state_seen() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.5)
                .with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            )
            .with_ping(PingPayload::new().with_latency_calculation(42.0)),
        100.0,
        0.25,
        false,
    );

    assert!(sent, "state sync should emit after telemetry is available");
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "state sync should queue one outbound state message"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    assert_eq!(
        state_message
            .state
            .playstate
            .as_ref()
            .and_then(|p| p.position),
        Some(12.5),
        "outbound state should report local position"
    );
    assert_eq!(
        state_message
            .state
            .playstate
            .as_ref()
            .and_then(|p| p.paused),
        Some(true),
        "outbound state should report local paused state"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.latency_calculation),
        Some(42.0),
        "outbound ping should echo inbound latencyCalculation when present"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_latency_calculation),
        Some(100.0),
        "outbound ping should include client latency calculation"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_rtt),
        Some(0.25),
        "outbound ping should include client RTT"
    );
}

#[test]
fn client_runtime_state_sync_heartbeat_emits_ping_only_without_local_playback_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime.run_state_sync_heartbeat_legacy_ping_compatible(false),
        "heartbeat should queue a ping-only state after hello even without local playback telemetry"
    );
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "heartbeat should queue one outbound state message"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued heartbeat should be State");
    };
    assert_eq!(
        state_message.state.playstate, None,
        "heartbeat without local playback telemetry should omit playstate"
    );
    let ping = state_message
        .state
        .ping
        .as_ref()
        .expect("heartbeat should include ping metadata");
    assert!(
        ping.client_latency_calculation.unwrap_or(0.0) > 0.0,
        "heartbeat should include non-zero clientLatencyCalculation"
    );
    assert!(
        ping.client_rtt.is_some(),
        "heartbeat should include clientRtt even without local playback telemetry"
    );
}

#[test]
fn client_runtime_state_sync_heartbeat_reports_room_position_when_dont_slow_down_with_me_is_enabled()
 {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("room state should apply");
    let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
    session
        .room_playstate_updated_at_seconds
        .insert("room1".to_owned(), now_seconds - 2.0);

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(15.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime.run_state_sync_heartbeat_legacy_ping_compatible(true),
        "heartbeat should emit state while the session is active"
    );

    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued heartbeat should be State");
    };
    let outbound_position = state_message
        .state
        .playstate
        .as_ref()
        .and_then(|playstate| playstate.position)
        .expect("heartbeat should include playstate position");
    assert!(
        (11.0..13.5).contains(&outbound_position),
        "dontSlowDownWithMe should report the room position instead of the locally fast-forwarded player time, got {outbound_position}"
    );
    assert_eq!(
        state_message
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.paused),
        Some(false)
    );
}

#[test]
fn client_runtime_desync_correction_legacy_ping_forward_delay_compensates_borderline_fastforward_threshold()
 {
    fn local_unpaused_telemetry(position_seconds: f64) -> PlayerPlaybackTelemetryUpdate {
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(position_seconds)
            .with_paused(false)
    }

    fn runtime_fixture() -> ClientRuntime<RecordingPlayer, QueuedRuntimeControl> {
        let session = desync_session_with_remote_state(5.0, false, false, "bob");
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(local_unpaused_telemetry(0.2)),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        ClientRuntime::new(session, player, control)
    }

    let mut baseline_runtime = runtime_fixture();
    baseline_runtime
        .run_desync_correction_if_needed(0.0, false, false, false)
        .expect("initial behind detection should not fail");
    baseline_runtime
        .player_mut()
        .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
    baseline_runtime
        .run_desync_correction_if_needed(4.0, false, false, false)
        .expect("borderline fastforward check should not fail");
    assert_eq!(
        baseline_runtime.player().position,
        None,
        "without forward-delay compensation, local position should stay just above fastforward threshold"
    );

    let mut compensated_runtime = runtime_fixture();
    compensated_runtime
        .ping_metrics_legacy_compatible
        .forward_delay_seconds = 0.35;
    compensated_runtime
        .run_desync_correction_if_needed(0.0, false, false, false)
        .expect("initial behind detection with forward delay should not fail");
    compensated_runtime
        .player_mut()
        .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
    compensated_runtime
        .run_desync_correction_if_needed(4.0, false, false, false)
        .expect("compensated fastforward check should not fail");
    assert_eq!(
        compensated_runtime.player().position,
        Some(5.25),
        "forward-delay compensation should push a borderline behind client over the fastforward threshold"
    );
}
