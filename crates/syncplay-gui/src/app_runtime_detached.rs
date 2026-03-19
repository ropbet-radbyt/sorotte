use std::path::Path;

use syncplay_client_app::app_boundary::state::{
    StoredClientSettingsRuntimeSnapshot, stored_client_settings_runtime_snapshot_legacy_compatible,
};

#[cfg(not(test))]
use super::remote_services;
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiQueuedSessionTransportHandle,
    GuiTcpSessionTransportDriver,
};
use super::shell_state::{
    GuiCommandRuntimeSnapshot, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot,
    SyncplayGuiShellAppState,
};

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn detached_runtime_settings_for_state(
        state: &SyncplayGuiShellAppState,
    ) -> StoredClientSettingsRuntimeSnapshot {
        stored_client_settings_runtime_snapshot_legacy_compatible(
            &state.configuration.to_stored_settings(),
        )
    }

    pub(super) fn ensure_detached_client_core_chat_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<(), String> {
        if self.session.is_none() {
            let runtime_settings = Self::detached_runtime_settings_for_state(state);
            self.session_default_room = runtime_settings.settings.room.clone();
            self.session = Some(Box::new(
                GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                    runtime_settings.settings.username.unwrap_or_default(),
                    runtime_settings.settings.room.unwrap_or_default(),
                    runtime_settings.controlled_room_password_override,
                )?,
            ));
            self.session_projects_to_shell = false;
        }
        if self.session_transport.is_none() {
            self.session_transport = Some(GuiQueuedSessionTransportHandle::default());
        }
        self.sync_detached_session_preferences_and_player_state(state)?;
        Ok(())
    }

    pub(super) fn sync_detached_session_preferences_and_player_state(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<(), String> {
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        session.set_autoplay_enabled(state.main_window.autoplay_active)?;
        session.set_autoplay_threshold(state.main_window.autoplay_threshold)?;
        Ok(())
    }

    pub(super) fn refresh_public_servers_without_session(
        _current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(refreshed_servers) =
            GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_env()?
        {
            return Ok(refreshed_servers);
        }
        #[cfg(test)]
        {
            Ok(
                GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                    _current_servers,
                ),
            )
        }
        #[cfg(not(test))]
        {
            let refreshed_servers = remote_services::fetch_public_servers(_language)?;
            Ok(
                GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                    refreshed_servers,
                ),
            )
        }
    }

    pub(super) fn detached_missing_media_target(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<String> {
        if let Some(local_file) = self.player_local_file.as_ref() {
            if let Some(path) = local_file
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                return Some(path.to_owned());
            }
            let name = local_file.name.trim();
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }

        if let Some(index) = state.selection.selected_main_window_playlist
            && let Some(row) = state.main_window.playlist.get(index)
        {
            let label = row.label.trim();
            if !label.is_empty() {
                return Some(label.to_owned());
            }
        }

        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.trim())
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
    }

    pub(super) fn detached_missing_media_target_file_name(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<String, String> {
        let Some(target) = self.detached_missing_media_target(state) else {
            return Err(
                "Detached GUI missing-media search could not determine a target file from the current player or playlist state."
                    .to_owned(),
            );
        };
        if target.contains("://") {
            return Err(
                "Detached GUI missing-media search does not support URL-based media targets."
                    .to_owned(),
            );
        }
        let Some(file_name) = Path::new(&target)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return Err(
                "Detached GUI missing-media search could not derive a file name from the current player or playlist state."
                    .to_owned(),
            );
        };
        let file_name = file_name.trim();
        if file_name.is_empty() {
            return Err(
                "Detached GUI missing-media search could not derive a non-empty file name from the current player or playlist state."
                    .to_owned(),
            );
        }
        Ok(file_name.to_owned())
    }

    pub(super) fn search_missing_media_without_session(
        &self,
        state: &SyncplayGuiShellAppState,
        directories: Vec<String>,
    ) -> Result<Option<String>, String> {
        let target_file_name = self.detached_missing_media_target_file_name(state)?;
        for directory in directories {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                    &target_file_name,
                    Path::new(trimmed),
                )?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    pub(super) fn session_active(&self) -> bool {
        self.session_projects_to_shell
    }

    pub(super) fn sessionless_main_window_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> MainWindowRuntimeSnapshot {
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        let player_attached = self.player.is_some();
        snapshot.can_toggle_pause = player_attached;
        snapshot.can_seek = player_attached;
        snapshot.can_undo_seek = false;
        snapshot.can_set_offset = player_attached;
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.can_manage_playlist = player_attached && snapshot.shared_playlist_enabled;
        if !snapshot.shared_playlist_enabled {
            snapshot.playlist = self.player_local_file_playlist_entries();
        }
        if player_attached && let Some(paused) = self.player_paused {
            snapshot.playback_paused = paused;
        }
        snapshot.autoplay_countdown_seconds = None;
        snapshot.user_offset_seconds = self.user_offset_seconds;
        snapshot
    }

    pub(super) fn sessionless_menu_dialog_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let settings = state.configuration.to_stored_settings();
        let desired_show_chat_enabled = settings.chat_input_enabled.unwrap_or(false)
            || settings.chat_output_enabled.unwrap_or(false);
        let desired_show_playlist_enabled = settings.shared_playlist_enabled.unwrap_or(false);
        let mut action_overrides = Vec::new();
        for (section_title, action_label, enabled) in [
            ("Window", "Show Chat", desired_show_chat_enabled),
            ("Window", "Show Playlist", desired_show_playlist_enabled),
            ("Advanced", "Create Controlled Room", false),
            ("Advanced", "Identify As Controller", false),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == section_title)
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title,
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

    pub(super) fn sessionless_projection_actions(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        let main_window_snapshot = self.sessionless_main_window_snapshot(state);
        if main_window_snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window) {
            actions.push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                main_window_snapshot,
            ));
        }
        if let Some(menu_snapshot) = self.sessionless_menu_dialog_runtime_snapshot(state) {
            actions.push(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
                menu_snapshot,
            ));
        }
        actions
    }

    pub(super) fn push_runtime_error_notification(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        message: String,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message,
            }],
        );
    }

    pub(super) fn complete_saved_server_connect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        clear_pending: bool,
    ) {
        let Some(target) = projected_state.saved_session_connect_target() else {
            let message =
                "Configured server connect requires a saved host and a valid port.".to_owned();
            if clear_pending {
                self.clear_pending_operation_with_runtime_error(handle, projected_state, message);
            } else {
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return;
        };
        let transport_driver = match GuiTcpSessionTransportDriver::connect_from_host_arg(
            &target.address,
        ) {
            Ok(driver) => driver,
            Err(error) => {
                let message = format!(
                    "Configured server connect through the detached session runtime failed: {error}"
                );
                if clear_pending {
                    self.clear_pending_operation_with_runtime_error(
                        handle,
                        projected_state,
                        message,
                    );
                } else {
                    Self::push_runtime_error_notification(handle, projected_state, message);
                }
                return;
            }
        };
        let default_room = target.room.clone();
        let session = match GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
            target.username,
            target.room,
            target.controlled_room_password_override,
        ) {
            Ok(session) => session,
            Err(error) => {
                let message = format!(
                    "Configured server connect through the detached session runtime failed: {error}"
                );
                if clear_pending {
                    self.clear_pending_operation_with_runtime_error(
                        handle,
                        projected_state,
                        message,
                    );
                } else {
                    Self::push_runtime_error_notification(handle, projected_state, message);
                }
                return;
            }
        };

        self.session = Some(Box::new(session));
        self.session_projects_to_shell = true;
        self.session_default_room = Some(default_room);
        self.pending_room_change_request = None;
        if self.session_transport.is_none() {
            self.session_transport = Some(GuiQueuedSessionTransportHandle::default());
        }
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session_transport_driver = Some(Box::new(transport_driver));

        let mut actions = self.sessionless_projection_actions(projected_state);
        if clear_pending {
            actions.push(GuiShellAction::CompleteSavedServerConnect);
        }
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn complete_session_disconnect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.session_default_room = None;
        self.pending_room_change_request = None;

        let mut actions = self.sessionless_projection_actions(projected_state);
        actions.push(GuiShellAction::CompleteSessionDisconnect);
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn push_actions_and_project(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) {
        if actions.is_empty() {
            return;
        }
        handle.push_actions(actions.clone());
        for action in actions {
            let _ = projected_state.apply(action);
        }
    }

    pub(super) fn clear_pending_operation_with_runtime_error(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        message: String,
    ) {
        let mut cleared_state = projected_state.clone();
        cleared_state.pending_operation = None;
        let actions = vec![
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: self
                    .command_availability_for_runtime_state(&cleared_state, self.player.is_some()),
                pending_operation: None,
            }),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message,
            },
        ];
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn clear_pending_operation_runtime_state(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let mut cleared_state = projected_state.clone();
        cleared_state.pending_operation = None;
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
                GuiCommandRuntimeSnapshot {
                    command_availability: self.command_availability_for_runtime_state(
                        &cleared_state,
                        self.player.is_some(),
                    ),
                    pending_operation: None,
                },
            )],
        );
    }
}
