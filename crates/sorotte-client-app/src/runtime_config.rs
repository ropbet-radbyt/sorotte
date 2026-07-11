use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
};

use sorotte_client_core::{
    DesyncCorrectionConfig, PrivacyMode, ReadinessAutoplayConfig, SessionBehaviorConfig,
    UnpauseActionMode,
};
use sorotte_secret::SecretValue;

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

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackConfig {
    pub player_path: Option<PathBuf>,
    pub per_player_arguments: BTreeMap<PathBuf, Vec<String>>,
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

impl Default for PlaybackConfig {
    fn default() -> Self {
        let behavior = SessionBehaviorConfig::default();
        Self {
            player_path: None,
            per_player_arguments: BTreeMap::new(),
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
