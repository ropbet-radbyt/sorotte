use super::super::remote_services;
use super::*;

#[allow(
    clippy::large_enum_variant,
    reason = "GUI runtime requests are intentionally centralized until the runtime bridge is split by domain."
)]
#[allow(
    dead_code,
    reason = "This enum is the GUI runtime command vocabulary; feature and smoke targets construct different subsets."
)]
#[derive(Debug, Clone, PartialEq)]
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
    RetryPlayerLaunch,
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
    SendChatMessage(String),
    SeekOffset(f64),
    SeekToPosition(f64),
    AdvancePlaylistIndex,
    SetPlaybackPaused(bool),
    TogglePlaybackPause,
    CompletePendingOperation(GuiPendingCompletionRequest),
    CancelPendingOperation(GuiPendingOperationKind),
}
