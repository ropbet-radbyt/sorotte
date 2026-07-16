use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use sorotte_secret::SecretValue;

use super::super::GuiLaunchMode;

macro_rules! define_setting_ids {
    ($($variant:ident => ($section:literal, $label:literal, $automation_id:literal)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub(in crate::app) enum SettingId {
            $($variant),+
        }

        impl SettingId {
            pub(in crate::app) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(in crate::app) const fn section(self) -> &'static str {
                match self {
                    $(Self::$variant => $section),+
                }
            }

            pub(in crate::app) const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }

            pub(in crate::app) const fn automation_id(self) -> &'static str {
                match self {
                    $(Self::$variant => $automation_id),+
                }
            }

            pub(in crate::app) fn from_automation_id(value: &str) -> Option<Self> {
                match value {
                    $($automation_id => Some(Self::$variant)),+,
                    _ => None,
                }
            }

            pub(in crate::app) fn section_automation_id(self) -> &'static str {
                let id = self.automation_id();
                if id.starts_with("settings.connection.") || id.starts_with("settings.player.") {
                    "settings.section.connection"
                } else if id.starts_with("settings.playback.") {
                    "settings.section.playback"
                } else if id.starts_with("settings.privacy.") {
                    "settings.section.privacy"
                } else if id.starts_with("settings.sync.") {
                    "settings.section.sync"
                } else if id.starts_with("settings.streaming.") {
                    "settings.section.streaming"
                } else if id.starts_with("settings.media_library.") {
                    "settings.section.media_library"
                } else if id.starts_with("settings.chat.") {
                    "settings.section.chat"
                } else if id.starts_with("settings.osd.") {
                    "settings.section.osd"
                } else {
                    "settings.section.general"
                }
            }
        }
    };
}

define_setting_ids! {
    ConnectionHost => ("Connection", "Host", "settings.connection.host"),
    ConnectionPort => ("Connection", "Port", "settings.connection.port"),
    ConnectionUsername => ("Connection", "Username", "settings.connection.username"),
    ConnectionRoom => ("Connection", "Room", "settings.connection.room"),
    ConnectionServerPassword => (
        "Connection",
        "Server Password",
        "settings.connection.server_password"
    ),
    PlayerExecutable => ("Connection", "Player Path", "settings.player.executable"),
    PlayerArguments => ("Connection", "Player Arguments", "settings.player.arguments"),
    ConnectionPublicServerCount => (
        "Connection",
        "Public Servers",
        "settings.connection.public_server_count"
    ),
    ConnectionRoomHistory => (
        "Connection",
        "Room History",
        "settings.connection.room_history"
    ),
    ConnectionRoomHistoryCount => (
        "Connection",
        "Room History Count",
        "settings.connection.room_history_count"
    ),
    PlaybackReadyAtStart => (
        "Readiness",
        "Ready At Start",
        "settings.playback.ready_at_start"
    ),
    PlaybackAutoplay => ("Readiness", "Autoplay", "settings.playback.autoplay"),
    PlaybackRequireSameFilenames => (
        "Readiness",
        "Require Same Filenames",
        "settings.playback.require_same_filenames"
    ),
    PlaybackSharedPlaylists => (
        "Readiness",
        "Shared Playlists",
        "settings.playback.shared_playlists"
    ),
    PlaybackPauseOnLeave => (
        "Readiness",
        "Pause On Leave",
        "settings.playback.pause_on_leave"
    ),
    PlaybackLoopPlaylist => (
        "Readiness",
        "Loop At End Of Playlist",
        "settings.playback.loop_playlist"
    ),
    PlaybackLoopSingleFiles => (
        "Readiness",
        "Loop Single Files",
        "settings.playback.loop_single_files"
    ),
    PlaybackUnpauseAction => (
        "Readiness",
        "Unpause Action",
        "settings.playback.unpause_action"
    ),
    PlaybackAutoplayMinUsers => (
        "Readiness",
        "Autoplay Min Users",
        "settings.playback.autoplay_min_users"
    ),
    PrivacyFilename => (
        "Privacy",
        "Filename Privacy",
        "settings.privacy.filename"
    ),
    PrivacyFilesize => (
        "Privacy",
        "Filesize Privacy",
        "settings.privacy.filesize"
    ),
    PrivacyTrustedDomainsOnly => (
        "Privacy",
        "Trusted Domains Only",
        "settings.privacy.trusted_domains_only"
    ),
    PrivacyTrustedDomains => (
        "Privacy",
        "Trusted Domains",
        "settings.privacy.trusted_domains"
    ),
    PrivacyTrustedDomainCount => (
        "Privacy",
        "Trusted Domain Count",
        "settings.privacy.trusted_domain_count"
    ),
    SyncRewindOnDesync => ("Desync", "Rewind On Desync", "settings.sync.rewind_on_desync"),
    SyncFastforwardOnDesync => (
        "Desync",
        "Fastforward On Desync",
        "settings.sync.fastforward_on_desync"
    ),
    SyncSlowOnDesync => ("Desync", "Slow On Desync", "settings.sync.slow_on_desync"),
    SyncDontSlowDownWithMe => (
        "Desync",
        "Dont Slow Down With Me",
        "settings.sync.dont_slow_down_with_me"
    ),
    SyncRewindThreshold => (
        "Desync",
        "Rewind Threshold",
        "settings.sync.rewind_threshold"
    ),
    SyncFastforwardThreshold => (
        "Desync",
        "Fastforward Threshold",
        "settings.sync.fastforward_threshold"
    ),
    SyncSlowdownThreshold => (
        "Desync",
        "Slowdown Threshold",
        "settings.sync.slowdown_threshold"
    ),
    StreamingQuality => ("Streaming", "Quality", "settings.streaming.quality"),
    StreamingCustomFormat => (
        "Streaming",
        "Custom Format",
        "settings.streaming.custom_format"
    ),
    StreamingBufferTargetSeconds => (
        "Streaming",
        "Buffer Target Seconds",
        "settings.streaming.buffer_target_seconds"
    ),
    StreamingReadAheadSeconds => (
        "Streaming",
        "Read Ahead Seconds",
        "settings.streaming.read_ahead_seconds"
    ),
    StreamingMemoryCacheMib => (
        "Streaming",
        "Memory Cache MiB",
        "settings.streaming.memory_cache_mib"
    ),
    StreamingDiskCache => ("Streaming", "Disk Cache", "settings.streaming.disk_cache"),
    StreamingRecoveryPolicy => (
        "Streaming",
        "Recovery Policy",
        "settings.streaming.recovery_policy"
    ),
    StreamingMaximumCatchupRate => (
        "Streaming",
        "Maximum Catchup Rate",
        "settings.streaming.maximum_catchup_rate"
    ),
    StreamingHardSeekThresholdSeconds => (
        "Streaming",
        "Hard Seek Threshold Seconds",
        "settings.streaming.hard_seek_threshold_seconds"
    ),
    StreamingMaximumHardSeeks => (
        "Streaming",
        "Maximum Hard Seeks",
        "settings.streaming.maximum_hard_seeks"
    ),
    StreamingStabilityIntervalSeconds => (
        "Streaming",
        "Stability Interval Seconds",
        "settings.streaming.stability_interval_seconds"
    ),
    StreamingRecoveryRetryBudget => (
        "Streaming",
        "Recovery Retry Budget",
        "settings.streaming.recovery_retry_budget"
    ),
    StreamingRecoveryCooldownSeconds => (
        "Streaming",
        "Recovery Cooldown Seconds",
        "settings.streaming.recovery_cooldown_seconds"
    ),
    StreamingRoomBufferingPolicy => (
        "Streaming",
        "Room Buffering Policy",
        "settings.streaming.room_buffering_policy"
    ),
    StreamingRoomQuorumPercent => (
        "Streaming",
        "Room Quorum Percent",
        "settings.streaming.room_quorum_percent"
    ),
    StreamingRoomMaximumPauseSeconds => (
        "Streaming",
        "Room Maximum Pause Seconds",
        "settings.streaming.room_maximum_pause_seconds"
    ),
    StreamingStartSynchronization => (
        "Streaming",
        "Start Synchronization",
        "settings.streaming.start_synchronization"
    ),
    StreamingStartQuorumPercent => (
        "Streaming",
        "Start Quorum Percent",
        "settings.streaming.start_quorum_percent"
    ),
    StreamingStartTimeoutSeconds => (
        "Streaming",
        "Start Timeout Seconds",
        "settings.streaming.start_timeout_seconds"
    ),
    StreamingStartTimeoutAction => (
        "Streaming",
        "Start Timeout Action",
        "settings.streaming.start_timeout_action"
    ),
    StreamingQualityDowngradeSuggestions => (
        "Streaming",
        "Quality Downgrade Suggestions",
        "settings.streaming.quality_downgrade_suggestions"
    ),
    StreamingEffectiveMpvOptions => (
        "Streaming",
        "Effective mpv Options",
        "settings.streaming.effective_mpv_options"
    ),
    MediaLibraryDirectories => (
        "Media Search",
        "Directories",
        "settings.media_library.directories"
    ),
    MediaLibraryDirectoryCount => (
        "Media Search",
        "Directory Count",
        "settings.media_library.directory_count"
    ),
    MediaLibraryFirstFileTimeout => (
        "Media Search",
        "First File Timeout",
        "settings.media_library.first_file_timeout"
    ),
    MediaLibrarySearchTimeout => (
        "Media Search",
        "Search Timeout",
        "settings.media_library.search_timeout"
    ),
    MediaLibraryDoubleCheckInterval => (
        "Media Search",
        "Double Check Interval",
        "settings.media_library.double_check_interval"
    ),
    MediaLibraryWarningThreshold => (
        "Media Search",
        "Warning Threshold",
        "settings.media_library.warning_threshold"
    ),
    ChatInputEnabled => ("Chat", "Chat Input", "settings.chat.input_enabled"),
    ChatOutputEnabled => ("Chat", "Chat Output", "settings.chat.output_enabled"),
    ChatDirectInput => ("Chat", "Direct Input", "settings.chat.direct_input"),
    ChatMoveOsd => ("Chat", "Move OSD", "settings.chat.move_osd"),
    ChatInputPosition => ("Chat", "Input Position", "settings.chat.input_position"),
    ChatOutputMode => ("Chat", "Output Mode", "settings.chat.output_mode"),
    ChatMaxLines => ("Chat", "Max Lines", "settings.chat.max_lines"),
    ChatInputFont => ("Chat", "Input Font", "settings.chat.input_font"),
    ChatInputFontSize => ("Chat", "Input Font Size", "settings.chat.input_font_size"),
    ChatInputFontWeight => (
        "Chat",
        "Input Font Weight",
        "settings.chat.input_font_weight"
    ),
    ChatInputColor => ("Chat", "Input Color", "settings.chat.input_color"),
    ChatOutputFont => ("Chat", "Output Font", "settings.chat.output_font"),
    ChatOutputFontSize => (
        "Chat",
        "Output Font Size",
        "settings.chat.output_font_size"
    ),
    ChatOutputFontWeight => (
        "Chat",
        "Output Font Weight",
        "settings.chat.output_font_weight"
    ),
    ChatTopMargin => ("Chat", "Top Margin", "settings.chat.top_margin"),
    ChatLeftMargin => ("Chat", "Left Margin", "settings.chat.left_margin"),
    ChatBottomMargin => ("Chat", "Bottom Margin", "settings.chat.bottom_margin"),
    ChatOsdMargin => ("Chat", "OSD Margin", "settings.chat.osd_margin"),
    OsdShow => ("OSD", "Show OSD", "settings.osd.show"),
    OsdShowDuration => ("OSD", "Show Duration", "settings.osd.show_duration"),
    OsdShowSameRoom => ("OSD", "Show Same Room", "settings.osd.show_same_room"),
    OsdShowWarnings => ("OSD", "Show Warnings", "settings.osd.show_warnings"),
    OsdShowSlowdown => ("OSD", "Show Slowdown", "settings.osd.show_slowdown"),
    OsdShowNoncontroller => (
        "OSD",
        "Show Noncontroller",
        "settings.osd.show_noncontroller"
    ),
    OsdShowDifferentRoom => (
        "OSD",
        "Show Different Room",
        "settings.osd.show_different_room"
    ),
    OsdShowContactInfo => (
        "OSD",
        "Show Contact Info",
        "settings.osd.show_contact_info"
    ),
    OsdNotificationTimeout => (
        "OSD",
        "Notification Timeout",
        "settings.osd.notification_timeout"
    ),
    OsdAlertTimeout => ("OSD", "Alert Timeout", "settings.osd.alert_timeout"),
    OsdChatTimeout => ("OSD", "Chat Timeout", "settings.osd.chat_timeout"),
    GeneralLanguage => ("System", "Language", "settings.general.language"),
    GeneralCheckForUpdatesAutomatically => (
        "System",
        "Check for updates automatically",
        "settings.general.check_for_updates_automatically"
    ),
    GeneralUpdateChannel => ("System", "Update Channel", "settings.general.update_channel"),
    GeneralAutosaveJoinsToList => (
        "System",
        "Autosave Joins To List",
        "settings.general.autosave_joins_to_list"
    ),
    GeneralForceGuiPrompt => (
        "System",
        "Force GUI Prompt",
        "settings.general.force_gui_prompt"
    ),
    DiagnosticsSupportedLanguages => (
        "System",
        "Supported Languages",
        "settings.diagnostics.supported_languages"
    ),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(
    dead_code,
    reason = "Immediate feature transactions share this apply taxonomy but are not editable SettingId controls."
)]
pub(in crate::app) enum GuiSettingApplyRequirement {
    Immediate,
    OnSave,
    Reconnect,
    RestartPlayer,
    RestartApplication,
}

impl GuiSettingApplyRequirement {
    pub(in crate::app) const fn label(self) -> &'static str {
        match self {
            Self::Immediate => "Applies immediately",
            Self::OnSave => "Applies when saved",
            Self::Reconnect => "Reconnect required",
            Self::RestartPlayer => "Player restart required",
            Self::RestartApplication => "Sorotte restart required",
        }
    }

    pub(in crate::app) const fn automation_id(self) -> &'static str {
        match self {
            Self::Immediate => "settings.apply.immediate",
            Self::OnSave => "settings.apply.on_save",
            Self::Reconnect => "settings.apply.reconnect",
            Self::RestartPlayer => "settings.apply.restart_player",
            Self::RestartApplication => "settings.apply.restart_application",
        }
    }
}

impl SettingId {
    pub(in crate::app) const fn apply_requirement(self) -> GuiSettingApplyRequirement {
        match self {
            Self::ConnectionHost
            | Self::ConnectionPort
            | Self::ConnectionUsername
            | Self::ConnectionRoom
            | Self::ConnectionServerPassword
            | Self::PlaybackReadyAtStart
            | Self::PlaybackAutoplay
            | Self::PlaybackRequireSameFilenames
            | Self::PlaybackSharedPlaylists
            | Self::PlaybackPauseOnLeave
            | Self::PlaybackLoopPlaylist
            | Self::PlaybackLoopSingleFiles
            | Self::PlaybackUnpauseAction
            | Self::PlaybackAutoplayMinUsers
            | Self::PrivacyFilename
            | Self::PrivacyFilesize
            | Self::PrivacyTrustedDomainsOnly
            | Self::PrivacyTrustedDomains
            | Self::PrivacyTrustedDomainCount
            | Self::SyncRewindOnDesync
            | Self::SyncFastforwardOnDesync
            | Self::SyncSlowOnDesync
            | Self::SyncDontSlowDownWithMe
            | Self::SyncRewindThreshold
            | Self::SyncFastforwardThreshold
            | Self::SyncSlowdownThreshold
            | Self::ChatInputEnabled
            | Self::OsdShowDuration
            | Self::OsdShowSameRoom
            | Self::OsdShowWarnings
            | Self::OsdShowSlowdown
            | Self::OsdShowNoncontroller
            | Self::OsdShowDifferentRoom
            | Self::StreamingQuality
            | Self::StreamingBufferTargetSeconds
            | Self::StreamingRecoveryPolicy
            | Self::StreamingMaximumCatchupRate
            | Self::StreamingHardSeekThresholdSeconds
            | Self::StreamingMaximumHardSeeks
            | Self::StreamingStabilityIntervalSeconds
            | Self::StreamingRecoveryRetryBudget
            | Self::StreamingRecoveryCooldownSeconds
            | Self::StreamingRoomBufferingPolicy
            | Self::StreamingRoomQuorumPercent
            | Self::StreamingRoomMaximumPauseSeconds
            | Self::StreamingStartSynchronization
            | Self::StreamingStartQuorumPercent
            | Self::StreamingStartTimeoutSeconds
            | Self::StreamingStartTimeoutAction
            | Self::StreamingQualityDowngradeSuggestions => GuiSettingApplyRequirement::Reconnect,
            Self::PlayerExecutable | Self::PlayerArguments => {
                GuiSettingApplyRequirement::RestartPlayer
            }
            Self::GeneralLanguage | Self::GeneralForceGuiPrompt => {
                GuiSettingApplyRequirement::RestartApplication
            }
            Self::ConnectionPublicServerCount
            | Self::ConnectionRoomHistory
            | Self::ConnectionRoomHistoryCount
            | Self::ChatDirectInput
            | Self::ChatOutputEnabled
            | Self::ChatMoveOsd
            | Self::ChatInputPosition
            | Self::ChatOutputMode
            | Self::ChatMaxLines
            | Self::ChatInputFont
            | Self::ChatInputFontSize
            | Self::ChatInputFontWeight
            | Self::ChatInputColor
            | Self::ChatOutputFont
            | Self::ChatOutputFontSize
            | Self::ChatOutputFontWeight
            | Self::ChatTopMargin
            | Self::ChatLeftMargin
            | Self::ChatBottomMargin
            | Self::ChatOsdMargin
            | Self::OsdShow
            | Self::OsdShowContactInfo
            | Self::OsdNotificationTimeout
            | Self::OsdAlertTimeout
            | Self::OsdChatTimeout
            | Self::StreamingCustomFormat
            | Self::StreamingReadAheadSeconds
            | Self::StreamingMemoryCacheMib
            | Self::StreamingDiskCache
            | Self::StreamingEffectiveMpvOptions
            | Self::MediaLibraryDirectories
            | Self::MediaLibraryDirectoryCount
            | Self::MediaLibraryFirstFileTimeout
            | Self::MediaLibrarySearchTimeout
            | Self::MediaLibraryDoubleCheckInterval
            | Self::MediaLibraryWarningThreshold
            | Self::GeneralCheckForUpdatesAutomatically
            | Self::GeneralUpdateChannel
            | Self::GeneralAutosaveJoinsToList
            | Self::DiagnosticsSupportedLanguages => GuiSettingApplyRequirement::OnSave,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiSettingValueOrigin {
    StoredOverride,
    ApplicationDefault,
    DraftChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiResolvedSettingValue<T> {
    pub(in crate::app) stored_override: Option<T>,
    pub(in crate::app) effective: T,
}

impl<T> GuiResolvedSettingValue<T> {
    pub(in crate::app) fn origin(&self) -> GuiSettingValueOrigin {
        if self.stored_override.is_some() {
            GuiSettingValueOrigin::StoredOverride
        } else {
            GuiSettingValueOrigin::ApplicationDefault
        }
    }
}

impl<T: PartialEq> GuiResolvedSettingValue<T> {
    pub(in crate::app) fn origin_against_persisted(
        &self,
        persisted: &Self,
    ) -> GuiSettingValueOrigin {
        if self.stored_override != persisted.stored_override {
            GuiSettingValueOrigin::DraftChange
        } else {
            self.origin()
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum SecretDraft {
    Unchanged,
    Replace(SecretValue),
    Clear,
}

impl std::fmt::Debug for SecretDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unchanged => formatter.write_str("Unchanged"),
            Self::Replace(_) => formatter.write_str("Replace([REDACTED])"),
            Self::Clear => formatter.write_str("Clear"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiConnectionSettingsSection {
    pub(in crate::app) host: Option<String>,
    pub(in crate::app) port: Option<u16>,
    pub(in crate::app) username: Option<String>,
    pub(in crate::app) room: Option<String>,
    pub(in crate::app) server_password_set: bool,
    pub(in crate::app) player_path: Option<String>,
    pub(in crate::app) player_arguments_text: String,
    pub(in crate::app) room_history_text: String,
    pub(in crate::app) public_server_count: usize,
    pub(in crate::app) room_history_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiReadinessSection {
    pub(in crate::app) ready_at_start: bool,
    pub(in crate::app) autoplay_enabled: bool,
    pub(in crate::app) autoplay_require_same_filenames: bool,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) pause_on_leave: bool,
    pub(in crate::app) loop_at_end_of_playlist: bool,
    pub(in crate::app) loop_single_files: bool,
    pub(in crate::app) unpause_action: GuiResolvedSettingValue<String>,
    pub(in crate::app) autoplay_min_users: GuiResolvedSettingValue<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPrivacySection {
    pub(in crate::app) filename_privacy_mode_label: String,
    pub(in crate::app) filesize_privacy_mode_label: String,
    pub(in crate::app) only_switch_to_trusted_domains: bool,
    pub(in crate::app) trusted_domains_text: String,
    pub(in crate::app) trusted_domain_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiDesyncSection {
    pub(in crate::app) rewind_on_desync: bool,
    pub(in crate::app) fastforward_on_desync: bool,
    pub(in crate::app) slow_on_desync: bool,
    pub(in crate::app) dont_slow_down_with_me: bool,
    pub(in crate::app) rewind_threshold_seconds: Option<f64>,
    pub(in crate::app) fastforward_threshold_seconds: Option<f64>,
    pub(in crate::app) slowdown_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiStreamingSection {
    pub(in crate::app) quality_label: String,
    pub(in crate::app) custom_format: Option<String>,
    pub(in crate::app) buffer_target_seconds: f64,
    pub(in crate::app) read_ahead_seconds: f64,
    pub(in crate::app) memory_cache_mebibytes: u64,
    pub(in crate::app) disk_cache_enabled: bool,
    pub(in crate::app) recovery_policy_label: String,
    pub(in crate::app) maximum_catchup_rate: f64,
    pub(in crate::app) hard_seek_threshold_seconds: f64,
    pub(in crate::app) maximum_hard_seeks: u32,
    pub(in crate::app) stability_interval_seconds: f64,
    pub(in crate::app) retry_budget: u32,
    pub(in crate::app) recovery_cooldown_seconds: f64,
    pub(in crate::app) room_buffering_policy_label: String,
    pub(in crate::app) room_quorum_percent: f64,
    pub(in crate::app) room_maximum_pause_seconds: f64,
    pub(in crate::app) start_policy_label: String,
    pub(in crate::app) start_quorum_percent: f64,
    pub(in crate::app) start_timeout_seconds: f64,
    pub(in crate::app) start_timeout_action_label: String,
    pub(in crate::app) quality_downgrade_suggestions: bool,
    pub(in crate::app) effective_mpv_options: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiMediaSearchSection {
    pub(in crate::app) media_directories_text: String,
    pub(in crate::app) media_directory_count: usize,
    pub(in crate::app) folder_search_first_file_timeout_seconds: Option<f64>,
    pub(in crate::app) folder_search_timeout_seconds: Option<f64>,
    pub(in crate::app) folder_search_double_check_interval_seconds: Option<f64>,
    pub(in crate::app) folder_search_warning_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiChatSection {
    pub(in crate::app) chat_input_enabled: bool,
    pub(in crate::app) chat_output_enabled: bool,
    pub(in crate::app) chat_direct_input: bool,
    pub(in crate::app) chat_move_osd: bool,
    pub(in crate::app) chat_max_lines: Option<i64>,
    pub(in crate::app) chat_input_position_label: String,
    pub(in crate::app) chat_input_font_family: Option<String>,
    pub(in crate::app) chat_input_relative_font_size: Option<i64>,
    pub(in crate::app) chat_input_font_weight: Option<i64>,
    pub(in crate::app) chat_input_font_color: Option<String>,
    pub(in crate::app) chat_output_mode_label: String,
    pub(in crate::app) chat_output_font_family: Option<String>,
    pub(in crate::app) chat_output_relative_font_size: Option<i64>,
    pub(in crate::app) chat_output_font_weight: Option<i64>,
    pub(in crate::app) chat_top_margin: Option<i64>,
    pub(in crate::app) chat_left_margin: Option<i64>,
    pub(in crate::app) chat_bottom_margin: Option<i64>,
    pub(in crate::app) chat_osd_margin: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiOsdSection {
    pub(in crate::app) show_osd: bool,
    pub(in crate::app) show_duration_notification: bool,
    pub(in crate::app) show_same_room_osd: bool,
    pub(in crate::app) show_osd_warnings: bool,
    pub(in crate::app) show_slowdown_osd: bool,
    pub(in crate::app) show_noncontroller_osd: bool,
    pub(in crate::app) show_different_room_osd: bool,
    pub(in crate::app) show_contact_info: bool,
    pub(in crate::app) notification_timeout_seconds: Option<i64>,
    pub(in crate::app) alert_timeout_seconds: Option<i64>,
    pub(in crate::app) chat_timeout_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiSystemSection {
    pub(in crate::app) language_tag: String,
    pub(in crate::app) check_for_updates_automatically: bool,
    pub(in crate::app) update_channel_label: String,
    pub(in crate::app) autosave_joins_to_list: bool,
    pub(in crate::app) force_gui_prompt: bool,
    pub(in crate::app) compatibility_startup_entry_count: usize,
    pub(in crate::app) ignored_startup_exception_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiDialogControlKind {
    TextInput,
    TextArea,
    PasswordInput,
    Checkbox,
    Select,
    NumericInput,
    ReadOnly,
}

impl GuiDialogControlKind {
    #[cfg(test)]
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::TextInput => "text",
            Self::TextArea => "textarea",
            Self::PasswordInput => "password",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::NumericInput => "numeric",
            Self::ReadOnly => "readonly",
        }
    }

    pub(in crate::app) fn is_editable(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiDialogControl {
    pub(in crate::app) id: SettingId,
    pub(in crate::app) label: &'static str,
    pub(in crate::app) kind: GuiDialogControlKind,
    pub(in crate::app) value: String,
}

impl std::fmt::Debug for GuiDialogControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = if self.kind == GuiDialogControlKind::PasswordInput {
            sorotte_secret::REDACTED_SECRET
        } else {
            &self.value
        };
        formatter
            .debug_struct("GuiDialogControl")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("value", &value)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum GuiConfigurationTextValue {
    Plain(String),
    Secret(SecretValue),
}

impl GuiConfigurationTextValue {
    pub(in crate::app) fn for_control(
        kind: GuiDialogControlKind,
        value: impl Into<String>,
    ) -> Self {
        let value = value.into();
        if kind == GuiDialogControlKind::PasswordInput {
            Self::Secret(value.into())
        } else {
            Self::Plain(value)
        }
    }

    pub(in crate::app) fn expose_for_ui(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Secret(value) => value.expose_secret(),
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn as_str(&self) -> &str {
        self.expose_for_ui()
    }

    pub(in crate::app) fn expose_for_config_apply(&self) -> &str {
        self.expose_for_ui()
    }
}

impl std::fmt::Display for GuiConfigurationTextValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(value) => formatter.write_str(value),
            Self::Secret(_) => formatter.write_str(sorotte_secret::REDACTED_SECRET),
        }
    }
}

impl From<String> for GuiConfigurationTextValue {
    fn from(value: String) -> Self {
        Self::Plain(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiDialogSection {
    pub(in crate::app) title: &'static str,
    pub(in crate::app) controls: Vec<GuiDialogControl>,
}

impl GuiDialogSection {
    pub(in crate::app) fn control_mut(&mut self, id: SettingId) -> Option<&mut GuiDialogControl> {
        self.controls.iter_mut().find(|control| control.id == id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct FirstRunConfigurationDialogState {
    pub(in crate::app) launch_mode: GuiLaunchMode,
    pub(in crate::app) connection: GuiConnectionSettingsSection,
    pub(in crate::app) readiness: GuiReadinessSection,
    pub(in crate::app) privacy: GuiPrivacySection,
    pub(in crate::app) desync: GuiDesyncSection,
    pub(in crate::app) streaming: GuiStreamingSection,
    pub(in crate::app) media_search: GuiMediaSearchSection,
    pub(in crate::app) chat: GuiChatSection,
    pub(in crate::app) osd: GuiOsdSection,
    pub(in crate::app) system: GuiSystemSection,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct FirstRunConfigurationDialogDraft {
    pub(in crate::app) launch_mode: GuiLaunchMode,
    pub(in crate::app) compatibility_startup_entry_count: usize,
    pub(in crate::app) ignored_startup_exception_count: usize,
    pub(in crate::app) sections: Vec<GuiDialogSection>,
    pub(in crate::app) settings: StoredClientSettingsMvp,
    pub(in crate::app) server_password: SecretDraft,
}
