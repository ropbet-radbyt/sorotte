use std::{fs::OpenOptions, io::Write, path::PathBuf};

use crate::app::mpv_launch::ManagedMpvLaunchConfig;
use sorotte_client_app::app_boundary::state::EffectiveMpvStreamingOption;
use sorotte_client_core::ExternalPlayerAvailability;
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCommand, PlayerCommandId, PlayerError,
    PlayerEventAcknowledgementToken, PlayerEventBatch, PlayerMediaGeneration,
};
use sorotte_player_mpv::{MpvAdapter, SyncplayUiSettings};

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

    fn unload(&mut self) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }
}

pub(in super::super) struct GuiTestPlayerAdapter {
    adapter: Box<MpvAdapter>,
    open_file_observation_path: Option<PathBuf>,
}

impl Default for GuiTestPlayerAdapter {
    fn default() -> Self {
        Self {
            adapter: Box::new(MpvAdapter::simulated()),
            open_file_observation_path: None,
        }
    }
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

    use sorotte_player_api::{
        PlayerAdapter, PlayerCommandSemanticResult, PlayerEvent, PlayerSemanticOutcome,
    };

    use super::{GuiOwnedPlayer, GuiTestPlayerAdapter, local_file_update_for_player_path};

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

    #[test]
    fn gui_owned_player_forwards_unload_to_its_active_adapter() {
        let mut player = GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default());
        player
            .open_file("episode.mkv")
            .expect("the GUI test player should accept a media load");
        let GuiOwnedPlayer::Test(adapter) = &player else {
            unreachable!();
        };
        assert_eq!(adapter.adapter.current_path(), Some("episode.mkv"));

        player
            .unload()
            .expect("the GUI owner should forward canonical media retirement");

        let GuiOwnedPlayer::Test(adapter) = &player else {
            unreachable!();
        };
        assert_eq!(adapter.adapter.current_path(), None);
    }

    #[test]
    fn gui_test_player_delivers_scoped_load_completion_until_acknowledged() {
        let mut player = GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default());

        while let Some(batch) = player.take_player_event_batch() {
            player
                .acknowledge_player_event_batch(batch.acknowledgement_token)
                .expect("initial snapshot should acknowledge");
        }

        let started = player
            .open_file_tracked("episode.mkv")
            .expect("the GUI test player should track the media load");
        let command_id = started.player_command_id.expect("load should be tracked");
        let batch = player
            .take_player_event_batch()
            .expect("media load should produce ordered observations");
        let generation = batch
            .events
            .iter()
            .find_map(|event| match &event.event {
                PlayerEvent::LocalFileChanged {
                    media_generation,
                    update,
                    ..
                } if update.path.as_deref() == Some("episode.mkv") => Some(*media_generation),
                _ => None,
            })
            .expect("file identity should carry its load generation");
        assert!(batch.semantic_outcomes.iter().any(|outcome| {
            matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::Command(outcome)
                    if outcome.command_id == command_id
                        && outcome.media_generation == Some(generation)
                        && outcome.result == PlayerCommandSemanticResult::Completed
            )
        }));
        let redelivered = player
            .take_player_event_batch()
            .expect("unacknowledged load should be redelivered");
        assert_eq!(
            redelivered.acknowledgement_token,
            batch.acknowledgement_token
        );
        assert_eq!(redelivered.sequence_boundary, batch.sequence_boundary);
        assert_eq!(redelivered.semantic_outcomes, batch.semantic_outcomes);

        player
            .acknowledge_player_event_batch(batch.acknowledgement_token)
            .expect("consumed observations should acknowledge");
        assert!(player.take_player_event_batch().is_none());
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
        // Match managed mpv's paused startup without inventing a native Play.
        self.adapter.set_paused(true)?;
        self.adapter.open_file(path)
    }

    fn execute_tracked(&mut self, command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        if let PlayerCommand::OpenFile(path) = &command {
            self.record_open_file_observation(path)?;
            self.adapter.set_paused(true)?;
        }
        self.adapter.execute_tracked(command)
    }

    fn supports_transport_telemetry(&self) -> bool {
        self.adapter.supports_transport_telemetry()
    }

    fn maintain_runtime_leases_nonblocking(&mut self) {
        self.adapter.maintain_runtime_leases_nonblocking();
    }

    fn maintain_runtime_integrations(&mut self) {
        self.adapter.maintain_runtime_integrations();
    }

    fn unload(&mut self) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter.unload()
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter.set_paused(paused)
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter.set_position(position_seconds)
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.adapter.set_playback_rate(rate)
    }

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        self.adapter.take_player_event_batch()
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        self.adapter.acknowledge_player_event_batch(token)
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
    fn adapter(&self) -> &dyn PlayerAdapter {
        match self {
            Self::Test(player) => player,
            Self::Mpv(player) => player.as_ref(),
            #[cfg(test)]
            Self::Custom(player) => player.as_ref(),
        }
    }

    fn adapter_mut(&mut self) -> &mut dyn PlayerAdapter {
        match self {
            Self::Test(player) => player,
            Self::Mpv(player) => player.as_mut(),
            #[cfg(test)]
            Self::Custom(player) => player.as_mut(),
        }
    }

    pub(in super::super) fn name(&self) -> &'static str {
        self.adapter().name()
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
        let supports_telemetry = match self {
            Self::Test(player) => player.supports_transport_telemetry(),
            Self::Mpv(player) if !player.is_connected() => {
                return ExternalPlayerAvailability::Disconnected;
            }
            Self::Mpv(player) => player.supports_transport_telemetry(),
            #[cfg(test)]
            Self::Custom(player) => player.supports_transport_telemetry(),
        };
        if supports_telemetry {
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
        self.adapter_mut().maintain_runtime_leases_nonblocking()
    }

    fn maintain_runtime_integrations(&mut self) {
        self.adapter_mut().maintain_runtime_integrations()
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().open_file(path)
    }

    fn unload(&mut self) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().unload()
    }

    fn execute_tracked(&mut self, command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        self.adapter_mut().execute_tracked(command)
    }

    fn set_option_string(
        &mut self,
        name: &str,
        value: &str,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().set_option_string(name, value)
    }

    fn apply_profile(&mut self, profile: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().apply_profile(profile)
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().set_paused(paused)
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().set_position(position_seconds)
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), sorotte_player_api::PlayerError> {
        self.adapter_mut().set_playback_rate(rate)
    }

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        self.adapter_mut().take_player_event_batch()
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        self.adapter_mut().acknowledge_player_event_batch(token)
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        self.adapter_mut().take_pending_chat_request()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in super::super) enum GuiPlayerLaunchRuntimeState {
    None,
    TestPlayer,
    ExplicitMpvIpc {
        ipc_path: String,
        ui_settings: Box<SyncplayUiSettings>,
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

    pub(in super::super) fn mpv_ui_settings(&self) -> Option<&SyncplayUiSettings> {
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
