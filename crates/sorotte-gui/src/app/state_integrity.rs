use super::runtime_localization::localize_gui_runtime_message;
use super::shell_state::{
    GuiDialogControlKind, GuiFocusedConfigurationControlState, GuiPendingOperationKind,
    GuiValidationIssue, GuiValidationState, SorotteGuiShellAppState,
    playlist_entries_multiline_text,
};
use super::support::{nonempty_room_name_text, normalized_editable_text};

impl SorotteGuiShellAppState {
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
        let Some(control) = self.configuration.control(session.id) else {
            self.text_edit_session = None;
            return;
        };
        if !control.kind.is_editable() || control.kind == GuiDialogControlKind::Checkbox {
            self.text_edit_session = None;
            return;
        }
        session.is_dirty = session.buffer.expose_for_ui() != control.value;
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
        session.is_dirty =
            normalized_editable_text(session.password_buffer.expose_secret()).is_some();
    }

    pub(super) fn sync_focused_configuration_control_to_text_edit_session(&mut self) {
        let Some(session) = self.text_edit_session.as_ref() else {
            return;
        };
        let Some(control) = self.configuration.control(session.id) else {
            return;
        };
        let activation_count = self
            .focused_configuration_control
            .as_ref()
            .filter(|focused| focused.id == session.id)
            .map_or(0, |focused| focused.activation_count);
        self.focused_configuration_control = Some(GuiFocusedConfigurationControlState {
            id: session.id,
            kind: control.kind,
            activation_count,
        });
    }

    pub(super) fn normalize_focused_configuration_control(&mut self) {
        let Some(focused) = self.focused_configuration_control.as_mut() else {
            return;
        };
        let Some(control) = self.configuration.control(focused.id) else {
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
        self.validation.last_action_error = Some(localize_gui_runtime_message(
            &message,
            Some(self.runtime_language_tag()),
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
            GuiPendingOperationKind::DiscardConfigurationChanges => {
                self.cancel_discard_configuration_changes()
            }
            GuiPendingOperationKind::ReloadConfiguration => self.cancel_configuration_reload(),
            GuiPendingOperationKind::ClearGuiData => self.cancel_clear_gui_data(),
            GuiPendingOperationKind::ChangeConfigStorageRoot => {
                self.cancel_config_storage_root_change()
            }
            GuiPendingOperationKind::ConnectSavedServer => self.cancel_saved_server_connect(),
            GuiPendingOperationKind::DisconnectSession => self.cancel_session_disconnect(),
            GuiPendingOperationKind::ConnectPublicServer => {
                self.cancel_selected_public_server_connect()
            }
            GuiPendingOperationKind::RefreshPublicServers => self.cancel_public_server_refresh(),
            GuiPendingOperationKind::SearchMissingMedia => self.cancel_missing_media_search(),
            GuiPendingOperationKind::SetPlaybackPause(_) => self.cancel_playback_pause_state(),
            GuiPendingOperationKind::TogglePlaybackPause => self.cancel_playback_pause_toggle(),
            GuiPendingOperationKind::SendChatMessage => self.cancel_local_chat_send(),
        }
    }

    pub(super) fn refresh_command_availability(&mut self) {
        self.commands = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override
            .apply_to(&mut self.commands);
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
    }

    pub(super) fn validation_issues(&self) -> Vec<GuiValidationIssue> {
        super::configuration_model::GuiConfigurationContext {
            configuration: &self.configuration,
            public_servers: &self.public_servers,
            media_search: &self.media_search,
        }
        .validation_issues()
    }
}
