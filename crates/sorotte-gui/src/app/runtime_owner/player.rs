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

use sorotte_client_app::app_boundary::state::ClientConfig;
use sorotte_client_app::app_boundary::state::StreamingQualitySuggestionReason;
use sorotte_client_core::{
    CoordinatorPlayerCommand, MediaLoadIntent, MediaTransportKind, PlaybackBarrierTimeoutAction,
    PlayerCommandCause, logical_media_id_for_local_file_update,
};
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCommandId, PlayerCommandProgress,
    PlayerCommandProgressState, PlayerCommandResult, PlayerMediaGeneration, PlayerMediaLoadOutcome,
};
use sorotte_player_mpv::LegacySyncplayOsdKind;

use super::super::media_search_cache::{
    current_media_search_cache_generation, current_unix_time_millis,
    load_persisted_media_search_root_index_at_root, normalized_media_search_root_key,
    persist_media_search_root_index_borrowed_at_root_if_cache_generation,
};
use super::super::runtime_bridge::{GuiSharedPlaylistOpenDispatch, GuiSharedPlaylistOpenItem};
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiAttachedPlayerRuntimeAction, GuiClientCoreChatSessionRuntimeAdapter,
    GuiLocalPlayerUnpauseDecision, GuiOwnedPlayer,
};
use super::super::shell_state::{
    GuiMediaIndexRuntimeSnapshot, GuiMediaSourceProviderId, GuiPlaylistEntryId,
    GuiPlaylistResolutionStep, GuiPlaylistSourcePolicy, GuiPlaylistSourceSelectionOrigin,
    GuiPlaylistSourceStatus, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiStreamHelperHealth, GuiStreamTargetKind, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SorotteGuiShellAppState, browser_is_url, browser_stream_target_kind,
    shuffle_playlist_entries_in_place,
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
mod plex_miss;
mod resolution_attempt;
mod shared_playlist;
mod stream_load;
mod telemetry;

use super::{
    ATTACHED_PLAYER_PAUSE_COMMAND_SUPPRESSION, GuiAttachedMediaSearchBuildProgress,
    GuiAttachedMediaSearchBuildState, GuiAttachedMediaSearchBuildStatus,
    GuiAttachedMediaSearchIndex, GuiAttachedMediaSearchRootIndex,
    GuiAttachedMediaSearchRootRefreshResult, GuiAutomaticMediaResolutionTrigger,
    GuiPendingAttachedMediaResolution, GuiPendingAttachedPlayerPauseCommand,
    GuiPendingAttachedRoomUnpauseObservation, GuiPersistedConfigRuntimeOwner,
    GuiPlaylistLocalOriginBindingOutcome, GuiPlexStreamResolveOutcome,
    GuiPlexStreamResolveWorkerResult, GuiUserMediaTargetResolution,
    GuiUserMediaTargetResolutionSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectedPlaylistMediaSyncOutcome {
    NoChange,
    MatchedCurrentTarget,
    StartedLoading,
}

impl SelectedPlaylistMediaSyncOutcome {
    pub(super) fn selection_started(self) -> bool {
        !matches!(self, Self::NoChange)
    }

    pub(super) fn selection_handoff_ready(self, pending_playlist_reset: bool) -> bool {
        matches!(self, Self::MatchedCurrentTarget) && pending_playlist_reset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StartedMediaLoad {
    pub(super) feedback_message: String,
    pub(super) player_command_id: Option<PlayerCommandId>,
    pub(super) player_media_generation: Option<PlayerMediaGeneration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaylistResolutionAttemptState {
    Resolving,
    Loading,
    Active,
    Failed,
    Superseded,
}

#[derive(Clone)]
pub(super) struct PlaylistResolutionAttempt {
    pub(super) row_id: GuiPlaylistEntryId,
    pub(super) playlist_generation: u64,
    pub(super) target: String,
    pub(super) policy: GuiPlaylistSourcePolicy,
    pub(super) candidate_provider: Option<GuiMediaSourceProviderId>,
    pub(super) candidate: Option<media_resolution::GuiMediaResolutionCandidate>,
    pub(super) player_command_id: Option<PlayerCommandId>,
    pub(super) player_media_generation: Option<PlayerMediaGeneration>,
    pub(super) state: PlaylistResolutionAttemptState,
    pub(super) failed_candidates: Vec<media_resolution::GuiMediaResolutionCandidate>,
    pub(super) failed_candidate_retry_at: Option<Instant>,
    pub(super) fallback_pending: bool,
    pub(super) handoff_pending: bool,
}

impl std::fmt::Debug for PlaylistResolutionAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaylistResolutionAttempt")
            .field("row_id", &self.row_id)
            .field("playlist_generation", &self.playlist_generation)
            .field("target", &sorotte_secret::REDACTED_SECRET)
            .field("policy", &self.policy)
            .field("candidate_provider", &self.candidate_provider)
            .field("candidate", &self.candidate)
            .field("player_command_id", &self.player_command_id)
            .field("player_media_generation", &self.player_media_generation)
            .field("state", &self.state)
            .field("failed_candidate_count", &self.failed_candidates.len())
            .field(
                "failed_candidate_retry_scheduled",
                &self.failed_candidate_retry_at.is_some(),
            )
            .field("fallback_pending", &self.fallback_pending)
            .field("handoff_pending", &self.handoff_pending)
            .finish()
    }
}

impl PlaylistResolutionAttempt {
    fn new(
        row_id: GuiPlaylistEntryId,
        playlist_generation: u64,
        target: String,
        policy: GuiPlaylistSourcePolicy,
    ) -> Self {
        Self {
            row_id,
            playlist_generation,
            target,
            policy,
            candidate_provider: None,
            candidate: None,
            player_command_id: None,
            player_media_generation: None,
            state: PlaylistResolutionAttemptState::Resolving,
            failed_candidates: Vec::new(),
            failed_candidate_retry_at: None,
            fallback_pending: false,
            handoff_pending: false,
        }
    }

    fn matches_scope(
        &self,
        row_id: GuiPlaylistEntryId,
        playlist_generation: u64,
        target: &str,
        policy: GuiPlaylistSourcePolicy,
    ) -> bool {
        self.row_id == row_id
            && self.playlist_generation == playlist_generation
            && self.target == target
            && self.policy == policy
            && self.state != PlaylistResolutionAttemptState::Superseded
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct PlexResolutionMissKey {
    pub(super) row_id: GuiPlaylistEntryId,
    pub(super) playlist_generation: u64,
    pub(super) policy: GuiPlaylistSourcePolicy,
    pub(super) stream_trigger_key: String,
}

impl std::fmt::Debug for PlexResolutionMissKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlexResolutionMissKey")
            .field("row_id", &self.row_id)
            .field("playlist_generation", &self.playlist_generation)
            .field("policy", &self.policy)
            .field("stream_trigger_key", &sorotte_secret::REDACTED_SECRET)
            .finish()
    }
}

#[derive(Clone)]
pub(super) struct PlexMissState {
    pub(super) key: PlexResolutionMissKey,
    pub(super) last_attempt_at: Instant,
    pub(super) next_retry_at: Instant,
    pub(super) attempt_count: u32,
    pub(super) retry_in_flight: bool,
}

impl std::fmt::Debug for PlexMissState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlexMissState")
            .field("key", &self.key)
            .field("last_attempt_at", &self.last_attempt_at)
            .field("next_retry_at", &self.next_retry_at)
            .field("attempt_count", &self.attempt_count)
            .field("retry_in_flight", &self.retry_in_flight)
            .finish()
    }
}
