use std::{collections::BTreeMap, sync::LazyLock};

use sorotte_client_core::{PrivacyMode, UnpauseActionMode};
use sorotte_secret::SecretValue;

use crate::language::normalized_runtime_language_tag;
use crate::stored_settings::{
    AutoplayThresholdOverride, StoredClientSettings, autoplay_threshold_override_setting_value,
    parse_autoplay_min_users_override, parse_unpause_action_mode, privacy_mode_syncplay_name,
    unpause_action_mode_syncplay_name,
};
use crate::syncplay_ini_values::{
    format_serialized_per_player_arguments_map, format_serialized_public_servers_list,
    format_serialized_string_list, parse_serialized_per_player_arguments_map,
    parse_serialized_public_servers_list, parse_serialized_string_list,
};

use super::helpers::{
    format_ini_bool, format_ini_non_negative_f64, parse_ini_bool, parse_ini_i64,
    parse_ini_non_negative_f64, parse_ini_port,
};

/// The on-disk spelling and codec belong together. Runtime validation and GUI
/// draft parsing have different rules and remain at their respective boundaries.
pub(super) struct IniField {
    pub section: &'static str,
    pub key: &'static str,
    pub read: fn(&mut StoredClientSettings, &str),
    pub write: fn(&StoredClientSettings) -> Option<String>,
}

impl IniField {
    fn new(
        section: &'static str,
        key: &'static str,
        read: fn(&mut StoredClientSettings, &str),
        write: fn(&StoredClientSettings) -> Option<String>,
    ) -> Self {
        Self {
            section,
            key,
            read,
            write,
        }
    }
}

struct Codec<T> {
    parse: fn(&str) -> Option<T>,
    format: fn(&T) -> Option<String>,
}

impl<T> Codec<T> {
    fn read(&self, field: &mut Option<T>, text: &str) {
        if let Some(value) = (self.parse)(text) {
            *field = Some(value);
        }
    }

    fn write(&self, field: Option<&T>) -> Option<String> {
        field.and_then(self.format)
    }
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

fn read_language(settings: &mut StoredClientSettings, text: &str) {
    // A nonempty unsupported language clears an earlier value, including an
    // earlier duplicate. Other invalid INI values leave it unchanged.
    if !text.is_empty() {
        settings.language = normalized_runtime_language_tag(text).map(ToOwned::to_owned);
    }
}

const TEXT: Codec<String> = Codec {
    parse: non_empty,
    format: |value| Some(value.clone()),
};
const LOWERCASE_TEXT: Codec<String> = Codec {
    parse: |text: &str| non_empty(text).map(|value| value.to_ascii_lowercase()),
    format: |value| Some(value.clone()),
};
const SECRET: Codec<SecretValue> = Codec {
    parse: |text| non_empty(text).map(SecretValue::from),
    format: |value| Some(value.expose_secret().to_owned()),
};
const BOOL: Codec<bool> = Codec {
    parse: parse_ini_bool,
    format: |value| Some(format_ini_bool(*value).to_owned()),
};
const PORT: Codec<u16> = Codec {
    parse: parse_ini_port,
    format: |value| Some(value.to_string()),
};
const SIGNED: Codec<i64> = Codec {
    parse: parse_ini_i64,
    format: |value| Some(value.to_string()),
};
const U32: Codec<u32> = Codec {
    parse: |text: &str| text.parse().ok(),
    format: |value| Some(value.to_string()),
};
const U64: Codec<u64> = Codec {
    parse: |text: &str| text.parse().ok(),
    format: |value| Some(value.to_string()),
};
const NON_NEGATIVE: Codec<f64> = Codec {
    parse: parse_ini_non_negative_f64,
    format: |value| format_ini_non_negative_f64(*value),
};
const STRING_LIST: Codec<Vec<String>> = Codec {
    parse: parse_serialized_string_list,
    format: |value| Some(format_serialized_string_list(value)),
};
const MEDIA_DIRECTORIES: Codec<Vec<String>> = Codec {
    parse: |text| {
        parse_serialized_string_list(text).map(|directories| {
            directories
                .into_iter()
                .filter_map(|directory| {
                    let directory = directory.trim();
                    (!directory.is_empty()).then(|| directory.to_owned())
                })
                .collect()
        })
    },
    format: |value| Some(format_serialized_string_list(value)),
};
const PUBLIC_SERVERS: Codec<Vec<(String, String)>> = Codec {
    parse: parse_serialized_public_servers_list,
    format: |value| Some(format_serialized_public_servers_list(value)),
};
const PLAYER_ARGUMENTS: Codec<BTreeMap<String, Vec<String>>> = Codec {
    parse: parse_serialized_per_player_arguments_map,
    format: |value| Some(format_serialized_per_player_arguments_map(value)),
};
const PRIVACY: Codec<PrivacyMode> = Codec {
    parse: PrivacyMode::from_syncplay_name,
    format: |value| Some(privacy_mode_syncplay_name(*value).to_owned()),
};
const UNPAUSE: Codec<UnpauseActionMode> = Codec {
    parse: parse_unpause_action_mode,
    format: |value| Some(unpause_action_mode_syncplay_name(value.clone()).to_owned()),
};
const AUTOPLAY_THRESHOLD: Codec<AutoplayThresholdOverride> = Codec {
    parse: parse_autoplay_min_users_override,
    format: |value| Some(autoplay_threshold_override_setting_value(value)),
};

pub(super) static INI_FIELDS: LazyLock<[IniField; 107]> = LazyLock::new(ini_fields);

// Initialize once in the established insertion order. Ordinary typed bindings
// keep the field codecs visible to both the compiler and coverage tools.
fn ini_fields() -> [IniField; 107] {
    [
        IniField::new(
            "server_data",
            "host",
            |settings, text| TEXT.read(&mut settings.host, text),
            |settings| TEXT.write(settings.host.as_ref()),
        ),
        IniField::new(
            "server_data",
            "port",
            |settings, text| PORT.read(&mut settings.port, text),
            |settings| PORT.write(settings.port.as_ref()),
        ),
        IniField::new(
            "server_data",
            "password",
            |settings, text| SECRET.read(&mut settings.server_password, text),
            |settings| SECRET.write(settings.server_password.as_ref()),
        ),
        IniField::new(
            "server_data",
            "tlsPolicy",
            |settings, text| TEXT.read(&mut settings.tls_policy, text),
            |settings| TEXT.write(settings.tls_policy.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "name",
            |settings, text| TEXT.read(&mut settings.username, text),
            |settings| TEXT.write(settings.username.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "room",
            |settings, text| TEXT.read(&mut settings.room, text),
            |settings| TEXT.write(settings.room.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "roomList",
            |settings, text| STRING_LIST.read(&mut settings.room_list, text),
            |settings| STRING_LIST.write(settings.room_list.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "playerPath",
            |settings, text| TEXT.read(&mut settings.player_path, text),
            |settings| TEXT.write(settings.player_path.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "perPlayerArguments",
            |settings, text| PLAYER_ARGUMENTS.read(&mut settings.per_player_arguments, text),
            |settings| PLAYER_ARGUMENTS.write(settings.per_player_arguments.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingQualityPreset",
            |settings, text| LOWERCASE_TEXT.read(&mut settings.streaming_quality_preset, text),
            |settings| LOWERCASE_TEXT.write(settings.streaming_quality_preset.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingCustomFormat",
            |settings, text| TEXT.read(&mut settings.streaming_custom_format, text),
            |settings| TEXT.write(settings.streaming_custom_format.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRecoveryPolicy",
            |settings, text| LOWERCASE_TEXT.read(&mut settings.streaming_recovery_policy, text),
            |settings| LOWERCASE_TEXT.write(settings.streaming_recovery_policy.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRoomBufferingPolicy",
            |settings, text| {
                LOWERCASE_TEXT.read(&mut settings.streaming_room_buffering_policy, text)
            },
            |settings| LOWERCASE_TEXT.write(settings.streaming_room_buffering_policy.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingStartPolicy",
            |settings, text| LOWERCASE_TEXT.read(&mut settings.streaming_start_policy, text),
            |settings| LOWERCASE_TEXT.write(settings.streaming_start_policy.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingStartTimeoutAction",
            |settings, text| {
                LOWERCASE_TEXT.read(&mut settings.streaming_start_timeout_action, text)
            },
            |settings| LOWERCASE_TEXT.write(settings.streaming_start_timeout_action.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingBufferTarget",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_buffer_target_seconds, text),
            |settings| NON_NEGATIVE.write(settings.streaming_buffer_target_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingReadAhead",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_read_ahead_seconds, text),
            |settings| NON_NEGATIVE.write(settings.streaming_read_ahead_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingMaxCatchupRate",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_max_catchup_rate, text),
            |settings| NON_NEGATIVE.write(settings.streaming_max_catchup_rate.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingHardSeekThreshold",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.streaming_hard_seek_threshold_seconds, text)
            },
            |settings| NON_NEGATIVE.write(settings.streaming_hard_seek_threshold_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingStabilityInterval",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.streaming_stability_interval_seconds, text)
            },
            |settings| NON_NEGATIVE.write(settings.streaming_stability_interval_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRecoveryCooldown",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.streaming_recovery_cooldown_seconds, text)
            },
            |settings| NON_NEGATIVE.write(settings.streaming_recovery_cooldown_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRoomQuorumPercent",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_room_quorum_percent, text),
            |settings| NON_NEGATIVE.write(settings.streaming_room_quorum_percent.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRoomMaxPause",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.streaming_room_max_pause_seconds, text)
            },
            |settings| NON_NEGATIVE.write(settings.streaming_room_max_pause_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingStartQuorumPercent",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_start_quorum_percent, text),
            |settings| NON_NEGATIVE.write(settings.streaming_start_quorum_percent.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingStartTimeout",
            |settings, text| NON_NEGATIVE.read(&mut settings.streaming_start_timeout_seconds, text),
            |settings| NON_NEGATIVE.write(settings.streaming_start_timeout_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingMemoryCacheMiB",
            |settings, text| U64.read(&mut settings.streaming_memory_cache_mebibytes, text),
            |settings| U64.write(settings.streaming_memory_cache_mebibytes.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingMaxHardSeeks",
            |settings, text| U32.read(&mut settings.streaming_max_hard_seeks_per_episode, text),
            |settings| U32.write(settings.streaming_max_hard_seeks_per_episode.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingRecoveryRetryBudget",
            |settings, text| U32.read(&mut settings.streaming_recovery_retry_budget, text),
            |settings| U32.write(settings.streaming_recovery_retry_budget.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingDiskCacheEnabled",
            |settings, text| BOOL.read(&mut settings.streaming_disk_cache_enabled, text),
            |settings| BOOL.write(settings.streaming_disk_cache_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "streamingQualityDowngradeSuggestions",
            |settings, text| BOOL.read(&mut settings.streaming_quality_downgrade_suggestions, text),
            |settings| BOOL.write(settings.streaming_quality_downgrade_suggestions.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaSearchDirectories",
            |settings, text| MEDIA_DIRECTORIES.read(&mut settings.media_search_directories, text),
            |settings| MEDIA_DIRECTORIES.write(settings.media_search_directories.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "publicServers",
            |settings, text| PUBLIC_SERVERS.read(&mut settings.public_servers, text),
            |settings| PUBLIC_SERVERS.write(settings.public_servers.as_ref()),
        ),
        IniField::new(
            "plugins",
            "streamSupportEnabled",
            |settings, text| BOOL.read(&mut settings.stream_support_plugin_enabled, text),
            |settings| BOOL.write(settings.stream_support_plugin_enabled.as_ref()),
        ),
        IniField::new(
            "plugins",
            "mediaMatchingEnabled",
            |settings, text| BOOL.read(&mut settings.media_matching_plugin_enabled, text),
            |settings| BOOL.write(settings.media_matching_plugin_enabled.as_ref()),
        ),
        IniField::new(
            "plugins",
            "plexEnabled",
            |settings, text| BOOL.read(&mut settings.plex_plugin_enabled, text),
            |settings| BOOL.write(settings.plex_plugin_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaMatchFingerprintingEnabled",
            |settings, text| BOOL.read(&mut settings.media_match_fingerprinting_enabled, text),
            |settings| BOOL.write(settings.media_match_fingerprinting_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaMatchBackgroundWarmupEnabled",
            |settings, text| BOOL.read(&mut settings.media_match_background_warmup_enabled, text),
            |settings| BOOL.write(settings.media_match_background_warmup_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaMatchWireSharingEnabled",
            |settings, text| BOOL.read(&mut settings.media_match_wire_sharing_enabled, text),
            |settings| BOOL.write(settings.media_match_wire_sharing_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaMatchRuntimeToleranceEnabled",
            |settings, text| BOOL.read(&mut settings.media_match_runtime_tolerance_enabled, text),
            |settings| BOOL.write(settings.media_match_runtime_tolerance_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "mediaMatchAutoplayPolicy",
            |settings, text| TEXT.read(&mut settings.media_match_autoplay_policy, text),
            |settings| TEXT.write(settings.media_match_autoplay_policy.as_ref()),
        ),
        IniField::new(
            "plex",
            "syncEnabled",
            |settings, text| BOOL.read(&mut settings.plex_sync_enabled, text),
            |settings| BOOL.write(settings.plex_sync_enabled.as_ref()),
        ),
        IniField::new(
            "plex",
            "streamingEnabled",
            |settings, text| BOOL.read(&mut settings.plex_streaming_enabled, text),
            |settings| BOOL.write(settings.plex_streaming_enabled.as_ref()),
        ),
        IniField::new(
            "plex",
            "userToken",
            |settings, text| SECRET.read(&mut settings.plex_user_token, text),
            |settings| SECRET.write(settings.plex_user_token.as_ref()),
        ),
        IniField::new(
            "plex",
            "selectedServerId",
            |settings, text| TEXT.read(&mut settings.plex_selected_server_id, text),
            |settings| TEXT.write(settings.plex_selected_server_id.as_ref()),
        ),
        IniField::new(
            "plex",
            "selectedServerUrl",
            |settings, text| TEXT.read(&mut settings.plex_selected_server_url, text),
            |settings| TEXT.write(settings.plex_selected_server_url.as_ref()),
        ),
        IniField::new(
            "plex",
            "selectedServerToken",
            |settings, text| SECRET.read(&mut settings.plex_selected_server_token, text),
            |settings| SECRET.write(settings.plex_selected_server_token.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "folderSearchFirstFileTimeout",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.folder_search_first_file_timeout_seconds, text)
            },
            |settings| {
                NON_NEGATIVE.write(settings.folder_search_first_file_timeout_seconds.as_ref())
            },
        ),
        IniField::new(
            "client_settings",
            "folderSearchTimeout",
            |settings, text| NON_NEGATIVE.read(&mut settings.folder_search_timeout_seconds, text),
            |settings| NON_NEGATIVE.write(settings.folder_search_timeout_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "folderSearchDoubleCheckInterval",
            |settings, text| {
                NON_NEGATIVE.read(
                    &mut settings.folder_search_double_check_interval_seconds,
                    text,
                )
            },
            |settings| {
                NON_NEGATIVE.write(
                    settings
                        .folder_search_double_check_interval_seconds
                        .as_ref(),
                )
            },
        ),
        IniField::new(
            "client_settings",
            "folderSearchWarningThreshold",
            |settings, text| {
                NON_NEGATIVE.read(&mut settings.folder_search_warning_threshold_seconds, text)
            },
            |settings| {
                NON_NEGATIVE.write(settings.folder_search_warning_threshold_seconds.as_ref())
            },
        ),
        IniField::new(
            "client_settings",
            "forceGuiPrompt",
            |settings, text| BOOL.read(&mut settings.force_gui_prompt, text),
            |settings| BOOL.write(settings.force_gui_prompt.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "autoplayInitialState",
            |settings, text| BOOL.read(&mut settings.autoplay_initial_state, text),
            |settings| BOOL.write(settings.autoplay_initial_state.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "autoplayRequireSameFilenames",
            |settings, text| BOOL.read(&mut settings.autoplay_require_same_filenames, text),
            |settings| BOOL.write(settings.autoplay_require_same_filenames.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "readyAtStart",
            |settings, text| BOOL.read(&mut settings.ready_at_start, text),
            |settings| BOOL.write(settings.ready_at_start.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "sharedPlaylistEnabled",
            |settings, text| BOOL.read(&mut settings.shared_playlist_enabled, text),
            |settings| BOOL.write(settings.shared_playlist_enabled.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "pauseOnLeave",
            |settings, text| BOOL.read(&mut settings.pause_on_leave, text),
            |settings| BOOL.write(settings.pause_on_leave.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "loopAtEndOfPlaylist",
            |settings, text| BOOL.read(&mut settings.loop_at_end_of_playlist, text),
            |settings| BOOL.write(settings.loop_at_end_of_playlist.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "loopSingleFiles",
            |settings, text| BOOL.read(&mut settings.loop_single_files, text),
            |settings| BOOL.write(settings.loop_single_files.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "onlySwitchToTrustedDomains",
            |settings, text| BOOL.read(&mut settings.only_switch_to_trusted_domains, text),
            |settings| BOOL.write(settings.only_switch_to_trusted_domains.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "trustedDomains",
            |settings, text| STRING_LIST.read(&mut settings.trusted_domains, text),
            |settings| STRING_LIST.write(settings.trusted_domains.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "rewindOnDesync",
            |settings, text| BOOL.read(&mut settings.rewind_on_desync, text),
            |settings| BOOL.write(settings.rewind_on_desync.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "fastforwardOnDesync",
            |settings, text| BOOL.read(&mut settings.fastforward_on_desync, text),
            |settings| BOOL.write(settings.fastforward_on_desync.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "slowOnDesync",
            |settings, text| BOOL.read(&mut settings.slow_on_desync, text),
            |settings| BOOL.write(settings.slow_on_desync.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "dontSlowDownWithMe",
            |settings, text| BOOL.read(&mut settings.dont_slow_down_with_me, text),
            |settings| BOOL.write(settings.dont_slow_down_with_me.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "rewindThreshold",
            |settings, text| NON_NEGATIVE.read(&mut settings.rewind_threshold_seconds, text),
            |settings| NON_NEGATIVE.write(settings.rewind_threshold_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "fastforwardThreshold",
            |settings, text| NON_NEGATIVE.read(&mut settings.fastforward_threshold_seconds, text),
            |settings| NON_NEGATIVE.write(settings.fastforward_threshold_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "slowdownThreshold",
            |settings, text| NON_NEGATIVE.read(&mut settings.slowdown_threshold_seconds, text),
            |settings| NON_NEGATIVE.write(settings.slowdown_threshold_seconds.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "unpauseAction",
            |settings, text| UNPAUSE.read(&mut settings.unpause_action, text),
            |settings| UNPAUSE.write(settings.unpause_action.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "autoplayMinUsers",
            |settings, text| AUTOPLAY_THRESHOLD.read(&mut settings.autoplay_min_users, text),
            |settings| AUTOPLAY_THRESHOLD.write(settings.autoplay_min_users.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "filenamePrivacyMode",
            |settings, text| PRIVACY.read(&mut settings.filename_privacy_mode, text),
            |settings| PRIVACY.write(settings.filename_privacy_mode.as_ref()),
        ),
        IniField::new(
            "client_settings",
            "filesizePrivacyMode",
            |settings, text| PRIVACY.read(&mut settings.filesize_privacy_mode, text),
            |settings| PRIVACY.write(settings.filesize_privacy_mode.as_ref()),
        ),
        IniField::new("general", "language", read_language, |settings| {
            TEXT.write(settings.language.as_ref())
        }),
        IniField::new(
            "general",
            "checkForUpdatesAutomatically",
            |settings, text| BOOL.read(&mut settings.check_for_updates_automatically, text),
            |settings| BOOL.write(settings.check_for_updates_automatically.as_ref()),
        ),
        IniField::new(
            "general",
            "updateChannel",
            |settings, text| LOWERCASE_TEXT.read(&mut settings.update_channel, text),
            |settings| LOWERCASE_TEXT.write(settings.update_channel.as_ref()),
        ),
        IniField::new(
            "general",
            "lastCheckedForUpdates",
            |settings, text| TEXT.read(&mut settings.last_checked_for_updates, text),
            |settings| TEXT.write(settings.last_checked_for_updates.as_ref()),
        ),
        IniField::new(
            "gui",
            "autosaveJoinsToList",
            |settings, text| BOOL.read(&mut settings.autosave_joins_to_list, text),
            |settings| BOOL.write(settings.autosave_joins_to_list.as_ref()),
        ),
        IniField::new(
            "gui",
            "showOSD",
            |settings, text| BOOL.read(&mut settings.show_osd, text),
            |settings| BOOL.write(settings.show_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputEnabled",
            |settings, text| BOOL.read(&mut settings.chat_input_enabled, text),
            |settings| BOOL.write(settings.chat_input_enabled.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputFontUnderline",
            |settings, text| BOOL.read(&mut settings.chat_input_font_underline, text),
            |settings| BOOL.write(settings.chat_input_font_underline.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputFontFamily",
            |settings, text| TEXT.read(&mut settings.chat_input_font_family, text),
            |settings| TEXT.write(settings.chat_input_font_family.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputRelativeFontSize",
            |settings, text| SIGNED.read(&mut settings.chat_input_relative_font_size, text),
            |settings| SIGNED.write(settings.chat_input_relative_font_size.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputFontWeight",
            |settings, text| SIGNED.read(&mut settings.chat_input_font_weight, text),
            |settings| SIGNED.write(settings.chat_input_font_weight.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputFontColor",
            |settings, text| TEXT.read(&mut settings.chat_input_font_color, text),
            |settings| TEXT.write(settings.chat_input_font_color.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatInputPosition",
            |settings, text| TEXT.read(&mut settings.chat_input_position, text),
            |settings| TEXT.write(settings.chat_input_position.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatDirectInput",
            |settings, text| BOOL.read(&mut settings.chat_direct_input, text),
            |settings| BOOL.write(settings.chat_direct_input.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputEnabled",
            |settings, text| BOOL.read(&mut settings.chat_output_enabled, text),
            |settings| BOOL.write(settings.chat_output_enabled.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputFontUnderline",
            |settings, text| BOOL.read(&mut settings.chat_output_font_underline, text),
            |settings| BOOL.write(settings.chat_output_font_underline.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputFontFamily",
            |settings, text| TEXT.read(&mut settings.chat_output_font_family, text),
            |settings| TEXT.write(settings.chat_output_font_family.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputRelativeFontSize",
            |settings, text| SIGNED.read(&mut settings.chat_output_relative_font_size, text),
            |settings| SIGNED.write(settings.chat_output_relative_font_size.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputFontWeight",
            |settings, text| SIGNED.read(&mut settings.chat_output_font_weight, text),
            |settings| SIGNED.write(settings.chat_output_font_weight.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOutputMode",
            |settings, text| TEXT.read(&mut settings.chat_output_mode, text),
            |settings| TEXT.write(settings.chat_output_mode.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatMoveOSD",
            |settings, text| BOOL.read(&mut settings.chat_move_osd, text),
            |settings| BOOL.write(settings.chat_move_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatMaxLines",
            |settings, text| SIGNED.read(&mut settings.chat_max_lines, text),
            |settings| SIGNED.write(settings.chat_max_lines.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatTopMargin",
            |settings, text| SIGNED.read(&mut settings.chat_top_margin, text),
            |settings| SIGNED.write(settings.chat_top_margin.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatLeftMargin",
            |settings, text| SIGNED.read(&mut settings.chat_left_margin, text),
            |settings| SIGNED.write(settings.chat_left_margin.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatBottomMargin",
            |settings, text| SIGNED.read(&mut settings.chat_bottom_margin, text),
            |settings| SIGNED.write(settings.chat_bottom_margin.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatOSDMargin",
            |settings, text| SIGNED.read(&mut settings.chat_osd_margin, text),
            |settings| SIGNED.write(settings.chat_osd_margin.as_ref()),
        ),
        IniField::new(
            "gui",
            "notificationTimeout",
            |settings, text| SIGNED.read(&mut settings.notification_timeout_seconds, text),
            |settings| SIGNED.write(settings.notification_timeout_seconds.as_ref()),
        ),
        IniField::new(
            "gui",
            "alertTimeout",
            |settings, text| SIGNED.read(&mut settings.alert_timeout_seconds, text),
            |settings| SIGNED.write(settings.alert_timeout_seconds.as_ref()),
        ),
        IniField::new(
            "gui",
            "chatTimeout",
            |settings, text| SIGNED.read(&mut settings.chat_timeout_seconds, text),
            |settings| SIGNED.write(settings.chat_timeout_seconds.as_ref()),
        ),
        IniField::new(
            "gui",
            "showDurationNotification",
            |settings, text| BOOL.read(&mut settings.show_duration_notification, text),
            |settings| BOOL.write(settings.show_duration_notification.as_ref()),
        ),
        IniField::new(
            "gui",
            "showSameRoomOSD",
            |settings, text| BOOL.read(&mut settings.show_same_room_osd, text),
            |settings| BOOL.write(settings.show_same_room_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "showOSDWarnings",
            |settings, text| BOOL.read(&mut settings.show_osd_warnings, text),
            |settings| BOOL.write(settings.show_osd_warnings.as_ref()),
        ),
        IniField::new(
            "gui",
            "showSlowdownOSD",
            |settings, text| BOOL.read(&mut settings.show_slowdown_osd, text),
            |settings| BOOL.write(settings.show_slowdown_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "showNonControllerOSD",
            |settings, text| BOOL.read(&mut settings.show_noncontroller_osd, text),
            |settings| BOOL.write(settings.show_noncontroller_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "showDifferentRoomOSD",
            |settings, text| BOOL.read(&mut settings.show_different_room_osd, text),
            |settings| BOOL.write(settings.show_different_room_osd.as_ref()),
        ),
        IniField::new(
            "gui",
            "showContactInfo",
            |settings, text| BOOL.read(&mut settings.show_contact_info, text),
            |settings| BOOL.write(settings.show_contact_info.as_ref()),
        ),
    ]
}
