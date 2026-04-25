use super::*;

#[test]
fn server_runtime_fanout_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace("server_runtime_fanout.python_trace.json");
}

#[test]
fn server_runtime_playlist_controller_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace("server_runtime_playlist_controller.python_trace.json");
}

#[test]
fn server_runtime_controlled_room_permissions_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace(
        "server_runtime_controlled_room_permissions.python_trace.json",
    );
}

#[test]
fn server_runtime_controlled_room_invalid_password_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace(
        "server_runtime_controlled_room_invalid_password.python_trace.json",
    );
}

#[test]
fn server_runtime_controlled_room_state_forced_correction_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace(
        "server_runtime_controlled_room_state_forced_correction.python_trace.json",
    );
}

#[test]
fn server_runtime_state_metadata_forwarding_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace(
        "server_runtime_state_metadata_forwarding.python_trace.json",
    );
}

#[test]
fn server_runtime_state_propagation_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace("server_runtime_state_propagation.python_trace.json");
}

#[test]
fn server_runtime_state_periodic_timeout_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace(
        "server_runtime_state_periodic_timeout.python_trace.json",
    );
}

#[test]
fn server_runtime_state_latency_metrics_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace("server_runtime_state_latency_metrics.python_trace.json");
}

#[test]
fn server_runtime_username_conflict_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace("server_runtime_username_conflict.python_trace.json");
}

#[test]
fn server_runtime_motd_template_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_motd_template(
        "server_runtime_motd_template.python_trace.json",
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    );
}

#[test]
fn server_runtime_motd_template_outdated_client_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_motd_template(
        "server_runtime_motd_template_outdated_client.python_trace.json",
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    );
}

#[test]
fn server_runtime_persistent_rooms_notice_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_overrides(
        "server_runtime_persistent_rooms_notice.python_trace.json",
        None,
        true,
    );
}

#[test]
fn server_runtime_persistent_rooms_lifecycle_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_overrides(
        "server_runtime_persistent_rooms_lifecycle.python_trace.json",
        None,
        true,
    );
}

#[test]
fn server_runtime_permanent_rooms_file_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_full_overrides(
        "server_runtime_permanent_rooms_file.python_trace.json",
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    );
}

#[test]
fn server_runtime_persistent_rooms_timeout_list_updates_matches_captured_python_trace_shape() {
    assert_runtime_matches_captured_trace_with_overrides(
        "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
        None,
        true,
    );
}
