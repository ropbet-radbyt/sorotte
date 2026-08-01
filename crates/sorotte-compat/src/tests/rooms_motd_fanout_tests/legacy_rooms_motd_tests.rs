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
fn legacy_permanent_room_snapshot_setter_alternate_is_context_exact() {
    let steps = load_server_runtime_scenario_fixture(PERMANENT_ROOMS_FILE_SCENARIO)
        .expect("permanent-room scenario should load");
    let request_line = &steps
        .get(8)
        .expect("Bob rejoin step should exist")
        .request_line;
    let alice_request_line = &steps
        .first()
        .expect("Alice initial Hello step should exist")
        .request_line;
    let other_room_request_line = request_line.replace("\"permanent-room\"", "\"other-room\"");
    let original = json!({
        "Set": {
            "playlistChange": {"files": ["episode.mkv"], "user": "bob"},
            "playlistIndex": {"index": 4, "user": "bob"},
        }
    });
    let mut canonical = original.clone();
    canonicalize_legacy_permanent_room_snapshot_setter(
        PERMANENT_ROOMS_FILE_SCENARIO,
        8,
        "client-3",
        request_line,
        "client-3",
        &mut canonical,
    );
    assert_eq!(
        canonical.pointer("/Set/playlistChange/user"),
        Some(&json!("alice"))
    );
    assert_eq!(
        canonical.pointer("/Set/playlistIndex/user"),
        Some(&json!("alice"))
    );
    assert_eq!(
        canonical.pointer("/Set/playlistChange/files"),
        Some(&json!(["episode.mkv"]))
    );
    assert_eq!(
        canonical.pointer("/Set/playlistIndex/index"),
        Some(&json!(4))
    );

    for (label, scenario, step, request_client, request, output_client) in [
        (
            "scenario",
            "server_runtime_state_propagation.jsonl",
            8,
            "client-3",
            request_line.as_str(),
            "client-3",
        ),
        (
            "step",
            PERMANENT_ROOMS_FILE_SCENARIO,
            7,
            "client-3",
            request_line.as_str(),
            "client-3",
        ),
        (
            "request client",
            PERMANENT_ROOMS_FILE_SCENARIO,
            8,
            "client-2",
            request_line.as_str(),
            "client-3",
        ),
        (
            "request payload",
            PERMANENT_ROOMS_FILE_SCENARIO,
            8,
            "client-3",
            alice_request_line.as_str(),
            "client-3",
        ),
        (
            "request room",
            PERMANENT_ROOMS_FILE_SCENARIO,
            8,
            "client-3",
            other_room_request_line.as_str(),
            "client-3",
        ),
        (
            "request message",
            PERMANENT_ROOMS_FILE_SCENARIO,
            8,
            "client-3",
            r#"{"Set":{"ready":{"isReady":true}}}"#,
            "client-3",
        ),
        (
            "output recipient",
            PERMANENT_ROOMS_FILE_SCENARIO,
            8,
            "client-3",
            request_line.as_str(),
            "client-2",
        ),
    ] {
        let mut candidate = original.clone();
        canonicalize_legacy_permanent_room_snapshot_setter(
            scenario,
            step,
            request_client,
            request,
            output_client,
            &mut candidate,
        );
        assert_eq!(candidate, original, "wrong {label} must not canonicalize");
    }

    let mut other_user = json!({
        "Set": {"playlistChange": {"files": [], "user": "mallory"}}
    });
    let expected_other_user = other_user.clone();
    canonicalize_legacy_permanent_room_snapshot_setter(
        PERMANENT_ROOMS_FILE_SCENARIO,
        8,
        "client-3",
        request_line,
        "client-3",
        &mut other_user,
    );
    assert_eq!(other_user, expected_other_user);
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
