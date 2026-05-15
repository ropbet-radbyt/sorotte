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
        let local_ready_available =
            self.main_window.playback.can_set_ready && self.pending_operation.is_none();
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
            local_ready_available,
            false,
        );
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

        let connection_status_tooltip = match connection_status {
            "connected" => format!("Connected to {connection_target}."),
            "connecting" => format!("Connecting to {connection_target}."),
            "disconnecting" => "Disconnecting from the current session.".to_owned(),
            "disconnected" => format!("Disconnected. Saved server: {connection_target}."),
            _ => "No server is configured.".to_owned(),
        };
        let room_control_tooltip = self.main_window.room_control_status.clone();
        let room_playback_state_tooltip = if self.main_window.playback_paused {
            "Room state: paused"
        } else {
            "Room state: playing"
        };
        let mut participant_indices: Vec<usize> = self
            .main_window
            .users
            .iter()
            .enumerate()
            .filter(|(_, user)| user.room_name == self.main_window.room_name)
            .map(|(index, _)| index)
            .collect();
        participant_indices.sort_by_key(|index| {
            let user = &self.main_window.users[*index];
            (!user.is_self, *index)
        });
        let mut participant_children = participant_indices
            .into_iter()
            .map(|user_index| {
                let user = &self.main_window.users[user_index];
                let mut cue_parts = Vec::new();
                if !user.has_file {
                    cue_parts.push("no-file".to_owned());
                }
                if user.filename_differs {
                    cue_parts.push("name-diff".to_owned());
                }
                if user.filesize_differs {
                    cue_parts.push("size-diff".to_owned());
                }
                if user.fileduration_differs {
                    cue_parts.push("duration-diff".to_owned());
                }
                if user.file_is_url && !user.file_is_trusted {
                    cue_parts.push("untrusted-url".to_owned());
                }
                let cue_suffix = if cue_parts.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", cue_parts.join(", "))
                };
                let mut user_children = vec![GuiWidgetNode::layout(
                    format!("main-window:user:{user_index}:summary"),
                    format!("{} Summary", user.username),
                    GuiLayoutMode::KeyValueGrid {
                        min_pair_width: 220.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:state"),
                            "State",
                            GuiWidgetKind::Status,
                            Some(format!(
                                "self={}, ready={}, controller={}",
                                bool_label(user.is_self),
                                bool_label(user.is_ready),
                                bool_label(user.is_controller),
                            )),
                            true,
                            user.is_self,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:size"),
                            "Size",
                            GuiWidgetKind::Status,
                            Some(if user.file_size_label.is_empty() {
                                "(none)".to_owned()
                            } else {
                                user.file_size_label.clone()
                            }),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:duration"),
                            "Duration",
                            GuiWidgetKind::Status,
                            Some(if user.file_duration_label.is_empty() {
                                "(none)".to_owned()
                            } else {
                                user.file_duration_label.clone()
                            }),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:file"),
                            "File",
                            GuiWidgetKind::Status,
                            Some(format!("{}{}", user.file_name_label, cue_suffix)),
                            true,
                            false,
                        ),
                    ],
                )];
                if user.is_self {
                    user_children.push(ready_button.clone());
                }

                let mut user_panel = GuiWidgetNode::branch(
                    format!("main-window:user:{user_index}"),
                    &user.username,
                    GuiWidgetKind::Panel,
                    user_children,
                );
                user_panel.selected = user.is_self;
                user_panel
            })
            .collect::<Vec<_>>();
        if participant_children.is_empty() {
            participant_children.push(GuiWidgetNode::leaf(
                "main-window:participants:empty",
                "Participants",
                GuiWidgetKind::Status,
                Some("No users in this room.".to_owned()),
                true,
                false,
            ));
        }

        let mut session_summary_children = vec![
            GuiWidgetNode::leaf(
                "main-window:connection-status",
                "Status",
                GuiWidgetKind::Status,
                Some(connection_status.to_owned()),
                true,
                false,
            )
            .with_tooltip(connection_status_tooltip),
            GuiWidgetNode::leaf(
                "main-window:connection-target",
                "Server",
                GuiWidgetKind::Status,
                Some(connection_target),
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
            )
            .with_tooltip(room_control_tooltip),
            GuiWidgetNode::leaf(
                "main-window:room-playback-state",
                "Room State",
                GuiWidgetKind::Status,
                Some(bool_label(self.main_window.playback_paused).to_owned()),
                true,
                false,
            )
            .with_tooltip(room_playback_state_tooltip),
            GuiWidgetNode::layout(
                "main-window:room-header:actions",
                "Room Header Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 104.0,
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
        ];

        if let Some(header_actions) = session_summary_children
            .iter_mut()
            .find(|node| node.id == "main-window:room-header:actions")
        {
            header_actions.children.insert(
                1,
                GuiWidgetNode::leaf(
                    "main-window:room-actions:toggle",
                    "Change Room",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    self.main_window_room_change_expanded,
                ),
            );
        }

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

        session_summary_children.push(GuiWidgetNode::layout(
            "main-window:participants",
            "Participants",
            GuiLayoutMode::Stack,
            participant_children,
        ));

        let session_summary = GuiWidgetNode::branch(
            "main-window:connection",
            "Room",
            GuiWidgetKind::Panel,
            session_summary_children,
        )
        .with_min_content_height(320.0);

        let summary_column = GuiWidgetNode::layout(
            "main-window:summary-column",
            "Summary Column",
            GuiLayoutMode::Stack,
            vec![session_summary.clone()],
        );

        (player_setup_panel, summary_column)
    }
}
