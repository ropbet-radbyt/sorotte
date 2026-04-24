use std::path::PathBuf;

use super::env_trimmed;

fn syncplay_config_names_legacy_compatible() -> [&'static str; 2] {
    [".syncplay", "syncplay.ini"]
}

fn syncplay_cli_config_path_override() -> Option<PathBuf> {
    env_trimmed("SYNCPLAY_CLIENT_CONFIG_PATH").map(PathBuf::from)
}

pub(super) fn syncplay_cli_legacy_gui_qsettings_root_override() -> Option<PathBuf> {
    env_trimmed("SYNCPLAY_CLIENT_LEGACY_QSETTINGS_ROOT").map(PathBuf::from)
}

pub(super) fn default_syncplay_cli_config_root_legacy_compatible() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_trimmed("APPDATA").map(PathBuf::from);
    }
    if let Some(xdg_config_home) = env_trimmed("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg_config_home));
    }
    env_trimmed("HOME").map(|home| PathBuf::from(home).join(".config"))
}

pub(super) fn resolve_syncplay_cli_config_path_legacy_compatible() -> Option<PathBuf> {
    if let Some(path) = syncplay_cli_config_path_override() {
        return Some(path);
    }
    if let Ok(cwd) = std::env::current_dir() {
        for name in syncplay_config_names_legacy_compatible() {
            let candidate = cwd.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let root = default_syncplay_cli_config_root_legacy_compatible()?;
    for name in syncplay_config_names_legacy_compatible() {
        let candidate = root.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    Some(root.join("syncplay.ini"))
}
