#[derive(PartialEq, Eq)]
pub enum PlayerError {
    Unsupported(&'static str),
    NotConnected,
    OperationFailed(String),
}

impl std::fmt::Debug for PlayerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(operation) => formatter
                .debug_tuple("Unsupported")
                .field(operation)
                .finish(),
            Self::NotConnected => formatter.write_str("NotConnected"),
            Self::OperationFailed(message) => formatter
                .debug_struct("OperationFailed")
                .field("message_bytes", &message.len())
                .finish(),
        }
    }
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(operation) => {
                write!(formatter, "operation not supported: {operation}")
            }
            Self::NotConnected => formatter.write_str("player is not connected"),
            Self::OperationFailed(message) => {
                let message = if text_may_contain_credentials(message) {
                    sorotte_secret::REDACTED_SECRET
                } else {
                    message
                };
                write!(formatter, "operation failed: {message}")
            }
        }
    }
}

impl std::error::Error for PlayerError {}

fn text_may_contain_credentials(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower
        .match_indices(['=', ':'])
        .any(|(delimiter_index, _)| credential_key_before(&lower, delimiter_index))
        || lower
            .match_indices("%3d")
            .any(|(delimiter_index, _)| credential_key_before(&lower, delimiter_index))
}

fn credential_key_before(value: &str, delimiter_index: usize) -> bool {
    let key = value[..delimiter_index]
        .rsplit(['?', '&', ',', '{', '[', '\n', '\r', ':'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches(|character| matches!(character, '\\' | '"' | '\''));
    !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        && ["password", "token", "secret", "credential"]
            .into_iter()
            .any(|sensitive_word| key.contains(sensitive_word))
}

#[cfg(test)]
mod error_display_redaction_tests {
    use super::PlayerError;

    #[test]
    fn operation_failure_display_redacts_credentials_without_hiding_parser_diagnostics() {
        const MARKER: &str = "player-error-secret-canary-985d7a";
        let sensitive = PlayerError::OperationFailed(format!(
            "failed URL https://media.example/video?X-Plex-Token={MARKER}"
        ));
        let ordinary = PlayerError::OperationFailed("unexpected token: EOF".to_owned());

        assert!(!format!("{sensitive}").contains(MARKER));
        assert_eq!(
            format!("{ordinary}"),
            "operation failed: unexpected token: EOF"
        );
    }
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

#[derive(Clone, PartialEq)]
pub enum PlayerCommand {
    OpenFile(String),
    SetOptionString { name: String, value: String },
    ApplyProfile(String),
    SetPaused(bool),
    Play(PlayerPlayIntent),
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

impl std::fmt::Debug for PlayerCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self {
            Self::OpenFile(_) => "OpenFile(<redacted>)",
            Self::SetOptionString { .. } => "SetOptionString(<redacted>)",
            Self::ApplyProfile(_) => "ApplyProfile(<redacted>)",
            Self::SetPaused(_) => "SetPaused",
            Self::Play(_) => "Play",
            Self::SetPosition(_) => "SetPosition",
            Self::SetPlaybackRate(_) => "SetPlaybackRate",
            Self::SetMuted(_) => "SetMuted",
            Self::SetVolume(_) => "SetVolume",
            Self::SetDeinterlace(_) => "SetDeinterlace",
            Self::SetKeepaspect(_) => "SetKeepaspect",
            Self::SetKeepaspectWindow(_) => "SetKeepaspectWindow",
            Self::SetFullscreen(_) => "SetFullscreen",
            Self::SetOntop(_) => "SetOntop",
            Self::SetBorder(_) => "SetBorder",
            Self::SetForceWindow(_) => "SetForceWindow",
            Self::SetKeepOpen(_) => "SetKeepOpen",
            Self::SetKeepOpenPause(_) => "SetKeepOpenPause",
            Self::SetCursorAutohideFsOnly(_) => "SetCursorAutohideFsOnly",
            Self::SetStopScreensaver(_) => "SetStopScreensaver",
            Self::SetSubVisibility(_) => "SetSubVisibility",
            Self::SetOsdBar(_) => "SetOsdBar",
            Self::SetWindowMaximized(_) => "SetWindowMaximized",
            Self::SetWindowMinimized(_) => "SetWindowMinimized",
        };
        formatter.write_str(variant)
    }
}

/// Observation requirements for an unpause command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerPlayIntent {
    /// Resume an already-started file. mpv does not emit `playback-restart`
    /// for this transition, so fresh position advancement is the completion
    /// signal.
    Resume,
    /// Start playback after a new file load. The baseline is captured when
    /// loading begins, because a paused load can emit `playback-restart`
    /// before the later unpause command is sent.
    StartAfterLoad { baseline_restart_sequence: u64 },
    /// Start playback after a seek operation. The baseline is captured when
    /// seeking begins for the same reason: the restart can precede unpause.
    StartAfterSeek { baseline_restart_sequence: u64 },
}

impl PlayerCommand {
    pub const fn required_capability(&self) -> PlayerCapability {
        match self {
            Self::OpenFile(_) => PlayerCapability::OpenFile,
            Self::SetOptionString { .. } => PlayerCapability::SetOption,
            Self::ApplyProfile(_) => PlayerCapability::ApplyProfile,
            Self::SetPaused(_)
            | Self::Play(_)
            | Self::SetPosition(_)
            | Self::SetPlaybackRate(_) => PlayerCapability::Playback,
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

#[derive(Clone, PartialEq)]
pub struct LocalFileUpdate {
    pub name: String,
    pub duration_seconds: Option<f64>,
    pub size_bytes: Option<u64>,
    pub path: Option<String>,
}

impl std::fmt::Debug for LocalFileUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name: &dyn std::fmt::Debug = if self.name.contains("://") {
            &sorotte_secret::REDACTED_SECRET
        } else {
            &self.name
        };
        formatter
            .debug_struct("LocalFileUpdate")
            .field("name", name)
            .field("duration_seconds", &self.duration_seconds)
            .field("size_bytes", &self.size_bytes)
            .field(
                "path",
                &self.path.as_ref().map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .finish()
    }
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

/// Identifies one media load attempt within a player adapter instance.
///
/// Generations are local to an adapter. They allow a playback coordinator to
/// discard delayed observations from media that has already been replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerMediaGeneration(u64);

impl PlayerMediaGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for PlayerMediaGeneration {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

/// A monotonic timestamp measured from the creation of a player adapter.
///
/// The timestamp deliberately has no wall-clock meaning. Consumers can use it
/// to order observations and calculate local elapsed durations without making
/// a platform-specific `Instant` part of the player API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerObservationTimestamp(std::time::Duration);

impl PlayerObservationTimestamp {
    pub const fn from_adapter_start(elapsed: std::time::Duration) -> Self {
        Self(elapsed)
    }

    pub const fn elapsed_since_adapter_start(self) -> std::time::Duration {
        self.0
    }
}

/// Identifies one tracked command within a player adapter instance.
///
/// IDs are local to an adapter and remain valid until the adapter reports a
/// terminal [`PlayerCommandResult`] for them. A command reply from the player
/// only establishes [`PlayerCommandProgressState::Accepted`]; callers must
/// wait for the corresponding observed result before treating the requested
/// effect as applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerCommandId(u64);

impl PlayerCommandId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for PlayerCommandId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

/// Why an accepted player command reached a failed terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerCommandFailureKind {
    /// No matching player observation arrived before the adapter deadline.
    TimedOut,
    /// The media ended or failed before the requested effect was observed.
    MediaEnded,
    /// The player transport disconnected after accepting the command.
    TransportDisconnected,
    /// The adapter could not classify the observed player failure further.
    Unknown,
}

/// Terminal result for an observation-acknowledged player command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerCommandResult {
    /// The requested effect was observed on the matching media generation.
    Completed,
    /// A newer command made this command obsolete before it completed.
    Superseded,
    /// The command was accepted but could not be observed to completion.
    Failed(PlayerCommandFailureKind),
}

/// Current lifecycle state of a tracked player command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerCommandProgressState {
    /// The underlying player accepted the command. This is not completion.
    Accepted,
    /// The adapter observed a terminal command result.
    Finished(PlayerCommandResult),
}

/// One progress update for an observation-acknowledged player command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerCommandProgress {
    pub command_id: PlayerCommandId,
    pub media_generation: Option<PlayerMediaGeneration>,
    pub observed_at: Option<PlayerObservationTimestamp>,
    pub observed_position_seconds: Option<f64>,
    pub state: PlayerCommandProgressState,
}

impl PlayerCommandProgress {
    pub const fn accepted(
        command_id: PlayerCommandId,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
    ) -> Self {
        Self {
            command_id,
            media_generation,
            observed_at,
            observed_position_seconds: None,
            state: PlayerCommandProgressState::Accepted,
        }
    }

    pub const fn finished(
        command_id: PlayerCommandId,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
        observed_position_seconds: Option<f64>,
        result: PlayerCommandResult,
    ) -> Self {
        Self {
            command_id,
            media_generation,
            observed_at,
            observed_position_seconds,
            state: PlayerCommandProgressState::Finished(result),
        }
    }

    pub const fn result(self) -> Option<PlayerCommandResult> {
        match self.state {
            PlayerCommandProgressState::Accepted => None,
            PlayerCommandProgressState::Finished(result) => Some(result),
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self.state, PlayerCommandProgressState::Finished(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlayerTransportPhase {
    #[default]
    Empty,
    Loading,
    Prebuffering,
    ReadyPaused,
    Playing,
    Rebuffering,
    Seeking,
    Ended,
    Failed,
}

/// Player-observed classification of the active media timeline.
///
/// `Unknown` is an explicit pending classification for adapters that can
/// distinguish finite VOD from a moving live window. It is different from an
/// absent sparse field, which means that the adapter provides no such signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlayerTimelineKind {
    #[default]
    Unknown,
    Vod,
    SlidingLive,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSeekableRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl PlayerSeekableRange {
    pub const fn new(start_seconds: f64, end_seconds: f64) -> Self {
        Self {
            start_seconds,
            end_seconds,
        }
    }

    pub fn shifted(self, delta_seconds: f64) -> Self {
        Self {
            start_seconds: self.start_seconds + delta_seconds,
            end_seconds: self.end_seconds + delta_seconds,
        }
    }
}

/// Generation-aware observations used for transport readiness and recovery.
///
/// This is intentionally separate from [`PlayerPlaybackTelemetryUpdate`]. The
/// older type remains source-compatible for existing clients while this richer
/// stream can evolve around observed player behavior instead of command
/// acceptance. Fields are sparse: `None` means that observation was not part
/// of this update, not that the player necessarily lacks the capability.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerTransportTelemetryUpdate {
    pub media_generation: Option<PlayerMediaGeneration>,
    pub observed_at: Option<PlayerObservationTimestamp>,
    pub phase: Option<PlayerTransportPhase>,
    pub position_seconds: Option<f64>,
    pub playback_rate: Option<f64>,
    pub logical_pause: Option<bool>,
    pub paused_for_cache: Option<bool>,
    pub cache_buffering_percent: Option<f64>,
    pub seeking: Option<bool>,
    pub seekable: Option<bool>,
    /// Current-generation player classification of the media timeline.
    pub timeline_kind: Option<PlayerTimelineKind>,
    pub core_idle: Option<bool>,
    pub demuxer_cache_idle: Option<bool>,
    pub playback_restart_sequence: Option<u64>,
    pub eof_reached: Option<bool>,
    pub seekable_ranges: Option<Vec<PlayerSeekableRange>>,
    /// Conservative locally usable window for a confirmed sliding source.
    /// This never claims to be the source's complete DVR window; separate
    /// tracks and uncached source regions can make it a strict subset.
    pub known_live_seekable_window: Option<PlayerSeekableRange>,
    pub buffered_ahead_seconds: Option<f64>,
    pub buffered_ahead_bytes: Option<u64>,
    pub input_rate_bytes_per_second: Option<u64>,
    pub error_kind: Option<PlayerMediaLoadFailureKind>,
}

impl PlayerTransportTelemetryUpdate {
    pub fn new(
        media_generation: PlayerMediaGeneration,
        observed_at: PlayerObservationTimestamp,
    ) -> Self {
        Self {
            media_generation: Some(media_generation),
            observed_at: Some(observed_at),
            ..Self::default()
        }
    }

    pub fn with_phase(mut self, phase: PlayerTransportPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_position_seconds(mut self, position_seconds: f64) -> Self {
        self.position_seconds = Some(position_seconds);
        self
    }

    pub fn with_logical_pause(mut self, logical_pause: bool) -> Self {
        self.logical_pause = Some(logical_pause);
        self
    }

    pub fn merge_from(&mut self, newer: Self) {
        let replaces_live_window = newer.timeline_kind == Some(PlayerTimelineKind::SlidingLive)
            && newer.seekable_ranges.is_some();
        if let Some(media_generation) = newer.media_generation {
            self.media_generation = Some(media_generation);
        }
        if let Some(observed_at) = newer.observed_at {
            self.observed_at = Some(observed_at);
        }
        if let Some(phase) = newer.phase {
            self.phase = Some(phase);
        }
        if let Some(position_seconds) = newer.position_seconds {
            self.position_seconds = Some(position_seconds);
        }
        if let Some(playback_rate) = newer.playback_rate {
            self.playback_rate = Some(playback_rate);
        }
        if let Some(paused_for_cache) = newer.paused_for_cache {
            self.paused_for_cache = Some(paused_for_cache);
            if paused_for_cache && self.logical_pause == Some(true) {
                self.logical_pause = None;
            }
        }
        if let Some(logical_pause) = newer.logical_pause
            && !(logical_pause && self.paused_for_cache == Some(true))
        {
            self.logical_pause = Some(logical_pause);
        }
        if let Some(cache_buffering_percent) = newer.cache_buffering_percent {
            self.cache_buffering_percent = Some(cache_buffering_percent);
        }
        if let Some(seeking) = newer.seeking {
            self.seeking = Some(seeking);
        }
        if let Some(seekable) = newer.seekable {
            self.seekable = Some(seekable);
        }
        if let Some(timeline_kind) = newer.timeline_kind {
            self.timeline_kind = Some(timeline_kind);
        }
        if let Some(core_idle) = newer.core_idle {
            self.core_idle = Some(core_idle);
        }
        if let Some(demuxer_cache_idle) = newer.demuxer_cache_idle {
            self.demuxer_cache_idle = Some(demuxer_cache_idle);
        }
        if let Some(playback_restart_sequence) = newer.playback_restart_sequence {
            self.playback_restart_sequence = Some(playback_restart_sequence);
        }
        if let Some(eof_reached) = newer.eof_reached {
            self.eof_reached = Some(eof_reached);
        }
        if let Some(seekable_ranges) = newer.seekable_ranges {
            self.seekable_ranges = Some(seekable_ranges);
        }
        if replaces_live_window {
            self.known_live_seekable_window = newer.known_live_seekable_window;
        } else if let Some(known_live_seekable_window) = newer.known_live_seekable_window {
            self.known_live_seekable_window = Some(known_live_seekable_window);
        }
        if let Some(buffered_ahead_seconds) = newer.buffered_ahead_seconds {
            self.buffered_ahead_seconds = Some(buffered_ahead_seconds);
        }
        if let Some(buffered_ahead_bytes) = newer.buffered_ahead_bytes {
            self.buffered_ahead_bytes = Some(buffered_ahead_bytes);
        }
        if let Some(input_rate_bytes_per_second) = newer.input_rate_bytes_per_second {
            self.input_rate_bytes_per_second = Some(input_rate_bytes_per_second);
        }
        if let Some(error_kind) = newer.error_kind {
            self.error_kind = Some(error_kind);
        }
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

#[derive(Clone, PartialEq, Eq)]
pub struct PlayerMediaLoadFailure {
    pub kind: PlayerMediaLoadFailureKind,
    pub message: String,
}

impl std::fmt::Debug for PlayerMediaLoadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayerMediaLoadFailure")
            .field("kind", &self.kind)
            .field("message_bytes", &self.message.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlayerMediaLoadOutcome {
    pub requested_target: String,
    pub loaded_target: Option<String>,
    pub failure: Option<PlayerMediaLoadFailure>,
}

impl std::fmt::Debug for PlayerMediaLoadOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayerMediaLoadOutcome")
            .field("requested_target", &sorotte_secret::REDACTED_SECRET)
            .field(
                "loaded_target",
                &self
                    .loaded_target
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("failure", &self.failure)
            .finish()
    }
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
            PlayerCommand::Play(_) => self.set_paused(false),
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
    /// Executes a command whose effect must be acknowledged by player
    /// observations.
    ///
    /// The compatibility command methods remain available. Adapters that do
    /// not implement tracked execution reject this additive operation rather
    /// than manufacturing an ID with no completion semantics.
    fn execute_tracked(&mut self, _command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        Err(PlayerError::Unsupported("execute_tracked"))
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
    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        None
    }
    fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
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

    fn execute_tracked(&mut self, _command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        Err(PlayerError::NotConnected)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DisconnectedPlayer, LocalFileUpdate, PlayerAdapter, PlayerCapabilities, PlayerCapability,
        PlayerCommand, PlayerCommandFailureKind, PlayerCommandId, PlayerCommandProgress,
        PlayerCommandProgressState, PlayerCommandResult, PlayerError, PlayerMediaGeneration,
        PlayerMediaLoadFailureKind, PlayerMediaLoadOutcome, PlayerObservationTimestamp,
        PlayerPlaybackTelemetryUpdate, PlayerSeekableRange, PlayerTimelineKind,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
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
        assert_eq!(player.take_command_progress(), None);
        assert_eq!(player.take_media_load_outcome(), None);
        assert_eq!(player.take_pending_chat_request(), None);
        assert_eq!(player.capabilities(), PlayerCapabilities::NONE);
        assert_eq!(
            player.execute(PlayerCommand::SetPaused(true)),
            Err(PlayerError::Unsupported("set_paused"))
        );
        assert_eq!(
            player.execute_tracked(PlayerCommand::SetPaused(true)),
            Err(PlayerError::Unsupported("execute_tracked"))
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
        assert_eq!(
            player.execute_tracked(PlayerCommand::SetPaused(false)),
            Err(PlayerError::NotConnected)
        );
        assert_eq!(player.capabilities(), PlayerCapabilities::NONE);
    }

    #[test]
    fn command_progress_distinguishes_acceptance_from_terminal_results() {
        let command_id = PlayerCommandId::new(9);
        let generation = PlayerMediaGeneration::new(4);
        let observed_at =
            PlayerObservationTimestamp::from_adapter_start(std::time::Duration::from_millis(20));
        let accepted =
            PlayerCommandProgress::accepted(command_id, Some(generation), Some(observed_at));
        let timed_out = PlayerCommandProgress::finished(
            command_id,
            Some(generation),
            Some(observed_at),
            Some(12.5),
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
        );

        assert_eq!(command_id.get(), 9);
        assert_eq!(accepted.state, PlayerCommandProgressState::Accepted);
        assert_eq!(accepted.result(), None);
        assert!(!accepted.is_terminal());
        assert_eq!(
            timed_out.result(),
            Some(PlayerCommandResult::Failed(
                PlayerCommandFailureKind::TimedOut
            ))
        );
        assert!(timed_out.is_terminal());
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
    fn transport_telemetry_carries_generation_lifecycle_and_cache_hints() {
        let generation = PlayerMediaGeneration::new(7);
        let observed_at =
            PlayerObservationTimestamp::from_adapter_start(std::time::Duration::from_millis(125));
        let mut update = PlayerTransportTelemetryUpdate::new(generation, observed_at)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position_seconds(42.5)
            .with_logical_pause(false);
        update.paused_for_cache = Some(true);
        update.seeking = Some(false);
        update.seekable = Some(true);
        update.timeline_kind = Some(PlayerTimelineKind::SlidingLive);
        update.seekable_ranges = Some(vec![PlayerSeekableRange::new(10.0, 80.0)]);
        update.known_live_seekable_window = Some(PlayerSeekableRange::new(10.0, 80.0));
        update.buffered_ahead_seconds = Some(5.25);
        update.input_rate_bytes_per_second = Some(2_000_000);

        assert_eq!(update.media_generation, Some(generation));
        assert_eq!(generation.get(), 7);
        assert_eq!(
            update
                .observed_at
                .expect("timestamp should be present")
                .elapsed_since_adapter_start(),
            std::time::Duration::from_millis(125)
        );
        assert_eq!(update.phase, Some(PlayerTransportPhase::Rebuffering));
        assert_eq!(update.position_seconds, Some(42.5));
        assert_eq!(update.logical_pause, Some(false));
        assert_eq!(update.paused_for_cache, Some(true));
        assert_eq!(update.timeline_kind, Some(PlayerTimelineKind::SlidingLive));
        assert_eq!(
            update.known_live_seekable_window,
            Some(PlayerSeekableRange::new(10.0, 80.0))
        );
        assert_eq!(update.seekable_ranges.as_ref().map(Vec::len), Some(1));
        assert_eq!(update.buffered_ahead_seconds, Some(5.25));
        assert_eq!(update.input_rate_bytes_per_second, Some(2_000_000));
    }

    #[test]
    fn merging_empty_live_cache_snapshot_clears_previous_usable_window() {
        let mut pending = PlayerTransportTelemetryUpdate {
            timeline_kind: Some(PlayerTimelineKind::SlidingLive),
            seekable_ranges: Some(vec![PlayerSeekableRange::new(80.0, 100.0)]),
            known_live_seekable_window: Some(PlayerSeekableRange::new(80.0, 100.0)),
            ..PlayerTransportTelemetryUpdate::default()
        };
        pending.merge_from(PlayerTransportTelemetryUpdate {
            timeline_kind: Some(PlayerTimelineKind::SlidingLive),
            seekable_ranges: Some(Vec::new()),
            known_live_seekable_window: None,
            ..PlayerTransportTelemetryUpdate::default()
        });

        assert_eq!(pending.seekable_ranges, Some(Vec::new()));
        assert_eq!(pending.known_live_seekable_window, None);
    }

    #[test]
    fn merging_cache_pause_does_not_turn_it_into_logical_pause() {
        let mut pending = PlayerTransportTelemetryUpdate {
            logical_pause: Some(true),
            ..PlayerTransportTelemetryUpdate::default()
        };
        pending.merge_from(PlayerTransportTelemetryUpdate {
            paused_for_cache: Some(true),
            ..PlayerTransportTelemetryUpdate::default()
        });

        assert_eq!(pending.logical_pause, None);
        assert_eq!(pending.paused_for_cache, Some(true));
    }

    #[test]
    fn cache_release_update_can_restore_an_explicit_logical_pause() {
        let mut pending = PlayerTransportTelemetryUpdate {
            paused_for_cache: Some(true),
            ..PlayerTransportTelemetryUpdate::default()
        };
        pending.merge_from(PlayerTransportTelemetryUpdate {
            logical_pause: Some(true),
            paused_for_cache: Some(false),
            ..PlayerTransportTelemetryUpdate::default()
        });

        assert_eq!(pending.logical_pause, Some(true));
        assert_eq!(pending.paused_for_cache, Some(false));
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

    #[test]
    fn media_target_debug_canary_redacts_tokenized_urls() {
        let secret = "player-media-token-canary";
        let target = format!("https://plex.invalid/video?X-Plex-Token={secret}");
        let command = PlayerCommand::OpenFile(target.clone());
        let update = LocalFileUpdate::new(target.clone()).with_path(target.clone());
        let outcome = PlayerMediaLoadOutcome::failure(
            target.clone(),
            Some(target.clone()),
            PlayerMediaLoadFailureKind::Network,
            format!("failed to load {target}"),
        );
        let error = PlayerError::OperationFailed(format!("failed to load {target}"));

        for debug in [
            format!("{command:?}"),
            format!("{update:?}"),
            format!("{outcome:?}"),
        ] {
            assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!debug.contains(secret));
        }
        for rendered in [format!("{error:?}"), error.to_string()] {
            assert!(!rendered.contains(secret));
        }
        assert!(error.to_string().contains(sorotte_secret::REDACTED_SECRET));
    }
}
