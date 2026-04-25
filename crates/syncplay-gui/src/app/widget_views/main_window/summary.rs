use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn main_window_summary_projection(&self) -> (Option<GuiWidgetNode>, GuiWidgetNode) {
        let can_edit_room = self.pending_operation.is_none();
        let can_set_local_room = can_edit_room && !self.commands.can_disconnect_session;
        let can_request_runtime_room_change = can_edit_room && self.commands.can_disconnect_session;
        let room_draft = self
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default()
            .to_owned();
        let has_room_draft = configured_room_name_text(&room_draft).is_some();
        let has_joined_room = joined_room_name_text(&self.main_window.room_name).is_some();
        let playlist_has_entries = !self.main_window.playlist.is_empty();
        let controls_available = playlist_has_entries && self.pending_operation.is_none();
        let saved_session_target = self.saved_session_connect_target();
        let connection_status = match self.pending_operation.as_ref().map(|pending| pending.kind) {
            Some(GuiPendingOperationKind::ConnectSavedServer) => "connecting",
            Some(GuiPendingOperationKind::DisconnectSession) => "disconnecting",
            _ if self.commands.can_disconnect_session => "connected",
            _ if saved_session_target.is_some() => "disconnected",
            _ => "not-configured",
        };
        let connection_target = saved_session_target
            .as_ref()
            .map(|target| target.address.clone())
            .unwrap_or_else(|| "(not configured)".to_owned());
        let username = self
            .configuration
            .control_value("Connection", "Username")
            .and_then(normalized_editable_text)
            .unwrap_or_else(|| "(not configured)".to_owned());

        let player_setup_panel = self.player_setup_issue.as_ref().map(|issue| {
            GuiWidgetNode::branch(
                "main-window:player-setup",
                "Playback Recovery",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:title",
                        "Title",
                        GuiWidgetKind::Status,
                        self.player_setup_issue_title().map(str::to_owned),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:summary",
                        "Summary",
                        GuiWidgetKind::Status,
                        self.player_setup_issue_summary().map(str::to_owned),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:detail",
                        "Detail",
                        GuiWidgetKind::Status,
                        Some(issue.message.clone()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:player-setup:actions",
                        "Playback Recovery Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:autodetect",
                                "Auto-detect mpv",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:choose-path",
                                "Choose mpv.exe",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:retry",
                                "Retry mpv",
                                GuiWidgetKind::Button,
                                None,
                                self.player_setup_retry_available(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:open-settings",
                                "Open Settings",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                        ],
                    ),
                ],
            )
        });

        let mut session_summary_children = vec![
            GuiWidgetNode::layout(
                "main-window:session-summary:grid",
                "Session Summary Grid",
                GuiLayoutMode::KeyValueGrid {
                    min_pair_width: 220.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:connection-status",
                        "Status",
                        GuiWidgetKind::Status,
                        Some(connection_status.to_owned()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:connection-target",
                        "Server",
                        GuiWidgetKind::Status,
                        Some(connection_target),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:username",
                        "Username",
                        GuiWidgetKind::Status,
                        Some(username),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:room",
                        "Room",
                        GuiWidgetKind::Status,
                        Some(self.main_window.room_name.clone()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:room-control",
                        "Room Control",
                        GuiWidgetKind::Status,
                        Some(self.main_window.room_control_status.clone()),
                        true,
                        false,
                    ),
                ],
            ),
            GuiWidgetNode::layout(
                "main-window:connection:buttons",
                "Connection Buttons",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:connection:connect",
                        self.saved_session_connect_button_label(),
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_connect_saved_server,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:connection:disconnect",
                        "Disconnect",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_disconnect_session,
                        false,
                    ),
                ],
            ),
            GuiWidgetNode::layout(
                "main-window:room-actions:toggle-row",
                "Room Change",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![GuiWidgetNode::leaf(
                    "main-window:room-actions:toggle",
                    if self.main_window_room_change_expanded {
                        "Hide Room Change"
                    } else {
                        "Change Room"
                    },
                    GuiWidgetKind::Button,
                    None,
                    true,
                    self.main_window_room_change_expanded,
                )],
            ),
        ];

        if self.main_window_room_change_expanded {
            session_summary_children.push(GuiWidgetNode::branch(
                "main-window:room-actions",
                "Room",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:room-actions:form",
                        "Room Actions Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:room-input",
                            "Room",
                            GuiWidgetKind::TextInput,
                            Some(room_draft),
                            can_edit_room,
                            false,
                        )],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:room-actions:buttons",
                        "Room Action Buttons",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:room:set",
                                "Set Room",
                                GuiWidgetKind::Button,
                                None,
                                can_set_local_room && has_room_draft,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room:join",
                                "Join Room",
                                GuiWidgetKind::Button,
                                None,
                                can_request_runtime_room_change && has_room_draft,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room:leave",
                                "Leave Room",
                                GuiWidgetKind::Button,
                                None,
                                can_request_runtime_room_change && has_joined_room,
                                false,
                            ),
                        ],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:controller-actions",
                        "Controller Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:room-actions:create-controlled-room",
                                "Create Controlled Room",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none() && has_joined_room,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room-actions:identify-controller",
                                "Identify As Controller",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none()
                                    && self.main_window.room_name.as_str().starts_with('+'),
                                false,
                            ),
                        ],
                    ),
                ],
            ));
        }

        let session_summary = GuiWidgetNode::branch(
            "main-window:connection",
            "Session",
            GuiWidgetKind::Panel,
            session_summary_children,
        );

        let mut control_buttons = Vec::new();
        if self.main_window.show_playback_buttons {
            control_buttons.extend([
                GuiWidgetNode::leaf(
                    "main-window:control:play",
                    "Play",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:pause",
                    "Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:toggle-pause",
                    "Toggle Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:seek",
                    "Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_seek && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:undo-seek",
                    "Undo Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_undo_seek && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:set-offset",
                    "Set Offset",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_set_offset && controls_available,
                    false,
                ),
            ]);
        }
        let local_user_ready = self.displayed_local_main_window_user_ready();
        let ready_button = GuiWidgetNode::leaf(
            "main-window:control:set-ready",
            if local_user_ready {
                "Ready"
            } else {
                "Not Ready"
            },
            GuiWidgetKind::Button,
            None,
            self.main_window.playback.can_set_ready
                && controls_available
                && !self.local_ready_transition_pending(),
            false,
        );
        let controls_panel = GuiWidgetNode::branch(
            "main-window:controls",
            "Controls",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::layout(
                    "main-window:playback-summary:grid",
                    "Playback Summary Grid",
                    GuiLayoutMode::KeyValueGrid {
                        min_pair_width: 220.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            "main-window:playback-paused",
                            "Playback",
                            GuiWidgetKind::Status,
                            Some(bool_label(self.main_window.playback_paused).to_owned()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:autoplay",
                            "Autoplay",
                            GuiWidgetKind::Status,
                            Some(bool_label(self.main_window.autoplay_active).to_owned()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:autoplay-threshold",
                            "Min Users",
                            GuiWidgetKind::Status,
                            Some(self.main_window.autoplay_threshold.to_string()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:user-offset",
                            "Offset",
                            GuiWidgetKind::Status,
                            Some(format!("{:.3}", self.main_window.user_offset_seconds)),
                            true,
                            false,
                        ),
                    ],
                ),
                GuiWidgetNode::layout(
                    "main-window:controls:playback-actions",
                    "Playback Controls",
                    GuiLayoutMode::CompactButtonWrap {
                        button_width: 40.0,
                        button_height: 36.0,
                        gap: 8.0,
                    },
                    control_buttons,
                ),
                GuiWidgetNode::layout(
                    "main-window:controls:ready",
                    "Playback Ready",
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    vec![ready_button],
                ),
            ],
        );

        let autoplay_panel = self.main_window.show_autoplay_controls.then(|| {
            GuiWidgetNode::branch(
                "main-window:autoplay-controls",
                "Autoplay Controls",
                GuiWidgetKind::Panel,
                vec![GuiWidgetNode::layout(
                    "main-window:autoplay-controls:buttons",
                    "Autoplay Control Buttons",
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            "main-window:control:autoplay-toggle",
                            "Autoplay",
                            GuiWidgetKind::Checkbox,
                            Some(self.main_window.autoplay_active.to_string()),
                            self.main_window.playback.can_toggle_autoplay && controls_available,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:control:autoplay-threshold-down",
                            "Autoplay -",
                            GuiWidgetKind::Button,
                            None,
                            self.main_window.playback.can_adjust_autoplay_threshold
                                && controls_available
                                && self.main_window.autoplay_threshold > 2,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:control:autoplay-threshold-up",
                            "Autoplay +",
                            GuiWidgetKind::Button,
                            None,
                            self.main_window.playback.can_adjust_autoplay_threshold
                                && controls_available
                                && self.main_window.autoplay_threshold < 99,
                            false,
                        ),
                    ],
                )],
            )
        });

        let summary_column = GuiWidgetNode::layout(
            "main-window:summary-column",
            "Summary Column",
            GuiLayoutMode::Stack,
            autoplay_panel.clone().into_iter().fold(
                vec![session_summary.clone(), controls_panel.clone()],
                |mut children, panel| {
                    children.push(panel);
                    children
                },
            ),
        );

        (player_setup_panel, summary_column)
    }
}
