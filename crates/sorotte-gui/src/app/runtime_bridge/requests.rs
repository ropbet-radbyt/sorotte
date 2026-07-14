use super::super::remote_services;
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiPlexPlaylistJobCancellationReason {
    PickerClosed,
    OperationContextInvalidated,
}

#[allow(
    clippy::large_enum_variant,
    reason = "GUI runtime requests are intentionally centralized until the runtime bridge is split by domain."
)]
#[allow(
    dead_code,
    reason = "This enum is the GUI runtime command vocabulary; feature and smoke targets construct different subsets."
)]
#[derive(Clone, PartialEq)]
pub(in crate::app) enum GuiRuntimeRequest {
    CheckForUpdates {
        language: String,
        update_channel: Option<String>,
        user_initiated: bool,
    },
    DownloadUpdate(remote_services::UpdateCandidate),
    DownloadAndInstallUpdate(remote_services::UpdateCandidate),
    ApplyStagedUpdate(remote_services::StagedUpdate),
    OpenMediaFiles {
        paths: Vec<String>,
        load_into_shared_playlist: bool,
        playlist_insert_slot: Option<usize>,
    },
    ImportSharedPlaylistFile {
        path: String,
        shuffled: bool,
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
        password: SecretValue,
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
    ResolvePlaylistSource {
        index: usize,
        provider_id: GuiMediaSourceProviderId,
    },
    RetryPlayerLaunch,
    SetPluginEnabled {
        plugin: GuiPluginSelection,
        enabled: bool,
    },
    InstallStreamHelper,
    IntegrateStreamHelperDownloader(String),
    IntegrateStreamHelperJsRuntime(String),
    OpenStreamHelperInstallLocation,
    RecheckStreamHelper,
    RetryPendingStreamMediaOpen,
    InstallMediaMatchTools,
    ImportMediaMatchFfmpeg(String),
    ImportMediaMatchFfprobe(String),
    OpenMediaMatchInstallLocation,
    RecheckMediaMatchTools,
    RebuildMediaMatchIndex,
    CancelMediaMatchRebuild,
    ClearMediaMatchCache,
    SetMediaMatchFingerprintingEnabled(bool),
    SetMediaMatchBackgroundWarmupEnabled(bool),
    SetMediaMatchWireSharingEnabled(bool),
    SetMediaMatchRuntimeToleranceEnabled(bool),
    SetMediaMatchAutoplayPolicy(sorotte_media_match::MediaMatchAutoplayPolicy),
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
    SearchSelectedPlexServerMedia {
        query: String,
    },
    ResolvePlexPlaylistItem {
        rating_key: String,
    },
    CancelPlexPlaylistJobs {
        reason: GuiPlexPlaylistJobCancellationReason,
    },
    SendChatMessage(String),
    SeekOffset(f64),
    SeekToPosition(f64),
    KeepWaitingForSeekPreparation,
    CancelSeekPreparation,
    JoinNearestBufferedSeekPreparation,
    AdvancePlaylistIndex,
    SetPlaybackPaused(bool),
    TogglePlaybackPause,
    CompletePendingOperation(GuiPendingCompletionRequest),
    CancelPendingOperation(GuiPendingOperationKind),
}

impl std::fmt::Debug for GuiRuntimeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
                playlist_insert_slot,
            } => formatter
                .debug_struct("OpenMediaFiles")
                .field("paths", &sorotte_secret::REDACTED_SECRET)
                .field("path_count", &paths.len())
                .field("load_into_shared_playlist", load_into_shared_playlist)
                .field("playlist_insert_slot", playlist_insert_slot)
                .finish(),
            Self::ImportSharedPlaylistFile { shuffled, .. } => formatter
                .debug_struct("ImportSharedPlaylistFile")
                .field("path", &sorotte_secret::REDACTED_SECRET)
                .field("shuffled", shuffled)
                .finish(),
            Self::OpenMainWindowUserMedia(_) => formatter
                .debug_tuple("OpenMainWindowUserMedia")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::OpenMainWindowUserContainingFolder(_) => formatter
                .debug_tuple("OpenMainWindowUserContainingFolder")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::SetRoom(_) => formatter
                .debug_tuple("SetRoom")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::RequestControllerAuth { password, .. } => formatter
                .debug_struct("RequestControllerAuth")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
            Self::QueuePlaylistEntry {
                select_after_queue, ..
            } => formatter
                .debug_struct("QueuePlaylistEntry")
                .field("entry", &sorotte_secret::REDACTED_SECRET)
                .field("select_after_queue", select_after_queue)
                .finish(),
            Self::ReplacePlaylist {
                files,
                selected_index,
            } => formatter
                .debug_struct("ReplacePlaylist")
                .field("files", &sorotte_secret::REDACTED_SECRET)
                .field("file_count", &files.len())
                .field("selected_index", selected_index)
                .finish(),
            Self::SelectPlexServer {
                machine_identifier, ..
            } => formatter
                .debug_struct("SelectPlexServer")
                .field("machine_identifier", machine_identifier)
                .field("uri", &sorotte_secret::REDACTED_SECRET)
                .finish(),
            _ => formatter
                .debug_tuple("GuiRuntimeRequest")
                .field(&std::mem::discriminant(self))
                .finish(),
        }
    }
}

#[cfg(test)]
mod media_target_debug_tests {
    use super::*;

    #[test]
    fn runtime_media_requests_redact_tokenized_targets() {
        let secret = "https://media.example/video?token=runtime-request-canary";
        let requests = [
            GuiRuntimeRequest::OpenMediaFiles {
                paths: vec![secret.to_owned()],
                load_into_shared_playlist: false,
                playlist_insert_slot: None,
            },
            GuiRuntimeRequest::ImportSharedPlaylistFile {
                path: secret.to_owned(),
                shuffled: false,
            },
            GuiRuntimeRequest::OpenMainWindowUserMedia(secret.to_owned()),
            GuiRuntimeRequest::OpenMainWindowUserContainingFolder(secret.to_owned()),
            GuiRuntimeRequest::QueuePlaylistEntry {
                entry: secret.to_owned(),
                select_after_queue: true,
            },
        ];

        let debug = format!("{requests:?}");
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains("runtime-request-canary"));
    }
}
