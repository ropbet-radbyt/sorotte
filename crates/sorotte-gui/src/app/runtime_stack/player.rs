use std::{collections::VecDeque, path::Path};

use crate::app::mpv_launch::ManagedMpvLaunchConfig;
use sorotte_client_app::app_boundary::state::EffectiveMpvStreamingOption;
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCommand, PlayerCommandId, PlayerCommandProgress,
    PlayerError, PlayerMediaLoadOutcome, PlayerPlaybackTelemetryUpdate,
    PlayerTransportTelemetryUpdate,
};
use sorotte_player_mpv::{LegacySyncplayUiSettings, MpvAdapter};

pub(in super::super) struct GuiNoopClientRuntimePlayer;

impl PlayerAdapter for GuiNoopClientRuntimePlayer {
    fn name(&self) -> &'static str {
        "gui-client-runtime-noop"
    }

    fn set_paused(&mut self, _paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }

    fn set_position(
        &mut self,
        _position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }

    fn set_playback_rate(&mut self, _rate: f64) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }
}

#[derive(Default)]
pub(in super::super) struct GuiTestPlayerAdapter {
    local_file_updates: VecDeque<LocalFileUpdate>,
    playback_updates: VecDeque<PlayerPlaybackTelemetryUpdate>,
    media_load_outcomes: VecDeque<PlayerMediaLoadOutcome>,
}

impl GuiTestPlayerAdapter {
    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        LocalFileUpdate::new(name).with_path(path.to_owned())
    }
}

impl PlayerAdapter for GuiTestPlayerAdapter {
    fn name(&self) -> &'static str {
        "test"
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.local_file_updates
            .push_back(Self::local_file_update_for_path(path));
        self.media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(path, Some(path.to_owned())));
        self.playback_updates.push_back(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(0.0),
        );
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.playback_updates
            .push_back(PlayerPlaybackTelemetryUpdate::default().with_paused(paused));
        Ok(())
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.playback_updates.push_back(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(position_seconds),
        );
        Ok(())
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.local_file_updates.pop_front()
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        self.playback_updates.pop_front()
    }

    fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
        self.media_load_outcomes.pop_front()
    }
}

pub(in super::super) enum GuiOwnedPlayer {
    Test(GuiTestPlayerAdapter),
    Mpv(Box<MpvAdapter>),
    #[cfg(test)]
    Custom(Box<dyn PlayerAdapter + Send>),
}

impl GuiOwnedPlayer {
    pub(in super::super) fn name(&self) -> &'static str {
        match self {
            Self::Test(player) => player.name(),
            Self::Mpv(player) => player.name(),
            #[cfg(test)]
            Self::Custom(player) => player.name(),
        }
    }

    pub(in super::super) fn as_mpv_mut(&mut self) -> Option<&mut MpvAdapter> {
        match self {
            Self::Mpv(player) => Some(player),
            #[cfg(test)]
            Self::Test(_) | Self::Custom(_) => None,
            #[cfg(not(test))]
            Self::Test(_) => None,
        }
    }

    pub(in super::super) fn open_file_tracked(&mut self, path: &str) -> Result<(), PlayerError> {
        match self.execute_tracked(PlayerCommand::OpenFile(path.to_owned())) {
            Ok(_) => Ok(()),
            Err(PlayerError::Unsupported("execute_tracked")) => self.open_file(path),
            Err(error) => Err(error),
        }
    }
}

impl PlayerAdapter for GuiOwnedPlayer {
    fn name(&self) -> &'static str {
        self.name()
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.open_file(path),
            Self::Mpv(player) => player.open_file(path),
            #[cfg(test)]
            Self::Custom(player) => player.open_file(path),
        }
    }

    fn execute_tracked(&mut self, command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        match self {
            Self::Test(player) => player.execute_tracked(command),
            Self::Mpv(player) => player.execute_tracked(command),
            #[cfg(test)]
            Self::Custom(player) => player.execute_tracked(command),
        }
    }

    fn set_option_string(
        &mut self,
        name: &str,
        value: &str,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_option_string(name, value),
            Self::Mpv(player) => player.set_option_string(name, value),
            #[cfg(test)]
            Self::Custom(player) => player.set_option_string(name, value),
        }
    }

    fn apply_profile(&mut self, profile: &str) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.apply_profile(profile),
            Self::Mpv(player) => player.apply_profile(profile),
            #[cfg(test)]
            Self::Custom(player) => player.apply_profile(profile),
        }
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_paused(paused),
            Self::Mpv(player) => player.set_paused(paused),
            #[cfg(test)]
            Self::Custom(player) => player.set_paused(paused),
        }
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_position(position_seconds),
            Self::Mpv(player) => player.set_position(position_seconds),
            #[cfg(test)]
            Self::Custom(player) => player.set_position(position_seconds),
        }
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), sorotte_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_playback_rate(rate),
            Self::Mpv(player) => player.set_playback_rate(rate),
            #[cfg(test)]
            Self::Custom(player) => player.set_playback_rate(rate),
        }
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        match self {
            Self::Test(player) => player.take_local_file_update(),
            Self::Mpv(player) => player.take_local_file_update(),
            #[cfg(test)]
            Self::Custom(player) => player.take_local_file_update(),
        }
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        match self {
            Self::Test(player) => player.take_playback_telemetry_update(),
            Self::Mpv(player) => player.take_playback_telemetry_update(),
            #[cfg(test)]
            Self::Custom(player) => player.take_playback_telemetry_update(),
        }
    }

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        match self {
            Self::Test(player) => player.take_transport_telemetry_update(),
            Self::Mpv(player) => player.take_transport_telemetry_update(),
            #[cfg(test)]
            Self::Custom(player) => player.take_transport_telemetry_update(),
        }
    }

    fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
        match self {
            Self::Test(player) => player.take_command_progress(),
            Self::Mpv(player) => player.take_command_progress(),
            #[cfg(test)]
            Self::Custom(player) => player.take_command_progress(),
        }
    }

    fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
        match self {
            Self::Test(player) => player.take_media_load_outcome(),
            Self::Mpv(player) => player.take_media_load_outcome(),
            #[cfg(test)]
            Self::Custom(player) => player.take_media_load_outcome(),
        }
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        match self {
            Self::Test(player) => player.take_pending_chat_request(),
            Self::Mpv(player) => player.take_pending_chat_request(),
            #[cfg(test)]
            Self::Custom(player) => player.take_pending_chat_request(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in super::super) enum GuiPlayerLaunchRuntimeState {
    None,
    TestPlayer,
    ExplicitMpvIpc {
        ipc_path: String,
        ui_settings: Box<LegacySyncplayUiSettings>,
        effective_streaming_options: Vec<EffectiveMpvStreamingOption>,
    },
    ManagedMpv(Box<ManagedMpvLaunchConfig>),
    UnsupportedConfiguredPlayer {
        player_path: String,
    },
}

impl GuiPlayerLaunchRuntimeState {
    pub(in super::super) fn default_unavailability_reason(&self) -> Option<String> {
        match self {
            Self::UnsupportedConfiguredPlayer { player_path } => Some(format!(
                "GUI-owned player launch currently supports mpv only; saved player path '{player_path}' was not started."
            )),
            Self::None | Self::TestPlayer | Self::ExplicitMpvIpc { .. } | Self::ManagedMpv(_) => {
                None
            }
        }
    }

    pub(in super::super) fn can_attach_on_demand(&self) -> bool {
        matches!(
            self,
            Self::TestPlayer | Self::ExplicitMpvIpc { .. } | Self::ManagedMpv(_)
        )
    }

    pub(in super::super) fn can_apply_mpv_ui_settings_in_place(&self, next: &Self) -> bool {
        match (self, next) {
            (
                Self::ExplicitMpvIpc {
                    ipc_path: current_path,
                    ..
                },
                Self::ExplicitMpvIpc {
                    ipc_path: next_path,
                    ..
                },
            ) => current_path == next_path,
            (Self::ManagedMpv(current), Self::ManagedMpv(next)) => {
                current.matches_process_target(next)
            }
            _ => false,
        }
    }

    pub(in super::super) fn mpv_ui_settings(&self) -> Option<&LegacySyncplayUiSettings> {
        match self {
            Self::ExplicitMpvIpc { ui_settings, .. } => Some(ui_settings),
            Self::ManagedMpv(config) => Some(&config.ui_settings),
            Self::None | Self::TestPlayer | Self::UnsupportedConfiguredPlayer { .. } => None,
        }
    }

    pub(in super::super) fn effective_mpv_streaming_options(
        &self,
    ) -> Option<&[EffectiveMpvStreamingOption]> {
        match self {
            Self::ExplicitMpvIpc {
                effective_streaming_options,
                ..
            } => Some(effective_streaming_options),
            Self::ManagedMpv(config) => Some(&config.effective_streaming_options),
            Self::None | Self::TestPlayer | Self::UnsupportedConfiguredPlayer { .. } => None,
        }
    }
}
