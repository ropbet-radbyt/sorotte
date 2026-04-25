use super::*;

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
        let actual_event = events
            .get(step_number - 1)
            .expect("expected step index should exist in replay output");

        assert_eq!(
            actual_event.outbound_lines.len(),
            expected_outputs.len(),
            "mismatch in outbound count at scenario step {step_number}"
        );

        for (index, expected_output) in expected_outputs.iter().enumerate() {
            let expected_client = expected_output
                .get("client")
                .and_then(Value::as_str)
                .expect("expected output should contain client");
            let expected_message = expected_output
                .get("message")
                .expect("expected output should contain message");

            let actual_output = &actual_event.outbound_lines[index];
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
