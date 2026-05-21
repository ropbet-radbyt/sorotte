use syncplay_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    state::{
        parse_autoplay_min_users_override_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible,
    },
};
use syncplay_client_core::PrivacyMode;

use super::runtime_localization::localize_gui_runtime_message_legacy_compatible;
use super::shell_state::{
    GuiDialogControlKind, GuiFocusedConfigurationControlState, GuiPendingOperationKind,
    GuiValidationIssue, GuiValidationState, SyncplayGuiShellAppState,
    playlist_entries_multiline_text,
};
use super::support::{
    nonempty_room_name_text, normalized_editable_text, parse_trusted_domains_text,
};

impl SyncplayGuiShellAppState {
    pub(super) fn normalize_public_server_edit_session(&mut self) {
        let mut selected_index_to_apply = None;
        let Some(session) = self.public_server_edit_session.as_mut() else {
            return;
        };
        if let Some(index) = session.editing_index {
            let matching_index = self
                .public_servers
                .servers
                .get(index)
                .filter(|row| {
                    session.original_label.as_deref() == Some(row.label.as_str())
                        && session.original_address.as_deref() == Some(row.address.as_str())
                })
                .map(|_| index)
                .or_else(|| {
                    self.public_servers.servers.iter().position(|row| {
                        session.original_label.as_deref() == Some(row.label.as_str())
                            && session.original_address.as_deref() == Some(row.address.as_str())
                    })
                });
            let Some(index) = matching_index else {
                self.public_server_edit_session = None;
                return;
            };
            session.editing_index = Some(index);
            selected_index_to_apply = Some(index);
            let Some(row) = self.public_servers.servers.get(index) else {
                self.public_server_edit_session = None;
                return;
            };
            session.is_dirty =
                session.label_buffer != row.label || session.address_buffer != row.address;
            if !session.is_dirty {
                session.label_buffer = row.label.clone();
                session.address_buffer = row.address.clone();
                session.original_label = Some(row.label.clone());
                session.original_address = Some(row.address.clone());
            }
        } else {
            session.is_dirty = !session.label_buffer.trim().is_empty()
                || !session.address_buffer.trim().is_empty();
        }
        if let Some(index) = selected_index_to_apply {
            self.set_selected_public_server_index(Some(index));
        }
    }

    pub(super) fn normalize_main_window_user_edit_session(&mut self) {
        let Some(session) = self.main_window_user_edit_session.as_mut() else {
            return;
        };
        let matching_index = self
            .main_window
            .users
            .get(session.editing_index)
            .filter(|user| {
                user.username
                    .eq_ignore_ascii_case(&session.original_username)
            })
            .map(|_| session.editing_index)
            .or_else(|| {
                self.main_window.users.iter().position(|user| {
                    user.username
                        .eq_ignore_ascii_case(&session.original_username)
                })
            });
        let Some(index) = matching_index else {
            self.main_window_user_edit_session = None;
            return;
        };
        session.editing_index = index;
        let Some(user) = self.main_window.users.get(index) else {
            self.main_window_user_edit_session = None;
            return;
        };
        session.is_dirty = session.username_buffer != user.username;
        if !session.is_dirty {
            session.username_buffer = user.username.clone();
            session.original_username = user.username.clone();
        }
        self.selection.selected_main_window_user = Some(index);
        for (user_index, user) in self.main_window.users.iter_mut().enumerate() {
            user.is_selected = user_index == index;
        }
    }

    pub(super) fn normalize_text_edit_session(&mut self) {
        let Some(session) = self.text_edit_session.as_mut() else {
            return;
        };
        let Some(control) = self.configuration.control(session.section, session.label) else {
            self.text_edit_session = None;
            return;
        };
        if !control.kind.is_editable() || control.kind == GuiDialogControlKind::Checkbox {
            self.text_edit_session = None;
            return;
        }
        session.is_dirty = session.buffer != control.value;
    }

    pub(super) fn normalize_playlist_text_edit_session(&mut self) {
        if !self.shared_playlist_events_enabled() {
            self.playlist_text_edit_session = None;
            return;
        }
        let current_value =
            playlist_entries_multiline_text(&self.current_shared_playlist_entries());
        let Some(session) = self.playlist_text_edit_session.as_mut() else {
            return;
        };
        session.is_dirty = session.buffer != current_value;
    }

    pub(super) fn normalize_playlist_url_edit_session(&mut self) {
        if !self.shared_playlist_events_enabled() {
            self.playlist_url_edit_session = None;
            return;
        }
        let Some(session) = self.playlist_url_edit_session.as_mut() else {
            return;
        };
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
    }

    pub(super) fn normalize_media_url_edit_session(&mut self) {
        let Some(session) = self.media_url_edit_session.as_mut() else {
            return;
        };
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
    }

    pub(super) fn normalize_controlled_room_create_session(&mut self) {
        let default_room_name = self.controlled_room_create_default_room_name();
        let Some(session) = self.controlled_room_create_session.as_mut() else {
            return;
        };
        let Some(default_room_name) = default_room_name else {
            self.controlled_room_create_session = None;
            return;
        };
        session.is_dirty = nonempty_room_name_text(&session.room_buffer)
            .is_some_and(|room_name| room_name != default_room_name);
    }

    pub(super) fn normalize_controller_auth_edit_session(&mut self) {
        let current_room_name = self
            .current_joined_main_window_room_name()
            .map(str::to_owned);
        let Some(session) = self.controller_auth_edit_session.as_mut() else {
            return;
        };
        let Some(current_room_name) = current_room_name else {
            self.controller_auth_edit_session = None;
            return;
        };
        if !current_room_name.starts_with('+') {
            self.controller_auth_edit_session = None;
            return;
        }
        session.room_name = current_room_name;
        session.is_dirty = normalized_editable_text(&session.password_buffer).is_some();
    }

    pub(super) fn sync_focused_configuration_control_to_text_edit_session(&mut self) {
        let Some(session) = self.text_edit_session.as_ref() else {
            return;
        };
        let Some(control) = self.configuration.control(session.section, session.label) else {
            return;
        };
        let activation_count = self
            .focused_configuration_control
            .as_ref()
            .filter(|focused| focused.section == session.section && focused.label == session.label)
            .map_or(0, |focused| focused.activation_count);
        self.focused_configuration_control = Some(GuiFocusedConfigurationControlState {
            section: session.section,
            label: session.label,
            kind: control.kind,
            activation_count,
        });
    }

    pub(super) fn normalize_focused_configuration_control(&mut self) {
        let Some(focused) = self.focused_configuration_control.as_mut() else {
            return;
        };
        let Some(control) = self.configuration.control(focused.section, focused.label) else {
            self.focused_configuration_control = None;
            return;
        };
        if !control.kind.is_editable() {
            self.focused_configuration_control = None;
            return;
        }
        focused.kind = control.kind;
    }

    pub(super) fn refresh_validation(&mut self) {
        let last_action_error = self.validation.last_action_error.clone();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_playlist_text_edit_session();
        self.normalize_playlist_url_edit_session();
        self.normalize_media_url_edit_session();
        self.normalize_controlled_room_create_session();
        self.normalize_controller_auth_edit_session();
        let mut issues = self.validation_issues();
        issues.extend(self.runtime_validation_issues.iter().cloned());
        self.sync_focused_configuration_control_to_text_edit_session();
        self.validation = GuiValidationState {
            issues,
            last_action_error,
        };
        self.refresh_command_availability();
    }

    pub(super) fn clear_action_error_and_refresh(&mut self) {
        self.validation.last_action_error = None;
        self.refresh_validation();
    }

    pub(super) fn record_action_error(&mut self, message: impl Into<String>) -> bool {
        let message = message.into();
        self.validation.last_action_error = Some(localize_gui_runtime_message_legacy_compatible(
            &message,
            Some(self.runtime_language_tag_legacy_compatible()),
        ));
        self.refresh_validation();
        false
    }

    pub(super) fn cancel_pending_operation(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No GUI operation is currently in progress.");
        };
        match pending.kind {
            GuiPendingOperationKind::SaveConfiguration => self.cancel_configuration_save(),
            GuiPendingOperationKind::ResetConfiguration => self.cancel_configuration_reset(),
            GuiPendingOperationKind::ReloadConfiguration => self.cancel_configuration_reload(),
            GuiPendingOperationKind::ClearGuiData => self.cancel_clear_gui_data(),
            GuiPendingOperationKind::ConnectSavedServer => self.cancel_saved_server_connect(),
            GuiPendingOperationKind::DisconnectSession => self.cancel_session_disconnect(),
            GuiPendingOperationKind::ConnectPublicServer => {
                self.cancel_selected_public_server_connect()
            }
            GuiPendingOperationKind::RefreshPublicServers => self.cancel_public_server_refresh(),
            GuiPendingOperationKind::SearchMissingMedia => self.cancel_missing_media_search(),
            GuiPendingOperationKind::TogglePlaybackPause => self.cancel_playback_pause_toggle(),
            GuiPendingOperationKind::SendChatMessage => self.cancel_local_chat_send(),
        }
    }

    pub(super) fn refresh_command_availability(&mut self) {
        self.commands = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override
            .apply_to(&mut self.commands);
    }

    pub(super) fn validation_issues(&self) -> Vec<GuiValidationIssue> {
        let mut issues = Vec::new();

        self.push_u16_validation_issue(
            &mut issues,
            "Connection",
            "Port",
            "must be a valid TCP port from 1 to 65535.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Readiness",
            "Unpause Action",
            |value| parse_unpause_action_mode_legacy_compatible(value).is_some(),
            "must be a supported unpause action mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Readiness",
            "Autoplay Min Users",
            |value| {
                value == "app-default"
                    || parse_autoplay_min_users_override_legacy_compatible(value).is_some()
            },
            "must be a supported autoplay threshold or 'app-default'.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Privacy",
            "Filename Privacy",
            |value| PrivacyMode::from_legacy_name(value).is_some(),
            "must be a supported privacy mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Privacy",
            "Filesize Privacy",
            |value| PrivacyMode::from_legacy_name(value).is_some(),
            "must be a supported privacy mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Privacy",
            "Trusted Domains",
            |value| parse_trusted_domains_text(value).is_some(),
            "must be a comma/semicolon-separated list or legacy bracketed list.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Chat",
            "Input Position",
            |value| matches!(value, "Top" | "Middle" | "Bottom"),
            "must be Top, Middle, or Bottom.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "Chat",
            "Output Mode",
            |value| matches!(value, "Chatroom" | "Scrolling"),
            "must be Chatroom or Scrolling.",
        );
        for (section, label) in [
            ("Desync", "Rewind Threshold"),
            ("Desync", "Fastforward Threshold"),
            ("Desync", "Slowdown Threshold"),
            ("Media Search", "First File Timeout"),
            ("Media Search", "Search Timeout"),
            ("Media Search", "Double Check Interval"),
            ("Media Search", "Warning Threshold"),
        ] {
            self.push_nonnegative_f64_validation_issue(
                &mut issues,
                section,
                label,
                "must be a finite non-negative number.",
            );
        }
        for (section, label, message) in [
            ("Chat", "Input Font Size", "must be a positive integer."),
            ("Chat", "Output Font Size", "must be a positive integer."),
        ] {
            self.push_positive_i64_validation_issue(&mut issues, section, label, message);
        }
        for (section, label) in [
            ("Chat", "Input Font Weight"),
            ("Chat", "Output Font Weight"),
            ("Chat", "Top Margin"),
            ("Chat", "Left Margin"),
            ("Chat", "Bottom Margin"),
            ("Chat", "OSD Margin"),
            ("OSD", "Notification Timeout"),
            ("OSD", "Alert Timeout"),
            ("OSD", "Chat Timeout"),
        ] {
            self.push_nonnegative_i64_validation_issue(
                &mut issues,
                section,
                label,
                "must be a non-negative integer.",
            );
        }
        self.push_positive_i64_validation_issue(
            &mut issues,
            "Chat",
            "Max Lines",
            "must be a positive integer.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "System",
            "Language",
            |value| normalized_legacy_runtime_language_tag_legacy_compatible(value).is_some(),
            "must be one of the supported legacy language tags.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            "System",
            "Update Channel",
            |value| matches!(value.to_ascii_lowercase().as_str(), "stable" | "dev"),
            "must be stable or dev.",
        );

        let mut seen_directories = std::collections::BTreeSet::new();
        for directory in &self.media_search.directories {
            if !seen_directories.insert(directory.path.clone()) {
                issues.push(GuiValidationIssue {
                    scope: "Media Search".to_owned(),
                    label: "Directories".to_owned(),
                    message: "contains duplicate search directories.".to_owned(),
                });
                break;
            }
        }

        for row in &self.public_servers.servers {
            let (host, _) =
                parse_host_and_optional_port_from_host_arg_legacy_compatible(&row.address);
            if host.trim().is_empty() {
                issues.push(GuiValidationIssue {
                    scope: "Public Servers".to_owned(),
                    label: "Address".to_owned(),
                    message: format!("'{}' is not a valid server address.", row.address),
                });
            }
        }

        issues
    }

    pub(super) fn push_parse_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        section: &'static str,
        label: &'static str,
        is_valid: impl FnOnce(&str) -> bool,
        message: &'static str,
    ) {
        let Some(value) = self.configuration.control_value(section, label) else {
            return;
        };
        let Some(normalized) = normalized_editable_text(value) else {
            return;
        };
        if !is_valid(&normalized) {
            issues.push(GuiValidationIssue {
                scope: section.to_owned(),
                label: label.to_owned(),
                message: message.to_owned(),
            });
        }
    }

    pub(super) fn push_u16_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        section: &'static str,
        label: &'static str,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            section,
            label,
            |value| value.parse::<u16>().is_ok_and(|parsed| parsed > 0),
            message,
        );
    }

    pub(super) fn push_nonnegative_f64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        section: &'static str,
        label: &'static str,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            section,
            label,
            |value| {
                value
                    .parse::<f64>()
                    .is_ok_and(|parsed| parsed.is_finite() && parsed >= 0.0)
            },
            message,
        );
    }

    pub(super) fn push_positive_i64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        section: &'static str,
        label: &'static str,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            section,
            label,
            |value| value.parse::<i64>().is_ok_and(|parsed| parsed > 0),
            message,
        );
    }

    pub(super) fn push_nonnegative_i64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        section: &'static str,
        label: &'static str,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            section,
            label,
            |value| value.parse::<i64>().is_ok_and(|parsed| parsed >= 0),
            message,
        );
    }
}
