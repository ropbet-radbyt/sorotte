use std::path::Path;

use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCacheTelemetryUpdate, PlayerCapabilities, PlayerCommand,
    PlayerCommandId, PlayerCommandProgress, PlayerError, PlayerEventBatch,
    PlayerLocalFileObservation, PlayerMediaLoadObservation, PlayerMediaLoadOutcome,
    PlayerPlaybackTelemetryUpdate, PlayerTransportTelemetryUpdate,
};

use crate::MpvAdapter;

pub struct ConnectedMpvPlayer(MpvAdapter);

impl ConnectedMpvPlayer {
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, PlayerError> {
        MpvAdapter::with_json_ipc(path).map(Self)
    }

    pub fn into_inner(self) -> MpvAdapter {
        self.0
    }

    pub fn is_connected(&self) -> bool {
        self.0.is_connected()
    }

    pub fn take_ipc_connection_events(&mut self) -> Vec<crate::MpvIpcConnectionEvent> {
        self.0.take_ipc_connection_events()
    }

    #[cfg(test)]
    pub(crate) fn from_test_adapter(adapter: MpvAdapter) -> Self {
        Self(adapter)
    }
}

#[derive(Debug)]
pub struct SimulatedPlayer(MpvAdapter);

impl SimulatedPlayer {
    pub fn new() -> Self {
        Self(MpvAdapter::simulated())
    }

    pub fn into_inner(self) -> MpvAdapter {
        self.0
    }

    pub fn is_connected(&self) -> bool {
        self.0.is_connected()
    }

    #[cfg(test)]
    pub(crate) fn test_adapter(&self) -> &MpvAdapter {
        &self.0
    }
}

impl Default for SimulatedPlayer {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! impl_player_wrapper {
    ($player:ty, $name:literal) => {
        impl PlayerAdapter for $player {
            fn name(&self) -> &'static str {
                $name
            }

            fn maintain_runtime_leases_nonblocking(&mut self) {
                self.0.maintain_runtime_leases_nonblocking();
            }

            fn maintain_runtime_integrations(&mut self) {
                self.0.maintain_runtime_integrations();
            }

            fn capabilities(&self) -> PlayerCapabilities {
                self.0.capabilities()
            }

            fn execute(&mut self, command: PlayerCommand) -> Result<(), PlayerError> {
                self.0.execute(command)
            }

            fn execute_tracked(
                &mut self,
                command: PlayerCommand,
            ) -> Result<PlayerCommandId, PlayerError> {
                self.0.execute_tracked(command)
            }

            fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
                self.0.take_local_file_update()
            }

            fn take_local_file_observation(&mut self) -> Option<PlayerLocalFileObservation> {
                self.0.take_local_file_observation()
            }

            fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
                self.0.take_playback_telemetry_update()
            }

            fn take_transport_telemetry_update(
                &mut self,
            ) -> Option<PlayerTransportTelemetryUpdate> {
                self.0.take_transport_telemetry_update()
            }

            fn take_cache_telemetry_update(&mut self) -> Option<PlayerCacheTelemetryUpdate> {
                self.0.take_cache_telemetry_update()
            }

            fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
                self.0.take_command_progress()
            }

            fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
                self.0.take_media_load_outcome()
            }

            fn take_media_load_observation(&mut self) -> Option<PlayerMediaLoadObservation> {
                self.0.take_media_load_observation()
            }

            fn take_ordered_event_batch(&mut self) -> Option<PlayerEventBatch> {
                self.0.take_ordered_event_batch()
            }

            fn take_pending_chat_request(&mut self) -> Option<String> {
                self.0.take_pending_chat_request()
            }
        }
    };
}

impl_player_wrapper!(ConnectedMpvPlayer, "mpv");
impl_player_wrapper!(SimulatedPlayer, "simulated-mpv");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_wrapper_forwards_cache_telemetry() {
        let mut player = SimulatedPlayer::new();
        player.0.inject_test_cache_telemetry_update();

        let update = player
            .take_cache_telemetry_update()
            .expect("wrapper should forward cache telemetry from the inner adapter");
        assert!(update.media_generation.is_some());
        assert!(update.observed_at.is_some());
        assert_eq!(update.buffered_ahead_seconds, Some(5.0));
    }
}
