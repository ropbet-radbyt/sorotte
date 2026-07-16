use std::path::Path;

use serde_json::{Map, Value};
use sorotte_client_app::app_boundary::{
    persistence::upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    state::{
        StoredClientSettingsMvp, StoredClientSettingsRuntimeSnapshot,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use sorotte_client_core::PlayerCommandCause;
use sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY;
use sorotte_player_api::{LocalFileUpdate, PlayerAdapter};

use super::media_match_support::media_match_wire_value_for_path;
#[cfg(not(test))]
use super::remote_services;
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiLocalPlayerUnpauseDecision,
    GuiQueuedSessionTransportHandle, GuiThreadedTcpSessionTransportDriver,
};
use super::shell_state::{
    GuiSavedConfigurationRuntimeSnapshot, GuiSavedServerConnectIntent,
    GuiSavedSessionConnectTarget, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MainWindowShellState, MenuActionId, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SorotteGuiShellAppState,
};
use super::support::{autoplay_threshold_from_settings, system_time_seconds};

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn detached_runtime_settings_for_state(
        state: &SorotteGuiShellAppState,
    ) -> StoredClientSettingsRuntimeSnapshot {
        stored_client_settings_runtime_snapshot_legacy_compatible(
            &state.configuration.to_stored_settings(),
        )
    }

    fn session_runtime_settings_for_state(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> StoredClientSettingsRuntimeSnapshot {
        if !self.session_projects_to_shell || self.session.is_none() {
            return Self::detached_runtime_settings_for_state(state);
        }
        if let Some(runtime_settings) = self.active_session_settings.as_ref() {
            return runtime_settings.clone();
        }

        // Test and embedding adapters can be installed without going through a
        // network connect. Pin their first effective settings just like a real
        // active session instead of following later edits to the settings form.
        let runtime_settings = Self::detached_runtime_settings_for_state(state);
        self.active_session_configured_settings = Some(runtime_settings.clone());
        self.active_session_settings = Some(runtime_settings.clone());
        runtime_settings
    }

    pub(super) fn saved_server_connect_target_for_runtime_settings(
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Option<GuiSavedSessionConnectTarget> {
        let connection = &runtime_settings.config.connection;
        let host = connection.host.as_deref()?.trim();
        if host.is_empty() {
            return None;
        }
        let port = connection.port.get();
        Some(GuiSavedSessionConnectTarget {
            address: format!("{host}:{port}"),
            username: connection
                .username
                .as_ref()
                .map(|username| username.as_str().to_owned())
                .unwrap_or_default(),
            room: connection
                .room
                .as_ref()
                .map(|room| room.as_str().to_owned())
                .unwrap_or_default(),
            controlled_room_password_override: runtime_settings
                .controlled_room_password_override
                .clone()
                .or_else(|| connection.controlled_room_password.clone()),
        })
    }

    pub(super) fn ensure_detached_client_core_chat_session(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Result<(), String> {
        if self.session.is_none() {
            let runtime_settings = Self::detached_runtime_settings_for_state(state);
            self.session_default_room = runtime_settings
                .config
                .connection
                .room
                .as_ref()
                .map(|room| room.as_str().to_owned());
            let mut session = GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                runtime_settings
                    .config
                    .connection
                    .username
                    .as_ref()
                    .map(|username| username.as_str().to_owned())
                    .unwrap_or_default(),
                runtime_settings
                    .config
                    .connection
                    .room
                    .as_ref()
                    .map(|room| room.as_str().to_owned())
                    .unwrap_or_default(),
                runtime_settings
                    .config
                    .connection
                    .controlled_room_password
                    .clone(),
            )?;
            session.apply_runtime_settings_snapshot(&runtime_settings)?;
            self.install_session_runtime(Box::new(session));
            self.session_projects_to_shell = false;
            self.last_published_local_file = None;
            self.last_published_media_match_signature = None;
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

    fn media_match_wire_signature_for_local_file(
        &self,
        state: &SorotteGuiShellAppState,
        local_file: Option<&LocalFileUpdate>,
    ) -> Option<Value> {
        if !state.media_match.settings.fingerprinting_enabled
            || !state.media_match.settings.wire_sharing_enabled
        {
            return None;
        }
        if !self.session.as_ref().is_some_and(|session| {
            session.server_handshake_completed() && session.server_media_match_supported()
        }) {
            return None;
        }
        if !self.media_match_wire_signature_allowed_for_local_file(state, local_file) {
            return None;
        }
        let root = self.legacy_gui_qsettings_root();
        let root = root.as_deref()?;
        let path = local_file.and_then(|local_file| local_file.path.as_deref())?;
        media_match_wire_value_for_path(root, path)
    }

    fn attach_media_match_wire_signature_to_file_payload(
        payload: &mut Value,
        signature: Option<&Value>,
    ) {
        let Some(signature) = signature else {
            return;
        };
        if let Value::Object(entries) = payload {
            entries.insert(MEDIA_MATCH_FILE_PAYLOAD_KEY.to_owned(), signature.clone());
        }
    }

    fn should_defer_attached_player_pause_sync(
        &mut self,
        previous_session_paused: Option<bool>,
        target_paused: bool,
    ) -> bool {
        // mpv can briefly report pause=true while loading or starting media. Confirm
        // attached-player pauses on the following GUI pump before changing readiness.
        if !target_paused
            || previous_session_paused == Some(true)
            || self.runtime_pump_generation == 0
        {
            self.pending_attached_player_pause_confirmation_pump = None;
            return false;
        }

        match self.pending_attached_player_pause_confirmation_pump {
            Some(pump_generation) if pump_generation != self.runtime_pump_generation => {
                self.pending_attached_player_pause_confirmation_pump = None;
                false
            }
            Some(_) => true,
            None => {
                self.pending_attached_player_pause_confirmation_pump =
                    Some(self.runtime_pump_generation);
                true
            }
        }
    }

    pub(super) fn sync_detached_session_preferences_and_player_state(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Result<(), String> {
        let runtime_settings = self.session_runtime_settings_for_state(state);
        if !self.session_projects_to_shell {
            self.session_default_room = runtime_settings
                .config
                .connection
                .room
                .as_ref()
                .map(|room| room.as_str().to_owned());
        }
        let filename_privacy_mode = runtime_settings.config.playback.filename_privacy_mode;
        let filesize_privacy_mode = runtime_settings.config.playback.filesize_privacy_mode;
        let player_local_file = self.player_local_file.clone();
        let media_match_signature =
            self.media_match_wire_signature_for_local_file(state, player_local_file.as_ref());
        let file_publish_pending = !self.player_local_file_placeholder
            && (player_local_file != self.last_published_local_file
                || (player_local_file.is_some()
                    && media_match_signature != self.last_published_media_match_signature));
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
        let player_observation_is_end_of_file = self.attached_player_observation_is_end_of_file();
        let (previous_session_paused, supports_playback_pause_changes) = {
            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            session.sync_runtime_settings(&runtime_settings)?;
            session.sync_local_playback_cache_state(
                self.player_paused_for_cache,
                self.player_cache_buffering_percent,
            )?;
            (
                session.local_pause_state(),
                session.supports_playback_pause_changes(),
            )
        };
        let player_paused = self.player_paused;
        let player_paused_for_cache = self.player_paused_for_cache == Some(true);
        let pending_local_attached_pause_override_update = {
            let mut pending_local_attached_pause_override_update = None;

            if supports_playback_pause_changes
                && !player_paused_for_cache
                && !player_observation_is_end_of_file
                && let Some(target_paused) = player_paused
                && previous_session_paused != Some(target_paused)
            {
                if self
                    .should_defer_attached_player_pause_sync(previous_session_paused, target_paused)
                {
                    let Some(session) = self.session.as_mut() else {
                        return Ok(());
                    };
                    session.sync_local_playback_telemetry(
                        previous_session_paused,
                        self.player_position_seconds,
                    )?;
                    pending_local_attached_pause_override_update = Some(Some(target_paused));
                } else {
                    let Some(session) = self.session.as_mut() else {
                        return Ok(());
                    };
                    session.sync_local_playback_telemetry(
                        previous_session_paused,
                        self.player_position_seconds,
                    )?;
                    let direct_unpause_decision = if !target_paused {
                        session.handle_local_player_unpause_attempt()?
                    } else {
                        GuiLocalPlayerUnpauseDecision::NotApplicable
                    };
                    if direct_unpause_decision == GuiLocalPlayerUnpauseDecision::Block {
                        session.sync_local_playback_telemetry(
                            Some(true),
                            self.player_position_seconds,
                        )?;
                        self.rollback_attached_player_pause_intent(target_paused);
                        if self.player.is_some() {
                            let command_id = self.begin_attached_player_pause_command(
                                true,
                                PlayerCommandCause::ReadinessGateHold,
                            )?;
                            let result = self
                                .player
                                .as_mut()
                                .expect("readiness-gate correction player was checked")
                                .set_paused(true);
                            let command_succeeded = result.is_ok();
                            let command_result_error = self
                                .finish_attached_player_pause_command(command_id, command_succeeded)
                                .err();
                            if let Err(error) = result {
                                if let Some(command_result_error) = command_result_error {
                                    eprintln!(
                                        "warning: failed to register readiness-gate correction failure: {command_result_error}"
                                    );
                                }
                                return Err(format!(
                                    "Attached player readiness/pause correction failed while restoring the paused state: {error}"
                                ));
                            }
                            if let Some(error) = command_result_error {
                                eprintln!(
                                    "warning: failed to register readiness-gate correction: {error}"
                                );
                            }
                            self.note_local_attached_player_pause_command(true);
                        }
                        self.player_paused = Some(true);
                    } else {
                        let Some(session) = self.session.as_mut() else {
                            return Ok(());
                        };
                        let _ = session.set_playback_paused(target_paused)?;
                    }
                    let Some(session) = self.session.as_mut() else {
                        return Ok(());
                    };
                    let corrected_paused = session
                        .local_pause_state()
                        .filter(|corrected_paused| Some(*corrected_paused) != self.player_paused);
                    if let Some(corrected_paused) = corrected_paused {
                        self.rollback_attached_player_pause_intent(target_paused);
                        if self.player.is_some() {
                            let command_id = self.begin_attached_player_pause_command(
                                corrected_paused,
                                PlayerCommandCause::RemoteRoomSynchronization,
                            )?;
                            let result = self
                                .player
                                .as_mut()
                                .expect("room-state correction player was checked")
                                .set_paused(corrected_paused);
                            let command_succeeded = result.is_ok();
                            let command_result_error = self
                                .finish_attached_player_pause_command(command_id, command_succeeded)
                                .err();
                            if let Err(error) = result {
                                if let Some(command_result_error) = command_result_error {
                                    eprintln!(
                                        "warning: failed to register room-state correction failure: {command_result_error}"
                                    );
                                }
                                return Err(format!(
                                    "Attached player readiness/pause correction failed while restoring the paused state: {error}"
                                ));
                            }
                            if let Some(error) = command_result_error {
                                eprintln!(
                                    "warning: failed to register room-state correction: {error}"
                                );
                            }
                            self.note_local_attached_player_pause_command(corrected_paused);
                        }
                        self.player_paused = Some(corrected_paused);
                    }
                    let Some(session) = self.session.as_mut() else {
                        return Ok(());
                    };
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
                }
            } else {
                let pause_confirmation_was_pending = self
                    .pending_attached_player_pause_confirmation_pump
                    .is_some();
                if player_paused != Some(true) || previous_session_paused == Some(true) {
                    self.pending_attached_player_pause_confirmation_pump = None;
                    if pause_confirmation_was_pending && player_paused != Some(true) {
                        pending_local_attached_pause_override_update = Some(None);
                    }
                }
                let Some(session) = self.session.as_mut() else {
                    return Ok(());
                };
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
        let (autoplay_enabled, autoplay_threshold) = if self.session_projects_to_shell {
            (
                runtime_settings.config.readiness.autoplay_initial_state,
                autoplay_threshold_from_settings(&runtime_settings.settings),
            )
        } else {
            (
                state.main_window.autoplay_active,
                state.main_window.autoplay_threshold,
            )
        };
        {
            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            session.set_autoplay_enabled(autoplay_enabled)?;
            session.set_autoplay_threshold(autoplay_threshold)?;
        }
        let publish_file = file_publish_pending
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.server_handshake_completed());
        if publish_file {
            let mut file_payload =
                Self::local_file_payload_legacy_compatible(player_local_file.as_ref());
            Self::attach_media_match_wire_signature_to_file_payload(
                &mut file_payload,
                media_match_signature.as_ref(),
            );
            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            session.publish_local_file_legacy_compatible(
                &file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            )?;
            let published_file = player_local_file.clone();
            let published_media_match_signature = media_match_signature.clone();
            self.last_published_local_file = player_local_file;
            self.last_published_media_match_signature = media_match_signature;
            if published_media_match_signature.is_some() {
                self.clear_local_shared_playlist_media_match_signature_path_if_current(
                    published_file.as_ref(),
                );
            }
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
        state: &SorotteGuiShellAppState,
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
        state: &SorotteGuiShellAppState,
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
        state: &SorotteGuiShellAppState,
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
            let player_playlist = self.player_local_file_playlist_entries();
            if snapshot.playlist != player_playlist {
                snapshot.playlist = player_playlist;
                snapshot.playlist_entry_ids.clear();
                snapshot.playlist_source_states.clear();
                snapshot.active_playlist_index = None;
            }
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
        state: &SorotteGuiShellAppState,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let mut action_overrides = Vec::new();
        for (id, enabled) in [
            (MenuActionId::CreateControlledRoom, false),
            (MenuActionId::IdentifyAsController, false),
        ] {
            let current_enabled = state.menus.action(id).map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride { id, enabled });
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
        state: &SorotteGuiShellAppState,
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
        projected_state: &mut SorotteGuiShellAppState,
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
        projected_state: &SorotteGuiShellAppState,
        intent: GuiSavedServerConnectIntent,
        submitted_settings: &StoredClientSettingsMvp,
    ) -> Result<(), String> {
        if intent != GuiSavedServerConnectIntent::SaveAndConnect {
            return Ok(());
        }

        let path = self
            .persisted_settings_config_path_for_request(projected_state)
            .ok_or_else(|| {
                "Configuration save failed: no writable GUI config path is available".to_owned()
            })?;
        upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, submitted_settings)
            .map_err(|error| format!("Configuration save failed: {error}"))?;
        self.config_path = Some(path);
        Ok(())
    }

    pub(super) fn complete_saved_server_connect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        clear_pending: bool,
    ) {
        let submitted = projected_state
            .pending_saved_server_connect_intent
            .map(|intent| {
                (
                    intent,
                    projected_state.submitted_saved_server_connect_settings(intent),
                )
            });
        self.complete_saved_server_connect_runtime_with_submission(
            handle,
            projected_state,
            clear_pending,
            submitted,
        );
    }

    pub(super) fn complete_submitted_saved_server_connect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        clear_pending: bool,
        intent: GuiSavedServerConnectIntent,
        submitted_settings: StoredClientSettingsMvp,
    ) {
        self.complete_saved_server_connect_runtime_with_submission(
            handle,
            projected_state,
            clear_pending,
            Some((intent, submitted_settings)),
        );
    }

    fn complete_saved_server_connect_runtime_with_submission(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        clear_pending: bool,
        submitted: Option<(GuiSavedServerConnectIntent, StoredClientSettingsMvp)>,
    ) {
        let (connect_intent, active_settings) = submitted.unwrap_or_else(|| {
            (
                GuiSavedServerConnectIntent::ConnectOnce,
                projected_state.saved_configuration.clone(),
            )
        });
        match self.save_configuration_for_connect_runtime(
            projected_state,
            connect_intent,
            &active_settings,
        ) {
            Ok(()) if connect_intent == GuiSavedServerConnectIntent::SaveAndConnect => {
                self.promote_on_save_runtime_fields(&active_settings);
                self.adopt_saved_player_launch_state_when_inactive(&active_settings);
                let pending_requirements =
                    self.pending_apply_requirements_action(projected_state, &active_settings);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                            GuiSavedConfigurationRuntimeSnapshot {
                                settings: active_settings.clone(),
                            },
                        ),
                        pending_requirements,
                    ],
                );
            }
            Ok(()) => {}
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
        let runtime_settings =
            stored_client_settings_runtime_snapshot_legacy_compatible(&active_settings);
        let Some(target) =
            Self::saved_server_connect_target_for_runtime_settings(&runtime_settings)
        else {
            let message =
                "Configured server connect requires a saved host and a valid port.".to_owned();
            if clear_pending {
                self.clear_pending_operation_with_runtime_error(handle, projected_state, message);
            } else {
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return;
        };
        let transport_driver = match GuiThreadedTcpSessionTransportDriver::connect_from_host_arg(
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
        if let Err(error) = session.apply_runtime_settings_snapshot(&runtime_settings) {
            let message = format!(
                "Configured server connect through the detached session runtime failed: {error}"
            );
            if clear_pending {
                self.clear_pending_operation_with_runtime_error(handle, projected_state, message);
            } else {
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return;
        }

        self.install_active_session_runtime(Box::new(session), runtime_settings);
        self.reset_session_transport_reconnect_state();
        self.session_default_room = Some(default_room);
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
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
        actions.push(self.pending_apply_requirements_action(
            projected_state,
            &projected_state.saved_configuration,
        ));
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn complete_session_disconnect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let _ = self.interrupt_attached_playback_recovery_impl("session disconnect");
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
        self.remove_session_runtime();
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.reset_session_transport_reconnect_state();
        self.session_default_room = None;
        self.pending_room_change_request = None;
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
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
        actions.push(self.pending_apply_requirements_action(
            projected_state,
            &projected_state.saved_configuration,
        ));
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn push_actions_and_project(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
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
        projected_state: &mut SorotteGuiShellAppState,
        message: String,
    ) {
        let actions = vec![
            GuiShellAction::CompletePendingOperation,
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
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::CompletePendingOperation],
        );
    }
}
