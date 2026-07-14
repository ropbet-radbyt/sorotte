use super::*;
use crate::app::runtime_owner::GuiPendingPlaylistSourceResolution;
use std::time::SystemTime;

use super::media_resolution::{
    GuiMediaResolutionCandidate, GuiMediaResolutionPlan, GuiMediaResolutionTarget,
};
use sorotte_plex::{
    PlexClientConfig, PlexMatchCacheStagedWrite, cache::PlexMatchCache, http::PlexHttpClient,
    is_plex_playlist_uri, library::PlexStreamTarget, parse_plex_playlist_uri, redact_plex_token,
    resolver::PlexMediaResolver,
};

enum GuiPlexStreamResolutionState {
    Ready(Option<Box<PlexStreamTarget>>),
    Pending,
    Disabled,
}

struct GuiPlaylistSourceStateUpdate<'a> {
    index: usize,
    target: &'a str,
    provider_id: GuiMediaSourceProviderId,
    status: GuiPlaylistSourceStatus,
    detail: String,
    resolution_steps: Vec<GuiPlaylistResolutionStep>,
}

impl GuiPersistedConfigRuntimeOwner {
    fn playlist_source_resolution_index_for_state(
        state: &SorotteGuiShellAppState,
        pending: &GuiPendingPlaylistSourceResolution,
    ) -> Option<usize> {
        if state
            .main_window
            .playlist
            .get(pending.index)
            .and_then(|row| normalized_editable_text(&row.label))
            .as_deref()
            == Some(pending.target.as_str())
        {
            return Some(pending.index);
        }

        state.main_window.playlist.iter().position(|row| {
            normalized_editable_text(&row.label).as_deref() == Some(pending.target.as_str())
        })
    }

    pub(in crate::app::runtime_owner) fn retry_pending_playlist_source_resolution(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(pending) = self.pending_playlist_source_resolution.clone() else {
            return false;
        };
        let Some(index) =
            Self::playlist_source_resolution_index_for_state(projected_state, &pending)
        else {
            self.pending_playlist_source_resolution = None;
            return false;
        };
        let provider_id = pending.provider_id.clone();
        if index != pending.index {
            self.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
                index,
                target: pending.target,
                provider_id: provider_id.clone(),
            });
        }
        self.handle_resolve_playlist_source_request(handle, projected_state, index, provider_id)
    }

    pub(in crate::app::runtime_owner) fn clear_pending_playlist_source_resolution_for_provider(
        &mut self,
        provider_id: &GuiMediaSourceProviderId,
    ) {
        if self
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| &pending.provider_id == provider_id)
        {
            self.pending_playlist_source_resolution = None;
        }
    }

    fn record_playlist_source_resolution_status(
        &mut self,
        index: usize,
        target: &str,
        provider_id: &GuiMediaSourceProviderId,
        status: GuiPlaylistSourceStatus,
    ) {
        if status == GuiPlaylistSourceStatus::Pending {
            self.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
                index,
                target: target.to_owned(),
                provider_id: provider_id.clone(),
            });
            return;
        }

        if self
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.index == index && pending.target == target)
        {
            self.pending_playlist_source_resolution = None;
        }
    }

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
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<Option<String>, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(None);
        };
        if let Some(path) = self.local_shared_playlist_media_path_for_target(state, &target) {
            return Ok(Some(path));
        }
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
        let playback = ClientConfig::resolve(&settings).config.playback;
        for root in playback.media_search_directories {
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

    fn resolve_main_window_user_media_target_local_only(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(GuiUserMediaTargetResolution::Missing);
        };
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
        staged_cache_write: Option<Result<PlexMatchCacheStagedWrite, String>>,
    ) -> Result<(), String> {
        debug_assert!(
            self.plex_sync_rx.is_none(),
            "stream cache results must not apply while watch sync owns the Plex engine"
        );
        let mut engine = self.take_plex_sync_engine(config)?;
        *engine.cache_mut() = cache;
        self.plex_sync_engine = Some(engine);
        let cache_save_error = staged_cache_write.and_then(|staged| match staged {
            Ok(staged) => staged
                .commit()
                .err()
                .map(|error| format!("Failed to commit Plex match cache: {error}")),
            Err(error) => Some(error),
        });
        if let Some(error) = cache_save_error {
            eprintln!("warning: {error}");
        }
        Ok(())
    }

    pub(in crate::app::runtime_owner) fn pump_plex_stream_resolution_worker(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> bool {
        let Some(rx) = self.plex_stream_resolve_rx.take() else {
            return false;
        };
        let current_context =
            self.plex_operation_context(&state.configuration.to_stored_settings());
        match rx.try_recv() {
            Ok(result) => {
                if self.plex_stream_resolve_trigger_key.as_deref()
                    == Some(result.trigger_key.as_str())
                    && result.operation_context == current_context
                    && self.plex_stream_resolve_context.as_ref() == Some(&current_context)
                {
                    self.plex_stream_resolve_trigger_key = None;
                    self.plex_stream_resolve_context = None;
                    self.plex_stream_resolve_result = Some(result);
                    self.last_attached_media_resolution_trigger = None;
                    return true;
                }
                self.plex_stream_resolve_trigger_key = None;
                self.plex_stream_resolve_context = None;
                false
            }
            Err(TryRecvError::Empty) => {
                self.plex_stream_resolve_rx = Some(rx);
                false
            }
            Err(TryRecvError::Disconnected) => {
                let operation_context = self.plex_stream_resolve_context.take();
                if let (Some(trigger_key), Some(operation_context)) = (
                    self.plex_stream_resolve_trigger_key.take(),
                    operation_context,
                ) && operation_context == current_context
                {
                    self.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
                        operation_context,
                        trigger_key,
                        result: Err(
                            "Plex stream resolution worker stopped before returning a result."
                                .to_owned(),
                        ),
                        staged_cache_write: None,
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
        self.plex_stream_resolve_context = None;
        self.plex_stream_resolve_result = None;
    }

    pub(in crate::app::runtime_owner) fn discard_unconsumed_plex_stream_resolution_result(
        &mut self,
    ) {
        self.plex_stream_resolve_result = None;
    }

    pub(in crate::app::runtime_owner) fn plex_stream_resolution_owns_cache_snapshot(&self) -> bool {
        self.plex_stream_resolve_rx.is_some()
            || self.plex_stream_resolve_trigger_key.is_some()
            || self.plex_stream_resolve_context.is_some()
            || self.plex_stream_resolve_result.is_some()
    }

    pub(in crate::app::runtime_owner) fn take_plex_stream_resolution_waiting_for_sync(
        &mut self,
    ) -> bool {
        if self.plex_stream_resolve_rx.is_some() || self.plex_stream_resolve_result.is_some() {
            return false;
        }
        let waiting = self.plex_stream_resolve_trigger_key.is_some()
            || self.plex_stream_resolve_context.is_some();
        if waiting {
            self.plex_stream_resolve_trigger_key = None;
            self.plex_stream_resolve_context = None;
        }
        waiting
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
        let operation_context =
            self.plex_operation_context(&state.configuration.to_stored_settings());
        let trigger_key = Self::plex_stream_resolution_trigger_key(&config, target);

        if self.plex_stream_resolve_context.as_ref() != Some(&operation_context)
            && (self.plex_stream_resolve_rx.is_some()
                || self.plex_stream_resolve_trigger_key.is_some())
        {
            self.clear_plex_stream_resolution_state();
        }

        if self
            .plex_stream_resolve_result
            .as_ref()
            .is_some_and(|result| {
                result.trigger_key != trigger_key || result.operation_context != operation_context
            })
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
            let GuiPlexStreamResolveWorkerResult {
                result,
                staged_cache_write,
                ..
            } = result;
            let outcome = result?;
            self.apply_plex_stream_resolution_cache(config, outcome.cache, staged_cache_write)?;
            return Ok(GuiPlexStreamResolutionState::Ready(
                outcome.stream_target.map(Box::new),
            ));
        }

        if self.plex_stream_resolve_rx.is_some() {
            return Ok(GuiPlexStreamResolutionState::Pending);
        }

        if self.plex_sync_rx.is_some() {
            self.plex_stream_resolve_trigger_key = Some(trigger_key);
            self.plex_stream_resolve_context = Some(operation_context);
            return Ok(GuiPlexStreamResolutionState::Pending);
        }

        let engine = self.take_plex_sync_engine(config.clone())?;
        let cache = engine.cache().clone();
        self.plex_sync_engine = Some(engine);
        let client = self.ensure_plex_client()?.clone();
        let cache_before = cache.clone();
        let cache_path = self.plex_cache_path();
        let worker_target = target.to_owned();
        let worker_trigger_key = trigger_key.clone();
        let worker_operation_context = operation_context.clone();
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
                let staged_cache_write = result.as_ref().ok().and_then(|outcome| {
                    if outcome.cache == cache_before {
                        return None;
                    }
                    cache_path.map(|path| {
                        outcome.cache.stage_to_path(&path).map_err(|error| {
                            format!(
                                "Failed to stage Plex match cache after stream resolution: {error}"
                            )
                        })
                    })
                });
                let _ = tx.send(GuiPlexStreamResolveWorkerResult {
                    operation_context: worker_operation_context,
                    trigger_key: worker_trigger_key,
                    result,
                    staged_cache_write,
                });
            })
            .map_err(|error| format!("Failed to start Plex stream resolution worker: {error}"))?;
        self.plex_stream_resolve_rx = Some(rx);
        self.plex_stream_resolve_trigger_key = Some(trigger_key);
        self.plex_stream_resolve_context = Some(operation_context);
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
        let Some((playlist_index, target)) = self.current_shared_playlist_index_and_target(state)
        else {
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
        let source_override =
            Self::selected_playlist_source_override_for_index(state, playlist_index);
        let source_provider = source_override
            .as_ref()
            .map(|provider_id| provider_id.as_str())
            .unwrap_or("automatic");

        // Reconcile retained drag/drop paths before the resolution trigger short-circuit.
        // Removing a dropped file must invalidate the previous local-first decision so
        // Automatic can continue through media search and Plex fallback.
        let retained_local_path =
            self.local_shared_playlist_media_path_for_target(state, plan.target());

        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let trigger = self.automatic_media_resolution_trigger(
            plan.target(),
            source_provider,
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

        if let Some(provider_id) = source_override.as_ref() {
            if !self.preflight_room_stream_target(state, plan.target()) {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            return self.sync_selected_playlist_source_override_to_attached_player(
                state,
                plan.target(),
                provider_id,
            );
        }

        if let Some(path) = retained_local_path {
            self.clear_plex_stream_resolution_state();
            plan.push_user_media_candidate(path, GuiUserMediaTargetResolutionSource::QuickLocal);
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            return self.open_media_resolution_candidate(
                plan.target(),
                plan.best_candidate()
                    .cloned()
                    .expect("retained local-path candidate should exist"),
                false,
            );
        }

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
                } else if self.media_match_remote_lookup_pending_for_target(state, plan.target()) {
                    plan.record_pending_media_match();
                }
            }
            Ok(GuiUserMediaTargetResolution::Missing) | Err(_) => {
                if let Some(path) =
                    self.media_match_cached_room_candidate_for_target(state, plan.target())
                {
                    plan.push_media_match_candidate(path);
                } else if self.media_match_remote_lookup_pending_for_target(state, plan.target()) {
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

    fn selected_playlist_source_override_for_index(
        state: &SorotteGuiShellAppState,
        index: usize,
    ) -> Option<GuiMediaSourceProviderId> {
        let source_state = &state.main_window.playlist.get(index)?.source_state;
        source_state
            .provider_selection_is_explicit
            .then(|| source_state.current_provider_id.clone())
    }

    pub(super) fn sync_selected_playlist_source_override_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
        provider_id: &GuiMediaSourceProviderId,
    ) -> SelectedPlaylistMediaSyncOutcome {
        if provider_id == &GuiMediaSourceProviderId::local() {
            return self.sync_selected_local_playlist_source_to_attached_player(state, target);
        }
        if provider_id == &GuiMediaSourceProviderId::media_matching() {
            return self
                .sync_selected_media_match_playlist_source_to_attached_player(state, target);
        }
        if provider_id == &GuiMediaSourceProviderId::plex_stream() {
            return self
                .sync_selected_plex_stream_playlist_source_to_attached_player(state, target);
        }
        SelectedPlaylistMediaSyncOutcome::NoChange
    }

    fn sync_selected_local_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let mut plan = GuiMediaResolutionPlan::new(target);
        match self.resolve_main_window_user_media_target_local_only(state, plan.target()) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, source }) => {
                plan.push_user_media_candidate(path, source);
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                plan.record_pending_media_search();
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            Ok(GuiUserMediaTargetResolution::Missing) | Err(_) => {
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

    fn sync_selected_media_match_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let mut local_plan = GuiMediaResolutionPlan::new(target);
        match self.resolve_main_window_user_media_target_local_only(state, local_plan.target()) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, source }) => {
                local_plan.push_user_media_candidate(path, source);
                let Some(candidate) = local_plan.best_candidate().cloned() else {
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                };
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                }
                return self.open_media_resolution_candidate(local_plan.target(), candidate, false);
            }
            Ok(GuiUserMediaTargetResolution::Pending)
            | Ok(GuiUserMediaTargetResolution::Missing)
            | Err(_) => {}
        }

        if !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
            || !state.media_match.settings.fingerprinting_enabled
        {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }

        let Some(path) = self.media_match_cached_room_candidate_for_target(state, target) else {
            let _ = self.media_match_remote_lookup_pending_for_target(state, target);
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let mut plan = GuiMediaResolutionPlan::new(target);
        plan.push_media_match_candidate(path);
        let Some(candidate) = plan.best_candidate().cloned() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(plan.target(), candidate, false)
    }

    fn sync_selected_plex_stream_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let mut plan = GuiMediaResolutionPlan::new(target);
        match self.cached_or_queue_plex_stream_target_for_media_target(state, plan.target()) {
            Ok(GuiPlexStreamResolutionState::Ready(Some(stream_target))) => {
                plan.push_plex_stream_candidate(*stream_target);
            }
            Ok(
                GuiPlexStreamResolutionState::Ready(None)
                | GuiPlexStreamResolutionState::Disabled
                | GuiPlexStreamResolutionState::Pending,
            ) => {
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

    pub(in crate::app::runtime_owner) fn handle_resolve_playlist_source_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
        provider_id: GuiMediaSourceProviderId,
    ) -> bool {
        let Some(target) = projected_state
            .main_window
            .playlist
            .get(index)
            .and_then(|row| normalized_editable_text(&row.label))
        else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "No playlist row exists at the requested index.".to_owned(),
            );
            return false;
        };

        if provider_id == GuiMediaSourceProviderId::local() {
            return self.resolve_playlist_source_local(handle, projected_state, index, &target);
        }
        if provider_id == GuiMediaSourceProviderId::media_matching() {
            return self.resolve_playlist_source_media_match(
                handle,
                projected_state,
                index,
                &target,
            );
        }
        if provider_id == GuiMediaSourceProviderId::plex_stream() {
            return self.resolve_playlist_source_plex_stream(
                handle,
                projected_state,
                index,
                &target,
            );
        }

        self.publish_playlist_source_state(
            handle,
            projected_state,
            GuiPlaylistSourceStateUpdate {
                index,
                target: &target,
                provider_id,
                status: GuiPlaylistSourceStatus::Disabled,
                detail: "The requested playlist source is not registered.".to_owned(),
                resolution_steps: vec![],
            },
        );
        true
    }

    fn resolve_playlist_source_local(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
        target: &str,
    ) -> bool {
        let provider_id = GuiMediaSourceProviderId::local();
        match self.resolve_main_window_user_media_target_local_only(projected_state, target) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, source }) => {
                let mut plan = GuiMediaResolutionPlan::new(target);
                plan.push_user_media_candidate(path.clone(), source);
                let Some(candidate) = plan.best_candidate().cloned() else {
                    return true;
                };
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    self.publish_playlist_source_state(
                        handle,
                        projected_state,
                        GuiPlaylistSourceStateUpdate {
                            index,
                            target,
                            provider_id,
                            status: GuiPlaylistSourceStatus::Failed,
                            detail: "No attached player is available for the resolved local file."
                                .to_owned(),
                            resolution_steps: vec![Self::playlist_resolution_step(
                                GuiMediaSourceProviderId::local(),
                                "Local",
                                GuiPlaylistSourceStatus::Failed,
                                Some("Resolved locally, but no player is attached.".to_owned()),
                            )],
                        },
                    );
                    return true;
                }
                let outcome = self.open_media_resolution_candidate(target, candidate, true);
                let (status, detail) = if outcome.selection_ready() {
                    (
                        GuiPlaylistSourceStatus::Active,
                        format!("Loaded local target: {path}."),
                    )
                } else {
                    (
                        GuiPlaylistSourceStatus::Failed,
                        format!("Resolved local target but the player did not load it: {path}."),
                    )
                };
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status,
                        detail: detail.clone(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::local(),
                            "Local",
                            status,
                            Some(detail),
                        )],
                    },
                );
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Pending,
                        detail: "Local media-search index is still resolving this entry."
                            .to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::local(),
                            "Local",
                            GuiPlaylistSourceStatus::Pending,
                            Some("Waiting for local media-search index refresh.".to_owned()),
                        )],
                    },
                );
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Missing,
                        detail: "No local file matched this playlist entry.".to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::local(),
                            "Local",
                            GuiPlaylistSourceStatus::Missing,
                            Some(
                                "Checked direct, current-player, and indexed local paths."
                                    .to_owned(),
                            ),
                        )],
                    },
                );
            }
            Err(message) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Failed,
                        detail: message.clone(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::local(),
                            "Local",
                            GuiPlaylistSourceStatus::Failed,
                            Some(message),
                        )],
                    },
                );
            }
        }
        true
    }

    fn resolve_playlist_source_media_match(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
        target: &str,
    ) -> bool {
        match self.resolve_main_window_user_media_target_local_only(projected_state, target) {
            Ok(GuiUserMediaTargetResolution::Resolved { .. }) => {
                return self.resolve_playlist_source_local(handle, projected_state, index, target);
            }
            Ok(GuiUserMediaTargetResolution::Pending)
            | Ok(GuiUserMediaTargetResolution::Missing)
            | Err(_) => {}
        }

        let provider_id = GuiMediaSourceProviderId::media_matching();
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
            || !projected_state.media_match.settings.fingerprinting_enabled
        {
            self.publish_playlist_source_state(
                handle,
                projected_state,
                GuiPlaylistSourceStateUpdate {
                    index,
                    target,
                    provider_id,
                    status: GuiPlaylistSourceStatus::Disabled,
                    detail: "Media Matching is disabled for this client.".to_owned(),
                    resolution_steps: vec![Self::playlist_resolution_step(
                        GuiMediaSourceProviderId::media_matching(),
                        "Media Matching",
                        GuiPlaylistSourceStatus::Disabled,
                        Some("Enable the plugin and fingerprinting to use this source.".to_owned()),
                    )],
                },
            );
            return true;
        }

        if let Some(path) =
            self.media_match_cached_room_candidate_for_target(projected_state, target)
        {
            let mut plan = GuiMediaResolutionPlan::new(target);
            plan.push_media_match_candidate(path.clone());
            let Some(candidate) = plan.best_candidate().cloned() else {
                return true;
            };
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Failed,
                        detail: "No attached player is available for the Media Matching result."
                            .to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::media_matching(),
                            "Media Matching",
                            GuiPlaylistSourceStatus::Failed,
                            Some("Resolved a match, but no player is attached.".to_owned()),
                        )],
                    },
                );
                return true;
            }
            let outcome = self.open_media_resolution_candidate(target, candidate, true);
            let (status, detail) = if outcome.selection_ready() {
                (
                    GuiPlaylistSourceStatus::Active,
                    format!("Loaded Media Matching target: {path}."),
                )
            } else {
                (
                    GuiPlaylistSourceStatus::Failed,
                    format!(
                        "Resolved Media Matching target but the player did not load it: {path}."
                    ),
                )
            };
            self.publish_playlist_source_state(
                handle,
                projected_state,
                GuiPlaylistSourceStateUpdate {
                    index,
                    target,
                    provider_id,
                    status,
                    detail: detail.clone(),
                    resolution_steps: vec![Self::playlist_resolution_step(
                        GuiMediaSourceProviderId::media_matching(),
                        "Media Matching",
                        status,
                        Some(detail),
                    )],
                },
            );
        } else if self.media_match_remote_lookup_pending_for_target(projected_state, target) {
            self.publish_playlist_source_state(
                handle,
                projected_state,
                GuiPlaylistSourceStateUpdate {
                    index,
                    target,
                    provider_id,
                    status: GuiPlaylistSourceStatus::Pending,
                    detail: "Media Matching lookup is running.".to_owned(),
                    resolution_steps: vec![Self::playlist_resolution_step(
                        GuiMediaSourceProviderId::media_matching(),
                        "Media Matching",
                        GuiPlaylistSourceStatus::Pending,
                        Some("Waiting for the Media Matching worker.".to_owned()),
                    )],
                },
            );
        } else {
            self.publish_playlist_source_state(
                handle,
                projected_state,
                GuiPlaylistSourceStateUpdate {
                    index,
                    target,
                    provider_id,
                    status: GuiPlaylistSourceStatus::Missing,
                    detail: "Media Matching did not find a usable target.".to_owned(),
                    resolution_steps: vec![Self::playlist_resolution_step(
                        GuiMediaSourceProviderId::media_matching(),
                        "Media Matching",
                        GuiPlaylistSourceStatus::Missing,
                        Some("No cached or worker match is available for this entry.".to_owned()),
                    )],
                },
            );
        }
        true
    }

    fn resolve_playlist_source_plex_stream(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
        target: &str,
    ) -> bool {
        let provider_id = GuiMediaSourceProviderId::plex_stream();
        match self.cached_or_queue_plex_stream_target_for_media_target(projected_state, target) {
            Ok(GuiPlexStreamResolutionState::Ready(Some(stream_target))) => {
                let mut plan = GuiMediaResolutionPlan::new(target);
                plan.push_plex_stream_candidate(*stream_target);
                let Some(candidate) = plan.best_candidate().cloned() else {
                    return true;
                };
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    self.publish_playlist_source_state(
                        handle,
                        projected_state,
                        GuiPlaylistSourceStateUpdate {
                            index,
                            target,
                            provider_id,
                            status: GuiPlaylistSourceStatus::Failed,
                            detail: "No attached player is available for the Plex stream."
                                .to_owned(),
                            resolution_steps: vec![Self::playlist_resolution_step(
                                GuiMediaSourceProviderId::plex_stream(),
                                "Plex Stream",
                                GuiPlaylistSourceStatus::Failed,
                                Some("Resolved a stream, but no player is attached.".to_owned()),
                            )],
                        },
                    );
                    return true;
                }
                let outcome = self.open_media_resolution_candidate(target, candidate, true);
                let (status, detail) = if outcome.selection_ready() {
                    (
                        GuiPlaylistSourceStatus::Active,
                        "Loaded Plex stream target.".to_owned(),
                    )
                } else {
                    (
                        GuiPlaylistSourceStatus::Failed,
                        "Resolved Plex stream target but the player did not load it.".to_owned(),
                    )
                };
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status,
                        detail: detail.clone(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::plex_stream(),
                            "Plex Stream",
                            status,
                            Some(detail),
                        )],
                    },
                );
            }
            Ok(GuiPlexStreamResolutionState::Pending) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Pending,
                        detail: "Plex stream resolution is running.".to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::plex_stream(),
                            "Plex Stream",
                            GuiPlaylistSourceStatus::Pending,
                            Some("Waiting for the Plex stream worker.".to_owned()),
                        )],
                    },
                );
            }
            Ok(GuiPlexStreamResolutionState::Ready(None)) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Missing,
                        detail: "Plex did not find a stream target for this entry.".to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::plex_stream(),
                            "Plex Stream",
                            GuiPlaylistSourceStatus::Missing,
                            Some("Plex returned no streamable match.".to_owned()),
                        )],
                    },
                );
            }
            Ok(GuiPlexStreamResolutionState::Disabled) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Disabled,
                        detail: "Plex streaming is disabled or not configured.".to_owned(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::plex_stream(),
                            "Plex Stream",
                            GuiPlaylistSourceStatus::Disabled,
                            Some("Enable Plex streaming and select a server if needed.".to_owned()),
                        )],
                    },
                );
            }
            Err(message) => {
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Failed,
                        detail: message.clone(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::plex_stream(),
                            "Plex Stream",
                            GuiPlaylistSourceStatus::Failed,
                            Some(message),
                        )],
                    },
                );
            }
        }
        true
    }

    fn publish_playlist_source_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        update: GuiPlaylistSourceStateUpdate<'_>,
    ) {
        let GuiPlaylistSourceStateUpdate {
            index,
            target,
            provider_id,
            status,
            detail,
            resolution_steps,
        } = update;
        let mut source_state = projected_state
            .main_window
            .playlist
            .get(index)
            .map(|row| row.source_state.clone())
            .unwrap_or_else(|| projected_state.playlist_source_state_for_entry(target));
        self.record_playlist_source_resolution_status(index, target, &provider_id, status);
        source_state.current_provider_id = provider_id;
        source_state.status = status;
        source_state.detail = Some(detail);
        source_state.resolution_steps = resolution_steps;
        let _ = projected_state.set_playlist_source_state(index, source_state);
        handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
        ));
    }

    fn playlist_resolution_step(
        provider_id: GuiMediaSourceProviderId,
        label: &str,
        status: GuiPlaylistSourceStatus,
        detail: Option<String>,
    ) -> GuiPlaylistResolutionStep {
        GuiPlaylistResolutionStep {
            provider_id,
            label: label.to_owned(),
            status,
            detail,
        }
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

#[cfg(test)]
mod plex_cache_coordination_tests {
    use std::{collections::BTreeMap, sync::mpsc};

    use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
    use sorotte_plex::{PlexCachedMatch, PlexMediaType, PlexSyncStatus};

    use super::*;
    use crate::app::runtime_owner::GuiPlexSyncWorkerResult;

    fn streaming_settings() -> StoredClientSettingsMvp {
        StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_sync_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        }
    }

    fn cache_with_rating_key(rating_key: &str) -> PlexMatchCache {
        PlexMatchCache {
            entries: BTreeMap::from([(
                "server:id:machine:path:movie.mkv".to_owned(),
                PlexCachedMatch {
                    rating_key: rating_key.to_owned(),
                    title: format!("Movie {rating_key}"),
                    media_type: PlexMediaType::Movie,
                    duration_millis: Some(90_000),
                },
            )]),
        }
    }

    #[test]
    fn sync_and_stream_resolution_handoff_the_engine_without_competing_workers() {
        let settings = streaming_settings();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let config = super::super::super::plex::plex_config_from_settings(&settings);
        let sync_engine = owner
            .take_plex_sync_engine(config)
            .expect("test sync engine should be created");
        let operation_context = owner.plex_operation_context(&settings);
        let (sync_tx, sync_rx) = mpsc::channel::<GuiPlexSyncWorkerResult>();
        owner.plex_sync_rx = Some(sync_rx);

        let resolution = owner
            .cached_or_queue_plex_stream_target_for_media_target(&state, "plex://")
            .expect("Plex stream resolution should defer without failing");

        assert!(matches!(resolution, GuiPlexStreamResolutionState::Pending));
        assert!(owner.plex_stream_resolve_rx.is_none());
        assert!(owner.plex_sync_engine.is_none());
        assert!(owner.plex_stream_resolution_owns_cache_snapshot());

        sync_tx
            .send(GuiPlexSyncWorkerResult {
                operation_context,
                engine: sync_engine,
                status: PlexSyncStatus::ready(),
                staged_cache_write: None,
            })
            .expect("watch-sync completion should queue");
        assert!(owner.sync_plex_watch_state(&handle, &mut state));
        assert!(owner.plex_sync_rx.is_none());
        assert!(owner.plex_sync_engine.is_some());
        assert!(owner.take_plex_stream_resolution_waiting_for_sync());
        assert!(!owner.plex_stream_resolution_owns_cache_snapshot());

        let retry = owner
            .cached_or_queue_plex_stream_target_for_media_target(&state, "plex://")
            .expect("deferred stream resolution should retry after sync completion");
        assert!(matches!(retry, GuiPlexStreamResolutionState::Pending));
        assert!(owner.plex_stream_resolve_rx.is_some());
        assert!(owner.plex_sync_engine.is_some());
        assert!(owner.plex_sync_rx.is_none());

        owner.plex_sync_next_tick_due_at = None;
        assert!(!owner.sync_plex_watch_state(&handle, &mut state));
        assert!(
            owner.plex_sync_rx.is_none(),
            "watch sync must stay suspended until the stream snapshot applies"
        );
    }

    #[test]
    fn accepted_stream_result_commits_its_prepared_cache_only_when_consumed() {
        let sequence = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-gui-plex-stream-stage-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let settings = streaming_settings();
        let state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.ini")));
        let cache_path = owner
            .plex_cache_path()
            .expect("configured owner should provide a Plex cache path");
        let original = cache_with_rating_key("original");
        let replacement = cache_with_rating_key("replacement");
        original
            .save_to_path(&cache_path)
            .expect("original cache should persist");
        let staged_cache_write = replacement
            .stage_to_path(&cache_path)
            .expect("replacement cache should stage off the owner thread");

        let config = super::super::super::plex::plex_config_from_settings(&settings);
        let target = "plex://machine/library/metadata/movie";
        let trigger_key =
            GuiPersistedConfigRuntimeOwner::plex_stream_resolution_trigger_key(&config, target);
        let operation_context = owner.plex_operation_context(&settings);
        let (result_tx, result_rx) = mpsc::channel();
        result_tx
            .send(GuiPlexStreamResolveWorkerResult {
                operation_context: operation_context.clone(),
                trigger_key: trigger_key.clone(),
                result: Ok(GuiPlexStreamResolveOutcome {
                    stream_target: None,
                    cache: replacement.clone(),
                }),
                staged_cache_write: Some(Ok(staged_cache_write)),
            })
            .expect("prepared stream result should queue");
        owner.plex_stream_resolve_rx = Some(result_rx);
        owner.plex_stream_resolve_trigger_key = Some(trigger_key);
        owner.plex_stream_resolve_context = Some(operation_context);

        assert!(owner.pump_plex_stream_resolution_worker(&state));
        assert_eq!(
            PlexMatchCache::load_from_path(&cache_path)
                .expect("original cache should remain readable before consumption"),
            original,
            "receiving the worker result must not replace the accepted cache yet"
        );

        let resolution = owner
            .cached_or_queue_plex_stream_target_for_media_target(&state, target)
            .expect("current-context stream result should apply");
        assert!(matches!(
            resolution,
            GuiPlexStreamResolutionState::Ready(None)
        ));
        assert_eq!(
            PlexMatchCache::load_from_path(&cache_path)
                .expect("accepted replacement cache should remain readable"),
            replacement
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unconsumed_stream_result_releases_its_cache_snapshot_and_staged_temp() {
        let sequence = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-gui-plex-stream-orphan-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let cache_path = root.join("plex-match-cache.json");
        let original = cache_with_rating_key("original");
        let replacement = cache_with_rating_key("orphan");
        original
            .save_to_path(&cache_path)
            .expect("original cache should persist");
        let staged_cache_write = replacement
            .stage_to_path(&cache_path)
            .expect("orphan cache should stage off the owner thread");
        let settings = streaming_settings();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
            operation_context: owner.plex_operation_context(&settings),
            trigger_key: "orphan-trigger".to_owned(),
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: None,
                cache: replacement,
            }),
            staged_cache_write: Some(Ok(staged_cache_write)),
        });
        assert!(owner.plex_stream_resolution_owns_cache_snapshot());
        assert!(
            std::fs::read_dir(&root)
                .expect("cache directory should remain readable")
                .count()
                > 1,
            "the prepared replacement should exist until the retry window closes"
        );

        owner.discard_unconsumed_plex_stream_resolution_result();

        assert!(!owner.plex_stream_resolution_owns_cache_snapshot());
        assert_eq!(
            PlexMatchCache::load_from_path(&cache_path)
                .expect("unconsumed result must preserve the accepted cache"),
            original
        );
        assert_eq!(
            std::fs::read_dir(&root)
                .expect("cache directory should remain readable")
                .count(),
            1,
            "dropping the unconsumed staged write should remove its temporary file"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
