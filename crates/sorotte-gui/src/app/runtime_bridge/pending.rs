use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiPendingCompletionRequest {
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
    pub(in crate::app) fn from_state(state: &SorotteGuiShellAppState) -> Option<Self> {
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

    pub(in crate::app::runtime_bridge) fn into_action(self) -> GuiShellAction {
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
pub(in crate::app) enum GuiPendingRoomChangeRequest {
    Join { requested_room: String },
    ReturnToDefault { previous_room: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiSharedPlaylistOpenDispatch {
    pub(in crate::app) playlist_entries: Vec<String>,
    pub(in crate::app) player_paths: Option<Vec<String>>,
    pub(in crate::app) imported_from_file: bool,
}
