use super::*;

pub(in crate::tests) fn is_background_idle_state_message(message: &ProtocolMessage) -> bool {
    match message {
        ProtocolMessage::State(payload) => {
            payload.state.playstate.as_ref().is_some_and(|playstate| {
                playstate.paused == Some(true)
                    && playstate.do_seek != Some(true)
                    && playstate
                        .position
                        .is_some_and(|position| position.abs() <= 0.01)
            })
        }
        _ => false,
    }
}

pub(in crate::tests) fn legacy_server_prerequisites_missing(error: &InteropError) -> bool {
    if required_live_interop_enabled() {
        return false;
    }
    match error {
        InteropError::LegacySyncplayCheckoutMissing(_) | InteropError::PythonSpawn { .. } => true,
        InteropError::LegacyServerExited { stderr, .. }
        | InteropError::LegacyServerStartTimeout { stderr, .. } => {
            let lowered = stderr.to_ascii_lowercase();
            lowered.contains("no module named 'twisted'")
                || lowered.contains("unable import twisted")
                || lowered.contains("unable to import twisted")
        }
        _ => false,
    }
}

pub(in crate::tests) fn legacy_server_parity_assertions_enabled() -> bool {
    required_live_interop_enabled()
        || std::env::var("SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY")
            .ok()
            .is_some_and(|value| {
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
            })
}

pub(in crate::tests) fn legacy_tls_parity_prerequisites_strict_enabled() -> bool {
    required_live_interop_enabled()
        || std::env::var("SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY")
            .ok()
            .is_some_and(|value| {
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
            })
}
