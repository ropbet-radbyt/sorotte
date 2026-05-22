use super::*;

impl SorotteGuiShellAppState {
    pub(crate) fn configuration_widget_tree(&self) -> GuiWidgetNode {
        let busy = self.pending_operation.is_some();
        let player_arguments_enabled = self
            .configuration
            .settings
            .player_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let section_cards =
            self.configuration
                .sections
                .iter()
                .map(|section| {
                    let controls = section
                        .controls
                        .iter()
                        .map(|control| {
                            let active_edit_session =
                                self.text_edit_session.as_ref().and_then(|session| {
                                    ((session.section == section.title)
                                        && (session.label == control.label))
                                        .then_some(session)
                                });
                            let focused = self.focused_configuration_control.as_ref().is_some_and(
                                |focused| {
                                    focused.section == section.title
                                        && focused.label == control.label
                                },
                            );
                            let value = active_edit_session
                                .map(|session| session.buffer.clone())
                                .unwrap_or_else(|| control.value.clone());
                            let enabled = control.kind.is_editable()
                                && !busy
                                && !((section.title == "Connection"
                                    && control.label == "Player Arguments")
                                    && !player_arguments_enabled);
                            GuiWidgetNode::leaf(
                                format!("config:{}:{}", section.title, control.label),
                                control.label,
                                control.kind.widget_kind(),
                                Some(value),
                                enabled,
                                focused,
                            )
                        })
                        .collect::<Vec<_>>();

                    let panel = GuiWidgetNode::branch(
                        format!("config-section:{}", section.title),
                        section.title,
                        GuiWidgetKind::Panel,
                        vec![GuiWidgetNode::layout(
                            format!("config-section:{}:form", section.title),
                            format!("{} Form", section.title),
                            GuiLayoutMode::FormGrid {
                                label_width: 132.0,
                                min_field_width: 160.0,
                            },
                            controls,
                        )],
                    );
                    if section.title == "Connection" {
                        panel.with_span(2)
                    } else {
                        panel
                    }
                })
                .collect::<Vec<_>>();

        let player_setup_panel = self.player_setup_issue.as_ref().map(|issue| {
            let mut content = vec![
                GuiWidgetNode::leaf(
                    "config-player-setup:title",
                    "Title",
                    GuiWidgetKind::Status,
                    self.player_setup_issue_title().map(str::to_owned),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-player-setup:summary",
                    "Summary",
                    GuiWidgetKind::Status,
                    self.player_setup_issue_summary().map(str::to_owned),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-player-setup:detail",
                    "Detail",
                    GuiWidgetKind::Status,
                    Some(issue.message.clone()),
                    true,
                    false,
                ),
            ];
            if self.connect_blocked_by_player_setup_issue() {
                content.push(GuiWidgetNode::leaf(
                    "config-player-setup:blocking",
                    "Connect Status",
                    GuiWidgetKind::Status,
                    self.player_setup_connect_block_message(),
                    true,
                    false,
                ));
            }
            content.push(GuiWidgetNode::layout(
                "config-player-setup:actions",
                "Player Setup Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "config-player-setup:autodetect",
                        "Auto-detect mpv",
                        GuiWidgetKind::Button,
                        None,
                        self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-player-setup:choose-path",
                        "Choose mpv.exe",
                        GuiWidgetKind::Button,
                        None,
                        self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-player-setup:retry",
                        "Retry mpv",
                        GuiWidgetKind::Button,
                        None,
                        self.player_setup_retry_available(),
                        false,
                    ),
                ],
            ));
            GuiWidgetNode::branch(
                "config-player-setup",
                "mpv Setup",
                GuiWidgetKind::Panel,
                content,
            )
        });

        let commands_panel = GuiWidgetNode::branch(
            "config-commands",
            "Commands",
            GuiWidgetKind::Panel,
            vec![GuiWidgetNode::layout(
                "config-commands:buttons",
                "Command Buttons",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "config-command:connect",
                        self.saved_session_connect_button_label(),
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_connect_saved_server && self.validation.issues.is_empty(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:disconnect",
                        "Disconnect",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_disconnect_session,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:save",
                        "Save",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_save_configuration,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:reset",
                        "Reset",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_reset_configuration,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:reload",
                        "Reload",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_reload_configuration,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:edit-room-history",
                        "Edit Room History",
                        GuiWidgetKind::Button,
                        None,
                        self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "config-command:clear-gui-data",
                        "Clear GUI Data",
                        GuiWidgetKind::Button,
                        None,
                        self.pending_operation.is_none(),
                        false,
                    ),
                ],
            )],
        );

        let room_history_panel = self.room_history_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "room-history:edit-session",
                "Room History Edit",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "room-history:edit:entries",
                        "Room History Entries",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "room-history:edit:actions",
                        "Room History Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "room-history:edit:commit",
                                "Save Room History",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "room-history:edit:cancel",
                                "Cancel Room History Edit",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                        ],
                    ),
                ],
            )
        });

        let section_card = |title: &str| {
            section_cards
                .iter()
                .find(|card| card.label == title)
                .cloned()
        };
        let public_servers_panel = self.public_server_widget_tree();
        let media_search_panel = self.media_search_widget_tree();

        let mut overview_children = vec![GuiWidgetNode::layout(
            "configuration:sections",
            "Configuration Sections",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 420.0,
                max_columns: 3,
            },
            section_cards.clone(),
        )];
        overview_children.push(GuiWidgetNode::layout(
            "configuration:setup-workflows",
            "Setup Workflows",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 360.0,
                max_columns: 2,
            },
            vec![public_servers_panel.clone(), media_search_panel.clone()],
        ));
        if let Some(panel) = room_history_panel.clone() {
            overview_children.push(panel);
        }
        let overview_content = GuiWidgetNode::layout(
            "configuration:content:overview",
            "Overview Content",
            GuiLayoutMode::Stack,
            overview_children,
        );

        let connection_card = section_card("Connection").map(|panel| panel.with_span(1));
        let connection_tools = GuiWidgetNode::layout(
            "configuration:connection-tools",
            "Connection Tools",
            GuiLayoutMode::Stack,
            [Some(media_search_panel.clone()), section_card("Desync")]
                .into_iter()
                .flatten()
                .collect(),
        );
        let connection_content = GuiWidgetNode::layout(
            "configuration:content:connection",
            "Connection Content",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 260.0,
                max_columns: 3,
            },
            [
                connection_card,
                Some(public_servers_panel.clone()),
                Some(connection_tools),
                room_history_panel.clone(),
            ]
            .into_iter()
            .flatten()
            .collect(),
        );

        let playback_search_content = GuiWidgetNode::layout(
            "configuration:content:playback-search",
            "Playback And Search Content",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 420.0,
                max_columns: 3,
            },
            ["Readiness", "Desync", "Media Search"]
                .into_iter()
                .filter_map(section_card)
                .chain([media_search_panel.clone()])
                .collect(),
        );

        let privacy_chat_content = GuiWidgetNode::layout(
            "configuration:content:privacy-chat",
            "Privacy And Chat Content",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 420.0,
                max_columns: 2,
            },
            ["Privacy", "Chat"]
                .into_iter()
                .filter_map(section_card)
                .collect(),
        );

        let storage_change_enabled =
            self.pending_operation.is_none() && !self.config_storage.external_override_active;
        let storage_location_panel = GuiWidgetNode::branch(
            "config-storage",
            "Storage Location",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::layout(
                    "config-storage:paths",
                    "Storage Paths",
                    GuiLayoutMode::FormGrid {
                        label_width: 132.0,
                        min_field_width: 160.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            "config-storage:config-path",
                            "Config File",
                            GuiWidgetKind::ReadOnly,
                            self.config_storage.config_path.clone(),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:root",
                            "Storage Root",
                            GuiWidgetKind::ReadOnly,
                            self.config_storage.storage_root.clone(),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:source",
                            "Source",
                            GuiWidgetKind::ReadOnly,
                            Some(self.config_storage.source_label.clone()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:default-root",
                            "Default Root",
                            GuiWidgetKind::ReadOnly,
                            self.config_storage.default_storage_root.clone(),
                            true,
                            false,
                        ),
                    ],
                ),
                GuiWidgetNode::layout(
                    "config-storage:actions",
                    "Storage Actions",
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            "config-storage:root:browse",
                            "Browse",
                            GuiWidgetKind::Button,
                            None,
                            storage_change_enabled,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:root:default",
                            "Use Default",
                            GuiWidgetKind::Button,
                            None,
                            storage_change_enabled,
                            false,
                        ),
                    ],
                ),
            ],
        );

        let interface_system_content = GuiWidgetNode::layout(
            "configuration:content:interface-system",
            "Interface And System Content",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 420.0,
                max_columns: 2,
            },
            ["OSD", "System"]
                .into_iter()
                .filter_map(section_card)
                .chain([storage_location_panel])
                .collect(),
        );

        let selected_content = match self.selected_configuration_tab {
            GuiConfigurationTab::Overview => overview_content,
            GuiConfigurationTab::Connection => connection_content,
            GuiConfigurationTab::PlaybackSearch => playback_search_content,
            GuiConfigurationTab::PrivacyChat => privacy_chat_content,
            GuiConfigurationTab::InterfaceSystem => interface_system_content,
        };

        GuiWidgetNode::layout(
            "configuration-root",
            "Setup",
            GuiLayoutMode::Stack,
            player_setup_panel
                .into_iter()
                .chain(self.setup_action_alert_widget_tree())
                .chain([
                    GuiWidgetNode::layout(
                        "configuration:tabs",
                        "Configuration Tabs",
                        GuiLayoutMode::TabStrip {
                            min_tab_width: 132.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "configuration:tab:overview",
                                "Overview",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                self.selected_configuration_tab == GuiConfigurationTab::Overview,
                            ),
                            GuiWidgetNode::leaf(
                                "configuration:tab:connection",
                                "Connection",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                self.selected_configuration_tab == GuiConfigurationTab::Connection,
                            ),
                            GuiWidgetNode::leaf(
                                "configuration:tab:playback-search",
                                "Playback & Search",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                self.selected_configuration_tab
                                    == GuiConfigurationTab::PlaybackSearch,
                            ),
                            GuiWidgetNode::leaf(
                                "configuration:tab:privacy-chat",
                                "Privacy & Chat",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                self.selected_configuration_tab == GuiConfigurationTab::PrivacyChat,
                            ),
                            GuiWidgetNode::leaf(
                                "configuration:tab:interface-system",
                                "Interface & System",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                self.selected_configuration_tab
                                    == GuiConfigurationTab::InterfaceSystem,
                            ),
                        ],
                    ),
                    selected_content,
                    commands_panel,
                ])
                .collect(),
        )
    }

    fn setup_action_alert_widget_tree(&self) -> Option<GuiWidgetNode> {
        let (level, message) = if let Some(error) = self.validation.last_action_error.as_ref() {
            (GuiTransientNotificationLevel::Error, error.clone())
        } else {
            let notification = self.notifications.iter().rev().find(|notification| {
                !matches!(notification.level, GuiTransientNotificationLevel::Info)
            })?;
            (notification.level, notification.message.clone())
        };

        let mut children = vec![
            GuiWidgetNode::leaf(
                "configuration:alert:close",
                "Dismiss Alert",
                GuiWidgetKind::Button,
                None,
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "configuration:alert:level",
                "Level",
                GuiWidgetKind::Status,
                Some(level.label().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "configuration:alert:message",
                "Message",
                GuiWidgetKind::Status,
                Some(message.clone()),
                true,
                false,
            ),
        ];

        let lower_message = message.to_ascii_lowercase();
        let mut actions = Vec::new();
        if lower_message.contains("player") || lower_message.contains("mpv") {
            actions.push(GuiWidgetNode::leaf(
                "configuration:alert:fix-player-path",
                "Fix Player Path",
                GuiWidgetKind::Button,
                None,
                self.pending_operation.is_none(),
                false,
            ));
        }
        if !actions.is_empty() {
            children.push(GuiWidgetNode::layout(
                "configuration:alert:actions",
                "Alert Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 150.0,
                },
                actions,
            ));
        }

        Some(GuiWidgetNode::branch(
            "configuration:action-alert",
            level.label(),
            GuiWidgetKind::Panel,
            children,
        ))
    }
}
