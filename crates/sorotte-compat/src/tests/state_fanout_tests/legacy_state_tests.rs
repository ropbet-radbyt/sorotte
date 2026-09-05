use super::*;

#[test]
fn legacy_server_state_propagation_matches_runtime_core_behavior() {
    let steps = load_server_runtime_scenario_fixture("server_runtime_state_propagation.jsonl")
        .expect("state propagation scenario fixture should load");
    let rust_events = replay_server_runtime_scenario_steps(&steps)
        .expect("state propagation scenario should replay through server runtime");
    let legacy_events = match run_legacy_server_fanout_roundtrip(&steps) {
        Ok(events) => events,
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!("legacy state propagation test skipped due to missing prerequisites: {err}");
            return;
        }
        Err(err) => panic!(
            "legacy state propagation roundtrip should succeed for probe scenario, got: {err}"
        ),
    };

    let rust_state_event = rust_events
        .get(2)
        .expect("step 3 state event should exist for runtime replay");
    let legacy_state_event = legacy_events
        .get(2)
        .expect("step 3 state event should exist for legacy replay");

    let mut rust_state_summaries: Vec<(String, String, bool, bool, f64)> = rust_state_event
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let message = decode_message_line(&outbound.line).ok()?;
            if is_background_idle_state_message(&message) {
                return None;
            }
            let ProtocolMessage::State(payload) = message else {
                return None;
            };
            let playstate = payload.state.playstate?;
            let ping = payload.state.ping?;
            assert!(
                ping.latency_calculation.is_some(),
                "runtime state update should include latencyCalculation"
            );
            assert_eq!(
                ping.server_rtt,
                Some(0.0),
                "runtime state update should include serverRtt=0"
            );
            Some((
                outbound.client_id.clone(),
                playstate.set_by.unwrap_or_default(),
                playstate.paused.unwrap_or_default(),
                playstate.do_seek.unwrap_or_default(),
                playstate.position.unwrap_or_default(),
            ))
        })
        .collect();
    rust_state_summaries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut legacy_state_summaries: Vec<(String, String, bool, bool, f64)> = legacy_state_event
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let message = decode_message_line(&outbound.line).ok()?;
            if is_background_idle_state_message(&message) {
                return None;
            }
            let ProtocolMessage::State(payload) = message else {
                return None;
            };
            let playstate = payload.state.playstate?;
            let ping = payload.state.ping?;
            assert!(
                ping.latency_calculation.is_some(),
                "legacy state update should include latencyCalculation"
            );
            assert_eq!(
                ping.server_rtt,
                Some(0.0),
                "legacy state update should include serverRtt=0"
            );
            Some((
                outbound.client_id.clone(),
                playstate.set_by.unwrap_or_default(),
                playstate.paused.unwrap_or_default(),
                playstate.do_seek.unwrap_or_default(),
                playstate.position.unwrap_or_default(),
            ))
        })
        .collect();
    legacy_state_summaries.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(
        rust_state_summaries.len(),
        2,
        "runtime step 3 should broadcast state to sender and room peer"
    );
    assert_eq!(
        legacy_state_summaries.len(),
        2,
        "legacy step 3 should broadcast state to sender and room peer"
    );

    let expected_recipients = vec!["client-1".to_owned(), "client-2".to_owned()];
    assert_eq!(
        rust_state_summaries
            .iter()
            .map(|summary| summary.0.clone())
            .collect::<Vec<_>>(),
        expected_recipients
    );
    assert_eq!(
        legacy_state_summaries
            .iter()
            .map(|summary| summary.0.clone())
            .collect::<Vec<_>>(),
        expected_recipients
    );
    for (_, set_by, paused, do_seek, position) in rust_state_summaries {
        assert_eq!(set_by, "alice");
        assert!(!paused);
        assert!(!do_seek);
        assert_eq!(position, 12.5);
    }
    for (_, set_by, paused, do_seek, position) in legacy_state_summaries {
        assert_eq!(set_by, "alice");
        assert!(!paused);
        assert!(!do_seek);
        assert!(
            (position - 12.5).abs() <= 0.01,
            "legacy playstate position should stay near requested position"
        );
    }

    let rust_unchanged_state_event = rust_events
        .get(3)
        .expect("step 4 unchanged-playstate event should exist for runtime replay");
    let legacy_unchanged_state_event = legacy_events
        .get(3)
        .expect("step 4 unchanged-playstate event should exist for legacy replay");
    assert!(
        rust_unchanged_state_event.outbound_lines.is_empty(),
        "runtime unchanged playstate update should produce no immediate outbound lines"
    );
    assert!(
        legacy_unchanged_state_event.outbound_lines.is_empty(),
        "legacy unchanged playstate update should produce no immediate outbound lines"
    );

    let rust_ping_only_event = rust_events
        .get(4)
        .expect("step 5 ping-only event should exist for runtime replay");
    let legacy_ping_only_event = legacy_events
        .get(4)
        .expect("step 5 ping-only event should exist for legacy replay");
    assert!(
        rust_ping_only_event.outbound_lines.is_empty(),
        "runtime ping-only state update should produce no immediate outbound lines"
    );
    assert!(
        legacy_ping_only_event.outbound_lines.is_empty(),
        "legacy ping-only state update should produce no immediate outbound lines"
    );
}

#[test]
fn legacy_server_state_latency_metrics_matches_runtime_core_behavior() {
    let steps = load_server_runtime_scenario_fixture("server_runtime_state_latency_metrics.jsonl")
        .expect("state latency-metrics scenario fixture should load");
    let rust_events = replay_server_runtime_scenario_steps(&steps)
        .expect("state latency-metrics scenario should replay through server runtime");
    let legacy_events = match run_legacy_server_fanout_roundtrip(&steps) {
        Ok(events) => events,
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy state latency-metrics test skipped due to missing prerequisites: {err}"
            );
            return;
        }
        Err(err) => panic!(
            "legacy state latency-metrics roundtrip should succeed for probe scenario, got: {err}"
        ),
    };

    let rust_state_event = rust_events
        .last()
        .expect("final step state event should exist for runtime replay");
    let legacy_state_event = legacy_events
        .last()
        .expect("final step state event should exist for legacy replay");

    let parse_summary = |line: &str| -> Option<(String, bool, bool, f64, f64)> {
        let message = decode_message_line(line).ok()?;
        if is_background_idle_state_message(&message) {
            return None;
        }
        let ProtocolMessage::State(payload) = message else {
            return None;
        };
        let playstate = payload.state.playstate?;
        if playstate.do_seek != Some(true) {
            return None;
        }
        let ping = payload.state.ping?;
        Some((
            playstate.set_by.unwrap_or_default(),
            playstate.paused.unwrap_or_default(),
            playstate.do_seek.unwrap_or_default(),
            playstate.position.unwrap_or_default(),
            ping.server_rtt.unwrap_or_default(),
        ))
    };

    let rust_sender = rust_state_event
        .outbound_lines
        .iter()
        .find_map(|outbound| {
            if outbound.client_id != "client-1" {
                return None;
            }
            parse_summary(&outbound.line)
        })
        .expect("runtime replay should include sender-directed state output");
    let rust_peer = rust_state_event
        .outbound_lines
        .iter()
        .find_map(|outbound| {
            if outbound.client_id != "client-2" {
                return None;
            }
            parse_summary(&outbound.line)
        })
        .expect("runtime replay should include peer-directed state output");

    let legacy_sender = legacy_state_event
        .outbound_lines
        .iter()
        .find_map(|outbound| {
            if outbound.client_id != "client-1" {
                return None;
            }
            parse_summary(&outbound.line)
        })
        .expect("legacy replay should include sender-directed state output");
    let legacy_peer = legacy_state_event
        .outbound_lines
        .iter()
        .find_map(|outbound| {
            if outbound.client_id != "client-2" {
                return None;
            }
            parse_summary(&outbound.line)
        })
        .expect("legacy replay should include peer-directed state output");

    for (set_by, paused, do_seek, position, server_rtt) in [&rust_sender, &rust_peer] {
        assert_eq!(set_by, "alice");
        assert!(!paused);
        assert!(*do_seek);
        assert!(
            (*position - 18.0).abs() <= 0.000_001,
            "runtime should apply forward delay to shared position"
        );
        assert!(
            *server_rtt >= 0.0,
            "runtime state updates should include non-negative serverRtt"
        );
    }
    assert!(
        (rust_sender.4 - 10.0).abs() <= 0.000_001,
        "runtime sender-directed state should include derived non-zero serverRtt"
    );
    assert_eq!(
        rust_peer.4, 0.0,
        "runtime peer-directed state should include default serverRtt"
    );

    for (set_by, paused, do_seek, _position, server_rtt) in [&legacy_sender, &legacy_peer] {
        assert_eq!(set_by, "alice");
        assert!(!paused);
        assert!(*do_seek);
        assert!(
            *server_rtt >= 0.0,
            "legacy state updates should include non-negative serverRtt"
        );
    }
    assert!(
        (10.0..12.0).contains(&legacy_sender.4),
        "the live echo must measure the actual ten-second client delay: {:?}",
        legacy_sender
    );
    let expected_live_position = 5.0 + legacy_sender.4 / 2.0 + (legacy_sender.4 - 2.0);
    assert!(
        (legacy_sender.3 - expected_live_position).abs() <= 0.01,
        "live upstream must apply its measured RTT and clientRtt to forward delay"
    );
    assert_eq!(
        legacy_peer.4, 0.0,
        "the peer has not echoed a ping challenge"
    );
    assert!(
        (legacy_sender.3 - legacy_peer.3).abs() <= 0.01,
        "legacy sender and peer should receive equivalent forwarded positions"
    );
    assert!(
        legacy_sender.4 >= legacy_peer.4,
        "legacy sender-directed serverRtt should not be lower than peer default"
    );
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_state_metadata_forwarding_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_metadata_forwarding.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for state metadata forwarding scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_state_periodic_timeout_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_periodic_timeout.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for state periodic-timeout scenario should succeed, got: {err}"
        ),
    }
}
