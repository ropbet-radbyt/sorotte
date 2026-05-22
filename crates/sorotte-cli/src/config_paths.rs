use std::path::PathBuf;

use super::env_trimmed;

const SOROTTE_CONFIG_FILE_NAME: &str = "sorotte.ini";

fn sorotte_cli_config_path_override() -> Option<PathBuf> {
    env_trimmed("SOROTTE_CLIENT_CONFIG_PATH").map(PathBuf::from)
}

pub(super) fn sorotte_cli_gui_state_root_override() -> Option<PathBuf> {
    env_trimmed("SOROTTE_CLIENT_GUI_STATE_ROOT").map(PathBuf::from)
}

pub(super) fn default_sorotte_cli_config_root() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_trimmed("APPDATA").map(|root| PathBuf::from(root).join("Sorotte"));
    }
    if cfg!(target_os = "macos") {
        return env_trimmed("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Sorotte")
        });
    }
    if let Some(xdg_config_home) = env_trimmed("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg_config_home).join("sorotte"));
    }
    env_trimmed("HOME").map(|home| PathBuf::from(home).join(".config").join("sorotte"))
}

pub(super) fn resolve_sorotte_cli_config_path() -> Option<PathBuf> {
    if let Some(path) = sorotte_cli_config_path_override() {
        return Some(path);
    }
    Some(default_sorotte_cli_config_root()?.join(SOROTTE_CONFIG_FILE_NAME))
}
