use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, TryRecvError},
    },
    time::{Duration, Instant},
};

use syncplay_player_api::{LocalFileUpdate, PlayerAdapter};
use syncplay_player_mpv::LegacySyncplayOsdKind;

use super::super::media_search_cache::{
    current_unix_time_millis, load_persisted_media_search_root_index_at_root,
    normalized_media_search_root_key, persist_media_search_root_index_borrowed_at_root,
};
use super::super::runtime_bridge::GuiSharedPlaylistOpenDispatch;
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiAttachedPlayerRuntimeAction, GuiClientCoreChatSessionRuntimeAdapter,
    GuiLocalPlayerUnpauseDecision, GuiOwnedPlayer,
};
use super::super::shell_state::{
    GuiMediaIndexRuntimeSnapshot, GuiShellAction, GuiShellView, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SyncplayGuiShellAppState, browser_is_url,
};
use super::super::startup_support::env_trimmed;
use super::super::support::{
    normalized_editable_text, shared_playlist_entry_for_media_path, system_time_seconds,
};
use super::{
    GuiAttachedMediaSearchBuildProgress, GuiAttachedMediaSearchBuildState,
    GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
    GuiAttachedMediaSearchRootIndex, GuiAttachedMediaSearchRootRefreshResult,
    GuiAutomaticMediaResolutionTrigger, GuiMediaIndexJobId, GuiPendingAttachedMediaResolution,
    GuiPersistedConfigRuntimeOwner, GuiUserMediaTargetResolution,
};

const LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT: f64 = 20.0;
const LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT: f64 = 30.0;
const MEDIA_INDEX_PROGRESS_INTERVAL_MILLIS_DEFAULT: u64 = 250;
const PLAYLIST_LOAD_NEXT_FILE_MINIMUM_LENGTH_SECONDS: f64 = 10.0;
const PLAYLIST_LOAD_NEXT_FILE_TIME_FROM_END_THRESHOLD_SECONDS: f64 = 5.0;

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

impl GuiPersistedConfigRuntimeOwner {
    fn selected_opened_entry_offset(
        selected_playlist_index: Option<usize>,
        opened_entry_count: usize,
        playlist_insert_slot: Option<usize>,
    ) -> Option<usize> {
        let selected_index = selected_playlist_index?;
        if opened_entry_count == 0 {
            return None;
        }
        match playlist_insert_slot {
            Some(insert_slot) => selected_index
                .checked_sub(insert_slot)
                .filter(|offset| *offset < opened_entry_count),
            None => Some(selected_index).filter(|offset| *offset < opened_entry_count),
        }
    }

    pub(crate) fn sync_pending_local_attached_pause_override_from_session(&mut self) {
        let session_pause_state = self
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state());
        let room_pause_state = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playstate_for_attached_player_sync())
            .and_then(|playstate| playstate.paused);
        self.pending_local_attached_pause_override = match (session_pause_state, room_pause_state) {
            (Some(session_pause_state), Some(room_pause_state))
                if room_pause_state != session_pause_state =>
            {
                Some(session_pause_state)
            }
            _ => None,
        };
    }

    pub(super) fn open_media_unavailable_message_impl(&self, selected_paths: &[String]) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media requires a playback runtime connection; the selected file was not opened."
                .to_owned()
        } else {
            format!(
                "Opening media requires a playback runtime connection; {} selected files were not opened.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_open_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                .to_owned()
        } else {
            format!(
                "Opening media into the shared playlist requires a session or playback runtime connection; {} selected files were not opened or queued.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_session_unavailable_message_impl(&self) -> String {
        "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
            .to_owned()
    }

    pub(super) fn shared_playlist_control_unavailable_message_impl(&self) -> String {
        "Shared playlist control is unavailable for the active room; the selected media was not added to the room playlist or opened in the attached player."
            .to_owned()
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        let lower_path = path.to_ascii_lowercase();
        if !(lower_path.ends_with(".txt")
            || lower_path.ends_with(".m3u")
            || lower_path.ends_with(".m3u8"))
        {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = contents
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    pub(super) fn shared_playlist_open_dispatch_for_paths_impl(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| shared_playlist_entry_for_media_path(path))
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
            imported_from_file: false,
        })
    }

    fn shared_playlist_open_success_message(
        dispatch: &GuiSharedPlaylistOpenDispatch,
        entry_count: usize,
    ) -> String {
        if dispatch.imported_from_file {
            if entry_count == 1 {
                "Imported 1 entry into the shared playlist.".to_owned()
            } else {
                format!("Imported {entry_count} entries into the shared playlist.")
            }
        } else if entry_count == 1 {
            "Loaded 1 selected media entry into the shared playlist.".to_owned()
        } else {
            format!("Loaded {entry_count} selected media entries into the shared playlist.")
        }
    }

    pub(super) fn seek_unavailable_message_impl(&self, offset_seconds: f64) -> String {
        let base = format!(
            "Playback seek requires a playback runtime connection; the {offset_seconds} second request was not applied."
        );
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn toggle_pause_unavailable_message_impl(&self) -> String {
        let base =
            "Playback toggle requires a playback runtime connection; the pause request was not applied."
                .to_owned();
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn send_chat_unavailable_message_impl(&self) -> String {
        "Chat sending requires a session runtime connection; the message was not sent.".to_owned()
    }

    pub(super) fn push_player_success_impl(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn push_player_error_impl(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn open_media_files_through_attached_player_result_impl(
        &mut self,
        paths: &[String],
    ) -> Option<Result<String, String>> {
        if paths.is_empty() || self.player.is_none() {
            return None;
        }

        let selected_path = paths[0].clone();
        let (player_name, open_result) = {
            let player = self.player.as_mut().expect("player should exist");
            (player.name(), player.open_file(&selected_path))
        };
        Some(match open_result {
            Ok(()) => {
                self.player_local_file =
                    Some(Self::placeholder_local_file_for_path(&selected_path));
                self.player_local_file_placeholder = true;
                self.player_position_seconds = Some(0.0);
                self.refresh_player_state_impl();
                if let Some(session) = self.session.as_mut() {
                    let _ = session.mark_local_media_opened_not_ready();
                }
                if paths.len() == 1 {
                    Ok(format!(
                        "Opened media file through the attached {player_name} player: {selected_path}."
                    ))
                } else {
                    Ok(format!(
                        "Opened the first selected media file through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                        paths.len() - 1
                    ))
                }
            }
            Err(error) => Err(format!(
                "Opening media through the attached {player_name} player failed: {error}"
            )),
        })
    }

    pub(super) fn open_media_files_through_attached_player_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        match self.open_media_files_through_attached_player_result_impl(&paths) {
            Some(Ok(message)) => Self::push_player_success_impl(handle, message),
            Some(Err(message)) => Self::push_player_error_impl(handle, message),
            None => {}
        }
    }

    fn playlist_target_for_index(state: &SyncplayGuiShellAppState, index: usize) -> Option<String> {
        if !state.main_window.shared_playlist_enabled {
            return None;
        }

        state
            .main_window
            .playlist
            .get(index)
            .and_then(|target| normalized_editable_text(&target.label))
    }

    fn current_shared_playlist_target(&self, state: &SyncplayGuiShellAppState) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index())
            .and_then(|index| Self::playlist_target_for_index(state, index))
            .or_else(|| {
                self.active_shared_playlist_index
                    .and_then(|index| Self::playlist_target_for_index(state, index))
            })
    }

    pub(super) fn current_player_matches_media_target(&self, target: &str) -> bool {
        let Some(local_file) = self.player_local_file.as_ref() else {
            return false;
        };

        if let Some(path) = local_file.path.as_deref()
            && ((cfg!(windows) && path.eq_ignore_ascii_case(target))
                || (!cfg!(windows) && path == target))
        {
            return true;
        }

        let target_name = if browser_is_url(target) {
            Some(target)
        } else {
            Path::new(target).file_name().and_then(|name| name.to_str())
        };
        target_name.is_some_and(|target_name| {
            if cfg!(windows) {
                local_file.name.eq_ignore_ascii_case(target_name)
            } else {
                local_file.name == target_name
            }
        })
    }

    fn local_file_identity_matches(current: &LocalFileUpdate, next: &LocalFileUpdate) -> bool {
        let current_path = current
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let next_path = next
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        if let (Some(current_path), Some(next_path)) = (current_path, next_path) {
            return if cfg!(windows) {
                current_path.eq_ignore_ascii_case(next_path)
            } else {
                current_path == next_path
            };
        }

        let current_name = current.name.trim();
        let next_name = next.name.trim();
        if current_name.is_empty() || next_name.is_empty() {
            return false;
        }

        if cfg!(windows) {
            current_name.eq_ignore_ascii_case(next_name)
        } else {
            current_name == next_name
        }
    }

    fn local_file_update_replaces_current_file(
        current: Option<&LocalFileUpdate>,
        next: &LocalFileUpdate,
    ) -> bool {
        match current {
            Some(current) => !Self::local_file_identity_matches(current, next),
            None => true,
        }
    }

    pub(super) fn global_position_seconds_from_player_position_impl(
        &self,
        player_position_seconds: f64,
    ) -> f64 {
        player_position_seconds - self.user_offset_seconds
    }

    pub(super) fn player_target_position_seconds_for_global_position_impl(
        &self,
        global_position_seconds: f64,
    ) -> f64 {
        (global_position_seconds + self.user_offset_seconds).max(0.0)
    }

    fn current_player_file_duration_seconds(&self) -> Option<f64> {
        self.player_local_file
            .as_ref()
            .and_then(|local_file| local_file.duration_seconds)
            .filter(|duration_seconds| duration_seconds.is_finite() && *duration_seconds >= 0.0)
    }

    fn clamp_player_position_to_file_duration(&mut self) {
        let Some(duration_seconds) = self.current_player_file_duration_seconds() else {
            return;
        };
        let Some(position_seconds) = self
            .player_position_seconds
            .filter(|position_seconds| position_seconds.is_finite())
        else {
            return;
        };
        self.player_position_seconds = Some(position_seconds.clamp(0.0, duration_seconds));
    }

    pub(in super::super) fn advance_playlist_index_for_attached_player_impl(
        &mut self,
    ) -> Result<(), String> {
        let attached_player_actions = {
            let Some(session) = self.session.as_mut() else {
                return Err(
                    "Advancing the shared playlist requires an active session runtime.".to_owned(),
                );
            };
            session.advance_playlist_index_attached_player_actions()?
        };
        if attached_player_actions.is_empty() {
            let Some(session) = self.session.as_mut() else {
                return Err(
                    "Advancing the shared playlist requires an active session runtime.".to_owned(),
                );
            };
            return session.advance_playlist_index();
        }

        for action in attached_player_actions {
            match action {
                GuiAttachedPlayerRuntimeAction::Paused(paused) => {
                    if let Some(player) = self.player.as_mut() {
                        player.set_paused(paused).map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance pause dispatch failed: {error}"
                            )
                        })?;
                    }
                    self.player_paused = Some(paused);
                }
                GuiAttachedPlayerRuntimeAction::Position(position_seconds) => {
                    let player_target_position_seconds = self
                        .player_target_position_seconds_for_global_position_impl(position_seconds);
                    if let Some(player) = self.player.as_mut() {
                        player
                            .set_position(player_target_position_seconds)
                            .map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance seek dispatch failed: {error}"
                            )
                            })?;
                    }
                    self.player_position_seconds = Some(position_seconds);
                    self.clamp_player_position_to_file_duration();
                }
                GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate) => {
                    if let Some(player) = self.player.as_mut() {
                        player.set_playback_rate(playback_rate).map_err(|error| {
                            format!(
                                "Attached player shared-playlist advance playback-rate dispatch failed: {error}"
                            )
                        })?;
                    }
                }
            }
        }

        if let Some(session) = self.session.as_mut() {
            session
                .sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        }
        Ok(())
    }

    pub(crate) fn take_playlist_auto_advance_eof_trigger_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        playlist_control_available: bool,
        can_auto_advance_to_next_playlist_item: bool,
    ) -> bool {
        let should_trigger = state.main_window.shared_playlist_enabled
            && playlist_control_available
            && can_auto_advance_to_next_playlist_item
            && self.player_paused == Some(true)
            && self
                .current_player_file_duration_seconds()
                .filter(|duration_seconds| {
                    *duration_seconds > PLAYLIST_LOAD_NEXT_FILE_MINIMUM_LENGTH_SECONDS
                })
                .zip(
                    self.player_position_seconds
                        .filter(|position_seconds| position_seconds.is_finite()),
                )
                .is_some_and(|(duration_seconds, position_seconds)| {
                    (position_seconds - duration_seconds).abs()
                        < PLAYLIST_LOAD_NEXT_FILE_TIME_FROM_END_THRESHOLD_SECONDS
                });
        let trigger = should_trigger && !self.playlist_auto_advance_eof_latched;
        self.playlist_auto_advance_eof_latched = should_trigger;
        trigger
    }

    fn quick_existing_media_target_path(target: &Path) -> Option<String> {
        target
            .is_file()
            .then(|| target.to_string_lossy().into_owned())
    }

    fn quick_resolve_main_window_user_media_target(
        &self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<Option<String>, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(None);
        };
        if browser_is_url(&target) {
            return Ok(Some(target.to_owned()));
        }

        let target_path = Path::new(&target);
        if let Some(path) = Self::quick_existing_media_target_path(target_path) {
            return Ok(Some(path));
        }

        let target_file_name = target_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty());

        if let Some(local_path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
        {
            let local_path = Path::new(local_path);
            let matches_local_file = local_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(&target));
            if matches_local_file && local_path.is_file() {
                return Ok(Some(local_path.to_string_lossy().into_owned()));
            }
            if let Some(parent) = local_path.parent() {
                if let Some(path) = Self::quick_existing_media_target_path(&parent.join(&target)) {
                    return Ok(Some(path));
                }
                if let Some(file_name) = target_file_name
                    && let Some(path) =
                        Self::quick_existing_media_target_path(&parent.join(file_name))
                {
                    return Ok(Some(path));
                }
            }
        }

        let settings = state.configuration.to_stored_settings();
        for directory in settings.media_search_directories.unwrap_or_default() {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            let root = Path::new(trimmed);
            if let Some(path) = Self::quick_existing_media_target_path(&root.join(&target)) {
                return Ok(Some(path));
            }
            if let Some(file_name) = target_file_name
                && let Some(path) = Self::quick_existing_media_target_path(&root.join(file_name))
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn automatic_media_search_roots(&self, state: &SyncplayGuiShellAppState) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let mut seen = BTreeSet::new();

        let mut push_root = |path: &Path| {
            if !path.is_dir() {
                return;
            }
            let key = normalized_media_search_root_key(path);
            if seen.insert(key) {
                roots.push(path.to_path_buf());
            }
        };

        if let Some(local_path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .map(PathBuf::from)
            && let Some(parent) = local_path.parent()
        {
            push_root(parent);
        }

        let settings = state.configuration.to_stored_settings();
        for directory in settings.media_search_directories.unwrap_or_default() {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            push_root(Path::new(trimmed));
        }

        roots
    }

    fn automatic_media_search_root_keys(search_roots: &[PathBuf]) -> Vec<String> {
        search_roots
            .iter()
            .map(|path| normalized_media_search_root_key(path))
            .collect()
    }

    fn next_attached_media_search_job_id(&mut self) -> GuiMediaIndexJobId {
        self.attached_media_search_job_sequence =
            self.attached_media_search_job_sequence.wrapping_add(1);
        GuiMediaIndexJobId(self.attached_media_search_job_sequence)
    }

    fn media_index_progress_interval() -> Duration {
        env_trimmed("SYNCPLAY_GUI_MEDIA_INDEX_PROGRESS_INTERVAL_MS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value != 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(MEDIA_INDEX_PROGRESS_INTERVAL_MILLIS_DEFAULT))
    }

    fn set_attached_media_search_build_state(
        &mut self,
        roots: &[String],
        state: GuiAttachedMediaSearchBuildState,
    ) {
        self.attached_media_search_build_state = state;
        if matches!(state, GuiAttachedMediaSearchBuildState::Idle) {
            self.attached_media_search_build_roots.clear();
        } else {
            self.attached_media_search_build_roots = roots.to_vec();
        }
    }

    fn sync_attached_media_search_build_state_from_index(&mut self, roots: &[String]) {
        if self.pending_attached_media_resolution.is_some() {
            return;
        }
        let state = match self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)
        {
            Some(index) if index.roots_requiring_refresh.is_empty() => {
                GuiAttachedMediaSearchBuildState::Ready
            }
            Some(_) => GuiAttachedMediaSearchBuildState::Stale,
            None if roots.is_empty() => GuiAttachedMediaSearchBuildState::Idle,
            None => GuiAttachedMediaSearchBuildState::Stale,
        };
        self.set_attached_media_search_build_state(roots, state);
    }

    fn current_player_media_path(&self) -> Option<String> {
        self.player_local_file
            .as_ref()
            .and_then(|file| file.path.clone())
    }

    fn automatic_media_resolution_trigger(
        &self,
        target: &str,
        roots: &[String],
    ) -> GuiAutomaticMediaResolutionTrigger {
        GuiAutomaticMediaResolutionTrigger {
            target: target.to_owned(),
            roots: roots.to_vec(),
            current_player_path: self.current_player_media_path(),
            index_revision: self.attached_media_search_index_revision,
            retry_due: self.attached_media_search_retry_due(),
        }
    }

    fn should_rerun_automatic_media_resolution(
        &self,
        trigger: &GuiAutomaticMediaResolutionTrigger,
    ) -> bool {
        self.last_attached_media_resolution_trigger.as_ref() != Some(trigger)
    }

    fn maybe_publish_attached_media_search_progress(
        &mut self,
        progress: GuiAttachedMediaSearchBuildProgress,
    ) {
        let now = Instant::now();
        let should_publish_immediately =
            self.attached_media_search_progress
                .as_ref()
                .is_none_or(|current| {
                    current.total_roots != progress.total_roots
                        || current.completed_roots != progress.completed_roots
                        || current.current_root_key != progress.current_root_key
                });
        let should_publish = should_publish_immediately
            || self
                .attached_media_search_progress_updated_at
                .is_none_or(|updated_at| {
                    now.duration_since(updated_at) >= Self::media_index_progress_interval()
                });
        if !should_publish {
            return;
        }
        if self.attached_media_search_progress.as_ref() == Some(&progress) {
            return;
        }
        self.attached_media_search_progress = Some(progress);
        self.attached_media_search_progress_updated_at = Some(now);
    }

    fn positive_duration_from_seconds_or_default(
        value: Option<f64>,
        default_seconds: f64,
    ) -> Duration {
        let seconds = value.unwrap_or(default_seconds);
        if !seconds.is_finite() || seconds <= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(seconds)
        }
    }

    fn automatic_media_search_timeout(state: &SyncplayGuiShellAppState) -> Duration {
        let settings = state.configuration.to_stored_settings();
        Self::positive_duration_from_seconds_or_default(
            settings.folder_search_timeout_seconds,
            LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT,
        )
    }

    pub(super) fn automatic_media_search_retry_interval(
        state: &SyncplayGuiShellAppState,
    ) -> Duration {
        let settings = state.configuration.to_stored_settings();
        Self::positive_duration_from_seconds_or_default(
            settings.folder_search_double_check_interval_seconds,
            LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT,
        )
    }

    fn attached_media_search_retry_due(&self) -> bool {
        match self.attached_media_search_next_retry_at {
            Some(deadline) => Instant::now() >= deadline,
            None => true,
        }
    }

    fn attached_media_search_refresh_pending(&self) -> bool {
        self.pending_attached_media_resolution.is_some()
            || self
                .attached_media_search_index
                .as_ref()
                .is_some_and(|index| !index.roots_requiring_refresh.is_empty())
    }

    fn cancel_pending_attached_media_search_index_build_impl(&mut self) {
        let pending_roots =
            if let Some(pending_resolution) = self.pending_attached_media_resolution.take() {
                pending_resolution
                    .cancel_flag
                    .store(true, Ordering::Relaxed);
                pending_resolution.roots.clone()
            } else {
                Vec::new()
            };
        self.attached_media_search_progress = None;
        self.attached_media_search_progress_updated_at = None;
        let state_roots = if pending_roots.is_empty() {
            self.attached_media_search_build_roots.clone()
        } else {
            pending_roots
        };
        self.sync_attached_media_search_build_state_from_index(&state_roots);
    }

    fn load_persisted_attached_media_search_index(
        &self,
        search_roots: &[PathBuf],
        roots: &[String],
        retry_interval: Duration,
    ) -> GuiAttachedMediaSearchIndex {
        let mut index = GuiAttachedMediaSearchIndex::new(roots.to_vec());
        let cache_root = self.legacy_gui_qsettings_root();
        let now_unix_ms = current_unix_time_millis();
        let stale_after_ms = retry_interval.as_millis().min(u128::from(u64::MAX)) as u64;

        for root in search_roots {
            let root_key = normalized_media_search_root_key(root);
            let persisted = cache_root.as_ref().and_then(|cache_root| {
                load_persisted_media_search_root_index_at_root(cache_root, root)
                    .ok()
                    .flatten()
            });
            match persisted {
                Some(persisted) => {
                    if now_unix_ms.saturating_sub(persisted.built_at_unix_ms) > stale_after_ms {
                        index.roots_requiring_refresh.insert(root_key.clone());
                    }
                    index.root_indexes_by_key.insert(
                        root_key.clone(),
                        GuiAttachedMediaSearchRootIndex {
                            root_key,
                            root_path: root.clone(),
                            built_at_unix_ms: persisted.built_at_unix_ms,
                            candidates_by_name: persisted.candidates_by_name,
                        },
                    );
                }
                None => {
                    index.roots_requiring_refresh.insert(root_key);
                }
            }
        }

        index
    }

    fn ensure_loaded_attached_media_search_index(
        &mut self,
        search_roots: &[PathBuf],
        roots: &[String],
        retry_interval: Duration,
    ) {
        if self
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| index.roots == roots)
        {
            self.sync_attached_media_search_build_state_from_index(roots);
            return;
        }
        self.cancel_pending_attached_media_search_index_build_impl();
        self.attached_media_search_next_retry_at = None;
        self.attached_media_search_index = Some(self.load_persisted_attached_media_search_index(
            search_roots,
            roots,
            retry_interval,
        ));
        self.sync_attached_media_search_build_state_from_index(roots);
    }

    fn normalized_media_search_path_key(path: &Path) -> String {
        let mut key = path.to_string_lossy().replace('\\', "/");
        while key.ends_with('/') && key.len() > 1 {
            key.pop();
        }
        if cfg!(windows) {
            key.to_ascii_lowercase()
        } else {
            key
        }
    }

    fn path_is_under_directory(candidate: &Path, directory: &Path) -> bool {
        let candidate_key = Self::normalized_media_search_path_key(candidate);
        let directory_key = Self::normalized_media_search_path_key(directory);
        candidate_key == directory_key || candidate_key.starts_with(&format!("{directory_key}/"))
    }

    fn cached_missing_media_candidate_path(root_path: &Path, relative_path: &str) -> PathBuf {
        let direct_path = root_path.join(relative_path);
        if cfg!(windows) || !relative_path.contains('\\') || direct_path.is_file() {
            return direct_path;
        }

        let normalized_path = root_path.join(relative_path.replace('\\', "/"));
        if normalized_path.is_file() {
            normalized_path
        } else {
            direct_path
        }
    }

    fn cached_missing_media_target_path(
        &self,
        index: &GuiAttachedMediaSearchIndex,
        target: &str,
    ) -> Option<String> {
        let target_key =
            GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(target)?;
        let current_parent = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let mut best_match: Option<((usize, usize, usize, String), String)> = None;

        for (root_order, root_key) in index.roots.iter().enumerate() {
            let Some(root_index) = index.root_indexes_by_key.get(root_key) else {
                continue;
            };
            let Some(candidates) = root_index.candidates_by_name.get(&target_key) else {
                continue;
            };
            for relative_path in candidates {
                let candidate_path =
                    Self::cached_missing_media_candidate_path(&root_index.root_path, relative_path);
                if !candidate_path.is_file() {
                    continue;
                }
                let locality_rank = current_parent
                    .as_ref()
                    .map(|parent| {
                        usize::from(!Self::path_is_under_directory(&candidate_path, parent))
                    })
                    .unwrap_or(1);
                let depth = Path::new(relative_path).components().count();
                let lexical = if cfg!(windows) {
                    relative_path.replace('\\', "/").to_ascii_lowercase()
                } else {
                    relative_path.replace('\\', "/")
                };
                let rank = (locality_rank, root_order, depth, lexical);
                let candidate_path = candidate_path.to_string_lossy().into_owned();
                if best_match
                    .as_ref()
                    .is_none_or(|(best_rank, _)| rank < *best_rank)
                {
                    best_match = Some((rank, candidate_path));
                }
            }
        }

        best_match.map(|(_, path)| path)
    }

    fn media_index_progress_root_label(path: &Path) -> String {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| path.display().to_string())
    }

    fn media_index_progress_message(progress: &GuiAttachedMediaSearchBuildProgress) -> String {
        let root_label = Self::media_index_progress_root_label(&progress.current_root_path);
        let current_root = progress
            .completed_roots
            .saturating_add(1)
            .min(progress.total_roots.max(1));
        format!(
            "Indexing media {current_root}/{}: {} folders, {} files ({root_label})",
            progress.total_roots.max(1),
            progress.scanned_directories,
            progress.indexed_files,
        )
    }

    pub(super) fn media_index_runtime_snapshot_impl(&self) -> GuiMediaIndexRuntimeSnapshot {
        if let Some(progress) = self.attached_media_search_progress.as_ref() {
            return GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some(Self::media_index_progress_message(progress)),
            };
        }
        if self.pending_attached_media_resolution.is_some() {
            return GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some("Indexing media library...".to_owned()),
            };
        }
        GuiMediaIndexRuntimeSnapshot::default()
    }

    fn build_attached_media_search_roots_in_parallel(
        search_roots: Vec<PathBuf>,
        cache_root: Option<PathBuf>,
        cancel_flag: Arc<AtomicBool>,
        latest_progress: Arc<Mutex<Option<GuiAttachedMediaSearchBuildProgress>>>,
        deadline: Option<Instant>,
    ) -> Vec<GuiAttachedMediaSearchRootRefreshResult> {
        let total_roots = search_roots.len();
        let total_workers =
            GuiClientCoreChatSessionRuntimeAdapter::configured_missing_media_parallelism().max(1);
        let root_worker_count = total_roots.min(total_workers).max(1);
        let per_root_worker_count = (total_workers / root_worker_count).max(1);
        let pending_roots = Arc::new(Mutex::new(
            search_roots
                .into_iter()
                .enumerate()
                .collect::<VecDeque<(usize, PathBuf)>>(),
        ));
        let completed_roots = Arc::new(AtomicUsize::new(0));
        let results = Arc::new(Mutex::new(Vec::<(
            usize,
            GuiAttachedMediaSearchRootRefreshResult,
        )>::new()));

        std::thread::scope(|scope| {
            for _ in 0..root_worker_count {
                let pending_roots = Arc::clone(&pending_roots);
                let completed_roots = Arc::clone(&completed_roots);
                let results = Arc::clone(&results);
                let latest_progress = Arc::clone(&latest_progress);
                let cache_root = cache_root.clone();
                let cancel_flag = Arc::clone(&cancel_flag);
                scope.spawn(move || loop {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some((root_order, root)) = pending_roots
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .pop_front()
                    else {
                        return;
                    };

                    let root_key = normalized_media_search_root_key(&root);
                    let root_path = root.clone();
                    let mut report_progress = |scanned_directories: usize, indexed_files: usize| {
                        if cancel_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let progress = GuiAttachedMediaSearchBuildProgress {
                            total_roots,
                            completed_roots: completed_roots.load(Ordering::Relaxed),
                            current_root_key: root_key.clone(),
                            current_root_path: root_path.clone(),
                            scanned_directories,
                            indexed_files,
                        };
                        let mut latest = latest_progress
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        *latest = Some(progress);
                    };

                    let result = match GuiClientCoreChatSessionRuntimeAdapter::build_missing_media_file_name_index_for_path_with_progress_and_workers(
                        &root,
                        deadline,
                        cancel_flag.as_ref(),
                        per_root_worker_count,
                        &mut report_progress,
                    ) {
                        Ok(candidates_by_name) => {
                            let root_index = GuiAttachedMediaSearchRootIndex {
                                root_key: root_key.clone(),
                                root_path: root.clone(),
                                built_at_unix_ms: current_unix_time_millis(),
                                candidates_by_name,
                            };
                            if let Some(cache_root) = cache_root.as_ref() {
                                let _ = persist_media_search_root_index_borrowed_at_root(
                                    cache_root,
                                    &root_index.root_key,
                                    &root_index.root_path,
                                    root_index.built_at_unix_ms,
                                    &root_index.candidates_by_name,
                                );
                            }
                            GuiAttachedMediaSearchRootRefreshResult {
                                root_key,
                                index: Some(root_index),
                                error: None,
                            }
                        }
                        Err(error) => GuiAttachedMediaSearchRootRefreshResult {
                            root_key,
                            index: None,
                            error: Some(error),
                        },
                    };

                    results
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push((root_order, result));
                    completed_roots.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        let mut results = results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        results.sort_by_key(|(root_order, _)| *root_order);
        results.drain(..).map(|(_, result)| result).collect()
    }

    pub(super) fn poll_attached_media_search_index_build(
        &mut self,
        retry_interval: Duration,
    ) -> bool {
        let Some(pending_resolution) = self.pending_attached_media_resolution.take() else {
            return false;
        };
        let roots = pending_resolution.roots.clone();
        if let Some(progress) = pending_resolution
            .latest_progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            self.set_attached_media_search_build_state(
                &roots,
                GuiAttachedMediaSearchBuildState::Building,
            );
            self.maybe_publish_attached_media_search_progress(progress);
        }

        match pending_resolution.result_rx.try_recv() {
            Ok(GuiAttachedMediaSearchBuildStatus::Completed(results)) => {
                let index = self
                    .attached_media_search_index
                    .get_or_insert_with(|| GuiAttachedMediaSearchIndex::new(roots.clone()));
                let mut refresh_retry_required = false;
                index.roots = roots.clone();
                for result in results {
                    match result.index {
                        Some(root_index) => {
                            index
                                .root_indexes_by_key
                                .insert(result.root_key.clone(), root_index);
                            index.roots_requiring_refresh.remove(&result.root_key);
                        }
                        None => {
                            index
                                .roots_requiring_refresh
                                .insert(result.root_key.clone());
                            if result.error.is_some() {
                                refresh_retry_required = true;
                            }
                        }
                    }
                }
                self.attached_media_search_progress = None;
                self.attached_media_search_progress_updated_at = None;
                self.attached_media_search_index_revision =
                    self.attached_media_search_index_revision.wrapping_add(1);
                self.attached_media_search_next_retry_at =
                    refresh_retry_required.then_some(Instant::now() + retry_interval);
                let next_state = if refresh_retry_required {
                    if index.root_indexes_by_key.is_empty() {
                        GuiAttachedMediaSearchBuildState::Failed
                    } else {
                        GuiAttachedMediaSearchBuildState::Stale
                    }
                } else {
                    GuiAttachedMediaSearchBuildState::Ready
                };
                self.set_attached_media_search_build_state(&roots, next_state);
                false
            }
            Ok(GuiAttachedMediaSearchBuildStatus::Cancelled) => {
                self.attached_media_search_progress = None;
                self.attached_media_search_progress_updated_at = None;
                self.sync_attached_media_search_build_state_from_index(&roots);
                false
            }
            Err(TryRecvError::Empty) => {
                if self.attached_media_search_progress.is_some() {
                    self.set_attached_media_search_build_state(
                        &roots,
                        GuiAttachedMediaSearchBuildState::Building,
                    );
                }
                self.pending_attached_media_resolution = Some(pending_resolution);
                true
            }
            Err(TryRecvError::Disconnected) => {
                let has_cached_index = if let Some(index) =
                    self.attached_media_search_index.as_mut()
                    && index.roots == roots
                {
                    index.roots_requiring_refresh.extend(roots.iter().cloned());
                    !index.root_indexes_by_key.is_empty()
                } else {
                    false
                };
                self.attached_media_search_progress = None;
                self.attached_media_search_progress_updated_at = None;
                self.attached_media_search_index_revision =
                    self.attached_media_search_index_revision.wrapping_add(1);
                self.attached_media_search_next_retry_at = Some(Instant::now() + retry_interval);
                self.set_attached_media_search_build_state(
                    &roots,
                    if has_cached_index {
                        GuiAttachedMediaSearchBuildState::Stale
                    } else {
                        GuiAttachedMediaSearchBuildState::Failed
                    },
                );
                false
            }
        }
    }

    fn queue_attached_media_search_index_build(
        &mut self,
        search_roots: Vec<PathBuf>,
        roots: Vec<String>,
        search_timeout: Duration,
    ) {
        if search_roots.is_empty() {
            self.attached_media_search_progress = None;
            self.attached_media_search_progress_updated_at = None;
            self.set_attached_media_search_build_state(
                &roots,
                GuiAttachedMediaSearchBuildState::Idle,
            );
            return;
        }
        let (result_tx, result_rx) = mpsc::channel();
        let cache_root = self.legacy_gui_qsettings_root();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let latest_progress = Arc::new(Mutex::new(None));
        let job_id = self.next_attached_media_search_job_id();
        if let Some(root) = search_roots.first() {
            self.maybe_publish_attached_media_search_progress(
                GuiAttachedMediaSearchBuildProgress {
                    total_roots: search_roots.len(),
                    completed_roots: 0,
                    current_root_key: normalized_media_search_root_key(root),
                    current_root_path: root.clone(),
                    scanned_directories: 0,
                    indexed_files: 0,
                },
            );
        }
        self.set_attached_media_search_build_state(
            &roots,
            GuiAttachedMediaSearchBuildState::Queued,
        );
        let worker_cancel_flag = Arc::clone(&cancel_flag);
        let worker_latest_progress = Arc::clone(&latest_progress);
        let status_cancel_flag = Arc::clone(&cancel_flag);
        std::thread::spawn(move || {
            let deadline = Some(Instant::now() + search_timeout);
            let results = Self::build_attached_media_search_roots_in_parallel(
                search_roots,
                cache_root,
                worker_cancel_flag,
                worker_latest_progress,
                deadline,
            );
            let status = if status_cancel_flag.load(Ordering::Relaxed) {
                GuiAttachedMediaSearchBuildStatus::Cancelled
            } else {
                GuiAttachedMediaSearchBuildStatus::Completed(results)
            };
            let _ = result_tx.send(status);
        });
        self.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
            job_id,
            roots,
            cancel_flag,
            latest_progress,
            result_rx,
        });
    }

    fn queued_attached_media_search_refresh_roots(
        index: &GuiAttachedMediaSearchIndex,
        search_roots: &[PathBuf],
    ) -> Vec<PathBuf> {
        search_roots
            .iter()
            .filter(|root| {
                index
                    .roots_requiring_refresh
                    .contains(&normalized_media_search_root_key(root))
            })
            .cloned()
            .collect()
    }

    fn queue_attached_media_search_refresh_if_needed(
        &mut self,
        search_roots: &[PathBuf],
        roots: &[String],
        retry_interval: Duration,
        search_timeout: Duration,
    ) -> bool {
        if self.pending_attached_media_resolution.is_some() {
            return false;
        }
        let Some(index) = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)
        else {
            return false;
        };

        let mut refresh_roots =
            Self::queued_attached_media_search_refresh_roots(index, search_roots);
        let retry_due = self.attached_media_search_retry_due();
        if refresh_roots.is_empty() {
            if self.attached_media_search_next_retry_at.is_none() {
                self.attached_media_search_next_retry_at = Some(Instant::now() + retry_interval);
                return false;
            }
            if !retry_due {
                return false;
            }
            if let Some(index) = self
                .attached_media_search_index
                .as_mut()
                .filter(|index| index.roots == roots)
            {
                index.roots_requiring_refresh.extend(roots.iter().cloned());
            }
            refresh_roots = search_roots.to_vec();
        } else if self.attached_media_search_next_retry_at.is_some() && !retry_due {
            return false;
        }

        self.attached_media_search_next_retry_at = None;
        self.queue_attached_media_search_index_build(refresh_roots, roots.to_vec(), search_timeout);
        true
    }

    fn resolve_main_window_user_media_target_from_index(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target: &str,
        reset_retry_on_target_change: bool,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(GuiUserMediaTargetResolution::Missing);
        };
        if reset_retry_on_target_change
            && self.unresolved_attached_media_target.as_deref() != Some(target.as_str())
            && !self.attached_media_search_refresh_pending()
        {
            self.attached_media_search_next_retry_at = None;
        }
        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let retry_interval = Self::automatic_media_search_retry_interval(state);
        if let Some(path) = self.quick_resolve_main_window_user_media_target(state, &target)? {
            if search_roots.is_empty() {
                if !self.attached_media_search_refresh_pending() {
                    self.cancel_pending_attached_media_search_index_build_impl();
                    self.attached_media_search_next_retry_at = None;
                }
            } else {
                self.ensure_loaded_attached_media_search_index(
                    &search_roots,
                    &roots,
                    retry_interval,
                );
                let _ = self.poll_attached_media_search_index_build(retry_interval);
                let _ = self.queue_attached_media_search_refresh_if_needed(
                    &search_roots,
                    &roots,
                    retry_interval,
                    Self::automatic_media_search_timeout(state),
                );
                if !self.attached_media_search_refresh_pending() {
                    self.cancel_pending_attached_media_search_index_build_impl();
                    self.attached_media_search_next_retry_at = None;
                }
            }
            self.unresolved_attached_media_target = None;
            return Ok(GuiUserMediaTargetResolution::Resolved(path));
        }

        if search_roots.is_empty() {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.attached_media_search_index = None;
            self.set_attached_media_search_build_state(
                &roots,
                GuiAttachedMediaSearchBuildState::Idle,
            );
            return Ok(GuiUserMediaTargetResolution::Missing);
        }
        self.ensure_loaded_attached_media_search_index(&search_roots, &roots, retry_interval);
        let build_pending = self.poll_attached_media_search_index_build(retry_interval);
        if let Some(found_path) = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)
            .and_then(|index| self.cached_missing_media_target_path(index, &target))
        {
            let _ = self.queue_attached_media_search_refresh_if_needed(
                &search_roots,
                &roots,
                retry_interval,
                Self::automatic_media_search_timeout(state),
            );
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return Ok(GuiUserMediaTargetResolution::Resolved(found_path));
        }
        self.unresolved_attached_media_target = Some(target);
        let queued_refresh = if build_pending {
            false
        } else {
            self.queue_attached_media_search_refresh_if_needed(
                &search_roots,
                &roots,
                retry_interval,
                Self::automatic_media_search_timeout(state),
            )
        };
        if build_pending || queued_refresh || self.attached_media_search_refresh_pending() {
            Ok(GuiUserMediaTargetResolution::Pending)
        } else {
            Ok(GuiUserMediaTargetResolution::Missing)
        }
    }

    fn resolve_main_window_user_media_target_for_automatic_sync(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_from_index(state, target, true)
    }

    pub(super) fn resolve_main_window_user_media_target(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_from_index(state, target, false)
    }

    pub(super) fn sync_selected_shared_playlist_media_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let Some(target) = self.current_shared_playlist_target(state) else {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            self.last_attached_media_resolution_trigger = None;
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };

        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let trigger = self.automatic_media_resolution_trigger(&target, &roots);
        if !self.should_rerun_automatic_media_resolution(&trigger) {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent())
                && self.current_player_matches_media_target(&target)
            {
                self.unresolved_attached_media_target = None;
                if !self.attached_media_search_refresh_pending() {
                    self.attached_media_search_next_retry_at = None;
                }
                return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
            }
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.last_attached_media_resolution_trigger = Some(trigger);

        let resolved_target = match self
            .resolve_main_window_user_media_target_for_automatic_sync(state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending | GuiUserMediaTargetResolution::Missing)
            | Err(_) => return SelectedPlaylistMediaSyncOutcome::NoChange,
        };

        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if self.current_player_matches_media_target(&resolved_target) {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
        }

        let player_paths = [resolved_target];
        let opened = self
            .open_media_files_through_attached_player_result_impl(&player_paths)
            .is_some_and(|result| result.is_ok());
        if opened {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return SelectedPlaylistMediaSyncOutcome::OpenedNewMedia;
        }
        SelectedPlaylistMediaSyncOutcome::NoChange
    }

    fn open_selected_playlist_media_path_through_attached_player_impl(
        &mut self,
        player_paths: &[String],
    ) -> SelectedPlaylistMediaSyncOutcome {
        let Some(selected_path) = player_paths.first() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };

        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if self.current_player_matches_media_target(selected_path) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
        }

        let player_paths = [selected_path.clone()];
        let opened = self
            .open_media_files_through_attached_player_result_impl(&player_paths)
            .is_some_and(|result| result.is_ok());
        if opened {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return SelectedPlaylistMediaSyncOutcome::OpenedNewMedia;
        }
        SelectedPlaylistMediaSyncOutcome::NoChange
    }

    pub(super) fn apply_pending_playlist_index_reset_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        opened_selected_media: bool,
    ) {
        if !opened_selected_media {
            return;
        }
        if self
            .current_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| !self.current_player_matches_media_target(target))
        {
            return;
        }
        if !self.player_local_file_ready_for_attached_sync() {
            return;
        }
        let Some(pause_before_sync) = self
            .session
            .as_mut()
            .and_then(|session| session.take_pending_playlist_index_reset_intent())
        else {
            return;
        };

        self.suppressed_attached_room_playstate_after_playlist_reset = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playstate());
        let reset_target_position_seconds =
            self.player_target_position_seconds_for_global_position_impl(0.0);

        let Some(player) = self.player.as_mut() else {
            return;
        };

        let mut state_changed = false;
        match player.set_position(reset_target_position_seconds) {
            Ok(()) => {
                self.player_position_seconds = Some(0.0);
                state_changed = true;
            }
            Err(error) if Self::attached_player_playlist_reset_error_is_transient(&error) => {
                if let Some(session) = self.session.as_mut() {
                    session.note_local_playlist_index_reset_intent(pause_before_sync);
                }
                return;
            }
            Err(error) => {
                eprintln!(
                    "warning: failed to rewind the attached player for a playlist switch reset: {error}"
                );
            }
        }
        if pause_before_sync {
            match player.set_paused(true) {
                Ok(()) => {
                    self.player_paused = Some(true);
                    state_changed = true;
                }
                Err(error) if Self::attached_player_playlist_reset_error_is_transient(&error) => {
                    if let Some(session) = self.session.as_mut() {
                        session.note_local_playlist_index_reset_intent(true);
                    }
                    return;
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to pause the attached player for a playlist switch reset: {error}"
                    );
                }
            }
        }

        if let Some(session) = self.session.as_mut()
            && let Err(error) =
                session.sync_local_playback_telemetry(pause_before_sync.then_some(true), Some(0.0))
        {
            eprintln!(
                "warning: failed to mirror playlist switch reset telemetry into the session runtime: {error}"
            );
        }

        self.last_applied_attached_room_playstate = None;
        if state_changed {
            self.refresh_player_state_impl();
        }
    }

    fn attached_player_playlist_reset_error_is_transient(
        error: &syncplay_player_api::PlayerError,
    ) -> bool {
        let syncplay_player_api::PlayerError::OperationFailed(message) = error else {
            return false;
        };
        let lower = message.to_ascii_lowercase();
        lower.contains("property unavailable") || lower.contains("no file loaded")
    }

    pub(super) fn sync_session_playstate_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        force: bool,
    ) {
        if self
            .current_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| !self.current_player_matches_media_target(target))
        {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        if self.player.is_none() {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        if !self.player_local_file_ready_for_attached_sync() {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        let Some((playstate, raw_playstate, local_username)) =
            self.session.as_ref().and_then(|session| {
                session
                    .current_room_playstate_for_attached_player_sync()
                    .map(|playstate| {
                        (
                            playstate,
                            session.current_room_playstate(),
                            session.local_username().map(str::to_owned),
                        )
                    })
            })
        else {
            self.last_applied_attached_room_playstate = None;
            return;
        };
        if let Some(suppressed_playstate) = self
            .suppressed_attached_room_playstate_after_playlist_reset
            .as_ref()
        {
            if raw_playstate.as_ref() == Some(suppressed_playstate) {
                return;
            }
            self.suppressed_attached_room_playstate_after_playlist_reset = None;
        }
        let playstate_unchanged =
            !force && self.last_applied_attached_room_playstate.as_ref() == Some(&playstate);
        let set_by_is_local_user = playstate
            .set_by
            .as_deref()
            .zip(local_username.as_deref())
            .is_some_and(|(set_by, local_username)| set_by == local_username);
        if self.pending_local_attached_pause_override == playstate.paused {
            self.pending_local_attached_pause_override = None;
        }
        let suppress_stale_room_pause_sync = self
            .pending_local_attached_pause_override
            .is_some_and(|pending_paused| playstate.paused != Some(pending_paused));
        let sync_paused_state = (!suppress_stale_room_pause_sync)
            .then_some(playstate.paused)
            .flatten();
        let initial_room_playstate_sync = self.last_applied_attached_room_playstate.is_none();
        let allow_initial_self_origin_position_sync =
            force && self.player_position_seconds.is_none() && initial_room_playstate_sync;
        let allow_initial_remote_position_sync =
            initial_room_playstate_sync && !set_by_is_local_user;
        let user_offset_seconds = self.user_offset_seconds;
        let should_seek_for_room_playstate = force
            || playstate.do_seek == Some(true)
            || sync_paused_state == Some(true)
            || allow_initial_remote_position_sync;

        let mut state_changed = false;
        let mut room_playstate_sync_failed = false;
        if !playstate_unchanged {
            if let Some(position_seconds) = playstate.position_seconds
                && (!set_by_is_local_user || allow_initial_self_origin_position_sync)
                && should_seek_for_room_playstate
                && (force
                    || self
                        .player_position_seconds
                        .map(|current_position_seconds| {
                            (current_position_seconds - position_seconds).abs() > f64::EPSILON
                        })
                        .unwrap_or(true))
            {
                let sync_position_seconds = (position_seconds + user_offset_seconds).max(0.0);
                match self
                    .player
                    .as_mut()
                    .expect("player should exist while syncing playback position")
                    .set_position(sync_position_seconds)
                {
                    Ok(()) => {
                        self.player_position_seconds = Some(position_seconds);
                        state_changed = true;
                    }
                    Err(error) => {
                        room_playstate_sync_failed = true;
                        eprintln!(
                            "warning: failed to sync session playback position to the attached player: {error}"
                        );
                    }
                }
            }

            if let Some(paused) = sync_paused_state
                && (force || self.player_paused != Some(paused))
            {
                match self
                    .player
                    .as_mut()
                    .expect("player should exist while syncing playback pause state")
                    .set_paused(paused)
                {
                    Ok(()) => {
                        self.player_paused = Some(paused);
                        state_changed = true;
                    }
                    Err(error) => {
                        room_playstate_sync_failed = true;
                        eprintln!(
                            "warning: failed to sync session playback pause state to the attached player: {error}"
                        );
                    }
                }
            }

            if !room_playstate_sync_failed {
                self.last_applied_attached_room_playstate = Some(playstate.clone());
            }
        }

        if !state_changed {
            let attached_runtime_actions = self
                .session
                .as_mut()
                .map(|session| session.attached_player_runtime_actions(system_time_seconds()));
            match attached_runtime_actions {
                Some(Ok(actions)) => {
                    for action in actions {
                        match action {
                            GuiAttachedPlayerRuntimeAction::Paused(paused) => {
                                match self
                                    .player
                                    .as_mut()
                                    .expect(
                                        "player should exist while applying attached pause correction",
                                    )
                                    .set_paused(paused)
                                {
                                    Ok(()) => {
                                        self.player_paused = Some(paused);
                                        state_changed = true;
                                        if let Some(session) = self.session.as_mut()
                                            && let Err(error) = session.sync_local_playback_telemetry(
                                                Some(paused),
                                                self.player_position_seconds,
                                            )
                                        {
                                            eprintln!(
                                                "warning: failed to mirror attached-player pause correction into the session runtime: {error}"
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "warning: failed to apply attached-player pause correction: {error}"
                                        );
                                    }
                                }
                            }
                            GuiAttachedPlayerRuntimeAction::Position(position_seconds) => {
                                let sync_position_seconds =
                                    (position_seconds + user_offset_seconds).max(0.0);
                                match self
                                    .player
                                    .as_mut()
                                    .expect("player should exist while applying desync correction")
                                    .set_position(sync_position_seconds)
                                {
                                    Ok(()) => {
                                        self.player_position_seconds = Some(position_seconds);
                                        state_changed = true;
                                        if let Some(session) = self.session.as_mut()
                                            && let Err(error) = session.sync_local_playback_telemetry(
                                                self.player_paused,
                                                Some(position_seconds),
                                            )
                                        {
                                            eprintln!(
                                                "warning: failed to mirror desync-corrected playback position into the session runtime: {error}"
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "warning: failed to apply attached-player desync position correction: {error}"
                                        );
                                    }
                                }
                            }
                            GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate) => {
                                if let Err(error) = self
                                    .player
                                    .as_mut()
                                    .expect("player should exist while applying playback-rate correction")
                                    .set_playback_rate(playback_rate)
                                {
                                    eprintln!(
                                        "warning: failed to apply attached-player playback-rate correction: {error}"
                                    );
                                }
                            }
                        }
                    }
                }
                Some(Err(error)) => {
                    eprintln!(
                        "warning: failed to evaluate attached-player desync correction actions: {error}"
                    );
                }
                None => {}
            }
        }

        if !room_playstate_sync_failed && self.last_applied_attached_room_playstate.is_none() {
            self.last_applied_attached_room_playstate = Some(playstate);
        }
        if state_changed {
            self.refresh_player_state_impl();
        }
    }

    pub(super) fn open_main_window_user_media_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!("Indexing media library to resolve user media: {target}."),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if projected_state.playlist_backed_media_opens_preferred() {
            self.open_media_files_through_shared_playlist_runtime_impl(
                handle,
                projected_state,
                vec![resolved_target],
                None,
            );
            return;
        }

        self.ensure_configured_player_attached();
        if self.player.is_some() {
            self.open_media_files_through_attached_player_impl(handle, vec![resolved_target]);
        } else {
            Self::push_runtime_unavailable(
                handle,
                self.open_media_unavailable_message_impl(&[resolved_target]),
            );
        }
    }

    fn project_loaded_shared_playlist_into_state(
        projected_state: &mut SyncplayGuiShellAppState,
        entries: Vec<String>,
        selected_index: Option<usize>,
    ) -> bool {
        let entries = SyncplayGuiShellAppState::normalize_shared_playlist_entries(entries);
        let selected_index = selected_index
            .filter(|_| !entries.is_empty())
            .map(|index| index.min(entries.len().saturating_sub(1)));
        projected_state.main_window.shared_playlist_enabled = true;
        projected_state.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        projected_state.apply_shared_playlist_entries(entries.clone(), selected_index, false);
        true
    }

    fn open_system_file_browser_for_path(path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err(format!(
                "Could not open a containing folder for '{}': no parent directory exists.",
                path.display()
            ));
        };

        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg(parent);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(parent);
            command
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(parent);
            command
        };

        command.spawn().map_err(|error| {
            format!(
                "Opening the containing folder for '{}' failed: {error}",
                path.display()
            )
        })?;
        Ok(())
    }

    pub(super) fn open_main_window_user_containing_folder_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!(
                            "Indexing media library to resolve a local path for user media: {target}."
                        ),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if browser_is_url(&resolved_target) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Cannot open a containing folder for the stream URL: {resolved_target}."),
            );
            return;
        }

        if let Err(error) = Self::open_system_file_browser_for_path(Path::new(&resolved_target)) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }

    pub(super) fn open_media_files_through_shared_playlist_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        paths: Vec<String>,
        playlist_insert_slot: Option<usize>,
    ) {
        let selected_paths = paths
            .into_iter()
            .filter_map(|path| normalized_editable_text(&path))
            .collect::<Vec<_>>();
        if selected_paths.is_empty() {
            return;
        }

        let dispatch =
            match Self::shared_playlist_open_dispatch_for_paths_impl(selected_paths.clone()) {
                Ok(dispatch) => dispatch,
                Err(error) => {
                    Self::push_runtime_unavailable(handle, error);
                    return;
                }
            };
        let current_playlist_entry_count = projected_state.main_window.playlist.len();
        let (playlist_entries, selected_playlist_index) = projected_state
            .shared_playlist_entries_after_media_open_from_state(
                dispatch.playlist_entries.clone(),
                playlist_insert_slot,
            );
        let opened_entry_count = if playlist_insert_slot.is_some() {
            playlist_entries
                .len()
                .saturating_sub(current_playlist_entry_count)
        } else {
            playlist_entries.len()
        };
        if playlist_insert_slot.is_some() && opened_entry_count == 0 {
            return;
        }
        let selected_opened_entry_offset = Self::selected_opened_entry_offset(
            selected_playlist_index,
            opened_entry_count,
            playlist_insert_slot,
        );

        if self.session.is_none() {
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }

            if !Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
            ) {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }
            self.active_shared_playlist_index = selected_playlist_index;
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                    MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                )],
            );

            let selected_media_sync = selected_opened_entry_offset
                .and_then(|offset| {
                    dispatch
                        .player_paths
                        .as_ref()
                        .and_then(|player_paths| player_paths.get(offset).cloned())
                })
                .map(|selected_path| {
                    self.open_selected_playlist_media_path_through_attached_player_impl(&[
                        selected_path,
                    ])
                })
                .unwrap_or(SelectedPlaylistMediaSyncOutcome::NoChange);
            let selection_handoff_ready = selected_media_sync.selection_handoff_ready(false);
            self.sync_session_playstate_to_attached_player_impl(
                projected_state,
                selection_handoff_ready,
            );

            let success_message =
                Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            let warning = self.shared_playlist_session_unavailable_message_impl();
            handle.push_actions([
                GuiShellAction::SwitchView(GuiShellView::MainWindow),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: success_message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(success_message),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: warning.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(warning),
            ]);
            return;
        }

        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.playlist_control_available())
        {
            Self::push_runtime_unavailable(
                handle,
                self.shared_playlist_control_unavailable_message_impl(),
            );
            return;
        }

        let session_result = self
            .session
            .as_mut()
            .expect("session should exist")
            .replace_playlist(playlist_entries.clone(), selected_playlist_index);
        let session_success = session_result.is_ok();
        if session_success {
            self.active_shared_playlist_index = selected_playlist_index;
        }
        let selected_media_sync = if session_success
            && Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
            ) {
            selected_opened_entry_offset
                .and_then(|offset| {
                    dispatch
                        .player_paths
                        .as_ref()
                        .and_then(|player_paths| player_paths.get(offset).cloned())
                })
                .map(|selected_path| {
                    self.open_selected_playlist_media_path_through_attached_player_impl(&[
                        selected_path,
                    ])
                })
                .unwrap_or(SelectedPlaylistMediaSyncOutcome::NoChange)
        } else {
            SelectedPlaylistMediaSyncOutcome::NoChange
        };
        if selected_media_sync.selection_ready()
            && let Some(session) = self.session.as_mut()
        {
            session.note_local_playlist_index_reset_intent(true);
        }
        let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
            self.session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent()),
        );
        self.apply_pending_playlist_index_reset_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );
        self.sync_session_playstate_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );

        let mut actions = Vec::new();
        if session_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::MainWindow));
        }
        if session_success {
            let message = Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        if let Err(error) = session_result {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
        }
        handle.push_actions(actions);
    }

    pub(super) fn emit_gui_actions_to_attached_player_impl(&mut self, actions: &[GuiShellAction]) {
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return;
        };
        let mut already_emitted_osd_messages = BTreeSet::new();
        for action in actions {
            match action {
                GuiShellAction::PushChatMessage { sender, message } => {
                    if let Err(error) =
                        player.show_syncplay_legacy_chat_message(&format!("<{sender}> {message}"))
                    {
                        eprintln!(
                            "warning: failed to display GUI chat notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::PushTransientNotification { level, message } => {
                    already_emitted_osd_messages.insert(message.clone());
                    let kind = match level {
                        GuiTransientNotificationLevel::Info
                        | GuiTransientNotificationLevel::Success => {
                            LegacySyncplayOsdKind::Notification
                        }
                        GuiTransientNotificationLevel::Warning
                        | GuiTransientNotificationLevel::Error => LegacySyncplayOsdKind::Alert,
                    };
                    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
                        eprintln!(
                            "warning: failed to display GUI notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if already_emitted_osd_messages.insert(message.clone()) =>
                {
                    if let Err(error) = player
                        .show_syncplay_legacy_message(message, LegacySyncplayOsdKind::Notification)
                    {
                        eprintln!(
                            "warning: failed to display GUI system-chat event via mpv OSD: {error}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn drain_player_chat_input_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let mut errors = Vec::new();
        let chat_ready = self
            .session
            .as_ref()
            .is_some_and(|session| session.attached_player_chat_input_ready());
        let unavailable_message = self
            .session
            .as_ref()
            .map(|session| session.attached_player_chat_input_unavailable_message())
            .unwrap_or_else(|| {
                "Chat input from the attached player requires an active session with chat support."
                    .to_owned()
            });
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
            if !chat_ready {
                errors.push(unavailable_message.clone());
                continue;
            }
            let send_result = self
                .session
                .as_mut()
                .expect("session should exist when draining player chat")
                .send_chat_message(message.clone());
            if let Err(error) = send_result {
                errors.push(format!(
                    "Chat input from the attached player could not be sent: {error}"
                ));
            }
        }

        if !errors.is_empty() {
            Self::push_actions_and_project(
                handle,
                projected_state,
                errors
                    .into_iter()
                    .flat_map(|message| {
                        [
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: message.clone(),
                            },
                            GuiShellAction::AnnounceSystemChatEvent(message),
                        ]
                    })
                    .collect(),
            );
        }
    }

    pub(super) fn refresh_player_state_impl(&mut self) {
        let user_offset_seconds = self.user_offset_seconds;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        while let Some(update) = player.take_playback_telemetry_update() {
            if let Some(paused) = update.paused {
                self.player_paused = Some(paused);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
        }
        while let Some(update) = player.take_local_file_update() {
            let file_changed = Self::local_file_update_replaces_current_file(
                self.player_local_file.as_ref(),
                &update,
            );
            self.player_local_file = Some(update);
            self.player_local_file_placeholder = false;
            if file_changed || self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
        self.clamp_player_position_to_file_duration();
    }

    fn player_local_file_ready_for_attached_sync(&self) -> bool {
        self.player_local_file.is_some() && !self.player_local_file_placeholder
    }

    pub(super) fn sync_manual_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<bool, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(true);
        };
        session
            .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))?;
        let seek_recorded = session.record_manual_seek_to_position(target_position_seconds)?;
        if !seek_recorded {
            return Ok(false);
        }
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(true)
    }

    pub(super) fn sync_playback_pause_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let _ = session.set_playback_paused(target_paused)?;
        session.sync_local_playback_telemetry(Some(target_paused), self.player_position_seconds)?;
        self.sync_pending_local_attached_pause_override_from_session();
        Ok(())
    }

    pub(super) fn apply_playback_pause_change_with_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(bool, Option<String>), String> {
        let mut sync_error = None;
        if !target_paused {
            match self.preflight_local_player_unpause_against_detached_session_impl(
                state,
                previous_paused,
            ) {
                Ok(GuiLocalPlayerUnpauseDecision::Block) => {
                    self.player_paused = Some(true);
                    self.refresh_player_state_impl();
                    return Ok((true, None));
                }
                Ok(GuiLocalPlayerUnpauseDecision::Allow) => {
                    self.player
                        .as_mut()
                        .expect("player should exist while applying playback pause change")
                        .set_paused(false)
                        .map_err(|error| {
                            format!(
                                "Playback pause toggle through the attached player failed while resuming playback: {error}"
                            )
                        })?;
                    self.player_paused = Some(false);
                    self.refresh_player_state_impl();
                    let mut telemetry_synced = false;
                    if let Some(session) = self.session.as_mut() {
                        match session.sync_local_playback_telemetry(
                            Some(false),
                            self.player_position_seconds,
                        ) {
                            Ok(()) => {
                                telemetry_synced = true;
                                if let Err(error) = session.finalize_local_player_unpause_attempt()
                                {
                                    sync_error = Some(error);
                                } else if let Err(error) =
                                    session.emit_immediate_playback_state_update()
                                {
                                    sync_error = Some(error);
                                }
                            }
                            Err(error) => sync_error = Some(error),
                        }
                    }
                    if telemetry_synced {
                        self.sync_pending_local_attached_pause_override_from_session();
                    }
                    return Ok((false, sync_error));
                }
                Ok(GuiLocalPlayerUnpauseDecision::NotApplicable) => {}
                Err(error) => sync_error = Some(error),
            }
        }

        self.player
            .as_mut()
            .expect("player should exist while applying playback pause change")
            .set_paused(target_paused)
            .map_err(|error| {
                format!("Playback pause toggle through the attached player failed: {error}")
            })?;
        self.player_paused = Some(target_paused);
        self.refresh_player_state_impl();
        if let Err(error) = self.sync_playback_pause_into_detached_session_impl(
            state,
            previous_paused,
            target_paused,
        ) && sync_error.is_none()
        {
            sync_error = Some(error);
        }
        Ok((target_paused, sync_error))
    }

    fn preflight_local_player_unpause_against_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
    ) -> Result<GuiLocalPlayerUnpauseDecision, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let decision = session.handle_local_player_unpause_attempt()?;
        if decision == GuiLocalPlayerUnpauseDecision::Block {
            session.sync_local_playback_telemetry(Some(true), self.player_position_seconds)?;
        }
        Ok(decision)
    }

    pub(super) fn undo_seek_target_position_from_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        Ok(session.pending_undo_seek_target_position())
    }

    pub(super) fn commit_undo_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        if !session.commit_undo_seek()? {
            return Err(
                "Playback undo seek is unavailable because no earlier seek target is recorded."
                    .to_owned(),
            );
        }
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(())
    }
}
