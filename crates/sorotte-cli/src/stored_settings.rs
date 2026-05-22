use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::anyhow;
#[cfg(test)]
use sorotte_client_app::app_boundary::persistence::{
    parse_sorotte_ini_stored_client_settings_mvp as shared_parse_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp as shared_upsert_sorotte_ini_stored_client_settings_mvp,
};
use sorotte_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    persistence::{
        clear_sorotte_ini_stored_client_settings_mvp_at_path as shared_clear_sorotte_ini_stored_client_settings_mvp_at_path,
        load_sorotte_ini_stored_client_settings_mvp_from_path as shared_load_sorotte_ini_stored_client_settings_mvp_from_path,
        update_sorotte_ini_stored_client_settings_mvp_at_path as shared_update_sorotte_ini_stored_client_settings_mvp_at_path,
        upsert_sorotte_ini_stored_client_settings_mvp_at_path as shared_upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    },
    state::{
        StoredClientSettingsEnvPresence, StoredClientSettingsMvp,
        stored_client_settings_config_plan_legacy_compatible,
    },
};
use sorotte_player_mpv::{LegacySyncplayUiSettings, MpvAdapter};

use crate::client_args::LegacyClientArgOverrides;
use crate::client_config::ClientLoopConfig;
use crate::config_paths::{
    default_sorotte_cli_config_root, resolve_sorotte_cli_config_path,
    sorotte_cli_gui_state_root_override,
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
    clear_sorotte_cli_gui_state, clear_sorotte_cli_stored_settings_legacy_compatible,
    load_sorotte_cli_stored_settings_mvp_legacy_compatible,
    persist_sorotte_cli_language_setting_legacy_compatible,
    persist_sorotte_cli_per_player_arguments_setting_legacy_compatible,
    persist_sorotte_cli_player_path_setting_legacy_compatible,
    persist_sorotte_cli_stored_settings_mvp_legacy_compatible,
};
#[cfg(test)]
pub(super) use self::persistence::{
    parse_sorotte_ini_stored_client_settings_mvp, upsert_sorotte_ini_stored_client_settings_mvp,
};
pub(super) use self::player_defaults::apply_stored_legacy_startup_player_defaults_if_arg_absent;
pub(super) use self::ui_settings::apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible;
#[cfg(test)]
pub(super) use self::ui_settings::{
    LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER, legacy_syncplay_ui_settings_from_stored_settings,
    legacy_syncplayintf_script_source_with_chat_input_bridge_legacy_compatible,
};
