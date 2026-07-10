#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlayerError {
    #[error("operation not supported: {0}")]
    Unsupported(&'static str),
    #[error("player is not connected")]
    NotConnected,
    #[error("operation failed: {0}")]
    OperationFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PlayerCapability {
    OpenFile,
    SetOption,
    ApplyProfile,
    Playback,
    Audio,
    Video,
    Window,
    Subtitles,
    Osd,
    Telemetry,
    ChatInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerCapabilities(u64);

impl PlayerCapabilities {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self((1 << 11) - 1);

    pub const fn contains(self, capability: PlayerCapability) -> bool {
        self.0 & (1 << capability as u8) != 0
    }

    pub fn from_capabilities(capabilities: impl IntoIterator<Item = PlayerCapability>) -> Self {
        capabilities
            .into_iter()
            .fold(Self::NONE, |result, capability| {
                Self(result.0 | (1 << capability as u8))
            })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerCommand {
    OpenFile(String),
    SetOptionString { name: String, value: String },
    ApplyProfile(String),
    SetPaused(bool),
    SetPosition(f64),
    SetPlaybackRate(f64),
    SetMuted(bool),
    SetVolume(f64),
    SetDeinterlace(bool),
    SetKeepaspect(bool),
    SetKeepaspectWindow(bool),
    SetFullscreen(bool),
    SetOntop(bool),
    SetBorder(bool),
    SetForceWindow(bool),
    SetKeepOpen(bool),
    SetKeepOpenPause(bool),
    SetCursorAutohideFsOnly(bool),
    SetStopScreensaver(bool),
    SetSubVisibility(bool),
    SetOsdBar(bool),
    SetWindowMaximized(bool),
    SetWindowMinimized(bool),
}

impl PlayerCommand {
    pub const fn required_capability(&self) -> PlayerCapability {
        match self {
            Self::OpenFile(_) => PlayerCapability::OpenFile,
            Self::SetOptionString { .. } => PlayerCapability::SetOption,
            Self::ApplyProfile(_) => PlayerCapability::ApplyProfile,
            Self::SetPaused(_) | Self::SetPosition(_) | Self::SetPlaybackRate(_) => {
                PlayerCapability::Playback
            }
            Self::SetMuted(_) | Self::SetVolume(_) => PlayerCapability::Audio,
            Self::SetDeinterlace(_) | Self::SetKeepaspect(_) | Self::SetKeepaspectWindow(_) => {
                PlayerCapability::Video
            }
            Self::SetFullscreen(_)
            | Self::SetOntop(_)
            | Self::SetBorder(_)
            | Self::SetForceWindow(_)
            | Self::SetKeepOpen(_)
            | Self::SetKeepOpenPause(_)
            | Self::SetCursorAutohideFsOnly(_)
            | Self::SetStopScreensaver(_)
            | Self::SetWindowMaximized(_)
            | Self::SetWindowMinimized(_) => PlayerCapability::Window,
            Self::SetSubVisibility(_) => PlayerCapability::Subtitles,
            Self::SetOsdBar(_) => PlayerCapability::Osd,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalFileUpdate {
    pub name: String,
    pub duration_seconds: Option<f64>,
    pub size_bytes: Option<u64>,
    pub path: Option<String>,
}

impl LocalFileUpdate {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            duration_seconds: None,
            size_bytes: None,
            path: None,
        }
    }

    pub fn with_duration_seconds(mut self, duration_seconds: f64) -> Self {
        self.duration_seconds = Some(duration_seconds);
        self
    }

    pub fn with_size_bytes(mut self, size_bytes: u64) -> Self {
        self.size_bytes = Some(size_bytes);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerPlaybackTelemetryUpdate {
    pub paused: Option<bool>,
    pub position_seconds: Option<f64>,
    pub playback_rate: Option<f64>,
    pub paused_for_cache: Option<bool>,
    pub cache_buffering_percent: Option<f64>,
}

impl PlayerPlaybackTelemetryUpdate {
    pub fn with_paused(mut self, paused: bool) -> Self {
        self.paused = Some(paused);
        self
    }

    pub fn with_position_seconds(mut self, position_seconds: f64) -> Self {
        self.position_seconds = Some(position_seconds);
        self
    }

    pub fn with_playback_rate(mut self, playback_rate: f64) -> Self {
        self.playback_rate = Some(playback_rate);
        self
    }

    pub fn with_paused_for_cache(mut self, paused_for_cache: bool) -> Self {
        self.paused_for_cache = Some(paused_for_cache);
        self
    }

    pub fn with_cache_buffering_percent(mut self, cache_buffering_percent: f64) -> Self {
        self.cache_buffering_percent = Some(cache_buffering_percent);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerMediaLoadFailureKind {
    LoadAborted,
    FormatUnsupported,
    Network,
    HelperMissing,
    HelperBroken,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerMediaLoadFailure {
    pub kind: PlayerMediaLoadFailureKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerMediaLoadOutcome {
    pub requested_target: String,
    pub loaded_target: Option<String>,
    pub failure: Option<PlayerMediaLoadFailure>,
}

impl PlayerMediaLoadOutcome {
    pub fn success(requested_target: impl Into<String>, loaded_target: Option<String>) -> Self {
        Self {
            requested_target: requested_target.into(),
            loaded_target,
            failure: None,
        }
    }

    pub fn failure(
        requested_target: impl Into<String>,
        loaded_target: Option<String>,
        kind: PlayerMediaLoadFailureKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            requested_target: requested_target.into(),
            loaded_target,
            failure: Some(PlayerMediaLoadFailure {
                kind,
                message: message.into(),
            }),
        }
    }

    pub fn succeeded(&self) -> bool {
        self.failure.is_none()
    }
}

pub trait PlayerAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> PlayerCapabilities {
        PlayerCapabilities::NONE
    }
    fn execute(&mut self, command: PlayerCommand) -> Result<(), PlayerError> {
        match command {
            PlayerCommand::OpenFile(path) => self.open_file(&path),
            PlayerCommand::SetOptionString { name, value } => self.set_option_string(&name, &value),
            PlayerCommand::ApplyProfile(profile) => self.apply_profile(&profile),
            PlayerCommand::SetPaused(paused) => self.set_paused(paused),
            PlayerCommand::SetPosition(position) => self.set_position(position),
            PlayerCommand::SetPlaybackRate(rate) => self.set_playback_rate(rate),
            PlayerCommand::SetMuted(muted) => self.set_muted(muted),
            PlayerCommand::SetVolume(volume) => self.set_volume(volume),
            PlayerCommand::SetDeinterlace(enabled) => self.set_deinterlace(enabled),
            PlayerCommand::SetKeepaspect(enabled) => self.set_keepaspect(enabled),
            PlayerCommand::SetKeepaspectWindow(enabled) => self.set_keepaspect_window(enabled),
            PlayerCommand::SetFullscreen(enabled) => self.set_fullscreen(enabled),
            PlayerCommand::SetOntop(enabled) => self.set_ontop(enabled),
            PlayerCommand::SetBorder(enabled) => self.set_border(enabled),
            PlayerCommand::SetForceWindow(enabled) => self.set_force_window(enabled),
            PlayerCommand::SetKeepOpen(enabled) => self.set_keep_open(enabled),
            PlayerCommand::SetKeepOpenPause(enabled) => self.set_keep_open_pause(enabled),
            PlayerCommand::SetCursorAutohideFsOnly(enabled) => {
                self.set_cursor_autohide_fs_only(enabled)
            }
            PlayerCommand::SetStopScreensaver(enabled) => self.set_stop_screensaver(enabled),
            PlayerCommand::SetSubVisibility(enabled) => self.set_sub_visibility(enabled),
            PlayerCommand::SetOsdBar(enabled) => self.set_osd_bar(enabled),
            PlayerCommand::SetWindowMaximized(enabled) => self.set_window_maximized(enabled),
            PlayerCommand::SetWindowMinimized(enabled) => self.set_window_minimized(enabled),
        }
    }
    fn open_file(&mut self, _path: &str) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("open_file"))
    }
    fn set_option_string(&mut self, _name: &str, _value: &str) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_option_string"))
    }
    fn apply_profile(&mut self, _profile: &str) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("apply_profile"))
    }
    fn set_paused(&mut self, _paused: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_paused"))
    }
    fn set_position(&mut self, _position_seconds: f64) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_position"))
    }
    fn set_playback_rate(&mut self, _rate: f64) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_playback_rate"))
    }
    fn set_muted(&mut self, _muted: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_muted"))
    }
    fn set_volume(&mut self, _volume: f64) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_volume"))
    }
    fn set_deinterlace(&mut self, _deinterlace: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_deinterlace"))
    }
    fn set_keepaspect(&mut self, _keepaspect: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_keepaspect"))
    }
    fn set_keepaspect_window(&mut self, _keepaspect_window: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_keepaspect_window"))
    }
    fn set_fullscreen(&mut self, _fullscreen: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_fullscreen"))
    }
    fn set_ontop(&mut self, _ontop: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_ontop"))
    }
    fn set_border(&mut self, _border: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_border"))
    }
    fn set_force_window(&mut self, _force_window: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_force_window"))
    }
    fn set_keep_open(&mut self, _keep_open: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_keep_open"))
    }
    fn set_keep_open_pause(&mut self, _keep_open_pause: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_keep_open_pause"))
    }
    fn set_cursor_autohide_fs_only(
        &mut self,
        _cursor_autohide_fs_only: bool,
    ) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_cursor_autohide_fs_only"))
    }
    fn set_stop_screensaver(&mut self, _stop_screensaver: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_stop_screensaver"))
    }
    fn set_sub_visibility(&mut self, _sub_visibility: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_sub_visibility"))
    }
    fn set_osd_bar(&mut self, _osd_bar: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_osd_bar"))
    }
    fn set_window_maximized(&mut self, _window_maximized: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_window_maximized"))
    }
    fn set_window_minimized(&mut self, _window_minimized: bool) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("set_window_minimized"))
    }
    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        None
    }
    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        None
    }
    fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
        None
    }
    fn take_pending_chat_request(&mut self) -> Option<String> {
        None
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DisconnectedPlayer;

impl PlayerAdapter for DisconnectedPlayer {
    fn name(&self) -> &'static str {
        "disconnected"
    }

    fn execute(&mut self, _command: PlayerCommand) -> Result<(), PlayerError> {
        Err(PlayerError::NotConnected)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DisconnectedPlayer, LocalFileUpdate, PlayerAdapter, PlayerCapabilities, PlayerCapability,
        PlayerCommand, PlayerError, PlayerMediaLoadFailureKind, PlayerMediaLoadOutcome,
        PlayerPlaybackTelemetryUpdate,
    };

    struct DummyPlayer;

    impl PlayerAdapter for DummyPlayer {
        fn name(&self) -> &'static str {
            "dummy"
        }
    }

    #[test]
    fn unsupported_methods_error_by_default() {
        let mut player = DummyPlayer;
        assert_eq!(
            player.open_file("movie.mkv"),
            Err(PlayerError::Unsupported("open_file"))
        );
        assert_eq!(
            player.set_option_string("script-opts", "osc=no"),
            Err(PlayerError::Unsupported("set_option_string"))
        );
        assert_eq!(
            player.apply_profile("fast"),
            Err(PlayerError::Unsupported("apply_profile"))
        );
        assert_eq!(
            player.set_paused(true),
            Err(PlayerError::Unsupported("set_paused"))
        );
        assert_eq!(
            player.set_position(12.0),
            Err(PlayerError::Unsupported("set_position"))
        );
        assert_eq!(
            player.set_playback_rate(0.95),
            Err(PlayerError::Unsupported("set_playback_rate"))
        );
        assert_eq!(
            player.set_muted(true),
            Err(PlayerError::Unsupported("set_muted"))
        );
        assert_eq!(
            player.set_volume(50.0),
            Err(PlayerError::Unsupported("set_volume"))
        );
        assert_eq!(
            player.set_deinterlace(true),
            Err(PlayerError::Unsupported("set_deinterlace"))
        );
        assert_eq!(
            player.set_keepaspect(true),
            Err(PlayerError::Unsupported("set_keepaspect"))
        );
        assert_eq!(
            player.set_keepaspect_window(true),
            Err(PlayerError::Unsupported("set_keepaspect_window"))
        );
        assert_eq!(
            player.set_fullscreen(true),
            Err(PlayerError::Unsupported("set_fullscreen"))
        );
        assert_eq!(
            player.set_ontop(true),
            Err(PlayerError::Unsupported("set_ontop"))
        );
        assert_eq!(
            player.set_border(true),
            Err(PlayerError::Unsupported("set_border"))
        );
        assert_eq!(
            player.set_force_window(true),
            Err(PlayerError::Unsupported("set_force_window"))
        );
        assert_eq!(
            player.set_keep_open(true),
            Err(PlayerError::Unsupported("set_keep_open"))
        );
        assert_eq!(
            player.set_keep_open_pause(true),
            Err(PlayerError::Unsupported("set_keep_open_pause"))
        );
        assert_eq!(
            player.set_cursor_autohide_fs_only(true),
            Err(PlayerError::Unsupported("set_cursor_autohide_fs_only"))
        );
        assert_eq!(
            player.set_stop_screensaver(true),
            Err(PlayerError::Unsupported("set_stop_screensaver"))
        );
        assert_eq!(
            player.set_sub_visibility(true),
            Err(PlayerError::Unsupported("set_sub_visibility"))
        );
        assert_eq!(
            player.set_osd_bar(true),
            Err(PlayerError::Unsupported("set_osd_bar"))
        );
        assert_eq!(
            player.set_window_maximized(true),
            Err(PlayerError::Unsupported("set_window_maximized"))
        );
        assert_eq!(
            player.set_window_minimized(true),
            Err(PlayerError::Unsupported("set_window_minimized"))
        );
        assert_eq!(player.name(), "dummy");
        assert_eq!(player.take_local_file_update(), None);
        assert_eq!(player.take_playback_telemetry_update(), None);
        assert_eq!(player.take_media_load_outcome(), None);
        assert_eq!(player.take_pending_chat_request(), None);
        assert_eq!(player.capabilities(), PlayerCapabilities::NONE);
        assert_eq!(
            player.execute(PlayerCommand::SetPaused(true)),
            Err(PlayerError::Unsupported("set_paused"))
        );
    }

    #[test]
    fn player_commands_advertise_required_capabilities() {
        assert_eq!(
            PlayerCommand::OpenFile("movie.mkv".to_owned()).required_capability(),
            PlayerCapability::OpenFile
        );
        assert_eq!(
            PlayerCommand::SetVolume(50.0).required_capability(),
            PlayerCapability::Audio
        );
        let capabilities = PlayerCapabilities::from_capabilities([
            PlayerCapability::OpenFile,
            PlayerCapability::Playback,
        ]);
        assert!(capabilities.contains(PlayerCapability::OpenFile));
        assert!(capabilities.contains(PlayerCapability::Playback));
        assert!(!capabilities.contains(PlayerCapability::Audio));
    }

    #[test]
    fn disconnected_player_rejects_commands_explicitly() {
        let mut player = DisconnectedPlayer;
        assert_eq!(
            player.execute(PlayerCommand::OpenFile("movie.mkv".to_owned())),
            Err(PlayerError::NotConnected)
        );
        assert_eq!(player.capabilities(), PlayerCapabilities::NONE);
    }

    #[test]
    fn local_file_update_builder_sets_expected_fields() {
        let update = LocalFileUpdate::new("movie.mkv")
            .with_duration_seconds(95.5)
            .with_size_bytes(123_456_789)
            .with_path("C:/media/movie.mkv");

        assert_eq!(update.name, "movie.mkv");
        assert_eq!(update.duration_seconds, Some(95.5));
        assert_eq!(update.size_bytes, Some(123_456_789));
        assert_eq!(update.path.as_deref(), Some("C:/media/movie.mkv"));
    }

    #[test]
    fn playback_telemetry_update_builder_sets_expected_fields() {
        let update = PlayerPlaybackTelemetryUpdate::default()
            .with_paused(true)
            .with_position_seconds(12.5)
            .with_playback_rate(0.95)
            .with_paused_for_cache(true)
            .with_cache_buffering_percent(37.5);

        assert_eq!(update.paused, Some(true));
        assert_eq!(update.position_seconds, Some(12.5));
        assert_eq!(update.playback_rate, Some(0.95));
        assert_eq!(update.paused_for_cache, Some(true));
        assert_eq!(update.cache_buffering_percent, Some(37.5));
    }

    #[test]
    fn media_load_outcome_builders_capture_success_and_failure() {
        let success =
            PlayerMediaLoadOutcome::success("requested.mp4", Some("loaded.mp4".to_owned()));
        assert!(success.succeeded());
        assert_eq!(success.loaded_target.as_deref(), Some("loaded.mp4"));

        let failure = PlayerMediaLoadOutcome::failure(
            "requested.mp4",
            None,
            PlayerMediaLoadFailureKind::HelperMissing,
            "yt-dlp was not found",
        );
        assert!(!failure.succeeded());
        assert_eq!(
            failure.failure.as_ref().map(|item| item.kind),
            Some(PlayerMediaLoadFailureKind::HelperMissing)
        );
        assert_eq!(
            failure.failure.as_ref().map(|item| item.message.as_str()),
            Some("yt-dlp was not found")
        );
    }
}
