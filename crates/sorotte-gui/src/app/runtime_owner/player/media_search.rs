use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn quick_existing_media_target_path(target: &Path) -> Option<String> {
        target
            .is_file()
            .then(|| target.to_string_lossy().into_owned())
    }

    fn quick_resolve_main_window_user_media_target(
        &self,
        state: &SorotteGuiShellAppState,
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

    fn resolve_main_window_user_media_target_from_index(
        &mut self,
        state: &SorotteGuiShellAppState,
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
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_from_index(state, target, true)
    }

    pub(in crate::app::runtime_owner) fn resolve_main_window_user_media_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_from_index(state, target, false)
    }

    pub(in crate::app::runtime_owner) fn sync_selected_shared_playlist_media_to_attached_player_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let Some(target) = self.current_shared_playlist_target(state) else {
            self.refresh_stream_helper_runtime_snapshot_for_target(None);
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            self.last_attached_media_resolution_trigger = None;
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };

        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let trigger = self.automatic_media_resolution_trigger(
            &target,
            &roots,
            self.media_match_remote_resolution_token_for_state(state),
        );
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
        if !self.preflight_room_stream_target(state, &target) {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }

        let resolved_target = match self
            .resolve_main_window_user_media_target_for_automatic_sync(state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) => path,
            Ok(GuiUserMediaTargetResolution::Pending) => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            Ok(GuiUserMediaTargetResolution::Missing) | Err(_) => {
                let Some(path) = self.media_match_cached_room_candidate_for_target(state, &target)
                else {
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                };
                path
            }
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
        self.prepare_stream_load_tracking(&player_paths[0], false);
        let open_result = self.open_media_files_through_attached_player_result_impl(&player_paths);
        if let Some(Err(message)) = open_result {
            self.queue_stream_warning(message);
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if open_result.is_some_and(|result| result.is_ok()) {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return SelectedPlaylistMediaSyncOutcome::OpenedNewMedia;
        }
        SelectedPlaylistMediaSyncOutcome::NoChange
    }

    pub(super) fn open_selected_playlist_media_path_through_attached_player_impl(
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
        if !self.preflight_user_stream_target(selected_path) {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if self.current_player_matches_media_target(selected_path) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
        }

        let player_paths = [selected_path.clone()];
        self.prepare_stream_load_tracking(&player_paths[0], true);
        let open_result = self.open_media_files_through_attached_player_result_impl(&player_paths);
        if let Some(Err(message)) = open_result {
            self.queue_stream_error(message);
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if open_result.is_some_and(|result| result.is_ok()) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return SelectedPlaylistMediaSyncOutcome::OpenedNewMedia;
        }
        SelectedPlaylistMediaSyncOutcome::NoChange
    }
}
