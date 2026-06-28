use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, TryRecvError},
    },
    time::{Duration, Instant},
};

use sorotte_player_api::{LocalFileUpdate, PlayerAdapter, PlayerMediaLoadOutcome};
use sorotte_player_mpv::LegacySyncplayOsdKind;

use super::super::media_search_cache::{
    current_media_search_cache_generation, current_unix_time_millis,
    load_persisted_media_search_root_index_at_root, normalized_media_search_root_key,
    persist_media_search_root_index_borrowed_at_root_if_cache_generation,
};
use super::super::runtime_bridge::GuiSharedPlaylistOpenDispatch;
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiAttachedPlayerRuntimeAction, GuiClientCoreChatSessionRuntimeAdapter,
    GuiLocalPlayerUnpauseDecision, GuiOwnedPlayer,
};
use super::super::shell_state::{
    GuiMediaIndexRuntimeSnapshot, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiStreamHelperHealth, GuiStreamTargetKind, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SorotteGuiShellAppState, browser_is_url, browser_stream_target_kind,
};
use super::super::startup_support::env_trimmed;
use super::super::support::{
    normalized_editable_text, shared_playlist_entry_for_media_path, system_time_seconds,
};
mod attached_sync;
mod detached_session;
mod media_index;
mod media_open;
mod media_resolution;
mod media_search;
mod messages;
mod player_state;
mod playlist_sync;
mod shared_playlist;
mod stream_load;
mod telemetry;

use super::{
    ATTACHED_PLAYER_PAUSE_COMMAND_SUPPRESSION, GuiAttachedMediaSearchBuildProgress,
    GuiAttachedMediaSearchBuildState, GuiAttachedMediaSearchBuildStatus,
    GuiAttachedMediaSearchIndex, GuiAttachedMediaSearchRootIndex,
    GuiAttachedMediaSearchRootRefreshResult, GuiAutomaticMediaResolutionTrigger,
    GuiPendingAttachedMediaResolution, GuiPendingAttachedPlayerPauseCommand,
    GuiPersistedConfigRuntimeOwner, GuiPlexStreamResolveOutcome, GuiPlexStreamResolveWorkerResult,
    GuiUserMediaTargetResolution, GuiUserMediaTargetResolutionSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectedPlaylistMediaSyncOutcome {
    NoChange,
    MatchedCurrentTarget,
    OpenedNewMedia,
}

impl SelectedPlaylistMediaSyncOutcome {
    pub(super) fn selection_ready(self) -> bool {
        !matches!(self, Self::NoChange)
    }

    pub(super) fn selection_handoff_ready(self, pending_playlist_reset: bool) -> bool {
        matches!(self, Self::OpenedNewMedia)
            || (matches!(self, Self::MatchedCurrentTarget) && pending_playlist_reset)
    }
}
