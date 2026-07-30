//! Generated black-box contracts for persisted configuration composition.
//!
//! The oracle is the independently generated model below. The tests only use
//! the public app boundary to render and parse `sorotte.ini`, normalize a
//! runtime snapshot, and project environment-aware startup overrides.

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use sorotte_client_app::app_boundary::{
    persistence::{
        parse_sorotte_ini_stored_client_settings_mvp, upsert_sorotte_ini_stored_client_settings_mvp,
    },
    state::{
        AutoplayThresholdOverride, StoredClientSettingsConfigPlan, StoredClientSettingsEnvPresence,
        StoredClientSettingsV1, stored_client_settings_config_plan_legacy_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

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

    fn mark_present(self, presence: &mut StoredClientSettingsEnvPresence) {
        match self {
            Self::Host => presence.host = true,
            Self::Port => presence.port = true,
            Self::ServerPassword => presence.server_password = true,
            Self::Username => presence.username = true,
            Self::Room => presence.room = true,
            Self::Autoplay => presence.autoplay = true,
            Self::AutoplayRequireSameFilenames => {
                presence.autoplay_require_same_filenames = true;
            }
            Self::ReadyAtStart => presence.ready_at_start = true,
            Self::SharedPlaylist => presence.shared_playlist_enabled = true,
            Self::PauseOnLeave => presence.pause_on_leave = true,
            Self::LoopAtEndOfPlaylist => presence.loop_at_end_of_playlist = true,
            Self::LoopSingleFiles => presence.loop_single_files = true,
            Self::OnlySwitchToTrustedDomains => {
                presence.only_switch_to_trusted_domains = true;
            }
            Self::TrustedDomains => presence.trusted_domains = true,
            Self::RewindOnDesync => presence.rewind_on_desync = true,
            Self::FastforwardOnDesync => presence.fastforward_on_desync = true,
            Self::SlowOnDesync => presence.slow_on_desync = true,
            Self::DontSlowDownWithMe => presence.dont_slow_down_with_me = true,
            Self::RewindThreshold => presence.rewind_threshold_seconds = true,
            Self::FastforwardThreshold => presence.fastforward_threshold_seconds = true,
            Self::SlowdownThreshold => presence.slowdown_threshold_seconds = true,
            Self::UnpauseAction => presence.unpause_action = true,
            Self::AutoplayMinUsers => presence.autoplay_min_users = true,
            Self::FilenamePrivacyMode => presence.filename_privacy_mode = true,
            Self::FilesizePrivacyMode => presence.filesize_privacy_mode = true,
            Self::ShowDurationNotification => presence.show_duration_notification = true,
            Self::ShowSameRoomOsd => presence.show_same_room_osd = true,
            Self::ShowOsdWarnings => presence.show_osd_warnings = true,
            Self::ShowNoncontrollerOsd => presence.show_noncontroller_osd = true,
            Self::ShowDifferentRoomOsd => presence.show_different_room_osd = true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ProjectedValue {
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

    fn to_stored(&self) -> StoredClientSettingsV1 {
        StoredClientSettingsV1 {
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
            ..StoredClientSettingsV1::default()
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

fn stored_values(settings: &StoredClientSettingsV1) -> Vec<ProjectedValue> {
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

fn plan_values(plan: &StoredClientSettingsConfigPlan) -> Vec<Option<ProjectedValue>> {
    vec![
        plan.host.clone().map(ProjectedValue::Text),
        plan.port.map(ProjectedValue::Port),
        plan.server_password
            .as_ref()
            .map(|value| ProjectedValue::Text(value.expose_secret().to_owned())),
        plan.username.clone().map(ProjectedValue::Text),
        plan.room.clone().map(ProjectedValue::Text),
        plan.autoplay_enabled.map(ProjectedValue::Bool),
        plan.autoplay_require_same_filenames
            .map(ProjectedValue::Bool),
        plan.ready_at_start_override.map(ProjectedValue::Bool),
        plan.shared_playlists_enabled_override
            .map(ProjectedValue::Bool),
        plan.pause_on_leave_override.map(ProjectedValue::Bool),
        plan.loop_at_end_of_playlist_override
            .map(ProjectedValue::Bool),
        plan.loop_single_files_override.map(ProjectedValue::Bool),
        plan.only_switch_to_trusted_domains_override
            .map(ProjectedValue::Bool),
        plan.trusted_domains_override
            .clone()
            .map(ProjectedValue::TextList),
        plan.rewind_on_desync_override.map(ProjectedValue::Bool),
        plan.fastforward_on_desync_override
            .map(ProjectedValue::Bool),
        plan.slow_on_desync_override.map(ProjectedValue::Bool),
        plan.dont_slow_down_with_me_override
            .map(ProjectedValue::Bool),
        plan.rewind_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        plan.fastforward_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        plan.slowdown_threshold_seconds_override
            .map(|value| ProjectedValue::Seconds(value.to_bits())),
        plan.unpause_action_override
            .as_ref()
            .map(|value| ProjectedValue::UnpauseAction(unpause_action_name(value))),
        plan.auto_play_threshold_override
            .as_ref()
            .map(|value| ProjectedValue::AutoplayMinUsers(autoplay_min_users_name(value))),
        plan.filename_privacy_mode
            .map(|value| ProjectedValue::PrivacyMode(privacy_mode_name(value))),
        plan.filesize_privacy_mode
            .map(|value| ProjectedValue::PrivacyMode(privacy_mode_name(value))),
        plan.show_duration_notification_override
            .map(ProjectedValue::Bool),
        plan.show_same_room_osd_override.map(ProjectedValue::Bool),
        plan.show_osd_warnings_override.map(ProjectedValue::Bool),
        plan.show_noncontroller_osd_override
            .map(ProjectedValue::Bool),
        plan.show_different_room_osd_override
            .map(ProjectedValue::Bool),
    ]
}

fn render_parse_and_plan(
    model: &GeneratedConfig,
    existing: &str,
    env_presence: &StoredClientSettingsEnvPresence,
) -> (
    String,
    StoredClientSettingsV1,
    StoredClientSettingsConfigPlan,
) {
    let rendered = upsert_sorotte_ini_stored_client_settings_mvp(existing, &model.to_stored());
    let parsed = parse_sorotte_ini_stored_client_settings_mvp(&rendered);
    let plan = stored_client_settings_config_plan_legacy_compatible(&parsed, env_presence);
    (rendered, parsed, plan)
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
    fn supported_fields_roundtrip_project_and_remain_idempotent(
        model_words in any::<[u64; 8]>(),
        fixture_words in any::<[u64; 2]>(),
    ) {
        let model = GeneratedConfig::from_words(model_words);
        let expected = model.values();
        let (fixture, sentinels) = unknown_fixture(fixture_words);
        let (rendered, parsed, plan) = render_parse_and_plan(
            &model,
            &fixture,
            &StoredClientSettingsEnvPresence::default(),
        );

        prop_assert_eq!(stored_values(&parsed), expected.clone());
        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&parsed);
        prop_assert!(
            snapshot.validation_issues.is_empty(),
            "generated canonical settings must be valid: {:?}",
            snapshot.validation_issues,
        );
        prop_assert_eq!(
            plan_values(&plan),
            expected.into_iter().map(Some).collect::<Vec<_>>(),
        );
        prop_assert_eq!(snapshot.controlled_room_password_override, None);

        for sentinel in sentinels {
            prop_assert!(
                rendered.lines().any(|line| line == sentinel),
                "unknown INI content was not preserved: {sentinel:?}",
            );
        }

        let rerendered = upsert_sorotte_ini_stored_client_settings_mvp(&rendered, &parsed);
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

        let (_, original_parsed, original_plan) = render_parse_and_plan(
            &original,
            "",
            &StoredClientSettingsEnvPresence::default(),
        );
        let (_, changed_parsed, changed_plan) = render_parse_and_plan(
            &changed,
            "",
            &StoredClientSettingsEnvPresence::default(),
        );
        let original_stored = stored_values(&original_parsed);
        let changed_stored = stored_values(&changed_parsed);
        let original_projection = plan_values(&original_plan);
        let changed_projection = plan_values(&changed_plan);

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
        let rendered = upsert_sorotte_ini_stored_client_settings_mvp("", &model.to_stored());
        let parsed = parse_sorotte_ini_stored_client_settings_mvp(&rendered);
        let baseline = stored_client_settings_config_plan_legacy_compatible(
            &parsed,
            &StoredClientSettingsEnvPresence::default(),
        );
        let mut presence = StoredClientSettingsEnvPresence::default();
        field.mark_present(&mut presence);
        let suppressed =
            stored_client_settings_config_plan_legacy_compatible(&parsed, &presence);
        let baseline_values = plan_values(&baseline);
        let suppressed_values = plan_values(&suppressed);

        for index in 0..FIELD_COUNT {
            if index == field as usize {
                prop_assert!(
                    baseline_values[index].is_some(),
                    "generated baseline {field:?} override must be present",
                );
                prop_assert_eq!(
                    &suppressed_values[index],
                    &None,
                    "environment presence did not suppress {:?}",
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
