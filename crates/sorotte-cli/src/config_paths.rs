use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use super::env_trimmed;
use sorotte_client_app::app_boundary::storage::{
    SorotteClientStoragePaths, resolve_sorotte_client_storage_paths,
};

#[derive(Debug, Clone, Default)]
struct CliConfigPathOverrides {
    config_path: Option<PathBuf>,
    config_root: Option<PathBuf>,
}

static CLI_CONFIG_PATH_OVERRIDES: OnceLock<Mutex<CliConfigPathOverrides>> = OnceLock::new();

fn cli_config_path_overrides() -> &'static Mutex<CliConfigPathOverrides> {
    CLI_CONFIG_PATH_OVERRIDES.get_or_init(|| Mutex::new(CliConfigPathOverrides::default()))
}

pub(super) fn set_sorotte_cli_config_cli_overrides(
    config_path: Option<PathBuf>,
    config_root: Option<PathBuf>,
) {
    let mut overrides = cli_config_path_overrides()
        .lock()
        .expect("CLI config-path override lock should not be poisoned");
    overrides.config_path = config_path;
    overrides.config_root = config_root;
}

fn current_cli_config_path_overrides() -> CliConfigPathOverrides {
    cli_config_path_overrides()
        .lock()
        .expect("CLI config-path override lock should not be poisoned")
        .clone()
}

pub(super) fn sorotte_cli_gui_state_root_override() -> Option<PathBuf> {
    env_trimmed("SOROTTE_CLIENT_GUI_STATE_ROOT").map(PathBuf::from)
}

pub(super) fn resolve_sorotte_cli_storage_paths() -> Option<SorotteClientStoragePaths> {
    let overrides = current_cli_config_path_overrides();
    resolve_sorotte_client_storage_paths(overrides.config_path, overrides.config_root)
}

pub(super) fn resolve_sorotte_cli_storage_root() -> Option<PathBuf> {
    resolve_sorotte_cli_storage_paths().map(|paths| paths.storage_root)
}

pub(super) fn resolve_sorotte_cli_config_path() -> Option<PathBuf> {
    resolve_sorotte_cli_storage_paths().map(|paths| paths.config_path)
}
