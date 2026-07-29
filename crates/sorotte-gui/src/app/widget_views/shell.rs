use super::*;

impl SorotteGuiShellAppState {
    pub(crate) fn update_indicator_widget_tree(&self) -> GuiWidgetNode {
        let model = self
            .update_check
            .indicator_model(Some(self.runtime_language_tag_legacy_compatible()));
        let mut node = GuiWidgetNode::branch(
            "shell:update-indicator",
            model.title.clone(),
            GuiWidgetKind::Button,
            vec![
                GuiWidgetNode::leaf(
                    "shell:update-indicator:title",
                    "Update Indicator",
                    GuiWidgetKind::Status,
                    Some(model.title),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:update-indicator:detail",
                    "Update Detail",
                    GuiWidgetKind::Status,
                    Some(model.detail),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:update-indicator:tone",
                    "Update Tone",
                    GuiWidgetKind::Status,
                    Some(model.tone.label().to_owned()),
                    true,
                    false,
                ),
            ],
        );
        node.enabled = model.enabled;
        node
    }

    pub(crate) fn command_status_widget_tree(&self) -> GuiWidgetNode {
        let items = [
            ("busy", "Busy", self.pending_operation.is_some()),
            ("save", "Save", self.commands.can_save_configuration),
            (
                "discard",
                "Discard changes",
                self.commands.can_reset_configuration,
            ),
            ("reload", "Reload", self.commands.can_reload_configuration),
            (
                "connect-saved-server",
                "Connect Saved Server",
                self.commands.can_connect_saved_server,
            ),
            (
                "disconnect-session",
                "Disconnect Session",
                self.commands.can_disconnect_session,
            ),
            (
                "connect-public-server",
                "Connect Public Server",
                self.commands.can_connect_public_server,
            ),
            (
                "refresh-public-servers",
                "Refresh Public Servers",
                self.commands.can_refresh_public_servers,
            ),
            (
                "search-missing-media",
                "Search Missing Media",
                self.commands.can_search_missing_media,
            ),
            (
                "toggle-pause",
                "Toggle Pause",
                self.commands.can_toggle_pause,
            ),
            (
                "send-chat-message",
                "Send Chat Message",
                self.commands.can_send_chat_message,
            ),
        ]
        .into_iter()
        .map(|(id, label, enabled)| {
            GuiWidgetNode::leaf(
                format!("shell:command:{id}"),
                label,
                GuiWidgetKind::Status,
                Some(if id == "busy" {
                    bool_label(enabled).to_owned()
                } else if enabled {
                    "enabled".to_owned()
                } else {
                    "disabled".to_owned()
                }),
                true,
                false,
            )
        })
        .collect();

        GuiWidgetNode::branch("shell:commands", "Commands", GuiWidgetKind::List, items)
    }

    pub(crate) fn validation_widget_tree(&self) -> GuiWidgetNode {
        let mut children = vec![
            GuiWidgetNode::leaf(
                "shell:validation:status",
                "Status",
                GuiWidgetKind::Status,
                Some(if self.validation.issues.is_empty() {
                    "clean".to_owned()
                } else {
                    format!("{} issue(s)", self.validation.issues.len())
                }),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "shell:validation:last-action-error",
                "Last Action Error",
                GuiWidgetKind::Status,
                Some(
                    self.validation
                        .last_action_error
                        .clone()
                        .unwrap_or("(none)".to_owned()),
                ),
                true,
                false,
            ),
        ];
        children.extend(
            self.validation
                .issues
                .iter()
                .enumerate()
                .map(|(index, issue)| {
                    GuiWidgetNode::leaf(
                        issue.setting_id.map_or_else(
                            || format!("shell:validation:issue:{index}"),
                            |id| format!("{}.validation", id.automation_id()),
                        ),
                        format!("{} / {}", issue.scope, issue.label),
                        GuiWidgetKind::ListItem,
                        Some(issue.message.clone()),
                        true,
                        false,
                    )
                }),
        );
        GuiWidgetNode::branch(
            "shell:validation",
            "Validation",
            GuiWidgetKind::List,
            children,
        )
    }

    pub(crate) fn shell_widget_tree(&self) -> GuiWidgetNode {
        let mut configuration = self.configuration_widget_tree();
        configuration.selected = self.active_view == GuiShellView::Setup;

        let mut main_window = self.main_window_widget_tree();
        main_window.selected = self.active_view == GuiShellView::Room;

        let mut plugins = self.plugins_widget_tree();
        plugins.selected = self.active_view == GuiShellView::Plugins;

        let menus = self.menu_dialog_widget_tree();

        let notifications = GuiWidgetNode::branch(
            "shell:notifications",
            "Notifications",
            GuiWidgetKind::List,
            self.notifications
                .iter()
                .enumerate()
                .map(|(index, notification)| {
                    GuiWidgetNode::leaf(
                        format!("shell:notification:{index}"),
                        notification.level.label(),
                        GuiWidgetKind::ListItem,
                        Some(notification.message.clone()),
                        true,
                        false,
                    )
                })
                .collect(),
        );

        GuiWidgetNode::branch(
            "shell-root",
            "Sorotte GUI",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "shell:active-view",
                    "Active View",
                    GuiWidgetKind::Status,
                    Some(self.active_view.label().to_owned()),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:open-modal",
                    "Open Modal",
                    GuiWidgetKind::Status,
                    Some(
                        self.open_modal
                            .map(GuiShellModal::label)
                            .unwrap_or("(none)")
                            .to_owned(),
                    ),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:pending-operation",
                    "Pending Operation",
                    GuiWidgetKind::Status,
                    Some(
                        self.pending_operation
                            .as_ref()
                            .map(|pending| pending.kind.label())
                            .unwrap_or("(none)")
                            .to_owned(),
                    ),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:media-index-active",
                    "Media Index Active",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.media_index_status.active).to_owned()),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:media-index-status",
                    "Media Index Status",
                    GuiWidgetKind::Status,
                    Some(
                        self.media_index_status
                            .message
                            .clone()
                            .unwrap_or_else(|| "(idle)".to_owned()),
                    ),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:stream-helper-remediation-active",
                    "Stream Helper Remediation Active",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.stream_helper_remediation.active).to_owned()),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:stream-helper-remediation-label",
                    "Stream Helper Remediation",
                    GuiWidgetKind::Status,
                    Some(
                        self.stream_helper_remediation
                            .label
                            .clone()
                            .unwrap_or_else(|| "(idle)".to_owned()),
                    ),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:stream-helper-remediation-detail",
                    "Stream Helper Remediation Detail",
                    GuiWidgetKind::Status,
                    Some(
                        self.stream_helper_remediation
                            .detail
                            .clone()
                            .unwrap_or_else(|| "(idle)".to_owned()),
                    ),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "shell:stream-helper-remediation-progress",
                    "Stream Helper Remediation Progress",
                    GuiWidgetKind::Status,
                    Some(format!(
                        "{:.3}",
                        self.stream_helper_remediation.progress_fraction
                    )),
                    true,
                    false,
                ),
                self.update_indicator_widget_tree(),
                self.shell_modal_widget_tree(),
                self.command_status_widget_tree(),
                self.validation_widget_tree(),
                notifications,
                main_window,
                configuration,
                plugins,
                menus,
            ],
        )
    }

    pub(crate) fn render_shell_widgets(&self, renderer: &mut impl GuiWidgetRenderer) {
        self.shell_widget_tree().render_with(renderer);
    }
}
