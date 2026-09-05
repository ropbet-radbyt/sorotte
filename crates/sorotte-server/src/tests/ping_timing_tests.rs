use super::*;

fn connected_runtime() -> ServerRuntime {
    let mut runtime = ServerRuntime::new();
    runtime.set_clock_overrides_seconds(Some(100.0), Some(1.0));
    runtime
        .handle_line(
            "peer",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    runtime
}

fn issue(runtime: &mut ServerRuntime) -> f64 {
    let ProtocolMessage::State(state) =
        runtime.forced_state_sync_message_for_client("peer", 0.0, true, false, None)
    else {
        panic!("expected State");
    };
    state.state.ping.unwrap().latency_calculation.unwrap()
}

#[test]
fn matching_echo_uses_elapsed_time_across_wall_jumps_and_only_once() {
    for wall in [-10_000.0, 100.0, 50_000.0] {
        let mut runtime = connected_runtime();
        let echo = issue(&mut runtime);
        runtime.set_clock_overrides_seconds(Some(wall), Some(2.25));
        runtime.ingest_client_ping_metrics("peer", Some(echo), Some(1.25));
        assert_eq!(runtime.server_rtt_seconds("peer"), 1.25);
        assert_eq!(runtime.forward_delay_seconds("peer"), 0.625);
        runtime.set_clock_overrides_seconds(Some(wall), Some(4.0));
        runtime.ingest_client_ping_metrics("peer", Some(echo), Some(0.0));
        assert_eq!(
            runtime.server_rtt_seconds("peer"),
            1.25,
            "duplicate echo poisoned timing"
        );
    }
}

#[test]
fn rejected_echoes_preserve_estimates_and_cannot_cross_connection_epochs() {
    let mut runtime = connected_runtime();
    let echo = issue(&mut runtime);
    runtime.set_clock_overrides_seconds(Some(101.0), Some(2.0));
    runtime.ingest_client_ping_metrics("peer", Some(echo), None);
    let prior = runtime.forward_delay_seconds("peer");
    for invalid in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        echo + 1000.0,
        echo - 0.5,
    ] {
        runtime.ingest_client_ping_metrics("peer", Some(invalid), Some(0.0));
        assert_eq!(runtime.forward_delay_seconds("peer"), prior);
    }
    let retired = issue(&mut runtime);
    runtime
        .handle_line(
            "peer",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let current = issue(&mut runtime);
    assert_ne!(
        current, retired,
        "same-wall reconnect reused an echo identity"
    );
    runtime.ingest_client_ping_metrics("peer", Some(retired), Some(0.0));
    assert_eq!(runtime.server_rtt_seconds("peer"), 0.0);
    runtime.set_clock_overrides_seconds(Some(102.0), Some(3.0));
    runtime.ingest_client_ping_metrics("peer", Some(current), Some(1.0));
    assert_eq!(runtime.server_rtt_seconds("peer"), 1.0);
}

#[test]
fn challenge_capacity_age_and_sender_values_are_bounded() {
    let mut runtime = connected_runtime();
    let first = issue(&mut runtime);
    for _ in 0..100 {
        issue(&mut runtime);
    }
    assert_eq!(
        runtime.client_state_counters["peer"]
            .outstanding_ping_challenges
            .len(),
        64
    );
    runtime.ingest_client_ping_metrics("peer", Some(first), Some(0.0));
    assert_eq!(runtime.server_rtt_seconds("peer"), 0.0);
    let valid = issue(&mut runtime);
    runtime.set_clock_overrides_seconds(Some(189.0), Some(90.0));
    for bad_sender in [f64::NAN, f64::INFINITY, -1.0, 1000.0] {
        runtime.ingest_client_ping_metrics("peer", Some(valid), Some(bad_sender));
        assert_eq!(runtime.server_rtt_seconds("peer"), 0.0);
    }
    runtime.ingest_client_ping_metrics("peer", Some(valid), Some(50.0));
    assert_eq!(
        runtime.server_rtt_seconds("peer"),
        89.0,
        "legitimate large RTT should be supported"
    );
    let expired = issue(&mut runtime);
    runtime.set_clock_overrides_seconds(Some(0.0), Some(180.001));
    runtime.ingest_client_ping_metrics("peer", Some(expired), Some(0.0));
    assert_eq!(
        runtime.server_rtt_seconds("peer"),
        89.0,
        "old echo must not replace estimate"
    );
    assert_eq!(
        runtime.forward_delay_seconds("peer"),
        0.0,
        "old estimate must age out"
    );
}

#[test]
fn issuing_and_consuming_challenges_prunes_expired_queue_entries() {
    let mut runtime = connected_runtime();
    runtime.set_clock_overrides_seconds(Some(109.0), Some(10.0));
    let old = issue(&mut runtime);
    runtime.set_clock_overrides_seconds(Some(189.0), Some(90.0));
    let current = issue(&mut runtime);
    assert!(
        runtime.client_state_counters["peer"]
            .outstanding_ping_challenges
            .iter()
            .any(|(wire, _)| *wire == old)
    );
    runtime.set_clock_overrides_seconds(Some(204.0), Some(105.0));
    runtime.ingest_client_ping_metrics("peer", Some(current), Some(15.0));
    assert_eq!(runtime.server_rtt_seconds("peer"), 15.0);
    assert!(
        runtime.client_state_counters["peer"]
            .outstanding_ping_challenges
            .is_empty()
    );

    let old = issue(&mut runtime);
    runtime.set_clock_overrides_seconds(Some(299.0), Some(200.0));
    let current = issue(&mut runtime);
    let pending = &runtime.client_state_counters["peer"].outstanding_ping_challenges;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, current);
    assert_ne!(pending[0].0, old);
}

#[test]
fn sender_rtt_boundaries_preserve_the_server_owned_forward_delay() {
    for (sender, expected) in [(0.0, 3.0), (2.0, 1.0), (3.0, 1.0), (90.0, 1.0)] {
        let mut runtime = connected_runtime();
        let echo = issue(&mut runtime);
        runtime.set_clock_overrides_seconds(Some(102.0), Some(3.0));
        runtime.ingest_client_ping_metrics("peer", Some(echo), Some(sender));
        assert_eq!(runtime.server_rtt_seconds("peer"), 2.0);
        assert_eq!(runtime.forward_delay_seconds("peer"), expected);
    }
}

#[test]
fn ancient_unmatched_echo_cannot_amplify_a_valid_seek() {
    let mut runtime = ServerRuntime::new();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line(
            "peer",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let output = runtime.handle_line_fanout("peer", r#"{"State":{"playstate":{"position":30,"paused":false,"doSeek":true},"ping":{"latencyCalculation":-1000,"clientRtt":0}}}"#).unwrap();
    let positions: Vec<_> = decode_directed_lines(&output)
        .into_iter()
        .filter_map(|(_, message)| {
            let ProtocolMessage::State(state) = message else {
                return None;
            };
            state.state.playstate.and_then(|state| state.position)
        })
        .collect();
    assert!(
        !positions.is_empty(),
        "invalid timing must not suppress the seek"
    );
    assert!(
        positions.iter().all(|position| *position == 30.0),
        "ancient echo amplified the seek: {positions:?}"
    );
}
