use std::path::Path;

use serde_json::{Map, Value};
use syncplay_client_app::app_boundary::{
    persistence::upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    state::{
        StoredClientSettingsMvp, StoredClientSettingsRuntimeSnapshot,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use syncplay_client_core::PrivacyMode;
use syncplay_player_api::{LocalFileUpdate, PlayerAdapter};

#[cfg(not(test))]
use super::remote_services;
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiQueuedSessionTransportHandle,
    GuiTcpSessionTransportDriver,
};
use super::shell_state::{
    GuiCommandRuntimeSnapshot, GuiSavedConfigurationRuntimeSnapshot, GuiShellAction,
    GuiTransientNotificationLevel, MainWindowRuntimeSnapshot, MainWindowShellState,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, SyncplayGuiShellAppState,
};
use super::startup_support::env_trimmed;
use super::support::system_time_seconds;

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
            let mut session = GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                runtime_settings
                    .settings
                    .username
                    .clone()
                    .unwrap_or_default(),
                runtime_settings.settings.room.clone().unwrap_or_default(),
                runtime_settings.controlled_room_password_override.clone(),
            )?;
            session.apply_runtime_settings_snapshot(&runtime_settings);
            self.session = Some(Box::new(session));
            self.session_projects_to_shell = false;
            self.last_published_local_file = None;
        }
        if self.session_transport.is_none() {
            self.session_transport = Some(GuiQueuedSessionTransportHandle::default());
        }
        self.sync_detached_session_preferences_and_player_state(state)?;
        Ok(())
    }

    fn local_file_payload_legacy_compatible(local_file: Option<&LocalFileUpdate>) -> Value {
        let mut payload = Map::new();
        let Some(local_file) = local_file else {
            return Value::Object(payload);
        };

        payload.insert("name".to_owned(), Value::String(local_file.name.clone()));
        if let Some(duration_seconds) = local_file.duration_seconds {
            payload.insert("duration".to_owned(), Value::from(duration_seconds));
        }
        if let Some(size_bytes) = local_file.size_bytes {
            payload.insert("size".to_owned(), Value::from(size_bytes));
        }
        if let Some(path) = local_file.path.as_ref() {
            payload.insert("path".to_owned(), Value::String(path.clone()));
        }
        Value::Object(payload)
    }

    pub(super) fn sync_detached_session_preferences_and_player_state(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<(), String> {
        let runtime_settings = Self::detached_runtime_settings_for_state(state);
        if !self.session_projects_to_shell {
            self.session_default_room = runtime_settings.settings.room.clone();
        }
        let filename_privacy_mode = runtime_settings
            .settings
            .filename_privacy_mode
            .unwrap_or(PrivacyMode::SendRaw);
        let filesize_privacy_mode = runtime_settings
            .settings
            .filesize_privacy_mode
            .unwrap_or(PrivacyMode::SendRaw);
        let player_local_file = self.player_local_file.clone();
        let file_publish_pending = player_local_file != self.last_published_local_file;
        let playlist_control_available = self
            .session
            .as_ref()
            .is_some_and(|session| session.playlist_control_available());
        let can_auto_advance_to_next_playlist_item = self
            .session
            .as_ref()
            .is_some_and(|session| session.can_auto_advance_to_next_playlist_item());
        let auto_advance_playlist_at_eof = self.take_playlist_auto_advance_eof_trigger_impl(
            state,
            playlist_control_available,
            can_auto_advance_to_next_playlist_item,
        );
        let pending_local_attached_pause_override_update = {
            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            let previous_session_paused = session.local_pause_state();
            let mut pending_local_attached_pause_override_update = None;
            session.sync_runtime_settings(&runtime_settings)?;
            if session.supports_playback_pause_changes()
                && let Some(target_paused) = self.player_paused
                && previous_session_paused != Some(target_paused)
            {
                session.sync_local_playback_telemetry(
                    previous_session_paused,
                    self.player_position_seconds,
                )?;
                let _ = session.set_playback_paused(target_paused)?;
                if let Some(corrected_paused) = session.local_pause_state()
                    && Some(corrected_paused) != self.player_paused
                {
                    if let Some(player) = self.player.as_mut() {
                        player
                            .set_paused(corrected_paused)
                            .map_err(|error| {
                                format!(
                                    "Attached player readiness/pause correction failed while restoring the paused state: {error}"
                                )
                            })?;
                    }
                    self.player_paused = Some(corrected_paused);
                }
                session.sync_local_playback_telemetry(
                    self.player_paused,
                    self.player_position_seconds,
                )?;
                pending_local_attached_pause_override_update = Some(
                    match (
                        session.local_pause_state(),
                        session
                            .current_room_playstate_for_attached_player_sync()
                            .and_then(|playstate| playstate.paused),
                    ) {
                        (Some(session_pause_state), Some(room_pause_state))
                            if room_pause_state != session_pause_state =>
                        {
                            Some(session_pause_state)
                        }
                        _ => None,
                    },
                );
            } else {
                session.sync_local_playback_telemetry(
                    self.player_paused,
                    self.player_position_seconds,
                )?;
            }
            pending_local_attached_pause_override_update
        };
        if auto_advance_playlist_at_eof {
            self.advance_playlist_index_for_attached_player_impl()?;
        }
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session.set_autoplay_enabled(state.main_window.autoplay_active)?;
        session.set_autoplay_threshold(state.main_window.autoplay_threshold)?;
        if file_publish_pending && session.server_handshake_completed() {
            let file_payload =
                Self::local_file_payload_legacy_compatible(player_local_file.as_ref());
            session.publish_local_file_legacy_compatible(
                &file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            )?;
            self.last_published_local_file = player_local_file;
        }
        if let Some(pending_local_attached_pause_override) =
            pending_local_attached_pause_override_update
        {
            self.pending_local_attached_pause_override = pending_local_attached_pause_override;
        }
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

    pub(super) fn session_active(&self) -> bool {
        self.session_projects_to_shell
    }

    pub(super) fn sessionless_main_window_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> MainWindowRuntimeSnapshot {
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        snapshot.room_control_status = MainWindowShellState::room_control_status_without_session();
        let player_runtime_available = self.player_runtime_available_for_actions();
        snapshot.can_toggle_pause = player_runtime_available;
        snapshot.can_seek = player_runtime_available;
        snapshot.can_undo_seek = false;
        snapshot.can_set_offset = true;
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.can_manage_playlist = player_runtime_available && snapshot.shared_playlist_enabled;
        if !snapshot.shared_playlist_enabled {
            snapshot.playlist = self.player_local_file_playlist_entries();
        }
        if self.player.is_some()
            && let Some(paused) = self.player_paused
        {
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

    pub(super) fn save_configuration_for_connect_runtime(
        &mut self,
        projected_state: &SyncplayGuiShellAppState,
    ) -> Result<Option<StoredClientSettingsMvp>, String> {
        if !projected_state.pending_saved_server_connect_saves_configuration {
            return Ok(None);
        }

        let settings = projected_state.configuration.to_stored_settings();
        if let Some(path) = self.config_path.as_ref() {
            upsert_syncplay_ini_stored_client_settings_mvp_at_path(path, &settings)
                .map_err(|error| format!("Configuration save failed: {error}"))?;
        }
        self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), false);
        Ok(Some(settings))
    }

    pub(super) fn complete_saved_server_connect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        clear_pending: bool,
    ) {
        match self.save_configuration_for_connect_runtime(projected_state) {
            Ok(Some(settings)) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                        GuiSavedConfigurationRuntimeSnapshot { settings },
                    )],
                );
            }
            Ok(None) => {}
            Err(message) => {
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
        }

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
        let runtime_settings = Self::detached_runtime_settings_for_state(projected_state);
        let mut session = match GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
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
        session.apply_runtime_settings_snapshot(&runtime_settings);

        self.session = Some(Box::new(session));
        self.session_projects_to_shell = true;
        self.reset_session_transport_reconnect_state();
        self.session_default_room = Some(default_room);
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.pending_attached_media_resolution = None;
        self.unresolved_attached_media_target = None;
        self.clear_session_attached_player_sync_state();
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
        let disconnect_error = if let Some(session) = self.session.as_mut() {
            let sync_result = session
                .sync_local_playback_telemetry(self.player_paused, self.player_position_seconds);
            sync_result
                .and_then(|()| session.disconnect_session(system_time_seconds()))
                .err()
        } else {
            None
        };
        self.apply_session_transport_disconnect_pause(handle, projected_state);
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.reset_session_transport_reconnect_state();
        self.session_default_room = None;
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.pending_attached_media_resolution = None;
        self.unresolved_attached_media_target = None;
        self.clear_session_attached_player_sync_state();

        let mut actions = self.sessionless_projection_actions(projected_state);
        if let Some(error) = disconnect_error {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error,
            });
        }
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
