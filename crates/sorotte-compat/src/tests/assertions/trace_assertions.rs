use super::*;

fn is_stale_captured_join_idle_state(output: &Value) -> bool {
    let Some(message) = output.get("message") else {
        return false;
    };
    let Some(state) = message.get("State").and_then(Value::as_object) else {
        return false;
    };
    if state.contains_key("ignoringOnTheFly") {
        return false;
    }
    let Some(playstate) = state.get("playstate").and_then(Value::as_object) else {
        return false;
    };
    playstate.get("setBy").is_none_or(Value::is_null)
        && playstate
            .get("position")
            .and_then(Value::as_f64)
            .is_some_and(|position| position.abs() <= f64::EPSILON)
        && playstate.get("paused") == Some(&Value::Bool(true))
        && playstate.get("doSeek") == Some(&Value::Bool(false))
}

fn is_null_playlist_index_snapshot_message(message: &Value) -> bool {
    message
        .get("Set")
        .and_then(|set| set.get("playlistIndex"))
        .and_then(Value::as_object)
        .and_then(|playlist_index| playlist_index.get("index"))
        .is_some_and(Value::is_null)
}

fn is_null_playlist_index_snapshot_output(output: &Value) -> bool {
    output
        .get("message")
        .is_some_and(is_null_playlist_index_snapshot_message)
}

fn is_null_playlist_index_snapshot_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|message| is_null_playlist_index_snapshot_message(&message))
}

fn is_state_message(message: &Value) -> bool {
    message.get("State").is_some()
}

fn is_state_output(output: &Value) -> bool {
    output.get("message").is_some_and(is_state_message)
}

fn is_state_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|message| is_state_message(&message))
}

fn trace_allows_periodic_state_count_drift(scenario_name: &str) -> bool {
    matches!(
        scenario_name,
        "server_runtime_state_periodic_timeout"
            | "server_runtime_state_periodic_timeout.jsonl"
            | "server_runtime_persistent_rooms_timeout_list_updates"
            | "server_runtime_persistent_rooms_timeout_list_updates.jsonl"
    )
}

fn filtered_expected_trace_outputs<'a>(
    scenario_name: &str,
    expected_outputs: &'a [Value],
) -> Vec<&'a Value> {
    let step_has_hello = expected_outputs.iter().any(|output| {
        output
            .get("message")
            .is_some_and(|message| message.get("Hello").is_some())
    });
    let filter_state_outputs = trace_allows_periodic_state_count_drift(scenario_name);
    expected_outputs
        .iter()
        .filter(|output| {
            !(is_null_playlist_index_snapshot_output(output)
                || step_has_hello && is_stale_captured_join_idle_state(output)
                || filter_state_outputs && is_state_output(output))
        })
        .collect()
}

pub(in crate::tests) fn assert_runtime_matches_captured_trace(trace_fixture_name: &str) {
    assert_runtime_matches_captured_trace_with_overrides(trace_fixture_name, None, false);
}

pub(in crate::tests) fn assert_runtime_matches_captured_trace_with_motd_template(
    trace_fixture_name: &str,
    runtime_motd_template: Option<&str>,
) {
    assert_runtime_matches_captured_trace_with_overrides(
        trace_fixture_name,
        runtime_motd_template,
        false,
    );
}

pub(in crate::tests) fn assert_runtime_matches_captured_trace_with_overrides(
    trace_fixture_name: &str,
    runtime_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
) {
    assert_runtime_matches_captured_trace_with_full_overrides(
        trace_fixture_name,
        runtime_motd_template,
        runtime_persistent_rooms_enabled,
        &[],
    );
}

pub(in crate::tests) fn assert_runtime_matches_captured_trace_with_full_overrides(
    trace_fixture_name: &str,
    runtime_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
    runtime_permanent_rooms: &[&str],
) {
    let expected_path = scenario_fixture_path(trace_fixture_name);
    let expected_value: Value = serde_json::from_str(
        &std::fs::read_to_string(&expected_path)
            .expect("expected parity trace fixture should be readable"),
    )
    .expect("expected parity trace fixture should be valid JSON");

    let scenario_name = expected_value
        .get("scenario")
        .and_then(Value::as_str)
        .expect("expected trace fixture should contain scenario field");
    let normalization_options = normalization_options_for_runtime_trace_scenario(scenario_name);
    let steps = load_server_runtime_scenario_fixture(scenario_name)
        .expect("scenario fixture should be readable for runtime trace comparison");
    let events = replay_server_runtime_scenario_steps_with_full_overrides(
        &steps,
        runtime_motd_template,
        runtime_persistent_rooms_enabled,
        runtime_permanent_rooms,
    )
    .expect("scenario should replay through server runtime");

    let expected_steps = expected_value
        .get("steps")
        .and_then(Value::as_array)
        .expect("expected trace fixture should contain steps array");
    assert_eq!(
        events.len(),
        expected_steps.len(),
        "scenario step count mismatch for captured trace fixture '{trace_fixture_name}'"
    );

    for expected_step in expected_steps {
        let step_number = expected_step
            .get("step")
            .and_then(Value::as_u64)
            .expect("expected step should contain numeric step") as usize;
        let expected_outputs = expected_step
            .get("outputs")
            .and_then(Value::as_array)
            .expect("expected step should contain outputs array");
        let expected_outputs = filtered_expected_trace_outputs(scenario_name, expected_outputs);
        let actual_event = events
            .get(step_number - 1)
            .expect("expected step index should exist in replay output");
        let filter_state_outputs = trace_allows_periodic_state_count_drift(scenario_name);
        let actual_outputs: Vec<_> = actual_event
            .outbound_lines
            .iter()
            .filter(|line| {
                !(is_null_playlist_index_snapshot_line(&line.line)
                    || filter_state_outputs && is_state_line(&line.line))
            })
            .collect();

        assert_eq!(
            actual_outputs.len(),
            expected_outputs.len(),
            "mismatch in outbound count at scenario step {step_number}"
        );

        for (index, (expected_output, actual_output)) in expected_outputs
            .iter()
            .zip(actual_outputs.iter())
            .enumerate()
        {
            let expected_client = expected_output
                .get("client")
                .and_then(Value::as_str)
                .expect("expected output should contain client");
            let expected_message = expected_output
                .get("message")
                .expect("expected output should contain message");

            let mut actual_message: Value = normalize_cross_impl_message_with_options(
                serde_json::from_str(&actual_output.line)
                    .expect("actual outbound line should decode to JSON value"),
                normalization_options,
            );
            let mut expected_message = normalize_cross_impl_message_with_options(
                expected_message.clone(),
                normalization_options,
            );
            canonicalize_legacy_hello_fields(&mut actual_message);
            canonicalize_legacy_hello_fields(&mut expected_message);

            assert_eq!(
                actual_output.client_id, expected_client,
                "mismatch in routed client at step {step_number} output {index}"
            );
            assert_eq!(
                actual_message, expected_message,
                "mismatch in message shape/order at step {step_number} output {index}"
            );
        }
    }
}
