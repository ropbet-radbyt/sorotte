use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError},
    },
    time::{Duration, Instant},
};

use syncplay_player_api::PlayerAdapter;
use syncplay_player_mpv::LegacySyncplayOsdKind;

use super::super::runtime_bridge::GuiSharedPlaylistOpenDispatch;
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{GuiClientCoreChatSessionRuntimeAdapter, GuiOwnedPlayer};
use super::super::shell_state::{
    GuiShellAction, GuiShellView, GuiTransientNotificationLevel, MainWindowRuntimeSnapshot,
    SyncplayGuiShellAppState, browser_is_url,
};
use super::super::support::normalized_editable_text;
use super::{
    GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
    GuiPendingAttachedMediaResolution, GuiPersistedConfigRuntimeOwner,
};

const LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT: f64 = 20.0;
const LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT: f64 = 30.0;

impl GuiPersistedConfigRuntimeOwner {
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

    fn shared_playlist_entry_for_media_path(path: &str) -> Option<String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains("://") {
            return Some(trimmed.to_owned());
        }
        Some(
            Path::new(trimmed)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(trimmed)
                .to_owned(),
        )
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
            .filter_map(|path| Self::shared_playlist_entry_for_media_path(path))
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

    fn shared_playlist_open_success_message(dispatch: &GuiSharedPlaylistOpenDispatch) -> String {
        let entry_count = dispatch.playlist_entries.len();
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

    fn selected_shared_playlist_target(state: &SyncplayGuiShellAppState) -> Option<String> {
        if !state.main_window.shared_playlist_enabled {
            return None;
        }

        state
            .selection
            .selected_main_window_playlist
            .and_then(|index| state.main_window.playlist.get(index))
            .and_then(|target| normalized_editable_text(&target.label))
    }

    pub(super) fn current_player_matches_media_target(&self, target: &str) -> bool {
        let Some(local_file) = self.player_local_file.as_ref() else {
            return false;
        };

        if let Some(path) = local_file.path.as_deref() {
            if (cfg!(windows) && path.eq_ignore_ascii_case(target))
                || (!cfg!(windows) && path == target)
            {
                return true;
            }
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

    fn automatic_media_search_root_key(path: &Path) -> String {
        let key = path.to_string_lossy().into_owned();
        if cfg!(windows) {
            key.to_ascii_lowercase()
        } else {
            key
        }
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
            let key = Self::automatic_media_search_root_key(path);
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
            .map(|path| Self::automatic_media_search_root_key(path))
            .collect()
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

    fn automatic_media_search_retry_interval(state: &SyncplayGuiShellAppState) -> Duration {
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

    fn cancel_pending_attached_media_search_index_build_impl(&mut self) {
        if let Some(pending_resolution) = self.pending_attached_media_resolution.take() {
            pending_resolution
                .cancel_flag
                .store(true, Ordering::Relaxed);
        }
    }

    fn cached_missing_media_target_path(
        index: &GuiAttachedMediaSearchIndex,
        target: &str,
    ) -> Option<String> {
        GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(target)
            .and_then(|key| index.paths_by_name.get(&key).cloned())
    }

    fn poll_attached_media_search_index_build(
        &mut self,
        roots: &[String],
        retry_interval: Duration,
    ) -> Result<bool, String> {
        let Some(pending_resolution) = self.pending_attached_media_resolution.take() else {
            return Ok(false);
        };
        if pending_resolution.roots != roots {
            pending_resolution
                .cancel_flag
                .store(true, Ordering::Relaxed);
            return Ok(false);
        }

        match pending_resolution.result_rx.try_recv() {
            Ok(GuiAttachedMediaSearchBuildStatus::Completed(index)) => {
                self.attached_media_search_index = Some(index);
                self.attached_media_search_next_retry_at = None;
                Ok(false)
            }
            Ok(GuiAttachedMediaSearchBuildStatus::Cancelled) => Ok(false),
            Ok(GuiAttachedMediaSearchBuildStatus::Failed(error)) => {
                self.attached_media_search_next_retry_at = Some(Instant::now() + retry_interval);
                Err(error)
            }
            Err(TryRecvError::Empty) => {
                self.pending_attached_media_resolution = Some(pending_resolution);
                Ok(true)
            }
            Err(TryRecvError::Disconnected) => {
                self.attached_media_search_next_retry_at = Some(Instant::now() + retry_interval);
                Err("Automatic media indexing terminated unexpectedly.".to_owned())
            }
        }
    }

    fn queue_attached_media_search_index_build(
        &mut self,
        search_roots: Vec<PathBuf>,
        roots: Vec<String>,
        search_timeout: Duration,
    ) {
        let (result_tx, result_rx) = mpsc::channel();
        let result_roots = roots.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let worker_cancel_flag = cancel_flag.clone();
        std::thread::spawn(move || {
            let mut paths_by_name = HashMap::new();
            let deadline = Some(Instant::now() + search_timeout);
            for root in search_roots {
                match GuiClientCoreChatSessionRuntimeAdapter::build_missing_media_file_name_index_for_path(
                    &mut paths_by_name,
                    &root,
                    deadline,
                    worker_cancel_flag.as_ref(),
                ) {
                    Ok(()) => {}
                    Err(error) => {
                        let status = if worker_cancel_flag.load(Ordering::Relaxed) {
                            GuiAttachedMediaSearchBuildStatus::Cancelled
                        } else {
                            GuiAttachedMediaSearchBuildStatus::Failed(error)
                        };
                        let _ = result_tx.send(status);
                        return;
                    }
                }
            }
            let status = if worker_cancel_flag.load(Ordering::Relaxed) {
                GuiAttachedMediaSearchBuildStatus::Cancelled
            } else {
                GuiAttachedMediaSearchBuildStatus::Completed(GuiAttachedMediaSearchIndex {
                    roots: result_roots,
                    paths_by_name,
                })
            };
            let _ = result_tx.send(status);
        });
        self.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
            roots,
            cancel_flag,
            result_rx,
        });
    }

    fn resolve_main_window_user_media_target_for_automatic_sync(
        &mut self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<Option<String>, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(None);
        };
        if self.unresolved_attached_media_target.as_deref() != Some(target.as_str()) {
            self.attached_media_search_next_retry_at = None;
        }
        if let Some(path) = self.quick_resolve_main_window_user_media_target(state, &target)? {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return Ok(Some(path));
        }

        let search_roots = self.automatic_media_search_roots(state);
        if search_roots.is_empty() {
            self.cancel_pending_attached_media_search_index_build_impl();
            return Ok(None);
        }
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let retry_interval = Self::automatic_media_search_retry_interval(state);
        let build_pending = self.poll_attached_media_search_index_build(&roots, retry_interval)?;
        if let Some(found_path) = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)
            .and_then(|index| Self::cached_missing_media_target_path(index, &target))
        {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return Ok(Some(found_path));
        }
        let matching_index_available = self
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| index.roots == roots);
        self.unresolved_attached_media_target = Some(target);
        if build_pending {
            return Ok(None);
        }
        if !self.attached_media_search_retry_due() {
            return Ok(None);
        }
        if matching_index_available && self.attached_media_search_next_retry_at.is_none() {
            self.attached_media_search_next_retry_at = Some(Instant::now() + retry_interval);
            return Ok(None);
        }

        self.attached_media_search_next_retry_at = None;
        self.queue_attached_media_search_index_build(
            search_roots,
            roots,
            Self::automatic_media_search_timeout(state),
        );
        Ok(None)
    }

    fn resolve_main_window_user_media_target(
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
        if target_path.is_file() {
            return Ok(Some(target.to_owned()));
        }

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
            if let Some(parent) = local_path.parent()
                && let Some(found_path) =
                    GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                        &target, parent,
                    )?
            {
                return Ok(Some(found_path));
            }
        }

        let search_roots = self.automatic_media_search_roots(state);
        let search_root_keys = Self::automatic_media_search_root_keys(&search_roots);
        if let Some(found_path) = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == search_root_keys)
            .and_then(|index| Self::cached_missing_media_target_path(index, &target))
        {
            return Ok(Some(found_path));
        }

        let settings = state.configuration.to_stored_settings();
        for directory in settings.media_search_directories.unwrap_or_default() {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                    &target,
                    Path::new(trimmed),
                )?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    pub(super) fn sync_selected_shared_playlist_media_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> bool {
        let Some(target) = Self::selected_shared_playlist_target(state) else {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return false;
        };

        let resolved_target =
            match self.resolve_main_window_user_media_target_for_automatic_sync(state, &target) {
                Ok(Some(path)) => path,
                Ok(None) | Err(_) => return false,
            };

        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return false;
        }
        if self.current_player_matches_media_target(&resolved_target) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return false;
        }

        let player_paths = [resolved_target];
        let opened = self
            .open_media_files_through_attached_player_result_impl(&player_paths)
            .is_some_and(|result| result.is_ok());
        if opened {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
        }
        opened
    }

    fn open_selected_playlist_media_path_through_attached_player_impl(
        &mut self,
        player_paths: &[String],
    ) -> bool {
        let Some(selected_path) = player_paths.first() else {
            return false;
        };

        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return false;
        }
        if self.current_player_matches_media_target(selected_path) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return false;
        }

        let player_paths = [selected_path.clone()];
        let opened = self
            .open_media_files_through_attached_player_result_impl(&player_paths)
            .is_some_and(|result| result.is_ok());
        if opened {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
        }
        opened
    }

    pub(super) fn sync_session_playstate_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        force: bool,
    ) {
        if Self::selected_shared_playlist_target(state)
            .as_deref()
            .is_some_and(|target| !self.current_player_matches_media_target(target))
        {
            self.last_applied_attached_room_playstate = None;
            return;
        }
        let Some(player) = self.player.as_mut() else {
            self.last_applied_attached_room_playstate = None;
            return;
        };
        let Some(playstate) = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playstate())
        else {
            self.last_applied_attached_room_playstate = None;
            return;
        };
        if !force && self.last_applied_attached_room_playstate.as_ref() == Some(&playstate) {
            return;
        }

        let mut state_changed = false;
        if let Some(position_seconds) = playstate.position_seconds
            && (force || playstate.do_seek == Some(true))
            && (force
                || self
                    .player_position_seconds
                    .map(|current_position_seconds| {
                        (current_position_seconds - position_seconds).abs() > f64::EPSILON
                    })
                    .unwrap_or(true))
        {
            match player.set_position(position_seconds) {
                Ok(()) => {
                    self.player_position_seconds = Some(position_seconds);
                    state_changed = true;
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to sync session playback position to the attached player: {error}"
                    );
                }
            }
        }

        if let Some(paused) = playstate.paused
            && (force || self.player_paused != Some(paused))
        {
            match player.set_paused(paused) {
                Ok(()) => {
                    self.player_paused = Some(paused);
                    state_changed = true;
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to sync session playback pause state to the attached player: {error}"
                    );
                }
            }
        }

        self.last_applied_attached_room_playstate = Some(playstate);
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
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
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
    ) -> bool {
        let entries = SyncplayGuiShellAppState::normalize_shared_playlist_entries(entries);
        projected_state.main_window.shared_playlist_enabled = true;
        projected_state.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        projected_state
            .apply_shared_playlist_entries(entries.clone(), (!entries.is_empty()).then_some(0));
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
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
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
                dispatch.playlist_entries.clone(),
            ) {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                    MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                )],
            );

            let opened_selected_media =
                dispatch
                    .player_paths
                    .as_deref()
                    .is_some_and(|player_paths| {
                        self.open_selected_playlist_media_path_through_attached_player_impl(
                            player_paths,
                        )
                    });
            self.sync_session_playstate_to_attached_player_impl(
                projected_state,
                opened_selected_media,
            );

            let success_message = Self::shared_playlist_open_success_message(&dispatch);
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
            .replace_playlist(
                dispatch.playlist_entries.clone(),
                (!dispatch.playlist_entries.is_empty()).then_some(0),
            );
        let session_success = session_result.is_ok();
        let opened_selected_media = if session_success
            && Self::project_loaded_shared_playlist_into_state(
                projected_state,
                dispatch.playlist_entries.clone(),
            ) {
            dispatch
                .player_paths
                .as_deref()
                .is_some_and(|player_paths| {
                    self.open_selected_playlist_media_path_through_attached_player_impl(
                        player_paths,
                    )
                })
        } else {
            false
        };
        self.sync_session_playstate_to_attached_player_impl(projected_state, opened_selected_media);

        let mut actions = Vec::new();
        if session_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::MainWindow));
        }
        if session_success {
            let message = Self::shared_playlist_open_success_message(&dispatch);
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
        if self.session.is_none() {
            return;
        }

        let mut errors = Vec::new();
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
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
        let Some(player) = self.player.as_mut() else {
            return;
        };
        while let Some(update) = player.take_playback_telemetry_update() {
            if let Some(paused) = update.paused {
                self.player_paused = Some(paused);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds);
            }
        }
        while let Some(update) = player.take_local_file_update() {
            self.player_local_file = Some(update);
            if self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
    }

    pub(super) fn sync_manual_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))?;
        let _ = session.record_manual_seek_to_position(target_position_seconds)?;
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(())
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
        Ok(())
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
        if !session.undo_seek()? {
            return Ok(None);
        }
        let target = session.local_position_seconds();
        session.sync_local_playback_telemetry(self.player_paused, target)?;
        Ok(target)
    }
}
