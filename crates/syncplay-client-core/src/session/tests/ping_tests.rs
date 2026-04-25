use super::*;

#[test]
fn client_runtime_state_sync_reconcile_legacy_ping_wrapper_tracks_and_emits_ping_metrics() {
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

    let inbound_latency_calculation = unix_wall_clock_time_seconds_legacy_compatible() - 0.05;
    let sent = runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            )
            .with_ping(PingPayload::new().with_latency_calculation(inbound_latency_calculation)),
        false,
    );

    assert!(
        sent,
        "legacy ping wrapper should emit after telemetry is available"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    let ping = state_message
        .state
        .ping
        .as_ref()
        .expect("outbound state should include ping");
    assert_eq!(
        ping.latency_calculation,
        Some(inbound_latency_calculation),
        "outbound ping should echo inbound latencyCalculation"
    );
    assert!(
        ping.client_latency_calculation.unwrap_or(0.0) > 0.0,
        "outbound ping should include non-zero clientLatencyCalculation"
    );
    let client_rtt = ping
        .client_rtt
        .expect("outbound ping should include clientRtt");
    assert!(
        (0.0..2.0).contains(&client_rtt),
        "outbound ping should include a plausible clientRtt, got {client_rtt}"
    );
    assert_eq!(
        ping.server_rtt, None,
        "client outbound ping should not echo serverRtt from inbound state"
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_tracks_rtt_from_inbound_state_ping() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();
    let inbound_state = StatePayload::new().with_ping(
        PingPayload::new()
            .with_latency_calculation(100.0)
            .with_client_rtt(0.25),
    );

    ping_metrics.observe_inbound_state_at(&inbound_state, 100.2);

    assert!(
        (ping_metrics.client_rtt_seconds() - 0.2).abs() < 1e-9,
        "client RTT should be computed from now - inbound latencyCalculation"
    );
    assert!(
        (ping_metrics.forward_delay_seconds() - 0.1).abs() < 1e-9,
        "without inbound serverRtt, forward delay should default to averageRTT/2"
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_tracks_server_rtt_and_forward_delay_estimate() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(100.0)
                .with_client_rtt(0.25)
                .with_server_rtt(0.12),
        ),
        100.3,
    );

    assert!(
        (ping_metrics.client_rtt_seconds() - 0.3).abs() < 1e-9,
        "client RTT should track now - inbound latencyCalculation"
    );
    assert!(
        (ping_metrics.server_rtt_seconds() - 0.12).abs() < 1e-9,
        "server RTT should track inbound ping.serverRtt"
    );
    assert!(
        (ping_metrics.forward_delay_seconds() - 0.33).abs() < 1e-9,
        "forward delay should use server-like formula averageRTT/2 + (clientRTT - serverRTT) when clientRTT is larger"
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_ignores_invalid_negative_ping_inputs() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(10.0)
                .with_client_rtt(0.1),
        ),
        10.4,
    );
    let baseline = ping_metrics.client_rtt_seconds();

    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(20.0)
                .with_client_rtt(-1.0),
        ),
        20.5,
    );
    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(25.0)
                .with_client_rtt(0.1)
                .with_server_rtt(-1.0),
        ),
        25.4,
    );
    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(PingPayload::new().with_latency_calculation(30.0)),
        29.0,
    );

    assert_eq!(
        ping_metrics.client_rtt_seconds(),
        baseline,
        "invalid ping inputs should not overwrite the tracked RTT"
    );
}
