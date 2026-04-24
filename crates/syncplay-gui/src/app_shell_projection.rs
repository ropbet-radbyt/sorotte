use syncplay_client_app::app_boundary::state::{
    StoredClientSettingsMvp, stored_client_settings_runtime_snapshot_legacy_compatible,
};

use super::shell_state::{
    FirstRunConfigurationDialogDraft, GuiCommandAvailabilityRuntimeOverride,
    GuiCommandAvailabilityState, GuiControlledRoomCreateSessionState,
    GuiControllerAuthEditSessionState, GuiFocusedConfigurationControlState,
    GuiMainWindowUserEditSessionState, GuiMediaIndexStatusState, GuiPendingOperationState,
    GuiPlaylistTextEditSessionState, GuiPublicServerEditSessionState,
    GuiRoomHistoryEditSessionState, GuiSelectionState, GuiShellModal, GuiTextEditSessionState,
    GuiTransientNotification, GuiUrlEditSessionState, GuiValidationState, MainWindowChatRow,
    MainWindowPlaybackControls, MainWindowPlaylistRow, MainWindowRoomRow, MainWindowShellState,
    MainWindowUserRow, MediaSearchDirectoryRow, MediaSearchWorkflowRuntimeFlags,
    MediaSearchWorkflowShellState, MenuActionShellItem, MenuDialogShellState,
    MenuSectionShellState, PublicServerBrowserRow, PublicServerBrowserRuntimeFlags,
    PublicServerBrowserShellState, SyncplayGuiShellAppState,
};
use super::support::{
    autoplay_threshold_from_settings, bool_label, optional_index_text, optional_seconds_text,
};

impl FirstRunConfigurationDialogDraft {
    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "syncplay-gui setup surface initialized in {} mode ({} startup entries, {} ignored exception).",
            self.launch_mode.label(),
            self.compatibility_startup_entry_count,
            self.ignored_startup_exception_count,
        )];

        for section in &self.sections {
            lines.push(format!("[{}]", section.title));
            for control in &section.controls {
                lines.push(format!(
                    "- {} [{}]: {}",
                    control.label,
                    control.kind.label(),
                    control.value
                ));
            }
        }

        lines.push(
            "Native window widgets use a room-first shell with a grouped setup state model, a typed dialog control schema, and an editable draft that round-trips back into shared client settings."
                .to_owned(),
        );
        lines
    }
}

impl GuiValidationState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "[Validation] status={}, last_action_error={}",
            if self.issues.is_empty() {
                "clean".to_owned()
            } else {
                format!("{} issue(s)", self.issues.len())
            },
            self.last_action_error.as_deref().unwrap_or("(none)")
        )];

        for issue in &self.issues {
            lines.push(format!(
                "- {} / {}: {}",
                issue.scope, issue.label, issue.message
            ));
        }

        lines
    }
}

impl GuiSelectionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Selection] user={}, playlist={}, menu={}, media_directory={}",
            optional_index_text(self.selected_main_window_user),
            optional_index_text(self.selected_main_window_playlist),
            self.selected_menu_action.map_or_else(
                || "(none)".to_owned(),
                |(section, action)| format!("{section}:{action}")
            ),
            optional_index_text(self.selected_media_search_directory),
        )]
    }
}

impl GuiCommandAvailabilityState {
    pub(super) fn any_enabled(&self) -> bool {
        self.can_save_configuration
            || self.can_reset_configuration
            || self.can_reload_configuration
            || self.can_connect_saved_server
            || self.can_disconnect_session
            || self.can_connect_public_server
            || self.can_refresh_public_servers
            || self.can_search_missing_media
            || self.can_toggle_pause
            || self.can_send_chat_message
    }

    pub(super) fn render_lines(
        &self,
        pending_operation: Option<&GuiPendingOperationState>,
    ) -> Vec<String> {
        vec![
            format!(
                "[Commands] busy={}, save_configuration={}, reset_configuration={}, reload_configuration={}, connect_saved_server={}, disconnect_session={}, connect_public_server={}, refresh_public_servers={}, search_missing_media={}, toggle_pause={}, send_chat_message={}",
                bool_label(pending_operation.is_some()),
                bool_label(self.can_save_configuration),
                bool_label(self.can_reset_configuration),
                bool_label(self.can_reload_configuration),
                bool_label(self.can_connect_saved_server),
                bool_label(self.can_disconnect_session),
                bool_label(self.can_connect_public_server),
                bool_label(self.can_refresh_public_servers),
                bool_label(self.can_search_missing_media),
                bool_label(self.can_toggle_pause),
                bool_label(self.can_send_chat_message),
            ),
            format!(
                "[Pending] operation={}",
                pending_operation
                    .map(|pending| pending.kind.label())
                    .unwrap_or("(none)")
            ),
        ]
    }
}

impl GuiCommandAvailabilityRuntimeOverride {
    pub(super) fn from_baseline_and_snapshot(
        baseline: &GuiCommandAvailabilityState,
        snapshot: &GuiCommandAvailabilityState,
    ) -> Self {
        Self {
            can_save_configuration: (baseline.can_save_configuration
                != snapshot.can_save_configuration)
                .then_some(snapshot.can_save_configuration),
            can_reset_configuration: (baseline.can_reset_configuration
                != snapshot.can_reset_configuration)
                .then_some(snapshot.can_reset_configuration),
            can_reload_configuration: (baseline.can_reload_configuration
                != snapshot.can_reload_configuration)
                .then_some(snapshot.can_reload_configuration),
            can_connect_saved_server: (baseline.can_connect_saved_server
                != snapshot.can_connect_saved_server)
                .then_some(snapshot.can_connect_saved_server),
            can_disconnect_session: (baseline.can_disconnect_session
                != snapshot.can_disconnect_session)
                .then_some(snapshot.can_disconnect_session),
            can_connect_public_server: (baseline.can_connect_public_server
                != snapshot.can_connect_public_server)
                .then_some(snapshot.can_connect_public_server),
            can_refresh_public_servers: (baseline.can_refresh_public_servers
                != snapshot.can_refresh_public_servers)
                .then_some(snapshot.can_refresh_public_servers),
            can_search_missing_media: (baseline.can_search_missing_media
                != snapshot.can_search_missing_media)
                .then_some(snapshot.can_search_missing_media),
            can_toggle_pause: (baseline.can_toggle_pause != snapshot.can_toggle_pause)
                .then_some(snapshot.can_toggle_pause),
            can_send_chat_message: (baseline.can_send_chat_message
                != snapshot.can_send_chat_message)
                .then_some(snapshot.can_send_chat_message),
        }
    }

    pub(super) fn apply_to(&self, command_availability: &mut GuiCommandAvailabilityState) {
        if let Some(value) = self.can_save_configuration {
            command_availability.can_save_configuration = value;
        }
        if let Some(value) = self.can_reset_configuration {
            command_availability.can_reset_configuration = value;
        }
        if let Some(value) = self.can_reload_configuration {
            command_availability.can_reload_configuration = value;
        }
        if let Some(value) = self.can_connect_saved_server {
            command_availability.can_connect_saved_server = value;
        }
        if let Some(value) = self.can_disconnect_session {
            command_availability.can_disconnect_session = value;
        }
        if let Some(value) = self.can_connect_public_server {
            command_availability.can_connect_public_server = value;
        }
        if let Some(value) = self.can_refresh_public_servers {
            command_availability.can_refresh_public_servers = value;
        }
        if let Some(value) = self.can_search_missing_media {
            command_availability.can_search_missing_media = value;
        }
        if let Some(value) = self.can_toggle_pause {
            command_availability.can_toggle_pause = value;
        }
        if let Some(value) = self.can_send_chat_message {
            command_availability.can_send_chat_message = value;
        }
    }

    pub(super) fn normalize_for_baseline(&mut self, baseline: &GuiCommandAvailabilityState) {
        if self.can_save_configuration == Some(baseline.can_save_configuration) {
            self.can_save_configuration = None;
        }
        if self.can_reset_configuration == Some(baseline.can_reset_configuration) {
            self.can_reset_configuration = None;
        }
        if self.can_reload_configuration == Some(baseline.can_reload_configuration) {
            self.can_reload_configuration = None;
        }
        if self.can_connect_saved_server == Some(baseline.can_connect_saved_server) {
            self.can_connect_saved_server = None;
        }
        if self.can_disconnect_session == Some(baseline.can_disconnect_session) {
            self.can_disconnect_session = None;
        }
        if self.can_connect_public_server == Some(baseline.can_connect_public_server) {
            self.can_connect_public_server = None;
        }
        if self.can_refresh_public_servers == Some(baseline.can_refresh_public_servers) {
            self.can_refresh_public_servers = None;
        }
        if self.can_search_missing_media == Some(baseline.can_search_missing_media) {
            self.can_search_missing_media = None;
        }
        if self.can_toggle_pause == Some(baseline.can_toggle_pause) {
            self.can_toggle_pause = None;
        }
        if self.can_send_chat_message == Some(baseline.can_send_chat_message) {
            self.can_send_chat_message = None;
        }
    }
}

impl GuiFocusedConfigurationControlState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Control Focus] focused={} / {}, kind={}, activations={}",
            self.section,
            self.label,
            self.kind.label(),
            self.activation_count
        )]
    }
}

impl GuiPublicServerEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Public Server Edit] editing_index={}, dirty={}, label={}, address={}",
            self.editing_index
                .map_or_else(|| "(new)".to_owned(), |index| index.to_string()),
            bool_label(self.is_dirty),
            self.label_buffer,
            self.address_buffer,
        )]
    }
}

impl GuiMainWindowUserEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Main Window User Edit] editing_index={}, dirty={}, username={}",
            self.editing_index,
            bool_label(self.is_dirty),
            self.username_buffer,
        )]
    }
}

impl GuiTransientNotification {
    fn render_line(&self) -> String {
        format!("- {}: {}", self.level.label(), self.message)
    }
}

impl GuiMediaIndexStatusState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Media Index] active={}, message={}",
            bool_label(self.active),
            self.message.as_deref().unwrap_or("(idle)")
        )]
    }
}

impl GuiTextEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Text Edit] editing={} / {}, dirty={}, buffer={}",
            self.section,
            self.label,
            bool_label(self.is_dirty),
            self.buffer
        )]
    }
}

impl GuiPlaylistTextEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Playlist Edit] dirty={}, entries={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}

impl GuiUrlEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[URL Edit] dirty={}, lines={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}

impl GuiControlledRoomCreateSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Controlled Room Create] dirty={}, room={}",
            bool_label(self.is_dirty),
            self.room_buffer
        )]
    }
}

impl GuiControllerAuthEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Controller Auth Edit] dirty={}, room={}, password_set={}",
            bool_label(self.is_dirty),
            self.room_name,
            bool_label(!self.password_buffer.is_empty())
        )]
    }
}

impl GuiRoomHistoryEditSessionState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        vec![format!(
            "[Room History Edit] dirty={}, entries={}",
            bool_label(self.is_dirty),
            self.buffer.lines().count()
        )]
    }
}

impl MainWindowShellState {
    pub(super) fn room_control_status_without_session() -> String {
        "Unavailable: no active server session.".to_owned()
    }

    pub(super) fn room_control_status_waiting_for_server() -> String {
        "Pending: waiting for server room state.".to_owned()
    }

    pub(super) fn room_control_status_uncontrolled_room() -> String {
        "Not required: current room is not controlled.".to_owned()
    }

    pub(super) fn room_control_status_granted() -> String {
        "Granted by server: you control this room.".to_owned()
    }

    pub(super) fn room_control_status_locked() -> String {
        "Not granted by server: room controls are locked.".to_owned()
    }

    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
        let room_name = runtime_settings
            .settings
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("(no room joined)")
            .to_owned();
        let username = settings
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("You")
            .to_owned();
        let shared_playlist_enabled = settings.shared_playlist_enabled.unwrap_or(false);
        let controlled_room_active = room_name.starts_with('+');

        let mut playlist = Vec::new();
        if shared_playlist_enabled {
            playlist.push(MainWindowPlaylistRow {
                label: "Playlist pane ready for shared entries".to_owned(),
                is_selected: true,
            });
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
            active_playlist_index: None,
            chat: if settings.chat_output_enabled.unwrap_or(false) {
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
            autoplay_active: settings.autoplay_initial_state.unwrap_or(false),
            autoplay_threshold: autoplay_threshold_from_settings(settings),
            autoplay_countdown_seconds: None,
            user_offset_seconds: 0.0,
            show_playback_buttons: true,
            show_autoplay_controls: true,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<String> {
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

impl MenuDialogShellState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let shared_playlist_enabled = settings.shared_playlist_enabled.unwrap_or(false);
        let chat_enabled = settings.chat_input_enabled.unwrap_or(false)
            || settings.chat_output_enabled.unwrap_or(false);

        Self {
            sections: vec![
                MenuSectionShellState {
                    title: "File",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Open Media File",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Open Media Search",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Open Public Server Browser",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Exit",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Playback",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Play",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Pause",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Toggle Pause",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Seek",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Undo Seek",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Shared Playlist",
                            enabled: false,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Advanced",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Create Controlled Room",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Identify As Controller",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Trusted Domains",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Set Offset",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "TLS Certificates",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Update Check",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Window",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Show Chat",
                            enabled: chat_enabled,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Show Playlist",
                            enabled: shared_playlist_enabled,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Show Users",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Playback Buttons",
                            enabled: true,
                            is_selected: true,
                        },
                        MenuActionShellItem {
                            label: "Autoplay",
                            enabled: true,
                            is_selected: true,
                        },
                        MenuActionShellItem {
                            label: "Hide Empty Rooms",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Help",
                    actions: vec![
                        MenuActionShellItem {
                            label: "About",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Manual / Command Help",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Check for Updates",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
            ],
            tls_prompt_expected: settings.only_switch_to_trusted_domains.unwrap_or(false),
            update_notice_expected: false,
            about_dialog_available: true,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec!["[Menus & Dialogs]".to_owned()];

        for section in &self.sections {
            lines.push(format!("{}:", section.title));
            for action in &section.actions {
                lines.push(format!(
                    "- {} [enabled={}, selected={}]",
                    action.label,
                    bool_label(action.enabled),
                    bool_label(action.is_selected),
                ));
            }
        }

        lines.push(format!(
            "Dialog Prompts: tls_certificate={}, update_notice={}, about={}",
            bool_label(self.tls_prompt_expected),
            bool_label(self.update_notice_expected),
            bool_label(self.about_dialog_available),
        ));

        lines
    }
}

impl PublicServerBrowserShellState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let servers = settings
            .public_servers
            .as_ref()
            .map(|entries| {
                entries
                    .iter()
                    .enumerate()
                    .map(|(index, (label, address))| PublicServerBrowserRow {
                        label: label.clone(),
                        address: address.clone(),
                        is_selected: index == 0,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Self {
            can_connect: !servers.is_empty(),
            can_refresh: true,
            can_add_custom_server: true,
            servers,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "[Public Server Browser]".to_owned(),
            format!(
                "Actions: connect={}, refresh={}, add_custom={}",
                bool_label(self.can_connect),
                bool_label(self.can_refresh),
                bool_label(self.can_add_custom_server),
            ),
            format!("Servers ({}):", self.servers.len()),
        ];

        if self.servers.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for server in &self.servers {
                lines.push(format!(
                    "- {} @ {} [selected={}]",
                    server.label,
                    server.address,
                    bool_label(server.is_selected),
                ));
            }
        }

        lines
    }

    pub(super) fn apply_runtime_flags(&mut self, runtime_flags: PublicServerBrowserRuntimeFlags) {
        self.can_connect = runtime_flags.can_connect && !self.servers.is_empty();
        self.can_refresh = runtime_flags.can_refresh;
        self.can_add_custom_server = runtime_flags.can_add_custom_server;
    }
}

impl PublicServerBrowserRuntimeFlags {
    pub(super) fn from_shell_state(state: &PublicServerBrowserShellState) -> Self {
        Self {
            can_connect: state.can_connect,
            can_refresh: state.can_refresh,
            can_add_custom_server: state.can_add_custom_server,
        }
    }
}

impl MediaSearchWorkflowShellState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let directories = settings
            .media_search_directories
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|path| MediaSearchDirectoryRow {
                path,
                is_selected: false,
            })
            .collect::<Vec<_>>();

        Self {
            can_browse_directories: true,
            can_search_missing_media: !directories.is_empty(),
            first_file_timeout_seconds: settings.folder_search_first_file_timeout_seconds,
            search_timeout_seconds: settings.folder_search_timeout_seconds,
            double_check_interval_seconds: settings.folder_search_double_check_interval_seconds,
            warning_threshold_seconds: settings.folder_search_warning_threshold_seconds,
            directories,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "[Media Search Workflow]".to_owned(),
            format!(
                "Actions: browse_directories={}, search_missing_media={}",
                bool_label(self.can_browse_directories),
                bool_label(self.can_search_missing_media),
            ),
            format!(
                "Timing: first_file={}, search={}, double_check={}, warning={}",
                optional_seconds_text(self.first_file_timeout_seconds),
                optional_seconds_text(self.search_timeout_seconds),
                optional_seconds_text(self.double_check_interval_seconds),
                optional_seconds_text(self.warning_threshold_seconds),
            ),
            format!("Directories ({}):", self.directories.len()),
        ];

        if self.directories.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for directory in &self.directories {
                lines.push(format!(
                    "- {} [selected={}]",
                    directory.path,
                    bool_label(directory.is_selected),
                ));
            }
        }

        lines
    }

    pub(super) fn apply_runtime_flags(&mut self, runtime_flags: MediaSearchWorkflowRuntimeFlags) {
        self.can_browse_directories = runtime_flags.can_browse_directories;
        self.can_search_missing_media =
            runtime_flags.can_search_missing_media && !self.directories.is_empty();
    }
}

impl MediaSearchWorkflowRuntimeFlags {
    pub(super) fn from_shell_state(state: &MediaSearchWorkflowShellState) -> Self {
        Self {
            can_browse_directories: state.can_browse_directories,
            can_search_missing_media: state.can_search_missing_media,
        }
    }
}

impl SyncplayGuiShellAppState {
    pub(super) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "[Shell App State] active_view={}, open_modal={}",
            self.active_view.label(),
            self.open_modal
                .map(GuiShellModal::label)
                .unwrap_or("(none)")
        )];
        lines.extend(self.selection.render_lines());
        lines.extend(self.commands.render_lines(self.pending_operation.as_ref()));
        lines.push(format!(
            "[Chat Send] pending_message={}",
            self.outgoing_chat_message.as_deref().unwrap_or("(none)")
        ));
        lines.extend(self.media_index_status.render_lines());
        lines.extend(
            self.focused_configuration_control
                .as_ref()
                .map(GuiFocusedConfigurationControlState::render_lines)
                .unwrap_or_else(|| vec!["[Control Focus] focused=(none)".to_owned()]),
        );
        lines.extend(
            self.public_server_edit_session
                .as_ref()
                .map(GuiPublicServerEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Public Server Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.main_window_user_edit_session
                .as_ref()
                .map(GuiMainWindowUserEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Main Window User Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.text_edit_session
                .as_ref()
                .map(GuiTextEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Text Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.playlist_text_edit_session
                .as_ref()
                .map(GuiPlaylistTextEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Playlist Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.playlist_url_edit_session
                .as_ref()
                .map(GuiUrlEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Playlist URL Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.media_url_edit_session
                .as_ref()
                .map(GuiUrlEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Media URL Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.controlled_room_create_session
                .as_ref()
                .map(GuiControlledRoomCreateSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Controlled Room Create] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.controller_auth_edit_session
                .as_ref()
                .map(GuiControllerAuthEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Controller Auth Edit] editing=(none)".to_owned()]),
        );
        lines.extend(
            self.room_history_edit_session
                .as_ref()
                .map(GuiRoomHistoryEditSessionState::render_lines)
                .unwrap_or_else(|| vec!["[Room History Edit] editing=(none)".to_owned()]),
        );
        lines.push(format!(
            "[Player Setup] status={}",
            self.player_setup_issue
                .as_ref()
                .map(|issue| issue.kind.label())
                .unwrap_or("(none)")
        ));
        if let Some(issue) = self.player_setup_issue.as_ref() {
            lines.push(format!("- detail: {}", issue.message));
        }
        lines.push(format!(
            "[Notifications] count={}",
            self.notifications.len()
        ));
        if self.notifications.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for notification in &self.notifications {
                lines.push(notification.render_line());
            }
        }
        lines.extend(self.validation.render_lines());
        lines.extend(self.configuration.render_lines());
        lines.extend(self.main_window.render_lines());
        lines.extend(self.menus.render_lines());
        lines.extend(self.public_servers.render_lines());
        lines.extend(self.media_search.render_lines());
        lines.push(
            "syncplay-gui now has a unified shell app state and action reducer for future native widget binding."
                .to_owned(),
        );
        lines
    }
}
