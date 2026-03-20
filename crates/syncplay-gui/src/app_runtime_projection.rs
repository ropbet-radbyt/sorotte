use std::path::Path;

use syncplay_player_api::LocalFileUpdate;

use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::shell_state::{
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiShellAction,
    MainWindowRuntimeSnapshot, MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot,
    SyncplayGuiShellAppState,
};
use super::GuiPersistedConfigRuntimeOwner;

impl GuiPersistedConfigRuntimeOwner {
    fn format_local_file_playlist_entry(local_file: &LocalFileUpdate) -> String {
        let mut details = Vec::new();
        if let Some(duration_seconds) = local_file.duration_seconds {
            details.push(format!("{duration_seconds:.3}s"));
        }
        if let Some(size_bytes) = local_file.size_bytes {
            details.push(format!("{size_bytes} bytes"));
        }
        if details.is_empty() {
            local_file.name.clone()
        } else {
            format!("{} [{}]", local_file.name, details.join(", "))
        }
    }

    pub(super) fn player_local_file_playlist_entries_impl(&self) -> Vec<String> {
        self.player_local_file
            .as_ref()
            .map(Self::format_local_file_playlist_entry)
            .into_iter()
            .collect()
    }

    pub(super) fn command_availability_for_runtime_state_impl(
        &self,
        state: &SyncplayGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        let player_runtime_available =
            player_attached || self.player_runtime_available_for_actions();
        let settings = state.configuration.to_stored_settings();
        let busy = state.pending_operation.is_some();
        let command_availability = GuiCommandAvailabilityState {
            can_save_configuration: !busy && state.validation.issues.is_empty(),
            can_reset_configuration: !busy && state.has_unsaved_configuration_changes(),
            can_reload_configuration: !busy,
            can_connect_saved_server: !busy && state.saved_session_connect_target().is_some(),
            can_disconnect_session: !busy && self.session_active(),
            can_connect_public_server: !busy && state.public_servers.can_connect,
            can_refresh_public_servers: !busy && state.public_servers.can_refresh,
            can_search_missing_media: !busy && state.media_search.can_search_missing_media,
            can_toggle_pause: !busy && player_runtime_available,
            can_send_chat_message: !busy && settings.chat_input_enabled.unwrap_or(false),
        };
        if let Some(session) = self.session.as_ref() {
            session.adjust_command_availability(state, command_availability)
        } else {
            command_availability
        }
    }

    pub(super) fn placeholder_local_file_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        LocalFileUpdate::new(name).with_path(path.to_owned())
    }

    pub(super) fn sync_player_runtime_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
    ) {
        let _ = self.poll_attached_media_search_index_build(
            Self::automatic_media_search_retry_interval(state),
        );
        let player_attached = self.player.is_some();
        let player_runtime_available = self.player_runtime_available_for_actions();

        let mut desired_main_window =
            MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        let mut main_window_changed = false;

        if desired_main_window.can_toggle_pause != player_runtime_available {
            desired_main_window.can_toggle_pause = player_runtime_available;
            main_window_changed = true;
        }
        if desired_main_window.can_seek != player_runtime_available {
            desired_main_window.can_seek = player_runtime_available;
            main_window_changed = true;
        }
        if desired_main_window.can_set_offset != player_runtime_available {
            desired_main_window.can_set_offset = player_runtime_available;
            main_window_changed = true;
        }
        let can_manage_playlist = self
            .session
            .as_ref()
            .map(|session| {
                desired_main_window.shared_playlist_enabled && session.playlist_control_available()
            })
            .unwrap_or(player_runtime_available && desired_main_window.shared_playlist_enabled);
        if desired_main_window.can_manage_playlist != can_manage_playlist {
            desired_main_window.can_manage_playlist = can_manage_playlist;
            main_window_changed = true;
        }
        if !desired_main_window.shared_playlist_enabled {
            let desired_playlist = self.player_local_file_playlist_entries_impl();
            if desired_main_window.playlist != desired_playlist {
                desired_main_window.playlist = desired_playlist;
                main_window_changed = true;
            }
        }
        if player_attached
            && let Some(paused) = self.player_paused
            && desired_main_window.playback_paused != paused
        {
            desired_main_window.playback_paused = paused;
            main_window_changed = true;
        }
        if (desired_main_window.user_offset_seconds - self.user_offset_seconds).abs() > f64::EPSILON
        {
            desired_main_window.user_offset_seconds = self.user_offset_seconds;
            main_window_changed = true;
        }
        if main_window_changed {
            handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                desired_main_window,
            ));
        }

        let desired_media_index_status = self.media_index_runtime_snapshot_impl();
        if state.media_index_status.active != desired_media_index_status.active
            || state.media_index_status.message != desired_media_index_status.message
        {
            handle.push_action(GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
                desired_media_index_status,
            ));
        }

        let mut action_overrides = Vec::new();
        for (action_label, enabled) in [
            ("Play", player_attached),
            ("Pause", player_attached),
            ("Toggle Pause", player_attached),
            ("Seek", player_attached),
            (
                "Undo Seek",
                state.pending_operation.is_none() && state.main_window.playback.can_undo_seek,
            ),
            (
                "Playlist Actions",
                self.session
                    .as_ref()
                    .map(|session| {
                        state.main_window.shared_playlist_enabled
                            && session.playlist_control_available()
                    })
                    .unwrap_or(player_attached && state.main_window.shared_playlist_enabled),
            ),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == "Playback")
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label,
                    enabled,
                });
            }
        }
        let current_offset_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Advanced")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Set Offset")
            })
            .map(|action| action.enabled);
        let desired_offset_enabled = state.pending_operation.is_none() && player_attached;
        if current_offset_enabled
            .is_some_and(|current_enabled| current_enabled != desired_offset_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Advanced",
                action_label: "Set Offset",
                enabled: desired_offset_enabled,
            });
        }
        if !action_overrides.is_empty() {
            handle.push_action(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
                MenuDialogRuntimeSnapshot {
                    action_overrides,
                    tls_prompt_expected: state.menus.tls_prompt_expected,
                    update_notice_expected: state.menus.update_notice_expected,
                    about_dialog_available: state.menus.about_dialog_available,
                },
            ));
        }

        let desired_command_availability =
            self.command_availability_for_runtime_state_impl(state, player_attached);
        if state.commands != desired_command_availability {
            handle.push_action(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
                GuiCommandRuntimeSnapshot {
                    command_availability: desired_command_availability,
                    pending_operation: state.pending_operation.as_ref().map(|pending| pending.kind),
                },
            ));
        }
    }
}
