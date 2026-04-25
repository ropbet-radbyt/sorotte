use super::*;
use crate::legacy_server::run_legacy_server_fanout_roundtrip_with_full_overrides;
use crate::scenario_replay::run_python_fanout_roundtrip_with_full_overrides;

#[cfg(feature = "trace-capture")]
pub fn capture_legacy_server_trace_fixture(
    scenario_name: &str,
    trace_fixture_name: &str,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_salt(
        scenario_name,
        trace_fixture_name,
        DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
    )
}

#[cfg(feature = "trace-capture")]
pub fn capture_legacy_server_trace_fixture_with_salt(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_salt_and_motd_template(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        None,
    )
}

#[cfg(feature = "trace-capture")]
pub fn capture_legacy_server_trace_fixture_with_salt_and_motd_template(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_overrides(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        motd_template,
        false,
    )
}

#[cfg(feature = "trace-capture")]
pub fn capture_legacy_server_trace_fixture_with_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_full_overrides(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

#[cfg(feature = "trace-capture")]
pub(crate) fn capture_legacy_server_trace_fixture_with_full_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let events = run_legacy_server_fanout_roundtrip_with_full_overrides(
        &steps,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
    )?;
    let trace_value = scenario_events_to_trace_fixture_value(scenario_name, &events)?;
    fs::write(
        scenario_fixture_path(trace_fixture_name),
        format!("{}\n", serde_json::to_string_pretty(&trace_value)?),
    )?;
    Ok(())
}

#[cfg(feature = "trace-capture")]
pub fn capture_python_trace_fixture(
    scenario_name: &str,
    trace_fixture_name: &str,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_motd_template(scenario_name, trace_fixture_name, None)
}

#[cfg(feature = "trace-capture")]
pub fn capture_python_trace_fixture_with_motd_template(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_overrides(
        scenario_name,
        trace_fixture_name,
        motd_template,
        false,
    )
}

#[cfg(feature = "trace-capture")]
pub fn capture_python_trace_fixture_with_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_full_overrides(
        scenario_name,
        trace_fixture_name,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

#[cfg(feature = "trace-capture")]
pub(crate) fn capture_python_trace_fixture_with_full_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let events = run_python_fanout_roundtrip_with_full_overrides(
        &steps,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        false,
    )?;
    let trace_value = scenario_events_to_trace_fixture_value(scenario_name, &events)?;
    fs::write(
        scenario_fixture_path(trace_fixture_name),
        format!("{}\n", serde_json::to_string_pretty(&trace_value)?),
    )?;
    Ok(())
}

#[cfg(feature = "trace-capture")]
pub(crate) fn scenario_events_to_trace_fixture_value(
    scenario_name: &str,
    events: &[ServerRuntimeScenarioEvent],
) -> Result<Value, InteropError> {
    let mut steps = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let mut outputs = Vec::with_capacity(event.outbound_lines.len());
        for outbound in &event.outbound_lines {
            outputs.push(json!({
                "client": outbound.client_id,
                "message": serde_json::from_str::<Value>(&outbound.line)?,
            }));
        }
        steps.push(json!({
            "step": index + 1,
            "outputs": outputs,
        }));
    }
    Ok(json!({
        "scenario": scenario_name,
        "steps": steps,
    }))
}
