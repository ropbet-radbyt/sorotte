use super::*;

impl SyncplayGuiShellAppState {
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
                                label_width: 160.0,
                                min_field_width: 220.0,
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

        let stream_support_panel = self.stream_helper_status_available().then(|| {
            let mut content = vec![
                GuiWidgetNode::leaf(
                    "config-stream-support:title",
                    "Title",
                    GuiWidgetKind::Status,
                    Some(self.stream_helper_status_title().to_owned()),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-stream-support:summary",
                    "Summary",
                    GuiWidgetKind::Status,
                    Some(self.stream_helper_status_summary()),
                    true,
                    false,
                ),
            ];
            if self.stream_helper_remediation.active {
                content.push(GuiWidgetNode::leaf(
                    "config-stream-support:remediation",
                    "Remediation",
                    GuiWidgetKind::Status,
                    self.stream_helper_remediation.label.clone(),
                    true,
                    false,
                ));
                content.push(GuiWidgetNode::leaf(
                    "config-stream-support:remediation-progress",
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
                    content.push(GuiWidgetNode::leaf(
                        "config-stream-support:remediation-detail",
                        "Remediation Detail",
                        GuiWidgetKind::Status,
                        Some(detail.clone()),
                        true,
                        false,
                    ));
                }
            }
            content.push(GuiWidgetNode::layout(
                "config-stream-support:actions",
                "Stream Support Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![GuiWidgetNode::leaf(
                    "config-stream-support:manage",
                    "Manage Stream Support",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    self.open_modal == Some(GuiShellModal::StreamSupport),
                )],
            ));
            GuiWidgetNode::branch(
                "config-stream-support",
                "Stream Support",
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
                        "config-command:edit-room-history",
                        "Edit Room History",
                        GuiWidgetKind::Button,
                        None,
                        self.pending_operation.is_none(),
                        false,
                    ),
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

        let mut overview_children = vec![GuiWidgetNode::layout(
            "configuration:sections",
            "Configuration Sections",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 420.0,
                max_columns: 3,
            },
            section_cards.clone(),
        )];
        if let Some(panel) = room_history_panel.clone() {
            overview_children.push(panel);
        }
        let overview_content = GuiWidgetNode::layout(
            "configuration:content:overview",
            "Overview Content",
            GuiLayoutMode::Stack,
            overview_children,
        );

        let connection_content = GuiWidgetNode::layout(
            "configuration:content:connection",
            "Connection Content",
            GuiLayoutMode::Stack,
            [section_card("Connection"), room_history_panel.clone()]
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
            "Configuration",
            GuiLayoutMode::Stack,
            player_setup_panel
                .into_iter()
                .chain(stream_support_panel)
                .chain([
                    commands_panel,
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
                ])
                .collect(),
        )
    }
}
