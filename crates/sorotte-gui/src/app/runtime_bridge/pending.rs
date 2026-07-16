use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiPendingCompletionRequest {
    SaveConfiguration(StoredClientSettingsMvp),
    DiscardConfigurationChanges(StoredClientSettingsMvp),
    ReloadConfiguration(StoredClientSettingsMvp),
    ClearGuiData,
    ChangeConfigStorageRoot {
        target: GuiConfigStorageChangeTarget,
        settings: StoredClientSettingsMvp,
    },
    ConnectSavedServer,
    DisconnectSession,
    ConnectPublicServer,
    RefreshPublicServers(Vec<(String, String)>),
    SearchMissingMedia,
    SetPlaybackPause(bool),
    TogglePlaybackPause,
    SendChatMessage(String),
}

impl GuiPendingCompletionRequest {
    pub(in crate::app) fn from_state(state: &SorotteGuiShellAppState) -> Option<Self> {
        let pending = state.pending_operation.as_ref()?;
        Some(match pending.kind {
            GuiPendingOperationKind::SaveConfiguration => {
                Self::SaveConfiguration(state.configuration.to_stored_settings())
            }
            GuiPendingOperationKind::DiscardConfigurationChanges => {
                Self::DiscardConfigurationChanges(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ReloadConfiguration => {
                Self::ReloadConfiguration(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ClearGuiData => Self::ClearGuiData,
            GuiPendingOperationKind::ChangeConfigStorageRoot => Self::ChangeConfigStorageRoot {
                target: state.pending_config_storage_target.clone()?,
                settings: state.configuration.to_stored_settings(),
            },
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
            GuiPendingOperationKind::SetPlaybackPause(paused) => Self::SetPlaybackPause(paused),
            GuiPendingOperationKind::TogglePlaybackPause => Self::TogglePlaybackPause,
            GuiPendingOperationKind::SendChatMessage => {
                Self::SendChatMessage(state.outgoing_chat_message.clone()?)
            }
        })
    }

    pub(in crate::app::runtime_bridge) fn into_action(self) -> GuiShellAction {
        match self {
            Self::SaveConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationSave(settings)
            }
            Self::DiscardConfigurationChanges(settings) => {
                GuiShellAction::CompleteDiscardConfigurationChanges(settings)
            }
            Self::ReloadConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationReload(settings)
            }
            Self::ClearGuiData => GuiShellAction::CompleteClearGuiData,
            Self::ChangeConfigStorageRoot { .. } => GuiShellAction::CancelConfigStorageRootChange,
            Self::ConnectSavedServer => GuiShellAction::CompleteSavedServerConnect,
            Self::DisconnectSession => GuiShellAction::CompleteSessionDisconnect,
            Self::ConnectPublicServer => GuiShellAction::CompleteSelectedPublicServerConnect,
            Self::RefreshPublicServers(servers) => {
                GuiShellAction::CompletePublicServerRefresh(servers)
            }
            Self::SearchMissingMedia => GuiShellAction::CompleteMissingMediaSearch(None),
            Self::SetPlaybackPause(paused) => GuiShellAction::CompletePlaybackPauseState(paused),
            Self::TogglePlaybackPause => GuiShellAction::CompletePlaybackPauseToggle,
            Self::SendChatMessage(_) => GuiShellAction::CompleteLocalChatSend,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum GuiPendingRoomChangeRequest {
    Join { requested_room: String },
    ReturnToDefault { previous_room: String },
}

impl std::fmt::Debug for GuiPendingRoomChangeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Join { .. } => formatter
                .debug_struct("Join")
                .field("requested_room", &sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::ReturnToDefault { .. } => formatter
                .debug_struct("ReturnToDefault")
                .field("previous_room", &sorotte_secret::REDACTED_SECRET)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiSharedPlaylistOpenItem {
    pub(in crate::app) published_entry: String,
    pub(in crate::app) local_origin: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiSharedPlaylistOpenDispatch {
    pub(in crate::app) items: Vec<GuiSharedPlaylistOpenItem>,
    pub(in crate::app) imported_from_file: bool,
}

impl GuiSharedPlaylistOpenDispatch {
    pub(in crate::app) fn playlist_entries(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| item.published_entry.clone())
            .collect()
    }
}

impl std::fmt::Debug for GuiSharedPlaylistOpenDispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiSharedPlaylistOpenDispatch")
            .field("items", &sorotte_secret::REDACTED_SECRET)
            .field("item_count", &self.items.len())
            .field(
                "local_origin_count",
                &self
                    .items
                    .iter()
                    .filter(|item| item.local_origin.is_some())
                    .count(),
            )
            .field("imported_from_file", &self.imported_from_file)
            .finish()
    }
}

#[cfg(test)]
mod media_target_debug_tests {
    use super::*;

    #[test]
    fn pending_room_and_playlist_dispatch_debug_redact_paths_and_targets() {
        let secret = "https://media.example/item?token=pending-dispatch-canary";
        let room = GuiPendingRoomChangeRequest::Join {
            requested_room: secret.to_owned(),
        };
        let dispatch = GuiSharedPlaylistOpenDispatch {
            items: vec![GuiSharedPlaylistOpenItem {
                published_entry: secret.to_owned(),
                local_origin: Some(secret.to_owned()),
            }],
            imported_from_file: false,
        };

        for debug in [format!("{room:?}"), format!("{dispatch:?}")] {
            assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!debug.contains("pending-dispatch-canary"));
        }
    }
}
