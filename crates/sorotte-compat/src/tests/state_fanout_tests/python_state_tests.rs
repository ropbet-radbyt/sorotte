use super::*;

#[test]
fn python_fanout_roundtrip_matches_runtime_on_state_ping_forward_delay_metrics() {
    let steps = vec![
            ServerRuntimeScenarioStep {
                client_id: "client-1".to_owned(),
                request_line: r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#
                    .to_owned(),
                advance_seconds: 0.0,
                legacy_advance_seconds: None,
            },
            ServerRuntimeScenarioStep {
                client_id: "client-2".to_owned(),
                request_line: r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#
                    .to_owned(),
                advance_seconds: 0.0,
                legacy_advance_seconds: None,
            },
            ServerRuntimeScenarioStep {
                client_id: "client-1".to_owned(),
                request_line: r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":true},"ping":{"latencyCalculation":-10.0,"clientRtt":2.0}}}"#
                    .to_owned(),
                advance_seconds: 0.0,
                legacy_advance_seconds: None,
            },
        ];
    let rust_events = replay_server_runtime_scenario_steps(&steps)
        .expect("state ping-forward-delay scenario should replay through runtime");
    let python_events = match run_python_fanout_roundtrip(&steps) {
        Ok(events) => events,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => {
            panic!("python fanout interop for ping-forward-delay scenario failed: {err}")
        }
    };

    let rust_state_event = rust_events
        .get(2)
        .expect("runtime state step should exist")
        .outbound_lines
        .iter()
        .find(|line| line.client_id == "client-1")
        .expect("runtime sender output should exist");
    let python_state_event = python_events
        .get(2)
        .expect("python state step should exist")
        .outbound_lines
        .iter()
        .find(|line| line.client_id == "client-1")
        .expect("python sender output should exist");

    let rust_message =
        decode_message_line(&rust_state_event.line).expect("runtime sender output should decode");
    let python_message =
        decode_message_line(&python_state_event.line).expect("python sender output should decode");
    let ProtocolMessage::State(rust_payload) = rust_message else {
        panic!("runtime sender output should be state");
    };
    let ProtocolMessage::State(python_payload) = python_message else {
        panic!("python sender output should be state");
    };

    let rust_position = rust_payload
        .state
        .playstate
        .as_ref()
        .and_then(|playstate| playstate.position)
        .expect("runtime state should include playstate position");
    let python_position = python_payload
        .state
        .playstate
        .as_ref()
        .and_then(|playstate| playstate.position)
        .expect("python state should include playstate position");
    assert!(
        (rust_position - 18.0).abs() <= 0.000_001,
        "runtime should apply forward delay to position"
    );
    assert!(
        (python_position - 18.0).abs() <= 0.000_001,
        "python probe should apply forward delay to position"
    );

    let rust_server_rtt = rust_payload
        .state
        .ping
        .as_ref()
        .and_then(|ping| ping.server_rtt)
        .expect("runtime state should include serverRtt");
    let python_server_rtt = python_payload
        .state
        .ping
        .as_ref()
        .and_then(|ping| ping.server_rtt)
        .expect("python state should include serverRtt");
    assert!(
        (rust_server_rtt - 10.0).abs() <= 0.000_001,
        "runtime sender serverRtt should reflect ping RTT update"
    );
    assert!(
        (python_server_rtt - 10.0).abs() <= 0.000_001,
        "python sender serverRtt should reflect ping RTT update"
    );
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_fanout_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario("server_runtime_fanout.jsonl") {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!("python fanout interop should succeed, got: {err}"),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_tls_send_available_scenario() {
    let cert_path = temporary_tls_directory_path("tls-fanout-available");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let steps = vec![ServerRuntimeScenarioStep {
        client_id: "client-1".to_owned(),
        request_line: r#"{"TLS":{"startTLS":"send"}}"#.to_owned(),
        advance_seconds: 0.0,
        legacy_advance_seconds: None,
    }];

    let rust_events = {
        let mut runtime = ServerRuntime::new();
        runtime.set_tls_cert_path(Some(cert_path.clone()));
        runtime.set_time_now_override_seconds(Some(0.0));

        let mut events = Vec::new();
        for step in &steps {
            let mut outbound_lines = runtime
                .advance_time_and_collect_fanout(step.advance_seconds)
                .expect("runtime fanout tick should encode");
            outbound_lines.extend(
                runtime
                    .handle_line_fanout(&step.client_id, &step.request_line)
                    .expect("runtime step should succeed"),
            );
            events.push(super::ServerRuntimeScenarioEvent {
                client_id: step.client_id.clone(),
                request_line: step.request_line.clone(),
                outbound_lines,
            });
        }
        events
    };

    let python_events = match run_python_fanout_roundtrip_with_tls_available(&steps, true) {
        Ok(events) => events,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            let _ = fs::remove_dir_all(&cert_path);
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => panic!("python tls fanout interop should succeed, got: {err}"),
    };

    assert_eq!(python_events.len(), rust_events.len());
    for (python_event, rust_event) in python_events.iter().zip(rust_events.iter()) {
        assert_eq!(python_event.client_id, rust_event.client_id);
        assert_eq!(python_event.request_line, rust_event.request_line);
        assert_eq!(
            python_event.outbound_lines.len(),
            rust_event.outbound_lines.len()
        );
        for (python_output, rust_output) in python_event
            .outbound_lines
            .iter()
            .zip(rust_event.outbound_lines.iter())
        {
            assert_eq!(python_output.client_id, rust_output.client_id);
            let python_value = normalize_cross_impl_message(
                serde_json::from_str::<Value>(&python_output.line)
                    .expect("python output line should decode"),
            );
            let rust_value = normalize_cross_impl_message(
                serde_json::from_str::<Value>(&rust_output.line)
                    .expect("runtime output line should decode"),
            );
            assert_eq!(python_value, rust_value);
        }
    }

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_state_propagation_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_propagation.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for state propagation scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_state_metadata_forwarding_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_metadata_forwarding.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for state metadata forwarding scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_state_periodic_timeout_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_periodic_timeout.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for state periodic-timeout scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_state_latency_metrics_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_state_latency_metrics.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for state latency-metrics scenario should succeed, got: {err}"
        ),
    }
}
