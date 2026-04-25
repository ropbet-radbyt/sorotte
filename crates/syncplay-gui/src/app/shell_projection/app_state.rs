use super::*;

impl SyncplayGuiShellAppState {
    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "[Shell App State] active_view={}, open_modal={}",
            self.active_view.label(),
            self.open_modal
                .map(GuiShellModal::label)
                .unwrap_or("(none)")
        )];
        lines.extend(self.selection.render_lines());
        lines.extend(self.commands.render_lines(self.pending_operation.as_ref()));
        lines.push(format!(
            "[Chat Send] pending_message={}",
            self.outgoing_chat_message.as_deref().unwrap_or("(none)")
        ));
        lines.extend(self.media_index_status.render_lines());
        lines.extend(
            self.focused_configuration_control
                .as_ref()
                .map(GuiFocusedConfigurationControlState::render_lines)
                .unwrap_or_else(|| vec!["[Control Focus] focused=(none)".to_owned()]),
        );
        lines.extend(
            self.public_server_edit_session
                .as_ref()
                .map(GuiPublicServerEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Public Server Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.main_window_user_edit_session
                .as_ref()
                .map(GuiMainWindowUserEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Main Window User Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.text_edit_session
                .as_ref()
                .map(GuiTextEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Text Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.playlist_text_edit_session
                .as_ref()
                .map(GuiPlaylistTextEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Playlist Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.playlist_url_edit_session
                .as_ref()
                .map(GuiUrlEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Playlist URL Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.media_url_edit_session
                .as_ref()
                .map(GuiUrlEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Media URL Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.controlled_room_create_session
                .as_ref()
                .map(GuiControlledRoomCreateSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Controlled Room Create] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.controller_auth_edit_session
                .as_ref()
                .map(GuiControllerAuthEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Controller Auth Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.room_history_edit_session
                .as_ref()
                .map(GuiRoomHistoryEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Room History Edit] editing=(none)".to_owned()]),
        );
        lines.push(format!(
            "[Player Setup] status={}",
            self.player_setup_issue
                .as_ref()
                .map(|issue| issue.kind.label())
                .unwrap_or("(none)")
        ));
        if let Some(issue) = self.player_setup_issue.as_ref() {
            lines.push(format!("- detail: {}", issue.message));
        }
        lines.push(format!(
            "[Notifications] count={}",
            self.notifications.len()
        ));
        if self.notifications.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for notification in &self.notifications {
                lines.push(notification.render_line());
            }
        }
        lines.extend(self.validation.render_lines());
        lines.extend(self.configuration.render_lines());
        lines.extend(self.main_window.render_lines());
        lines.extend(self.menus.render_lines());
        lines.extend(self.public_servers.render_lines());
        lines.extend(self.media_search.render_lines());
        lines.push(
            "syncplay-gui now has a unified shell app state and action reducer for future native widget binding."
                .to_owned(),
        );
        lines
    }
}
