use super::*;

#[allow(
    clippy::large_enum_variant,
    reason = "The shell action vocabulary is still centralized while reducer domains are being split incrementally."
)]
#[allow(
    dead_code,
    reason = "The action enum is the full GUI command vocabulary; feature and smoke paths construct subsets."
)]
#[derive(Clone, PartialEq)]
pub(in crate::app) enum GuiShellAction {
    SwitchView(GuiShellView),
    SelectConfigurationTab(GuiConfigurationTab),
    OpenModal(GuiShellModal),
    CloseModal,
    DismissUpdateNotice,
    BeginUpdateCheck {
        user_initiated: bool,
    },
    ApplyUpdateCheckResult(remote_services::LegacyUpdateCheckResult),
    ActivateUpdateIndicator,
    BeginUpdateDownload,
    BeginUpdateInstall,
    ApplyUpdateDownloadResult(remote_services::UpdateDownloadResult),
    BeginStagedUpdateApply,
    ApplyStagedUpdateLaunchResult(remote_services::UpdateApplyLaunchResult),
    ApplyStartupPublicServerCache(Vec<(String, String)>),
    TrustTlsCertificatePrompt,
    RejectTlsCertificatePrompt,
    TriggerSelectedMenuAction,
    SetPluginEnabled {
        plugin: GuiPluginSelection,
        enabled: bool,
    },
    AnnounceTlsCertificatePromptRequired,
    AnnounceUpdateNoticeAvailable,
    AnnounceAboutDialogRequested,
    AnnounceHelpRequested,
    ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot),
    ApplyGuiFeedbackRuntimeSnapshot(GuiFeedbackRuntimeSnapshot),
    ApplyGuiErrorRuntimeSnapshot(GuiErrorRuntimeSnapshot),
    ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot),
    ApplyGuiMediaIndexRuntimeSnapshot(GuiMediaIndexRuntimeSnapshot),
    ApplyGuiPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot),
    ApplyGuiSeekPreparationRuntimeSnapshot(GuiSeekPreparationRuntimeSnapshot),
    ApplyGuiStreamHelperRuntimeSnapshot(GuiStreamHelperRuntimeSnapshot),
    ApplyGuiStreamHelperRemediationRuntimeSnapshot(GuiStreamHelperRemediationRuntimeSnapshot),
    ApplyGuiMediaMatchRuntimeSnapshot(GuiMediaMatchRuntimeSnapshot),
    ApplyGuiMediaMatchRemediationRuntimeSnapshot(GuiMediaMatchRemediationRuntimeSnapshot),
    ApplyGuiPlexRuntimeSnapshot(GuiPlexRuntimeSnapshot),
    ApplyGuiInteractionRuntimeSnapshot(GuiInteractionRuntimeSnapshot),
    ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot),
    ApplyGuiConfigurationDraftRuntimeSnapshot(GuiConfigurationDraftRuntimeSnapshot),
    ApplyGuiSavedConfigurationRuntimeSnapshot(GuiSavedConfigurationRuntimeSnapshot),
    ApplyGuiConfigurationRuntimeSnapshot(GuiConfigurationRuntimeSnapshot),
    ApplyGuiConfigStorageRuntimeSnapshot(GuiConfigStorageRuntimeSnapshot),
    BeginConfigurationSave,
    CompleteConfigurationSave(StoredClientSettingsMvp),
    CancelConfigurationSave,
    BeginConfigurationReset,
    CompleteConfigurationReset(StoredClientSettingsMvp),
    CancelConfigurationReset,
    BeginConfigurationReload,
    CompleteConfigurationReload(StoredClientSettingsMvp),
    CancelConfigurationReload,
    BeginClearGuiData,
    CompleteClearGuiData,
    CancelClearGuiData,
    BeginConfigStorageRootChange(String),
    BeginConfigStorageDefaultReset,
    CompleteConfigStorageRootChange {
        snapshot: GuiConfigStorageRuntimeSnapshot,
        settings: StoredClientSettingsMvp,
    },
    CancelConfigStorageRootChange,
    BeginPendingOperation(GuiPendingOperationKind),
    CompletePendingOperation,
    CancelPendingOperation,
    FocusConfigurationControl {
        section: &'static str,
        label: &'static str,
    },
    ActivateFocusedConfigurationControl,
    ClearConfigurationControlFocus,
    BeginAddPublicServer,
    BeginEditSelectedPublicServer,
    UpdatePublicServerEditLabel(String),
    UpdatePublicServerEditAddress(String),
    CommitPublicServerEdit,
    CancelPublicServerEdit,
    RemoveSelectedPublicServer,
    BeginEditSelectedMainWindowUser,
    UpdateMainWindowUserEdit(String),
    CommitMainWindowUserEdit,
    CancelMainWindowUserEdit,
    PushTransientNotification {
        level: GuiTransientNotificationLevel,
        message: String,
    },
    DismissTransientNotification(usize),
    DismissSetupAlert,
    ClearTransientNotifications,
    BeginConfigurationTextEdit {
        section: &'static str,
        label: &'static str,
    },
    UpdateConfigurationTextEdit(GuiConfigurationTextValue),
    CommitConfigurationTextEdit,
    CancelConfigurationTextEdit,
    BeginRoomHistoryEdit,
    UpdateRoomHistoryEdit(String),
    CommitRoomHistoryEdit,
    CancelRoomHistoryEdit,
    BeginSharedPlaylistTextEdit,
    UpdateSharedPlaylistTextEdit(String),
    CancelSharedPlaylistTextEdit,
    BeginSharedPlaylistUrlEdit,
    UpdateSharedPlaylistUrlEdit(String),
    CancelSharedPlaylistUrlEdit,
    BeginPlexPlaylistSearch,
    UpdatePlexPlaylistSearchQuery(String),
    SubmitPlexPlaylistSearch {
        query: String,
    },
    CompletePlexPlaylistSearch {
        query: String,
        results: Vec<GuiPlexPlaylistSearchResult>,
        error: Option<String>,
    },
    SelectPlexPlaylistSearchResult(usize),
    AddSelectedPlexPlaylistSearchResult,
    CompletePlexPlaylistItemResolve {
        rating_key: String,
        error: Option<String>,
    },
    CancelPlexPlaylistSearch,
    BeginMediaUrlEdit,
    UpdateMediaUrlEdit(String),
    CancelMediaUrlEdit,
    BeginCreateControlledRoomEdit,
    UpdateCreateControlledRoomEdit(String),
    CancelCreateControlledRoomEdit,
    BeginControllerAuthEdit,
    UpdateControllerAuthPasswordEdit(SecretValue),
    CancelControllerAuthEdit,
    UpdateNewMainWindowUserDraft(String),
    CommitNewMainWindowUser,
    AppendSharedPlaylistEntries(Vec<String>),
    ReplaceSharedPlaylistEntries(Vec<String>),
    LoadSharedPlaylistFromFile {
        path: String,
        entries: Vec<String>,
        shuffled: bool,
    },
    SaveSharedPlaylistToFile(String),
    SelectMainWindowUser(usize),
    AddMainWindowUser(String),
    AnnounceMainWindowUserJoined(String),
    AnnounceSelectedMainWindowUserRenamed(String),
    AnnounceSelectedMainWindowUserLeft,
    BeginPlaybackPause,
    BeginPlaybackResume,
    BeginPlaybackPauseToggle,
    CompletePlaybackPauseState(bool),
    CancelPlaybackPauseState,
    CompletePlaybackPauseToggle,
    CancelPlaybackPauseToggle,
    AnnouncePlaybackPaused,
    AnnouncePlaybackResumed,
    RequestSeekPrompt,
    RequestOffsetPrompt,
    RequestPlaybackUndoSeek,
    RequestSeekPreparationKeepWaiting,
    RequestSeekPreparationCancel,
    RequestSeekPreparationJoinNearest,
    AnnounceLocalUserReady,
    AnnounceLocalUserNotReady,
    AnnounceAutoplayState(bool),
    AnnounceAutoplayThreshold(usize),
    AnnounceSharedPlaylistLoaded(Vec<String>),
    AnnounceSharedPlaylistEntryAdded(String),
    AnnounceSharedPlaylistSelectionChanged(usize),
    AnnounceSelectedSharedPlaylistEntryRemoved,
    UndoSharedPlaylistChange,
    ShuffleRemainingSharedPlaylist,
    ShuffleEntireSharedPlaylist,
    BeginLocalChatSend(String),
    CompleteLocalChatSend,
    CancelLocalChatSend,
    AnnounceRemoteChatMessage {
        sender: String,
        message: String,
    },
    AnnounceSystemChatEvent(String),
    AnnounceControlledRoomCreated {
        room: String,
        password: SecretValue,
    },
    ToggleSelectedMainWindowUserReady,
    ToggleSelectedMainWindowUserController,
    RemoveSelectedMainWindowUser,
    SelectMainWindowPlaylist(usize),
    ActivateMainWindowPlaylist(usize),
    SelectMainWindowPlaylistSource {
        index: usize,
        provider_id: GuiMediaSourceProviderId,
    },
    SelectMainWindowPlaylistDefaultSource {
        source_id: GuiPlaylistDefaultSourceId,
    },
    MoveMainWindowPlaylistRow {
        from_index: usize,
        to_index: usize,
    },
    MoveSelectedMainWindowPlaylistUp,
    MoveSelectedMainWindowPlaylistDown,
    RemoveSelectedMainWindowPlaylist,
    SelectMenuAction {
        section_index: usize,
        action_index: usize,
    },
    SelectMediaSearchDirectory(usize),
    SelectPlugin(GuiPluginSelection),
    MoveSelectedMediaSearchDirectoryUp,
    MoveSelectedMediaSearchDirectoryDown,
    RemoveSelectedMediaSearchDirectory,
    EditConfigurationText {
        section: &'static str,
        label: &'static str,
        value: GuiConfigurationTextValue,
    },
    EditConfigurationBool {
        section: &'static str,
        label: &'static str,
        value: bool,
    },
    AnnouncePublicServerSelectionChanged(usize),
    BeginSavedServerConnect,
    CompleteSavedServerConnect,
    CancelSavedServerConnect,
    BeginSessionDisconnect,
    CompleteSessionDisconnect,
    CancelSessionDisconnect,
    BeginSelectedPublicServerConnect,
    CompleteSelectedPublicServerConnect,
    BeginPublicServerRefresh,
    CompletePublicServerRefresh(Vec<(String, String)>),
    AnnounceCustomPublicServerAdded {
        label: String,
        address: String,
    },
    SelectPublicServer(usize),
    AddMediaSearchDirectory(String),
    AnnounceMediaSearchDirectorySelected(usize),
    AnnounceMediaSearchDirectoryBrowsed(String),
    BeginMissingMediaSearch,
    CompleteMissingMediaSearch(Option<String>),
    RetryPlayerLaunch,
    InstallStreamHelper,
    IntegrateStreamHelperDownloader(String),
    IntegrateStreamHelperJsRuntime(String),
    RecheckStreamHelper,
    RetryPendingStreamMediaOpen,
    OpenStreamHelperInstallLocation,
    InstallMediaMatchTools,
    ImportMediaMatchFfmpeg(String),
    ImportMediaMatchFfprobe(String),
    RecheckMediaMatchTools,
    RebuildMediaMatchIndex,
    CancelMediaMatchRebuild,
    ClearMediaMatchCache,
    OpenMediaMatchInstallLocation,
    SetMediaMatchFingerprintingEnabled(bool),
    SetMediaMatchBackgroundWarmupEnabled(bool),
    SetMediaMatchWireSharingEnabled(bool),
    SetMediaMatchRuntimeToleranceEnabled(bool),
    SetMediaMatchAutoplayPolicy(MediaMatchAutoplayPolicy),
    StartPlexAuth,
    PollPlexAuth,
    RefreshPlexServers,
    SelectPlexServer {
        machine_identifier: String,
        uri: String,
    },
    TogglePlexSync(bool),
    TogglePlexStreaming(bool),
    DisconnectPlex,
    ToggleMainWindowPlaybackButtons,
    ToggleMainWindowAutoplayControls,
    ToggleMainWindowHideEmptyRooms,
    ToggleMainWindowRoomChange,
    RequestMainWindowUserMediaOpen(String),
    RequestMainWindowUserContainingFolderOpen(String),
    RequestMainWindowUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: SecretValue,
    },
    AddTrustedDomain(String),
    JoinMainWindowRoom(String),
    LeaveMainWindowRoom,
    SetMainWindowRoom(String),
    ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot),
    ApplyGuiRuntimeSnapshot(SorotteGuiRuntimeSnapshot),
    PushChatMessage {
        sender: String,
        message: String,
    },
}

impl std::fmt::Debug for GuiShellAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpdateConfigurationTextEdit(value) => formatter
                .debug_tuple("UpdateConfigurationTextEdit")
                .field(value)
                .finish(),
            Self::EditConfigurationText {
                section,
                label,
                value,
            } => formatter
                .debug_struct("EditConfigurationText")
                .field("section", section)
                .field("label", label)
                .field("value", value)
                .finish(),
            Self::UpdateControllerAuthPasswordEdit(password) => formatter
                .debug_tuple("UpdateControllerAuthPasswordEdit")
                .field(password)
                .finish(),
            Self::AnnounceControlledRoomCreated { password, .. } => formatter
                .debug_struct("AnnounceControlledRoomCreated")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
            Self::RequestControllerAuth { password, .. } => formatter
                .debug_struct("RequestControllerAuth")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
            Self::RequestMainWindowUserMediaOpen(_) => formatter
                .debug_tuple("RequestMainWindowUserMediaOpen")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::RequestMainWindowUserContainingFolderOpen(_) => formatter
                .debug_tuple("RequestMainWindowUserContainingFolderOpen")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            _ => formatter
                .debug_tuple("GuiShellAction")
                .field(&std::mem::discriminant(self))
                .finish(),
        }
    }
}

#[cfg(test)]
mod media_target_debug_tests {
    use super::*;

    #[test]
    fn shell_media_actions_redact_tokenized_targets() {
        let secret = "https://media.example/video?token=shell-action-canary";
        let actions = [
            GuiShellAction::RequestMainWindowUserMediaOpen(secret.to_owned()),
            GuiShellAction::RequestMainWindowUserContainingFolderOpen(secret.to_owned()),
        ];

        let debug = format!("{actions:?}");
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains("shell-action-canary"));
    }
}
