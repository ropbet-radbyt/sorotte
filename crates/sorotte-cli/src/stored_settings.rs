use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::anyhow;
#[cfg(test)]
use sorotte_client_app::app_boundary::persistence::{
    parse_sorotte_ini_stored_client_settings as shared_parse_sorotte_ini_stored_client_settings,
    upsert_sorotte_ini_stored_client_settings as shared_upsert_sorotte_ini_stored_client_settings,
};
use sorotte_client_app::app_boundary::{
    language::normalized_runtime_language_tag,
    persistence::{
        clear_sorotte_ini_stored_client_settings_at_path as shared_clear_sorotte_ini_stored_client_settings_at_path,
        load_sorotte_ini_stored_client_settings_from_path as shared_load_sorotte_ini_stored_client_settings_from_path,
        update_sorotte_ini_stored_client_settings_at_path as shared_update_sorotte_ini_stored_client_settings_at_path,
        upsert_sorotte_ini_stored_client_settings_at_path as shared_upsert_sorotte_ini_stored_client_settings_at_path,
    },
    state::{
        ClientConfig, StoredClientSettings, StoredClientSettingsEnvPresence,
        stored_client_settings_config_plan,
    },
};
use sorotte_player_mpv::{
    MpvAdapter, SorotteBridgeFailureKind, SorotteBridgeHealth, SyncplayUiSettings,
};

use crate::client_args::SyncplayClientArgOverrides;
use crate::client_config::ClientLoopConfig;
use crate::config_paths::{
    resolve_sorotte_cli_config_path, resolve_sorotte_cli_storage_root,
    sorotte_cli_gui_state_root_override,
};
use crate::env_support::{env_port, env_trimmed};

mod config_apply;
mod media_search;
mod persistence;
mod player_defaults;
mod ui_settings;

use self::player_defaults::normalize_player_path_for_stored_per_player_arguments_lookup;

pub(super) use self::config_apply::apply_stored_client_settings_if_env_absent;
pub(super) use self::media_search::apply_stored_media_search_startup_file_fallback_if_missing;
#[cfg(test)]
pub(super) use self::media_search::resolve_startup_file_with_media_search_fallback;
pub(super) use self::persistence::{
    clear_sorotte_cli_gui_state, clear_sorotte_cli_stored_settings,
    load_sorotte_cli_stored_settings, persist_sorotte_cli_language_setting,
    persist_sorotte_cli_per_player_arguments_setting, persist_sorotte_cli_player_path_setting,
    persist_sorotte_cli_stored_settings,
};
#[cfg(test)]
pub(super) use self::persistence::{
    parse_sorotte_ini_stored_client_settings, upsert_sorotte_ini_stored_client_settings,
};
pub(super) use self::player_defaults::apply_stored_startup_player_defaults_if_arg_absent;
pub(super) use self::ui_settings::apply_syncplay_ui_settings_to_mpv_adapter;
#[cfg(test)]
pub(super) use self::ui_settings::syncplay_ui_settings_from_stored_settings;
