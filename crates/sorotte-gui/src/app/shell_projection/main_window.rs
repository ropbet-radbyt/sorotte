use super::*;

impl MainWindowShellState {
    pub(in crate::app) fn room_control_status_without_session() -> String {
        "Unavailable: no active server session.".to_owned()
    }

    pub(in crate::app) fn room_control_status_waiting_for_server() -> String {
        "Pending: waiting for server room state.".to_owned()
    }

    pub(in crate::app) fn room_control_status_uncontrolled_room() -> String {
        "Not required: current room is not controlled.".to_owned()
    }

    pub(in crate::app) fn room_control_status_granted() -> String {
        "Granted by server: you control this room.".to_owned()
    }

    pub(in crate::app) fn room_control_status_locked() -> String {
        "Not granted by server: room controls are locked.".to_owned()
    }

    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
        let room_name = runtime_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str())
            .unwrap_or("(no room joined)")
            .to_owned();
        let username = runtime_settings
            .config
            .connection
            .username
            .as_ref()
            .map(|username| username.as_str())
            .unwrap_or("You")
            .to_owned();
        let shared_playlist_enabled = runtime_settings.config.playback.shared_playlist_enabled;
        let controlled_room_active = room_name.starts_with('+');

        let playlist_default_source = GuiPlaylistDefaultSourceState::default();
        let mut playlist = Vec::new();
        if shared_playlist_enabled {
            let label = "Playlist pane ready for shared entries".to_owned();
            playlist.push(MainWindowPlaylistRow::inferred(label, true));
        }

        Self {
            room_name: room_name.clone(),
            room_control_status: Self::room_control_status_without_session(),
            shared_playlist_enabled,
            controlled_room_active,
            hide_empty_rooms: false,
            rooms: vec![MainWindowRoomRow {
                room_name: room_name.clone(),
                is_controlled: controlled_room_active,
                has_named_users: true,
            }],
            users: vec![MainWindowUserRow {
                username,
                room_name,
                is_self: true,
                is_ready: false,
                is_controller: controlled_room_active,
                has_file: false,
                file_name: None,
                file_name_label: "No file".to_owned(),
                file_size_label: String::new(),
                file_duration_label: String::new(),
                file_is_url: false,
                file_is_trusted: true,
                filename_differs: false,
                filesize_differs: false,
                fileduration_differs: false,
                is_selected: true,
            }],
            playlist,
            playlist_default_source,
            active_playlist_index: None,
            chat: if runtime_settings.config.interface.chat_output_enabled {
                vec![MainWindowChatRow {
                    sender: "system".to_owned(),
                    message: "Chat pane ready".to_owned(),
                }]
            } else {
                Vec::new()
            },
            playback: MainWindowPlaybackControls {
                can_toggle_pause: false,
                can_seek: false,
                can_undo_seek: false,
                can_set_offset: false,
                can_toggle_autoplay: true,
                can_adjust_autoplay_threshold: true,
                can_set_ready: true,
                can_set_others_ready: false,
                can_manage_playlist: false,
            },
            playback_paused: false,
            autoplay_active: runtime_settings.config.readiness.autoplay_initial_state,
            autoplay_threshold: autoplay_threshold_from_settings(settings),
            autoplay_countdown_seconds: None,
            user_offset_seconds: 0.0,
            show_playback_buttons: true,
            show_autoplay_controls: true,
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "[Room]".to_owned(),
            format!(
                "Room: {} (shared_playlist={}, controlled_room={})",
                self.room_name,
                bool_label(self.shared_playlist_enabled),
                bool_label(self.controlled_room_active),
            ),
            format!("Room Control: {}", self.room_control_status),
            format!(
                "Browser: hide_empty_rooms={}, rooms={}",
                bool_label(self.hide_empty_rooms),
                self.rooms.len(),
            ),
            format!(
                "Playback Controls: pause={}, seek={}, undo_seek={}, offset={}, autoplay={}, autoplay_threshold={}, ready={}, others_ready={}, playlist={}, show_buttons={}, show_autoplay={}",
                bool_label(self.playback.can_toggle_pause),
                bool_label(self.playback.can_seek),
                bool_label(self.playback.can_undo_seek),
                bool_label(self.playback.can_set_offset),
                bool_label(self.playback.can_toggle_autoplay),
                bool_label(self.playback.can_adjust_autoplay_threshold),
                bool_label(self.playback.can_set_ready),
                bool_label(self.playback.can_set_others_ready),
                bool_label(self.playback.can_manage_playlist),
                bool_label(self.show_playback_buttons),
                bool_label(self.show_autoplay_controls),
            ),
            format!(
                "Playback State: paused={}, autoplay={}, autoplay_threshold={}, autoplay_countdown={}, offset={}",
                bool_label(self.playback_paused),
                bool_label(self.autoplay_active),
                self.autoplay_threshold,
                self.autoplay_countdown_seconds
                    .map(|seconds| seconds.to_string())
                    .unwrap_or_else(|| "(none)".to_owned()),
                self.user_offset_seconds,
            ),
            format!("Rooms ({}):", self.rooms.len()),
        ];

        for room in &self.rooms {
            lines.push(format!(
                "- {} [controlled={}, named_users={}]",
                room.room_name,
                bool_label(room.is_controlled),
                bool_label(room.has_named_users),
            ));
        }

        lines.push(format!("Users ({}):", self.users.len()));
        for user in &self.users {
            lines.push(format!(
                "- {} @ {} [self={}, ready={}, controller={}, selected={}, file={}, size={}, duration={}, diffs=name:{}/size:{}/duration:{}, trusted_url={}]",
                user.username,
                user.room_name,
                bool_label(user.is_self),
                bool_label(user.is_ready),
                bool_label(user.is_controller),
                bool_label(user.is_selected),
                user.file_name_label,
                if user.file_size_label.is_empty() {
                    "(none)"
                } else {
                    &user.file_size_label
                },
                if user.file_duration_label.is_empty() {
                    "(none)"
                } else {
                    &user.file_duration_label
                },
                bool_label(user.filename_differs),
                bool_label(user.filesize_differs),
                bool_label(user.fileduration_differs),
                bool_label(user.file_is_trusted),
            ));
        }

        lines.push(format!("Playlist ({}):", self.playlist.len()));
        if self.playlist.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for item in &self.playlist {
                lines.push(format!(
                    "- {} [selected={}]",
                    item.label,
                    bool_label(item.is_selected)
                ));
            }
        }

        lines.push(format!("Chat ({}):", self.chat.len()));
        if self.chat.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for entry in &self.chat {
                lines.push(format!("- {}: {}", entry.sender, entry.message));
            }
        }

        lines
    }
}
