use super::*;

impl SorotteGuiShellAppState {
    pub(crate) fn configuration_widget_tree(&self) -> GuiWidgetNode {
        let busy = self.pending_operation.is_some();
        let changed_setting_ids = self
            .configuration
            .changed_setting_ids_against(&self.saved_configuration);
        let mut apply_requirements = changed_setting_ids
            .iter()
            .map(|id| id.apply_requirement())
            .collect::<std::collections::BTreeSet<_>>();
        if self.pending_config_storage_target.is_some() {
            apply_requirements.insert(GuiSettingApplyRequirement::OnSave);
        }
        if self.has_unsaved_configuration_changes() && apply_requirements.is_empty() {
            apply_requirements.insert(GuiSettingApplyRequirement::OnSave);
        }
        let resolved_draft =
            FirstRunConfigurationDialogState::from_stored_settings(&self.configuration.settings);
        let resolved_persisted =
            FirstRunConfigurationDialogState::from_stored_settings(&self.saved_configuration);
        let player_arguments_enabled = self
            .configuration
            .settings
            .player_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let section_cards = self
            .configuration
            .sections
            .iter()
            .map(|section| {
                let section_id = section
                    .controls
                    .first()
                    .map(|control| control.id.section_automation_id())
                    .expect("settings sections contain at least one typed control");
                let mut controls = section
                    .controls
                    .iter()
                    .map(|control| {
                        let active_edit_session = self
                            .text_edit_session
                            .as_ref()
                            .and_then(|session| (session.id == control.id).then_some(session));
                        let focused = self
                            .focused_configuration_control
                            .as_ref()
                            .is_some_and(|focused| focused.id == control.id);
                        let value = active_edit_session
                            .map(|session| session.buffer.expose_for_ui().to_owned())
                            .unwrap_or_else(|| control.value.clone());
                        let enabled = control.kind.is_editable()
                            && !busy
                            && (control.id != SettingId::PlayerArguments
                                || player_arguments_enabled)
                            && (control.id != SettingId::ConnectionServerPassword
                                || matches!(
                                    &self.configuration.server_password,
                                    SecretDraft::Replace(_)
                                ));
                        GuiWidgetNode::leaf(
                            control.id.automation_id(),
                            control.label,
                            control.kind.widget_kind(),
                            Some(value),
                            enabled,
                            focused,
                        )
                    })
                    .collect::<Vec<_>>();

                if section
                    .controls
                    .iter()
                    .any(|control| control.id == SettingId::ConnectionServerPassword)
                {
                    let changing =
                        matches!(&self.configuration.server_password, SecretDraft::Replace(_));
                    controls.extend([
                        GuiWidgetNode::leaf(
                            "settings.connection.server_password.status",
                            "Password status",
                            GuiWidgetKind::ReadOnly,
                            Some(if self.configuration.server_password_is_configured() {
                                "Password is configured.".to_owned()
                            } else {
                                "No password is configured.".to_owned()
                            }),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "settings.connection.server_password.change",
                            if changing {
                                "Cancel password change"
                            } else {
                                "Change password"
                            },
                            GuiWidgetKind::Button,
                            None,
                            !busy,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "settings.connection.server_password.remove",
                            "Remove password",
                            GuiWidgetKind::Button,
                            None,
                            !busy && self.configuration.server_password_is_configured(),
                            false,
                        ),
                    ]);
                }

                for id in [
                    SettingId::PlaybackUnpauseAction,
                    SettingId::PlaybackAutoplayMinUsers,
                ] {
                    if section.controls.iter().any(|control| control.id == id) {
                        let origin = match id {
                            SettingId::PlaybackUnpauseAction => resolved_draft
                                .readiness
                                .unpause_action
                                .origin_against_persisted(
                                    &resolved_persisted.readiness.unpause_action,
                                ),
                            SettingId::PlaybackAutoplayMinUsers => resolved_draft
                                .readiness
                                .autoplay_min_users
                                .origin_against_persisted(
                                    &resolved_persisted.readiness.autoplay_min_users,
                                ),
                            _ => unreachable!("only resolved settings have origin rows"),
                        };
                        controls.push(GuiWidgetNode::leaf(
                            format!("{}.origin", id.automation_id()),
                            format!("{} source", id.label()),
                            GuiWidgetKind::ReadOnly,
                            Some(
                                match origin {
                                    GuiSettingValueOrigin::StoredOverride => "Stored override",
                                    GuiSettingValueOrigin::ApplicationDefault => {
                                        "Using application default"
                                    }
                                    GuiSettingValueOrigin::DraftChange => "Unsaved change",
                                }
                                .to_owned(),
                            ),
                            true,
                            false,
                        ));
                    }
                }

                let panel = GuiWidgetNode::branch(
                    section_id,
                    section.title,
                    GuiWidgetKind::Panel,
                    vec![GuiWidgetNode::layout(
                        format!("{section_id}.form"),
                        format!("{} Form", section.title),
                        GuiLayoutMode::FormGrid {
                            label_width: 132.0,
                            min_field_width: 160.0,
                        },
                        controls,
                    )],
                );
                if section_id == "settings.section.connection" {
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
                        self.player_setup_retry_label(),
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

        let mut command_buttons = vec![
            GuiWidgetNode::leaf(
                "config-command:connect-once",
                "Connect once",
                GuiWidgetKind::Button,
                None,
                self.commands.can_connect_saved_server,
                false,
            ),
            GuiWidgetNode::leaf(
                "config-command:save-and-connect",
                "Save & connect",
                GuiWidgetKind::Button,
                None,
                self.commands.can_connect_saved_server
                    && self.validation.issues.is_empty()
                    && self.pending_config_storage_target.is_none(),
                false,
            ),
        ];
        if self.has_unsaved_configuration_changes() {
            command_buttons.extend([
                GuiWidgetNode::leaf(
                    "config-command:discard",
                    "Discard changes",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_reset_configuration,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:save",
                    "Save changes",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_save_configuration,
                    false,
                ),
            ]);
        }
        command_buttons.extend([
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
                self.pending_operation.is_none() && !self.clear_gui_data_confirmation_visible,
                false,
            ),
            GuiWidgetNode::leaf(
                "config-command:clear-gui-data",
                "Clear GUI Data",
                GuiWidgetKind::Button,
                None,
                self.pending_operation.is_none() && !self.clear_gui_data_confirmation_visible,
                false,
            ),
        ]);

        let change_summary = self.has_unsaved_configuration_changes().then(|| {
            let mut children = vec![GuiWidgetNode::leaf(
                "configuration:changes:count",
                "Unsaved changes",
                GuiWidgetKind::Status,
                Some(
                    (changed_setting_ids.len()
                        + usize::from(self.pending_config_storage_target.is_some()))
                    .max(1)
                    .to_string(),
                ),
                true,
                false,
            )];
            children.extend(apply_requirements.iter().map(|requirement| {
                GuiWidgetNode::leaf(
                    requirement.automation_id(),
                    "Apply requirement",
                    GuiWidgetKind::Status,
                    Some(requirement.label().to_owned()),
                    true,
                    false,
                )
            }));
            children.extend(changed_setting_ids.iter().map(|id| {
                GuiWidgetNode::leaf(
                    format!("{}.apply-requirement", id.automation_id()),
                    format!("{} apply requirement", id.label()),
                    GuiWidgetKind::ReadOnly,
                    Some(id.apply_requirement().label().to_owned()),
                    true,
                    false,
                )
            }));
            GuiWidgetNode::branch(
                "configuration:changes",
                "Pending changes",
                GuiWidgetKind::Panel,
                children,
            )
        });

        let commands_panel = GuiWidgetNode::branch(
            "config-commands",
            "Commands",
            GuiWidgetKind::Panel,
            change_summary
                .into_iter()
                .chain([GuiWidgetNode::layout(
                    "config-commands:buttons",
                    "Command Buttons",
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    command_buttons,
                )])
                .collect(),
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

        let section_card = |anchor: SettingId| {
            section_cards
                .iter()
                .find(|card| card.id == anchor.section_automation_id())
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

        let connection_card =
            section_card(SettingId::ConnectionHost).map(|panel| panel.with_span(1));
        let connection_tools = GuiWidgetNode::layout(
            "configuration:connection-tools",
            "Connection Tools",
            GuiLayoutMode::Stack,
            [
                Some(media_search_panel.clone()),
                section_card(SettingId::SyncRewindOnDesync),
            ]
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
            [
                SettingId::PlaybackReadyAtStart,
                SettingId::SyncRewindOnDesync,
                SettingId::StreamingQuality,
                SettingId::MediaLibraryDirectories,
            ]
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
            [SettingId::PrivacyFilename, SettingId::ChatInputEnabled]
                .into_iter()
                .filter_map(section_card)
                .collect(),
        );

        let storage_change_enabled =
            self.pending_operation.is_none() && !self.config_storage.external_override_active;
        let mut storage_config_path = self.config_storage.config_path.clone();
        let mut storage_root = self.config_storage.storage_root.clone();
        let mut storage_source = self.config_storage.source_label.clone();
        if let Some(target) = self.pending_config_storage_target.as_ref() {
            let (target_root, target_source) = match target {
                GuiConfigStorageChangeTarget::CustomRoot(root) => {
                    (Some(root.clone()), "selected custom root (save to apply)")
                }
                GuiConfigStorageChangeTarget::DefaultRoot => (
                    self.config_storage.default_storage_root.clone(),
                    "selected default root (save to apply)",
                ),
            };
            storage_root = target_root.clone();
            storage_config_path = target_root.map(|root| {
                std::path::PathBuf::from(root)
                    .join("sorotte.ini")
                    .to_string_lossy()
                    .into_owned()
            });
            storage_source = target_source.to_owned();
        }
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
                            storage_config_path,
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:root",
                            "Storage Root",
                            GuiWidgetKind::ReadOnly,
                            storage_root,
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "config-storage:source",
                            "Source",
                            GuiWidgetKind::ReadOnly,
                            Some(storage_source),
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
            [SettingId::OsdShow, SettingId::GeneralLanguage]
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
                .chain(self.clear_gui_data_confirmation_widget_tree())
                .chain(self.setup_pending_apply_widget_tree())
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

    fn setup_pending_apply_widget_tree(&self) -> Option<GuiWidgetNode> {
        if self.pending_apply_requirements.is_empty() {
            return None;
        }
        let children = self
            .pending_apply_requirements
            .iter()
            .copied()
            .map(|requirement| {
                GuiWidgetNode::leaf(
                    format!(
                        "configuration:pending-apply:{}",
                        requirement.automation_id()
                    ),
                    "Saved change requirement",
                    GuiWidgetKind::Status,
                    Some(requirement.label().to_owned()),
                    true,
                    false,
                )
            })
            .collect();
        Some(GuiWidgetNode::branch(
            "configuration:pending-apply",
            "Saved changes pending apply",
            GuiWidgetKind::Panel,
            children,
        ))
    }

    fn clear_gui_data_confirmation_widget_tree(&self) -> Option<GuiWidgetNode> {
        self.clear_gui_data_confirmation_visible.then(|| {
            GuiWidgetNode::branch(
                "configuration:clear-gui-data-confirmation",
                "Clear GUI data?",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "configuration:clear-gui-data-confirmation:warning",
                        "Warning",
                        GuiWidgetKind::Status,
                        Some(
                            "This permanently removes saved settings, GUI state, caches, and managed tools."
                                .to_owned(),
                        ),
                        true,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "configuration:clear-gui-data-confirmation:actions",
                        "Confirmation actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 180.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "config-command:cancel-clear-gui-data",
                                "Keep my data",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "config-command:confirm-clear-gui-data",
                                "Permanently clear GUI data",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                        ],
                    ),
                ],
            )
        })
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
