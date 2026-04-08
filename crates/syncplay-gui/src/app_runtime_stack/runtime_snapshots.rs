use super::super::DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD;
use super::super::shell_state::{
    GuiInteractionRuntimeSnapshot, MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot,
    MainWindowRuntimeUserSnapshot, MainWindowShellState, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SyncplayGuiShellAppState, browser_format_duration_label,
    browser_format_size_label, browser_is_url, browser_uri_is_trusted,
};
use super::super::support::{nonempty_room_name_text, normalized_editable_text};
use super::GuiClientCoreChatSessionRuntimeAdapter;

impl GuiClientCoreChatSessionRuntimeAdapter {
    fn room_control_status_for_runtime_snapshot(&self, controlled_room_active: bool) -> String {
        let session = self.runtime.session();
        if session.server_chat_supported().is_none() {
            return MainWindowShellState::room_control_status_waiting_for_server();
        }
        if !controlled_room_active {
            return MainWindowShellState::room_control_status_uncontrolled_room();
        }
        if session.local_can_control().unwrap_or(false) {
            MainWindowShellState::room_control_status_granted()
        } else {
            MainWindowShellState::room_control_status_locked()
        }
    }

    pub(super) fn shared_playlist_control_available(&self) -> bool {
        self.shared_playlist_server_supported()
            && self.runtime.session().local_can_control().unwrap_or(false)
    }

    pub(super) fn session_runtime_rooms(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<MainWindowRuntimeRoomSnapshot> {
        let session = self.runtime.session();
        let mut rooms = session
            .room_names()
            .into_iter()
            .filter_map(|room_name| {
                nonempty_room_name_text(&room_name).map(|room_name| MainWindowRuntimeRoomSnapshot {
                    has_named_users: !session.usernames_in_room(&room_name).is_empty(),
                    is_controlled: room_name.starts_with('+'),
                    room_name,
                })
            })
            .collect::<Vec<_>>();
        if rooms.is_empty()
            && let Some(room_name) = nonempty_room_name_text(&state.main_window.room_name)
        {
            rooms.push(MainWindowRuntimeRoomSnapshot {
                has_named_users: false,
                is_controlled: room_name.starts_with('+'),
                room_name,
            });
        }
        rooms
    }

    pub(super) fn session_runtime_users(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<MainWindowRuntimeUserSnapshot> {
        let session = self.runtime.session();
        let settings = state.configuration.to_stored_settings();
        let trusted_domains = settings.trusted_domains.unwrap_or_default();
        let only_switch_to_trusted_domains =
            settings.only_switch_to_trusted_domains.unwrap_or(true);
        let local_username = session.username.as_deref();
        let mut users = Vec::new();
        for room_name in session.room_names() {
            for username in session.usernames_in_room(&room_name) {
                let is_self = local_username == Some(username.as_str());
                let file_name = session
                    .user_file_name(&username)
                    .and_then(normalized_editable_text);
                let file_is_url = file_name.as_deref().is_some_and(browser_is_url);
                let file_is_trusted = file_name.as_deref().is_none_or(|file_name| {
                    browser_uri_is_trusted(
                        file_name,
                        only_switch_to_trusted_domains,
                        &trusted_domains,
                    )
                });
                let differences = session
                    .file_differences_for_user(&username)
                    .unwrap_or_default();
                users.push(MainWindowRuntimeUserSnapshot {
                    username: username.clone(),
                    room_name: room_name.clone(),
                    is_self,
                    is_ready: session.user_ready(&username).unwrap_or(false),
                    is_controller: session.user_controller(&username).unwrap_or(false),
                    has_file: session
                        .user_has_file(&username)
                        .unwrap_or(file_name.is_some()),
                    file_name,
                    file_size_label: browser_format_size_label(session.user_file_size(&username)),
                    file_duration_label: browser_format_duration_label(
                        session.user_file_duration(&username),
                    ),
                    file_is_url,
                    file_is_trusted,
                    filename_differs: differences.filename,
                    filesize_differs: differences.filesize,
                    fileduration_differs: differences.fileduration,
                });
            }
        }
        users
    }

    pub(super) fn main_window_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<MainWindowRuntimeSnapshot> {
        let baseline_main_window =
            MainWindowShellState::from_stored_settings(&state.configuration.to_stored_settings());
        let session = self.runtime.session();
        let shared_playlist_server_supported = self.shared_playlist_server_supported();
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        snapshot.room_name = baseline_main_window.room_name.clone();
        snapshot.room_control_status = baseline_main_window.room_control_status.clone();
        snapshot.shared_playlist_enabled =
            shared_playlist_server_supported && baseline_main_window.shared_playlist_enabled;
        snapshot.controlled_room_active = baseline_main_window.controlled_room_active;
        snapshot.hide_empty_rooms = state.main_window.hide_empty_rooms;
        snapshot.rooms = baseline_main_window
            .rooms
            .clone()
            .into_iter()
            .map(|room| MainWindowRuntimeRoomSnapshot {
                room_name: room.room_name,
                is_controlled: room.is_controlled,
                has_named_users: room.has_named_users,
            })
            .collect();
        snapshot.users = baseline_main_window
            .users
            .iter()
            .map(|user| MainWindowRuntimeUserSnapshot {
                username: user.username.clone(),
                room_name: user.room_name.clone(),
                is_self: user.is_self,
                is_ready: user.is_ready,
                is_controller: user.is_controller,
                has_file: user.has_file,
                file_name: user.file_name.clone(),
                file_size_label: user.file_size_label.clone(),
                file_duration_label: user.file_duration_label.clone(),
                file_is_url: user.file_is_url,
                file_is_trusted: user.file_is_trusted,
                filename_differs: user.filename_differs,
                filesize_differs: user.filesize_differs,
                fileduration_differs: user.fileduration_differs,
            })
            .collect();
        snapshot.playlist = baseline_main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect();
        snapshot.can_set_ready = baseline_main_window.playback.can_set_ready;
        snapshot.can_set_others_ready = baseline_main_window.playback.can_set_others_ready;
        snapshot.playback_paused = baseline_main_window.playback_paused;
        snapshot.autoplay_active = state.main_window.autoplay_active;
        snapshot.autoplay_threshold = state.main_window.autoplay_threshold;
        snapshot.autoplay_countdown_seconds = state.main_window.autoplay_countdown_seconds;
        snapshot.user_offset_seconds = state.main_window.user_offset_seconds;
        snapshot.show_playback_buttons = state.main_window.show_playback_buttons;
        snapshot.show_autoplay_controls = state.main_window.show_autoplay_controls;
        if let Some(room_name) = session
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let controlled_room_active = room_name.starts_with('+');
            snapshot.room_name = room_name.to_owned();
            snapshot.controlled_room_active = controlled_room_active;
            snapshot.rooms = self.session_runtime_rooms(state);
            snapshot.users = self.session_runtime_users(state);
        }
        snapshot.room_control_status =
            self.room_control_status_for_runtime_snapshot(snapshot.controlled_room_active);
        if shared_playlist_server_supported
            && let Some(playlist) = self.projected_current_room_playlist()
        {
            snapshot.shared_playlist_enabled = true;
            snapshot.playlist = playlist.files.clone();
        } else if !shared_playlist_server_supported {
            snapshot.playlist.clear();
        }
        snapshot.can_manage_playlist =
            snapshot.shared_playlist_enabled && self.shared_playlist_control_available();
        snapshot.can_undo_seek = session.last_seek_position_before_manual_seek().is_some();
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.autoplay_active = session.autoplay_enabled();
        snapshot.autoplay_threshold = session
            .readiness_autoplay_config()
            .auto_play_threshold
            .unwrap_or(DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD);
        snapshot.autoplay_countdown_seconds = session
            .autoplay_timer_is_running()
            .then(|| session.autoplay_time_left_seconds().max(0.0).floor() as u32);
        if let Some(playstate) = session.current_room_playstate()
            && let Some(paused) = playstate.paused
        {
            snapshot.playback_paused = paused;
        }
        if let Some(paused) = session.local_paused() {
            snapshot.playback_paused = paused;
        }
        if session.server_chat_supported().is_none() {
            snapshot.can_set_ready = false;
        } else if let Some(server_readiness_supported) = session.server_readiness_supported() {
            snapshot.can_set_ready = server_readiness_supported;
        }
        snapshot.can_set_others_ready = session
            .server_set_others_readiness_supported()
            .unwrap_or(false)
            && session.local_can_control().unwrap_or(false);
        (snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window))
            .then_some(snapshot)
    }

    pub(super) fn session_playlist_selection_index(&self, playlist_len: usize) -> Option<usize> {
        self.projected_current_room_playlist()
            .and_then(|playlist| playlist.index)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|&index| index < playlist_len)
    }

    pub(super) fn interaction_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
        playlist_len: usize,
    ) -> Option<GuiInteractionRuntimeSnapshot> {
        let selected_main_window_playlist = self.session_playlist_selection_index(playlist_len);
        if state.selection.selected_main_window_playlist == selected_main_window_playlist {
            return None;
        }

        let mut snapshot = GuiInteractionRuntimeSnapshot::from_shell_state(state);
        snapshot.selection.selected_main_window_playlist = selected_main_window_playlist;
        Some(snapshot)
    }

    pub(super) fn menu_dialog_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
        shared_playlist_enabled: bool,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let mut action_overrides = Vec::new();
        let settings = state.configuration.to_stored_settings();
        let session_room_name = self
            .runtime
            .session()
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let managed_rooms_server_supported = self.managed_rooms_server_supported();
        let create_controlled_room_enabled =
            managed_rooms_server_supported && session_room_name.is_some();
        let identify_as_controller_enabled = managed_rooms_server_supported
            && session_room_name.is_some_and(|room_name| room_name.starts_with('+'));
        let config_chat_enabled = settings.chat_input_enabled.unwrap_or(false)
            || settings.chat_output_enabled.unwrap_or(false);
        let desired_show_chat_enabled =
            config_chat_enabled && self.runtime.session().server_chat_supported() == Some(true);

        let current_show_chat_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .map(|action| action.enabled);
        if current_show_chat_enabled
            .is_some_and(|current_enabled| current_enabled != desired_show_chat_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: desired_show_chat_enabled,
            });
        }

        let current_show_playlist_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .map(|action| action.enabled);
        if current_show_playlist_enabled
            .is_some_and(|current_enabled| current_enabled != shared_playlist_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: shared_playlist_enabled,
            });
        }

        let current_playlist_actions_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Playlist Actions")
            })
            .map(|action| action.enabled);
        let desired_playlist_actions_enabled =
            shared_playlist_enabled && self.shared_playlist_control_available();
        if current_playlist_actions_enabled
            .is_some_and(|current_enabled| current_enabled != desired_playlist_actions_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: desired_playlist_actions_enabled,
            });
        }

        for (action_label, enabled) in [
            ("Create Controlled Room", create_controlled_room_enabled),
            ("Identify As Controller", identify_as_controller_enabled),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == "Advanced")
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label,
                    enabled,
                });
            }
        }

        if action_overrides.is_empty() {
            return None;
        }

        Some(MenuDialogRuntimeSnapshot {
            action_overrides,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        })
    }
}
