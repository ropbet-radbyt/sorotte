use std::collections::BTreeSet;

use syncplay_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    state::{
        StoredClientSettingsMvp, parse_autoplay_min_users_override_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible,
    },
};
use syncplay_client_core::PrivacyMode;

use super::shell_state::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, GuiDialogControl,
    GuiDialogControlKind,
};
use super::support::{bool_label, normalized_editable_text, parse_trusted_domains_text};

#[cfg(test)]
#[path = "app_configuration_draft/tests.rs"]
mod tests;

impl FirstRunConfigurationDialogDraft {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let state = FirstRunConfigurationDialogState::from_stored_settings(settings);
        Self {
            launch_mode: state.launch_mode,
            compatibility_startup_entry_count: state.system.compatibility_startup_entry_count,
            ignored_startup_exception_count: state.system.ignored_startup_exception_count,
            sections: state.dialog_sections(),
            settings: settings.clone(),
        }
    }

    pub(super) fn to_stored_settings(&self) -> StoredClientSettingsMvp {
        self.settings.clone()
    }

    pub(super) fn has_unsaved_changes_against(&self, settings: &StoredClientSettingsMvp) -> bool {
        self.settings != *settings || self.sections != Self::from_stored_settings(settings).sections
    }

    pub(super) fn control(&self, section: &str, label: &str) -> Option<&GuiDialogControl> {
        self.sections
            .iter()
            .find(|candidate| candidate.title == section)
            .and_then(|candidate| {
                candidate
                    .controls
                    .iter()
                    .find(|control| control.label == label)
            })
    }

    pub(super) fn control_value(&self, section: &str, label: &str) -> Option<&str> {
        self.control(section, label)
            .map(|control| control.value.as_str())
    }

    pub(super) fn control_identity(
        &self,
        section: &str,
        label: &str,
    ) -> Option<(&'static str, &'static str, GuiDialogControlKind)> {
        let section = self
            .sections
            .iter()
            .find(|candidate| candidate.title == section)?;
        let control = section
            .controls
            .iter()
            .find(|candidate| candidate.label == label)?;
        Some((section.title, control.label, control.kind))
    }

    pub(super) fn apply_text_value(&mut self, section: &str, label: &str, value: &str) -> bool {
        let Some(kind) = self.control(section, label).map(|control| control.kind) else {
            return false;
        };
        if !kind.is_editable() || kind == GuiDialogControlKind::Checkbox {
            return false;
        }
        let Some(control) = self.control_mut(section, label) else {
            return false;
        };
        control.value = value.to_owned();
        self.apply_text_value_to_settings(section, label, value);
        self.refresh_derived_controls();
        true
    }

    pub(super) fn apply_bool_value(&mut self, section: &str, label: &str, value: bool) -> bool {
        let Some(kind) = self.control(section, label).map(|control| control.kind) else {
            return false;
        };
        if kind != GuiDialogControlKind::Checkbox {
            return false;
        }
        let Some(control) = self.control_mut(section, label) else {
            return false;
        };
        control.value = bool_label(value).to_owned();
        self.apply_bool_value_to_settings(section, label, value);
        self.refresh_derived_controls();
        true
    }

    pub(super) fn room_history_multiline_text(&self) -> String {
        self.settings
            .room_list
            .as_deref()
            .map(|rooms| rooms.join("\n"))
            .unwrap_or_default()
    }

    pub(super) fn apply_room_history_multiline_text(&mut self, value: &str) {
        let rooms = value
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<BTreeSet<_>>();
        self.settings.room_list = (!rooms.is_empty()).then(|| rooms.into_iter().collect());
        self.refresh_derived_controls();
    }

    fn control_mut(&mut self, section: &str, label: &str) -> Option<&mut GuiDialogControl> {
        self.sections
            .iter_mut()
            .find(|candidate| candidate.title == section)
            .and_then(|candidate| candidate.control_mut(label))
    }

    fn apply_text_value_to_settings(&mut self, section: &str, label: &str, value: &str) {
        match (section, label) {
            ("Connection", "Host") => {
                self.settings.host = normalized_editable_text(value);
            }
            ("Connection", "Port") => {
                self.settings.port = parse_optional_u16(value);
            }
            ("Connection", "Username") => {
                self.settings.username = normalized_editable_text(value);
            }
            ("Connection", "Room") => {
                self.settings.room = normalized_editable_text(value);
            }
            ("Connection", "Server Password") => {
                self.settings.server_password = normalized_editable_text(value);
            }
            ("Connection", "Player Path") => {
                self.settings.player_path = normalized_editable_text(value);
            }
            ("Readiness", "Unpause Action") => {
                self.settings.unpause_action = normalized_editable_text(value)
                    .as_deref()
                    .and_then(parse_unpause_action_mode_legacy_compatible);
            }
            ("Readiness", "Autoplay Min Users") => {
                self.settings.autoplay_min_users = normalized_editable_text(value)
                    .as_deref()
                    .and_then(|value| {
                        if value.eq_ignore_ascii_case("app-default") {
                            None
                        } else {
                            parse_autoplay_min_users_override_legacy_compatible(value)
                        }
                    });
            }
            ("Privacy", "Filename Privacy") => {
                self.settings.filename_privacy_mode = normalized_editable_text(value)
                    .as_deref()
                    .and_then(PrivacyMode::from_legacy_name);
            }
            ("Privacy", "Filesize Privacy") => {
                self.settings.filesize_privacy_mode = normalized_editable_text(value)
                    .as_deref()
                    .and_then(PrivacyMode::from_legacy_name);
            }
            ("Privacy", "Trusted Domains") => {
                self.settings.trusted_domains = parse_trusted_domains_text(value);
            }
            ("Desync", "Rewind Threshold") => {
                self.settings.rewind_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            ("Desync", "Fastforward Threshold") => {
                self.settings.fastforward_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            ("Desync", "Slowdown Threshold") => {
                self.settings.slowdown_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            ("Media Search", "First File Timeout") => {
                self.settings.folder_search_first_file_timeout_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            ("Media Search", "Search Timeout") => {
                self.settings.folder_search_timeout_seconds = parse_optional_nonnegative_f64(value);
            }
            ("Media Search", "Double Check Interval") => {
                self.settings.folder_search_double_check_interval_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            ("Media Search", "Warning Threshold") => {
                self.settings.folder_search_warning_threshold_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            ("Chat", "Max Lines") => {
                self.settings.chat_max_lines = parse_optional_positive_i64(value);
            }
            ("Chat", "Input Font") => {
                self.settings.chat_input_font_family = normalized_editable_text(value);
            }
            ("Chat", "Output Font") => {
                self.settings.chat_output_font_family = normalized_editable_text(value);
            }
            ("System", "Language") => {
                self.settings.language = normalized_editable_text(value)
                    .as_deref()
                    .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
                    .map(str::to_owned);
            }
            _ => {}
        }
    }

    fn apply_bool_value_to_settings(&mut self, section: &str, label: &str, value: bool) {
        match (section, label) {
            ("Readiness", "Ready At Start") => {
                self.settings.ready_at_start = Some(value);
            }
            ("Readiness", "Autoplay") => {
                self.settings.autoplay_initial_state = Some(value);
            }
            ("Readiness", "Require Same Filenames") => {
                self.settings.autoplay_require_same_filenames = Some(value);
            }
            ("Readiness", "Shared Playlists") => {
                self.settings.shared_playlist_enabled = Some(value);
            }
            ("Readiness", "Pause On Leave") => {
                self.settings.pause_on_leave = Some(value);
            }
            ("Privacy", "Trusted Domains Only") => {
                self.settings.only_switch_to_trusted_domains = Some(value);
            }
            ("Desync", "Rewind On Desync") => {
                self.settings.rewind_on_desync = Some(value);
            }
            ("Desync", "Fastforward On Desync") => {
                self.settings.fastforward_on_desync = Some(value);
            }
            ("Desync", "Slow On Desync") => {
                self.settings.slow_on_desync = Some(value);
            }
            ("Desync", "Dont Slow Down With Me") => {
                self.settings.dont_slow_down_with_me = Some(value);
            }
            ("Chat", "Chat Input") => {
                self.settings.chat_input_enabled = Some(value);
            }
            ("Chat", "Chat Output") => {
                self.settings.chat_output_enabled = Some(value);
            }
            ("Chat", "Direct Input") => {
                self.settings.chat_direct_input = Some(value);
            }
            ("Chat", "Move OSD") => {
                self.settings.chat_move_osd = Some(value);
            }
            ("OSD", "Show OSD") => {
                self.settings.show_osd = Some(value);
            }
            ("OSD", "Show Duration") => {
                self.settings.show_duration_notification = Some(value);
            }
            ("OSD", "Show Same Room") => {
                self.settings.show_same_room_osd = Some(value);
            }
            ("OSD", "Show Warnings") => {
                self.settings.show_osd_warnings = Some(value);
            }
            ("OSD", "Show Noncontroller") => {
                self.settings.show_noncontroller_osd = Some(value);
            }
            ("OSD", "Show Different Room") => {
                self.settings.show_different_room_osd = Some(value);
            }
            ("OSD", "Show Contact Info") => {
                self.settings.show_contact_info = Some(value);
            }
            ("System", "Auto Update") => {
                self.settings.check_for_updates_automatically = Some(value);
            }
            _ => {}
        }
    }

    fn refresh_derived_controls(&mut self) {
        let baseline = FirstRunConfigurationDialogState::from_stored_settings(&self.settings)
            .dialog_sections();
        for section in &mut self.sections {
            let Some(baseline_section) = baseline
                .iter()
                .find(|candidate| candidate.title == section.title)
            else {
                continue;
            };
            for control in &mut section.controls {
                let Some(baseline_control) = baseline_section
                    .controls
                    .iter()
                    .find(|candidate| candidate.label == control.label)
                else {
                    continue;
                };
                if !control.kind.is_editable()
                    || control.kind == GuiDialogControlKind::Checkbox
                    || (section.title == "Connection" && control.label == "Server Password")
                {
                    control.value = baseline_control.value.clone();
                }
            }
        }
    }
}

fn parse_optional_u16(value: &str) -> Option<u16> {
    normalized_editable_text(value)?
        .parse::<u16>()
        .ok()
        .filter(|parsed| *parsed > 0)
}

fn parse_optional_nonnegative_f64(value: &str) -> Option<f64> {
    normalized_editable_text(value)?
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite() && *parsed >= 0.0)
}

fn parse_optional_positive_i64(value: &str) -> Option<i64> {
    normalized_editable_text(value)?
        .parse::<i64>()
        .ok()
        .filter(|parsed| *parsed > 0)
}
