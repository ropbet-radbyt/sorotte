use std::collections::BTreeMap;

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

struct Codec<T> {
    read: fn(&mut Option<T>, &str),
    write: fn(&T) -> Option<String>,
}

macro_rules! codec {
    ($parse:expr, $format:expr) => {
        Codec {
            read: |field, text| {
                if let Some(value) = $parse(text) {
                    *field = Some(value);
                }
            },
            write: $format,
        }
    };
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

const TEXT: Codec<String> = codec!(non_empty, |value| Some(value.clone()));
const LOWERCASE_TEXT: Codec<String> = codec!(
    |text: &str| non_empty(text).map(|value| value.to_ascii_lowercase()),
    |value| Some(value.clone())
);
const LANGUAGE: Codec<String> = Codec {
    read: |field, text| {
        // A nonempty unsupported language clears an earlier value, including
        // an earlier duplicate. Other invalid INI values leave it unchanged.
        if !text.is_empty() {
            *field = normalized_runtime_language_tag(text).map(ToOwned::to_owned);
        }
    },
    write: |value| Some(value.clone()),
};
const SECRET: Codec<SecretValue> =
    codec!(|text| non_empty(text).map(SecretValue::from), |value| Some(
        value.expose_secret().to_owned()
    ));
const BOOL: Codec<bool> = codec!(parse_ini_bool, |value| Some(
    format_ini_bool(*value).to_owned()
));
const PORT: Codec<u16> = codec!(parse_ini_port, |value| Some(value.to_string()));
const SIGNED: Codec<i64> = codec!(parse_ini_i64, |value| Some(value.to_string()));
const U32: Codec<u32> = codec!(|text: &str| text.parse().ok(), |value| Some(
    value.to_string()
));
const U64: Codec<u64> = codec!(|text: &str| text.parse().ok(), |value| Some(
    value.to_string()
));
const NON_NEGATIVE: Codec<f64> = codec!(parse_ini_non_negative_f64, |value| {
    format_ini_non_negative_f64(*value)
});
const STRING_LIST: Codec<Vec<String>> = codec!(parse_serialized_string_list, |value| Some(
    format_serialized_string_list(value)
));
const MEDIA_DIRECTORIES: Codec<Vec<String>> = codec!(
    |text| parse_serialized_string_list(text).map(|directories| {
        directories
            .into_iter()
            .filter_map(|directory| {
                let directory = directory.trim();
                (!directory.is_empty()).then(|| directory.to_owned())
            })
            .collect()
    }),
    |value| Some(format_serialized_string_list(value))
);
const PUBLIC_SERVERS: Codec<Vec<(String, String)>> = codec!(
    parse_serialized_public_servers_list,
    |value| Some(format_serialized_public_servers_list(value))
);
const PLAYER_ARGUMENTS: Codec<BTreeMap<String, Vec<String>>> = codec!(
    parse_serialized_per_player_arguments_map,
    |value| Some(format_serialized_per_player_arguments_map(value))
);
const PRIVACY: Codec<PrivacyMode> = codec!(PrivacyMode::from_syncplay_name, |value| Some(
    privacy_mode_syncplay_name(*value).to_owned()
));
const UNPAUSE: Codec<UnpauseActionMode> = codec!(parse_unpause_action_mode, |value| Some(
    unpause_action_mode_syncplay_name(value.clone()).to_owned()
));
const AUTOPLAY_THRESHOLD: Codec<AutoplayThresholdOverride> = codec!(
    parse_autoplay_min_users_override,
    |value| Some(autoplay_threshold_override_setting_value(value))
);

macro_rules! ini_fields {
    ($($section:literal { $($key:literal => $field:ident: $codec:ident;)* })*) => {
        pub(super) const INI_FIELDS: &[IniField] = &[
            $($(IniField {
                section: $section,
                key: $key,
                read: |settings, text| ($codec.read)(&mut settings.$field, text),
                write: |settings| settings.$field.as_ref().and_then($codec.write),
            },)*)*
        ];
    };
}

// Keep the established insertion order when creating or extending an INI file.
ini_fields! {
    "server_data" {
        "host" => host: TEXT;
        "port" => port: PORT;
        "password" => server_password: SECRET;
        "tlsPolicy" => tls_policy: TEXT;
    }
    "client_settings" {
        "name" => username: TEXT;
        "room" => room: TEXT;
        "roomList" => room_list: STRING_LIST;
        "playerPath" => player_path: TEXT;
        "perPlayerArguments" => per_player_arguments: PLAYER_ARGUMENTS;
        "streamingQualityPreset" => streaming_quality_preset: LOWERCASE_TEXT;
        "streamingCustomFormat" => streaming_custom_format: TEXT;
        "streamingRecoveryPolicy" => streaming_recovery_policy: LOWERCASE_TEXT;
        "streamingRoomBufferingPolicy" => streaming_room_buffering_policy: LOWERCASE_TEXT;
        "streamingStartPolicy" => streaming_start_policy: LOWERCASE_TEXT;
        "streamingStartTimeoutAction" => streaming_start_timeout_action: LOWERCASE_TEXT;
        "streamingBufferTarget" => streaming_buffer_target_seconds: NON_NEGATIVE;
        "streamingReadAhead" => streaming_read_ahead_seconds: NON_NEGATIVE;
        "streamingMaxCatchupRate" => streaming_max_catchup_rate: NON_NEGATIVE;
        "streamingHardSeekThreshold" => streaming_hard_seek_threshold_seconds: NON_NEGATIVE;
        "streamingStabilityInterval" => streaming_stability_interval_seconds: NON_NEGATIVE;
        "streamingRecoveryCooldown" => streaming_recovery_cooldown_seconds: NON_NEGATIVE;
        "streamingRoomQuorumPercent" => streaming_room_quorum_percent: NON_NEGATIVE;
        "streamingRoomMaxPause" => streaming_room_max_pause_seconds: NON_NEGATIVE;
        "streamingStartQuorumPercent" => streaming_start_quorum_percent: NON_NEGATIVE;
        "streamingStartTimeout" => streaming_start_timeout_seconds: NON_NEGATIVE;
        "streamingMemoryCacheMiB" => streaming_memory_cache_mebibytes: U64;
        "streamingMaxHardSeeks" => streaming_max_hard_seeks_per_episode: U32;
        "streamingRecoveryRetryBudget" => streaming_recovery_retry_budget: U32;
        "streamingDiskCacheEnabled" => streaming_disk_cache_enabled: BOOL;
        "streamingQualityDowngradeSuggestions" => streaming_quality_downgrade_suggestions: BOOL;
        "mediaSearchDirectories" => media_search_directories: MEDIA_DIRECTORIES;
        "publicServers" => public_servers: PUBLIC_SERVERS;
    }
    "plugins" {
        "streamSupportEnabled" => stream_support_plugin_enabled: BOOL;
        "mediaMatchingEnabled" => media_matching_plugin_enabled: BOOL;
        "plexEnabled" => plex_plugin_enabled: BOOL;
    }
    "client_settings" {
        "mediaMatchFingerprintingEnabled" => media_match_fingerprinting_enabled: BOOL;
        "mediaMatchBackgroundWarmupEnabled" => media_match_background_warmup_enabled: BOOL;
        "mediaMatchWireSharingEnabled" => media_match_wire_sharing_enabled: BOOL;
        "mediaMatchRuntimeToleranceEnabled" => media_match_runtime_tolerance_enabled: BOOL;
        "mediaMatchAutoplayPolicy" => media_match_autoplay_policy: TEXT;
    }
    "plex" {
        "syncEnabled" => plex_sync_enabled: BOOL;
        "streamingEnabled" => plex_streaming_enabled: BOOL;
        "userToken" => plex_user_token: SECRET;
        "selectedServerId" => plex_selected_server_id: TEXT;
        "selectedServerUrl" => plex_selected_server_url: TEXT;
        "selectedServerToken" => plex_selected_server_token: SECRET;
    }
    "client_settings" {
        "folderSearchFirstFileTimeout" => folder_search_first_file_timeout_seconds: NON_NEGATIVE;
        "folderSearchTimeout" => folder_search_timeout_seconds: NON_NEGATIVE;
        "folderSearchDoubleCheckInterval" => folder_search_double_check_interval_seconds: NON_NEGATIVE;
        "folderSearchWarningThreshold" => folder_search_warning_threshold_seconds: NON_NEGATIVE;
        "forceGuiPrompt" => force_gui_prompt: BOOL;
        "autoplayInitialState" => autoplay_initial_state: BOOL;
        "autoplayRequireSameFilenames" => autoplay_require_same_filenames: BOOL;
        "readyAtStart" => ready_at_start: BOOL;
        "sharedPlaylistEnabled" => shared_playlist_enabled: BOOL;
        "pauseOnLeave" => pause_on_leave: BOOL;
        "loopAtEndOfPlaylist" => loop_at_end_of_playlist: BOOL;
        "loopSingleFiles" => loop_single_files: BOOL;
        "onlySwitchToTrustedDomains" => only_switch_to_trusted_domains: BOOL;
        "trustedDomains" => trusted_domains: STRING_LIST;
        "rewindOnDesync" => rewind_on_desync: BOOL;
        "fastforwardOnDesync" => fastforward_on_desync: BOOL;
        "slowOnDesync" => slow_on_desync: BOOL;
        "dontSlowDownWithMe" => dont_slow_down_with_me: BOOL;
        "rewindThreshold" => rewind_threshold_seconds: NON_NEGATIVE;
        "fastforwardThreshold" => fastforward_threshold_seconds: NON_NEGATIVE;
        "slowdownThreshold" => slowdown_threshold_seconds: NON_NEGATIVE;
        "unpauseAction" => unpause_action: UNPAUSE;
        "autoplayMinUsers" => autoplay_min_users: AUTOPLAY_THRESHOLD;
        "filenamePrivacyMode" => filename_privacy_mode: PRIVACY;
        "filesizePrivacyMode" => filesize_privacy_mode: PRIVACY;
    }
    "general" {
        "language" => language: LANGUAGE;
        "checkForUpdatesAutomatically" => check_for_updates_automatically: BOOL;
        "updateChannel" => update_channel: LOWERCASE_TEXT;
        "lastCheckedForUpdates" => last_checked_for_updates: TEXT;
    }
    "gui" {
        "autosaveJoinsToList" => autosave_joins_to_list: BOOL;
        "showOSD" => show_osd: BOOL;
        "chatInputEnabled" => chat_input_enabled: BOOL;
        "chatInputFontUnderline" => chat_input_font_underline: BOOL;
        "chatInputFontFamily" => chat_input_font_family: TEXT;
        "chatInputRelativeFontSize" => chat_input_relative_font_size: SIGNED;
        "chatInputFontWeight" => chat_input_font_weight: SIGNED;
        "chatInputFontColor" => chat_input_font_color: TEXT;
        "chatInputPosition" => chat_input_position: TEXT;
        "chatDirectInput" => chat_direct_input: BOOL;
        "chatOutputEnabled" => chat_output_enabled: BOOL;
        "chatOutputFontUnderline" => chat_output_font_underline: BOOL;
        "chatOutputFontFamily" => chat_output_font_family: TEXT;
        "chatOutputRelativeFontSize" => chat_output_relative_font_size: SIGNED;
        "chatOutputFontWeight" => chat_output_font_weight: SIGNED;
        "chatOutputMode" => chat_output_mode: TEXT;
        "chatMoveOSD" => chat_move_osd: BOOL;
        "chatMaxLines" => chat_max_lines: SIGNED;
        "chatTopMargin" => chat_top_margin: SIGNED;
        "chatLeftMargin" => chat_left_margin: SIGNED;
        "chatBottomMargin" => chat_bottom_margin: SIGNED;
        "chatOSDMargin" => chat_osd_margin: SIGNED;
        "notificationTimeout" => notification_timeout_seconds: SIGNED;
        "alertTimeout" => alert_timeout_seconds: SIGNED;
        "chatTimeout" => chat_timeout_seconds: SIGNED;
        "showDurationNotification" => show_duration_notification: BOOL;
        "showSameRoomOSD" => show_same_room_osd: BOOL;
        "showOSDWarnings" => show_osd_warnings: BOOL;
        "showSlowdownOSD" => show_slowdown_osd: BOOL;
        "showNonControllerOSD" => show_noncontroller_osd: BOOL;
        "showDifferentRoomOSD" => show_different_room_osd: BOOL;
        "showContactInfo" => show_contact_info: BOOL;
    }
}
