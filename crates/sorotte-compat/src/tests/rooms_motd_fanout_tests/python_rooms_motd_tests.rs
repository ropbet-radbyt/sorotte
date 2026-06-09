use super::*;

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_username_conflict_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_username_conflict.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for username conflict scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_motd_template_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
        MOTD_TEMPLATE_SCENARIO,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => {
            panic!("python fanout interop for motd-template scenario should succeed, got: {err}")
        }
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_motd_template_outdated_client_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
        MOTD_TEMPLATE_OUTDATED_SCENARIO,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for motd-template outdated-client scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_notice_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_NOTICE_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for persistent-rooms notice scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_lifecycle_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for persistent-rooms lifecycle scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        None,
        None,
        true,
        true,
        PERMANENT_ROOMS_FILE_LIST,
        PERMANENT_ROOMS_FILE_LIST,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for permanent-rooms-file scenario should succeed, got: {err}"
        ),
    }
}

#[test]
#[ignore = "Sorotte intentionally extends the protocol timeout beyond the Python reference for media-match liveness"]
fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_timeout_list_updates_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for persistent timeout-list-updates scenario should succeed, got: {err}"
        ),
    }
}
