use std::{
    ops::{Deref, DerefMut},
    path::Path,
};

use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCapabilities, PlayerCommand, PlayerError,
    PlayerMediaLoadOutcome, PlayerPlaybackTelemetryUpdate,
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

    #[cfg(test)]
    pub(crate) fn from_test_adapter(adapter: MpvAdapter) -> Self {
        Self(adapter)
    }
}

impl Deref for ConnectedMpvPlayer {
    type Target = MpvAdapter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ConnectedMpvPlayer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
}

impl Default for SimulatedPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for SimulatedPlayer {
    type Target = MpvAdapter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SimulatedPlayer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

macro_rules! impl_player_wrapper {
    ($player:ty, $name:literal) => {
        impl PlayerAdapter for $player {
            fn name(&self) -> &'static str {
                $name
            }

            fn capabilities(&self) -> PlayerCapabilities {
                self.0.capabilities()
            }

            fn execute(&mut self, command: PlayerCommand) -> Result<(), PlayerError> {
                self.0.execute(command)
            }

            fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
                self.0.take_local_file_update()
            }

            fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
                self.0.take_playback_telemetry_update()
            }

            fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
                self.0.take_media_load_outcome()
            }

            fn take_pending_chat_request(&mut self) -> Option<String> {
                self.0.take_pending_chat_request()
            }
        }
    };
}

impl_player_wrapper!(ConnectedMpvPlayer, "mpv");
impl_player_wrapper!(SimulatedPlayer, "simulated-mpv");
