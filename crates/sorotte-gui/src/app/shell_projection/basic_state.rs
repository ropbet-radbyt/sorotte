use super::*;

impl FirstRunConfigurationDialogDraft {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "sorotte-gui setup surface initialized in {} mode ({} startup entries, {} ignored exception).",
            self.launch_mode.label(),
            self.compatibility_startup_entry_count,
            self.ignored_startup_exception_count,
        )];

        for section in &self.sections {
            lines.push(format!("[{}]", section.title));
            for control in &section.controls {
                lines.push(format!(
                    "- {} [{}]: {}",
                    control.label,
                    control.kind.label(),
                    control.value
                ));
            }
        }

        lines.push(
            "Native window widgets use a room-first shell with a grouped setup state model, a typed dialog control schema, and an editable draft that round-trips back into shared client settings."
                .to_owned(),
        );
        lines
    }
}

impl GuiValidationState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "[Validation] status={}, last_action_error={}",
            if self.issues.is_empty() {
                "clean".to_owned()
            } else {
                format!("{} issue(s)", self.issues.len())
            },
            self.last_action_error.as_deref().unwrap_or("(none)")
        )];

        for issue in &self.issues {
            lines.push(format!(
                "- {} / {}: {}",
                issue.scope, issue.label, issue.message
            ));
        }

        lines
    }
}

impl GuiSelectionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Selection] user={}, playlist={}, menu={}, media_directory={}",
            optional_index_text(self.selected_main_window_user),
            optional_index_text(self.selected_main_window_playlist),
            self.selected_menu_action.map_or_else(
                || "(none)".to_owned(),
                |(section, action)| format!("{section}:{action}")
            ),
            optional_index_text(self.selected_media_search_directory),
        )]
    }
}

impl GuiCommandAvailabilityState {
    pub(in crate::app) fn any_enabled(&self) -> bool {
        self.can_save_configuration
            || self.can_reset_configuration
            || self.can_reload_configuration
            || self.can_connect_saved_server
            || self.can_disconnect_session
            || self.can_connect_public_server
            || self.can_refresh_public_servers
            || self.can_search_missing_media
            || self.can_toggle_pause
            || self.can_send_chat_message
    }

    #[cfg(test)]
    pub(in crate::app) fn render_lines(
        &self,
        pending_operation: Option<&GuiPendingOperationState>,
    ) -> Vec<String> {
        vec![
            format!(
                "[Commands] busy={}, save_configuration={}, reset_configuration={}, reload_configuration={}, connect_saved_server={}, disconnect_session={}, connect_public_server={}, refresh_public_servers={}, search_missing_media={}, toggle_pause={}, send_chat_message={}",
                bool_label(pending_operation.is_some()),
                bool_label(self.can_save_configuration),
                bool_label(self.can_reset_configuration),
                bool_label(self.can_reload_configuration),
                bool_label(self.can_connect_saved_server),
                bool_label(self.can_disconnect_session),
                bool_label(self.can_connect_public_server),
                bool_label(self.can_refresh_public_servers),
                bool_label(self.can_search_missing_media),
                bool_label(self.can_toggle_pause),
                bool_label(self.can_send_chat_message),
            ),
            format!(
                "[Pending] operation={}",
                pending_operation
                    .map(|pending| pending.kind.label())
                    .unwrap_or("(none)")
            ),
        ]
    }
}

impl GuiCommandAvailabilityRuntimeOverride {
    pub(in crate::app) fn from_baseline_and_snapshot(
        baseline: &GuiCommandAvailabilityState,
        snapshot: &GuiCommandAvailabilityState,
    ) -> Self {
        Self {
            // Settings persistence commands are owned entirely by the local draft state. A
            // runtime snapshot can disable session operations, but must not leave a stale clean,
            // dirty, valid, or busy decision attached to Save/Discard/Reload.
            can_save_configuration: None,
            can_reset_configuration: None,
            can_reload_configuration: None,
            can_connect_saved_server: (baseline.can_connect_saved_server
                != snapshot.can_connect_saved_server)
                .then_some(snapshot.can_connect_saved_server),
            can_disconnect_session: (baseline.can_disconnect_session
                != snapshot.can_disconnect_session)
                .then_some(snapshot.can_disconnect_session),
            can_connect_public_server: (baseline.can_connect_public_server
                != snapshot.can_connect_public_server)
                .then_some(snapshot.can_connect_public_server),
            can_refresh_public_servers: (baseline.can_refresh_public_servers
                != snapshot.can_refresh_public_servers)
                .then_some(snapshot.can_refresh_public_servers),
            can_search_missing_media: (baseline.can_search_missing_media
                != snapshot.can_search_missing_media)
                .then_some(snapshot.can_search_missing_media),
            can_toggle_pause: (baseline.can_toggle_pause != snapshot.can_toggle_pause)
                .then_some(snapshot.can_toggle_pause),
            can_send_chat_message: (baseline.can_send_chat_message
                != snapshot.can_send_chat_message)
                .then_some(snapshot.can_send_chat_message),
            chat_unavailable_reason: (baseline.chat_unavailable_reason
                != snapshot.chat_unavailable_reason)
                .then_some(snapshot.chat_unavailable_reason.clone()),
        }
    }

    pub(in crate::app) fn apply_to(&self, command_availability: &mut GuiCommandAvailabilityState) {
        if let Some(value) = self.can_connect_saved_server {
            command_availability.can_connect_saved_server = value;
        }
        if let Some(value) = self.can_disconnect_session {
            command_availability.can_disconnect_session = value;
        }
        if let Some(value) = self.can_connect_public_server {
            command_availability.can_connect_public_server = value;
        }
        if let Some(value) = self.can_refresh_public_servers {
            command_availability.can_refresh_public_servers = value;
        }
        if let Some(value) = self.can_search_missing_media {
            command_availability.can_search_missing_media = value;
        }
        if let Some(value) = self.can_toggle_pause {
            command_availability.can_toggle_pause = value;
        }
        if let Some(value) = self.can_send_chat_message {
            command_availability.can_send_chat_message = value;
        }
        if let Some(value) = self.chat_unavailable_reason.as_ref() {
            command_availability.chat_unavailable_reason = value.clone();
        }
    }

    pub(in crate::app) fn normalize_for_baseline(
        &mut self,
        baseline: &GuiCommandAvailabilityState,
    ) {
        self.can_save_configuration = None;
        self.can_reset_configuration = None;
        self.can_reload_configuration = None;
        if self.can_connect_saved_server == Some(baseline.can_connect_saved_server) {
            self.can_connect_saved_server = None;
        }
        if self.can_disconnect_session == Some(baseline.can_disconnect_session) {
            self.can_disconnect_session = None;
        }
        if self.can_connect_public_server == Some(baseline.can_connect_public_server) {
            self.can_connect_public_server = None;
        }
        if self.can_refresh_public_servers == Some(baseline.can_refresh_public_servers) {
            self.can_refresh_public_servers = None;
        }
        if self.can_search_missing_media == Some(baseline.can_search_missing_media) {
            self.can_search_missing_media = None;
        }
        if self.can_toggle_pause == Some(baseline.can_toggle_pause) {
            self.can_toggle_pause = None;
        }
        if self.can_send_chat_message == Some(baseline.can_send_chat_message) {
            self.can_send_chat_message = None;
        }
        if self.chat_unavailable_reason == Some(baseline.chat_unavailable_reason.clone()) {
            self.chat_unavailable_reason = None;
        }
    }
}

impl GuiFocusedConfigurationControlState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Control Focus] focused={} ({} / {}), kind={}, activations={}",
            self.id.automation_id(),
            self.id.section(),
            self.id.label(),
            self.kind.label(),
            self.activation_count
        )]
    }
}

impl GuiPublicServerEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Public Server Edit] editing_index={}, dirty={}, label={}, address={}",
            self.editing_index
                .map_or_else(|| "(new)".to_owned(), |index| index.to_string()),
            bool_label(self.is_dirty),
            self.label_buffer,
            self.address_buffer,
        )]
    }
}

impl GuiMainWindowUserEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Main Window User Edit] editing_index={}, dirty={}, username={}",
            self.editing_index,
            bool_label(self.is_dirty),
            self.username_buffer,
        )]
    }
}

impl GuiTransientNotification {
    #[cfg(test)]
    pub(in crate::app::shell_projection) fn render_line(&self) -> String {
        format!("- {}: {}", self.level.label(), self.message)
    }
}

impl GuiMediaIndexStatusState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Media Index] active={}, message={}",
            bool_label(self.active),
            self.message.as_deref().unwrap_or("(idle)")
        )]
    }
}

impl GuiTextEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Text Edit] editing={} ({} / {}), dirty={}, buffer={}",
            self.id.automation_id(),
            self.id.section(),
            self.id.label(),
            bool_label(self.is_dirty),
            self.buffer
        )]
    }
}

impl GuiPlaylistTextEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Playlist Edit] dirty={}, entries={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}

impl GuiUrlEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[URL Edit] dirty={}, lines={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}

impl GuiControlledRoomCreateSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Controlled Room Create] dirty={}, room={}",
            bool_label(self.is_dirty),
            self.room_buffer
        )]
    }
}

impl GuiControllerAuthEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Controller Auth Edit] dirty={}, room={}, password_set={}",
            bool_label(self.is_dirty),
            self.room_name,
            bool_label(!self.password_buffer.is_empty())
        )]
    }
}

impl GuiRoomHistoryEditSessionState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Room History Edit] dirty={}, entries={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}
