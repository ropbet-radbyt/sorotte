//! Main-window model projection without UI editing or navigation state.
use super::playlist_model::GuiPlaylistSources;
use super::shell_state::*;
use super::support::{nonempty_room_name_text, normalized_editable_text};
use sorotte_client_app::app_boundary::readiness::ReadinessPresentationProtocol;
use sorotte_client_app::app_boundary::state::StoredClientSettings;

pub(super) struct GuiMainWindowProjection<'a> {
    pub(super) main_window: &'a mut MainWindowShellState,
    pub(super) selection: &'a mut GuiSelectionState,
    pub(super) main_window_playlist_selection_is_local: &'a mut bool,
    pub(super) playlist_undo_snapshot: &'a mut Option<Vec<String>>,
    pub(super) playlist_source_undo_snapshot: &'a mut Option<Vec<GuiPlaylistSourceState>>,
    pub(super) playlist_entry_id_undo_snapshot: &'a mut Option<Vec<GuiPlaylistEntryId>>,
    pub(super) pending_local_ready_target: &'a mut Option<bool>,
    pub(super) menus: &'a mut MenuDialogShellState,
    pub(super) sources: GuiPlaylistSources<'a>,
}
impl GuiMainWindowProjection<'_> {
    pub(super) fn apply(
        &mut self,
        snapshot: MainWindowRuntimeSnapshot,
    ) -> Result<(), &'static str> {
        let Some(room_name) = nonempty_room_name_text(&snapshot.room_name) else {
            return Err("Main-window runtime snapshots must include a non-empty room name.");
        };
        if !snapshot.playlist_entry_ids.is_empty()
            && snapshot.playlist_entry_ids.len() != snapshot.playlist.len()
        {
            return Err(
                "Main-window runtime snapshots must align playlist row identities with entries.",
            );
        }

        let mut normalized_rooms = Vec::with_capacity(snapshot.rooms.len());
        for room in snapshot.rooms {
            let Some(room_name) = nonempty_room_name_text(&room.room_name) else {
                return Err("Main-window runtime snapshots cannot contain empty room names.");
            };
            if normalized_rooms.iter().any(|existing: &MainWindowRoomRow| {
                existing.room_name.eq_ignore_ascii_case(&room_name)
            }) {
                return Err("Main-window runtime snapshots cannot contain duplicate room names.");
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
                return Err("Main-window runtime snapshots cannot contain empty user names.");
            };
            let user_room_name =
                nonempty_room_name_text(&user.room_name).unwrap_or_else(|| room_name.clone());
            if normalized_users.iter().any(|existing: &MainWindowUserRow| {
                existing.username.eq_ignore_ascii_case(&username)
            }) {
                return Err("Main-window runtime snapshots cannot contain duplicate user names.");
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
                participant_status: user.participant_status,
                start_barrier_status: user.start_barrier_status,
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
            (*self.playlist_undo_snapshot) = None;
            (*self.playlist_source_undo_snapshot) = None;
            (*self.playlist_entry_id_undo_snapshot) = None;
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
                return Err("Main-window runtime snapshots cannot contain empty playlist entries.");
            };
            let previous_row = super::playlist_model::reconciled_playlist_row(
                &previous_playlist,
                &mut used_previous_rows,
                index,
                &label,
                snapshot_playlist_entry_ids.get(index).copied(),
            );
            let mut source_state = playlist_scope_unchanged
                .then(|| snapshot.playlist_source_states.get(index).cloned())
                .flatten()
                .map(|state| {
                    self.sources
                        .refreshed_playlist_source_state_for_entry(&label, state)
                })
                .or_else(|| {
                    previous_row.as_ref().map(|row| {
                        self.sources.refreshed_playlist_source_state_for_entry(
                            &label,
                            row.source_state.clone(),
                        )
                    })
                })
                .unwrap_or_else(|| self.sources.playlist_source_state_for_entry(&label));
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
            return Err("Main-window runtime snapshots cannot activate a missing playlist row.");
        }

        let mut normalized_chat = Vec::with_capacity(snapshot.chat.len());
        for row in snapshot.chat {
            let Some(sender) = normalized_editable_text(&row.sender) else {
                return Err("Main-window runtime snapshots cannot contain empty chat senders.");
            };
            let Some(message) = normalized_editable_text(&row.message) else {
                return Err("Main-window runtime snapshots cannot contain empty chat messages.");
            };
            normalized_chat.push(MainWindowChatRow { sender, message });
        }

        let previously_selected_username = self
            .selection
            .selected_main_window_user
            .and_then(|index| self.main_window.users.get(index))
            .map(|user| user.username.clone());
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
            _ => (*self.pending_local_ready_target).filter(|target| {
                snapshot.can_set_ready
                    && normalized_users
                        .iter()
                        .find(|user| user.is_self)
                        .is_some_and(|user| user.is_ready != *target)
            }),
        };

        (*self.main_window) = MainWindowShellState {
            room_name,
            room_control_status: snapshot.room_control_status,
            shared_playlist_enabled: snapshot.shared_playlist_enabled,
            controlled_room_active: snapshot.controlled_room_active,
            hide_empty_rooms: snapshot.hide_empty_rooms,
            rooms: normalized_rooms,
            users: normalized_users,
            readiness: normalized_readiness,
            room_playback_intent: snapshot.room_playback_intent,
            playlist: normalized_playlist,
            playlist_default_source: self.sources.refreshed_playlist_source_default_state(
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
        (*self.pending_local_ready_target) = pending_local_ready_target;
        self.set_menu_action_checked(
            MenuActionId::TogglePlaybackButtons,
            self.main_window.show_playback_buttons,
        );
        self.set_menu_action_checked(
            MenuActionId::ToggleAutoplayControls,
            self.main_window.show_autoplay_controls,
        );
        self.set_menu_action_checked(
            MenuActionId::ToggleHideEmptyRooms,
            self.main_window.hide_empty_rooms,
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
            && (*self.main_window_playlist_selection_is_local)
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
        Ok(())
    }

    fn set_menu_action_checked(&mut self, id: MenuActionId, checked: bool) {
        if let Some(action) = self.menus.action_mut(id) {
            action.is_checked = checked;
        }
    }
    fn set_main_window_playlist_selection(&mut self, index: Option<usize>, local: bool) {
        self.selection.selected_main_window_playlist = index;
        *self.main_window_playlist_selection_is_local = local && index.is_some();
    }
}

impl GuiMainWindowProjection<'_> {
    pub(super) fn reapply_runtime_main_window_surface_from_snapshot(
        &mut self,
        connected: bool,
        previous_settings: &StoredClientSettings,
        current_snapshot: &MainWindowRuntimeSnapshot,
    ) {
        let previous_baseline = MainWindowRuntimeSnapshot::from_shell_state(
            &MainWindowShellState::from_stored_settings(previous_settings),
        );
        let preserve_connected_room_surface = connected;
        let configured_playlist = self
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>();

        if preserve_connected_room_surface
            || current_snapshot.room_name != previous_baseline.room_name
        {
            self.main_window.room_name = current_snapshot.room_name.clone();
        }
        if preserve_connected_room_surface
            || current_snapshot.room_control_status != previous_baseline.room_control_status
        {
            self.main_window.room_control_status = current_snapshot.room_control_status.clone();
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
            self.set_menu_action_checked(
                MenuActionId::ToggleHideEmptyRooms,
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
                    participant_status: user.participant_status.clone(),
                    start_barrier_status: user.start_barrier_status.clone(),
                    is_selected: false,
                })
                .collect();
        }
        if preserve_connected_room_surface
            || current_snapshot.room_playback_intent != previous_baseline.room_playback_intent
        {
            self.main_window.room_playback_intent = current_snapshot.room_playback_intent.clone();
        }
        let preserve_runtime_playlist = current_snapshot.playlist != previous_baseline.playlist
            || configured_playlist == previous_baseline.playlist;
        if preserve_runtime_playlist {
            self.remember_shared_playlist_undo_snapshot_if_changed(&current_snapshot.playlist);
            let previous_rows = self.main_window.playlist.clone();
            let mut used_previous_rows = vec![false; previous_rows.len()];
            self.main_window.playlist = current_snapshot
                .playlist
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    let previous_row = super::playlist_model::reconciled_playlist_row(
                        &previous_rows,
                        &mut used_previous_rows,
                        index,
                        label,
                        current_snapshot.playlist_entry_ids.get(index).copied(),
                    );
                    let mut source_state = current_snapshot
                        .playlist_source_states
                        .get(index)
                        .cloned()
                        .map(|state| {
                            self.sources
                                .refreshed_playlist_source_state_for_entry(label, state)
                        })
                        .or_else(|| {
                            previous_row.as_ref().map(|row| {
                                self.sources.refreshed_playlist_source_state_for_entry(
                                    label,
                                    row.source_state.clone(),
                                )
                            })
                        })
                        .unwrap_or_else(|| self.sources.playlist_source_state_for_entry(label));
                    if let Some(entry_id) = current_snapshot.playlist_entry_ids.get(index).copied()
                    {
                        source_state.entry_id = entry_id;
                    }
                    super::shell_state::MainWindowPlaylistRow {
                        entry_id: source_state.entry_id,
                        label: label.clone(),
                        is_selected: false,
                        source_state,
                    }
                })
                .collect();
        }
        if current_snapshot.playlist != previous_baseline.playlist
            || current_snapshot.playlist_entry_ids != previous_baseline.playlist_entry_ids
            || current_snapshot.active_playlist_index != previous_baseline.active_playlist_index
        {
            self.main_window.active_playlist_index = current_snapshot
                .active_playlist_index
                .filter(|index| *index < self.main_window.playlist.len());
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
            self.set_menu_action_checked(
                MenuActionId::TogglePlaybackButtons,
                current_snapshot.show_playback_buttons,
            );
        }
        if current_snapshot.show_autoplay_controls != previous_baseline.show_autoplay_controls {
            self.main_window.show_autoplay_controls = current_snapshot.show_autoplay_controls;
            self.set_menu_action_checked(
                MenuActionId::ToggleAutoplayControls,
                current_snapshot.show_autoplay_controls,
            );
        }
    }

    fn remember_shared_playlist_undo_snapshot_if_changed(&mut self, next: &[String]) {
        let current =
            super::playlist_model::current_shared_playlist_entries(&self.main_window.playlist);
        if current != next {
            *self.playlist_source_undo_snapshot = Some(
                self.main_window
                    .playlist
                    .iter()
                    .map(|row| row.source_state.clone())
                    .collect(),
            );
            *self.playlist_entry_id_undo_snapshot = Some(
                self.main_window
                    .playlist
                    .iter()
                    .map(|row| row.entry_id)
                    .collect(),
            );
            *self.playlist_undo_snapshot = Some(current);
        }
    }
}
