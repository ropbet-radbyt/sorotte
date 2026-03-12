use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn configuration_widget_tree(&self) -> GuiWidgetNode {
        let busy = self.pending_operation.is_some();
        let mut children =
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
                            GuiWidgetNode::leaf(
                                format!("config:{}:{}", section.title, control.label),
                                control.label,
                                control.kind.widget_kind(),
                                Some(value),
                                control.kind.is_editable() && !busy,
                                focused,
                            )
                        })
                        .collect();

                    GuiWidgetNode::branch(
                        format!("config-section:{}", section.title),
                        section.title,
                        GuiWidgetKind::Panel,
                        controls,
                    )
                })
                .collect::<Vec<_>>();

        children.push(GuiWidgetNode::branch(
            "config-commands",
            "Commands",
            GuiWidgetKind::Panel,
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
                    self.commands.can_connect_saved_server,
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
        ));

        if let Some(session) = &self.room_history_edit_session {
            children.push(GuiWidgetNode::branch(
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
            ));
        }

        GuiWidgetNode::branch(
            "configuration-root",
            "Configuration",
            GuiWidgetKind::Panel,
            children,
        )
    }

    pub(super) fn main_window_widget_tree(&self) -> GuiWidgetNode {
        let can_edit_room = self.pending_operation.is_none();
        let can_set_local_room = can_edit_room && !self.commands.can_disconnect_session;
        let can_request_runtime_room_change = can_edit_room && self.commands.can_disconnect_session;
        let room_draft = self
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default()
            .to_owned();
        let has_room_draft = !room_draft.trim().is_empty();
        let has_joined_room = {
            let joined_room = self.main_window.room_name.trim();
            !joined_room.is_empty() && joined_room != "(no room joined)"
        };
        let can_manage_playlist =
            self.main_window.playback.can_manage_playlist && self.pending_operation.is_none();
        let selected_playlist_index = self.selection.selected_main_window_playlist;
        let can_move_playlist_up =
            can_manage_playlist && selected_playlist_index.is_some_and(|index| index > 0);
        let can_move_playlist_down = can_manage_playlist
            && selected_playlist_index
                .is_some_and(|index| index + 1 < self.main_window.playlist.len());
        let can_remove_playlist = can_manage_playlist && selected_playlist_index.is_some();
        let can_add_playlist_entry =
            can_manage_playlist && !self.new_playlist_entry_draft.trim().is_empty();
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
        let mut children = vec![
            GuiWidgetNode::branch(
                "main-window:connection",
                "Connection",
                GuiWidgetKind::Panel,
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
                        "Target",
                        GuiWidgetKind::Status,
                        Some(connection_target),
                        true,
                        false,
                    ),
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
            GuiWidgetNode::leaf(
                "main-window:room",
                "Room",
                GuiWidgetKind::Status,
                Some(self.main_window.room_name.clone()),
                true,
                false,
            ),
            GuiWidgetNode::branch(
                "main-window:room-actions",
                "Room Actions",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:room-input",
                        "Room Draft",
                        GuiWidgetKind::TextInput,
                        Some(room_draft),
                        can_edit_room,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:room:set",
                        "Set Current Room",
                        GuiWidgetKind::Button,
                        None,
                        can_set_local_room && has_room_draft,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:room:join",
                        "Join Draft Room",
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
            GuiWidgetNode::leaf(
                "main-window:playback-paused",
                "Playback Paused",
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
                "Autoplay Min Users",
                GuiWidgetKind::Status,
                Some(self.main_window.autoplay_threshold.to_string()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "main-window:autoplay-countdown",
                "Autoplay Countdown",
                GuiWidgetKind::Status,
                Some(
                    self.main_window
                        .autoplay_countdown_seconds
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "(none)".to_owned()),
                ),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "main-window:user-offset",
                "Playback Offset",
                GuiWidgetKind::Status,
                Some(format!("{:.3}", self.main_window.user_offset_seconds)),
                true,
                false,
            ),
        ];

        children.push(self.main_window_browser_widget_node());

        children.push(GuiWidgetNode::branch(
            "main-window:playlist",
            "Playlist",
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
        ));

        children.push(GuiWidgetNode::branch(
            "main-window:playlist-actions",
            "Playlist Actions",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playlist:new",
                    "New Entry",
                    GuiWidgetKind::TextInput,
                    Some(self.new_playlist_entry_draft.clone()),
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:add",
                    "Add Entry",
                    GuiWidgetKind::Button,
                    None,
                    can_add_playlist_entry,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:up",
                    "Move Selected Up",
                    GuiWidgetKind::Button,
                    None,
                    can_move_playlist_up,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:down",
                    "Move Selected Down",
                    GuiWidgetKind::Button,
                    None,
                    can_move_playlist_down,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:remove",
                    "Remove Selected",
                    GuiWidgetKind::Button,
                    None,
                    can_remove_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-files",
                    "Add Files",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-url",
                    "Add URLs",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:open-url",
                    "Open URL",
                    GuiWidgetKind::Button,
                    None,
                    self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:open-selected",
                    "Open Selected",
                    GuiWidgetKind::Button,
                    None,
                    can_open_selected_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:open-selected-folder",
                    "Open Selected Folder",
                    GuiWidgetKind::Button,
                    None,
                    can_open_selected_playlist_folder,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:trust-selected",
                    selected_playlist_domain
                        .as_deref()
                        .map(|domain| format!("Trust {domain}"))
                        .unwrap_or_else(|| "Trust Selected Domain".to_owned()),
                    GuiWidgetKind::Button,
                    None,
                    can_trust_selected_playlist_domain,
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
                    "main-window:playlist:undo",
                    "Undo Playlist",
                    GuiWidgetKind::Button,
                    None,
                    can_undo_playlist,
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
                GuiWidgetNode::leaf(
                    "main-window:playlist:load",
                    "Load Playlist",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:load-shuffle",
                    "Load + Shuffle",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:save",
                    "Save Playlist",
                    GuiWidgetKind::Button,
                    None,
                    can_save_playlist,
                    false,
                ),
            ],
        ));

        if let Some(session) = &self.playlist_text_edit_session {
            children.push(GuiWidgetNode::branch(
                "main-window:playlist-edit",
                "Playlist Editor",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:text",
                        "Playlist Entries",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:commit",
                        "Apply Playlist",
                        GuiWidgetKind::Button,
                        None,
                        session.is_dirty,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:cancel",
                        "Cancel Playlist Edit",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                ],
            ));
        }

        if let Some(session) = &self.playlist_url_edit_session {
            children.push(GuiWidgetNode::branch(
                "main-window:playlist-url-edit",
                "Playlist URLs",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:text",
                        "URLs",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:commit",
                        "Add URLs To Playlist",
                        GuiWidgetKind::Button,
                        None,
                        session.is_dirty,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:cancel",
                        "Cancel URL Entry",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                ],
            ));
        }

        if let Some(session) = &self.media_url_edit_session {
            children.push(GuiWidgetNode::branch(
                "main-window:media-url-edit",
                "Open URL",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:media-url-edit:text",
                        "URL",
                        GuiWidgetKind::TextInput,
                        Some(session.buffer.clone()),
                        self.pending_operation.is_none(),
                        false,
                    ),
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
            ));
        }

        if let Some(session) = &self.controlled_room_create_session {
            let can_create_controlled_room = normalized_editable_text(
                &controlled_room_base_name_legacy_compatible(&session.room_buffer),
            )
            .is_some();
            children.push(GuiWidgetNode::branch(
                "main-window:controlled-room-create",
                "Create Controlled Room",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:controlled-room-create:room",
                        "Room Name",
                        GuiWidgetKind::TextInput,
                        Some(session.room_buffer.clone()),
                        self.pending_operation.is_none(),
                        false,
                    ),
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
            ));
        }

        if let Some(session) = &self.controller_auth_edit_session {
            children.push(GuiWidgetNode::branch(
                "main-window:controller-auth",
                "Identify As Controller",
                GuiWidgetKind::Panel,
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
            ));
        }

        children.push(GuiWidgetNode::branch(
            "main-window:chat",
            "Chat",
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
        ));

        children.push(GuiWidgetNode::leaf(
            "main-window:chat-input",
            "Chat Input",
            GuiWidgetKind::TextInput,
            Some(self.outgoing_chat_message.clone().unwrap_or_default()),
            self.commands.can_send_chat_message,
            false,
        ));

        let mut control_children = Vec::new();
        if self.main_window.show_playback_buttons {
            control_children.extend([
                GuiWidgetNode::leaf(
                    "main-window:control:play",
                    "Play",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:pause",
                    "Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:toggle-pause",
                    "Toggle Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_toggle_pause,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:seek",
                    "Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_seek && self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:undo-seek",
                    "Undo Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_undo_seek && self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:set-offset",
                    "Set Offset",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_set_offset && self.pending_operation.is_none(),
                    false,
                ),
            ]);
        }
        control_children.push(GuiWidgetNode::leaf(
            "main-window:control:set-ready",
            "Set Ready",
            GuiWidgetKind::Button,
            None,
            self.main_window.playback.can_set_ready && self.pending_operation.is_none(),
            false,
        ));
        children.push(GuiWidgetNode::branch(
            "main-window:controls",
            "Controls",
            GuiWidgetKind::Panel,
            control_children,
        ));

        if self.main_window.show_autoplay_controls {
            children.push(GuiWidgetNode::branch(
                "main-window:autoplay-controls",
                "Autoplay Controls",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:control:autoplay-toggle",
                        "Autoplay",
                        GuiWidgetKind::Checkbox,
                        Some(self.main_window.autoplay_active.to_string()),
                        self.main_window.playback.can_toggle_autoplay
                            && self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:control:autoplay-threshold-down",
                        "Autoplay -",
                        GuiWidgetKind::Button,
                        None,
                        self.main_window.playback.can_adjust_autoplay_threshold
                            && self.pending_operation.is_none()
                            && self.main_window.autoplay_threshold > 2,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:control:autoplay-threshold-up",
                        "Autoplay +",
                        GuiWidgetKind::Button,
                        None,
                        self.main_window.playback.can_adjust_autoplay_threshold
                            && self.pending_operation.is_none()
                            && self.main_window.autoplay_threshold < 99,
                        false,
                    ),
                ],
            ));
        }

        GuiWidgetNode::branch(
            "main-window-root",
            "Main Window",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn main_window_browser_widget_node(&self) -> GuiWidgetNode {
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
            let mut children = vec![
                GuiWidgetNode::leaf(
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
                ),
                GuiWidgetNode::leaf(
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
                children.push(GuiWidgetNode::branch(
                    format!("main-window:user:{user_index}"),
                    &user.username,
                    GuiWidgetKind::Panel,
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
                            can_mutate_browser_settings && user.has_file && !user.file_is_url,
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
                ));
            }

            if !has_visible_users {
                children.push(GuiWidgetNode::leaf(
                    format!("main-window:room-group:{room_index}:empty"),
                    "Users",
                    GuiWidgetKind::Status,
                    Some("(empty room)".to_owned()),
                    true,
                    false,
                ));
            }

            room_children.push(GuiWidgetNode::branch(
                format!("main-window:room-group:{room_index}"),
                &room.room_name,
                GuiWidgetKind::Panel,
                children,
            ));
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

        GuiWidgetNode::branch(
            "main-window:browser",
            "Room Browser",
            GuiWidgetKind::Panel,
            room_children,
        )
    }

    pub(super) fn menu_dialog_widget_tree(&self) -> GuiWidgetNode {
        let mut children = self
            .menus
            .sections
            .iter()
            .enumerate()
            .map(|(section_index, section)| {
                GuiWidgetNode::branch(
                    format!("menus:section:{section_index}"),
                    section.title,
                    GuiWidgetKind::Panel,
                    section
                        .actions
                        .iter()
                        .enumerate()
                        .map(|(action_index, action)| {
                            GuiWidgetNode::leaf(
                                format!("menus:action:{section_index}:{action_index}"),
                                action.label,
                                GuiWidgetKind::Button,
                                None,
                                action.enabled,
                                action.is_selected,
                            )
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();

        children.push(GuiWidgetNode::branch(
            "menus:dialogs",
            "Dialogs",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "menus:dialog:tls",
                    "TLS Certificate Prompt",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.menus.tls_prompt_expected).to_owned()),
                    true,
                    self.open_modal == Some(GuiShellModal::TlsCertificatePrompt),
                ),
                GuiWidgetNode::leaf(
                    "menus:dialog:update",
                    "Update Notice",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.menus.update_notice_expected).to_owned()),
                    true,
                    self.open_modal == Some(GuiShellModal::UpdateNotice),
                ),
                GuiWidgetNode::leaf(
                    "menus:dialog:about",
                    "About Dialog",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.menus.about_dialog_available).to_owned()),
                    self.menus.about_dialog_available,
                    self.open_modal == Some(GuiShellModal::About),
                ),
            ],
        ));

        GuiWidgetNode::branch(
            "menus-root",
            "Menus & Dialogs",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn shell_modal_widget_tree(&self) -> GuiWidgetNode {
        let Some(modal) = self.open_modal else {
            return GuiWidgetNode::branch("shell:modal", "Modal", GuiWidgetKind::Panel, Vec::new());
        };
        let mut children = vec![GuiWidgetNode::leaf(
            "shell:modal:kind",
            "Modal Kind",
            GuiWidgetKind::Status,
            Some(modal.label().to_owned()),
            true,
            false,
        )];
        if modal == GuiShellModal::UpdateNotice {
            if let Some(message) = self.update_check.message.as_ref() {
                children.push(GuiWidgetNode::leaf(
                    "shell:modal:update:message",
                    "Message",
                    GuiWidgetKind::Status,
                    Some(message.clone()),
                    true,
                    false,
                ));
            }
            if let Some(url) = self.update_check.url.as_ref() {
                children.push(GuiWidgetNode::leaf(
                    "shell:modal:update:url",
                    "Update URL",
                    GuiWidgetKind::Status,
                    Some(url.clone()),
                    true,
                    false,
                ));
            }
        }
        children.extend(GuiWidgetEguiRenderer::modal_actions(modal).into_iter().map(
            |(id, label, _)| {
                GuiWidgetNode::leaf(id, label, GuiWidgetKind::Button, None, true, false)
            },
        ));
        children.push(GuiWidgetNode::leaf(
            "shell:modal:close",
            "Close",
            GuiWidgetKind::Button,
            None,
            true,
            false,
        ));
        GuiWidgetNode::branch("shell:modal", "Modal", GuiWidgetKind::Panel, children)
    }

    pub(super) fn public_server_widget_tree(&self) -> GuiWidgetNode {
        let has_selected_server = self.selected_public_server_index().is_some();
        let can_run_server_commands =
            self.pending_operation.is_none() && self.public_server_edit_session.is_none();
        let mut children = vec![GuiWidgetNode::branch(
            "public-servers:list",
            "Servers",
            GuiWidgetKind::List,
            self.public_servers
                .servers
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("public-servers:row:{index}"),
                        &row.label,
                        GuiWidgetKind::ListItem,
                        Some(row.address.clone()),
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        )];

        children.push(GuiWidgetNode::branch(
            "public-servers:commands",
            "Commands",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "public-servers:command:connect",
                    "Connect",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_connect_public_server && can_run_server_commands,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "public-servers:command:refresh",
                    "Refresh",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_refresh_public_servers && can_run_server_commands,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "public-servers:command:add-custom",
                    "Add Custom Server",
                    GuiWidgetKind::Button,
                    None,
                    self.public_servers.can_add_custom_server && can_run_server_commands,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "public-servers:command:edit",
                    "Edit Selected",
                    GuiWidgetKind::Button,
                    None,
                    has_selected_server && can_run_server_commands,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "public-servers:command:remove",
                    "Remove Selected",
                    GuiWidgetKind::Button,
                    None,
                    has_selected_server && can_run_server_commands,
                    false,
                ),
            ],
        ));

        if let Some(session) = &self.public_server_edit_session {
            children.push(GuiWidgetNode::branch(
                "public-servers:edit-session",
                "Edit Session",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "public-servers:edit:label",
                        "Label",
                        GuiWidgetKind::TextInput,
                        Some(session.label_buffer.clone()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:edit:address",
                        "Address",
                        GuiWidgetKind::TextInput,
                        Some(session.address_buffer.clone()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:edit:commit",
                        "Save Changes",
                        GuiWidgetKind::Button,
                        None,
                        session.is_dirty,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:edit:cancel",
                        "Cancel Edit",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                ],
            ));
        }

        GuiWidgetNode::branch(
            "public-servers-root",
            "Public Servers",
            GuiWidgetKind::Panel,
            children,
        )
    }

    pub(super) fn media_search_widget_tree(&self) -> GuiWidgetNode {
        let selected_directory_index = self.selection.selected_media_search_directory;
        let can_manage_directories = self.pending_operation.is_none();
        let can_move_directory_up =
            can_manage_directories && selected_directory_index.is_some_and(|index| index > 0);
        let can_move_directory_down = can_manage_directories
            && selected_directory_index
                .is_some_and(|index| index + 1 < self.media_search.directories.len());
        let can_remove_directory = can_manage_directories && selected_directory_index.is_some();
        let mut children = vec![GuiWidgetNode::branch(
            "media-search:directories",
            "Directories",
            GuiWidgetKind::List,
            self.media_search
                .directories
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("media-search:directory:{index}"),
                        &row.path,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        )];

        children.push(GuiWidgetNode::branch(
            "media-search:commands",
            "Commands",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "media-search:command:browse",
                    "Browse Directories",
                    GuiWidgetKind::Button,
                    None,
                    self.media_search.can_browse_directories && self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:command:search",
                    "Search Missing Media",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_search_missing_media,
                    false,
                ),
            ],
        ));

        children.push(GuiWidgetNode::branch(
            "media-search:timing",
            "Timing",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "media-search:timing:first-file",
                    "First File Timeout",
                    GuiWidgetKind::Status,
                    Some(optional_seconds_text(
                        self.media_search.first_file_timeout_seconds,
                    )),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:timing:search",
                    "Search Timeout",
                    GuiWidgetKind::Status,
                    Some(optional_seconds_text(
                        self.media_search.search_timeout_seconds,
                    )),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:timing:double-check",
                    "Double Check Interval",
                    GuiWidgetKind::Status,
                    Some(optional_seconds_text(
                        self.media_search.double_check_interval_seconds,
                    )),
                    true,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:timing:warning-threshold",
                    "Warning Threshold",
                    GuiWidgetKind::Status,
                    Some(optional_seconds_text(
                        self.media_search.warning_threshold_seconds,
                    )),
                    true,
                    false,
                ),
            ],
        ));

        children.push(GuiWidgetNode::branch(
            "media-search:directory-actions",
            "Directory Actions",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "media-search:directory:up",
                    "Move Selected Up",
                    GuiWidgetKind::Button,
                    None,
                    can_move_directory_up,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:directory:down",
                    "Move Selected Down",
                    GuiWidgetKind::Button,
                    None,
                    can_move_directory_down,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "media-search:directory:remove",
                    "Remove Selected",
                    GuiWidgetKind::Button,
                    None,
                    can_remove_directory,
                    false,
                ),
            ],
        ));

        GuiWidgetNode::branch(
            "media-search-root",
            "Media Search",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn command_status_widget_tree(&self) -> GuiWidgetNode {
        let items = [
            ("busy", "Busy", self.pending_operation.is_some()),
            ("save", "Save", self.commands.can_save_configuration),
            ("reset", "Reset", self.commands.can_reset_configuration),
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

    fn validation_widget_tree(&self) -> GuiWidgetNode {
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
                        format!("shell:validation:issue:{index}"),
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

    fn quick_actions_widget_tree(&self) -> GuiWidgetNode {
        let can_open_media_file = self
            .menus
            .sections
            .iter()
            .find(|section| section.title == "File")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Open Media File")
            })
            .is_some_and(|action| action.enabled);

        GuiWidgetNode::branch(
            "shell:quick-actions",
            "Quick Actions",
            GuiWidgetKind::Panel,
            vec![GuiWidgetNode::leaf(
                "shell:quick:open-media-file",
                "Quick Open Media File",
                GuiWidgetKind::Button,
                None,
                can_open_media_file,
                false,
            )],
        )
    }

    pub(super) fn shell_widget_tree(&self) -> GuiWidgetNode {
        let mut configuration = self.configuration_widget_tree();
        configuration.selected = self.active_view == GuiShellView::Configuration;

        let mut main_window = self.main_window_widget_tree();
        main_window.selected = self.active_view == GuiShellView::MainWindow;

        let mut menus = self.menu_dialog_widget_tree();
        menus.selected = self.active_view == GuiShellView::MenusAndDialogs;

        let mut public_servers = self.public_server_widget_tree();
        public_servers.selected = self.active_view == GuiShellView::PublicServers;

        let mut media_search = self.media_search_widget_tree();
        media_search.selected = self.active_view == GuiShellView::MediaSearch;

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
            "Syncplay GUI",
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
                self.shell_modal_widget_tree(),
                self.quick_actions_widget_tree(),
                self.command_status_widget_tree(),
                self.validation_widget_tree(),
                notifications,
                configuration,
                main_window,
                menus,
                public_servers,
                media_search,
            ],
        )
    }

    pub(super) fn render_shell_widgets(&self, renderer: &mut impl GuiWidgetRenderer) {
        self.shell_widget_tree().render_with(renderer);
    }

    fn reapply_runtime_main_window_surface_from_snapshot(
        &mut self,
        previous_settings: &StoredClientSettingsMvp,
        current_snapshot: &MainWindowRuntimeSnapshot,
    ) {
        let previous_baseline = MainWindowRuntimeSnapshot::from_shell_state(
            &MainWindowShellState::from_stored_settings(previous_settings),
        );
        let preserve_connected_room_surface = self.commands.can_disconnect_session;

        if preserve_connected_room_surface
            || current_snapshot.room_name != previous_baseline.room_name
        {
            self.main_window.room_name = current_snapshot.room_name.clone();
        }
        if current_snapshot.shared_playlist_enabled != previous_baseline.shared_playlist_enabled {
            self.main_window.shared_playlist_enabled = current_snapshot.shared_playlist_enabled;
        }
        if preserve_connected_room_surface
            || current_snapshot.controlled_room_active != previous_baseline.controlled_room_active
        {
            self.main_window.controlled_room_active = current_snapshot.controlled_room_active;
        }
        if current_snapshot.hide_empty_rooms != previous_baseline.hide_empty_rooms {
            self.main_window.hide_empty_rooms = current_snapshot.hide_empty_rooms;
            self.set_menu_action_selected(
                "Window",
                "Hide Empty Rooms",
                current_snapshot.hide_empty_rooms,
            );
        }
        if preserve_connected_room_surface || current_snapshot.rooms != previous_baseline.rooms {
            self.main_window.rooms = current_snapshot
                .rooms
                .iter()
                .map(|room| MainWindowRoomRow {
                    room_name: room.room_name.clone(),
                    is_controlled: room.is_controlled,
                    has_named_users: room.has_named_users,
                })
                .collect();
        }
        if preserve_connected_room_surface || current_snapshot.users != previous_baseline.users {
            self.main_window.users = current_snapshot
                .users
                .iter()
                .map(|user| MainWindowUserRow {
                    username: user.username.clone(),
                    room_name: user.room_name.clone(),
                    is_self: user.is_self,
                    is_ready: user.is_ready,
                    is_controller: user.is_controller,
                    has_file: user.has_file,
                    file_name: user.file_name.clone(),
                    file_name_label: user
                        .file_name
                        .clone()
                        .unwrap_or_else(|| "No file".to_owned()),
                    file_size_label: user.file_size_label.clone(),
                    file_duration_label: user.file_duration_label.clone(),
                    file_is_url: user.file_is_url,
                    file_is_trusted: user.file_is_trusted,
                    filename_differs: user.filename_differs,
                    filesize_differs: user.filesize_differs,
                    fileduration_differs: user.fileduration_differs,
                    is_selected: false,
                })
                .collect();
        }
        if current_snapshot.playlist != previous_baseline.playlist {
            self.remember_shared_playlist_undo_snapshot_if_changed(&current_snapshot.playlist);
            self.main_window.playlist = current_snapshot
                .playlist
                .iter()
                .map(|label| MainWindowPlaylistRow {
                    label: label.clone(),
                    is_selected: false,
                })
                .collect();
        }
        if current_snapshot.chat != previous_baseline.chat {
            self.main_window.chat = current_snapshot
                .chat
                .iter()
                .map(|row| MainWindowChatRow {
                    sender: row.sender.clone(),
                    message: row.message.clone(),
                })
                .collect();
        }
        if current_snapshot.can_toggle_pause != previous_baseline.can_toggle_pause {
            self.main_window.playback.can_toggle_pause = current_snapshot.can_toggle_pause;
        }
        if current_snapshot.can_seek != previous_baseline.can_seek {
            self.main_window.playback.can_seek = current_snapshot.can_seek;
        }
        if current_snapshot.can_undo_seek != previous_baseline.can_undo_seek {
            self.main_window.playback.can_undo_seek = current_snapshot.can_undo_seek;
        }
        if current_snapshot.can_set_offset != previous_baseline.can_set_offset {
            self.main_window.playback.can_set_offset = current_snapshot.can_set_offset;
        }
        if current_snapshot.can_toggle_autoplay != previous_baseline.can_toggle_autoplay {
            self.main_window.playback.can_toggle_autoplay = current_snapshot.can_toggle_autoplay;
        }
        if current_snapshot.can_adjust_autoplay_threshold
            != previous_baseline.can_adjust_autoplay_threshold
        {
            self.main_window.playback.can_adjust_autoplay_threshold =
                current_snapshot.can_adjust_autoplay_threshold;
        }
        if current_snapshot.can_set_ready != previous_baseline.can_set_ready {
            self.main_window.playback.can_set_ready = current_snapshot.can_set_ready;
        }
        if current_snapshot.can_set_others_ready != previous_baseline.can_set_others_ready {
            self.main_window.playback.can_set_others_ready = current_snapshot.can_set_others_ready;
        }
        if current_snapshot.can_manage_playlist != previous_baseline.can_manage_playlist {
            self.main_window.playback.can_manage_playlist = current_snapshot.can_manage_playlist;
        }
        if current_snapshot.playback_paused != previous_baseline.playback_paused {
            self.main_window.playback_paused = current_snapshot.playback_paused;
        }
        if current_snapshot.autoplay_active != previous_baseline.autoplay_active {
            self.main_window.autoplay_active = current_snapshot.autoplay_active;
        }
        if current_snapshot.autoplay_threshold != previous_baseline.autoplay_threshold {
            self.main_window.autoplay_threshold = current_snapshot.autoplay_threshold;
        }
        if current_snapshot.autoplay_countdown_seconds
            != previous_baseline.autoplay_countdown_seconds
        {
            self.main_window.autoplay_countdown_seconds =
                current_snapshot.autoplay_countdown_seconds;
        }
        if (current_snapshot.user_offset_seconds - previous_baseline.user_offset_seconds).abs()
            > f64::EPSILON
        {
            self.main_window.user_offset_seconds = current_snapshot.user_offset_seconds;
        }
        if current_snapshot.show_playback_buttons != previous_baseline.show_playback_buttons {
            self.main_window.show_playback_buttons = current_snapshot.show_playback_buttons;
            self.set_menu_action_selected(
                "Window",
                "Playback Buttons",
                current_snapshot.show_playback_buttons,
            );
        }
        if current_snapshot.show_autoplay_controls != previous_baseline.show_autoplay_controls {
            self.main_window.show_autoplay_controls = current_snapshot.show_autoplay_controls;
            self.set_menu_action_selected(
                "Window",
                "Autoplay",
                current_snapshot.show_autoplay_controls,
            );
        }
    }

    fn preserves_runtime_dialog_expectations(
        &self,
        previous_settings: &StoredClientSettingsMvp,
    ) -> (bool, bool) {
        let previous_baseline = MenuDialogShellState::from_stored_settings(previous_settings);
        (
            self.menus.tls_prompt_expected != previous_baseline.tls_prompt_expected,
            self.menus.update_notice_expected != previous_baseline.update_notice_expected,
        )
    }

    fn preserves_runtime_public_server_surface(
        &self,
        previous_settings: &StoredClientSettingsMvp,
    ) -> bool {
        let previous_baseline =
            PublicServerBrowserShellState::from_stored_settings(previous_settings);
        PublicServerBrowserRuntimeFlags::from_shell_state(&self.public_servers)
            != PublicServerBrowserRuntimeFlags::from_shell_state(&previous_baseline)
    }

    fn preserves_runtime_media_search_surface(
        &self,
        previous_settings: &StoredClientSettingsMvp,
    ) -> bool {
        let previous_baseline =
            MediaSearchWorkflowShellState::from_stored_settings(previous_settings);
        MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_search)
            != MediaSearchWorkflowRuntimeFlags::from_shell_state(&previous_baseline)
    }

    pub(super) fn sync_derived_surfaces_from_configuration_settings(
        &mut self,
        previous_settings: &StoredClientSettingsMvp,
    ) {
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.main_window);
        let settings = self.configuration.to_stored_settings();
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.public_servers.servers.clone());
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_search.directories.clone());
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(previous_settings);
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(previous_settings);
        let preserved_public_server_flags = preserve_public_servers
            .then(|| PublicServerBrowserRuntimeFlags::from_shell_state(&self.public_servers));
        let preserve_media_search = self.preserves_runtime_media_search_surface(previous_settings);
        let preserved_media_search_flags = preserve_media_search
            .then(|| MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_search));
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let tls_prompt_expected = self.menus.tls_prompt_expected;
        let update_notice_expected = self.menus.update_notice_expected;
        let about_dialog_available = self.menus.about_dialog_available;
        self.main_window = MainWindowShellState::from_stored_settings(&settings);
        self.reapply_runtime_main_window_surface_from_snapshot(
            previous_settings,
            &preserved_main_window_runtime_snapshot,
        );
        self.menus = MenuDialogShellState::from_stored_settings(&settings);
        if preserve_tls_prompt_expected {
            self.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.menus.update_notice_expected = update_notice_expected;
        }
        self.menus.about_dialog_available = about_dialog_available;
        self.public_servers = PublicServerBrowserShellState::from_stored_settings(&settings);
        if let Some(servers) = preserved_public_server_rows {
            self.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.public_servers.apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        self.media_search = MediaSearchWorkflowShellState::from_stored_settings(&settings);
        if let Some(directories) = preserved_media_search_directories {
            self.media_search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_search.apply_runtime_flags(runtime_flags);
        }
        self.normalize_runtime_menu_action_overrides_for_settings(&settings);
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.normalize_focused_configuration_control();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
    }

    pub(super) fn resync_from_settings(&mut self, settings: StoredClientSettingsMvp) {
        let previous_settings = self.configuration.to_stored_settings();
        let active_view = self.active_view;
        let open_modal = self.open_modal;
        let selection = self.selection.clone();
        let runtime_menu_action_overrides = self.runtime_menu_action_overrides.clone();
        let runtime_command_availability_override =
            self.runtime_command_availability_override.clone();
        let pending_operation = self.pending_operation.clone();
        let outgoing_chat_message = self.outgoing_chat_message.clone();
        let new_main_window_user_draft = self.new_main_window_user_draft.clone();
        let new_playlist_entry_draft = self.new_playlist_entry_draft.clone();
        let focused_configuration_control = self.focused_configuration_control.clone();
        let public_server_edit_session = self.public_server_edit_session.clone();
        let main_window_user_edit_session = self.main_window_user_edit_session.clone();
        let text_edit_session = self.text_edit_session.clone();
        let playlist_text_edit_session = self.playlist_text_edit_session.clone();
        let playlist_url_edit_session = self.playlist_url_edit_session.clone();
        let media_url_edit_session = self.media_url_edit_session.clone();
        let room_history_edit_session = self.room_history_edit_session.clone();
        let update_check = self.update_check.clone();
        let runtime_validation_issues = self.runtime_validation_issues.clone();
        let notifications = self.notifications.clone();
        let last_media_dialog_directory = self.last_media_dialog_directory.clone();
        let last_action_error = self.validation.last_action_error.clone();
        let playlist_undo_snapshot = self.playlist_undo_snapshot.clone();
        let playlist_shuffle_nonce = self.playlist_shuffle_nonce;
        let saved_configuration = self.saved_configuration.clone();
        let tls_prompt_expected = self.menus.tls_prompt_expected;
        let update_notice_expected = self.menus.update_notice_expected;
        let about_dialog_available = self.menus.about_dialog_available;
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.main_window);
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(&previous_settings);
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.public_servers.servers.clone());
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(&previous_settings);
        let preserved_public_server_flags = preserve_public_servers
            .then(|| PublicServerBrowserRuntimeFlags::from_shell_state(&self.public_servers));
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_search.directories.clone());
        let preserve_media_search = self.preserves_runtime_media_search_surface(&previous_settings);
        let preserved_media_search_flags = preserve_media_search
            .then(|| MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_search));

        *self = Self::from_stored_settings(&settings);
        self.active_view = active_view;
        self.open_modal = open_modal;
        self.selection = selection;
        self.runtime_menu_action_overrides = runtime_menu_action_overrides;
        self.runtime_command_availability_override = runtime_command_availability_override;
        self.pending_operation = pending_operation;
        self.outgoing_chat_message = outgoing_chat_message;
        self.new_main_window_user_draft = new_main_window_user_draft;
        self.new_playlist_entry_draft = new_playlist_entry_draft;
        self.focused_configuration_control = focused_configuration_control;
        self.public_server_edit_session = public_server_edit_session;
        self.main_window_user_edit_session = main_window_user_edit_session;
        self.text_edit_session = text_edit_session;
        self.playlist_text_edit_session = playlist_text_edit_session;
        self.playlist_url_edit_session = playlist_url_edit_session;
        self.media_url_edit_session = media_url_edit_session;
        self.room_history_edit_session = room_history_edit_session;
        self.update_check = update_check;
        self.runtime_validation_issues = runtime_validation_issues;
        self.notifications = notifications;
        self.last_media_dialog_directory = last_media_dialog_directory;
        self.playlist_undo_snapshot = playlist_undo_snapshot;
        self.playlist_shuffle_nonce = playlist_shuffle_nonce;
        self.saved_configuration = saved_configuration;
        if preserve_tls_prompt_expected {
            self.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.menus.update_notice_expected = update_notice_expected;
        }
        self.menus.about_dialog_available = about_dialog_available;
        if let Some(servers) = preserved_public_server_rows {
            self.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.public_servers.apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        if let Some(directories) = preserved_media_search_directories {
            self.media_search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_search.apply_runtime_flags(runtime_flags);
        }
        self.normalize_runtime_menu_action_overrides_for_settings(&settings);
        self.reapply_runtime_main_window_surface_from_snapshot(
            &previous_settings,
            &preserved_main_window_runtime_snapshot,
        );
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.normalize_focused_configuration_control();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.validation.last_action_error = last_action_error;
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
    }

    pub(super) fn has_unsaved_configuration_changes(&self) -> bool {
        self.configuration
            .has_unsaved_changes_against(&self.saved_configuration)
    }
}
