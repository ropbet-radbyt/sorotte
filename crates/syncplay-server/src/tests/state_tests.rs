use super::*;

#[test]
fn state_playstate_updates_are_broadcast_to_room_members_with_metadata() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false}}}"#,
        )
        .expect("state playstate update should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert_eq!(directed_messages.len(), 2);
    assert!(
        has_state_update(&directed_messages, "client-1", "alice", 12.5, false, false),
        "sender should receive reflected state update with setBy and ping metadata"
    );
    assert!(
        has_state_update(&directed_messages, "client-2", "alice", 12.5, false, false),
        "room peer should receive reflected state update with setBy and ping metadata"
    );
}

#[test]
fn state_playstate_without_seek_or_pause_change_produces_no_immediate_outbound_messages() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let first_update = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false}}}"#,
        )
        .expect("first state update should trigger forced room fanout");
    assert_eq!(
        first_update.len(),
        2,
        "pause transition should force state propagation to room members"
    );

    let second_update = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":13.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("second state update should be accepted");
    assert!(
        second_update.is_empty(),
        "state updates without seek/pause transitions should not force immediate fanout"
    );
}

#[test]
fn state_forced_update_forwards_sender_client_metadata_once() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let first_forced_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false},"ping":{"clientLatencyCalculation":124.1,"clientRtt":0.12},"ignoringOnTheFly":{"client":7}}}"#,
        )
        .expect("first forced state update should fan out");
    let first_forced_messages = decode_directed_lines(&first_forced_lines);
    assert_eq!(first_forced_messages.len(), 2);

    for (recipient, message) in &first_forced_messages {
        let ProtocolMessage::State(payload) = message else {
            panic!("forced state update should produce state messages");
        };
        let ping = payload
            .state
            .ping
            .as_ref()
            .expect("forced state update should include ping");
        let ignore = payload
            .state
            .ignoring_on_the_fly
            .as_ref()
            .expect("forced state update should include ignore counters");
        if recipient == "client-1" {
            assert_eq!(ping.client_latency_calculation, Some(124.1));
            assert_eq!(ignore.client, Some(7));
        } else {
            assert_eq!(ping.client_latency_calculation, None);
            assert_eq!(ignore.client, None);
        }
    }

    let second_forced_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"ignoringOnTheFly":{"server":1},"playstate":{"position":13.0,"paused":false,"doSeek":true}}}"#,
        )
        .expect("second forced state update should fan out");
    let second_forced_messages = decode_directed_lines(&second_forced_lines);
    assert_eq!(second_forced_messages.len(), 2);
    for (recipient, message) in &second_forced_messages {
        let ProtocolMessage::State(payload) = message else {
            panic!("forced state update should produce state messages");
        };
        let ping = payload
            .state
            .ping
            .as_ref()
            .expect("forced state update should include ping");
        let ignore = payload
            .state
            .ignoring_on_the_fly
            .as_ref()
            .expect("forced state update should include ignore counters");
        assert_eq!(
            ping.client_latency_calculation, None,
            "client latency passthrough should be consumed after first forced send"
        );
        assert_eq!(
            ignore.client, None,
            "client ignore passthrough should be consumed after first forced send"
        );
        if recipient == "client-1" {
            assert_eq!(
                ignore.server,
                Some(1),
                "sender counter should reset after ack and increment again"
            );
        } else if recipient == "client-2" {
            assert_eq!(
                ignore.server,
                Some(2),
                "peer counter should continue incrementing without ack"
            );
        } else {
            panic!("unexpected recipient for forced state fanout");
        }
    }
}

#[test]
fn state_ping_only_client_metadata_is_forwarded_on_next_forced_update() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let ping_only_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"ping":{"clientLatencyCalculation":222.2},"ignoringOnTheFly":{"client":5}}}"#,
        )
        .expect("ping-only update should be accepted");
    assert!(
        ping_only_lines.is_empty(),
        "ping-only updates should still emit no immediate fanout"
    );

    runtime.set_time_now_override_seconds(Some(100.25));
    let forced_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":3.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("subsequent forced state update should fan out");
    let forced_messages = decode_directed_lines(&forced_lines);
    assert_eq!(forced_messages.len(), 2);
    let sender_message = forced_messages
        .iter()
        .find(|(recipient, _)| recipient == "client-1")
        .expect("sender should receive forced update")
        .1
        .clone();
    let ProtocolMessage::State(payload) = sender_message else {
        panic!("sender forced output should be state");
    };
    assert_eq!(
        payload
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_latency_calculation),
        Some(222.45)
    );
    assert_eq!(
        payload
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client),
        Some(5)
    );
}

#[test]
fn state_ping_metrics_apply_forward_delay_and_non_zero_server_rtt_for_sender() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":true},"ping":{"latencyCalculation":90.0,"clientRtt":2.0}}}"#,
        )
        .expect("state update with ping metrics should be accepted");
    let directed_messages = decode_directed_lines(&directed_lines);
    assert_eq!(directed_messages.len(), 2);

    for (recipient, message) in directed_messages {
        let ProtocolMessage::State(payload) = message else {
            panic!("state update should fanout as state messages");
        };
        let playstate = payload
            .state
            .playstate
            .as_ref()
            .expect("fanout state should include playstate");
        assert!(
            playstate
                .position
                .is_some_and(|position| (position - 18.0).abs() <= 0.000_001),
            "forward delay should be applied to unpaused position updates"
        );
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(playstate.do_seek, Some(true));
        let server_rtt = payload
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.server_rtt)
            .expect("fanout state should include ping.serverRtt");
        if recipient == "client-1" {
            assert!(
                (server_rtt - 10.0).abs() <= 0.000_001,
                "sender should receive updated non-zero serverRtt from ping metrics"
            );
        } else if recipient == "client-2" {
            assert_eq!(
                server_rtt, 0.0,
                "peer without inbound ping metrics should keep default zero serverRtt"
            );
        } else {
            panic!("unexpected recipient");
        }
    }
}

#[test]
fn controlled_room_non_controller_state_update_gets_forced_corrections() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
        )
        .expect("controller auth on uncontrolled room should respond");
    runtime
        .handle_line_fanout(
            "client-1",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("alice switch to controlled room should succeed");
    runtime
        .handle_line_fanout(
            "client-2",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("bob switch to controlled room should succeed");
    runtime
        .handle_line_fanout(
            "client-2",
            r#"{"State":{"ignoringOnTheFly":{"server":1},"ping":{"latencyCalculation":100.0}}}"#,
        )
        .expect("bob should ack room-switch forced state before sending updates");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("non-controller state update should receive correction pair");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert_eq!(
        directed_messages.len(),
        2,
        "non-controller forced correction should emit exactly two directed state updates"
    );
    assert!(
        directed_messages
            .iter()
            .all(|(recipient, _)| recipient == "client-2"),
        "non-controller correction flow should be directed only to sender"
    );

    let ProtocolMessage::State(first_state) = &directed_messages[0].1 else {
        panic!("first correction should be a state message");
    };
    let first_playstate = first_state
        .state
        .playstate
        .as_ref()
        .expect("first correction should include playstate");
    assert_eq!(first_playstate.position, Some(0.0));
    assert_eq!(first_playstate.paused, Some(false));
    assert_eq!(first_playstate.do_seek, Some(false));
    assert_eq!(first_playstate.set_by.as_deref(), Some("bob"));
    assert_eq!(
        first_state
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.server),
        Some(1)
    );

    let ProtocolMessage::State(second_state) = &directed_messages[1].1 else {
        panic!("second correction should be a state message");
    };
    let second_playstate = second_state
        .state
        .playstate
        .as_ref()
        .expect("second correction should include playstate");
    assert_eq!(second_playstate.position, Some(0.0));
    assert_eq!(second_playstate.paused, Some(true));
    assert_eq!(second_playstate.do_seek, Some(true));
    assert_eq!(second_playstate.set_by, None);
    assert_eq!(
        second_state
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.server),
        Some(2)
    );
}

#[test]
fn state_ping_only_update_produces_no_immediate_outbound_messages() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"ping":{"latencyCalculation":123.4,"clientLatencyCalculation":124.1,"clientRtt":0.12}}}"#,
        )
        .expect("state ping-only update should be accepted");

    assert!(
        directed_lines.is_empty(),
        "ping-only update should not emit immediate state fanout"
    );
}

#[test]
fn state_requires_existing_session() {
    let mut runtime = ServerRuntime::default();
    let err = runtime
        .handle_line(
            "unknown-client",
            r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect_err("state without hello should fail");
    assert!(matches!(err, ServerRuntimeError::MissingSession(_)));
}

#[test]
fn first_join_state_emits_after_initial_delay() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    let hello_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    let hello_messages = decode_directed_lines(&hello_lines);
    assert!(
        hello_messages
            .iter()
            .all(|(_, message)| !matches!(message, ProtocolMessage::State(_))),
        "initial state is emitted by the scheduled watcher tick, not the Hello response"
    );

    let first_state_lines = runtime
        .advance_time_and_collect_fanout(super::INITIAL_SERVER_STATE_DELAY_SECONDS)
        .expect("initial state tick should encode outbound fanout lines");
    let first_state_messages = decode_directed_lines(&first_state_lines);
    assert_eq!(first_state_messages.len(), 1);
    let (recipient, message) = &first_state_messages[0];
    assert_eq!(recipient, "client-1");
    let ProtocolMessage::State(payload) = message else {
        panic!("initial scheduled update should be a state message");
    };
    let playstate = payload
        .state
        .playstate
        .as_ref()
        .expect("initial scheduled state should include playstate");
    assert_eq!(playstate.position, Some(0.0));
    assert_eq!(playstate.paused, Some(true));
    assert_eq!(playstate.do_seek, Some(false));
}

#[test]
fn periodic_state_updates_emit_after_time_advance() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let periodic_lines = runtime
        .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
        .expect("periodic state tick should encode outbound fanout lines");
    let periodic_messages = decode_directed_lines(&periodic_lines);

    assert_eq!(
        periodic_messages.len(),
        2,
        "one periodic idle state update should be emitted per connected client"
    );
    let mut recipients = BTreeSet::new();
    for (recipient, message) in periodic_messages {
        recipients.insert(recipient);
        let ProtocolMessage::State(payload) = message else {
            panic!("periodic output should be state message");
        };
        let playstate = payload
            .state
            .playstate
            .as_ref()
            .expect("periodic state update should include playstate");
        assert_eq!(playstate.position, Some(0.0));
        assert_eq!(playstate.paused, Some(true));
        assert_eq!(playstate.do_seek, Some(false));
        assert_eq!(
            playstate.set_by.as_deref(),
            Some("alice"),
            "periodic idle updates should carry room setBy watcher identity"
        );
        assert!(
            payload.state.ignoring_on_the_fly.is_none(),
            "periodic idle updates should not include server ignore counters"
        );
    }
    assert_eq!(
        recipients,
        BTreeSet::from(["client-1".to_owned(), "client-2".to_owned()])
    );
}

#[test]
fn periodic_state_updates_age_playing_room_position() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .advance_time_and_collect_fanout(super::INITIAL_SERVER_STATE_DELAY_SECONDS)
        .expect("initial scheduled state should encode");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":12.0,"paused":false,"doSeek":true}}}"#,
        )
        .expect("playing state update should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"State":{"ignoringOnTheFly":{"server":1}}}"#)
        .expect("client should acknowledge forced state before periodic tick");

    let periodic_lines = runtime
        .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
        .expect("periodic state tick should encode outbound fanout lines");
    let periodic_messages = decode_directed_lines(&periodic_lines);
    let state_message = periodic_messages
        .iter()
        .find_map(|(recipient, message)| {
            if recipient == "client-1" {
                Some(message)
            } else {
                None
            }
        })
        .expect("client should receive a periodic state update");
    let ProtocolMessage::State(payload) = state_message else {
        panic!("periodic output should be state message");
    };
    let playstate = payload
        .state
        .playstate
        .as_ref()
        .expect("periodic state update should include playstate");

    assert!(
        playstate
            .position
            .is_some_and(|position| (position - 13.0).abs() <= 0.000_001),
        "playing room position should be aged by elapsed playback time"
    );
    assert_eq!(playstate.paused, Some(false));
}

#[test]
fn periodic_playing_room_state_uses_slowest_watcher_position() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    runtime
        .advance_time_and_collect_fanout(super::INITIAL_SERVER_STATE_DELAY_SECONDS)
        .expect("initial scheduled states should encode");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.0}}}"#,
        )
        .expect("alice file update should succeed");
    runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.0}}}"#,
        )
        .expect("bob file update should succeed");

    let sample_time = -0.25;
    runtime.set_time_now_override_seconds(Some(sample_time));
    let start_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true}}}"#,
        )
        .expect("alice should start playback");
    let start_messages = decode_directed_lines(&start_lines);
    acknowledge_directed_state_counters(&mut runtime, &start_messages);

    let bob_sample_lines = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("bob playback sample should be accepted");
    assert!(
        bob_sample_lines.is_empty(),
        "non-seek playing samples should not force immediate room fanout"
    );

    let elapsed_seconds = super::SERVER_STATE_INTERVAL_SECONDS
        + super::INITIAL_SERVER_STATE_DELAY_SECONDS
        - sample_time;
    let periodic_lines = runtime
        .advance_time_and_collect_fanout(elapsed_seconds)
        .expect("periodic state tick should encode outbound fanout lines");
    let periodic_messages = decode_directed_lines(&periodic_lines);
    assert_eq!(periodic_messages.len(), 2);
    for (_, message) in periodic_messages {
        let ProtocolMessage::State(payload) = message else {
            panic!("periodic output should be state message");
        };
        let playstate = payload
            .state
            .playstate
            .as_ref()
            .expect("periodic state update should include playstate");
        let position = playstate
            .position
            .expect("periodic state update should include position");
        let expected_position = 5.0 + elapsed_seconds;
        assert!(
            (position - expected_position).abs() <= 0.000_001,
            "playing room position should follow the slowest watcher: expected {expected_position}, got {position}"
        );
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(playstate.set_by.as_deref(), Some("bob"));
    }
}

#[test]
fn room_switch_sends_destination_room_playstate() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":40.0,"paused":false,"doSeek":true}}}"#,
        )
        .expect("alice should make room1 active");

    runtime.set_time_now_override_seconds(Some(103.0));
    let directed_lines = runtime
        .handle_line_fanout("client-2", r#"{"Set":{"room":{"name":"room1"}}}"#)
        .expect("bob room switch should succeed");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_state_update(&directed_messages, "client-2", "alice", 43.0, false, true),
        "switching client should receive destination room's aged playstate"
    );
}

#[test]
fn periodic_timeout_disconnects_stale_client_and_broadcasts_left_event() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    acknowledge_server_state_counter(&mut runtime, "client-1", 1);
    acknowledge_server_state_counter(&mut runtime, "client-2", 1);

    let _ = runtime
        .advance_time_and_collect_fanout(10.0)
        .expect("periodic state ticks before timeout should encode");
    runtime
        .handle_line_fanout(
            "client-2",
            r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
        )
        .expect("ping-only update should refresh client timeout timestamp");

    let timeout_dispatch = runtime
        .advance_time_and_collect_dispatch(4.0)
        .expect("timeout tick should encode outbound fanout lines");
    let timeout_messages = decode_directed_lines(&timeout_dispatch.outbound_lines);

    assert!(
        runtime.session("client-1").is_none(),
        "stale client should be dropped after protocol timeout"
    );
    assert!(
        runtime.session("client-2").is_some(),
        "recently updated peer should remain connected"
    );
    assert!(
        has_user_event(&timeout_messages, "client-2", "alice", "left"),
        "peer should receive left event when stale client is dropped"
    );
    assert!(
        has_close_transport_action(&timeout_dispatch.transport_actions, "client-1"),
        "stale network clients should be closed after timeout"
    );
}
