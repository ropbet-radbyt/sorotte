use super::*;

const PING_ASSERTION_EPSILON: f64 = 1e-9;

fn assert_ping_value(actual: f64, expected: f64, context: &str) {
    assert!(
        (actual - expected).abs() < PING_ASSERTION_EPSILON,
        "{context}: expected {expected}, got {actual}"
    );
}

fn ping_metric_snapshot(metrics: ClientPingMetricsLegacyCompatible) -> (f64, f64, f64) {
    (
        metrics.client_rtt_seconds(),
        metrics.server_rtt_seconds(),
        metrics.forward_delay_seconds(),
    )
}

fn independent_unix_wall_clock_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test host clock should be after the Unix epoch")
        .as_secs_f64()
}

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

    let now = unix_wall_clock_time_seconds_legacy_compatible();
    let inbound_latency_calculation = now - 0.05;
    let inbound_client_latency_calculation = now - 0.08;
    let sent = runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            )
            .with_ping(
                PingPayload::new()
                    .with_latency_calculation(inbound_latency_calculation)
                    .with_client_latency_calculation(inbound_client_latency_calculation)
                    .with_server_rtt(0.02),
            ),
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
            .with_latency_calculation(99.0)
            .with_client_latency_calculation(100.0)
            .with_server_rtt(0.05),
    );

    ping_metrics.observe_inbound_state_at(&inbound_state, 100.2);

    assert_ping_value(
        ping_metrics.client_rtt_seconds(),
        0.2,
        "client RTT should be computed from now - inbound clientLatencyCalculation",
    );
    assert_ping_value(
        ping_metrics.forward_delay_seconds(),
        0.25,
        "forward delay should include the server RTT delta reported by the sender",
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_does_not_measure_rtt_from_latency_echo() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();
    let inbound_state =
        StatePayload::new().with_ping(PingPayload::new().with_latency_calculation(100.0));

    ping_metrics.observe_inbound_state_at(&inbound_state, 100.2);

    assert_eq!(
        ping_metrics.client_rtt_seconds(),
        0.0,
        "latencyCalculation is echoed back to the server and should not update client RTT"
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_tracks_server_rtt_and_forward_delay_estimate() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(99.0)
                .with_client_latency_calculation(100.0)
                .with_server_rtt(0.12),
        ),
        100.3,
    );

    assert_ping_value(
        ping_metrics.client_rtt_seconds(),
        0.3,
        "client RTT should track now - inbound clientLatencyCalculation",
    );
    assert_ping_value(
        ping_metrics.server_rtt_seconds(),
        0.12,
        "server RTT should track inbound ping.serverRtt",
    );
    assert_ping_value(
        ping_metrics.forward_delay_seconds(),
        0.33,
        "forward delay should use server-like formula averageRTT/2 + (clientRTT - serverRTT) when clientRTT is larger",
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_ignores_incomplete_and_invalid_inputs_atomically() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

    ping_metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(10.0)
                .with_client_latency_calculation(10.0)
                .with_server_rtt(0.1),
        ),
        10.4,
    );
    let baseline = ping_metric_snapshot(ping_metrics);

    let invalid_samples = [
        ("missing ping", StatePayload::new(), 20.0),
        (
            "empty ping",
            StatePayload::new().with_ping(PingPayload::new()),
            20.0,
        ),
        (
            "missing client latency calculation",
            StatePayload::new().with_ping(PingPayload::new().with_server_rtt(0.1)),
            20.0,
        ),
        (
            "missing server RTT",
            StatePayload::new().with_ping(PingPayload::new().with_client_latency_calculation(19.5)),
            20.0,
        ),
        (
            "NaN client latency calculation",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(f64::NAN)
                    .with_server_rtt(0.1),
            ),
            20.0,
        ),
        (
            "infinite client latency calculation",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(f64::INFINITY)
                    .with_server_rtt(0.1),
            ),
            20.0,
        ),
        (
            "negative infinite client latency calculation",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(f64::NEG_INFINITY)
                    .with_server_rtt(0.1),
            ),
            20.0,
        ),
        (
            "NaN server RTT",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(19.5)
                    .with_server_rtt(f64::NAN),
            ),
            20.0,
        ),
        (
            "infinite server RTT",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(19.5)
                    .with_server_rtt(f64::INFINITY),
            ),
            20.0,
        ),
        (
            "negative server RTT",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(19.5)
                    .with_server_rtt(-1.0),
            ),
            20.0,
        ),
        (
            "client timestamp after observation time",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(20.5)
                    .with_server_rtt(0.1),
            ),
            20.0,
        ),
        (
            "NaN observation time",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(19.5)
                    .with_server_rtt(0.1),
            ),
            f64::NAN,
        ),
        (
            "infinite observation time",
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(19.5)
                    .with_server_rtt(0.1),
            ),
            f64::INFINITY,
        ),
    ];

    for (case, state, now_seconds) in invalid_samples {
        ping_metrics.observe_inbound_state_at(&state, now_seconds);
        assert_eq!(
            ping_metric_snapshot(ping_metrics),
            baseline,
            "{case} must preserve every previously observed ping metric"
        );
    }
}

#[test]
fn client_ping_metrics_legacy_compatible_accepts_zero_and_equality_boundaries() {
    let mut zero_server_rtt = ClientPingMetricsLegacyCompatible::default();
    zero_server_rtt.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_client_latency_calculation(10.0)
                .with_server_rtt(0.0),
        ),
        10.4,
    );
    assert_ping_value(
        zero_server_rtt.client_rtt_seconds(),
        0.4,
        "zero server RTT should remain a valid sample",
    );
    assert_ping_value(
        zero_server_rtt.forward_delay_seconds(),
        0.6,
        "zero server RTT should use the positive client/server delta",
    );

    let mut zero_client_rtt = ClientPingMetricsLegacyCompatible::default();
    zero_client_rtt.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_client_latency_calculation(20.0)
                .with_server_rtt(0.25),
        ),
        20.0,
    );
    assert_ping_value(
        zero_client_rtt.server_rtt_seconds(),
        0.25,
        "zero client RTT should remain a valid sample",
    );
    assert_ping_value(
        zero_client_rtt.forward_delay_seconds(),
        0.0,
        "zero client RTT should produce zero initial average and forward delay",
    );

    let mut equal_rtts = ClientPingMetricsLegacyCompatible::default();
    equal_rtts.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_client_latency_calculation(30.0)
                .with_server_rtt(0.4),
        ),
        30.4,
    );
    assert_ping_value(
        equal_rtts.forward_delay_seconds(),
        0.2,
        "equal client and server RTTs should use only half the moving average",
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_applies_multi_sample_moving_average() {
    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

    for (client_latency_calculation, server_rtt, now_seconds) in
        [(10.0, 0.1, 10.4), (20.0, 0.1, 21.4), (30.0, 1.0, 30.8)]
    {
        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(client_latency_calculation)
                    .with_server_rtt(server_rtt),
            ),
            now_seconds,
        );
    }

    assert_ping_value(
        ping_metrics.client_rtt_seconds(),
        0.8,
        "latest client RTT should be retained separately from the moving average",
    );
    assert_ping_value(
        ping_metrics.server_rtt_seconds(),
        1.0,
        "latest server RTT should be retained",
    );
    assert_ping_value(
        ping_metrics.forward_delay_seconds(),
        0.29375,
        "three samples should use the 0.85/0.15 moving average and the no-positive-delta branch",
    );
}

#[test]
fn client_ping_metrics_legacy_compatible_wall_clock_entry_points_report_unix_time() {
    let before_seconds = independent_unix_wall_clock_seconds();
    let direct_seconds = unix_wall_clock_time_seconds_legacy_compatible();
    let metric_seconds =
        ClientPingMetricsLegacyCompatible::default().client_latency_calculation_now();

    let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();
    ping_metrics.observe_inbound_state(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_client_latency_calculation(0.0)
                .with_server_rtt(0.0),
        ),
    );
    let observed_rtt_seconds = ping_metrics.client_rtt_seconds();
    let after_seconds = independent_unix_wall_clock_seconds();
    let lower_bound = before_seconds.min(after_seconds) - 2.0;
    let upper_bound = before_seconds.max(after_seconds) + 2.0;

    for (entry_point, value) in [
        ("wall-clock helper", direct_seconds),
        ("client latency calculation wrapper", metric_seconds),
        ("inbound-state observation wrapper", observed_rtt_seconds),
    ] {
        assert!(
            value.is_finite() && (lower_bound..=upper_bound).contains(&value),
            "{entry_point} should report the current Unix wall-clock time within \
             [{lower_bound}, {upper_bound}], got {value}"
        );
    }
}
