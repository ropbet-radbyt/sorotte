use super::*;

use sorotte_client_core::ExternalPlayerAvailability;

struct TelemetryCapablePlayer;

impl PlayerAdapter for TelemetryCapablePlayer {
    fn name(&self) -> &'static str {
        "telemetry-capable"
    }

    fn supports_transport_telemetry(&self) -> bool {
        true
    }
}

struct AvailabilityProbe {
    observations: Arc<Mutex<Vec<ExternalPlayerAvailability>>>,
}

impl AvailabilityProbe {
    fn into_session(self) -> crate::app::GuiClientSession {
        crate::app::runtime_stack::test_support::active_session().with_observer(move |event| {
            if let crate::app::runtime_stack::test_support::SessionObservation::Availability(
                value,
            ) = event
            {
                self.observations.lock().unwrap().push(value);
            }
        })
    }
}

#[test]
fn runtime_owner_reports_player_state_on_session_handoff_and_detach() {
    let observations = Arc::new(Mutex::new(Vec::new()));
    let session = || {
        Box::new(
            AvailabilityProbe {
                observations: observations.clone(),
            }
            .into_session(),
        )
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryCapablePlayer)));
    owner.install_session_runtime(session());
    owner.detach_player();

    owner.remove_session_runtime();
    owner.install_session_runtime(session());

    owner.remove_session_runtime();
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        sorotte_player_api::DisconnectedPlayer,
    )));
    owner.install_session_runtime(session());

    owner.remove_session_runtime();
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.install_session_runtime(session());

    assert_eq!(
        *observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![
            ExternalPlayerAvailability::Connecting,
            ExternalPlayerAvailability::Disconnected,
            ExternalPlayerAvailability::Unavailable,
            ExternalPlayerAvailability::TelemetryUnavailable,
            ExternalPlayerAvailability::Connecting,
        ]
    );
}
