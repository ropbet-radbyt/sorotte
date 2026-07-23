use super::*;
use crate::app::media_match_support::{MediaAliasMatchKind, MediaMatchInventoryExactResolution};
use crate::app::runtime_owner::GuiPendingPlaylistSourceResolution;
use std::time::SystemTime;

use super::media_resolution::{
    GuiMediaResolutionCandidate, GuiMediaResolutionDecision, GuiMediaResolutionFallbackPolicy,
    GuiMediaResolutionPlan, GuiMediaResolutionTarget,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GuiLocalMediaSearchAliases {
    exact_file_name: Option<String>,
    fallback_title: Option<String>,
    direct_target: Option<String>,
}

impl GuiLocalMediaSearchAliases {
    fn for_target(target: &str) -> Self {
        if !is_plex_playlist_uri(target) {
            return Self {
                direct_target: Some(target.to_owned()),
                ..Self::default()
            };
        }

        let Ok(uri) = parse_plex_playlist_uri(target) else {
            return Self::default();
        };
        let exact_file_name = uri.file_name.and_then(|file_name| {
            Path::new(&file_name)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        });
        let fallback_title = uri
            .title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty())
            .filter(|title| exact_file_name.as_deref() != Some(title.as_str()));

        Self {
            exact_file_name,
            fallback_title,
            direct_target: None,
        }
    }

    fn ordered_candidates(&self) -> Vec<&str> {
        self.exact_file_name
            .iter()
            .chain(self.fallback_title.iter())
            .chain(self.direct_target.iter())
            .map(String::as_str)
            .collect()
    }
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
            .is_some_and(|row| row.entry_id == pending.entry_id)
        {
            return Some(pending.index);
        }

        state
            .main_window
            .playlist
            .iter()
            .position(|row| row.entry_id == pending.entry_id)
    }

    pub(in crate::app::runtime_owner) fn retry_pending_playlist_source_resolution(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(pending) = self.pending_playlist_source_resolution.clone() else {
            return false;
        };
        if pending.generation != self.playlist_resolution.generation {
            self.pending_playlist_source_resolution = None;
            return false;
        }
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
                entry_id: pending.entry_id,
                generation: pending.generation,
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
        state: &SorotteGuiShellAppState,
        index: usize,
        target: &str,
        provider_id: &GuiMediaSourceProviderId,
        status: GuiPlaylistSourceStatus,
    ) {
        self.reconcile_local_shared_playlist_media_paths(state);
        if status == GuiPlaylistSourceStatus::Pending {
            let Some(entry_id) = state
                .main_window
                .playlist
                .get(index)
                .map(|row| row.entry_id)
            else {
                return;
            };
            self.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
                index,
                entry_id,
                generation: self.playlist_resolution.generation,
                target: target.to_owned(),
                provider_id: provider_id.clone(),
            });
            return;
        }

        if self
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| {
                state
                    .main_window
                    .playlist
                    .get(index)
                    .is_some_and(|row| row.entry_id == pending.entry_id)
                    && pending.target == target
            })
        {
            self.pending_playlist_source_resolution = None;
        }
    }

    pub(in crate::app::runtime_owner) fn local_media_search_candidates_for_target(
        target: &str,
    ) -> Vec<String> {
        GuiLocalMediaSearchAliases::for_target(target)
            .ordered_candidates()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn media_paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
        let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
        let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
        Self::normalized_current_player_match_key(&left.to_string_lossy())
            == Self::normalized_current_player_match_key(&right.to_string_lossy())
    }

    fn media_alias_name_matches(left: &str, right: &str) -> bool {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }

    fn remote_media_alias_name_matches(left: &str, right: &str) -> bool {
        left.eq_ignore_ascii_case(right)
    }

    fn uncorroborated_current_player_title_collision_path(&self, target: &str) -> Option<String> {
        if !is_plex_playlist_uri(target) || self.current_player_matches_media_target(target) {
            return None;
        }
        let local_file = self.player_local_file.as_ref()?;
        let local_path = local_file.path.as_deref()?;
        let local_name = Path::new(local_path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| local_file.name.trim());
        let aliases = GuiLocalMediaSearchAliases::for_target(target);
        let exact_file_name_matches = aliases
            .exact_file_name
            .as_deref()
            .is_some_and(|file_name| Self::remote_media_alias_name_matches(local_name, file_name));
        if exact_file_name_matches {
            // Preserve the established filename + size/identity requirement in
            // `current_player_matches_media_target` for an already-open file.
            return Some(local_path.to_owned());
        }
        let fallback_title_matches = aliases
            .fallback_title
            .as_deref()
            .is_some_and(|title| Self::remote_media_alias_name_matches(local_name, title));
        if !fallback_title_matches {
            return None;
        }

        let Ok(uri) = parse_plex_playlist_uri(target) else {
            return Some(local_path.to_owned());
        };
        let local_size_bytes = local_file.size_bytes.or_else(|| {
            std::fs::metadata(local_path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
        });
        let size_matches = uri
            .size_bytes
            .zip(local_size_bytes)
            .is_some_and(|(target_size, local_size)| target_size == local_size);
        let duration_matches = uri
            .duration_millis
            .zip(local_file.duration_seconds)
            .is_some_and(|(target_millis, local_seconds)| {
                local_seconds.is_finite()
                    && ((target_millis as f64 / 1_000.0) - local_seconds).abs() <= 1.0
            });
        (!size_matches && !duration_matches).then(|| local_path.to_owned())
    }

    fn quick_existing_media_target_path(
        target: &Path,
        excluded_current_path: Option<&str>,
    ) -> Option<String> {
        if !target.is_file()
            || excluded_current_path.is_some_and(|excluded| {
                Self::media_paths_refer_to_same_file(target, Path::new(excluded))
            })
        {
            return None;
        }
        Some(target.to_string_lossy().into_owned())
    }

    fn indexed_resolution_excludes_current_player_collision(
        resolution: &GuiUserMediaTargetResolution,
        excluded_current_path: Option<&str>,
    ) -> bool {
        let GuiUserMediaTargetResolution::Resolved { path, .. } = resolution else {
            return false;
        };
        excluded_current_path.is_some_and(|excluded| {
            Self::media_paths_refer_to_same_file(Path::new(path), Path::new(excluded))
        })
    }

    fn quick_local_media_resolution(path: String) -> GuiUserMediaTargetResolution {
        GuiUserMediaTargetResolution::Resolved {
            path,
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        }
    }

    fn quick_resolve_single_media_alias(
        target_candidate: &str,
        current_local_path: Option<&Path>,
        media_search_directories: &[PathBuf],
        target_is_plex_uri: bool,
        excluded_current_path: Option<&str>,
    ) -> Option<GuiUserMediaTargetResolution> {
        let target_path = Path::new(target_candidate);
        if let Some(path) =
            Self::quick_existing_media_target_path(target_path, excluded_current_path)
        {
            return Some(Self::quick_local_media_resolution(path));
        }

        if !target_is_plex_uri
            && let Some(local_path) = current_local_path
            && local_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| Self::media_alias_name_matches(name, target_candidate))
            && local_path.is_file()
        {
            return Some(Self::quick_local_media_resolution(
                local_path.to_string_lossy().into_owned(),
            ));
        }

        let target_file_name = target_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let mut candidate_names = vec![target_candidate];
        if let Some(file_name) = target_file_name
            && file_name != target_candidate
        {
            candidate_names.push(file_name);
        }

        if let Some(parent) = current_local_path.and_then(Path::parent) {
            for candidate_name in &candidate_names {
                if let Some(path) = Self::quick_existing_media_target_path(
                    &parent.join(candidate_name),
                    excluded_current_path,
                ) {
                    return Some(Self::quick_local_media_resolution(path));
                }
            }
        }

        for candidate_name in candidate_names {
            let mut matches = media_search_directories
                .iter()
                .filter_map(|root| {
                    Self::quick_existing_media_target_path(
                        &root.join(candidate_name),
                        excluded_current_path,
                    )
                })
                .collect::<Vec<_>>();
            matches.sort_by(|left, right| {
                if cfg!(windows) {
                    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
                } else {
                    left.cmp(right)
                }
            });
            matches.dedup_by(|left, right| {
                if cfg!(windows) {
                    left.eq_ignore_ascii_case(right)
                } else {
                    left == right
                }
            });
            match matches.len() {
                0 => {}
                1 => return matches.pop().map(Self::quick_local_media_resolution),
                candidate_count => {
                    return Some(GuiUserMediaTargetResolution::Ambiguous { candidate_count });
                }
            }
        }
        None
    }

    fn case_folded_current_player_path_for_media_alias(
        &self,
        target: &str,
        target_candidate: &str,
    ) -> Option<String> {
        if cfg!(windows) || is_plex_playlist_uri(target) {
            return None;
        }
        let local_path = self
            .player_local_file
            .as_ref()?
            .path
            .as_deref()
            .map(Path::new)?;
        let local_name = local_path.file_name()?.to_str()?;
        (local_name != target_candidate
            && local_name.eq_ignore_ascii_case(target_candidate)
            && local_path.is_file())
        .then(|| local_path.to_string_lossy().into_owned())
    }

    fn quick_resolve_main_window_user_media_alias(
        &self,
        state: &SorotteGuiShellAppState,
        target: &str,
        target_candidate: &str,
        excluded_current_path: Option<&str>,
    ) -> Option<GuiUserMediaTargetResolution> {
        if browser_is_url(target) {
            return Some(Self::quick_local_media_resolution(target.to_owned()));
        }
        let current_local_path = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .map(Path::new);
        let target_is_plex_uri = is_plex_playlist_uri(target);
        if target_is_plex_uri
            && self.current_player_matches_media_target(target)
            && let Some(local_path) = current_local_path
            && local_path.is_file()
        {
            return Some(Self::quick_local_media_resolution(
                local_path.to_string_lossy().into_owned(),
            ));
        }

        let settings = self.runtime_operation_settings(state);
        let playback = ClientConfig::resolve(&settings).config.playback;
        Self::quick_resolve_single_media_alias(
            target_candidate,
            current_local_path,
            &playback.media_search_directories,
            target_is_plex_uri,
            excluded_current_path,
        )
    }

    fn resolve_main_window_user_media_target_by_evidence_class(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
        reset_retry_on_target_change: bool,
        include_exact_inventory: bool,
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
        let retry_interval = self.automatic_media_search_retry_interval(state);
        let target_aliases = GuiLocalMediaSearchAliases::for_target(&target);
        let target_candidates = target_aliases.ordered_candidates();
        let excluded_current_path =
            self.uncorroborated_current_player_title_collision_path(&target);
        let mut index_prepared = false;
        let mut build_pending = false;
        let mut deferred_case_folded_current_path = None;
        let mut deferred_case_folded_inventory_resolution = None;

        // Alias order is an evidence order, not a per-layer convenience. Exhaust
        // quick, indexed, and exact-inventory evidence for the Plex filename before
        // allowing its human-readable title to resolve or become ambiguous.
        for target_candidate in target_candidates {
            let is_exact_file_name =
                target_aliases.exact_file_name.as_deref() == Some(target_candidate);
            if deferred_case_folded_current_path.is_none() {
                deferred_case_folded_current_path =
                    self.case_folded_current_player_path_for_media_alias(&target, target_candidate);
            }
            if let Some(resolution) = self.quick_resolve_main_window_user_media_alias(
                state,
                &target,
                target_candidate,
                excluded_current_path.as_deref(),
            ) {
                match &resolution {
                    GuiUserMediaTargetResolution::Resolved { .. } => {
                        if Path::new(&target).is_absolute()
                            || browser_is_url(&target)
                            || search_roots.is_empty()
                        {
                            self.cancel_pending_attached_media_search_index_build_impl();
                            self.attached_media_search_next_retry_at = None;
                        }
                        self.unresolved_attached_media_target = None;
                    }
                    GuiUserMediaTargetResolution::Ambiguous { .. } => {
                        self.unresolved_attached_media_target = Some(target);
                    }
                    GuiUserMediaTargetResolution::Pending
                    | GuiUserMediaTargetResolution::Missing => {
                        unreachable!("quick media resolution is resolved or ambiguous")
                    }
                }
                return Ok(resolution);
            }

            if search_roots.is_empty() {
                continue;
            }
            if !index_prepared {
                self.ensure_loaded_attached_media_search_index(
                    &search_roots,
                    &roots,
                    retry_interval,
                );
                build_pending = self.poll_attached_media_search_index_build(retry_interval);
                index_prepared = true;
            }

            let indexed_resolution = self
                .attached_media_search_index
                .as_ref()
                .filter(|index| index.roots == roots)
                .and_then(|index| self.cached_missing_media_target_path(index, target_candidate))
                .filter(|resolution| {
                    !Self::indexed_resolution_excludes_current_player_collision(
                        resolution,
                        excluded_current_path.as_deref(),
                    )
                });
            if let Some(indexed_resolution) = indexed_resolution {
                match &indexed_resolution {
                    GuiUserMediaTargetResolution::Resolved { .. } => {
                        self.unresolved_attached_media_target = None;
                        if !self.attached_media_search_refresh_pending() {
                            self.attached_media_search_next_retry_at = None;
                        }
                    }
                    GuiUserMediaTargetResolution::Ambiguous { .. } => {
                        self.unresolved_attached_media_target = Some(target);
                    }
                    GuiUserMediaTargetResolution::Pending
                    | GuiUserMediaTargetResolution::Missing => {
                        unreachable!("cached media-index matches are resolved or ambiguous")
                    }
                }
                return Ok(indexed_resolution);
            }

            if include_exact_inventory
                && let Some(inventory_resolution) = self
                    .media_match_cached_exact_inventory_resolution_for_target(
                        state,
                        target_candidate,
                        &search_roots,
                    )
            {
                let match_kind = match &inventory_resolution {
                    MediaMatchInventoryExactResolution::Resolved { match_kind, .. }
                    | MediaMatchInventoryExactResolution::Ambiguous { match_kind, .. } => {
                        *match_kind
                    }
                };
                let is_folded_match = match_kind == MediaAliasMatchKind::FoldedCase;
                match inventory_resolution {
                    MediaMatchInventoryExactResolution::Resolved { path, .. }
                        if !excluded_current_path.as_deref().is_some_and(|excluded| {
                            Self::media_paths_refer_to_same_file(
                                Path::new(&path),
                                Path::new(excluded),
                            )
                        }) && !deferred_case_folded_current_path.as_deref().is_some_and(
                            |deferred| {
                                Self::media_paths_refer_to_same_file(
                                    Path::new(&path),
                                    Path::new(deferred),
                                )
                            },
                        ) =>
                    {
                        if is_folded_match {
                            deferred_case_folded_inventory_resolution =
                                Some(MediaMatchInventoryExactResolution::Resolved {
                                    path,
                                    match_kind,
                                });
                            break;
                        }
                        self.unresolved_attached_media_target = None;
                        if !self.attached_media_search_refresh_pending() {
                            self.attached_media_search_next_retry_at = None;
                        }
                        return Ok(GuiUserMediaTargetResolution::Resolved {
                            path,
                            source: GuiUserMediaTargetResolutionSource::MediaMatchExactInventory,
                        });
                    }
                    MediaMatchInventoryExactResolution::Resolved { .. } => {}
                    MediaMatchInventoryExactResolution::Ambiguous {
                        candidate_count, ..
                    } => {
                        if is_folded_match {
                            deferred_case_folded_inventory_resolution =
                                Some(MediaMatchInventoryExactResolution::Ambiguous {
                                    candidate_count,
                                    match_kind,
                                });
                            break;
                        }
                        self.unresolved_attached_media_target = Some(target);
                        return Ok(GuiUserMediaTargetResolution::Ambiguous { candidate_count });
                    }
                }
            }

            if is_exact_file_name && (build_pending || self.attached_media_search_in_flight()) {
                self.unresolved_attached_media_target = Some(target);
                return Ok(GuiUserMediaTargetResolution::Pending);
            }
        }

        if search_roots.is_empty() {
            self.cancel_pending_attached_media_search_index_build_impl();
            self.attached_media_search_index = None;
            self.set_attached_media_search_build_state(
                &roots,
                GuiAttachedMediaSearchBuildState::Idle,
            );
            if let Some(path) = deferred_case_folded_current_path {
                self.unresolved_attached_media_target = None;
                self.attached_media_search_next_retry_at = None;
                return Ok(Self::quick_local_media_resolution(path));
            }
            return Ok(GuiUserMediaTargetResolution::Missing);
        }
        if !index_prepared {
            self.ensure_loaded_attached_media_search_index(&search_roots, &roots, retry_interval);
            build_pending = self.poll_attached_media_search_index_build(retry_interval);
        }
        self.unresolved_attached_media_target = Some(target);
        if !build_pending {
            let _ = self.queue_attached_media_search_refresh_if_needed(
                &search_roots,
                &roots,
                retry_interval,
                self.automatic_media_search_timeout(state),
            );
        }
        if self.attached_media_search_in_flight() {
            Ok(GuiUserMediaTargetResolution::Pending)
        } else if let Some(resolution) = deferred_case_folded_inventory_resolution {
            match resolution {
                MediaMatchInventoryExactResolution::Resolved { path, .. } => {
                    self.unresolved_attached_media_target = None;
                    // Preserve any scheduled double-check so a later exact-case file can
                    // replace this compatibility fallback.
                    Ok(GuiUserMediaTargetResolution::Resolved {
                        path,
                        source: GuiUserMediaTargetResolutionSource::MediaMatchExactInventory,
                    })
                }
                MediaMatchInventoryExactResolution::Ambiguous {
                    candidate_count, ..
                } => Ok(GuiUserMediaTargetResolution::Ambiguous { candidate_count }),
            }
        } else if let Some(path) = deferred_case_folded_current_path {
            self.unresolved_attached_media_target = None;
            // Preserve the scheduled double-check so a later exact-case file can replace this
            // compatibility fallback without repeated resolutions postponing the refresh.
            Ok(Self::quick_local_media_resolution(path))
        } else {
            Ok(GuiUserMediaTargetResolution::Missing)
        }
    }

    fn resolve_main_window_user_media_target_from_index(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
        reset_retry_on_target_change: bool,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_by_evidence_class(
            state,
            target,
            reset_retry_on_target_change,
            true,
        )
    }

    pub(in crate::app::runtime_owner) fn resolve_main_window_user_media_target_for_automatic_sync(
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
        self.resolve_main_window_user_media_target_by_evidence_class(state, target, false, false)
    }

    pub(in crate::app::runtime_owner) fn resolve_main_window_user_media_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Result<GuiUserMediaTargetResolution, String> {
        self.resolve_main_window_user_media_target_from_index(state, target, false)
    }

    fn plex_stream_resolution_config_for_target(
        &self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<PlexClientConfig> {
        if !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return None;
        }
        let settings = self.runtime_operation_settings(state);
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

    fn plex_resolution_miss_key(
        &self,
        state: &SorotteGuiShellAppState,
        target: &str,
        row_id: GuiPlaylistEntryId,
        policy: GuiPlaylistSourcePolicy,
    ) -> Option<PlexResolutionMissKey> {
        if policy != GuiPlaylistSourcePolicy::Automatic {
            return None;
        }
        let config = self.plex_stream_resolution_config_for_target(state, target)?;
        Some(PlexResolutionMissKey {
            row_id,
            playlist_generation: self.playlist_resolution.generation,
            policy,
            stream_trigger_key: Self::plex_stream_resolution_trigger_key(&config, target),
        })
    }

    pub(in crate::app::runtime_owner) fn active_plex_miss_retry_due(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> bool {
        let Some((index, target)) = self.current_shared_playlist_index_and_target(state) else {
            self.plex_miss_state = None;
            return false;
        };
        let Some(row) = state.main_window.playlist.get(index) else {
            self.plex_miss_state = None;
            return false;
        };
        let Some(key) =
            self.plex_resolution_miss_key(state, &target, row.entry_id, row.source_state.policy)
        else {
            self.plex_miss_state = None;
            return false;
        };
        self.reconcile_plex_miss_key(&key);
        self.matching_plex_miss_retry_due(&key, Instant::now())
    }

    fn resolve_plex_stream_target_with_parts(
        config: PlexClientConfig,
        client: PlexHttpClient,
        cache: PlexMatchCache,
        target: &str,
    ) -> GuiPlexStreamResolveOutcome {
        let mut resolver = PlexMediaResolver::new(config, client, cache);
        let result = resolver
            .resolve_stream_target(target, SystemTime::now())
            .map_err(|error| {
                redact_plex_token(&format!(
                    "Resolving Plex stream target for '{target}' failed: {error}"
                ))
            });
        let (_, _, cache) = resolver.into_parts();
        GuiPlexStreamResolveOutcome {
            stream_target: result,
            cache,
        }
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
        let current_context = self.plex_operation_context(&self.runtime_operation_settings(state));
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
                self.clear_plex_stream_resolution_state();
                false
            }
            Err(TryRecvError::Empty) => {
                self.plex_stream_resolve_rx = Some(rx);
                false
            }
            Err(TryRecvError::Disconnected) => {
                let trigger_key = self.plex_stream_resolve_trigger_key.clone();
                let operation_context = self.plex_stream_resolve_context.clone();
                if let (Some(trigger_key), Some(operation_context)) =
                    (trigger_key, operation_context)
                    && operation_context == current_context
                {
                    self.plex_stream_resolve_trigger_key = None;
                    self.plex_stream_resolve_context = None;
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
                self.clear_plex_stream_resolution_state();
                false
            }
        }
    }

    pub(in crate::app::runtime_owner) fn clear_plex_stream_resolution_state(&mut self) {
        let discarded_trigger_key = self.plex_stream_resolve_trigger_key.as_deref().or_else(|| {
            self.plex_stream_resolve_result
                .as_ref()
                .map(|result| result.trigger_key.as_str())
        });
        if let Some(miss) = self.plex_miss_state.as_mut()
            && miss.retry_in_flight
            && discarded_trigger_key
                .is_none_or(|trigger_key| trigger_key == miss.key.stream_trigger_key.as_str())
        {
            miss.retry_in_flight = false;
        }
        self.plex_stream_resolve_rx = None;
        self.plex_stream_resolve_trigger_key = None;
        self.plex_stream_resolve_context = None;
        self.plex_stream_resolve_result = None;
    }

    fn clear_plex_stream_resolution_state_for_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) {
        if self.plex_stream_resolution_state_matches_target(state, target) {
            self.clear_plex_stream_resolution_state();
        }
    }

    fn plex_stream_resolution_state_matches_target(
        &self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> bool {
        let Some(config) = self.plex_stream_resolution_config_for_target(state, target) else {
            return false;
        };
        let trigger_key = Self::plex_stream_resolution_trigger_key(&config, target);
        self.plex_stream_resolve_trigger_key.as_deref() == Some(trigger_key.as_str())
            || self
                .plex_stream_resolve_result
                .as_ref()
                .is_some_and(|result| result.trigger_key == trigger_key)
    }

    fn clear_orphaned_plex_stream_resolution_state(
        &mut self,
        state: &SorotteGuiShellAppState,
        active_target: &str,
    ) {
        if !self.plex_stream_resolution_owns_cache_snapshot()
            || self.plex_stream_resolution_state_matches_target(state, active_target)
        {
            return;
        }
        let retained_for_pending_request = self
            .pending_playlist_source_resolution
            .as_ref()
            .filter(|pending| pending.provider_id == GuiMediaSourceProviderId::plex_stream())
            .is_some_and(|pending| {
                self.plex_stream_resolution_state_matches_target(state, &pending.target)
            });
        if !retained_for_pending_request {
            self.clear_plex_stream_resolution_state();
        }
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
            self.last_attached_media_resolution_trigger = None;
            if let Some(miss) = self.plex_miss_state.as_mut() {
                miss.retry_in_flight = false;
            }
        }
        waiting
    }

    fn cached_or_queue_plex_stream_target_for_media_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
        consume_ready: bool,
    ) -> Result<GuiPlexStreamResolutionState, String> {
        let Some(config) = self.plex_stream_resolution_config_for_target(state, target) else {
            self.clear_plex_stream_resolution_state();
            return Ok(GuiPlexStreamResolutionState::Disabled);
        };
        let operation_context =
            self.plex_operation_context(&self.runtime_operation_settings(state));
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
            self.clear_plex_stream_resolution_state();
        }

        if self
            .plex_stream_resolve_result
            .as_ref()
            .is_some_and(|result| result.trigger_key == trigger_key)
        {
            if !consume_ready {
                let result = self
                    .plex_stream_resolve_result
                    .as_ref()
                    .expect("checked plex stream resolve result should exist");
                return match result.result.as_ref() {
                    Ok(outcome) => outcome
                        .stream_target
                        .as_ref()
                        .map(|target| {
                            GuiPlexStreamResolutionState::Ready(target.clone().map(Box::new))
                        })
                        .map_err(Clone::clone),
                    Err(error) => Err(error.clone()),
                };
            }
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
            let stream_target = outcome.stream_target?;
            return Ok(GuiPlexStreamResolutionState::Ready(
                stream_target.map(Box::new),
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
                let outcome = Self::resolve_plex_stream_target_with_parts(
                    config,
                    client,
                    cache,
                    &worker_target,
                );
                let staged_cache_write = (outcome.cache != cache_before)
                    .then(|| {
                        cache_path.map(|path| {
                            outcome.cache.stage_to_path(&path).map_err(|error| {
                            format!(
                                "Failed to stage Plex match cache after stream resolution: {error}"
                            )
                        })
                        })
                    })
                    .flatten();
                let _ = tx.send(GuiPlexStreamResolveWorkerResult {
                    operation_context: worker_operation_context,
                    trigger_key: worker_trigger_key,
                    result: Ok(outcome),
                    staged_cache_write,
                });
            })
            .map_err(|error| format!("Failed to start Plex stream resolution worker: {error}"))?;
        self.plex_stream_resolve_rx = Some(rx);
        self.plex_stream_resolve_trigger_key = Some(trigger_key);
        self.plex_stream_resolve_context = Some(operation_context);
        Ok(GuiPlexStreamResolutionState::Pending)
    }

    pub(super) fn open_media_resolution_candidate(
        &mut self,
        state: &SorotteGuiShellAppState,
        requested_target: &str,
        candidate: GuiMediaResolutionCandidate,
        user_initiated: bool,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let plex_operation_context =
            matches!(candidate.target(), GuiMediaResolutionTarget::PlexStream(_))
                .then(|| self.plex_operation_context(&self.runtime_operation_settings(state)));
        self.open_media_resolution_candidate_with_plex_context(
            requested_target,
            candidate,
            user_initiated,
            plex_operation_context,
        )
    }

    fn open_media_resolution_candidate_with_plex_context(
        &mut self,
        requested_target: &str,
        candidate: GuiMediaResolutionCandidate,
        user_initiated: bool,
        plex_operation_context: Option<GuiPlexOperationContext>,
    ) -> SelectedPlaylistMediaSyncOutcome {
        if user_initiated {
            self.rearm_failed_playlist_candidates_for_explicit_provider(&candidate.provider_id());
        }
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.candidate_plex_operation_context =
                matches!(candidate.target(), GuiMediaResolutionTarget::PlexStream(_))
                    .then_some(plex_operation_context)
                    .flatten();
        }
        match candidate.target() {
            GuiMediaResolutionTarget::CurrentPlayer => {
                self.unresolved_attached_media_target = None;
                if !self.attached_media_search_refresh_pending() {
                    self.attached_media_search_next_retry_at = None;
                }
                let provider_id = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .filter(|attempt| attempt.state == PlaylistResolutionAttemptState::Active)
                    .and_then(|attempt| attempt.candidate_provider.clone())
                    .unwrap_or_else(|| {
                        if self.pending_logical_media_override.is_some()
                            || self.player_local_file.as_ref().is_some_and(|file| {
                                file.path.as_deref().is_some_and(is_plex_playlist_uri)
                            })
                        {
                            GuiMediaSourceProviderId::plex_stream()
                        } else {
                            GuiMediaSourceProviderId::local()
                        }
                    });
                self.complete_current_playlist_resolution_from_current_player(provider_id);
                SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
            }
            GuiMediaResolutionTarget::LocalPath(resolved_target) => {
                if self.current_player_matches_media_target(resolved_target) {
                    self.unresolved_attached_media_target = None;
                    if !self.attached_media_search_refresh_pending() {
                        self.attached_media_search_next_retry_at = None;
                    }
                    self.complete_current_playlist_resolution_from_current_player(
                        candidate.provider_id(),
                    );
                    return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
                }

                let player_paths = [resolved_target.clone()];
                self.prepare_stream_load_tracking(&player_paths[0], user_initiated);
                let open_result = self.open_media_files_through_attached_player_result_impl(
                    &player_paths,
                    user_initiated,
                );
                match open_result {
                    Some(Ok(started)) => {
                        self.begin_playlist_resolution_candidate_load(candidate.clone(), &started);
                        self.plex_miss_state = None;
                        self.unresolved_attached_media_target = None;
                        if !self.attached_media_search_refresh_pending() {
                            self.attached_media_search_next_retry_at = None;
                        }
                        SelectedPlaylistMediaSyncOutcome::StartedLoading
                    }
                    Some(Err(message)) => {
                        self.fail_playlist_resolution_candidate(candidate.clone());
                        if user_initiated {
                            self.queue_stream_error(message);
                        } else {
                            self.queue_stream_warning(message);
                        }
                        SelectedPlaylistMediaSyncOutcome::NoChange
                    }
                    None => {
                        self.fail_playlist_resolution_candidate(candidate.clone());
                        SelectedPlaylistMediaSyncOutcome::NoChange
                    }
                }
            }
            GuiMediaResolutionTarget::PlexStream(stream_target) => {
                let open_result = self.open_plex_stream_target_through_attached_player_result_impl(
                    requested_target,
                    stream_target.as_ref().clone(),
                    user_initiated,
                );
                match open_result {
                    Some(Ok(started)) => {
                        self.begin_playlist_resolution_candidate_load(candidate.clone(), &started);
                        self.unresolved_attached_media_target = None;
                        if !self.attached_media_search_refresh_pending() {
                            self.attached_media_search_next_retry_at = None;
                        }
                        SelectedPlaylistMediaSyncOutcome::StartedLoading
                    }
                    Some(Err(message)) => {
                        self.fail_playlist_resolution_candidate(candidate.clone());
                        if user_initiated {
                            self.queue_stream_error(message);
                        } else {
                            self.queue_stream_warning(message);
                        }
                        SelectedPlaylistMediaSyncOutcome::NoChange
                    }
                    None => {
                        self.fail_playlist_resolution_candidate(candidate.clone());
                        SelectedPlaylistMediaSyncOutcome::NoChange
                    }
                }
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
            self.supersede_playlist_resolution_attempt();
            self.plex_miss_state = None;
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let mut plan = GuiMediaResolutionPlan::new(target);
        let Some((playlist_entry_id, source_state)) = state
            .main_window
            .playlist
            .get(playlist_index)
            .map(|row| (row.entry_id, row.source_state.clone()))
        else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        // Establish the room/session/playlist generation before binding a
        // command-correlated attempt. Local-origin lookup also reconciles this
        // scope, so doing it here prevents a first resolution from invalidating
        // its own freshly-created attempt.
        self.reconcile_local_shared_playlist_media_paths(state);
        self.ensure_playlist_resolution_attempt(
            playlist_entry_id,
            self.playlist_resolution.generation,
            plan.target(),
            source_state.policy,
        );
        self.reconcile_failed_playlist_candidates(state, Instant::now());
        if source_state.policy != GuiPlaylistSourcePolicy::Automatic {
            self.plex_miss_state = None;
        }
        if self
            .playlist_resolution_attempt
            .as_ref()
            .is_some_and(|attempt| attempt.state == PlaylistResolutionAttemptState::Loading)
        {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        let failed_candidates = self.failed_playlist_resolution_candidates();
        self.clear_orphaned_plex_stream_resolution_state(state, plan.target());
        // Reconcile retained drag/drop paths before the resolution trigger short-circuit.
        // Removing a dropped file must invalidate the previous local-first decision so
        // Automatic can continue through media search and Plex fallback.
        let retained_local_path = matches!(
            source_state.policy,
            GuiPlaylistSourcePolicy::Automatic
                | GuiPlaylistSourcePolicy::ForceLocal
                | GuiPlaylistSourcePolicy::PreferMediaMatching
        )
        .then(|| {
            state
                .main_window
                .playlist
                .get(playlist_index)
                .map(|row| row.entry_id)
                .and_then(|entry_id| self.local_shared_playlist_media_path_for_row(state, entry_id))
        })
        .flatten();

        let search_roots = self.automatic_media_search_roots(state);
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let trigger = self.automatic_media_resolution_trigger(
            state,
            plan.target(),
            Some(playlist_entry_id),
            source_state.policy,
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

        if let Some(path) = retained_local_path {
            self.clear_plex_stream_resolution_state_for_target(state, plan.target());
            plan.push_user_media_candidate(path, GuiUserMediaTargetResolutionSource::QuickLocal);
            plan.exclude_failed_candidates(&failed_candidates);
            if let Some(candidate) = plan.best_candidate().cloned() {
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                }
                let outcome =
                    self.open_media_resolution_candidate(state, plan.target(), candidate, false);
                if outcome != SelectedPlaylistMediaSyncOutcome::NoChange {
                    return outcome;
                }
            }
        }

        match source_state.policy {
            GuiPlaylistSourcePolicy::ForceLocal => {
                self.clear_plex_stream_resolution_state_for_target(state, plan.target());
                return self
                    .sync_selected_local_playlist_source_to_attached_player(state, plan.target());
            }
            GuiPlaylistSourcePolicy::PreferMediaMatching => {
                self.clear_plex_stream_resolution_state_for_target(state, plan.target());
                return self
                    .sync_selected_preferred_media_match_playlist_source_to_attached_player(
                        state,
                        plan.target(),
                    );
            }
            GuiPlaylistSourcePolicy::ForceMediaMatching => {
                self.clear_plex_stream_resolution_state_for_target(state, plan.target());
                return self.sync_selected_media_match_playlist_source_to_attached_player(
                    state,
                    plan.target(),
                );
            }
            GuiPlaylistSourcePolicy::ForcePlex => {
                return self.sync_selected_plex_stream_playlist_source_to_attached_player(
                    state,
                    plan.target(),
                );
            }
            GuiPlaylistSourcePolicy::Automatic => {}
        }

        if source_state.selection_origin == GuiPlaylistSourceSelectionOrigin::UserOverride {
            let provider_id = source_state
                .preferred_provider_id()
                .unwrap_or(&source_state.current_provider_id);
            if !self.preflight_room_stream_target(state, plan.target()) {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            return self.sync_selected_playlist_source_override_to_attached_player(
                state,
                plan.target(),
                provider_id,
            );
        }

        if self.current_player_matches_media_target(plan.target()) {
            self.clear_plex_stream_resolution_state_for_target(state, plan.target());
            plan.push_current_player_candidate();
            return self.open_media_resolution_candidate(
                state,
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
            Ok(
                GuiUserMediaTargetResolution::Ambiguous { .. }
                | GuiUserMediaTargetResolution::Missing,
            )
            | Err(_) => {
                if let Some(path) =
                    self.media_match_cached_room_candidate_for_target(state, plan.target())
                {
                    plan.push_media_match_candidate(path);
                } else if self.media_match_remote_lookup_pending_for_target(state, plan.target()) {
                    plan.record_pending_media_match();
                }
            }
        }

        plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());

        if plan.best_candidate().is_none() {
            let plex_miss_key = self.plex_resolution_miss_key(
                state,
                plan.target(),
                playlist_entry_id,
                source_state.policy,
            );
            if let Some(key) = plex_miss_key.as_ref() {
                self.reconcile_plex_miss_key(key);
            } else {
                self.plex_miss_state = None;
            }
            let completed_result_ready = plex_miss_key.as_ref().is_some_and(|key| {
                self.plex_stream_resolve_result
                    .as_ref()
                    .is_some_and(|result| result.trigger_key == key.stream_trigger_key)
            });
            let resolution_allowed = completed_result_ready
                || plex_miss_key
                    .as_ref()
                    .is_none_or(|key| self.plex_resolution_allowed_now(key, Instant::now()));
            if resolution_allowed {
                match self.cached_or_queue_plex_stream_target_for_media_target(
                    state,
                    plan.target(),
                    false,
                ) {
                    Ok(GuiPlexStreamResolutionState::Ready(Some(stream_target))) => {
                        // Keep the completed worker result until Plex actually wins the
                        // priority decision. A live local-index step may still outrank it.
                        plan.push_plex_stream_candidate(*stream_target);
                    }
                    Ok(GuiPlexStreamResolutionState::Ready(None)) => {
                        let consume_result = self
                            .cached_or_queue_plex_stream_target_for_media_target(
                                state,
                                plan.target(),
                                true,
                            );
                        if let Some(key) = plex_miss_key.clone() {
                            self.record_plex_resolution_miss(key, Instant::now());
                        }
                        if let Err(message) = consume_result {
                            self.queue_stream_warning(message);
                        }
                    }
                    Ok(GuiPlexStreamResolutionState::Disabled) => {}
                    Ok(GuiPlexStreamResolutionState::Pending) => {
                        plan.record_pending_plex_stream();
                    }
                    Err(message) => {
                        let message = self
                            .cached_or_queue_plex_stream_target_for_media_target(
                                state,
                                plan.target(),
                                true,
                            )
                            .err()
                            .unwrap_or(message);
                        if let Some(key) = plex_miss_key {
                            self.record_plex_resolution_miss(key, Instant::now());
                        }
                        self.queue_stream_warning(message);
                    }
                }
            }
        }

        plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());

        let candidate = match plan.decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority)
        {
            GuiMediaResolutionDecision::Ready(candidate) => candidate,
            GuiMediaResolutionDecision::WaitingForHigherPriority
            | GuiMediaResolutionDecision::Exhausted => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
        };
        if matches!(candidate.target(), GuiMediaResolutionTarget::PlexStream(_)) {
            let plex_miss_key = self.plex_resolution_miss_key(
                state,
                plan.target(),
                playlist_entry_id,
                source_state.policy,
            );
            match self.cached_or_queue_plex_stream_target_for_media_target(
                state,
                plan.target(),
                true,
            ) {
                Ok(GuiPlexStreamResolutionState::Ready(Some(_))) => {
                    if let Some(key) = plex_miss_key.as_ref() {
                        self.clear_plex_resolution_miss_for_key(key);
                    }
                }
                Ok(GuiPlexStreamResolutionState::Ready(None)) => {
                    if let Some(key) = plex_miss_key {
                        self.record_plex_resolution_miss(key, Instant::now());
                    }
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                }
                Ok(
                    GuiPlexStreamResolutionState::Pending | GuiPlexStreamResolutionState::Disabled,
                ) => return SelectedPlaylistMediaSyncOutcome::NoChange,
                Err(message) => {
                    if let Some(key) = plex_miss_key {
                        self.record_plex_resolution_miss(key, Instant::now());
                    }
                    self.queue_stream_warning(message);
                    return SelectedPlaylistMediaSyncOutcome::NoChange;
                }
            }
        }
        if !matches!(candidate.target(), GuiMediaResolutionTarget::PlexStream(_)) {
            self.clear_plex_stream_resolution_state_for_target(state, plan.target());
        }
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(state, plan.target(), candidate, false)
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
            Ok(
                GuiUserMediaTargetResolution::Ambiguous { .. }
                | GuiUserMediaTargetResolution::Missing,
            )
            | Err(_) => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
        }

        plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());

        let candidate = match plan.decision(GuiMediaResolutionFallbackPolicy::AllowReadyFallback) {
            GuiMediaResolutionDecision::Ready(candidate) => candidate,
            GuiMediaResolutionDecision::WaitingForHigherPriority
            | GuiMediaResolutionDecision::Exhausted => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
        };
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(state, plan.target(), candidate, false)
    }

    fn sync_selected_preferred_media_match_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let mut local_plan = GuiMediaResolutionPlan::new(target);
        match self.resolve_main_window_user_media_target_local_only(state, local_plan.target()) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, source }) => {
                local_plan.push_user_media_candidate(path, source);
                local_plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());
                if let Some(candidate) = local_plan.best_candidate().cloned() {
                    self.ensure_configured_player_attached();
                    if self.player.is_none() {
                        return SelectedPlaylistMediaSyncOutcome::NoChange;
                    }
                    let outcome = self.open_media_resolution_candidate(
                        state,
                        local_plan.target(),
                        candidate,
                        false,
                    );
                    if outcome != SelectedPlaylistMediaSyncOutcome::NoChange {
                        return outcome;
                    }
                }
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                return SelectedPlaylistMediaSyncOutcome::NoChange;
            }
            Ok(
                GuiUserMediaTargetResolution::Ambiguous { .. }
                | GuiUserMediaTargetResolution::Missing,
            )
            | Err(_) => {}
        }

        self.sync_selected_media_match_playlist_source_to_attached_player(state, target)
    }

    fn sync_selected_media_match_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        if !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
            || !state.media_match.settings.fingerprinting_enabled
        {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }

        let search_roots = self.automatic_media_search_roots(state);
        let excluded_current_path = self.uncorroborated_current_player_title_collision_path(target);
        let Some(path) = self
            .media_match_cached_exact_inventory_candidate_for_target(state, target, &search_roots)
            .filter(|path| {
                !excluded_current_path.as_deref().is_some_and(|excluded| {
                    Self::media_paths_refer_to_same_file(Path::new(path), Path::new(excluded))
                })
            })
            .or_else(|| self.media_match_cached_room_candidate_for_target(state, target))
        else {
            let _ = self.media_match_remote_lookup_pending_for_target(state, target);
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let mut plan = GuiMediaResolutionPlan::new(target);
        plan.push_media_match_candidate(path);
        plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());
        let Some(candidate) = plan.best_candidate().cloned() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        self.cancel_attached_media_search_after_media_match_resolution();
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(state, plan.target(), candidate, false)
    }

    fn sync_selected_plex_stream_playlist_source_to_attached_player(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let mut plan = GuiMediaResolutionPlan::new(target);
        match self.cached_or_queue_plex_stream_target_for_media_target(state, plan.target(), true) {
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

        plan.exclude_failed_candidates(&self.failed_playlist_resolution_candidates());

        let Some(candidate) = plan.best_candidate().cloned() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.open_media_resolution_candidate(state, plan.target(), candidate, false)
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
        let Some((row_id, policy, source_status)) =
            projected_state.main_window.playlist.get(index).map(|row| {
                (
                    row.entry_id,
                    row.source_state.policy,
                    row.source_state.status,
                )
            })
        else {
            return false;
        };
        if source_status == GuiPlaylistSourceStatus::Resolving
            && self
                .playlist_resolution_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.state != PlaylistResolutionAttemptState::Resolving)
        {
            self.supersede_playlist_resolution_attempt();
        }
        self.ensure_playlist_resolution_attempt(
            row_id,
            self.playlist_resolution.generation,
            &target,
            policy,
        );
        self.rearm_failed_playlist_candidates_for_explicit_provider(&provider_id);

        if provider_id != GuiMediaSourceProviderId::plex_stream() {
            self.clear_plex_stream_resolution_state_for_target(projected_state, &target);
        }

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
        let exact_origin = projected_state
            .main_window
            .playlist
            .get(index)
            .map(|row| row.entry_id)
            .and_then(|entry_id| {
                self.local_shared_playlist_media_path_for_row(projected_state, entry_id)
            });
        let resolution = exact_origin.map_or_else(
            || self.resolve_main_window_user_media_target_local_only(projected_state, target),
            |path| {
                Ok(GuiUserMediaTargetResolution::Resolved {
                    path,
                    source: GuiUserMediaTargetResolutionSource::QuickLocal,
                })
            },
        );
        match resolution {
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
                let outcome =
                    self.open_media_resolution_candidate(projected_state, target, candidate, true);
                let (status, detail) = match outcome {
                    SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget => (
                        GuiPlaylistSourceStatus::Active,
                        "The attached player confirmed the local media target.".to_owned(),
                    ),
                    SelectedPlaylistMediaSyncOutcome::StartedLoading => (
                        GuiPlaylistSourceStatus::Loading,
                        "Waiting for the attached player to confirm the local media load."
                            .to_owned(),
                    ),
                    SelectedPlaylistMediaSyncOutcome::NoChange => (
                        GuiPlaylistSourceStatus::Failed,
                        "The attached player did not accept the resolved local media target."
                            .to_owned(),
                    ),
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
            Ok(GuiUserMediaTargetResolution::Ambiguous { candidate_count }) => {
                let detail = format!(
                    "Local media search found {candidate_count} equally credible files; choose a more specific playlist path."
                );
                self.publish_playlist_source_state(
                    handle,
                    projected_state,
                    GuiPlaylistSourceStateUpdate {
                        index,
                        target,
                        provider_id,
                        status: GuiPlaylistSourceStatus::Failed,
                        detail: detail.clone(),
                        resolution_steps: vec![Self::playlist_resolution_step(
                            GuiMediaSourceProviderId::local(),
                            "Local",
                            GuiPlaylistSourceStatus::Failed,
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
            let outcome =
                self.open_media_resolution_candidate(projected_state, target, candidate, true);
            let (status, detail) = match outcome {
                SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget => (
                    GuiPlaylistSourceStatus::Active,
                    "The attached player confirmed the Media Matching target.".to_owned(),
                ),
                SelectedPlaylistMediaSyncOutcome::StartedLoading => (
                    GuiPlaylistSourceStatus::Loading,
                    "Waiting for the attached player to confirm the Media Matching load."
                        .to_owned(),
                ),
                SelectedPlaylistMediaSyncOutcome::NoChange => (
                    GuiPlaylistSourceStatus::Failed,
                    "The attached player did not accept the Media Matching target.".to_owned(),
                ),
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
        match self.cached_or_queue_plex_stream_target_for_media_target(
            projected_state,
            target,
            true,
        ) {
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
                let outcome =
                    self.open_media_resolution_candidate(projected_state, target, candidate, true);
                let (status, detail) = match outcome {
                    SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget => (
                        GuiPlaylistSourceStatus::Active,
                        "The attached player confirmed the Plex stream target.".to_owned(),
                    ),
                    SelectedPlaylistMediaSyncOutcome::StartedLoading => (
                        GuiPlaylistSourceStatus::Loading,
                        "Waiting for the attached player to confirm the Plex stream load."
                            .to_owned(),
                    ),
                    SelectedPlaylistMediaSyncOutcome::NoChange => (
                        GuiPlaylistSourceStatus::Failed,
                        "The attached player did not accept the resolved Plex stream target."
                            .to_owned(),
                    ),
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
        self.record_playlist_source_resolution_status(
            projected_state,
            index,
            target,
            &provider_id,
            status,
        );
        source_state.set_resolved_provider(provider_id);
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
        let requested_target = selected_path.clone();
        if let Some(index) = state.main_window.active_playlist_index
            && let Some(row) = state.main_window.playlist.get(index)
        {
            self.ensure_playlist_resolution_attempt(
                row.entry_id,
                self.playlist_resolution.generation,
                &row.label,
                row.source_state.policy,
            );
        }
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
            let provider_id = if selected_path_is_plex_uri {
                GuiMediaSourceProviderId::plex_stream()
            } else {
                GuiMediaSourceProviderId::local()
            };
            self.complete_current_playlist_resolution_from_current_player(provider_id);
            return SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget;
        }

        if selected_path_is_plex_uri {
            match self
                .resolve_main_window_user_media_target_for_automatic_sync(state, &selected_path)
            {
                Ok(GuiUserMediaTargetResolution::Resolved { path, .. }) => {
                    selected_path = path;
                }
                Ok(
                    GuiUserMediaTargetResolution::Ambiguous { .. }
                    | GuiUserMediaTargetResolution::Pending
                    | GuiUserMediaTargetResolution::Missing,
                )
                | Err(_) => {
                    let stream_target = match self
                        .cached_or_queue_plex_stream_target_for_media_target(
                            state,
                            &selected_path,
                            true,
                        ) {
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
                    let mut plan = GuiMediaResolutionPlan::new(&requested_target);
                    plan.push_plex_stream_candidate(stream_target);
                    let Some(candidate) = plan.best_candidate().cloned() else {
                        return SelectedPlaylistMediaSyncOutcome::NoChange;
                    };
                    let outcome = self.open_media_resolution_candidate(
                        state,
                        &requested_target,
                        candidate,
                        true,
                    );
                    if outcome != SelectedPlaylistMediaSyncOutcome::NoChange {
                        self.cancel_pending_attached_media_search_index_build_impl();
                    }
                    return outcome;
                }
            }
        }

        let mut plan = GuiMediaResolutionPlan::new(&requested_target);
        plan.push_user_media_candidate(
            selected_path,
            GuiUserMediaTargetResolutionSource::QuickLocal,
        );
        let Some(candidate) = plan.best_candidate().cloned() else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let outcome =
            self.open_media_resolution_candidate(state, &requested_target, candidate, true);
        if outcome != SelectedPlaylistMediaSyncOutcome::NoChange {
            self.cancel_pending_attached_media_search_index_build_impl();
        }
        outcome
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
    fn operation_context_invalidation_releases_retry_with_unchanged_stream_key() {
        let mut previous_settings = streaming_settings();
        previous_settings.plex_sync_enabled = Some(false);
        let next_settings = streaming_settings();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&next_settings);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let target = "episode.mkv";
        let previous_config =
            super::super::super::plex::plex_config_from_settings(&previous_settings);
        let next_config = super::super::super::plex::plex_config_from_settings(&next_settings);
        let previous_trigger_key =
            GuiPersistedConfigRuntimeOwner::plex_stream_resolution_trigger_key(
                &previous_config,
                target,
            );
        let next_trigger_key = GuiPersistedConfigRuntimeOwner::plex_stream_resolution_trigger_key(
            &next_config,
            target,
        );
        assert_eq!(
            previous_trigger_key, next_trigger_key,
            "watch-sync ownership is intentionally outside the stream-resolution key"
        );

        let now = Instant::now();
        let miss_key = PlexResolutionMissKey {
            row_id: GuiPlaylistEntryId::next(),
            playlist_generation: 4,
            policy: GuiPlaylistSourcePolicy::Automatic,
            stream_trigger_key: previous_trigger_key.clone(),
        };
        owner.plex_miss_state = Some(PlexMissState {
            key: miss_key.clone(),
            last_attempt_at: now,
            next_retry_at: now,
            attempt_count: 1,
            retry_in_flight: false,
        });
        assert!(owner.plex_resolution_allowed_now(&miss_key, now));

        let (_worker_tx, worker_rx) = mpsc::channel::<GuiPlexStreamResolveWorkerResult>();
        owner.plex_stream_resolve_rx = Some(worker_rx);
        owner.plex_stream_resolve_trigger_key = Some(previous_trigger_key);
        owner.plex_stream_resolve_context = Some(owner.plex_operation_context(&previous_settings));

        owner.invalidate_plex_operation_context_if_settings_changed(
            &handle,
            &mut state,
            &previous_settings,
            &next_settings,
        );

        assert!(!owner.plex_stream_resolution_owns_cache_snapshot());
        assert!(
            !owner.plex_miss_state.as_ref().unwrap().retry_in_flight,
            "discarding the old-context worker must release the active miss retry"
        );
        assert!(
            owner.plex_resolution_allowed_now(&miss_key, now),
            "the unchanged stream key must be eligible to retry immediately"
        );
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
            .cached_or_queue_plex_stream_target_for_media_target(&state, "plex://", true)
            .expect("Plex stream resolution should defer without failing");

        assert!(matches!(resolution, GuiPlexStreamResolutionState::Pending));
        assert!(owner.plex_stream_resolve_rx.is_none());
        assert!(owner.plex_sync_engine.is_none());
        assert!(owner.plex_stream_resolution_owns_cache_snapshot());
        owner.last_attached_media_resolution_trigger = Some(GuiAutomaticMediaResolutionTrigger {
            target: "deferred.mkv".to_owned(),
            playlist_entry_id: None,
            playlist_generation: 0,
            source_provider: "automatic".to_owned(),
            plex_operation_context: Some(owner.plex_operation_context(&settings)),
            roots: Vec::new(),
            media_match_remote_targets: String::new(),
            current_player_path: None,
            index_revision: 0,
            retry_due: true,
        });
        let now = Instant::now();
        owner.plex_miss_state = Some(PlexMissState {
            key: PlexResolutionMissKey {
                row_id: GuiPlaylistEntryId::next(),
                playlist_generation: 0,
                policy: GuiPlaylistSourcePolicy::Automatic,
                stream_trigger_key: "deferred".to_owned(),
            },
            last_attempt_at: now,
            next_retry_at: now,
            attempt_count: 1,
            retry_in_flight: true,
        });

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
        assert!(
            owner.last_attached_media_resolution_trigger.is_none(),
            "watch-sync handoff must invalidate the stale automatic trigger so the retry runs"
        );
        assert!(
            !owner.plex_miss_state.as_ref().unwrap().retry_in_flight,
            "watch-sync handoff must release the Plex-miss retry latch"
        );

        let retry = owner
            .cached_or_queue_plex_stream_target_for_media_target(&state, "plex://", true)
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
                    stream_target: Ok(None),
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
            .cached_or_queue_plex_stream_target_for_media_target(&state, target, true)
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
    fn failed_refreshed_stream_match_still_commits_evicted_stale_cache() {
        let sequence = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-gui-plex-stream-error-stage-{}-{sequence}",
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
        let stale = cache_with_rating_key("same-unusable-rating-key");
        let evicted = PlexMatchCache::default();
        stale
            .save_to_path(&cache_path)
            .expect("stale cache should persist before resolution");
        let staged_cache_write = evicted
            .stage_to_path(&cache_path)
            .expect("evicted cache should stage even when refreshed metadata fails");

        let config = super::super::super::plex::plex_config_from_settings(&settings);
        let target = "same-unusable-rating-key.mkv";
        let trigger_key =
            GuiPersistedConfigRuntimeOwner::plex_stream_resolution_trigger_key(&config, target);
        let operation_context = owner.plex_operation_context(&settings);
        let (result_tx, result_rx) = mpsc::channel();
        result_tx
            .send(GuiPlexStreamResolveWorkerResult {
                operation_context: operation_context.clone(),
                trigger_key: trigger_key.clone(),
                result: Ok(GuiPlexStreamResolveOutcome {
                    stream_target: Err(
                        "refreshed Plex rating key still returned missing metadata".to_owned()
                    ),
                    cache: evicted.clone(),
                }),
                staged_cache_write: Some(Ok(staged_cache_write)),
            })
            .expect("failed stream result should queue with its mutated cache");
        owner.plex_stream_resolve_rx = Some(result_rx);
        owner.plex_stream_resolve_trigger_key = Some(trigger_key);
        owner.plex_stream_resolve_context = Some(operation_context);

        assert!(owner.pump_plex_stream_resolution_worker(&state));
        let error =
            match owner.cached_or_queue_plex_stream_target_for_media_target(&state, target, true) {
                Ok(_) => panic!("the refreshed metadata failure must remain visible"),
                Err(error) => error,
            };

        assert!(error.contains("missing metadata"));
        assert_eq!(
            PlexMatchCache::load_from_path(&cache_path)
                .expect("the accepted eviction must remain readable after the error"),
            evicted,
            "the next GUI retry must not reload the same stale positive cache entry"
        );
        assert_eq!(
            owner
                .plex_sync_engine
                .as_ref()
                .expect("the accepted cache should return to the shared engine")
                .cache(),
            &evicted
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
                stream_target: Ok(None),
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

        owner.clear_plex_stream_resolution_state();

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
