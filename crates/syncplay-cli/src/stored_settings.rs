use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::anyhow;
#[cfg(test)]
use syncplay_client_app::app_boundary::persistence::{
    parse_syncplay_ini_stored_client_settings_mvp as shared_parse_syncplay_ini_stored_client_settings_mvp,
    upsert_syncplay_ini_stored_client_settings_mvp as shared_upsert_syncplay_ini_stored_client_settings_mvp,
};
use syncplay_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    persistence::{
        clear_syncplay_ini_stored_client_settings_mvp_at_path as shared_clear_syncplay_ini_stored_client_settings_mvp_at_path,
        load_syncplay_ini_stored_client_settings_mvp_from_path as shared_load_syncplay_ini_stored_client_settings_mvp_from_path,
        update_syncplay_ini_stored_client_settings_mvp_at_path as shared_update_syncplay_ini_stored_client_settings_mvp_at_path,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path as shared_upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    },
    state::{
        StoredClientSettingsEnvPresence, StoredClientSettingsMvp,
        stored_client_settings_config_plan_legacy_compatible,
    },
};
use syncplay_player_mpv::{LegacySyncplayUiSettings, MpvAdapter};

use crate::client_args::LegacyClientArgOverrides;
use crate::client_config::ClientLoopConfig;
use crate::config_paths::{
    default_syncplay_cli_config_root_legacy_compatible,
    resolve_syncplay_cli_config_path_legacy_compatible,
    syncplay_cli_legacy_gui_qsettings_root_override,
};
use crate::env_support::{env_port, env_trimmed};

mod config_apply;
mod media_search;
mod persistence;
mod player_defaults;
mod ui_settings;

use self::player_defaults::normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible;

pub(super) use self::config_apply::apply_stored_client_settings_mvp_if_env_absent;
pub(super) use self::media_search::apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible;
#[cfg(test)]
pub(super) use self::media_search::resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible;
pub(super) use self::persistence::{
    clear_syncplay_cli_gui_qsettings_legacy_compatible,
    clear_syncplay_cli_stored_settings_legacy_compatible,
    load_syncplay_cli_stored_settings_mvp_legacy_compatible,
    persist_syncplay_cli_language_setting_legacy_compatible,
    persist_syncplay_cli_per_player_arguments_setting_legacy_compatible,
    persist_syncplay_cli_player_path_setting_legacy_compatible,
    persist_syncplay_cli_stored_settings_mvp_legacy_compatible,
};
#[cfg(test)]
pub(super) use self::persistence::{
    parse_syncplay_ini_stored_client_settings_mvp, upsert_syncplay_ini_stored_client_settings_mvp,
};
pub(super) use self::player_defaults::apply_stored_legacy_startup_player_defaults_if_arg_absent;
pub(super) use self::ui_settings::apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible;
#[cfg(test)]
pub(super) use self::ui_settings::{
    LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER, legacy_syncplay_ui_settings_from_stored_settings,
    legacy_syncplayintf_script_source_with_chat_input_bridge_legacy_compatible,
};
