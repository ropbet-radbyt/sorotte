use super::runtime_localization::localize_gui_runtime_message_legacy_compatible;
use super::shell_state::{
    GuiConfigurationTab, GuiShellModal, GuiShellView, GuiTransientNotification,
    GuiTransientNotificationLevel, MainWindowChatRow, MainWindowPlaybackControls,
    MainWindowPlaylistRow, MainWindowRoomRow, MainWindowRuntimeSnapshot, MainWindowShellState,
    MainWindowUserRow, MenuActionId, SettingId, SorotteGuiRuntimeSnapshot, SorotteGuiShellAppState,
};
use super::support::{
    NO_ROOM_JOINED_LABEL, joined_room_name_text, nonempty_room_name_text, normalized_editable_text,
};
use sorotte_client_app::app_boundary::readiness::ReadinessPresentationProtocol;

impl SorotteGuiShellAppState {
    pub(super) fn close_modal_window(&mut self) -> bool {
        let Some(modal) = self.open_modal.take() else {
            return false;
        };
        if modal == GuiShellModal::UpdateNotice {
            self.menus.update_notice_expected = false;
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_about_dialog_requested(&mut self) -> bool {
        if !self.menus.about_dialog_available {
            return self.record_action_error("The About dialog is unavailable.");
        }
        self.open_modal = Some(GuiShellModal::About);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_help_requested(&mut self) -> bool {
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Opening the Sorotte client guide in your browser.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn trigger_selected_menu_action(&mut self) -> bool {
        let Some((section_index, action_index)) = self.selection.selected_menu_action else {
            return self.record_action_error("No menu action is currently selected.");
        };
        let Some(action_id) = self.menus.action_id_at(section_index, action_index) else {
            return self.record_action_error("No menu action exists at the requested index.");
        };
        self.invoke_menu_action(action_id)
    }

    pub(super) fn invoke_menu_action(&mut self, action_id: MenuActionId) -> bool {
        if self.menus.action(action_id).is_none() {
            return self.record_action_error("The requested menu action is not available.");
        }
        if !self.menu_action_available_now(action_id) {
            return self.record_action_error("The selected menu action is currently disabled.");
        }

        match action_id {
            MenuActionId::OpenMedia => {
                self.push_transient_notification(
                    GuiTransientNotificationLevel::Info,
                    "Open media file requested.".to_owned(),
                );
                self.push_system_chat_message("Open media file requested.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::OpenMediaSearch => {
                self.active_view = GuiShellView::Setup;
                self.select_configuration_tab(GuiConfigurationTab::PlaybackSearch);
                self.push_system_chat_message("Media search opened.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::OpenPublicServerBrowser => {
                self.active_view = GuiShellView::Setup;
                self.select_configuration_tab(GuiConfigurationTab::Connection);
                self.push_system_chat_message("Public server browser opened.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::Exit => {
                self.push_transient_notification(
                    GuiTransientNotificationLevel::Warning,
                    "Exit requested.".to_owned(),
                );
                self.push_system_chat_message("Exit requested.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::Play => self.begin_playback_pause_state(false),
            MenuActionId::Pause => self.begin_playback_pause_state(true),
            MenuActionId::TogglePause => self.begin_playback_pause_toggle(),
            MenuActionId::Seek | MenuActionId::UndoSeek | MenuActionId::SetOffset => {
                self.request_main_window_playback_control()
            }
            MenuActionId::SharedPlaylist => {
                self.active_view = GuiShellView::Room;
                self.push_system_chat_message("Shared playlist opened.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::TrustedDomains => {
                self.active_view = GuiShellView::Setup;
                self.select_configuration_tab(GuiConfigurationTab::PrivacyChat);
                self.push_system_chat_message("Trusted domains opened.".to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            MenuActionId::CreateControlledRoom => self.begin_create_controlled_room_edit(),
            MenuActionId::IdentifyAsController => self.begin_controller_auth_edit(),
            MenuActionId::TlsCertificates => self.announce_tls_certificate_prompt_required(),
            MenuActionId::TogglePlaybackButtons => self.toggle_main_window_playback_buttons(),
            MenuActionId::About => self.announce_about_dialog_requested(),
            MenuActionId::Help => self.announce_help_requested(),
            MenuActionId::CheckForUpdates => self.begin_update_check(true),
        }
    }

    pub(super) fn toggle_selected_main_window_user_ready(&mut self) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some(user) = self.main_window.users.get_mut(index) else {
            return self.record_action_error("No main-window user exists at the requested index.");
        };

        user.is_ready = !user.is_ready;
        let username = user.username.clone();
        let readiness = user.is_ready;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!(
                "User readiness updated: {} -> {}.",
                username,
                if readiness { "ready" } else { "not ready" }
            ),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn toggle_selected_main_window_user_controller(&mut self) -> bool {
        if !self.main_window.controlled_room_active {
            return self.record_action_error(
                "Controller state can only be changed while a controlled room is active.",
            );
        }
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some(user) = self.main_window.users.get_mut(index) else {
            return self.record_action_error("No main-window user exists at the requested index.");
        };

        user.is_controller = !user.is_controller;
        let username = user.username.clone();
        let is_controller = user.is_controller;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!(
                "Controller status updated: {} -> {}.",
                username,
                if is_controller {
                    "controller"
                } else {
                    "participant"
                }
            ),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn remove_selected_main_window_user(&mut self) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some(user) = self.main_window.users.get(index) else {
            return self.record_action_error("No main-window user exists at the requested index.");
        };
        if user.is_self {
            return self.record_action_error(
                "The local user row cannot be removed from the main-window shell.",
            );
        }

        let username = user.username.clone();
        let room_name = user.room_name.clone();
        self.main_window.users.remove(index);
        if let Some(room) = self
            .main_window
            .rooms
            .iter_mut()
            .find(|room| room.room_name == room_name)
        {
            room.has_named_users = self
                .main_window
                .users
                .iter()
                .any(|user| user.room_name == room_name);
        }
        if let Some(session) = self.main_window_user_edit_session.as_mut() {
            if session.editing_index == index {
                self.main_window_user_edit_session = None;
            } else if session.editing_index > index {
                session.editing_index -= 1;
            }
        }
        self.selection.selected_main_window_user = if self.main_window.users.is_empty() {
            None
        } else if index >= self.main_window.users.len() {
            Some(self.main_window.users.len() - 1)
        } else {
            Some(index)
        };
        self.apply_selection_to_surfaces();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            format!("User removed: {username}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn set_main_window_room_state(&mut self, room: Option<String>) {
        let room_value = room.unwrap_or_default();
        let _ = self
            .configuration
            .apply_text_value(SettingId::ConnectionRoom, &room_value);
        let controlled_room_active = room_value.starts_with('+');
        self.main_window.room_name = if room_value.is_empty() {
            NO_ROOM_JOINED_LABEL.to_owned()
        } else {
            room_value
        };
        if !self
            .main_window
            .rooms
            .iter()
            .any(|room| room.room_name == self.main_window.room_name)
        {
            self.main_window.rooms.push(MainWindowRoomRow {
                room_name: self.main_window.room_name.clone(),
                is_controlled: controlled_room_active,
                has_named_users: !self.main_window.users.is_empty(),
            });
        }
        self.main_window.controlled_room_active = controlled_room_active;
        for user in &mut self.main_window.users {
            user.is_controller = false;
            if user.is_self {
                user.room_name = self.main_window.room_name.clone();
            }
        }
        if let Some(user) = self.main_window.users.first_mut() {
            user.is_controller = controlled_room_active;
        }
        for room in &mut self.main_window.rooms {
            room.is_controlled = room.room_name.starts_with('+');
            room.has_named_users = self
                .main_window
                .users
                .iter()
                .any(|user| user.room_name == room.room_name);
        }
    }

    pub(super) fn replace_main_window_runtime_snapshot(
        &mut self,
        snapshot: MainWindowRuntimeSnapshot,
    ) -> bool {
        let Some(room_name) = nonempty_room_name_text(&snapshot.room_name) else {
            return self.record_action_error(
                "Main-window runtime snapshots must include a non-empty room name.",
            );
        };
        if !snapshot.playlist_entry_ids.is_empty()
            && snapshot.playlist_entry_ids.len() != snapshot.playlist.len()
        {
            return self.record_action_error(
                "Main-window runtime snapshots must align playlist row identities with entries.",
            );
        }

        let mut normalized_rooms = Vec::with_capacity(snapshot.rooms.len());
        for room in snapshot.rooms {
            let Some(room_name) = nonempty_room_name_text(&room.room_name) else {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain empty room names.",
                );
            };
            if normalized_rooms.iter().any(|existing: &MainWindowRoomRow| {
                existing.room_name.eq_ignore_ascii_case(&room_name)
            }) {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain duplicate room names.",
                );
            }
            normalized_rooms.push(MainWindowRoomRow {
                room_name,
                is_controlled: room.is_controlled,
                has_named_users: room.has_named_users,
            });
        }

        let mut normalized_users = Vec::with_capacity(snapshot.users.len());
        for user in snapshot.users {
            let Some(username) = normalized_editable_text(&user.username) else {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain empty user names.",
                );
            };
            let user_room_name =
                nonempty_room_name_text(&user.room_name).unwrap_or_else(|| room_name.clone());
            if normalized_users.iter().any(|existing: &MainWindowUserRow| {
                existing.username.eq_ignore_ascii_case(&username)
            }) {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain duplicate user names.",
                );
            }
            if !normalized_rooms
                .iter()
                .any(|room| room.room_name == user_room_name)
            {
                normalized_rooms.push(MainWindowRoomRow {
                    room_name: user_room_name.clone(),
                    is_controlled: user_room_name.starts_with('+'),
                    has_named_users: true,
                });
            }
            normalized_users.push(MainWindowUserRow {
                username,
                room_name: user_room_name,
                is_self: user.is_self,
                is_ready: user.is_ready,
                is_controller: user.is_controller,
                has_file: user.has_file,
                file_name_label: user
                    .file_name
                    .clone()
                    .unwrap_or_else(|| "No file".to_owned()),
                file_name: user.file_name,
                file_size_label: user.file_size_label,
                file_duration_label: user.file_duration_label,
                file_is_url: user.file_is_url,
                file_is_trusted: user.file_is_trusted,
                filename_differs: user.filename_differs,
                filesize_differs: user.filesize_differs,
                fileduration_differs: user.fileduration_differs,
                is_selected: false,
            });
        }
        if !normalized_rooms
            .iter()
            .any(|room| room.room_name == room_name)
        {
            normalized_rooms.push(MainWindowRoomRow {
                room_name: room_name.clone(),
                is_controlled: snapshot.controlled_room_active || room_name.starts_with('+'),
                has_named_users: normalized_users
                    .iter()
                    .any(|user| user.room_name == room_name),
            });
        }
        for room in &mut normalized_rooms {
            room.has_named_users = normalized_users
                .iter()
                .any(|user| user.room_name == room.room_name);
        }

        let mut normalized_readiness = snapshot.readiness;
        normalized_readiness.retain(|username, presentation| {
            presentation.protocol == ReadinessPresentationProtocol::V2
                && presentation.username.eq_ignore_ascii_case(username)
                && normalized_users
                    .iter()
                    .any(|user| user.username.eq_ignore_ascii_case(username))
        });

        let playlist_scope_unchanged = self.main_window.room_name == room_name
            && self.main_window.shared_playlist_enabled == snapshot.shared_playlist_enabled;
        if !playlist_scope_unchanged {
            self.playlist_undo_snapshot = None;
            self.playlist_source_undo_snapshot = None;
            self.playlist_entry_id_undo_snapshot = None;
        }
        let previous_playlist = if playlist_scope_unchanged {
            self.main_window.playlist.clone()
        } else {
            Vec::new()
        };
        let mut used_previous_rows = vec![false; previous_playlist.len()];
        let mut normalized_playlist = Vec::with_capacity(snapshot.playlist.len());
        let snapshot_playlist_entry_ids = if playlist_scope_unchanged {
            snapshot.playlist_entry_ids.clone()
        } else {
            Vec::new()
        };
        for (index, entry) in snapshot.playlist.into_iter().enumerate() {
            let Some(label) = normalized_editable_text(&entry) else {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain empty playlist entries.",
                );
            };
            let previous_row = Self::reconciled_playlist_row(
                &previous_playlist,
                &mut used_previous_rows,
                index,
                &label,
                snapshot_playlist_entry_ids.get(index).copied(),
            );
            let mut source_state = playlist_scope_unchanged
                .then(|| snapshot.playlist_source_states.get(index).cloned())
                .flatten()
                .map(|state| self.refreshed_playlist_source_state_for_entry(&label, state))
                .or_else(|| {
                    previous_row.as_ref().map(|row| {
                        self.refreshed_playlist_source_state_for_entry(
                            &label,
                            row.source_state.clone(),
                        )
                    })
                })
                .unwrap_or_else(|| self.playlist_source_state_for_entry(&label));
            if let Some(entry_id) = snapshot_playlist_entry_ids.get(index).copied() {
                source_state.entry_id = entry_id;
            }
            normalized_playlist.push(MainWindowPlaylistRow {
                entry_id: source_state.entry_id,
                label,
                is_selected: false,
                source_state,
            });
        }
        if snapshot
            .active_playlist_index
            .is_some_and(|index| index >= normalized_playlist.len())
        {
            return self.record_action_error(
                "Main-window runtime snapshots cannot activate a missing playlist row.",
            );
        }

        let mut normalized_chat = Vec::with_capacity(snapshot.chat.len());
        for row in snapshot.chat {
            let Some(sender) = normalized_editable_text(&row.sender) else {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain empty chat senders.",
                );
            };
            let Some(message) = normalized_editable_text(&row.message) else {
                return self.record_action_error(
                    "Main-window runtime snapshots cannot contain empty chat messages.",
                );
            };
            normalized_chat.push(MainWindowChatRow { sender, message });
        }

        let previously_selected_username = self
            .selection
            .selected_main_window_user
            .and_then(|index| self.main_window.users.get(index))
            .map(|user| user.username.clone());
        let previous_main_window_user_edit_session = self.main_window_user_edit_session.clone();
        let previously_selected_playlist_id = self
            .selection
            .selected_main_window_playlist
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let can_preserve_local_playlist_selection = self.main_window.room_name == room_name
            && self.main_window.shared_playlist_enabled == snapshot.shared_playlist_enabled;
        let local_readiness = normalized_users
            .iter()
            .find(|user| user.is_self)
            .and_then(|user| normalized_readiness.get(&user.username));
        let pending_local_ready_target = match local_readiness {
            Some(readiness) if readiness.protocol == ReadinessPresentationProtocol::V2 => {
                if readiness.pending_is_acknowledged() {
                    None
                } else if readiness.has_unacknowledged_pending_intent() {
                    Some(readiness.displayed_ready())
                } else {
                    None
                }
            }
            _ => self.pending_local_ready_target.filter(|target| {
                snapshot.can_set_ready
                    && normalized_users
                        .iter()
                        .find(|user| user.is_self)
                        .is_some_and(|user| user.is_ready != *target)
            }),
        };

        self.main_window = MainWindowShellState {
            room_name,
            room_control_status: snapshot.room_control_status,
            shared_playlist_enabled: snapshot.shared_playlist_enabled,
            controlled_room_active: snapshot.controlled_room_active,
            hide_empty_rooms: snapshot.hide_empty_rooms,
            rooms: normalized_rooms,
            users: normalized_users,
            readiness: normalized_readiness,
            playlist: normalized_playlist,
            playlist_default_source: self.refreshed_playlist_source_default_state(
                self.main_window.playlist_default_source.clone(),
            ),
            active_playlist_index: snapshot.active_playlist_index,
            chat: normalized_chat,
            playback: MainWindowPlaybackControls {
                can_toggle_pause: snapshot.can_toggle_pause,
                can_seek: snapshot.can_seek,
                can_undo_seek: snapshot.can_undo_seek,
                can_set_offset: snapshot.can_set_offset,
                can_toggle_autoplay: snapshot.can_toggle_autoplay,
                can_adjust_autoplay_threshold: snapshot.can_adjust_autoplay_threshold,
                can_set_ready: snapshot.can_set_ready,
                can_set_others_ready: snapshot.can_set_others_ready,
                can_manage_playlist: snapshot.can_manage_playlist,
            },
            playback_paused: snapshot.playback_paused,
            autoplay_active: snapshot.autoplay_active,
            autoplay_threshold: snapshot.autoplay_threshold,
            autoplay_countdown_seconds: snapshot.autoplay_countdown_seconds,
            user_offset_seconds: snapshot.user_offset_seconds,
            show_playback_buttons: snapshot.show_playback_buttons,
            show_autoplay_controls: snapshot.show_autoplay_controls,
        };
        self.pending_local_ready_target = pending_local_ready_target;
        self.set_menu_action_checked(
            MenuActionId::TogglePlaybackButtons,
            self.main_window.show_playback_buttons,
        );
        self.selection.selected_main_window_user = previously_selected_username
            .as_deref()
            .and_then(|username| {
                self.main_window
                    .users
                    .iter()
                    .position(|user| user.username == username)
            })
            .or_else(|| (!self.main_window.users.is_empty()).then_some(0));
        let preserve_local_playlist_selection = can_preserve_local_playlist_selection
            && self.main_window_playlist_selection_is_local
            && previously_selected_playlist_id.is_some_and(|entry_id| {
                self.main_window
                    .playlist
                    .iter()
                    .any(|row| row.entry_id == entry_id)
            });
        self.set_main_window_playlist_selection(
            previously_selected_playlist_id
                .and_then(|entry_id| {
                    self.main_window
                        .playlist
                        .iter()
                        .position(|row| row.entry_id == entry_id)
                })
                .or_else(|| (!self.main_window.playlist.is_empty()).then_some(0)),
            preserve_local_playlist_selection,
        );
        self.main_window_user_edit_session = previous_main_window_user_edit_session;
        self.normalize_main_window_user_edit_session();
        self.normalize_selection();
        self.apply_selection_to_surfaces();
        true
    }

    pub(super) fn apply_main_window_runtime_snapshot(
        &mut self,
        snapshot: MainWindowRuntimeSnapshot,
    ) -> bool {
        if !self.replace_main_window_runtime_snapshot(snapshot) {
            return false;
        }
        self.clear_action_error_and_refresh();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
        true
    }

    pub(super) fn apply_gui_runtime_snapshot(
        &mut self,
        snapshot: SorotteGuiRuntimeSnapshot,
    ) -> bool {
        let previously_selected_public_server_address = self
            .selected_public_server_index()
            .and_then(|index| self.public_servers.servers.get(index))
            .map(|row| row.address.clone());
        let previously_selected_media_search_path = self
            .selection
            .selected_media_search_directory
            .and_then(|index| self.media_search.directories.get(index))
            .map(|row| row.path.clone());

        if !self.replace_main_window_runtime_snapshot(snapshot.main_window) {
            return false;
        }

        self.active_view = snapshot.active_view;
        self.open_modal = snapshot.open_modal;
        self.menus.tls_prompt_expected = snapshot.tls_prompt_expected;
        self.menus.update_notice_expected = snapshot.update_notice_expected;
        self.menus.about_dialog_available = snapshot.about_dialog_available;
        self.sync_dialog_menu_actions_from_runtime_state();

        self.public_servers = snapshot.public_servers;
        self.public_servers.can_connect =
            self.public_servers.can_connect && !self.public_servers.servers.is_empty();
        let selected_public_server = self
            .public_servers
            .servers
            .iter()
            .position(|row| row.is_selected)
            .or_else(|| {
                previously_selected_public_server_address
                    .as_deref()
                    .and_then(|address| {
                        self.public_servers
                            .servers
                            .iter()
                            .position(|row| row.address == address)
                    })
            })
            .or_else(|| (!self.public_servers.servers.is_empty()).then_some(0));
        self.set_selected_public_server_index(selected_public_server);

        self.media_search = snapshot.media_search;
        self.media_search.can_search_missing_media =
            self.media_search.can_search_missing_media && !self.media_search.directories.is_empty();
        self.selection.selected_media_search_directory = self
            .media_search
            .directories
            .iter()
            .position(|row| row.is_selected)
            .or_else(|| {
                previously_selected_media_search_path
                    .as_deref()
                    .and_then(|path| {
                        self.media_search
                            .directories
                            .iter()
                            .position(|row| row.path == path)
                    })
            })
            .or_else(|| (!self.media_search.directories.is_empty()).then_some(0));

        self.normalize_selection();
        self.apply_selection_to_surfaces();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.normalize_focused_configuration_control();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "GUI runtime snapshot applied.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
        true
    }

    pub(super) fn push_system_chat_message(&mut self, message: String) {
        let message = localize_gui_runtime_message_legacy_compatible(
            &message,
            Some(self.runtime_language_tag_legacy_compatible()),
        );
        self.main_window.chat.push(MainWindowChatRow {
            sender: "system".to_owned(),
            message,
        });
    }

    pub(super) fn join_main_window_room(&mut self, room: String) -> bool {
        if nonempty_room_name_text(&room).is_none() {
            return self.record_action_error("Room name cannot be empty.");
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn leave_main_window_room(&mut self) -> bool {
        if joined_room_name_text(&self.main_window.room_name).is_none() {
            return self.record_action_error("No joined room is currently active.");
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn move_selected_media_search_directory(&mut self, delta: isize) -> bool {
        let Some(index) = self.selection.selected_media_search_directory else {
            return self.record_action_error("No media-search directory is currently selected.");
        };

        let mut settings = self.configuration.to_stored_settings();
        let mut directories = settings.media_search_directories.take().unwrap_or_default();
        let Some(target_index) = index.checked_add_signed(delta) else {
            return self
                .record_action_error("The selected media-search directory cannot move further.");
        };
        if index >= directories.len() || target_index >= directories.len() {
            return self
                .record_action_error("The selected media-search directory cannot move further.");
        }

        directories.swap(index, target_index);
        settings.media_search_directories = Some(directories);
        self.resync_from_settings(settings);
        self.selection.selected_media_search_directory = Some(target_index);
        self.apply_selection_to_surfaces();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Sorotte media directories have been updated.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn remove_selected_media_search_directory(&mut self) -> bool {
        let Some(index) = self.selection.selected_media_search_directory else {
            return self.record_action_error("No media-search directory is currently selected.");
        };

        let mut settings = self.configuration.to_stored_settings();
        let mut directories = settings.media_search_directories.take().unwrap_or_default();
        if index >= directories.len() {
            return self
                .record_action_error("No media-search directory exists at the requested index.");
        }

        directories.remove(index);
        settings.media_search_directories = if directories.is_empty() {
            None
        } else {
            Some(directories)
        };
        self.resync_from_settings(settings);
        self.selection.selected_media_search_directory = if self.media_search.directories.is_empty()
        {
            None
        } else if index >= self.media_search.directories.len() {
            Some(self.media_search.directories.len() - 1)
        } else {
            Some(index)
        };
        self.apply_selection_to_surfaces();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Sorotte media directories have been updated.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn push_transient_notification(
        &mut self,
        level: GuiTransientNotificationLevel,
        message: String,
    ) {
        let message = localize_gui_runtime_message_legacy_compatible(
            &message,
            Some(self.runtime_language_tag_legacy_compatible()),
        );
        self.notifications
            .push(GuiTransientNotification { level, message });
        const MAX_TRANSIENT_NOTIFICATIONS: usize = 5;
        if self.notifications.len() > MAX_TRANSIENT_NOTIFICATIONS {
            let overflow = self.notifications.len() - MAX_TRANSIENT_NOTIFICATIONS;
            self.notifications.drain(0..overflow);
        }
    }
}
