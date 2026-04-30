use super::*;

const LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT: f64 = 20.0;
const LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT: f64 = 30.0;
const MEDIA_INDEX_PROGRESS_INTERVAL_MILLIS_DEFAULT: u64 = 250;

type CachedMissingMediaMatchRank = (usize, usize, usize, usize, String);
type CachedMissingMediaMatch = (CachedMissingMediaMatchRank, String);

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn automatic_media_search_roots(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<PathBuf> {
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

    pub(super) fn automatic_media_search_root_keys(search_roots: &[PathBuf]) -> Vec<String> {
        search_roots
            .iter()
            .map(|path| normalized_media_search_root_key(path))
            .collect()
    }

    fn media_index_progress_interval() -> Duration {
        env_trimmed("SYNCPLAY_GUI_MEDIA_INDEX_PROGRESS_INTERVAL_MS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value != 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(MEDIA_INDEX_PROGRESS_INTERVAL_MILLIS_DEFAULT))
    }

    pub(super) fn set_attached_media_search_build_state(
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

    pub(super) fn automatic_media_resolution_trigger(
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

    pub(super) fn should_rerun_automatic_media_resolution(
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

    pub(super) fn automatic_media_search_timeout(state: &SyncplayGuiShellAppState) -> Duration {
        let settings = state.configuration.to_stored_settings();
        Self::positive_duration_from_seconds_or_default(
            settings.folder_search_timeout_seconds,
            LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT,
        )
    }

    pub(in crate::app::runtime_owner) fn automatic_media_search_retry_interval(
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

    pub(super) fn attached_media_search_refresh_pending(&self) -> bool {
        self.pending_attached_media_resolution.is_some()
            || self
                .attached_media_search_index
                .as_ref()
                .is_some_and(|index| !index.roots_requiring_refresh.is_empty())
    }

    pub(super) fn cancel_pending_attached_media_search_index_build_impl(&mut self) {
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

    pub(super) fn ensure_loaded_attached_media_search_index(
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

    fn cached_missing_media_relative_target_key(target: &str) -> Option<String> {
        if browser_is_url(target) {
            return None;
        }
        let target = target.trim();
        if target.is_empty() {
            return None;
        }
        let target_path = Path::new(target);
        if target_path.is_absolute() {
            return None;
        }
        if !(target.contains('/') || target.contains('\\') || target_path.components().count() > 1)
        {
            return None;
        }
        let mut saw_normal_component = false;
        for component in target_path.components() {
            match component {
                std::path::Component::Normal(_) => saw_normal_component = true,
                std::path::Component::CurDir
                | std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_) => return None,
            }
        }
        if !saw_normal_component {
            return None;
        }
        let mut key = target.replace('\\', "/");
        while key.ends_with('/') && key.len() > 1 {
            key.pop();
        }
        Some(if cfg!(windows) {
            key.to_ascii_lowercase()
        } else {
            key
        })
    }

    pub(in crate::app::runtime_owner) fn cached_missing_media_target_path(
        &self,
        index: &GuiAttachedMediaSearchIndex,
        target: &str,
    ) -> Option<String> {
        let target_key =
            GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(target)?;
        let target_relative_key = Self::cached_missing_media_relative_target_key(target);
        let current_parent = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let mut best_match: Option<CachedMissingMediaMatch> = None;

        for (root_order, root_key) in index.roots.iter().enumerate() {
            let Some(root_index) = index.root_indexes_by_key.get(root_key) else {
                continue;
            };
            let Some(candidates) = root_index.candidates_by_name.get(&target_key) else {
                continue;
            };
            for relative_path in candidates {
                let relative_path_key = Self::normalized_media_search_path_key(Path::new(
                    &relative_path.replace('\\', "/"),
                ));
                let relative_path_rank = target_relative_key
                    .as_ref()
                    .map(|target_relative_key| {
                        usize::from(relative_path_key != *target_relative_key)
                    })
                    .unwrap_or(0);
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
                let rank = (
                    relative_path_rank,
                    locality_rank,
                    root_order,
                    depth,
                    lexical,
                );
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

    pub(in crate::app::runtime_owner) fn media_index_progress_message(
        progress: &GuiAttachedMediaSearchBuildProgress,
    ) -> String {
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

    pub(in crate::app::runtime_owner) fn media_index_runtime_snapshot_impl(
        &self,
    ) -> GuiMediaIndexRuntimeSnapshot {
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
        cache_generation: u64,
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
                                let _ =
                                    persist_media_search_root_index_borrowed_at_root_if_cache_generation(
                                    cache_root,
                                    &root_index.root_key,
                                    &root_index.root_path,
                                    root_index.built_at_unix_ms,
                                    &root_index.candidates_by_name,
                                    cache_generation,
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

    pub(in crate::app::runtime_owner) fn poll_attached_media_search_index_build(
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
        let cache_generation = current_media_search_cache_generation();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let latest_progress = Arc::new(Mutex::new(None));
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
                cache_generation,
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

    pub(super) fn queue_attached_media_search_refresh_if_needed(
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
}
