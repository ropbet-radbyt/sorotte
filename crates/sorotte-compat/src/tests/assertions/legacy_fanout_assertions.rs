use super::*;

pub(in crate::tests) fn assert_legacy_server_fanout_matches_server_runtime_for_scenario(
    scenario_name: &str,
) -> Result<(), InteropError> {
    assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name,
        None,
        None,
        false,
        false,
    )
}

pub(in crate::tests) fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    legacy_motd_template: Option<&str>,
) -> Result<(), InteropError> {
    assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name,
        runtime_motd_template,
        legacy_motd_template,
        false,
        false,
    )
}

pub(in crate::tests) fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    legacy_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
    legacy_persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        scenario_name,
        runtime_motd_template,
        legacy_motd_template,
        runtime_persistent_rooms_enabled,
        legacy_persistent_rooms_enabled,
        &[],
        &[],
    )
}

pub(in crate::tests) fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
    scenario_name: &str,
    runtime_motd_template: Option<&str>,
    legacy_motd_template: Option<&str>,
    runtime_persistent_rooms_enabled: bool,
    legacy_persistent_rooms_enabled: bool,
    runtime_permanent_rooms: &[&str],
    legacy_permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let normalization_options = normalization_options_for_legacy_scenario(scenario_name);
    let mut timing_canonicalizer = LegacyTimingCanonicalizer::default();
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let rust_events = replay_server_runtime_scenario_steps_with_full_overrides(
        &steps,
        runtime_motd_template,
        runtime_persistent_rooms_enabled,
        runtime_permanent_rooms,
    )?;
    let legacy_events = run_legacy_server_fanout_roundtrip_with_full_overrides(
        &steps,
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        legacy_motd_template,
        legacy_persistent_rooms_enabled,
        legacy_permanent_rooms,
    )?;

    assert_eq!(legacy_events.len(), rust_events.len());
    for (index, (legacy_event, rust_event)) in
        legacy_events.iter().zip(rust_events.iter()).enumerate()
    {
        let mut legacy_outputs: Vec<(String, Value)> = Vec::new();
        for output in &legacy_event.outbound_lines {
            let include_output = decode_message_line(&output.line)
                .ok()
                .is_some_and(|message| !is_background_idle_state_message(&message));
            if !include_output {
                continue;
            }

            let mut normalized = normalize_cross_impl_message_with_options(
                serde_json::from_str(&output.line)
                    .expect("legacy outbound line should decode as JSON"),
                normalization_options,
            );
            timing_canonicalizer.canonicalize_message(&mut normalized, LegacyTimingSide::Legacy);
            canonicalize_legacy_hello_fields(&mut normalized);
            canonicalize_legacy_set_user_features(&mut normalized);
            canonicalize_legacy_list_fields(&mut normalized);
            legacy_outputs.push((output.client_id.clone(), normalized));
        }

        let mut rust_outputs: Vec<(String, Value)> = Vec::new();
        for output in &rust_event.outbound_lines {
            let include_output = decode_message_line(&output.line)
                .ok()
                .is_some_and(|message| !is_background_idle_state_message(&message));
            if !include_output {
                continue;
            }

            let mut normalized = normalize_cross_impl_message_with_options(
                serde_json::from_str(&output.line)
                    .expect("rust outbound line should decode as JSON"),
                normalization_options,
            );
            timing_canonicalizer.canonicalize_message(&mut normalized, LegacyTimingSide::Runtime);
            canonicalize_legacy_hello_fields(&mut normalized);
            canonicalize_legacy_set_user_features(&mut normalized);
            canonicalize_legacy_list_fields(&mut normalized);
            rust_outputs.push((output.client_id.clone(), normalized));
        }
        assert_eq!(
            legacy_event.client_id, rust_event.client_id,
            "request client mismatch at step {index}"
        );
        assert_eq!(
            legacy_event.request_line, rust_event.request_line,
            "request line mismatch at step {index}"
        );
        let mut legacy_sequences = BTreeMap::<String, Vec<Value>>::new();
        for (client_id, message) in legacy_outputs {
            legacy_sequences.entry(client_id).or_default().push(message);
        }
        let mut rust_sequences = BTreeMap::<String, Vec<Value>>::new();
        for (client_id, message) in rust_outputs {
            rust_sequences.entry(client_id).or_default().push(message);
        }

        // Socket polling can interleave different recipients nondeterministically,
        // but every recipient observes a defined protocol sequence. Comparing each
        // recipient's complete sequence catches message-order regressions without
        // inventing a cross-socket total order.
        assert_eq!(
            legacy_sequences, rust_sequences,
            "per-recipient outbound sequence mismatch at step {index}"
        );
    }

    Ok(())
}
