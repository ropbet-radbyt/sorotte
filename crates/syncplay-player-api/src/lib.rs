#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlayerError {
    #[error("operation not supported: {0}")]
    Unsupported(&'static str),
    #[error("operation failed: {0}")]
    OperationFailed(String),
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
}

pub trait PlayerAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn open_file(&mut self, _path: &str) -> Result<(), PlayerError> {
        Err(PlayerError::Unsupported("open_file"))
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
}

#[cfg(test)]
mod tests {
    use super::{LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate};

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
            .with_playback_rate(0.95);

        assert_eq!(update.paused, Some(true));
        assert_eq!(update.position_seconds, Some(12.5));
        assert_eq!(update.playback_rate, Some(0.95));
    }
}
