use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn open_main_window_user_media_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!("Indexing media library to resolve user media: {target}."),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if projected_state.playlist_backed_media_opens_preferred() {
            self.open_media_files_through_shared_playlist_runtime_impl(
                handle,
                projected_state,
                vec![resolved_target],
                None,
            );
            return;
        }

        self.ensure_configured_player_attached();
        if self.player.is_some() {
            if !self.preflight_user_stream_target(&resolved_target) {
                return;
            }
            self.prepare_stream_load_tracking(&resolved_target, true);
            self.open_media_files_through_attached_player_impl(handle, vec![resolved_target]);
        } else {
            Self::push_runtime_unavailable(
                handle,
                self.open_media_unavailable_message_impl(&[resolved_target]),
            );
        }
    }

    fn project_loaded_shared_playlist_into_state(
        projected_state: &mut SorotteGuiShellAppState,
        entries: Vec<String>,
        selected_index: Option<usize>,
    ) -> bool {
        let entries = SorotteGuiShellAppState::normalize_shared_playlist_entries(entries);
        let selected_index = selected_index
            .filter(|_| !entries.is_empty())
            .map(|index| index.min(entries.len().saturating_sub(1)));
        projected_state.main_window.shared_playlist_enabled = true;
        projected_state.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        projected_state.apply_shared_playlist_entries(entries.clone(), selected_index, false);
        projected_state.main_window.active_playlist_index = selected_index;
        true
    }

    fn open_system_folder(path: &Path, description: &str) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg(path);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(path);
            command
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(path);
            command
        };

        command.spawn().map_err(|error| {
            format!(
                "Opening {description} at '{}' failed: {error}",
                path.display(),
            )
        })?;
        Ok(())
    }

    fn open_system_file_browser_for_path(path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err(format!(
                "Could not open a containing folder for '{}': no parent directory exists.",
                path.display()
            ));
        };
        Self::open_system_folder(parent, "the containing folder")
    }

    pub(in crate::app::runtime_owner) fn open_main_window_user_containing_folder_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!(
                            "Indexing media library to resolve a local path for user media: {target}."
                        ),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if browser_is_url(&resolved_target) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Cannot open a containing folder for the stream URL: {resolved_target}."),
            );
            return;
        }

        if let Err(error) = Self::open_system_file_browser_for_path(Path::new(&resolved_target)) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }

    pub(in crate::app::runtime_owner) fn open_media_files_through_shared_playlist_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        paths: Vec<String>,
        playlist_insert_slot: Option<usize>,
    ) {
        let selected_paths = paths
            .into_iter()
            .filter_map(|path| normalized_editable_text(&path))
            .collect::<Vec<_>>();
        if selected_paths.is_empty() {
            return;
        }

        let dispatch =
            match Self::shared_playlist_open_dispatch_for_paths_impl(selected_paths.clone()) {
                Ok(dispatch) => dispatch,
                Err(error) => {
                    Self::push_runtime_unavailable(handle, error);
                    return;
                }
            };
        let current_playlist_entry_count = projected_state.main_window.playlist.len();
        let current_playlist_index =
            self.shared_playlist_mutation_current_index(projected_state, false);
        let (playlist_entries, selected_playlist_index) = projected_state
            .shared_playlist_entries_after_media_open_from_state_with_current_index(
                dispatch.playlist_entries.clone(),
                playlist_insert_slot,
                current_playlist_index,
            );
        let opened_entry_count = if playlist_insert_slot.is_some() {
            playlist_entries
                .len()
                .saturating_sub(current_playlist_entry_count)
        } else {
            playlist_entries.len()
        };
        if playlist_insert_slot.is_some() && opened_entry_count == 0 {
            return;
        }
        let selected_opened_entry_offset = Self::selected_opened_entry_offset(
            selected_playlist_index,
            opened_entry_count,
            playlist_insert_slot,
        );

        if self.session.is_none() {
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }

            if !Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
            ) {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }
            self.active_shared_playlist_index = selected_playlist_index;
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                    MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                )],
            );

            let selected_media_sync = selected_opened_entry_offset
                .and_then(|offset| {
                    dispatch
                        .player_paths
                        .as_ref()
                        .and_then(|player_paths| player_paths.get(offset).cloned())
                })
                .map(|selected_path| {
                    self.open_selected_playlist_media_path_through_attached_player_impl(&[
                        selected_path,
                    ])
                })
                .unwrap_or(SelectedPlaylistMediaSyncOutcome::NoChange);
            let selection_handoff_ready = selected_media_sync.selection_handoff_ready(false);
            self.sync_session_playstate_to_attached_player_impl(
                projected_state,
                selection_handoff_ready,
            );

            let success_message =
                Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            let warning = self.shared_playlist_session_unavailable_message_impl();
            handle.push_actions([
                GuiShellAction::SwitchView(GuiShellView::Room),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: success_message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(success_message),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: warning.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(warning),
            ]);
            return;
        }

        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.playlist_control_available())
        {
            Self::push_runtime_unavailable(
                handle,
                self.shared_playlist_control_unavailable_message_impl(),
            );
            return;
        }

        let Some(session) = self.session.as_mut() else {
            Self::push_runtime_unavailable(
                handle,
                self.shared_playlist_session_unavailable_message_impl(),
            );
            return;
        };
        let session_result =
            session.replace_playlist(playlist_entries.clone(), selected_playlist_index);
        let session_success = session_result.is_ok();
        if session_success {
            self.active_shared_playlist_index = selected_playlist_index;
        }
        let selected_media_sync = if session_success
            && Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
            ) {
            selected_opened_entry_offset
                .and_then(|offset| {
                    dispatch
                        .player_paths
                        .as_ref()
                        .and_then(|player_paths| player_paths.get(offset).cloned())
                })
                .map(|selected_path| {
                    self.open_selected_playlist_media_path_through_attached_player_impl(&[
                        selected_path,
                    ])
                })
                .unwrap_or(SelectedPlaylistMediaSyncOutcome::NoChange)
        } else {
            SelectedPlaylistMediaSyncOutcome::NoChange
        };
        if selected_media_sync.selection_ready()
            && let Some(session) = self.session.as_mut()
        {
            session.note_local_playlist_index_reset_intent(true);
        }
        let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
            self.session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent()),
        );
        self.apply_pending_playlist_index_reset_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );
        self.sync_session_playstate_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );

        let mut actions = Vec::new();
        if session_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::Room));
        }
        if session_success {
            let message = Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        if let Err(error) = session_result {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
        }
        handle.push_actions(actions);
    }

    pub(in crate::app::runtime_owner) fn open_stream_helper_install_location_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        install_location: PathBuf,
    ) {
        if let Err(error) = fs::create_dir_all(&install_location) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not prepare the managed stream-helper install location '{}': {error}",
                    install_location.display()
                ),
            );
            return;
        }
        if let Err(error) = Self::open_system_folder(
            &install_location,
            "the managed stream-helper install location",
        ) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }
}
