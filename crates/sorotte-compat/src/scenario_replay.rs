use super::*;

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
            let mut outbound_lines =
                runtime.advance_time_and_collect_fanout(step.advance_seconds)?;
            outbound_lines.extend(runtime.handle_line_fanout(&step.client_id, &step.request_line)?);
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
