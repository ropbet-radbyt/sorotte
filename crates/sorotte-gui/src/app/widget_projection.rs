use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

use super::shell_state::{
    MainWindowChatRow, MainWindowRoomRow, MainWindowRuntimeSnapshot, MainWindowShellState,
    MainWindowUserRow, MediaSearchWorkflowRuntimeFlags, MediaSearchWorkflowShellState,
    MenuDialogShellState, PublicServerBrowserRuntimeFlags, PublicServerBrowserShellState,
    SorotteGuiShellAppState,
};

impl SorotteGuiShellAppState {
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
        if current_snapshot.playlist != previous_baseline.playlist
            || current_snapshot.playlist_source_states != previous_baseline.playlist_source_states
        {
            self.remember_shared_playlist_undo_snapshot_if_changed(&current_snapshot.playlist);
            let previous_rows = self.main_window.playlist.clone();
            let mut used_previous_rows = vec![false; previous_rows.len()];
            self.main_window.playlist = current_snapshot
                .playlist
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    let source_state = current_snapshot
                        .playlist_source_states
                        .get(index)
                        .cloned()
                        .map(|state| self.refreshed_playlist_source_state_for_entry(label, state))
                        .or_else(|| {
                            Self::reconciled_playlist_source_state(
                                &previous_rows,
                                &mut used_previous_rows,
                                index,
                                label,
                            )
                            .map(|state| {
                                self.refreshed_playlist_source_state_for_entry(label, state)
                            })
                        })
                        .unwrap_or_else(|| self.playlist_source_state_for_entry(label));
                    super::shell_state::MainWindowPlaylistRow {
                        label: label.clone(),
                        is_selected: false,
                        source_state,
                    }
                })
                .collect();
        }
        if current_snapshot.playlist != previous_baseline.playlist
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
        let preserved_media_index_status = self.media_index_status.clone();
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
        self.media_index_status = preserved_media_index_status;
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
        let selected_plugin = self.selected_plugin;
        let open_modal = self.open_modal;
        let selection = self.selection.clone();
        let runtime_menu_action_overrides = self.runtime_menu_action_overrides.clone();
        let runtime_command_availability_override =
            self.runtime_command_availability_override.clone();
        let pending_operation = self.pending_operation.clone();
        let outgoing_chat_message = self.outgoing_chat_message.clone();
        let new_main_window_user_draft = self.new_main_window_user_draft.clone();
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
        let media_index_status = self.media_index_status.clone();
        let player_setup_issue = self.player_setup_issue.clone();
        let plex = self.plex.clone();
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
        self.selected_plugin = selected_plugin;
        self.open_modal = open_modal;
        self.selection = selection;
        self.runtime_menu_action_overrides = runtime_menu_action_overrides;
        self.runtime_command_availability_override = runtime_command_availability_override;
        self.pending_operation = pending_operation;
        self.outgoing_chat_message = outgoing_chat_message;
        self.new_main_window_user_draft = new_main_window_user_draft;
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
        self.media_index_status = media_index_status;
        self.player_setup_issue = player_setup_issue;
        self.plex = plex;
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
