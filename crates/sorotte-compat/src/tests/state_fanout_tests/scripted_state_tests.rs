use super::*;

#[test]
fn scripted_server_runtime_state_propagation_scenario_replays_state_fanout() {
    let events = replay_server_runtime_scenario_fixture("server_runtime_state_propagation.jsonl")
        .expect("state propagation scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 5);

    let state_event = events.get(2).expect("step 3 state event should be present");
    assert_eq!(state_event.client_id, "client-1");
    assert_eq!(state_event.outbound_lines.len(), 2);
    for outbound in &state_event.outbound_lines {
        let message = decode_message_line(&outbound.line).expect("state fanout line should decode");
        match message {
            ProtocolMessage::State(payload) => {
                let playstate = payload
                    .state
                    .playstate
                    .as_ref()
                    .expect("state fanout should include playstate");
                assert_eq!(playstate.set_by.as_deref(), Some("alice"));
                assert_eq!(playstate.position, Some(12.5));
                assert_eq!(playstate.paused, Some(false));
                assert_eq!(playstate.do_seek, Some(false));
                assert!(
                    payload
                        .state
                        .ping
                        .as_ref()
                        .is_some_and(|ping| ping.latency_calculation.is_some()),
                    "state fanout should include ping metadata"
                );
                assert!(
                    payload
                        .state
                        .ignoring_on_the_fly
                        .as_ref()
                        .is_some_and(|ignore| ignore.server == Some(1)),
                    "state fanout should include ignoringOnTheFly server counter"
                );
            }
            other => panic!("expected state response at step 3, got {}", other.kind()),
        }
    }

    let unchanged_state_event = events
        .get(3)
        .expect("step 4 unchanged-playstate event should be present");
    assert_eq!(unchanged_state_event.client_id, "client-1");
    assert!(
        unchanged_state_event.outbound_lines.is_empty(),
        "playstate updates without seek/pause transitions should not produce immediate fanout"
    );

    let ping_only_event = events
        .get(4)
        .expect("step 5 ping-only state event should be present");
    assert_eq!(ping_only_event.client_id, "client-1");
    assert!(
        ping_only_event.outbound_lines.is_empty(),
        "ping-only state updates should not produce immediate fanout"
    );
}

#[test]
fn scripted_server_runtime_state_metadata_forwarding_scenario_replays_sender_passthrough() {
    let events =
        replay_server_runtime_scenario_fixture("server_runtime_state_metadata_forwarding.jsonl")
            .expect("state metadata forwarding scenario fixture should replay");
    assert_eq!(events.len(), 6);

    let first_forced_event = events
        .get(2)
        .expect("step 3 first forced state event should be present");
    assert_eq!(first_forced_event.outbound_lines.len(), 2);
    for outbound in &first_forced_event.outbound_lines {
        let message =
            decode_message_line(&outbound.line).expect("step 3 state fanout line should decode");
        let ProtocolMessage::State(payload) = message else {
            panic!("step 3 outputs should be state updates");
        };
        let ping = payload
            .state
            .ping
            .as_ref()
            .expect("step 3 state update should include ping");
        let ignore = payload
            .state
            .ignoring_on_the_fly
            .as_ref()
            .expect("step 3 state update should include ignore counters");
        if outbound.client_id == "client-1" {
            assert_eq!(ping.client_latency_calculation, Some(124.1));
            assert_eq!(ignore.client, Some(4));
        } else {
            assert_eq!(ping.client_latency_calculation, None);
            assert_eq!(ignore.client, None);
        }
    }

    let second_forced_event = events
        .get(3)
        .expect("step 4 second forced state event should be present");
    assert_eq!(second_forced_event.outbound_lines.len(), 2);
    for outbound in &second_forced_event.outbound_lines {
        let message =
            decode_message_line(&outbound.line).expect("step 4 state fanout line should decode");
        let ProtocolMessage::State(payload) = message else {
            panic!("step 4 outputs should be state updates");
        };
        assert_eq!(
            payload
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation),
            None,
            "client latency passthrough should be consumed after first forced send"
        );
        assert_eq!(
            payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.client),
            None,
            "client ignore passthrough should be consumed after first forced send"
        );
    }

    let ping_only_event = events
        .get(4)
        .expect("step 5 ping-only metadata event should be present");
    assert!(
        ping_only_event.outbound_lines.is_empty(),
        "step 5 ping-only metadata should not produce immediate fanout"
    );

    let final_forced_event = events
        .get(5)
        .expect("step 6 forced pause-change event should be present");
    assert_eq!(final_forced_event.outbound_lines.len(), 2);
    let sender_output = final_forced_event
        .outbound_lines
        .iter()
        .find(|output| output.client_id == "client-1")
        .expect("step 6 should include sender-directed forced state output");
    let sender_message =
        decode_message_line(&sender_output.line).expect("step 6 sender output should decode");
    let ProtocolMessage::State(payload) = sender_message else {
        panic!("step 6 sender output should be state update");
    };
    assert_eq!(
        payload
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_latency_calculation),
        Some(126.1),
        "queued ping metadata should be forwarded on next forced update"
    );
    assert_eq!(
        payload
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client),
        Some(8),
        "queued client ignore counter should be forwarded on next forced update"
    );
}

#[test]
fn scripted_server_runtime_state_latency_metrics_scenario_applies_forward_delay_and_sender_rtt() {
    let events =
        replay_server_runtime_scenario_fixture("server_runtime_state_latency_metrics.jsonl")
            .expect("state latency-metrics scenario fixture should replay through runtime");
    assert_eq!(events.len(), 3);

    let state_event = events.get(2).expect("step 3 state event should be present");
    assert_eq!(state_event.client_id, "client-1");
    assert_eq!(state_event.outbound_lines.len(), 2);

    let mut saw_sender = false;
    let mut saw_peer = false;

    for outbound in &state_event.outbound_lines {
        let message =
            decode_message_line(&outbound.line).expect("step 3 outbound line should decode");
        let ProtocolMessage::State(payload) = message else {
            panic!("step 3 outputs should be state updates");
        };
        let playstate = payload
            .state
            .playstate
            .as_ref()
            .expect("step 3 state update should include playstate");
        assert_eq!(playstate.set_by.as_deref(), Some("alice"));
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(playstate.do_seek, Some(true));
        assert!(
            (playstate
                .position
                .expect("state update should include position")
                - 18.0)
                .abs()
                <= 0.000_001,
            "forward delay should be applied to shared position"
        );

        let ping = payload
            .state
            .ping
            .as_ref()
            .expect("step 3 state update should include ping");
        let server_rtt = ping
            .server_rtt
            .expect("state update should include serverRtt");

        if outbound.client_id == "client-1" {
            saw_sender = true;
            assert!(
                (server_rtt - 10.0).abs() <= 0.000_001,
                "sender-directed update should include derived non-zero serverRtt"
            );
        } else if outbound.client_id == "client-2" {
            saw_peer = true;
            assert_eq!(
                server_rtt, 0.0,
                "peer-directed update should retain default serverRtt"
            );
        } else {
            panic!("unexpected outbound recipient '{}'", outbound.client_id);
        }
    }

    assert!(
        saw_sender,
        "step 3 should include sender-directed state update"
    );
    assert!(saw_peer, "step 3 should include peer-directed state update");
}

#[test]
fn scripted_server_runtime_state_periodic_timeout_scenario_emits_periodic_and_drops_stale_client() {
    let events =
        replay_server_runtime_scenario_fixture("server_runtime_state_periodic_timeout.jsonl")
            .expect("periodic-timeout scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 5);

    let periodic_event = events
        .get(2)
        .expect("step 3 periodic-state event should be present");
    assert!(
        periodic_event.outbound_lines.iter().any(|line| {
            line.client_id == "client-1"
                && decode_message_line(&line.line)
                    .ok()
                    .is_some_and(|message| matches!(message, ProtocolMessage::State(_)))
        }),
        "step 3 should include periodic state updates for stale client"
    );
    assert!(
        periodic_event.outbound_lines.iter().any(|line| {
            line.client_id == "client-2"
                && decode_message_line(&line.line)
                    .ok()
                    .is_some_and(|message| matches!(message, ProtocolMessage::State(_)))
        }),
        "step 3 should include periodic state updates for active client"
    );

    let timeout_event = events
        .get(3)
        .expect("step 4 timeout event should be present");
    assert!(
        timeout_event.outbound_lines.iter().any(|line| {
            if line.client_id != "client-2" {
                return false;
            }
            decode_message_line(&line.line).ok().is_some_and(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload
                            .set
                            .user
                            .as_ref()
                            .and_then(|users| users.get("alice"))
                            .and_then(|user| user.event.as_ref())
                            .and_then(|event| event.get("left"))
                            == Some(&json!(true))
                )
            })
        }),
        "step 4 should notify active peers when stale client is disconnected"
    );

    let list_event = events.get(4).expect("step 5 list event should be present");
    let list_message = decode_message_line(
        &list_event
            .outbound_lines
            .first()
            .expect("step 5 should include list response")
            .line,
    )
    .expect("step 5 list output should decode");
    let ProtocolMessage::List(payload) = list_message else {
        panic!("step 5 output should be list response");
    };
    let ListPayload::Rooms(rooms) = payload.list else {
        panic!("list response should include room entries");
    };
    let room_users = rooms
        .get("room1")
        .expect("room1 should still exist for active user");
    assert!(
        room_users.contains_key("bob"),
        "active user should remain present after timeout handling"
    );
    assert!(
        !room_users.contains_key("alice"),
        "stale disconnected user should be removed from room list"
    );
}
