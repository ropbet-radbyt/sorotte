//! Generated black-box contracts for persisted configuration composition.
//!
//! The oracle is the independently generated model below. The tests only use
//! the public app boundary for persistence and runtime resolution, then apply
//! the result to the actual CLI configuration with an injected environment.

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use sorotte_client_app::app_boundary::{
    persistence::{
        parse_sorotte_ini_stored_client_settings, upsert_sorotte_ini_stored_client_settings,
    },
    state::{
        AutoplayThresholdOverride, StoredClientSettings, stored_client_settings_runtime_snapshot,
    },
};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

use super::ClientLoopConfig;
use super::tests::configured;

const DEFAULT_CASES: u32 = 512;
const MAX_CASES: u32 = 100_000;
const PROPERTY_SEED: u64 = 0xC0F1_6C0A_2026_0730;
const FIELD_COUNT: usize = 30;

fn parse_case_budget(raw: &str) -> Result<u32, String> {
    raw.parse::<u32>()
        .ok()
        .filter(|cases| *cases > 0)
        .map(|cases| cases.min(MAX_CASES))
        .ok_or_else(|| format!("PROPTEST_CASES must be an integer from 1 to {MAX_CASES}"))
}

fn configured_proptest() -> ProptestConfig {
    let cases = match std::env::var_os("PROPTEST_CASES") {
        None => DEFAULT_CASES,
        Some(raw) => raw
            .to_str()
            .ok_or_else(|| {
                format!("PROPTEST_CASES must be valid Unicode and an integer from 1 to {MAX_CASES}")
            })
            .and_then(parse_case_budget)
            .unwrap_or_else(|message| panic!("{message}")),
    };
    ProptestConfig {
        cases,
        max_shrink_iters: 20_000,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum OverrideField {
    Host,
    Port,
    ServerPassword,
    Username,
    Room,
    Autoplay,
    AutoplayRequireSameFilenames,
    ReadyAtStart,
    SharedPlaylist,
    PauseOnLeave,
    LoopAtEndOfPlaylist,
    LoopSingleFiles,
    OnlySwitchToTrustedDomains,
    TrustedDomains,
    RewindOnDesync,
    FastforwardOnDesync,
    SlowOnDesync,
    DontSlowDownWithMe,
    RewindThreshold,
    FastforwardThreshold,
    SlowdownThreshold,
    UnpauseAction,
    AutoplayMinUsers,
    FilenamePrivacyMode,
    FilesizePrivacyMode,
    ShowDurationNotification,
    ShowSameRoomOsd,
    ShowOsdWarnings,
    ShowNoncontrollerOsd,
    ShowDifferentRoomOsd,
}

impl OverrideField {
    const ALL: [Self; FIELD_COUNT] = [
        Self::Host,
        Self::Port,
        Self::ServerPassword,
        Self::Username,
        Self::Room,
        Self::Autoplay,
        Self::AutoplayRequireSameFilenames,
        Self::ReadyAtStart,
        Self::SharedPlaylist,
        Self::PauseOnLeave,
        Self::LoopAtEndOfPlaylist,
        Self::LoopSingleFiles,
        Self::OnlySwitchToTrustedDomains,
        Self::TrustedDomains,
        Self::RewindOnDesync,
        Self::FastforwardOnDesync,
        Self::SlowOnDesync,
        Self::DontSlowDownWithMe,
        Self::RewindThreshold,
        Self::FastforwardThreshold,
        Self::SlowdownThreshold,
        Self::UnpauseAction,
        Self::AutoplayMinUsers,
        Self::FilenamePrivacyMode,
        Self::FilesizePrivacyMode,
        Self::ShowDurationNotification,
        Self::ShowSameRoomOsd,
        Self::ShowOsdWarnings,
        Self::ShowNoncontrollerOsd,
        Self::ShowDifferentRoomOsd,
    ];

    fn from_selector(selector: usize) -> Self {
        Self::ALL[selector % FIELD_COUNT]
    }

    fn env_name(self) -> &'static str {
        match self {
            Self::Host => "SOROTTE_CLIENT_HOST",
            Self::Port => "SOROTTE_CLIENT_PORT",
            Self::ServerPassword => "SOROTTE_CLIENT_SERVER_PASSWORD",
            Self::Username => "SOROTTE_CLIENT_USERNAME",
            Self::Room => "SOROTTE_CLIENT_ROOM",
            Self::Autoplay => "SOROTTE_CLIENT_AUTOPLAY",
            Self::AutoplayRequireSameFilenames => "SOROTTE_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES",
            Self::ReadyAtStart => "SOROTTE_CLIENT_READY_AT_START",
            Self::SharedPlaylist => "SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED",
            Self::PauseOnLeave => "SOROTTE_CLIENT_PAUSE_ON_LEAVE",
            Self::LoopAtEndOfPlaylist => "SOROTTE_CLIENT_LOOP_AT_END_OF_PLAYLIST",
            Self::LoopSingleFiles => "SOROTTE_CLIENT_LOOP_SINGLE_FILES",
            Self::OnlySwitchToTrustedDomains => "SOROTTE_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS",
            Self::TrustedDomains => "SOROTTE_CLIENT_TRUSTED_DOMAINS",
            Self::RewindOnDesync => "SOROTTE_CLIENT_REWIND_ON_DESYNC",
            Self::FastforwardOnDesync => "SOROTTE_CLIENT_FASTFORWARD_ON_DESYNC",
            Self::SlowOnDesync => "SOROTTE_CLIENT_SLOW_ON_DESYNC",
            Self::DontSlowDownWithMe => "SOROTTE_CLIENT_DONT_SLOW_DOWN_WITH_ME",
            Self::RewindThreshold => "SOROTTE_CLIENT_REWIND_THRESHOLD_SECONDS",
            Self::FastforwardThreshold => "SOROTTE_CLIENT_FASTFORWARD_THRESHOLD_SECONDS",
            Self::SlowdownThreshold => "SOROTTE_CLIENT_SLOWDOWN_THRESHOLD_SECONDS",
            Self::UnpauseAction => "SOROTTE_CLIENT_UNPAUSE_ACTION",
            Self::AutoplayMinUsers => "SOROTTE_CLIENT_AUTOPLAY_MIN_USERS",
            Self::FilenamePrivacyMode => "SOROTTE_CLIENT_FILENAME_PRIVACY_MODE",
            Self::FilesizePrivacyMode => "SOROTTE_CLIENT_FILESIZE_PRIVACY_MODE",
            Self::ShowDurationNotification => "SOROTTE_CLIENT_SHOW_DURATION_NOTIFICATION",
            Self::ShowSameRoomOsd => "SOROTTE_CLIENT_SHOW_SAME_ROOM_OSD",
            Self::ShowOsdWarnings => "SOROTTE_CLIENT_SHOW_OSD_WARNINGS",
            Self::ShowNoncontrollerOsd => "SOROTTE_CLIENT_SHOW_NONCONTROLLER_OSD",
            Self::ShowDifferentRoomOsd => "SOROTTE_CLIENT_SHOW_DIFFERENT_ROOM_OSD",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ProjectedValue {
    Text(String),
    Port(u16),
    Bool(bool),
    TextList(Vec<String>),
    Seconds(u64),
    UnpauseAction(&'static str),
    AutoplayMinUsers(String),
    PrivacyMode(&'static str),
}

#[derive(Clone, Debug)]
struct GeneratedConfig {
    host: String,
    port: u16,
    server_password: String,
    username: String,
    room: String,
    autoplay: bool,
    autoplay_require_same_filenames: bool,
    ready_at_start: bool,
    shared_playlist_enabled: bool,
    pause_on_leave: bool,
    loop_at_end_of_playlist: bool,
    loop_single_files: bool,
    only_switch_to_trusted_domains: bool,
    trusted_domains: Vec<String>,
    rewind_on_desync: bool,
    fastforward_on_desync: bool,
    slow_on_desync: bool,
    dont_slow_down_with_me: bool,
    rewind_threshold_seconds: f64,
    fastforward_threshold_seconds: f64,
    slowdown_threshold_seconds: f64,
    unpause_action: UnpauseActionMode,
    autoplay_min_users: AutoplayThresholdOverride,
    filename_privacy_mode: PrivacyMode,
    filesize_privacy_mode: PrivacyMode,
    show_duration_notification: bool,
    show_same_room_osd: bool,
    show_osd_warnings: bool,
    show_noncontroller_osd: bool,
    show_different_room_osd: bool,
}

impl GeneratedConfig {
    fn from_words(words: [u64; 8]) -> Self {
        let flag = |bit: u32| (words[3] & (1_u64 << bit)) != 0;
        let privacy = |value: u64| match value % 3 {
            0 => PrivacyMode::SendRaw,
            1 => PrivacyMode::SendHashed,
            _ => PrivacyMode::DoNotSend,
        };
        let unpause_action = match words[6] % 4 {
            0 => UnpauseActionMode::IfAlreadyReady,
            1 => UnpauseActionMode::IfOthersReady,
            2 => UnpauseActionMode::IfMinUsersReady,
            _ => UnpauseActionMode::Always,
        };
        let autoplay_min_users = if words[7] & 1 == 0 {
            AutoplayThresholdOverride::Disable
        } else {
            AutoplayThresholdOverride::Set(1 + (words[7] as usize % 32))
        };

        Self {
            host: format!("node-{:016x}.example", words[0]),
            port: 1_024 + (words[1] % 54_000) as u16,
            server_password: format!("password-{:016x}", words[2]),
            username: format!("user-{:016x}", words[0].rotate_left(17)),
            room: format!("room-{:016x}", words[1].rotate_right(11)),
            autoplay: flag(0),
            autoplay_require_same_filenames: flag(1),
            ready_at_start: flag(2),
            shared_playlist_enabled: flag(3),
            pause_on_leave: flag(4),
            loop_at_end_of_playlist: flag(5),
            loop_single_files: flag(6),
            only_switch_to_trusted_domains: flag(7),
            trusted_domains: vec![
                format!("media-{:016x}.example", words[4]),
                format!("stream-{:016x}.example", words[5]),
            ],
            rewind_on_desync: flag(8),
            fastforward_on_desync: flag(9),
            slow_on_desync: flag(10),
            dont_slow_down_with_me: flag(11),
            rewind_threshold_seconds: (1 + words[4] % 2_000) as f64 / 8.0,
            fastforward_threshold_seconds: (1 + words[5] % 2_000) as f64 / 8.0,
            slowdown_threshold_seconds: (1 + words[6] % 2_000) as f64 / 8.0,
            unpause_action,
            autoplay_min_users,
            filename_privacy_mode: privacy(words[6] >> 8),
            filesize_privacy_mode: privacy(words[7] >> 8),
            show_duration_notification: flag(12),
            show_same_room_osd: flag(13),
            show_osd_warnings: flag(14),
            show_noncontroller_osd: flag(15),
            show_different_room_osd: flag(16),
        }
    }

    fn to_stored(&self) -> StoredClientSettings {
        StoredClientSettings {
            host: Some(self.host.clone()),
            port: Some(self.port),
            server_password: Some(self.server_password.clone().into()),
            username: Some(self.username.clone()),
            room: Some(self.room.clone()),
            autoplay_initial_state: Some(self.autoplay),
            autoplay_require_same_filenames: Some(self.autoplay_require_same_filenames),
            ready_at_start: Some(self.ready_at_start),
            shared_playlist_enabled: Some(self.shared_playlist_enabled),
            pause_on_leave: Some(self.pause_on_leave),
            loop_at_end_of_playlist: Some(self.loop_at_end_of_playlist),
            loop_single_files: Some(self.loop_single_files),
            only_switch_to_trusted_domains: Some(self.only_switch_to_trusted_domains),
            trusted_domains: Some(self.trusted_domains.clone()),
            rewind_on_desync: Some(self.rewind_on_desync),
            fastforward_on_desync: Some(self.fastforward_on_desync),
            slow_on_desync: Some(self.slow_on_desync),
            dont_slow_down_with_me: Some(self.dont_slow_down_with_me),
            rewind_threshold_seconds: Some(self.rewind_threshold_seconds),
            fastforward_threshold_seconds: Some(self.fastforward_threshold_seconds),
            slowdown_threshold_seconds: Some(self.slowdown_threshold_seconds),
            unpause_action: Some(self.unpause_action.clone()),
            autoplay_min_users: Some(self.autoplay_min_users.clone()),
            filename_privacy_mode: Some(self.filename_privacy_mode),
            filesize_privacy_mode: Some(self.filesize_privacy_mode),
            show_duration_notification: Some(self.show_duration_notification),
            show_same_room_osd: Some(self.show_same_room_osd),
            show_osd_warnings: Some(self.show_osd_warnings),
            show_noncontroller_osd: Some(self.show_noncontroller_osd),
            show_different_room_osd: Some(self.show_different_room_osd),
            ..StoredClientSettings::default()
        }
    }

    fn mutate(&mut self, field: OverrideField) {
        match field {
            OverrideField::Host => self.host.push_str(".alt"),
            OverrideField::Port => self.port += 1,
            OverrideField::ServerPassword => self.server_password.push_str("-alt"),
            OverrideField::Username => self.username.push_str("-alt"),
            OverrideField::Room => self.room.push_str("-alt"),
            OverrideField::Autoplay => self.autoplay = !self.autoplay,
            OverrideField::AutoplayRequireSameFilenames => {
                self.autoplay_require_same_filenames = !self.autoplay_require_same_filenames;
            }
            OverrideField::ReadyAtStart => self.ready_at_start = !self.ready_at_start,
            OverrideField::SharedPlaylist => {
                self.shared_playlist_enabled = !self.shared_playlist_enabled;
            }
            OverrideField::PauseOnLeave => self.pause_on_leave = !self.pause_on_leave,
            OverrideField::LoopAtEndOfPlaylist => {
                self.loop_at_end_of_playlist = !self.loop_at_end_of_playlist;
            }
            OverrideField::LoopSingleFiles => {
                self.loop_single_files = !self.loop_single_files;
            }
            OverrideField::OnlySwitchToTrustedDomains => {
                self.only_switch_to_trusted_domains = !self.only_switch_to_trusted_domains;
            }
            OverrideField::TrustedDomains => self.trusted_domains[0].push_str(".alt"),
            OverrideField::RewindOnDesync => {
                self.rewind_on_desync = !self.rewind_on_desync;
            }
            OverrideField::FastforwardOnDesync => {
                self.fastforward_on_desync = !self.fastforward_on_desync;
            }
            OverrideField::SlowOnDesync => self.slow_on_desync = !self.slow_on_desync,
            OverrideField::DontSlowDownWithMe => {
                self.dont_slow_down_with_me = !self.dont_slow_down_with_me;
            }
            OverrideField::RewindThreshold => self.rewind_threshold_seconds += 0.125,
            OverrideField::FastforwardThreshold => {
                self.fastforward_threshold_seconds += 0.125;
            }
            OverrideField::SlowdownThreshold => self.slowdown_threshold_seconds += 0.125,
            OverrideField::UnpauseAction => {
                self.unpause_action = match self.unpause_action {
                    UnpauseActionMode::IfAlreadyReady => UnpauseActionMode::IfOthersReady,
                    UnpauseActionMode::IfOthersReady => UnpauseActionMode::IfMinUsersReady,
                    UnpauseActionMode::IfMinUsersReady => UnpauseActionMode::Always,
                    UnpauseActionMode::Always => UnpauseActionMode::IfAlreadyReady,
                };
            }
            OverrideField::AutoplayMinUsers => {
                self.autoplay_min_users = match self.autoplay_min_users {
                    AutoplayThresholdOverride::Disable => AutoplayThresholdOverride::Set(1),
                    AutoplayThresholdOverride::Set(_) => AutoplayThresholdOverride::Disable,
                };
            }
            OverrideField::FilenamePrivacyMode => {
                self.filename_privacy_mode = next_privacy_mode(self.filename_privacy_mode);
            }
            OverrideField::FilesizePrivacyMode => {
                self.filesize_privacy_mode = next_privacy_mode(self.filesize_privacy_mode);
            }
            OverrideField::ShowDurationNotification => {
                self.show_duration_notification = !self.show_duration_notification;
            }
            OverrideField::ShowSameRoomOsd => {
                self.show_same_room_osd = !self.show_same_room_osd;
            }
            OverrideField::ShowOsdWarnings => {
                self.show_osd_warnings = !self.show_osd_warnings;
            }
            OverrideField::ShowNoncontrollerOsd => {
                self.show_noncontroller_osd = !self.show_noncontroller_osd;
            }
            OverrideField::ShowDifferentRoomOsd => {
                self.show_different_room_osd = !self.show_different_room_osd;
            }
        }
    }

    fn values(&self) -> Vec<ProjectedValue> {
        vec![
            ProjectedValue::Text(self.host.clone()),
            ProjectedValue::Port(self.port),
            ProjectedValue::Text(self.server_password.clone()),
            ProjectedValue::Text(self.username.clone()),
            ProjectedValue::Text(self.room.clone()),
            ProjectedValue::Bool(self.autoplay),
            ProjectedValue::Bool(self.autoplay_require_same_filenames),
            ProjectedValue::Bool(self.ready_at_start),
            ProjectedValue::Bool(self.shared_playlist_enabled),
            ProjectedValue::Bool(self.pause_on_leave),
            ProjectedValue::Bool(self.loop_at_end_of_playlist),
            ProjectedValue::Bool(self.loop_single_files),
            ProjectedValue::Bool(self.only_switch_to_trusted_domains),
            ProjectedValue::TextList(self.trusted_domains.clone()),
            ProjectedValue::Bool(self.rewind_on_desync),
            ProjectedValue::Bool(self.fastforward_on_desync),
            ProjectedValue::Bool(self.slow_on_desync),
            ProjectedValue::Bool(self.dont_slow_down_with_me),
            ProjectedValue::Seconds(self.rewind_threshold_seconds.to_bits()),
            ProjectedValue::Seconds(self.fastforward_threshold_seconds.to_bits()),
            ProjectedValue::Seconds(self.slowdown_threshold_seconds.to_bits()),
            ProjectedValue::UnpauseAction(unpause_action_name(&self.unpause_action)),
            ProjectedValue::AutoplayMinUsers(autoplay_min_users_name(&self.autoplay_min_users)),
            ProjectedValue::PrivacyMode(privacy_mode_name(self.filename_privacy_mode)),
            ProjectedValue::PrivacyMode(privacy_mode_name(self.filesize_privacy_mode)),
            ProjectedValue::Bool(self.show_duration_notification),
            ProjectedValue::Bool(self.show_same_room_osd),
            ProjectedValue::Bool(self.show_osd_warnings),
            ProjectedValue::Bool(self.show_noncontroller_osd),
            ProjectedValue::Bool(self.show_different_room_osd),
        ]
    }
}

fn next_privacy_mode(mode: PrivacyMode) -> PrivacyMode {
    match mode {
        PrivacyMode::SendRaw => PrivacyMode::SendHashed,
        PrivacyMode::SendHashed => PrivacyMode::DoNotSend,
        PrivacyMode::DoNotSend => PrivacyMode::SendRaw,
    }
}

fn unpause_action_name(action: &UnpauseActionMode) -> &'static str {
    match action {
        UnpauseActionMode::IfAlreadyReady => "if-already-ready",
        UnpauseActionMode::IfOthersReady => "if-others-ready",
        UnpauseActionMode::IfMinUsersReady => "if-min-users-ready",
        UnpauseActionMode::Always => "always",
    }
}

fn autoplay_min_users_name(value: &AutoplayThresholdOverride) -> String {
    match value {
        AutoplayThresholdOverride::Disable => "disabled".to_owned(),
        AutoplayThresholdOverride::Set(count) => format!("minimum:{count}"),
    }
}

fn privacy_mode_name(mode: PrivacyMode) -> &'static str {
    match mode {
        PrivacyMode::SendRaw => "raw",
        PrivacyMode::SendHashed => "hashed",
        PrivacyMode::DoNotSend => "none",
    }
}

fn required<T>(value: Option<T>, field: OverrideField) -> T {
    value.unwrap_or_else(|| panic!("generated field {field:?} must remain present"))
}

fn stored_values(settings: &StoredClientSettings) -> Vec<ProjectedValue> {
    vec![
        ProjectedValue::Text(required(settings.host.clone(), OverrideField::Host)),
        ProjectedValue::Port(required(settings.port, OverrideField::Port)),
        ProjectedValue::Text(
            required(
                settings.server_password.as_ref(),
                OverrideField::ServerPassword,
            )
            .expose_secret()
            .to_owned(),
        ),
        ProjectedValue::Text(required(settings.username.clone(), OverrideField::Username)),
        ProjectedValue::Text(required(settings.room.clone(), OverrideField::Room)),
        ProjectedValue::Bool(required(
            settings.autoplay_initial_state,
            OverrideField::Autoplay,
        )),
        ProjectedValue::Bool(required(
            settings.autoplay_require_same_filenames,
            OverrideField::AutoplayRequireSameFilenames,
        )),
        ProjectedValue::Bool(required(
            settings.ready_at_start,
            OverrideField::ReadyAtStart,
        )),
        ProjectedValue::Bool(required(
            settings.shared_playlist_enabled,
            OverrideField::SharedPlaylist,
        )),
        ProjectedValue::Bool(required(
            settings.pause_on_leave,
            OverrideField::PauseOnLeave,
        )),
        ProjectedValue::Bool(required(
            settings.loop_at_end_of_playlist,
            OverrideField::LoopAtEndOfPlaylist,
        )),
        ProjectedValue::Bool(required(
            settings.loop_single_files,
            OverrideField::LoopSingleFiles,
        )),
        ProjectedValue::Bool(required(
            settings.only_switch_to_trusted_domains,
            OverrideField::OnlySwitchToTrustedDomains,
        )),
        ProjectedValue::TextList(required(
            settings.trusted_domains.clone(),
            OverrideField::TrustedDomains,
        )),
        ProjectedValue::Bool(required(
            settings.rewind_on_desync,
            OverrideField::RewindOnDesync,
        )),
        ProjectedValue::Bool(required(
            settings.fastforward_on_desync,
            OverrideField::FastforwardOnDesync,
        )),
        ProjectedValue::Bool(required(
            settings.slow_on_desync,
            OverrideField::SlowOnDesync,
        )),
        ProjectedValue::Bool(required(
            settings.dont_slow_down_with_me,
            OverrideField::DontSlowDownWithMe,
        )),
        ProjectedValue::Seconds(
            required(
                settings.rewind_threshold_seconds,
                OverrideField::RewindThreshold,
            )
            .to_bits(),
        ),
        ProjectedValue::Seconds(
            required(
                settings.fastforward_threshold_seconds,
                OverrideField::FastforwardThreshold,
            )
            .to_bits(),
        ),
        ProjectedValue::Seconds(
            required(
                settings.slowdown_threshold_seconds,
                OverrideField::SlowdownThreshold,
            )
            .to_bits(),
        ),
        ProjectedValue::UnpauseAction(unpause_action_name(&required(
            settings.unpause_action.clone(),
            OverrideField::UnpauseAction,
        ))),
        ProjectedValue::AutoplayMinUsers(autoplay_min_users_name(&required(
            settings.autoplay_min_users.clone(),
            OverrideField::AutoplayMinUsers,
        ))),
        ProjectedValue::PrivacyMode(privacy_mode_name(required(
            settings.filename_privacy_mode,
            OverrideField::FilenamePrivacyMode,
        ))),
        ProjectedValue::PrivacyMode(privacy_mode_name(required(
            settings.filesize_privacy_mode,
            OverrideField::FilesizePrivacyMode,
        ))),
        ProjectedValue::Bool(required(
            settings.show_duration_notification,
            OverrideField::ShowDurationNotification,
        )),
        ProjectedValue::Bool(required(
            settings.show_same_room_osd,
            OverrideField::ShowSameRoomOsd,
        )),
        ProjectedValue::Bool(required(
            settings.show_osd_warnings,
            OverrideField::ShowOsdWarnings,
        )),
        ProjectedValue::Bool(required(
            settings.show_noncontroller_osd,
            OverrideField::ShowNoncontrollerOsd,
        )),
        ProjectedValue::Bool(required(
            settings.show_different_room_osd,
            OverrideField::ShowDifferentRoomOsd,
        )),
    ]
}

pub(super) fn config_values(config: &ClientLoopConfig) -> Vec<Option<ProjectedValue>> {
    vec![
        Some(ProjectedValue::Text(config.host.clone())),
        Some(ProjectedValue::Port(config.port)),
        config
            .server_password
            .as_ref()
            .map(|value| ProjectedValue::Text(value.expose_secret().to_owned())),
        Some(ProjectedValue::Text(config.username.clone())),
        Some(ProjectedValue::Text(config.room.clone())),
        Some(ProjectedValue::Bool(config.autoplay_enabled)),
        Some(ProjectedValue::Bool(config.autoplay_require_same_filenames)),
        config.ready_at_start_override.map(ProjectedValue::Bool),
        config
            .shared_playlists_enabled_override
            .map(ProjectedValue::Bool),
        config.pause_on_leave_override.map(ProjectedValue::Bool),
        config
            .loop_at_end_of_playlist_override
            .map(ProjectedValue::Bool),
        config.loop_single_files_override.map(ProjectedValue::Bool),
        config
            .only_switch_to_trusted_domains_override
            .map(ProjectedValue::Bool),
        config
            .trusted_domains_override
            .clone()
            .map(ProjectedValue::TextList),
        config.rewind_on_desync_override.map(ProjectedValue::Bool),
        config
            .fastforward_on_desync_override
            .map(ProjectedValue::Bool),
        config.slow_on_desync_override.map(ProjectedValue::Bool),
        config
            .dont_slow_down_with_me_override
            .map(ProjectedValue::Bool),
        config
            .rewind_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        config
            .fastforward_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        config
            .slowdown_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        config
            .unpause_action_override
            .as_ref()
            .map(|value| ProjectedValue::UnpauseAction(unpause_action_name(value))),
        config
            .auto_play_threshold_override
            .as_ref()
            .map(|value| ProjectedValue::AutoplayMinUsers(autoplay_min_users_name(value))),
        Some(ProjectedValue::PrivacyMode(privacy_mode_name(
            config.filename_privacy_mode,
        ))),
        Some(ProjectedValue::PrivacyMode(privacy_mode_name(
            config.filesize_privacy_mode,
        ))),
        config
            .show_duration_notification_override
            .map(ProjectedValue::Bool),
        config.show_same_room_osd_override.map(ProjectedValue::Bool),
        config.show_osd_warnings_override.map(ProjectedValue::Bool),
        config
            .show_noncontroller_osd_override
            .map(ProjectedValue::Bool),
        config
            .show_different_room_osd_override
            .map(ProjectedValue::Bool),
    ]
}

fn render_parse_and_apply(
    model: &GeneratedConfig,
    existing: &str,
) -> (String, StoredClientSettings, ClientLoopConfig) {
    let rendered = upsert_sorotte_ini_stored_client_settings(existing, &model.to_stored());
    let parsed = parse_sorotte_ini_stored_client_settings(&rendered);
    let config = configured(&parsed, &[]);
    (rendered, parsed, config)
}

fn unknown_fixture(words: [u64; 2]) -> (String, Vec<String>) {
    let comment = format!("; future-comment-{:016x}", words[0]);
    let unknown_section = format!("future_extension_{:08x}", words[0] as u32);
    let unknown_key = format!("futureKey{:08x}", words[1] as u32);
    let unknown_value = format!("value-{:016x}", words[0] ^ words[1]);
    let server_key = format!("futureServerKey{:08x}", (words[1] >> 32) as u32);
    let client_key = format!("futureClientKey{:08x}", (words[0] >> 32) as u32);
    let contents = format!(
        "{comment}\n\
         [{unknown_section}]\n\
         {unknown_key} = {unknown_value}\n\
         [server_data]\n\
         {server_key} = preserve-server\n\
         [client_settings]\n\
         {client_key} = preserve-client\n"
    );
    let sentinels = vec![
        comment,
        format!("[{unknown_section}]"),
        format!("{unknown_key} = {unknown_value}"),
        format!("{server_key} = preserve-server"),
        format!("{client_key} = preserve-client"),
    ];
    (contents, sentinels)
}

#[test]
fn property_case_budget_is_fail_closed_and_bounded() {
    assert_eq!(parse_case_budget("1"), Ok(1));
    assert_eq!(parse_case_budget("2048"), Ok(2_048));
    assert_eq!(
        parse_case_budget("100001"),
        Ok(MAX_CASES),
        "excess depth should remain deterministically capped"
    );
    for malformed in ["", "0", "-1", "1.5", "lots"] {
        assert!(
            parse_case_budget(malformed).is_err(),
            "{malformed:?} must not silently reduce coverage"
        );
    }
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn absent_saved_values_and_present_environment_preserve_existing_cli_values(
        model_words in any::<[u64; 8]>(),
    ) {
        let model = GeneratedConfig::from_words(model_words);
        let initial = configured(&model.to_stored(), &[]);
        let expected = config_values(&initial);
        let mut config = initial.clone();
        super::apply_stored_client_settings(&mut config, &StoredClientSettings::default(), |_| None);
        prop_assert_eq!(config_values(&config), expected.clone());

        let mut changed = model.clone();
        for field in OverrideField::ALL {
            changed.mutate(field);
        }
        super::apply_stored_client_settings(&mut config, &changed.to_stored(), |_| Some("1".into()));
        prop_assert_eq!(config_values(&config), expected);
    }

    #[test]
    fn supported_fields_roundtrip_project_and_remain_idempotent(
        model_words in any::<[u64; 8]>(),
        fixture_words in any::<[u64; 2]>(),
    ) {
        let model = GeneratedConfig::from_words(model_words);
        let expected = model.values();
        let (fixture, sentinels) = unknown_fixture(fixture_words);
        let (rendered, parsed, config) = render_parse_and_apply(
            &model,
            &fixture,
        );

        prop_assert_eq!(stored_values(&parsed), expected.clone());
        let snapshot = stored_client_settings_runtime_snapshot(&parsed);
        prop_assert!(
            snapshot.validation_issues.is_empty(),
            "generated canonical settings must be valid: {:?}",
            snapshot.validation_issues,
        );
        prop_assert_eq!(
            config_values(&config),
            expected.into_iter().map(Some).collect::<Vec<_>>(),
        );
        prop_assert_eq!(snapshot.controlled_room_password_override, None);

        for sentinel in sentinels {
            prop_assert!(
                rendered.lines().any(|line| line == sentinel),
                "unknown INI content was not preserved: {sentinel:?}",
            );
        }

        let rerendered = upsert_sorotte_ini_stored_client_settings(&rendered, &parsed);
        prop_assert_eq!(rerendered, rendered);
    }

    #[test]
    fn changing_one_stored_field_does_not_disturb_other_projections(
        model_words in any::<[u64; 8]>(),
        field_selector in 0_usize..FIELD_COUNT,
    ) {
        let field = OverrideField::from_selector(field_selector);
        let original = GeneratedConfig::from_words(model_words);
        let mut changed = original.clone();
        changed.mutate(field);

        let (_, original_parsed, original_config) = render_parse_and_apply(
            &original,
            "",
        );
        let (_, changed_parsed, changed_config) = render_parse_and_apply(
            &changed,
            "",
        );
        let original_stored = stored_values(&original_parsed);
        let changed_stored = stored_values(&changed_parsed);
        let original_projection = config_values(&original_config);
        let changed_projection = config_values(&changed_config);

        for index in 0..FIELD_COUNT {
            if index == field as usize {
                prop_assert_ne!(
                    &changed_stored[index],
                    &original_stored[index],
                    "selected stored field {:?} did not change",
                    field,
                );
                prop_assert_ne!(
                    &changed_projection[index],
                    &original_projection[index],
                    "selected projected field {:?} did not change",
                    field,
                );
            } else {
                prop_assert_eq!(
                    &changed_stored[index],
                    &original_stored[index],
                    "stored field {:?} changed while mutating {:?}",
                    OverrideField::ALL[index],
                    field,
                );
                prop_assert_eq!(
                    &changed_projection[index],
                    &original_projection[index],
                    "projected field {:?} changed while mutating {:?}",
                    OverrideField::ALL[index],
                    field,
                );
            }
        }
    }

    #[test]
    fn one_present_environment_field_suppresses_exactly_its_stored_override(
        model_words in any::<[u64; 8]>(),
        field_selector in 0_usize..FIELD_COUNT,
    ) {
        let field = OverrideField::from_selector(field_selector);
        let model = GeneratedConfig::from_words(model_words);
        let rendered = upsert_sorotte_ini_stored_client_settings("", &model.to_stored());
        let parsed = parse_sorotte_ini_stored_client_settings(&rendered);
        let baseline = configured(&parsed, &[]);
        let suppressed = configured(&parsed, &[field.env_name()]);
        let initial_values = config_values(&crate::tests::test_client_loop_config());
        let baseline_values = config_values(&baseline);
        let suppressed_values = config_values(&suppressed);

        for index in 0..FIELD_COUNT {
            if index == field as usize {
                prop_assert!(
                    baseline_values[index].is_some(),
                    "generated baseline {field:?} override must be present",
                );
                prop_assert_eq!(
                    &suppressed_values[index],
                    &initial_values[index],
                    "environment presence did not preserve the initial CLI value for {:?}",
                    field,
                );
            } else {
                prop_assert_eq!(
                    &suppressed_values[index],
                    &baseline_values[index],
                    "environment presence for {:?} disturbed {:?}",
                    field,
                    OverrideField::ALL[index],
                );
            }
        }
    }
}
