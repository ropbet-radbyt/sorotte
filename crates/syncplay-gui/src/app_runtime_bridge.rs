#[cfg(test)]
#[path = "app_runtime_bridge/tests.rs"]
mod tests;

use syncplay_client_app::app_boundary::{
    commands::LocalOffsetCommand, state::StoredClientSettingsMvp,
};

use super::render_io::GuiDroppedFilesRequest;
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::{
    GuiPendingOperationKind, GuiSavedConfigurationRuntimeSnapshot, GuiShellAction, GuiShellView,
    GuiTransientNotificationLevel, SyncplayGuiShellAppState,
};
use super::support::format_offset_command;

pub(super) trait GuiNativeRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool;

    fn drain_runtime_actions(&mut self) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn dispatch_runtime_request(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_open_media_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction>;

    fn actions_for_selected_media_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
    ) -> Vec<GuiShellAction> {
        self.actions_for_open_media_files(
            state,
            paths,
            state.playlist_backed_media_opens_preferred(),
        )
    }

    fn actions_for_dropped_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        request: GuiDroppedFilesRequest,
    ) -> Vec<GuiShellAction> {
        self.dispatch_runtime_request(
            state,
            GuiRuntimeRequest::OpenMediaFiles {
                paths: request.paths,
                load_into_shared_playlist: request.target.load_into_shared_playlist(state),
                playlist_insert_slot: request.playlist_insert_slot,
            },
        )
    }

    fn actions_for_seek_offset(&mut self, offset_seconds: f64) -> Vec<GuiShellAction>;

    fn actions_for_undo_seek(&mut self) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_set_offset(&mut self, _command: LocalOffsetCommand) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_autoplay_enabled_change(&mut self, _enabled: bool) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_autoplay_threshold_change(&mut self, _threshold: usize) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_media_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_join(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _room: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_leave(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_local_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _username: String,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_controller_auth_request(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _room: String,
        _password: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_commit(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _entry: String,
        _select_after_queue: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_selection_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_removal(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_reorder(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _playlist: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_undo(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_remaining(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_entire(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction>;

    fn actions_for_pending_cancel(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction>;
}

pub(super) trait GuiNativeRuntimePump {
    fn pump(&mut self, state: &SyncplayGuiShellAppState);
}

pub(super) trait GuiQueuedRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SyncplayGuiShellAppState);
}

#[derive(Default)]
pub(super) struct GuiNoopRuntimePump;

impl GuiNativeRuntimePump for GuiNoopRuntimePump {
    fn pump(&mut self, _state: &SyncplayGuiShellAppState) {}
}

#[allow(dead_code)]
#[derive(Default)]
pub(super) struct GuiPreviewRuntimeOwner;

#[allow(dead_code)]
impl GuiPreviewRuntimeOwner {
    fn push_preview_response(
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
        request: GuiRuntimeRequest,
    ) {
        let actions = request.preview_actions_for_state(state);
        if !actions.is_empty() {
            handle.push_actions(actions);
        }
    }
}

impl GuiQueuedRuntimeOwner for GuiPreviewRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SyncplayGuiShellAppState) {
        for request in handle.drain_requests() {
            Self::push_preview_response(handle, state, request);
        }
    }
}

#[derive(Default)]
pub(super) struct GuiPreviewRuntimeBridge;

impl GuiPreviewRuntimeBridge {
    pub(super) fn preview_open_media_file_actions(
        state: Option<&SyncplayGuiShellAppState>,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
        playlist_insert_slot: Option<usize>,
    ) -> Vec<GuiShellAction> {
        if paths.is_empty() {
            return Vec::new();
        }

        let mut actions = vec![GuiShellAction::SwitchView(GuiShellView::MainWindow)];
        if load_into_shared_playlist {
            match GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(paths) {
                Ok(dispatch) => {
                    let playlist_entries = state
                        .map(|state| {
                            state
                                .shared_playlist_entries_after_media_open_from_state(
                                    dispatch.playlist_entries.clone(),
                                    playlist_insert_slot,
                                )
                                .0
                        })
                        .unwrap_or(dispatch.playlist_entries);
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

    pub(super) fn preview_pending_completion_actions(
        state: &SyncplayGuiShellAppState,
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

    pub(super) fn preview_pending_cancel_actions(
        state: &SyncplayGuiShellAppState,
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
        state: &SyncplayGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        request.preview_actions_for_state(state)
    }

    fn actions_for_open_media_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
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
        state: &SyncplayGuiShellAppState,
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
        _state: &SyncplayGuiShellAppState,
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
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_completion_actions(state)
    }

    fn actions_for_pending_cancel(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_cancel_actions(state)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiPendingCompletionRequest {
    SaveConfiguration(StoredClientSettingsMvp),
    ResetConfiguration(StoredClientSettingsMvp),
    ReloadConfiguration(StoredClientSettingsMvp),
    ClearGuiData,
    ConnectSavedServer,
    DisconnectSession,
    ConnectPublicServer,
    RefreshPublicServers(Vec<(String, String)>),
    SearchMissingMedia,
    TogglePlaybackPause,
    SendChatMessage(String),
}

impl GuiPendingCompletionRequest {
    pub(super) fn from_state(state: &SyncplayGuiShellAppState) -> Option<Self> {
        let pending = state.pending_operation.as_ref()?;
        Some(match pending.kind {
            GuiPendingOperationKind::SaveConfiguration => {
                Self::SaveConfiguration(state.configuration.to_stored_settings())
            }
            GuiPendingOperationKind::ResetConfiguration => {
                Self::ResetConfiguration(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ReloadConfiguration => {
                Self::ReloadConfiguration(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ClearGuiData => Self::ClearGuiData,
            GuiPendingOperationKind::ConnectSavedServer => Self::ConnectSavedServer,
            GuiPendingOperationKind::DisconnectSession => Self::DisconnectSession,
            GuiPendingOperationKind::ConnectPublicServer => Self::ConnectPublicServer,
            GuiPendingOperationKind::RefreshPublicServers => Self::RefreshPublicServers(
                state
                    .public_servers
                    .servers
                    .iter()
                    .map(|row| (row.label.clone(), row.address.clone()))
                    .collect(),
            ),
            GuiPendingOperationKind::SearchMissingMedia => Self::SearchMissingMedia,
            GuiPendingOperationKind::TogglePlaybackPause => Self::TogglePlaybackPause,
            GuiPendingOperationKind::SendChatMessage => {
                Self::SendChatMessage(state.outgoing_chat_message.clone()?)
            }
        })
    }

    fn into_action(self) -> GuiShellAction {
        match self {
            Self::SaveConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationSave(settings)
            }
            Self::ResetConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationReset(settings)
            }
            Self::ReloadConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationReload(settings)
            }
            Self::ClearGuiData => GuiShellAction::CompleteClearGuiData,
            Self::ConnectSavedServer => GuiShellAction::CompleteSavedServerConnect,
            Self::DisconnectSession => GuiShellAction::CompleteSessionDisconnect,
            Self::ConnectPublicServer => GuiShellAction::CompleteSelectedPublicServerConnect,
            Self::RefreshPublicServers(servers) => {
                GuiShellAction::CompletePublicServerRefresh(servers)
            }
            Self::SearchMissingMedia => GuiShellAction::CompleteMissingMediaSearch(None),
            Self::TogglePlaybackPause => GuiShellAction::CompletePlaybackPauseToggle,
            Self::SendChatMessage(_) => GuiShellAction::CompleteLocalChatSend,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiPendingRoomChangeRequest {
    Join { requested_room: String },
    ReturnToDefault { previous_room: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiSharedPlaylistOpenDispatch {
    pub(super) playlist_entries: Vec<String>,
    pub(super) player_paths: Option<Vec<String>>,
    pub(super) imported_from_file: bool,
}

#[allow(clippy::large_enum_variant)]
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiRuntimeRequest {
    OpenMediaFiles {
        paths: Vec<String>,
        load_into_shared_playlist: bool,
        playlist_insert_slot: Option<usize>,
    },
    OpenMainWindowUserMedia(String),
    OpenMainWindowUserContainingFolder(String),
    UndoSeek,
    SetOffset(LocalOffsetCommand),
    SetAutoplayEnabled(bool),
    SetAutoplayThreshold(usize),
    SetRoom(String),
    ReturnToDefaultRoom,
    SetLocalReady(bool),
    SetReadyForUser {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    QueuePlaylistEntry {
        entry: String,
        select_after_queue: bool,
    },
    SetPlaylistIndex(usize),
    DeletePlaylistIndex(usize),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    ReplacePlaylist {
        files: Vec<String>,
        selected_index: Option<usize>,
    },
    SendChatMessage(String),
    SeekOffset(f64),
    SeekToPosition(f64),
    AdvancePlaylistIndex,
    TogglePlaybackPause,
    CompletePendingOperation(GuiPendingCompletionRequest),
    CancelPendingOperation(GuiPendingOperationKind),
}

impl GuiRuntimeRequest {
    pub(super) fn preview_actions_for_state(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        match self {
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
                playlist_insert_slot,
            } => GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                Some(state),
                paths.clone(),
                *load_into_shared_playlist || state.playlist_backed_media_opens_preferred(),
                *playlist_insert_slot,
            ),
            Self::OpenMainWindowUserMedia(target) => {
                GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                    Some(state),
                    vec![target.clone()],
                    state.playlist_backed_media_opens_preferred(),
                    None,
                )
            }
            Self::SendChatMessage(_message) => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Chat sent.".to_owned(),
            }],
            Self::SeekToPosition(target_position_seconds) => {
                let message = format!("Seek requested: target {target_position_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::AdvancePlaylistIndex => Vec::new(),
            Self::TogglePlaybackPause => {
                if state.main_window.playback_paused {
                    vec![GuiShellAction::AnnouncePlaybackResumed]
                } else {
                    vec![GuiShellAction::AnnouncePlaybackPaused]
                }
            }
            Self::CompletePendingOperation(GuiPendingCompletionRequest::ConnectSavedServer)
                if state.pending_saved_server_connect_saves_configuration =>
            {
                vec![
                    GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                        GuiSavedConfigurationRuntimeSnapshot {
                            settings: state.configuration.to_stored_settings(),
                        },
                    ),
                    GuiShellAction::CompleteSavedServerConnect,
                ]
            }
            _ => self.preview_actions(),
        }
    }

    pub(super) fn preview_actions(&self) -> Vec<GuiShellAction> {
        match self {
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
                playlist_insert_slot,
            } => GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                None,
                paths.clone(),
                *load_into_shared_playlist,
                *playlist_insert_slot,
            ),
            Self::OpenMainWindowUserMedia(target) => {
                GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                    None,
                    vec![target.clone()],
                    false,
                    None,
                )
            }
            Self::OpenMainWindowUserContainingFolder(target) => {
                let message = format!("Open containing folder requested: {target}.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::UndoSeek => vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Undo seek requested.".to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent("Undo seek requested.".to_owned()),
            ],
            Self::SetOffset(command) => {
                let message = format!("Offset requested: {}.", format_offset_command(command));
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetAutoplayEnabled(_)
            | Self::SetAutoplayThreshold(_)
            | Self::SetReadyForUser { .. }
            | Self::RequestControllerAuth { .. }
            | Self::QueuePlaylistEntry { .. }
            | Self::SetPlaylistIndex(_)
            | Self::AdvancePlaylistIndex
            | Self::DeletePlaylistIndex(_)
            | Self::UndoPlaylistChange
            | Self::ShuffleRemainingPlaylist
            | Self::ShuffleEntirePlaylist
            | Self::ReplacePlaylist { .. }
            | Self::SetRoom(_)
            | Self::ReturnToDefaultRoom
            | Self::SendChatMessage(_)
            | Self::TogglePlaybackPause => Vec::new(),
            Self::SeekOffset(offset_seconds) => {
                let message = format!("Seek requested: {offset_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SeekToPosition(target_position_seconds) => {
                let message = format!("Seek requested: target {target_position_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetLocalReady(_) => Vec::new(),
            Self::CompletePendingOperation(request) => vec![request.clone().into_action()],
            Self::CancelPendingOperation(_) => vec![GuiShellAction::CancelPendingOperation],
        }
    }
}
