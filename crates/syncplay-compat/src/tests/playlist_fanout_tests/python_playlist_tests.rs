use super::*;

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_controller_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_playlist_controller.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist/controller scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_cross_room_ready_list_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        "server_runtime_cross_room_ready_list.jsonl",
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for cross-room ready/list scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_room_switch_peer_transition_scoping_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist room-switch peer-transition scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_double_room_switch_scoping_scenario()
{
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist double-room-switch scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_room_switch_snapshot_then_destination_update_ordering_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_UPDATE_ORDERING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist room-switch snapshot/update-ordering scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_room_switch_snapshot_then_old_room_update_ordering_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_OLD_ROOM_UPDATE_ORDERING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist room-switch snapshot/old-room-update-ordering scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_room_switch_snapshot_then_old_then_destination_update_ordering_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_OLD_THEN_DESTINATION_UPDATE_ORDERING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist room-switch snapshot/old-then-destination-update-ordering scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_playlist_room_switch_snapshot_then_destination_then_old_update_ordering_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_THEN_OLD_UPDATE_ORDERING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for playlist room-switch snapshot/destination-then-old-update-ordering scenario should succeed, got: {err}"
        ),
    }
}
