use std::path::{Path, PathBuf};

use crate::app::runtime_owner::GuiUserMediaTargetResolutionSource;
use crate::app::{
    LEGACY_GUI_QSETTINGS_STORE_NAMES, shell_state::GuiConfigStorageChangeTarget,
    ui_state::legacy_gui_qsettings_store_path,
};

use super::*;

fn config_storage_metadata_is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_complete_public_server_connect_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        selected_server: (String, String),
        active_settings: sorotte_client_app::app_boundary::state::StoredClientSettingsMvp,
    ) -> bool {
        let replace_owned_transport = self.session.is_none() || self.session_transport.is_some();
        let runtime_settings =
            stored_client_settings_runtime_snapshot_legacy_compatible(&active_settings);
        let replacement_transport_driver = if replace_owned_transport {
            GuiThreadedTcpSessionTransportDriver::connect_from_host_arg_with_tls_policy(
                &selected_server.1,
                runtime_settings.config.connection.tls_policy,
            )
            .map(|driver| Some(Box::new(driver) as Box<dyn GuiSessionTransportDriver + Send>))
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
        let default_room = runtime_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str().to_owned());
        let connect_result = if replace_owned_transport {
            let connection = &runtime_settings.config.connection;
            let mut session =
                match GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                    connection
                        .username
                        .as_ref()
                        .map(|username| username.as_str().to_owned())
                        .unwrap_or_default(),
                    connection
                        .room
                        .as_ref()
                        .map(|room| room.as_str().to_owned())
                        .unwrap_or_default(),
                    connection.controlled_room_password.clone(),
                ) {
                    Ok(session) => session,
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
            session
                .apply_runtime_settings_snapshot(&runtime_settings)
                .and_then(|()| session.connect_public_server(Some(selected_server.clone())))
                .map(|()| {
                    self.install_active_session_runtime(
                        Box::new(session),
                        runtime_settings.clone(),
                    );
                })
        } else {
            let Some(session) = self.session.as_mut() else {
                self.clear_pending_operation_with_runtime_error(
                    handle,
                    projected_state,
                    "Public server connect could not bootstrap a detached client-core session runtime."
                        .to_owned(),
                );
                return false;
            };
            session
                .sync_runtime_settings(&runtime_settings)
                .and_then(|()| session.connect_public_server(Some(selected_server.clone())))
                .map(|()| {
                    self.active_session_configured_settings = Some(runtime_settings.clone());
                    self.active_session_settings = Some(runtime_settings.clone());
                    self.session_projects_to_shell = true;
                })
        };
        match connect_result {
            Ok(()) => {
                if !replace_owned_transport {
                    self.report_current_external_player_availability();
                }
                self.reset_session_transport_reconnect_state();
                self.session_default_room = default_room;
                self.pending_room_change_request = None;
                self.clear_session_causal_player_effect_state();
                self.last_published_local_file = None;
                self.last_published_media_match_signature = None;
                if let Some(driver) = replacement_transport_driver {
                    self.replace_owned_session_transport_driver(driver);
                }
                let pending_requirements = self.pending_apply_requirements_action(
                    projected_state,
                    &projected_state.saved_configuration,
                );
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CompleteSelectedPublicServerConnect,
                        pending_requirements,
                    ],
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
        self.handle_complete_public_server_refresh_request_with_fetcher(
            handle,
            projected_state,
            requested_servers,
            Self::refresh_public_servers_without_session,
        )
    }

    pub(in crate::app) fn handle_complete_public_server_refresh_request_with_fetcher<F>(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        requested_servers: Vec<(String, String)>,
        fetch_detached: F,
    ) -> bool
    where
        F: FnOnce(Option<&str>) -> Result<Vec<(String, String)>, String>,
    {
        let current_servers = projected_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect();
        let language = Some(projected_state.runtime_language_tag_legacy_compatible());
        let refresh_result = if let Some(session) = self.session.as_mut() {
            session.refresh_public_servers(current_servers, language)
        } else {
            // The request retains the current rows for portable fallback compatibility. An
            // owner-backed manual refresh must never substitute that cache for a remote result.
            let _ = requested_servers;
            fetch_detached(language)
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
                format!("Public server refresh failed: {error}"),
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
                    GuiUserMediaTargetResolution::Resolved { path, source } => {
                        normalized_editable_text(&path).map(|path| (path, source))
                    }
                    GuiUserMediaTargetResolution::Pending => return true,
                    GuiUserMediaTargetResolution::Ambiguous { .. }
                    | GuiUserMediaTargetResolution::Missing => target_file_name
                        .as_deref()
                        .and_then(|target| {
                            self.media_match_cached_room_candidate_for_target(
                                projected_state,
                                target,
                            )
                        })
                        .map(|path| {
                            (
                                path,
                                GuiUserMediaTargetResolutionSource::MediaMatchExactInventory,
                            )
                        }),
                };
                self.ensure_configured_player_attached();
                match found_path {
                    Some((path, source)) if self.player.is_some() => {
                        self.clear_pending_operation_runtime_state(handle, projected_state);
                        if self.current_player_matches_media_target(&path)
                            || self.current_player_is_loading_media_target(&path)
                        {
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
                            self.supersede_playlist_resolution_attempt();
                            match self.open_media_files_through_attached_player_result_impl(
                                std::slice::from_ref(&path),
                                true,
                            ) {
                                Some(Ok(started)) => {
                                    self.bind_started_local_media_load_to_current_playlist(
                                        projected_state,
                                        path,
                                        source,
                                        &started,
                                    );
                                    Self::push_player_success(handle, started.feedback_message);
                                }
                                Some(Err(message)) => Self::push_player_error(handle, message),
                                None => {}
                            }
                        }
                    }
                    found_path => Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::CompleteMissingMediaSearch(
                            found_path.map(|(path, _)| path),
                        )],
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
                Err(error) => Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CompleteLocalChatSend,
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: format!(
                                "Chat sending through the attached session runtime failed: {error}"
                            ),
                        },
                    ],
                ),
            }
        } else {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CompleteLocalChatSend,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: self.send_chat_unavailable_message(),
                    },
                ],
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
        let previous_settings = projected_state.saved_configuration.clone();
        let Some(path) = self.persisted_settings_config_path_for_request(projected_state) else {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CancelConfigurationSave,
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message:
                            "Configuration save failed: no writable GUI config path is available."
                                .to_owned(),
                    },
                ],
            );
            return true;
        };
        match merge_sorotte_ini_stored_client_settings_mvp_at_path(
            &path,
            &previous_settings,
            &settings,
        ) {
            Ok(settings) => {
                self.config_path = Some(path);
                self.promote_on_save_runtime_fields(&settings);
                if self.apply_saved_player_settings_in_place(&settings) {
                    self.promote_restart_player_runtime_fields(&settings);
                }
                self.adopt_saved_player_launch_state_when_inactive(&settings);
                self.invalidate_plex_operation_context_if_settings_changed(
                    handle,
                    projected_state,
                    &previous_settings,
                    &settings,
                );
                let pending_requirements =
                    self.pending_apply_requirements_action(projected_state, &settings);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CompleteConfigurationSave(settings),
                        pending_requirements,
                    ],
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
        self.invalidate_plex_operation_context(handle, projected_state);
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::CompleteDiscardConfigurationChanges(
                settings,
            )],
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
            self.invalidate_plex_operation_context(handle, projected_state);
            let pending_requirements =
                self.pending_apply_requirements_action(projected_state, &fallback_settings);
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![
                    GuiShellAction::CompleteConfigurationReload(fallback_settings),
                    pending_requirements,
                ],
            );
            return false;
        };
        match load_sorotte_ini_stored_client_settings_mvp_from_path(path) {
            Ok(Some(settings)) => {
                self.invalidate_plex_operation_context(handle, projected_state);
                self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
                self.promote_on_save_runtime_fields(&settings);
                if self.current_player_core_state_is_applied() {
                    self.promote_restart_player_runtime_fields(&settings);
                }
                let pending_requirements =
                    self.pending_apply_requirements_action(projected_state, &settings);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CompleteConfigurationReload(settings),
                        pending_requirements,
                    ],
                );
            }
            Ok(None) => {
                self.invalidate_plex_operation_context(handle, projected_state);
                self.sync_player_from_lookup_and_settings(
                    &env_trimmed,
                    Some(&fallback_settings),
                    true,
                );
                self.promote_on_save_runtime_fields(&fallback_settings);
                if self.current_player_core_state_is_applied() {
                    self.promote_restart_player_runtime_fields(&fallback_settings);
                }
                let pending_requirements =
                    self.pending_apply_requirements_action(projected_state, &fallback_settings);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CompleteConfigurationReload(fallback_settings),
                        pending_requirements,
                    ],
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
                self.invalidate_plex_operation_context(handle, projected_state);
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

        let source = self.persisted_settings_config_path_for_request(projected_state);
        let settings = match relocate_sorotte_ini_stored_client_settings_mvp_at_path(
            source.as_deref(),
            &paths.config_path,
            &projected_state.saved_configuration,
            &settings,
            || persist_sorotte_client_install_locator(&install_root, &paths.storage_root),
        ) {
            Ok(settings) => settings,
            Err(error) => {
                return self.cancel_config_storage_change_with_error(
                    handle,
                    projected_state,
                    error.to_string(),
                );
            }
        };

        let copy_warnings =
            Self::copy_known_storage_entries_best_effort(old_root.as_deref(), &paths.storage_root);
        self.invalidate_plex_operation_context(handle, projected_state);
        self.config_path = Some(paths.config_path.clone());
        self.promote_on_save_runtime_fields(&settings);
        if self.apply_saved_player_settings_in_place(&settings) {
            self.promote_restart_player_runtime_fields(&settings);
        }
        self.adopt_saved_player_launch_state_when_inactive(&settings);
        self.clear_attached_media_search_runtime_cache();
        let stream_helper_snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(None);
        let snapshot = GuiConfigStorageRuntimeSnapshot::from_storage_paths(&paths);
        let pending_requirements =
            self.pending_apply_requirements_action(projected_state, &settings);
        let mut actions = vec![
            GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings },
            pending_requirements,
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
        let metadata = match std::fs::symlink_metadata(src) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                warnings.push(format!("{}: {error}", src.display()));
                return;
            }
        };
        if config_storage_metadata_is_reparse_or_symlink(&metadata) {
            warnings.push(format!(
                "{}: refusing to copy linked storage path",
                src.display()
            ));
            return;
        }
        match std::fs::symlink_metadata(dst) {
            Ok(metadata) if config_storage_metadata_is_reparse_or_symlink(&metadata) => {
                warnings.push(format!(
                    "{}: refusing to write through linked storage path",
                    dst.display()
                ));
                return;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warnings.push(format!("{}: {error}", dst.display()));
                return;
            }
        }
        if metadata.is_file() {
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
        if !metadata.is_dir() {
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

#[cfg(test)]
mod config_relocation_link_tests {
    use super::*;

    fn config_copy_test_root(label: &str) -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sorotte-config-copy-{label}-{}-{unique_suffix}",
            std::process::id()
        ))
    }

    #[cfg(windows)]
    #[test]
    fn config_storage_copy_does_not_follow_descendant_junctions() {
        let root = config_copy_test_root("junction");
        let old_root = root.join("old");
        let new_root = root.join("new");
        let outside_root = root.join("outside");
        let old_cache = old_root.join("cache");
        std::fs::create_dir_all(&old_cache).expect("old cache root should be created");
        std::fs::create_dir_all(&outside_root).expect("outside root should be created");
        std::fs::write(outside_root.join("private-token.txt"), b"outside")
            .expect("outside fixture should be written");

        let junction = old_cache.join("external");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside_root)
            .status()
            .expect("junction command should start");
        assert!(status.success(), "test junction should be created");

        let warnings = GuiPersistedConfigRuntimeOwner::copy_known_storage_entries_best_effort(
            Some(&old_root),
            &new_root,
        );

        assert!(
            !new_root
                .join("cache")
                .join("external")
                .join("private-token.txt")
                .exists(),
            "config relocation must not traverse a descendant junction and copy files outside the old config root; warnings: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("refusing to copy linked storage path")),
            "skipping a configured storage junction should be visible: {warnings:?}"
        );
        let _ = std::fs::remove_dir(&junction);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn config_storage_copy_does_not_follow_descendant_symlinks() {
        let root = config_copy_test_root("symlink");
        let old_root = root.join("old");
        let new_root = root.join("new");
        let outside_root = root.join("outside");
        let old_cache = old_root.join("cache");
        std::fs::create_dir_all(&old_cache).expect("old cache root should be created");
        std::fs::create_dir_all(&outside_root).expect("outside root should be created");
        std::fs::write(outside_root.join("private-token.txt"), b"outside")
            .expect("outside fixture should be written");
        std::os::unix::fs::symlink(&outside_root, old_cache.join("external"))
            .expect("test symlink should be created");

        let warnings = GuiPersistedConfigRuntimeOwner::copy_known_storage_entries_best_effort(
            Some(&old_root),
            &new_root,
        );

        assert!(
            !new_root
                .join("cache")
                .join("external")
                .join("private-token.txt")
                .exists(),
            "config relocation must not traverse a descendant symlink; warnings: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("refusing to copy linked storage path")),
            "skipping a configured storage symlink should be visible: {warnings:?}"
        );
        let _ = std::fs::remove_file(old_cache.join("external"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
