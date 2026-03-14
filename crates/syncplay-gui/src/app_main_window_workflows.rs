use syncplay_client_app::app_boundary::commands::controlled_room_base_name_legacy_compatible;

use super::shell_state::{
    GuiControlledRoomCreateSessionState, GuiControllerAuthEditSessionState,
    GuiPendingOperationKind, GuiPendingOperationState, GuiShellView, GuiTransientNotificationLevel,
    MainWindowRoomRow, MainWindowUserRow, SyncplayGuiShellAppState,
};
use super::support::normalized_editable_text;

impl SyncplayGuiShellAppState {
    pub(super) fn move_selected_main_window_playlist(&mut self, delta: isize) -> bool {
        if !self.main_window.playback.can_manage_playlist {
            return self.record_action_error(
                "Playlist row movement is unavailable when shared playlist controls are disabled.",
            );
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No playlist row is currently selected.");
        };
        let Some(target_index) = index.checked_add_signed(delta) else {
            return self.record_action_error("The selected playlist row cannot move further.");
        };
        if target_index >= self.main_window.playlist.len() {
            return self.record_action_error("The selected playlist row cannot move further.");
        }

        let next_entries = {
            let mut entries = self.current_shared_playlist_entries();
            entries.swap(index, target_index);
            entries
        };
        self.remember_shared_playlist_undo_snapshot_if_changed(&next_entries);
        self.main_window.playlist.swap(index, target_index);
        self.selection.selected_main_window_playlist = Some(target_index);
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn remove_selected_main_window_playlist(&mut self) -> bool {
        if !self.main_window.playback.can_manage_playlist {
            return self.record_action_error(
                "Playlist row removal is unavailable when shared playlist controls are disabled.",
            );
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No playlist row is currently selected.");
        };
        if index >= self.main_window.playlist.len() {
            return self.record_action_error("No playlist row exists at the requested index.");
        }

        self.main_window.playlist.remove(index);
        self.selection.selected_main_window_playlist = if self.main_window.playlist.is_empty() {
            None
        } else if index >= self.main_window.playlist.len() {
            Some(self.main_window.playlist.len() - 1)
        } else {
            Some(index)
        };
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn add_main_window_user(&mut self, username: String) -> bool {
        let Some(username) = normalized_editable_text(&username) else {
            return self.record_action_error("Main-window user names must be non-empty.");
        };
        if self
            .main_window
            .users
            .iter()
            .any(|user| user.username.eq_ignore_ascii_case(&username))
        {
            return self.record_action_error("A main-window user with that name already exists.");
        }

        let room_name = self.main_window.room_name.clone();
        if !self
            .main_window
            .rooms
            .iter()
            .any(|room| room.room_name == room_name)
        {
            self.main_window.rooms.push(MainWindowRoomRow {
                room_name: room_name.clone(),
                is_controlled: room_name.starts_with('+'),
                has_named_users: true,
            });
        }
        self.main_window.users.push(MainWindowUserRow {
            username: username.clone(),
            room_name: room_name.clone(),
            is_self: false,
            is_ready: false,
            is_controller: false,
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
            is_selected: false,
        });
        if let Some(room) = self
            .main_window
            .rooms
            .iter_mut()
            .find(|room| room.room_name == room_name)
        {
            room.has_named_users = true;
        }
        self.selection.selected_main_window_user = Some(self.main_window.users.len() - 1);
        self.apply_selection_to_surfaces();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("User joined: {username}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn rename_main_window_user_at_index(
        &mut self,
        index: usize,
        requested_username: String,
        empty_error_message: &'static str,
        missing_error_message: &'static str,
    ) -> Option<(String, String)> {
        let Some(username) = normalized_editable_text(&requested_username) else {
            self.record_action_error(empty_error_message);
            return None;
        };
        if self
            .main_window
            .users
            .iter()
            .enumerate()
            .any(|(other_index, user)| {
                other_index != index && user.username.eq_ignore_ascii_case(&username)
            })
        {
            self.record_action_error("A main-window user with that name already exists.");
            return None;
        }
        let Some(user) = self.main_window.users.get_mut(index) else {
            if self
                .main_window_user_edit_session
                .as_ref()
                .is_some_and(|session| session.editing_index == index)
            {
                self.main_window_user_edit_session = None;
            }
            self.record_action_error(missing_error_message);
            return None;
        };

        let previous_username = user.username.clone();
        user.username = username.clone();
        if user.is_self
            && !self
                .configuration
                .apply_text_value("Connection", "Username", &username)
        {
            user.username = previous_username;
            self.record_action_error(
                "The local user name could not be synchronized back into configuration state.",
            );
            return None;
        }

        Some((previous_username, username))
    }

    pub(super) fn announce_main_window_user_joined(&mut self, username: String) -> bool {
        if !self.add_main_window_user(username) {
            return false;
        }
        let Some(user) = self.main_window.users.last() else {
            return self.record_action_error(
                "The announced main-window user could not be resolved after joining.",
            );
        };
        self.push_system_chat_message(format!("{} joined the room.", user.username));
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_selected_main_window_user_renamed(&mut self, username: String) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some((previous_username, renamed_username)) = self.rename_main_window_user_at_index(
            index,
            username,
            "Renamed main-window user names must be non-empty.",
            "The main-window user being renamed no longer exists.",
        ) else {
            return false;
        };

        self.main_window_user_edit_session = None;
        self.push_system_chat_message(format!(
            "{previous_username} is now known as {renamed_username}.",
        ));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("User renamed: {previous_username} -> {renamed_username}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_selected_main_window_user_left(&mut self) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some(user) = self.main_window.users.get(index) else {
            return self.record_action_error("No main-window user exists at the requested index.");
        };
        let username = user.username.clone();
        if !self.remove_selected_main_window_user() {
            return false;
        }
        self.push_system_chat_message(format!("{username} left the room."));
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn local_main_window_user_index(&self) -> Option<usize> {
        self.main_window.users.iter().position(|user| user.is_self)
    }

    pub(super) fn current_joined_main_window_room_name(&self) -> Option<&str> {
        let room_name = self.main_window.room_name.trim();
        if room_name.is_empty() || room_name == "(no room joined)" {
            None
        } else {
            Some(room_name)
        }
    }

    pub(super) fn main_window_local_can_control_current_room(&self) -> bool {
        if !self.main_window.controlled_room_active {
            return true;
        }
        let Some(room_name) = self.current_joined_main_window_room_name() else {
            return false;
        };
        self.main_window
            .users
            .iter()
            .any(|user| user.is_self && user.room_name == room_name && user.is_controller)
    }

    pub(super) fn can_request_main_window_user_ready_change(
        &self,
        user: &MainWindowUserRow,
    ) -> bool {
        self.pending_operation.is_none()
            && self.commands.can_disconnect_session
            && self.main_window.playback.can_set_ready
            && self.main_window.playback.can_set_others_ready
            && !user.is_self
            && self
                .current_joined_main_window_room_name()
                .is_some_and(|room_name| user.room_name == room_name)
            && self.main_window_local_can_control_current_room()
    }

    pub(super) fn controlled_room_create_default_room_name(&self) -> Option<String> {
        self.current_joined_main_window_room_name()
            .map(controlled_room_base_name_legacy_compatible)
            .and_then(|room_name| normalized_editable_text(&room_name))
    }

    pub(super) fn begin_create_controlled_room_edit(&mut self) -> bool {
        let Some(room_name) = self.controlled_room_create_default_room_name() else {
            return self.record_action_error(
                "A joined room is required before creating a controlled room.",
            );
        };
        self.active_view = GuiShellView::MainWindow;
        self.controlled_room_create_session = Some(GuiControlledRoomCreateSessionState {
            room_buffer: room_name,
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_create_controlled_room_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.controlled_room_create_session.as_mut() else {
            return self
                .record_action_error("No controlled-room creation editor is currently active.");
        };
        session.room_buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_create_controlled_room_edit(&mut self) -> bool {
        if self.controlled_room_create_session.is_none() {
            return self
                .record_action_error("No controlled-room creation editor is currently active.");
        }
        self.controlled_room_create_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_controller_auth_edit(&mut self) -> bool {
        let Some(room_name) = self
            .current_joined_main_window_room_name()
            .and_then(normalized_editable_text)
        else {
            return self.record_action_error(
                "A joined room is required before requesting controller access.",
            );
        };
        if !room_name.starts_with('+') {
            return self.record_action_error(
                "Controller access can only be requested while a controlled room is active.",
            );
        }
        self.active_view = GuiShellView::MainWindow;
        self.controller_auth_edit_session = Some(GuiControllerAuthEditSessionState {
            room_name,
            password_buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_controller_auth_password_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.controller_auth_edit_session.as_mut() else {
            return self.record_action_error("No controller-auth editor is currently active.");
        };
        session.password_buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_controller_auth_edit(&mut self) -> bool {
        if self.controller_auth_edit_session.is_none() {
            return self.record_action_error("No controller-auth editor is currently active.");
        }
        self.controller_auth_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_playback_pause_state(&mut self, paused: bool) -> bool {
        if self.main_window.playback_paused == paused {
            return self.record_action_error(if paused {
                "Playback is already paused."
            } else {
                "Playback is already running."
            });
        }
        self.begin_playback_pause_toggle()
    }

    pub(super) fn begin_playback_pause_toggle(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.main_window.playback.can_toggle_pause {
            return self.record_action_error(
                "Playback pause toggling is unavailable when pause controls are disabled.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::TogglePlaybackPause,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if self.main_window.playback_paused {
                "Playback resume requested.".to_owned()
            } else {
                "Playback pause requested.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_playback_pause_toggle(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No playback toggle is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::TogglePlaybackPause {
            return self.record_action_error("The active GUI operation is not a playback toggle.");
        }

        self.pending_operation = None;
        self.announce_playback_pause_state(!self.main_window.playback_paused)
    }

    pub(super) fn cancel_playback_pause_toggle(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No playback toggle is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::TogglePlaybackPause {
            return self.record_action_error("The active GUI operation is not a playback toggle.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Playback toggle canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_playback_pause_state(&mut self, paused: bool) -> bool {
        if !self.main_window.playback.can_toggle_pause {
            return self.record_action_error(
                "Playback pause state cannot change when pause controls are unavailable.",
            );
        }
        if self.main_window.playback_paused == paused {
            return self.record_action_error(if paused {
                "Playback is already paused."
            } else {
                "Playback is already running."
            });
        }

        self.main_window.playback_paused = paused;
        self.push_system_chat_message(if paused {
            "Playback paused.".to_owned()
        } else {
            "Playback resumed.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if paused {
                "Playback paused.".to_owned()
            } else {
                "Playback resumed.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_local_user_ready_state(&mut self, ready: bool) -> bool {
        if !self.main_window.playback.can_set_ready {
            return self.record_action_error(
                "Local readiness cannot change when ready controls are unavailable.",
            );
        }
        let Some(index) = self.local_main_window_user_index() else {
            return self
                .record_action_error("The local user row is missing from the main-window shell.");
        };
        let Some(user) = self.main_window.users.get_mut(index) else {
            return self
                .record_action_error("The local user row is missing from the main-window shell.");
        };
        if user.is_ready == ready {
            return self.record_action_error(if ready {
                "The local user is already marked ready."
            } else {
                "The local user is already marked not ready."
            });
        }

        user.is_ready = ready;
        self.push_system_chat_message(if ready {
            "You are now marked ready.".to_owned()
        } else {
            "You are now marked not ready.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if ready {
                "Local readiness updated: ready.".to_owned()
            } else {
                "Local readiness updated: not ready.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_autoplay_state(&mut self, active: bool) -> bool {
        if self.main_window.autoplay_active == active {
            return self.record_action_error(if active {
                "Autoplay is already active."
            } else {
                "Autoplay is already inactive."
            });
        }

        self.main_window.autoplay_active = active;
        self.push_system_chat_message(if active {
            "Autoplay enabled.".to_owned()
        } else {
            "Autoplay disabled.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if active {
                "Autoplay enabled.".to_owned()
            } else {
                "Autoplay disabled.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_autoplay_threshold(&mut self, threshold: usize) -> bool {
        if !(2..=99).contains(&threshold) {
            return self.record_action_error(
                "Autoplay minimum users must stay within the supported 2-99 range.",
            );
        }
        if self.main_window.autoplay_threshold == threshold {
            return self.record_action_error(
                "Autoplay minimum users is already set to the requested value.",
            );
        }

        self.main_window.autoplay_threshold = threshold;
        self.push_system_chat_message(format!("Autoplay minimum users set to {threshold}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Autoplay minimum users set to {threshold}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn toggle_main_window_playback_buttons(&mut self) -> bool {
        self.main_window.show_playback_buttons = !self.main_window.show_playback_buttons;
        self.set_menu_action_selected(
            "Window",
            "Playback Buttons",
            self.main_window.show_playback_buttons,
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn toggle_main_window_autoplay_controls(&mut self) -> bool {
        self.main_window.show_autoplay_controls = !self.main_window.show_autoplay_controls;
        self.set_menu_action_selected(
            "Window",
            "Autoplay",
            self.main_window.show_autoplay_controls,
        );
        self.clear_action_error_and_refresh();
        true
    }
}
