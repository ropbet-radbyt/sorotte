use super::*;

#[derive(Debug, Default)]
struct PositionSessionState {
    local_position_seconds: Option<f64>,
    synchronized_positions: Vec<f64>,
    recorded_manual_seeks: Vec<f64>,
}

struct PositionSession {
    state: std::sync::Arc<std::sync::Mutex<PositionSessionState>>,
}

impl GuiSessionRuntimeAdapter for PositionSession {
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

    fn sync_local_playback_telemetry(
        &mut self,
        _paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        if let Some(position_seconds) = position_seconds {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.local_position_seconds = Some(position_seconds);
            state.synchronized_positions.push(position_seconds);
        }
        Ok(())
    }

    fn record_manual_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.local_position_seconds = Some(position_seconds);
        state.recorded_manual_seeks.push(position_seconds);
        Ok(true)
    }

    fn local_position_seconds(&self) -> Option<f64> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .local_position_seconds
    }
}

#[derive(Default)]
struct PositionTelemetryPlayer {
    updates: std::collections::VecDeque<sorotte_player_api::PlayerTransportTelemetryUpdate>,
}

impl PlayerAdapter for PositionTelemetryPlayer {
    fn name(&self) -> &'static str {
        "position-telemetry"
    }

    fn take_transport_telemetry_update(
        &mut self,
    ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
        self.updates.pop_front()
    }
}

fn position_update(
    observed_at_seconds: f64,
    position_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    sorotte_player_api::PlayerTransportTelemetryUpdate::new(
        sorotte_player_api::PlayerMediaGeneration::new(1),
        sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
            std::time::Duration::from_secs_f64(observed_at_seconds),
        ),
    )
    .with_position_seconds(position_seconds)
}

#[test]
fn attached_position_telemetry_grounds_normal_progress_and_publishes_native_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.extend([
        position_update(0.0, 10.0),
        position_update(0.1, 10.1),
        position_update(0.2, 15.1),
    ]);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_paused = Some(false);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));

    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(state.synchronized_positions, vec![10.0, 10.1, 15.1]);
    assert_eq!(state.recorded_manual_seeks, vec![15.1]);
    assert_eq!(state.local_position_seconds, Some(15.1));
}

#[test]
fn attached_position_telemetry_does_not_republish_sorotte_owned_seek() {
    let session_state = std::sync::Arc::new(std::sync::Mutex::new(PositionSessionState::default()));
    let mut player = PositionTelemetryPlayer::default();
    player.updates.push_back(position_update(0.0, 10.0));

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_paused = Some(false);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(player)));
    owner.session = Some(Box::new(PositionSession {
        state: session_state.clone(),
    }));
    owner.refresh_player_state_impl();

    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_position_seconds = Some(20.0);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(PositionTelemetryPlayer {
        updates: std::collections::VecDeque::from([position_update(0.1, 20.0)]),
    })));
    owner.refresh_player_state_impl();

    let state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(state.recorded_manual_seeks.is_empty());
    assert_eq!(state.local_position_seconds, Some(20.0));
}
