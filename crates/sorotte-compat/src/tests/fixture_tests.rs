use super::*;

#[test]
fn protocol_hello_fixture_decodes() {
    assert!(fixture_decodes("hello_minimal.json"));
}

#[test]
fn legacy_syncplay_checkout_dir_defaults_to_repo_local_cache() {
    if std::env::var_os("SYNCPLAY_LEGACY_ROOT").is_some() {
        return;
    }

    assert!(
        super::legacy_syncplay_checkout_dir()
            .ends_with(Path::new(".interop-cache").join("syncplay-legacy"))
    );
}

#[test]
fn all_protocol_fixtures_decode() {
    let fixtures = all_protocol_fixture_names().expect("fixture names should be available");
    assert!(!fixtures.is_empty());

    for fixture in fixtures {
        assert!(
            fixture_decodes(&fixture),
            "expected fixture {fixture} to decode as protocol message"
        );
    }
}

#[test]
fn fixture_decode_returns_typed_message() {
    let message = decode_fixture("tls_send.json").expect("tls fixture should decode");
    assert!(matches!(message, ProtocolMessage::Tls(_)));
}

#[test]
fn decode_protocol_file_works_for_existing_fixture() {
    let path = super::fixture_path("error_message.json");
    assert!(decode_protocol_file(path.as_path()));
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "requires Twisted and writes fixture files from a live legacy server session"]
fn capture_legacy_server_state_latency_metrics_trace_fixture() {
    capture_legacy_server_trace_fixture(
        "server_runtime_state_latency_metrics.jsonl",
        "server_runtime_state_latency_metrics.legacy_trace.json",
    )
    .expect("state latency-metrics legacy trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "writes python fanout trace fixtures from current probe behavior"]
fn capture_python_state_latency_metrics_trace_fixture() {
    capture_python_trace_fixture(
        "server_runtime_state_latency_metrics.jsonl",
        "server_runtime_state_latency_metrics.python_trace.json",
    )
    .expect("state latency-metrics python trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "writes persistent-room lifecycle python/legacy trace fixtures"]
fn capture_persistent_rooms_lifecycle_trace_fixtures() {
    capture_python_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        "server_runtime_persistent_rooms_lifecycle.python_trace.json",
        None,
        true,
    )
    .expect("persistent-rooms lifecycle python trace capture should succeed");
    capture_legacy_server_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        "server_runtime_persistent_rooms_lifecycle.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
    )
    .expect("persistent-rooms lifecycle legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        "server_runtime_persistent_rooms_timeout_list_updates.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
    )
    .expect("persistent timeout-list-updates legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        "server_runtime_permanent_rooms_file.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    )
    .expect("permanent-rooms-file legacy trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "writes permanent-rooms-file python/legacy trace fixtures"]
fn capture_permanent_rooms_file_trace_fixtures() {
    capture_python_trace_fixture_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        "server_runtime_permanent_rooms_file.python_trace.json",
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    )
    .expect("permanent-rooms-file python trace capture should succeed");
    capture_legacy_server_trace_fixture_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        "server_runtime_permanent_rooms_file.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    )
    .expect("permanent-rooms-file legacy trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "requires Twisted and writes fixture files from a live legacy server session"]
fn capture_legacy_server_controlled_room_trace_fixtures() {
    capture_legacy_server_trace_fixture(
        "server_runtime_controlled_room_permissions.jsonl",
        "server_runtime_controlled_room_permissions.legacy_trace.json",
    )
    .expect("controlled-room permissions legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_controlled_room_invalid_password.jsonl",
        "server_runtime_controlled_room_invalid_password.legacy_trace.json",
    )
    .expect("controlled-room invalid-password legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_controlled_room_state_forced_correction.jsonl",
        "server_runtime_controlled_room_state_forced_correction.legacy_trace.json",
    )
    .expect("controlled-room forced-correction legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_state_propagation.jsonl",
        "server_runtime_state_propagation.legacy_trace.json",
    )
    .expect("state propagation legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_state_metadata_forwarding.jsonl",
        "server_runtime_state_metadata_forwarding.legacy_trace.json",
    )
    .expect("state metadata forwarding legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_state_periodic_timeout.jsonl",
        "server_runtime_state_periodic_timeout.legacy_trace.json",
    )
    .expect("state periodic-timeout legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_state_latency_metrics.jsonl",
        "server_runtime_state_latency_metrics.legacy_trace.json",
    )
    .expect("state latency-metrics legacy trace capture should succeed");
    capture_legacy_server_trace_fixture(
        "server_runtime_username_conflict.jsonl",
        "server_runtime_username_conflict.legacy_trace.json",
    )
    .expect("username conflict legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_salt_and_motd_template(
        MOTD_TEMPLATE_SCENARIO,
        "server_runtime_motd_template.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        Some(MOTD_TEMPLATE_LEGACY_FILE),
    )
    .expect("motd-template legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_salt_and_motd_template(
        MOTD_TEMPLATE_OUTDATED_SCENARIO,
        "server_runtime_motd_template_outdated_client.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        Some(MOTD_TEMPLATE_LEGACY_FILE),
    )
    .expect("motd-template outdated-client legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_NOTICE_SCENARIO,
        "server_runtime_persistent_rooms_notice.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
    )
    .expect("persistent-rooms notice legacy trace capture should succeed");
    capture_legacy_server_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        "server_runtime_persistent_rooms_lifecycle.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
    )
    .expect("persistent-rooms lifecycle legacy trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "writes persistent timeout-list-updates python/legacy trace fixtures"]
fn capture_persistent_rooms_timeout_list_updates_trace_fixtures() {
    capture_python_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
        None,
        true,
    )
    .expect("persistent timeout-list-updates python trace capture should succeed");
    capture_legacy_server_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        "server_runtime_persistent_rooms_timeout_list_updates.legacy_trace.json",
        super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
        None,
        true,
    )
    .expect("persistent timeout-list-updates legacy trace capture should succeed");
}

#[cfg(feature = "trace-capture")]
#[test]
#[ignore = "writes python fanout trace fixtures from current probe behavior"]
fn capture_python_fanout_trace_fixtures() {
    capture_python_trace_fixture(
        "server_runtime_fanout.jsonl",
        "server_runtime_fanout.python_trace.json",
    )
    .expect("fanout python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_playlist_controller.jsonl",
        "server_runtime_playlist_controller.python_trace.json",
    )
    .expect("playlist/controller python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_cross_room_ready_list.jsonl",
        "server_runtime_cross_room_ready_list.python_trace.json",
    )
    .expect("cross-room ready/list python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_controlled_room_permissions.jsonl",
        "server_runtime_controlled_room_permissions.python_trace.json",
    )
    .expect("controlled-room permissions python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_controlled_room_invalid_password.jsonl",
        "server_runtime_controlled_room_invalid_password.python_trace.json",
    )
    .expect("controlled-room invalid-password python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_controlled_room_state_forced_correction.jsonl",
        "server_runtime_controlled_room_state_forced_correction.python_trace.json",
    )
    .expect("controlled-room forced-correction python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_state_propagation.jsonl",
        "server_runtime_state_propagation.python_trace.json",
    )
    .expect("state propagation python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_state_metadata_forwarding.jsonl",
        "server_runtime_state_metadata_forwarding.python_trace.json",
    )
    .expect("state metadata forwarding python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_state_periodic_timeout.jsonl",
        "server_runtime_state_periodic_timeout.python_trace.json",
    )
    .expect("state periodic-timeout python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_state_latency_metrics.jsonl",
        "server_runtime_state_latency_metrics.python_trace.json",
    )
    .expect("state latency-metrics python trace capture should succeed");
    capture_python_trace_fixture(
        "server_runtime_username_conflict.jsonl",
        "server_runtime_username_conflict.python_trace.json",
    )
    .expect("username conflict python trace capture should succeed");
    capture_python_trace_fixture_with_motd_template(
        MOTD_TEMPLATE_SCENARIO,
        "server_runtime_motd_template.python_trace.json",
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    )
    .expect("motd-template python trace capture should succeed");
    capture_python_trace_fixture_with_motd_template(
        MOTD_TEMPLATE_OUTDATED_SCENARIO,
        "server_runtime_motd_template_outdated_client.python_trace.json",
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    )
    .expect("motd-template outdated-client python trace capture should succeed");
    capture_python_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_NOTICE_SCENARIO,
        "server_runtime_persistent_rooms_notice.python_trace.json",
        None,
        true,
    )
    .expect("persistent-rooms notice python trace capture should succeed");
    capture_python_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        "server_runtime_persistent_rooms_lifecycle.python_trace.json",
        None,
        true,
    )
    .expect("persistent-rooms lifecycle python trace capture should succeed");
    capture_python_trace_fixture_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
        None,
        true,
    )
    .expect("persistent timeout-list-updates python trace capture should succeed");
    capture_python_trace_fixture_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        "server_runtime_permanent_rooms_file.python_trace.json",
        None,
        true,
        PERMANENT_ROOMS_FILE_LIST,
    )
    .expect("permanent-rooms-file python trace capture should succeed");
}
