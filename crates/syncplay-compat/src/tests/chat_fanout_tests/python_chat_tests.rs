use super::*;

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_room_scoping_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(CHAT_ROOM_SCOPING_SCENARIO) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat room-scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_room_switch_sender_scoping_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_ROOM_SWITCH_SENDER_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat room-switch sender-scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_room_switch_peer_transition_scoping_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat room-switch peer-transition scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_room_switch_object_payload_scoping_scenario()
 {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_ROOM_SWITCH_OBJECT_PAYLOAD_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat room-switch object-payload scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_double_room_switch_scoping_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat double-room-switch scoping scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_username_normalization_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_USERNAME_NORMALIZATION_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat username-normalization scenario should succeed, got: {err}"
        ),
    }
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_chat_payload_normalization_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CHAT_PAYLOAD_NORMALIZATION_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for chat payload-normalization scenario should succeed, got: {err}"
        ),
    }
}
