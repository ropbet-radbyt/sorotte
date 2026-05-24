use std::path::{Path, PathBuf};

use crate::app::{
    LEGACY_GUI_QSETTINGS_STORE_NAMES, shell_state::GuiConfigStorageChangeTarget,
    ui_state::legacy_gui_qsettings_store_path,
};

use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_complete_public_server_connect_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let selected_server = projected_state
            .selected_public_server_index()
            .and_then(|index| projected_state.public_servers.servers.get(index))
            .map(|row| (row.label.clone(), row.address.clone()));
        let replace_owned_transport = self.session.is_none() || self.session_transport.is_some();
        let replacement_transport_driver = if replace_owned_transport {
            selected_server
                .as_ref()
                .map(|(_label, address)| {
                    GuiTcpSessionTransportDriver::connect_from_host_arg(address)
                        .map(|driver| Box::new(driver) as Box<dyn GuiSessionTransportDriver + Send>)
                })
                .transpose()
        } else {
            Ok(None)
        };
        let replacement_transport_driver = match replacement_transport_driver {
            Ok(driver) => driver,
            Err(error) => {
                self.clear_pending_operation_with_runtime_error(
                    handle,
                    projected_state,
                    format!(
                        "Public server connect through the attached session runtime failed: {error}"
                    ),
                );
                return false;
            }
        };
        if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
            self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                format!(
                    "Public server connect through the attached session runtime failed: {error}"
                ),
            );
            return false;
        }
        let Some(session) = self.session.as_mut() else {
            self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                "Public server connect could not bootstrap a detached client-core session runtime."
                    .to_owned(),
            );
            return false;
        };
        match session.connect_public_server(selected_server) {
            Ok(()) => {
                self.session_projects_to_shell = true;
                self.reset_session_transport_reconnect_state();
                self.pending_room_change_request = None;
                self.clear_session_attached_player_sync_state();
                self.last_published_local_file = None;
                if let Some(driver) = replacement_transport_driver {
                    if let Some(session_transport) = self.session_transport.as_ref() {
                        session_transport.clear_protocol_lines();
                    }
                    self.session_transport_driver = Some(driver);
                }
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteSelectedPublicServerConnect],
                )
            }
            Err(error) => self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                format!(
                    "Public server connect through the attached session runtime failed: {error}"
                ),
            ),
        }
        true
    }

    pub(super) fn handle_complete_public_server_refresh_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        requested_servers: Vec<(String, String)>,
    ) -> bool {
        let current_servers = projected_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect();
        let language = Some(projected_state.runtime_language_tag_legacy_compatible());
        let refresh_result = if let Some(session) = self.session.as_mut() {
            session.refresh_public_servers(current_servers, language)
        } else if !requested_servers.is_empty() {
            Ok(
                GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                    requested_servers,
                ),
            )
        } else {
            Self::refresh_public_servers_without_session(current_servers, language)
        };
        match refresh_result {
            Ok(servers) => Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::CompletePublicServerRefresh(servers)],
            ),
            Err(error) => self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                format!(
                    "Public server refresh through the attached session runtime failed: {error}"
                ),
            ),
        }
        true
    }

    pub(super) fn handle_complete_missing_media_search_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let target_file_name_result = if let Some(session) = self.session.as_ref() {
            session.missing_media_search_target_file_name()
        } else {
            self.detached_missing_media_target_file_name(projected_state)
        };
        let target_file_name = target_file_name_result.as_ref().ok().cloned();
        let search_result = match target_file_name_result {
            Ok(target_file_name) => {
                self.resolve_main_window_user_media_target(projected_state, &target_file_name)
            }
            Err(error) => Err(error),
        };
        match search_result {
            Ok(result) => {
                let found_path = match result {
                    GuiUserMediaTargetResolution::Resolved(path) => normalized_editable_text(&path),
                    GuiUserMediaTargetResolution::Pending => return true,
                    GuiUserMediaTargetResolution::Missing => {
                        target_file_name.as_deref().and_then(|target| {
                            self.media_match_cached_room_candidate_for_target(
                                projected_state,
                                target,
                            )
                        })
                    }
                };
                self.ensure_configured_player_attached();
                match found_path {
                    Some(path) if self.player.is_some() => {
                        self.clear_pending_operation_runtime_state(handle, projected_state);
                        if self.current_player_matches_media_target(&path) {
                            let player_name = self
                                .player
                                .as_ref()
                                .map(|player| player.name())
                                .unwrap_or("player");
                            Self::push_player_success(
                                handle,
                                format!(
                                    "Opened media file through the attached {player_name} player: {path}."
                                ),
                            );
                        } else {
                            self.open_media_files_through_attached_player(handle, vec![path]);
                        }
                    }
                    found_path => Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::CompleteMissingMediaSearch(found_path)],
                    ),
                }
            }
            Err(error) => self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                format!(
                    "Missing-media search through the attached session runtime failed: {error}"
                ),
            ),
        }
        true
    }

    pub(super) fn handle_complete_send_chat_message_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        message: String,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.send_chat_message(message) {
                Ok(()) => Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteLocalChatSend],
                ),
                Err(error) => self.clear_pending_operation_with_runtime_error(
                    handle,
                    projected_state,
                    format!("Chat sending through the attached session runtime failed: {error}"),
                ),
            }
        } else {
            self.clear_pending_operation_with_runtime_error(
                handle,
                projected_state,
                self.send_chat_unavailable_message(),
            );
        }
        true
    }

    pub(super) fn handle_complete_configuration_save_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        settings: sorotte_client_app::app_boundary::state::StoredClientSettingsMvp,
    ) -> bool {
        let Some(path) = self.config_path.as_ref() else {
            self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::CompleteConfigurationSave(settings)],
            );
            return false;
        };
        match upsert_sorotte_ini_stored_client_settings_mvp_at_path(path, &settings) {
            Ok(()) => {
                self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteConfigurationSave(settings)],
                );
            }
            Err(error) => Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CancelConfigurationSave,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Configuration save failed: {error}"),
                    },
                ],
            ),
        }
        true
    }

    pub(super) fn handle_complete_configuration_reset_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        settings: sorotte_client_app::app_boundary::state::StoredClientSettingsMvp,
    ) -> bool {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::CompleteConfigurationReset(settings)],
        );
        true
    }

    pub(super) fn handle_complete_configuration_reload_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        fallback_settings: sorotte_client_app::app_boundary::state::StoredClientSettingsMvp,
    ) -> bool {
        let Some(path) = self.config_path.as_ref() else {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::CompleteConfigurationReload(
                    fallback_settings,
                )],
            );
            return false;
        };
        match load_sorotte_ini_stored_client_settings_mvp_from_path(path) {
            Ok(Some(settings)) => {
                self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteConfigurationReload(settings)],
                );
            }
            Ok(None) => {
                self.sync_player_from_lookup_and_settings(
                    &env_trimmed,
                    Some(&fallback_settings),
                    true,
                );
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteConfigurationReload(
                        fallback_settings,
                    )],
                );
            }
            Err(error) => Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CancelConfigurationReload,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Configuration reload failed: {error}"),
                    },
                ],
            ),
        }
        true
    }

    pub(super) fn handle_complete_clear_gui_data_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        match self.clear_gui_data() {
            Ok(()) => {
                self.sync_player_from_lookup_and_settings(&env_trimmed, None, true);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteClearGuiData],
                )
            }
            Err(error) => Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CancelClearGuiData,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Clear GUI data failed: {error}"),
                    },
                ],
            ),
        }
        true
    }

    pub(super) fn handle_complete_config_storage_root_change_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: GuiConfigStorageChangeTarget,
        settings: sorotte_client_app::app_boundary::state::StoredClientSettingsMvp,
    ) -> bool {
        let old_root = self.legacy_gui_qsettings_root();
        let (paths, install_root) = match self.config_storage_paths_for_change_target(target) {
            Ok(resolved) => resolved,
            Err(error) => {
                return self.cancel_config_storage_change_with_error(
                    handle,
                    projected_state,
                    error,
                );
            }
        };

        if let Err(error) = ensure_sorotte_client_storage_root(&paths.storage_root) {
            return self.cancel_config_storage_change_with_error(
                handle,
                projected_state,
                error.to_string(),
            );
        }

        if let Err(error) =
            upsert_sorotte_ini_stored_client_settings_mvp_at_path(&paths.config_path, &settings)
        {
            return self.cancel_config_storage_change_with_error(
                handle,
                projected_state,
                format!("Configuration save failed: {error}"),
            );
        }

        if let Err(error) =
            persist_sorotte_client_install_locator(&install_root, &paths.storage_root)
        {
            return self.cancel_config_storage_change_with_error(
                handle,
                projected_state,
                error.to_string(),
            );
        }

        let copy_warnings =
            Self::copy_known_storage_entries_best_effort(old_root.as_deref(), &paths.storage_root);
        self.config_path = Some(paths.config_path.clone());
        self.clear_attached_media_search_runtime_cache();
        let stream_helper_snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(None);
        self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
        let snapshot = GuiConfigStorageRuntimeSnapshot::from_storage_paths(&paths);
        let mut actions = vec![
            GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings },
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(stream_helper_snapshot),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: format!("Config location updated: {}.", paths.storage_root.display()),
            },
        ];
        if !copy_warnings.is_empty() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: format!(
                    "Config location updated, but {} existing storage item(s) could not be copied.",
                    copy_warnings.len()
                ),
            });
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        true
    }

    fn config_storage_paths_for_change_target(
        &self,
        target: GuiConfigStorageChangeTarget,
    ) -> Result<(SorotteClientStoragePaths, PathBuf), String> {
        let default_root = default_sorotte_client_config_root().ok_or_else(|| {
            "Cannot resolve the default Sorotte config root on this platform.".to_owned()
        })?;
        let install_root = current_sorotte_client_install_root().ok_or_else(|| {
            "Cannot resolve the Sorotte install folder for sorotte.ini.".to_owned()
        })?;
        let current_dir = std::env::current_dir().ok();
        let root = match target {
            GuiConfigStorageChangeTarget::CustomRoot(root) => {
                normalize_path(PathBuf::from(root), current_dir)
            }
            GuiConfigStorageChangeTarget::DefaultRoot => default_root.clone(),
        };
        Ok((
            SorotteClientStoragePaths::from_root(
                root,
                default_root,
                SorotteClientStorageSource::InstallConfigRoot,
                None,
            ),
            install_root,
        ))
    }

    fn cancel_config_storage_change_with_error(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        message: String,
    ) -> bool {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::CancelConfigStorageRootChange,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: format!("Config location change failed: {message}"),
                },
            ],
        );
        false
    }

    fn copy_known_storage_entries_best_effort(
        old_root: Option<&Path>,
        new_root: &Path,
    ) -> Vec<String> {
        let Some(old_root) = old_root else {
            return Vec::new();
        };
        if old_root == new_root {
            return Vec::new();
        }

        let mut warnings = Vec::new();
        for store_name in LEGACY_GUI_QSETTINGS_STORE_NAMES {
            Self::copy_storage_path_best_effort(
                &legacy_gui_qsettings_store_path(old_root, store_name),
                &legacy_gui_qsettings_store_path(new_root, store_name),
                &mut warnings,
            );
        }
        for entry_name in ["cache", "tools", "updates"] {
            Self::copy_storage_path_best_effort(
                &old_root.join(entry_name),
                &new_root.join(entry_name),
                &mut warnings,
            );
        }
        warnings
    }

    fn copy_storage_path_best_effort(src: &Path, dst: &Path, warnings: &mut Vec<String>) {
        if !src.exists() {
            return;
        }
        if src.is_file() {
            if dst.exists() {
                return;
            }
            if let Some(parent) = dst.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                warnings.push(format!("{}: {error}", dst.display()));
                return;
            }
            if let Err(error) = std::fs::copy(src, dst) {
                warnings.push(format!("{}: {error}", src.display()));
            }
            return;
        }
        if !src.is_dir() {
            return;
        }
        if let Err(error) = std::fs::create_dir_all(dst) {
            warnings.push(format!("{}: {error}", dst.display()));
            return;
        }
        let Ok(entries) = std::fs::read_dir(src) else {
            warnings.push(format!("{}: failed reading directory", src.display()));
            return;
        };
        for entry in entries {
            match entry {
                Ok(entry) => Self::copy_storage_path_best_effort(
                    &entry.path(),
                    &dst.join(entry.file_name()),
                    warnings,
                ),
                Err(error) => warnings.push(format!("{}: {error}", src.display())),
            }
        }
    }
}
