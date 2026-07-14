use super::*;

#[derive(Default)]
pub(in crate::app) struct GuiPreviewRuntimeBridge;

impl GuiPreviewRuntimeBridge {
    pub(in crate::app) fn preview_open_media_file_actions(
        state: Option<&SorotteGuiShellAppState>,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
        playlist_insert_slot: Option<usize>,
    ) -> Vec<GuiShellAction> {
        if paths.is_empty() {
            return Vec::new();
        }

        let mut actions = vec![GuiShellAction::SwitchView(GuiShellView::Room)];
        if load_into_shared_playlist {
            match GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(paths) {
                Ok(dispatch) => {
                    let (playlist_entries, opened_entry_count) = state
                        .map(|state| {
                            let current_count = state.main_window.playlist.len();
                            let current_index = state
                                .main_window
                                .active_playlist_index
                                .or_else(|| {
                                    (!state.main_window_playlist_selection_is_local)
                                        .then_some(state.selection.selected_main_window_playlist)
                                        .flatten()
                                });
                            let (playlist_entries, _) = state
                                .shared_playlist_entries_after_media_open_from_state_with_current_index(
                                    dispatch.playlist_entries(),
                                    playlist_insert_slot,
                                    current_index,
                                );
                            let opened_entry_count = if playlist_insert_slot.is_some() {
                                playlist_entries.len().saturating_sub(current_count)
                            } else {
                                playlist_entries.len()
                            };
                            (playlist_entries, opened_entry_count)
                        })
                        .unwrap_or_else(|| {
                            let playlist_entries = dispatch.playlist_entries();
                            let opened_entry_count = playlist_entries.len();
                            (playlist_entries, opened_entry_count)
                        });
                    if playlist_insert_slot.is_some() && opened_entry_count == 0 {
                        return Vec::new();
                    }
                    actions.push(GuiShellAction::AnnounceSharedPlaylistLoaded(
                        playlist_entries,
                    ));
                }
                Err(error) => {
                    actions.push(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    });
                    actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
                }
            }
            return actions;
        }

        let message = if paths.len() == 1 {
            format!("Media file selected: {}.", paths[0])
        } else {
            format!("Media files selected: {} entries.", paths.len())
        };
        actions.push(GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Info,
            message: message.clone(),
        });
        actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        actions
    }

    fn preview_seek_actions(offset_seconds: f64) -> Vec<GuiShellAction> {
        let message = format!("Seek requested: {offset_seconds} seconds.");
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn preview_offset_actions(command: &LocalOffsetCommand) -> Vec<GuiShellAction> {
        let message = format!("Offset requested: {}.", format_offset_command(command));
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    pub(in crate::app) fn preview_pending_completion_actions(
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        if state.pending_saved_server_connect_saves_configuration
            && state
                .pending_operation
                .as_ref()
                .is_some_and(|pending| pending.kind == GuiPendingOperationKind::ConnectSavedServer)
        {
            return vec![
                GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                    GuiSavedConfigurationRuntimeSnapshot {
                        settings: state.configuration.to_stored_settings(),
                    },
                ),
                GuiShellAction::CompleteSavedServerConnect,
            ];
        }

        GuiPendingCompletionRequest::from_state(state)
            .map(GuiPendingCompletionRequest::into_action)
            .into_iter()
            .collect()
    }

    pub(in crate::app) fn preview_pending_cancel_actions(
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        state
            .pending_operation
            .as_ref()
            .map(|_| GuiShellAction::CancelPendingOperation)
            .into_iter()
            .collect()
    }
}

impl GuiNativeRuntimeBridge for GuiPreviewRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool {
        true
    }

    fn dispatch_runtime_request(
        &mut self,
        state: &SorotteGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        request.preview_actions_for_state(state)
    }

    fn actions_for_open_media_files(
        &mut self,
        state: &SorotteGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction> {
        Self::preview_open_media_file_actions(
            Some(state),
            paths,
            load_into_shared_playlist || state.playlist_backed_media_opens_preferred(),
            None,
        )
    }

    fn actions_for_seek_offset(&mut self, offset_seconds: f64) -> Vec<GuiShellAction> {
        Self::preview_seek_actions(offset_seconds)
    }

    fn actions_for_undo_seek(&mut self) -> Vec<GuiShellAction> {
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Undo seek requested.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Undo seek requested.".to_owned()),
        ]
    }

    fn actions_for_set_offset(&mut self, command: LocalOffsetCommand) -> Vec<GuiShellAction> {
        Self::preview_offset_actions(&command)
    }

    fn actions_for_main_window_user_media_open(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        Self::preview_open_media_file_actions(
            Some(state),
            vec![target],
            state.playlist_backed_media_opens_preferred(),
            None,
        )
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SorotteGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        let message = format!("Open containing folder requested: {target}.");
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_completion_actions(state)
    }

    fn actions_for_pending_cancel(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_cancel_actions(state)
    }
}
