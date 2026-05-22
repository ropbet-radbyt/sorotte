use super::*;

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_username_conflict_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        "server_runtime_username_conflict.jsonl",
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for username conflict scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_motd_template_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
        MOTD_TEMPLATE_SCENARIO,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        Some(MOTD_TEMPLATE_LEGACY_FILE),
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for motd-template scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_motd_template_outdated_client_scenario()
{
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
        MOTD_TEMPLATE_OUTDATED_SCENARIO,
        Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        Some(MOTD_TEMPLATE_LEGACY_FILE),
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for motd-template outdated-client scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_notice_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_NOTICE_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for persistent-rooms notice scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_lifecycle_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for persistent-rooms lifecycle scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        PERMANENT_ROOMS_FILE_SCENARIO,
        None,
        None,
        true,
        true,
        PERMANENT_ROOMS_FILE_LIST,
        PERMANENT_ROOMS_FILE_LIST,
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for permanent-rooms-file scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_timeout_list_updates_scenario()
 {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }
    match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
        None,
        None,
        true,
        true,
    ) {
        Ok(()) => {}
        Err(err) if legacy_server_prerequisites_missing(&err) => {
            eprintln!(
                "legacy server fanout interop test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy server fanout interop for persistent timeout-list-updates scenario should succeed, got: {err}"
        ),
    }
}
