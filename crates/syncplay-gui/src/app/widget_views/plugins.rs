use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn plugins_widget_tree(&self) -> GuiWidgetNode {
        let stream_support_selected = true;
        let plugin_list = GuiWidgetNode::branch(
            "plugins:list",
            "Plugins",
            GuiWidgetKind::Panel,
            vec![GuiWidgetNode::leaf(
                "plugins:list:stream-support",
                "Stream Support",
                GuiWidgetKind::ListItem,
                Some(self.stream_helper.health.label().to_owned()),
                true,
                stream_support_selected,
            )],
        );

        let detail = self.stream_support_plugin_detail_widget_tree();
        GuiWidgetNode::layout(
            "plugins-root",
            "Plugins",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 260.0,
                max_columns: 3,
            },
            vec![plugin_list, detail.with_span(2)],
        )
    }

    fn stream_support_plugin_detail_widget_tree(&self) -> GuiWidgetNode {
        let mut children = Vec::new();
        if let Some(alert) = self.stream_support_plugin_alert_widget_tree() {
            children.push(alert);
        }

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:status",
            "Stream Support Status",
            GuiLayoutMode::KeyValueGrid {
                min_pair_width: 260.0,
            },
            self.stream_support_plugin_status_rows(),
        ));

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:actions",
            "Stream Support Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 150.0,
            },
            vec![
                GuiWidgetNode::leaf(
                    "plugins:stream-support:install",
                    "Install Helper",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("install"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:import-downloader",
                    "Import yt-dlp",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("import"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:import-js-runtime",
                    "Import Deno",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("import"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:open-location",
                    "Open Install Location",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_helper.open_install_location_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:recheck",
                    "Recheck Support",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("recheck"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:retry",
                    "Retry URL",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("retry"),
                    false,
                ),
            ],
        ));

        GuiWidgetNode::branch(
            "plugins:stream-support",
            "Stream Support",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn stream_support_plugin_status_rows(&self) -> Vec<GuiWidgetNode> {
        let mut rows = vec![
            GuiWidgetNode::leaf(
                "plugins:stream-support:title",
                "Title",
                GuiWidgetKind::Status,
                Some(self.stream_helper_status_title().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:summary",
                "Summary",
                GuiWidgetKind::Status,
                Some(self.stream_helper_status_summary()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:health",
                "Health",
                GuiWidgetKind::Status,
                Some(self.stream_helper.health.label().to_owned()),
                true,
                false,
            ),
        ];

        if let Some(install_location) = self.stream_helper.install_location.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:install-location",
                "Install Location",
                GuiWidgetKind::Status,
                Some(install_location.clone()),
                true,
                false,
            ));
        }
        if let Some(downloader_status) = self.stream_helper.downloader_status.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:downloader-status",
                "yt-dlp",
                GuiWidgetKind::Status,
                Some(downloader_status.clone()),
                true,
                false,
            ));
        }
        if let Some(js_runtime_status) = self.stream_helper.js_runtime_status.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:js-runtime-status",
                "Deno",
                GuiWidgetKind::Status,
                Some(js_runtime_status.clone()),
                true,
                false,
            ));
        }
        if let Some(target) = self.stream_helper.target.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:target",
                "Target",
                GuiWidgetKind::Status,
                Some(target.clone()),
                true,
                false,
            ));
        }
        if self.stream_helper_remediation.active {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:remediation",
                "Remediation",
                GuiWidgetKind::Status,
                self.stream_helper_remediation.label.clone(),
                true,
                false,
            ));
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:remediation-progress",
                "Progress",
                GuiWidgetKind::Status,
                Some(format!(
                    "{:.0}%",
                    self.stream_helper_remediation.progress_fraction * 100.0
                )),
                true,
                false,
            ));
            if let Some(detail) = self.stream_helper_remediation.detail.as_ref() {
                rows.push(GuiWidgetNode::leaf(
                    "plugins:stream-support:remediation-detail",
                    "Remediation Detail",
                    GuiWidgetKind::Status,
                    Some(detail.clone()),
                    true,
                    false,
                ));
            }
        }

        rows
    }

    fn stream_support_plugin_alert_widget_tree(&self) -> Option<GuiWidgetNode> {
        if self.stream_helper.health == GuiStreamHelperHealth::Healthy
            && !self.stream_helper_remediation.active
        {
            return None;
        }

        let level = match self.stream_helper.health {
            GuiStreamHelperHealth::Broken => GuiTransientNotificationLevel::Error,
            GuiStreamHelperHealth::Healthy if self.stream_helper_remediation.active => {
                GuiTransientNotificationLevel::Success
            }
            _ => GuiTransientNotificationLevel::Warning,
        };
        let message = self
            .stream_helper
            .message
            .clone()
            .unwrap_or_else(|| self.stream_helper_status_summary());

        let mut children = vec![
            GuiWidgetNode::leaf(
                "plugins:stream-support:alert:level",
                "Level",
                GuiWidgetKind::Status,
                Some(level.label().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:alert:message",
                "Message",
                GuiWidgetKind::Status,
                Some(message),
                true,
                false,
            ),
        ];

        let mut alert_actions = Vec::new();
        if self.stream_helper.install_supported {
            alert_actions.push(GuiWidgetNode::leaf(
                "plugins:stream-support:alert:install",
                "Install Helper",
                GuiWidgetKind::Button,
                None,
                self.stream_support_plugin_action_enabled("install"),
                false,
            ));
        }
        if self.stream_helper.retry_available {
            alert_actions.push(GuiWidgetNode::leaf(
                "plugins:stream-support:alert:retry",
                "Retry URL",
                GuiWidgetKind::Button,
                None,
                self.stream_support_plugin_action_enabled("retry"),
                false,
            ));
        }
        alert_actions.push(GuiWidgetNode::leaf(
            "plugins:stream-support:alert:recheck",
            "Recheck Support",
            GuiWidgetKind::Button,
            None,
            self.stream_support_plugin_action_enabled("recheck"),
            false,
        ));

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:alert:actions",
            "Alert Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 150.0,
            },
            alert_actions,
        ));

        Some(GuiWidgetNode::branch(
            "plugins:stream-support:alert",
            level.label(),
            GuiWidgetKind::Panel,
            children,
        ))
    }

    fn stream_support_plugin_action_enabled(&self, action: &str) -> bool {
        match action {
            "install" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.install_supported
            }
            "import" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.integration_supported
            }
            "recheck" => self.pending_operation.is_none() && !self.stream_helper_remediation.active,
            "retry" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.retry_available
            }
            _ => false,
        }
    }
}
