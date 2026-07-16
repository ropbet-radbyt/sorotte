use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use sorotte_client_app::app_boundary::{
    persistence::{
        format_serialized_public_servers_list_legacy_compatible,
        parse_serialized_public_servers_list_legacy_compatible,
    },
    state::StoredClientSettingsMvp,
};

use super::LEGACY_GUI_QSETTINGS_STORE_NAMES;
use super::remote_services;
use super::runtime_localization::{
    localized_update_checked_at_line_legacy_compatible,
    localized_update_dismiss_hint_line_legacy_compatible,
    localized_update_notice_available_message_legacy_compatible,
};
use super::shell_state::{
    GuiConfigurationTab, GuiShellView, GuiTransientNotificationLevel, MenuActionId,
    SorotteGuiShellAppState,
};
use super::support::autoplay_threshold_from_settings;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiPersistedUiState {
    pub(super) active_view: Option<GuiShellView>,
    pub(super) configuration_tab: Option<GuiConfigurationTab>,
    pub(super) selected_public_server_address: Option<String>,
    pub(super) selected_media_search_directory: Option<String>,
    pub(super) hide_empty_rooms: bool,
    pub(super) show_playback_buttons: Option<bool>,
    pub(super) show_autoplay_controls: Option<bool>,
    pub(super) autoplay_checked: Option<bool>,
    pub(super) autoplay_min_users: Option<usize>,
    pub(super) last_media_dialog_directory: Option<String>,
    pub(super) last_checked_for_updates: Option<String>,
    pub(super) public_servers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiUpdateCheckState {
    pub(super) status: Option<remote_services::LegacyUpdateCheckStatus>,
    pub(super) message: Option<String>,
    pub(super) url: Option<String>,
    pub(super) candidate: Option<remote_services::UpdateCandidate>,
    pub(super) download_state: remote_services::UpdateDownloadState,
    pub(super) staged_update: Option<remote_services::StagedUpdate>,
    pub(super) self_update_supported: bool,
    pub(super) last_checked_for_updates: Option<String>,
    pub(super) user_initiated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiUpdateIndicatorTone {
    Idle,
    Progress,
    Success,
    Info,
    Warning,
    Error,
}

impl GuiUpdateIndicatorTone {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Progress => "progress",
            Self::Success => "success",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiUpdateIndicatorModel {
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) tone: GuiUpdateIndicatorTone,
    pub(super) enabled: bool,
}

impl GuiUpdateCheckState {
    pub(super) fn body_lines(&self, language: Option<&str>) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(message) = self.message.as_deref() {
            lines.push(message.to_owned());
        } else {
            lines.push(
                localized_update_notice_available_message_legacy_compatible(language).to_owned(),
            );
        }
        if let Some(timestamp) = self.last_checked_for_updates.as_deref() {
            lines.push(localized_update_checked_at_line_legacy_compatible(
                language, timestamp,
            ));
        } else {
            lines.push(localized_update_dismiss_hint_line_legacy_compatible(language).to_owned());
        }
        lines
    }

    pub(super) fn status_level(&self) -> GuiTransientNotificationLevel {
        match self.status.as_ref() {
            Some(remote_services::LegacyUpdateCheckStatus::UpToDate) => {
                GuiTransientNotificationLevel::Success
            }
            Some(remote_services::LegacyUpdateCheckStatus::Checking) => {
                GuiTransientNotificationLevel::Info
            }
            Some(remote_services::LegacyUpdateCheckStatus::UpdateAvailable) => {
                GuiTransientNotificationLevel::Info
            }
            Some(remote_services::LegacyUpdateCheckStatus::Failed)
            | Some(remote_services::LegacyUpdateCheckStatus::Unknown(_))
            | None => GuiTransientNotificationLevel::Warning,
        }
    }

    pub(super) fn indicator_model(&self, language: Option<&str>) -> GuiUpdateIndicatorModel {
        if self.update_install_launching() {
            return GuiUpdateIndicatorModel {
                title: "Installing update".to_owned(),
                detail: "Sorotte will restart.".to_owned(),
                tone: GuiUpdateIndicatorTone::Progress,
                enabled: false,
            };
        }
        if matches!(
            self.download_state,
            remote_services::UpdateDownloadState::Downloading
        ) {
            return GuiUpdateIndicatorModel {
                title: "Downloading update".to_owned(),
                detail: "Staging package.".to_owned(),
                tone: GuiUpdateIndicatorTone::Progress,
                enabled: false,
            };
        }
        if self.can_restart_to_update() {
            return GuiUpdateIndicatorModel {
                title: "Ready to install".to_owned(),
                detail: "Click to restart Sorotte.".to_owned(),
                tone: GuiUpdateIndicatorTone::Info,
                enabled: true,
            };
        }
        match self.status.as_ref() {
            Some(remote_services::LegacyUpdateCheckStatus::Checking) => GuiUpdateIndicatorModel {
                title: "Checking for updates".to_owned(),
                detail: "Please wait.".to_owned(),
                tone: GuiUpdateIndicatorTone::Progress,
                enabled: false,
            },
            Some(remote_services::LegacyUpdateCheckStatus::UpdateAvailable)
                if self.candidate.is_some() && self.self_update_supported =>
            {
                GuiUpdateIndicatorModel {
                    title: "Update available".to_owned(),
                    detail: "Click to install.".to_owned(),
                    tone: GuiUpdateIndicatorTone::Info,
                    enabled: true,
                }
            }
            Some(remote_services::LegacyUpdateCheckStatus::UpdateAvailable) => {
                GuiUpdateIndicatorModel {
                    title: "Manual update available".to_owned(),
                    detail: "Packaged install required.".to_owned(),
                    tone: GuiUpdateIndicatorTone::Warning,
                    enabled: true,
                }
            }
            Some(remote_services::LegacyUpdateCheckStatus::UpToDate)
                if !self.self_update_supported =>
            {
                GuiUpdateIndicatorModel {
                    title: "Self-update unavailable".to_owned(),
                    detail: self
                        .message
                        .clone()
                        .unwrap_or_else(|| "Packaged install required.".to_owned()),
                    tone: GuiUpdateIndicatorTone::Idle,
                    enabled: true,
                }
            }
            Some(remote_services::LegacyUpdateCheckStatus::UpToDate) => GuiUpdateIndicatorModel {
                title: "Up to date".to_owned(),
                detail: self.indicator_checked_detail(language),
                tone: GuiUpdateIndicatorTone::Success,
                enabled: true,
            },
            Some(remote_services::LegacyUpdateCheckStatus::Failed)
            | Some(remote_services::LegacyUpdateCheckStatus::Unknown(_)) => {
                GuiUpdateIndicatorModel {
                    title: "Update failed".to_owned(),
                    detail: "Click to retry.".to_owned(),
                    tone: GuiUpdateIndicatorTone::Error,
                    enabled: true,
                }
            }
            None => GuiUpdateIndicatorModel {
                title: "Update".to_owned(),
                detail: "Not checked yet.".to_owned(),
                tone: GuiUpdateIndicatorTone::Idle,
                enabled: true,
            },
        }
    }

    pub(super) fn update_indicator_activation_action(&self) -> Option<GuiUpdateIndicatorAction> {
        if self.update_install_launching()
            || matches!(
                self.status,
                Some(remote_services::LegacyUpdateCheckStatus::Checking)
            )
            || matches!(
                self.download_state,
                remote_services::UpdateDownloadState::Downloading
            )
        {
            return None;
        }
        if self.can_restart_to_update() {
            return Some(GuiUpdateIndicatorAction::ApplyStaged);
        }
        if self.candidate.is_some() && self.self_update_supported {
            return Some(GuiUpdateIndicatorAction::InstallAvailable);
        }
        Some(GuiUpdateIndicatorAction::Check)
    }

    pub(super) fn can_restart_to_update(&self) -> bool {
        self.staged_update.is_some() && self.self_update_supported
    }

    fn indicator_checked_detail(&self, language: Option<&str>) -> String {
        self.last_checked_for_updates
            .as_deref()
            .map(|timestamp| {
                localized_update_checked_at_line_legacy_compatible(language, timestamp)
            })
            .unwrap_or_else(|| "Checked recently.".to_owned())
    }

    fn update_install_launching(&self) -> bool {
        self.message.as_deref().is_some_and(|message| {
            message == "Launching update helper..." || message.starts_with("Update helper started.")
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiUpdateIndicatorAction {
    Check,
    InstallAvailable,
    ApplyStaged,
}

impl GuiPersistedUiState {
    pub(super) fn from_shell_state(state: &SorotteGuiShellAppState) -> Self {
        let saved_public_servers = state
            .saved_configuration
            .public_servers
            .clone()
            .unwrap_or_default();
        let current_public_servers = state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>();
        let current_settings = state.configuration.to_stored_settings();
        Self {
            active_view: (state.active_view != GuiShellView::Setup).then_some(state.active_view),
            configuration_tab: (state.selected_configuration_tab
                != GuiConfigurationTab::Connection)
                .then_some(state.selected_configuration_tab),
            selected_public_server_address: state
                .selected_public_server_address()
                .map(str::to_owned),
            selected_media_search_directory: state
                .selection
                .selected_media_search_directory
                .and_then(|index| state.media_search.directories.get(index))
                .map(|row| row.path.clone()),
            hide_empty_rooms: state.main_window.hide_empty_rooms,
            show_playback_buttons: (!state.main_window.show_playback_buttons)
                .then_some(state.main_window.show_playback_buttons),
            show_autoplay_controls: (!state.main_window.show_autoplay_controls)
                .then_some(state.main_window.show_autoplay_controls),
            autoplay_checked: Some(state.main_window.autoplay_active).filter(|value| {
                Some(*value)
                    != state
                        .saved_configuration
                        .autoplay_initial_state
                        .or(Some(false))
            }),
            autoplay_min_users: Some(state.main_window.autoplay_threshold).filter(|value| {
                *value != autoplay_threshold_from_settings(&state.saved_configuration)
            }),
            last_media_dialog_directory: state.last_media_dialog_directory.clone(),
            last_checked_for_updates: (current_settings.last_checked_for_updates
                != state.saved_configuration.last_checked_for_updates)
                .then(|| current_settings.last_checked_for_updates.clone())
                .flatten(),
            public_servers: if current_public_servers != saved_public_servers {
                current_public_servers
            } else {
                Vec::new()
            },
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.active_view.is_none()
            && self.configuration_tab.is_none()
            && self.selected_public_server_address.is_none()
            && self.selected_media_search_directory.is_none()
            && !self.hide_empty_rooms
            && self.show_playback_buttons.is_none()
            && self.show_autoplay_controls.is_none()
            && self.autoplay_checked.is_none()
            && self.autoplay_min_users.is_none()
            && self.last_media_dialog_directory.is_none()
            && self.last_checked_for_updates.is_none()
            && self.public_servers.is_empty()
    }

    pub(super) fn merge_into_startup_settings(&self, settings: &mut StoredClientSettingsMvp) {
        if let Some(last_checked_for_updates) = self.last_checked_for_updates.as_ref() {
            settings.last_checked_for_updates = Some(last_checked_for_updates.clone());
        }
        if !self.public_servers.is_empty() {
            settings.public_servers = Some(self.public_servers.clone());
        }
    }

    pub(super) fn apply_to_shell_state(&self, state: &mut SorotteGuiShellAppState) {
        if let Some(active_view) = self.active_view {
            state.active_view = active_view;
        }
        if let Some(tab) = self.configuration_tab {
            state.select_configuration_tab(tab);
        }
        state.last_media_dialog_directory = self.last_media_dialog_directory.clone();
        if let Some(selected_address) = self.selected_public_server_address.as_deref()
            && let Some(index) = state
                .public_servers
                .servers
                .iter()
                .position(|row| row.address == selected_address)
        {
            let _ = state.apply_public_server_selection(index);
        }
        if let Some(selected_directory) = self.selected_media_search_directory.as_deref()
            && let Some(index) = state
                .media_search
                .directories
                .iter()
                .position(|row| row.path == selected_directory)
        {
            state.selection.selected_media_search_directory = Some(index);
        }
        if self.hide_empty_rooms {
            state.main_window.hide_empty_rooms = true;
        }
        if let Some(show_playback_buttons) = self.show_playback_buttons {
            state.main_window.show_playback_buttons = show_playback_buttons;
            state.set_menu_action_checked(
                MenuActionId::TogglePlaybackButtons,
                show_playback_buttons,
            );
        }
        if let Some(show_autoplay_controls) = self.show_autoplay_controls {
            state.main_window.show_autoplay_controls = show_autoplay_controls;
        }
        if let Some(autoplay_checked) = self.autoplay_checked {
            state.main_window.autoplay_active = autoplay_checked;
        }
        if let Some(autoplay_min_users) = self.autoplay_min_users {
            state.main_window.autoplay_threshold = autoplay_min_users;
        }
        state.normalize_selection();
        state.apply_selection_to_surfaces();
    }
}

fn legacy_gui_qsettings_store_dir(root: &Path) -> PathBuf {
    root.to_path_buf()
}

pub(super) fn legacy_gui_qsettings_store_path(root: &Path, store_name: &str) -> PathBuf {
    legacy_gui_qsettings_store_dir(root).join(format!("{store_name}.ini"))
}

fn parse_legacy_gui_qsettings_ini(contents: &str) -> BTreeMap<(String, String), String> {
    let mut current_section = String::new();
    let mut values = BTreeMap::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current_section = section.trim().to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(
            (current_section.clone(), key.trim().to_owned()),
            value.trim().replace("%%", "%"),
        );
    }
    values
}

fn write_legacy_gui_qsettings_ini(
    path: &Path,
    sections: &[(&str, Vec<(&str, String)>)],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create Sorotte GUI state directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut contents = String::new();
    for (section, entries) in sections {
        if entries.is_empty() {
            continue;
        }
        contents.push('[');
        contents.push_str(section);
        contents.push_str("]\n");
        for (key, value) in entries {
            contents.push_str(key);
            contents.push_str(" = ");
            contents.push_str(&value.replace('%', "%%"));
            contents.push('\n');
        }
        contents.push('\n');
    }
    std::fs::write(path, contents).map_err(|error| {
        format!(
            "failed to persist Sorotte GUI state {}: {error}",
            path.display()
        )
    })
}

fn remove_file_if_exists(path: &Path, context: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(format!(
            "{context} path is not a file and cannot be cleared: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path)
        .map_err(|error| format!("failed clearing {context} {}: {error}", path.display()))?;
    Ok(true)
}

pub(super) fn clear_legacy_gui_qsettings_files_at_root(root: &Path) -> Result<bool, String> {
    let mut changed = false;
    for store_name in LEGACY_GUI_QSETTINGS_STORE_NAMES {
        changed |= remove_file_if_exists(
            &legacy_gui_qsettings_store_path(root, store_name),
            "Sorotte GUI state",
        )?;
    }
    Ok(changed)
}

pub(super) fn persist_gui_ui_state_at_root(
    root: &Path,
    state: &GuiPersistedUiState,
) -> Result<(), String> {
    if state.is_empty() {
        clear_legacy_gui_qsettings_files_at_root(root)?;
        return Ok(());
    }

    write_legacy_gui_qsettings_ini(
        &legacy_gui_qsettings_store_path(root, "MainWindow"),
        &[(
            "MainWindow",
            [
                state
                    .active_view
                    .map(|view| ("activeView", view.label().to_owned())),
                state
                    .configuration_tab
                    .map(|tab| ("configurationTab", tab.label().to_owned())),
                state
                    .hide_empty_rooms
                    .then(|| ("hideEmptyRooms", "true".to_owned())),
                state
                    .show_playback_buttons
                    .map(|value| ("showPlaybackButtons", value.to_string())),
                state
                    .show_autoplay_controls
                    .map(|value| ("showAutoPlayButton", value.to_string())),
                state
                    .autoplay_checked
                    .map(|value| ("autoplayChecked", value.to_string())),
                state
                    .autoplay_min_users
                    .map(|value| ("autoplayMinUsers", value.to_string())),
                state
                    .selected_public_server_address
                    .as_ref()
                    .map(|value| ("selectedPublicServerAddress", value.clone())),
                state
                    .selected_media_search_directory
                    .as_ref()
                    .map(|value| ("selectedMediaSearchDirectory", value.clone())),
            ]
            .into_iter()
            .flatten()
            .collect(),
        )],
    )?;

    let mut interface_sections = Vec::new();
    if let Some(last_checked_for_updates) = state.last_checked_for_updates.as_ref() {
        interface_sections.push((
            "Update",
            vec![("lastCheckedQt", last_checked_for_updates.clone())],
        ));
    }
    if !state.public_servers.is_empty() {
        interface_sections.push((
            "PublicServerList",
            vec![(
                "publicServers",
                format_serialized_public_servers_list_legacy_compatible(&state.public_servers),
            )],
        ));
    }
    if interface_sections.is_empty() {
        remove_file_if_exists(
            &legacy_gui_qsettings_store_path(root, "Interface"),
            "Sorotte GUI state",
        )?;
    } else {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "Interface"),
            &interface_sections,
        )?;
    }

    if let Some(directory) = state.last_media_dialog_directory.as_ref() {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "MediaBrowseDialog"),
            &[("MediaBrowseDialog", vec![("mediadir", directory.clone())])],
        )?;
    } else {
        remove_file_if_exists(
            &legacy_gui_qsettings_store_path(root, "MediaBrowseDialog"),
            "Sorotte GUI state",
        )?;
    }

    Ok(())
}

pub(super) fn load_gui_ui_state_from_root(
    root: &Path,
) -> Result<Option<GuiPersistedUiState>, String> {
    let mut state = GuiPersistedUiState::default();

    let main_window_path = legacy_gui_qsettings_store_path(root, "MainWindow");
    if main_window_path.exists() {
        let contents = std::fs::read_to_string(&main_window_path).map_err(|error| {
            format!(
                "failed to read Sorotte GUI state {}: {error}",
                main_window_path.display()
            )
        })?;
        let parsed = parse_legacy_gui_qsettings_ini(&contents);
        state.active_view = parsed
            .get(&(String::from("MainWindow"), String::from("activeView")))
            .and_then(|value| GuiShellView::from_label(value));
        state.configuration_tab = parsed
            .get(&(String::from("MainWindow"), String::from("configurationTab")))
            .and_then(|value| GuiConfigurationTab::from_label(value));
        state.selected_public_server_address = parsed
            .get(&(
                String::from("MainWindow"),
                String::from("selectedPublicServerAddress"),
            ))
            .cloned()
            .filter(|value| !value.trim().is_empty());
        state.hide_empty_rooms = parsed
            .get(&(String::from("MainWindow"), String::from("hideEmptyRooms")))
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        state.show_playback_buttons = parsed
            .get(&(
                String::from("MainWindow"),
                String::from("showPlaybackButtons"),
            ))
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        state.show_autoplay_controls = parsed
            .get(&(
                String::from("MainWindow"),
                String::from("showAutoPlayButton"),
            ))
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        state.autoplay_checked = parsed
            .get(&(String::from("MainWindow"), String::from("autoplayChecked")))
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        state.autoplay_min_users = parsed
            .get(&(String::from("MainWindow"), String::from("autoplayMinUsers")))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value >= 2);
        state.selected_media_search_directory = parsed
            .get(&(
                String::from("MainWindow"),
                String::from("selectedMediaSearchDirectory"),
            ))
            .cloned()
            .filter(|value| !value.trim().is_empty());
    }

    let interface_path = legacy_gui_qsettings_store_path(root, "Interface");
    if interface_path.exists() {
        let contents = std::fs::read_to_string(&interface_path).map_err(|error| {
            format!(
                "failed to read Sorotte GUI state {}: {error}",
                interface_path.display()
            )
        })?;
        let parsed = parse_legacy_gui_qsettings_ini(&contents);
        state.last_checked_for_updates = parsed
            .get(&(String::from("Update"), String::from("lastCheckedQt")))
            .cloned()
            .filter(|value| !value.trim().is_empty());
        state.public_servers = parsed
            .get(&(
                String::from("PublicServerList"),
                String::from("publicServers"),
            ))
            .and_then(|value| parse_serialized_public_servers_list_legacy_compatible(value))
            .unwrap_or_default();
    }

    let media_browse_path = legacy_gui_qsettings_store_path(root, "MediaBrowseDialog");
    if media_browse_path.exists() {
        let contents = std::fs::read_to_string(&media_browse_path).map_err(|error| {
            format!(
                "failed to read Sorotte GUI state {}: {error}",
                media_browse_path.display()
            )
        })?;
        let parsed = parse_legacy_gui_qsettings_ini(&contents);
        state.last_media_dialog_directory = parsed
            .get(&(String::from("MediaBrowseDialog"), String::from("mediadir")))
            .cloned()
            .filter(|value| !value.trim().is_empty());
    }

    if state.is_empty() {
        Ok(None)
    } else {
        Ok(Some(state))
    }
}
