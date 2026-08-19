use std::{collections::VecDeque, fs::OpenOptions, io::Write, path::PathBuf};

use crate::app::mpv_launch::ManagedMpvLaunchConfig;
use sorotte_client_app::app_boundary::state::EffectiveMpvStreamingOption;
use sorotte_client_core::ExternalPlayerAvailability;
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCacheTelemetryUpdate, PlayerCapability, PlayerCommand,
    PlayerCommandId, PlayerCommandProgress, PlayerError, PlayerEventAcknowledgementToken,
    PlayerEventBatch, PlayerEventDeliveryMode, PlayerLocalFileObservation, PlayerMediaGeneration,
    PlayerMediaLoadObservation, PlayerMediaLoadOutcome, PlayerObservationBatch,
    PlayerPlaybackTelemetryUpdate, PlayerTransportTelemetryUpdate,
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
    open_file_observation_path: Option<PathBuf>,
}

const TEST_PLAYER_OBSERVATION_PATH_ENV: &str = "SOROTTE_GUI_TEST_PLAYER_OBSERVATION_PATH";

pub(in crate::app) fn local_file_update_for_player_path(path: &str) -> LocalFileUpdate {
    let name = if path.contains("://") {
        path.to_owned()
    } else {
        path.rsplit(['/', '\\'])
            .find(|component| !component.is_empty())
            .unwrap_or(path)
            .to_owned()
    };
    LocalFileUpdate::new(name).with_path(path.to_owned())
}

#[cfg(test)]
mod path_tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use sorotte_player_api::PlayerAdapter;

    use super::{GuiTestPlayerAdapter, local_file_update_for_player_path};

    #[test]
    fn local_file_identity_accepts_both_path_separator_styles() {
        for path in [
            "C:\\private\\shows\\episode.mkv",
            "/private/shows/episode.mkv",
        ] {
            let update = local_file_update_for_player_path(path);
            assert_eq!(update.name, "episode.mkv");
            assert_eq!(update.path.as_deref(), Some(path));
        }
    }

    #[test]
    fn network_media_identity_preserves_the_full_url() {
        let path = "https://media.example.test/watch/episode.mkv";
        let update = local_file_update_for_player_path(path);

        assert_eq!(update.name, path);
        assert_eq!(update.path.as_deref(), Some(path));
    }

    #[test]
    fn test_player_observation_records_the_exact_open_file_path_as_json() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-gui-test-player-observation-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("observation fixture directory should be created");
        let observation_path = root.join("open-file.jsonl");
        let media_path = "C:\\media\\episode \"one\".mkv";
        let mut player = GuiTestPlayerAdapter {
            open_file_observation_path: Some(observation_path.clone()),
            ..GuiTestPlayerAdapter::default()
        };

        player
            .open_file(media_path)
            .expect("test player should record the Open File command");

        let payload = std::fs::read_to_string(&observation_path)
            .expect("test player observation should be readable");
        let observation: serde_json::Value =
            serde_json::from_str(payload.trim()).expect("observation should be valid JSON");
        assert_eq!(observation["event"], "open_file");
        assert_eq!(observation["path"], media_path);

        std::fs::remove_file(observation_path)
            .expect("observation fixture file should be removable");
        std::fs::remove_dir(root).expect("observation fixture directory should be removable");
    }
}

impl GuiTestPlayerAdapter {
    pub(in crate::app) fn from_environment() -> Self {
        Self {
            open_file_observation_path: std::env::var_os(TEST_PLAYER_OBSERVATION_PATH_ENV)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            ..Self::default()
        }
    }

    fn record_open_file_observation(&self, path: &str) -> Result<(), PlayerError> {
        let Some(observation_path) = self.open_file_observation_path.as_ref() else {
            return Ok(());
        };
        let payload = serde_json::to_string(&serde_json::json!({
            "event": "open_file",
            "path": path,
        }))
        .map_err(|error| {
            PlayerError::OperationFailed(format!(
                "failed to serialize test-player observation: {error}"
            ))
        })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(observation_path)
            .map_err(|error| {
                PlayerError::OperationFailed(format!(
                    "failed to open test-player observation file {}: {error}",
                    observation_path.display()
                ))
            })?;
        writeln!(file, "{payload}").map_err(|error| {
            PlayerError::OperationFailed(format!(
                "failed to write test-player observation file {}: {error}",
                observation_path.display()
            ))
        })?;
        file.flush().map_err(|error| {
            PlayerError::OperationFailed(format!(
                "failed to flush test-player observation file {}: {error}",
                observation_path.display()
            ))
        })
    }
}

impl PlayerAdapter for GuiTestPlayerAdapter {
    fn name(&self) -> &'static str {
        "test"
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.record_open_file_observation(path)?;
        self.local_file_updates
            .push_back(local_file_update_for_player_path(path));
        self.media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(path, Some(path.to_owned())));
        self.playback_updates.push_back(
            PlayerPlaybackTelemetryUpdate::default()
                // Managed mpv is launched with `--pause`; reporting an
                // unpaused open here invents a native Play gesture and can
                // incorrectly promote the local user to Ready.
                .with_paused(true)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GuiStartedMediaLoad {
    pub(in super::super) player_command_id: Option<PlayerCommandId>,
    pub(in super::super) player_media_generation: Option<PlayerMediaGeneration>,
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

    pub(in super::super) fn external_availability(&self) -> ExternalPlayerAvailability {
        let capabilities = match self {
            Self::Test(player) => player.capabilities(),
            Self::Mpv(player) if !player.is_connected() => {
                return ExternalPlayerAvailability::Disconnected;
            }
            Self::Mpv(player) => player.capabilities(),
            #[cfg(test)]
            Self::Custom(player) => player.capabilities(),
        };
        if capabilities.contains(PlayerCapability::Telemetry) {
            ExternalPlayerAvailability::Connecting
        } else {
            ExternalPlayerAvailability::TelemetryUnavailable
        }
    }

    pub(in super::super) fn open_file_tracked(
        &mut self,
        path: &str,
    ) -> Result<GuiStartedMediaLoad, PlayerError> {
        match self.execute_tracked(PlayerCommand::OpenFile(path.to_owned())) {
            Ok(player_command_id) => Ok(GuiStartedMediaLoad {
                player_command_id: Some(player_command_id),
                player_media_generation: None,
            }),
            Err(PlayerError::Unsupported("execute_tracked")) => {
                self.open_file(path)?;
                Ok(GuiStartedMediaLoad {
                    player_command_id: None,
                    player_media_generation: None,
                })
            }
            Err(error) => Err(error),
        }
    }

    pub(in super::super) fn set_position_tracked(
        &mut self,
        position_seconds: f64,
    ) -> Result<Option<PlayerCommandId>, PlayerError> {
        match self.execute_tracked(PlayerCommand::SetPosition(position_seconds)) {
            Ok(player_command_id) => Ok(Some(player_command_id)),
            Err(PlayerError::Unsupported("execute_tracked")) => {
                self.set_position(position_seconds)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

impl PlayerAdapter for GuiOwnedPlayer {
    fn name(&self) -> &'static str {
        self.name()
    }

    fn maintain_runtime_leases_nonblocking(&mut self) {
        match self {
            Self::Test(player) => player.maintain_runtime_leases_nonblocking(),
            Self::Mpv(player) => player.maintain_runtime_leases_nonblocking(),
            #[cfg(test)]
            Self::Custom(player) => player.maintain_runtime_leases_nonblocking(),
        }
    }

    fn maintain_runtime_integrations(&mut self) {
        match self {
            Self::Test(player) => player.maintain_runtime_integrations(),
            Self::Mpv(player) => player.maintain_runtime_integrations(),
            #[cfg(test)]
            Self::Custom(player) => player.maintain_runtime_integrations(),
        }
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

    fn take_local_file_observation(&mut self) -> Option<PlayerLocalFileObservation> {
        match self {
            Self::Test(player) => player.take_local_file_observation(),
            Self::Mpv(player) => player.take_local_file_observation(),
            #[cfg(test)]
            Self::Custom(player) => player.take_local_file_observation(),
        }
    }

    fn take_media_load_observation(&mut self) -> Option<PlayerMediaLoadObservation> {
        match self {
            Self::Test(player) => player.take_media_load_observation(),
            Self::Mpv(player) => player.take_media_load_observation(),
            #[cfg(test)]
            Self::Custom(player) => player.take_media_load_observation(),
        }
    }

    fn take_ordered_event_batch(&mut self) -> Option<PlayerObservationBatch> {
        match self {
            Self::Test(player) => player.take_ordered_event_batch(),
            Self::Mpv(player) => player.take_ordered_event_batch(),
            #[cfg(test)]
            Self::Custom(player) => player.take_ordered_event_batch(),
        }
    }

    fn request_ordered_event_reacquisition(&mut self) {
        match self {
            Self::Test(player) => player.request_ordered_event_reacquisition(),
            Self::Mpv(player) => player.request_ordered_event_reacquisition(),
            #[cfg(test)]
            Self::Custom(player) => player.request_ordered_event_reacquisition(),
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

    fn take_cache_telemetry_update(&mut self) -> Option<PlayerCacheTelemetryUpdate> {
        match self {
            Self::Test(player) => player.take_cache_telemetry_update(),
            Self::Mpv(player) => player.take_cache_telemetry_update(),
            #[cfg(test)]
            Self::Custom(player) => player.take_cache_telemetry_update(),
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

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        match self {
            Self::Test(player) => player.take_player_event_batch(),
            Self::Mpv(player) => player.take_player_event_batch(),
            #[cfg(test)]
            Self::Custom(player) => player.take_player_event_batch(),
        }
    }

    fn player_event_delivery_mode(&self) -> PlayerEventDeliveryMode {
        match self {
            Self::Test(player) => player.player_event_delivery_mode(),
            Self::Mpv(player) => player.player_event_delivery_mode(),
            #[cfg(test)]
            Self::Custom(player) => player.player_event_delivery_mode(),
        }
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        match self {
            Self::Test(player) => player.acknowledge_player_event_batch(token),
            Self::Mpv(player) => player.acknowledge_player_event_batch(token),
            #[cfg(test)]
            Self::Custom(player) => player.acknowledge_player_event_batch(token),
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
