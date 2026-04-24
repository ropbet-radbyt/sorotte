use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn main_window_widget_tree(&self) -> GuiWidgetNode {
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
        let can_manage_playlist =
            self.main_window.playback.can_manage_playlist && self.pending_operation.is_none();
        let playlist_has_entries = !self.main_window.playlist.is_empty();
        let controls_available = playlist_has_entries && self.pending_operation.is_none();
        let selected_playlist_index = self.selection.selected_main_window_playlist;
        let can_remove_playlist = can_manage_playlist && selected_playlist_index.is_some();
        let selected_playlist_entry = self.selected_shared_playlist_entry().map(str::to_owned);
        let selected_playlist_is_url = selected_playlist_entry
            .as_deref()
            .is_some_and(browser_is_url);
        let trusted_domains = self
            .configuration
            .to_stored_settings()
            .trusted_domains
            .unwrap_or_default();
        let selected_playlist_domain = selected_playlist_entry
            .as_deref()
            .and_then(browser_domain_from_url);
        let can_open_selected_playlist =
            self.pending_operation.is_none() && selected_playlist_entry.is_some();
        let can_open_selected_playlist_folder = self.pending_operation.is_none()
            && selected_playlist_entry.is_some()
            && !selected_playlist_is_url;
        let can_trust_selected_playlist_domain = self.pending_operation.is_none()
            && selected_playlist_domain.is_some()
            && selected_playlist_entry
                .as_deref()
                .is_some_and(|entry| !browser_uri_is_trusted(entry, true, &trusted_domains));
        let can_save_playlist =
            self.pending_operation.is_none() && !self.main_window.playlist.is_empty();
        let can_shuffle_remaining = can_manage_playlist
            && selected_playlist_index
                .is_some_and(|index| index + 1 < self.main_window.playlist.len());
        let can_shuffle_entire = can_manage_playlist && !self.main_window.playlist.is_empty();
        let can_undo_playlist = can_manage_playlist
            && self
                .playlist_undo_snapshot
                .as_ref()
                .is_some_and(|previous| *previous != self.current_shared_playlist_entries());
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

        let playlist_panel = GuiWidgetNode::branch(
            "main-window:playlist",
            "Entries",
            GuiWidgetKind::List,
            self.main_window
                .playlist
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("main-window:playlist:{index}"),
                        &row.label,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        )
        .with_min_content_height(220.0);

        let mut playlist_add_menu = GuiWidgetNode::branch(
            "main-window:playlist:add-menu",
            "Add",
            GuiWidgetKind::Button,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-files",
                    "Choose Files...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-url",
                    "Paste URLs...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
            ],
        );
        playlist_add_menu.enabled = can_manage_playlist;

        let mut playlist_more_menu = GuiWidgetNode::branch(
            "main-window:playlist:more-menu",
            "More",
            GuiWidgetKind::Button,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playlist:load",
                    "Load Playlist...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:save",
                    "Save Playlist...",
                    GuiWidgetKind::Button,
                    None,
                    can_save_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:load-shuffle",
                    "Load + Shuffle...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:undo",
                    "Undo",
                    GuiWidgetKind::Button,
                    None,
                    can_undo_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:shuffle-remaining",
                    "Shuffle Remaining",
                    GuiWidgetKind::Button,
                    None,
                    can_shuffle_remaining,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:shuffle-entire",
                    "Shuffle Entire",
                    GuiWidgetKind::Button,
                    None,
                    can_shuffle_entire,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:edit",
                    "Edit Playlist",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
            ],
        );
        playlist_more_menu.enabled = !playlist_more_menu.children.is_empty();

        let playlist_header = GuiWidgetNode::layout(
            "main-window:playlist-header:actions",
            "Playlist Header Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 118.0,
            },
            vec![playlist_add_menu, playlist_more_menu],
        );

        let playlist_selection_bar = selected_playlist_entry.as_ref().map(|_| {
            let mut selection_actions = vec![GuiWidgetNode::leaf(
                "main-window:playlist:open-selected",
                "Open",
                GuiWidgetKind::Button,
                None,
                can_open_selected_playlist,
                false,
            )];
            if can_open_selected_playlist_folder {
                selection_actions.push(GuiWidgetNode::leaf(
                    "main-window:playlist:open-selected-folder",
                    "Open Folder",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                ));
            }
            if can_trust_selected_playlist_domain {
                selection_actions.push(GuiWidgetNode::leaf(
                    "main-window:playlist:trust-selected",
                    selected_playlist_domain
                        .as_deref()
                        .map(|domain| format!("Trust {domain}"))
                        .unwrap_or_else(|| "Trust Selected Domain".to_owned()),
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                ));
            }
            selection_actions.push(GuiWidgetNode::leaf(
                "main-window:playlist:remove",
                "Remove",
                GuiWidgetKind::Button,
                None,
                can_remove_playlist,
                false,
            ));

            GuiWidgetNode::layout(
                "main-window:playlist-selection:actions",
                "Selected Playlist Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                selection_actions,
            )
        });

        let chat_panel = GuiWidgetNode::branch(
            "main-window:chat-panel",
            "Chat",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::branch(
                    "main-window:chat",
                    "Chat History",
                    GuiWidgetKind::List,
                    self.main_window
                        .chat
                        .iter()
                        .enumerate()
                        .map(|(index, row)| {
                            GuiWidgetNode::leaf(
                                format!("main-window:chat:{index}"),
                                &row.sender,
                                GuiWidgetKind::ListItem,
                                Some(row.message.clone()),
                                true,
                                false,
                            )
                        })
                        .collect(),
                )
                .with_min_content_height(180.0),
                GuiWidgetNode::layout(
                    "main-window:chat-compose",
                    "Chat Compose",
                    GuiLayoutMode::Stack,
                    vec![
                        GuiWidgetNode::leaf(
                            "main-window:chat-input",
                            "Chat Input",
                            GuiWidgetKind::TextInput,
                            Some(self.outgoing_chat_message.clone().unwrap_or_default()),
                            self.commands.can_send_chat_message,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:chat:send",
                            "Send",
                            GuiWidgetKind::Button,
                            None,
                            self.commands.can_send_chat_message
                                && self
                                    .outgoing_chat_message
                                    .as_deref()
                                    .and_then(normalized_editable_text)
                                    .is_some(),
                            false,
                        ),
                    ],
                ),
            ],
        );

        let playlist_text_edit_panel = self.playlist_text_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:playlist-edit",
                "Playlist Editor",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:close",
                        "Close",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:text",
                        "Playlist Entries",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:playlist-edit:actions",
                        "Playlist Editor Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:playlist-edit:commit",
                            "Apply Playlist",
                            GuiWidgetKind::Button,
                            None,
                            session.is_dirty,
                            false,
                        )],
                    ),
                ],
            )
        });

        let playlist_url_edit_panel = self.playlist_url_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:playlist-url-edit",
                "Playlist URLs",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:close",
                        "Close",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:text",
                        "URLs",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:playlist-url-edit:actions",
                        "Playlist URL Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:playlist-url-edit:commit",
                            "Add URLs To Playlist",
                            GuiWidgetKind::Button,
                            None,
                            session.is_dirty,
                            false,
                        )],
                    ),
                ],
            )
        });

        let mut playlist_column_children = vec![GuiWidgetNode::branch(
            "main-window:playlist-surface",
            "Shared Playlist",
            GuiWidgetKind::Panel,
            [playlist_header.clone()]
                .into_iter()
                .chain([playlist_panel.clone()])
                .chain(playlist_selection_bar.clone())
                .collect(),
        )];
        playlist_column_children.extend(
            [
                playlist_text_edit_panel.clone(),
                playlist_url_edit_panel.clone(),
            ]
            .into_iter()
            .flatten(),
        );
        let playlist_column = GuiWidgetNode::layout(
            "main-window:playlist-column",
            "Playlist Column",
            GuiLayoutMode::Stack,
            playlist_column_children,
        );

        let media_url_edit_panel = self.media_url_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:media-url-edit",
                "Open URL",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:media-url-edit:form",
                        "Open URL Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:media-url-edit:text",
                            "URL",
                            GuiWidgetKind::TextInput,
                            Some(session.buffer.clone()),
                            self.pending_operation.is_none(),
                            false,
                        )],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:media-url-edit:actions",
                        "Open URL Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:media-url-edit:commit",
                                "Open URL",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:media-url-edit:cancel",
                                "Cancel Open URL",
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

        let controlled_room_create_panel =
            self.controlled_room_create_session.as_ref().map(|session| {
                let can_create_controlled_room = normalized_editable_text(
                    &controlled_room_base_name_legacy_compatible(&session.room_buffer),
                )
                .is_some();
                GuiWidgetNode::branch(
                    "main-window:controlled-room-create",
                    "Create Controlled Room",
                    GuiWidgetKind::Panel,
                    vec![
                        GuiWidgetNode::layout(
                            "main-window:controlled-room-create:form",
                            "Controlled Room Form",
                            GuiLayoutMode::FormGrid {
                                label_width: 160.0,
                                min_field_width: 220.0,
                            },
                            vec![GuiWidgetNode::leaf(
                                "main-window:controlled-room-create:room",
                                "Room Name",
                                GuiWidgetKind::TextInput,
                                Some(session.room_buffer.clone()),
                                self.pending_operation.is_none(),
                                false,
                            )],
                        ),
                        GuiWidgetNode::layout(
                            "main-window:controlled-room-create:actions",
                            "Controlled Room Actions",
                            GuiLayoutMode::ButtonWrap {
                                min_button_width: 140.0,
                            },
                            vec![
                                GuiWidgetNode::leaf(
                                    "main-window:controlled-room-create:commit",
                                    "Create Controlled Room",
                                    GuiWidgetKind::Button,
                                    None,
                                    can_create_controlled_room,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    "main-window:controlled-room-create:cancel",
                                    "Cancel Controlled Room Creation",
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

        let controller_auth_panel = self.controller_auth_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:controller-auth",
                "Identify As Controller",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:controller-auth:form",
                        "Controller Auth Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:room",
                                "Room",
                                GuiWidgetKind::Status,
                                Some(session.room_name.clone()),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:password",
                                "Password",
                                GuiWidgetKind::PasswordInput,
                                Some(session.password_buffer.clone()),
                                self.pending_operation.is_none(),
                                false,
                            ),
                        ],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:controller-auth:actions",
                        "Controller Auth Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:commit",
                                "Identify As Controller",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:cancel",
                                "Cancel Controller Auth",
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

        let room_browser = self
            .main_window_browser_widget_node()
            .with_min_content_height(260.0);
        let work_top_region = GuiWidgetNode::layout(
            "main-window:work-top-region",
            "Room Work Top Region",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 240.0,
                max_columns: 2,
            },
            vec![room_browser.clone(), playlist_column.clone()],
        );
        let work_area = GuiWidgetNode::layout(
            "main-window:work-area",
            "Room Work Area",
            GuiLayoutMode::Stack,
            vec![work_top_region, chat_panel.clone()],
        )
        .with_span(2);
        let top_region = GuiWidgetNode::layout(
            "main-window:top-region",
            "Room Dashboard",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 240.0,
                max_columns: 3,
            },
            vec![summary_column.clone(), work_area],
        );

        let mut overview_children = vec![top_region];
        let mut overview_editor_panels = Vec::new();
        for panel in [
            media_url_edit_panel.clone(),
            controlled_room_create_panel.clone(),
            controller_auth_panel.clone(),
        ]
        .into_iter()
        .flatten()
        {
            overview_editor_panels.push(panel);
        }
        if !overview_editor_panels.is_empty() {
            if overview_editor_panels.len() == 1
                && let Some(editor_panel) = overview_editor_panels.first_mut()
            {
                *editor_panel = editor_panel.clone().with_span(2);
            }
            overview_children.push(GuiWidgetNode::layout(
                "main-window:editors",
                "Room Editors",
                GuiLayoutMode::ResponsiveColumns {
                    min_column_width: 420.0,
                    max_columns: 2,
                },
                overview_editor_panels,
            ));
        }

        let overview_content = GuiWidgetNode::layout(
            "main-window:content",
            "Room Content",
            GuiLayoutMode::Stack,
            overview_children,
        );

        GuiWidgetNode::layout(
            "main-window-root",
            "Room",
            GuiLayoutMode::Stack,
            player_setup_panel
                .into_iter()
                .chain([overview_content])
                .collect(),
        )
    }

    pub(crate) fn main_window_browser_widget_node(&self) -> GuiWidgetNode {
        let can_join_room =
            self.pending_operation.is_none() && self.commands.can_disconnect_session;
        let can_open_media =
            self.pending_operation.is_none() && self.media_open_runtime_available();
        let can_mutate_browser_settings = self.pending_operation.is_none();
        let mut room_children = Vec::new();

        for (room_index, room) in self.main_window.rooms.iter().enumerate() {
            if self.main_window.hide_empty_rooms && !room.has_named_users {
                continue;
            }

            let current_room = room.room_name == self.main_window.room_name;
            let mut room_group_children = vec![
                GuiWidgetNode::layout(
                    format!("main-window:room-group:{room_index}:summary"),
                    format!("{} Summary", room.room_name),
                    GuiLayoutMode::KeyValueGrid {
                        min_pair_width: 220.0,
                    },
                    vec![GuiWidgetNode::leaf(
                        format!("main-window:room-group:{room_index}:state"),
                        "State",
                        GuiWidgetKind::Status,
                        Some(format!(
                            "current={}, controlled={}, named_users={}",
                            bool_label(current_room),
                            bool_label(room.is_controlled),
                            bool_label(room.has_named_users),
                        )),
                        true,
                        false,
                    )],
                ),
                GuiWidgetNode::layout(
                    format!("main-window:room-group:{room_index}:actions"),
                    format!("{} Actions", room.room_name),
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    vec![GuiWidgetNode::leaf(
                        format!("main-window:room-group:{room_index}:join"),
                        if current_room {
                            "Current Room"
                        } else {
                            "Join Room"
                        },
                        GuiWidgetKind::Button,
                        None,
                        can_join_room && !current_room,
                        false,
                    )],
                ),
            ];

            let mut has_visible_users = false;
            for (user_index, user) in self.main_window.users.iter().enumerate() {
                if user.room_name != room.room_name {
                    continue;
                }
                has_visible_users = true;

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
                let trusted_domain = user
                    .file_name
                    .as_deref()
                    .filter(|file_name| browser_is_url(file_name) && !user.file_is_trusted)
                    .and_then(browser_domain_from_url);
                let can_change_ready = self.can_request_main_window_user_ready_change(user);

                let mut user_panel = GuiWidgetNode::branch(
                    format!("main-window:user:{user_index}"),
                    &user.username,
                    GuiWidgetKind::Panel,
                    vec![
                        GuiWidgetNode::layout(
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
                                    user.is_selected,
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
                        ),
                        GuiWidgetNode::layout(
                            format!("main-window:user:{user_index}:actions"),
                            format!("{} Actions", user.username),
                            GuiLayoutMode::ButtonWrap {
                                min_button_width: 140.0,
                            },
                            vec![
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:open"),
                                    if user.file_is_url {
                                        "Open Stream"
                                    } else {
                                        "Open User File"
                                    },
                                    GuiWidgetKind::Button,
                                    None,
                                    can_open_media && user.has_file && !user.is_self,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:folder"),
                                    "Open Containing Folder",
                                    GuiWidgetKind::Button,
                                    None,
                                    can_mutate_browser_settings
                                        && user.has_file
                                        && !user.file_is_url,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:trust"),
                                    trusted_domain
                                        .as_deref()
                                        .map(|domain| format!("Trust {domain}"))
                                        .unwrap_or_else(|| "Trust Domain".to_owned()),
                                    GuiWidgetKind::Button,
                                    None,
                                    can_mutate_browser_settings && trusted_domain.is_some(),
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:ready"),
                                    if user.is_ready {
                                        format!("Set {} Not Ready", user.username)
                                    } else {
                                        format!("Set {} Ready", user.username)
                                    },
                                    GuiWidgetKind::Button,
                                    None,
                                    can_change_ready,
                                    false,
                                ),
                            ],
                        ),
                    ],
                );
                user_panel.selected = user.is_selected;
                room_group_children.push(user_panel);
            }

            if !has_visible_users {
                room_group_children.push(GuiWidgetNode::leaf(
                    format!("main-window:room-group:{room_index}:empty"),
                    "Users",
                    GuiWidgetKind::Status,
                    Some("(empty room)".to_owned()),
                    true,
                    false,
                ));
            }

            let mut room_panel = GuiWidgetNode::branch(
                format!("main-window:room-group:{room_index}"),
                &room.room_name,
                GuiWidgetKind::Panel,
                room_group_children,
            );
            room_panel.selected = current_room;
            room_children.push(room_panel);
        }

        if room_children.is_empty() {
            room_children.push(GuiWidgetNode::leaf(
                "main-window:browser:empty",
                "Room Browser",
                GuiWidgetKind::Status,
                Some("No visible rooms.".to_owned()),
                true,
                false,
            ));
        }

        room_children.insert(
            0,
            GuiWidgetNode::leaf(
                "main-window:browser:hide-empty",
                "Hide Empty Rooms",
                GuiWidgetKind::Checkbox,
                Some(bool_label(self.main_window.hide_empty_rooms).to_owned()),
                can_mutate_browser_settings,
                false,
            ),
        );

        GuiWidgetNode::branch(
            "main-window:browser",
            "Room Browser",
            GuiWidgetKind::Panel,
            room_children,
        )
    }
}
