use super::*;
use std::time::SystemTime;

use super::media_resolution::{
    GuiMediaResolutionCandidate, GuiMediaResolutionPlan, GuiMediaResolutionTarget,
};
use sorotte_plex::{
    PlexClientConfig, PlexHttpClient, PlexMatchCache, PlexMediaResolver, PlexStreamTarget,
    is_plex_playlist_uri, parse_plex_playlist_uri, redact_plex_token,
};

enum GuiPlexStreamResolutionState {
    Ready(Option<Box<PlexStreamTarget>>),
    Pending,
    Disabled,
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn local_media_search_candidates_for_target(
        target: &str,
    ) -> Vec<String> {
        let mut candidates = Vec::new();
        if is_plex_playlist_uri(target) {
            if let Ok(uri) = parse_plex_playlist_uri(target) {
                if let Some(file_name) = uri.file_name
                    && let Some(name) = Path::new(&file_name)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                {
                    candidates.push(name.to_owned());
                }
                if let Some(title) = uri.title
                    && !title.trim().is_empty()
                {
                    candidates.push(title.trim().to_owned());
                }
            }
        } else {
            candidates.push(target.to_owned());
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }

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
        let target_candidates = Self::local_media_search_candidates_for_target(&target);
        if target_candidates.is_empty() {
            return Ok(None);
        }

        for target_candidate in &target_candidates {
            let target_path = Path::new(target_candidate);
            if let Some(path) = Self::quick_existing_media_target_path(target_path) {
                return Ok(Some(path));
            }
        }

        let target_file_names = target_candidates
            .iter()
            .filter_map(|target_candidate| {
                Path::new(target_candidate)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();

        if let Some(local_path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
        {
            let local_path = Path::new(local_path);
            let matches_local_file = local_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    target_candidates
                        .iter()
                        .any(|target_candidate| name.eq_ignore_ascii_case(target_candidate))
                });
            if matches_local_file && local_path.is_file() {
                return Ok(Some(local_path.to_string_lossy().into_owned()));
            }
            if let Some(parent) = local_path.parent() {
                for target_candidate in &target_candidates {
                    if let Some(path) =
                        Self::quick_existing_media_target_path(&parent.join(target_candidate))
                    {
                        return Ok(Some(path));
                    }
                }
                for file_name in &target_file_names {
                    if let Some(path) =
                        Self::quick_existing_media_target_path(&parent.join(file_name))
                    {
                        return Ok(Some(path));
                    }
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
            for target_candidate in &target_candidates {
                if let Some(path) =
                    Self::quick_existing_media_target_path(&root.join(target_candidate))
                {
                    return Ok(Some(path));
                }
            }
            for file_name in &target_file_names {
                if let Some(path) = Self::quick_existing_media_target_path(&root.join(file_name)) {
                    return Ok(Some(path));
                }
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
            if Path::new(&target).is_absolute()
                || browser_is_url(&target)
                || search_roots.is_empty()
            {
                self.cancel_pending_attached_media_search_index_build_impl();
                self.attached_media_search_next_retry_at = None;
            }
            self.unresolved_attached_media_target = None;
            return Ok(GuiUserMediaTargetResolution::Resolved {
                path,
                source: GuiUserMediaTargetResolutionSource::QuickLocal,
            });
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
        let indexed_target_candidates = Self::local_media_search_candidates_for_target(&target);
        if let Some(found_path) = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)
            .and_then(|index| {
                indexed_target_candidates
                    .iter()
                    .find_map(|candidate| self.cached_missing_media_target_path(index, candidate))
            })
        {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return Ok(GuiUserMediaTargetResolution::Resolved {
                path: found_path,
                source: GuiUserMediaTargetResolutionSource::MediaSearchIndex,
            });
        }
        if let Some(path) = self.media_match_cached_exact_inventory_candidate_for_target(
            state,
            &target,
            &search_roots,
        ) {
            self.unresolved_attached_media_target = None;
            if !self.attached_media_search_refresh_pending() {
                self.attached_media_search_next_retry_at = None;
            }
            return Ok(GuiUserMediaTargetResolution::Resolved {
                path,
                source: GuiUserMediaTargetResolutionSource::MediaMatchExactInventory,
            });
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

    fn plex_stream_resolution_config_for_target(
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<PlexClientConfig> {
        if !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return None;
        }
        let settings = state.configuration.to_stored_settings();
        let config = super::super::plex::plex_config_from_settings(&settings);
        if !config.streaming_enabled {
            return None;
        }
        let target_is_plex_uri = is_plex_playlist_uri(target);
        if !target_is_plex_uri && !config.has_selected_server() {
            return None;
        }
        Some(config)
    }

    fn plex_stream_resolution_trigger_key(config: &PlexClientConfig, target: &str) -> String {
        let mut token_hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        config.user_token.hash(&mut token_hasher);
        config.selected_server_token.hash(&mut token_hasher);
        format!(
            "{}\nstreaming={}\nserver-id={}\nserver-url={}\ntoken-hash={:016x}",
            target,
            config.streaming_enabled,
            config.selected_server_id.as_deref().unwrap_or_default(),
            config.selected_server_url.as_deref().unwrap_or_default(),
            token_hasher.finish()
        )
    }

    fn resolve_plex_stream_target_with_parts(
        config: PlexClientConfig,
        client: PlexHttpClient,
        cache: PlexMatchCache,
        target: &str,
    ) -> Result<(Option<PlexStreamTarget>, PlexMatchCache), String> {
        let mut resolver = PlexMediaResolver::new(config, client, cache);
        let result = resolver
            .resolve_stream_target(target, SystemTime::now())
            .map_err(|error| {
                redact_plex_token(&format!(
                    "Resolving Plex stream target for '{target}' failed: {error}"
                ))
            })?;
        let (_, _, cache) = resolver.into_parts();
        Ok((result, cache))
    }

    fn apply_plex_stream_resolution_cache(
        &mut self,
        config: PlexClientConfig,
        cache: PlexMatchCache,
    ) -> Result<(), String> {
        let mut engine = self.take_plex_sync_engine(config)?;
        let cache_changed = engine.cache() != &cache;
        *engine.cache_mut() = cache.clone();
        self.plex_sync_engine = Some(engine);
        if cache_changed
            && let Some(cache_path) = self.plex_cache_path()
            && let Err(error) = cache.save_to_path(&cache_path)
        {
            eprintln!("warning: failed to save Plex match cache after stream resolution: {error}");
        }
        Ok(())
    }

    pub(in crate::app::runtime_owner) fn pump_plex_stream_resolution_worker(&mut self) -> bool {
        let Some(rx) = self.plex_stream_resolve_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                if self.plex_stream_resolve_trigger_key.as_deref()
                    == Some(result.trigger_key.as_str())
                {
                    self.plex_stream_resolve_trigger_key = None;
                    self.plex_stream_resolve_result = Some(result);
                    self.last_attached_media_resolution_trigger = None;
                    return true;
                }
                false
            }
            Err(TryRecvError::Empty) => {
                self.plex_stream_resolve_rx = Some(rx);
                false
            }
            Err(TryRecvError::Disconnected) => {
                if let Some(trigger_key) = self.plex_stream_resolve_trigger_key.take() {
                    self.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
                        trigger_key,
                        target: String::new(),
                        result: Err(
                            "Plex stream resolution worker stopped before returning a result."
                                .to_owned(),
                        ),
                    });
                    self.last_attached_media_resolution_trigger = None;
                    return true;
                }
                false
            }
        }
    }

    pub(in crate::app::runtime_owner) fn clear_plex_stream_resolution_state(&mut self) {
        self.plex_stream_resolve_rx = None;
        self.plex_stream_resolve_trigger_key = None;
        self.plex_stream_resolve_result = None;
    }

    fn cached_or_queue_plex_stream_target_for_media_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<GuiPlexStreamResolutionState, String> {
        let Some(config) = Self::plex_stream_resolution_config_for_target(state, target) else {
            self.clear_plex_stream_resolution_state();
            return Ok(GuiPlexStreamResolutionState::Disabled);
        };
        let trigger_key = Self::plex_stream_resolution_trigger_key(&config, target);

        if self
            .plex_stream_resolve_result
            .as_ref()
            .is_some_and(|result| result.trigger_key != trigger_key)
        {
            self.plex_stream_resolve_result = None;
        }

        if self
            .plex_stream_resolve_result
            .as_ref()
            .is_some_and(|result| result.trigger_key == trigger_key)
        {
            let result = self
                .plex_stream_resolve_result
                .take()
                .expect("checked plex stream resolve result should exist");
            let outcome = result.result?;
            self.apply_plex_stream_resolution_cache(config, outcome.cache)?;
            return Ok(GuiPlexStreamResolutionState::Ready(
                outcome.stream_target.map(Box::new),
            ));
        }

        if self.plex_stream_resolve_rx.is_some() {
            return Ok(GuiPlexStreamResolutionState::Pending);
        }

        let engine = self.take_plex_sync_engine(config.clone())?;
        let cache = engine.cache().clone();
        self.plex_sync_engine = Some(engine);
        let client = self.ensure_plex_client()?.clone();
        let worker_target = target.to_owned();
        let worker_trigger_key = trigger_key.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("sorotte-gui-plex-stream-resolve".to_owned())
            .spawn(move || {
                let result = Self::resolve_plex_stream_target_with_parts(
                    config,
                    client,
                    cache,
                    &worker_target,
                )
                .map(|(stream_target, cache)| GuiPlexStreamResolveOutcome {
                    stream_target,
                    cache,
                });
                let _ = tx.send(GuiPlexStreamResolveWorkerResult {
                    trigger_key: worker_trigger_key,
                    target: worker_target,
                    result,
                });
            })
            .map_err(|error| format!("Failed to start Plex stream resolution worker: {error}"))?;
        self.plex_stream_resolve_rx = Some(rx);
        self.plex_stream_resolve_trigger_key = Some(trigger_key);
        Ok(GuiPlexStreamResolutionState::Pending)
    }

    fn open_media_resolution_candidate(
        &mut self,
        requested_target: &str,
        candidate: GuiMediaResolutionCandidate,
        user_initiated: bool,
    ) -> SelectedPlaylistMediaSyncOutcome {
        match candidate.target() {
            GuiMediaResolutionTarget::CurrentPlayer => {
                self.unresolved_attached_media_target = None;
                if !self.attached_media_search_refresh_pending() {
                    self.attached_media_search_next_retry_at = None;
                }
                SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
            }
            GuiMediaResolutionTarget::LocalPath(resolved_target) => {
                if self.current_player_matches_media_target(resolved_target) {
                    self.unresolved_attached_media_target = None;
                    if !self.attached_media_search_refresh_pending() {
                        self.attached_media_search_next_retry_at = None;
                    }
                    return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
                }

                let player_paths = [resolved_target.clone()];
                self.prepare_stream_load_tracking(&player_paths[0], user_initiated);
                let open_result =
                    self.open_media_files_through_attached_player_result_impl(&player_paths);
                if let Some(Err(message)) = open_result {
                    if user_initiated {
                        self.queue_stream_error(message);
                    } else {
                        self.queue_stream_warning(message);
                    }
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
            GuiMediaResolutionTarget::PlexStream(stream_target) => {
                let open_result = self.open_plex_stream_target_through_attached_player_result_impl(
                    requested_target,
                    stream_target.as_ref().clone(),
                    user_initiated,
                );
                if let Some(Err(message)) = open_result {
                    if user_initiated {
                        self.queue_stream_error(message);
                    } else {
                        self.queue_stream_warning(message);
                    }
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
        }
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
            self.clear_plex_stream_resolution_state();
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let mut plan = GuiMediaResolutionPlan::new(target);

        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let trigger = self.automatic_media_resolution_trigger(
            plan.target(),
            &roots,
            self.media_match_remote_resolution_token_for_state(state),
        );
        if !self.should_rerun_automatic_media_resolution(&trigger) {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent())
                && self.current_player_matches_media_target(plan.target())
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

        if self.current_player_matches_media_target(plan.target()) {
            plan.push_current_player_candidate();
            return self.open_media_resolution_candidate(
                plan.target(),
                plan.best_candidate()
                    .cloned()
                    .expect("current-player candidate should exist"),
                false,
            );
        }

        if !self.preflight_room_stream_target(state, plan.target()) {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }

        match self.resolve_main_window_user_media_target_for_automatic_sync(state, plan.target()) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, source }) => {
                plan.push_user_media_candidate(path, source);
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                plan.record_pending_media_search();
                if let Some(path) =
                    self.media_match_cached_room_candidate_for_target(state, plan.target())
                {
                    plan.push_media_match_candidate(path);
                } else if self.media_match_remote_lookup_rx.is_some() {
                    plan.record_pending_media_match();
                }
            }
            Ok(GuiUserMediaTargetResolution::Missing) | Err(_) => {
                if let Some(path) =
                    self.media_match_cached_room_candidate_for_target(state, plan.target())
                {
                    plan.push_media_match_candidate(path);
                } else if self.media_match_remote_lookup_rx.is_some() {
                    plan.record_pending_media_match();
                }
            }
        }

        if let Some(candidate) = plan.best_candidate().cloned() {
            if plan.has_pending_media_search_above(candidate.priority()) {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            return self.open_media_resolution_candidate(plan.target(), candidate, false);
        }

        match self.cached_or_queue_plex_stream_target_for_media_target(state, plan.target()) {
            Ok(GuiPlexStreamResolutionState::Ready(Some(stream_target))) => {
                plan.push_plex_stream_candidate(*stream_target);
            }
            Ok(
                GuiPlexStreamResolutionState::Ready(None) | GuiPlexStreamResolutionState::Disabled,
            ) => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            Ok(GuiPlexStreamResolutionState::Pending) => {
                plan.record_pending_plex_stream();
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            Err(message) => {
                self.queue_stream_warning(message);
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
        }

        let Some(candidate) = plan.best_candidate().cloned() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(plan.target(), candidate, false)
    }

    pub(in crate::app::runtime_owner) fn open_selected_playlist_media_path_through_attached_player_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
        player_paths: &[String],
    ) -> SelectedPlaylistMediaSyncOutcome {
        let Some(selected_path) = player_paths.first() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let mut selected_path = selected_path.clone();
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        let selected_path_is_plex_uri = is_plex_playlist_uri(&selected_path);
        if browser_stream_target_kind(&selected_path, None) == GuiStreamTargetKind::ExtractorPageUrl
            && !state
                .plugin_enablement
                .enabled_for(GuiPluginSelection::StreamSupport)
        {
            self.queue_stream_warning(
                "Stream Support is disabled; extractor-backed URLs cannot be opened until it is enabled."
                    .to_owned(),
            );
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if !self.preflight_user_stream_target(&selected_path) {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        if self.current_player_matches_media_target(&selected_path) {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.unresolved_attached_media_target = None;
            self.attached_media_search_next_retry_at = None;
            return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
        }

        if selected_path_is_plex_uri {
            match self
                .resolve_main_window_user_media_target_for_automatic_sync(state, &selected_path)
            {
                Ok(GuiUserMediaTargetResolution::Resolved { path, .. }) => {
                    selected_path = path;
                }
                Ok(GuiUserMediaTargetResolution::Pending)
                | Ok(GuiUserMediaTargetResolution::Missing)
                | Err(_) => {
                    let stream_target = match self
                        .cached_or_queue_plex_stream_target_for_media_target(state, &selected_path)
                    {
                        Ok(GuiPlexStreamResolutionState::Ready(Some(stream_target))) => {
                            *stream_target
                        }
                        Ok(
                            GuiPlexStreamResolutionState::Ready(None)
                            | GuiPlexStreamResolutionState::Disabled,
                        )
                        | Ok(GuiPlexStreamResolutionState::Pending) => {
                            return SelectedPlaylistMediaSyncOutcome::NoChange;
                        }
                        Err(message) => {
                            self.queue_stream_error(message);
                            return SelectedPlaylistMediaSyncOutcome::NoChange;
                        }
                    };
                    let open_result = self
                        .open_plex_stream_target_through_attached_player_result_impl(
                            &selected_path,
                            stream_target,
                            true,
                        );
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
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                }
            }
        }

        let player_paths = [selected_path];
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
