use super::*;

pub(in crate::tests) fn assert_python_fanout_matches_server_runtime_for_scenario(
    scenario_name: &str,
) -> Result<(), InteropError> {
    assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name,
        None,
        None,
        false,
        false,
    )
}

pub(in crate::tests) fn assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    probe_motd_template: Option<&str>,
) -> Result<(), InteropError> {
    assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name,
        runtime_motd_template,
        probe_motd_template,
        false,
        false,
    )
}

pub(in crate::tests) fn assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    probe_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
    probe_persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        scenario_name,
        runtime_motd_template,
        probe_motd_template,
        runtime_persistent_rooms_enabled,
        probe_persistent_rooms_enabled,
        &[],
        &[],
    )
}

pub(in crate::tests) fn assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    probe_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
    probe_persistent_rooms_enabled: bool,
    runtime_permanent_rooms: &[&str],
    probe_permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let normalization_options = normalization_options_for_runtime_python_scenario(scenario_name);
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let rust_events = replay_server_runtime_scenario_steps_with_full_overrides(
        &steps,
        runtime_motd_template,
        runtime_persistent_rooms_enabled,
        runtime_permanent_rooms,
    )?;
    let python_events = run_python_fanout_roundtrip_with_full_overrides(
        &steps,
        probe_motd_template,
        probe_persistent_rooms_enabled,
        probe_permanent_rooms,
        false,
    )?;

    assert_eq!(python_events.len(), rust_events.len());
    for (index, (python_event, rust_event)) in
        python_events.iter().zip(rust_events.iter()).enumerate()
    {
        assert_eq!(
            python_event.client_id, rust_event.client_id,
            "request client mismatch at step {index}"
        );
        assert_eq!(
            python_event.request_line, rust_event.request_line,
            "request line mismatch at step {index}"
        );

        let python_outputs: Vec<_> = python_event
            .outbound_lines
            .iter()
            .map(|output| {
                let mut message = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&output.line)
                        .expect("python outbound line should decode as JSON"),
                    normalization_options,
                );
                canonicalize_legacy_hello_fields(&mut message);
                canonicalize_intentional_username_collision_divergence(
                    scenario_name,
                    true,
                    &mut message,
                );
                canonicalize_intentional_current_index_divergence(
                    scenario_name,
                    index,
                    true,
                    &mut message,
                );
                ComparableOutbound {
                    client_id: output.client_id.clone(),
                    message,
                }
            })
            .collect();
        let rust_outputs: Vec<_> = rust_event
            .outbound_lines
            .iter()
            .map(|output| {
                let mut message = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&output.line)
                        .expect("rust outbound line should decode as JSON"),
                    normalization_options,
                );
                canonicalize_legacy_hello_fields(&mut message);
                canonicalize_intentional_username_collision_divergence(
                    scenario_name,
                    false,
                    &mut message,
                );
                canonicalize_intentional_current_index_divergence(
                    scenario_name,
                    index,
                    false,
                    &mut message,
                );
                ComparableOutbound {
                    client_id: output.client_id.clone(),
                    message,
                }
            })
            .collect();
        let rust_outputs =
            without_unshared_runtime_playlist_index_normalizations(&python_outputs, &rust_outputs);

        assert_eq!(
            python_outputs.len(),
            rust_outputs.len(),
            "outbound count mismatch at step {index}\npython: {python_outputs:#?}\nrust: {rust_outputs:#?}"
        );

        for (output_index, (python_output, rust_output)) in
            python_outputs.iter().zip(rust_outputs.iter()).enumerate()
        {
            assert_eq!(
                python_output.client_id, rust_output.client_id,
                "outbound client mismatch at step {index} output {output_index}"
            );
            assert!(
                comparable_outbounds_match(
                    &python_outputs,
                    output_index,
                    &rust_outputs,
                    output_index,
                ),
                "outbound message mismatch at step {index} output {output_index}\npython: {:#?}\nrust: {:#?}",
                python_output.message,
                rust_output.message,
            );
        }
    }

    Ok(())
}
