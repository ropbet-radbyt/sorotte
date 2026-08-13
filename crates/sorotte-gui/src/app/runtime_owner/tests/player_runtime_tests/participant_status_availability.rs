use super::*;

use sorotte_client_core::ExternalPlayerAvailability;
use sorotte_player_api::{PlayerCapabilities, PlayerCapability};

struct TelemetryCapablePlayer;

impl PlayerAdapter for TelemetryCapablePlayer {
    fn name(&self) -> &'static str {
        "telemetry-capable"
    }

    fn capabilities(&self) -> PlayerCapabilities {
        PlayerCapabilities::from_capabilities([PlayerCapability::Telemetry])
    }
}

struct AvailabilityRecordingSession {
    observations: Arc<Mutex<Vec<ExternalPlayerAvailability>>>,
}

impl GuiSessionRuntimeAdapter for AvailabilityRecordingSession {
    fn set_external_player_availability(
        &mut self,
        availability: ExternalPlayerAvailability,
        _now_seconds: f64,
    ) -> Result<bool, String> {
        self.observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(availability);
        Ok(true)
    }

    fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
        Ok(())
    }

    fn connect_public_server(
        &mut self,
        _selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        Ok(current_servers)
    }

    fn search_missing_media(
        &mut self,
        _directories: Vec<String>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}

#[test]
fn runtime_owner_reports_player_state_on_session_handoff_and_detach() {
    let observations = Arc::new(Mutex::new(Vec::new()));
    let session = || {
        Box::new(AvailabilityRecordingSession {
            observations: observations.clone(),
        }) as Box<dyn GuiSessionRuntimeAdapter + Send>
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryCapablePlayer)));
    owner.install_session_runtime(session());
    owner.detach_player();

    owner.remove_session_runtime();
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
        ]
    );
}
