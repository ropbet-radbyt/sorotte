use super::*;

/// A scenario-only client action. Resolve it before advancing the scenario
/// clock, so a delayed response echoes the challenge the client already saw.
/// The marker is never passed to either implementation's protocol decoder.
pub(crate) fn prepare_scenario_request_line(
    request_line: &str,
    client_id: &str,
    previous_events: &[ServerRuntimeScenarioEvent],
) -> Result<String, InteropError> {
    if !request_line.contains("$lastServerChallenge") && !request_line.contains("$serverChallenge:")
    {
        return Ok(request_line.to_owned());
    }
    let mut request: Value = serde_json::from_str(request_line)?;
    let marker = request
        .pointer("/State/ping/latencyCalculation")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let captured_events = if marker == "$lastServerChallenge" {
        previous_events
    } else if let Some(step) = marker.strip_prefix("$serverChallenge:") {
        let index = step
            .parse::<usize>()
            .ok()
            .and_then(|step| step.checked_sub(1))
            .filter(|index| *index < previous_events.len())
            .ok_or_else(|| {
                InteropError::InvalidScenarioStep(
                    "ping echo requires an earlier scenario step".to_owned(),
                )
            })?;
        &previous_events[index..=index]
    } else {
        return Ok(request_line.to_owned());
    };
    let challenge = captured_events
        .iter()
        .rev()
        .find_map(|event| {
            event.outbound_lines.iter().rev().find_map(|outbound| {
                if outbound.client_id != client_id {
                    return None;
                }
                let message: Value = serde_json::from_str(&outbound.line).ok()?;
                message
                    .pointer("/State/ping/latencyCalculation")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
            })
        })
        .ok_or_else(|| {
            InteropError::InvalidScenarioStep(
                "ping echo requires a previously captured challenge for this client".to_owned(),
            )
        })?;
    request["State"]["ping"]["latencyCalculation"] = Value::from(challenge);
    Ok(serde_json::to_string(&request)?)
}

pub fn replay_server_runtime_scenario_steps(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_motd_template(steps, None)
}

pub fn replay_server_runtime_scenario_steps_with_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_overrides(steps, motd_template, false)
}

pub fn replay_server_runtime_scenario_steps_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_full_overrides(
        steps,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

pub(crate) fn replay_server_runtime_scenario_steps_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    let mut runtime =
        ServerRuntime::with_room_password_salt(DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT);
    if let Some(template) = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
    {
        runtime.set_motd_template(Some(template.to_owned()));
    }
    runtime.set_persistent_rooms_enabled(persistent_rooms_enabled);
    let temporary_rooms_db_path = if persistent_rooms_enabled && !permanent_rooms.is_empty() {
        let path = create_temporary_legacy_rooms_db_file_path()?;
        runtime.set_persistent_rooms_db_path(Some(path.clone()))?;
        Some(path)
    } else {
        None
    };
    runtime.set_permanent_rooms(permanent_rooms.iter().copied().map(str::to_owned));
    runtime.set_time_now_override_seconds(Some(0.0));
    let result = (|| {
        let mut events = Vec::with_capacity(steps.len());
        for step in steps {
            let request_line =
                prepare_scenario_request_line(&step.request_line, &step.client_id, &events)?;
            let mut outbound_lines =
                runtime.advance_time_and_collect_fanout(step.advance_seconds)?;
            outbound_lines.extend(runtime.handle_line_fanout(&step.client_id, &request_line)?);
            events.push(ServerRuntimeScenarioEvent {
                client_id: step.client_id.clone(),
                request_line: step.request_line.clone(),
                outbound_lines,
            });
        }
        Ok(events)
    })();
    if let Some(path) = temporary_rooms_db_path {
        let _ = fs::remove_file(path);
    }
    result
}

pub fn replay_server_runtime_scenario_fixture(
    name: &str,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    let steps = load_server_runtime_scenario_fixture(name)?;
    replay_server_runtime_scenario_steps(&steps)
}

pub fn run_python_fanout_roundtrip(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_motd_template(steps, None)
}

pub fn run_python_fanout_roundtrip_with_tls_available(
    steps: &[ServerRuntimeScenarioStep],
    tls_available: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_full_overrides(steps, None, false, &[], tls_available)
}

pub fn run_python_fanout_roundtrip_with_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_overrides(steps, motd_template, false)
}

pub fn run_python_fanout_roundtrip_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_full_overrides(
        steps,
        motd_template,
        persistent_rooms_enabled,
        &[],
        false,
    )
}

pub(crate) fn run_python_fanout_roundtrip_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
    tls_available: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

    let payload = serde_json::to_vec(&json!({
        "events": steps
            .iter()
            .map(|step| json!({
                "client": step.client_id,
                "line": step.request_line,
                "advanceSeconds": step
                    .legacy_advance_seconds
                    .unwrap_or(step.advance_seconds),
            }))
            .collect::<Vec<_>>(),
    }))?;
    let stdout = run_python_probe_raw_with_overrides(
        &["--fanout-batch"],
        &payload,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        tls_available,
    )?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let output_sets = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for fanout response".to_owned(),
            )
        })?;

    if output_sets.len() != steps.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "fanout response count mismatch: expected {}, got {}",
            steps.len(),
            output_sets.len()
        )));
    }

    let mut events = Vec::with_capacity(steps.len());
    for (index, output_set) in output_sets.iter().enumerate() {
        let output_values = output_set.as_array().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "outputs[{index}] should be an array of directed outputs"
            ))
        })?;

        let mut outbound_lines = Vec::with_capacity(output_values.len());
        for output_value in output_values {
            let directed_client = output_value
                .get("client")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "outputs[{index}] entry is missing client field"
                    ))
                })?;
            let directed_message = output_value.get("message").ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "outputs[{index}] entry is missing message field"
                ))
            })?;
            let line = serde_json::to_string(directed_message)?;
            let _ = decode_message_line(&line)?;
            outbound_lines.push(DirectedOutboundLine {
                client_id: directed_client.to_owned(),
                line,
                delivery: ServerOutboundDelivery::Reliable,
            });
        }

        events.push(ServerRuntimeScenarioEvent {
            client_id: steps[index].client_id.clone(),
            request_line: steps[index].request_line.clone(),
            outbound_lines,
        });
    }

    Ok(events)
}

#[cfg(test)]
mod scenario_ping_echo_tests {
    use super::*;

    const ECHO: &str =
        r#"{"State":{"ping":{"latencyCalculation":"$lastServerChallenge","clientRtt":2}}}"#;

    #[test]
    fn client_echo_uses_only_its_latest_captured_challenge() {
        let events = vec![ServerRuntimeScenarioEvent {
            client_id: "requester".to_owned(),
            request_line: "{}".to_owned(),
            outbound_lines: [("alice", 89.0), ("alice", 90.0), ("bob", 99.0)]
                .into_iter()
                .map(|(client, challenge)| DirectedOutboundLine {
                    client_id: client.to_owned(),
                    line: json!({"State":{"ping":{"latencyCalculation":challenge}}}).to_string(),
                    delivery: ServerOutboundDelivery::Reliable,
                })
                .collect(),
        }];
        let prepared = prepare_scenario_request_line(ECHO, "alice", &events).unwrap();
        let value: Value = serde_json::from_str(&prepared).unwrap();
        assert_eq!(
            value.pointer("/State/ping/latencyCalculation"),
            Some(&json!(90.0))
        );
        assert_eq!(value.pointer("/State/ping/clientRtt"), Some(&json!(2)));
        assert!(decode_message_line(&prepared).is_ok());
        assert!(prepare_scenario_request_line(ECHO, "unknown", &events).is_err());
        assert!(prepare_scenario_request_line(ECHO, "alice", &[]).is_err());
        let selected = ECHO.replace("$lastServerChallenge", "$serverChallenge:1");
        assert_eq!(
            prepare_scenario_request_line(&selected, "alice", &events).unwrap(),
            prepared
        );
        for invalid in [
            "$serverChallenge:0",
            "$serverChallenge:2",
            "$serverChallenge:x",
        ] {
            assert!(
                prepare_scenario_request_line(
                    &ECHO.replace("$lastServerChallenge", invalid),
                    "alice",
                    &events
                )
                .is_err()
            );
        }
    }

    #[test]
    fn ordinary_and_malformed_wire_inputs_are_not_rewritten() {
        for line in [
            r#"{"State":{"ping":{"latencyCalculation":-10}}}"#,
            "not-json",
        ] {
            assert_eq!(
                prepare_scenario_request_line(line, "alice", &[]).unwrap(),
                line
            );
        }
    }
}
