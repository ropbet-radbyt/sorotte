use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
};

use sorotte_client_core::{
    DesyncCorrectionConfig, PrivacyMode, ReadinessAutoplayConfig, SessionBehaviorConfig,
    UnpauseActionMode,
};
use sorotte_secret::{RedactedCommandArgs, SecretValue};

use crate::{
    legacy_language::normalized_legacy_runtime_language_tag_legacy_compatible,
    legacy_runtime_config::{
        normalize_controlled_room_input_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
    },
    legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsV1},
};

const DEFAULT_SERVER_PORT: u16 = 8999;
const DEFAULT_FIRST_FILE_TIMEOUT_SECONDS: f64 = 25.0;
const DEFAULT_FOLDER_SEARCH_TIMEOUT_SECONDS: f64 = 20.0;
const DEFAULT_FOLDER_SEARCH_DOUBLE_CHECK_SECONDS: f64 = 30.0;
const DEFAULT_FOLDER_SEARCH_WARNING_SECONDS: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfigIssue {
    pub field: String,
    pub message: String,
}

impl ClientConfigIssue {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ClientConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfigErrors {
    issues: Vec<ClientConfigIssue>,
}

impl ClientConfigErrors {
    pub fn issues(&self) -> &[ClientConfigIssue] {
        &self.issues
    }

    pub fn into_issues(self) -> Vec<ClientConfigIssue> {
        self.issues
    }
}

impl fmt::Display for ClientConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid client configuration")?;
        for issue in &self.issues {
            write!(f, "\n- {issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ClientConfigErrors {}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientConfigResolution {
    pub config: ClientConfig,
    pub issues: Vec<ClientConfigIssue>,
}

impl ClientConfigResolution {
    pub fn into_result(self) -> Result<ClientConfig, ClientConfigErrors> {
        if self.issues.is_empty() {
            Ok(self.config)
        } else {
            Err(ClientConfigErrors {
                issues: self.issues,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Username(String);

impl Username {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty_trimmed(value.into(), "username").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomName(String);

impl RoomName {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty_trimmed(value.into(), "room name").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for RoomName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Seconds(f64);

impl Seconds {
    pub fn new(value: f64) -> Result<Self, String> {
        if !value.is_finite() {
            return Err("must be finite".to_owned());
        }
        if value < 0.0 {
            return Err("must be zero or greater".to_owned());
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f64 {
        self.0
    }

    pub fn as_millis(self) -> u64 {
        (self.0 * 1_000.0).min(u64::MAX as f64) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct PlaybackRate(f64);

impl PlaybackRate {
    pub fn new(value: f64) -> Result<Self, String> {
        if !value.is_finite() {
            return Err("must be finite".to_owned());
        }
        if value <= 0.0 {
            return Err("must be greater than zero".to_owned());
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Percent(f64);

impl Percent {
    pub fn new(value: f64) -> Result<Self, String> {
        if !value.is_finite() {
            return Err("must be finite".to_owned());
        }
        if !(0.0..=100.0).contains(&value) {
            return Err("must be between 0 and 100".to_owned());
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamingQualityPreset {
    #[default]
    Auto,
    Best,
    Balanced,
    Max1080p,
    Max720p,
    Max480p,
    Compatibility,
    Custom,
}

impl StreamingQualityPreset {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "best" => Some(Self::Best),
            "balanced" => Some(Self::Balanced),
            "1080p" | "max1080p" | "max-1080p" => Some(Self::Max1080p),
            "720p" | "max720p" | "max-720p" => Some(Self::Max720p),
            "480p" | "max480p" | "max-480p" => Some(Self::Max480p),
            "compatibility" | "combined" => Some(Self::Compatibility),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Best => "best",
            Self::Balanced => "balanced",
            Self::Max1080p => "1080p",
            Self::Max720p => "720p",
            Self::Max480p => "480p",
            Self::Compatibility => "compatibility",
            Self::Custom => "custom",
        }
    }

    fn ytdl_format(self, custom: Option<&str>) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Best => Some("bestvideo*+bestaudio/best".to_owned()),
            Self::Balanced => {
                Some("bestvideo*[height<=1080][fps<=30]+bestaudio/best[height<=1080]".to_owned())
            }
            Self::Max1080p => {
                Some("bestvideo*[height<=1080]+bestaudio/best[height<=1080]".to_owned())
            }
            Self::Max720p => Some("bestvideo*[height<=720]+bestaudio/best[height<=720]".to_owned()),
            Self::Max480p => Some("bestvideo*[height<=480]+bestaudio/best[height<=480]".to_owned()),
            Self::Compatibility => {
                Some("best[ext=mp4][height<=720]/best[height<=720]/best".to_owned())
            }
            Self::Custom => custom
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamingRecoveryPolicy {
    PreserveContent,
    #[default]
    Balanced,
    StayClosest,
    PauseRoom,
}

impl StreamingRecoveryPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "preserve" | "preserve-content" => Some(Self::PreserveContent),
            "balanced" => Some(Self::Balanced),
            "stay-closest" | "closest" => Some(Self::StayClosest),
            "pause-room" => Some(Self::PauseRoom),
            _ => None,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::PreserveContent => "preserve-content",
            Self::Balanced => "balanced",
            Self::StayClosest => "stay-closest",
            Self::PauseRoom => "pause-room",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomBufferingPolicy {
    #[default]
    Independent,
    PauseController,
    PauseEligible,
    Quorum,
}

impl RoomBufferingPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "independent" => Some(Self::Independent),
            "pause-controller" | "controller" | "host" => Some(Self::PauseController),
            "pause-eligible" | "any-eligible" | "all" => Some(Self::PauseEligible),
            "quorum" => Some(Self::Quorum),
            _ => None,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::PauseController => "pause-controller",
            Self::PauseEligible => "pause-eligible",
            Self::Quorum => "quorum",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartSynchronizationPolicy {
    #[default]
    Immediate,
    WaitForController,
    WaitForAllEligible,
    Quorum,
}

impl StartSynchronizationPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "immediate" | "legacy" => Some(Self::Immediate),
            "wait-controller" | "controller" => Some(Self::WaitForController),
            "wait-all" | "all-eligible" | "all" => Some(Self::WaitForAllEligible),
            "quorum" => Some(Self::Quorum),
            _ => None,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::WaitForController => "wait-controller",
            Self::WaitForAllEligible => "wait-all",
            Self::Quorum => "quorum",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartTimeoutAction {
    #[default]
    Continue,
    RemainPaused,
    AskController,
}

impl StartTimeoutAction {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "continue" => Some(Self::Continue),
            "remain-paused" | "paused" => Some(Self::RemainPaused),
            "ask-controller" | "ask" => Some(Self::AskController),
            _ => None,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::RemainPaused => "remain-paused",
            Self::AskController => "ask-controller",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamingBufferConfig {
    pub target: Seconds,
    pub read_ahead: Seconds,
    pub memory_cache_mebibytes: u64,
    pub disk_cache_enabled: bool,
}

impl Default for StreamingBufferConfig {
    fn default() -> Self {
        Self {
            target: Seconds(5.0),
            read_ahead: Seconds(30.0),
            memory_cache_mebibytes: 150,
            disk_cache_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamingRecoveryConfig {
    pub policy: StreamingRecoveryPolicy,
    pub max_catchup_rate: PlaybackRate,
    pub hard_seek_threshold: Seconds,
    pub max_hard_seeks_per_episode: u32,
    pub stability_interval: Seconds,
    pub retry_budget: u32,
    pub cooldown: Seconds,
}

impl Default for StreamingRecoveryConfig {
    fn default() -> Self {
        Self {
            policy: StreamingRecoveryPolicy::Balanced,
            max_catchup_rate: PlaybackRate(1.05),
            hard_seek_threshold: Seconds(8.0),
            max_hard_seeks_per_episode: 1,
            stability_interval: Seconds(4.0),
            retry_budget: 1,
            cooldown: Seconds(10.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoomBufferingConfig {
    pub policy: RoomBufferingPolicy,
    pub quorum: Percent,
    pub maximum_pause: Seconds,
}

impl Default for RoomBufferingConfig {
    fn default() -> Self {
        Self {
            policy: RoomBufferingPolicy::Independent,
            quorum: Percent(75.0),
            maximum_pause: Seconds(30.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartSynchronizationConfig {
    pub policy: StartSynchronizationPolicy,
    pub quorum: Percent,
    pub timeout: Seconds,
    pub timeout_action: StartTimeoutAction,
}

impl Default for StartSynchronizationConfig {
    fn default() -> Self {
        Self {
            policy: StartSynchronizationPolicy::Immediate,
            quorum: Percent(75.0),
            timeout: Seconds(15.0),
            timeout_action: StartTimeoutAction::Continue,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamingPlaybackConfig {
    pub quality: StreamingQualityPreset,
    pub custom_format: Option<String>,
    pub buffering: StreamingBufferConfig,
    pub recovery: StreamingRecoveryConfig,
    pub room_buffering: RoomBufferingConfig,
    pub start_synchronization: StartSynchronizationConfig,
    pub quality_downgrade_suggestions: bool,
}

impl Default for StreamingPlaybackConfig {
    fn default() -> Self {
        Self {
            quality: StreamingQualityPreset::Auto,
            custom_format: None,
            buffering: StreamingBufferConfig::default(),
            recovery: StreamingRecoveryConfig::default(),
            room_buffering: RoomBufferingConfig::default(),
            start_synchronization: StartSynchronizationConfig::default(),
            quality_downgrade_suggestions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveMpvStreamingOption {
    pub name: String,
    pub configured_value: String,
    pub effective_value: String,
    pub overridden_by_advanced_arguments: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingQualitySuggestionReason {
    RepeatedRebuffering,
    InsufficientObservedInputRate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingQualityDowngradeSuggestion {
    pub current: StreamingQualityPreset,
    pub recommended: StreamingQualityPreset,
    pub reason: StreamingQualitySuggestionReason,
}

impl StreamingPlaybackConfig {
    pub fn playback_coordinator_config(&self) -> sorotte_client_core::PlaybackCoordinatorConfig {
        sorotte_client_core::PlaybackCoordinatorConfig {
            recovery_policy: match self.recovery.policy {
                StreamingRecoveryPolicy::PreserveContent => {
                    sorotte_client_core::RecoveryPolicy::PreserveContent
                }
                StreamingRecoveryPolicy::Balanced => sorotte_client_core::RecoveryPolicy::Balanced,
                StreamingRecoveryPolicy::StayClosest => {
                    sorotte_client_core::RecoveryPolicy::StayClosest
                }
                StreamingRecoveryPolicy::PauseRoom => {
                    sorotte_client_core::RecoveryPolicy::PauseRoom
                }
            },
            hard_seek_threshold_seconds: self.recovery.hard_seek_threshold.get(),
            maximum_catchup_rate: self.recovery.max_catchup_rate.get(),
            maximum_hard_seeks_per_episode: self.recovery.max_hard_seeks_per_episode,
            stability_interval_seconds: self.recovery.stability_interval.get(),
            command_retry_budget: self.recovery.retry_budget,
            command_retry_cooldown_seconds: self.recovery.cooldown.get(),
            seek_preparation_minimum_headroom_seconds: self.buffering.target.get(),
            ..sorotte_client_core::PlaybackCoordinatorConfig::default()
        }
    }

    /// Returns options intended for mpv's per-file network-media load map.
    ///
    /// These must not be installed as process-wide defaults: in particular,
    /// `cache-secs` would otherwise cap local-file read-ahead as well.
    pub fn network_media_mpv_arguments(&self) -> Vec<String> {
        let mut options = vec![
            ("cache", "auto".to_owned()),
            ("cache-pause", "yes".to_owned()),
            ("cache-pause-initial", "yes".to_owned()),
            (
                "cache-pause-wait",
                format_decimal(self.buffering.target.get()),
            ),
            (
                "cache-secs",
                format_decimal(self.buffering.read_ahead.get()),
            ),
            (
                "demuxer-max-bytes",
                format!("{}MiB", self.buffering.memory_cache_mebibytes),
            ),
            (
                "cache-on-disk",
                if self.buffering.disk_cache_enabled {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
            ),
        ];
        if let Some(format) = self.quality.ytdl_format(self.custom_format.as_deref()) {
            options.push(("ytdl-format", format));
        }
        options
            .into_iter()
            .map(|(name, value)| format!("--{name}={value}"))
            .collect()
    }

    pub fn effective_mpv_options(
        &self,
        advanced_arguments: &[String],
    ) -> Vec<EffectiveMpvStreamingOption> {
        let advanced = parse_mpv_option_arguments(advanced_arguments);
        self.network_media_mpv_arguments()
            .into_iter()
            .filter_map(|argument| {
                let body = argument.strip_prefix("--")?;
                let (name, configured_value) = body.split_once('=')?;
                let override_value = advanced.get(name);
                Some(EffectiveMpvStreamingOption {
                    name: name.to_owned(),
                    configured_value: configured_value.to_owned(),
                    effective_value: override_value
                        .cloned()
                        .unwrap_or_else(|| configured_value.to_owned()),
                    overridden_by_advanced_arguments: override_value.is_some(),
                })
            })
            .collect()
    }

    pub fn quality_downgrade_suggestion(
        &self,
        metrics: &sorotte_client_core::PlaybackCoordinatorMetrics,
        approximate_selected_bitrate_bytes_per_second: Option<u64>,
    ) -> Option<StreamingQualityDowngradeSuggestion> {
        if !self.quality_downgrade_suggestions {
            return None;
        }
        let reason =
            if metrics.buffer_episode_count >= 3 || metrics.total_buffer_duration_seconds >= 15.0 {
                StreamingQualitySuggestionReason::RepeatedRebuffering
            } else if approximate_selected_bitrate_bytes_per_second
                .zip(metrics.last_input_rate_bytes_per_second)
                .is_some_and(|(required, observed)| {
                    observed.saturating_mul(100) < required.saturating_mul(115)
                })
            {
                StreamingQualitySuggestionReason::InsufficientObservedInputRate
            } else {
                return None;
            };
        let recommended = match self.quality {
            StreamingQualityPreset::Auto
            | StreamingQualityPreset::Best
            | StreamingQualityPreset::Custom => StreamingQualityPreset::Balanced,
            StreamingQualityPreset::Balanced | StreamingQualityPreset::Max1080p => {
                StreamingQualityPreset::Max720p
            }
            StreamingQualityPreset::Max720p => StreamingQualityPreset::Max480p,
            StreamingQualityPreset::Max480p => StreamingQualityPreset::Compatibility,
            StreamingQualityPreset::Compatibility => return None,
        };
        Some(StreamingQualityDowngradeSuggestion {
            current: self.quality,
            recommended,
            reason,
        })
    }
}

fn format_decimal(value: f64) -> String {
    let mut value = format!("{value:.3}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

fn parse_mpv_option_arguments(arguments: &[String]) -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        let Some(body) = argument.strip_prefix("--") else {
            index += 1;
            continue;
        };
        if let Some(name) = body.strip_prefix("no-") {
            options.insert(name.to_owned(), "no".to_owned());
        } else if let Some((name, value)) = body.split_once('=') {
            options.insert(name.to_owned(), value.to_owned());
        } else if let Some(value) = arguments
            .get(index + 1)
            .filter(|value| !value.starts_with('-'))
        {
            options.insert(body.to_owned(), value.clone());
            index += 1;
        } else {
            options.insert(body.to_owned(), "yes".to_owned());
        }
        index += 1;
    }
    options
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerPort(u16);

impl ServerPort {
    pub fn new(value: u16) -> Result<Self, String> {
        if value == 0 {
            return Err("must be between 1 and 65535".to_owned());
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicServerConfig {
    pub label: String,
    pub address: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TlsPolicy {
    RequireTls,
    #[default]
    PreferTls,
    Plaintext,
}

impl TlsPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "requiretls" | "require_tls" | "require-tls" | "require" => Some(Self::RequireTls),
            "prefertls" | "prefer_tls" | "prefer-tls" | "prefer" | "auto" => Some(Self::PreferTls),
            "plaintext" | "plain" | "disabled" | "off" => Some(Self::Plaintext),
            _ => None,
        }
    }

    pub const fn default_for_credentials(has_credentials: bool) -> Self {
        if has_credentials {
            Self::RequireTls
        } else {
            Self::PreferTls
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientConfig {
    pub connection: ConnectionConfig,
    pub synchronization: SyncConfig,
    pub playback: PlaybackConfig,
    pub readiness: ReadinessConfig,
    pub interface: InterfaceConfig,
    pub plugins: PluginConfig,
    pub plex: PlexConfig,
    pub media_match: MediaMatchConfig,
}

impl ClientConfig {
    pub fn resolve(settings: &StoredClientSettingsV1) -> ClientConfigResolution {
        resolve_client_config(settings)
    }

    pub fn try_from_stored(settings: &StoredClientSettingsV1) -> Result<Self, ClientConfigErrors> {
        resolve_client_config(settings).into_result()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionConfig {
    pub host: Option<String>,
    pub port: ServerPort,
    pub server_password: Option<SecretValue>,
    pub username: Option<Username>,
    pub room: Option<RoomName>,
    pub controlled_room_password: Option<SecretValue>,
    pub tls_policy: TlsPolicy,
    pub room_history: Vec<RoomName>,
    pub public_servers: Vec<PublicServerConfig>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            host: None,
            port: ServerPort(DEFAULT_SERVER_PORT),
            server_password: None,
            username: None,
            room: None,
            controlled_room_password: None,
            tls_policy: TlsPolicy::PreferTls,
            room_history: Vec::new(),
            public_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncConfig {
    pub rewind_on_desync: bool,
    pub fastforward_on_desync: bool,
    pub slow_on_desync: bool,
    pub dont_slow_down_with_me: bool,
    pub rewind_threshold: Seconds,
    pub fastforward_threshold: Seconds,
    pub slowdown_threshold: Seconds,
}

impl Default for SyncConfig {
    fn default() -> Self {
        let defaults = DesyncCorrectionConfig::default();
        Self {
            rewind_on_desync: defaults.rewind_on_desync,
            fastforward_on_desync: defaults.fastforward_on_desync,
            slow_on_desync: defaults.slow_on_desync,
            dont_slow_down_with_me: false,
            rewind_threshold: Seconds(defaults.rewind_threshold_seconds),
            fastforward_threshold: Seconds(defaults.fastforward_threshold_seconds),
            slowdown_threshold: Seconds(defaults.slowdown_threshold_seconds),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct PlaybackConfig {
    pub player_path: Option<PathBuf>,
    pub per_player_arguments: BTreeMap<PathBuf, Vec<String>>,
    pub streaming: StreamingPlaybackConfig,
    pub media_search_directories: Vec<PathBuf>,
    pub first_file_timeout: Seconds,
    pub folder_search_timeout: Seconds,
    pub folder_search_double_check_interval: Seconds,
    pub folder_search_warning_threshold: Seconds,
    pub default_rate: PlaybackRate,
    pub default_volume: Percent,
    pub shared_playlist_enabled: bool,
    pub pause_on_leave: bool,
    pub loop_at_end_of_playlist: bool,
    pub loop_single_files: bool,
    pub only_switch_to_trusted_domains: bool,
    pub trusted_domains: Vec<String>,
    pub filename_privacy_mode: PrivacyMode,
    pub filesize_privacy_mode: PrivacyMode,
}

impl fmt::Debug for PlaybackConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackConfig")
            .field("player_path", &self.player_path)
            .field(
                "per_player_argument_player_count",
                &self.per_player_arguments.len(),
            )
            .field(
                "per_player_arguments",
                &RedactedCommandArgs::from_args(
                    self.per_player_arguments
                        .values()
                        .flat_map(|args| args.iter()),
                ),
            )
            .field("streaming", &self.streaming)
            .field("media_search_directories", &self.media_search_directories)
            .field("first_file_timeout", &self.first_file_timeout)
            .field("folder_search_timeout", &self.folder_search_timeout)
            .field(
                "folder_search_double_check_interval",
                &self.folder_search_double_check_interval,
            )
            .field(
                "folder_search_warning_threshold",
                &self.folder_search_warning_threshold,
            )
            .field("default_rate", &self.default_rate)
            .field("default_volume", &self.default_volume)
            .field("shared_playlist_enabled", &self.shared_playlist_enabled)
            .field("pause_on_leave", &self.pause_on_leave)
            .field("loop_at_end_of_playlist", &self.loop_at_end_of_playlist)
            .field("loop_single_files", &self.loop_single_files)
            .field(
                "only_switch_to_trusted_domains",
                &self.only_switch_to_trusted_domains,
            )
            .field("trusted_domains", &self.trusted_domains)
            .field("filename_privacy_mode", &self.filename_privacy_mode)
            .field("filesize_privacy_mode", &self.filesize_privacy_mode)
            .finish()
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        let behavior = SessionBehaviorConfig::default();
        Self {
            player_path: None,
            per_player_arguments: BTreeMap::new(),
            streaming: StreamingPlaybackConfig::default(),
            media_search_directories: Vec::new(),
            first_file_timeout: Seconds(DEFAULT_FIRST_FILE_TIMEOUT_SECONDS),
            folder_search_timeout: Seconds(DEFAULT_FOLDER_SEARCH_TIMEOUT_SECONDS),
            folder_search_double_check_interval: Seconds(
                DEFAULT_FOLDER_SEARCH_DOUBLE_CHECK_SECONDS,
            ),
            folder_search_warning_threshold: Seconds(DEFAULT_FOLDER_SEARCH_WARNING_SECONDS),
            default_rate: PlaybackRate(1.0),
            default_volume: Percent(100.0),
            shared_playlist_enabled: true,
            pause_on_leave: behavior.pause_on_leave,
            loop_at_end_of_playlist: behavior.loop_at_end_of_playlist,
            loop_single_files: behavior.loop_single_files,
            only_switch_to_trusted_domains: behavior.only_switch_to_trusted_domains,
            trusted_domains: behavior.trusted_domains,
            filename_privacy_mode: PrivacyMode::SendRaw,
            filesize_privacy_mode: PrivacyMode::SendRaw,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadinessConfig {
    pub autoplay_initial_state: bool,
    pub autoplay_require_same_filenames: bool,
    pub ready_at_start: bool,
    pub unpause_action: UnpauseActionMode,
    pub autoplay_min_users: AutoplayThresholdOverride,
    pub show_duration_notification: bool,
}

impl Default for ReadinessConfig {
    fn default() -> Self {
        let defaults = ReadinessAutoplayConfig::default();
        Self {
            autoplay_initial_state: false,
            autoplay_require_same_filenames: defaults.autoplay_require_same_filenames,
            ready_at_start: false,
            unpause_action: defaults.unpause_action,
            autoplay_min_users: AutoplayThresholdOverride::Disable,
            show_duration_notification: defaults.show_duration_notification,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceConfig {
    pub language: String,
    pub check_for_updates_automatically: bool,
    pub update_channel: String,
    pub last_checked_for_updates: Option<String>,
    pub force_gui_prompt: bool,
    pub autosave_joins_to_list: bool,
    pub show_osd: bool,
    pub chat_input_enabled: bool,
    pub chat_input_font_underline: bool,
    pub chat_input_font_family: String,
    pub chat_input_relative_font_size: i64,
    pub chat_input_font_weight: i64,
    pub chat_input_font_color: String,
    pub chat_input_position: String,
    pub chat_direct_input: bool,
    pub chat_output_enabled: bool,
    pub chat_output_font_underline: bool,
    pub chat_output_font_family: String,
    pub chat_output_relative_font_size: i64,
    pub chat_output_font_weight: i64,
    pub chat_output_mode: String,
    pub chat_move_osd: bool,
    pub chat_max_lines: i64,
    pub chat_top_margin: i64,
    pub chat_left_margin: i64,
    pub chat_bottom_margin: i64,
    pub chat_osd_margin: i64,
    pub notification_timeout: Seconds,
    pub alert_timeout: Seconds,
    pub chat_timeout: Seconds,
    pub show_same_room_osd: bool,
    pub show_osd_warnings: bool,
    pub show_slowdown_osd: bool,
    pub show_noncontroller_osd: bool,
    pub show_different_room_osd: bool,
    pub show_contact_info: bool,
}

impl Default for InterfaceConfig {
    fn default() -> Self {
        let behavior = SessionBehaviorConfig::default();
        Self {
            language: "en".to_owned(),
            check_for_updates_automatically: false,
            update_channel: "stable".to_owned(),
            last_checked_for_updates: None,
            force_gui_prompt: false,
            autosave_joins_to_list: false,
            show_osd: true,
            chat_input_enabled: true,
            chat_input_font_underline: false,
            chat_input_font_family: "sans-serif".to_owned(),
            chat_input_relative_font_size: 24,
            chat_input_font_weight: 1,
            chat_input_font_color: "#FFFF00".to_owned(),
            chat_input_position: "Top".to_owned(),
            chat_direct_input: false,
            chat_output_enabled: true,
            chat_output_font_underline: false,
            chat_output_font_family: "sans-serif".to_owned(),
            chat_output_relative_font_size: 24,
            chat_output_font_weight: 1,
            chat_output_mode: "Chatroom".to_owned(),
            chat_move_osd: true,
            chat_max_lines: 7,
            chat_top_margin: 25,
            chat_left_margin: 20,
            chat_bottom_margin: 30,
            chat_osd_margin: 110,
            notification_timeout: Seconds(3.0),
            alert_timeout: Seconds(5.0),
            chat_timeout: Seconds(7.0),
            show_same_room_osd: behavior.show_same_room_osd,
            show_osd_warnings: behavior.show_osd_warnings,
            show_slowdown_osd: false,
            show_noncontroller_osd: behavior.show_noncontroller_osd,
            show_different_room_osd: behavior.show_different_room_osd,
            show_contact_info: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    pub stream_support_enabled: bool,
    pub media_matching_enabled: bool,
    pub plex_enabled: bool,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            stream_support_enabled: true,
            media_matching_enabled: true,
            plex_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlexConfig {
    pub sync_enabled: bool,
    pub streaming_enabled: bool,
    pub user_token: Option<SecretValue>,
    pub selected_server_id: Option<String>,
    pub selected_server_url: Option<String>,
    pub selected_server_token: Option<SecretValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatchConfig {
    pub fingerprinting_enabled: bool,
    pub background_warmup_enabled: bool,
    pub wire_sharing_enabled: bool,
    pub runtime_tolerance_enabled: bool,
    pub autoplay_policy: String,
}

impl Default for MediaMatchConfig {
    fn default() -> Self {
        Self {
            fingerprinting_enabled: false,
            background_warmup_enabled: true,
            wire_sharing_enabled: true,
            runtime_tolerance_enabled: true,
            autoplay_policy: "DiagnosticsOnly".to_owned(),
        }
    }
}

pub fn resolve_client_config(settings: &StoredClientSettingsV1) -> ClientConfigResolution {
    let mut config = ClientConfig::default();
    let mut issues = Vec::new();

    let resolved_public_servers = resolve_public_servers(settings, &mut issues);
    config.connection.public_servers = resolved_public_servers.configs;
    config.connection.room_history = resolve_room_history(settings, &mut issues);

    if let Some(raw_host) = resolve_optional_text("host", settings.host.as_deref(), &mut issues) {
        let endpoint = resolve_endpoint("host", &raw_host, &mut issues);
        config.connection.host = non_empty_trimmed(endpoint.host, "host").ok();
        if settings.port.is_none()
            && let Some(port) = endpoint.port
        {
            config.connection.port = port;
        }
    }
    if config.connection.host.is_none()
        && let Some(endpoint) = resolved_public_servers.first_endpoint
    {
        config.connection.host = non_empty_trimmed(endpoint.host, "host").ok();
        if settings.port.is_none()
            && let Some(port) = endpoint.port
        {
            config.connection.port = port;
        }
    }
    if let Some(port) = settings.port {
        match ServerPort::new(port) {
            Ok(port) => config.connection.port = port,
            Err(message) => issues.push(ClientConfigIssue::new("port", message)),
        }
    }
    config.connection.server_password = resolve_secret(settings.server_password.as_ref());
    config.connection.username = resolve_optional_newtype(
        "username",
        settings.username.as_deref(),
        Username::new,
        &mut issues,
    );

    let raw_room = settings
        .room
        .as_deref()
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            settings
                .room_list
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(String::as_str)
                .map(str::trim)
                .find(|room| !room.is_empty())
                .map(str::to_owned)
        });
    if settings
        .room
        .as_deref()
        .is_some_and(|room| room.trim().is_empty())
    {
        issues.push(ClientConfigIssue::new("room", "must not be empty"));
    }
    if let Some(raw_room) = raw_room {
        let (room, password) = normalize_controlled_room_input_legacy_compatible(raw_room);
        match RoomName::new(room) {
            Ok(room) => config.connection.room = Some(room),
            Err(message) => issues.push(ClientConfigIssue::new("room", message)),
        }
        config.connection.controlled_room_password = password.map(Into::into);
    }
    config.connection.tls_policy = TlsPolicy::default_for_credentials(
        config.connection.server_password.is_some()
            || config.connection.controlled_room_password.is_some(),
    );
    if let Some(raw_tls_policy) = settings.tls_policy.as_deref() {
        match TlsPolicy::parse(raw_tls_policy) {
            Some(policy) => config.connection.tls_policy = policy,
            None => issues.push(ClientConfigIssue::new(
                "tls_policy",
                "must be RequireTls, PreferTls, or Plaintext",
            )),
        }
    }

    config.synchronization.rewind_on_desync = settings
        .rewind_on_desync
        .unwrap_or(config.synchronization.rewind_on_desync);
    config.synchronization.fastforward_on_desync = settings
        .fastforward_on_desync
        .unwrap_or(config.synchronization.fastforward_on_desync);
    config.synchronization.slow_on_desync = settings
        .slow_on_desync
        .unwrap_or(config.synchronization.slow_on_desync);
    config.synchronization.dont_slow_down_with_me = settings
        .dont_slow_down_with_me
        .unwrap_or(config.synchronization.dont_slow_down_with_me);
    config.synchronization.rewind_threshold = resolve_seconds(
        "rewind_threshold_seconds",
        settings.rewind_threshold_seconds,
        config.synchronization.rewind_threshold,
        &mut issues,
    );
    config.synchronization.fastforward_threshold = resolve_seconds(
        "fastforward_threshold_seconds",
        settings.fastforward_threshold_seconds,
        config.synchronization.fastforward_threshold,
        &mut issues,
    );
    config.synchronization.slowdown_threshold = resolve_seconds(
        "slowdown_threshold_seconds",
        settings.slowdown_threshold_seconds,
        config.synchronization.slowdown_threshold,
        &mut issues,
    );

    config.playback.player_path =
        resolve_path("player_path", settings.player_path.as_deref(), &mut issues);
    config.playback.per_player_arguments = resolve_per_player_arguments(settings, &mut issues);
    config.playback.streaming = resolve_streaming_playback_config(settings, &mut issues);
    config.playback.media_search_directories =
        resolve_media_search_directories(settings, &mut issues);
    config.playback.first_file_timeout = resolve_seconds(
        "folder_search_first_file_timeout_seconds",
        settings.folder_search_first_file_timeout_seconds,
        config.playback.first_file_timeout,
        &mut issues,
    );
    config.playback.folder_search_timeout = resolve_seconds(
        "folder_search_timeout_seconds",
        settings.folder_search_timeout_seconds,
        config.playback.folder_search_timeout,
        &mut issues,
    );
    config.playback.folder_search_double_check_interval = resolve_seconds(
        "folder_search_double_check_interval_seconds",
        settings.folder_search_double_check_interval_seconds,
        config.playback.folder_search_double_check_interval,
        &mut issues,
    );
    config.playback.folder_search_warning_threshold = resolve_seconds(
        "folder_search_warning_threshold_seconds",
        settings.folder_search_warning_threshold_seconds,
        config.playback.folder_search_warning_threshold,
        &mut issues,
    );
    config.playback.shared_playlist_enabled = settings
        .shared_playlist_enabled
        .unwrap_or(config.playback.shared_playlist_enabled);
    config.playback.pause_on_leave = settings
        .pause_on_leave
        .unwrap_or(config.playback.pause_on_leave);
    config.playback.loop_at_end_of_playlist = settings
        .loop_at_end_of_playlist
        .unwrap_or(config.playback.loop_at_end_of_playlist);
    config.playback.loop_single_files = settings
        .loop_single_files
        .unwrap_or(config.playback.loop_single_files);
    config.playback.only_switch_to_trusted_domains = settings
        .only_switch_to_trusted_domains
        .unwrap_or(config.playback.only_switch_to_trusted_domains);
    if let Some(domains) = settings.trusted_domains.as_ref() {
        config.playback.trusted_domains = normalize_string_list(domains);
    }
    config.playback.filename_privacy_mode = settings
        .filename_privacy_mode
        .unwrap_or(config.playback.filename_privacy_mode);
    config.playback.filesize_privacy_mode = settings
        .filesize_privacy_mode
        .unwrap_or(config.playback.filesize_privacy_mode);

    config.readiness.autoplay_initial_state = settings
        .autoplay_initial_state
        .unwrap_or(config.readiness.autoplay_initial_state);
    config.readiness.autoplay_require_same_filenames = settings
        .autoplay_require_same_filenames
        .unwrap_or(config.readiness.autoplay_require_same_filenames);
    config.readiness.ready_at_start = settings
        .ready_at_start
        .unwrap_or(config.readiness.ready_at_start);
    config.readiness.unpause_action = settings
        .unpause_action
        .clone()
        .unwrap_or(config.readiness.unpause_action);
    if let Some(threshold) = settings.autoplay_min_users.as_ref() {
        match threshold {
            AutoplayThresholdOverride::Set(0) => issues.push(ClientConfigIssue::new(
                "autoplay_min_users",
                "minimum user count must be greater than zero",
            )),
            value => config.readiness.autoplay_min_users = value.clone(),
        }
    }
    config.readiness.show_duration_notification = settings
        .show_duration_notification
        .unwrap_or(config.readiness.show_duration_notification);

    apply_interface_settings(settings, &mut config.interface, &mut issues);

    config.plugins.stream_support_enabled = settings
        .stream_support_plugin_enabled
        .unwrap_or(config.plugins.stream_support_enabled);
    config.plugins.media_matching_enabled = settings
        .media_matching_plugin_enabled
        .unwrap_or(config.plugins.media_matching_enabled);
    config.plugins.plex_enabled = settings
        .plex_plugin_enabled
        .unwrap_or(config.plugins.plex_enabled);

    config.plex.sync_enabled = settings.plex_sync_enabled.unwrap_or(false);
    config.plex.streaming_enabled = settings.plex_streaming_enabled.unwrap_or(false);
    config.plex.user_token = resolve_secret(settings.plex_user_token.as_ref());
    config.plex.selected_server_id =
        normalized_optional_text(settings.plex_selected_server_id.as_deref());
    config.plex.selected_server_url =
        normalized_optional_text(settings.plex_selected_server_url.as_deref());
    config.plex.selected_server_token =
        resolve_secret(settings.plex_selected_server_token.as_ref());

    config.media_match.fingerprinting_enabled = settings
        .media_match_fingerprinting_enabled
        .unwrap_or(config.media_match.fingerprinting_enabled);
    config.media_match.background_warmup_enabled = settings
        .media_match_background_warmup_enabled
        .unwrap_or(config.media_match.background_warmup_enabled);
    config.media_match.wire_sharing_enabled = settings
        .media_match_wire_sharing_enabled
        .unwrap_or(config.media_match.wire_sharing_enabled);
    config.media_match.runtime_tolerance_enabled = settings
        .media_match_runtime_tolerance_enabled
        .unwrap_or(config.media_match.runtime_tolerance_enabled);
    if let Some(policy) = settings.media_match_autoplay_policy.as_deref() {
        match non_empty_trimmed(policy.to_owned(), "media-match autoplay policy") {
            Ok(policy) => config.media_match.autoplay_policy = policy,
            Err(message) => issues.push(ClientConfigIssue::new(
                "media_match_autoplay_policy",
                message,
            )),
        }
    }

    ClientConfigResolution { config, issues }
}

fn apply_interface_settings(
    settings: &StoredClientSettingsV1,
    interface: &mut InterfaceConfig,
    issues: &mut Vec<ClientConfigIssue>,
) {
    if let Some(language) = settings.language.as_deref() {
        if let Some(language) = normalized_legacy_runtime_language_tag_legacy_compatible(language) {
            interface.language = language.to_owned();
        } else {
            issues.push(ClientConfigIssue::new(
                "language",
                "must be a supported language tag",
            ));
        }
    }
    interface.check_for_updates_automatically = settings
        .check_for_updates_automatically
        .unwrap_or(interface.check_for_updates_automatically);
    apply_optional_text(
        "update_channel",
        settings.update_channel.as_deref(),
        &mut interface.update_channel,
        issues,
    );
    interface.last_checked_for_updates =
        normalized_optional_text(settings.last_checked_for_updates.as_deref());
    interface.force_gui_prompt = settings
        .force_gui_prompt
        .unwrap_or(interface.force_gui_prompt);
    interface.autosave_joins_to_list = settings
        .autosave_joins_to_list
        .unwrap_or(interface.autosave_joins_to_list);
    interface.show_osd = settings.show_osd.unwrap_or(interface.show_osd);
    interface.chat_input_enabled = settings
        .chat_input_enabled
        .unwrap_or(interface.chat_input_enabled);
    interface.chat_input_font_underline = settings
        .chat_input_font_underline
        .unwrap_or(interface.chat_input_font_underline);
    apply_optional_text(
        "chat_input_font_family",
        settings.chat_input_font_family.as_deref(),
        &mut interface.chat_input_font_family,
        issues,
    );
    apply_positive_i64(
        "chat_input_relative_font_size",
        settings.chat_input_relative_font_size,
        &mut interface.chat_input_relative_font_size,
        issues,
    );
    if let Some(value) = settings.chat_input_font_weight {
        interface.chat_input_font_weight = value;
    }
    apply_optional_text(
        "chat_input_font_color",
        settings.chat_input_font_color.as_deref(),
        &mut interface.chat_input_font_color,
        issues,
    );
    apply_optional_text(
        "chat_input_position",
        settings.chat_input_position.as_deref(),
        &mut interface.chat_input_position,
        issues,
    );
    interface.chat_direct_input = settings
        .chat_direct_input
        .unwrap_or(interface.chat_direct_input);
    interface.chat_output_enabled = settings
        .chat_output_enabled
        .unwrap_or(interface.chat_output_enabled);
    interface.chat_output_font_underline = settings
        .chat_output_font_underline
        .unwrap_or(interface.chat_output_font_underline);
    apply_optional_text(
        "chat_output_font_family",
        settings.chat_output_font_family.as_deref(),
        &mut interface.chat_output_font_family,
        issues,
    );
    apply_positive_i64(
        "chat_output_relative_font_size",
        settings.chat_output_relative_font_size,
        &mut interface.chat_output_relative_font_size,
        issues,
    );
    if let Some(value) = settings.chat_output_font_weight {
        interface.chat_output_font_weight = value;
    }
    apply_optional_text(
        "chat_output_mode",
        settings.chat_output_mode.as_deref(),
        &mut interface.chat_output_mode,
        issues,
    );
    interface.chat_move_osd = settings.chat_move_osd.unwrap_or(interface.chat_move_osd);
    apply_positive_i64(
        "chat_max_lines",
        settings.chat_max_lines,
        &mut interface.chat_max_lines,
        issues,
    );
    apply_nonnegative_i64(
        "chat_top_margin",
        settings.chat_top_margin,
        &mut interface.chat_top_margin,
        issues,
    );
    apply_nonnegative_i64(
        "chat_left_margin",
        settings.chat_left_margin,
        &mut interface.chat_left_margin,
        issues,
    );
    apply_nonnegative_i64(
        "chat_bottom_margin",
        settings.chat_bottom_margin,
        &mut interface.chat_bottom_margin,
        issues,
    );
    apply_nonnegative_i64(
        "chat_osd_margin",
        settings.chat_osd_margin,
        &mut interface.chat_osd_margin,
        issues,
    );
    interface.notification_timeout = resolve_integer_seconds(
        "notification_timeout_seconds",
        settings.notification_timeout_seconds,
        interface.notification_timeout,
        issues,
    );
    interface.alert_timeout = resolve_integer_seconds(
        "alert_timeout_seconds",
        settings.alert_timeout_seconds,
        interface.alert_timeout,
        issues,
    );
    interface.chat_timeout = resolve_integer_seconds(
        "chat_timeout_seconds",
        settings.chat_timeout_seconds,
        interface.chat_timeout,
        issues,
    );
    interface.show_same_room_osd = settings
        .show_same_room_osd
        .unwrap_or(interface.show_same_room_osd);
    interface.show_osd_warnings = settings
        .show_osd_warnings
        .unwrap_or(interface.show_osd_warnings);
    interface.show_slowdown_osd = settings
        .show_slowdown_osd
        .unwrap_or(interface.show_slowdown_osd);
    interface.show_noncontroller_osd = settings
        .show_noncontroller_osd
        .unwrap_or(interface.show_noncontroller_osd);
    interface.show_different_room_osd = settings
        .show_different_room_osd
        .unwrap_or(interface.show_different_room_osd);
    interface.show_contact_info = settings
        .show_contact_info
        .unwrap_or(interface.show_contact_info);
}

struct ResolvedEndpoint {
    host: String,
    port: Option<ServerPort>,
    is_valid: bool,
}

struct ResolvedPublicServers {
    configs: Vec<PublicServerConfig>,
    first_endpoint: Option<ResolvedEndpoint>,
}

fn resolve_endpoint(
    field: impl Into<String>,
    address: &str,
    issues: &mut Vec<ClientConfigIssue>,
) -> ResolvedEndpoint {
    let field = field.into();
    let (host, embedded_port) =
        parse_host_and_optional_port_from_host_arg_legacy_compatible(address);
    let (port, is_valid) = match embedded_port.map(ServerPort::new) {
        Some(Ok(port)) => (Some(port), true),
        Some(Err(message)) => {
            issues.push(ClientConfigIssue::new(
                field,
                format!("embedded port {message}"),
            ));
            (None, false)
        }
        None => (None, true),
    };
    ResolvedEndpoint {
        host,
        port,
        is_valid,
    }
}

fn resolve_public_servers(
    settings: &StoredClientSettingsV1,
    issues: &mut Vec<ClientConfigIssue>,
) -> ResolvedPublicServers {
    let mut configs = Vec::new();
    let mut first_endpoint = None;
    for (index, (label, address)) in settings
        .public_servers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let label = label.trim();
        let address = address.trim();
        let field = format!("public_servers[{index}].address");
        if address.is_empty() {
            issues.push(ClientConfigIssue::new(field, "must not be empty"));
            continue;
        }

        let endpoint = resolve_endpoint(field, address, issues);
        if !endpoint.is_valid {
            continue;
        }
        if first_endpoint.is_none() {
            first_endpoint = Some(endpoint);
        }
        configs.push(PublicServerConfig {
            label: label.to_owned(),
            address: address.to_owned(),
        });
    }

    ResolvedPublicServers {
        configs,
        first_endpoint,
    }
}

fn resolve_room_history(
    settings: &StoredClientSettingsV1,
    issues: &mut Vec<ClientConfigIssue>,
) -> Vec<RoomName> {
    let mut seen = BTreeSet::new();
    settings
        .room_list
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter_map(|(index, room)| {
            let (room, _) = normalize_controlled_room_input_legacy_compatible(room.clone());
            match RoomName::new(room) {
                Ok(room) if seen.insert(room.clone()) => Some(room),
                Ok(_) => None,
                Err(message) => {
                    issues.push(ClientConfigIssue::new(
                        format!("room_list[{index}]"),
                        message,
                    ));
                    None
                }
            }
        })
        .collect()
}

fn resolve_per_player_arguments(
    settings: &StoredClientSettingsV1,
    issues: &mut Vec<ClientConfigIssue>,
) -> BTreeMap<PathBuf, Vec<String>> {
    settings
        .per_player_arguments
        .as_ref()
        .into_iter()
        .flat_map(|values| values.iter())
        .filter_map(|(path, arguments)| {
            resolve_path("per_player_arguments path", Some(path), issues)
                .map(|path| (path, arguments.clone()))
        })
        .collect()
}

fn resolve_streaming_playback_config(
    settings: &StoredClientSettingsV1,
    issues: &mut Vec<ClientConfigIssue>,
) -> StreamingPlaybackConfig {
    let mut config = StreamingPlaybackConfig::default();

    if let Some(value) = settings.streaming_quality_preset.as_deref() {
        match StreamingQualityPreset::parse(value) {
            Some(value) => config.quality = value,
            None => issues.push(ClientConfigIssue::new(
                "streaming_quality_preset",
                "must be auto, best, balanced, 1080p, 720p, 480p, compatibility, or custom",
            )),
        }
    }
    config.custom_format = normalized_optional_text(settings.streaming_custom_format.as_deref());
    if config.quality == StreamingQualityPreset::Custom && config.custom_format.is_none() {
        issues.push(ClientConfigIssue::new(
            "streaming_custom_format",
            "must be set when streaming quality is custom",
        ));
        config.quality = StreamingQualityPreset::Auto;
    }

    config.buffering.target = resolve_positive_seconds(
        "streaming_buffer_target_seconds",
        settings.streaming_buffer_target_seconds,
        config.buffering.target,
        issues,
    );
    config.buffering.read_ahead = resolve_positive_seconds(
        "streaming_read_ahead_seconds",
        settings.streaming_read_ahead_seconds,
        config.buffering.read_ahead,
        issues,
    );
    if config.buffering.read_ahead.get() < config.buffering.target.get() {
        issues.push(ClientConfigIssue::new(
            "streaming_read_ahead_seconds",
            "must be at least the buffering target",
        ));
        config.buffering.read_ahead = config.buffering.target;
    }
    if let Some(value) = settings.streaming_memory_cache_mebibytes {
        if value == 0 {
            issues.push(ClientConfigIssue::new(
                "streaming_memory_cache_mebibytes",
                "must be greater than zero",
            ));
        } else {
            config.buffering.memory_cache_mebibytes = value;
        }
    }
    config.buffering.disk_cache_enabled = settings
        .streaming_disk_cache_enabled
        .unwrap_or(config.buffering.disk_cache_enabled);

    if let Some(value) = settings.streaming_recovery_policy.as_deref() {
        match StreamingRecoveryPolicy::parse(value) {
            Some(value) => config.recovery.policy = value,
            None => issues.push(ClientConfigIssue::new(
                "streaming_recovery_policy",
                "must be preserve-content, balanced, stay-closest, or pause-room",
            )),
        }
    }
    if let Some(value) = settings.streaming_max_catchup_rate {
        if value.is_finite() && (1.0..=1.25).contains(&value) {
            config.recovery.max_catchup_rate = PlaybackRate(value);
        } else {
            issues.push(ClientConfigIssue::new(
                "streaming_max_catchup_rate",
                "must be between 1.0 and 1.25",
            ));
        }
    }
    config.recovery.hard_seek_threshold = resolve_positive_seconds(
        "streaming_hard_seek_threshold_seconds",
        settings.streaming_hard_seek_threshold_seconds,
        config.recovery.hard_seek_threshold,
        issues,
    );
    config.recovery.max_hard_seeks_per_episode = settings
        .streaming_max_hard_seeks_per_episode
        .unwrap_or(config.recovery.max_hard_seeks_per_episode);
    config.recovery.stability_interval = resolve_positive_seconds(
        "streaming_stability_interval_seconds",
        settings.streaming_stability_interval_seconds,
        config.recovery.stability_interval,
        issues,
    );
    config.recovery.retry_budget = settings
        .streaming_recovery_retry_budget
        .unwrap_or(config.recovery.retry_budget);
    config.recovery.cooldown = resolve_seconds(
        "streaming_recovery_cooldown_seconds",
        settings.streaming_recovery_cooldown_seconds,
        config.recovery.cooldown,
        issues,
    );

    if let Some(value) = settings.streaming_room_buffering_policy.as_deref() {
        match RoomBufferingPolicy::parse(value) {
            Some(value) => config.room_buffering.policy = value,
            None => issues.push(ClientConfigIssue::new(
                "streaming_room_buffering_policy",
                "must be independent, pause-controller, pause-eligible, or quorum",
            )),
        }
    }
    config.room_buffering.quorum = resolve_positive_percent(
        "streaming_room_quorum_percent",
        settings.streaming_room_quorum_percent,
        config.room_buffering.quorum,
        issues,
    );
    config.room_buffering.maximum_pause = resolve_positive_seconds(
        "streaming_room_max_pause_seconds",
        settings.streaming_room_max_pause_seconds,
        config.room_buffering.maximum_pause,
        issues,
    );

    if let Some(value) = settings.streaming_start_policy.as_deref() {
        match StartSynchronizationPolicy::parse(value) {
            Some(value) => config.start_synchronization.policy = value,
            None => issues.push(ClientConfigIssue::new(
                "streaming_start_policy",
                "must be immediate, wait-controller, wait-all, or quorum",
            )),
        }
    }
    config.start_synchronization.quorum = resolve_positive_percent(
        "streaming_start_quorum_percent",
        settings.streaming_start_quorum_percent,
        config.start_synchronization.quorum,
        issues,
    );
    config.start_synchronization.timeout = resolve_positive_seconds(
        "streaming_start_timeout_seconds",
        settings.streaming_start_timeout_seconds,
        config.start_synchronization.timeout,
        issues,
    );
    if let Some(value) = settings.streaming_start_timeout_action.as_deref() {
        match StartTimeoutAction::parse(value) {
            Some(value) => config.start_synchronization.timeout_action = value,
            None => issues.push(ClientConfigIssue::new(
                "streaming_start_timeout_action",
                "must be continue, remain-paused, or ask-controller",
            )),
        }
    }
    config.quality_downgrade_suggestions = settings
        .streaming_quality_downgrade_suggestions
        .unwrap_or(config.quality_downgrade_suggestions);

    config
}

fn resolve_media_search_directories(
    settings: &StoredClientSettingsV1,
    issues: &mut Vec<ClientConfigIssue>,
) -> Vec<PathBuf> {
    settings
        .media_search_directories
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            resolve_path(
                &format!("media_search_directories[{index}]"),
                Some(path),
                issues,
            )
        })
        .collect()
}

fn resolve_secret(value: Option<&SecretValue>) -> Option<SecretValue> {
    value
        .map(|value| value.expose_secret().trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_owned().into())
}

fn resolve_optional_text(
    field: &str,
    value: Option<&str>,
    issues: &mut Vec<ClientConfigIssue>,
) -> Option<String> {
    let value = value?;
    match non_empty_trimmed(value.to_owned(), field) {
        Ok(value) => Some(value),
        Err(message) => {
            issues.push(ClientConfigIssue::new(field, message));
            None
        }
    }
}

fn resolve_optional_newtype<T>(
    field: &str,
    value: Option<&str>,
    constructor: impl FnOnce(String) -> Result<T, String>,
    issues: &mut Vec<ClientConfigIssue>,
) -> Option<T> {
    let value = value?;
    match constructor(value.to_owned()) {
        Ok(value) => Some(value),
        Err(message) => {
            issues.push(ClientConfigIssue::new(field, message));
            None
        }
    }
}

fn resolve_path(
    field: &str,
    value: Option<&str>,
    issues: &mut Vec<ClientConfigIssue>,
) -> Option<PathBuf> {
    resolve_optional_text(field, value, issues).map(PathBuf::from)
}

fn resolve_seconds(
    field: &str,
    value: Option<f64>,
    default: Seconds,
    issues: &mut Vec<ClientConfigIssue>,
) -> Seconds {
    let Some(value) = value else {
        return default;
    };
    match Seconds::new(value) {
        Ok(value) => value,
        Err(message) => {
            issues.push(ClientConfigIssue::new(field, message));
            default
        }
    }
}

fn resolve_positive_seconds(
    field: &str,
    value: Option<f64>,
    default: Seconds,
    issues: &mut Vec<ClientConfigIssue>,
) -> Seconds {
    let resolved = resolve_seconds(field, value, default, issues);
    if value.is_some() && resolved.get() == 0.0 {
        issues.push(ClientConfigIssue::new(field, "must be greater than zero"));
        default
    } else {
        resolved
    }
}

fn resolve_positive_percent(
    field: &str,
    value: Option<f64>,
    default: Percent,
    issues: &mut Vec<ClientConfigIssue>,
) -> Percent {
    let Some(value) = value else {
        return default;
    };
    match Percent::new(value) {
        Ok(value) if value.get() > 0.0 => value,
        Ok(_) => {
            issues.push(ClientConfigIssue::new(field, "must be greater than zero"));
            default
        }
        Err(message) => {
            issues.push(ClientConfigIssue::new(field, message));
            default
        }
    }
}

fn resolve_integer_seconds(
    field: &str,
    value: Option<i64>,
    default: Seconds,
    issues: &mut Vec<ClientConfigIssue>,
) -> Seconds {
    resolve_seconds(field, value.map(|value| value as f64), default, issues)
}

fn apply_optional_text(
    field: &str,
    value: Option<&str>,
    target: &mut String,
    issues: &mut Vec<ClientConfigIssue>,
) {
    if let Some(value) = resolve_optional_text(field, value, issues) {
        *target = value;
    }
}

fn apply_positive_i64(
    field: &str,
    value: Option<i64>,
    target: &mut i64,
    issues: &mut Vec<ClientConfigIssue>,
) {
    let Some(value) = value else {
        return;
    };
    if value > 0 {
        *target = value;
    } else {
        issues.push(ClientConfigIssue::new(field, "must be greater than zero"));
    }
}

fn apply_nonnegative_i64(
    field: &str,
    value: Option<i64>,
    target: &mut i64,
    issues: &mut Vec<ClientConfigIssue>,
) {
    let Some(value) = value else {
        return;
    };
    if value >= 0 {
        *target = value;
    } else {
        issues.push(ClientConfigIssue::new(field, "must be zero or greater"));
    }
}

fn normalized_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert((*value).to_owned()))
        .map(str::to_owned)
        .collect()
}

fn non_empty_trimmed(value: String, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_policy_defaults_to_required_for_remote_saved_credentials() {
        let remote = ClientConfig::try_from_stored(&StoredClientSettingsV1 {
            host: Some("sync.example".to_owned()),
            server_password: Some("saved-secret".into()),
            ..StoredClientSettingsV1::default()
        })
        .expect("remote credential settings should resolve");
        assert_eq!(remote.connection.tls_policy, TlsPolicy::RequireTls);

        let loopback = ClientConfig::try_from_stored(&StoredClientSettingsV1 {
            host: Some("127.0.0.1".to_owned()),
            server_password: Some("local-secret".into()),
            ..StoredClientSettingsV1::default()
        })
        .expect("loopback credential settings should resolve");
        assert_eq!(loopback.connection.tls_policy, TlsPolicy::RequireTls);
        assert_eq!(TlsPolicy::parse("plaintext"), Some(TlsPolicy::Plaintext));
        assert_eq!(TlsPolicy::parse("require-tls"), Some(TlsPolicy::RequireTls));

        let explicit = ClientConfig::try_from_stored(&StoredClientSettingsV1 {
            host: Some("sync.example".to_owned()),
            server_password: Some("saved-secret".into()),
            tls_policy: Some("Plaintext".to_owned()),
            ..StoredClientSettingsV1::default()
        })
        .expect("explicit TLS policy should resolve");
        assert_eq!(explicit.connection.tls_policy, TlsPolicy::Plaintext);
    }

    #[test]
    fn absent_streaming_start_policy_keeps_immediate_legacy_behavior() {
        assert_eq!(
            StartSynchronizationPolicy::default(),
            StartSynchronizationPolicy::Immediate
        );
        assert_eq!(
            StartSynchronizationConfig::default().policy,
            StartSynchronizationPolicy::Immediate
        );

        let config = ClientConfig::try_from_stored(&StoredClientSettingsV1::default())
            .expect("empty settings should resolve")
            .playback
            .streaming;

        assert_eq!(config.quality, StreamingQualityPreset::Auto);
        assert_eq!(config.recovery.max_hard_seeks_per_episode, 1);
        assert_eq!(config.recovery.max_catchup_rate.get(), 1.05);
        assert_eq!(
            config.start_synchronization.policy,
            StartSynchronizationPolicy::Immediate
        );
        assert_eq!(
            config.room_buffering.policy,
            RoomBufferingPolicy::Independent
        );
        assert!(
            config
                .network_media_mpv_arguments()
                .contains(&"--cache-pause-wait=5".to_owned())
        );
    }

    #[test]
    fn legacy_ini_without_streaming_start_policy_migrates_to_immediate() {
        let settings = crate::sorotte_ini::parse_sorotte_ini_stored_client_settings_mvp(
            "[client_settings]\nname = legacy-user\n",
        );
        assert_eq!(settings.streaming_start_policy, None);

        let config = ClientConfig::try_from_stored(&settings)
            .expect("legacy settings without a start policy should resolve")
            .playback
            .streaming;
        assert_eq!(
            config.start_synchronization.policy,
            StartSynchronizationPolicy::Immediate
        );
    }

    #[test]
    fn explicit_wait_all_start_policy_remains_opt_in() {
        let settings = StoredClientSettingsV1 {
            streaming_start_policy: Some("wait-all".to_owned()),
            ..StoredClientSettingsV1::default()
        };

        let config = ClientConfig::try_from_stored(&settings)
            .expect("explicit wait-all start policy should resolve")
            .playback
            .streaming;

        assert_eq!(
            config.start_synchronization.policy,
            StartSynchronizationPolicy::WaitForAllEligible
        );
    }

    #[test]
    fn streaming_settings_resolve_to_typed_policy_and_network_media_mpv_arguments() {
        let settings = StoredClientSettingsV1 {
            streaming_quality_preset: Some("720p".to_owned()),
            streaming_buffer_target_seconds: Some(8.0),
            streaming_read_ahead_seconds: Some(45.0),
            streaming_memory_cache_mebibytes: Some(256),
            streaming_disk_cache_enabled: Some(true),
            streaming_recovery_policy: Some("stay-closest".to_owned()),
            streaming_max_catchup_rate: Some(1.08),
            streaming_max_hard_seeks_per_episode: Some(1),
            streaming_room_buffering_policy: Some("quorum".to_owned()),
            streaming_start_policy: Some("wait-all".to_owned()),
            streaming_start_timeout_action: Some("remain-paused".to_owned()),
            ..StoredClientSettingsV1::default()
        };

        let config = ClientConfig::try_from_stored(&settings)
            .expect("valid streaming settings should resolve")
            .playback
            .streaming;
        assert_eq!(config.quality, StreamingQualityPreset::Max720p);
        assert_eq!(config.recovery.policy, StreamingRecoveryPolicy::StayClosest);
        assert_eq!(config.room_buffering.policy, RoomBufferingPolicy::Quorum);
        assert_eq!(
            config.start_synchronization.policy,
            StartSynchronizationPolicy::WaitForAllEligible
        );
        assert!(config.network_media_mpv_arguments().iter().any(|argument| {
            argument == "--ytdl-format=bestvideo*[height<=720]+bestaudio/best[height<=720]"
        }));
        assert!(
            config
                .network_media_mpv_arguments()
                .contains(&"--demuxer-max-bytes=256MiB".to_owned())
        );
        assert!(
            config
                .network_media_mpv_arguments()
                .contains(&"--cache-on-disk=yes".to_owned())
        );
        let coordinator = config.playback_coordinator_config();
        assert_eq!(
            coordinator.recovery_policy,
            sorotte_client_core::RecoveryPolicy::StayClosest
        );
        assert_eq!(coordinator.maximum_hard_seeks_per_episode, 1);
        assert_eq!(coordinator.maximum_catchup_rate, 1.08);
        assert_eq!(coordinator.seek_preparation_minimum_headroom_seconds, 8.0);
    }

    #[test]
    fn advanced_player_arguments_are_reported_as_effective_overrides() {
        let config = StreamingPlaybackConfig::default();
        let effective = config.effective_mpv_options(&[
            "--cache-pause-wait=12".to_owned(),
            "--no-cache-on-disk".to_owned(),
        ]);

        let wait = effective
            .iter()
            .find(|option| option.name == "cache-pause-wait")
            .expect("wait option should be present");
        assert_eq!(wait.configured_value, "5");
        assert_eq!(wait.effective_value, "12");
        assert!(wait.overridden_by_advanced_arguments);

        let disk = effective
            .iter()
            .find(|option| option.name == "cache-on-disk")
            .expect("disk option should be present");
        assert_eq!(disk.effective_value, "no");
        assert!(disk.overridden_by_advanced_arguments);
    }

    #[test]
    fn adaptive_quality_advisor_suggests_bounded_step_down_without_auto_switching() {
        let config = StreamingPlaybackConfig {
            quality: StreamingQualityPreset::Max1080p,
            ..StreamingPlaybackConfig::default()
        };
        let metrics = sorotte_client_core::PlaybackCoordinatorMetrics {
            buffer_episode_count: 3,
            ..sorotte_client_core::PlaybackCoordinatorMetrics::default()
        };

        assert_eq!(
            config.quality_downgrade_suggestion(&metrics, None),
            Some(StreamingQualityDowngradeSuggestion {
                current: StreamingQualityPreset::Max1080p,
                recommended: StreamingQualityPreset::Max720p,
                reason: StreamingQualitySuggestionReason::RepeatedRebuffering,
            })
        );
        assert_eq!(config.quality, StreamingQualityPreset::Max1080p);
    }

    #[test]
    fn adaptive_quality_advisor_uses_input_rate_headroom_and_honors_opt_out() {
        let metrics = sorotte_client_core::PlaybackCoordinatorMetrics {
            last_input_rate_bytes_per_second: Some(900_000),
            ..sorotte_client_core::PlaybackCoordinatorMetrics::default()
        };
        let mut config = StreamingPlaybackConfig {
            quality: StreamingQualityPreset::Max720p,
            ..StreamingPlaybackConfig::default()
        };
        assert_eq!(
            config
                .quality_downgrade_suggestion(&metrics, Some(1_000_000))
                .map(|suggestion| suggestion.reason),
            Some(StreamingQualitySuggestionReason::InsufficientObservedInputRate)
        );
        config.quality_downgrade_suggestions = false;
        assert_eq!(
            config.quality_downgrade_suggestion(&metrics, Some(1_000_000)),
            None
        );
    }

    #[test]
    fn invalid_streaming_configuration_reports_actionable_fields() {
        let resolution = ClientConfig::resolve(&StoredClientSettingsV1 {
            streaming_quality_preset: Some("custom".to_owned()),
            streaming_buffer_target_seconds: Some(8.0),
            streaming_read_ahead_seconds: Some(4.0),
            streaming_max_catchup_rate: Some(1.5),
            streaming_room_quorum_percent: Some(0.0),
            ..StoredClientSettingsV1::default()
        });

        let fields = resolution
            .issues
            .iter()
            .map(|issue| issue.field.as_str())
            .collect::<BTreeSet<_>>();
        assert!(fields.contains("streaming_custom_format"));
        assert!(fields.contains("streaming_read_ahead_seconds"));
        assert!(fields.contains("streaming_max_catchup_rate"));
        assert!(fields.contains("streaming_room_quorum_percent"));
    }

    #[test]
    fn empty_stored_settings_keep_shared_playlists_enabled_by_default() {
        assert!(PlaybackConfig::default().shared_playlist_enabled);

        let config = ClientConfig::try_from_stored(&StoredClientSettingsV1::default())
            .expect("empty stored settings should resolve");
        assert!(config.playback.shared_playlist_enabled);
    }

    #[test]
    fn legacy_ini_without_shared_playlist_field_keeps_compatibility_default() {
        let settings = crate::sorotte_ini::parse_sorotte_ini_stored_client_settings_mvp(
            "[client_settings]\nname = legacy-user\n",
        );
        assert_eq!(settings.shared_playlist_enabled, None);

        let config = ClientConfig::try_from_stored(&settings)
            .expect("legacy settings without the field should resolve");
        assert!(config.playback.shared_playlist_enabled);
    }

    #[test]
    fn explicit_shared_playlist_settings_override_compatibility_default() {
        for enabled in [true, false] {
            let settings = StoredClientSettingsV1 {
                shared_playlist_enabled: Some(enabled),
                ..StoredClientSettingsV1::default()
            };
            let config = ClientConfig::try_from_stored(&settings)
                .expect("explicit shared-playlist setting should resolve");
            assert_eq!(config.playback.shared_playlist_enabled, enabled);
        }
    }

    #[test]
    fn resolves_storage_dto_into_sliced_runtime_config() {
        let settings = StoredClientSettingsV1 {
            host: Some("  example.org  ".to_owned()),
            port: Some(8998),
            username: Some(" alice ".to_owned()),
            room: Some(" room-a ".to_owned()),
            rewind_threshold_seconds: Some(6.5),
            player_path: Some(" C:/mpv/mpv.exe ".to_owned()),
            stream_support_plugin_enabled: Some(false),
            plex_sync_enabled: Some(true),
            ..StoredClientSettingsV1::default()
        };

        let config = ClientConfig::try_from_stored(&settings).expect("settings should resolve");
        assert_eq!(config.connection.host.as_deref(), Some("example.org"));
        assert_eq!(config.connection.port.get(), 8998);
        assert_eq!(
            config.connection.username.as_ref().map(Username::as_str),
            Some("alice")
        );
        assert_eq!(
            config.connection.room.as_ref().map(RoomName::as_str),
            Some("room-a")
        );
        assert_eq!(config.synchronization.rewind_threshold.get(), 6.5);
        assert_eq!(
            config.playback.player_path.as_deref(),
            Some(std::path::Path::new("C:/mpv/mpv.exe"))
        );
        assert!(!config.plugins.stream_support_enabled);
        assert!(config.plex.sync_enabled);
    }

    #[test]
    fn embedded_zero_port_in_host_is_reported_instead_of_silently_using_the_default() {
        let resolution = ClientConfig::resolve(&StoredClientSettingsV1 {
            host: Some("example.org:0".to_owned()),
            ..StoredClientSettingsV1::default()
        });

        assert_eq!(
            resolution.config.connection.host.as_deref(),
            Some("example.org")
        );
        assert_eq!(resolution.config.connection.port.get(), DEFAULT_SERVER_PORT);
        assert_eq!(
            resolution.issues,
            vec![ClientConfigIssue::new(
                "host",
                "embedded port must be between 1 and 65535",
            )]
        );
        assert!(resolution.into_result().is_err());
    }

    #[test]
    fn embedded_zero_port_in_public_server_is_reported_and_filtered_before_fallback() {
        let resolution = ClientConfig::resolve(&StoredClientSettingsV1 {
            public_servers: Some(vec![
                ("Invalid".to_owned(), "public.example:0".to_owned()),
                ("Primary".to_owned(), "fallback.example:8123".to_owned()),
            ]),
            ..StoredClientSettingsV1::default()
        });

        assert_eq!(
            resolution.config.connection.host.as_deref(),
            Some("fallback.example")
        );
        assert_eq!(resolution.config.connection.port.get(), 8123);
        assert_eq!(
            resolution.config.connection.public_servers,
            vec![PublicServerConfig {
                label: "Primary".to_owned(),
                address: "fallback.example:8123".to_owned(),
            }]
        );
        assert_eq!(
            resolution.issues,
            vec![ClientConfigIssue::new(
                "public_servers[0].address",
                "embedded port must be between 1 and 65535",
            )]
        );
        assert!(resolution.into_result().is_err());
    }

    #[test]
    fn embedded_port_issues_aggregate_and_explicit_port_validation_remains_independent() {
        let resolution = ClientConfig::resolve(&StoredClientSettingsV1 {
            host: Some("example.org:0".to_owned()),
            port: Some(0),
            public_servers: Some(vec![("Secondary".to_owned(), "[2001:db8::1]:0".to_owned())]),
            ..StoredClientSettingsV1::default()
        });

        assert_eq!(resolution.config.connection.port.get(), DEFAULT_SERVER_PORT);
        assert_eq!(
            resolution
                .issues
                .iter()
                .map(|issue| issue.field.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["host", "port", "public_servers[0].address"])
        );
        assert_eq!(resolution.issues.len(), 3);
    }

    #[test]
    fn valid_embedded_ports_keep_host_ipv4_ipv6_parsing_and_explicit_port_precedence() {
        for (raw_host, expected_host) in [
            ("example.org:7001", "example.org"),
            ("127.0.0.1:7001", "127.0.0.1"),
            ("[2001:db8::1]:7001", "[2001:db8::1]"),
        ] {
            let config = ClientConfig::try_from_stored(&StoredClientSettingsV1 {
                host: Some(raw_host.to_owned()),
                ..StoredClientSettingsV1::default()
            })
            .expect("valid embedded endpoint should resolve");
            assert_eq!(config.connection.host.as_deref(), Some(expected_host));
            assert_eq!(config.connection.port.get(), 7001);
        }

        let config = ClientConfig::try_from_stored(&StoredClientSettingsV1 {
            host: Some("[2001:db8::1]:7001".to_owned()),
            port: Some(7002),
            ..StoredClientSettingsV1::default()
        })
        .expect("valid explicit port should override an embedded port");
        assert_eq!(config.connection.host.as_deref(), Some("[2001:db8::1]"));
        assert_eq!(config.connection.port.get(), 7002);
    }

    #[test]
    fn reports_all_invalid_values_and_keeps_safe_fallbacks() {
        let settings = StoredClientSettingsV1 {
            host: Some("   ".to_owned()),
            port: Some(0),
            username: Some(" ".to_owned()),
            rewind_threshold_seconds: Some(-1.0),
            folder_search_timeout_seconds: Some(f64::NAN),
            chat_input_relative_font_size: Some(0),
            notification_timeout_seconds: Some(-3),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(0)),
            ..StoredClientSettingsV1::default()
        };

        let resolution = ClientConfig::resolve(&settings);
        let fields = resolution
            .issues
            .iter()
            .map(|issue| issue.field.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(resolution.issues.len(), 8);
        for field in [
            "host",
            "port",
            "username",
            "rewind_threshold_seconds",
            "folder_search_timeout_seconds",
            "chat_input_relative_font_size",
            "notification_timeout_seconds",
            "autoplay_min_users",
        ] {
            assert!(fields.contains(field), "missing issue for {field}");
        }
        assert_eq!(resolution.config.connection.port.get(), DEFAULT_SERVER_PORT);
        assert_eq!(
            resolution.config.synchronization.rewind_threshold,
            SyncConfig::default().rewind_threshold
        );
        assert!(resolution.into_result().is_err());
    }

    #[test]
    fn invariant_newtypes_reject_out_of_range_values() {
        assert!(ServerPort::new(0).is_err());
        assert!(Seconds::new(-0.1).is_err());
        assert!(Seconds::new(f64::INFINITY).is_err());
        assert!(PlaybackRate::new(0.0).is_err());
        assert!(Percent::new(100.1).is_err());
        assert!(Username::new("  ").is_err());
        assert!(RoomName::new("room").is_ok());
    }

    #[test]
    fn controlled_room_password_is_normalized_into_redacted_secret() {
        let settings = StoredClientSettingsV1 {
            room: Some("+room:ABC123DEF456:pass-word".to_owned()),
            ..StoredClientSettingsV1::default()
        };
        let config = ClientConfig::try_from_stored(&settings).expect("room should resolve");
        assert_eq!(
            config.connection.room.as_ref().map(RoomName::as_str),
            Some("+room:ABC123DEF456")
        );
        let password = config
            .connection
            .controlled_room_password
            .as_ref()
            .expect("password should be present");
        assert_eq!(format!("{password:?}"), "<redacted>");
        assert_eq!(password.expose_secret(), "PASS-WORD");
    }

    #[test]
    fn resolved_runtime_config_debug_redacts_every_credential() {
        let secrets = [
            "server-password-config-secret",
            "plex-user-config-secret",
            "plex-server-config-secret",
            "ROOM-HISTORY-CONFIG-SECRET",
        ];
        let settings = StoredClientSettingsV1 {
            server_password: Some(secrets[0].into()),
            room_list: Some(vec![format!("+history:ABC123DEF456:{}", secrets[3])]),
            plex_user_token: Some(secrets[1].into()),
            plex_selected_server_token: Some(secrets[2].into()),
            ..StoredClientSettingsV1::default()
        };

        let rendered = format!("{:?}", ClientConfig::resolve(&settings).config);
        for secret in secrets {
            assert!(!rendered.contains(secret));
        }
        assert!(rendered.contains("<redacted>"));
        assert_eq!(
            ClientConfig::resolve(&settings)
                .config
                .connection
                .room_history[0]
                .as_str(),
            "+history:ABC123DEF456"
        );
        assert_eq!(
            ClientConfig::resolve(&settings)
                .config
                .connection
                .controlled_room_password
                .as_ref()
                .map(SecretValue::expose_secret),
            Some(secrets[3])
        );
    }
}
