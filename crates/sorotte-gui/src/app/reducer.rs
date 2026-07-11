use sorotte_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;

use super::shell_state::{
    GuiConfigurationTextValue, GuiDialogControlKind, GuiFocusedConfigurationControlState,
    GuiMainWindowUserEditSessionState, GuiPendingOperationKind, GuiPendingOperationState,
    GuiPublicServerEditSessionState, GuiShellAction, GuiTextEditSessionState,
    GuiTransientNotificationLevel, SorotteGuiShellAppState,
    apply_media_match_settings_to_stored_settings,
};
use super::support::nonempty_room_name_text;

mod configuration_operations;
mod editing_actions;
mod main_window_actions;
mod misc_actions;
mod service_actions;
mod shell_runtime_actions;

impl SorotteGuiShellAppState {
    pub(super) fn apply(&mut self, action: GuiShellAction) -> bool {
        match &action {
            GuiShellAction::SwitchView(_)
            | GuiShellAction::SelectConfigurationTab(_)
            | GuiShellAction::OpenModal(_)
            | GuiShellAction::CloseModal
            | GuiShellAction::DismissUpdateNotice
            | GuiShellAction::BeginUpdateCheck { .. }
            | GuiShellAction::ApplyUpdateCheckResult(_)
            | GuiShellAction::ActivateUpdateIndicator
            | GuiShellAction::BeginUpdateDownload
            | GuiShellAction::BeginUpdateInstall
            | GuiShellAction::ApplyUpdateDownloadResult(_)
            | GuiShellAction::BeginStagedUpdateApply
            | GuiShellAction::ApplyStagedUpdateLaunchResult(_)
            | GuiShellAction::ApplyStartupPublicServerCache(_)
            | GuiShellAction::TrustTlsCertificatePrompt
            | GuiShellAction::RejectTlsCertificatePrompt
            | GuiShellAction::TriggerSelectedMenuAction
            | GuiShellAction::AnnounceTlsCertificatePromptRequired
            | GuiShellAction::AnnounceUpdateNoticeAvailable
            | GuiShellAction::AnnounceAboutDialogRequested
            | GuiShellAction::AnnounceHelpRequested
            | GuiShellAction::ApplyMenuDialogRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiErrorRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiCommandRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiPlexRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiDraftRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(_) => {
                self.apply_shell_runtime_action(action)
            }
            GuiShellAction::BeginConfigurationSave
            | GuiShellAction::CompleteConfigurationSave(_)
            | GuiShellAction::CancelConfigurationSave
            | GuiShellAction::BeginConfigurationReset
            | GuiShellAction::CompleteConfigurationReset(_)
            | GuiShellAction::CancelConfigurationReset
            | GuiShellAction::BeginConfigurationReload
            | GuiShellAction::CompleteConfigurationReload(_)
            | GuiShellAction::CancelConfigurationReload
            | GuiShellAction::BeginClearGuiData
            | GuiShellAction::CompleteClearGuiData
            | GuiShellAction::CancelClearGuiData
            | GuiShellAction::BeginConfigStorageRootChange(_)
            | GuiShellAction::BeginConfigStorageDefaultReset
            | GuiShellAction::CompleteConfigStorageRootChange { .. }
            | GuiShellAction::CancelConfigStorageRootChange => {
                self.apply_configuration_operation_action(action)
            }
            GuiShellAction::BeginPendingOperation(_)
            | GuiShellAction::CompletePendingOperation
            | GuiShellAction::CancelPendingOperation
            | GuiShellAction::FocusConfigurationControl { .. }
            | GuiShellAction::ActivateFocusedConfigurationControl
            | GuiShellAction::ClearConfigurationControlFocus
            | GuiShellAction::BeginAddPublicServer
            | GuiShellAction::BeginEditSelectedPublicServer
            | GuiShellAction::UpdatePublicServerEditLabel(_)
            | GuiShellAction::UpdatePublicServerEditAddress(_)
            | GuiShellAction::CommitPublicServerEdit
            | GuiShellAction::CancelPublicServerEdit
            | GuiShellAction::RemoveSelectedPublicServer
            | GuiShellAction::BeginEditSelectedMainWindowUser
            | GuiShellAction::UpdateMainWindowUserEdit(_)
            | GuiShellAction::CommitMainWindowUserEdit
            | GuiShellAction::CancelMainWindowUserEdit => self.apply_editing_action(action),
            GuiShellAction::PushTransientNotification { .. }
            | GuiShellAction::DismissTransientNotification(_)
            | GuiShellAction::DismissSetupAlert
            | GuiShellAction::ClearTransientNotifications
            | GuiShellAction::BeginConfigurationTextEdit { .. }
            | GuiShellAction::UpdateConfigurationTextEdit(_)
            | GuiShellAction::CommitConfigurationTextEdit
            | GuiShellAction::CancelConfigurationTextEdit
            | GuiShellAction::BeginRoomHistoryEdit
            | GuiShellAction::UpdateRoomHistoryEdit(_)
            | GuiShellAction::CommitRoomHistoryEdit
            | GuiShellAction::CancelRoomHistoryEdit
            | GuiShellAction::BeginSharedPlaylistTextEdit
            | GuiShellAction::UpdateSharedPlaylistTextEdit(_)
            | GuiShellAction::CancelSharedPlaylistTextEdit
            | GuiShellAction::BeginSharedPlaylistUrlEdit
            | GuiShellAction::UpdateSharedPlaylistUrlEdit(_)
            | GuiShellAction::CancelSharedPlaylistUrlEdit
            | GuiShellAction::BeginPlexPlaylistSearch
            | GuiShellAction::UpdatePlexPlaylistSearchQuery(_)
            | GuiShellAction::SubmitPlexPlaylistSearch { .. }
            | GuiShellAction::CompletePlexPlaylistSearch { .. }
            | GuiShellAction::SelectPlexPlaylistSearchResult(_)
            | GuiShellAction::AddSelectedPlexPlaylistSearchResult
            | GuiShellAction::CompletePlexPlaylistItemResolve { .. }
            | GuiShellAction::CancelPlexPlaylistSearch
            | GuiShellAction::BeginMediaUrlEdit
            | GuiShellAction::UpdateMediaUrlEdit(_)
            | GuiShellAction::CancelMediaUrlEdit
            | GuiShellAction::BeginCreateControlledRoomEdit
            | GuiShellAction::UpdateCreateControlledRoomEdit(_)
            | GuiShellAction::CancelCreateControlledRoomEdit
            | GuiShellAction::BeginControllerAuthEdit
            | GuiShellAction::UpdateControllerAuthPasswordEdit(_)
            | GuiShellAction::CancelControllerAuthEdit
            | GuiShellAction::UpdateNewMainWindowUserDraft(_)
            | GuiShellAction::CommitNewMainWindowUser
            | GuiShellAction::AppendSharedPlaylistEntries(_)
            | GuiShellAction::ReplaceSharedPlaylistEntries(_)
            | GuiShellAction::LoadSharedPlaylistFromFile { .. }
            | GuiShellAction::SaveSharedPlaylistToFile(_)
            | GuiShellAction::SelectMainWindowUser(_)
            | GuiShellAction::AddMainWindowUser(_)
            | GuiShellAction::AnnounceMainWindowUserJoined(_)
            | GuiShellAction::AnnounceSelectedMainWindowUserRenamed(_)
            | GuiShellAction::AnnounceSelectedMainWindowUserLeft
            | GuiShellAction::BeginPlaybackPause
            | GuiShellAction::BeginPlaybackResume
            | GuiShellAction::BeginPlaybackPauseToggle
            | GuiShellAction::CompletePlaybackPauseState(_)
            | GuiShellAction::CancelPlaybackPauseState
            | GuiShellAction::CompletePlaybackPauseToggle
            | GuiShellAction::CancelPlaybackPauseToggle
            | GuiShellAction::AnnouncePlaybackPaused
            | GuiShellAction::AnnouncePlaybackResumed
            | GuiShellAction::RequestSeekPrompt
            | GuiShellAction::RequestOffsetPrompt
            | GuiShellAction::RequestPlaybackUndoSeek
            | GuiShellAction::AnnounceLocalUserReady
            | GuiShellAction::AnnounceLocalUserNotReady
            | GuiShellAction::AnnounceAutoplayState(_)
            | GuiShellAction::AnnounceAutoplayThreshold(_)
            | GuiShellAction::AnnounceSharedPlaylistLoaded(_)
            | GuiShellAction::AnnounceSharedPlaylistEntryAdded(_)
            | GuiShellAction::AnnounceSharedPlaylistSelectionChanged(_)
            | GuiShellAction::AnnounceSelectedSharedPlaylistEntryRemoved
            | GuiShellAction::UndoSharedPlaylistChange
            | GuiShellAction::ShuffleRemainingSharedPlaylist
            | GuiShellAction::ShuffleEntireSharedPlaylist
            | GuiShellAction::BeginLocalChatSend(_)
            | GuiShellAction::CompleteLocalChatSend
            | GuiShellAction::CancelLocalChatSend
            | GuiShellAction::AnnounceRemoteChatMessage { .. }
            | GuiShellAction::AnnounceSystemChatEvent(_)
            | GuiShellAction::AnnounceControlledRoomCreated { .. }
            | GuiShellAction::ToggleSelectedMainWindowUserReady
            | GuiShellAction::ToggleSelectedMainWindowUserController
            | GuiShellAction::RemoveSelectedMainWindowUser
            | GuiShellAction::SelectMainWindowPlaylist(_)
            | GuiShellAction::ActivateMainWindowPlaylist(_)
            | GuiShellAction::SelectMainWindowPlaylistSource { .. }
            | GuiShellAction::SelectMainWindowPlaylistDefaultSource { .. }
            | GuiShellAction::MoveMainWindowPlaylistRow { .. }
            | GuiShellAction::MoveSelectedMainWindowPlaylistUp
            | GuiShellAction::MoveSelectedMainWindowPlaylistDown
            | GuiShellAction::RemoveSelectedMainWindowPlaylist => {
                self.apply_main_window_action(action)
            }
            GuiShellAction::SelectMenuAction { .. }
            | GuiShellAction::SelectMediaSearchDirectory(_)
            | GuiShellAction::SelectPlugin(_)
            | GuiShellAction::MoveSelectedMediaSearchDirectoryUp
            | GuiShellAction::MoveSelectedMediaSearchDirectoryDown
            | GuiShellAction::RemoveSelectedMediaSearchDirectory
            | GuiShellAction::EditConfigurationText { .. }
            | GuiShellAction::EditConfigurationBool { .. }
            | GuiShellAction::AnnouncePublicServerSelectionChanged(_)
            | GuiShellAction::BeginSavedServerConnect
            | GuiShellAction::CompleteSavedServerConnect
            | GuiShellAction::CancelSavedServerConnect
            | GuiShellAction::BeginSessionDisconnect
            | GuiShellAction::CompleteSessionDisconnect
            | GuiShellAction::CancelSessionDisconnect
            | GuiShellAction::BeginSelectedPublicServerConnect
            | GuiShellAction::CompleteSelectedPublicServerConnect
            | GuiShellAction::BeginPublicServerRefresh
            | GuiShellAction::CompletePublicServerRefresh(_)
            | GuiShellAction::AnnounceCustomPublicServerAdded { .. }
            | GuiShellAction::SelectPublicServer(_)
            | GuiShellAction::AddMediaSearchDirectory(_)
            | GuiShellAction::AnnounceMediaSearchDirectorySelected(_)
            | GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(_)
            | GuiShellAction::BeginMissingMediaSearch
            | GuiShellAction::CompleteMissingMediaSearch(_) => self.apply_service_action(action),
            GuiShellAction::RetryPlayerLaunch
            | GuiShellAction::InstallStreamHelper
            | GuiShellAction::SetPluginEnabled { .. }
            | GuiShellAction::IntegrateStreamHelperDownloader(_)
            | GuiShellAction::IntegrateStreamHelperJsRuntime(_)
            | GuiShellAction::RecheckStreamHelper
            | GuiShellAction::RetryPendingStreamMediaOpen
            | GuiShellAction::OpenStreamHelperInstallLocation
            | GuiShellAction::InstallMediaMatchTools
            | GuiShellAction::ImportMediaMatchFfmpeg(_)
            | GuiShellAction::ImportMediaMatchFfprobe(_)
            | GuiShellAction::RecheckMediaMatchTools
            | GuiShellAction::RebuildMediaMatchIndex
            | GuiShellAction::CancelMediaMatchRebuild
            | GuiShellAction::ClearMediaMatchCache
            | GuiShellAction::OpenMediaMatchInstallLocation
            | GuiShellAction::SetMediaMatchFingerprintingEnabled(_)
            | GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(_)
            | GuiShellAction::SetMediaMatchWireSharingEnabled(_)
            | GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(_)
            | GuiShellAction::SetMediaMatchAutoplayPolicy(_)
            | GuiShellAction::StartPlexAuth
            | GuiShellAction::PollPlexAuth
            | GuiShellAction::RefreshPlexServers
            | GuiShellAction::SelectPlexServer { .. }
            | GuiShellAction::TogglePlexSync(_)
            | GuiShellAction::TogglePlexStreaming(_)
            | GuiShellAction::DisconnectPlex
            | GuiShellAction::ToggleMainWindowPlaybackButtons
            | GuiShellAction::ToggleMainWindowAutoplayControls
            | GuiShellAction::ToggleMainWindowHideEmptyRooms
            | GuiShellAction::ToggleMainWindowRoomChange
            | GuiShellAction::RequestMainWindowUserMediaOpen(_)
            | GuiShellAction::RequestMainWindowUserContainingFolderOpen(_)
            | GuiShellAction::RequestMainWindowUserReady { .. }
            | GuiShellAction::RequestControllerAuth { .. }
            | GuiShellAction::AddTrustedDomain(_)
            | GuiShellAction::JoinMainWindowRoom(_)
            | GuiShellAction::LeaveMainWindowRoom
            | GuiShellAction::SetMainWindowRoom(_)
            | GuiShellAction::ApplyMainWindowRuntimeSnapshot(_)
            | GuiShellAction::ApplyGuiRuntimeSnapshot(_)
            | GuiShellAction::PushChatMessage { .. } => self.apply_misc_action(action),
        }
    }
}
