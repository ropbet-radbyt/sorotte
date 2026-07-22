use super::*;

const TRANSIENT_CANDIDATE_RETRY_BASE: Duration = Duration::from_secs(2);
const TRANSIENT_CANDIDATE_RETRY_MAX: Duration = Duration::from_secs(8);
const TRANSIENT_CANDIDATE_MAX_FAILURES: u32 = 4;

impl CandidateFailureDisposition {
    fn for_command_failure(kind: PlayerCommandFailureKind) -> Self {
        match kind {
            PlayerCommandFailureKind::TimedOut
            | PlayerCommandFailureKind::TransportDisconnected => Self::Transient,
            PlayerCommandFailureKind::MediaEnded => Self::Permanent,
            PlayerCommandFailureKind::Unknown => Self::ContextDependent,
        }
    }

    fn for_media_load_failure(kind: PlayerMediaLoadFailureKind) -> Self {
        match kind {
            PlayerMediaLoadFailureKind::Network | PlayerMediaLoadFailureKind::LoadAborted => {
                Self::Transient
            }
            PlayerMediaLoadFailureKind::FormatUnsupported => Self::Permanent,
            PlayerMediaLoadFailureKind::HelperMissing
            | PlayerMediaLoadFailureKind::HelperBroken
            | PlayerMediaLoadFailureKind::Unknown => Self::ContextDependent,
        }
    }
}

impl PlaylistResolutionCandidateFailure {
    fn excludes_candidate_at(&self, now: Instant) -> bool {
        match self.disposition {
            CandidateFailureDisposition::Permanent
            | CandidateFailureDisposition::ContextDependent => true,
            CandidateFailureDisposition::Transient => {
                self.failure_count >= TRANSIENT_CANDIDATE_MAX_FAILURES
                    || self.next_retry_at.is_some_and(|deadline| now < deadline)
            }
        }
    }

    fn transient_retry_due_at(&self, now: Instant) -> bool {
        self.disposition == CandidateFailureDisposition::Transient
            && self.failure_count < TRANSIENT_CANDIDATE_MAX_FAILURES
            && self.next_retry_at.is_some_and(|deadline| now >= deadline)
    }
}

fn transient_candidate_retry_delay(failure_count: u32) -> Option<Duration> {
    if failure_count >= TRANSIENT_CANDIDATE_MAX_FAILURES {
        return None;
    }
    let exponent = failure_count.saturating_sub(1).min(31);
    Some(std::cmp::min(
        TRANSIENT_CANDIDATE_RETRY_BASE.saturating_mul(1_u32 << exponent),
        TRANSIENT_CANDIDATE_RETRY_MAX,
    ))
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn bind_started_local_media_load_to_current_playlist(
        &mut self,
        state: &SorotteGuiShellAppState,
        resolved_path: String,
        source: GuiUserMediaTargetResolutionSource,
        started: &StartedMediaLoad,
    ) {
        let Some((playlist_index, target)) = self.current_shared_playlist_index_and_target(state)
        else {
            return;
        };
        self.reconcile_local_shared_playlist_media_paths(state);
        let Some(row) = state.main_window.playlist.get(playlist_index) else {
            return;
        };
        self.ensure_playlist_resolution_attempt(
            row.entry_id,
            self.playlist_resolution.generation,
            &target,
            row.source_state.policy,
        );
        let mut plan = media_resolution::GuiMediaResolutionPlan::new(target);
        plan.push_user_media_candidate(resolved_path, source);
        if let Some(candidate) = plan.best_candidate().cloned() {
            self.begin_playlist_resolution_candidate_load(candidate, started);
        }
    }

    pub(in crate::app) fn player_local_file_identity_confirmed_for_shared_sync(&self) -> bool {
        let playlist_load_unconfirmed =
            self.playlist_resolution_attempt
                .as_ref()
                .is_some_and(|attempt| {
                    attempt.playlist_generation == self.playlist_resolution.generation
                        && matches!(
                            attempt.state,
                            PlaylistResolutionAttemptState::Resolving
                                | PlaylistResolutionAttemptState::Loading
                        )
                });
        !self.player_local_file_placeholder && !playlist_load_unconfirmed
    }

    pub(super) fn ensure_playlist_resolution_attempt(
        &mut self,
        row_id: GuiPlaylistEntryId,
        playlist_generation: u64,
        target: &str,
        policy: GuiPlaylistSourcePolicy,
    ) {
        if self
            .playlist_resolution_attempt
            .as_ref()
            .is_some_and(|attempt| {
                attempt.matches_scope(row_id, playlist_generation, target, policy)
            })
        {
            return;
        }

        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.state = PlaylistResolutionAttemptState::Superseded;
        }
        self.pending_logical_media_override = None;
        self.playlist_resolution_attempt = Some(PlaylistResolutionAttempt::new(
            row_id,
            playlist_generation,
            target.to_owned(),
            policy,
        ));
    }

    pub(in crate::app::runtime_owner) fn supersede_playlist_resolution_attempt(&mut self) {
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.state = PlaylistResolutionAttemptState::Superseded;
        }
        self.playlist_resolution_attempt = None;
        self.pending_logical_media_override = None;
    }

    pub(super) fn failed_playlist_resolution_candidates(
        &self,
    ) -> Vec<media_resolution::GuiMediaResolutionCandidate> {
        let now = Instant::now();
        self.playlist_resolution_attempt
            .as_ref()
            .map(|attempt| {
                let due_transient_is_eligible =
                    attempt.state == PlaylistResolutionAttemptState::Resolving;
                attempt
                    .candidate_failures
                    .iter()
                    .filter(|failure| {
                        !due_transient_is_eligible || failure.excludes_candidate_at(now)
                    })
                    .map(|failure| failure.candidate.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(in crate::app::runtime_owner) fn active_playlist_candidate_retry_due(&self) -> bool {
        let now = Instant::now();
        self.playlist_resolution_attempt
            .as_ref()
            .is_some_and(|attempt| {
                attempt.playlist_generation == self.playlist_resolution.generation
                    && attempt.state == PlaylistResolutionAttemptState::Failed
                    && attempt
                        .candidate_failures
                        .iter()
                        .any(|failure| failure.transient_retry_due_at(now))
            })
    }

    fn candidate_local_file_evidence(
        candidate: &media_resolution::GuiMediaResolutionCandidate,
    ) -> Option<CandidateLocalFileEvidence> {
        let media_resolution::GuiMediaResolutionTarget::LocalPath(path) = candidate.target() else {
            return None;
        };
        if browser_is_url(path) {
            return None;
        }
        match fs::metadata(path) {
            Ok(metadata) => Some(CandidateLocalFileEvidence {
                exists: true,
                size_bytes: Some(metadata.len()),
                modified_unix_nanos: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_nanos()),
            }),
            Err(_) => Some(CandidateLocalFileEvidence {
                exists: false,
                size_bytes: None,
                modified_unix_nanos: None,
            }),
        }
    }

    fn candidate_failure_evidence(
        &self,
        candidate: &media_resolution::GuiMediaResolutionCandidate,
    ) -> CandidateFailureEvidence {
        let plex_operation_context = self
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| attempt.candidate_plex_operation_context.clone())
            .or_else(|| {
                self.last_attached_media_resolution_trigger
                    .as_ref()
                    .and_then(|trigger| trigger.plex_operation_context.clone())
            })
            .or_else(|| self.plex_stream_resolve_context.clone())
            .or_else(|| {
                self.plex_stream_resolve_result
                    .as_ref()
                    .map(|result| result.operation_context.clone())
            });
        CandidateFailureEvidence {
            local_index_revision: self.attached_media_search_index_revision,
            local_file: Self::candidate_local_file_evidence(candidate),
            plex_operation_context,
            media_match_result: self.media_match_remote_lookup_result.clone(),
            stream_helper_health: self.stream_helper_runtime_snapshot.health,
            player_attachment_epoch: self.player_attachment_epoch,
        }
    }

    fn candidate_failure_evidence_for_state(
        &self,
        state: &SorotteGuiShellAppState,
        candidate: &media_resolution::GuiMediaResolutionCandidate,
    ) -> CandidateFailureEvidence {
        CandidateFailureEvidence {
            local_index_revision: self.attached_media_search_index_revision,
            local_file: Self::candidate_local_file_evidence(candidate),
            plex_operation_context: Some(
                self.plex_operation_context(&self.runtime_operation_settings(state)),
            ),
            media_match_result: self.media_match_remote_lookup_result.clone(),
            stream_helper_health: self.stream_helper_runtime_snapshot.health,
            player_attachment_epoch: self.player_attachment_epoch,
        }
    }

    fn candidate_failure_evidence_changed(
        failure: &PlaylistResolutionCandidateFailure,
        current: &CandidateFailureEvidence,
    ) -> bool {
        let local_file_changed = failure.evidence.local_file != current.local_file;
        let player_changed =
            failure.evidence.player_attachment_epoch != current.player_attachment_epoch;
        let provider_context_changed = match failure.candidate.provider_kind() {
            media_resolution::GuiMediaResolutionProviderKind::Core => local_file_changed,
            media_resolution::GuiMediaResolutionProviderKind::MediaSearch => {
                local_file_changed
                    || failure.evidence.local_index_revision != current.local_index_revision
            }
            media_resolution::GuiMediaResolutionProviderKind::MediaMatch => {
                local_file_changed
                    || failure.evidence.local_index_revision != current.local_index_revision
                    || failure.evidence.media_match_result != current.media_match_result
            }
            media_resolution::GuiMediaResolutionProviderKind::Plex => failure
                .evidence
                .plex_operation_context
                .as_ref()
                .is_some_and(|previous| Some(previous) != current.plex_operation_context.as_ref()),
        };
        let helper_changed = matches!(
            failure.candidate.target(),
            media_resolution::GuiMediaResolutionTarget::LocalPath(target)
                if browser_stream_target_kind(target, None)
                    == GuiStreamTargetKind::ExtractorPageUrl
        ) && failure.evidence.stream_helper_health
            != current.stream_helper_health;

        match failure.disposition {
            CandidateFailureDisposition::Permanent => match failure.candidate.provider_kind() {
                media_resolution::GuiMediaResolutionProviderKind::Plex => provider_context_changed,
                media_resolution::GuiMediaResolutionProviderKind::Core
                | media_resolution::GuiMediaResolutionProviderKind::MediaSearch
                | media_resolution::GuiMediaResolutionProviderKind::MediaMatch => {
                    local_file_changed
                }
            },
            CandidateFailureDisposition::Transient => {
                player_changed || provider_context_changed || helper_changed
            }
            CandidateFailureDisposition::ContextDependent => {
                player_changed || provider_context_changed || helper_changed
            }
        }
    }

    fn prepare_failed_attempt_for_retry(attempt: &mut PlaylistResolutionAttempt) {
        attempt.candidate_provider = None;
        attempt.candidate = None;
        attempt.candidate_plex_operation_context = None;
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Resolving;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
    }

    pub(super) fn reconcile_failed_playlist_candidates(
        &mut self,
        state: &SorotteGuiShellAppState,
        now: Instant,
    ) -> bool {
        let rearmed_candidates = self
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| {
                attempt
                    .candidate_failures
                    .iter()
                    .filter(|failure| {
                        let current =
                            self.candidate_failure_evidence_for_state(state, &failure.candidate);
                        Self::candidate_failure_evidence_changed(failure, &current)
                    })
                    .map(|failure| failure.candidate.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        if !rearmed_candidates.is_empty() {
            attempt
                .candidate_failures
                .retain(|failure| !rearmed_candidates.contains(&failure.candidate));
        }
        let retry_due = attempt
            .candidate_failures
            .iter()
            .any(|failure| failure.transient_retry_due_at(now));
        if attempt.state != PlaylistResolutionAttemptState::Failed
            || (rearmed_candidates.is_empty() && !retry_due)
        {
            return false;
        }
        Self::prepare_failed_attempt_for_retry(attempt);
        self.last_attached_media_resolution_trigger = None;
        true
    }

    pub(super) fn rearm_failed_playlist_candidates_for_explicit_provider(
        &mut self,
        provider_id: &GuiMediaSourceProviderId,
    ) -> bool {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        let previous_len = attempt.candidate_failures.len();
        attempt
            .candidate_failures
            .retain(|failure| &failure.candidate.provider_id() != provider_id);
        if previous_len == attempt.candidate_failures.len() {
            return false;
        }
        if attempt.state == PlaylistResolutionAttemptState::Failed {
            Self::prepare_failed_attempt_for_retry(attempt);
        }
        self.last_attached_media_resolution_trigger = None;
        true
    }

    pub(super) fn begin_playlist_resolution_candidate_load(
        &mut self,
        candidate: media_resolution::GuiMediaResolutionCandidate,
        started: &StartedMediaLoad,
    ) {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        attempt.candidate_provider = Some(candidate.provider_id());
        attempt.candidate = Some(candidate);
        if !matches!(
            attempt
                .candidate
                .as_ref()
                .map(|candidate| candidate.target()),
            Some(media_resolution::GuiMediaResolutionTarget::PlexStream(_))
        ) {
            attempt.candidate_plex_operation_context = None;
        }
        attempt.player_command_id = started.player_command_id;
        attempt.player_media_generation = started.player_media_generation;
        attempt.state = PlaylistResolutionAttemptState::Loading;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
    }

    pub(super) fn complete_current_playlist_resolution_from_current_player(
        &mut self,
        provider_id: GuiMediaSourceProviderId,
    ) {
        let current_file = self.player_local_file.clone();
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        attempt.candidate_failures.retain(|failure| {
            let matches_confirmed_file = current_file.as_ref().is_some_and(|file| {
                file.path
                    .as_deref()
                    .is_some_and(|path| failure.candidate.matches_loaded_target(path))
                    || failure.candidate.matches_loaded_target(&file.name)
            });
            failure.candidate.provider_id() != provider_id && !matches_confirmed_file
        });
        attempt.candidate_provider = Some(provider_id);
        attempt.candidate = None;
        attempt.candidate_plex_operation_context = None;
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Active;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
    }

    pub(super) fn fail_playlist_resolution_candidate(
        &mut self,
        candidate: media_resolution::GuiMediaResolutionCandidate,
    ) {
        self.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::ContextDependent,
            Instant::now(),
        );
    }

    fn fail_playlist_resolution_candidate_at(
        &mut self,
        candidate: media_resolution::GuiMediaResolutionCandidate,
        disposition: CandidateFailureDisposition,
        now: Instant,
    ) {
        let evidence = self.candidate_failure_evidence(&candidate);
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        if let Some(failure) = attempt
            .candidate_failures
            .iter_mut()
            .find(|failure| failure.candidate == candidate)
        {
            failure.disposition = disposition;
            failure.failure_count = failure.failure_count.saturating_add(1);
            failure.next_retry_at = (disposition == CandidateFailureDisposition::Transient)
                .then(|| transient_candidate_retry_delay(failure.failure_count))
                .flatten()
                .map(|delay| now + delay);
            failure.evidence = evidence;
        } else {
            let failure_count = 1;
            let next_retry_at = (disposition == CandidateFailureDisposition::Transient)
                .then(|| transient_candidate_retry_delay(failure_count))
                .flatten()
                .map(|delay| now + delay);
            attempt
                .candidate_failures
                .push(PlaylistResolutionCandidateFailure {
                    candidate: candidate.clone(),
                    disposition,
                    failure_count,
                    next_retry_at,
                    evidence,
                });
        }
        attempt.candidate_provider = Some(candidate.provider_id());
        attempt.candidate = Some(candidate);
        attempt.candidate_plex_operation_context = None;
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Failed;
        attempt.fallback_pending = true;
        attempt.handoff_pending = false;
        self.last_attached_media_resolution_trigger = None;
    }

    fn clear_rejected_resolution_placeholder(
        &mut self,
        candidate: &media_resolution::GuiMediaResolutionCandidate,
    ) {
        let rejected_placeholder_is_current = self.player_local_file_placeholder
            && self.player_local_file.as_ref().is_some_and(|file| {
                file.path
                    .as_deref()
                    .is_some_and(|path| candidate.matches_loaded_target(path))
                    || candidate.matches_loaded_target(&file.name)
            });
        if rejected_placeholder_is_current {
            self.player_local_file = None;
            self.player_local_file_placeholder = false;
            self.player_position_seconds = None;
        }
    }

    pub(super) fn tracked_playlist_resolution_load_matches_outcome(
        &self,
        outcome: &PlayerMediaLoadOutcome,
    ) -> bool {
        self.playlist_resolution_attempt
            .as_ref()
            .filter(|attempt| {
                attempt.player_command_id.is_some()
                    && attempt.state == PlaylistResolutionAttemptState::Loading
            })
            .and_then(|attempt| attempt.candidate.as_ref())
            .is_some_and(|candidate| {
                candidate.matches_loaded_target(&outcome.requested_target)
                    || outcome
                        .loaded_target
                        .as_deref()
                        .is_some_and(|target| candidate.matches_loaded_target(target))
            })
    }

    pub(super) fn tracked_playlist_resolution_load_matches_local_file(
        &self,
        update: &LocalFileUpdate,
    ) -> bool {
        self.playlist_resolution_attempt
            .as_ref()
            .filter(|attempt| {
                attempt.player_command_id.is_some()
                    && attempt.state == PlaylistResolutionAttemptState::Loading
            })
            .and_then(|attempt| attempt.candidate.as_ref())
            .is_some_and(|candidate| {
                update
                    .path
                    .as_deref()
                    .is_some_and(|path| candidate.matches_loaded_target(path))
                    || candidate.matches_loaded_target(&update.name)
            })
    }

    fn player_progress_generation_matches(
        expected: Option<PlayerMediaGeneration>,
        observed: Option<PlayerMediaGeneration>,
    ) -> bool {
        match (expected, observed) {
            (Some(expected), Some(observed)) => expected == observed,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    fn update_pending_logical_override_from_command_progress(
        &mut self,
        progress: PlayerCommandProgress,
    ) {
        if self.clear_pending_logical_override_superseded_by_generation(progress.media_generation) {
            return;
        }
        let (should_clear_override, confirmed_logical_file, rejected_logical_file) = {
            let Some(pending) = self.pending_logical_media_override.as_mut() else {
                return;
            };
            if pending.player_command_id != Some(progress.command_id)
                || !Self::player_progress_generation_matches(
                    pending.player_media_generation,
                    progress.media_generation,
                )
            {
                return;
            }
            if pending.player_media_generation.is_none() {
                pending.player_media_generation = progress.media_generation;
            }
            match progress.state {
                PlayerCommandProgressState::Accepted => (false, None, None),
                PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                    pending.load_completed = true;
                    let confirmed = pending
                        .logical_file_observed
                        .then(|| pending.logical_file.clone());
                    (pending.logical_file_observed, confirmed, None)
                }
                PlayerCommandProgressState::Finished(
                    PlayerCommandResult::Failed(_) | PlayerCommandResult::Superseded,
                ) => (true, None, Some(pending.logical_file.clone())),
            }
        };
        if confirmed_logical_file
            .as_ref()
            .is_some_and(|logical_file| self.player_local_file.as_ref() == Some(logical_file))
        {
            self.player_local_file_placeholder = false;
        }
        if rejected_logical_file
            .as_ref()
            .is_some_and(|logical_file| self.player_local_file.as_ref() == Some(logical_file))
        {
            self.player_local_file = None;
            self.player_local_file_placeholder = false;
            self.player_position_seconds = None;
        }
        if should_clear_override {
            self.pending_logical_media_override = None;
        }
    }

    fn clear_pending_logical_override_superseded_by_generation(
        &mut self,
        observed: Option<PlayerMediaGeneration>,
    ) -> bool {
        let Some(observed) = observed else {
            return false;
        };
        let superseded = self
            .pending_logical_media_override
            .as_ref()
            .and_then(|pending| pending.player_media_generation)
            .is_some_and(|expected| observed > expected);
        if !superseded {
            return false;
        }
        let logical_file = self
            .pending_logical_media_override
            .as_ref()
            .map(|pending| pending.logical_file.clone());
        if self.player_local_file_placeholder
            && logical_file
                .as_ref()
                .is_some_and(|logical_file| self.player_local_file.as_ref() == Some(logical_file))
        {
            self.player_local_file = None;
            self.player_local_file_placeholder = false;
            self.player_position_seconds = None;
        }
        self.pending_logical_media_override = None;
        true
    }

    pub(super) fn reconcile_pending_logical_override_media_generation(
        &mut self,
        observed: Option<PlayerMediaGeneration>,
    ) {
        self.clear_pending_logical_override_superseded_by_generation(observed);
    }

    pub(super) fn handle_playlist_resolution_command_progress(
        &mut self,
        progress: PlayerCommandProgress,
    ) {
        self.update_pending_logical_override_from_command_progress(progress);

        let terminal_result = {
            let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
                return;
            };
            if attempt.player_command_id != Some(progress.command_id)
                || attempt.playlist_generation != self.playlist_resolution.generation
                || !Self::player_progress_generation_matches(
                    attempt.player_media_generation,
                    progress.media_generation,
                )
            {
                return;
            }
            if attempt.player_media_generation.is_none() {
                attempt.player_media_generation = progress.media_generation;
            }
            match progress.state {
                PlayerCommandProgressState::Accepted => return,
                PlayerCommandProgressState::Finished(result) => result,
            }
        };

        match terminal_result {
            PlayerCommandResult::Completed => {
                if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                    if let Some(candidate) = attempt.candidate.as_ref() {
                        attempt
                            .candidate_failures
                            .retain(|failure| &failure.candidate != candidate);
                    }
                    attempt.state = PlaylistResolutionAttemptState::Active;
                    attempt.candidate_plex_operation_context = None;
                    attempt.fallback_pending = false;
                    attempt.handoff_pending = true;
                }
                let confirmed_current_player = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.candidate.as_ref())
                    .is_some_and(|candidate| {
                        self.player_local_file.as_ref().is_some_and(|file| {
                            file.path
                                .as_deref()
                                .is_some_and(|path| candidate.matches_loaded_target(path))
                                || candidate.matches_loaded_target(&file.name)
                        })
                    });
                if confirmed_current_player {
                    self.player_local_file_placeholder = false;
                }
                self.last_attached_media_resolution_trigger = None;
            }
            PlayerCommandResult::Failed(kind) => {
                let failed_candidate = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.candidate.clone());
                if let Some(candidate) = failed_candidate {
                    self.clear_rejected_resolution_placeholder(&candidate);
                    self.fail_playlist_resolution_candidate_at(
                        candidate,
                        CandidateFailureDisposition::for_command_failure(kind),
                        Instant::now(),
                    );
                }
            }
            PlayerCommandResult::Superseded => {
                let superseded_candidate = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.candidate.clone());
                if let Some(candidate) = superseded_candidate.as_ref() {
                    self.clear_rejected_resolution_placeholder(candidate);
                }
                if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                    attempt.state = PlaylistResolutionAttemptState::Superseded;
                    attempt.fallback_pending = false;
                    attempt.handoff_pending = false;
                }
            }
        }
    }

    pub(super) fn handle_playlist_media_load_outcome(&mut self, outcome: &PlayerMediaLoadOutcome) {
        let matching_load = self
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| {
                (attempt.state == PlaylistResolutionAttemptState::Loading)
                    .then_some(attempt.candidate.as_ref())
                    .flatten()
                    .filter(|candidate| {
                        candidate.matches_loaded_target(&outcome.requested_target)
                            || outcome
                                .loaded_target
                                .as_deref()
                                .is_some_and(|target| candidate.matches_loaded_target(target))
                    })
                    .map(|candidate| (candidate.clone(), attempt.player_command_id.is_some()))
            });
        let Some((candidate, tracked)) = matching_load else {
            return;
        };
        if outcome.succeeded() {
            // A tracked command remains Loading until its correlated command
            // completion arrives. The media observation is only provisional.
            if tracked {
                return;
            }
            if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                attempt
                    .candidate_failures
                    .retain(|failure| failure.candidate != candidate);
                attempt.state = PlaylistResolutionAttemptState::Active;
                attempt.candidate_plex_operation_context = None;
                attempt.fallback_pending = false;
                attempt.handoff_pending = true;
            }
            self.last_attached_media_resolution_trigger = None;
        } else {
            let kind = outcome
                .failure
                .as_ref()
                .map(|failure| failure.kind)
                .unwrap_or(PlayerMediaLoadFailureKind::Unknown);
            self.clear_rejected_resolution_placeholder(&candidate);
            self.fail_playlist_resolution_candidate_at(
                candidate,
                CandidateFailureDisposition::for_media_load_failure(kind),
                Instant::now(),
            );
        }
    }

    pub(super) fn handle_untracked_playlist_local_file_observation(
        &mut self,
        update: &LocalFileUpdate,
    ) {
        let matching_observation =
            self.playlist_resolution_attempt
                .as_ref()
                .is_some_and(|attempt| {
                    attempt.player_command_id.is_none()
                        && attempt.state == PlaylistResolutionAttemptState::Loading
                        && attempt.candidate.as_ref().is_some_and(|candidate| {
                            update
                                .path
                                .as_deref()
                                .is_some_and(|path| candidate.matches_loaded_target(path))
                                || candidate.matches_loaded_target(&update.name)
                        })
                });
        if !matching_observation {
            return;
        }
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            if let Some(candidate) = attempt.candidate.as_ref() {
                attempt
                    .candidate_failures
                    .retain(|failure| &failure.candidate != candidate);
            }
            attempt.state = PlaylistResolutionAttemptState::Active;
            attempt.candidate_plex_operation_context = None;
            attempt.fallback_pending = false;
            attempt.handoff_pending = true;
        }
        self.last_attached_media_resolution_trigger = None;
    }

    pub(in crate::app::runtime_owner) fn take_playlist_resolution_fallback_pending(
        &mut self,
    ) -> bool {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        std::mem::take(&mut attempt.fallback_pending)
    }

    pub(in crate::app::runtime_owner) fn take_playlist_resolution_handoff_pending(
        &mut self,
    ) -> bool {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        std::mem::take(&mut attempt.handoff_pending)
    }

    pub(in crate::app::runtime_owner) fn playlist_resolution_source_state_for_projection(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> Option<(
        usize,
        super::super::super::shell_state::GuiPlaylistSourceState,
    )> {
        let attempt = self.playlist_resolution_attempt.as_ref()?;
        if attempt.playlist_generation != self.playlist_resolution.generation
            || attempt.state == PlaylistResolutionAttemptState::Superseded
        {
            return None;
        }
        let index = state
            .main_window
            .playlist
            .iter()
            .position(|row| row.entry_id == attempt.row_id)?;
        let mut source_state = state.main_window.playlist[index].source_state.clone();
        let Some(provider_id) = attempt.candidate_provider.clone() else {
            source_state.clear_resolved_provider();
            let matching_plex_miss = self.plex_miss_state.as_ref().filter(|miss| {
                miss.key.row_id == attempt.row_id
                    && miss.key.playlist_generation == attempt.playlist_generation
                    && miss.key.policy == attempt.policy
            });
            let resolution_in_flight = self.attached_media_search_in_flight()
                || self.media_match_remote_lookup_rx.is_some()
                || self.plex_stream_resolution_owns_cache_snapshot()
                || matching_plex_miss.is_some_and(|miss| miss.retry_in_flight);
            source_state.status = if resolution_in_flight {
                GuiPlaylistSourceStatus::Resolving
            } else {
                GuiPlaylistSourceStatus::Missing
            };
            source_state.detail = Some(if resolution_in_flight {
                "Searching the available media providers.".to_owned()
            } else if matching_plex_miss.is_some() {
                "No provider found a usable source; Plex will retry automatically.".to_owned()
            } else {
                "No available provider found a usable source.".to_owned()
            });
            source_state.resolution_steps = attempt
                .candidate_failures
                .iter()
                .map(|failure| {
                    let candidate = &failure.candidate;
                    let provider_id = candidate.provider_id();
                    GuiPlaylistResolutionStep {
                        label: if provider_id == GuiMediaSourceProviderId::plex_stream() {
                            "Plex Stream"
                        } else if provider_id == GuiMediaSourceProviderId::media_matching() {
                            "Media Matching"
                        } else {
                            "Local"
                        }
                        .to_owned(),
                        provider_id,
                        status: GuiPlaylistSourceStatus::Failed,
                        detail: Some(
                            "The attached player did not complete this candidate load.".to_owned(),
                        ),
                    }
                })
                .collect();
            return Some((index, source_state));
        };
        source_state.set_resolved_provider(provider_id.clone());
        source_state.status = match attempt.state {
            PlaylistResolutionAttemptState::Resolving => GuiPlaylistSourceStatus::Resolving,
            PlaylistResolutionAttemptState::Loading => GuiPlaylistSourceStatus::Loading,
            PlaylistResolutionAttemptState::Active => GuiPlaylistSourceStatus::Active,
            PlaylistResolutionAttemptState::Failed => GuiPlaylistSourceStatus::Failed,
            PlaylistResolutionAttemptState::Superseded => return None,
        };
        let provider_label = source_state.current_label.clone();
        let detail = match attempt.state {
            PlaylistResolutionAttemptState::Resolving => {
                format!("Resolving media with {provider_label}.")
            }
            PlaylistResolutionAttemptState::Loading => {
                format!("Waiting for the attached player to confirm the {provider_label} load.")
            }
            PlaylistResolutionAttemptState::Active => {
                format!("The attached player confirmed the {provider_label} load.")
            }
            PlaylistResolutionAttemptState::Failed if attempt.fallback_pending => {
                format!("The {provider_label} load failed; trying the next available provider.")
            }
            PlaylistResolutionAttemptState::Failed => {
                format!("The attached player rejected the {provider_label} load.")
            }
            PlaylistResolutionAttemptState::Superseded => return None,
        };
        source_state.detail = Some(detail.clone());
        source_state.resolution_steps = attempt
            .candidate_failures
            .iter()
            .map(|failure| {
                let candidate = &failure.candidate;
                let provider_id = candidate.provider_id();
                GuiPlaylistResolutionStep {
                    label: if provider_id == GuiMediaSourceProviderId::plex_stream() {
                        "Plex Stream"
                    } else if provider_id == GuiMediaSourceProviderId::media_matching() {
                        "Media Matching"
                    } else {
                        "Local"
                    }
                    .to_owned(),
                    provider_id,
                    status: GuiPlaylistSourceStatus::Failed,
                    detail: Some(
                        "The attached player did not complete this candidate load.".to_owned(),
                    ),
                }
            })
            .collect();
        if source_state.status != GuiPlaylistSourceStatus::Failed {
            source_state
                .resolution_steps
                .push(GuiPlaylistResolutionStep {
                    provider_id,
                    label: provider_label,
                    status: source_state.status,
                    detail: Some(detail),
                });
        }
        Some((index, source_state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime_owner::player::media_resolution::GuiMediaResolutionPlan;
    use crate::app::{
        GuiTestPlayerAdapter, StoredClientSettingsMvp,
        runtime_owner::GuiPendingLogicalMediaOverride,
    };
    use sorotte_player_api::{PlayerCommandFailureKind, PlayerCommandId};

    fn local_candidate(path: &str) -> media_resolution::GuiMediaResolutionCandidate {
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_user_media_candidate(
            path.to_owned(),
            GuiUserMediaTargetResolutionSource::QuickLocal,
        );
        plan.best_candidate().cloned().expect("local candidate")
    }

    fn media_search_candidate(path: &str) -> media_resolution::GuiMediaResolutionCandidate {
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_user_media_candidate(
            path.to_owned(),
            GuiUserMediaTargetResolutionSource::MediaSearchIndex,
        );
        plan.best_candidate()
            .cloned()
            .expect("media-search candidate")
    }

    fn media_match_candidate(path: &str) -> media_resolution::GuiMediaResolutionCandidate {
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_media_match_candidate(path.to_owned());
        plan.best_candidate()
            .cloned()
            .expect("Media Matching candidate")
    }

    fn plex_candidate() -> media_resolution::GuiMediaResolutionCandidate {
        let playlist_uri = sorotte_plex::PlexPlaylistUri {
            machine_identifier: "machine".to_owned(),
            rating_key: "123".to_owned(),
            title: Some("Episode".to_owned()),
            file_name: Some("episode.mkv".to_owned()),
            duration_millis: None,
            size_bytes: None,
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        };
        let stream_target = sorotte_plex::PlexStreamTarget {
            logical_file: LocalFileUpdate::new("episode.mkv"),
            matched_item: sorotte_plex::PlexMatchedItem {
                rating_key: "123".to_owned(),
                title: "Episode".to_owned(),
                media_type: sorotte_plex::PlexMediaType::Episode,
                duration_millis: None,
            },
            playlist_uri,
            playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(
                "https://plex.example/video?token=secret",
            ),
        };
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_plex_stream_candidate(stream_target);
        plan.best_candidate().cloned().expect("Plex candidate")
    }

    fn started(command_id: u64) -> StartedMediaLoad {
        StartedMediaLoad {
            feedback_message: "started".to_owned(),
            player_command_id: Some(PlayerCommandId::new(command_id)),
            player_media_generation: None,
        }
    }

    fn shell_state() -> SorotteGuiShellAppState {
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default())
    }

    struct TrackedFailureTelemetryPlayer {
        command_progress: VecDeque<PlayerCommandProgress>,
        media_load_outcomes: VecDeque<PlayerMediaLoadOutcome>,
    }

    impl PlayerAdapter for TrackedFailureTelemetryPlayer {
        fn name(&self) -> &'static str {
            "tracked-failure-telemetry"
        }

        fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
            self.command_progress.pop_front()
        }

        fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
            self.media_load_outcomes.pop_front()
        }
    }

    struct RejectingOpenPlayer;

    impl PlayerAdapter for RejectingOpenPlayer {
        fn name(&self) -> &'static str {
            "rejecting-open"
        }

        fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            Err(sorotte_player_api::PlayerError::OperationFailed(
                "rejected".to_owned(),
            ))
        }
    }

    fn owner_after_tracked_rich_failure(
        kind: PlayerMediaLoadFailureKind,
    ) -> GuiPersistedConfigRuntimeOwner {
        let row_id = GuiPlaylistEntryId::next();
        let target = "C:/media/episode.mkv";
        let candidate = local_candidate(target);
        let command_id = PlayerCommandId::new(101);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 1;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            1,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(101));
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
            TrackedFailureTelemetryPlayer {
                command_progress: VecDeque::from([PlayerCommandProgress::finished(
                    command_id,
                    None,
                    None,
                    None,
                    PlayerCommandResult::Failed(PlayerCommandFailureKind::MediaEnded),
                )]),
                media_load_outcomes: VecDeque::from([PlayerMediaLoadOutcome::failure(
                    target,
                    None,
                    kind,
                    "classified rich failure",
                )]),
            },
        )));

        // Production collection drains command progress first, but production
        // application deliberately applies media outcomes first.
        owner.refresh_player_state_impl();
        owner
    }

    #[test]
    fn player_failure_kinds_map_to_deliberate_candidate_dispositions() {
        for kind in [
            PlayerCommandFailureKind::TimedOut,
            PlayerCommandFailureKind::TransportDisconnected,
        ] {
            assert_eq!(
                CandidateFailureDisposition::for_command_failure(kind),
                CandidateFailureDisposition::Transient
            );
        }
        assert_eq!(
            CandidateFailureDisposition::for_command_failure(PlayerCommandFailureKind::MediaEnded),
            CandidateFailureDisposition::Permanent
        );
        assert_eq!(
            CandidateFailureDisposition::for_command_failure(PlayerCommandFailureKind::Unknown),
            CandidateFailureDisposition::ContextDependent
        );

        for kind in [
            PlayerMediaLoadFailureKind::Network,
            PlayerMediaLoadFailureKind::LoadAborted,
        ] {
            assert_eq!(
                CandidateFailureDisposition::for_media_load_failure(kind),
                CandidateFailureDisposition::Transient
            );
        }
        assert_eq!(
            CandidateFailureDisposition::for_media_load_failure(
                PlayerMediaLoadFailureKind::FormatUnsupported
            ),
            CandidateFailureDisposition::Permanent
        );
        for kind in [
            PlayerMediaLoadFailureKind::HelperMissing,
            PlayerMediaLoadFailureKind::HelperBroken,
            PlayerMediaLoadFailureKind::Unknown,
        ] {
            assert_eq!(
                CandidateFailureDisposition::for_media_load_failure(kind),
                CandidateFailureDisposition::ContextDependent
            );
        }
    }

    #[test]
    fn tracked_network_outcome_beats_later_generic_command_failure() {
        let mut owner = owner_after_tracked_rich_failure(PlayerMediaLoadFailureKind::Network);
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        let failure = attempt.candidate_failures.first().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Failed);
        assert_eq!(failure.disposition, CandidateFailureDisposition::Transient);
        assert_eq!(failure.failure_count, 1);
        let deadline = failure.next_retry_at.expect("transient retry deadline");

        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), deadline));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Resolving);
        assert_eq!(attempt.candidate_failures[0].failure_count, 1);
    }

    #[test]
    fn tracked_unsupported_outcome_stays_permanent_after_generic_command_failure() {
        let mut owner =
            owner_after_tracked_rich_failure(PlayerMediaLoadFailureKind::FormatUnsupported);
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        let failure = attempt.candidate_failures.first().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Failed);
        assert_eq!(failure.disposition, CandidateFailureDisposition::Permanent);
        assert_eq!(failure.failure_count, 1);
        assert!(failure.next_retry_at.is_none());

        assert!(!owner.reconcile_failed_playlist_candidates(
            &shell_state(),
            Instant::now() + Duration::from_secs(3_600),
        ));
        let failure = &owner
            .playlist_resolution_attempt
            .as_ref()
            .unwrap()
            .candidate_failures[0];
        assert_eq!(failure.disposition, CandidateFailureDisposition::Permanent);
        assert_eq!(failure.failure_count, 1);
    }

    #[test]
    fn tracked_helper_outcome_rearms_on_player_context_without_generic_overwrite() {
        let mut owner = owner_after_tracked_rich_failure(PlayerMediaLoadFailureKind::HelperMissing);
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        let failure = attempt.candidate_failures.first().unwrap();
        assert_eq!(
            failure.disposition,
            CandidateFailureDisposition::ContextDependent
        );
        assert_eq!(failure.failure_count, 1);
        assert!(failure.next_retry_at.is_none());

        owner.player_attachment_epoch = owner.player_attachment_epoch.wrapping_add(1);
        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), Instant::now()));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Resolving);
        assert!(attempt.candidate_failures.is_empty());
    }

    #[test]
    fn permanent_candidate_failure_stays_terminal_across_repeated_ticks() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/permanently-unsupported.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 1;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            1,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let failed_at = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            candidate.clone(),
            CandidateFailureDisposition::Permanent,
            failed_at,
        );
        let state = shell_state();

        for elapsed in [
            Duration::from_secs(2),
            Duration::from_secs(60),
            Duration::from_secs(600),
        ] {
            assert!(!owner.reconcile_failed_playlist_candidates(&state, failed_at + elapsed));
            let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
            assert_eq!(attempt.state, PlaylistResolutionAttemptState::Failed);
            assert_eq!(attempt.candidate_failures.len(), 1);
            assert_eq!(attempt.candidate_failures[0].candidate, candidate);
            assert!(attempt.candidate_failures[0].next_retry_at.is_none());
            assert!(!owner.active_playlist_candidate_retry_due());
        }
    }

    #[test]
    fn transient_candidate_failure_uses_two_four_eight_second_backoff_then_stops() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/transient.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 2;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            2,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let state = shell_state();
        let mut failure_time = Instant::now();

        for (failure_count, expected_delay) in [(1, 2), (2, 4), (3, 8)] {
            owner.fail_playlist_resolution_candidate_at(
                candidate.clone(),
                CandidateFailureDisposition::Transient,
                failure_time,
            );
            let failure = &owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .candidate_failures[0];
            assert_eq!(failure.failure_count, failure_count);
            let deadline = failure.next_retry_at.expect("retry should be scheduled");
            assert_eq!(
                deadline.duration_since(failure_time),
                Duration::from_secs(expected_delay)
            );
            assert!(
                !owner.reconcile_failed_playlist_candidates(
                    &state,
                    deadline - Duration::from_millis(1)
                )
            );
            assert!(owner.reconcile_failed_playlist_candidates(&state, deadline));
            failure_time = deadline;
        }

        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::Transient,
            failure_time,
        );
        let terminal = &owner
            .playlist_resolution_attempt
            .as_ref()
            .unwrap()
            .candidate_failures[0];
        assert_eq!(terminal.failure_count, TRANSIENT_CANDIDATE_MAX_FAILURES);
        assert!(terminal.next_retry_at.is_none());
        assert!(!owner.active_playlist_candidate_retry_due());
        assert!(!owner.reconcile_failed_playlist_candidates(
            &state,
            failure_time + Duration::from_secs(3_600)
        ));
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Failed
        );
    }

    #[test]
    fn due_transient_candidate_becomes_eligible_without_clearing_other_failures() {
        let row_id = GuiPlaylistEntryId::next();
        let transient = local_candidate("C:/media/transient.mkv");
        let permanent = local_candidate("C:/media/permanent.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 3;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            3,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            transient.clone(),
            CandidateFailureDisposition::Transient,
            now - Duration::from_secs(3),
        );
        owner.fail_playlist_resolution_candidate_at(
            permanent.clone(),
            CandidateFailureDisposition::Permanent,
            now - Duration::from_secs(3),
        );

        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));
        assert_eq!(
            owner.failed_playlist_resolution_candidates(),
            vec![permanent]
        );
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.candidate_failures.len(), 2);
        assert_eq!(attempt.candidate_failures[0].candidate, transient);
        assert_eq!(attempt.candidate_failures[0].failure_count, 1);
    }

    #[test]
    fn overdue_transient_stays_excluded_while_fallback_is_active() {
        let row_id = GuiPlaylistEntryId::next();
        let transient = local_candidate("C:/media/transient-primary.mkv");
        let fallback = media_match_candidate("C:/media/healthy-fallback.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 3;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            3,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            transient.clone(),
            CandidateFailureDisposition::Transient,
            now - Duration::from_secs(3),
        );
        owner.begin_playlist_resolution_candidate_load(fallback.clone(), &started(44));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(44),
            None,
            None,
            None,
            PlayerCommandResult::Completed,
        ));

        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Active
        );
        assert_eq!(
            owner.failed_playlist_resolution_candidates(),
            vec![transient.clone()],
            "an overdue primary must not replace confirmed fallback playback"
        );
        assert!(!owner.active_playlist_candidate_retry_due());

        owner.fail_playlist_resolution_candidate_at(
            fallback.clone(),
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        assert!(owner.active_playlist_candidate_retry_due());
        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));
        assert_eq!(
            owner.failed_playlist_resolution_candidates(),
            vec![fallback],
            "the overdue transient becomes eligible only after Failed transitions to Resolving"
        );
    }

    #[test]
    fn context_dependent_failure_rearms_after_player_attachment_epoch_changes() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/context-dependent.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        owner.player_attachment_epoch = owner.player_attachment_epoch.wrapping_add(1);

        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Resolving);
        assert!(attempt.candidate_failures.is_empty());
    }

    #[test]
    fn helper_health_change_rearms_only_extractor_page_candidates() {
        let now = Instant::now();
        let mut local_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        local_owner.playlist_resolution.generation = 4;
        local_owner.ensure_playlist_resolution_attempt(
            GuiPlaylistEntryId::next(),
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        local_owner.fail_playlist_resolution_candidate_at(
            local_candidate("C:/media/episode.mkv"),
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        local_owner.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Broken;
        assert!(!local_owner.reconcile_failed_playlist_candidates(&shell_state(), now));

        let mut extractor_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        extractor_owner.playlist_resolution.generation = 4;
        extractor_owner.ensure_playlist_resolution_attempt(
            GuiPlaylistEntryId::next(),
            4,
            "https://www.youtube.com/watch?v=episode",
            GuiPlaylistSourcePolicy::Automatic,
        );
        extractor_owner.fail_playlist_resolution_candidate_at(
            local_candidate("https://www.youtube.com/watch?v=episode"),
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        extractor_owner.stream_helper_runtime_snapshot.health = GuiStreamHelperHealth::Broken;
        assert!(extractor_owner.reconcile_failed_playlist_candidates(&shell_state(), now));
    }

    #[test]
    fn permanent_local_failure_rearms_only_after_its_file_evidence_changes() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-permanent-candidate-evidence-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("episode.mkv");
        std::fs::write(&path, b"broken").unwrap();
        let candidate = local_candidate(&path.to_string_lossy());
        let row_id = GuiPlaylistEntryId::next();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 5;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            5,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::Permanent,
            now,
        );

        assert!(!owner.reconcile_failed_playlist_candidates(&shell_state(), now));
        std::fs::write(&path, b"repaired with different length").unwrap();
        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));
        assert!(
            owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .candidate_failures
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn context_dependent_failures_rearm_on_local_index_and_media_match_result_changes() {
        let row_id = GuiPlaylistEntryId::next();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 6;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            6,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            media_search_candidate("C:/media/indexed.mkv"),
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        owner.attached_media_search_index_revision =
            owner.attached_media_search_index_revision.wrapping_add(1);
        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));

        owner.fail_playlist_resolution_candidate_at(
            media_match_candidate("C:/media/matched.mkv"),
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        owner.media_match_remote_lookup_result = Some(GuiMediaMatchRemoteLookupResult {
            trigger_key: "changed-result".to_owned(),
            candidate_path: Some("C:/media/matched.mkv".to_owned()),
        });
        assert!(owner.reconcile_failed_playlist_candidates(&shell_state(), now));
    }

    #[test]
    fn context_dependent_plex_failure_rearms_on_operation_context_change() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = plex_candidate();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 7;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            7,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForcePlex,
        );
        let initial_state = shell_state();
        let initial_context =
            owner.plex_operation_context(&owner.runtime_operation_settings(&initial_state));
        owner.last_attached_media_resolution_trigger = Some(GuiAutomaticMediaResolutionTrigger {
            target: "episode.mkv".to_owned(),
            playlist_entry_id: Some(row_id),
            playlist_generation: 7,
            source_provider: "plex-stream".to_owned(),
            plex_operation_context: Some(initial_context),
            roots: Vec::new(),
            media_match_remote_targets: String::new(),
            current_player_path: None,
            index_revision: 0,
            retry_due: false,
        });
        let now = Instant::now();
        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::ContextDependent,
            now,
        );
        assert!(!owner.reconcile_failed_playlist_candidates(&initial_state, now));

        let changed_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                plex_plugin_enabled: Some(true),
                plex_streaming_enabled: Some(true),
                plex_user_token: Some("different-account-token".into()),
                plex_selected_server_id: Some("different-machine".to_owned()),
                plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
                plex_selected_server_token: Some("different-server-token".into()),
                ..StoredClientSettingsMvp::default()
            });
        assert!(owner.reconcile_failed_playlist_candidates(&changed_state, now));
    }

    #[test]
    fn direct_plex_open_failure_records_context_and_rearms_when_it_changes() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = plex_candidate();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 8;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            8,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForcePlex,
        );
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RejectingOpenPlayer)));
        let initial_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                plex_plugin_enabled: Some(true),
                plex_streaming_enabled: Some(true),
                plex_user_token: Some("account-token".into()),
                plex_selected_server_id: Some("machine".to_owned()),
                plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
                plex_selected_server_token: Some("server-token".into()),
                ..StoredClientSettingsMvp::default()
            });
        let initial_context =
            owner.plex_operation_context(&owner.runtime_operation_settings(&initial_state));

        assert_eq!(
            owner.open_media_resolution_candidate(&initial_state, "episode.mkv", candidate, true,),
            SelectedPlaylistMediaSyncOutcome::NoChange
        );
        let attempt = owner
            .playlist_resolution_attempt
            .as_ref()
            .expect("playlist resolution attempt");
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Failed);
        assert_eq!(attempt.candidate_failures.len(), 1);
        assert_eq!(
            attempt.candidate_failures[0].disposition,
            CandidateFailureDisposition::ContextDependent
        );
        assert_eq!(
            attempt.candidate_failures[0]
                .evidence
                .plex_operation_context
                .as_ref(),
            Some(&initial_context)
        );

        let changed_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                plex_plugin_enabled: Some(true),
                plex_streaming_enabled: Some(true),
                plex_user_token: Some("different-account-token".into()),
                plex_selected_server_id: Some("different-machine".to_owned()),
                plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
                plex_selected_server_token: Some("different-server-token".into()),
                ..StoredClientSettingsMvp::default()
            });
        assert!(owner.reconcile_failed_playlist_candidates(&changed_state, Instant::now()));
        let attempt = owner
            .playlist_resolution_attempt
            .as_ref()
            .expect("playlist resolution attempt");
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Resolving);
        assert!(attempt.candidate_failures.is_empty());
    }

    #[test]
    fn detach_player_advances_attachment_epoch_only_when_an_attachment_ends() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        let initial_epoch = owner.player_attachment_epoch;

        owner.detach_player();
        assert_eq!(owner.player_attachment_epoch, initial_epoch.wrapping_add(1));
        owner.detach_player();
        assert_eq!(
            owner.player_attachment_epoch,
            initial_epoch.wrapping_add(1),
            "repeated empty detach calls are not new player lifecycles"
        );
    }

    #[test]
    fn explicit_same_provider_action_rearms_permanent_candidate() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/repaired-by-user.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 5;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            5,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForceLocal,
        );
        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::Permanent,
            Instant::now(),
        );

        assert!(
            owner.rearm_failed_playlist_candidates_for_explicit_provider(
                &GuiMediaSourceProviderId::local()
            )
        );
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Resolving);
        assert!(attempt.candidate_failures.is_empty());
    }

    #[test]
    fn accepted_open_stays_loading_until_matching_observed_completion() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/episode.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 9;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            9,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(11));

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            PlayerCommandId::new(11),
            Some(PlayerMediaGeneration::new(3)),
            None,
        ));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
        assert_eq!(
            attempt.player_media_generation,
            Some(PlayerMediaGeneration::new(3))
        );

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(11),
            Some(PlayerMediaGeneration::new(3)),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Active
        );
    }

    #[test]
    fn untracked_open_stays_loading_until_matching_file_observation() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/episode.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 9;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            9,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(
            candidate,
            &StartedMediaLoad {
                feedback_message: "started".to_owned(),
                player_command_id: None,
                player_media_generation: None,
            },
        );

        owner.handle_untracked_playlist_local_file_observation(
            &LocalFileUpdate::new("other.mkv").with_path("C:/media/other.mkv"),
        );
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Loading
        );
        owner.handle_untracked_playlist_local_file_observation(
            &LocalFileUpdate::new("episode.mkv").with_path("C:/media/episode.mkv"),
        );
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Active
        );
    }

    #[test]
    fn mismatched_command_or_generation_cannot_complete_current_attempt() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/episode.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(20));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            PlayerCommandId::new(20),
            Some(PlayerMediaGeneration::new(8)),
            None,
        ));

        for (command_id, generation) in [(19, 8), (20, 7)] {
            owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
                PlayerCommandId::new(command_id),
                Some(PlayerMediaGeneration::new(generation)),
                None,
                None,
                PlayerCommandResult::Completed,
            ));
        }
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Loading
        );
    }

    #[test]
    fn superseded_same_row_completion_cannot_activate_the_replacement_attempt() {
        let row_id = GuiPlaylistEntryId::next();
        let path = "C:/media/episode.mkv";
        let candidate = local_candidate(path);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 6;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            6,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForceLocal,
        );
        owner.begin_playlist_resolution_candidate_load(candidate.clone(), &started(30));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            PlayerCommandId::new(30),
            Some(PlayerMediaGeneration::new(1)),
            None,
        ));

        owner.player_local_file = Some(LocalFileUpdate::new("episode.mkv").with_path(path));
        owner.player_local_file_placeholder = true;
        owner.supersede_playlist_resolution_attempt();
        owner.ensure_playlist_resolution_attempt(
            row_id,
            6,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForceLocal,
        );
        assert!(
            !owner.current_player_matches_media_target(path),
            "an accepted placeholder must not satisfy a replacement source request"
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(31));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            PlayerCommandId::new(31),
            Some(PlayerMediaGeneration::new(2)),
            None,
        ));

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(30),
            Some(PlayerMediaGeneration::new(1)),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        let replacement = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(replacement.state, PlaylistResolutionAttemptState::Loading);
        assert_eq!(
            replacement.player_command_id,
            Some(PlayerCommandId::new(31))
        );

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(31),
            Some(PlayerMediaGeneration::new(2)),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Active
        );
    }

    #[test]
    fn current_media_match_candidate_retains_media_match_attribution() {
        let row_id = GuiPlaylistEntryId::next();
        let path = "C:/media/alternate-encode.mkv";
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_media_match_candidate(path.to_owned());
        let candidate = plan.best_candidate().cloned().unwrap();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 8;
        owner.player_local_file =
            Some(LocalFileUpdate::new("alternate-encode.mkv").with_path(path));
        owner.ensure_playlist_resolution_attempt(
            row_id,
            8,
            "episode.mkv",
            GuiPlaylistSourcePolicy::ForceMediaMatching,
        );

        assert_eq!(
            owner.open_media_resolution_candidate(&shell_state(), "episode.mkv", candidate, false),
            SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
        );
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert_eq!(
            attempt.candidate_provider,
            Some(GuiMediaSourceProviderId::media_matching())
        );
    }

    #[test]
    fn confirmed_external_load_recovers_failed_automatic_candidate() {
        let target = "C:/media/externally-recovered.mkv";
        let candidate = local_candidate(target);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        state.apply_shared_playlist_entries(vec![target.to_owned()], Some(0), false);
        state.main_window.active_playlist_index = Some(0);

        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        owner.active_shared_playlist_index = Some(0);
        owner.reconcile_local_shared_playlist_media_paths(&state);
        let row_id = state.main_window.playlist[0].entry_id;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            owner.playlist_resolution.generation,
            target,
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.fail_playlist_resolution_candidate_at(
            candidate,
            CandidateFailureDisposition::ContextDependent,
            Instant::now(),
        );
        owner.player_local_file =
            Some(LocalFileUpdate::new("externally-recovered.mkv").with_path(target));
        owner.player_local_file_placeholder = false;

        assert_eq!(
            owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
            SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
        );
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert_eq!(
            attempt.candidate_provider,
            Some(GuiMediaSourceProviderId::local())
        );
        assert!(attempt.candidate_failures.is_empty());
        assert!(owner.failed_playlist_resolution_candidates().is_empty());
    }

    #[test]
    fn terminal_failure_excludes_candidate_and_requests_fallback() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/broken.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 2;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            2,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate.clone(), &started(5));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(5),
            Some(PlayerMediaGeneration::new(1)),
            None,
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::Unknown),
        ));

        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Failed);
        assert_eq!(attempt.candidate_failures.len(), 1);
        assert_eq!(attempt.candidate_failures[0].candidate, candidate);
        assert!(attempt.fallback_pending);
        assert!(owner.last_attached_media_resolution_trigger.is_none());
    }

    #[test]
    fn logical_override_ignores_old_commands_and_wrong_media_generations() {
        let row_id = GuiPlaylistEntryId::next();
        let command_id = PlayerCommandId::new(22);
        let generation = PlayerMediaGeneration::new(7);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(
                "https://plex.example/stream?token=secret",
            ),
            logical_file: LocalFileUpdate::new("episode.mkv"),
            user_initiated: false,
            player_command_id: Some(command_id),
            player_media_generation: Some(generation),
            playlist_row_id: Some(row_id),
            playlist_generation: 4,
            load_completed: false,
            logical_file_observed: false,
        });

        for (observed_command, observed_generation) in [(21, 7), (22, 6)] {
            owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
                PlayerCommandId::new(observed_command),
                Some(PlayerMediaGeneration::new(observed_generation)),
                None,
                None,
                PlayerCommandResult::Completed,
            ));
        }
        assert!(
            !owner
                .pending_logical_media_override
                .as_ref()
                .unwrap()
                .load_completed
        );

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        let pending = owner.pending_logical_media_override.as_mut().unwrap();
        assert!(pending.load_completed);
        pending.logical_file_observed = true;

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        assert!(owner.pending_logical_media_override.is_none());
    }

    #[test]
    fn authoritative_newer_transport_generation_clears_pending_logical_override() {
        struct GenerationPlayer {
            update: Option<sorotte_player_api::PlayerTransportTelemetryUpdate>,
        }

        impl PlayerAdapter for GenerationPlayer {
            fn name(&self) -> &'static str {
                "generation-player"
            }

            fn take_transport_telemetry_update(
                &mut self,
            ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
                self.update.take()
            }
        }

        let row_id = GuiPlaylistEntryId::next();
        let logical_file = LocalFileUpdate::new("episode.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(
                "https://plex.example/stream?token=secret",
            ),
            logical_file: logical_file.clone(),
            user_initiated: false,
            player_command_id: Some(PlayerCommandId::new(22)),
            player_media_generation: Some(PlayerMediaGeneration::new(7)),
            playlist_row_id: Some(row_id),
            playlist_generation: 4,
            load_completed: false,
            logical_file_observed: false,
        });
        owner.player_local_file = Some(logical_file);
        owner.player_local_file_placeholder = true;

        owner.reconcile_pending_logical_override_media_generation(Some(
            PlayerMediaGeneration::new(6),
        ));
        assert!(
            owner.pending_logical_media_override.is_some(),
            "a delayed older generation must not clear the current pending override"
        );

        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(GenerationPlayer {
            update: Some(sorotte_player_api::PlayerTransportTelemetryUpdate::new(
                PlayerMediaGeneration::new(8),
                sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                    Duration::from_millis(1),
                ),
            )),
        })));
        owner.refresh_player_state_impl();

        assert!(
            owner.pending_logical_media_override.is_none(),
            "an authoritative newer media generation must invalidate an unconsumed Plex identity"
        );
        assert!(
            owner.player_local_file.is_none() && !owner.player_local_file_placeholder,
            "the superseded optimistic logical file must not outlive its generation"
        );
    }
}
